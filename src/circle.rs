use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

const CIRCLE_API_BASE: &str = "https://api.circle.com";

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountBalances {
    pub available_balances: Vec<Balance>,
    pub unsettled_balances: Vec<Balance>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Balance {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
struct CircleBalanceResponse {
    data: CircleBalanceData,
}

#[derive(Debug, Deserialize)]
struct CircleBalanceData {
    available: Vec<CircleAmount>,
    unsettled: Vec<CircleAmount>,
}

#[derive(Debug, Deserialize)]
struct CircleAmount {
    amount: String,
    currency: String,
}

pub struct CircleClient {
    api_key: String,
    client: reqwest::Client,
}

impl CircleClient {
    pub fn new() -> Result<Self> {
        let api_key = env::var("CIRCLE_API_KEY")
            .context("CIRCLE_API_KEY environment variable not set")?;

        Ok(Self {
            api_key,
            client: reqwest::Client::new(),
        })
    }

    pub async fn get_balances(&self) -> Result<AccountBalances> {
        let url = format!("{}/v1/businessAccount/balances", CIRCLE_API_BASE);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to send request to Circle API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Circle API request failed with status {}: {}", status, error_text);
        }

        let balance_response: CircleBalanceResponse = response
            .json()
            .await
            .context("Failed to parse Circle API response")?;

        // Convert Circle amounts (strings) to floats
        let mut available_balances = Vec::new();
        for amount in balance_response.data.available {
            let balance = Balance {
                amount: amount.amount.parse::<f64>()
                    .context(format!("Failed to parse available amount: {}", amount.amount))?,
                currency: amount.currency,
            };
            available_balances.push(balance);
        }

        let mut unsettled_balances = Vec::new();
        for amount in balance_response.data.unsettled {
            let balance = Balance {
                amount: amount.amount.parse::<f64>()
                    .context(format!("Failed to parse unsettled amount: {}", amount.amount))?,
                currency: amount.currency,
            };
            unsettled_balances.push(balance);
        }

        Ok(AccountBalances {
            available_balances,
            unsettled_balances,
        })
    }
}
