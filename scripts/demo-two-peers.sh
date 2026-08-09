#!/usr/bin/env bash
#
# Runs two CabalMesh peers side by side on one Mac, each with its own wallet,
# ledger and mesh identity, and each holding one side of a matching order.
#
# Why this exists
#
#   The interesting part of the intent flow only happens between two nodes:
#   peer A composes BUY, peer B composes SELL, they discover each other over
#   mDNS, mirror each other's order, agree the price deterministically, and the
#   sell side settles on-chain. One instance can only ever show half of that.
#
#   Both instances are the same binary. What separates them is CABALMESH_DATA_DIR
#   (see src-tauri/src/app_paths.rs) — a different directory means a different
#   vault, so each peer mints its own wallet on first launch rather than the two
#   sharing one identity and "trading" with themselves.
#
# Usage
#
#   scripts/demo-two-peers.sh            # launch both peers
#   scripts/demo-two-peers.sh --reset    # fresh orders, same wallets
#   scripts/demo-two-peers.sh --wipe     # new wallets too (unfunds the demo)
#
# `--reset` deliberately keeps each peer's vault. Devnet SOL is funded per
# address, so regenerating the wallets between runs would strand the funding in
# an identity nothing uses any more — which is exactly what you do not want
# after finally getting an airdrop through.
#
# The seeded orders match: BUY 0.1 SOL under 96.00 against SELL 0.1 SOL above
# 94.00, which clears at the midpoint, 95.00 USDC/SOL.
#
# Funding: settlement is a real Solana devnet escrow, so the SELL peer needs
# devnet SOL. This script never moves funds — it prints both addresses and the
# airdrop command for you to run.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${CABALMESH_DEMO_DIR:-$HOME/.cabalmesh-demo}"
PEER_A="$DEMO_DIR/peer-a"
PEER_B="$DEMO_DIR/peer-b"
VITE_PORT=1420
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}"
BINARY="$TARGET_DIR/debug/cabalmesh"

case "${1:-}" in
  --reset)
    echo "==> Clearing both ledgers (wallets kept)"
    rm -f "$PEER_A/intents.json" "$PEER_B/intents.json"
    rm -f "$PEER_A/pending_relay_txs.json" "$PEER_B/pending_relay_txs.json"
    ;;
  --wipe)
    echo "==> Wiping $DEMO_DIR, including both wallets"
    rm -rf "$PEER_A" "$PEER_B"
    ;;
esac

mkdir -p "$PEER_A" "$PEER_B"

# One open order per peer, written straight into the ledger so both windows
# start with something to match. The shapes here are cabal_core's serde
# representation: `price` is a bare number of cents (UsdPrice is transparent),
# and `amount.raw` is lamports.
seed_order() {
  local dir="$1" action="$2" condition="$3" price="$4"
  if [[ -f "$dir/intents.json" ]]; then
    echo "==> $(basename "$dir") already has a ledger; leaving it alone"
    return
  fi
  cat > "$dir/intents.json" <<JSON
{
  "intents": [
    {
      "id": "int-00000001",
      "draft": {
        "action": "$action",
        "asset": "SOL",
        "condition": { "kind": "$condition", "price": $price },
        "amount": { "raw": 100000000, "decimals": 9 },
        "mode": "SHARK",
        "privacy": "MEDIUM"
      },
      "status": { "status": "BROADCAST", "route_len": 1 },
      "createdAt": $(date +%s),
      "updatedAt": $(date +%s)
    }
  ],
  "nextId": 1
}
JSON
  echo "==> Seeded $(basename "$dir"): $action 0.1 SOL ${condition} $((price / 100)).00"
}

seed_order "$PEER_A" BUY under 9600
seed_order "$PEER_B" SELL above 9400

# The same feature set `tauri dev` builds with, so this reuses its artifacts
# rather than compiling the workspace a second time.
echo "==> Building the desktop binary"
(cd "$ROOT/src-tauri" && cargo build --no-default-features --features desktop-legacy)

# Both webviews load the dev server, so it runs once and is shared.
if ! curl -sf "http://localhost:$VITE_PORT" >/dev/null 2>&1; then
  echo "==> Starting the Vite dev server on $VITE_PORT"
  (cd "$ROOT" && npm run dev:mobile >"$DEMO_DIR/vite.log" 2>&1 &)
  for _ in $(seq 1 40); do
    curl -sf "http://localhost:$VITE_PORT" >/dev/null 2>&1 && break
    sleep 0.5
  done
fi

launch() {
  local dir="$1" name="$2"
  CABALMESH_DATA_DIR="$dir" "$BINARY" >"$dir/app.log" 2>&1 &
  echo "==> $name running as pid $! (log: $dir/app.log)"
}

launch "$PEER_A" "peer-a (BUY)"
sleep 3
launch "$PEER_B" "peer-b (SELL)"

# The address is written into each peer's chain snapshot on its first sync.
echo "==> Waiting for both wallets"
for _ in $(seq 1 40); do
  [[ -f "$PEER_A/snapshot.enc" && -f "$PEER_B/snapshot.enc" ]] && break
  sleep 0.5
done

address_of() {
  python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['assets'][0]['owner'])" \
    "$1/snapshot.enc" 2>/dev/null || echo "unknown"
}

SELLER="$(address_of "$PEER_B")"
BUYER="$(address_of "$PEER_A")"
cat <<EOF

  peer-a (BUY)   $BUYER
  peer-b (SELL)  $SELLER

  Who needs devnet SOL depends on which side moves the asset:

    SOL trade    the seller escrows and releases, so fund peer-b only:
                     solana airdrop 1 $SELLER --url devnet

    Boost NFT    the buyer pays for the NFT, and the seller pays to mint
                 and list it, so fund both:
                     solana airdrop 1 $SELLER --url devnet
                     solana airdrop 1 $BUYER --url devnet

  The CLI faucet is rate limited; https://faucet.solana.com takes the same
  addresses when it refuses.

  These wallets survive --reset. Only --wipe regenerates them, which strands
  whatever you funded.

  Then watch both windows: the orders mirror across the mesh, match at
  95.00 USDC / SOL, and peer-b settles the escrow to peer-a's wallet.
  Stop both with:  pkill -f cabalmesh

EOF
