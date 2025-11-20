use anyhow::{Context, Result};
use crate::storage::Chain;
use serde::{Deserialize, Serialize};
use serde_json::json;

fn get_default_rpc_url(chain: &Chain) -> Result<&'static str> {
    match chain {
        Chain::Ethereum => Ok("https://eth.llamarpc.com"),
        Chain::Polygon => Ok("https://polygon-rpc.com"),
        Chain::BinanceSmartChain => Ok("https://bsc-dataseed.binance.org"),
        Chain::Arbitrum => Ok("https://arb1.arbitrum.io/rpc"),
        Chain::Optimism => Ok("https://mainnet.optimism.io"),
        Chain::Avalanche => Ok("https://api.avax.network/ext/bc/C/rpc"),
        Chain::Base => Ok("https://mainnet.base.org"),
        Chain::Core => Ok("https://rpc.coredao.org"),
        _ => anyhow::bail!("Chain {:?} is not an EVM chain", chain),
    }
}

// Common ERC20 tokens by chain
fn get_common_tokens(chain: &Chain) -> Vec<(&'static str, &'static str)> {
    match chain {
        Chain::Ethereum => vec![
            ("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "USDC"),
            ("0xdAC17F958D2ee523a2206206994597C13D831ec7", "USDT"),
            ("0x6B175474E89094C44Da98b954EedeAC495271d0F", "DAI"),
        ],
        Chain::Polygon => vec![
            ("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359", "USDC"),
            ("0xc2132D05D31c914a87C6611C10748AEb04B58e8F", "USDT"),
            ("0x8f3Cf7ad23Cd3CaDbD9735AFf958023239c6A063", "DAI"),
        ],
        Chain::BinanceSmartChain => vec![
            ("0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d", "USDC"),
            ("0x55d398326f99059fF775485246999027B3197955", "USDT"),
            ("0x1AF3F329e8BE154074D8769D1FFa4eE058B1DBc3", "DAI"),
        ],
        Chain::Arbitrum => vec![
            ("0xaf88d065e77c8cC2239327C5EDb3A432268e5831", "USDC"),
            ("0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9", "USDT"),
            ("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1", "DAI"),
        ],
        Chain::Optimism => vec![
            ("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85", "USDC"),
            ("0x94b008aA00579c1307B0EF2c499aD98a8ce58e58", "USDT"),
            ("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1", "DAI"),
        ],
        Chain::Avalanche => vec![
            ("0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E", "USDC"),
            ("0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7", "USDT"),
            ("0xd586E7F844cEa2F87f50152665BCbc2C279D8d70", "DAI"),
        ],
        Chain::Base => vec![
            ("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "USDC"),
            ("0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb", "DAI"),
        ],
        Chain::Core => vec![
            ("0xa4151B2B3e269645181dCcF2D426cE75fcbDeca9", "USDT"),
            ("0x900101d06A7426441Ae63e9AB3B9b0F63Be145F1", "USDC"),
        ],
        _ => vec![],
    }
}

#[derive(Debug)]
pub struct TokenBalance {
    pub contract_address: String,
    pub name: Option<String>,
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
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

pub struct EvmClient {
    client: reqwest::Client,
    rpc_url: String,
    chain: Chain,
}

#[derive(Serialize, Deserialize)]
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

impl EvmClient {
    pub fn new(rpc_url: Option<String>, chain: Chain) -> Result<Self> {
        let url = match rpc_url {
            Some(u) => u,
            None => get_default_rpc_url(&chain)?.to_string(),
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Ok(Self {
            client,
            rpc_url: url,
            chain,
        })
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
            // Check if it's a rate limit error
            if error.message.contains("rate limit") || error.message.contains("too many requests") {
                anyhow::bail!("Rate limit exceeded. Try again in a moment or use a custom RPC URL with --rpc-url");
            }
            anyhow::bail!("RPC error: {}", error.message);
        }

        rpc_response
            .result
            .ok_or_else(|| anyhow::anyhow!("No result in RPC response"))
    }

    pub async fn get_balances(&self, address: &str) -> Result<AccountBalances> {
        // Validate EVM address format
        if !address.starts_with("0x") || address.len() != 42 {
            anyhow::bail!("Invalid EVM address format");
        }

        // Get ETH balance
        let balance_hex = self
            .rpc_call("eth_getBalance", json!([address, "latest"]))
            .await?;

        let balance_str = balance_hex
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid balance format"))?;

        // Convert hex string to u128
        let balance_wei = u128::from_str_radix(
            balance_str.trim_start_matches("0x"),
            16
        ).context("Failed to parse balance")?;

        // Convert wei to ETH (1 ETH = 10^18 wei)
        let eth_balance = balance_wei as f64 / 1_000_000_000_000_000_000.0;

        // Query ERC20 token balances
        let mut token_balances = Vec::new();
        let common_tokens = get_common_tokens(&self.chain);

        for (token_address, symbol) in common_tokens {
            // Add delay between token queries to avoid rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            match self.query_erc20_balance(address, token_address).await {
                Ok(Some(token_balance)) => {
                    token_balances.push(token_balance);
                }
                Ok(None) => {
                    // Token balance is zero, skip
                }
                Err(e) => {
                    eprintln!("Warning: Failed to query {} balance: {}", symbol, e);
                }
            }
        }

        Ok(AccountBalances {
            eth_balance,
            eth_usd_price: None,
            eth_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }

    async fn query_erc20_balance(&self, wallet_address: &str, token_address: &str) -> Result<Option<TokenBalance>> {
        // ERC20 balanceOf(address) function signature
        let balance_of_sig = "0x70a08231";

        // Encode the wallet address (remove 0x prefix, pad to 32 bytes)
        let wallet_addr_clean = wallet_address.trim_start_matches("0x");
        let padded_address = format!("{:0>64}", wallet_addr_clean);
        let data = format!("{}{}", balance_of_sig, padded_address);

        // Call eth_call
        let result = self.rpc_call("eth_call", json!([
            {
                "to": token_address,
                "data": data
            },
            "latest"
        ])).await?;

        let balance_hex = result
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid balance format"))?;

        // Parse balance
        let balance_u256 = u128::from_str_radix(
            balance_hex.trim_start_matches("0x"),
            16
        ).unwrap_or(0);

        // If balance is zero, return None
        if balance_u256 == 0 {
            return Ok(None);
        }

        // Query token metadata with delays between calls
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let decimals = self.query_erc20_decimals(token_address).await?;

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let name = self.query_erc20_name(token_address).await.ok();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let symbol = self.query_erc20_symbol(token_address).await.ok();

        // Calculate UI amount
        let divisor = 10_u128.pow(decimals as u32) as f64;
        let ui_amount = balance_u256 as f64 / divisor;

        Ok(Some(TokenBalance {
            contract_address: token_address.to_string(),
            name,
            symbol,
            decimals,
            ui_amount,
            usd_price: None,
            usd_value: None,
        }))
    }

    async fn query_erc20_decimals(&self, token_address: &str) -> Result<u8> {
        // decimals() function signature
        let decimals_sig = "0x313ce567";

        let result = self.rpc_call("eth_call", json!([
            {
                "to": token_address,
                "data": decimals_sig
            },
            "latest"
        ])).await?;

        let decimals_hex = result
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid decimals format"))?;

        let decimals = u8::from_str_radix(
            decimals_hex.trim_start_matches("0x"),
            16
        ).unwrap_or(18);

        Ok(decimals)
    }

    async fn query_erc20_name(&self, token_address: &str) -> Result<String> {
        // name() function signature
        let name_sig = "0x06fdde03";

        let result = self.rpc_call("eth_call", json!([
            {
                "to": token_address,
                "data": name_sig
            },
            "latest"
        ])).await?;

        let name_hex = result
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid name format"))?;

        // Decode string from hex (simplified - just try to decode UTF-8)
        let name = self.decode_string_from_hex(name_hex)?;
        Ok(name)
    }

    async fn query_erc20_symbol(&self, token_address: &str) -> Result<String> {
        // symbol() function signature
        let symbol_sig = "0x95d89b41";

        let result = self.rpc_call("eth_call", json!([
            {
                "to": token_address,
                "data": symbol_sig
            },
            "latest"
        ])).await?;

        let symbol_hex = result
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid symbol format"))?;

        // Decode string from hex
        let symbol = self.decode_string_from_hex(symbol_hex)?;
        Ok(symbol)
    }

    fn decode_string_from_hex(&self, hex: &str) -> Result<String> {
        let hex_clean = hex.trim_start_matches("0x");

        // Skip the first 64 characters (offset and length encoding)
        if hex_clean.len() < 128 {
            return Ok(String::new());
        }

        let data_hex = &hex_clean[128..];

        // Convert hex to bytes
        let bytes: Vec<u8> = (0..data_hex.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&data_hex[i..i+2], 16).ok())
            .collect();

        // Convert to UTF-8 string, removing null bytes
        let result = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_string();

        Ok(result)
    }
}
