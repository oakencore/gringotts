use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

const MERCURY_API_BASE: &str = "https://api.mercury.com/api/v1";

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountBalances {
    pub available_balance: f64,
    pub current_balance: f64,
    pub account_id: String,
    pub status: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub id: String,
    pub amount: f64,
    pub created_at: String,
    pub posted_at: Option<String>,
    pub status: String,
    pub note: Option<String>,
    pub bank_description: Option<String>,
    pub counterparty_name: Option<String>,
    pub kind: String,
    pub external_memo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TransactionsResponse {
    #[allow(dead_code)]
    total: i32,
    transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MercuryAccount {
    pub id: String,
    pub account_number: String,
    pub routing_number: String,
    pub name: String,
    pub status: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub available_balance: f64,
    pub current_balance: f64,
    pub kind: String,
    pub legal_business_name: String,
}

#[derive(Debug, Deserialize)]
struct AccountsResponse {
    accounts: Vec<MercuryAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MercuryAccountResponse {
    available_balance: f64,
    current_balance: f64,
    id: String,
    status: String,
    created_at: Option<String>,
}

pub struct MercuryClient {
    api_key: String,
    client: reqwest::Client,
}

impl MercuryClient {
    pub fn new() -> Result<Self> {
        let api_key = env::var("MERCURY_API_KEY")
            .context("MERCURY_API_KEY environment variable not set")?;

        Ok(Self {
            api_key,
            client: reqwest::Client::new(),
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let response = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer secret-token:{}", self.api_key))
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to send request to Mercury API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Mercury API request failed with status {}: {}", status, error_text);
        }

        response.json().await.context("Failed to parse Mercury API response")
    }

    pub async fn list_accounts(&self) -> Result<Vec<MercuryAccount>> {
        let url = format!("{}/accounts", MERCURY_API_BASE);
        let result: AccountsResponse = self.get(&url).await?;
        Ok(result.accounts)
    }

    pub async fn get_account_balance(&self, account_id: &str) -> Result<AccountBalances> {
        let url = format!("{}/account/{}", MERCURY_API_BASE, account_id);
        let account: MercuryAccountResponse = self.get(&url).await?;

        Ok(AccountBalances {
            available_balance: account.available_balance,
            current_balance: account.current_balance,
            account_id: account.id,
            status: account.status,
            created_at: account.created_at,
        })
    }

    pub async fn get_transactions(
        &self,
        account_id: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Vec<Transaction>> {
        let mut all_transactions = Vec::new();
        let mut offset = 0;
        let limit = 100;

        loop {
            let mut url = format!(
                "{}/account/{}/transactions?limit={}&offset={}",
                MERCURY_API_BASE, account_id, limit, offset
            );

            if let Some(start_date) = start {
                url.push_str(&format!("&start={}", start_date));
            }
            if let Some(end_date) = end {
                url.push_str(&format!("&end={}", end_date));
            }

            let result: TransactionsResponse = self.get(&url).await?;
            let count = result.transactions.len();
            all_transactions.extend(result.transactions);

            if count < limit {
                break;
            }
            offset += limit;
        }

        Ok(all_transactions)
    }
}
