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
    pub eth_balance: f64,
    pub eth_usd_price: Option<f64>,
    pub eth_usd_value: Option<f64>,
    #[allow(dead_code)]
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

pub struct StarknetClient {
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

impl StarknetClient {
    pub fn new(rpc_url: Option<String>) -> Self {
        // Use free public RPC from Nethermind (Blast API is no longer available)
        let url = rpc_url.unwrap_or_else(|| "https://free-rpc.nethermind.io/mainnet-juno".to_string());
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
        // Validate address format (Starknet addresses are 0x-prefixed hex)
        if !address.starts_with("0x") {
            anyhow::bail!("Invalid Starknet address format: must start with 0x");
        }

        // ETH contract address on Starknet
        let eth_contract = "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";

        // Query ETH balance using starknet_call
        // Call balanceOf(address) function
        let result = self
            .rpc_call(
                "starknet_call",
                json!({
                    "request": {
                        "contract_address": eth_contract,
                        "entry_point_selector": "0x2e4263afad30923c891518314c3c95dbe830a16874e8abc5777a9a20b54c76e", // balanceOf selector
                        "calldata": [address]
                    },
                    "block_id": "latest"
                }),
            )
            .await?;

        // Parse balance result
        let balance_hex = result
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid balance format"))?;

        // Parse hex string to u128
        let balance_wei = u128::from_str_radix(
            balance_hex.trim_start_matches("0x"),
            16
        ).unwrap_or(0);

        // Convert wei to ETH (1 ETH = 10^18 wei)
        let eth_balance = balance_wei as f64 / 1_000_000_000_000_000_000.0;

        // Token balances would require additional contract calls
        // For now, we'll just return the native ETH balance
        let token_balances = Vec::new();

        Ok(AccountBalances {
            eth_balance,
            eth_usd_price: None,
            eth_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }
}

// Implement PriceEnrichable trait for Starknet balances
impl crate::PriceEnrichable for AccountBalances {
    const NATIVE_SYMBOL: &'static str = "ETH";

    fn native_balance(&self) -> f64 {
        self.eth_balance
    }

    fn set_native_usd_price(&mut self, price: f64) {
        self.eth_usd_price = Some(price);
    }

    fn set_native_usd_value(&mut self, value: f64) {
        self.eth_usd_value = Some(value);
    }

    fn set_total_usd_value(&mut self, value: f64) {
        self.total_usd_value = Some(value);
    }

    // Starknet doesn't have token balances yet, use default implementation
}
