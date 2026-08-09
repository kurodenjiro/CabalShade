# Intent matching, deals, and relay — what changed and how to test it

Everything below is desktop-only (`chỉ chạy trên máy tính`) and Solana **devnet**.

## 1. What the system now does

### Matching (`src-tauri/crates/cabal-core/src/intent.rs`)

`IntentDraft::match_with` is the whole agreement, and it is deterministic and
symmetric — both devices run it over the same two orders and get the same
answer, so a match needs no negotiation round trip:

- opposite actions (a buy fills against a sell);
- same asset;
- same amount (the escrow settles one whole leg; there is no partial-fill
  ledger);
- overlapping price bands, priced at **the midpoint of the overlap**.

`UNDER`/`ABOVE` are strict, so the bands step one cent inward: `UNDER 96.00`
against `ABOVE 94.00` clears at **95.00**, and `UNDER 95.00` against
`ABOVE 95.00` does **not** match at all.

Where matching happens: on broadcast, when a peer's order arrives, and at
bootstrap for orders restored from disk (two sides composed either side of a
restart would otherwise sit open forever).

### Roles (`src-tauri/src/deal.rs`)

Because both sides match identically, the roles are fixed by the orders
themselves: **the sell side moves the asset**.

- **Seller** — runs the local-LLM negotiation, escrows to the buyer's wallet,
  releases, then announces `trade_settled` with the real signature.
- **Buyer** — parks in `WAITING`, and settles only after it has **verified the
  announced signature on-chain** and confirmed the announcement names the peer
  and the order it actually matched with.

Nothing fabricates a transition: RPC unreachable → signed offline, queued for
relay, `WAITING`; chain rejects → both sides `FAILED`.

Cancelling one side releases the other **back to the open book** so it can
match again.

### Local LLM

`deal.rs::negotiate` → `matcher.rs::negotiate_trade` → real Ollama
(`qwen2.5:0.5b` at `127.0.0.1:11434`, 20s timeout), run **only on the paying
side** — two devices running a non-deterministic model would produce two
different numbers with no way to reconcile them.

Its authority is bounded twice: any price it proposes must satisfy *both*
orders' conditions or it is discarded, and a model error or timeout falls back
to the deterministic midpoint rather than deadlocking the deal. In practice
the 0.5b model returns out-of-range prices (it reads cents as dollars), so the
midpoint is what is used — the model's real contribution is accept/reject.

### Relay (`src-tauri/src/relay.rs`) — new

Both halves now exist in Rust, so relaying no longer depends on which screen is
mounted (the old handler lived in the deleted `src/App.tsx`, which is why
queued transactions were being broadcast to peers that did nothing with them):

- **receive** a peer's `relay_tx` → if this node has RPC, submit it → report
  back `relay_confirmed`;
- **hear** a report about our own queued transaction → mark it confirmed, which
  is what lets `resume_pending_settlements` fire the release leg;
- **drain** our own queue every 10s once the RPC is reachable again, rather than
  waiting for a volunteer that may never come.

A relayer pays nothing: the transaction arrives fully signed by its originator,
who is also its fee payer.

### Boost NFT trades take the other route

A Boost trade is not an escrow. The seller lists the NFT on-chain from the
marketplace screen and broadcasts a `SELL BOOST NFT` intent carrying the mint;
the buyer composes `BUY BOOST NFT` with a lamport ceiling. Once matched, the
**buyer** pays — `buy_boost` spends the buyer's SOL, so only the buyer can sign
it. The seller therefore publishes a `boost_purchase_request` naming the mint,
and the buyer acts on it.

Three things were wrong with that path and are now fixed:

- **An online buyer never completed the purchase.** The buyer always signed
  offline and queued for relay, so the trade only closed if some *other* peer
  volunteered to submit it — and if this device's own drain loop got there
  first, the peer's attempt came back `failed` and nothing settled at all. The
  buyer now submits directly whenever it can reach the chain, and a queued
  purchase that this device later drains settles the deal too. Relay is the
  fallback for a genuinely offline buyer, not the only route.
- **The buyer could overpay.** `buy_boost` pays whatever the listing account
  says and takes no ceiling; the negotiated price came from the intent bands,
  and the marketplace lists at a fixed 100000 lamports regardless. A buyer whose
  ceiling was below that still matched and would have paid the listing price.
  The buyer now reads the listing price on-chain and refuses anything its own
  condition does not accept.
- **A live trade could be recorded as cancelled.** Both sides cancelled after
  30 seconds of silence, while a signed purchase might still be in flight — so
  the ledger could say "called off" for a trade that then completed and moved
  the NFT. Silence is no longer treated as proof: the order stays open and the
  chase keeps asking.

### Repeated announcements actually leave the node

`mesh.rs` derives a gossipsub message id from a **hash of the payload**, so a
byte-identical second publish was discarded as a duplicate. That is right for
gossip forwarding and wrong for every deliberate repeat this app makes:
re-offering an open order to a peer that just appeared, re-announcing a
settlement, asking again whether a trade settled. All of them were silently
suppressed — including, in testing, an order that a peer therefore never saw at
all, so the two never matched.

`broadcast_intent` now stamps a per-publish nonce into the envelope. Receivers
ignore the field; gossipsub's own forwarding still deduplicates normally.

### Recovering a missed settlement

A gossip topic has no store-and-forward, so a settlement announced while the
counterparty was away reached nobody. Two recoveries:

- **push** — on `PeerDiscovered`, re-announce every trade this device has
  settled (rebuilt from the ledger, so a replay says exactly what the original
  said; receivers verify on-chain and ignore duplicates);
- **pull** — every 30s, a waiting buyer publishes a `settlement_query` to its
  counterparty and reads the counterparty's escrow account on-chain. A live
  escrow naming this wallet is reported as **locked, not paid** — the order is
  never marked settled off the back of it.

**Known limit:** if the seller never returns *and* the escrow has already been
released and closed, the buyer cannot attribute the payment on its own. It has
the funds but keeps showing `WAITING`. Closing that needs transaction
introspection (`get_transaction`), which needs a dependency this build does not
carry.

### UI

- List row: `MATCHED WITH <wallet> · 95.00 USDC / SOL`, a `PEER <wallet>` badge
  on a mirrored order, and `MATCHED` instead of `NEGOTIATING`.
- Detail: a **DEAL** panel with the counterparty's full wallet, agreed price,
  `YOU SEND` / `YOU RECEIVE`, the settlement signature, and a **VIEW ON SOLANA
  EXPLORER** button (real signatures now — the bridge used to report
  `escrow-1`).
- The manual **SETTLE INTENT** button appears only for an order with no
  counterparty; a matched pair settles through its counterparty's wallet, and a
  peer's mirrored order offers no actions at all.

## 2. How to test

### Two peers on one Mac

```bash
scripts/demo-two-peers.sh --reset
```

Each peer gets its own data directory (`~/.cabalmesh-demo/peer-{a,b}`) via
`CABALMESH_DATA_DIR`, so each mints its **own wallet** — they are genuinely two
nodes, not one app trading with itself. Peer A is seeded `BUY 0.1 SOL under
96.00`, peer B `SELL 0.1 SOL above 94.00`.

The script prints both addresses. **The sell side must be funded** or
settlement fails honestly with `Attempt to debit an account but found no record
of a prior credit`:

```bash
solana airdrop 1 <peer-b-address> --url devnet
```

Then `scripts/demo-two-peers.sh --reset` again and watch both windows.

What you should see, and where:

| Where | Expected |
| --- | --- |
| Both windows, INTENTS | two rows — your own order and `PEER <wallet>` |
| Both | `MATCHED WITH … · 95.00 USDC / SOL` |
| Peer B (sell) | `ROUTING` → `SETTLED`, DEAL panel shows the signature |
| Peer A (buy) | `WAITING` → `SETTLED` once it verifies B's signature |
| Peer A detail | `VIEW ON SOLANA EXPLORER` opens the real devnet transaction |

Ledgers and logs to inspect directly:

```bash
python3 -m json.tool ~/.cabalmesh-demo/peer-a/intents.json
tail -f ~/.cabalmesh-demo/peer-b/app.log
```

Stop everything with `pkill -f cabalmesh`.

### Testing a Boost NFT purchase

Both wallets need devnet SOL (claiming and buying both cost fees).

1. On peer B: **New → SELL → SELL NFT** opens the marketplace. `CLAIM DEMO
   BOOST` mints one, then list it — that lists on-chain at 100000 lamports
   *and* broadcasts a `SELL BOOST NFT` intent carrying the mint.
2. On peer A: **New → BUY → BUY BOOST NFT**, ceiling `0.0001` SOL or higher.
3. They match, the seller sends a purchase request, and peer A buys.

Expect in peer A's log: `LISTING VERIFIED AT 100000 LAMPORTS` then
`PURCHASED ON DEVNET. TX …`, and both sides `SETTLED`.

Set the ceiling *below* 0.0001 SOL (e.g. `0.00005`) to check the guard: the
orders still match on their bands, but peer A refuses with
`LISTING WANTS 100000 LAMPORTS, ABOVE YOUR LIMIT` rather than overpaying.

### Testing the offline / relay path

Give one peer an unroutable RPC so its call times out (a *refused* connection
fails fast and does not trigger offline signing — it must hang):

```bash
CABALMESH_DATA_DIR=$HOME/.cabalmesh-demo/peer-b SOLANA_RPC_URL=http://10.255.255.1:9999 /tmp/cabalshade-target/debug/cabalmesh
```

Expect in `peer-b/app.log`: `signing offline for mesh relay` then
`queued for mesh relay`; in `peer-a/app.log`: `relayed a peer's transaction`
(or `relay submission rejected`, reported back honestly); then in `peer-b`:
`a peer relayed our queued transaction`.

Note the peer needs a `chain_cache.json` from an earlier online run — a device
that has never been online has no blockhash to sign against.

### Testing the missed-announcement recovery

1. Run both peers with a funded seller until they match.
2. Kill peer A (the buyer) *before* peer B settles.
3. Let peer B settle alone — its announcement reaches nobody.
4. Restart peer A. On peer discovery, B re-announces; A verifies the signature
   on-chain and moves from `WAITING` to `SETTLED`. Its own 30s
   `settlement_query` reaches the same result if discovery fires first.

### Automated checks

```bash
CARGO_TARGET_DIR=/tmp/cabalshade-target cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

105 lib tests + 53 domain tests, covering the matching rules (bands, midpoint,
symmetry, strict boundaries), pairing and unpairing, role assignment, the
announcement rebuild, and the mesh wire shapes.

```bash
./node_modules/.bin/tsc -p tsconfig.mobile.json --noEmit
npm run bindings:check
```

## 3. Verified on 2026-08-09

- **A complete settlement, end to end, on devnet.** Two peers matched at 9500
  cents; the seller created and released the escrow; the buyer verified the
  announced signature against the chain and settled on the evidence.

  ```
  peer-b  Create escrow    4kT8VWYHfe7foC5gfEbpN8HCFsZhn7DZqiQsup77T26aiJSjZ5Uj7pmjXUq9FjeFpXgWQhL3CoWMhUBqkbRK5ER2
  peer-b  Release escrow   pSXLPDPeQtBtPYU8c36LyMRQ5BfgzWCyGYGDntx9G4ydzvhQ3i2hXEG6fzahUzUQD95Ewtso3JcQsb7Qn3HhHoX
  peer-a  received trade_settled → verified on-chain → SETTLED
  ```

  The release is `finalized` with no error, and the balances moved the way the
  trade says they should: the buyer `1.0 → 1.1 SOL`, the seller
  `1.0 → 0.89999 SOL` (0.1 plus fees). Both ledgers record the same release
  signature as the proof.
- **Matching across two peers** over real mDNS + gossipsub: each mirrored the
  other's order and both computed 9500 cents independently.
- **Relay round trip**, with one peer's RPC black-holed:

  ```
  peer-b  RPC unreachable — signing offline for mesh relay   (Create escrow)
  peer-b  intent broadcast  intent_type=relay_tx
  peer-a  Received relay_tx request: tx-1786273426654-3b6a1a9f
  peer-a  relay submission rejected  error=… Blockhash not found
  peer-a  intent broadcast  intent_type=relay_confirmed
  peer-b  a peer relayed our queued transaction  status=failed
  ```

  The submission was rejected because the offline peer signed against a stale
  cached blockhash — which is the honest outcome for that setup, and it was
  reported back and recorded rather than lost. Before this work, every step
  after the first broadcast was missing.
- **The buyer's `settlement_query`** fired on schedule and repeated (the nonce
  fix); the seller received it and correctly stayed **silent**, having nothing
  settled to report.
- **Restart reconciliation** paired two orders restored from disk.
- **Local LLM** returns `{"decision":"accept","price_usdc":"9600"}` in ~1.3s;
  that price is outside the buyer's ceiling, so it is discarded and the
  deterministic 95.00 stands — the guardrail doing its job.

Not yet verified: the **settled-state UI** — the DEAL panel and the explorer
button — was not opened during this run, only the ledgers and the chain were
checked.

The Boost fixes above are covered by unit tests (the mint travels with the
seller's side, a listing above the ceiling is refused, a pair still matches when
the seller names no price) but the purchase itself is **not yet verified
on-chain** — claiming a demo boost and buying it both need a funded wallet.
