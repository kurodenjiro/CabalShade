# CabalMesh

> **The Zero-Identity Autonomous Layer for Mesh-to-Solana Private Intents**

A decentralized, privacy-first infrastructure enabling autonomous AI Agents to negotiate and execute transactions over a physical Mesh Network, settling on Solana (devnet by default, via MagicBlock Ephemeral Rollups).

## 🎯 Philosophy

In this network, you are a **Nobody**. Every trace—from your physical location (IP) and negotiation tactics to your on-chain financial footprint—is erased, leaving only a cryptographically verified result.

## 🏗️ Architecture

### The "Nobody" Stack (Privacy-in-Depth)

1. **The Cloak Layer** (Mesh Networking)
   - libp2p for offline peer-to-peer communication (QUIC + TCP, Noise/Yamux)
   - mDNS discovery without internet dependency
   - Multi-hop metadata stripping
   - Relay + DCUtR hole-punching for off-LAN discovery

2. **The Invisible Brain** (Confidential Computation)
   - Ollama AI Agents with "Shark Mode" aggressive negotiation
   - Noir ZK-Circuits for privacy-preserving verification (desktop-only)
   - Confidential Compute (FHE/MPC) integration ready

3. **The Settlement Layer** (Solana)
   - An on-chain `cabal_escrow` Anchor program locks an atomic SOL ↔ Circle USDC devnet trade
   - MagicBlock Ephemeral Rollups for instant settlement
   - Instant Session keys for sub-second mesh-side agent authority delegation

### Active deployment

The current default deployment is Solana devnet. The app has this program ID
compiled in as the default escrow contract, so mobile builds do not depend on a `.env` file:

| Program | Network | Address | Explorer |
|---|---|---|---|
| `cabal_escrow` | Solana devnet | `7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq` | [View on Solana Explorer](https://explorer.solana.com/address/7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq?cluster=devnet) |
| `cabal_boost` | Solana devnet | `DVJ6GqkLAGwxceuMLJoKBKrfCposypoMCpBEHFea9GNa` | [View on Solana Explorer](https://explorer.solana.com/address/DVJ6GqkLAGwxceuMLJoKBKrfCposypoMCpBEHFea9GNa?cluster=devnet) |

The Anchor deployment transaction is recorded in
[`docs/mvp-p1-p5-checklist.md`](docs/mvp-p1-p5-checklist.md).
The latest atomic TradeEscrow upgrade transaction is
[`4FTKHJmyNPsier1Q6znx5FeayN4WDL37TwAfgQjaWCPrEy8LMnzf7DfXFm5Largw83Nyxjimr3aEmAnP9Y5neV6a`](https://explorer.solana.com/tx/4FTKHJmyNPsier1Q6znx5FeayN4WDL37TwAfgQjaWCPrEy8LMnzf7DfXFm5Largw83Nyxjimr3aEmAnP9Y5neV6a?cluster=devnet).
The isolated boost program deploy transaction is
`2mN3R2gZUAbhfyZ6Viu3bkdvHzemuLEzKRUpSnfjGb4ofwzcw8qzAivrfHPxwrk2cghHPLj6Xhu91jqD8zGRCein`.
For the MVP demo wallet, a devnet boost mint is preloaded:
`47kzUSjnFYi99zDL9DEFdr1DjGU9DSiRP8VdqTNQ8kTG` (`+2.50%`, 24-hour expiry).
Its registration transaction is
`4AAM1eaRCYRbEWg8BGg9fe1tJtDQWF9g84Jrj7vx6REtuZNqfNnXJ5FPhJFB3U9cobpgw3JSdDSb1ouuKDrcxjy1`.

### SPL boost NFTs

Relay rewards are intended to be transferable SPL Token NFTs (decimals `0`,
one item per mint). Only boost items are tradeable through mesh. Consuming an
item must burn the NFT, and its recorded expiry ends the relay/AI boost even if
the wallet remains online. Mesh carries listing and buy intents; Solana remains
the source of truth for payment, ownership, burn, and expiry.

The mobile Vault includes the `USE / BURN` and `SELL VIA MESH` workflow. The
dedicated `cabal_boost` program is now deployed independently from escrow; the
Rust bridge submits `use_boost` and `list_boost` transactions from the primary
wallet. Inventory indexing and buyer-side `buy_boost` settlement are next.

## 🚀 Quick Start

### Prerequisites

- **Rust** 1.85+ (`rustup update stable`)
- **Node.js** 18+
- **Ollama** (for AI agent) - [Install](https://ollama.ai)
- **Nargo** (for Noir circuits, optional, desktop) - [Install](https://noir-lang.org)

### Installation

```bash
cd CabalShade
npm install
```

### Run Development Server

```bash
npm run tauri dev
```

This will:
1. Start the Vite dev server (frontend)
2. Initialize the Rust Tauri backend
3. Launch the mesh network with mDNS discovery
4. Open the Nexus UI

### Mobile (iOS / Android)

```bash
npm run tauri -- ios dev            # named simulator
npm run tauri -- android dev        # emulator or attached device
```

The mobile build uses the design-system UI (`src/mobile-entry/`), the desktop build uses the frozen RPG UI (`src/`). ZK proving and the local Ollama agent are desktop-only.

## 💻 Usage

### The Nexus Interface

The main UI displays:

- **Central Radar**: Pulsing violet circle representing your mesh presence
- **Peer Dots**: Violet dots appear as nearby nodes are discovered
- **Status Bar**: Shows Internet, Mesh Nodes count, and Privacy level
- **Intent Composer**: Command bar at bottom for entering privacy intents
- **Thought Stream**: Live log of operations on the right side

### Example Intent

```
Buy 10 SOL under 95 USDC using Shark Mode
```

The system will:
1. Generate a Noir ZK-proof of your balance
2. Negotiate via Ollama AI (localhost:11434)
3. Broadcast encrypted intent to mesh
4. Match counterparty wallets and settle through the on-chain escrow program when online

### Going Offline

1. **Disconnect Wi-Fi** - The Internet LED turns red
2. **Post Intent** - Data flows through mesh (Mesh LED stays green)
3. **Reconnect** - Settlement executes on Solana

## 🔧 Project Structure

```
CabalShade/
├── src/                          # React frontend (frozen desktop RPG UI)
│   ├── App.tsx                   # Nexus UI
│   ├── components/               # UI components
│   ├── screens/                  # Mobile-style screen shells
│   ├── ds/                       # Design system (components, tokens)
│   ├── mobile-entry/             # Mobile UI entry point (separate Vite root)
│   ├── solana-settlement.ts      # Read-only Solana helper (@solana/web3.js)
│   └── types/bindings.ts         # Generated ts-rs bindings (do not edit)
├── src-tauri/                    # Rust backend (Tauri workspace)
│   ├── src/                      # App crate
│   │   ├── commands.rs           # Reshaped 28-command surface
│   │   ├── legacy/               # Frozen 50-command desktop adapter
│   │   ├── solana_bridge.rs      # Solana identity/RPC/escrow bridge
│   │   ├── mesh.rs               # libp2p mesh networking
│   │   └── ...                   # agent, zk_handler, matcher, etc.
│   ├── crates/                   # Workspace crates
│   │   ├── cabal-core/           # Domain model (no I/O)
│   │   ├── cabal-store/          # Atomic JSON persistence
│   │   └── cabal-vault/          # Encrypted key storage
│   └── tests/                    # IPC contract + lifecycle tests
├── anchor-escrow/                # Solana Anchor escrow program
│   └── programs/cabal-escrow/    # Escrow + MagicBlock Ephemeral Rollup
├── noir-circuit/                 # Noir ZK circuits
│   └── src/main.nr               # Bid verification circuit
└── docs/                         # Mobile architecture & research docs
```

## 🎨 Key Features

### 1. Offline Intent Execution
Post tasks while completely offline. Local mesh agents relay, negotiate, and sign deals, only hitting Solana when an internet gateway is reached.

### 2. Verifiable Aggression
Noir proofs ensure your AI agent followed your "Aggressive" strategy without cheating or leaking your price ceiling.

### 3. Sybil-Resistant ZK-Reputation
Nodes prove honesty via zero-knowledge without revealing interaction history.

### 4. Atomic SOL ↔ USDC escrow
`cabal_escrow` includes a two-sided `TradeEscrow` PDA. The seller opens a trade and locks SOL; the buyer locks Circle USDC devnet (mint `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`, six decimals). `release_trade` transfers both legs in one Solana transaction, so a failure rolls back both transfers. `refund_trade` returns funded legs after expiry.

### Solana asset settlement

The mesh envelope carries the counterparty's public Solana receiving wallet;
private keys never leave the encrypted local vault.

## 🧪 Testing

### Rust Backend

```bash
cd src-tauri
cargo check            # Verify compilation
cargo test --features ts-rs --workspace   # Run tests (bindings included)
npm run bindings:check # Verify src/types/bindings.ts is up to date
```

### Frontend

```bash
npm run build    # Production build
npm run preview  # Preview build
```

### Anchor Escrow Program

```bash
cd anchor-escrow
cargo test       # Rust program tests
```

### Multi-Node Mesh Test

1. Run two instances on different network interfaces
2. Watch mDNS peer discovery in console
3. Send intent from one node
4. Observe Gossipsub propagation

## 🔐 Privacy Layers

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Physical** | libp2p + mDNS | Hide IP/location |
| **Negotiation** | Ollama + FHE | Protect strategy |
| **Verification** | Noir ZK | Prove without revealing |
| **Settlement** | Atomic SOL ↔ Circle USDC escrow (Solana) | Trustless two-sided settlement |

## 📦 Dependencies

### Rust
- `libp2p` - P2P networking (QUIC, TCP, gossipsub, mDNS, relay, DCUtR)
- `tokio` - Async runtime
- `reqwest` - HTTP client (rustls)
- `serde` - Serialization
- `solana-client` / `solana-sdk` - Solana RPC, signing, transactions
- `tracing` - Structured logging (logcat on Android, OSLog on iOS)

### TypeScript
- `@tauri-apps/api` - Tauri IPC
- `react` - UI framework
- `@solana/web3.js` - Read-only Solana RPC helper

### Contracts
- `anchor-lang` + `ephemeral-rollups-sdk` - Solana escrow program

## 🎯 Use Cases

1. **Hyper-Local Confidential Trade**: P2P marketplaces in disaster zones, festivals, or censored regions
2. **Institutional Execution**: Hide market entry/exit from public order books
3. **Private AI Labor**: Outsource tasks without revealing identities

## 🗺️ Roadmap / Deferred Ideas

### Buyer/seller LLM-to-LLM price negotiation
The MVP runs a single guarded local-LLM proposal (`qwen2.5:0.5b` through Ollama) after two mesh intents match. A full back-and-forth negotiation protocol would additionally need:
- A negotiation protocol over the mesh (offer → counter-offer → accept/reject message types, similar in spirit to the existing `relay_tx`/`content_request` intent types in `mesh.rs`).
- Hard guardrails enforced in Rust (not trusted to the model): the buyer's agent must never bid above the user's price ceiling, the seller's agent must never accept below their floor.
- A round limit, so two agents can't loop forever.

### Atomic worker wiring

The deployed contract is ready for `open_trade → lock_trade_usdc → release_trade`.
The mesh worker currently exchanges the counterparty wallet and holds a matched
order at `WAITING FOR BOTH SOL + USDC LOCKS` while the final shared `trade_id`
coordinator is being wired. It does **not** fall back to the old one-sided SOL
escrow for a matched SOL/USDC order.

**Guardrail:** the local model is never trusted to authorize a transfer; Rust validates every proposal against the two declared price conditions before the settlement worker can continue.

## 🤝 Contributing

Contributions welcome!

1. Fork the repo
2. Create your feature branch
3. Commit changes
4. Push and open a PR

## 📄 License

MIT License - see LICENSE file

## 🙏 Acknowledgments

- **Solana** - Settlement layer
- **MagicBlock** - Ephemeral Rollups
- **Noir** - Zero-knowledge circuits
- **libp2p** - P2P networking
- **Tauri** - Cross-platform desktop framework
- **Ollama** - Local AI models
