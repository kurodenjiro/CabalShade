use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error;

use crate::blockchain_bridge::AssetListingView;
use cabal_core::{Condition, IntentDraft, UsdPrice};

#[derive(Debug, Serialize, Deserialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    system: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub seller: String,
    pub description: String,
    /// Listing price in native SOL, never AVAX/wei.
    pub price_sol: String,
    pub reason: String,
}

/// A local-LLM proposal for a directly matched mesh order. The proposal is
/// informational until the deterministic condition checks below accept it.
#[derive(Debug, Clone)]
pub struct TradeNegotiation {
    pub accepted: bool,
    pub price: Option<UsdPrice>,
}

/// Matches a buyer's free-text intent against real on-chain listings using
/// the same local Ollama model the Shark negotiation agent already uses.
pub struct MatchAgent {
    client: Client,
    ollama_url: Option<String>,
}

impl MatchAgent {
    pub fn new(ollama_url: Option<String>) -> Self {
        MatchAgent {
            client: Client::new(),
            ollama_url,
        }
    }

    /// Resolved per request rather than cached at construction, so a URL set at
    /// runtime — the only way to reach a model on iOS — applies without a restart.
    fn url(&self) -> String {
        self.ollama_url
            .clone()
            .unwrap_or_else(crate::ollama_config::url)
    }

    pub async fn match_intent(
        &self,
        intent: &str,
        price_ceiling: f64,
        listings: &[AssetListingView],
    ) -> Result<Option<MatchResult>, Box<dyn Error>> {
        if listings.is_empty() {
            return Ok(None);
        }

        let catalog = listings
            .iter()
            .map(|l| format!("- id {}: \"{}\" — {} SOL", l.id, l.description, l.price_avax))
            .collect::<Vec<_>>()
            .join("\n");

        let system_prompt = format!(
            r#"You match a buyer's intent to the single best listing from a marketplace catalog.

RULES:
1. Only choose a listing if it genuinely matches what the buyer is looking for.
2. If nothing matches well, return matched_id: null.
3. Never reason about price limits yourself — that is checked separately.
4. Respond ONLY with JSON in this exact format:
{{
  "matched_id": <listing id number or null>,
  "reason": "<brief explanation>"
}}

Catalog:
{}
"#,
            catalog
        );

        let request = OllamaRequest {
            model: "qwen2.5:0.5b".to_string(),
            prompt: format!("Buyer intent: {}", intent),
            stream: false,
            system: Some(system_prompt),
        };

        let response = self
            .client
            .post(format!("{}/api/generate", self.url()))
            .json(&request)
            .send()
            .await?;

        let ollama_response: OllamaResponse = response.json().await?;
        let json_slice = crate::llm_json::extract_json_object(&ollama_response.response);

        let parsed: serde_json::Value = serde_json::from_str(json_slice)
            .unwrap_or_else(|_| serde_json::json!({ "matched_id": null, "reason": "No match" }));

        // The model frequently emits JSON-shaped-but-invalid output (e.g.
        // `"matched_id": id 3` instead of `3`), which fails strict parsing
        // above and silently looks like "no match" even though the model
        // did pick something — recover the id with a looser scan before
        // giving up.
        let matched_id = match parsed["matched_id"].as_u64() {
            Some(id) => id,
            None => match crate::llm_json::recover_number_field(json_slice, "matched_id") {
                Some(id) => id,
                None => return Ok(None),
            },
        };

        let listing = match listings.iter().find(|l| l.id == matched_id) {
            Some(l) => l,
            None => return Ok(None), // model hallucinated an id that doesn't exist
        };

        // Never trust the model on money — verify the price ceiling ourselves.
        let price: f64 = listing.price_avax.parse().unwrap_or(f64::MAX);
        if price > price_ceiling {
            println!(
                "⚠️  Match rejected: listing {} price {} SOL exceeds ceiling {} SOL",
                matched_id, price, price_ceiling
            );
            return Ok(None);
        }

        Ok(Some(MatchResult {
            seller: listing.seller.clone(),
            description: listing.description.clone(),
            price_sol: listing.price_avax.clone(),
            reason: parsed["reason"].as_str().unwrap_or("Matched").to_string(),
        }))
    }

    /// Calls the local Ollama API to negotiate a price for an already matched
    /// BUY/SELL pair. The model never authorizes a transfer: its output is
    /// parsed and bounded by the orders' declared price conditions.
    pub async fn negotiate_trade(
        &self,
        local: &IntentDraft,
        remote: &IntentDraft,
    ) -> Result<TradeNegotiation, Box<dyn Error>> {
        let request = OllamaRequest {
            model: "qwen2.5:0.5b".to_string(),
            prompt: format!(
                "Local order: {}\nRemote order: {}",
                serde_json::to_string(local)?,
                serde_json::to_string(remote)?,
            ),
            stream: false,
            system: Some(
                "You negotiate one matched SOL/USDC order. Respond ONLY JSON: {\"decision\":\"accept\"|\"reject\",\"price_usdc\":\"decimal\"}. The price is USDC per SOL. Accept only when amounts and conditions are compatible."
                    .to_string(),
            ),
        };
        let response = self
            .client
            .post(format!("{}/api/generate", self.url()))
            .json(&request)
            .send()
            .await?;
        let body: OllamaResponse = response.json().await?;
        let json = crate::llm_json::extract_json_object(&body.response);
        let value: serde_json::Value = serde_json::from_str(json)?;
        let accepted = value["decision"].as_str() == Some("accept");
        let price = value["price_usdc"]
            .as_str()
            .and_then(|raw| UsdPrice::parse(raw).ok())
            .filter(|candidate| condition_allows(&local.condition, *candidate))
            .filter(|candidate| condition_allows(&remote.condition, *candidate));
        Ok(TradeNegotiation { accepted, price })
    }
}

fn condition_allows(condition: &Condition, candidate: UsdPrice) -> bool {
    match condition {
        Condition::Under { price } => candidate <= *price,
        Condition::Above { price } => candidate >= *price,
        Condition::Any => true,
    }
}
