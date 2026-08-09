//! Happy-path end-to-end test for the intent lifecycle.
//!
//! This test drives the core domain flow that the desktop UI demos:
//!   1. Compose an intent
//!   2. Broadcast it to the mesh
//!   3. Transition through settlement
//!
//! The on-chain escrow create/release itself is exercised by the bridge's own
//! tests and requires a funded devnet wallet; here we verify that the store and
//! the mesh handle correctly carry an intent through broadcast and into the
//! settlement flow.

use cabalmesh_lib::mesh_handle::{MeshCommand, MeshHandle};
use std::sync::Arc;

use cabal_core::{
    Action, Condition, ExecutionMode, IntentDraft, IntentStatus, IntentStore, PrivacyLevel,
    TokenAmount, UsdPrice,
};

/// A minimal mock mesh that records every published intent.
struct RecordingMesh {
    published: Arc<std::sync::Mutex<Vec<cabalmesh_lib::mesh::PrivacyIntent>>>,
}

impl RecordingMesh {
    fn new() -> (MeshHandle, Self) {
        let (handle, mut rx) = MeshHandle::channel();
        let published = Arc::new(std::sync::Mutex::new(Vec::new()));
        let published_clone = published.clone();

        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                if let MeshCommand::Publish { intent, reply } = cmd {
                    let _ = published_clone.lock().unwrap().push(*intent);
                    let _ = reply.send(Ok(()));
                }
                // Snapshot/SetOffline/NearbyNodes are not used in this path.
            }
        });

        (handle, Self { published })
    }
}

/// A `BUY SOL` draft — the exact intent the prototype demo composes.
fn buy_sol_draft() -> IntentDraft {
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
        proof: cabal_core::ids::ProofHash::new("0xa4f2c9e1b70d5533"),
        filled_at: UsdPrice::from_cents(9421),
        elapsed_ms: 11_400,
    }
}

/// Compose -> broadcast: the intent is created as a draft, the mesh publish is
/// accepted, and the store advances it to the active `Broadcast` state.
#[test]
fn broadcast_publishes_to_the_mesh_and_advances_the_store() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (handle, mesh) = RecordingMesh::new();
        let mut store = IntentStore::new();
        let now = 1_700_000_000;

        let id = store.create(buy_sol_draft(), now);
        assert_eq!(store.get(&id).unwrap().status, IntentStatus::Draft);

        // The store transition must be accepted before we publish.
        store
            .transition(&id, IntentStatus::Broadcast { route_len: 1 }, now + 1)
            .unwrap();
        assert!(store.get(&id).unwrap().status.is_active());

        // Publish to the mock mesh and verify the intent actually left.
        handle
            .publish(cabalmesh_lib::mesh::PrivacyIntent {
                intent_type: "trade".into(),
                payload: serde_json::to_string(&buy_sol_draft()).unwrap(),
                encrypted: true,
                relay_path: vec!["origin_node".into()],
                relay_fee: None,
            })
            .await
            .unwrap();

        let published = mesh.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].intent_type, "trade");
        assert!(published[0].encrypted);
    });
}

/// A broadcast intent cannot settle until it has found a route — the lifecycle
/// table rejects the jump, exactly as the production `settle_intent` command
/// enforces before it will even attempt an on-chain transaction.
#[test]
fn settlement_requires_a_route_before_the_store_will_allow_it() {
    let mut store = IntentStore::new();
    let now = 1_700_000_000;

    let id = store.create(buy_sol_draft(), now);
    store
        .transition(&id, IntentStatus::Broadcast { route_len: 1 }, now + 1)
        .unwrap();

    // Broadcast -> Settled is illegal; there is nothing to settle through.
    assert!(store.transition(&id, settled(), now + 2).is_err());

    // FindingRoute -> Settled is the legal path the command layer drives.
    store
        .transition(&id, IntentStatus::FindingRoute, now + 2)
        .unwrap();
    store.transition(&id, settled(), now + 3).unwrap();

    let stored = store.get(&id).unwrap();
    assert!(stored.status.is_terminal());
    assert_eq!(stored.updated_at, now + 3);
}

/// The broadcast intent is eventually persisted — a restart must restore the
/// post-broadcast state rather than the draft, which is what keeps the offline
/// relay loop honest.
#[test]
fn a_broadcast_intent_round_trips_through_the_snapshot() {
    let mut store = IntentStore::new();
    let now = 1_700_000_000;

    let id = store.create(buy_sol_draft(), now);
    store
        .transition(&id, IntentStatus::Broadcast { route_len: 1 }, now + 1)
        .unwrap();

    let json = serde_json::to_string(&store.snapshot()).unwrap();
    let mut restored = IntentStore::new();
    restored.restore(serde_json::from_str(&json).unwrap());

    let stored = restored.get(&id).unwrap();
    assert!(matches!(stored.status, IntentStatus::Broadcast { .. }));
    // The id counter survived, so a new intent cannot collide with the restored one.
    let next = restored.create(buy_sol_draft(), now + 2);
    assert_ne!(next, id);
}

/// The mock mesh asserts on real behavior — a publish is recorded only when
/// the handle delivers it, and the recording is drained in order.
#[test]
fn published_intents_are_delivered_in_order() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (handle, mesh) = RecordingMesh::new();

        for i in 0..8 {
            handle
                .publish(cabalmesh_lib::mesh::PrivacyIntent {
                    intent_type: "trade".into(),
                    payload: format!("intent-{i}"),
                    encrypted: true,
                    relay_path: vec!["origin_node".into()],
                    relay_fee: None,
                })
                .await
                .unwrap();
        }

        let published = mesh.published.lock().unwrap();
        let payloads: Vec<&str> = published.iter().map(|i| i.payload.as_str()).collect();
        assert_eq!(
            payloads,
            (0..8).map(|i| format!("intent-{i}")).collect::<Vec<_>>().as_slice()
        );
    });
}
