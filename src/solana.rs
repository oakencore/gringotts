use anyhow::{Context, Result};
use base64::prelude::*;
use mpl_token_metadata::accounts::Metadata;
use solana_account_decoder_client_types::UiAccountData;
use solana_client::rpc_client::{RpcClient, GetConfirmedSignaturesForAddress2Config};
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::program_pack::Pack;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::env;
use std::str::FromStr;
use std::collections::HashMap;

const HELIUS_RPC_TEMPLATE: &str = "https://mainnet.helius-rpc.com/?api-key={}";
const SOLANA_PUBLIC_RPC: &str = "https://api.mainnet-beta.solana.com";

fn get_default_rpc_url() -> Option<String> {
    match env::var("HELIUS_API_KEY") {
        Ok(api_key) => Some(HELIUS_RPC_TEMPLATE.replace("{}", &api_key).to_string()),
        Err(_) => {
            eprintln!("Warning: HELIUS_API_KEY not set. Using public Solana RPC.");
            eprintln!("For better performance, get a Helius API key at https://helius.dev");
            None
        }
    }
}

pub struct SolanaClient {
    client: RpcClient,
}

#[derive(Debug)]
pub struct TokenBalance {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: u8,
    pub ui_amount: f64,
    pub usd_price: Option<f64>,
    pub usd_value: Option<f64>,
}

#[derive(Debug)]
pub struct AccountBalances {
    pub sol_balance: f64,
    pub sol_usd_price: Option<f64>,
    pub sol_usd_value: Option<f64>,
    pub token_balances: Vec<TokenBalance>,
    pub total_usd_value: Option<f64>,
}

#[derive(Debug)]
pub struct SolanaTransaction {
    pub signature: String,
    pub timestamp: Option<i64>,
    pub slot: u64,
    pub success: bool,
    pub memo: Option<String>,
    pub sol_change: f64, // SOL change (positive = received, negative = sent)
}

impl SolanaClient {
    pub fn new(rpc_url: Option<String>) -> Self {
        let url = rpc_url
            .or_else(get_default_rpc_url)
            .unwrap_or_else(|| SOLANA_PUBLIC_RPC.to_string());

        Self {
            client: RpcClient::new(url),
        }
    }

    fn get_token_metadata(&self, mint: &Pubkey) -> Option<(String, String)> {
        // Derive the metadata PDA for this mint
        let metadata_seeds = &[
            b"metadata",
            mpl_token_metadata::ID.as_ref(),
            mint.as_ref(),
        ];

        let (metadata_pda, _) = Pubkey::find_program_address(
            metadata_seeds,
            &mpl_token_metadata::ID,
        );

        // Try to fetch the metadata account
        if let Ok(account_data) = self.client.get_account_data(&metadata_pda) {
            if let Ok(metadata) = Metadata::from_bytes(&account_data) {
                return Some((metadata.name.trim_matches('\0').to_string(), metadata.symbol.trim_matches('\0').to_string()));
            }
        }

        None
    }

    fn get_mint_decimals(&self, mint: &Pubkey) -> Option<u8> {
        if let Ok(account_data) = self.client.get_account_data(mint) {
            if let Ok(mint_account) = spl_token::state::Mint::unpack(&account_data) {
                return Some(mint_account.decimals);
            }
        }
        None
    }

    pub fn get_balances(&self, address: &str) -> Result<AccountBalances> {
        let pubkey = Pubkey::from_str(address)
            .context("Invalid Solana address")?;

        // Get SOL balance
        let lamports = self.client
            .get_balance(&pubkey)
            .context("Failed to fetch SOL balance")?;
        let sol_balance = lamports as f64 / 1_000_000_000.0;

        // Get token accounts
        let token_accounts = self.client
            .get_token_accounts_by_owner(&pubkey, solana_client::rpc_request::TokenAccountsFilter::ProgramId(spl_token::id()))
            .context("Failed to fetch token accounts")?;

        let mut token_balances = Vec::new();

        for account in token_accounts {
            // Handle different UiAccountData formats
            match &account.account.data {
                UiAccountData::Binary(data, _encoding) => {
                    // Decode base64 data
                    if let Ok(decoded) = BASE64_STANDARD.decode(data) {
                        // Parse token account data
                        if let Ok(token_account) = spl_token::state::Account::unpack(&decoded) {
                            let mint_pubkey = token_account.mint;

                            // Fetch decimals from mint
                            let decimals = self.get_mint_decimals(&mint_pubkey).unwrap_or(0);

                            // Calculate UI amount
                            let ui_amount = if decimals > 0 {
                                token_account.amount as f64 / 10_f64.powi(decimals as i32)
                            } else {
                                token_account.amount as f64
                            };

                            // Try to fetch metadata
                            let (name, symbol) = self.get_token_metadata(&mint_pubkey)
                                .map(|(n, s)| (Some(n), Some(s)))
                                .unwrap_or((None, None));

                            token_balances.push(TokenBalance {
                                mint: mint_pubkey.to_string(),
                                name,
                                symbol,
                                decimals,
                                ui_amount,
                                usd_price: None,
                                usd_value: None,
                            });
                        }
                    }
                }
                UiAccountData::Json(parsed) => {
                    // Handle JSON parsed account data
                    if let Some(info) = parsed.parsed.as_object() {
                        if let (Some(info_obj), Some(type_str)) =
                            (info.get("info").and_then(|v| v.as_object()), info.get("type").and_then(|v| v.as_str())) {
                            if type_str == "account" {
                                let mint = info_obj.get("mint")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let decimals = info_obj.get("tokenAmount")
                                    .and_then(|v| v.as_object())
                                    .and_then(|ta| ta.get("decimals"))
                                    .and_then(|d| d.as_u64())
                                    .unwrap_or(0) as u8;
                                let ui_amount = info_obj.get("tokenAmount")
                                    .and_then(|v| v.as_object())
                                    .and_then(|ta| ta.get("uiAmount"))
                                    .and_then(|u| u.as_f64())
                                    .unwrap_or(0.0);

                                // Try to fetch metadata
                                let (name, symbol) = if let Ok(mint_pubkey) = Pubkey::from_str(&mint) {
                                    self.get_token_metadata(&mint_pubkey)
                                        .map(|(n, s)| (Some(n), Some(s)))
                                        .unwrap_or((None, None))
                                } else {
                                    (None, None)
                                };

                                token_balances.push(TokenBalance {
                                    mint,
                                    name,
                                    symbol,
                                    decimals,
                                    ui_amount,
                                    usd_price: None,
                                    usd_value: None,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(AccountBalances {
            sol_balance,
            sol_usd_price: None,
            sol_usd_value: None,
            token_balances,
            total_usd_value: None,
        })
    }

    pub fn get_transactions(&self, address: &str, limit: usize) -> Result<Vec<SolanaTransaction>> {
        let pubkey = Pubkey::from_str(address)
            .context("Invalid Solana address")?;
        let address_str = address.to_string();

        // Get recent transaction signatures
        let config = GetConfirmedSignaturesForAddress2Config {
            limit: Some(limit),
            ..Default::default()
        };

        let signatures = self.client
            .get_signatures_for_address_with_config(&pubkey, config)
            .context("Failed to fetch transaction signatures")?;

        let mut transactions = Vec::new();

        // Only fetch detailed info for first 20 transactions to avoid rate limits
        let detail_limit = 20.min(signatures.len());

        for (idx, sig_info) in signatures.iter().enumerate() {
            let signature_str = sig_info.signature.clone();
            let timestamp = sig_info.block_time;
            let slot = sig_info.slot;
            let success = sig_info.err.is_none();
            let memo = sig_info.memo.clone();

            // Try to get transaction details to calculate SOL change (only for first few)
            let mut sol_change: f64 = 0.0;

            if idx < detail_limit {
                if let Ok(sig) = Signature::from_str(&signature_str) {
                    let tx_config = RpcTransactionConfig {
                        encoding: Some(UiTransactionEncoding::JsonParsed),
                        commitment: Some(CommitmentConfig::confirmed()),
                        max_supported_transaction_version: Some(0),
                    };

                    // Small delay between RPC calls to avoid rate limiting
                    if idx > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }

                    match self.client.get_transaction_with_config(&sig, tx_config) {
                        Ok(tx) => {
                            // Calculate balance change from pre/post balances
                            if let Some(meta) = &tx.transaction.meta {
                                // Get account keys from the transaction JSON
                                let account_keys = Self::extract_account_keys(&tx.transaction.transaction)
                                    .unwrap_or_default();

                                for (i, key) in account_keys.iter().enumerate() {
                                    if key == &address_str {
                                        if i < meta.pre_balances.len() && i < meta.post_balances.len() {
                                            let lamport_change = meta.post_balances[i] as i64 - meta.pre_balances[i] as i64;
                                            sol_change = lamport_change as f64 / 1_000_000_000.0;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Transaction fetch failed - likely rate limited or old transaction
                        }
                    }
                }
            }

            transactions.push(SolanaTransaction {
                signature: signature_str,
                timestamp,
                slot,
                success,
                memo,
                sol_change,
            });
        }

        Ok(transactions)
    }

    fn extract_account_keys(tx: &solana_transaction_status_client_types::EncodedTransaction) -> Option<Vec<String>> {
        use solana_transaction_status_client_types::EncodedTransaction;

        match tx {
            EncodedTransaction::Json(ui_tx) => {
                // For JSON format, extract keys from the message
                match &ui_tx.message {
                    solana_transaction_status_client_types::UiMessage::Parsed(parsed) => {
                        Some(parsed.account_keys.iter().map(|k| k.pubkey.clone()).collect())
                    }
                    solana_transaction_status_client_types::UiMessage::Raw(raw) => {
                        Some(raw.account_keys.clone())
                    }
                }
            }
            EncodedTransaction::LegacyBinary(_) | EncodedTransaction::Binary(_, _) => {
                // For binary formats, we'd need to decode - skip for now
                None
            }
            EncodedTransaction::Accounts(accounts_tx) => {
                Some(accounts_tx.account_keys.iter().map(|k| k.pubkey.clone()).collect())
            }
        }
    }
}

// Implement PriceEnrichable trait for Solana balances
impl crate::PriceEnrichable for AccountBalances {
    const NATIVE_SYMBOL: &'static str = "SOL";

    fn native_balance(&self) -> f64 {
        self.sol_balance
    }

    fn set_native_usd_price(&mut self, price: f64) {
        self.sol_usd_price = Some(price);
    }

    fn set_native_usd_value(&mut self, value: f64) {
        self.sol_usd_value = Some(value);
    }

    fn set_total_usd_value(&mut self, value: f64) {
        self.total_usd_value = Some(value);
    }

    fn enrich_token_balances(&mut self, price_cache: &HashMap<String, f64>) -> f64 {
        let mut token_total = 0.0;
        for token in &mut self.token_balances {
            if let Some(symbol) = &token.symbol {
                if let Some(&price) = price_cache.get(symbol) {
                    token.usd_price = Some(price);
                    token.usd_value = Some(token.ui_amount * price);
                    token_total += token.usd_value.unwrap_or(0.0);
                }
            }
        }
        token_total
    }
}
