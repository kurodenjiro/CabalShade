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
use crate::intent::{IntentDraft, IntentStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
        };
        self.intents.insert(id.clone(), stored);
        id
    }

    /// Looks an intent up by id.
    #[must_use]
    pub fn get(&self, id: &IntentId) -> Option<&StoredIntent> {
        self.intents.get(id)
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
