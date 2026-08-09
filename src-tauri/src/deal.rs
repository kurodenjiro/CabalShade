//! Turning a matched pair into a settled trade.
//!
//! # The two sides
//!
//! Matching happens independently on both devices: each ledger holds the local
//! order and a mirror of the peer's, and [`cabal_core::IntentDraft::match_with`]
//! is deterministic, so both reach the same pair and the same price without a
//! negotiation round trip.
//!
//! That symmetry is also the hazard — if both sides acted, the trade would pay
//! twice. So the roles are fixed by the orders themselves: **the sell side
//! moves the asset**. The seller escrows and releases to the buyer's wallet;
//! the buyer waits, then verifies the announced signature against the chain
//! before writing the trade into its own ledger. A claim from a peer is never
//! enough on its own.
//!
//! Nothing here fabricates a transition. When the RPC is unreachable the deal
//! parks in `WAITING` with the transaction queued for mesh relay, and when the
//! chain rejects the settlement both sides go to `FAILED`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::blockchain_bridge::BlockchainBridge;
use crate::matcher::MatchAgent;
use crate::mesh_handle::MeshHandle;
use cabal_core::{Action, IntentId, IntentStatus, IntentStore, UsdPrice};

/// The announcement a payer publishes once its escrow release has confirmed.
///
/// Intent ids are always the ones each side's **own** ledger uses, because
/// that is the only name a receiver can look itself up by — the mirror it
/// holds of the sender's order has a different local id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeSettled {
    /// The buy order, as the buyer's own ledger names it.
    pub buyer_intent: String,
    /// The sell order, as the seller's own ledger names it.
    pub seller_intent: String,
    /// The real release signature on Solana devnet.
    pub signature: String,
    /// The agreed price, in cents. Display only — the escrow settles the asset
    /// leg, and the counter-leg is out of scope for this MVP.
    pub price_cents: Option<u64>,
    /// The settled amount, e.g. `0.1`.
    pub amount: String,
    /// The payer's own public address. The receiver checks it against the
    /// wallet it matched with, so an announcement can only come from the peer
    /// the order was actually paired with.
    pub wallet: String,
}

/// A buyer asking the peer it matched with whether the trade has settled.
///
/// The announcement is published once, over a gossip topic with no
/// store-and-forward. A counterparty that was offline at that moment would
/// otherwise wait forever — with the asset already in its wallet. This is the
/// pull side of the same fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementQuery {
    /// The order being asked about, as the *seller's* own ledger names it.
    pub seller_intent: String,
    /// The asker's own order, so the reply can be addressed.
    pub buyer_intent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoostPurchaseRequest {
    pub buyer_intent: String,
    pub seller_intent: String,
    pub mint: String,
    pub seller_wallet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoostTradeSettled {
    pub buyer_intent: String,
    pub seller_intent: String,
    pub signature: String,
    /// What the chain actually charged, in lamports. The seller has no other
    /// way to learn this: only the buyer reads the listing account before
    /// paying, and the seller's own record still holds the band midpoint the
    /// two orders matched at, which is a number nobody was ever going to pay.
    /// `None` only if the verified price was somehow never recorded — never
    /// filled in with a guess.
    pub price_lamports: Option<u64>,
}

/// Everything a deal worker needs. Cloned per deal rather than borrowed: the
/// worker outlives the mesh event that started it.
#[derive(Clone)]
pub struct DealContext {
    pub app: AppHandle,
    pub bridge: Arc<Mutex<BlockchainBridge>>,
    pub matcher: Arc<MatchAgent>,
    pub mesh: Option<MeshHandle>,
    pub intents: Arc<std::sync::Mutex<IntentStore>>,
}

impl DealContext {
    fn lock(&self) -> std::sync::MutexGuard<'_, IntentStore> {
        self.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Writes the ledger to disk. Called after every transition a restart must
    /// not lose — which is all of them, once funds are involved.
    fn persist(&self) {
        let snapshot = self.lock().snapshot();
        let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json"))
            .save(&snapshot);
    }

    fn notify(&self, ids: &[&IntentId]) {
        for id in ids {
            let _ = self
                .app
                .emit("intent-updated", serde_json::json!({ "id": id.as_str() }));
        }
    }

    /// One line of the agent transcript, addressed to both sides of the deal so
    /// either detail screen can show it.
    fn say(&self, ids: &[&IntentId], text: &str) {
        for id in ids {
            let _ = self.app.emit(
                "agent-exchange",
                serde_json::json!({ "id": id.as_str(), "text": text }),
            );
        }
    }

    async fn publish(&self, intent_type: &str, payload: String) {
        if let Some(mesh) = self.mesh.as_ref() {
            let _ = mesh
                .publish(crate::mesh::PrivacyIntent {
                    intent_type: intent_type.to_string(),
                    payload,
                    encrypted: false,
                    relay_path: vec!["origin_node".into()],
                    relay_fee: None,
                })
                .await;
        }
    }

    /// Moves both sides of a deal to the same state, persists, and notifies.
    /// Illegal transitions are dropped rather than forced: the lifecycle table
    /// is the authority on what may follow what.
    fn advance(&self, ids: &[&IntentId], to: &IntentStatus) {
        {
            let mut store = self.lock();
            for id in ids {
                let _ = store.transition(id, to.clone(), crate::commands::now_secs());
            }
        }
        self.persist();
        self.notify(ids);
    }
}

/// The two sides of a matched pair, resolved into roles.
struct Roles {
    seller: IntentId,
    buyer: IntentId,
    /// Whether the sell order was composed here. Only then does this device
    /// move funds.
    we_are_seller: bool,
    /// Where the asset goes: the buyer's wallet, or this device's own when the
    /// buy order is local.
    payee_wallet: String,
    /// This device's own receiving address. Named in the settlement
    /// announcement so the counterparty can check the payer is the peer it
    /// matched with, and not a third party naming someone else's order.
    own_wallet: String,
    /// The ids each side's own ledger uses, for the settlement announcement.
    seller_remote_id: String,
    buyer_remote_id: String,
    asset: String,
    amount: String,
    boost_mint: Option<String>,
}

impl Roles {
    fn resolve(store: &IntentStore, left: &IntentId, right: &IntentId, own_wallet: &str) -> Option<Self> {
        let left_intent = store.get(left)?;
        let right_intent = store.get(right)?;
        let (seller, buyer) = match (left_intent.draft.action, right_intent.draft.action) {
            (Action::Sell, Action::Buy) => (left_intent, right_intent),
            (Action::Buy, Action::Sell) => (right_intent, left_intent),
            // Same-side pairs are not trades; `match_with` never produces one.
            _ => return None,
        };

        // A mirrored order names itself by the id its originator uses; a local
        // one by the id in this ledger.
        let remote_id = |intent: &cabal_core::StoredIntent| {
            intent
                .origin
                .as_ref()
                .map_or_else(|| intent.id.to_string(), |o| o.intent_id.clone())
        };

        Some(Self {
            we_are_seller: seller.is_local(),
            payee_wallet: buyer
                .origin
                .as_ref()
                .map_or_else(|| own_wallet.to_string(), |o| o.wallet.clone()),
            own_wallet: own_wallet.to_string(),
            seller_remote_id: remote_id(seller),
            buyer_remote_id: remote_id(buyer),
            asset: seller.draft.asset.to_string(),
            amount: seller.draft.amount.to_plain_string(),
            boost_mint: seller.boost_mint.as_deref().map(str::to_string),
            seller: seller.id.clone(),
            buyer: buyer.id.clone(),
        })
    }
}

/// Pairs anything in the restored ledger that should already be paired.
///
/// Matching normally happens the moment an order is broadcast or arrives from
/// a peer. Orders restored from disk missed both events — two compatible sides
/// composed either side of a restart would otherwise sit open forever, looking
/// at each other.
pub fn reconcile(ctx: DealContext) {
    let open: Vec<IntentId> = ctx
        .lock()
        .all()
        .into_iter()
        .filter(|intent| intent.is_open())
        .map(|intent| intent.id.clone())
        .collect();

    let mut paired = Vec::new();
    {
        let mut store = ctx.lock();
        let now = crate::commands::now_secs();
        for id in open {
            // Re-checked inside the loop: an earlier iteration may have already
            // spoken for this order.
            if let Some((other, terms)) = store.find_counterparty(&id) {
                if store.pair(&id, &other, terms, now).is_ok() {
                    paired.push((id, other));
                }
            }
        }
    }
    if paired.is_empty() {
        return;
    }
    ctx.persist();
    for (left, right) in paired {
        ctx.notify(&[&left, &right]);
        spawn(ctx.clone(), left, right);
    }
}

/// Runs a matched pair through to settlement, in the background.
///
/// Spawned rather than awaited: chain confirmation must never block the mesh
/// event loop that discovered the match, or the next peer order goes unheard.
pub fn spawn(ctx: DealContext, left: IntentId, right: IntentId) {
    tauri::async_runtime::spawn(async move {
        let own_wallet = ctx.bridge.lock().await.get_primary_address();
        let Some(roles) = ({
            let store = ctx.lock();
            Roles::resolve(&store, &left, &right, &own_wallet)
        }) else {
            return;
        };
        let sides = [&roles.seller, &roles.buyer];

        ctx.say(&sides, "MESH AGENT: OPPOSITE ORDER MATCHED.");

        // The buyer does not move funds on the SOL route. It parks in WAITING
        // and settles only when the seller's announced signature checks out
        // on-chain. On the Boost route it does pay, but not from here — the
        // purchase is driven by the seller's `boost_purchase_request`.
        if !roles.we_are_seller {
            ctx.say(&sides, "SETTLEMENT AGENT: AWAITING COUNTERPARTY ESCROW.");
            ctx.advance(&sides, &IntentStatus::Waiting);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let still_waiting = {
                let store = ctx.lock();
                sides.iter().all(|id| {
                    store
                        .get(id)
                        .is_some_and(|intent| matches!(intent.status, IntentStatus::Waiting))
                })
            };
            if still_waiting {
                // Not cancelled. Thirty seconds of silence is not proof that
                // nothing happened: the counterparty may be mid-settlement, and
                // on the Boost route this side may already be holding a signed
                // purchase waiting on a relay. Cancelling here would put
                // "called off" in the ledger for a trade that then completes.
                // `chase_pending_settlements` keeps asking, and a verified
                // signature closes it.
                ctx.say(&sides, "SETTLEMENT AGENT: NOTHING YET. ORDER STAYS OPEN AND KEEPS ASKING.");
            }
            return;
        }

        // The paying side's decisions are traced from here on. Everything this
        // worker says otherwise goes out as a UI event, which is invisible the
        // moment you are reading a log file to find out why a deal stalled.
        tracing::info!(
            seller = %roles.seller,
            buyer = %roles.buyer,
            asset = %roles.asset,
            "deal worker: acting as the paying side"
        );

        let price = negotiate(&ctx, &roles, &sides).await;
        let Some(price) = price else {
            tracing::info!("deal worker: local agent rejected the terms; orders stay open");
            ctx.say(&sides, "AGENT-0X123..2413: TERMS REJECTED. ORDERS REMAIN OPEN.");
            ctx.advance(&sides, &IntentStatus::Waiting);
            return;
        };

        // Boost NFT intents still negotiate through the same local agent so
        // both sides get a real matched price. Its final marketplace purchase
        // needs a buyer-signed relay transaction (the seller can never sign
        // away the buyer's SOL), so keep the agreed order waiting rather than
        // sending it through the SOL escrow route.
        if roles.asset != "SOL" {
            let Some(mint) = roles.boost_mint.clone() else {
                tracing::warn!(
                    seller = %roles.seller,
                    "boost deal has no mint on the sell order; nothing to buy"
                );
                ctx.say(&sides, "BOOST MARKET ERROR: SELLER LISTING HAS NO NFT MINT. INTENT CANCELLED.");
                ctx.advance(&sides, &IntentStatus::Cancelled);
                return;
            };
            tracing::info!(%mint, buyer_intent = %roles.buyer_remote_id, "publishing boost purchase request");
            ctx.say(&sides, "BOOST MARKET: TERMS AGREED. REQUESTING BUYER-SIGNED RELAY PURCHASE.");
            ctx.publish("boost_purchase_request", serde_json::to_string(&BoostPurchaseRequest {
                buyer_intent: roles.buyer_remote_id.clone(),
                seller_intent: roles.seller_remote_id.clone(),
                mint,
                seller_wallet: roles.own_wallet.clone(),
            }).unwrap_or_default()).await;
            ctx.advance(&sides, &IntentStatus::Waiting);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let still_waiting = {
                let store = ctx.lock();
                sides.iter().all(|id| {
                    store
                        .get(id)
                        .is_some_and(|intent| matches!(intent.status, IntentStatus::Waiting))
                })
            };
            if still_waiting {
                // Deliberately *not* cancelled. By this point the buyer may
                // hold a signed purchase waiting on a relay; cancelling here
                // and having that transaction land later would leave this
                // ledger claiming a trade was called off while the NFT
                // actually changed hands. The order stays open and
                // `boost_trade_settled` closes it whenever the purchase
                // confirms.
                ctx.say(&sides, "BOOST MARKET: NO PURCHASE YET. ORDER STAYS OPEN.");
            }
            return;
        }

        ctx.say(&sides, "MESH AGENTS: ROUTE LOCKED. SUBMITTING ESCROW…");
        ctx.advance(&sides, &IntentStatus::FindingRoute);

        settle(&ctx, &roles, &sides, price).await;
    });
}

/// Asks the local model for a price inside the matched band, and records it.
///
/// Only the paying side runs this: a model is not deterministic, and two
/// devices reaching different numbers would be a disagreement neither could
/// resolve. Returns `None` when the model rejects the terms outright; a model
/// or network failure falls back to the deterministic clearing price rather
/// than deadlocking the deal.
async fn negotiate(ctx: &DealContext, roles: &Roles, sides: &[&IntentId]) -> Option<Option<UsdPrice>> {
    let (seller_draft, buyer_draft, band_price) = {
        let store = ctx.lock();
        let seller = store.get(&roles.seller)?;
        let buyer = store.get(&roles.buyer)?;
        (
            seller.draft.clone(),
            buyer.draft.clone(),
            seller.matched.as_ref().and_then(|m| m.price),
        )
    };

    ctx.say(sides, "AGENT-0X123..2413: ANALYSING TERMS…");
    let proposal = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        ctx.matcher.negotiate_trade(&seller_draft, &buyer_draft),
    )
    .await
    .ok()
    .and_then(Result::ok);

    if proposal.as_ref().is_some_and(|deal| !deal.accepted) {
        return None;
    }
    // `negotiate_trade` already discards any price outside both conditions, so
    // whatever survives is inside the matched band.
    let price = proposal.and_then(|deal| deal.price).or(band_price);

    {
        let mut store = ctx.lock();
        for id in [&roles.seller, &roles.buyer] {
            store.set_match_price(id, price);
            let _ = store.transition(
                id,
                IntentStatus::Negotiating { bids: 1, best: price },
                crate::commands::now_secs(),
            );
        }
    }
    ctx.persist();
    ctx.notify(sides);
    ctx.say(
        sides,
        &price.map_or_else(
            || "AGENT-0X123..2413: TERMS ACCEPTED AT MARKET.".to_string(),
            |p| format!("AGENT-0X123..2413: TERMS ACCEPTED AT {:.2} USDC / SOL.", p.cents() as f64 / 100.0),
        ),
    );
    Some(price)
}

/// The paying leg: escrow to the buyer's wallet, release, announce.
async fn settle(ctx: &DealContext, roles: &Roles, sides: &[&IntentId], price: Option<UsdPrice>) {
    let settled = ctx
        .bridge
        .lock()
        .await
        .settle_on_chain(&roles.payee_wallet, &roles.amount)
        .await
        .map_err(|error| error.to_string());

    let settled = match settled {
        Ok(settled) => settled,
        Err(error) => {
            // Also to the log: the UI line reaches whoever is watching the
            // window, and a demo that fails on an unfunded wallet is diagnosed
            // from the terminal.
            tracing::warn!(
                payee = %roles.payee_wallet,
                amount = %roles.amount,
                %error,
                "settlement rejected on-chain"
            );
            ctx.say(sides, &format!("ESCROW: SETTLEMENT REJECTED. {error}"));
            ctx.advance(
                sides,
                &IntentStatus::Failed { reason: cabal_core::FailureReason::SettlementRejected },
            );
            return;
        }
    };

    // Anything signed offline still needs a peer with connectivity to submit
    // it. Broadcast the real raw transaction and park the deal — a queued
    // create has not paid anyone yet.
    if !settled.queued_for_relay.is_empty() {
        if let Some(mesh) = ctx.mesh.as_ref() {
            for queued in &settled.queued_for_relay {
                let _ = mesh
                    .publish(crate::mesh::PrivacyIntent {
                        intent_type: "relay_tx".into(),
                        payload: serde_json::json!({
                            "type": "RelayTx",
                            "queue_id": queued.id,
                            "raw_tx_hex": queued.raw_tx_hex,
                            "summary": queued.summary,
                        })
                        .to_string(),
                        encrypted: false,
                        relay_path: vec!["origin_node".into()],
                        relay_fee: None,
                    })
                    .await;
            }
        }
        ctx.say(sides, "ESCROW: OFFLINE SIGNED. QUEUED FOR MESH RELAY.");
        ctx.advance(sides, &IntentStatus::Waiting);
        return;
    }

    ctx.say(sides, &format!("ESCROW CREATED. TX {}", settled.create_tx));
    ctx.say(sides, &format!("RELEASED ON SOLANA DEVNET. TX {}", settled.release_tx));

    let elapsed = {
        let store = ctx.lock();
        store
            .get(&roles.seller)
            .map(|intent| crate::commands::now_secs().saturating_sub(intent.created_at))
            .unwrap_or_default()
    };
    let status = IntentStatus::Settled {
        proof: cabal_core::ProofHash::new(settled.release_tx.clone()),
        filled_at: price.unwrap_or_else(|| UsdPrice::from_cents(0)),
        elapsed_ms: u32::try_from(elapsed.saturating_mul(1000)).unwrap_or(u32::MAX),
    };
    ctx.advance(sides, &status);

    // Tell the counterparty, with the evidence. It verifies before believing.
    if let Some(mesh) = ctx.mesh.as_ref() {
        let announcement = TradeSettled {
            buyer_intent: roles.buyer_remote_id.clone(),
            seller_intent: roles.seller_remote_id.clone(),
            signature: settled.release_tx,
            price_cents: price.map(UsdPrice::cents),
            amount: roles.amount.clone(),
            wallet: roles.own_wallet.clone(),
        };
        let _ = mesh
            .publish(crate::mesh::PrivacyIntent {
                intent_type: "trade_settled".into(),
                payload: serde_json::to_string(&announcement).unwrap_or_default(),
                encrypted: false,
                relay_path: vec!["origin_node".into()],
                relay_fee: None,
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cabal_core::{Condition, ExecutionMode, IntentDraft, PrivacyLevel, RemoteOrigin, TokenAmount};

    const OWN: &str = "6VzCXuDCVMSMHkjHufkjhbcMYQpiV3Vt3bh5AB5AQKPB";
    const PEER: &str = "GyzdBSo87y5vT4oyoBCAdeT7hSz4C2ihj89QrVGCpdRa";

    fn draft(action: Action) -> IntentDraft {
        IntentDraft {
            action,
            asset: "SOL".into(),
            condition: Condition::Any,
            amount: TokenAmount::parse("0.1", 9).unwrap(),
            mode: ExecutionMode::Shark,
            privacy: PrivacyLevel::Medium,
        }
    }

    /// A ledger holding one local order and one mirrored peer order, paired.
    fn paired(local: Action) -> (IntentStore, IntentId, IntentId) {
        let mut store = IntentStore::new();
        let now = 1_700_000_000;
        let mine = store.create(draft(local), now);
        let theirs = store.create_remote(
            draft(match local {
                Action::Buy => Action::Sell,
                Action::Sell => Action::Buy,
            }),
            RemoteOrigin { intent_id: "int-000000ff".into(), wallet: PEER.into() },
            now,
        );
        for id in [&mine, &theirs] {
            store
                .transition(id, IntentStatus::Broadcast { route_len: 1 }, now)
                .unwrap();
        }
        let (found, terms) = store.find_counterparty(&mine).unwrap();
        store.pair(&mine, &found, terms, now).unwrap();
        (store, mine, theirs)
    }

    #[test]
    fn the_sell_side_moves_the_asset() {
        let (store, mine, theirs) = paired(Action::Sell);
        let roles = Roles::resolve(&store, &mine, &theirs, OWN).unwrap();
        assert_eq!(roles.seller, mine);
        assert_eq!(roles.buyer, theirs);
        assert!(roles.we_are_seller, "the local sell order is ours to fund");
        assert_eq!(roles.payee_wallet, PEER, "the asset goes to the buyer");
    }

    #[test]
    fn a_local_buy_waits_rather_than_paying() {
        // The same pair seen from the other device. If this side also paid, the
        // trade would settle twice.
        let (store, mine, theirs) = paired(Action::Buy);
        let roles = Roles::resolve(&store, &mine, &theirs, OWN).unwrap();
        assert_eq!(roles.seller, theirs);
        assert_eq!(roles.buyer, mine);
        assert!(!roles.we_are_seller);
    }

    #[test]
    fn the_argument_order_does_not_decide_the_roles() {
        let (store, mine, theirs) = paired(Action::Sell);
        assert_eq!(
            Roles::resolve(&store, &mine, &theirs, OWN).unwrap().seller,
            Roles::resolve(&store, &theirs, &mine, OWN).unwrap().seller
        );
    }

    #[test]
    fn each_side_is_named_as_its_own_ledger_names_it() {
        // The announcement has to be readable by the peer, whose ledger knows
        // its order as `int-000000ff` — not by the id our mirror was given.
        let (store, mine, theirs) = paired(Action::Sell);
        let roles = Roles::resolve(&store, &mine, &theirs, OWN).unwrap();
        assert_eq!(roles.seller_remote_id, mine.to_string());
        assert_eq!(roles.buyer_remote_id, "int-000000ff");
    }

    #[test]
    fn the_announcement_names_the_payer_the_buyer_matched_with() {
        // The buyer's own record holds the seller's wallet, and that is what it
        // checks the announcement against — so the two must be the same field.
        let (store, mine, theirs) = paired(Action::Sell);
        let roles = Roles::resolve(&store, &mine, &theirs, OWN).unwrap();
        assert_eq!(roles.own_wallet, OWN);

        let (buyer_store, buy_side, _) = paired(Action::Buy);
        let record = buyer_store.get(&buy_side).unwrap().matched.as_ref().unwrap();
        assert_eq!(
            record.wallet, PEER,
            "the buyer verifies against the seller's wallet, not its own"
        );
    }

    #[test]
    fn two_local_orders_settle_to_this_wallet() {
        let mut store = IntentStore::new();
        let now = 1_700_000_000;
        let sell = store.create(draft(Action::Sell), now);
        let buy = store.create(draft(Action::Buy), now);
        for id in [&sell, &buy] {
            store
                .transition(id, IntentStatus::Broadcast { route_len: 1 }, now)
                .unwrap();
        }
        let (found, terms) = store.find_counterparty(&sell).unwrap();
        store.pair(&sell, &found, terms, now).unwrap();

        let roles = Roles::resolve(&store, &sell, &buy, OWN).unwrap();
        assert!(roles.we_are_seller);
        assert_eq!(roles.payee_wallet, OWN);
        assert_eq!(roles.amount, "0.1");
    }

    /// Drives a paired ledger to settled, as the paying side would.
    fn settled(store: &mut IntentStore, seller: &IntentId, buyer: &IntentId, signature: &str) {
        let now = 1_700_000_010;
        for id in [seller, buyer] {
            store.transition(id, IntentStatus::FindingRoute, now).unwrap();
            store
                .transition(
                    id,
                    IntentStatus::Settled {
                        proof: cabal_core::ProofHash::new(signature),
                        filled_at: UsdPrice::from_cents(9500),
                        elapsed_ms: 1_000,
                    },
                    now,
                )
                .unwrap();
        }
    }

    #[test]
    fn a_settled_trade_can_be_re_announced_from_the_ledger_alone() {
        let (mut store, mine, theirs) = paired(Action::Sell);
        settled(&mut store, &mine, &theirs, "5xYsignature");

        let announcement = announcement_for(&store, &mine, OWN).expect("we paid, so we announce");
        assert_eq!(announcement.signature, "5xYsignature");
        assert_eq!(announcement.seller_intent, mine.to_string());
        // Addressed by the id the *buyer's* ledger uses, not by our mirror's.
        assert_eq!(announcement.buyer_intent, "int-000000ff");
        assert_eq!(announcement.wallet, OWN);
        // Both sides of this pair said `Any`, so there is no agreed number to
        // report — and none is invented.
        assert_eq!(announcement.price_cents, None);
    }

    #[test]
    fn the_re_announced_price_is_the_one_the_pair_agreed() {
        let mut store = IntentStore::new();
        let now = 1_700_000_000;
        let mut sell = draft(Action::Sell);
        sell.condition = Condition::Above { price: UsdPrice::from_cents(9400) };
        let mut buy = draft(Action::Buy);
        buy.condition = Condition::Under { price: UsdPrice::from_cents(9600) };

        let mine = store.create(sell, now);
        let theirs = store.create_remote(
            buy,
            RemoteOrigin { intent_id: "int-000000ff".into(), wallet: PEER.into() },
            now,
        );
        for id in [&mine, &theirs] {
            store
                .transition(id, IntentStatus::Broadcast { route_len: 1 }, now)
                .unwrap();
        }
        let (found, terms) = store.find_counterparty(&mine).unwrap();
        store.pair(&mine, &found, terms, now).unwrap();
        settled(&mut store, &mine, &theirs, "5xYsignature");

        assert_eq!(
            announcement_for(&store, &mine, OWN).unwrap().price_cents,
            Some(9500)
        );
    }

    #[test]
    fn only_the_side_that_paid_announces() {
        // The buyer holds the same settled pair and must stay quiet — two
        // announcements for one trade would have the peers verifying each
        // other's claims in a loop.
        let (mut store, mine, theirs) = paired(Action::Buy);
        settled(&mut store, &theirs, &mine, "5xYsignature");
        assert!(announcement_for(&store, &mine, OWN).is_none());
        // Nor do we speak for the peer's own order that we merely mirror.
        assert!(announcement_for(&store, &theirs, OWN).is_none());
    }

    #[test]
    fn an_unsettled_trade_has_nothing_to_announce() {
        let (store, mine, _) = paired(Action::Sell);
        assert!(announcement_for(&store, &mine, OWN).is_none());
    }

    /// A Boost pair: the seller's order carries the concrete NFT, the buyer's
    /// names only a price ceiling in lamports.
    fn boost_pair(ceiling_lamports: u64) -> (IntentStore, IntentId, IntentId) {
        let mut store = IntentStore::new();
        let now = 1_700_000_000;
        let mut sell = draft(Action::Sell);
        sell.asset = "BOOST NFT".into();
        sell.amount = TokenAmount::parse("1", 0).unwrap();
        sell.condition = Condition::Any;
        let mut buy = draft(Action::Buy);
        buy.asset = "BOOST NFT".into();
        buy.amount = TokenAmount::parse("1", 0).unwrap();
        buy.condition = Condition::Under { price: UsdPrice::from_cents(ceiling_lamports) };

        let mine = store.create(sell, now);
        store.set_boost_mint(&mine, Some("BoostMint111111111111111111111111111111111".into()));
        let theirs = store.create_remote(
            buy,
            RemoteOrigin { intent_id: "int-000000ff".into(), wallet: PEER.into() },
            now,
        );
        for id in [&mine, &theirs] {
            store
                .transition(id, IntentStatus::Broadcast { route_len: 1 }, now)
                .unwrap();
        }
        let (found, terms) = store.find_counterparty(&mine).expect("boost sides match");
        store.pair(&mine, &found, terms, now).unwrap();
        (store, mine, theirs)
    }

    #[test]
    fn a_boost_seller_carries_the_mint_the_buyer_needs() {
        // The buyer's order names no NFT — it cannot, it is shopping. The mint
        // has to travel with the seller's side or the purchase has nothing to
        // spend against.
        let (store, mine, theirs) = boost_pair(200_000);
        let roles = Roles::resolve(&store, &mine, &theirs, OWN).unwrap();
        assert!(roles.we_are_seller);
        assert_eq!(roles.asset, "BOOST NFT");
        assert_eq!(
            roles.boost_mint.as_deref(),
            Some("BoostMint111111111111111111111111111111111")
        );
    }

    /// The guard `on_boost_purchase_request` applies before spending.
    fn buys(condition: cabal_core::Condition, listing_lamports: u64) -> bool {
        condition
            .ceiling()
            .is_none_or(|limit| listing_lamports <= limit.cents())
    }

    #[test]
    fn a_listing_above_the_buyers_limit_is_refused() {
        // `buy_boost` pays whatever the listing account says and takes no
        // ceiling of its own, so this check is the only thing between the
        // buyer's stated limit and its wallet.
        let (store, _, theirs) = boost_pair(50_000);
        let condition = store.get(&theirs).unwrap().draft.condition;
        assert!(!buys(condition, 100_000), "50000-lamport limit must refuse a 100000 listing");
        assert!(buys(condition, 49_999));
    }

    #[test]
    fn a_listing_at_exactly_the_limit_is_bought() {
        // The regression: the compose form defaults to a 0.0001 SOL ceiling
        // and the marketplace lists at exactly 100000 lamports, so the app's
        // own defaults refused each other and no Boost buy could complete.
        let (store, _, theirs) = boost_pair(100_000);
        let condition = store.get(&theirs).unwrap().draft.condition;
        assert!(buys(condition, 100_000));
    }

    #[test]
    fn a_buyer_that_named_no_limit_accepts_the_asking_price() {
        assert!(buys(cabal_core::Condition::Any, 100_000));
    }

    #[test]
    fn a_boost_pair_still_matches_when_the_seller_names_no_price() {
        // The marketplace lists at a fixed price and broadcasts `Any`, so the
        // band comes entirely from the buyer. Matching must survive that.
        let (store, mine, _) = boost_pair(200_000);
        let record = store.get(&mine).unwrap().matched.as_ref().unwrap();
        assert_eq!(record.price, Some(UsdPrice::from_cents(199_999)));
    }

    #[test]
    fn the_query_wire_shape_is_camel_case() {
        let json = serde_json::to_string(&SettlementQuery {
            seller_intent: "int-000000ff".into(),
            buyer_intent: "int-00000001".into(),
        })
        .unwrap();
        assert!(json.contains("\"sellerIntent\""), "{json}");
        assert!(json.contains("\"buyerIntent\""), "{json}");
    }

    #[test]
    fn the_announcement_wire_shape_is_camel_case() {
        // Both devices parse this; renaming a field silently breaks settlement
        // notification between builds.
        let json = serde_json::to_string(&TradeSettled {
            buyer_intent: "int-00000001".into(),
            seller_intent: "int-000000ff".into(),
            signature: "5xY".into(),
            price_cents: Some(9500),
            amount: "0.1".into(),
            wallet: PEER.into(),
        })
        .unwrap();
        assert!(json.contains("\"buyerIntent\":\"int-00000001\""), "{json}");
        assert!(json.contains("\"sellerIntent\":\"int-000000ff\""), "{json}");
        let parsed: TradeSettled = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.price_cents, Some(9500));
    }
}

/// Rebuilds the announcement for a settled sell order this device paid.
///
/// Kept derivable from the ledger rather than remembered separately, so a
/// replay months later says exactly what the original said — the ledger is
/// already the record of what happened, and a second copy could disagree with
/// it.
fn announcement_for(
    store: &IntentStore,
    id: &IntentId,
    own_wallet: &str,
) -> Option<TradeSettled> {
    let intent = store.get(id)?;
    let IntentStatus::Settled { proof, .. } = &intent.status else {
        return None;
    };
    // Only the payer announces, and only for its own order.
    if !intent.is_local() || intent.draft.action != Action::Sell {
        return None;
    }
    let record = intent.matched.as_ref()?;
    let buyer = store.get(&record.counterparty)?;
    let buyer_intent = buyer.origin.as_ref()?.intent_id.clone();
    if buyer_intent.is_empty() {
        return None;
    }
    Some(TradeSettled {
        buyer_intent,
        seller_intent: intent.id.to_string(),
        signature: proof.to_string(),
        price_cents: record.price.map(UsdPrice::cents),
        amount: intent.draft.amount.to_plain_string(),
        wallet: own_wallet.to_string(),
    })
}

/// Re-announces every trade this device has settled.
///
/// Called when a peer appears: the counterparty that missed the original
/// announcement is, by definition, one that was not connected at the time —
/// so reconnection is exactly the moment to say it again. Receivers verify the
/// signature on-chain before acting, and a side that already settled ignores
/// it, so repeating is harmless.
pub fn replay_settlements(ctx: DealContext) {
    tauri::async_runtime::spawn(async move {
        let own_wallet = ctx.bridge.lock().await.get_primary_address();
        let announcements: Vec<TradeSettled> = {
            let store = ctx.lock();
            store
                .all()
                .into_iter()
                .filter_map(|intent| announcement_for(&store, &intent.id, &own_wallet))
                .collect()
        };
        for announcement in announcements {
            let Ok(payload) = serde_json::to_string(&announcement) else { continue };
            ctx.publish("trade_settled", payload).await;
        }
    });
}

/// Answers a counterparty asking whether its trade settled.
pub fn on_settlement_query(ctx: DealContext, query: SettlementQuery) {
    tauri::async_runtime::spawn(async move {
        let own_wallet = ctx.bridge.lock().await.get_primary_address();
        let id = IntentId::new(query.seller_intent);
        let announcement = {
            let store = ctx.lock();
            announcement_for(&store, &id, &own_wallet)
        };
        // No answer when there is nothing settled to report. Silence is the
        // honest reply to "did you pay me?" when the answer is "not yet".
        let Some(announcement) = announcement else { return };
        if announcement.buyer_intent != query.buyer_intent {
            return;
        }
        let Ok(payload) = serde_json::to_string(&announcement) else { return };
        ctx.publish("trade_settled", payload).await;
    });
}

/// Buyer-side handler for a matched Boost listing. Only the local buyer can
/// sign this instruction because it spends that wallet's SOL.
pub fn on_boost_purchase_request(ctx: DealContext, request: BoostPurchaseRequest) {
    tauri::async_runtime::spawn(async move {
        let buyer = IntentId::new(request.buyer_intent.clone());
        let seller_mirror = {
            let store = ctx.lock();
            let Some(intent) = store.get(&buyer) else { return };
            if !intent.is_local() || intent.draft.action != Action::Buy || intent.draft.asset.as_ref() != "BOOST NFT" {
                return;
            }
            let Some(record) = intent.matched.as_ref() else { return };
            let Some(seller) = store.get(&record.counterparty) else { return };
            if seller.origin.as_ref().is_none_or(|origin| origin.intent_id != request.seller_intent) {
                return;
            }
            seller.id.clone()
        };

        // What the listing will actually charge, read from the chain rather
        // than taken from the seller's word or from the negotiated number.
        // `buy_boost` pays the listing price with no ceiling of its own, so
        // this is the only thing standing between the buyer's stated limit and
        // its wallet.
        let condition = {
            let store = ctx.lock();
            let Some(intent) = store.get(&buyer) else { return };
            intent.draft.condition
        };
        let listed = ctx
            .bridge
            .lock()
            .await
            .boost_listing_price(&request.seller_wallet, &request.mint)
            .await;
        // Compared against the buyer's *limit*, not against the price two
        // orders negotiated. A limit fills at the limit: a ceiling of 0.0001
        // buys a listing asking exactly 0.0001, which is the trade the user
        // asked for and, as it happens, the app's own default on both sides.
        let ceiling = condition.ceiling();
        match listed {
            Some(lamports) if ceiling.is_none_or(|limit| lamports <= limit.cents()) => {
                ctx.say(
                    &[&buyer, &seller_mirror],
                    &format!("BOOST MARKET: LISTING VERIFIED AT {lamports} LAMPORTS."),
                );
                // The listing price is what the chain will charge, so it is the
                // price of this deal. The band midpoint the orders matched on
                // was never going to be paid by anyone, and leaving it in the
                // ledger would misreport what the trade cost.
                {
                    let mut store = ctx.lock();
                    store.set_match_price(&buyer, Some(UsdPrice::from_cents(lamports)));
                    store.set_match_price(&seller_mirror, Some(UsdPrice::from_cents(lamports)));
                }
                ctx.persist();
            }
            Some(lamports) => {
                tracing::warn!(
                    mint = %request.mint,
                    lamports,
                    ceiling = ceiling.map(UsdPrice::cents),
                    "boost listing price is above the buyer's limit"
                );
                ctx.say(
                    &[&buyer, &seller_mirror],
                    &format!("BOOST MARKET: LISTING WANTS {lamports} LAMPORTS, ABOVE YOUR LIMIT. NOT BUYING."),
                );
                ctx.advance(&[&buyer, &seller_mirror], &IntentStatus::Cancelled);
                return;
            }
            None => {
                ctx.say(
                    &[&buyer, &seller_mirror],
                    "BOOST MARKET: NO LISTING FOUND ON-CHAIN FOR THAT NFT. NOT BUYING.",
                );
                ctx.advance(&[&buyer, &seller_mirror], &IntentStatus::Cancelled);
                return;
            }
        }

        // Submit it directly whenever this wallet can reach the chain. Queueing
        // unconditionally made the purchase depend on some *other* peer
        // volunteering to relay it — so an online buyer alone with its
        // counterparty never completed, and the seller's timer cancelled a deal
        // both sides had agreed. Relay is the fallback for an offline buyer,
        // not the only route.
        if ctx.bridge.lock().await.check_rpc_reachable().await {
            let bought = ctx
                .bridge
                .lock()
                .await
                .buy_boost_nft(&request.mint, &request.seller_wallet)
                .await;
            match bought {
                Ok(signature) => {
                    {
                        let mut store = ctx.lock();
                        store.set_boost_mint(&buyer, Some(request.mint.clone()));
                    }
                    ctx.say(&[&buyer, &seller_mirror], &format!("BOOST MARKET: PURCHASED ON DEVNET. TX {signature}"));
                    settle_boost_purchase(&ctx, &buyer, &seller_mirror, signature).await;
                }
                Err(error) => {
                    // The chain answered and said no — a missing listing, an
                    // unfunded buyer. Queueing that same instruction for a peer
                    // would only reproduce the rejection somewhere else.
                    tracing::warn!(mint = %request.mint, %error, "boost purchase rejected on-chain");
                    ctx.say(&[&buyer, &seller_mirror], &format!("BOOST MARKET ERROR: PURCHASE REJECTED ({error})."));
                    ctx.advance(&[&buyer, &seller_mirror], &IntentStatus::Cancelled);
                }
            }
            return;
        }

        let queued = ctx
            .bridge
            .lock()
            .await
            .queue_buy_boost_nft(&request.mint, &request.seller_wallet)
            .await;
        let queued = match queued {
            Ok(queued) => queued,
            Err(error) => {
                ctx.say(&[&buyer, &seller_mirror], &format!("BOOST MARKET ERROR: BUYER COULD NOT SIGN PURCHASE ({error})."));
                ctx.advance(&[&buyer, &seller_mirror], &IntentStatus::Cancelled);
                return;
            }
        };
        {
            let mut store = ctx.lock();
            store.set_relay_queue_id(&buyer, queued.id.clone());
            store.set_boost_mint(&buyer, Some(request.mint.clone()));
        }
        ctx.persist();
        ctx.notify(&[&buyer]);
        ctx.say(&[&buyer, &seller_mirror], "BOOST MARKET: BUYER SIGNED PURCHASE. QUEUED FOR MESH RELAY.");
        ctx.publish("relay_tx", serde_json::json!({
            "type": "RelayTx", "queue_id": queued.id, "raw_tx_hex": queued.raw_tx_hex,
            "summary": queued.summary,
        }).to_string()).await;
    });
}

/// Writes a completed Boost purchase into the ledger and tells the seller.
///
/// One place, because the purchase can complete three ways — submitted
/// directly by an online buyer, relayed by a peer, or drained from this
/// device's own queue once its RPC returned — and all three are the same fact:
/// the buyer's signature is on-chain and the NFT has moved.
async fn settle_boost_purchase(
    ctx: &DealContext,
    buyer: &IntentId,
    seller_mirror: &IntentId,
    signature: String,
) {
    let (price, seller_intent) = {
        let store = ctx.lock();
        let price = store
            .get(buyer)
            .and_then(|intent| intent.matched.as_ref().and_then(|m| m.price));
        let seller_intent = store
            .get(seller_mirror)
            .and_then(|intent| intent.origin.as_ref().map(|origin| origin.intent_id.clone()));
        (price, seller_intent)
    };
    ctx.advance(
        &[buyer, seller_mirror],
        &IntentStatus::Settled {
            proof: cabal_core::ProofHash::new(signature.clone()),
            filled_at: price.unwrap_or_else(|| UsdPrice::from_cents(0)),
            elapsed_ms: 0,
        },
    );
    // The seller cannot see the buyer's transaction land, so it is told —
    // and it verifies the signature for itself before believing this. `price`
    // here is the real listing price the guard in `on_boost_purchase_request`
    // already verified on-chain and wrote into the buyer's own record — not
    // the band midpoint the two orders matched at, which the seller's ledger
    // still holds and nobody was ever going to pay.
    if let Some(seller_intent) = seller_intent {
        ctx.publish(
            "boost_trade_settled",
            serde_json::to_string(&BoostTradeSettled {
                buyer_intent: buyer.to_string(),
                seller_intent,
                signature,
                price_lamports: price.map(UsdPrice::cents),
            })
            .unwrap_or_default(),
        )
        .await;
    }
}

pub fn on_boost_relay_confirmed(ctx: DealContext, queue_id: String, signature: String) {
    tauri::async_runtime::spawn(async move {
        let (buyer, seller_mirror) = {
            let store = ctx.lock();
            let Some(buyer) = store.by_relay_queue_id(&queue_id) else { return };
            // Already settled by whichever route got there first. Two relayers
            // and this device's own drain can all report the same purchase.
            if buyer.status.is_terminal() {
                return;
            }
            let Some(record) = buyer.matched.as_ref() else { return };
            let Some(seller) = store.get(&record.counterparty) else { return };
            (buyer.id.clone(), seller.id.clone())
        };
        ctx.say(&[&buyer, &seller_mirror], "BOOST MARKET: RELAY CONFIRMED. NFT PURCHASE SETTLED.");
        settle_boost_purchase(&ctx, &buyer, &seller_mirror, signature).await;
    });
}

pub fn on_boost_trade_settled(ctx: DealContext, announcement: BoostTradeSettled) {
    tauri::async_runtime::spawn(async move {
        let seller = IntentId::new(announcement.seller_intent);
        let buyer_mirror = {
            let store = ctx.lock();
            let Some(intent) = store.get(&seller) else { return };
            if !intent.is_local() || intent.draft.action != Action::Sell || intent.draft.asset.as_ref() != "BOOST NFT" { return; }
            let Some(record) = intent.matched.as_ref() else { return };
            let Some(buyer) = store.get(&record.counterparty) else { return };
            if buyer.origin.as_ref().is_none_or(|origin| origin.intent_id != announcement.buyer_intent) { return; }
            buyer.id.clone()
        };
        // The announced price is what the buyer actually verified on-chain
        // before paying. The seller's own record still holds the band
        // midpoint from negotiation — recorded here so both ledgers agree on
        // what the trade cost, not just that it happened.
        let price = announcement.price_lamports.map(UsdPrice::from_cents);
        {
            let mut store = ctx.lock();
            store.set_match_price(&seller, price);
            store.set_match_price(&buyer_mirror, price);
        }
        ctx.advance(&[&seller, &buyer_mirror], &IntentStatus::Settled {
            proof: cabal_core::ProofHash::new(announcement.signature),
            filled_at: price.unwrap_or_else(|| UsdPrice::from_cents(0)),
            elapsed_ms: 0,
        });
        ctx.say(&[&seller, &buyer_mirror], "BOOST MARKET: BUYER RELAY CONFIRMED. NFT PURCHASE SETTLED.");
    });
}

/// Chases the trades this device is owed.
///
/// Runs on a timer for every local order that matched, is waiting, and whose
/// counterparty is the paying side. Two things happen per pass:
///
/// - the counterparty is asked directly, which recovers a settlement whose
///   announcement was published while this node was offline;
/// - the counterparty's escrow account is read on-chain. A live escrow naming
///   this wallet as payee is real evidence the other side is performing, and
///   it is reported as exactly that — the deal is not marked settled off the
///   back of it, because locked is not paid.
pub fn chase_pending_settlements(ctx: DealContext) {
    tauri::async_runtime::spawn(async move {
        let own_wallet = ctx.bridge.lock().await.get_primary_address();

        // (our waiting buy order, the seller's own id, the seller's wallet)
        let owed: Vec<(IntentId, String, String)> = {
            let store = ctx.lock();
            store
                .all()
                .into_iter()
                .filter(|intent| {
                    intent.is_local()
                        && intent.draft.action == Action::Buy
                        && matches!(intent.status, IntentStatus::Waiting)
                })
                .filter_map(|intent| {
                    let record = intent.matched.as_ref()?;
                    let seller = store.get(&record.counterparty)?;
                    let origin = seller.origin.as_ref()?;
                    (!origin.intent_id.is_empty()).then(|| {
                        (intent.id.clone(), origin.intent_id.clone(), origin.wallet.clone())
                    })
                })
                .collect()
        };

        for (buyer_id, seller_intent, seller_wallet) in owed {
            let query = SettlementQuery {
                seller_intent,
                buyer_intent: buyer_id.to_string(),
            };
            if let Ok(payload) = serde_json::to_string(&query) {
                ctx.publish("settlement_query", payload).await;
            }

            let lock = ctx.bridge.lock().await.escrow_lock_for(&seller_wallet).await;
            if let Some(lock) = lock {
                if lock.payee == own_wallet && lock.active {
                    ctx.say(
                        &[&buyer_id],
                        &format!(
                            "ON-CHAIN: COUNTERPARTY ESCROW LOCKED, {} LAMPORTS TO YOU.",
                            lock.lamports
                        ),
                    );
                }
            }
        }
    });
}

/// Handles a counterparty's settlement announcement.
///
/// The signature is checked against the chain before anything is written: a
/// peer saying "I paid you" is a claim, and an unverifiable claim leaves the
/// order exactly where it was — waiting.
pub fn on_trade_settled(ctx: DealContext, announcement: TradeSettled) {
    tauri::async_runtime::spawn(async move {
        let buyer = cabal_core::IntentId::new(announcement.buyer_intent.clone());
        let mirror = {
            let store = ctx.lock();
            // Ours, still live, and actually the buy side of this deal.
            let Some(intent) = store.get(&buyer) else { return };
            if !intent.is_local() || intent.status.is_terminal() {
                return;
            }
            // The announcement has to be about the pair this order actually
            // agreed to. Without this, any peer could name someone else's order
            // and a real-but-unrelated signature, and watch it settle.
            let Some(record) = intent.matched.as_ref() else { return };
            let Some(mirror) = store.get(&record.counterparty) else { return };
            let names_our_counterparty = mirror
                .origin
                .as_ref()
                .is_some_and(|o| o.intent_id == announcement.seller_intent);
            if !names_our_counterparty || record.wallet != announcement.wallet {
                tracing::warn!(
                    seller = %announcement.seller_intent,
                    "ignoring a settlement announcement for an order we did not match"
                );
                return;
            }
            mirror.id.clone()
        };

        // The peer can announce before this node's RPC has caught up with the
        // block, so give confirmation a few chances before disbelieving it.
        let mut confirmed = false;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            if ctx.bridge.lock().await.signature_confirmed(&announcement.signature).await {
                confirmed = true;
                break;
            }
        }

        let sides = [&buyer, &mirror];

        if !confirmed {
            ctx.say(
                &sides,
                "SETTLEMENT AGENT: COUNTERPARTY PROOF UNVERIFIED. ORDER STAYS OPEN.",
            );
            return;
        }

        ctx.say(&sides, &format!("COUNTERPARTY SETTLED. TX {}", announcement.signature));

        // `Settled` is only reachable once routed, so a pair still sitting in
        // NEGOTIATING is routed first rather than skipping a state.
        {
            let mut store = ctx.lock();
            let now = crate::commands::now_secs();
            for id in &sides {
                let needs_route = store
                    .get(id)
                    .is_some_and(|intent| matches!(intent.status, IntentStatus::Negotiating { .. }));
                if needs_route {
                    let _ = store.transition(id, IntentStatus::FindingRoute, now);
                }
            }
        }
        let elapsed = {
            let store = ctx.lock();
            store
                .get(&buyer)
                .map(|intent| crate::commands::now_secs().saturating_sub(intent.created_at))
                .unwrap_or_default()
        };
        ctx.advance(
            &sides,
            &IntentStatus::Settled {
                proof: cabal_core::ProofHash::new(announcement.signature),
                filled_at: announcement
                    .price_cents
                    .map_or_else(|| UsdPrice::from_cents(0), UsdPrice::from_cents),
                elapsed_ms: u32::try_from(elapsed.saturating_mul(1000)).unwrap_or(u32::MAX),
            },
        );
    });
}
