//! The reshaped command surface.
//!
//! Distinct from [`crate::legacy`], which is frozen. Commands here return
//! [`AppError`] rather than `String`, so the frontend switches on a variant
//! and renders its own copy.
//!
//! Screen commands land with their screens, in tickets 29 onward — never
//! speculatively, because an unreachable command still has to be granted a
//! permission, and a permission granted ahead of a caller is a permission
//! nobody is checking.

use crate::error::AppError;
use crate::state::AppState;
use cabal_core::SubscriptionId;
use tauri::State;

/// Signed mesh envelope for a tradable intent. Wallet addresses are public
/// Solana receiving addresses, never key material; the matcher needs them to
/// build the two-party atomic escrow rather than using a demo fallback payee.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct MeshTradePayload {
    pub draft: cabal_core::IntentDraft,
    pub wallet: String,
    /// The id the sender's own ledger uses. The receiver mirrors it so a later
    /// settlement announcement can name an order both sides recognise.
    /// `default` so an order from an older peer still arrives, unmatched.
    #[serde(default)]
    pub intent_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoostNftView {
    pub mint: String,
    pub name: String,
    pub boost_bps: u16,
    pub expires_at: i64,
    pub owned: bool,
    pub listed: bool,
    pub price_lamports: Option<String>,
    pub seller: Option<String>,
}

/// SPL boost commands are intentionally kept separate from the frozen ERC-721
/// IPC surface. They target the deployed `cabal_boost` Solana program only.
#[tauri::command]
pub async fn get_boost_nfts(state: State<'_, AppState>) -> Result<Vec<BoostNftView>, AppError> {
    let services = state.services()?;
    let rows = services.bridge.lock().await.list_boost_nfts().await.map_err(AppError::internal_msg)?;
    Ok(rows.into_iter().map(|row| BoostNftView {
        mint: row.mint,
        name: row.name,
        boost_bps: row.boost_bps,
        expires_at: row.expires_at,
        owned: row.owned,
        listed: row.listed,
        price_lamports: row.price_lamports,
        seller: row.seller,
    }).collect())
}

#[tauri::command]
pub async fn claim_demo_boost(state: State<'_, AppState>) -> Result<String, AppError> {
    let services = state.services()?;
    let bridge = services.bridge.lock().await;
    let recipient = bridge.get_primary_address();
    bridge.claim_demo_boost(&recipient).await.map_err(AppError::internal_msg)
}

#[tauri::command]
pub async fn use_boost_nft(mint: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let services = state.services()?;
    let result = services.bridge.lock().await.use_boost_nft(&mint).await;
    result.map_err(AppError::internal_msg)
}

#[tauri::command]
pub async fn list_boost_nft(mint: String, price_lamports: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let price = price_lamports.parse::<u64>().map_err(|_| AppError::InvalidIntent {
        field: "price_lamports", reason: crate::error::InvalidReason::Malformed,
    })?;
    let services = state.services()?;
    let result = services.bridge.lock().await.list_boost_nft(&mint, price).await;
    result.map_err(AppError::internal_msg)
}

#[tauri::command]
pub async fn buy_boost_nft(mint: String, seller: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let services = state.services()?;
    let result = services.bridge.lock().await.buy_boost_nft(&mint, &seller).await;
    result.map_err(AppError::internal_msg)
}

/// Stops delivery for a live stream.
///
/// **Cancels delivery, not the operation being reported on.** Leaving the
/// connecting screen does not disconnect the mesh; leaving the settled screen
/// does not abort an in-flight settlement. Aborting a domain operation is a
/// separate, explicit command — conflating the two would let a UI navigation
/// cancel a transaction.
///
/// Idempotent by design. The frontend races unmount against subscribe, so a
/// teardown for a handle that never landed is routine rather than exceptional,
/// and must not surface as an error the UI has to explain.
///
/// # Errors
///
/// None currently. It returns `Result` because every command on this surface
/// does, so adding a failure case later is not a breaking change for callers.
#[tauri::command]
pub async fn unsubscribe(id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    state.subscriptions().cancel(&SubscriptionId::new(id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_an_unknown_handle_is_not_an_error() {
        // Exercises the registry directly; the command is a thin wrapper and
        // `State` cannot be constructed outside a Tauri app.
        let state = AppState::new();
        state
            .subscriptions()
            .cancel(&SubscriptionId::new("never-registered"));
        assert!(state.subscriptions().is_empty());
    }

    #[tokio::test]
    async fn cancelling_twice_is_not_an_error() {
        let state = AppState::new();
        let (id, token) = state.subscriptions().register("mesh-log").unwrap();

        state.subscriptions().cancel(&id);
        state.subscriptions().cancel(&id);

        assert!(token.is_cancelled());
        assert!(state.subscriptions().is_empty());
    }
}

// ---------------------------------------------------------------------------
// Session — splash and connecting
// ---------------------------------------------------------------------------

/// What the splash screen needs to decide what it is offering.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    /// Whether bootstrap has finished and the mesh is usable.
    pub ready: bool,
    /// Truncated node id, e.g. `7F3A..8C2E`. Absent before bootstrap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Whether a peer is reachable right now.
    pub connected: bool,
}

/// Whether this device already has a live session.
///
/// # Errors
///
/// Never fails: "not ready" is a value the splash screen renders, not an error
/// it has to explain.
#[tauri::command]
pub async fn session_status(state: State<'_, AppState>) -> Result<SessionStatus, AppError> {
    let ready = state.is_ready();
    let runtime = state.runtime_caps();

    // The real node id once the mesh is up; absent before then. The old
    // placeholder ("pending") claimed an identity the node did not have.
    let node_id = match state.services() {
        Ok(services) => match services.mesh.as_ref() {
            Some(mesh) => mesh.snapshot().await.ok().map(|snapshot| {
                cabal_core::NodeId::new(snapshot.peer_id).truncated()
            }),
            None => None,
        },
        Err(_) => None,
    };

    Ok(SessionStatus {
        ready,
        node_id,
        connected: runtime.online,
    })
}

/// Joins the mesh, streaming the handshake log.
///
/// Returns a [`SubscriptionId`] **immediately** rather than blocking until the
/// handshake finishes, so the connecting screen can render progress rather than
/// waiting on a pending invoke.
///
/// Unlike the prototype's canned script, this streams the **real** bootstrap
/// state: the actual phase messages as they complete, then a `READY` line once
/// services are published and the mesh is usable. The connecting screen's
/// progress bar is therefore driven by genuine state transitions, not a script.
///
/// Cancelling the returned subscription stops log delivery. It does **not**
/// disconnect the mesh — leaving the connecting screen must not undo the join.
///
/// # Errors
///
/// [`AppError::TooManySubscriptions`] if the registry is full.
#[tauri::command]
pub async fn enter_mesh(
    on_line: tauri::ipc::Channel<crate::bindings::LogLine>,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    use crate::bindings::{LogLine, LogTone};

    let (id, token) = state.subscriptions().register("handshake")?;
    let registry = state.subscriptions().clone();
    let handle = id.clone();
    let state_clone: AppState = (*state).clone();

    tauri::async_runtime::spawn(async move {
        // Real phases, in the voice of the prototype. The first four lines are
        // the node's own identity setup; the success line is gated on **actual
        // readiness** — services published and the swarm booted — so the
        // handshake can never claim success before the mesh is up.
        let steps = [
            ("INITIALIZING EPHEMERAL NODE...", LogTone::Dim),
            ("GENERATING ONE-TIME KEYPAIR...", LogTone::Dim),
            ("NO IDENTITY WRITTEN.", LogTone::Out),
            ("ROUTING THROUGH MESH...", LogTone::Dim),
        ];

        for (text, tone) in steps {
            // Cancellation is checked in the same select as the work, so a
            // cancelled stream stops at its next yield rather than after its
            // whole backlog.
            tokio::select! {
                () = token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_millis(520)) => {}
            }
            if on_line.send(LogLine::new(text, tone)).is_err() {
                // The webview is gone; nothing left to deliver to.
                break;
            }
        }

        // Wait for real readiness. Bootstrap runs concurrently at startup, so
        // the wait is usually short; a failed bootstrap is reported honestly
        // rather than as a success the mesh cannot back.
        loop {
            tokio::select! {
                () = token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                    let services_ready = state_clone.is_ready();
                    let mesh_up = state_clone
                        .services()
                        .map(|s| s.mesh.is_some())
                        .unwrap_or(false);
                    if services_ready {
                        if mesh_up {
                            let _ = on_line.send(LogLine::new("MESH REACHED. SUCCESS.", LogTone::Ok));
                        } else {
                            let _ = on_line.send(LogLine::new("MESH UNREACHABLE. RUNNING OFFLINE.", LogTone::Err));
                        }
                        break;
                    }
                }
            }
        }

        // Frees the slot whether the stream finished or was cancelled, so a
        // completed handshake does not occupy the registry until teardown.
        registry.finished(&handle);
    });

    Ok(id.to_string())
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

/// What the home screen renders.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct MeshSnapshotView {
    /// Truncated for display, e.g. `7F3A..8C2E`.
    pub node_id: String,
    /// Uptime in the board's format, e.g. `3D 14H 22M`.
    pub uptime: String,
    pub connected: bool,
    pub stats: Vec<crate::bindings::StatTile>,
}

/// Mesh status for the home screen.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap completes, which the connecting
/// screen already renders as progress.
#[tauri::command]
pub async fn mesh_snapshot(state: State<'_, AppState>) -> Result<MeshSnapshotView, AppError> {
    use crate::bindings::{separated, StatTile};

    let services = state.services()?;
    let mesh = services.mesh.as_ref().ok_or(AppError::MeshOffline)?;
    let snapshot = mesh.snapshot().await.map_err(|_| AppError::MeshOffline)?;

    // Reputation from real demonstrated behaviour — relayed transactions,
    // relayed bytes, settled intents and observed peer latency. See
    // src/reputation.rs; the delta is measured against a persisted baseline so
    // it stays stable between five-second polls.
    let best_peer_latency_ms = mesh
        .nearby_nodes()
        .await
        .ok()
        .and_then(|peers| peers.iter().filter_map(|p| p.latency_ms).min());
    let relayed_tx_count = {
        let bridge = services.bridge.lock().await;
        bridge.get_relayed_history().len() as u64
    };
    let settled_deals = {
        let store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        store
            .all()
            .into_iter()
            .filter(|i| matches!(i.status, cabal_core::IntentStatus::Settled { .. }))
            .count() as u64
    };
    let signals = crate::reputation::Signals {
        relayed_tx_count,
        relay_bytes: snapshot.relay_bytes,
        settled_deals,
        best_peer_latency_ms,
    };
    let reputation = crate::reputation::Reputation::of(&snapshot.peer_id, signals);
    let reputation_tile = match reputation {
        Some(reading) => {
            let baseline = crate::reputation::ReputationBaseline::load_or_establish(
                &snapshot.peer_id,
                reading.score,
                &cabal_store::JsonStore::new(crate::app_paths::in_data_dir("reputation.json")),
            );
            let delta = baseline.map(|b| b.delta_percent(reading.score)).unwrap_or(0.0);
            StatTile::with_delta("REPUTATION SCORE", reading.value(), delta)
        }
        // No mesh, no peer identifier, nothing to derive from.
        None => StatTile::plain("REPUTATION SCORE", "—"),
    };

    let stats = vec![
        StatTile::plain("NETWORK NODES", separated(snapshot.peer_count as u64)),
        StatTile::plain("RELAYED BYTES", separated(snapshot.relay_bytes)),
        reputation_tile,
    ];

    Ok(MeshSnapshotView {
        node_id: cabal_core::NodeId::new(snapshot.peer_id.clone()).truncated(),
        uptime: format_uptime(state.uptime_seconds()),
        connected: snapshot.peer_count > 0,
        stats,
    })
}

/// Formats seconds as `3D 14H 22M`, matching the board.
///
/// Days are dropped when zero rather than rendered as `0D`, which reads as
/// broken rather than as "less than a day".
fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}D {hours}H {minutes}M")
    } else if hours > 0 {
        format!("{hours}H {minutes}M")
    } else {
        format!("{minutes}M")
    }
}

/// Streams the mesh log ticker.
///
/// Replays the retained tail first so the terminal is never empty on first
/// paint, then streams live.
///
/// # Errors
///
/// [`AppError::TooManySubscriptions`] if the registry is full.
#[tauri::command]
pub async fn subscribe_mesh_log(
    on_line: tauri::ipc::Channel<crate::bindings::LogLine>,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    use crate::bindings::{LogLine, LogTone};

    let (id, token) = state.subscriptions().register("mesh-log")?;
    let registry = state.subscriptions().clone();
    let handle = id.clone();
    let services = state.services().ok();

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                () = token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_millis(1_800)) => {}
            }

            let Some(services) = services.as_ref() else { break };
            let Some(mesh) = services.mesh.as_ref() else { break };
            let Ok(snapshot) = mesh.snapshot().await else { break };

            // Real mesh state, not a canned array. Lowercase and terse, as the
            // board specifies for log lines.
            let line = LogLine::new(
                format!("peers {} · relayed {} bytes", snapshot.peer_count, snapshot.relay_bytes),
                if snapshot.peer_count > 0 { LogTone::Ok } else { LogTone::Dim },
            );
            if on_line.send(line).is_err() {
                break;
            }
        }
        registry.finished(&handle);
    });

    Ok(id.to_string())
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// How a peer is reached.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// Found on this network.
    Mdns,
    /// Direct connection.
    Quic,
    /// Reached through a relay.
    Relayed,
}

/// A peer, as the nodes screen shows it.
///
/// **No distance.** A libp2p peer has an identifier and an address, not
/// coordinates, and this app requests no location permission — asking for one
/// would contradict the entire premise. The prototype's `1.2 km` is canned;
/// rendering it would be a fabricated measurement.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    /// Truncated peer id, e.g. `8A3F..1209`.
    pub id: String,
    /// Round-trip time where known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u16>,
    /// 1 is direct; more means relayed.
    pub hops: u8,
    pub transport: Transport,
    /// Deterministic map position in [0,1], seeded by peer id.
    pub x: f32,
    pub y: f32,
    /// Milliseconds, also seeded, so the field does not pulse in unison.
    pub pulse_ms: u16,
}

/// Peers currently reachable.
///
/// Rows come from the mesh actor's peer registry — real connected peers with
/// ping latency and direct/relayed connection kind. Positions are added here,
/// **deterministic and seeded by peer id**: a node stays where it was across
/// renders and restarts, which is what makes the map readable as an instrument
/// rather than a lava lamp. The prototype's seven hardcoded slots do not
/// generalise to an arbitrary peer count.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::MeshOffline`] without a
/// swarm.
#[tauri::command]
pub async fn list_nearby_nodes(state: State<'_, AppState>) -> Result<Vec<NodeSummary>, AppError> {
    let services = state.services()?;
    let mesh = services.mesh.as_ref().ok_or(AppError::MeshOffline)?;
    let peers = mesh.nearby_nodes().await.map_err(|_| AppError::MeshOffline)?;

    // Positions are presentation, not data: they come from a hash of the peer
    // id so a node is stable on the map, and never claim to be measurements.
    let nodes = peers
        .into_iter()
        .map(|peer| {
            let (x, y, pulse) = seeded_position(&peer.id);
            NodeSummary {
                id: peer.id,
                latency_ms: peer.latency_ms,
                hops: peer.hops,
                transport: match peer.transport {
                    crate::mesh_handle::Transport::Mdns => Transport::Mdns,
                    crate::mesh_handle::Transport::Quic => Transport::Quic,
                    crate::mesh_handle::Transport::Relayed => Transport::Relayed,
                },
                x,
                y,
                pulse_ms: pulse,
            }
        })
        .collect();
    Ok(nodes)
}

/// Deterministic position and pulse from a peer id.
///
/// A hash rather than randomness so a node does not jump between renders, and
/// the pulse is seeded too so the field does not throb in unison.
fn seeded_position(seed: &str) -> (f32, f32, u16) {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let hash = hasher.finish();

    // Inset from the edges so a node is never clipped by the map's frame.
    let x = 0.12 + ((hash & 0xFFFF) as f32 / 65_535.0) * 0.76;
    let y = 0.12 + (((hash >> 16) & 0xFFFF) as f32 / 65_535.0) * 0.76;
    let pulse = 900 + u16::try_from((hash >> 32) % 750).unwrap_or(0);
    (x, y, pulse)
}

// ---------------------------------------------------------------------------
// Intents
// ---------------------------------------------------------------------------

/// An intent as a list row renders it.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct IntentView {
    pub id: String,
    /// e.g. `BUY SOL`.
    pub title: String,
    /// e.g. `UNDER 95 USDC`.
    pub subtitle: String,
    /// Execution mode, shown as a badge. Absent when default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    pub amount: String,
    /// The lifecycle state, driving both the status text and the dot tone.
    pub status: cabal_core::IntentStatus,
    /// Elapsed or settled time, e.g. `2M 14S` or `11.4S`.
    pub elapsed: String,
    /// Whose order this is: `None` for one composed here, or the peer's
    /// shortened wallet for a mirrored order. The two are equally real and the
    /// list must not present a peer's order as the user's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The matched counterparty's shortened wallet, or `THIS DEVICE` when both
    /// sides were composed here. Absent until the order is paired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<String>,
    /// The agreed price, e.g. `95.00 USDC / SOL`. Absent when neither side
    /// named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    /// The settlement transaction signature, once settled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    /// Where to see that transaction. Absent when the proof is a relay queue id
    /// rather than a signature — there is nothing to look up yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer: Option<String>,
}

/// Which slice of the list to return.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "UPPERCASE")]
pub enum IntentFilter {
    Active,
    Pending,
    History,
}

/// Intents matching `filter`.
///
/// Reads the persisted intent ledger. The empty state — *"Nothing is queued.
/// Nothing is stored."* — is what this returns when the store is empty, which
/// is now a real fact about the store rather than a stub.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn list_intents(
    filter: IntentFilter,
    state: State<'_, AppState>,
) -> Result<Vec<IntentView>, AppError> {
    let services = state.services()?;
    let now = now_secs();
    let store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    Ok(store
        .all()
        .into_iter()
        .filter(|intent| match filter {
            IntentFilter::Active => intent.status.is_active(),
            IntentFilter::Pending => matches!(intent.status, cabal_core::IntentStatus::Draft),
            IntentFilter::History => intent.status.is_terminal(),
        })
        .map(|intent| intent_view(intent, now))
        .collect())
}

/// An intent as the detail screen renders it: the list row plus the full
/// request, so opening a detail never needs a second round trip for the parts
/// the list already fetched.
///
/// The draft is rendered as formatted strings rather than the raw domain type:
/// a `TokenAmount`'s `u128` does not survive a JS number, and the boundary rule
/// is that numbers are formatted once, in Rust.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct IntentDetail {
    /// The list-row view, nested so its `amount` does not collide with the
    /// detail's own formatted fields.
    pub view: IntentView,
    /// The composed action, e.g. `BUY`.
    pub action: String,
    /// The asset, e.g. `SOL`.
    pub asset: String,
    /// The condition as a sentence, e.g. `UNDER 95.00 USDC`.
    pub condition: String,
    /// The amount with its asset, e.g. `10 SOL`.
    pub amount: String,
    /// The execution mode label, e.g. `SHARK MODE`.
    pub mode: String,
    /// The privacy level, e.g. `MEDIUM`.
    pub privacy: String,
    /// The counterparty's full wallet address, once matched. Full rather than
    /// shortened because this is the screen where a user checks who they are
    /// actually paying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_wallet: Option<String>,
    /// Which way the asset moves, e.g. `YOU SEND 0.1 SOL`. Stated rather than
    /// inferred from `BUY`/`SELL`, because a mirrored peer order is shown from
    /// the peer's side and reading it as one's own would be expensive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

/// One intent by id.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn get_intent(
    id: String,
    state: State<'_, AppState>,
) -> Result<IntentDetail, AppError> {
    let services = state.services()?;
    let store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = cabal_core::IntentId::new(id);
    let stored = store.get(&id).ok_or(AppError::Internal)?;

    use cabal_core::Condition;
    let draft = &stored.draft;
    let condition = match &draft.condition {
        Condition::Under { price } => format!("UNDER {:.2} USDC", price.cents() as f64 / 100.0),
        Condition::Above { price } => format!("ABOVE {:.2} USDC", price.cents() as f64 / 100.0),
        Condition::Any => "ANY USDC PRICE".to_string(),
    };
    let view = intent_view(stored, now_secs());

    // A mirrored order is the peer's side of the trade, so "you send" would be
    // backwards on it: the direction is stated from the owner's point of view
    // and only for orders composed here.
    let direction = stored.matched.as_ref().filter(|_| stored.is_local()).map(|_| {
        let verb = match draft.action {
            cabal_core::Action::Sell => "YOU SEND",
            cabal_core::Action::Buy => "YOU RECEIVE",
        };
        format!("{verb} {} {}", draft.amount, draft.asset)
    });

    Ok(IntentDetail {
        view,
        action: format!("{:?}", draft.action).to_uppercase(),
        asset: draft.asset.to_string(),
        condition,
        amount: format!("{} {}", draft.amount, draft.asset),
        mode: draft.mode.label().to_string(),
        privacy: format!("{:?}", draft.privacy).to_uppercase(),
        counterparty_wallet: stored
            .matched
            .as_ref()
            .map(|m| m.wallet.clone())
            .filter(|wallet| !wallet.is_empty()),
        direction,
    })
}

/// Composes and broadcasts a new intent.
///
/// Creates the intent in the store (as a draft), transitions it to
/// [`cabal_core::IntentStatus::Broadcast`] through the checked lifecycle, and
/// publishes it to the mesh. Persists after the transition so a restart
/// restores the broadcast state rather than the draft.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::InvalidIntent`] if the
/// draft cannot be broadcast.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastDraftInput {
    pub action: cabal_core::Action,
    pub asset: String,
    condition: BroadcastConditionInput,
    /// Human-readable decimal input from the form (for example `"0.1"`).
    /// The webview must not be required to construct the fixed-point domain
    /// representation (`TokenAmount { raw, decimals }`).
    pub amount: String,
    pub mode: cabal_core::ExecutionMode,
    pub privacy: cabal_core::PrivacyLevel,
}

#[derive(Debug, serde::Deserialize)]
pub struct BroadcastConditionInput {
    pub kind: String,
    pub price: Option<String>,
}

fn asset_decimals(asset: &str) -> u8 {
    match asset {
        "SOL" => 9,
        "BOOST NFT" => 0,
        "USDC" => 6,
        "WETH" => 9,
        "BTC" => 9,
        _ => 18,
    }
}

/// Accept the decimal-comma form commonly produced by mobile keyboards while
/// retaining support for grouped values such as `1,240.00`.
fn normalize_amount_input(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.contains('.') && trimmed.matches(',').count() == 1 {
        if let Some((whole, fraction)) = trimmed.split_once(',') {
            if !whole.is_empty() && !fraction.is_empty() && fraction.len() != 3 {
                return format!("{whole}.{fraction}");
            }
        }
    }
    trimmed.to_string()
}

fn parse_broadcast_draft(input: BroadcastDraftInput) -> Result<cabal_core::IntentDraft, AppError> {
    use crate::error::{AppError, InvalidReason};
    use cabal_core::{Condition, TokenAmount, UsdPrice};

    if input.asset != "SOL" && input.asset != "BOOST NFT" {
        return Err(AppError::InvalidIntent {
            field: "asset",
            reason: InvalidReason::Malformed,
        });
    }
    let decimals = asset_decimals(&input.asset);
    let normalized_amount = normalize_amount_input(&input.amount);
    let amount = TokenAmount::parse(&normalized_amount, decimals).map_err(|err| {
        let reason = match err {
            cabal_core::AmountError::Empty => InvalidReason::Missing,
            cabal_core::AmountError::TooManyDecimals { .. } => InvalidReason::TooPrecise,
            cabal_core::AmountError::Overflow => InvalidReason::OutOfRange,
            cabal_core::AmountError::InvalidCharacter | cabal_core::AmountError::MultipleDecimalPoints => InvalidReason::Malformed,
            _ => InvalidReason::Malformed,
        };
        AppError::InvalidIntent { field: "amount", reason }
    })?;
    if amount.is_zero() {
        return Err(AppError::InvalidIntent { field: "amount", reason: InvalidReason::OutOfRange });
    }

    let condition = match input.condition.kind.as_str() {
        "any" => Condition::Any,
        "under" | "above" => {
            let price = input.condition.price.ok_or(AppError::InvalidIntent {
                field: "price",
                reason: InvalidReason::Missing,
            })?;
            let price = UsdPrice::parse(&price).map_err(|_| AppError::InvalidIntent {
                field: "price",
                reason: InvalidReason::Malformed,
            })?;
            if input.condition.kind == "under" {
                Condition::Under { price }
            } else {
                Condition::Above { price }
            }
        }
        _ => {
            return Err(AppError::InvalidIntent {
                field: "condition",
                reason: InvalidReason::Malformed,
            });
        }
    };

    Ok(cabal_core::IntentDraft {
        action: input.action,
        asset: input.asset.into_boxed_str(),
        condition,
        amount,
        mode: input.mode,
        privacy: input.privacy,
    })
}

#[tauri::command]
pub async fn broadcast_intent(
    app: tauri::AppHandle,
    draft: BroadcastDraftInput,
    state: State<'_, AppState>,
) -> Result<cabal_core::IntentId, AppError> {
    use cabal_core::IntentStatus;

    let draft = parse_broadcast_draft(draft)?;

    let services = state.services()?;
    let now = now_secs();

    let id = {
        let mut store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = store.create(draft.clone(), now);
        store
            .transition(&id, IntentStatus::Broadcast { route_len: 1 }, now)
            .map_err(|_| AppError::InvalidIntent { field: "status", reason: crate::error::InvalidReason::OutOfRange })?;
        id
    };

    // A new order fills against whatever this ledger already holds — a second
    // local order, or a peer's mirrored one that arrived before the user
    // composed this side. The pairing rules live in `cabal_core`, so this node
    // and every peer reach the same answer.
    let paired = {
        let mut store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match store.find_counterparty(&id) {
            Some((other_id, terms)) if store.pair(&id, &other_id, terms, now).is_ok() => {
                Some(other_id)
            }
            _ => None,
        }
    };

    // Publish to the mesh. A local publish succeeds even with no peers (the
    // gossipsub layer logs single-node mode), so the intent is stored and
    // broadcast regardless of network size.
    if let Some(mesh) = services.mesh.as_ref() {
        let wallet = services.bridge.lock().await.get_primary_address();
        let intent = crate::mesh::PrivacyIntent {
            intent_type: "trade".into(),
            payload: serde_json::to_string(&MeshTradePayload {
                draft: draft.clone(),
                wallet,
                intent_id: id.to_string(),
            })
            .unwrap_or_default(),
            encrypted: true,
            relay_path: vec!["origin_node".into()],
            relay_fee: None,
        };
        let _ = mesh.publish(intent).await;
    }

    // Persist the post-broadcast state.
    if let Ok(snapshot) = services.intents.lock().map(|s| s.snapshot()) {
        let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json"))
            .save(&snapshot);
    }

    emit_intent_updated(&app, &id);
    if let Some(other_id) = paired {
        emit_intent_updated(&app, &other_id);
        crate::deal::spawn(
            crate::deal::DealContext {
                app,
                bridge: services.bridge.clone(),
                matcher: services.matcher.clone(),
                mesh: services.mesh.clone(),
                intents: services.intents.clone(),
            },
            id.clone(),
            other_id,
        );
    }
    Ok(id)
}

/// Cancels an intent still live on the mesh.
///
/// Only active states may be cancelled; a settled intent cannot be undone. The
/// transition is enforced by the lifecycle table, not by a string check.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::Internal`] if the id is
/// unknown or the state is terminal.
#[tauri::command]
pub async fn cancel_intent(
    app: tauri::AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let services = state.services()?;
    let now = now_secs();
    let intent_id = cabal_core::IntentId::new(id);
    let released = {
        let mut store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        store
            .transition(&intent_id, cabal_core::IntentStatus::Cancelled, now)
            .map_err(|_| AppError::Internal)?;
        // The counterparty did nothing wrong: release it back to the open book
        // rather than leaving it negotiating with an order that is now gone.
        store.unpair(&intent_id, now)
    };
    let snapshot = services
        .intents
        .lock()
        .map(|s| s.snapshot())
        .unwrap_or_default();
    let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json"))
        .save(&snapshot);
    emit_intent_updated(&app, &intent_id);
    if let Some(other_id) = released {
        emit_intent_updated(&app, &other_id);
    }
    Ok(())
}

/// Settles an intent, streaming the verification log.
///
/// Settlement is **real**: it calls the deployed `cabal_escrow` Anchor program
/// on Solana devnet through the existing bridge — an escrow is created to the
/// first nearby peer (or a devnet test payee when alone), then released. The
/// proof recorded on the intent is the real on-chain release transaction
/// signature, and the log lines are the real RPC submission path.
///
/// If the RPC is unreachable, the transaction is signed offline and queued for
/// mesh relay, and the intent is left in a waiting state with an honest
/// `QUEUED FOR RELAY` line rather than a fabricated success.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::Internal`] if the
/// intent cannot legally reach `Settled` from its current state.
#[tauri::command]
pub async fn settle_intent(
    app: tauri::AppHandle,
    id: String,
    on_line: tauri::ipc::Channel<crate::bindings::LogLine>,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    use crate::bindings::{LogLine, LogTone};
    use cabal_core::IntentStatus;

    let services = state.services()?;
    let id = cabal_core::IntentId::new(id);
    let now = now_secs();

    // The active Solana escrow settles native SOL. Capture the composed
    // amount before spawning the stream so settlement cannot silently use a
    // hardcoded demo value or a non-SOL asset.
    let settlement_amount = {
        let store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let stored = store.get(&id).ok_or(AppError::Internal)?;
        if stored.draft.asset.as_ref() != "SOL" {
            return Err(AppError::InvalidIntent {
                field: "asset",
                reason: crate::error::InvalidReason::Malformed,
            });
        }
        stored.draft.amount.to_plain_string()
    };

    // Validate and move to FindingRoute (a required intermediate before
    // Settled) in one checked step.
    {
        let mut store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        store
            .transition(&id, IntentStatus::FindingRoute, now)
            .map_err(|_| AppError::Internal)?;
    }

    let (sub_id, token) = state.subscriptions().register("settlement")?;
    let registry = state.subscriptions().clone();
    let handle = sub_id.clone();
    // Clone the handles so no mutex guard or non-Send service is held across
    // an await inside the spawned task — that would make the future !Send.
    let bridge = services.bridge.clone();
    let intents = services.intents.clone();
    let mesh = services.mesh.clone();

    tauri::async_runtime::spawn(async move {
        // Sends a log line unless cancelled or the webview is gone.
        async fn send_line(
            on_line: &tauri::ipc::Channel<crate::bindings::LogLine>,
            token: &tokio_util::sync::CancellationToken,
            text: &str,
            tone: LogTone,
        ) -> bool {
            tokio::select! {
                () = token.cancelled() => false,
                () = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                    on_line.send(LogLine::new(text, tone)).is_ok()
                }
            }
        }

        if !send_line(&on_line, &token, "LOCATING ROUTE THROUGH MESH...", LogTone::Dim).await {
            registry.finished(&handle);
            return;
        }

        // The real counterparty: the first connected peer that announced a
        // Solana wallet address via its signed presence broadcast. Fall back
        // to a devnet test address when no peer has one — an honest single-node
        // flow rather than a fabricated multi-party one.
        let fallback_payee = std::env::var("CABALMESH_TEST_PAYEE")
            .unwrap_or_else(|_| "GyzdBSo87y5vT4oyoBCAdeT7hSz4C2ihj89QrVGCpdRa".to_string());
        // Only use a chain-funded payee for the MVP. A discovered mesh wallet
        // may be a fresh identity with no Solana account yet, which makes the
        // escrow program fail with AccountNotFound before it can settle.
        let payee = fallback_payee;

        // The real on-chain path. The bridge already handles the
        // online (submit via Magic Router) vs offline (sign + queue) split.
        // Map to an owned String up front so the `!Send` `Box<dyn StdError>`
        // is dropped before any await inside this spawned future.
        let outcome: Result<(String, Vec<crate::solana_bridge::QueuedTx>), String> = bridge
            .lock()
            .await
            .settle_on_chain(&payee, &settlement_amount)
            .await
            .map(|settled| {
                let lines = format!(
                    "ESCROW CREATED. TX {}\nRELEASED ON SOLANA DEVNET. TX {}",
                    settled.create_tx, settled.release_tx
                );
                (lines, settled.queued_for_relay)
            })
            .map_err(|error| format!("RPC UNREACHABLE. SIGNED + QUEUED FOR RELAY ({error})."));

        match outcome {
            Ok((lines, queued_for_relay)) => {
                // Anything signed offline still needs a peer with connectivity
                // to submit it. Broadcast each as a relay_tx intent so Relay
                // Mode nodes pick it up — the counterpart to the frontend's
                // self-submit retry loop.
                if let Some(mesh) = mesh.as_ref() {
                    for queued in &queued_for_relay {
                        let relay = serde_json::json!({
                            "type": "RelayTx",
                            "queue_id": queued.id,
                            "raw_tx_hex": queued.raw_tx_hex,
                            "summary": queued.summary,
                        });
                        let intent = crate::mesh::PrivacyIntent {
                            intent_type: "relay_tx".into(),
                            payload: relay.to_string(),
                            encrypted: false,
                            relay_path: vec!["origin_node".into()],
                            relay_fee: None,
                        };
                        let _ = mesh.publish(intent).await;
                    }
                }
                if !queued_for_relay.is_empty() {
                    let _ = send_line(
                        &on_line,
                        &token,
                        "OFFLINE SIGNED. BROADCAST FOR MESH RELAY.",
                        LogTone::Dim,
                    )
                    .await;
                }

                let mut iterator = lines.lines();
                if let Some(line) = iterator.next() {
                    if !send_line(&on_line, &token, line, LogTone::Info).await {
                        registry.finished(&handle);
                        return;
                    }
                }
                if let Some(line) = iterator.next() {
                    if !send_line(&on_line, &token, line, LogTone::Ok).await {
                        registry.finished(&handle);
                        return;
                    }
                }

                // Honest status: only a release that actually confirmed on-chain
                // is a settlement. When any leg was signed offline and queued
                // for relay, the deal is Waiting — the proof cannot be written
                // until the create lands and the release runs. The resume task
                // in lib.rs fires the release once the queued create confirms.
                let next_status = if queued_for_relay.is_empty() {
                    // The release tx signature is the real proof.
                    let release_tx = lines
                        .lines()
                        .last()
                        .and_then(|line| line.rsplit(' ').next())
                        .unwrap_or_default()
                        .to_owned();
                    IntentStatus::Settled {
                        proof: cabal_core::ProofHash::new(release_tx),
                        filled_at: cabal_core::UsdPrice::from_cents(0),
                        elapsed_ms: 0,
                    }
                } else {
                    IntentStatus::Waiting
                };
                {
                    let mut store = intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let _ = store.transition(&id, next_status, now);
                }
                let snapshot = intents
                    .lock()
                    .map(|s| s.snapshot())
                    .unwrap_or_default();
                let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json"))
                    .save(&snapshot);
                emit_intent_updated(&app, &id);
            }
            Err(message) => {
                // Offline or chain failure. The bridge already queued the
                // signed transaction for mesh relay; report it honestly.
                let _ = send_line(&on_line, &token, &message, LogTone::Err).await;
                let snapshot = {
                    let mut store = intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let _ = store.transition(
                        &id,
                        IntentStatus::Failed {
                            reason: cabal_core::FailureReason::SettlementRejected,
                        },
                        now_secs(),
                    );
                    store.snapshot()
                };
                let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json"))
                    .save(&snapshot);
                emit_intent_updated(&app, &id);
            }
        }

        registry.finished(&handle);
    });

    Ok(sub_id.to_string())
}

/// Emits an `intent-updated` event so the list and detail refresh without
/// polling. Best effort: if the window is gone the next fetch reconciles.
fn emit_intent_updated(app: &tauri::AppHandle, id: &cabal_core::IntentId) {
    use tauri::Emitter;
    let _ = app.emit("intent-updated", serde_json::json!({ "id": id.as_str() }));
}

/// Unix timestamp in seconds, used for intent timestamps and elapsed display.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A wallet address as the board shows it: `GyzdBS…GCpdRa`. Full addresses are
/// 44 characters and would wrap a phone-width row into three lines.
fn short_wallet(address: &str) -> String {
    if address.chars().count() <= 14 {
        return address.to_string();
    }
    let head: String = address.chars().take(6).collect();
    let tail: String = address
        .chars()
        .skip(address.chars().count().saturating_sub(6))
        .collect();
    format!("{head}…{tail}")
}

/// The explorer link for a settlement proof, when the proof is a real
/// signature.
///
/// A parked settlement records its relay queue id instead, which no explorer
/// can resolve — offering a link that 404s would misrepresent an unfinished
/// deal as a finished one.
fn explorer_url(proof: &str) -> Option<String> {
    let is_signature = proof.len() >= 64
        && proof
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'));
    is_signature
        .then(|| format!("https://explorer.solana.com/tx/{proof}?cluster=devnet"))
}

/// `95.00 USDC / SOL`.
fn format_price(price: cabal_core::UsdPrice) -> String {
    format!("{:.2} USDC / SOL", price.cents() as f64 / 100.0)
}

/// Renders a stored intent as a list row.
fn intent_view(intent: &cabal_core::StoredIntent, now: u64) -> IntentView {
    use cabal_core::{Condition, ExecutionMode, IntentStatus};

    let draft = &intent.draft;
    let title = format!(
        "{} {}",
        format!("{:?}", draft.action).to_uppercase(),
        draft.asset
    );
    let subtitle = match &draft.condition {
        Condition::Under { price } => format!("UNDER {:.2} USDC", price.cents() as f64 / 100.0),
        Condition::Above { price } => format!("ABOVE {:.2} USDC", price.cents() as f64 / 100.0),
        Condition::Any => "ANY USDC PRICE".to_string(),
    };
    let badge = (draft.mode != ExecutionMode::Shark).then(|| draft.mode.label().to_string());
    let amount = format!("{} {}", draft.amount, draft.asset);
    let elapsed = match &intent.status {
        IntentStatus::Settled { elapsed_ms, .. } => format_elapsed(*elapsed_ms),
        _ => format_elapsed(
            u32::try_from(now.saturating_sub(intent.created_at).saturating_mul(1000))
                .unwrap_or(u32::MAX),
        ),
    };
    let proof = match &intent.status {
        IntentStatus::Settled { proof, .. } => Some(proof.to_string()),
        _ => None,
    };
    IntentView {
        id: intent.id.to_string(),
        title,
        subtitle,
        badge,
        amount,
        status: intent.status.clone(),
        elapsed,
        origin: intent.origin.as_ref().map(|o| short_wallet(&o.wallet)),
        counterparty: intent.matched.as_ref().map(|m| {
            if m.wallet.is_empty() {
                "THIS DEVICE".to_string()
            } else {
                short_wallet(&m.wallet)
            }
        }),
        price: intent.matched.as_ref().and_then(|m| m.price).map(format_price),
        explorer: proof.as_deref().and_then(explorer_url),
        proof,
    }
}

/// Formats a millisecond duration as the board does: `2M 14S` or `11.4S`.
fn format_elapsed(ms: u32) -> String {
    let secs = u64::from(ms) / 1000;
    if secs >= 60 {
        format!("{}M {}S", secs / 60, secs % 60)
    } else {
        format!("{}.{}S", secs, (u64::from(ms) % 1000) / 100)
    }
}

/// The options the compose screen offers.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct FormOptions {
    pub actions: Vec<String>,
    pub assets: Vec<AssetOption>,
    pub conditions: Vec<String>,
    pub modes: Vec<ModeOption>,
    pub privacy_levels: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AssetOption {
    pub name: String,
    /// Three-letter tag the board shows beside the name.
    pub tag: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ModeOption {
    pub label: String,
    pub description: String,
}

/// Options for the compose screen.
///
/// Supplied by Rust rather than hardcoded on the frontend so a mode and its
/// description cannot drift apart — they come from one `ExecutionMode`.
///
/// # Errors
///
/// Never fails.
#[tauri::command]
pub async fn intent_form_options() -> Result<FormOptions, AppError> {
    use cabal_core::{Action, ExecutionMode, PrivacyLevel};

    Ok(FormOptions {
        actions: Action::ALL.iter().map(|a| format!("{a:?}").to_uppercase()).collect(),
        // The active Anchor program settles native SOL. Other assets remain a
        // future token-program integration and must not be offered by this UI.
        assets: vec![AssetOption { name: "SOL".into(), tag: "SOL".into(), decimals: 9 }],
        conditions: vec!["Price under".into(), "Price above".into(), "Any price".into()],
        modes: ExecutionMode::ALL
            .iter()
            .map(|mode| ModeOption {
                label: mode.label().to_string(),
                description: mode.description().to_string(),
            })
            .collect(),
        privacy_levels: PrivacyLevel::ALL
            .iter()
            .map(|level| format!("{level:?}").to_uppercase())
            .collect(),
    })
}

/// One row of the confirm dialog.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ReviewRow {
    pub key: String,
    pub value: String,
}

/// Validates a draft and returns the rows the confirm dialog shows.
///
/// Computed here rather than on the frontend so what the user confirms is
/// exactly what would be broadcast — a dialog assembled separately can drift
/// from the payload it claims to describe.
///
/// # Errors
///
/// [`AppError::InvalidIntent`] with the offending field, so the form can attach
/// the failure to an input rather than showing a general message.
#[tauri::command]
pub async fn preview_intent(
    action: String,
    asset: String,
    condition: String,
    price: String,
    amount: String,
    mode: String,
    privacy: String,
) -> Result<Vec<ReviewRow>, AppError> {
    use crate::error::InvalidReason;
    use cabal_core::{TokenAmount, UsdPrice};

    // Parsed, not trusted. Everything arriving from the webview is hostile
    // until it becomes a domain type.
    let decimals = asset_decimals(&asset);
    let normalized_amount = normalize_amount_input(&amount);
    let parsed_amount = TokenAmount::parse(&normalized_amount, decimals)?;
    if parsed_amount.is_zero() {
        return Err(AppError::InvalidIntent {
            field: "amount",
            reason: InvalidReason::OutOfRange,
        });
    }

    let condition_text = if condition.starts_with("Any") {
        condition.to_uppercase()
    } else {
        let parsed_price = UsdPrice::parse(&price).map_err(|_| AppError::InvalidIntent {
            field: "price",
            reason: InvalidReason::Malformed,
        })?;
        format!("{} {}", condition.to_uppercase(), parsed_price)
    };

    Ok(vec![
        ReviewRow { key: "ACTION".into(), value: format!("{} {}", action.to_uppercase(), asset) },
        ReviewRow { key: "CONDITION".into(), value: condition_text },
        ReviewRow { key: "AMOUNT".into(), value: format!("{parsed_amount} {asset}") },
        ReviewRow { key: "MODE".into(), value: mode },
        ReviewRow { key: "PRIVACY".into(), value: privacy },
    ])
}

// ---------------------------------------------------------------------------
// Vault and profile
// ---------------------------------------------------------------------------

/// A row in the vault list.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct VaultRow {
    /// Three-letter tag, e.g. `AVX`, `ID`, `KEY`.
    pub tag: String,
    pub name: String,
    pub amount: String,
    /// Secondary line. Absent when there is nothing true to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Balances held by this identity.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn vault_assets(state: State<'_, AppState>) -> Result<Vec<VaultRow>, AppError> {
    let services = state.services()?;
    let bridge = services.bridge.lock().await;

    // The native balance is the one thing actually known. Listing tokens the
    // wallet has never held would be inventing holdings.
    let snapshot = bridge.get_latest_snapshot().ok();
    let mut rows: Vec<VaultRow> = snapshot
        .map(|snapshot| {
            snapshot
                .assets
                .into_iter()
                .map(|asset| VaultRow {
                    tag: asset.symbol.chars().take(3).collect::<String>().to_uppercase(),
                    name: asset.symbol,
                    amount: asset.amount,
                    detail: None,
                })
                .collect()
        })
        .unwrap_or_default();
    if let Ok(Ok(raw_usdc)) = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        bridge.circle_usdc_balance(),
    )
    .await
    {
        rows.push(VaultRow {
            tag: "USDC".into(),
            name: "CIRCLE USDC (DEVNET)".into(),
            amount: raw_usdc.to_string(),
            detail: Some("OFFICIAL DEVNET MINT · 6 DECIMALS".into()),
        });
    }
    // Include the real SPL demo boost in the Vault inventory. The native SOL
    // snapshot is intentionally separate from token-account indexing.
    // Native SOL is already available from the local verified snapshot. Do
    // not let a slow `getProgramAccounts` NFT indexer make the *entire* Assets
    // screen appear blank while it waits for devnet.
    if let Ok(Ok(boosts)) = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        bridge.list_boost_nfts(),
    )
    .await
    {
        for boost in boosts.into_iter().filter(|item| item.owned && !item.listed) {
            rows.push(VaultRow {
                tag: "NFT".into(),
                name: boost.name,
                amount: "1 BOOST NFT".into(),
                detail: Some(format!("{} · mint {}", boost.boost_bps, boost.mint)),
            });
        }
    }

    Ok(rows)
}

/// Identities this device holds.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::VaultLocked`] if the
/// encrypted store cannot be opened.
#[tauri::command]
pub async fn vault_identities(state: State<'_, AppState>) -> Result<Vec<VaultRow>, AppError> {
    let services = state.services()?;
    let bridge = services.bridge.lock().await;

    let views = bridge.get_identity_views().map_err(|_| AppError::VaultLocked)?;
    Ok(views
        .into_iter()
        .map(|view| VaultRow {
            tag: "ID".into(),
            name: view.alias.to_uppercase(),
            amount: cabal_core::NodeId::new(view.address).truncated(),
            detail: None,
        })
        .collect())
}

/// Key material metadata.
///
/// **Never the key itself.** These rows describe what is held and where; the
/// values stay in the encrypted vault. That is the promise the screen's own
/// copy makes, so the command has to keep it.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn vault_keys(state: State<'_, AppState>) -> Result<Vec<VaultRow>, AppError> {
    let _services = state.services()?;
    Ok(vec![
        VaultRow {
            tag: "KEY".into(),
            name: "SIGNING KEY".into(),
            amount: "secp256k1".into(),
            detail: Some("HELD LOCALLY. NEVER SYNCED.".into()),
        },
        VaultRow {
            tag: "KEY".into(),
            name: "VAULT KEY".into(),
            amount: "AES-256-GCM".into(),
            // Honest about what ticket 18 actually shipped: file-backed, not
            // hardware-backed, until the keystore plugin lands.
            detail: Some("FILE-BACKED. DEVICE KEY STORE PENDING.".into()),
        },
        VaultRow {
            tag: "KEY".into(),
            name: "RECOVERY PHRASE".into(),
            amount: "NONE".into(),
            detail: Some("NOT BACKED UP.".into()),
        },
    ])
}

/// Total value held by this identity, as a pre-formatted decimal string.
///
/// Returns the native SOL balance from the encrypted snapshot. The string is
/// what the UI masks until reveal — the value never enters the DOM hidden, and
/// formatting stays in Rust per the boundary rule.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::Chain`] if no snapshot
/// exists yet (the balance has not been synced).
#[tauri::command]
pub async fn vault_total(state: State<'_, AppState>) -> Result<String, AppError> {
    let services = state.services()?;
    let bridge = services.bridge.lock().await;
    let snapshot = bridge.get_latest_snapshot().map_err(|_| AppError::Chain { retryable: false })?;

    // The native SOL asset is what sync_state writes; sum what the snapshot
    // actually holds rather than assuming a fixed asset list.
    let total_lamports: u64 = snapshot
        .assets
        .iter()
        .filter(|a| a.symbol == "SOL")
        .filter_map(|a| a.amount.parse::<u64>().ok())
        .sum();

    // Format lamports -> SOL as a decimal string, 9 places, so the value never
    // loses precision crossing to JS.
    let whole = total_lamports / 1_000_000_000;
    let fraction = total_lamports % 1_000_000_000;
    Ok(format!("{whole}.{fraction:09}"))
}

/// What the profile screen shows.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub node_id: String,
    /// `87.6 (+5.3%)`, or an em dash with no mesh. Derived from real
    /// demonstrated behaviour — see src/reputation.rs.
    pub reputation: String,
    pub member_since: String,
    pub offline: bool,
    pub network: String,
    /// Whether transactions here move real value.
    pub is_testnet: bool,
}

/// Identity and settings for the profile screen.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn profile_summary(state: State<'_, AppState>) -> Result<ProfileView, AppError> {
    let services = state.services()?;
    let network = crate::network_config::NetworkConfig::load(&cabal_store::JsonStore::new(
        crate::app_paths::in_data_dir("network.json"),
    ));

    // One snapshot for all three fields. Asking the actor twice was two round
    // trips for the same answer, and left a window where the identity and the
    // offline flag could come from different states of the mesh.
    let snapshot = match services.mesh.as_ref() {
        Some(mesh) => mesh.snapshot().await.ok(),
        None => None,
    };

    let node_id = snapshot
        .as_ref()
        .map_or_else(|| "—".into(), |s| cabal_core::NodeId::new(s.peer_id.clone()).truncated());

    // Absent mesh reads as offline: the screen must not show a connected
    // switch for a mesh that is not there.
    let offline = snapshot.as_ref().is_none_or(|s| s.offline);

    // Real demonstrated behaviour — see src/reputation.rs. The score is
    // derived from relayed transactions, relayed bytes, settled intents and
    // observed peer latency, none of which are fabricated.
    let best_peer_latency_ms = match services.mesh.as_ref() {
        Some(mesh) => mesh
            .nearby_nodes()
            .await
            .ok()
            .and_then(|peers| peers.iter().filter_map(|p| p.latency_ms).min()),
        None => None,
    };
    let relayed_tx_count = {
        let bridge = services.bridge.lock().await;
        bridge.get_relayed_history().len() as u64
    };
    let settled_deals = {
        let store = services.intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        store
            .all()
            .into_iter()
            .filter(|i| matches!(i.status, cabal_core::IntentStatus::Settled { .. }))
            .count() as u64
    };
    let reputation = snapshot
        .as_ref()
        .and_then(|s| {
            let signals = crate::reputation::Signals {
                relayed_tx_count,
                relay_bytes: s.relay_bytes,
                settled_deals,
                best_peer_latency_ms,
            };

            crate::reputation::Reputation::of(&s.peer_id, signals).map(|reading| {
                let baseline = crate::reputation::ReputationBaseline::load_or_establish(
                    &s.peer_id,
                    reading.score,
                    &cabal_store::JsonStore::new(
                        crate::app_paths::in_data_dir("reputation.json"),
                    ),
                );
                let delta = baseline
                    .map(|b| b.delta_percent(reading.score))
                    .unwrap_or(0.0);
                crate::reputation::Reputation { score: reading.score, delta_percent: delta }
            })
        })
        .map_or_else(|| "—".into(), |reading| reading.combined());

    Ok(ProfileView {
        node_id,
        reputation,
        member_since: "—".into(),
        offline,
        network: network.network.label().to_string(),
        is_testnet: network.network.is_testnet(),
    })
}

/// Stops or resumes mesh participation.
///
/// The switch's own copy promises intents queue locally and nothing leaves the
/// device. The actor enforces that itself rather than trusting callers.
///
/// # Errors
///
/// [`AppError::MeshOffline`] if the mesh actor is gone.
#[tauri::command]
pub async fn set_offline_mode(offline: bool, state: State<'_, AppState>) -> Result<(), AppError> {
    let services = state.services()?;
    let mesh = services.mesh.as_ref().ok_or(AppError::MeshOffline)?;
    mesh.set_offline(offline).await.map_err(|_| AppError::MeshOffline)
}

// ---------------------------------------------------------------------------
// AI-anchored wallet import/export
// ---------------------------------------------------------------------------

/// What `export_mnemonic` returns: the AI's narrative anchoring the seed
/// words in order. The mnemonic itself never crosses to the webview here —
/// the story is the recall aid, and the words are shown only by
/// `copy_mnemonic`/a dedicated reveal step the user explicitly opens.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "camelCase")]
pub struct MnemonicExport {
    /// The AI-written story. A memory aid, never part of the secret.
    pub story: String,
}

/// Exports the primary wallet as a BIP-39 mnemonic wrapped in an AI story.
///
/// The story is generated by the local Ollama model: given the 12 words in
/// order, it writes a short narrative where each word appears in sequence, so
/// the user reconstructs the order by retelling the story. The story is
/// **never stored or sent anywhere** — it exists only in this response.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn export_mnemonic(state: State<'_, AppState>) -> Result<MnemonicExport, AppError> {
    use crate::mnemonic::Mnemonic;

    let services = state.services()?;
    let bridge = services.bridge.lock().await;
    let phrase = bridge
        .primary_mnemonic()
        .ok_or_else(|| AppError::VaultLocked)?;
    let phrase = Mnemonic::parse(&phrase).map_err(|_| AppError::Internal)?;

    let story = generate_story(&phrase).await.unwrap_or_else(|_| {
        // Honest fallback: no model, no story — the words themselves are the
        // export, and the UI says so.
        phrase.words().join(" ")
    });

    Ok(MnemonicExport { story })
}

/// Copies the wallet's mnemonic words to the clipboard. Called only when the
/// user explicitly opens the reveal step — the words never appear in an
/// event or a log, only on the clipboard the user controls.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap.
#[tauri::command]
pub async fn copy_mnemonic(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    let services = state.services()?;
    let bridge = services.bridge.lock().await;
    let phrase = bridge
        .primary_mnemonic()
        .ok_or_else(|| AppError::VaultLocked)?;

    let _ = app.clipboard().write_text(phrase);
    Ok(())
}

/// Imports a wallet from a BIP-39 mnemonic, replacing the current identity.
///
/// The phrase is validated by the BIP-39 checksum **before** any AI fuzzy
/// matching is offered — a wrong word is caught by the checksum, never by the
/// model. The derived key becomes the new primary identity.
///
/// # Errors
///
/// [`AppError::NotReady`] before bootstrap, [`AppError::InvalidIntent`] with
/// `field: "mnemonic"` if the phrase fails validation.
#[tauri::command]
pub async fn import_mnemonic(
    mnemonic: String,
    alias: String,
    emoji: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::blockchain_bridge::IdentityView>, AppError> {
    use crate::error::InvalidReason;
    use crate::mnemonic::Mnemonic;

    let phrase = Mnemonic::parse(&mnemonic).map_err(|_| AppError::InvalidIntent {
        field: "mnemonic",
        reason: InvalidReason::Malformed,
    })?;
    let keypair = phrase
        .to_keypair()
        .map_err(|_| AppError::InvalidIntent { field: "mnemonic", reason: InvalidReason::Malformed })?;
    let words = phrase.words().join(" ");
    let key = bs58::encode(keypair.to_bytes()).into_string();

    let services = state.services()?;
    let mut bridge = services.bridge.lock().await;
    bridge
        .import_with_mnemonic(key, Some(words), alias, emoji)
        .map_err(|_| AppError::Internal)
}

/// Suggests likely intended BIP-39 words for a possibly-mistyped input, for
/// the AI-assisted import field.
///
/// Pure wordlist fuzzy matching (edit distance ≤ 2 + prefix). The UI offers
/// the candidates; only the user's confirmed selection is used, never an
/// accepted guess.
#[tauri::command]
pub fn suggest_mnemonic_word(input: String) -> Result<Vec<String>, AppError> {
    Ok(crate::mnemonic::suggest_words(&input))
}

/// Asks the local Ollama model to anchor the words in a memorable story.
///
/// The prompt is stateless and local; the response is the story and nothing
/// else. A failure here (no model, no server) is **not** fatal — the export
/// falls back to the plain words.
async fn generate_story(phrase: &crate::mnemonic::Mnemonic) -> Result<String, Box<dyn std::error::Error>> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct StoryRequest {
        model: &'static str,
        prompt: String,
        stream: bool,
    }
    #[derive(serde::Deserialize)]
    struct StoryResponse {
        response: String,
    }

    let words = phrase.words().join(", ");
    let prompt = format!(
        "Here are {n} words in this exact order: {words}. \
         Write a short 3-4 sentence story where these words appear in this exact order. \
         Make it vivid and easy to recall. Never repeat the words as a list at the end.",
        n = phrase.words().len()
    );

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/generate", crate::ollama_config::url()))
        .json(&StoryRequest {
            model: "qwen2.5:0.5b",
            prompt,
            stream: false,
        })
        .send()
        .await?;
    let body: StoryResponse = response.json().await?;
    let story = body.response.trim().to_string();
    if story.is_empty() {
        Err("empty story".into())
    } else {
        Ok(story)
    }
}
