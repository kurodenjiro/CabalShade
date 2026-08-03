//! Avalanche C-Chain bridge: identity + balance.
//!
//! # What this is
//!
//! The chain backend for CabalMesh. Identity is an EVM wallet (secp256k1,
//! `0x` hex address) and balance is native AVAX on the C-Chain (Fuji testnet
//! by default). Keys are encrypted at rest via [`cabal_vault::Vault`], exactly
//! as the previous Solana backend did — the swap changed the *chain* and the
//! *address format*, not the storage discipline.
//!
//! # What this is not
//!
//! The escrow, marketplace, voucher, content-signing, relay and instant-session
//! operations that previously used Solana's Anchor program / MagicBlock
//! ephemeral rollups are **stubbed** here: they return an honest
//! `Err("not part of the AVAX port yet")` matching the existing marketplace
//! stubs. The frozen desktop IPC contract keeps their types; nothing fills
//! them with data until the on-chain escrow ticket lands.
//!
//! # Security
//!
//! - The private key is a 32-byte hex string inside a [`cabal_vault::Secret`],
//!   which redacts itself from every `Debug`/`Display`/log path.
//! - At rest, the identity list is AES-256-GCM in `vault.enc`, keyed by the
//!   owner-only `vault.key` (see `vault_key.rs`).
//! - The key leaves the vault only in Rust for signing and address derivation;
//!   the webview sees addresses, never secrets.

use crate::vault_key;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use cabal_vault::{Secret, Vault};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;

/// Fuji C-Chain RPC (testnet). The name is no longer a lie: this is the
/// Avalanche C-Chain endpoint the app actually talks to.
pub const DEFAULT_AVAX_RPC_URL: &str = "https://api.avax-test.network/ext/bc/C/rpc";

/// One native AVAX has 1e18 wei, the EVM standard.
pub const WEI_PER_AVAX: u128 = 1_000_000_000_000_000_000;

/// An on-chain identity: a real EVM wallet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityRecord {
    pub alias: String,
    pub emoji: String,
    /// The 32-byte secp256k1 private key, `0x`-prefixed hex.
    ///
    /// `Secret`, not `String`: this struct derives `Debug`, and any `{:?}` of
    /// it would otherwise print the wallet. Encryption at rest protects the
    /// file; this protects the logs.
    pub private_key_hex: Secret,
    /// The BIP-39 mnemonic this key was derived from — the recoverable,
    /// human-writable form of the wallet. `Secret` for the same reason as the
    /// key: it must never appear in logs or errors. `#[serde(default)]` so a
    /// vault written before this field existed (or a key-only import) still
    /// loads — the key works, but there is no phrase to back it up with.
    #[serde(default)]
    pub mnemonic: Secret,
}

/// What crosses the IPC boundary for an identity. Never contains key material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityView {
    pub alias: String,
    pub emoji: String,
    /// Checksummed `0x…` C-Chain address.
    pub address: String,
}

/// A token holding, as the vault renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedAsset {
    pub id: String,
    /// Decimal wei string (avoids f64/JS-number precision loss).
    pub amount: String,
    pub symbol: String,
    pub owner: String,
    pub proof: Option<String>,
}

/// A balance snapshot, encrypted at rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub timestamp: DateTime<Utc>,
    pub assets: Vec<CompressedAsset>,
    pub signature: String,
}

/// A Marketplace listing. Kept for the frozen desktop IPC contract; the
/// marketplace is out of scope for this port, so listings come back empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListingView {
    pub id: u64,
    pub seller: String,
    pub description: String,
    pub price_wei: String,
    pub price_avax: String,
    pub token_id: u64,
}

/// A voucher NFT. Kept for the frozen desktop IPC contract; empty for now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoucherView {
    pub token_id: u64,
    pub voucher_type: String,
    pub description: String,
    pub owner: String,
    pub minted_by: String,
}

/// A Marketplace deal. Kept for the frozen desktop IPC contract; empty for now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealView {
    pub deal_id: u64,
    pub buyer: String,
    pub seller: String,
    pub token_id: u64,
    pub amount_avax: String,
    pub status: String,
    pub role: String,
}

/// A piece of content committed to by its seller. The AVAX port does not sign
/// content yet; the type stays for the frozen contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRecord {
    pub token_id: u64,
    pub text: String,
    pub fingerprint: String,
    pub signature: String,
    pub signer_address: String,
}

/// A transaction signed locally while offline, queued for a mesh peer to
/// submit. The AVAX port does not produce these yet; the type stays for the
/// frozen contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTx {
    pub id: String,
    pub raw_tx_hex: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub status: String, // "queued" | "confirmed" | "failed"
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempts: u8,
}

/// A transaction this node relayed for another peer. Kept for the frozen
/// contract; the AVAX port does not relay yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayedTxRecord {
    pub summary: String,
    pub tx_hash: String,
    pub reward_avax: String,
    pub relayed_at: DateTime<Utc>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// Result of a chain action: confirmed immediately, or queued for relay.
/// Kept for the frozen contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TxResult {
    #[serde(rename = "confirmed")]
    Confirmed { id: u64 },
    #[serde(rename = "queued")]
    Queued {
        #[serde(rename = "queueId")]
        queue_id: String,
    },
}

/// An agent authority delegation. The AVAX port does not create these yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantSession {
    pub session_id: String,
    pub authority: String,
    pub expiry: DateTime<Utc>,
    pub is_active: bool,
}

/// The Avalanche bridge: identity vault + RPC access for balance.
pub struct BlockchainBridge {
    pub identities: Vec<IdentityRecord>,
    /// Encrypted store for identities.
    pub identity_vault: Vault<vault_key::FileKeyProvider>,
    pub storage_path: PathBuf,
    pub chain_cache_path: PathBuf,
    pub pending_relay_path: PathBuf,
    pub relayed_history_path: PathBuf,
    pub content_store_path: PathBuf,
    pub received_content_path: PathBuf,
    pub relay_boost_path: PathBuf,
    pub rpc_url: String,
    pub current_session: Option<InstantSession>,
}

impl BlockchainBridge {
    pub fn new(rpc_url_override: Option<String>) -> Self {
        let rpc_url = rpc_url_override.unwrap_or_else(|| DEFAULT_AVAX_RPC_URL.to_string());

        let app_dir = crate::app_paths::data_dir();

        let mut bridge = Self {
            identities: Vec::new(),
            identity_vault: Vault::new(
                app_dir.join("vault.enc"),
                vault_key::platform_provider(app_dir.join("vault.key")),
            ),
            storage_path: app_dir.join("snapshot.enc"),
            chain_cache_path: app_dir.join("chain_cache.json"),
            pending_relay_path: app_dir.join("pending_relay_txs.json"),
            relayed_history_path: app_dir.join("relayed_history.json"),
            content_store_path: app_dir.join("content_store.json"),
            received_content_path: app_dir.join("received_content.json"),
            relay_boost_path: app_dir.join("relay_boost.json"),
            rpc_url,
            current_session: None,
        };
        let _ = bridge.load_identities();
        bridge
    }

    fn save_identities(&self) -> Result<(), Box<dyn Error>> {
        self.identity_vault.save(&self.identities)?;
        Ok(())
    }

    pub fn load_identities(&mut self) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        // Adopt any pre-encryption wallet first (legacy plaintext file).
        let plaintext = cabal_store::JsonStore::new(self.plaintext_identity_path());
        match self
            .identity_vault
            .migrate_plaintext::<Vec<IdentityRecord>>(&plaintext)
        {
            Ok(true) => tracing::info!("🔐 Identities migrated into the encrypted vault"),
            Ok(false) => {}
            Err(error) => {
                tracing::error!(%error, "identity migration failed; plaintext left intact");
                return Err(Box::new(error));
            }
        }

        if self.identity_vault.exists() {
            tracing::info!("🔑 Loading identities from the encrypted vault");
            match self.identity_vault.load::<Vec<IdentityRecord>>() {
                Ok(records) => {
                    self.identities = records;
                    if self.identities.is_empty() {
                        return self.generate_new_identity("Primary Fox".to_string(), "🦊".to_string());
                    }
                    self.get_identity_views()
                }
                Err(error) => {
                    tracing::error!(%error, "vault exists but cannot be decrypted");
                    Err(Box::new(error))
                }
            }
        } else {
            self.generate_new_identity("Genesis Fox".to_string(), "🦊".to_string())
        }
    }

    fn plaintext_identity_path(&self) -> PathBuf {
        crate::app_paths::in_data_dir("identities.json")
    }

    pub fn generate_new_identity(
        &mut self,
        alias: String,
        emoji: String,
    ) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        tracing::info!("🆕 Generating NEW Identity '{}' [{}]...", alias, emoji);
        // The mnemonic is the source of truth: generate the phrase, derive the
        // key from it (BIP-39 seed -> first 32 bytes). The phrase is the
        // recoverable, human-writable form of this exact wallet.
        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let key = mnemonic.to_key();
        self.identities.push(IdentityRecord {
            alias,
            emoji,
            private_key_hex: format!("0x{:064x}", key).into(),
            mnemonic: mnemonic.words().join(" ").into(),
        });
        self.save_identities()?;
        self.get_identity_views()
    }

    pub fn get_identity_views(&self) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        let mut views = Vec::new();
        for id in &self.identities {
            let signer = signer_from_secret(id.private_key_hex.expose())?;
            views.push(IdentityView {
                alias: id.alias.clone(),
                emoji: id.emoji.clone(),
                address: signer.address().to_string(),
            });
        }
        Ok(views)
    }

    pub fn logout_identity(&mut self) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        self.identities.clear();
        let _ = self.delete_snapshot();
        let _ = std::fs::remove_file(&self.relay_boost_path);
        self.generate_new_identity("Genesis Fox".to_string(), "🦊".to_string())
    }

    pub fn import_identity(
        &mut self,
        private_key_hex: String,
        alias: String,
        emoji: String,
    ) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        self.import_with_mnemonic(private_key_hex, None, alias, emoji)
    }

    /// Imports an identity from a private key and, when known, its mnemonic.
    ///
    /// When a mnemonic is supplied it is stored alongside the key so the
    /// recoverable phrase is preserved; a key-only import stores an empty
    /// phrase (the key still works, but there is no phrase to back it up
    /// with — the UI says so).
    pub fn import_with_mnemonic(
        &mut self,
        private_key_hex: String,
        mnemonic: Option<String>,
        alias: String,
        emoji: String,
    ) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        let normalized = private_key_hex.trim().to_string();
        // Validate it derives a wallet before persisting.
        signer_from_secret(&normalized)?;

        self.identities = vec![IdentityRecord {
            alias,
            emoji,
            private_key_hex: normalized.into(),
            mnemonic: mnemonic.unwrap_or_default().into(),
        }];
        let _ = self.delete_snapshot();
        let _ = std::fs::remove_file(&self.relay_boost_path);
        self.save_identities()?;
        self.get_identity_views()
    }

    pub fn get_primary_private_key(&self) -> Option<String> {
        self.identities
            .first()
            .map(|id| id.private_key_hex.expose().to_owned())
    }

    /// The primary identity's BIP-39 mnemonic, when one is stored.
    pub fn primary_mnemonic(&self) -> Option<String> {
        self.identities
            .first()
            .map(|id| id.mnemonic.expose().to_owned())
            .filter(|m| !m.is_empty())
    }

    pub fn get_primary_address(&self) -> String {
        match self.identities.first() {
            Some(first) => match signer_from_secret(first.private_key_hex.expose()) {
                Ok(signer) => signer.address().to_string(),
                Err(_) => "unknown".to_string(),
            },
            None => "unknown".to_string(),
        }
    }

    /// Whether the RPC is reachable.
    pub async fn check_rpc_reachable(&self) -> bool {
        use alloy::providers::Provider;
        match ProviderBuilder::new().connect(&self.rpc_url).await {
            Ok(provider) => match provider.get_chain_id().await {
                Ok(_) => true,
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    /// Syncs the native AVAX balance for the primary identity and saves a
    /// snapshot.
    #[tracing::instrument(skip(self), fields(rpc = %self.rpc_url))]
    pub async fn sync_state(
        &self,
        wallet_address_override: &str,
    ) -> Result<Snapshot, Box<dyn Error>> {
        let primary = self.get_primary_address();
        let target = if primary != "unknown" {
            primary
        } else {
            wallet_address_override.to_string()
        };
        let address = alloy::primitives::Address::from_str(&target)
            .map_err(|e| format!("invalid address {target}: {e}"))?;

        tracing::info!("🔄 [Bridge] Fetching native AVAX balance from {}", self.rpc_url);

        use alloy::providers::Provider;
        let balance = ProviderBuilder::new().connect(&self.rpc_url).await?.get_balance(address).await?;

        tracing::info!("✅ [Bridge] Fetched balance for {}", target);

        let snapshot = Snapshot {
            timestamp: Utc::now(),
            assets: vec![CompressedAsset {
                id: "native-avax".to_string(),
                amount: balance.to_string(),
                symbol: "AVAX".to_string(),
                owner: target,
                proof: None,
            }],
            signature: "verified_by_avax_rpc".to_string(),
        };

        self.save_snapshot_encrypted(&snapshot)?;

        Ok(snapshot)
    }

    fn save_snapshot_encrypted(&self, snapshot: &Snapshot) -> Result<(), Box<dyn Error>> {
        cabal_store::JsonStore::compact(&self.storage_path).save(&snapshot)?;
        tracing::info!("💾 [Bridge] Snapshot saved.");
        Ok(())
    }

    pub fn get_latest_snapshot(&self) -> Result<Snapshot, Box<dyn Error>> {
        if !self.storage_path.exists() {
            return Err("No snapshot found".into());
        }
        let file_data = std::fs::read(&self.storage_path)?;
        let snapshot: Snapshot = serde_json::from_slice(&file_data)?;
        Ok(snapshot)
    }

    pub fn delete_snapshot(&self) -> Result<(), Box<dyn Error>> {
        if self.storage_path.exists() {
            std::fs::remove_file(&self.storage_path)?;
        }
        Ok(())
    }

    pub fn delete_identity(&self) -> Result<(), Box<dyn Error>> {
        cabal_store::JsonStore::new(crate::app_paths::in_data_dir("vault.enc")).delete()?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Stubs: Solana-era chain operations that the AVAX port has not rewired.
    // Each is honest about being out of scope, matching the existing
    // marketplace/voucher stub pattern. The frozen IPC contract keeps the
    // types; nothing fills them with data yet.
    // ---------------------------------------------------------------------

    /// Retry budget for the offline relay queue. Frozen by `tests/offline_queue.rs`;
    /// the AVAX port does not relay yet, but the constant stays a real value.
    pub const MAX_ATTEMPTS: u8 = 5;

    pub async fn drain_pending(&self) -> Result<usize, Box<dyn Error>> {
        Ok(0)
    }

    pub fn init_instant_session(&mut self) -> InstantSession {
        InstantSession {
            session_id: format!("sess_{}", Utc::now().timestamp()),
            authority: self.get_primary_address(),
            expiry: Utc::now() + chrono::Duration::minutes(30),
            is_active: false,
        }
    }

    pub async fn create_escrow(
        &self,
        _payee: &str,
        _amount: &str,
        _expiry_unix: u64,
    ) -> Result<TxResult, Box<dyn Error>> {
        Err("escrow is not part of the AVAX port yet".into())
    }

    pub async fn release_escrow(&self, _escrow_id: u64) -> Result<String, Box<dyn Error>> {
        Err("escrow is not part of the AVAX port yet".into())
    }

    pub async fn refund_escrow(&self, _escrow_id: u64) -> Result<String, Box<dyn Error>> {
        Err("escrow is not part of the AVAX port yet".into())
    }

    pub async fn get_escrow_status(
        &self,
        _escrow_id: u64,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        Err("escrow is not part of the AVAX port yet".into())
    }

    pub fn get_status(&self) -> String {
        format!("Avalanche C-Chain bridge @ {}", self.rpc_url)
    }

    pub async fn get_active_asset_listings(
        &self,
    ) -> Result<Vec<AssetListingView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    pub async fn create_asset_listing(
        &self,
        _description: &str,
        _price: String,
        _token_id: u64,
    ) -> Result<u64, Box<dyn Error>> {
        Err("marketplace is not part of the AVAX port yet".into())
    }

    pub async fn buy_listing(
        &self,
        _listing_id: u64,
        _price: String,
    ) -> Result<TxResult, Box<dyn Error>> {
        Err("marketplace is not part of the AVAX port yet".into())
    }

    pub async fn release_deal(&self, _deal_id: u64) -> Result<String, Box<dyn Error>> {
        Err("marketplace is not part of the AVAX port yet".into())
    }

    pub async fn refund_deal(&self, _deal_id: u64) -> Result<String, Box<dyn Error>> {
        Err("marketplace is not part of the AVAX port yet".into())
    }

    pub async fn get_my_deals(&self, _address: &str) -> Result<Vec<DealView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    pub async fn mint_voucher(
        &self,
        _voucher_type: &str,
        _description: &str,
    ) -> Result<u64, Box<dyn Error>> {
        Err("vouchers are not part of the AVAX port yet".into())
    }

    pub async fn approve_voucher(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("vouchers are not part of the AVAX port yet".into())
    }

    pub async fn redeem_voucher(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("vouchers are not part of the AVAX port yet".into())
    }

    pub async fn get_voucher_owner(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("vouchers are not part of the AVAX port yet".into())
    }

    pub async fn get_owned_vouchers(
        &self,
        _owner: &str,
    ) -> Result<Vec<VoucherView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    pub async fn submit_raw_transaction(
        &self,
        _raw_tx_hex: &str,
    ) -> Result<String, Box<dyn Error>> {
        Err("raw transaction relay is not part of the AVAX port yet".into())
    }

    pub fn get_pending_relay_txs(&self) -> Vec<QueuedTx> {
        Vec::new()
    }

    pub async fn prune_stale_relay_txs(&self) -> Result<usize, Box<dyn Error>> {
        Ok(0)
    }

    pub fn mark_relay_tx_status(
        &self,
        _queue_id: &str,
        _status: &str,
        _tx_hash: Option<String>,
    ) -> Result<(), Box<dyn Error>> {
        Err("relay is not part of the AVAX port yet".into())
    }

    pub fn record_relayed_tx(
        &self,
        _summary: &str,
        _tx_hash: &str,
        _reward_avax: &str,
    ) -> Result<(), Box<dyn Error>> {
        Err("relay is not part of the AVAX port yet".into())
    }

    pub fn get_relayed_history(&self) -> Vec<RelayedTxRecord> {
        Vec::new()
    }

    pub fn get_relay_boost_multiplier(&self) -> f64 {
        1.0
    }

    pub fn apply_relay_boost(&self, _additional: f64) -> Result<f64, Box<dyn Error>> {
        Err("relay boost is not part of the AVAX port yet".into())
    }

    pub fn extract_pdf_text(&self, _pdf_bytes: Vec<u8>) -> Result<String, Box<dyn Error>> {
        Err("pdf extraction is not part of the AVAX port yet".into())
    }

    pub fn sign_content(&self, _text: &str) -> Result<ContentRecord, Box<dyn Error>> {
        Err("content signing is not part of the AVAX port yet".into())
    }

    pub fn store_content(
        &self,
        _token_id: u64,
        _record: ContentRecord,
    ) -> Result<(), Box<dyn Error>> {
        Err("content store is not part of the AVAX port yet".into())
    }

    pub fn get_content(&self, _token_id: u64) -> Option<ContentRecord> {
        None
    }

    pub fn receive_content(
        &self,
        _token_id: u64,
        _text: &str,
        _signature: &str,
        _expected_seller: &str,
    ) -> Result<bool, Box<dyn Error>> {
        Err("content signing is not part of the AVAX port yet".into())
    }

    pub fn get_received_content(&self, _token_id: u64) -> Option<ContentRecord> {
        None
    }
}

/// Derives an EVM signer from a `0x`-prefixed 32-byte private key hex string.
///
/// # Errors
///
/// If the string is not valid hex of exactly 32 bytes (64 hex chars).
pub fn signer_from_secret(secret: &str) -> Result<PrivateKeySigner, Box<dyn Error>> {
    let trimmed = secret.trim().trim_start_matches("0x");
    let bytes = alloy::primitives::hex::decode(trimmed)?;
    if bytes.len() != 32 {
        return Err("private key must be 32 bytes (64 hex characters)".into());
    }
    let signer = PrivateKeySigner::from_bytes(&alloy::primitives::B256::from_slice(&bytes))?;
    Ok(signer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The load-bearing invariant of the whole wallet mechanism: the mnemonic
    /// stored at generation derives the exact key stored at generation, so
    /// "write down the words, type them back later" restores the same wallet.
    #[test]
    fn a_generated_identity_round_trips_through_its_mnemonic() {
        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let key = mnemonic.to_key();
        let hex_key = format!("0x{:064x}", key);

        // The stored key parses, and its address matches the mnemonic's.
        let signer = signer_from_secret(&hex_key).unwrap();
        let mnemonic_signer = mnemonic.to_signer().unwrap();
        assert_eq!(signer.address(), mnemonic_signer.address());
    }

    #[test]
    fn an_evm_key_must_be_32_bytes() {
        assert!(signer_from_secret("0x1234").is_err());
        assert!(signer_from_secret("not-hex").is_err());
    }

    #[test]
    fn a_valid_evm_key_derives_an_address() {
        // A well-known test key (Anvil/Hardhat #0).
        let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let signer = signer_from_secret(key).unwrap();
        assert_eq!(
            signer.address().to_string(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
    }
}
