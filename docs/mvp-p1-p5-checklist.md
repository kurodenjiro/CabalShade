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

## P2 — settlement

- [x] Escrow create/release/refund are routed through the bridge.
- [x] Offline RPC signs and queues instead of claiming success.
- [x] Settlement progress is streamed through a Tauri channel.
- [x] Intent proof is the returned chain transaction signature.
- [x] Solana devnet escrow program deployed and verified on-chain (`7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq`, deploy tx `2zCuSsBjtcZJ4er16FmuW3ZdLqjur8EL2oQZmc5Bq6Wog1ePG3gxmz75MzGGcf9VoYMgMk2aX2pXZ9HkGvhT8kVq`).
- [ ] Device smoke test with a funded devnet wallet and explorer link.

## P3 — deterministic matching

- [x] Intent form options come from Rust.
- [x] Preview validates the same draft accepted by broadcast.
- [x] Matcher checks asset, amount, and price constraints deterministically.
- [ ] Multi-round agent negotiation remains disabled for MVP.

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
- [ ] Desktop keyboard/focus pass for create, cancel, settle, and offline states.
- [ ] Desktop devnet smoke test with a funded wallet.

## Release command set

```bash
./node_modules/.bin/tsc -p tsconfig.mobile.json --noEmit
CARGO_TARGET_DIR=/tmp/cabalshade-target cargo check --manifest-path src-tauri/Cargo.toml
CARGO_TARGET_DIR=/tmp/cabalshade-target cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

The remaining unchecked items require a desktop Tauri runtime and a funded
devnet wallet; they are not safely verifiable in a headless build environment.
