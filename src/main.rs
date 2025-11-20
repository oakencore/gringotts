mod aptos;
mod circle;
mod cli;
mod evm;
mod mercury;
mod near;
mod price;
mod solana;
mod starknet;
mod storage;
mod sui;
mod ui;

use anyhow::Result;
use aptos::AptosClient;
use circle::CircleClient;
use clap::Parser;
use cli::{Cli, Commands};
use evm::EvmClient;
use mercury::MercuryClient;
use near::NearClient;
use price::PriceService;
use solana::SolanaClient;
use starknet::StarknetClient;
use storage::{AddressBook, BankingAccount, BankingService, Chain, WalletAddress};
use sui::SuiClient;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug)]
pub struct AssetBalance {
    pub symbol: String,
    pub total_amount: f64,
    pub total_usd_value: f64,
}

#[derive(Debug)]
pub struct CompanySummary {
    pub company: String,
    pub assets: HashMap<String, AssetBalance>,
    pub total_usd_value: f64,
}

#[derive(Debug)]
pub struct PortfolioSummary {
    pub companies: HashMap<String, CompanySummary>,
    pub total_usd_value: f64,
}

// Helper functions for portfolio aggregation
fn get_company_key(company: &str) -> &str {
    if company.is_empty() { "Uncategorized" } else { company }
}

fn add_asset_to_portfolio(
    portfolio: &mut PortfolioSummary,
    company: &str,
    symbol: &str,
    amount: f64,
    usd_value: Option<f64>,
) {
    let company_key = get_company_key(company);
    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert_with(|| CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    let entry = company_summary.assets.entry(symbol.to_string()).or_insert(AssetBalance {
        symbol: symbol.to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    entry.total_amount += amount;
    if let Some(value) = usd_value {
        entry.total_usd_value += value;
        company_summary.total_usd_value += value;
        portfolio.total_usd_value += value;
    }
}

// Struct to hold wallet + balances during query phase
enum WalletBalances {
    Solana(WalletAddress, solana::AccountBalances),
    Evm(WalletAddress, evm::AccountBalances),
    Near(WalletAddress, near::AccountBalances),
    Aptos(WalletAddress, aptos::AccountBalances),
    Sui(WalletAddress, sui::AccountBalances),
    Starknet(WalletAddress, starknet::AccountBalances),
    Mercury(BankingAccount, mercury::AccountBalances),
    Circle(BankingAccount, circle::AccountBalances),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { company, name, address, chain } => {
            add_address(company, name, address, chain)?;
        }
        Commands::AddBank { company, name, account_id, service } => {
            add_banking_account(company, name, account_id, service)?;
        }
        Commands::List { company } => {
            list_addresses(company)?;
        }
        Commands::Remove { identifier } => {
            remove_address(identifier)?;
        }
        Commands::Query { rpc_url, no_prices } => {
            query_all(rpc_url, no_prices).await?;
        }
        Commands::QueryOne { name, rpc_url, no_prices } => {
            query_one(name, rpc_url, no_prices).await?;
        }
        Commands::ListMercuryAccounts => {
            list_mercury_accounts().await?;
        }
        Commands::SetupMercury { company } => {
            setup_mercury(company).await?;
        }
        Commands::ExportTransactions { name, format, start, end, output } => {
            export_transactions(name, format, start, end, output).await?;
        }
    }

    Ok(())
}

fn add_address(company: String, name: String, address: String, chain: Option<String>) -> Result<()> {
    let mut book = AddressBook::load()?;
    book.add_address(company, name.clone(), address.clone(), chain)?;
    book.save()?;

    ui::render_success(&format!("Added address '{}': {}", name, address));
    Ok(())
}

fn add_banking_account(company: String, name: String, account_id: String, service: String) -> Result<()> {
    let mut book = AddressBook::load()?;
    book.add_banking_account(company, name.clone(), account_id.clone(), service)?;
    book.save()?;

    ui::render_success(&format!("Added banking account '{}': {}", name, account_id));
    Ok(())
}

fn list_addresses(company_filter: Option<String>) -> Result<()> {
    let book = AddressBook::load()?;

    let (addresses, banking_accounts) = match company_filter {
        Some(ref filter) => {
            let filter_lower = filter.to_lowercase();
            let filtered_addresses: Vec<_> = book
                .addresses
                .iter()
                .filter(|a| a.company.to_lowercase().contains(&filter_lower))
                .cloned()
                .collect();
            let filtered_banking: Vec<_> = book
                .banking_accounts
                .iter()
                .filter(|a| a.company.to_lowercase().contains(&filter_lower))
                .cloned()
                .collect();
            (filtered_addresses, filtered_banking)
        }
        None => (book.addresses.clone(), book.banking_accounts.clone()),
    };

    ui::render_addresses(&addresses, &banking_accounts);
    Ok(())
}

fn remove_address(identifier: String) -> Result<()> {
    let mut book = AddressBook::load()?;

    // Try removing from addresses first
    let removed_crypto = book.remove_by_identifier(&identifier).is_ok();

    // If not found in addresses, try banking accounts
    let removed_bank = if !removed_crypto {
        book.remove_banking_account_by_identifier(&identifier).is_ok()
    } else {
        false
    };

    if !removed_crypto && !removed_bank {
        anyhow::bail!("Address or account with name '{}' not found", identifier);
    }

    book.save()?;
    ui::render_success(&format!("Removed '{}'", identifier));
    Ok(())
}

async fn query_all(rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        println!("No addresses or accounts tracked yet.");
        println!("Use 'gringotts add' to add blockchain addresses.");
        println!("Use 'gringotts add-bank' to add banking accounts.");
        return Ok(());
    }

    if no_prices {
        println!("\nQuerying balances for all tracked addresses and accounts (without prices)...\n");
    } else {
        println!("\nQuerying balances for all tracked addresses and accounts...\n");
    }

    // Phase 1: Query all balances (without prices)
    let total_items = book.addresses.len() + book.banking_accounts.len();
    let pb = ProgressBar::new(total_items as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} items ({eta})")
            .expect("valid progress bar template")
            .progress_chars("#>-")
    );
    pb.set_message("Fetching balances...");

    let mut all_balances: Vec<WalletBalances> = Vec::new();

    for wallet in book.addresses.iter() {
        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(rpc_url.clone());
                match client.get_balances(&wallet.address) {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Solana(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
            Chain::Near => {
                let client = NearClient::new(rpc_url.clone());
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Near(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
            Chain::Aptos => {
                let client = AptosClient::new(rpc_url.clone());
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Aptos(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
            Chain::Sui => {
                let client = SuiClient::new(rpc_url.clone());
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Sui(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
            Chain::Starknet => {
                let client = StarknetClient::new(rpc_url.clone());
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Starknet(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
            // All EVM chains
            Chain::Ethereum | Chain::Polygon | Chain::BinanceSmartChain | Chain::Arbitrum
            | Chain::Optimism | Chain::Avalanche | Chain::Base | Chain::Core => {
                match EvmClient::new(rpc_url.clone(), wallet.chain.clone()) {
                    Ok(client) => match client.get_balances(&wallet.address).await {
                        Ok(balances) => {
                            all_balances.push(WalletBalances::Evm(wallet.clone(), balances));
                        }
                        Err(e) => {
                            pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                        }
                    },
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to create EVM client for {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
        }
        pb.inc(1);
    }

    // Query banking accounts
    for account in book.banking_accounts.iter() {
        match &account.service {
            BankingService::Mercury => {
                match MercuryClient::new() {
                    Ok(client) => {
                        match client.get_account_balance(&account.account_id).await {
                            Ok(balances) => {
                                all_balances.push(WalletBalances::Mercury(account.clone(), balances));
                            }
                            Err(e) => {
                                pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", account.name, account.account_id, e));
                            }
                        }
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to initialize Mercury client: {}", e));
                    }
                }
            }
            BankingService::Circle => {
                match CircleClient::new() {
                    Ok(client) => {
                        match client.get_balances().await {
                            Ok(balances) => {
                                all_balances.push(WalletBalances::Circle(account.clone(), balances));
                            }
                            Err(e) => {
                                pb.println(format!("⚠ Warning: Failed to query {} Circle balances: {}", account.name, e));
                            }
                        }
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to initialize Circle client: {}", e));
                    }
                }
            }
        }
        pb.inc(1);
    }

    pb.finish_with_message(format!("✓ Successfully fetched balances from {} items", all_balances.len()));
    println!();

    // Phase 2 & 3: Extract symbols and fetch prices (skip if --no-prices)
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    if !no_prices {
        // Phase 2: Extract all unique token symbols
        let mut symbols: HashSet<String> = HashSet::new();
        for wallet_balance in &all_balances {
            match wallet_balance {
                WalletBalances::Solana(_, balances) => {
                    symbols.insert("SOL".to_string());
                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            symbols.insert(symbol.clone());
                        }
                    }
                }
                WalletBalances::Evm(_, balances) => {
                    symbols.insert("ETH".to_string());
                    for token in &balances.token_balances {
                        if let Some(symbol) = &token.symbol {
                            symbols.insert(symbol.clone());
                        }
                    }
                }
                WalletBalances::Near(_, _) => {
                    symbols.insert("NEAR".to_string());
                }
                WalletBalances::Aptos(_, _) => {
                    symbols.insert("APT".to_string());
                }
                WalletBalances::Sui(_, _) => {
                    symbols.insert("SUI".to_string());
                }
                WalletBalances::Starknet(_, _) => {
                    symbols.insert("ETH".to_string());
                }
                WalletBalances::Mercury(_, _) => {
                    // Mercury balances are already in USD, no price lookup needed
                }
                WalletBalances::Circle(_, _) => {
                    // Circle balances are already in USD/EUR, no price lookup needed
                }
            }
        }

        // Phase 3: Batch fetch prices for all symbols
        let price_service = PriceService::new()?;

        if !symbols.is_empty() {
            let price_pb = ProgressBar::new_spinner();
            price_pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .expect("valid spinner template")
            );
            price_pb.set_message(format!("Fetching USD prices for {} unique tokens...", symbols.len()));
            price_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            let symbols_vec: Vec<String> = symbols.into_iter().collect();
            match price_service.batch_fetch_prices(&symbols_vec).await {
                Ok(prices) => {
                    price_cache = prices;
                    price_pb.finish_with_message(format!("✓ Successfully fetched prices for {} symbols", price_cache.len()));
                }
                Err(e) => {
                    price_pb.finish_with_message(format!("⚠ Failed to fetch prices: {}", e));
                    price_pb.println("Balances will be displayed without USD values.");
                }
            }
            println!();
        }
    }

    // Phase 4: Enrich balances with cached prices and display
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    for wallet_balance in all_balances {
        match wallet_balance {
            WalletBalances::Solana(wallet, mut balances) => {
                enrich_solana_from_cache(&mut balances, &price_cache);
                ui::render_solana_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_solana_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Evm(wallet, mut balances) => {
                enrich_evm_from_cache(&mut balances, &price_cache);
                ui::render_evm_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_evm_balances(&mut portfolio, &wallet.company, &balances, &wallet.chain);
            }
            WalletBalances::Near(wallet, mut balances) => {
                enrich_near_from_cache(&mut balances, &price_cache);
                ui::render_near_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_near_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Aptos(wallet, mut balances) => {
                enrich_aptos_from_cache(&mut balances, &price_cache);
                ui::render_aptos_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_aptos_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Sui(wallet, mut balances) => {
                enrich_sui_from_cache(&mut balances, &price_cache);
                ui::render_sui_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_sui_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Starknet(wallet, mut balances) => {
                enrich_starknet_from_cache(&mut balances, &price_cache);
                ui::render_starknet_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_starknet_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Mercury(account, balances) => {
                ui::render_mercury_balances(&account.company, &account.name, &account.account_id, &balances, &account.service);
                aggregate_mercury_balances(&mut portfolio, &account.company, &balances);
            }
            WalletBalances::Circle(account, balances) => {
                ui::render_circle_balances(&account.company, &account.name, &balances, &account.service);
                aggregate_circle_balances(&mut portfolio, &account.company, &balances);
            }
        }
    }

    // Display portfolio summary
    ui::render_portfolio_summary(&portfolio);

    Ok(())
}

async fn query_one(name: String, rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    let book = AddressBook::load()?;

    // Try to find in crypto addresses first
    if let Some(wallet) = book.addresses.iter().find(|a| a.name == name) {
        query_crypto_address(wallet, rpc_url, no_prices).await?;
        return Ok(());
    }

    // Try to find in banking accounts
    if let Some(account) = book.banking_accounts.iter().find(|a| a.name == name) {
        query_banking_account(account).await?;
        return Ok(());
    }

    anyhow::bail!("Address or account '{}' not found", name)
}

async fn query_crypto_address(wallet: &WalletAddress, rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    if no_prices {
        println!("\nQuerying balance for '{}' (without prices)...\n", wallet.name);
    } else {
        println!("\nQuerying balance for '{}'...\n", wallet.name);
    }

    // Create price service and cache
    let price_service = PriceService::new()?;
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    // Batch fetch all known prices in a single API call (skip if --no-prices)
    if !no_prices {
        println!("Fetching cryptocurrency prices...");
        match price_service.batch_fetch_all_known_prices().await {
            Ok(prices) => {
                price_cache = prices;
                println!("Successfully fetched prices\n");
            }
            Err(e) => {
                eprintln!("Warning: Failed to batch fetch prices: {}", e);
                eprintln!("Will attempt to fetch prices individually as needed.\n");
            }
        }
    }

    match &wallet.chain {
        Chain::Solana => {
            let client = SolanaClient::new(rpc_url);
            query_and_display_solana(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache, no_prices).await?;
        }
        Chain::Near => {
            let client = NearClient::new(rpc_url);
            query_and_display_near(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        Chain::Aptos => {
            let client = AptosClient::new(rpc_url);
            query_and_display_aptos(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        Chain::Sui => {
            let client = SuiClient::new(rpc_url);
            query_and_display_sui(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        Chain::Starknet => {
            let client = StarknetClient::new(rpc_url);
            query_and_display_starknet(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        // All EVM chains
        Chain::Ethereum | Chain::Polygon | Chain::BinanceSmartChain | Chain::Arbitrum
        | Chain::Optimism | Chain::Avalanche | Chain::Base | Chain::Core => {
            let client = EvmClient::new(rpc_url, wallet.chain.clone())?;
            query_and_display_evm(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache, no_prices).await?;
        }
    }

    Ok(())
}

async fn enrich_with_usd_prices(
    balances: &mut solana::AccountBalances,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<()> {
    const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

    // Collect mint addresses that need price fetching (not in cache)
    let mut mints_to_fetch = Vec::new();

    // Check SOL
    if !price_cache.contains_key(SOL_MINT) {
        mints_to_fetch.push(SOL_MINT.to_string());
    }

    // Check tokens
    for token in &balances.token_balances {
        if !price_cache.contains_key(&token.mint) {
            mints_to_fetch.push(token.mint.clone());
        }
    }

    // Fetch prices only for tokens not in cache
    if !mints_to_fetch.is_empty() {
        let prices = price_service.get_prices(&mints_to_fetch).await?;
        // Update cache with newly fetched prices
        for (mint, price) in prices {
            price_cache.insert(mint, price);
        }
    }

    // Update SOL USD values from cache
    if let Some(&sol_price) = price_cache.get(SOL_MINT) {
        balances.sol_usd_price = Some(sol_price);
        balances.sol_usd_value = Some(balances.sol_balance * sol_price);
    }

    // Update token USD values from cache
    let mut total_value = balances.sol_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(&price) = price_cache.get(&token.mint) {
            token.usd_price = Some(price);
            token.usd_value = Some(token.ui_amount * price);
            if let Some(value) = token.usd_value {
                total_value += value;
            }
        }
    }

    balances.total_usd_value = Some(total_value);

    Ok(())
}

async fn enrich_with_eth_prices(
    balances: &mut evm::AccountBalances,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<()> {
    // Check cache for ETH price, fetch if not present
    if !price_cache.contains_key("ETH") {
        let eth_price = price_service.get_eth_price().await?;
        price_cache.insert("ETH".to_string(), eth_price);
    }

    // Use cached ETH price
    if let Some(&eth_price) = price_cache.get("ETH") {
        balances.eth_usd_price = Some(eth_price);
        balances.eth_usd_value = Some(balances.eth_balance * eth_price);
    }

    // Collect token symbols that need price fetching (not in cache)
    let symbols_to_fetch: Vec<String> = balances.token_balances
        .iter()
        .filter_map(|t| t.symbol.clone())
        .filter(|symbol| !price_cache.contains_key(symbol))
        .collect();

    // Fetch prices only for tokens not in cache
    if !symbols_to_fetch.is_empty() {
        match price_service.get_erc20_prices(&symbols_to_fetch).await {
            Ok(prices) => {
                // Update cache with newly fetched prices
                for (symbol, price) in prices {
                    price_cache.insert(symbol, price);
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to fetch ERC20 token prices: {}", e);
            }
        }
    }

    // Update token USD values from cache
    for token in &mut balances.token_balances {
        if let Some(symbol) = &token.symbol {
            if let Some(&price) = price_cache.get(symbol) {
                token.usd_price = Some(price);
                token.usd_value = Some(token.ui_amount * price);
            }
        }
    }

    // Calculate total value
    let mut total_value = balances.eth_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(value) = token.usd_value {
            total_value += value;
        }
    }

    balances.total_usd_value = Some(total_value);

    Ok(())
}

async fn query_and_display_solana(
    client: &SolanaClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<solana::AccountBalances> {
    match client.get_balances(address) {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache (skip if --no-prices)
            if !no_prices {
                if let Err(e) = enrich_with_usd_prices(&mut balances, price_service, price_cache).await {
                    eprintln!("Warning: Failed to fetch USD prices: {}", e);
                }
            }

            // Use the new UI renderer
            ui::render_solana_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Solana address")
        }
    }
}

async fn query_and_display_evm(
    client: &EvmClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<evm::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache (skip if --no-prices)
            if !no_prices {
                if let Err(e) = enrich_with_eth_prices(&mut balances, price_service, price_cache).await {
                    eprintln!("Warning: Failed to fetch USD prices: {}", e);
                }
            }

            // Use the new UI renderer
            ui::render_evm_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query EVM address")
        }
    }
}

fn aggregate_solana_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &solana::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "SOL", balances.sol_balance, balances.sol_usd_value);

    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        add_asset_to_portfolio(portfolio, company, symbol, token.ui_amount, token.usd_value);
    }
}

fn aggregate_evm_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &evm::AccountBalances, chain: &Chain) {
    let native_symbol = chain.native_token_symbol();
    add_asset_to_portfolio(portfolio, company, native_symbol, balances.eth_balance, balances.eth_usd_value);

    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        add_asset_to_portfolio(portfolio, company, symbol, token.ui_amount, token.usd_value);
    }
}

async fn query_and_display_near(
    client: &NearClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &PriceService,
    _price_cache: &mut HashMap<String, f64>
) -> Result<near::AccountBalances> {
    match client.get_balances(address).await {
        Ok(balances) => {
            ui::render_near_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query NEAR address")
        }
    }
}

fn aggregate_near_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &near::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "NEAR", balances.near_balance, balances.near_usd_value);
}

async fn query_and_display_aptos(
    client: &AptosClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &PriceService,
    _price_cache: &mut HashMap<String, f64>
) -> Result<aptos::AccountBalances> {
    match client.get_balances(address).await {
        Ok(balances) => {
            ui::render_aptos_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Aptos address")
        }
    }
}

fn aggregate_aptos_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &aptos::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "APT", balances.apt_balance, balances.apt_usd_value);
}

async fn query_and_display_sui(
    client: &SuiClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &PriceService,
    _price_cache: &mut HashMap<String, f64>
) -> Result<sui::AccountBalances> {
    match client.get_balances(address).await {
        Ok(balances) => {
            ui::render_sui_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Sui address")
        }
    }
}

fn aggregate_sui_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &sui::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "SUI", balances.sui_balance, balances.sui_usd_value);
}

async fn query_and_display_starknet(
    client: &StarknetClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &PriceService,
    _price_cache: &mut HashMap<String, f64>
) -> Result<starknet::AccountBalances> {
    match client.get_balances(address).await {
        Ok(balances) => {
            ui::render_starknet_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Starknet address")
        }
    }
}

fn aggregate_starknet_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &starknet::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "ETH", balances.eth_balance, balances.eth_usd_value);
}

// Cache-only enrich functions (no API calls, only use cached prices)

fn enrich_solana_from_cache(balances: &mut solana::AccountBalances, price_cache: &HashMap<String, f64>) {
    // Enrich SOL balance
    if let Some(&price) = price_cache.get("SOL") {
        balances.sol_usd_price = Some(price);
        balances.sol_usd_value = Some(balances.sol_balance * price);
    }

    // Enrich token balances
    let mut total_usd = balances.sol_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(symbol) = &token.symbol {
            if let Some(&price) = price_cache.get(symbol) {
                token.usd_price = Some(price);
                token.usd_value = Some(token.ui_amount * price);
                total_usd += token.usd_value.unwrap_or(0.0);
            }
        }
    }

    if total_usd > 0.0 {
        balances.total_usd_value = Some(total_usd);
    }
}

fn enrich_evm_from_cache(balances: &mut evm::AccountBalances, price_cache: &HashMap<String, f64>) {
    // Enrich ETH balance
    if let Some(&price) = price_cache.get("ETH") {
        balances.eth_usd_price = Some(price);
        balances.eth_usd_value = Some(balances.eth_balance * price);
    }

    // Enrich token balances
    let mut total_usd = balances.eth_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(symbol) = &token.symbol {
            if let Some(&price) = price_cache.get(symbol) {
                token.usd_price = Some(price);
                token.usd_value = Some(token.ui_amount * price);
                total_usd += token.usd_value.unwrap_or(0.0);
            }
        }
    }

    if total_usd > 0.0 {
        balances.total_usd_value = Some(total_usd);
    }
}

fn enrich_near_from_cache(balances: &mut near::AccountBalances, price_cache: &HashMap<String, f64>) {
    if let Some(&price) = price_cache.get("NEAR") {
        balances.near_usd_price = Some(price);
        balances.near_usd_value = Some(balances.near_balance * price);
        balances.total_usd_value = Some(balances.near_balance * price);
    }
}

fn enrich_aptos_from_cache(balances: &mut aptos::AccountBalances, price_cache: &HashMap<String, f64>) {
    if let Some(&price) = price_cache.get("APT") {
        balances.apt_usd_price = Some(price);
        balances.apt_usd_value = Some(balances.apt_balance * price);
        balances.total_usd_value = Some(balances.apt_balance * price);
    }
}

fn enrich_sui_from_cache(balances: &mut sui::AccountBalances, price_cache: &HashMap<String, f64>) {
    if let Some(&price) = price_cache.get("SUI") {
        balances.sui_usd_price = Some(price);
        balances.sui_usd_value = Some(balances.sui_balance * price);
        balances.total_usd_value = Some(balances.sui_balance * price);
    }
}

fn enrich_starknet_from_cache(balances: &mut starknet::AccountBalances, price_cache: &HashMap<String, f64>) {
    // Starknet uses ETH as native token
    if let Some(&price) = price_cache.get("ETH") {
        balances.eth_usd_price = Some(price);
        balances.eth_usd_value = Some(balances.eth_balance * price);
        balances.total_usd_value = Some(balances.eth_balance * price);
    }
}

fn aggregate_mercury_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &mercury::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "USD", balances.current_balance, Some(balances.current_balance));
}

fn aggregate_circle_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &circle::AccountBalances) {
    for balance in &balances.available_balances {
        let symbol = match balance.currency.as_str() {
            "USD" => "USDC",
            "EUR" => "EURC",
            _ => &balance.currency,
        };
        // Only USD has a known USD value; EUR would need conversion
        let usd_value = if balance.currency == "USD" { Some(balance.amount) } else { None };
        add_asset_to_portfolio(portfolio, company, symbol, balance.amount, usd_value);
    }
}

async fn query_banking_account(account: &BankingAccount) -> Result<()> {
    println!("\nQuerying balance for '{}'...\n", account.name);

    match &account.service {
        BankingService::Mercury => {
            let client = MercuryClient::new()?;
            match client.get_account_balance(&account.account_id).await {
                Ok(balances) => {
                    ui::render_mercury_balances(&account.company, &account.name, &account.account_id, &balances, &account.service);
                    Ok(())
                }
                Err(e) => {
                    ui::render_error(&format!("Error querying '{}' ({}): {}", account.name, account.account_id, e));
                    anyhow::bail!("Failed to query Mercury account")
                }
            }
        }
        BankingService::Circle => {
            let client = CircleClient::new()?;
            match client.get_balances().await {
                Ok(balances) => {
                    ui::render_circle_balances(&account.company, &account.name, &balances, &account.service);
                    Ok(())
                }
                Err(e) => {
                    ui::render_error(&format!("Error querying '{}' Circle balances: {}", account.name, e));
                    anyhow::bail!("Failed to query Circle account")
                }
            }
        }
    }
}

async fn export_transactions(
    name: String,
    format: String,
    start: Option<String>,
    end: Option<String>,
    output: Option<String>,
) -> Result<()> {
    let book = AddressBook::load()?;

    // Find the Mercury account
    let account = book
        .banking_accounts
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("Banking account '{}' not found", name))?;

    if !matches!(account.service, BankingService::Mercury) {
        anyhow::bail!("Transaction export is only supported for Mercury accounts");
    }

    // Validate date format (YYYY-MM-DD)
    let date_regex = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if let Some(ref s) = start {
        if !date_regex.is_match(s) {
            anyhow::bail!("Invalid start date format '{}'. Use YYYY-MM-DD.", s);
        }
    }
    if let Some(ref e) = end {
        if !date_regex.is_match(e) {
            anyhow::bail!("Invalid end date format '{}'. Use YYYY-MM-DD.", e);
        }
    }

    let client = MercuryClient::new()?;

    eprintln!("Fetching transactions for '{}'...", name);

    let transactions = client
        .get_transactions(
            &account.account_id,
            start.as_deref(),
            end.as_deref(),
        )
        .await?;

    eprintln!("Found {} transactions", transactions.len());

    let output_data = match format.to_lowercase().as_str() {
        "json" => {
            serde_json::to_string_pretty(&transactions)?
        }
        "csv" | _ => {
            let mut csv_output = String::new();
            csv_output.push_str("date,amount,status,counterparty,description,note,kind\n");

            for tx in &transactions {
                let raw_date = tx.posted_at.as_deref().unwrap_or(&tx.created_at);
                // Convert ISO date to DD-MM-YYYY
                let date = if raw_date.len() >= 10 {
                    let parts: Vec<&str> = raw_date[..10].split('-').collect();
                    if parts.len() == 3 {
                        format!("{}-{}-{}", parts[2], parts[1], parts[0])
                    } else {
                        raw_date.to_string()
                    }
                } else {
                    raw_date.to_string()
                };
                let counterparty = tx.counterparty_name.as_deref().unwrap_or("");
                let description = tx.bank_description.as_deref().unwrap_or("");
                let note = tx.note.as_deref().unwrap_or("");

                // Escape CSV fields
                let escape_csv = |s: &str| {
                    if s.contains(',') || s.contains('"') || s.contains('\n') {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s.to_string()
                    }
                };

                csv_output.push_str(&format!(
                    "{},{},{},{},{},{},{}\n",
                    date,
                    tx.amount,
                    tx.status,
                    escape_csv(counterparty),
                    escape_csv(description),
                    escape_csv(note),
                    tx.kind
                ));
            }
            csv_output
        }
    };

    match output {
        Some(path) => {
            let mut file = std::fs::File::create(&path)?;
            file.write_all(output_data.as_bytes())?;
            eprintln!("Exported to {}", path);
        }
        None => {
            print!("{}", output_data);
        }
    }

    Ok(())
}

async fn list_mercury_accounts() -> Result<()> {
    let client = MercuryClient::new()?;
    let accounts = client.list_accounts().await?;

    println!("\nMercury Accounts\n");
    println!("{:<40} {:<20} {:<10} {:<15} {:<15}", "ID", "Name", "Status", "Kind", "Balance");
    println!("{}", "-".repeat(100));

    for account in &accounts {
        println!(
            "{:<40} {:<20} {:<10} {:<15} ${:<14.2}",
            account.id,
            if account.name.len() > 18 {
                format!("{}...", &account.name[..15])
            } else {
                account.name.clone()
            },
            account.status,
            account.kind,
            account.current_balance
        );
    }

    println!("\nTotal: {} account(s)", accounts.len());
    Ok(())
}

async fn setup_mercury(company: String) -> Result<()> {
    let client = MercuryClient::new()?;
    let accounts = client.list_accounts().await?;

    if accounts.is_empty() {
        println!("No Mercury accounts found.");
        return Ok(());
    }

    println!("\nFound {} Mercury account(s):\n", accounts.len());

    for (i, account) in accounts.iter().enumerate() {
        println!(
            "  [{}] {} ({}) - ${:.2} - {}",
            i + 1,
            account.name,
            account.kind,
            account.current_balance,
            account.status
        );
    }

    println!("\nOptions:");
    println!("  [A] Add all accounts");
    println!("  [S] Select specific accounts (e.g., 1,3,5)");
    println!("  [N] Add none");
    print!("\nChoice: ");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let mut input = String::new();
    stdin.lock().read_line(&mut input)?;
    let choice = input.trim().to_uppercase();

    let selected_indices: Vec<usize> = match choice.as_str() {
        "A" => (0..accounts.len()).collect(),
        "N" => {
            println!("No accounts added.");
            return Ok(());
        }
        "S" => {
            print!("Enter account numbers (comma-separated, e.g., 1,3): ");
            io::stdout().flush()?;
            input.clear();
            stdin.lock().read_line(&mut input)?;

            input
                .trim()
                .split(',')
                .filter_map(|s| s.trim().parse::<usize>().ok())
                .filter(|&n| n >= 1 && n <= accounts.len())
                .map(|n| n - 1)
                .collect()
        }
        _ => {
            // Try parsing as comma-separated numbers directly
            choice
                .split(',')
                .filter_map(|s| s.trim().parse::<usize>().ok())
                .filter(|&n| n >= 1 && n <= accounts.len())
                .map(|n| n - 1)
                .collect()
        }
    };

    if selected_indices.is_empty() {
        println!("No valid accounts selected.");
        return Ok(());
    }

    let mut book = AddressBook::load()?;
    let mut added = 0;

    for idx in selected_indices {
        let account = &accounts[idx];

        // Create a friendly name from the Mercury account name
        let name = account.name.clone();

        // Check if already tracked
        if book.banking_accounts.iter().any(|a| a.account_id == account.id) {
            println!("  Skipping '{}' - already tracked", name);
            continue;
        }

        book.add_banking_account(
            company.clone(),
            name.clone(),
            account.id.clone(),
            "mercury".to_string(),
        )?;

        println!("  Added '{}'", name);
        added += 1;
    }

    book.save()?;

    if added > 0 {
        println!("\nSuccessfully added {} account(s) to tracking.", added);
    } else {
        println!("\nNo new accounts added.");
    }

    Ok(())
}
