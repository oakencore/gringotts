use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug)]
#[allow(dead_code)]
pub struct TokenBalance {
    pub coin_type: String,
    pub symbol: Option<String>,
    pub decimals: u8,
    pub ui_amount: f64,
    pub usd_price: Option<f64>,
    pub usd_value: Option<f64>,
}

#[derive(Debug)]
pub struct AccountBalances {
    pub sui_balance: f64,
    pub sui_usd_price: Option<f64>,
    pub sui_usd_value: Option<f64>,
    #[allow(dead_code)]
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

pub struct SuiClient {
    client: reqwest::Client,
    rpc_url: String,
}

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: serde_json::Value,
    id: u64,
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

impl SuiClient {
    pub fn new(rpc_url: Option<String>) -> Self {
        let url = rpc_url.unwrap_or_else(|| "https://fullnode.mainnet.sui.io:443".to_string());
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
            id: 1,
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
        // Validate address format (Sui addresses are 0x-prefixed hex, 64 chars after 0x)
        if !address.starts_with("0x") {
            anyhow::bail!("Invalid Sui address format: must start with 0x");
        }

        // Query SUI balance using suix_getBalance
        let result = self
            .rpc_call(
                "suix_getBalance",
                json!([address, "0x2::sui::SUI"]),
            )
            .await?;

        // Parse balance from MIST (10^9 MIST = 1 SUI)
        let balance_str = result
            .get("totalBalance")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid balance format"))?;

        let balance_mist: u64 = balance_str
            .parse()
            .context("Failed to parse SUI balance")?;

        // Convert MIST to SUI (1 SUI = 10^9 MIST)
        let sui_balance = balance_mist as f64 / 1_000_000_000.0;

        // Token balances would require additional queries
        // For now, we'll just return the native SUI balance
        let token_balances = Vec::new();

        Ok(AccountBalances {
            sui_balance,
            sui_usd_price: None,
            sui_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }
}
