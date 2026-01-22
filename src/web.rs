use crate::aptos::AptosClient;
use crate::cache::{CachedBalance, CachedToken, SharedCache};
use crate::circle::CircleClient;
use crate::evm::EvmClient;
use crate::mercury::MercuryClient;
use crate::near::NearClient;
use crate::price::PriceService;
use crate::solana::SolanaClient;
use crate::starknet::StarknetClient;
use crate::storage::{AddressBook, BankingService, Chain};
use crate::sui::SuiClient;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
    Form, Router,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

// Custom filter for formatting USD values
mod filters {
    pub fn format_usd(value: &f64) -> askama::Result<String> {
        let formatted = format!("{:.2}", value);
        Ok(add_commas(&formatted))
    }

    pub fn format_amount(value: &f64) -> askama::Result<String> {
        if *value < 0.0001 && *value > 0.0 {
            Ok(format!("{:.8}", value))
        } else if *value < 1.0 {
            Ok(format!("{:.6}", value))
        } else {
            let formatted = format!("{:.4}", value);
            Ok(add_commas(&formatted))
        }
    }

    fn add_commas(s: &str) -> String {
        let parts: Vec<&str> = s.split('.').collect();
        let integer_part = parts[0];
        let decimal_part = parts.get(1).unwrap_or(&"");

        let with_commas: String = integer_part
            .chars()
            .rev()
            .enumerate()
            .fold(String::new(), |mut acc, (i, c)| {
                if i > 0 && i % 3 == 0 {
                    acc.push(',');
                }
                acc.push(c);
                acc
            })
            .chars()
            .rev()
            .collect();

        if decimal_part.is_empty() {
            with_commas
        } else {
            format!("{}.{}", with_commas, decimal_part)
        }
    }

    pub fn replace(s: &str, from: &str, to: &str) -> askama::Result<String> {
        Ok(s.replace(from, to))
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    companies: Vec<CompanyGroup>,
    wallet_count: usize,
    bank_count: usize,
}

struct CompanyGroup {
    name: String,
    wallets: Vec<WalletView>,
    banking_accounts: Vec<BankingView>,
}

#[derive(Template)]
#[template(path = "balances.html")]
struct BalancesTemplate {
    total_usd: f64,
    companies: Vec<(String, Vec<AssetView>)>,
    error: String,
}

#[derive(Template)]
#[template(path = "account_row.html")]
struct AccountRowTemplate {
    name: String,
    company: String,
    address: String,
    chain: String,
}

#[derive(Template)]
#[template(path = "single_balance.html")]
struct SingleBalanceTemplate {
    name: String,
    address: String,
    chain: String,
    native_symbol: String,
    native_balance: f64,
    native_usd: f64,
    tokens: Vec<TokenView>,
    total_usd: f64,
    error: String,
}

struct TokenView {
    symbol: String,
    balance: f64,
    usd_value: f64,
}

#[derive(Template)]
#[template(path = "transactions.html")]
struct TransactionsTemplate {
    name: String,
    account_type: String,
    transactions: Vec<TransactionView>,
    error: String,
}

struct TransactionView {
    date: String,
    description: String,
    amount: f64,
    #[allow(dead_code)]
    currency: String,
    tx_type: String,
    status: String,
    counterparty: String,
}

struct WalletView {
    name: String,
    #[allow(dead_code)]
    company: String,
    address: String,
    chain: String,
}

struct BankingView {
    name: String,
    #[allow(dead_code)]
    company: String,
    account_id: String,
    service: String,
}

struct AssetView {
    symbol: String,
    amount: f64,
    usd_value: f64,
}

#[derive(Deserialize)]
struct AddAccountForm {
    company: String,
    name: String,
    address: String,
    chain: String,
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub cache: SharedCache,
}

/// Default refresh interval: 4 hours
const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 4 * 60 * 60;

/// Parse a duration string like "4h", "30m", "1d" into seconds.
/// Supported units: s (seconds), m (minutes), h (hours), d (days)
fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("Empty duration string");
    }

    // Find where the numeric part ends
    let num_end = s
        .chars()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(s.len());

    if num_end == 0 {
        anyhow::bail!("Duration must start with a number: {}", s);
    }

    let num: u64 = s[..num_end]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid number in duration: {}", s))?;

    let unit = s[num_end..].trim().to_lowercase();

    let secs = match unit.as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => num,
        "m" | "min" | "mins" | "minute" | "minutes" => num * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => num * 60 * 60,
        "d" | "day" | "days" => num * 24 * 60 * 60,
        _ => anyhow::bail!("Unknown duration unit '{}'. Use s, m, h, or d.", unit),
    };

    Ok(Duration::from_secs(secs))
}

/// Resolve the refresh interval from CLI flag, env var, or default.
/// Priority: CLI flag > GRINGOTTS_REFRESH_INTERVAL env var > default (4h)
fn resolve_refresh_interval(cli_interval: Option<String>) -> anyhow::Result<Duration> {
    // CLI flag takes priority
    if let Some(interval_str) = cli_interval {
        return parse_duration(&interval_str);
    }

    // Check environment variable
    if let Ok(env_interval) = std::env::var("GRINGOTTS_REFRESH_INTERVAL") {
        return parse_duration(&env_interval);
    }

    // Default: 4 hours
    Ok(Duration::from_secs(DEFAULT_REFRESH_INTERVAL_SECS))
}

/// Format a duration for display
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

pub async fn start_server(port: u16, refresh_interval: Option<String>) -> anyhow::Result<()> {
    // Resolve refresh interval
    let interval = resolve_refresh_interval(refresh_interval)?;
    let interval_display = format_duration(interval);

    // Load address book to verify it exists
    let book = AddressBook::load().unwrap_or_else(|_| AddressBook::new());
    let wallet_count = book.addresses.len();
    let bank_count = book.banking_accounts.len();

    // Initialize shared cache - loads from disk if available
    let cache = SharedCache::new();
    let cache_age = cache.read().await.cache_age_string();

    let state = Arc::new(AppState { cache });

    let app = Router::new()
        .route("/", get(index))
        .route("/accounts", post(add_account))
        .route("/accounts/:name", delete(remove_account))
        .route("/balances", get(query_balances))
        .route("/balances/:name", get(query_single_balance))
        .route("/transactions/:name", get(get_transactions))
        .route("/api/cache/status", get(cache_status))
        .with_state(state.clone());

    // Bind to 0.0.0.0 to accept connections from local network
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    println!("\n╔═══════════════════════════════════════════════════════════════╗");
    println!("║          Gringotts Web Server Started                        ║");
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║  Local:          http://localhost:{}                       ║", port);
    println!("║  Network:        http://<your-ip>:{}                      ║", port);
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║  Address Book:   {} wallets, {} bank accounts              ║", wallet_count, bank_count);
    println!("║  Cache:          ~/.gringotts/cache.json                    ║");
    println!("║  Last refresh:   {:>42} ║", cache_age);
    println!("║  Refresh every:  {:>42} ║", interval_display);
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║  To find your IP address:                                    ║");
    println!("║    macOS/Linux:  ifconfig | grep 'inet '                    ║");
    println!("║    Windows:      ipconfig                                    ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    // Spawn background refresh task
    let refresh_state = state.clone();
    tokio::spawn(async move {
        background_refresh_task(refresh_state, interval).await;
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Background task that refreshes balances on a configured interval
async fn background_refresh_task(state: Arc<AppState>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);

    // Skip the first immediate tick - don't refresh right at startup
    ticker.tick().await;

    loop {
        ticker.tick().await;

        println!("[{}] Starting background refresh...", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));

        if let Err(e) = refresh_all_balances(&state).await {
            eprintln!("[{}] Background refresh failed: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), e);
        } else {
            println!("[{}] Background refresh completed", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    }
}

/// Refresh all balances and update the cache
async fn refresh_all_balances(state: &Arc<AppState>) -> anyhow::Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        return Ok(());
    }

    let mut all_symbols: HashSet<String> = HashSet::new();

    // Query crypto wallets
    for wallet in &book.addresses {
        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address) {
                    all_symbols.insert("SOL".to_string());
                    let mut cached_tokens = vec![];
                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            all_symbols.insert(symbol.clone());
                            cached_tokens.push(CachedToken {
                                symbol: symbol.clone(),
                                balance: token.ui_amount,
                                usd_value: token.usd_value,
                            });
                        }
                    }
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "SOL".to_string(),
                        native_balance: balances.sol_balance,
                        native_usd_value: balances.sol_usd_value,
                        tokens: cached_tokens,
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Near => {
                let client = NearClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("NEAR".to_string());
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "NEAR".to_string(),
                        native_balance: balances.near_balance,
                        native_usd_value: balances.near_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Aptos => {
                let client = AptosClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("APT".to_string());
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "APT".to_string(),
                        native_balance: balances.apt_balance,
                        native_usd_value: balances.apt_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Sui => {
                let client = SuiClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("SUI".to_string());
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "SUI".to_string(),
                        native_balance: balances.sui_balance,
                        native_usd_value: balances.sui_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Starknet => {
                let client = StarknetClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("ETH".to_string());
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "ETH".to_string(),
                        native_balance: balances.eth_balance,
                        native_usd_value: balances.eth_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Ethereum
            | Chain::Polygon
            | Chain::BinanceSmartChain
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::Core => {
                if let Ok(client) = EvmClient::new(None, wallet.chain.clone()) {
                    if let Ok(balances) = client.get_balances(&wallet.address).await {
                        let native_symbol = wallet.chain.native_token_symbol();
                        all_symbols.insert(native_symbol.to_string());
                        let mut cached_tokens = vec![];
                        for token in &balances.token_balances {
                            if let Some(symbol) = &token.symbol {
                                all_symbols.insert(symbol.clone());
                                cached_tokens.push(CachedToken {
                                    symbol: symbol.clone(),
                                    balance: token.ui_amount,
                                    usd_value: token.usd_value,
                                });
                            }
                        }
                        let cached_balance = CachedBalance {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            native_symbol: native_symbol.to_string(),
                            native_balance: balances.eth_balance,
                            native_usd_value: balances.eth_usd_value,
                            tokens: cached_tokens,
                            total_usd_value: balances.total_usd_value,
                        };
                        let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                    }
                }
            }
        }
    }

    // Query banking accounts
    for account in &book.banking_accounts {
        match &account.service {
            BankingService::Mercury => {
                if let Ok(client) = MercuryClient::new() {
                    if let Ok(balances) = client.get_account_balance(&account.account_id).await {
                        let cached_balance = CachedBalance {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: "Mercury Banking".to_string(),
                            native_symbol: "USD".to_string(),
                            native_balance: balances.current_balance,
                            native_usd_value: Some(balances.current_balance),
                            tokens: vec![],
                            total_usd_value: Some(balances.current_balance),
                        };
                        let _ = state.cache.update_balance(&account.name, cached_balance).await;
                    }
                }
            }
            BankingService::Circle => {
                if let Ok(client) = CircleClient::new() {
                    if let Ok(balances) = client.get_balances().await {
                        let mut cached_tokens = vec![];
                        let mut total_usd = 0.0;
                        for balance in &balances.available_balances {
                            let symbol = match balance.currency.as_str() {
                                "USD" => "USDC",
                                "EUR" => "EURC",
                                _ => &balance.currency,
                            };
                            if balance.currency == "USD" {
                                total_usd += balance.amount;
                            }
                            cached_tokens.push(CachedToken {
                                symbol: symbol.to_string(),
                                balance: balance.amount,
                                usd_value: if balance.currency == "USD" { Some(balance.amount) } else { None },
                            });
                        }
                        let cached_balance = CachedBalance {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: "Circle".to_string(),
                            native_symbol: "USD".to_string(),
                            native_balance: total_usd,
                            native_usd_value: Some(total_usd),
                            tokens: cached_tokens,
                            total_usd_value: Some(total_usd),
                        };
                        let _ = state.cache.update_balance(&account.name, cached_balance).await;
                    }
                }
            }
        }
    }

    // Fetch and cache prices
    if let Ok(mut price_service) = PriceService::new() {
        let symbols: Vec<String> = all_symbols.into_iter().collect();
        if let Ok(prices) = price_service.batch_fetch_prices(&symbols).await {
            let _ = state.cache.update_prices(prices).await;
        }
    }

    // Mark full refresh
    let _ = state.cache.mark_refresh().await;

    Ok(())
}

/// API endpoint to check cache status
async fn cache_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.cache.read().await;
    let status = serde_json::json!({
        "last_refresh": cache.cache_age_string(),
        "cached_balances": cache.balances.len(),
        "cached_prices": cache.prices.data.len(),
    });
    (StatusCode::OK, axum::Json(status))
}

async fn index() -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(_) => AddressBook::new(),
    };

    let wallet_count = book.addresses.len();
    let bank_count = book.banking_accounts.len();

    // Group by company
    let mut company_map: HashMap<String, (Vec<WalletView>, Vec<BankingView>)> = HashMap::new();

    for w in &book.addresses {
        let company_name = if w.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            w.company.clone()
        };
        let entry = company_map.entry(company_name).or_insert((vec![], vec![]));
        entry.0.push(WalletView {
            name: w.name.clone(),
            company: w.company.clone(),
            address: w.address.clone(),
            chain: w.chain.display_name().to_string(),
        });
    }

    for a in &book.banking_accounts {
        let company_name = if a.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            a.company.clone()
        };
        let entry = company_map.entry(company_name).or_insert((vec![], vec![]));
        entry.1.push(BankingView {
            name: a.name.clone(),
            company: a.company.clone(),
            account_id: a.account_id.clone(),
            service: a.service.display_name().to_string(),
        });
    }

    // Sort companies alphabetically, but put "Uncategorized" last
    let mut companies: Vec<CompanyGroup> = company_map
        .into_iter()
        .map(|(name, (wallets, banking_accounts))| CompanyGroup {
            name,
            wallets,
            banking_accounts,
        })
        .collect();

    companies.sort_by(|a, b| {
        if a.name == "Uncategorized" {
            std::cmp::Ordering::Greater
        } else if b.name == "Uncategorized" {
            std::cmp::Ordering::Less
        } else {
            a.name.cmp(&b.name)
        }
    });

    let template = IndexTemplate {
        companies,
        wallet_count,
        bank_count,
    };

    Html(template.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

async fn add_account(Form(form): Form<AddAccountForm>) -> impl IntoResponse {
    let mut book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Error: {}", e))),
    };

    let chain_opt = if form.chain.is_empty() {
        None
    } else {
        Some(form.chain.clone())
    };

    let chain_display = if let Some(ref c) = chain_opt {
        Chain::from_str(c)
            .map(|ch| ch.display_name().to_string())
            .unwrap_or_else(|_| "Unknown".to_string())
    } else {
        // Auto-detect
        if form.address.starts_with("0x") && form.address.len() == 42 {
            "Ethereum".to_string()
        } else {
            "Solana".to_string()
        }
    };

    if let Err(e) = book.add_address(
        form.company.clone(),
        form.name.clone(),
        form.address.clone(),
        chain_opt,
    ) {
        return (StatusCode::BAD_REQUEST, Html(format!("Error: {}", e)));
    }

    if let Err(e) = book.save() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Error: {}", e)));
    }

    let template = AccountRowTemplate {
        name: form.name,
        company: form.company,
        address: form.address,
        chain: chain_display,
    };

    (StatusCode::OK, Html(template.render().unwrap_or_default()))
}

async fn remove_account(Path(name): Path<String>) -> impl IntoResponse {
    let mut book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Error: {}", e))),
    };

    // Try removing from addresses first
    let removed_crypto = book.remove_by_identifier(&name).is_ok();

    // If not found in addresses, try banking accounts
    let removed_bank = if !removed_crypto {
        book.remove_banking_account_by_identifier(&name).is_ok()
    } else {
        false
    };

    if !removed_crypto && !removed_bank {
        return (StatusCode::NOT_FOUND, Html("Account not found".to_string()));
    }

    if let Err(e) = book.save() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Error: {}", e)));
    }

    // Return empty to remove the row
    (StatusCode::OK, Html(String::new()))
}

async fn query_balances(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            return Html(
                BalancesTemplate {
                    total_usd: 0.0,
                    companies: vec![],
                    error: format!("Failed to load accounts: {}", e),
                }
                .render()
                .unwrap_or_default(),
            );
        }
    };

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        return Html(
            BalancesTemplate {
                total_usd: 0.0,
                companies: vec![],
                error: String::new(),
            }
            .render()
            .unwrap_or_default(),
        );
    }

    // Query all balances and aggregate
    let mut portfolio: HashMap<String, HashMap<String, (f64, f64)>> = HashMap::new();
    let mut all_symbols: HashSet<String> = HashSet::new();

    // Query crypto wallets
    for wallet in &book.addresses {
        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address) {
                    all_symbols.insert("SOL".to_string());
                    let company = if wallet.company.is_empty() {
                        "Uncategorized"
                    } else {
                        &wallet.company
                    };
                    let entry = portfolio.entry(company.to_string()).or_default();
                    let sol_entry = entry.entry("SOL".to_string()).or_insert((0.0, 0.0));
                    sol_entry.0 += balances.sol_balance;

                    // Build tokens for cache
                    let mut cached_tokens = vec![];
                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            all_symbols.insert(symbol.clone());
                            let token_entry = entry.entry(symbol.clone()).or_insert((0.0, 0.0));
                            token_entry.0 += token.ui_amount;
                            cached_tokens.push(CachedToken {
                                symbol: symbol.clone(),
                                balance: token.ui_amount,
                                usd_value: token.usd_value,
                            });
                        }
                    }

                    // Update cache
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "SOL".to_string(),
                        native_balance: balances.sol_balance,
                        native_usd_value: balances.sol_usd_value,
                        tokens: cached_tokens,
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Near => {
                let client = NearClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("NEAR".to_string());
                    let company = if wallet.company.is_empty() {
                        "Uncategorized"
                    } else {
                        &wallet.company
                    };
                    let entry = portfolio.entry(company.to_string()).or_default();
                    let near_entry = entry.entry("NEAR".to_string()).or_insert((0.0, 0.0));
                    near_entry.0 += balances.near_balance;

                    // Update cache
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "NEAR".to_string(),
                        native_balance: balances.near_balance,
                        native_usd_value: balances.near_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Aptos => {
                let client = AptosClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("APT".to_string());
                    let company = if wallet.company.is_empty() {
                        "Uncategorized"
                    } else {
                        &wallet.company
                    };
                    let entry = portfolio.entry(company.to_string()).or_default();
                    let apt_entry = entry.entry("APT".to_string()).or_insert((0.0, 0.0));
                    apt_entry.0 += balances.apt_balance;

                    // Update cache
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "APT".to_string(),
                        native_balance: balances.apt_balance,
                        native_usd_value: balances.apt_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Sui => {
                let client = SuiClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("SUI".to_string());
                    let company = if wallet.company.is_empty() {
                        "Uncategorized"
                    } else {
                        &wallet.company
                    };
                    let entry = portfolio.entry(company.to_string()).or_default();
                    let sui_entry = entry.entry("SUI".to_string()).or_insert((0.0, 0.0));
                    sui_entry.0 += balances.sui_balance;

                    // Update cache
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "SUI".to_string(),
                        native_balance: balances.sui_balance,
                        native_usd_value: balances.sui_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            Chain::Starknet => {
                let client = StarknetClient::new(None);
                if let Ok(balances) = client.get_balances(&wallet.address).await {
                    all_symbols.insert("ETH".to_string());
                    let company = if wallet.company.is_empty() {
                        "Uncategorized"
                    } else {
                        &wallet.company
                    };
                    let entry = portfolio.entry(company.to_string()).or_default();
                    let eth_entry = entry.entry("ETH".to_string()).or_insert((0.0, 0.0));
                    eth_entry.0 += balances.eth_balance;

                    // Update cache
                    let cached_balance = CachedBalance {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        native_symbol: "ETH".to_string(),
                        native_balance: balances.eth_balance,
                        native_usd_value: balances.eth_usd_value,
                        tokens: vec![],
                        total_usd_value: balances.total_usd_value,
                    };
                    let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                }
            }
            // EVM chains
            Chain::Ethereum
            | Chain::Polygon
            | Chain::BinanceSmartChain
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::Core => {
                if let Ok(client) = EvmClient::new(None, wallet.chain.clone()) {
                    if let Ok(balances) = client.get_balances(&wallet.address).await {
                        let native_symbol = wallet.chain.native_token_symbol();
                        all_symbols.insert(native_symbol.to_string());
                        let company = if wallet.company.is_empty() {
                            "Uncategorized"
                        } else {
                            &wallet.company
                        };
                        let entry = portfolio.entry(company.to_string()).or_default();
                        let native_entry =
                            entry.entry(native_symbol.to_string()).or_insert((0.0, 0.0));
                        native_entry.0 += balances.eth_balance;

                        let mut cached_tokens = vec![];
                        for token in &balances.token_balances {
                            if let Some(symbol) = &token.symbol {
                                all_symbols.insert(symbol.clone());
                                let token_entry = entry.entry(symbol.clone()).or_insert((0.0, 0.0));
                                token_entry.0 += token.ui_amount;
                                cached_tokens.push(CachedToken {
                                    symbol: symbol.clone(),
                                    balance: token.ui_amount,
                                    usd_value: token.usd_value,
                                });
                            }
                        }

                        // Update cache
                        let cached_balance = CachedBalance {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            native_symbol: native_symbol.to_string(),
                            native_balance: balances.eth_balance,
                            native_usd_value: balances.eth_usd_value,
                            tokens: cached_tokens,
                            total_usd_value: balances.total_usd_value,
                        };
                        let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
                    }
                }
            }
        }
    }

    // Query banking accounts
    for account in &book.banking_accounts {
        match &account.service {
            BankingService::Mercury => {
                if let Ok(client) = MercuryClient::new() {
                    if let Ok(balances) = client.get_account_balance(&account.account_id).await {
                        let company = if account.company.is_empty() {
                            "Uncategorized"
                        } else {
                            &account.company
                        };
                        let entry = portfolio.entry(company.to_string()).or_default();
                        let usd_entry = entry.entry("USD".to_string()).or_insert((0.0, 0.0));
                        usd_entry.0 += balances.current_balance;
                        usd_entry.1 += balances.current_balance; // USD is already in USD

                        // Update cache
                        let cached_balance = CachedBalance {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: "Mercury Banking".to_string(),
                            native_symbol: "USD".to_string(),
                            native_balance: balances.current_balance,
                            native_usd_value: Some(balances.current_balance),
                            tokens: vec![],
                            total_usd_value: Some(balances.current_balance),
                        };
                        let _ = state.cache.update_balance(&account.name, cached_balance).await;
                    }
                }
            }
            BankingService::Circle => {
                if let Ok(client) = CircleClient::new() {
                    if let Ok(balances) = client.get_balances().await {
                        let company = if account.company.is_empty() {
                            "Uncategorized"
                        } else {
                            &account.company
                        };
                        let entry = portfolio.entry(company.to_string()).or_default();
                        let mut cached_tokens = vec![];
                        let mut total_usd = 0.0;
                        for balance in &balances.available_balances {
                            let symbol = match balance.currency.as_str() {
                                "USD" => "USDC",
                                "EUR" => "EURC",
                                _ => &balance.currency,
                            };
                            let currency_entry = entry.entry(symbol.to_string()).or_insert((0.0, 0.0));
                            currency_entry.0 += balance.amount;
                            if balance.currency == "USD" {
                                currency_entry.1 += balance.amount;
                                total_usd += balance.amount;
                            }
                            cached_tokens.push(CachedToken {
                                symbol: symbol.to_string(),
                                balance: balance.amount,
                                usd_value: if balance.currency == "USD" { Some(balance.amount) } else { None },
                            });
                        }

                        // Update cache
                        let cached_balance = CachedBalance {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: "Circle".to_string(),
                            native_symbol: "USD".to_string(),
                            native_balance: total_usd,
                            native_usd_value: Some(total_usd),
                            tokens: cached_tokens,
                            total_usd_value: Some(total_usd),
                        };
                        let _ = state.cache.update_balance(&account.name, cached_balance).await;
                    }
                }
            }
        }
    }

    // Fetch prices for crypto assets
    if let Ok(mut price_service) = PriceService::new() {
        let symbols: Vec<String> = all_symbols.into_iter().collect();
        if let Ok(prices) = price_service.batch_fetch_prices(&symbols).await {
            // Update price cache
            let _ = state.cache.update_prices(prices.clone()).await;

            // Apply prices to portfolio
            for assets in portfolio.values_mut() {
                for (symbol, (amount, usd_value)) in assets.iter_mut() {
                    if *usd_value == 0.0 {
                        if let Some(&price) = prices.get(symbol) {
                            *usd_value = *amount * price;
                        }
                    }
                }
            }
        }
    }

    // Mark full refresh
    let _ = state.cache.mark_refresh().await;

    // Calculate totals and format for template
    let mut total_usd = 0.0;
    let mut companies: Vec<(String, Vec<AssetView>)> = Vec::new();

    let mut sorted_companies: Vec<_> = portfolio.into_iter().collect();
    sorted_companies.sort_by(|a, b| a.0.cmp(&b.0));

    for (company, assets) in sorted_companies {
        let mut asset_views: Vec<AssetView> = assets
            .into_iter()
            .map(|(symbol, (amount, usd_value))| {
                total_usd += usd_value;
                AssetView {
                    symbol,
                    amount,
                    usd_value,
                }
            })
            .collect();

        // Sort by USD value descending
        asset_views.sort_by(|a, b| b.usd_value.partial_cmp(&a.usd_value).unwrap());

        companies.push((company, asset_views));
    }

    Html(
        BalancesTemplate {
            total_usd,
            companies,
            error: String::new(),
        }
        .render()
        .unwrap_or_default(),
    )
}

async fn query_single_balance(Path(name): Path<String>) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            return Html(
                SingleBalanceTemplate {
                    name: name.clone(),
                    address: String::new(),
                    chain: String::new(),
                    native_symbol: String::new(),
                    native_balance: 0.0,
                    native_usd: 0.0,
                    tokens: vec![],
                    total_usd: 0.0,
                    error: format!("Failed to load accounts: {}", e),
                }
                .render()
                .unwrap_or_default(),
            );
        }
    };

    // Try to find in crypto addresses first
    if let Some(wallet) = book.addresses.iter().find(|a| a.name == name) {
        return query_wallet_balance(wallet).await;
    }

    // Try to find in banking accounts
    if let Some(account) = book.banking_accounts.iter().find(|a| a.name == name) {
        return query_bank_balance(account).await;
    }

    Html(
        SingleBalanceTemplate {
            name: name.clone(),
            address: String::new(),
            chain: String::new(),
            native_symbol: String::new(),
            native_balance: 0.0,
            native_usd: 0.0,
            tokens: vec![],
            total_usd: 0.0,
            error: format!("Account '{}' not found", name),
        }
        .render()
        .unwrap_or_default(),
    )
}

async fn query_wallet_balance(wallet: &crate::storage::WalletAddress) -> Html<String> {
    let chain_name = wallet.chain.display_name().to_string();
    let native_symbol = wallet.chain.native_token_symbol().to_string();

    let mut native_balance = 0.0;
    let mut native_usd = 0.0;
    let mut tokens: Vec<TokenView> = vec![];
    let mut total_usd = 0.0;
    let mut error = String::new();

    // Fetch prices
    let price_cache: HashMap<String, f64> = if let Ok(mut price_service) = PriceService::new() {
        price_service
            .batch_fetch_all_known_prices()
            .await
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    match &wallet.chain {
        Chain::Solana => {
            let client = SolanaClient::new(None);
            match client.get_balances(&wallet.address) {
                Ok(balances) => {
                    native_balance = balances.sol_balance;
                    if let Some(&price) = price_cache.get("SOL") {
                        native_usd = native_balance * price;
                        total_usd += native_usd;
                    }
                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            let usd = price_cache
                                .get(symbol)
                                .map(|p| token.ui_amount * p)
                                .unwrap_or(0.0);
                            total_usd += usd;
                            tokens.push(TokenView {
                                symbol: symbol.clone(),
                                balance: token.ui_amount,
                                usd_value: usd,
                            });
                        }
                    }
                }
                Err(e) => error = format!("Failed to query: {}", e),
            }
        }
        Chain::Near => {
            let client = NearClient::new(None);
            match client.get_balances(&wallet.address).await {
                Ok(balances) => {
                    native_balance = balances.near_balance;
                    if let Some(&price) = price_cache.get("NEAR") {
                        native_usd = native_balance * price;
                        total_usd = native_usd;
                    }
                }
                Err(e) => error = format!("Failed to query: {}", e),
            }
        }
        Chain::Aptos => {
            let client = AptosClient::new(None);
            match client.get_balances(&wallet.address).await {
                Ok(balances) => {
                    native_balance = balances.apt_balance;
                    if let Some(&price) = price_cache.get("APT") {
                        native_usd = native_balance * price;
                        total_usd = native_usd;
                    }
                }
                Err(e) => error = format!("Failed to query: {}", e),
            }
        }
        Chain::Sui => {
            let client = SuiClient::new(None);
            match client.get_balances(&wallet.address).await {
                Ok(balances) => {
                    native_balance = balances.sui_balance;
                    if let Some(&price) = price_cache.get("SUI") {
                        native_usd = native_balance * price;
                        total_usd = native_usd;
                    }
                }
                Err(e) => error = format!("Failed to query: {}", e),
            }
        }
        Chain::Starknet => {
            let client = StarknetClient::new(None);
            match client.get_balances(&wallet.address).await {
                Ok(balances) => {
                    native_balance = balances.eth_balance;
                    if let Some(&price) = price_cache.get("ETH") {
                        native_usd = native_balance * price;
                        total_usd = native_usd;
                    }
                }
                Err(e) => error = format!("Failed to query: {}", e),
            }
        }
        Chain::Ethereum
        | Chain::Polygon
        | Chain::BinanceSmartChain
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Base
        | Chain::Core => {
            if let Ok(client) = EvmClient::new(None, wallet.chain.clone()) {
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        native_balance = balances.eth_balance;
                        if let Some(&price) = price_cache.get("ETH") {
                            native_usd = native_balance * price;
                            total_usd += native_usd;
                        }
                        for token in &balances.token_balances {
                            if let Some(symbol) = &token.symbol {
                                let usd = price_cache
                                    .get(symbol)
                                    .map(|p| token.ui_amount * p)
                                    .unwrap_or(0.0);
                                total_usd += usd;
                                tokens.push(TokenView {
                                    symbol: symbol.clone(),
                                    balance: token.ui_amount,
                                    usd_value: usd,
                                });
                            }
                        }
                    }
                    Err(e) => error = format!("Failed to query: {}", e),
                }
            } else {
                error = "Failed to create EVM client".to_string();
            }
        }
    }

    // Sort tokens by USD value
    tokens.sort_by(|a, b| b.usd_value.partial_cmp(&a.usd_value).unwrap_or(std::cmp::Ordering::Equal));

    Html(
        SingleBalanceTemplate {
            name: wallet.name.clone(),
            address: wallet.address.clone(),
            chain: chain_name,
            native_symbol,
            native_balance,
            native_usd,
            tokens,
            total_usd,
            error,
        }
        .render()
        .unwrap_or_default(),
    )
}

async fn query_bank_balance(account: &crate::storage::BankingAccount) -> Html<String> {
    let service_name = account.service.display_name().to_string();

    match &account.service {
        BankingService::Mercury => {
            match MercuryClient::new() {
                Ok(client) => {
                    match client.get_account_balance(&account.account_id).await {
                        Ok(balances) => {
                            Html(
                                SingleBalanceTemplate {
                                    name: account.name.clone(),
                                    address: account.account_id.clone(),
                                    chain: service_name,
                                    native_symbol: "USD".to_string(),
                                    native_balance: balances.current_balance,
                                    native_usd: balances.current_balance,
                                    tokens: vec![],
                                    total_usd: balances.current_balance,
                                    error: String::new(),
                                }
                                .render()
                                .unwrap_or_default(),
                            )
                        }
                        Err(e) => Html(
                            SingleBalanceTemplate {
                                name: account.name.clone(),
                                address: account.account_id.clone(),
                                chain: service_name,
                                native_symbol: String::new(),
                                native_balance: 0.0,
                                native_usd: 0.0,
                                tokens: vec![],
                                total_usd: 0.0,
                                error: format!("Failed to query: {}", e),
                            }
                            .render()
                            .unwrap_or_default(),
                        ),
                    }
                }
                Err(e) => Html(
                    SingleBalanceTemplate {
                        name: account.name.clone(),
                        address: account.account_id.clone(),
                        chain: service_name,
                        native_symbol: String::new(),
                        native_balance: 0.0,
                        native_usd: 0.0,
                        tokens: vec![],
                        total_usd: 0.0,
                        error: format!("Failed to initialize client: {}", e),
                    }
                    .render()
                    .unwrap_or_default(),
                ),
            }
        }
        BankingService::Circle => {
            match CircleClient::new() {
                Ok(client) => {
                    match client.get_balances().await {
                        Ok(balances) => {
                            let mut tokens: Vec<TokenView> = vec![];
                            let mut total = 0.0;
                            for bal in &balances.available_balances {
                                let usd = if bal.currency == "USD" { bal.amount } else { 0.0 };
                                total += usd;
                                tokens.push(TokenView {
                                    symbol: bal.currency.clone(),
                                    balance: bal.amount,
                                    usd_value: usd,
                                });
                            }
                            Html(
                                SingleBalanceTemplate {
                                    name: account.name.clone(),
                                    address: account.account_id.clone(),
                                    chain: service_name,
                                    native_symbol: "USD".to_string(),
                                    native_balance: total,
                                    native_usd: total,
                                    tokens,
                                    total_usd: total,
                                    error: String::new(),
                                }
                                .render()
                                .unwrap_or_default(),
                            )
                        }
                        Err(e) => Html(
                            SingleBalanceTemplate {
                                name: account.name.clone(),
                                address: account.account_id.clone(),
                                chain: service_name,
                                native_symbol: String::new(),
                                native_balance: 0.0,
                                native_usd: 0.0,
                                tokens: vec![],
                                total_usd: 0.0,
                                error: format!("Failed to query: {}", e),
                            }
                            .render()
                            .unwrap_or_default(),
                        ),
                    }
                }
                Err(e) => Html(
                    SingleBalanceTemplate {
                        name: account.name.clone(),
                        address: account.account_id.clone(),
                        chain: service_name,
                        native_symbol: String::new(),
                        native_balance: 0.0,
                        native_usd: 0.0,
                        tokens: vec![],
                        total_usd: 0.0,
                        error: format!("Failed to initialize client: {}", e),
                    }
                    .render()
                    .unwrap_or_default(),
                ),
            }
        }
    }
}

async fn get_transactions(Path(name): Path<String>) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            return Html(
                TransactionsTemplate {
                    name: name.clone(),
                    account_type: String::new(),
                    transactions: vec![],
                    error: format!("Failed to load accounts: {}", e),
                }
                .render()
                .unwrap_or_default(),
            );
        }
    };

    // Check if it's a banking account
    if let Some(account) = book.banking_accounts.iter().find(|a| a.name == name) {
        return get_bank_transactions(account).await;
    }

    // Check if it's a crypto wallet
    if let Some(wallet) = book.addresses.iter().find(|a| a.name == name) {
        return get_wallet_transactions(wallet).await;
    }

    Html(
        TransactionsTemplate {
            name: name.clone(),
            account_type: String::new(),
            transactions: vec![],
            error: format!("Account '{}' not found", name),
        }
        .render()
        .unwrap_or_default(),
    )
}

async fn get_bank_transactions(account: &crate::storage::BankingAccount) -> Html<String> {
    match &account.service {
        BankingService::Mercury => {
            match MercuryClient::new() {
                Ok(client) => {
                    match client.get_transactions(&account.account_id, None, None).await {
                        Ok(txs) => {
                            let transactions: Vec<TransactionView> = txs
                                .iter()
                                .take(50) // Limit to 50 most recent
                                .map(|tx| {
                                    let date = tx.posted_at.as_ref().unwrap_or(&tx.created_at);
                                    let date_formatted = if date.len() >= 10 {
                                        date[..10].to_string()
                                    } else {
                                        date.clone()
                                    };

                                    let tx_type = if tx.amount >= 0.0 {
                                        "deposit".to_string()
                                    } else {
                                        "withdrawal".to_string()
                                    };

                                    let description = tx.bank_description
                                        .clone()
                                        .or(tx.note.clone())
                                        .or(tx.external_memo.clone())
                                        .unwrap_or_else(|| tx.kind.clone());

                                    TransactionView {
                                        date: date_formatted,
                                        description,
                                        amount: tx.amount,
                                        currency: "USD".to_string(),
                                        tx_type,
                                        status: tx.status.clone(),
                                        counterparty: tx.counterparty_name.clone().unwrap_or_default(),
                                    }
                                })
                                .collect();

                            Html(
                                TransactionsTemplate {
                                    name: account.name.clone(),
                                    account_type: "Mercury Banking".to_string(),
                                    transactions,
                                    error: String::new(),
                                }
                                .render()
                                .unwrap_or_default(),
                            )
                        }
                        Err(e) => Html(
                            TransactionsTemplate {
                                name: account.name.clone(),
                                account_type: "Mercury Banking".to_string(),
                                transactions: vec![],
                                error: format!("Failed to fetch transactions: {}", e),
                            }
                            .render()
                            .unwrap_or_default(),
                        ),
                    }
                }
                Err(e) => Html(
                    TransactionsTemplate {
                        name: account.name.clone(),
                        account_type: "Mercury Banking".to_string(),
                        transactions: vec![],
                        error: format!("Failed to initialize client: {}", e),
                    }
                    .render()
                    .unwrap_or_default(),
                ),
            }
        }
        BankingService::Circle => {
            Html(
                TransactionsTemplate {
                    name: account.name.clone(),
                    account_type: "Circle".to_string(),
                    transactions: vec![],
                    error: "Transaction history not available for Circle accounts".to_string(),
                }
                .render()
                .unwrap_or_default(),
            )
        }
    }
}

async fn get_wallet_transactions(wallet: &crate::storage::WalletAddress) -> Html<String> {
    let chain_name = wallet.chain.display_name();
    let explorer_url = match &wallet.chain {
        Chain::Solana => format!("https://solscan.io/account/{}", wallet.address),
        Chain::Ethereum => format!("https://etherscan.io/address/{}", wallet.address),
        Chain::Polygon => format!("https://polygonscan.com/address/{}", wallet.address),
        Chain::BinanceSmartChain => format!("https://bscscan.com/address/{}", wallet.address),
        Chain::Arbitrum => format!("https://arbiscan.io/address/{}", wallet.address),
        Chain::Optimism => format!("https://optimistic.etherscan.io/address/{}", wallet.address),
        Chain::Avalanche => format!("https://snowtrace.io/address/{}", wallet.address),
        Chain::Base => format!("https://basescan.org/address/{}", wallet.address),
        Chain::Core => format!("https://scan.coredao.org/address/{}", wallet.address),
        Chain::Near => format!("https://nearblocks.io/address/{}", wallet.address),
        Chain::Aptos => format!("https://explorer.aptoslabs.com/account/{}", wallet.address),
        Chain::Sui => format!("https://suiscan.xyz/account/{}", wallet.address),
        Chain::Starknet => format!("https://starkscan.co/contract/{}", wallet.address),
    };

    // For Solana, fetch actual transactions
    if let Chain::Solana = &wallet.chain {
        let client = SolanaClient::new(None);
        match client.get_transactions(&wallet.address, 50) {
            Ok(txs) => {
                let transactions: Vec<TransactionView> = txs
                    .iter()
                    .map(|tx| {
                        let date = tx.timestamp
                            .map(|ts| {
                                chrono::DateTime::from_timestamp(ts, 0)
                                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "Unknown".to_string())
                            })
                            .unwrap_or_else(|| "Pending".to_string());

                        let status = if tx.success { "Completed" } else { "Failed" };
                        let description = tx.memo.clone().unwrap_or_else(|| {
                            format!("Slot {}", tx.slot)
                        });

                        // Link to explorer for signature
                        let sig_short = if tx.signature.len() > 16 {
                            format!("{}...", &tx.signature[..16])
                        } else {
                            tx.signature.clone()
                        };

                        let tx_type = if tx.sol_change >= 0.0 {
                            "deposit".to_string()
                        } else {
                            "withdrawal".to_string()
                        };

                        TransactionView {
                            date,
                            description,
                            amount: tx.sol_change,
                            currency: "SOL".to_string(),
                            tx_type,
                            status: status.to_string(),
                            counterparty: sig_short,
                        }
                    })
                    .collect();

                return Html(
                    TransactionsTemplate {
                        name: wallet.name.clone(),
                        account_type: format!("{} Wallet", chain_name),
                        transactions,
                        error: format!("Note: For detailed transaction info, visit <a href=\"{}\" target=\"_blank\">Solscan</a>", explorer_url),
                    }
                    .render()
                    .unwrap_or_default(),
                );
            }
            Err(e) => {
                return Html(
                    TransactionsTemplate {
                        name: wallet.name.clone(),
                        account_type: format!("{} Wallet", chain_name),
                        transactions: vec![],
                        error: format!("Failed to fetch transactions: {}. <a href=\"{}\" target=\"_blank\">View on Solscan</a>", e, explorer_url),
                    }
                    .render()
                    .unwrap_or_default(),
                );
            }
        }
    }

    // For other chains, show a link to the block explorer
    Html(
        TransactionsTemplate {
            name: wallet.name.clone(),
            account_type: format!("{} Wallet", chain_name),
            transactions: vec![],
            error: format!("View transaction history on the block explorer: <a href=\"{}\" target=\"_blank\">{}</a>", explorer_url, explorer_url),
        }
        .render()
        .unwrap_or_default(),
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30sec").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30secs").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30second").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30seconds").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("30min").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("30mins").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("30minute").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("30minutes").unwrap(), Duration::from_secs(30 * 60));
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("4h").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("4hr").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("4hrs").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("4hour").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("4hours").unwrap(), Duration::from_secs(4 * 60 * 60));
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(24 * 60 * 60));
        assert_eq!(parse_duration("1day").unwrap(), Duration::from_secs(24 * 60 * 60));
        assert_eq!(parse_duration("1days").unwrap(), Duration::from_secs(24 * 60 * 60));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(7 * 24 * 60 * 60));
    }

    #[test]
    fn test_parse_duration_whitespace() {
        assert_eq!(parse_duration("  4h  ").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("4 h").unwrap(), Duration::from_secs(4 * 60 * 60));
    }

    #[test]
    fn test_parse_duration_case_insensitive() {
        assert_eq!(parse_duration("4H").unwrap(), Duration::from_secs(4 * 60 * 60));
        assert_eq!(parse_duration("30M").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("1D").unwrap(), Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("h").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("4x").is_err());
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(4 * 3600)), "4h");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(7 * 86400)), "7d");
    }

    #[test]
    fn test_resolve_refresh_interval_cli_priority() {
        // CLI flag should take priority
        let result = resolve_refresh_interval(Some("30m".to_string())).unwrap();
        assert_eq!(result, Duration::from_secs(30 * 60));
    }

    #[test]
    fn test_resolve_refresh_interval_default() {
        // Clear any env var that might be set
        std::env::remove_var("GRINGOTTS_REFRESH_INTERVAL");
        
        let result = resolve_refresh_interval(None).unwrap();
        assert_eq!(result, Duration::from_secs(DEFAULT_REFRESH_INTERVAL_SECS));
    }

    #[test]
    fn test_resolve_refresh_interval_env_var() {
        // Set env var
        std::env::set_var("GRINGOTTS_REFRESH_INTERVAL", "2h");
        
        let result = resolve_refresh_interval(None).unwrap();
        assert_eq!(result, Duration::from_secs(2 * 60 * 60));
        
        // Clean up
        std::env::remove_var("GRINGOTTS_REFRESH_INTERVAL");
    }

    #[test]
    fn test_resolve_refresh_interval_cli_overrides_env() {
        // Set env var
        std::env::set_var("GRINGOTTS_REFRESH_INTERVAL", "2h");
        
        // CLI should override
        let result = resolve_refresh_interval(Some("30m".to_string())).unwrap();
        assert_eq!(result, Duration::from_secs(30 * 60));
        
        // Clean up
        std::env::remove_var("GRINGOTTS_REFRESH_INTERVAL");
    }
}
