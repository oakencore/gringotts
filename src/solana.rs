use anyhow::{Context, Result};
use base64::prelude::*;
use mpl_token_metadata::accounts::Metadata;
use solana_account_decoder_client_types::UiAccountData;
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::program_pack::Pack;
use std::str::FromStr;

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

impl SolanaClient {
    pub fn new(rpc_url: Option<String>) -> Self {
        let url = rpc_url.unwrap_or_else(|| {
            "https://api.mainnet-beta.solana.com".to_string()
        });

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
}
