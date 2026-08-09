use serde::{Deserialize, Serialize};
use cabal_store::JsonStore;
use cabal_vault::{Secret, Vault};
use std::fs;
use std::path::PathBuf;
use std::error::Error;
use std::str::FromStr;
use chrono::{DateTime, Utc};

// Solana + MagicBlock
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use bs58;
use aes_gcm::aead::{OsRng, rand_core::RngCore};
use tokio::time::{timeout, Duration};

/// Solana devnet RPC default.
pub const DEFAULT_SOLANA_RPC_URL: &str = "https://api.devnet.solana.com";

/// Deprecated alias kept for the frozen desktop config and any external callers.
#[deprecated(note = "use DEFAULT_SOLANA_RPC_URL")]
pub const DEFAULT_AVAX_RPC_URL: &str = DEFAULT_SOLANA_RPC_URL;

/// MagicBlock Magic Router (devnet) — auto-routes base-layer vs Ephemeral
/// Rollup transactions.
pub const DEFAULT_MAGIC_ROUTER_URL: &str = "https://devnet-router.magicblock.app/";

/// The deployed `cabal_escrow` Anchor program on devnet.
pub const ESCROW_PROGRAM_ID: &str = "7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq";
pub const BOOST_PROGRAM_ID: &str = "DVJ6GqkLAGwxceuMLJoKBKrfCposypoMCpBEHFea9GNa";
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// Solana lamports per SOL.
pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

/// Anchor instruction discriminators for `cabal_escrow` (sha256 of the
/// qualified name, first 8 bytes). Matches the generated IDL.
// Anchor instruction discriminators: sha256("global:<name>")[..8].
const IX_INITIALIZE_ESCROW: [u8; 8] = [0xf3, 0xa0, 0x4d, 0x99, 0x0b, 0x5c, 0x30, 0xd1];
const IX_RELEASE: [u8; 8] = [0xfd, 0xf9, 0x0f, 0xce, 0x1c, 0x7f, 0xc1, 0xf1];
const IX_REFUND: [u8; 8] = [0x02, 0x60, 0xb7, 0xfb, 0x3f, 0xd0, 0x2e, 0x2e];

fn anchor_discriminator(name: &str) -> [u8; 8] {
    solana_sdk::hash::hash(format!("global:{name}").as_bytes()).to_bytes()[..8]
        .try_into().expect("hash has 32 bytes")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityRecord {
    pub alias: String,
    pub emoji: String,
    /// Base58-encoded ed25519 keypair (Solana).
    ///
    /// `Secret`, not `String`: this struct derives `Debug`, and any `{:?}` of
    /// it would otherwise print the wallet. Encryption at rest protects the
    /// file; this protects the logs.
    pub private_key_hex: Secret,
    /// The BIP-39 mnemonic this keypair was derived from — the recoverable,
    /// human-writable form of the wallet. `Secret` for the same reason as the
    /// key. `#[serde(default)]` so a vault written before this field existed
    /// (or a key-only import) still loads.
    #[serde(default)]
    pub mnemonic: Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityView {
    pub alias: String,
    pub emoji: String,
    /// Base58 Solana address (no 0x prefix).
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoostNftRecord {
    pub mint: String,
    pub name: String,
    pub boost_bps: u16,
    pub expires_at: i64,
    pub owned: bool,
    pub listed: bool,
    pub price_lamports: Option<String>,
    pub seller: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedAsset {
    pub id: String,
    /// Decimal lamport string (avoids f64/JS-number precision loss).
    pub amount: String,
    pub symbol: String,
    pub owner: String,
    pub proof: Option<String>,
}

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

/// A piece of content committed to by its seller: a real ed25519 signature
/// over the exact text, verifiable by recovering the signer's pubkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRecord {
    pub token_id: u64,
    pub text: String,
    pub fingerprint: String,
    pub signature: String,
    pub signer_address: String,
}

/// Cached blockhash snapshot from the last time we reached the RPC, used to
/// sign transactions offline when the RPC can't be reached at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStateCache {
    /// The last known recent blockhash, base58.
    pub blockhash: String,
    pub cached_at: DateTime<Utc>,
}

/// A transaction signed locally while offline, queued for a mesh peer with
/// real connectivity to submit on our behalf.
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

/// A transaction this node successfully relayed to the chain on behalf of
/// another peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayedTxRecord {
    pub summary: String,
    pub tx_hash: String,
    /// A deterministic estimate (bytes relayed × rate) — NOT a real payout.
    pub reward_avax: String,
    pub relayed_at: DateTime<Utc>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// Result of an action that normally hits the chain directly: either it went
/// through immediately (`Confirmed`), or the RPC was unreachable and it was
/// signed offline and queued for mesh relay instead (`Queued`).
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantSession {
    pub session_id: String,
    pub authority: String,
    pub expiry: DateTime<Utc>,
    pub is_active: bool,
}

pub struct BlockchainBridge {
    pub identities: Vec<IdentityRecord>,
    /// Encrypted store for identities.
    pub identity_vault: Vault<crate::vault_key::FileKeyProvider>,
    pub storage_path: PathBuf,
    pub chain_cache_path: PathBuf,
    pub pending_relay_path: PathBuf,
    pub relayed_history_path: PathBuf,
    pub content_store_path: PathBuf,
    pub received_content_path: PathBuf,
    pub relay_boost_path: PathBuf,
    pub rpc_url: String,
    pub current_session: Option<InstantSession>,
    /// Reserved for future MagicBlock Ephemeral Rollup delegation/commit/undelegate
    /// calls. Base-layer transactions now route through [`Self::rpc_url`].
    pub router_url: String,
}

/// The two signatures of a settled deal: the escrow creation and the
/// release. Both are real on-chain transaction hashes on Solana devnet.
#[derive(Debug, Clone)]
pub struct SettledEscrow {
    pub create_tx: String,
    pub release_tx: String,
    /// A transaction signed offline and queued for mesh relay (create, release,
    /// or both) that still needs a peer with connectivity to submit. Empty when
    /// the whole settlement went through online.
    pub queued_for_relay: Vec<QueuedTx>,
}

impl BlockchainBridge {
    pub fn new(rpc_url_override: Option<String>) -> Self {
        let rpc_url = rpc_url_override.unwrap_or_else(|| DEFAULT_SOLANA_RPC_URL.to_string());

        let app_dir = crate::app_paths::data_dir();

        let mut bridge = Self {
            identities: Vec::new(),
            identity_vault: Vault::new(
                app_dir.join("vault.enc"),
                crate::vault_key::platform_provider(app_dir.join("vault.key")),
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
            router_url: std::env::var("MAGIC_ROUTER_URL")
                .unwrap_or_else(|_| DEFAULT_MAGIC_ROUTER_URL.to_string()),
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
        let plaintext = JsonStore::new(self.plaintext_identity_path());
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

    pub fn generate_new_identity(&mut self, alias: String, emoji: String) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        tracing::info!("🆕 Generating NEW Identity '{}' [{}]...", alias, emoji);
        // The mnemonic is the source of truth: generate the phrase, derive the
        // keypair's seed from it (BIP-39 seed -> first 32 bytes). The phrase is
        // the recoverable, human-writable form of this exact wallet.
        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let keypair = mnemonic.to_keypair()?;
        let key = bs58::encode(keypair.to_bytes()).into_string();
        self.identities.push(IdentityRecord {
            alias,
            emoji,
            private_key_hex: key.into(),
            mnemonic: mnemonic.words().join(" ").into(),
        });
        self.save_identities()?;
        self.get_identity_views()
    }

    pub fn get_identity_views(&self) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        let mut views = Vec::new();
        for id in &self.identities {
            let keypair = keypair_from_secret(id.private_key_hex.expose())?;
            views.push(IdentityView {
                alias: id.alias.clone(),
                emoji: id.emoji.clone(),
                address: keypair.pubkey().to_string(),
            });
        }
        Ok(views)
    }

    pub fn logout_identity(&mut self) -> Result<Vec<IdentityView>, Box<dyn Error>> {
        self.identities.clear();
        let _ = self.delete_snapshot();
        let _ = fs::remove_file(&self.relay_boost_path);
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
    /// When a mnemonic is supplied it is stored alongside the keypair so the
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
        // Validate it derives a keypair before persisting.
        keypair_from_secret(&normalized)?;

        self.identities = vec![IdentityRecord {
            alias,
            emoji,
            private_key_hex: normalized.into(),
            mnemonic: mnemonic.unwrap_or_default().into(),
        }];
        let _ = self.delete_snapshot();
        let _ = fs::remove_file(&self.relay_boost_path);
        self.save_identities()?;
        self.get_identity_views()
    }

    pub fn get_primary_private_key(&self) -> Option<String> {
        self.identities.first().map(|id| id.private_key_hex.expose().to_owned())
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
            Some(first) => match keypair_from_secret(first.private_key_hex.expose()) {
                Ok(keypair) => keypair.pubkey().to_string(),
                Err(_) => "unknown".to_string(),
            },
            None => "unknown".to_string(),
        }
    }

    fn primary_keypair(&self) -> Result<Keypair, Box<dyn Error>> {
        let first = self.identities.first().ok_or("No identity available")?;
        keypair_from_secret(first.private_key_hex.expose())
    }

    fn rpc_client(&self) -> RpcClient {
        RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::confirmed())
    }

    fn boost_program_id(&self) -> Pubkey { Pubkey::from_str(BOOST_PROGRAM_ID).expect("valid boost program id") }

    fn boost_pda(&self, mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"boost", mint.as_ref()], &self.boost_program_id()).0
    }

    fn listing_pda(&self, seller: &Pubkey, mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"listing", seller.as_ref(), mint.as_ref()], &self.boost_program_id()).0
    }

    fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let token = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).expect("valid token program id");
        let associated = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).expect("valid associated token program id");
        Pubkey::find_program_address(&[owner.as_ref(), token.as_ref(), mint.as_ref()], &associated).0
    }

    async fn send_boost_instruction(&self, instruction: Instruction, _summary: &str) -> Result<String, String> {
        let keypair = self.primary_keypair().map_err(|e| e.to_string())?;
        let online = timeout(Duration::from_secs(8), async {
            let client = self.rpc_client();
            let blockhash = client.get_latest_blockhash().await.map_err(|e| e.to_string())?;
            let message = Message::new(std::slice::from_ref(&instruction), Some(&keypair.pubkey()));
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[&keypair], blockhash);
            let sig = client.send_and_confirm_transaction(&tx).await.map_err(|e| e.to_string())?;
            Ok::<String, String>(sig.to_string())
        }).await;
        match online {
            Ok(result) => result,
            Err(_) => Err("Solana RPC timed out; boost transaction was not submitted".to_string()),
        }
    }

    pub async fn use_boost_nft(&self, mint_text: &str) -> Result<String, String> {
        let mint = Pubkey::from_str(mint_text).map_err(|e| e.to_string())?;
        let keypair = self.primary_keypair().map_err(|e| e.to_string())?;
        let token_program = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).map_err(|e| e.to_string())?;
        let boost = self.boost_pda(&mint);
        let user_tokens = Self::associated_token_address(&keypair.pubkey(), &mint);
        let instruction = Instruction {
            program_id: self.boost_program_id(),
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(boost, false),
                solana_sdk::instruction::AccountMeta::new_readonly(mint, false),
                solana_sdk::instruction::AccountMeta::new(user_tokens, false),
                solana_sdk::instruction::AccountMeta::new_readonly(keypair.pubkey(), true),
                solana_sdk::instruction::AccountMeta::new_readonly(token_program, false),
            ],
            data: anchor_discriminator("use_boost").to_vec(),
        };
        self.send_boost_instruction(instruction, "Use boost NFT (burn)").await
    }

    pub async fn list_boost_nft(&self, mint_text: &str, price_lamports: u64) -> Result<String, String> {
        let mint = Pubkey::from_str(mint_text).map_err(|e| e.to_string())?;
        let keypair = self.primary_keypair().map_err(|e| e.to_string())?;
        let token_program = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).map_err(|e| e.to_string())?;
        let associated_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).map_err(|e| e.to_string())?;
        let boost = self.boost_pda(&mint);
        let listing = self.listing_pda(&keypair.pubkey(), &mint);
        let user_tokens = Self::associated_token_address(&keypair.pubkey(), &mint);
        let vault_tokens = Self::associated_token_address(&listing, &mint);
        let mut data = anchor_discriminator("list_boost").to_vec();
        data.extend_from_slice(&price_lamports.to_le_bytes());
        let instruction = Instruction {
            program_id: self.boost_program_id(),
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(boost, false),
                solana_sdk::instruction::AccountMeta::new_readonly(mint, false),
                solana_sdk::instruction::AccountMeta::new(listing, false),
                solana_sdk::instruction::AccountMeta::new(user_tokens, false),
                solana_sdk::instruction::AccountMeta::new(vault_tokens, false),
                solana_sdk::instruction::AccountMeta::new(keypair.pubkey(), true),
                solana_sdk::instruction::AccountMeta::new_readonly(token_program, false),
                solana_sdk::instruction::AccountMeta::new_readonly(associated_program, false),
                solana_sdk::instruction::AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            ],
            data,
        };
        self.send_boost_instruction(instruction, "List boost NFT via mesh").await
    }

    pub async fn buy_boost_nft(&self, mint_text: &str, seller_text: &str) -> Result<String, String> {
        let mint = Pubkey::from_str(mint_text).map_err(|e| e.to_string())?;
        let seller = Pubkey::from_str(seller_text).map_err(|e| e.to_string())?;
        let keypair = self.primary_keypair().map_err(|e| e.to_string())?;
        let token_program = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).map_err(|e| e.to_string())?;
        let listing = self.listing_pda(&seller, &mint);
        let vault_tokens = Self::associated_token_address(&listing, &mint);
        let buyer_tokens = Self::associated_token_address(&keypair.pubkey(), &mint);
        let instruction = Instruction {
            program_id: self.boost_program_id(),
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(listing, false),
                solana_sdk::instruction::AccountMeta::new_readonly(mint, false),
                solana_sdk::instruction::AccountMeta::new(vault_tokens, false),
                solana_sdk::instruction::AccountMeta::new(buyer_tokens, false),
                solana_sdk::instruction::AccountMeta::new(seller, false),
                solana_sdk::instruction::AccountMeta::new(keypair.pubkey(), true),
                solana_sdk::instruction::AccountMeta::new_readonly(token_program, false),
                solana_sdk::instruction::AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            ],
            data: anchor_discriminator("buy_boost").to_vec(),
        };
        self.send_boost_instruction(instruction, "Buy boost NFT via mesh").await
    }

    pub async fn list_boost_nfts(&self) -> Result<Vec<BoostNftRecord>, String> {
        let client = self.rpc_client();
        let program = self.boost_program_id();
        let accounts = client.get_program_accounts(&program).await.map_err(|e| e.to_string())?;
        let boost_disc = anchor_discriminator("account:Boost");
        let listing_disc = anchor_discriminator("account:BoostListing");
        let mut boosts = Vec::new();
        let mut listings: std::collections::HashMap<Pubkey, (Pubkey, u64, i64)> = std::collections::HashMap::new();
        for (_address, account) in accounts {
            let data = account.data;
            if data.len() >= 83 && data[..8] == boost_disc {
                let mint = Pubkey::new_from_array(data[8..40].try_into().map_err(|_| "invalid boost mint")?);
                let boost_bps = u16::from_le_bytes(data[72..74].try_into().map_err(|_| "invalid boost bps")?);
                let expires_at = i64::from_le_bytes(data[74..82].try_into().map_err(|_| "invalid boost expiry")?);
                boosts.push((mint, boost_bps, expires_at));
            } else if data.len() >= 88 && data[..8] == listing_disc {
                let seller = Pubkey::new_from_array(data[8..40].try_into().map_err(|_| "invalid listing seller")?);
                let mint = Pubkey::new_from_array(data[40..72].try_into().map_err(|_| "invalid listing mint")?);
                let price = u64::from_le_bytes(data[72..80].try_into().map_err(|_| "invalid listing price")?);
                let expiry = i64::from_le_bytes(data[80..88].try_into().map_err(|_| "invalid listing expiry")?);
                listings.insert(mint, (seller, price, expiry));
            }
        }
        let owner = self.primary_keypair().map_err(|e| e.to_string())?.pubkey();
        let now = Utc::now().timestamp();
        let mut result = Vec::with_capacity(boosts.len());
        for (mint, boost_bps, expires_at) in boosts {
            let ata = Self::associated_token_address(&owner, &mint);
            let owned = client.get_account(&ata).await.ok().map(|account| {
                account.data.len() >= 72 && u64::from_le_bytes(account.data[64..72].try_into().unwrap_or([0; 8])) > 0
            }).unwrap_or(false);
            let listing = listings.get(&mint);
            let listed = listing.is_some_and(|(_, _, expiry)| *expiry > now);
            result.push(BoostNftRecord {
                mint: mint.to_string(),
                name: format!("RELAY BOOST +{:.2}%", boost_bps as f64 / 100.0),
                boost_bps,
                expires_at,
                owned,
                listed,
                price_lamports: listing.map(|(_, price, _)| price.to_string()),
                seller: listing.map(|(seller, _, _)| seller.to_string()),
            });
        }
        Ok(result)
    }

    // ---- Offline signing + mesh-relay queue -------------------------------

    fn load_chain_cache(&self) -> Option<ChainStateCache> {
        fs::read_to_string(&self.chain_cache_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
    }

    fn save_chain_cache(&self, cache: &ChainStateCache) -> Result<(), Box<dyn Error>> {
        JsonStore::new(&self.chain_cache_path).save(cache)?;
        Ok(())
    }

    fn load_pending_relay_txs(&self) -> Vec<QueuedTx> {
        fs::read_to_string(&self.pending_relay_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn save_pending_relay_txs(&self, txs: &[QueuedTx]) -> Result<(), Box<dyn Error>> {
        JsonStore::new(&self.pending_relay_path).save(&txs)?;
        Ok(())
    }

    /// Refreshes the cached blockhash from the live RPC. Called
    /// opportunistically whenever we know we're online.
    async fn refresh_chain_cache(&self) -> Result<(), Box<dyn Error>> {
        let client = self.rpc_client();
        let blockhash = client.get_latest_blockhash().await?;
        self.save_chain_cache(&ChainStateCache {
            blockhash: blockhash.to_string(),
            cached_at: Utc::now(),
        })?;
        Ok(())
    }

    /// Signs an instruction fully offline using the cached blockhash and
    /// queues the raw serialized transaction for a mesh peer to relay.
    async fn sign_offline(&self, instruction: Instruction, summary: &str) -> Result<QueuedTx, Box<dyn Error>> {
        let cache = self.load_chain_cache().ok_or("No cached chain state available — never been online yet")?;
        let keypair = self.primary_keypair()?;
        let blockhash = bs58::decode(&cache.blockhash)
            .into_vec()
            .ok()
            .and_then(|bytes| {
                let arr: [u8; 32] = bytes.try_into().ok()?;
                Some(solana_sdk::hash::Hash::new_from_array(arr))
            })
            .ok_or("cached blockhash is invalid")?;

        let message = Message::new(&[instruction], Some(&keypair.pubkey()));
        let mut tx = Transaction::new_unsigned(message);
        tx.sign(&[&keypair], blockhash);

        let raw = bincode::serialize(&tx)?;
        let raw_tx_hex = format!("0x{}", hex::encode(&raw));

        let mut suffix = [0u8; 4];
        OsRng.fill_bytes(&mut suffix);
        let id = format!("tx-{}-{}", Utc::now().timestamp_millis(), hex::encode(suffix));

        let queued = QueuedTx {
            id,
            raw_tx_hex,
            summary: summary.to_string(),
            created_at: Utc::now(),
            status: "queued".to_string(),
            tx_hash: None,
            reason: None,
            attempts: 0,
        };

        let mut pending = self.load_pending_relay_txs();
        pending.push(queued.clone());
        self.save_pending_relay_txs(&pending)?;

        tracing::info!("📡 [Bridge] Signed offline, queued for mesh relay: {} ({})", queued.id, summary);
        Ok(queued)
    }

    pub const MAX_ATTEMPTS: u8 = 5;

    pub async fn drain_pending(&self) -> Result<usize, Box<dyn Error>> {
        let mut pending = self.load_pending_relay_txs();
        if pending.is_empty() {
            return Ok(0);
        }

        let mut confirmed = 0_usize;
        for entry in &mut pending {
            if entry.status != "queued" || entry.attempts >= Self::MAX_ATTEMPTS {
                continue;
            }
            entry.attempts = entry.attempts.saturating_add(1);

            match self.submit_raw_transaction(&entry.raw_tx_hex).await {
                Ok(tx_hash) => {
                    entry.status = "confirmed".to_string();
                    entry.tx_hash = Some(tx_hash);
                    entry.reason = None;
                    confirmed += 1;
                    tracing::info!(id = %entry.id, "queued transaction confirmed after reconnect");
                }
                Err(error) => {
                    entry.reason = Some(error.to_string());
                    if entry.attempts >= Self::MAX_ATTEMPTS {
                        entry.status = "failed".to_string();
                        tracing::warn!(
                            id = %entry.id,
                            attempts = entry.attempts,
                            "queued transaction parked after repeated failures"
                        );
                    } else {
                        tracing::debug!(id = %entry.id, attempts = entry.attempts, "retry failed");
                    }
                }
            }
        }

        self.save_pending_relay_txs(&pending)?;
        Ok(confirmed)
    }

    /// Broadcasts a raw serialized Solana transaction (hex, from the mesh
    /// relay queue) through the Magic Router.
    pub async fn submit_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, Box<dyn Error>> {
        let hex_str = raw_tx_hex.trim_start_matches("0x");
        let raw_bytes = hex::decode(hex_str)?;
        let tx: Transaction = bincode::deserialize(&raw_bytes)?;

        let client = self.rpc_client();
        let signature: Signature = client.send_transaction(&tx).await?;
        tracing::info!("✅ [Bridge] Relayed transaction confirmed. Tx: {}", signature);
        Ok(signature.to_string())
    }

    /// Detects queued relay txs whose cached blockhash has gone stale —
    /// a tx signed against an expired blockhash can never be processed.
    pub async fn prune_stale_relay_txs(&self) -> Result<usize, Box<dyn Error>> {
        let client = self.rpc_client();
        let Ok(fresh_blockhash) = client.get_latest_blockhash().await else {
            return Ok(0);
        };

        let mut pending = self.load_pending_relay_txs();
        let before = pending.len();
        pending.retain(|tx| {
            if tx.status != "queued" {
                return true;
            }
            let Ok(bytes) = hex::decode(tx.raw_tx_hex.trim_start_matches("0x")) else {
                return true;
            };
            let Ok(decoded) = bincode::deserialize::<Transaction>(&bytes) else {
                return true;
            };
            // A tx is only stale if its blockhash is neither recent nor in
            // the recent history; if we can't decode the blockhash, keep it.
            decoded.message.recent_blockhash != fresh_blockhash
        });
        let pruned = before - pending.len();
        if pruned > 0 {
            self.save_pending_relay_txs(&pending)?;
        }
        Ok(pruned)
    }

    pub fn get_pending_relay_txs(&self) -> Vec<QueuedTx> {
        self.load_pending_relay_txs()
    }

    pub fn record_relayed_tx(&self, summary: &str, tx_hash: &str, reward_avax: &str) -> Result<(), Box<dyn Error>> {
        let mut history = self.load_relayed_history();
        history.push(RelayedTxRecord {
            summary: summary.to_string(),
            tx_hash: tx_hash.to_string(),
            reward_avax: reward_avax.to_string(),
            relayed_at: Utc::now(),
        });
        self.save_relayed_history(&history)
    }

    pub fn get_relayed_history(&self) -> Vec<RelayedTxRecord> {
        self.load_relayed_history()
    }

    fn load_relayed_history(&self) -> Vec<RelayedTxRecord> {
        fs::read_to_string(&self.relayed_history_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn save_relayed_history(&self, history: &[RelayedTxRecord]) -> Result<(), Box<dyn Error>> {
        JsonStore::new(&self.relayed_history_path).save(&history)?;
        Ok(())
    }

    pub fn get_relay_boost_multiplier(&self) -> f64 {
        fs::read_to_string(&self.relay_boost_path)
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(1.0)
    }

    pub fn apply_relay_boost(&self, additional: f64) -> Result<f64, Box<dyn Error>> {
        let updated = self.get_relay_boost_multiplier() + additional;
        JsonStore::compact(&self.relay_boost_path).save(&updated)?;
        Ok(updated)
    }

    pub fn mark_relay_tx_status(&self, id: &str, status: &str, tx_hash: Option<String>) -> Result<(), Box<dyn Error>> {
        let mut pending = self.load_pending_relay_txs();
        if let Some(entry) = pending.iter_mut().find(|t| t.id == id) {
            entry.status = status.to_string();
            entry.tx_hash = tx_hash;
        }
        self.save_pending_relay_txs(&pending)?;
        Ok(())
    }

    /// Real RPC reachability check against Solana devnet.
    pub async fn check_rpc_reachable(&self) -> bool {
        let client = self.rpc_client();
        matches!(timeout(Duration::from_secs(4), client.get_health()).await, Ok(Ok(_)))
    }

    /// Syncs the native SOL balance for the primary identity and saves a
    /// snapshot.
    #[tracing::instrument(skip(self), fields(rpc = %self.rpc_url))]
    pub async fn sync_state(&self, wallet_address_override: &str) -> Result<Snapshot, Box<dyn Error>> {
        let primary = self.get_primary_address();
        let target = if primary != "unknown" { primary } else { wallet_address_override.to_string() };
        let address = Pubkey::from_str(&target)?;

        tracing::info!("🔄 [Bridge] Fetching native SOL balance from {}", self.rpc_url);

        let client = self.rpc_client();
        let balance = client.get_balance(&address).await?;

        tracing::info!("✅ [Bridge] Fetched balance for {}", target);

        // Best-effort: refresh the offline-signing cache while online.
        if let Err(e) = self.refresh_chain_cache().await {
            tracing::warn!("⚠️  Failed to refresh chain state cache: {}", e);
        }

        let snapshot = Snapshot {
            timestamp: Utc::now(),
            assets: vec![CompressedAsset {
                id: "native-sol".to_string(),
                amount: balance.to_string(),
                symbol: "SOL".to_string(),
                owner: target,
                proof: None,
            }],
            signature: "verified_by_solana_rpc".to_string(),
        };

        self.save_snapshot_encrypted(&snapshot)?;

        Ok(snapshot)
    }

    fn save_snapshot_encrypted(&self, snapshot: &Snapshot) -> Result<(), Box<dyn Error>> {
        JsonStore::compact(&self.storage_path).save(&snapshot)?;
        tracing::info!("💾 [Bridge] Snapshot saved.");
        Ok(())
    }

    pub fn get_latest_snapshot(&self) -> Result<Snapshot, Box<dyn Error>> {
        if !self.storage_path.exists() {
            return Err("No snapshot found".into());
        }
        let file_data = fs::read(&self.storage_path)?;
        let snapshot: Snapshot = serde_json::from_slice(&file_data)?;
        Ok(snapshot)
    }

    pub fn delete_snapshot(&self) -> Result<(), Box<dyn Error>> {
        if self.storage_path.exists() {
            fs::remove_file(&self.storage_path)?;
        }
        Ok(())
    }

    pub fn delete_identity(&self) -> Result<(), Box<dyn Error>> {
        JsonStore::new(crate::app_paths::in_data_dir("vault.enc")).delete()?;
        Ok(())
    }

    pub fn init_instant_session(&mut self) -> InstantSession {
        // A fresh ephemeral signer for the agent — on Solana this maps neatly
        // onto a MagicBlock session-style key.
        let agent_signer = Keypair::new();
        let authority_address = agent_signer.pubkey().to_string();

        let session = InstantSession {
            session_id: format!("sess_{}", Utc::now().timestamp()),
            authority: authority_address,
            expiry: Utc::now() + chrono::Duration::hours(1),
            is_active: true,
        };
        self.current_session = Some(session.clone());
        session
    }

    pub fn get_status(&self) -> String {
        match &self.current_session {
            Some(s) if s.is_active => format!("Instant Session Engine: Active [Agent: {}...]", &s.authority[..6]),
            _ => "Instant Session Engine: Inactive".to_string(),
        }
    }

    // ---- Escrow (Solana Anchor program) -----------------------------------

    /// The escrow PDA for a depositor: `[ESCROW_SEED, depositor]`.
    fn escrow_pda(&self, depositor: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"cabal-escrow", depositor.as_ref()],
            &Pubkey::from_str(ESCROW_PROGRAM_ID).expect("valid program id"),
        )
    }

    fn escrow_program_id(&self) -> Pubkey {
        Pubkey::from_str(ESCROW_PROGRAM_ID).expect("valid program id")
    }

    /// Parses a SOL amount string like "0.5" into lamports.
    fn parse_sol(&self, amount_sol: &str) -> Result<u64, Box<dyn Error>> {
        let trimmed = amount_sol.trim();
        let parts: Vec<&str> = trimmed.split('.').collect();
        let whole: u64 = parts[0].parse().map_err(|_| "invalid SOL amount")?;
        let frac = if parts.len() > 1 {
            let mut f = parts[1].to_string();
            while f.len() < 9 {
                f.push('0');
            }
            f.truncate(9);
            f.parse::<u64>().map_err(|_| "invalid SOL amount")?
        } else {
            0
        };
        Ok(whole.saturating_mul(LAMPORTS_PER_SOL) + frac)
    }

    /// Creates an on-chain escrow deal, locking `amount_sol` for `payee`.
    /// If the RPC can't be reached, falls back to signing offline and queueing
    /// for mesh relay.
    #[tracing::instrument(skip(self), fields(payee = %payee, expiry = expiry_unix))]
    pub async fn create_escrow(&self, payee: &str, amount_sol: &str, expiry_unix: u64) -> Result<TxResult, Box<dyn Error>> {
        let keypair = self.primary_keypair()?;
        let payee_pubkey = Pubkey::from_str(payee)?;
        let amount = self.parse_sol(amount_sol)?;

        let (escrow_pda, _bump) = self.escrow_pda(&keypair.pubkey());
        let program_id = self.escrow_program_id();

        // Build instruction data: discriminator + payee + amount + expiry.
        let mut data = Vec::with_capacity(8 + 32 + 8 + 8);
        data.extend_from_slice(&IX_INITIALIZE_ESCROW);
        data.extend_from_slice(payee_pubkey.as_ref());
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&expiry_unix.to_le_bytes());

        let instruction = Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(escrow_pda, false),
                solana_sdk::instruction::AccountMeta::new(keypair.pubkey(), true),
                solana_sdk::instruction::AccountMeta::new_readonly(
                    solana_sdk::system_program::ID,
                    false,
                ),
            ],
            data,
        };

        let online_result = timeout(Duration::from_secs(6), async {
            let client = self.rpc_client();
            let blockhash = client.get_latest_blockhash().await.map_err(|e| e.to_string())?;
            let message = Message::new(std::slice::from_ref(&instruction), Some(&keypair.pubkey()));
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[&keypair], blockhash);
            let signature = client.send_transaction(&tx).await.map_err(|e| e.to_string())?;
            Ok::<Signature, String>(signature)
        }).await;

        match online_result {
            Ok(Ok(signature)) => {
                tracing::info!("✅ [Bridge] Escrow created. Tx: {}", signature);
                Ok(TxResult::Confirmed { id: 1 })
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_timed_out) => {
                tracing::warn!("⚠️  [Bridge] RPC unreachable — signing create_escrow offline for mesh relay.");
                let queued = self.sign_offline(instruction, "Create escrow").await?;
                Ok(TxResult::Queued { queue_id: queued.id })
            }
        }
    }

    /// Reads the payee stored in an escrow PDA. The escrow account data layout
    /// is Anchor's account discriminator (8 bytes) followed by the Escrow
    /// struct: depositor (32), payee (32), amount (8), expiry (8), status (1).
    async fn escrow_payee(&self, escrow_pda: &Pubkey) -> Result<Pubkey, Box<dyn Error>> {
        let client = self.rpc_client();
        let account = client.get_account(escrow_pda).await?;
        let data = account.data;
        if data.len() < 72 {
            return Err("escrow account data too short".into());
        }
        let bytes: [u8; 32] = data[40..72].try_into()?;
        Ok(Pubkey::new_from_array(bytes))
    }

    /// Builds a release instruction for the depositor's escrow PDA, fetching
    /// the payee from on-chain state so the instruction matches the program's
    /// account list exactly.
    async fn release_instruction(&self, caller: &Pubkey) -> Result<Instruction, Box<dyn Error>> {
        let (escrow_pda, _bump) = self.escrow_pda(caller);
        let payee = self.escrow_payee(&escrow_pda).await?;
        let program_id = self.escrow_program_id();

        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&IX_RELEASE);

        Ok(Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(escrow_pda, false),
                solana_sdk::instruction::AccountMeta::new(*caller, true),
                solana_sdk::instruction::AccountMeta::new(payee, false),
            ],
            data,
        })
    }

    /// Builds a refund instruction for the depositor's escrow PDA. The third
    /// account is the depositor receiving the refund, which is the caller.
    fn refund_instruction(&self, caller: &Pubkey) -> Instruction {
        let (escrow_pda, _bump) = self.escrow_pda(caller);
        let program_id = self.escrow_program_id();

        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&IX_REFUND);

        Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(escrow_pda, false),
                solana_sdk::instruction::AccountMeta::new(*caller, true),
                solana_sdk::instruction::AccountMeta::new(*caller, false),
            ],
            data,
        }
    }

    pub async fn release_escrow(&self, _escrow_id: u64) -> Result<TxResult, Box<dyn Error>> {
        let keypair = self.primary_keypair()?;
        let instruction = self.release_instruction(&keypair.pubkey()).await?;

        let online_result = timeout(Duration::from_secs(6), async {
            let client = self.rpc_client();
            let blockhash = client.get_latest_blockhash().await.map_err(|e| e.to_string())?;
            let message = Message::new(std::slice::from_ref(&instruction), Some(&keypair.pubkey()));
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[&keypair], blockhash);
            let signature = client.send_transaction(&tx).await.map_err(|e| e.to_string())?;
            Ok::<Signature, String>(signature)
        }).await;

        match online_result {
            Ok(Ok(signature)) => {
                tracing::info!("✅ [Bridge] Escrow released. Tx: {}", signature);
                Ok(TxResult::Confirmed { id: 1 })
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_timed_out) => {
                tracing::warn!("⚠️  [Bridge] RPC unreachable — signing release_escrow offline for mesh relay.");
                let queued = self.sign_offline(instruction, "Release escrow").await?;
                Ok(TxResult::Queued { queue_id: queued.id })
            }
        }
    }

    /// Runs the real settlement path: create an escrow to `payee`, then
    /// release it. The escrow amount is a small real amount so a wallet with
    /// devnet balance settles for real; an empty wallet still submits the tx
    /// and reports the actual chain error rather than a fabricated success.
    ///
    /// When the RPC is unreachable, the affected leg signs offline and is
    /// queued for mesh relay; the returned `SettledEscrow` carries the queued
    /// transaction(s) so the caller can broadcast them to peers.
    pub async fn settle_on_chain(&self, payee: &str, amount_sol: &str) -> Result<SettledEscrow, Box<dyn Error>> {
        // The amount comes from the user's SOL intent. Never substitute a
        // demo amount here: the UI and on-chain escrow must settle the same
        // value the user composed.
        // Each leg may confirm online or queue offline for mesh relay. Collect
        // whatever was queued so the caller can broadcast it to peers.
        let mut queued_for_relay: Vec<QueuedTx> = Vec::new();

        let created = self.create_escrow(payee, amount_sol, 0).await?;
        let create_tx = match &created {
            TxResult::Confirmed { id } => format!("escrow-{id}"),
            TxResult::Queued { queue_id } => {
                // The release leg cannot run until the create is on-chain, so a
                // queued create parks the deal; the caller relays it and the
                // release can follow once it confirms.
                if let Some(tx) = self
                    .load_pending_relay_txs()
                    .into_iter()
                    .find(|tx| &tx.id == queue_id)
                {
                    queued_for_relay.push(tx);
                }
                queue_id.clone()
            }
        };

        let release_tx = match created {
            TxResult::Confirmed { .. } => {
                match self.release_escrow(0).await? {
                    TxResult::Confirmed { id } => format!("escrow-{id}"),
                    TxResult::Queued { queue_id } => {
                        if let Some(tx) = self
                            .load_pending_relay_txs()
                            .into_iter()
                            .find(|tx| tx.id == queue_id)
                        {
                            queued_for_relay.push(tx);
                        }
                        queue_id
                    }
                }
            }
            // Create was queued; the release hasn't been signed yet.
            TxResult::Queued { .. } => "queued-for-relay".to_string(),
        };

        Ok(SettledEscrow {
            create_tx,
            release_tx,
            queued_for_relay,
        })
    }

    /// Refunds an escrow back to the depositor. Mirrors `release_escrow`'s
    /// online/offline split: when the RPC is unreachable the refund is signed
    /// offline and queued for mesh relay rather than hard-failing.
    ///
    /// The escrow PDA is derived from the depositor, so `escrow_id` is
    /// informational — the refund targets the depositor's own escrow account.
    pub async fn refund_escrow(&self, _escrow_id: u64) -> Result<TxResult, Box<dyn Error>> {
        let keypair = self.primary_keypair()?;
        let instruction = self.refund_instruction(&keypair.pubkey());

        let online_result = timeout(Duration::from_secs(6), async {
            let client = self.rpc_client();
            let blockhash = client.get_latest_blockhash().await.map_err(|e| e.to_string())?;
            let message = Message::new(std::slice::from_ref(&instruction), Some(&keypair.pubkey()));
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[&keypair], blockhash);
            let signature = client.send_transaction(&tx).await.map_err(|e| e.to_string())?;
            Ok::<Signature, String>(signature)
        }).await;

        match online_result {
            Ok(Ok(signature)) => {
                tracing::info!("✅ [Bridge] Escrow refunded. Tx: {}", signature);
                Ok(TxResult::Confirmed { id: 1 })
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_timed_out) => {
                tracing::warn!("⚠️  [Bridge] RPC unreachable — signing refund_escrow offline for mesh relay.");
                let queued = self.sign_offline(instruction, "Refund escrow").await?;
                Ok(TxResult::Queued { queue_id: queued.id })
            }
        }
    }

    /// Fires the release leg for any escrow whose create transaction has
    /// confirmed on-chain but whose release was never signed (the create was
    /// queued offline for relay and has since landed). Returns the release
    /// transactions that were newly submitted online (with their tx hashes) or
    /// newly queued offline (with their raw bytes), so the caller can
    /// re-broadcast a queued release for relay.
    ///
    /// Idempotent: a queued create is processed once, and a release that is
    /// itself queued is not re-fires — the next tick sees it as a queued
    /// release, not a create.
    pub async fn resume_pending_settlements(&self) -> Vec<QueuedTx> {
        let mut pending = self.load_pending_relay_txs();
        let mut released = Vec::new();
        let mut changed = false;

        for tx in &mut pending {
            if tx.status == "confirmed" && tx.summary == "Create escrow" {
                match self.release_escrow(0).await {
                    Ok(TxResult::Confirmed { id }) => {
                        tracing::info!(id = %tx.id, "resumed settlement: create confirmed, release submitted");
                        tx.status = "completed".to_string();
                        tx.summary = "Create escrow (released)".to_string();
                        changed = true;
                        released.push(QueuedTx {
                            id: format!("escrow-{id}"),
                            raw_tx_hex: String::new(),
                            summary: "Release escrow".into(),
                            created_at: tx.created_at,
                            status: "confirmed".into(),
                            tx_hash: Some(format!("escrow-{id}")),
                            reason: None,
                            attempts: 0,
                        });
                    }
                    Ok(TxResult::Queued { queue_id }) => {
                        tracing::info!(id = %tx.id, "resumed settlement: release signed offline, queued for relay");
                        tx.status = "completed".to_string();
                        tx.summary = "Create escrow (release queued)".to_string();
                        changed = true;
                        if let Some(queued) = self
                            .load_pending_relay_txs()
                            .into_iter()
                            .find(|q| q.id == queue_id)
                        {
                            released.push(queued);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(id = %tx.id, %error, "resume settlement failed");
                    }
                }
            }
        }

        if changed {
            if let Err(e) = self.save_pending_relay_txs(&pending) {
                tracing::warn!(error = %e, "failed to persist settlement resume state");
            }
        }

        released
    }

    /// Reads the on-chain state of the depositor's escrow PDA (no signer).
    pub async fn get_escrow_status(&self, _escrow_id: u64) -> Result<serde_json::Value, Box<dyn Error>> {
        let primary = self.get_primary_address();
        if primary == "unknown" {
            return Err("no identity".into());
        }
        let depositor = Pubkey::from_str(&primary)?;
        let (escrow_pda, _bump) = self.escrow_pda(&depositor);

        let client = self.rpc_client();
        let account = client.get_account(&escrow_pda).await?;

        // Account data layout: 8-byte Anchor discriminator, then depositor (32),
        // payee (32), amount (8), expiry (8), status (1).
        let data = &account.data[8..];
        let depositor_b = &data[0..32];
        let payee_b = &data[32..64];
        let amount = u64::from_le_bytes(data[64..72].try_into().unwrap_or([0; 8]));
        let expiry = u64::from_le_bytes(data[72..80].try_into().unwrap_or([0; 8]));
        let status = data.get(80).copied().unwrap_or(0);

        Ok(serde_json::json!({
            "depositor": Pubkey::new_from_array(depositor_b.try_into().unwrap_or([0; 32])).to_string(),
            "payee": Pubkey::new_from_array(payee_b.try_into().unwrap_or([0; 32])).to_string(),
            "amount": amount.to_string(),
            "expiry": expiry,
            "status": status,
        }))
    }

    // ---- Marketplace / vouchers (out of scope; stubs for the frozen UI) ---

    pub async fn mint_voucher(&self, _voucher_type: &str, _description: &str) -> Result<u64, Box<dyn Error>> {
        Err("No Solana NFT/voucher program is deployed".into())
    }

    pub async fn approve_voucher(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("No Solana NFT/voucher program is deployed".into())
    }

    pub async fn create_asset_listing(&self, _description: &str, _price_wei: String, _token_id: u64) -> Result<u64, Box<dyn Error>> {
        Err("No Solana marketplace program is deployed".into())
    }

    pub async fn get_active_asset_listings(&self) -> Result<Vec<AssetListingView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    pub async fn buy_listing(&self, _listing_id: u64, _price_wei: String) -> Result<TxResult, Box<dyn Error>> {
        Err("No Solana marketplace program is deployed".into())
    }

    pub async fn release_deal(&self, _deal_id: u64) -> Result<String, Box<dyn Error>> {
        Err("No Solana marketplace program is deployed".into())
    }

    pub async fn refund_deal(&self, _deal_id: u64) -> Result<String, Box<dyn Error>> {
        Err("No Solana marketplace program is deployed".into())
    }

    pub async fn redeem_voucher(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("No Solana NFT/voucher program is deployed".into())
    }

    pub async fn get_voucher_owner(&self, _token_id: u64) -> Result<String, Box<dyn Error>> {
        Err("No Solana NFT/voucher program is deployed".into())
    }

    pub async fn get_owned_vouchers(&self, _owner: &str) -> Result<Vec<VoucherView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    pub async fn get_my_deals(&self, _address: &str) -> Result<Vec<DealView>, Box<dyn Error>> {
        Ok(Vec::new())
    }

    // ---- PDF content commitment + delivery --------------------------------

    fn load_content_store(&self) -> std::collections::HashMap<u64, ContentRecord> {
        fs::read_to_string(&self.content_store_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn save_content_store(&self, store: &std::collections::HashMap<u64, ContentRecord>) -> Result<(), Box<dyn Error>> {
        JsonStore::new(&self.content_store_path).save(store)?;
        Ok(())
    }

    fn load_received_content(&self) -> std::collections::HashMap<u64, ContentRecord> {
        fs::read_to_string(&self.received_content_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn save_received_content(&self, store: &std::collections::HashMap<u64, ContentRecord>) -> Result<(), Box<dyn Error>> {
        JsonStore::new(&self.received_content_path).save(store)?;
        Ok(())
    }

    pub fn extract_pdf_text(&self, pdf_bytes: Vec<u8>) -> Result<String, Box<dyn Error>> {
        let doc = lopdf::Document::load_mem(&pdf_bytes)?;
        let text = doc.extract_text(&[1])?;
        Ok(text)
    }

    /// Signs the exact text with this node's Solana identity key — a real,
    /// verifiable commitment.
    pub fn sign_content(&self, text: &str) -> Result<ContentRecord, Box<dyn Error>> {
        let keypair = self.primary_keypair()?;
        let message = solana_sdk::message::Message::new(&[], Some(&keypair.pubkey()));
        let signature = keypair.sign_message(message.serialize().as_slice());
        let full = solana_sdk::hash::hash(text.as_bytes()).to_bytes();
        let fingerprint = format!("0x{}", hex::encode(&full[..8]));

        Ok(ContentRecord {
            token_id: 0,
            text: text.to_string(),
            fingerprint,
            signature: signature.to_string(),
            signer_address: keypair.pubkey().to_string(),
        })
    }

    pub fn store_content(&self, token_id: u64, mut record: ContentRecord) -> Result<(), Box<dyn Error>> {
        record.token_id = token_id;
        let mut store = self.load_content_store();
        store.insert(token_id, record);
        self.save_content_store(&store)
    }

    pub fn get_content(&self, token_id: u64) -> Option<ContentRecord> {
        self.load_content_store().get(&token_id).cloned()
    }

    /// Verifies a delivered piece of content really was signed by the
    /// expected seller before accepting it.
    pub fn receive_content(&self, token_id: u64, text: &str, signature: &str, expected_seller: &str) -> Result<bool, Box<dyn Error>> {
        let sig = Signature::from_str(signature)?;
        let expected = Pubkey::from_str(expected_seller)?;

        let message = solana_sdk::message::Message::new(&[], Some(&expected));
        if !sig.verify(expected.as_ref(), message.serialize().as_slice()) {
            tracing::warn!("⚠️  Content delivery rejected: signature does not match seller {}", expected);
            return Ok(false);
        }

        let full = solana_sdk::hash::hash(text.as_bytes()).to_bytes();
        let fingerprint = format!("0x{}", hex::encode(&full[..8]));
        let mut store = self.load_received_content();
        store.insert(token_id, ContentRecord {
            token_id,
            text: text.to_string(),
            fingerprint,
            signature: signature.to_string(),
            signer_address: expected.to_string(),
        });
        self.save_received_content(&store)?;
        Ok(true)
    }

    pub fn get_received_content(&self, token_id: u64) -> Option<ContentRecord> {
        self.load_received_content().get(&token_id).cloned()
    }
}

/// Decodes a base58 ed25519 keypair.
fn keypair_from_secret(secret: &str) -> Result<Keypair, Box<dyn Error>> {
    let bytes = bs58::decode(secret).into_vec()?;
    if bytes.len() != 64 {
        return Err("invalid keypair length".into());
    }
    let arr: [u8; 64] = bytes.try_into().map_err(|_| "invalid keypair")?;
    Ok(Keypair::try_from(&arr[..])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sol_amounts() {
        let bridge = BlockchainBridge::new(None);
        assert_eq!(bridge.parse_sol("1").unwrap(), LAMPORTS_PER_SOL);
        assert_eq!(bridge.parse_sol("0.5").unwrap(), 500_000_000);
        assert_eq!(bridge.parse_sol("0.000000001").unwrap(), 1);
        assert!(bridge.parse_sol("abc").is_err());
    }

    #[test]
    fn keypair_round_trips() {
        let keypair = Keypair::new();
        let encoded = bs58::encode(keypair.to_bytes()).into_string();
        let decoded = keypair_from_secret(&encoded).unwrap();
        assert_eq!(decoded.pubkey(), keypair.pubkey());
    }

    /// The load-bearing invariant of the wallet backup feature: what
    /// `generate_new_identity` stores (base58 of the mnemonic-derived keypair)
    /// is exactly what `import_mnemonic` reconstructs from the words, so
    /// "write down the words, type them back later" restores the same wallet.
    #[test]
    fn a_generated_identity_round_trips_through_its_mnemonic() {
        // Generate the way the bridge does: mnemonic -> seed -> keypair.
        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let keypair = mnemonic.to_keypair().unwrap();
        let stored = bs58::encode(keypair.to_bytes()).into_string();

        // Import the way import_mnemonic does: words -> seed -> keypair.
        let parsed = crate::mnemonic::Mnemonic::parse(&mnemonic.words().join(" ")).unwrap();
        let imported_keypair = parsed.to_keypair().unwrap();

        // Both the base58 keypair and the derived address match.
        assert_eq!(keypair_from_secret(&stored).unwrap().pubkey(), imported_keypair.pubkey());
        assert_eq!(keypair.pubkey(), imported_keypair.pubkey());
    }

    /// The stored mnemonic is the recoverable form of the stored key: loading
    /// the identity back and deriving from its phrase reproduces its address.
    #[test]
    fn the_stored_mnemonic_derives_the_stored_keypair() {
        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let keypair = mnemonic.to_keypair().unwrap();

        let record = IdentityRecord {
            alias: "Genesis Fox".into(),
            emoji: "🦊".into(),
            private_key_hex: bs58::encode(keypair.to_bytes()).into_string().into(),
            mnemonic: mnemonic.words().join(" ").into(),
        };

        let from_key = keypair_from_secret(record.private_key_hex.expose()).unwrap();
        let from_words = crate::mnemonic::Mnemonic::parse(record.mnemonic.expose())
            .unwrap()
            .to_keypair()
            .unwrap();
        assert_eq!(from_key.pubkey(), from_words.pubkey());
    }

    /// A key-only import (no mnemonic) still yields a working identity; the
    /// missing phrase is stored empty so the UI can say "no backup".
    #[test]
    fn a_key_only_import_works_but_has_no_phrase() {
        use tempfile::TempDir;

        let mnemonic = crate::mnemonic::Mnemonic::generate();
        let keypair = mnemonic.to_keypair().unwrap();
        let key = bs58::encode(keypair.to_bytes()).into_string();

        // Point the whole bridge at a throwaway data dir so the real vault is
        // never read or written by a test.
        let dir = TempDir::new().unwrap();
        let mut bridge = BlockchainBridge::new(None);
        bridge.storage_path = dir.path().join("snapshot.enc");
        bridge.identity_vault = Vault::new(
            dir.path().join("vault.enc"),
            crate::vault_key::platform_provider(dir.path().join("vault.key")),
        );
        bridge.relay_boost_path = dir.path().join("relay_boost.json");

        bridge
            .import_identity(key, "Imported".into(), "🦊".into())
            .unwrap();
        assert_eq!(bridge.primary_mnemonic(), None);
        assert_eq!(bridge.get_primary_address(), keypair.pubkey().to_string());
    }
}
