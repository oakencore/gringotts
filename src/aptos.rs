use anyhow::{Context, Result};
use serde::Serialize;

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
    pub apt_balance: f64,
    pub apt_usd_price: Option<f64>,
    pub apt_usd_value: Option<f64>,
    #[allow(dead_code)]
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

pub struct AptosClient {
    client: reqwest::Client,
    api_url: String,
}

#[derive(Serialize)]
struct ViewRequest {
    function: String,
    type_arguments: Vec<String>,
    arguments: Vec<String>,
}

impl AptosClient {
    pub fn new(api_url: Option<String>) -> Self {
        let url = api_url.unwrap_or_else(|| "https://fullnode.mainnet.aptoslabs.com/v1".to_string());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            api_url: url,
        }
    }

    pub async fn get_balances(&self, address: &str) -> Result<AccountBalances> {
        // Normalize address: add 0x prefix if missing and validate hex format
        let normalized_address = if address.starts_with("0x") {
            address.to_string()
        } else {
            // Validate it's valid hex before adding prefix
            if !address.chars().all(|c| c.is_ascii_hexdigit()) {
                anyhow::bail!("Invalid Aptos address format: must be hexadecimal");
            }
            format!("0x{}", address)
        };

        // Use the view function to get balance (recommended approach)
        // This is more reliable than querying CoinStore resource
        let view_request = ViewRequest {
            function: "0x1::coin::balance".to_string(),
            type_arguments: vec!["0x1::aptos_coin::AptosCoin".to_string()],
            arguments: vec![normalized_address.clone()],
        };

        let url = format!("{}/view", self.api_url);
        let response = self
            .client
            .post(&url)
            .json(&view_request)
            .send()
            .await
            .context("Failed to send view request")?;

        if !response.status().is_success() {
            // If view function fails, account might not exist or have no balance
            return Ok(AccountBalances {
                apt_balance: 0.0,
                apt_usd_price: None,
                apt_usd_value: None,
                token_balances: Vec::new(),
                total_usd_value: None,
            });
        }

        // Parse response - view functions return an array with the result
        let result: Vec<String> = response
            .json()
            .await
            .context("Failed to parse view response")?;

        let balance_octas: u64 = result
            .get(0)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Convert octas to APT (1 APT = 10^8 octas)
        let apt_balance = balance_octas as f64 / 100_000_000.0;

        // Token balances would require querying other coin stores
        // For now, we'll just return the native APT balance
        let token_balances = Vec::new();

        Ok(AccountBalances {
            apt_balance,
            apt_usd_price: None,
            apt_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }
}
