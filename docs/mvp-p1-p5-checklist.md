# Cabal Mesh MVP — P1–P5 completion checklist

This is the release gate for the first usable vertical slice. The mobile UI must
remain usable without a network connection; the chain is Solana devnet only.

## P1 — persistence and offline

- [x] Intent ledger restores from `intents.json` on bootstrap.
- [x] Broadcast state is persisted before the IPC command returns.
- [x] Relay transactions use atomic JSON persistence.
- [x] Offline-signed transactions survive process termination.
- [x] Retry attempts are bounded and persisted.
- [x] Resume worker drains pending settlements after reconnect.
- [x] UI distinguishes `WAITING` from `SETTLED`.
- [x] Received `relay_tx` requests are submitted and reported back in Rust, so
      relaying no longer depends on a screen being mounted (`src/relay.rs`).
- [x] A peer's `relay_confirmed` report marks the local queue entry, which is
      what lets the resume worker fire the release leg.
- [x] Repeated mesh announcements are no longer suppressed by gossipsub's
      content-hash message id.

## P2 — settlement

- [x] Escrow create/release/refund are routed through the bridge.
- [x] Offline RPC signs and queues instead of claiming success.
- [x] Settlement progress is streamed through a Tauri channel.
- [x] Intent proof is the returned chain transaction signature.
- [x] Solana devnet escrow program deployed and verified on-chain (`7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq`, deploy tx `2zCuSsBjtcZJ4er16FmuW3ZdLqjur8EL2oQZmc5Bq6Wog1ePG3gxmz75MzGGcf9VoYMgMk2aX2pXZ9HkGvhT8kVq`).
- [x] Solana devnet escrow program upgraded after UI integration (upgrade tx `BkNHQWxuZV29CqxZ7ptJdaVEvL3SaZXCgpha4DYrJ6baJ77eSvcEqesPBkEPNNijHCuyKj3CTpEFurZRBzW4DNg`).
- [x] Device smoke test with a funded devnet wallet and explorer link. Two
      funded peers settled a real 0.1 SOL trade on 2026-08-09: create
      `4kT8VWYHfe7foC5gfEbpN8HCFsZhn7DZqiQsup77T26aiJSjZ5Uj7pmjXUq9FjeFpXgWQhL3CoWMhUBqkbRK5ER2`,
      release
      `pSXLPDPeQtBtPYU8c36LyMRQ5BfgzWCyGYGDntx9G4ydzvhQ3i2hXEG6fzahUzUQD95Ewtso3JcQsb7Qn3HhHoX`
      (finalized, no error). Balances moved 1.0 → 1.1 SOL on the buyer and
      1.0 → 0.89999 SOL on the seller.

## P3 — deterministic matching

- [x] Intent form options come from Rust.
- [x] Preview validates the same draft accepted by broadcast.
- [x] Matcher checks asset, amount, and price constraints deterministically.
- [x] Two opposite orders pair into one deal, priced at the midpoint of the
      overlapping band (`IntentDraft::match_with`).
- [x] The sell side settles; the buy side waits and verifies the announced
      signature on-chain, so a symmetric match cannot pay twice.
- [x] A cancelled order releases its counterparty back to the open book.
- [x] Orders restored from disk are re-paired at bootstrap.
- [ ] Multi-round agent negotiation remains disabled for MVP.

See `docs/intent-matching-and-relay.md` for the full description and the
manual test procedure.

### Two-peer demo

`scripts/demo-two-peers.sh` runs both sides on one Mac — separate data
directories via `CABALMESH_DATA_DIR`, so each peer mints its own wallet — with
one seeded order each (BUY under 96.00 against SELL above 94.00, clearing at
95.00). Verified 2026-08-09 with both wallets funded: the peers mirrored each other's
order over mDNS, matched at 9500 cents, and the sell side settled a real 0.1
SOL escrow to the buy side, which verified the signature on-chain before
recording it. Fund the wallets first — the script prints which one each
scenario needs.

## P4 — wallet and privacy

- [x] Mnemonic is not returned during ordinary wallet bootstrap.
- [x] Export requires an explicit user action.
- [x] Private wallet snapshot is encrypted at rest.
- [x] Offline mode prevents mesh publication.
- [x] UI labels the network and testnet state.
- [ ] Device-level clipboard auto-clear verification.

## P5 — QA gate

- [x] Mobile TypeScript compile.
- [x] Rust workspace check.
- [x] Intent happy-path tests.
- [x] Offline queue durability tests.
- [x] Subscription lifecycle tests.
- [x] Desktop Tauri window renders the mobile layout at 390×844.
- [x] Desktop keyboard/focus pass for create, cancel, settle, and offline states (dialog focus traps, Escape cancel, and dismiss buttons verified).
- [x] Desktop devnet smoke test with a funded wallet — two desktop instances,
      funded devnet wallets, one real settled trade (signatures in P2 above).
      Verified through the ledgers and the chain; the settled-state UI itself
      was not observed in that run.

## Release command set

```bash
./node_modules/.bin/tsc -p tsconfig.mobile.json --noEmit
CARGO_TARGET_DIR=/tmp/cabalshade-target cargo check --manifest-path src-tauri/Cargo.toml
CARGO_TARGET_DIR=/tmp/cabalshade-target cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

The remaining unchecked items require a desktop Tauri runtime and a funded
devnet wallet; they are not safely verifiable in a headless build environment.

## Known integration gap

The browser preview does not provide the real Tauri IPC bridge, so browser
smoke tests validate rendering and navigation only. The Solana program and UI
actions have been tested separately. A complete proof of the production path
(`UI click → Tauri IPC → MagicBlock ER release → explorer transaction`) is not
yet available because ER release is still failing on account/fee routing. Do
not describe the MVP as having a verified end-to-end ER transaction until a
 desktop Tauri run succeeds with a funded devnet wallet and an explorer link.

The integration test now funds its fresh depositor from the existing deployer
wallet, avoiding the public faucet limit. The ER path is modeled as two phases:
`releaseEr` changes delegated state, then `settle` pays the wallet after
commit/undelegate on Solana. The current MagicBlock devnet validator still
returns `InstructionFallbackNotFound` for the newly upgraded instruction,
which means its program cache must refresh before this path can be verified.

Desktop UI QA completed on the local desktop surface: opening Create Listing
places focus in the description field, Escape closes the modal, and modal
focus trapping is implemented for escrow release/refund. Offline queue
dismiss actions are real keyboard-focusable buttons. The production build was
verified using a temporary output directory because the existing `dist-desktop`
directory is owned by another user.
