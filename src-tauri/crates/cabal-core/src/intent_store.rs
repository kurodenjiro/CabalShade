//! The intent store: where intents live between composition and settlement.
//!
//! # Why this exists
//!
//! `list_intents` returned an empty list unconditionally — an honest rendering
//! of "nothing has been composed", but a dead end once composing exists. This
//! module is the persisted, transition-checked home for intents. It holds the
//! `cabal_core` types that already exist ([`crate::intent::IntentDraft`],
//! [`crate::intent::IntentStatus`]) and enforces the lifecycle through
//! [`IntentStatus::can_transition_to`] — the same table the proptest suite
//! covers.
//!
//! # What this module is not
//!
//! No I/O and no filesystem. Persistence is a [`serde`] document written
//! atomically by a caller-provided store (the app wires `cabal_store::JsonStore`
//! at a path from the platform's data directory). Keeping the store itself
//! path-free preserves this crate's "testable in milliseconds on the host"
//! charter: the rules are exercised on in-memory state.

use crate::ids::IntentId;
use crate::intent::{IntentDraft, IntentStatus, MatchTerms};
use crate::money::UsdPrice;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where an intent came from, when it is not this device's own.
///
/// A peer's order is mirrored into the local ledger so it can be matched and
/// rendered like any other, but the two nodes know it by different ids — this
/// records the id the originator uses, which is the only name both sides can
/// agree on when they later exchange a settlement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteOrigin {
    /// The id the originating peer knows this intent by.
    pub intent_id: String,
    /// The peer's public Solana receiving address. Never key material.
    pub wallet: String,
}

/// The counterparty an intent is paired with, and the terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRecord {
    /// The paired intent, as this ledger names it.
    pub counterparty: IntentId,
    /// The counterparty's public Solana receiving address. Empty when both
    /// sides were composed on this device.
    pub wallet: String,
    /// The price both conditions accept, from [`IntentDraft::match_with`] and
    /// then from whatever the negotiation agreed within those bounds.
    pub price: Option<UsdPrice>,
    /// The broadcast route length this order had before pairing, so unpairing
    /// restores the real one rather than inventing a hop count.
    #[serde(default)]
    pub route_len: u8,
}

/// A persisted intent: the user's request plus where it is in its life.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredIntent {
    pub id: IntentId,
    pub draft: IntentDraft,
    pub status: IntentStatus,
    /// Unix timestamp of composition, in seconds.
    pub created_at: u64,
    /// Unix timestamp of the last transition, in seconds.
    pub updated_at: u64,
    /// Set when this is a mirror of a peer's order rather than one composed
    /// here. `default` so ledgers written before matching existed still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<RemoteOrigin>,
    /// Set once this intent is paired with its opposite side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched: Option<MatchRecord>,
}

impl StoredIntent {
    /// Whether this order was composed on this device — as opposed to being a
    /// mirror of a peer's. Only a local order commits this wallet's funds.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        self.origin.is_none()
    }

    /// Whether this order can still be paired: live on the mesh, and not
    /// already spoken for.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.status, IntentStatus::Broadcast { .. }) && self.matched.is_none()
    }
}

/// Why a transition was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum IntentStoreError {
    /// No intent with that id exists.
    #[error("no intent with id {0}")]
    NotFound(IntentId),
    /// The transition table rejected the move. `from` and `to` are kept as
    /// fields for programmatic inspection; they do not interpolate into the
    /// message because `IntentStatus` deliberately has no `Display`.
    #[error("illegal intent transition")]
    IllegalTransition { from: IntentStatus, to: IntentStatus },
}

/// The persisted form of the store.
///
/// Kept as a plain struct so serialization is a single `serde` call — no
/// custom `Serialize` impl needed to hide internals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreSnapshot {
    intents: Vec<StoredIntent>,
    next_id: u64,
}

/// An in-memory, transition-checked intent ledger.
///
/// The app persists it through a caller-provided store; tests exercise it
/// directly. Indexed by [`BTreeMap`] so listing comes back in a stable order
/// (insertion order is lost on round-trip through a `Vec`).
#[derive(Debug, Default, Clone)]
pub struct IntentStore {
    intents: BTreeMap<IntentId, StoredIntent>,
    /// Monotonic id generator. Survives round-trips because it is persisted
    /// alongside the intents; a fresh counter would mint ids that collide with
    /// stored ones after a restart.
    next_id: u64,
}

impl IntentStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures the store for persistence.
    #[must_use]
    pub fn snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            intents: self.intents.values().cloned().collect(),
            next_id: self.next_id,
        }
    }

    /// Replaces the entire contents from a persisted snapshot.
    pub fn restore(&mut self, snapshot: StoreSnapshot) {
        self.intents = snapshot
            .intents
            .into_iter()
            .map(|i| (i.id.clone(), i))
            .collect();
        self.next_id = snapshot.next_id;
    }

    /// Composes a new draft intent and returns its id.
    ///
    /// The intent starts in [`IntentStatus::Draft`] — nothing has left the
    /// device. Broadcasting is a separate, checked transition.
    #[must_use]
    pub fn create(&mut self, draft: IntentDraft, now: u64) -> IntentId {
        self.next_id += 1;
        let id = IntentId::new(format!("int-{:08x}", self.next_id));
        let stored = StoredIntent {
            id: id.clone(),
            draft,
            status: IntentStatus::Draft,
            created_at: now,
            updated_at: now,
            origin: None,
            matched: None,
        };
        self.intents.insert(id.clone(), stored);
        id
    }

    /// Mirrors a peer's order into this ledger, tagged with where it came from.
    ///
    /// Separate from [`Self::create`] so a remote order can never be minted
    /// without its origin: an untagged mirror looks local, and a local order is
    /// one this device is willing to fund.
    #[must_use]
    pub fn create_remote(&mut self, draft: IntentDraft, origin: RemoteOrigin, now: u64) -> IntentId {
        let id = self.create(draft, now);
        if let Some(stored) = self.intents.get_mut(&id) {
            stored.origin = Some(origin);
        }
        id
    }

    /// Looks an intent up by id.
    #[must_use]
    pub fn get(&self, id: &IntentId) -> Option<&StoredIntent> {
        self.intents.get(id)
    }

    /// Looks a mirrored order up by the id its originating peer uses.
    ///
    /// An empty id matches nothing: a peer too old to send one leaves the field
    /// blank, and treating blank as a key would make every such order the same
    /// order.
    #[must_use]
    pub fn by_origin(&self, remote_intent_id: &str) -> Option<&StoredIntent> {
        if remote_intent_id.is_empty() {
            return None;
        }
        self.intents
            .values()
            .find(|intent| intent.origin.as_ref().is_some_and(|o| o.intent_id == remote_intent_id))
    }

    /// All intents, in creation order.
    #[must_use]
    pub fn all(&self) -> Vec<&StoredIntent> {
        self.intents.values().collect()
    }

    /// Attempts a status transition, enforcing the lifecycle table.
    ///
    /// # Errors
    ///
    /// [`IntentStoreError::NotFound`] if the id is unknown,
    /// [`IntentStoreError::IllegalTransition`] if the transition table rejects
    /// the move (e.g. settling a broadcast that never found a route).
    pub fn transition(
        &mut self,
        id: &IntentId,
        to: IntentStatus,
        now: u64,
    ) -> Result<(), IntentStoreError> {
        let stored = self.intents.get_mut(id).ok_or_else(|| IntentStoreError::NotFound(id.clone()))?;
        if !stored.status.can_transition_to(&to) {
            return Err(IntentStoreError::IllegalTransition {
                from: stored.status.clone(),
                to,
            });
        }
        stored.status = to;
        stored.updated_at = now;
        Ok(())
    }

    /// The open counterparty for `id`, and the terms the pair clears at.
    ///
    /// A pair must include at least one order composed here. Two mirrored peer
    /// orders may well match each other, but this node is not a party to that
    /// trade and must not pair — let alone settle — it.
    ///
    /// Ties go to the oldest open order, so both devices, scanning ledgers that
    /// hold the same two sides, pick the same one.
    #[must_use]
    pub fn find_counterparty(&self, id: &IntentId) -> Option<(IntentId, MatchTerms)> {
        let subject = self.get(id)?;
        if !subject.is_open() {
            return None;
        }
        self.intents
            .values()
            .filter(|other| other.id != subject.id && other.is_open())
            .filter(|other| subject.is_local() || other.is_local())
            .filter_map(|other| subject.draft.match_with(&other.draft).map(|terms| (other, terms)))
            .min_by_key(|(other, _)| (other.created_at, other.id.clone()))
            .map(|(other, terms)| (other.id.clone(), terms))
    }

    /// Pairs two open intents and moves both into negotiation.
    ///
    /// Each side records the *other* side's wallet, so the settlement leg has a
    /// real payee rather than a fallback address.
    ///
    /// # Errors
    ///
    /// [`IntentStoreError::NotFound`] if either id is unknown, or
    /// [`IntentStoreError::IllegalTransition`] if either side is no longer in a
    /// state that can negotiate.
    pub fn pair(
        &mut self,
        left: &IntentId,
        right: &IntentId,
        terms: MatchTerms,
        now: u64,
    ) -> Result<(), IntentStoreError> {
        let wallet_of = |id: &IntentId| -> Result<String, IntentStoreError> {
            let intent = self.get(id).ok_or_else(|| IntentStoreError::NotFound(id.clone()))?;
            Ok(intent.origin.as_ref().map_or_else(String::new, |o| o.wallet.clone()))
        };
        let left_wallet = wallet_of(left)?;
        let right_wallet = wallet_of(right)?;

        let negotiating = IntentStatus::Negotiating { bids: 1, best: terms.price };
        // Both transitions are checked before either record is written, so a
        // rejected side cannot leave the other holding a match to nothing.
        for id in [left, right] {
            let intent = self.get(id).ok_or_else(|| IntentStoreError::NotFound(id.clone()))?;
            if !intent.status.can_transition_to(&negotiating) {
                return Err(IntentStoreError::IllegalTransition {
                    from: intent.status.clone(),
                    to: negotiating.clone(),
                });
            }
        }

        for (id, counterparty, wallet) in [
            (left, right, right_wallet),
            (right, left, left_wallet),
        ] {
            let intent = self.intents.get_mut(id).expect("checked above");
            let route_len = match intent.status {
                IntentStatus::Broadcast { route_len } => route_len,
                _ => 1,
            };
            intent.matched = Some(MatchRecord {
                counterparty: counterparty.clone(),
                wallet,
                price: terms.price,
                route_len,
            });
            intent.status = negotiating.clone();
            intent.updated_at = now;
        }
        Ok(())
    }

    /// Breaks a pair, returning whichever side is still live to the open book.
    ///
    /// Called when one side is cancelled: the other has done nothing wrong and
    /// must be matchable again rather than negotiating with an order that no
    /// longer exists. A side that has moved past negotiation keeps its state —
    /// routing or settlement is not something a cancellation elsewhere undoes.
    ///
    /// Returns the counterparty that was released, if there was one.
    pub fn unpair(&mut self, id: &IntentId, now: u64) -> Option<IntentId> {
        let counterparty = self.intents.get_mut(id)?.matched.take()?.counterparty;
        let other = self.intents.get_mut(&counterparty)?;
        // Its own record holds its own hop count — the cancelled side's record
        // describes the cancelled side.
        let route_len = other.matched.take().map_or(1, |record| record.route_len);
        if matches!(other.status, IntentStatus::Negotiating { .. }) {
            other.status = IntentStatus::Broadcast { route_len };
            other.updated_at = now;
        }
        Some(counterparty)
    }

    /// Records the price the two sides settled on, once negotiation has moved
    /// it inside the matched band.
    pub fn set_match_price(&mut self, id: &IntentId, price: Option<UsdPrice>) {
        if let Some(record) = self.intents.get_mut(id).and_then(|i| i.matched.as_mut()) {
            record.price = price;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Action, Condition, ExecutionMode, FailureReason, PrivacyLevel};
    use crate::money::TokenAmount;

    fn draft() -> IntentDraft {
        IntentDraft {
            action: Action::Buy,
            asset: "SOL".into(),
            condition: Condition::Any,
            amount: TokenAmount::parse("10", 9).unwrap(),
            mode: ExecutionMode::Shark,
            privacy: PrivacyLevel::Medium,
        }
    }

    fn settled() -> IntentStatus {
        IntentStatus::Settled {
            proof: crate::ids::ProofHash::new("0xa4f2c9e1b70d5533"),
            filled_at: crate::money::UsdPrice::from_cents(9421),
            elapsed_ms: 11_400,
        }
    }

    #[test]
    fn create_mints_distinct_ids_in_order() {
        let mut store = IntentStore::new();
        let a = store.create(draft(), 1_700_000_000);
        let b = store.create(draft(), 1_700_000_001);
        assert_ne!(a, b);
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn a_fresh_intent_is_a_draft() {
        let mut store = IntentStore::new();
        let id = store.create(draft(), 1_700_000_000);
        assert_eq!(store.get(&id).unwrap().status, IntentStatus::Draft);
    }

    #[test]
    fn draft_can_only_broadcast_or_cancel() {
        let mut store = IntentStore::new();
        let id = store.create(draft(), 1_700_000_000);

        // Draft -> Settled is rejected by the table.
        assert!(matches!(
            store.transition(&id, settled(), 1_700_000_010),
            Err(IntentStoreError::IllegalTransition { .. })
        ));

        // Broadcast is legal.
        store
            .transition(&id, IntentStatus::Broadcast { route_len: 1 }, 1_700_000_010)
            .unwrap();
        assert!(store.get(&id).unwrap().status.is_active());
    }

    #[test]
    fn settlement_requires_a_route_first() {
        let mut store = IntentStore::new();
        let id = store.create(draft(), 1_700_000_000);

        store
            .transition(&id, IntentStatus::Broadcast { route_len: 1 }, 1_700_000_010)
            .unwrap();
        // Broadcast -> Settled is illegal; FindingRoute -> Settled is not.
        assert!(matches!(
            store.transition(&id, settled(), 1_700_000_020),
            Err(IntentStoreError::IllegalTransition { .. })
        ));
        store
            .transition(&id, IntentStatus::FindingRoute, 1_700_000_020)
            .unwrap();
        store.transition(&id, settled(), 1_700_000_030).unwrap();
    }

    #[test]
    fn unknown_ids_are_reported_not_panicked() {
        let mut store = IntentStore::new();
        let missing = IntentId::new("int-00000099");
        assert!(matches!(
            store.transition(&missing, IntentStatus::Cancelled, 1_700_000_000),
            Err(IntentStoreError::NotFound(_))
        ));
    }

    #[test]
    fn terminal_states_accept_nothing() {
        let mut store = IntentStore::new();
        let id = store.create(draft(), 1_700_000_000);
        store
            .transition(&id, IntentStatus::Broadcast { route_len: 1 }, 1_700_000_010)
            .unwrap();
        store
            .transition(&id, IntentStatus::FindingRoute, 1_700_000_020)
            .unwrap();
        store.transition(&id, settled(), 1_700_000_030).unwrap();
        assert!(matches!(
            store.transition(&id, IntentStatus::Cancelled, 1_700_000_040),
            Err(IntentStoreError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn failing_records_the_reason() {
        let mut store = IntentStore::new();
        let id = store.create(draft(), 1_700_000_000);
        store
            .transition(
                &id,
                IntentStatus::Failed { reason: FailureReason::NoRoute },
                1_700_000_010,
            )
            .unwrap();
        assert!(store.get(&id).unwrap().status.is_terminal());
    }

    fn sided(action: Action, condition: Condition) -> IntentDraft {
        IntentDraft { action, condition, ..draft() }
    }

    fn origin(intent_id: &str) -> RemoteOrigin {
        RemoteOrigin {
            intent_id: intent_id.to_string(),
            wallet: "GyzdBSo87y5vT4oyoBCAdeT7hSz4C2ihj89QrVGCpdRa".to_string(),
        }
    }

    /// Broadcasts `id` so it is open for matching.
    fn open(store: &mut IntentStore, id: &IntentId) {
        store
            .transition(id, IntentStatus::Broadcast { route_len: 1 }, 1_700_000_001)
            .unwrap();
    }

    #[test]
    fn a_local_order_pairs_with_a_mirrored_peer_order() {
        let mut store = IntentStore::new();
        let mine = store.create(sided(Action::Buy, Condition::Any), 1_700_000_000);
        let theirs = store.create_remote(
            sided(Action::Sell, Condition::Any),
            origin("int-0000002a"),
            1_700_000_000,
        );
        open(&mut store, &mine);
        open(&mut store, &theirs);

        let (found, terms) = store.find_counterparty(&mine).expect("opposite side is open");
        assert_eq!(found, theirs);
        store.pair(&mine, &theirs, terms, 1_700_000_002).unwrap();

        let record = store.get(&mine).unwrap().matched.as_ref().unwrap();
        assert_eq!(record.counterparty, theirs);
        // Each side carries the *other* side's wallet, which is what the
        // settlement leg pays.
        assert_eq!(record.wallet, origin("").wallet);
        assert!(store.get(&theirs).unwrap().matched.as_ref().unwrap().wallet.is_empty());
        assert!(matches!(
            store.get(&theirs).unwrap().status,
            IntentStatus::Negotiating { .. }
        ));
    }

    #[test]
    fn two_mirrored_peer_orders_do_not_pair_here() {
        let mut store = IntentStore::new();
        let one = store.create_remote(sided(Action::Buy, Condition::Any), origin("a"), 1_700_000_000);
        let two = store.create_remote(sided(Action::Sell, Condition::Any), origin("b"), 1_700_000_000);
        open(&mut store, &one);
        open(&mut store, &two);
        // This device is party to neither, so it has nothing to settle.
        assert_eq!(store.find_counterparty(&one), None);
    }

    #[test]
    fn an_already_matched_order_is_not_offered_again() {
        let mut store = IntentStore::new();
        let buy = store.create(sided(Action::Buy, Condition::Any), 1_700_000_000);
        let sell = store.create_remote(sided(Action::Sell, Condition::Any), origin("a"), 1_700_000_000);
        let late = store.create_remote(sided(Action::Sell, Condition::Any), origin("b"), 1_700_000_001);
        for id in [&buy, &sell, &late] {
            open(&mut store, id);
        }
        let (found, terms) = store.find_counterparty(&buy).unwrap();
        store.pair(&buy, &found, terms, 1_700_000_002).unwrap();

        assert_eq!(store.find_counterparty(&buy), None);
        assert_eq!(store.find_counterparty(&sell), None);
        // The third order is still open and still unmatched.
        assert!(store.get(&late).unwrap().is_open());
    }

    #[test]
    fn unpairing_returns_the_surviving_side_to_the_open_book() {
        let mut store = IntentStore::new();
        let mine = store.create(sided(Action::Buy, Condition::Any), 1_700_000_000);
        let theirs =
            store.create_remote(sided(Action::Sell, Condition::Any), origin("a"), 1_700_000_000);
        store
            .transition(&mine, IntentStatus::Broadcast { route_len: 3 }, 1_700_000_001)
            .unwrap();
        open(&mut store, &theirs);
        let (found, terms) = store.find_counterparty(&mine).unwrap();
        store.pair(&mine, &found, terms, 1_700_000_002).unwrap();

        // Cancelling one side releases the other with its real hop count.
        assert_eq!(store.unpair(&theirs, 1_700_000_003), Some(mine.clone()));
        store
            .transition(&theirs, IntentStatus::Cancelled, 1_700_000_003)
            .unwrap();
        let survivor = store.get(&mine).unwrap();
        assert_eq!(survivor.status, IntentStatus::Broadcast { route_len: 3 });
        assert!(survivor.is_open(), "it must be matchable again");
    }

    #[test]
    fn unpairing_does_not_rewind_a_side_that_has_already_routed() {
        let mut store = IntentStore::new();
        let mine = store.create(sided(Action::Buy, Condition::Any), 1_700_000_000);
        let theirs =
            store.create_remote(sided(Action::Sell, Condition::Any), origin("a"), 1_700_000_000);
        open(&mut store, &mine);
        open(&mut store, &theirs);
        let (found, terms) = store.find_counterparty(&mine).unwrap();
        store.pair(&mine, &found, terms, 1_700_000_002).unwrap();
        store
            .transition(&mine, IntentStatus::FindingRoute, 1_700_000_003)
            .unwrap();

        store.unpair(&theirs, 1_700_000_004);
        assert_eq!(store.get(&mine).unwrap().status, IntentStatus::FindingRoute);
    }

    #[test]
    fn incompatible_prices_leave_both_sides_open() {
        let mut store = IntentStore::new();
        let buy = store.create(
            sided(Action::Buy, Condition::Under { price: crate::money::UsdPrice::from_cents(9000) }),
            1_700_000_000,
        );
        let sell = store.create_remote(
            sided(Action::Sell, Condition::Above { price: crate::money::UsdPrice::from_cents(9500) }),
            origin("a"),
            1_700_000_000,
        );
        open(&mut store, &buy);
        open(&mut store, &sell);
        assert_eq!(store.find_counterparty(&buy), None);
    }

    #[test]
    fn a_pair_survives_persistence() {
        let mut store = IntentStore::new();
        let buy = store.create(sided(Action::Buy, Condition::Any), 1_700_000_000);
        let sell = store.create_remote(sided(Action::Sell, Condition::Any), origin("a"), 1_700_000_000);
        open(&mut store, &buy);
        open(&mut store, &sell);
        let (found, terms) = store.find_counterparty(&buy).unwrap();
        store.pair(&buy, &found, terms, 1_700_000_002).unwrap();

        let json = serde_json::to_string(&store.snapshot()).unwrap();
        let mut restored = IntentStore::new();
        restored.restore(serde_json::from_str(&json).unwrap());

        assert_eq!(
            restored.get(&buy).unwrap().matched.as_ref().unwrap().counterparty,
            sell
        );
        assert_eq!(restored.by_origin("a").map(|i| i.id.clone()), Some(sell));
    }

    #[test]
    fn a_ledger_written_before_matching_still_loads() {
        // No `origin` and no `matched` keys: exactly what the previous build
        // wrote. Losing those ledgers on upgrade would delete live intents.
        let json = r#"{
            "intents": [{
                "id": "int-00000001",
                "draft": {
                    "action": "BUY", "asset": "SOL",
                    "condition": { "kind": "any" },
                    "amount": { "raw": 10000000000, "decimals": 9 },
                    "mode": "SHARK", "privacy": "MEDIUM"
                },
                "status": { "status": "BROADCAST", "route_len": 1 },
                "createdAt": 1700000000, "updatedAt": 1700000000
            }],
            "nextId": 1
        }"#;
        let mut store = IntentStore::new();
        store.restore(serde_json::from_str(&json).unwrap());
        let restored = store.get(&IntentId::new("int-00000001")).unwrap();
        assert!(restored.is_local());
        assert!(restored.is_open());
    }

    #[test]
    fn snapshot_round_trips_including_the_id_counter() {
        let mut store = IntentStore::new();
        let a = store.create(draft(), 1_700_000_000);
        store
            .transition(&a, IntentStatus::Broadcast { route_len: 2 }, 1_700_000_010)
            .unwrap();

        let json = serde_json::to_string(&store.snapshot()).unwrap();
        let mut restored = IntentStore::new();
        restored.restore(serde_json::from_str(&json).unwrap());

        assert_eq!(restored.all().len(), 1);
        // The counter survived, so the next id does not collide with `a`.
        let b = restored.create(draft(), 1_700_000_020);
        assert_ne!(b, a);
    }
}
