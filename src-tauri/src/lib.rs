mod app_initializer;
mod app_paths;
mod platform;
mod ollama_config;
mod ollama_manager;

// Public because their types *are* the IPC contract: everything below
// serializes across the boundary to the webview, so the shapes are already
// public API whether or not Rust says so. `tests/ipc_contract.rs` pins them.
pub mod agent;
pub mod blockchain_bridge;
pub mod matcher;
pub mod mesh;

/// Solana + MagicBlock chain backend. `blockchain_bridge` re-exports this so
/// the frozen desktop surface and its IPC snapshots stay source-compatible.
pub mod solana_bridge;

/// BIP-39 mnemonic export/import for the wallet.
pub mod mnemonic;

/// Request handle onto the mesh actor. See src/mesh_handle.rs.
pub mod mesh_handle;

/// Bootstrap peer configuration. See src/bootstrap_config.rs.
pub mod bootstrap_config;

/// Chain selection and contract addresses. See src/network_config.rs.
pub mod network_config;
pub mod zk_handler;
mod llm_json;
mod lifecycle;
mod telemetry;
mod vault_key;

/// The Android Wi-Fi multicast lock mDNS needs. See src/multicast.rs.
pub mod multicast;

/// Android's platform trust store, which rustls will not start without.
/// See src/tls.rs.
pub mod tls;

/// Reputation derived from real demonstrated behaviour, in one place.
/// See src/reputation.rs.
pub mod reputation;

/// Managed application state. See src/state.rs.
pub mod state;

/// Lifecycle for live frontend streams. See src/subscriptions.rs.
pub mod subscriptions;

/// The reshaped command surface. See src/commands.rs.
pub mod commands;

/// Matched pairs, from agreement to a settled trade. See src/deal.rs.
pub mod deal;

/// Carrying other peers' offline-signed transactions. See src/relay.rs.
pub mod relay;

/// Presentation contracts shared with the frontend. See src/bindings.rs.
pub mod bindings;

/// The typed error union that crosses the IPC boundary. See src/error.rs.
pub mod error;

/// The frozen desktop IPC surface. See `legacy/mod.rs` for why it exists and
/// what it is not allowed to become.
#[cfg(all(desktop, feature = "desktop-legacy"))]
pub mod legacy;


use app_initializer::SystemBootstrap;
use agent::SharkAgent;
use matcher::MatchAgent;
use zk_handler::ZKHandler;
use ollama_manager::OllamaManager;
use blockchain_bridge::BlockchainBridge;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{Manager, Emitter};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // First thing, before anything can emit: on a device this is the only
    // channel that reaches a developer, so nothing useful is logged until it
    // is installed.
    telemetry::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Registers the JNI handle only. Nothing here is reachable over IPC —
        // see src/multicast.rs for why the webview gets no grant for it.
        .plugin(multicast::init())
        // Registered before anything that can make an HTTPS request: on Android
        // rustls has no trust store until this runs. See src/tls.rs.
        .plugin(tls::init())
        .setup(|app| {
            // Synchronously, before the webview exists. Bootstrap fills the
            // services in afterwards; until then commands get NotReady rather
            // than a panic from an unmanaged type.
            // Before anything can persist: Tauri knows the correct directory
            // on every platform, and a mobile sandbox has no other right answer.
            match app.path().app_data_dir() {
                Ok(dir) => app_paths::set(dir),
                Err(error) => tracing::error!(
                    target: "cabalmesh::paths",
                    %error,
                    "platform gave no app data directory; falling back"
                ),
            }

            let state = state::AppState::new();
            app.manage(state.clone());

            let app_handle = app.handle().clone();

            // Create consistent Ollama instance
            let ollama_manager = Arc::new(OllamaManager::new(Some("qwen2.5:0.5b".to_string())));
            let ollama_init = ollama_manager.clone();
            
            // Initialize Ollama in background
            tauri::async_runtime::spawn(async move {
                let ollama = ollama_init;

                // Mobile cannot spawn a local server, so there is nothing to
                // install or start — just check whether the configured remote
                // is reachable.
                if !platform::CAN_SPAWN_PROCESSES {
                    let url = ollama_config::url();
                    tracing::info!("🔍 Checking remote Ollama at {}...", url);
                    if ollama.health_check().await {
                        tracing::info!("✅ Remote Ollama is healthy");
                    } else {
                        tracing::warn!("⚠️  No Ollama at {}", url);
                        tracing::warn!("📝 Set one with the set_ollama_url command or ${}", ollama_config::ENV_VAR);
                    }
                    return;
                }

                tracing::info!("🔍 Checking Ollama installation...");
                if !ollama.is_installed() {
                    tracing::warn!("⚠️  Ollama not found!");
                    tracing::warn!("📝 Please install from: https://ollama.ai");
                    tracing::warn!("   Or run: brew install ollama");
                } else {
                    match ollama.initialize().await {
                        Ok(_) => {
                            tracing::info!("✅ Ollama ready!");
                            for i in 1..=10 {
                                if ollama.health_check().await {
                                    tracing::info!("✅ Ollama service is healthy");
                                    break;
                                }
                                if i == 10 {
                                    tracing::warn!("⚠️  Ollama service not responding");
                                }
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            }
                        }
                        Err(e) => {
                            tracing::error!("❌ Failed to initialize Ollama: {}", e);
                        }
                    }
                }
            });

            // Pass strong reference to mesh setup to store in AppState
            let ollama_state = ollama_manager.clone();

            // Initialize System via Bootstrap Workflow
            tauri::async_runtime::spawn(async move {
                // Shared Bridge Resource (Created here first)
                // Desktop only: there is no .env file in a mobile bundle, and no
                // environment to read it into. Mobile falls through to the
                // compiled-in default until ticket 24 replaces this with a
                // per-network address table.
                #[cfg(desktop)]
                dotenv::dotenv().ok();

                let rpc_url = std::env::var("SOLANA_RPC_URL")
                    .or_else(|_| std::env::var("AVAX_RPC_URL"))
                    .unwrap_or_else(|_| blockchain_bridge::DEFAULT_SOLANA_RPC_URL.to_string());

                let bridge = Arc::new(Mutex::new(BlockchainBridge::new(Some(rpc_url))));

                // The intent ledger, restored from the last run. `load_or`
                // treats a corrupt file as absent — these are user-composed
                // intents, replaceable, not a wallet.
                let intents = Arc::new(std::sync::Mutex::new({
                    let store = cabal_store::JsonStore::new(
                        crate::app_paths::in_data_dir("intents.json"),
                    );
                    let mut intent_store = cabal_core::IntentStore::new();
                    intent_store.restore(store.load_or(intent_store.snapshot()));
                    intent_store
                }));

                // 1. Phase 1
                SystemBootstrap::phase_1_sync(&bridge, &app_handle).await;

                // 2. Phase 2
                SystemBootstrap::phase_2_delegate(&bridge, &app_handle).await;

                // 3. Phase 3 & Network Start
                match SystemBootstrap::phase_3_network(&app_handle).await {
                    Ok((mut mesh, mesh_handle, mut event_rx, command_rx, event_tx)) => {
                        tracing::info!("✅ System Bootstrap Complete. Mesh Swarm Active.");

                        let relay_bytes = mesh.relay_bytes.clone();

                        // Start Mesh Loop (Background)
                        tokio::spawn(async move {
                            if let Err(e) = mesh.start(event_tx, command_rx).await {
                                tracing::warn!("Mesh network error: {}", e);
                            }
                        });

                        // Forward Mesh Events to Frontend
                        let handle_clone = app_handle.clone();
                        let remote_intents = intents.clone();
                        let auto_settle_bridge = bridge.clone();
                        let auto_settle_matcher = Arc::new(MatchAgent::new(None));
                        let rebroadcast_mesh = mesh_handle.clone();
                        let local_intents = intents.clone();
                        let rebroadcast_bridge = bridge.clone();
                        let deal_mesh = mesh_handle.clone();
                        let relay_bridge = bridge.clone();
                        let relay_mesh_handle = mesh_handle.clone();
                        let replay_bridge = bridge.clone();
                        let replay_mesh = mesh_handle.clone();
                        let replay_intents = intents.clone();
                        tokio::spawn(async move {
                            let relay_context = || crate::relay::RelayContext {
                                app: handle_clone.clone(),
                                bridge: relay_bridge.clone(),
                                mesh: Some(relay_mesh_handle.clone()),
                            };
                            while let Some(event) = event_rx.recv().await {
                                if matches!(&event, crate::mesh::MeshEvent::PeerDiscovered { .. }) {
                                    // A settlement announced while this peer was away reached
                                    // nobody: gossipsub does not hold messages for absent
                                    // subscribers. Reconnection is the moment to say it again.
                                    crate::deal::replay_settlements(crate::deal::DealContext {
                                        app: handle_clone.clone(),
                                        bridge: replay_bridge.clone(),
                                        matcher: auto_settle_matcher.clone(),
                                        mesh: Some(replay_mesh.clone()),
                                        intents: replay_intents.clone(),
                                    });
                                }
                                if matches!(&event, crate::mesh::MeshEvent::PeerDiscovered { .. }) {
                                    // Intents composed before the other demo app joined must
                                    // still be visible to it. Wait briefly for the mDNS dial to
                                    // finish, then re-announce active drafts through the real
                                    // gossipsub transport.
                                    let mesh = rebroadcast_mesh.clone();
                                    let intents = local_intents.clone();
                                    let bridge = rebroadcast_bridge.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
                                        // Only this device's own still-unmatched orders: a
                                        // mirror belongs to the peer that sent it, and an
                                        // order already paired is spoken for.
                                        let orders = intents
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .all()
                                            .into_iter()
                                            .filter(|entry| entry.is_local() && entry.is_open())
                                            .map(|entry| (entry.id.to_string(), entry.draft.clone(), entry.boost_mint.as_deref().map(str::to_string)))
                                            .collect::<Vec<_>>();
                                        let wallet = bridge.lock().await.get_primary_address();
                                        for (intent_id, draft, boost_mint) in orders {
                                            let _ = mesh.publish(crate::mesh::PrivacyIntent {
                                                intent_type: "trade".into(),
                                                payload: serde_json::to_string(&crate::commands::MeshTradePayload { draft, wallet: wallet.clone(), intent_id, boost_mint }).unwrap_or_default(),
                                                encrypted: true,
                                                relay_path: vec!["origin_node".into()],
                                                relay_fee: None,
                                            }).await;
                                        }
                                    });
                                }
                                // Relaying a peer's offline-signed transaction, and hearing
                                // back about our own. See src/relay.rs.
                                match &event {
                                    crate::mesh::MeshEvent::RelayTxReceived { queue_id, raw_tx_hex, summary } => {
                                        crate::relay::on_relay_request(
                                            relay_context(),
                                            queue_id.clone(),
                                            raw_tx_hex.clone(),
                                            summary.clone(),
                                        );
                                    }
                                    crate::mesh::MeshEvent::RelayConfirmed { queue_id, status, tx_hash } => {
                                        crate::relay::on_relay_report(
                                            relay_context(),
                                            queue_id.clone(),
                                            status.clone(),
                                            tx_hash.clone(),
                                        );
                                        if status == "confirmed" {
                                            if let Some(signature) = tx_hash.clone() {
                                                crate::deal::on_boost_relay_confirmed(
                                                    crate::deal::DealContext {
                                                        app: handle_clone.clone(),
                                                        bridge: auto_settle_bridge.clone(),
                                                        matcher: auto_settle_matcher.clone(),
                                                        mesh: Some(deal_mesh.clone()),
                                                        intents: remote_intents.clone(),
                                                    },
                                                    queue_id.clone(),
                                                    signature,
                                                );
                                            }
                                        }
                                    }
                                    _ => {}
                                }

                                if let crate::mesh::MeshEvent::IntentReceived { intent } = &event {
                                    let context = || crate::deal::DealContext {
                                        app: handle_clone.clone(),
                                        bridge: auto_settle_bridge.clone(),
                                        matcher: auto_settle_matcher.clone(),
                                        mesh: Some(deal_mesh.clone()),
                                        intents: remote_intents.clone(),
                                    };

                                    // A counterparty announcing that it has paid. Verified against
                                    // the chain before it is believed — see src/deal.rs.
                                    if intent.intent_type == "trade_settled" {
                                        if let Ok(announcement) =
                                            serde_json::from_str::<crate::deal::TradeSettled>(&intent.payload)
                                        {
                                            crate::deal::on_trade_settled(context(), announcement);
                                        }
                                        let _ = handle_clone.emit("mesh-event", event);
                                        continue;
                                    }

                                    // A counterparty asking whether we have paid it yet.
                                    if intent.intent_type == "settlement_query" {
                                        if let Ok(query) =
                                            serde_json::from_str::<crate::deal::SettlementQuery>(&intent.payload)
                                        {
                                            crate::deal::on_settlement_query(context(), query);
                                        }
                                        let _ = handle_clone.emit("mesh-event", event);
                                        continue;
                                    }

                                    if intent.intent_type == "boost_purchase_request" {
                                        if let Ok(request) = serde_json::from_str::<crate::deal::BoostPurchaseRequest>(&intent.payload) {
                                            crate::deal::on_boost_purchase_request(context(), request);
                                        }
                                        let _ = handle_clone.emit("mesh-event", event);
                                        continue;
                                    }

                                    if intent.intent_type == "boost_trade_settled" {
                                        if let Ok(announcement) = serde_json::from_str::<crate::deal::BoostTradeSettled>(&intent.payload) {
                                            crate::deal::on_boost_trade_settled(context(), announcement);
                                        }
                                        let _ = handle_clone.emit("mesh-event", event);
                                        continue;
                                    }

                                    if let Ok(payload) = serde_json::from_str::<crate::commands::MeshTradePayload>(&intent.payload) {
                                        let remote_wallet = payload.wallet;
                                        if remote_wallet.is_empty() { continue; }
                                        let now = crate::commands::now_secs();
                                        let mut store = remote_intents.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                        // Already mirrored: a peer re-announces its open orders
                                        // whenever it discovers someone, and one order must not
                                        // become several rows that each try to settle.
                                        if !payload.intent_id.is_empty()
                                            && store.by_origin(&payload.intent_id).is_some()
                                        {
                                            continue;
                                        }
                                        let remote_id = store.create_remote(
                                            payload.draft,
                                            cabal_core::RemoteOrigin {
                                                intent_id: payload.intent_id,
                                                wallet: remote_wallet,
                                            },
                                            now,
                                        );
                                        store.set_boost_mint(&remote_id, payload.boost_mint);
                                        let _ = store.transition(&remote_id, cabal_core::IntentStatus::Broadcast { route_len: intent.relay_path.len().min(u8::MAX as usize) as u8 }, now);
                                        let paired = match store.find_counterparty(&remote_id) {
                                            Some((other_id, terms))
                                                if store.pair(&remote_id, &other_id, terms, now).is_ok() =>
                                            {
                                                Some(other_id)
                                            }
                                            _ => None,
                                        };
                                        let snapshot = store.snapshot();
                                        drop(store);
                                        let _ = cabal_store::JsonStore::new(crate::app_paths::in_data_dir("intents.json")).save(&snapshot);
                                        // Remote intent arrival changes the list just as much as a
                                        // local broadcast. Notify the existing UI listener so the
                                        // matched state appears without a manual refresh.
                                        let _ = handle_clone.emit("intent-updated", serde_json::json!({ "id": remote_id.as_str() }));
                                        if let Some(other_id) = paired {
                                            let _ = handle_clone.emit("intent-updated", serde_json::json!({ "id": other_id.as_str() }));
                                            crate::deal::spawn(context(), other_id, remote_id);
                                        }
                                    }
                                }
                                let _ = handle_clone.emit("mesh-event", event);
                            }
                        });

                        // Resume parked settlements: when the escrow create leg
                        // was signed offline and queued for relay, it confirms
                        // asynchronously (self-submit or a peer's relay). Once
                        // it lands, fire the release leg. Mirrors the
                        // frontend's self-submit retry cadence, but works
                        // without the desktop UI.
                        let resume_bridge = bridge.clone();
                        let resume_mesh = mesh_handle.clone();
                        let chase_context = crate::deal::DealContext {
                            app: app_handle.clone(),
                            bridge: bridge.clone(),
                            matcher: Arc::new(MatchAgent::new(None)),
                            mesh: Some(mesh_handle.clone()),
                            intents: intents.clone(),
                        };
                        tokio::spawn(async move {
                            let mut ticker =
                                tokio::time::interval(std::time::Duration::from_secs(10));
                            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            let mut passes = 0_u32;
                            loop {
                                ticker.tick().await;

                                // This node's own queue first. A transaction signed while
                                // the RPC was down is this device's to submit the moment
                                // it can — waiting for a peer to volunteer would be
                                // slower and, alone on the mesh, never.
                                //
                                // Gated on reachability, and deliberately so: every
                                // bridge call shares one mutex, and submitting into an
                                // unreachable RPC holds that mutex for as long as the
                                // socket takes to give up. On the exact node this matters
                                // for — the offline one — that starves the relay reports
                                // and settlement chase that are trying to finish the
                                // deal. `check_rpc_reachable` is bounded at four seconds.
                                let has_queue = !resume_bridge.lock().await.get_pending_relay_txs().is_empty();
                                if has_queue && resume_bridge.lock().await.check_rpc_reachable().await {
                                    let confirmed = resume_bridge.lock().await.drain_pending_confirmed().await;
                                    if !confirmed.is_empty() {
                                        tracing::info!(count = confirmed.len(), "submitted queued transactions after reconnect");
                                    }
                                    // A queued Boost purchase settles a deal, and this
                                    // device just submitted it. Without this the trade
                                    // would only close if some peer happened to relay
                                    // the same transaction first.
                                    for tx in confirmed {
                                        if let Some(signature) = tx.tx_hash {
                                            crate::deal::on_boost_relay_confirmed(
                                                chase_context.clone(),
                                                tx.id,
                                                signature,
                                            );
                                        }
                                    }
                                }

                                // Every third pass: ask for anything owed to us. Slower
                                // than the drain because it talks to peers and the chain,
                                // and nothing here is urgent — the funds, if they moved,
                                // have already moved.
                                passes = passes.wrapping_add(1);
                                if passes % 3 == 0 {
                                    crate::deal::chase_pending_settlements(chase_context.clone());
                                }

                                let released = {
                                    let bridge = resume_bridge.lock().await;
                                    bridge.resume_pending_settlements().await
                                };
                                if released.is_empty() {
                                    continue;
                                }
                                // A release that itself queued offline still
                                // needs a peer with connectivity to submit it;
                                // broadcast the real raw transaction.
                                for queued in released
                                    .into_iter()
                                    .filter(|q| !q.raw_tx_hex.is_empty())
                                {
                                    let _ = resume_mesh
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
                        });

                        // Two compatible orders composed either side of a
                        // restart missed both matching events; pair them now
                        // that the ledger and the mesh are both up.
                        crate::deal::reconcile(crate::deal::DealContext {
                            app: app_handle.clone(),
                            bridge: bridge.clone(),
                            matcher: Arc::new(MatchAgent::new(None)),
                            mesh: Some(mesh_handle.clone()),
                            intents: intents.clone(),
                        });

                        state.set_services(state::Services {
                            mesh: Some(mesh_handle),
                            agent: Arc::new(SharkAgent::new(None)),
                            matcher: Arc::new(MatchAgent::new(None)),
                            zk_handler: Arc::new(ZKHandler::new(None)),
                            ollama: ollama_state,
                            bridge,
                            relay_bytes,
                            intents,
                        });

                        // Only once the mesh is actually participating. The
                        // lock keeps the Wi-Fi radio in a higher-power state,
                        // so taking it before there is anything to discover
                        // would be a battery cost with no benefit.
                        multicast::refresh(&app_handle);
                    }
                    Err(e) => {
                        tracing::error!("❌ Bootstrap Failed: {}", e);
                        // Publish anyway: without a mesh the chain and vault
                        // commands still work, and the UI can say so. Leaving
                        // services unset would make every command NotReady
                        // forever, which reads as a hang rather than an error.
                        state.set_services(state::Services {
                            mesh: None,
                            agent: Arc::new(SharkAgent::new(None)),
                            matcher: Arc::new(MatchAgent::new(None)),
                            zk_handler: Arc::new(ZKHandler::new(None)),
                            ollama: ollama_state,
                            bridge,
                            relay_bytes: Arc::new(AtomicU64::new(0)),
                            intents,
                        });
                    }
                }
            });

            Ok(())
        })
        // Handler registration is split by surface.
        //
        // The frozen desktop commands are compiled and registered only on
        // desktop with the `desktop-legacy` feature. The mobile handler never
        // sees them, which is what keeps `capabilities/mobile.json` able to
        // grant nothing.
        //
        // The reshaped screen commands are added to the mobile arm as they are
        // built (tickets 29 onward), never speculatively.
        .invoke_handler({
            #[cfg(all(desktop, feature = "desktop-legacy"))]
            {
                tauri::generate_handler![
                    commands::unsubscribe,
                    commands::session_status,
                    commands::enter_mesh,
                    commands::mesh_snapshot,
                    commands::subscribe_mesh_log,
                    commands::list_nearby_nodes,
                    commands::list_intents,
                    commands::intent_form_options,
                    commands::preview_intent,
                    commands::vault_assets,
                    commands::vault_identities,
                    commands::vault_keys,
                    commands::profile_summary,
                    commands::set_offline_mode,
                    commands::broadcast_intent,
                    commands::get_intent,
                    commands::cancel_intent,
                    commands::settle_intent,
                    commands::vault_total,
                    commands::export_mnemonic,
                    commands::copy_mnemonic,
                    commands::import_mnemonic,
                    commands::suggest_mnemonic_word,
                    commands::get_boost_nfts,
                    commands::claim_demo_boost,
                    commands::use_boost_nft,
                    commands::list_boost_nft,
                    commands::buy_boost_nft,
                    app_initializer::kill_switch,
            legacy::send_intent_to_mesh,
            legacy::analyze_pdf_content,
            legacy::generate_zk_proof,
            legacy::sync_blockchain_state,
            legacy::enable_instant_session,
            legacy::create_escrow,
            legacy::release_escrow,
            legacy::refund_escrow,
            legacy::get_escrow_status,
            legacy::get_bridge_status,
            legacy::check_rpc_reachable,
            legacy::get_wallet_snapshot,
            legacy::delete_wallet_snapshot,
            legacy::get_identity,
            legacy::get_primary_private_key,
            legacy::logout_wallet,
            legacy::import_wallet,
            legacy::mint_voucher,
            legacy::approve_voucher,
            legacy::create_asset_listing,
            legacy::get_active_asset_listings,
            legacy::buy_listing,
            legacy::release_deal,
            legacy::refund_deal,
            legacy::submit_raw_transaction,
            legacy::get_pending_relay_txs,
            legacy::prune_stale_relay_txs,
            legacy::mark_relay_tx_status,
            legacy::record_relayed_tx,
            legacy::get_relayed_history,
            legacy::get_relay_boost,
            legacy::apply_relay_boost,
            legacy::redeem_voucher,
            legacy::get_voucher_owner,
            legacy::get_owned_vouchers,
            legacy::get_my_deals,
            legacy::get_ollama_status,
            legacy::get_ollama_url,
            legacy::set_ollama_url,
            legacy::can_spawn_processes,
            legacy::extract_pdf_text,
            legacy::sign_content,
            legacy::store_content,
            legacy::get_content,
            legacy::receive_content,
            legacy::get_received_content,
            legacy::match_intent_to_listings,
            legacy::get_relay_stats,
            legacy::record_relay_bytes
                ]
            }
            #[cfg(not(all(desktop, feature = "desktop-legacy")))]
            {
                // Mobile gets the reshaped surface only. Screen commands join
                // it as their screens land.
                tauri::generate_handler![commands::unsubscribe, commands::session_status, commands::enter_mesh, commands::mesh_snapshot, commands::subscribe_mesh_log, commands::list_nearby_nodes, commands::list_intents, commands::intent_form_options, commands::preview_intent, commands::vault_assets, commands::vault_identities, commands::vault_keys, commands::profile_summary, commands::set_offline_mode, commands::broadcast_intent, commands::get_intent, commands::cancel_intent, commands::settle_intent, commands::vault_total, commands::export_mnemonic, commands::copy_mnemonic, commands::import_mnemonic, commands::suggest_mnemonic_word, commands::get_boost_nfts, commands::claim_demo_boost, commands::use_boost_nft, commands::list_boost_nft, commands::buy_boost_nft]
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Mobile lifecycle. Tauri 2.11 propagates these from the platform;
            // 2.9 did not, which is why an earlier plan specified a custom
            // plugin that is no longer needed.
            #[cfg(mobile)]
            if let tauri::RunEvent::WindowEvent { event, .. } = &event {
                match event {
                    tauri::WindowEvent::Suspended => lifecycle::on_suspend(app),
                    tauri::WindowEvent::Resumed => lifecycle::on_resume(app),
                    _ => {}
                }
            }
            let _ = (app, event);
        });
}
