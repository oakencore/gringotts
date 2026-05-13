use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug)]
#[allow(dead_code)]
pub struct TokenBalance {
    pub contract_address: String,
    pub symbol: Option<String>,
    pub decimals: u8,
    pub ui_amount: f64,
    pub usd_price: Option<f64>,
    pub usd_value: Option<f64>,
}

#[derive(Debug)]
pub struct AccountBalances {
    pub near_balance: f64,
    pub near_usd_price: Option<f64>,
    pub near_usd_value: Option<f64>,
    #[allow(dead_code)]
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

pub struct NearClient {
    client: reqwest::Client,
    rpc_url: String,
}

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: serde_json::Value,
    id: String,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    message: String,
}

impl NearClient {
    pub fn new(rpc_url: Option<String>) -> Self {
        let url = rpc_url.unwrap_or_else(|| "https://rpc.mainnet.near.org".to_string());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            rpc_url: url,
        }
    }

    async fn rpc_call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: "dontcare".to_string(),
        };

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .context("Failed to send RPC request")?;

        let rpc_response: JsonRpcResponse = response
            .json()
            .await
            .context("Failed to parse RPC response")?;

        if let Some(error) = rpc_response.error {
            anyhow::bail!("RPC error: {}", error.message);
        }

        rpc_response
            .result
            .ok_or_else(|| anyhow::anyhow!("No result in RPC response"))
    }

    pub async fn get_balances(&self, address: &str) -> Result<AccountBalances> {
        // Query account info to get NEAR balance
        let result = self
            .rpc_call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": address
                }),
            )
            .await?;

        // Parse balance from yoctoNEAR (10^24 yoctoNEAR = 1 NEAR)
        let balance_str = result
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid balance format"))?;

        let balance_yocto: u128 = balance_str
            .parse()
            .context("Failed to parse NEAR balance")?;

        // Convert yoctoNEAR to NEAR (1 NEAR = 10^24 yoctoNEAR)
        let near_balance = balance_yocto as f64 / 1_000_000_000_000_000_000_000_000.0;

        // Token balances for NEAR (NEP-141 tokens) would require additional contract calls
        // For now, we'll just return the native NEAR balance
        let token_balances = Vec::new();

        Ok(AccountBalances {
            near_balance,
            near_usd_price: None,
            near_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }
}

// Implement PriceEnrichable trait for NEAR balances
impl crate::PriceEnrichable for AccountBalances {
    const NATIVE_SYMBOL: &'static str = "NEAR";

    fn native_balance(&self) -> f64 {
        self.near_balance
    }

    fn set_native_usd_price(&mut self, price: f64) {
        self.near_usd_price = Some(price);
    }

    fn set_native_usd_value(&mut self, value: f64) {
        self.near_usd_value = Some(value);
    }

    fn set_total_usd_value(&mut self, value: f64) {
        self.total_usd_value = Some(value);
    }

    // NEAR doesn't have token balances yet, use default implementation
}
