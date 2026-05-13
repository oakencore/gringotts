use crate::banking::circle::CircleClient;
use crate::banking::mercury::MercuryClient;
use crate::chains::aptos::AptosClient;
use crate::chains::evm::EvmClient;
use crate::chains::near::NearClient;
use crate::chains::solana::SolanaClient;
use crate::chains::starknet::StarknetClient;
use crate::chains::sui::SuiClient;
use crate::services::cache::{CachedBalance, CachedToken, SharedCache};
use crate::services::price::PriceService;
use crate::storage::{AddressBook, BankingService, Chain};

use askama::Template;
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Form, Router,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

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

        // Handle negative sign separately
        let (sign, digits) = if let Some(d) = integer_part.strip_prefix('-') {
            ("-", d)
        } else {
            ("", integer_part)
        };

        let with_commas: String = digits
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
            format!("{}{}", sign, with_commas)
        } else {
            format!("{}{}.{}", sign, with_commas, decimal_part)
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
    filter: String,
    active_nav: String,
    has_visible_rows: bool,
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
    explorer_url: String,
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

#[allow(dead_code)]
struct GlobalTxView {
    date: String,
    timestamp: i64,
    source_name: String,
    source_chain: String,
    description: String,
    amount: f64,
    currency: String,
    status: String,
    explorer_url: String,
}

impl GlobalTxView {
    #[allow(dead_code)]
    fn from_solana(
        wallet: &crate::storage::WalletAddress,
        tx: &crate::chains::solana::SolanaTransaction,
    ) -> Self {
        // Verified against src/chains/solana.rs:55-62 - the timestamp field is
        // named `timestamp` (Option<i64>), NOT `block_time`.
        let date = match tx.timestamp {
            Some(ts) => chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            None => "pending".to_string(),
        };

        Self {
            date,
            timestamp: tx.timestamp.unwrap_or(0),
            source_name: wallet.name.clone(),
            source_chain: "Solana".to_string(),
            description: tx.memo.clone().unwrap_or_else(|| {
                let sig = &tx.signature;
                if sig.len() > 12 {
                    format!("{}…{}", &sig[..6], &sig[sig.len() - 6..])
                } else {
                    sig.clone()
                }
            }),
            amount: tx.sol_change,
            currency: "SOL".to_string(),
            status: if tx.success {
                "Confirmed".to_string()
            } else {
                "Failed".to_string()
            },
            explorer_url: format!("https://solscan.io/tx/{}", tx.signature),
        }
    }

    // The Mercury transaction type is exported as `Transaction`, not
    // `MercuryTransaction`. Verified against src/banking/mercury.rs:20.
    #[allow(dead_code)]
    fn from_mercury(
        account: &crate::storage::BankingAccount,
        tx: &crate::banking::mercury::Transaction,
    ) -> Self {
        let timestamp_str = tx.posted_at.as_ref().unwrap_or(&tx.created_at);
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let date = if timestamp_str.len() >= 16 {
            timestamp_str[..16].replace('T', " ")
        } else {
            timestamp_str.clone()
        };

        let description = tx
            .bank_description
            .clone()
            .or(tx.note.clone())
            .or(tx.external_memo.clone())
            .or_else(|| tx.counterparty_name.clone())
            .unwrap_or_else(|| tx.kind.clone());

        Self {
            date,
            timestamp,
            source_name: account.name.clone(),
            source_chain: "Mercury".to_string(),
            description,
            amount: tx.amount,
            currency: "USD".to_string(),
            status: tx.status.clone(),
            explorer_url: String::new(),
        }
    }
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

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    port: u16,
    refresh_interval_human: String,
    #[allow(dead_code)]
    refresh_interval_secs: u64,
    last_refresh: String,
    next_refresh: String,
    api_key_enabled: bool,
    cached_wallet_count: usize,
    cached_price_count: usize,
    price_cache_age: String,
    env_vars: Vec<EnvVarStatus>,
    active_nav: String,
}

struct EnvVarStatus {
    name: String,
    configured: bool,
}

#[derive(Deserialize)]
struct RefreshIntervalForm {
    interval: String,
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub cache: SharedCache,
    pub api_key: Option<String>,
    pub port: u16,
    pub refresh_interval_secs: Arc<RwLock<u64>>,
    /// Timestamp of last manual refresh for rate limiting (Unix seconds)
    pub last_manual_refresh: Arc<RwLock<Option<u64>>>,
    /// Whether a refresh is currently in progress
    pub refresh_in_progress: Arc<RwLock<bool>>,
}

/// Query parameters for API key authentication
#[derive(Deserialize)]
struct ApiKeyQuery {
    api_key: Option<String>,
}

/// Query parameters for the dashboard filter
#[derive(Deserialize)]
struct DashboardFilter {
    filter: Option<String>,
}

/// Middleware to validate API key authentication.
/// Checks X-API-Key header first, then ?api_key= query parameter.
/// Returns 401 Unauthorized if key is required but missing/invalid.
/// If no key is configured on the server, all requests are allowed.
async fn api_key_auth(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ApiKeyQuery>,
    request: Request,
    next: Next,
) -> Response {
    // If no API key is configured, allow all requests
    let expected_key = match &state.api_key {
        Some(key) => key,
        None => return next.run(request).await,
    };

    // Check X-API-Key header first
    let header_key = request
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Use header key if present, otherwise fall back to query parameter
    let provided_key = header_key.or(query.api_key);

    match provided_key {
        Some(key) if key == *expected_key => next.run(request).await,
        Some(_) => (StatusCode::UNAUTHORIZED, "Invalid API key").into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            "API key required. Provide via X-API-Key header or ?api_key= query parameter.",
        )
            .into_response(),
    }
}

/// Default refresh interval: 4 hours
const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 4 * 60 * 60;

/// Minimum refresh interval enforced by the Settings POST handler.
const MIN_REFRESH_INTERVAL_SECS: u64 = 30;

/// Environment variables surfaced on the Settings page as configured/not-configured.
const TRACKED_ENV_VARS: &[&str] = &[
    "HELIUS_API_KEY",
    "ALCHEMY_API_KEY",
    "SURGE_API_KEY",
    "MERCURY_API_KEY",
    "CIRCLE_API_KEY",
];

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

/// Maps the optional `filter` query string value to canonical
/// (filter_label, active_nav_label) strings.
fn resolve_dashboard_filter(input: Option<&str>) -> (&'static str, &'static str) {
    match input {
        Some("wallets") => ("wallets", "wallets"),
        Some("banking") => ("banking", "banking"),
        _ => ("all", "dashboard"),
    }
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

/// Compute (last_refresh_display, next_refresh_human) from cache state.
/// `iso_fmt` is the strftime format for the last_refresh display string;
/// `health_check` wants `"%Y-%m-%dT%H:%M:%SZ"`, `settings_page` wants
/// `"%Y-%m-%d %H:%M:%S"`.
fn compute_refresh_times(
    last_full_refresh: Option<u64>,
    interval_secs: u64,
    iso_fmt: &str,
) -> (String, String) {
    match last_full_refresh {
        Some(last_ts) => {
            let last_iso = chrono::DateTime::from_timestamp(last_ts as i64, 0)
                .map(|dt| dt.format(iso_fmt).to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let elapsed = now.saturating_sub(last_ts);
            let next_in = interval_secs.saturating_sub(elapsed);
            (last_iso, format_duration(Duration::from_secs(next_in)))
        }
        None => (
            "never".to_string(),
            format_duration(Duration::from_secs(interval_secs)),
        ),
    }
}

/// Resolve the API key from CLI flag or environment variable.
/// Priority: CLI flag > GRINGOTTS_API_KEY env var
fn resolve_api_key(cli_api_key: Option<String>) -> Option<String> {
    // CLI flag takes priority
    if cli_api_key.is_some() {
        return cli_api_key;
    }

    // Check environment variable
    std::env::var("GRINGOTTS_API_KEY").ok()
}

pub async fn start_server(
    port: u16,
    refresh_interval: Option<String>,
    eager: bool,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    // Resolve refresh interval
    let interval = resolve_refresh_interval(refresh_interval)?;

    // Resolve API key from CLI or env var
    let api_key = resolve_api_key(api_key);
    let interval_display = format_duration(interval);

    // Load address book to verify it exists
    let book = AddressBook::load().unwrap_or_else(|_| AddressBook::new());
    let wallet_count = book.addresses.len();
    let bank_count = book.banking_accounts.len();

    // Initialize shared cache - loads from disk if available
    let cache = SharedCache::new();
    let cache_age = cache.read().await.cache_age_string();

    // Determine if authentication is enabled
    let auth_status = if api_key.is_some() {
        "enabled (X-API-Key header or ?api_key= required)"
    } else {
        "disabled (all requests allowed)"
    };

    let state = Arc::new(AppState {
        cache,
        api_key,
        port,
        refresh_interval_secs: Arc::new(RwLock::new(interval.as_secs())),
        last_manual_refresh: Arc::new(RwLock::new(None)),
        refresh_in_progress: Arc::new(RwLock::new(false)),
    });

    // Determine startup mode description
    let startup_mode = if eager {
        "eager (fetching balances now)"
    } else {
        "lazy (serving cached data)"
    };

    // Routes that require authentication
    let protected_routes = Router::new()
        .route("/", get(index))
        .route("/settings", get(settings_page))
        .route("/settings/refresh-interval", post(update_refresh_interval))
        .route("/transactions", get(global_transactions))
        .route("/accounts", post(add_account))
        .route("/accounts/:name", delete(remove_account))
        .route("/balances", get(query_balances))
        .route("/balances/:name", get(query_single_balance))
        .route("/transactions/:name", get(get_transactions))
        .route("/api/wallets", get(get_wallets_json))
        .route("/api/companies", get(get_companies_json))
        .route("/api/balances", get(get_balances_json))
        .route(
            "/api/balances/company/:company",
            get(get_company_balances_json),
        )
        .route("/api/balances/:address", get(get_single_balance_json))
        .route("/api/totals", get(get_totals_json))
        .route("/api/totals/company/:company", get(get_company_totals_json))
        .route("/api/cache/status", get(cache_status))
        .route("/api/refresh", post(manual_refresh))
        .layer(middleware::from_fn_with_state(state.clone(), api_key_auth));

    // Public routes (no authentication required)
    let public_routes = Router::new().route("/health", get(health_check));

    let app = Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .with_state(state.clone());

    // Bind to 0.0.0.0 to accept connections from local network
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    println!("\n╔═══════════════════════════════════════════════════════════════════════════╗");
    println!("║                    Gringotts Web Server Started                          ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Local:          http://localhost:{}                                   ║",
        port
    );
    println!(
        "║  Network:        http://<your-ip>:{}                                  ║",
        port
    );
    println!("╠═══════════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Address Book:   {} wallets, {} bank accounts                          ║",
        wallet_count, bank_count
    );
    println!("║  Cache:          ~/.gringotts/cache.json                                ║");
    println!("║  Last refresh:   {:>54} ║", cache_age);
    println!("║  Refresh every:  {:>54} ║", interval_display);
    println!("║  Startup mode:   {:>54} ║", startup_mode);
    println!("║  Authentication: {:>54} ║", auth_status);
    println!("╠═══════════════════════════════════════════════════════════════════════════╣");
    println!("║  To find your IP address:                                                ║");
    println!("║    macOS/Linux:  ifconfig | grep 'inet '                                ║");
    println!("║    Windows:      ipconfig                                                ║");
    println!("╚═══════════════════════════════════════════════════════════════════════════╝\n");

    // If eager mode, perform initial refresh before starting the server
    if eager {
        println!(
            "[{}] Eager mode: fetching balances on startup...",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        if let Err(e) = refresh_all_balances(&state).await {
            eprintln!(
                "[{}] Warning: Initial balance fetch failed: {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                e
            );
        } else {
            println!(
                "[{}] Initial balance fetch completed",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }
    } else {
        println!(
            "[{}] Lazy mode: serving cached data until first scheduled refresh",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }

    // Spawn background refresh task
    let refresh_state = state.clone();
    tokio::spawn(async move {
        background_refresh_task(refresh_state).await;
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Background task that refreshes balances on a configurable interval.
/// Reads the current interval from state on each loop iteration so the
/// Settings UI's POST to /settings/refresh-interval takes effect on the
/// next tick.
async fn background_refresh_task(state: Arc<AppState>) {
    // Skip the first immediate tick - don't refresh right at startup
    let initial = *state.refresh_interval_secs.read().await;
    tokio::time::sleep(Duration::from_secs(initial)).await;

    loop {
        println!(
            "[{}] Starting background refresh...",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );

        if let Err(e) = refresh_all_balances(&state).await {
            eprintln!(
                "[{}] Background refresh failed: {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                e
            );
        } else {
            println!(
                "[{}] Background refresh completed",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }

        let next = *state.refresh_interval_secs.read().await;
        tokio::time::sleep(Duration::from_secs(next)).await;
    }
}

/// Refresh all balances and update the cache.
/// Uses graceful partial failure handling: failed wallet queries are logged
/// but do not halt the refresh process. Cached data is preserved for wallets
/// that fail to query (only successful queries update the cache).
async fn refresh_all_balances(state: &Arc<AppState>) -> anyhow::Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        return Ok(());
    }

    let mut all_symbols: HashSet<String> = HashSet::new();
    let mut success_count: usize = 0;
    let mut failure_count: usize = 0;
    let total_items = book.addresses.len() + book.banking_accounts.len();
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    // Query crypto wallets
    for wallet in &book.addresses {
        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(None);
                match client.get_balances(&wallet.address) {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                }
            }
            Chain::Near => {
                let client = NearClient::new(None);
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                }
            }
            Chain::Aptos => {
                let client = AptosClient::new(None);
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                }
            }
            Chain::Sui => {
                let client = SuiClient::new(None);
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                }
            }
            Chain::Starknet => {
                let client = StarknetClient::new(None);
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                }
            }
            Chain::Ethereum
            | Chain::Polygon
            | Chain::BinanceSmartChain
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::Core => match EvmClient::new(None, wallet.chain.clone()) {
                Ok(client) => match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} ({}): {}",
                            timestamp,
                            wallet.name,
                            wallet.chain.display_name(),
                            e
                        );
                        failure_count += 1;
                    }
                },
                Err(e) => {
                    eprintln!(
                        "[{}] Warning: Failed to create EVM client for {} ({}): {}",
                        timestamp,
                        wallet.name,
                        wallet.chain.display_name(),
                        e
                    );
                    failure_count += 1;
                }
            },
        }
    }

    // Query banking accounts
    for account in &book.banking_accounts {
        match &account.service {
            BankingService::Mercury => match MercuryClient::new() {
                Ok(client) => match client.get_account_balance(&account.account_id).await {
                    Ok(balances) => {
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
                        let _ = state
                            .cache
                            .update_balance(&account.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} (Mercury): {}",
                            timestamp, account.name, e
                        );
                        failure_count += 1;
                    }
                },
                Err(e) => {
                    eprintln!(
                        "[{}] Warning: Failed to initialize Mercury client: {}",
                        timestamp, e
                    );
                    failure_count += 1;
                }
            },
            BankingService::Circle => match CircleClient::new() {
                Ok(client) => match client.get_balances().await {
                    Ok(balances) => {
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
                                usd_value: if balance.currency == "USD" {
                                    Some(balance.amount)
                                } else {
                                    None
                                },
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
                        let _ = state
                            .cache
                            .update_balance(&account.name, cached_balance)
                            .await;
                        success_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[{}] Warning: Failed to query {} (Circle): {}",
                            timestamp, account.name, e
                        );
                        failure_count += 1;
                    }
                },
                Err(e) => {
                    eprintln!(
                        "[{}] Warning: Failed to initialize Circle client: {}",
                        timestamp, e
                    );
                    failure_count += 1;
                }
            },
        }
    }

    // Log summary of refresh results
    if failure_count > 0 {
        eprintln!(
            "[{}] Refresh partial: {} of {} succeeded, {} failed (cached data preserved for failures)",
            timestamp, success_count, total_items, failure_count
        );
    }

    // Fetch and cache prices
    if let Ok(mut price_service) = PriceService::new() {
        let symbols: Vec<String> = all_symbols.into_iter().collect();
        match price_service.batch_fetch_prices(&symbols).await {
            Ok(prices) => {
                let _ = state.cache.update_prices(prices).await;
            }
            Err(e) => {
                eprintln!("[{}] Warning: Failed to fetch prices: {}", timestamp, e);
            }
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

/// Rate limit for manual refresh: 5 minutes (300 seconds)
const MANUAL_REFRESH_RATE_LIMIT_SECS: u64 = 5 * 60;

/// API endpoint to trigger a manual balance refresh.
/// Rate-limited to once per 5 minutes.
/// Returns 429 Too Many Requests if rate limited.
/// Returns status indicating whether refresh was started or is already in progress.
async fn manual_refresh(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Atomically check rate limit + in-progress, and set both if allowed
    {
        let mut in_progress = state.refresh_in_progress.write().await;
        if *in_progress {
            let response = serde_json::json!({
                "status": "already_in_progress",
                "message": "A refresh is already in progress"
            });
            return (StatusCode::OK, axum::Json(response));
        }

        let mut last_refresh = state.last_manual_refresh.write().await;
        if let Some(last_ts) = *last_refresh {
            let elapsed = now.saturating_sub(last_ts);
            if elapsed < MANUAL_REFRESH_RATE_LIMIT_SECS {
                let retry_after = MANUAL_REFRESH_RATE_LIMIT_SECS - elapsed;
                let response = serde_json::json!({
                    "status": "rate_limited",
                    "message": "Manual refresh rate limited to once per 5 minutes",
                    "retry_after_seconds": retry_after
                });
                return (StatusCode::TOO_MANY_REQUESTS, axum::Json(response));
            }
        }

        // Both checks passed - atomically set both flags
        *in_progress = true;
        *last_refresh = Some(now);
    }

    // Spawn the refresh task so we don't block the response
    let state_clone = state.clone();
    tokio::spawn(async move {
        println!(
            "[{}] Manual refresh triggered via API",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );

        if let Err(e) = refresh_all_balances(&state_clone).await {
            eprintln!(
                "[{}] Manual refresh failed: {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                e
            );
        } else {
            println!(
                "[{}] Manual refresh completed",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }

        // Mark refresh as complete
        let mut in_progress = state_clone.refresh_in_progress.write().await;
        *in_progress = false;
    });

    let response = serde_json::json!({
        "status": "started",
        "message": "Balance refresh started"
    });
    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to get all balances as JSON.
/// Returns balances nested by company, then wallet, with USD values and metadata.
async fn get_balances_json(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Load address book to get company groupings
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Build a map from wallet/account name to company
    let mut name_to_company: HashMap<String, String> = HashMap::new();
    for wallet in &book.addresses {
        let company = if wallet.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            wallet.company.clone()
        };
        name_to_company.insert(wallet.name.clone(), company);
    }
    for account in &book.banking_accounts {
        let company = if account.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            account.company.clone()
        };
        name_to_company.insert(account.name.clone(), company);
    }

    let cache = state.cache.read().await;

    // Build nested structure: company -> wallets -> balance data
    let mut companies: HashMap<String, serde_json::Value> = HashMap::new();
    let mut total_usd_value = 0.0;

    for (name, entry) in &cache.balances {
        let balance = &entry.data;
        let company = name_to_company
            .get(name)
            .cloned()
            .unwrap_or_else(|| "Uncategorized".to_string());

        // Build token list
        let tokens: Vec<serde_json::Value> = balance
            .tokens
            .iter()
            .map(|t| {
                serde_json::json!({
                    "symbol": t.symbol,
                    "balance": t.balance,
                    "usd_value": t.usd_value
                })
            })
            .collect();

        // Build wallet entry
        let wallet_data = serde_json::json!({
            "address": balance.address_or_id,
            "chain": balance.chain_or_service,
            "native": {
                "symbol": balance.native_symbol,
                "balance": balance.native_balance,
                "usd_value": balance.native_usd_value
            },
            "tokens": tokens,
            "total_usd_value": balance.total_usd_value
        });

        // Add to total
        if let Some(usd) = balance.total_usd_value {
            total_usd_value += usd;
        }

        // Get or create company entry
        let company_entry = companies
            .entry(company.clone())
            .or_insert_with(|| serde_json::json!({ "wallets": {} }));

        // Add wallet to company
        if let Some(wallets) = company_entry.get_mut("wallets") {
            if let Some(wallets_obj) = wallets.as_object_mut() {
                wallets_obj.insert(name.clone(), wallet_data);
            }
        }
    }

    // Calculate company totals
    for (_company_name, company_data) in companies.iter_mut() {
        if let Some(wallets) = company_data.get("wallets") {
            if let Some(wallets_obj) = wallets.as_object() {
                let company_total: f64 = wallets_obj
                    .values()
                    .filter_map(|w| w.get("total_usd_value").and_then(|v| v.as_f64()))
                    .sum();
                if let Some(obj) = company_data.as_object_mut() {
                    obj.insert(
                        "total_usd_value".to_string(),
                        serde_json::json!(company_total),
                    );
                }
            }
        }
    }

    // Format last refresh timestamp as ISO 8601
    let last_refresh_iso = match cache.last_full_refresh {
        Some(ts) => chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        None => "never".to_string(),
    };

    // Get current timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let response = serde_json::json!({
        "timestamp": timestamp,
        "last_refresh": last_refresh_iso,
        "total_usd_value": total_usd_value,
        "companies": companies
    });

    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to get balances for a specific wallet by address.
/// Returns 404 if address not found in address book.
/// Returns same nested structure as full response but for single wallet.
async fn get_single_balance_json(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> impl IntoResponse {
    // Load address book to find the wallet
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Find wallet by address (check both wallet addresses and banking account IDs)
    let wallet = book.addresses.iter().find(|w| w.address == address);
    let banking = book
        .banking_accounts
        .iter()
        .find(|a| a.account_id == address);

    // Determine name and company based on what was found
    let (name, company) = match (wallet, banking) {
        (Some(w), _) => (
            w.name.clone(),
            if w.company.is_empty() {
                "Uncategorized".to_string()
            } else {
                w.company.clone()
            },
        ),
        (_, Some(a)) => (
            a.name.clone(),
            if a.company.is_empty() {
                "Uncategorized".to_string()
            } else {
                a.company.clone()
            },
        ),
        (None, None) => {
            let error = serde_json::json!({
                "error": "Address not found in address book"
            });
            return (StatusCode::NOT_FOUND, axum::Json(error));
        }
    };

    let cache = state.cache.read().await;

    // Look up balance by name in cache
    let balance_entry = match cache.balances.get(&name) {
        Some(entry) => entry,
        None => {
            let error = serde_json::json!({
                "error": "Balance data not found in cache - try refreshing"
            });
            return (StatusCode::NOT_FOUND, axum::Json(error));
        }
    };

    let balance = &balance_entry.data;

    // Build token list
    let tokens: Vec<serde_json::Value> = balance
        .tokens
        .iter()
        .map(|t| {
            serde_json::json!({
                "symbol": t.symbol,
                "balance": t.balance,
                "usd_value": t.usd_value
            })
        })
        .collect();

    // Build wallet entry
    let wallet_data = serde_json::json!({
        "address": balance.address_or_id,
        "chain": balance.chain_or_service,
        "native": {
            "symbol": balance.native_symbol,
            "balance": balance.native_balance,
            "usd_value": balance.native_usd_value
        },
        "tokens": tokens,
        "total_usd_value": balance.total_usd_value
    });

    // Build nested structure matching full response format
    let mut wallets: HashMap<String, serde_json::Value> = HashMap::new();
    wallets.insert(name.clone(), wallet_data);

    let company_total = balance.total_usd_value.unwrap_or(0.0);
    let company_data = serde_json::json!({
        "wallets": wallets,
        "total_usd_value": company_total
    });

    let mut companies: HashMap<String, serde_json::Value> = HashMap::new();
    companies.insert(company.clone(), company_data);

    // Format last refresh timestamp as ISO 8601
    let last_refresh_iso = match cache.last_full_refresh {
        Some(ts) => chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        None => "never".to_string(),
    };

    // Get current timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let response = serde_json::json!({
        "timestamp": timestamp,
        "last_refresh": last_refresh_iso,
        "total_usd_value": company_total,
        "companies": companies
    });

    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to get balances for all wallets tagged with a specific company.
/// Returns 404 if no wallets have that company tag.
/// Returns nested structure with company total and all wallets.
async fn get_company_balances_json(
    State(state): State<Arc<AppState>>,
    Path(company_param): Path<String>,
) -> impl IntoResponse {
    // Load address book to find wallets with matching company tag
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Normalize company name comparison (case-insensitive)
    let company_lower = company_param.to_lowercase();

    // Find all wallet names belonging to this company
    let mut wallet_names: Vec<String> = book
        .addresses
        .iter()
        .filter(|w| {
            let wallet_company = if w.company.is_empty() {
                "uncategorized"
            } else {
                &w.company
            };
            wallet_company.to_lowercase() == company_lower
        })
        .map(|w| w.name.clone())
        .collect();

    // Also include banking accounts with matching company
    let banking_names: Vec<String> = book
        .banking_accounts
        .iter()
        .filter(|a| {
            let account_company = if a.company.is_empty() {
                "uncategorized"
            } else {
                &a.company
            };
            account_company.to_lowercase() == company_lower
        })
        .map(|a| a.name.clone())
        .collect();

    wallet_names.extend(banking_names);

    // Return 404 if no wallets found for this company
    if wallet_names.is_empty() {
        let error = serde_json::json!({
            "error": format!("No wallets found with company tag '{}'", company_param)
        });
        return (StatusCode::NOT_FOUND, axum::Json(error));
    }

    let cache = state.cache.read().await;

    // Build wallets map for this company
    let mut wallets: HashMap<String, serde_json::Value> = HashMap::new();
    let mut company_total: f64 = 0.0;

    for name in &wallet_names {
        if let Some(entry) = cache.balances.get(name) {
            let balance = &entry.data;

            // Build token list
            let tokens: Vec<serde_json::Value> = balance
                .tokens
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "symbol": t.symbol,
                        "balance": t.balance,
                        "usd_value": t.usd_value
                    })
                })
                .collect();

            // Build wallet entry
            let wallet_data = serde_json::json!({
                "address": balance.address_or_id,
                "chain": balance.chain_or_service,
                "native": {
                    "symbol": balance.native_symbol,
                    "balance": balance.native_balance,
                    "usd_value": balance.native_usd_value
                },
                "tokens": tokens,
                "total_usd_value": balance.total_usd_value
            });

            // Add to company total
            if let Some(usd) = balance.total_usd_value {
                company_total += usd;
            }

            wallets.insert(name.clone(), wallet_data);
        }
    }

    // Use the original company name from the first matching wallet for display
    let display_company = book
        .addresses
        .iter()
        .find(|w| w.company.to_lowercase() == company_lower)
        .map(|w| {
            if w.company.is_empty() {
                "Uncategorized".to_string()
            } else {
                w.company.clone()
            }
        })
        .or_else(|| {
            book.banking_accounts
                .iter()
                .find(|a| a.company.to_lowercase() == company_lower)
                .map(|a| {
                    if a.company.is_empty() {
                        "Uncategorized".to_string()
                    } else {
                        a.company.clone()
                    }
                })
        })
        .unwrap_or(company_param.clone());

    // Build company data
    let company_data = serde_json::json!({
        "wallets": wallets,
        "total_usd_value": company_total
    });

    let mut companies: HashMap<String, serde_json::Value> = HashMap::new();
    companies.insert(display_company, company_data);

    // Format last refresh timestamp as ISO 8601
    let last_refresh_iso = match cache.last_full_refresh {
        Some(ts) => chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        None => "never".to_string(),
    };

    // Get current timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let response = serde_json::json!({
        "timestamp": timestamp,
        "last_refresh": last_refresh_iso,
        "total_usd_value": company_total,
        "companies": companies
    });

    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to get aggregated portfolio totals.
/// Returns total USD value with breakdowns by chain and by token.
async fn get_totals_json(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Load address book to get company and chain info
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Build maps from wallet/account name to company and chain
    let mut name_to_company: HashMap<String, String> = HashMap::new();
    let mut name_to_chain: HashMap<String, String> = HashMap::new();
    for wallet in &book.addresses {
        let company = if wallet.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            wallet.company.clone()
        };
        name_to_company.insert(wallet.name.clone(), company);
        name_to_chain.insert(wallet.name.clone(), wallet.chain.display_name().to_string());
    }
    for account in &book.banking_accounts {
        let company = if account.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            account.company.clone()
        };
        name_to_company.insert(account.name.clone(), company);
        name_to_chain.insert(
            account.name.clone(),
            account.service.display_name().to_string(),
        );
    }

    let cache = state.cache.read().await;

    // Aggregate totals
    let mut total_usd_value = 0.0;
    let mut by_chain: HashMap<String, f64> = HashMap::new();
    let mut by_token: HashMap<String, (f64, f64)> = HashMap::new(); // symbol -> (amount, usd_value)
    let mut by_company: HashMap<String, f64> = HashMap::new();

    for (name, entry) in &cache.balances {
        let balance = &entry.data;
        let chain = name_to_chain
            .get(name)
            .cloned()
            .unwrap_or_else(|| balance.chain_or_service.clone());
        let company = name_to_company
            .get(name)
            .cloned()
            .unwrap_or_else(|| "Uncategorized".to_string());

        // Add to total
        if let Some(usd) = balance.total_usd_value {
            total_usd_value += usd;

            // Add to chain breakdown
            *by_chain.entry(chain.clone()).or_insert(0.0) += usd;

            // Add to company breakdown
            *by_company.entry(company).or_insert(0.0) += usd;
        }

        // Add native token to token breakdown
        let native_usd = balance.native_usd_value.unwrap_or(0.0);
        let token_entry = by_token
            .entry(balance.native_symbol.clone())
            .or_insert((0.0, 0.0));
        token_entry.0 += balance.native_balance;
        token_entry.1 += native_usd;

        // Add other tokens to token breakdown
        for token in &balance.tokens {
            let token_usd = token.usd_value.unwrap_or(0.0);
            let entry = by_token.entry(token.symbol.clone()).or_insert((0.0, 0.0));
            entry.0 += token.balance;
            entry.1 += token_usd;
        }
    }

    // Convert by_chain to sorted vec of objects
    let mut chain_totals: Vec<serde_json::Value> = by_chain
        .into_iter()
        .map(|(chain, usd_value)| {
            serde_json::json!({
                "chain": chain,
                "total_usd_value": usd_value
            })
        })
        .collect();
    chain_totals.sort_by(|a, b| {
        let val_a = a
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let val_b = b
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        val_b
            .partial_cmp(&val_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Convert by_token to sorted vec of objects
    let mut token_totals: Vec<serde_json::Value> = by_token
        .into_iter()
        .map(|(symbol, (amount, usd_value))| {
            serde_json::json!({
                "symbol": symbol,
                "total_amount": amount,
                "total_usd_value": usd_value
            })
        })
        .collect();
    token_totals.sort_by(|a, b| {
        let val_a = a
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let val_b = b
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        val_b
            .partial_cmp(&val_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Convert by_company to sorted vec of objects
    let mut company_totals: Vec<serde_json::Value> = by_company
        .into_iter()
        .map(|(company, usd_value)| {
            serde_json::json!({
                "company": company,
                "total_usd_value": usd_value
            })
        })
        .collect();
    company_totals.sort_by(|a, b| {
        let val_a = a
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let val_b = b
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        val_b
            .partial_cmp(&val_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Format last refresh timestamp as ISO 8601
    let last_refresh_iso = match cache.last_full_refresh {
        Some(ts) => chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        None => "never".to_string(),
    };

    // Get current timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let response = serde_json::json!({
        "timestamp": timestamp,
        "last_refresh": last_refresh_iso,
        "total_usd_value": total_usd_value,
        "by_chain": chain_totals,
        "by_token": token_totals,
        "by_company": company_totals
    });

    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to get aggregated totals for a specific company.
/// Returns total USD value for that company with breakdowns by chain and token.
async fn get_company_totals_json(
    State(state): State<Arc<AppState>>,
    Path(company_param): Path<String>,
) -> impl IntoResponse {
    // Load address book to find wallets with matching company tag
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Normalize company name comparison (case-insensitive)
    let company_lower = company_param.to_lowercase();

    // Find all wallet names belonging to this company
    let mut wallet_names: Vec<String> = book
        .addresses
        .iter()
        .filter(|w| {
            let wallet_company = if w.company.is_empty() {
                "uncategorized"
            } else {
                &w.company
            };
            wallet_company.to_lowercase() == company_lower
        })
        .map(|w| w.name.clone())
        .collect();

    // Also include banking accounts with matching company
    let banking_names: Vec<String> = book
        .banking_accounts
        .iter()
        .filter(|a| {
            let account_company = if a.company.is_empty() {
                "uncategorized"
            } else {
                &a.company
            };
            account_company.to_lowercase() == company_lower
        })
        .map(|a| a.name.clone())
        .collect();

    wallet_names.extend(banking_names);

    // Return 404 if no wallets found for this company
    if wallet_names.is_empty() {
        let error = serde_json::json!({
            "error": format!("No wallets found with company tag '{}'", company_param)
        });
        return (StatusCode::NOT_FOUND, axum::Json(error));
    }

    // Build map from wallet name to chain
    let mut name_to_chain: HashMap<String, String> = HashMap::new();
    for wallet in &book.addresses {
        name_to_chain.insert(wallet.name.clone(), wallet.chain.display_name().to_string());
    }
    for account in &book.banking_accounts {
        name_to_chain.insert(
            account.name.clone(),
            account.service.display_name().to_string(),
        );
    }

    let cache = state.cache.read().await;

    // Aggregate totals for this company only
    let mut total_usd_value = 0.0;
    let mut by_chain: HashMap<String, f64> = HashMap::new();
    let mut by_token: HashMap<String, (f64, f64)> = HashMap::new(); // symbol -> (amount, usd_value)

    for name in &wallet_names {
        if let Some(entry) = cache.balances.get(name) {
            let balance = &entry.data;
            let chain = name_to_chain
                .get(name)
                .cloned()
                .unwrap_or_else(|| balance.chain_or_service.clone());

            // Add to total
            if let Some(usd) = balance.total_usd_value {
                total_usd_value += usd;

                // Add to chain breakdown
                *by_chain.entry(chain.clone()).or_insert(0.0) += usd;
            }

            // Add native token to token breakdown
            let native_usd = balance.native_usd_value.unwrap_or(0.0);
            let token_entry = by_token
                .entry(balance.native_symbol.clone())
                .or_insert((0.0, 0.0));
            token_entry.0 += balance.native_balance;
            token_entry.1 += native_usd;

            // Add other tokens to token breakdown
            for token in &balance.tokens {
                let token_usd = token.usd_value.unwrap_or(0.0);
                let entry = by_token.entry(token.symbol.clone()).or_insert((0.0, 0.0));
                entry.0 += token.balance;
                entry.1 += token_usd;
            }
        }
    }

    // Convert by_chain to sorted vec of objects
    let mut chain_totals: Vec<serde_json::Value> = by_chain
        .into_iter()
        .map(|(chain, usd_value)| {
            serde_json::json!({
                "chain": chain,
                "total_usd_value": usd_value
            })
        })
        .collect();
    chain_totals.sort_by(|a, b| {
        let val_a = a
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let val_b = b
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        val_b
            .partial_cmp(&val_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Convert by_token to sorted vec of objects
    let mut token_totals: Vec<serde_json::Value> = by_token
        .into_iter()
        .map(|(symbol, (amount, usd_value))| {
            serde_json::json!({
                "symbol": symbol,
                "total_amount": amount,
                "total_usd_value": usd_value
            })
        })
        .collect();
    token_totals.sort_by(|a, b| {
        let val_a = a
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let val_b = b
            .get("total_usd_value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        val_b
            .partial_cmp(&val_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Use the original company name from the first matching wallet for display
    let display_company = book
        .addresses
        .iter()
        .find(|w| w.company.to_lowercase() == company_lower)
        .map(|w| {
            if w.company.is_empty() {
                "Uncategorized".to_string()
            } else {
                w.company.clone()
            }
        })
        .or_else(|| {
            book.banking_accounts
                .iter()
                .find(|a| a.company.to_lowercase() == company_lower)
                .map(|a| {
                    if a.company.is_empty() {
                        "Uncategorized".to_string()
                    } else {
                        a.company.clone()
                    }
                })
        })
        .unwrap_or(company_param.clone());

    // Format last refresh timestamp as ISO 8601
    let last_refresh_iso = match cache.last_full_refresh {
        Some(ts) => chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        None => "never".to_string(),
    };

    // Get current timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let response = serde_json::json!({
        "timestamp": timestamp,
        "last_refresh": last_refresh_iso,
        "company": display_company,
        "total_usd_value": total_usd_value,
        "by_chain": chain_totals,
        "by_token": token_totals
    });

    (StatusCode::OK, axum::Json(response))
}

/// Health check endpoint - no authentication required.
/// Returns server status, last refresh time, and next scheduled refresh.
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.cache.read().await;
    let interval_secs = *state.refresh_interval_secs.read().await;

    // Calculate time until next refresh based on last refresh timestamp
    let (last_refresh_iso, next_refresh_in) =
        compute_refresh_times(cache.last_full_refresh, interval_secs, "%Y-%m-%dT%H:%M:%SZ");

    let status = serde_json::json!({
        "status": "healthy",
        "last_refresh": last_refresh_iso,
        "next_refresh_in": next_refresh_in,
    });

    (StatusCode::OK, axum::Json(status))
}

/// Settings page - renders runtime config, cache status, and env-var configured/not-configured.
async fn settings_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.cache.read().await;
    let interval_secs = *state.refresh_interval_secs.read().await;
    let interval = Duration::from_secs(interval_secs);

    let (last_refresh_iso, next_refresh) =
        compute_refresh_times(cache.last_full_refresh, interval_secs, "%Y-%m-%d %H:%M:%S");

    let cached_wallet_count = cache.balances.len();
    // cache.prices is CacheEntry<HashMap<...>>, so the actual map lives at .data.
    let cached_price_count = cache.prices.data.len();
    let price_cache_age = cache.cache_age_string();

    let env_vars: Vec<EnvVarStatus> = TRACKED_ENV_VARS
        .iter()
        .map(|name| EnvVarStatus {
            name: (*name).to_string(),
            configured: std::env::var(name).is_ok(),
        })
        .collect();

    Html(
        SettingsTemplate {
            port: state.port,
            refresh_interval_human: format_duration(interval),
            refresh_interval_secs: interval_secs,
            last_refresh: last_refresh_iso,
            next_refresh,
            api_key_enabled: state.api_key.is_some(),
            cached_wallet_count,
            cached_price_count,
            price_cache_age,
            env_vars,
            active_nav: "settings".to_string(),
        }
        .render()
        .unwrap_or_default(),
    )
}

/// Update the in-memory refresh interval. Session-only; CLI flag is for persistence.
async fn update_refresh_interval(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RefreshIntervalForm>,
) -> impl IntoResponse {
    let parsed = match parse_duration(&form.interval) {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("Invalid interval: {}", e)).into_response();
        }
    };

    let new_secs = parsed.as_secs();
    if new_secs < MIN_REFRESH_INTERVAL_SECS {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "Interval must be at least {} seconds",
                MIN_REFRESH_INTERVAL_SECS
            ),
        )
            .into_response();
    }

    *state.refresh_interval_secs.write().await = new_secs;

    // Redirect back to /settings so the browser follows after the plain form POST.
    axum::response::Redirect::to("/settings").into_response()
}

/// Stub for the global transactions page - real implementation in Task 7.
async fn global_transactions(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    Html("<p>Coming soon</p>".to_string())
}

/// API endpoint to list all tracked wallet addresses with metadata.
/// Returns wallet name, address, chain, and company tag for each tracked wallet.
async fn get_wallets_json() -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    let wallets: Vec<serde_json::Value> = book
        .addresses
        .iter()
        .map(|w| {
            serde_json::json!({
                "name": w.name,
                "address": w.address,
                "chain": w.chain.display_name(),
                "company": if w.company.is_empty() { None } else { Some(&w.company) }
            })
        })
        .collect();

    let banking_accounts: Vec<serde_json::Value> = book
        .banking_accounts
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "account_id": a.account_id,
                "service": a.service.display_name(),
                "company": if a.company.is_empty() { None } else { Some(&a.company) }
            })
        })
        .collect();

    let response = serde_json::json!({
        "wallets": wallets,
        "banking_accounts": banking_accounts,
        "total_wallets": wallets.len(),
        "total_banking_accounts": banking_accounts.len()
    });

    (StatusCode::OK, axum::Json(response))
}

/// API endpoint to list all unique company tags.
/// Returns sorted list of company names with wallet counts.
async fn get_companies_json() -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            let error = serde_json::json!({
                "error": format!("Failed to load address book: {}", e)
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error));
        }
    };

    // Count wallets and banking accounts per company
    let mut company_counts: HashMap<String, (usize, usize)> = HashMap::new();

    for wallet in &book.addresses {
        let company = if wallet.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            wallet.company.clone()
        };
        let entry = company_counts.entry(company).or_insert((0, 0));
        entry.0 += 1;
    }

    for account in &book.banking_accounts {
        let company = if account.company.is_empty() {
            "Uncategorized".to_string()
        } else {
            account.company.clone()
        };
        let entry = company_counts.entry(company).or_insert((0, 0));
        entry.1 += 1;
    }

    // Convert to sorted list
    let mut companies: Vec<serde_json::Value> = company_counts
        .into_iter()
        .map(|(name, (wallet_count, banking_count))| {
            serde_json::json!({
                "name": name,
                "wallet_count": wallet_count,
                "banking_account_count": banking_count,
                "total_accounts": wallet_count + banking_count
            })
        })
        .collect();

    // Sort alphabetically, but put "Uncategorized" last
    companies.sort_by(|a, b| {
        let name_a = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let name_b = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name_a == "Uncategorized" {
            std::cmp::Ordering::Greater
        } else if name_b == "Uncategorized" {
            std::cmp::Ordering::Less
        } else {
            name_a.cmp(name_b)
        }
    });

    let response = serde_json::json!({
        "companies": companies,
        "total_companies": companies.len()
    });

    (StatusCode::OK, axum::Json(response))
}

async fn index(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(_) => AddressBook::new(),
    };

    let wallet_count = book.addresses.len();
    let bank_count = book.banking_accounts.len();

    // Map filter query param to canonical strings
    let (filter, active_nav) = resolve_dashboard_filter(q.filter.as_deref());
    let filter = filter.to_string();
    let active_nav = active_nav.to_string();

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

    // Compute has_visible_rows AFTER companies is built
    let has_visible_rows = match filter.as_str() {
        "wallets" => companies.iter().any(|c| !c.wallets.is_empty()),
        "banking" => companies.iter().any(|c| !c.banking_accounts.is_empty()),
        _ => !companies.is_empty(),
    };

    Html(
        IndexTemplate {
            companies,
            wallet_count,
            bank_count,
            filter,
            active_nav,
            has_visible_rows,
        }
        .render()
        .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
}

async fn add_account(Form(form): Form<AddAccountForm>) -> impl IntoResponse {
    let mut book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("Error: {}", e)),
            )
        }
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("Error: {}", e)),
        );
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
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("Error: {}", e)),
            )
        }
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("Error: {}", e)),
        );
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
                    let _ = state
                        .cache
                        .update_balance(&wallet.name, cached_balance)
                        .await;
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
                    let _ = state
                        .cache
                        .update_balance(&wallet.name, cached_balance)
                        .await;
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
                    let _ = state
                        .cache
                        .update_balance(&wallet.name, cached_balance)
                        .await;
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
                    let _ = state
                        .cache
                        .update_balance(&wallet.name, cached_balance)
                        .await;
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
                    let _ = state
                        .cache
                        .update_balance(&wallet.name, cached_balance)
                        .await;
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
                        let _ = state
                            .cache
                            .update_balance(&wallet.name, cached_balance)
                            .await;
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
                        let _ = state
                            .cache
                            .update_balance(&account.name, cached_balance)
                            .await;
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
                            let currency_entry =
                                entry.entry(symbol.to_string()).or_insert((0.0, 0.0));
                            currency_entry.0 += balance.amount;
                            if balance.currency == "USD" {
                                currency_entry.1 += balance.amount;
                                total_usd += balance.amount;
                            }
                            cached_tokens.push(CachedToken {
                                symbol: symbol.to_string(),
                                balance: balance.amount,
                                usd_value: if balance.currency == "USD" {
                                    Some(balance.amount)
                                } else {
                                    None
                                },
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
                        let _ = state
                            .cache
                            .update_balance(&account.name, cached_balance)
                            .await;
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
                        if let Some(&price) = price_cache.get(&balances.native_symbol) {
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
    tokens.sort_by(|a, b| {
        b.usd_value
            .partial_cmp(&a.usd_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
        BankingService::Mercury => match MercuryClient::new() {
            Ok(client) => match client.get_account_balance(&account.account_id).await {
                Ok(balances) => Html(
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
                ),
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
            },
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
        },
        BankingService::Circle => match CircleClient::new() {
            Ok(client) => match client.get_balances().await {
                Ok(balances) => {
                    let mut tokens: Vec<TokenView> = vec![];
                    let mut total = 0.0;
                    for bal in &balances.available_balances {
                        let usd = if bal.currency == "USD" {
                            bal.amount
                        } else {
                            0.0
                        };
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
            },
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
        },
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
                    explorer_url: String::new(),
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
            explorer_url: String::new(),
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
                    match client
                        .get_transactions(&account.account_id, None, None)
                        .await
                    {
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

                                    let description = tx
                                        .bank_description
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
                                        counterparty: tx
                                            .counterparty_name
                                            .clone()
                                            .unwrap_or_default(),
                                    }
                                })
                                .collect();

                            Html(
                                TransactionsTemplate {
                                    name: account.name.clone(),
                                    account_type: "Mercury Banking".to_string(),
                                    transactions,
                                    error: String::new(),
                                    explorer_url: String::new(),
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
                                explorer_url: String::new(),
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
                        explorer_url: String::new(),
                    }
                    .render()
                    .unwrap_or_default(),
                ),
            }
        }
        BankingService::Circle => Html(
            TransactionsTemplate {
                name: account.name.clone(),
                account_type: "Circle".to_string(),
                transactions: vec![],
                error: "Transaction history not available for Circle accounts".to_string(),
                explorer_url: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ),
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
                        let date = tx
                            .timestamp
                            .map(|ts| {
                                chrono::DateTime::from_timestamp(ts, 0)
                                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "Unknown".to_string())
                            })
                            .unwrap_or_else(|| "Pending".to_string());

                        let status = if tx.success { "Completed" } else { "Failed" };
                        let description = tx
                            .memo
                            .clone()
                            .unwrap_or_else(|| format!("Slot {}", tx.slot));

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
                        error: "Note: For detailed transaction info, visit the block explorer."
                            .to_string(),
                        explorer_url: explorer_url.clone(),
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
                        error: format!("Failed to fetch transactions: {}", e),
                        explorer_url: explorer_url.clone(),
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
            error: "View transaction history on the block explorer.".to_string(),
            explorer_url,
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
        assert_eq!(
            parse_duration("30seconds").unwrap(),
            Duration::from_secs(30)
        );
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(
            parse_duration("30min").unwrap(),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            parse_duration("30mins").unwrap(),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            parse_duration("30minute").unwrap(),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            parse_duration("30minutes").unwrap(),
            Duration::from_secs(30 * 60)
        );
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(
            parse_duration("4h").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            parse_duration("4hr").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            parse_duration("4hrs").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            parse_duration("4hour").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            parse_duration("4hours").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(
            parse_duration("1d").unwrap(),
            Duration::from_secs(24 * 60 * 60)
        );
        assert_eq!(
            parse_duration("1day").unwrap(),
            Duration::from_secs(24 * 60 * 60)
        );
        assert_eq!(
            parse_duration("1days").unwrap(),
            Duration::from_secs(24 * 60 * 60)
        );
        assert_eq!(
            parse_duration("7d").unwrap(),
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }

    #[test]
    fn test_parse_duration_whitespace() {
        assert_eq!(
            parse_duration("  4h  ").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            parse_duration("4 h").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
    }

    #[test]
    fn test_parse_duration_case_insensitive() {
        assert_eq!(
            parse_duration("4H").unwrap(),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(parse_duration("30M").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(
            parse_duration("1D").unwrap(),
            Duration::from_secs(24 * 60 * 60)
        );
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

    #[test]
    fn test_resolve_api_key_cli_priority() {
        // CLI flag should take priority
        let result = resolve_api_key(Some("cli-key".to_string()));
        assert_eq!(result, Some("cli-key".to_string()));
    }

    #[test]
    fn test_resolve_api_key_env_var() {
        // Set env var
        std::env::set_var("GRINGOTTS_API_KEY", "env-key");

        let result = resolve_api_key(None);
        assert_eq!(result, Some("env-key".to_string()));

        // Clean up
        std::env::remove_var("GRINGOTTS_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_cli_overrides_env() {
        // Set env var
        std::env::set_var("GRINGOTTS_API_KEY", "env-key");

        // CLI should override
        let result = resolve_api_key(Some("cli-key".to_string()));
        assert_eq!(result, Some("cli-key".to_string()));

        // Clean up
        std::env::remove_var("GRINGOTTS_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_none() {
        // Clear any env var that might be set
        std::env::remove_var("GRINGOTTS_API_KEY");

        let result = resolve_api_key(None);
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_health_check_response() {
        use crate::services::cache::SharedCache;
        use axum::extract::State;

        // Create app state with fresh cache
        let state = Arc::new(AppState {
            cache: SharedCache::new(),
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)), // 1 hour
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        // Call health check
        let response = health_check(State(state)).await;
        let (status, _body) = response.into_response().into_parts();

        // Status should be OK (200)
        assert_eq!(status.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_check_with_refresh() {
        use crate::services::cache::SharedCache;
        use axum::extract::State;

        // Create app state with fresh cache
        let cache = SharedCache::new();

        // Mark a refresh
        cache.mark_refresh().await.unwrap();

        let state = Arc::new(AppState {
            cache,
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)), // 1 hour
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        // Call health check
        let response = health_check(State(state)).await;
        let (status, _body) = response.into_response().into_parts();

        // Status should be OK (200)
        assert_eq!(status.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_balances_json_response_structure() {
        use crate::services::cache::{CachedBalance, CachedToken, SharedCache};
        use axum::extract::State;

        // Create app state with fresh cache
        let cache = SharedCache::new();

        // Add a test balance to the cache
        let test_balance = CachedBalance {
            name: "Test Wallet".to_string(),
            address_or_id: "test123".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 10.0,
            native_usd_value: Some(1000.0),
            tokens: vec![CachedToken {
                symbol: "USDC".to_string(),
                balance: 500.0,
                usd_value: Some(500.0),
            }],
            total_usd_value: Some(1500.0),
        };
        cache
            .update_balance("Test Wallet", test_balance)
            .await
            .unwrap();

        // Mark a refresh
        cache.mark_refresh().await.unwrap();

        let state = Arc::new(AppState {
            cache,
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)),
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        // Call the endpoint
        let response = get_balances_json(State(state)).await;
        let (parts, _body) = response.into_response().into_parts();

        // Status should be OK (200)
        assert_eq!(parts.status, StatusCode::OK);
    }

    #[test]
    fn test_resolve_dashboard_filter() {
        assert_eq!(resolve_dashboard_filter(None), ("all", "dashboard"));
        assert_eq!(
            resolve_dashboard_filter(Some("wallets")),
            ("wallets", "wallets")
        );
        assert_eq!(
            resolve_dashboard_filter(Some("banking")),
            ("banking", "banking")
        );
        assert_eq!(
            resolve_dashboard_filter(Some("garbage")),
            ("all", "dashboard")
        );
        assert_eq!(resolve_dashboard_filter(Some("")), ("all", "dashboard"));
    }

    #[tokio::test]
    async fn test_update_refresh_interval_rejects_garbage() {
        use crate::services::cache::SharedCache;
        use axum::extract::{Form, State};
        use axum::response::IntoResponse;

        let state = Arc::new(AppState {
            cache: SharedCache::new(),
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)),
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        let form = Form(RefreshIntervalForm {
            interval: "not-a-duration".to_string(),
        });

        let response = update_refresh_interval(State(state), form).await;
        let (parts, _body) = response.into_response().into_parts();
        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_update_refresh_interval_writes_state() {
        use crate::services::cache::SharedCache;
        use axum::extract::{Form, State};

        let state = Arc::new(AppState {
            cache: SharedCache::new(),
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)),
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        let form = Form(RefreshIntervalForm {
            interval: "2h".to_string(),
        });

        let _response = update_refresh_interval(State(state.clone()), form).await;
        let stored = *state.refresh_interval_secs.read().await;
        assert_eq!(stored, 7200);
    }

    #[tokio::test]
    async fn test_update_refresh_interval_rejects_too_short() {
        use crate::services::cache::SharedCache;
        use axum::extract::{Form, State};
        use axum::response::IntoResponse;

        let state = Arc::new(AppState {
            cache: SharedCache::new(),
            api_key: None,
            port: 3000,
            refresh_interval_secs: Arc::new(RwLock::new(3600)),
            last_manual_refresh: Arc::new(RwLock::new(None)),
            refresh_in_progress: Arc::new(RwLock::new(false)),
        });

        let form = Form(RefreshIntervalForm {
            interval: "5s".to_string(),
        });

        let response = update_refresh_interval(State(state), form).await;
        let (parts, _body) = response.into_response().into_parts();
        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_global_tx_view_sort_and_truncate() {
        let mut rows: Vec<GlobalTxView> = (0..150)
            .map(|i| GlobalTxView {
                date: "2026-05-13".to_string(),
                timestamp: i as i64,
                source_name: "test".to_string(),
                source_chain: "Solana".to_string(),
                description: "".to_string(),
                amount: 0.0,
                currency: "SOL".to_string(),
                status: "Confirmed".to_string(),
                explorer_url: "".to_string(),
            })
            .collect();

        rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        rows.truncate(100);

        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0].timestamp, 149);
        assert_eq!(rows[99].timestamp, 50);
    }
}
