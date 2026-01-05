use crate::aptos::AptosClient;
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
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
    Form, Router,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

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

pub async fn start_server(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/accounts", post(add_account))
        .route("/accounts/:name", delete(remove_account))
        .route("/balances", get(query_balances))
        .route("/balances/:name", get(query_single_balance))
        .route("/transactions/:name", get(get_transactions));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Starting Gringotts web server at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
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

async fn query_balances() -> impl IntoResponse {
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

                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            all_symbols.insert(symbol.clone());
                            let token_entry = entry.entry(symbol.clone()).or_insert((0.0, 0.0));
                            token_entry.0 += token.ui_amount;
                        }
                    }
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

                        for token in &balances.token_balances {
                            if let Some(symbol) = &token.symbol {
                                all_symbols.insert(symbol.clone());
                                let token_entry = entry.entry(symbol.clone()).or_insert((0.0, 0.0));
                                token_entry.0 += token.ui_amount;
                            }
                        }
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
                            }
                        }
                    }
                }
            }
        }
    }

    // Fetch prices for crypto assets
    if let Ok(price_service) = PriceService::new() {
        let symbols: Vec<String> = all_symbols.into_iter().collect();
        if let Ok(prices) = price_service.batch_fetch_prices(&symbols).await {
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
    let price_cache: HashMap<String, f64> = if let Ok(price_service) = PriceService::new() {
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
