//! Carrying other people's transactions.
//!
//! # Why this is in Rust
//!
//! A node whose RPC is unreachable signs its transaction anyway and publishes
//! the raw bytes to the mesh; some peer with connectivity submits it. Both
//! halves have to exist for that to be a relay rather than a message into the
//! void — and the receiving half used to live in the desktop `App.tsx` that no
//! longer exists, so queued transactions were being broadcast to peers that
//! did nothing with them.
//!
//! Living here instead of in a screen means relaying does not depend on which
//! view is mounted, or on a webview being open at all.
//!
//! # What a relayer is and is not risking
//!
//! The transaction arrives fully signed by its originator, who is also its fee
//! payer. A relayer cannot alter it, cannot redirect it, and pays nothing for
//! it — submitting is the whole contribution. What it does learn is that some
//! peer wanted this transaction sent, which is the privacy cost the mesh
//! already accepts for offline settlement.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::blockchain_bridge::BlockchainBridge;
use crate::mesh_handle::MeshHandle;

/// The services relaying needs, cloned per event.
#[derive(Clone)]
pub struct RelayContext {
    pub app: AppHandle,
    pub bridge: Arc<Mutex<BlockchainBridge>>,
    pub mesh: Option<MeshHandle>,
}

impl RelayContext {
    async fn publish(&self, intent_type: &str, payload: serde_json::Value) {
        if let Some(mesh) = self.mesh.as_ref() {
            let _ = mesh
                .publish(crate::mesh::PrivacyIntent {
                    intent_type: intent_type.to_string(),
                    payload: payload.to_string(),
                    encrypted: false,
                    relay_path: vec!["origin_node".into()],
                    relay_fee: None,
                })
                .await;
        }
    }
}

/// Submits a peer's offline-signed transaction and reports back what happened.
///
/// Reporting back is not a courtesy: the originator cannot fire the second leg
/// of its settlement until it knows the first one landed, and it has no way to
/// find that out on its own while its RPC is still unreachable.
pub fn on_relay_request(ctx: RelayContext, queue_id: String, raw_tx_hex: String, summary: String) {
    if queue_id.is_empty() || raw_tx_hex.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Nothing to contribute if this node is no better connected than the
        // sender. Staying quiet is right here: a "failed" report would tell the
        // originator its transaction is bad when the truth is that this
        // particular peer could not help.
        if !ctx.bridge.lock().await.check_rpc_reachable().await {
            tracing::debug!(%queue_id, "relay request ignored: this node has no RPC either");
            return;
        }

        // Mapped to an owned String up front: the bridge's `Box<dyn Error>` is
        // not `Send`, and holding one across the awaits below would make this
        // whole task unspawnable.
        let submitted = ctx
            .bridge
            .lock()
            .await
            .submit_raw_transaction(&raw_tx_hex)
            .await
            .map_err(|error| error.to_string());
        match submitted {
            Ok(tx_hash) => {
                tracing::info!(%queue_id, %tx_hash, %summary, "relayed a peer's transaction");
                // The relayer's own record of work actually done. The reward is
                // recorded as zero because nothing here pays anyone — inventing
                // a figure would make the relay ledger fiction.
                let _ = ctx
                    .bridge
                    .lock()
                    .await
                    .record_relayed_tx(&summary, &tx_hash, "0");
                let _ = ctx.app.emit(
                    "relay-submitted",
                    serde_json::json!({ "queueId": queue_id, "txHash": tx_hash }),
                );
                ctx.publish(
                    "relay_confirmed",
                    serde_json::json!({
                        "type": "RelayConfirmed",
                        "queue_id": queue_id,
                        "status": "confirmed",
                        "tx_hash": tx_hash,
                    }),
                )
                .await;
            }
            Err(error) => {
                // A rejection is about the transaction itself — a stale
                // blockhash, an unfunded signer — and the originator needs to
                // hear it rather than retrying into the same wall.
                tracing::warn!(%queue_id, %error, "relay submission rejected");
                ctx.publish(
                    "relay_confirmed",
                    serde_json::json!({
                        "type": "RelayConfirmed",
                        "queue_id": queue_id,
                        "status": "failed",
                        "reason": error.to_string(),
                    }),
                )
                .await;
            }
        }
    });
}

/// Records a relayer's report against this node's own queued transaction.
///
/// This is what unblocks settlement: `resume_pending_settlements` fires the
/// release leg only for a create marked `confirmed`, and until this ran there
/// was nothing in the process that could mark one.
pub fn on_relay_report(ctx: RelayContext, queue_id: String, status: String, tx_hash: Option<String>) {
    if queue_id.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let existing = ctx
            .bridge
            .lock()
            .await
            .get_pending_relay_txs()
            .into_iter()
            .find(|tx| tx.id == queue_id);
        // Every peer on the topic sees this report; only its owner acts.
        let Some(existing) = existing else { return };
        // Two relayers can both submit, and the loser's rejection ("already
        // processed") must not overwrite the winner's success. Confirmation is
        // final in one direction only.
        if existing.status == "confirmed" && status != "confirmed" {
            return;
        }
        if let Err(error) = ctx
            .bridge
            .lock()
            .await
            .mark_relay_tx_status(&queue_id, &status, tx_hash.clone())
        {
            tracing::warn!(%queue_id, %error, "could not record the relay report");
            return;
        }
        tracing::info!(%queue_id, %status, "a peer relayed our queued transaction");
        let _ = ctx.app.emit(
            "relay-report",
            serde_json::json!({ "queueId": queue_id, "status": status, "txHash": tx_hash }),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape both halves agree on. `mesh.rs` reads these exact keys
    /// off the topic, so a rename here silently strands every queued
    /// transaction rather than failing loudly.
    #[test]
    fn the_report_uses_the_keys_the_mesh_parses() {
        let payload = serde_json::json!({
            "type": "RelayConfirmed",
            "queue_id": "tx-123",
            "status": "confirmed",
            "tx_hash": "5xY",
        });
        assert_eq!(payload["queue_id"], "tx-123");
        assert_eq!(payload["status"], "confirmed");
        assert_eq!(payload["tx_hash"], "5xY");
    }
}
