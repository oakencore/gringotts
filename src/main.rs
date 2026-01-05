mod cli;
mod storage;
mod solana;
mod evm;
mod price;
mod ui;
mod aptos;
mod near;
mod sui;
mod starknet;
mod mercury;
mod circle;
mod web;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use storage::{AddressBook, BankingAccount, BankingService, Chain, WalletAddress};
use solana::SolanaClient;
use evm::EvmClient;
use aptos::AptosClient;
use near::NearClient;
use sui::SuiClient;
use starknet::StarknetClient;
use mercury::MercuryClient;
use circle::CircleClient;
use price::PriceService;
use std::collections::{HashMap, HashSet};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;

// Portfolio summary structure
struct PortfolioSummary {
    companies: HashMap<String, CompanyAssets>,
    total_usd_value: f64,
}

struct CompanyAssets {
    assets: HashMap<String, AssetSummary>,
    total_usd_value: f64,
}

struct AssetSummary {
    symbol: String,
    amount: f64,
    usd_value: Option<f64>,
}

fn add_asset_to_portfolio(
    portfolio: &mut PortfolioSummary,
    company: &str,
    symbol: &str,
    amount: f64,
    usd_value: Option<f64>,
) {
    if amount == 0.0 {
        return;
    }

    let company_assets = portfolio
        .companies
        .entry(company.to_string())
        .or_insert_with(|| CompanyAssets {
            assets: HashMap::new(),
            total_usd_value: 0.0,
        });

    let asset = company_assets
        .assets
        .entry(symbol.to_string())
        .or_insert_with(|| AssetSummary {
            symbol: symbol.to_string(),
            amount: 0.0,
            usd_value: Some(0.0),
        });

    asset.amount += amount;
    if let Some(value) = usd_value {
        if let Some(ref mut asset_value) = asset.usd_value {
            *asset_value += value;
        }
        company_assets.total_usd_value += value;
        portfolio.total_usd_value += value;
    }
}

// Trait for price enrichment - eliminates duplicate code across chains
trait PriceEnrichable {
    const NATIVE_SYMBOL: &'static str;

    fn native_balance(&self) -> f64;
    fn set_native_usd_price(&mut self, price: f64);
    fn set_native_usd_value(&mut self, value: f64);
    fn set_total_usd_value(&mut self, value: f64);

    // Default implementation returns 0.0 for chains without tokens
    fn enrich_token_balances(&mut self, _price_cache: &HashMap<String, f64>) -> f64 {
        0.0
    }

    // Default enrichment implementation
    fn enrich_from_cache(&mut self, price_cache: &HashMap<String, f64>) {
        let mut total_usd = 0.0;

        // Enrich native token balance
        if let Some(&price) = price_cache.get(Self::NATIVE_SYMBOL) {
            self.set_native_usd_price(price);
            let native_value = self.native_balance() * price;
            self.set_native_usd_value(native_value);
            total_usd += native_value;
        }

        // Enrich token balances (if any)
        total_usd += self.enrich_token_balances(price_cache);

        if total_usd > 0.0 {
            self.set_total_usd_value(total_usd);
        }
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
    // Load environment variables from .env file if present
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    match cli.command {
        Commands::Add { company, name, address, chain } => {
            let detected_chain = if let Some(chain_str) = chain {
                Chain::from_str(&chain_str)?
            } else {
                // Auto-detect chain based on address format
                if address.len() == 42 && address.starts_with("0x") {
                    Chain::Ethereum
                } else {
                    Chain::Solana
                }
            };
            add_address(company, name, address, detected_chain)?;
        }
        Commands::List { .. } => {
            list_addresses()?;
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
        Commands::AddBank { company, name, account_id, service } => {
            let banking_service = BankingService::from_str(&service)?;
            add_banking_account(company, name, account_id, banking_service)?;
        }
        Commands::SetupMercury { company } => {
            setup_mercury_accounts(company).await?;
        }
        Commands::ListMercuryAccounts => {
            println!("Listing Mercury accounts...");
        }
        Commands::ExportTransactions { name, format, start, end, output } => {
            export_transactions(name, format, start, end, output).await?;
        }
        Commands::Serve { port } => {
            web::start_server(port).await?;
        }
    }

    Ok(())
}

fn add_address(company: String, name: String, address: String, chain: Chain) -> Result<()> {
    let mut book = AddressBook::load()?;

    let wallet = WalletAddress {
        company,
        name,
        address,
        chain,
    };

    book.addresses.push(wallet);
    book.save()?;

    ui::render_success("Address added successfully");
    Ok(())
}

fn list_addresses() -> Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        println!("No addresses or accounts tracked yet.");
        println!("Use 'gringotts add' to add blockchain addresses.");
        println!("Use 'gringotts add-bank' to add banking accounts.");
        return Ok(());
    }

    if !book.addresses.is_empty() {
        println!("\n=== Tracked Blockchain Addresses ===\n");
        for (i, wallet) in book.addresses.iter().enumerate() {
            println!("{}. {} - {} ({})", i + 1, wallet.name, wallet.address, wallet.chain.display_name());
            if !wallet.company.is_empty() {
                println!("   Company: {}", wallet.company);
            }
            println!();
        }
    }

    if !book.banking_accounts.is_empty() {
        println!("\n=== Tracked Banking Accounts ===\n");
        for (i, account) in book.banking_accounts.iter().enumerate() {
            println!("{}. {} - {} ({})", i + 1, account.name, account.account_id, account.service.display_name());
            if !account.company.is_empty() {
                println!("   Company: {}", account.company);
            }
            println!();
        }
    }

    Ok(())
}

fn remove_address(identifier: String) -> Result<()> {
    let mut book = AddressBook::load()?;

    // Try to remove by name first
    let initial_len = book.addresses.len();
    book.addresses.retain(|w| w.name != identifier && w.address != identifier);

    if book.addresses.len() < initial_len {
        book.save()?;
        ui::render_success(&format!("Removed '{}'", identifier));
        return Ok(());
    }

    // Try to remove from banking accounts
    let initial_bank_len = book.banking_accounts.len();
    book.banking_accounts.retain(|a| a.name != identifier && a.account_id != identifier);

    if book.banking_accounts.len() < initial_bank_len {
        book.save()?;
        ui::render_success(&format!("Removed '{}'", identifier));
        return Ok(());
    }

    ui::render_error(&format!("No address or account found with identifier '{}'", identifier));
    Ok(())
}

// Helper function to fetch all balances from wallets and banking accounts
async fn fetch_all_balances(
    book: &AddressBook,
    rpc_url: Option<String>,
) -> Vec<WalletBalances> {
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

    // Query blockchain wallets
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

    all_balances
}

// Helper function to extract unique token symbols from all balances
fn extract_token_symbols(all_balances: &[WalletBalances]) -> HashSet<String> {
    let mut symbols: HashSet<String> = HashSet::new();

    for wallet_balance in all_balances {
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
            WalletBalances::Mercury(_, _) | WalletBalances::Circle(_, _) => {
                // Banking balances are already in USD/EUR, no price lookup needed
            }
        }
    }

    symbols
}

// Helper function to fetch USD prices for token symbols
async fn fetch_prices_for_symbols(symbols: HashSet<String>) -> Result<HashMap<String, f64>> {
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    if symbols.is_empty() {
        return Ok(price_cache);
    }

    let price_service = PriceService::new()?;
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

    Ok(price_cache)
}

// Helper function to enrich balances with prices and display them
fn enrich_and_display_balances(
    all_balances: Vec<WalletBalances>,
    price_cache: &HashMap<String, f64>,
) -> PortfolioSummary {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    for wallet_balance in all_balances {
        match wallet_balance {
            WalletBalances::Solana(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_solana_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_solana_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Evm(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_evm_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_evm_balances(&mut portfolio, &wallet.company, &balances, &wallet.chain);
            }
            WalletBalances::Near(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_near_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_near_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Aptos(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_aptos_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_aptos_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Sui(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_sui_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
                aggregate_sui_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Starknet(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
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

    portfolio
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

    // Fetch all balances
    let all_balances = fetch_all_balances(&book, rpc_url).await;

    // Extract symbols and fetch prices (skip if --no-prices)
    let price_cache = if !no_prices {
        let symbols = extract_token_symbols(&all_balances);
        fetch_prices_for_symbols(symbols).await?
    } else {
        HashMap::new()
    };

    // Enrich balances with prices and display
    let portfolio = enrich_and_display_balances(all_balances, &price_cache);

    // Display portfolio summary
    ui::render_portfolio_summary(&portfolio);

    Ok(())
}

async fn query_one(identifier: String, rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    let book = AddressBook::load()?;

    // Try to find wallet by name or address
    let wallet = book.addresses.iter().find(|w| w.name == identifier || w.address == identifier);

    if let Some(wallet) = wallet {
        println!("\nQuerying balance for '{}'...\n", wallet.name);

        let price_service = PriceService::new()?;
        let mut price_cache: HashMap<String, f64> = HashMap::new();

        // Pre-fetch prices for common symbols if not in no_prices mode
        if !no_prices {
            println!("Fetching cryptocurrency prices...");
            let symbols = vec!["SOL".to_string(), "ETH".to_string(), "USDC".to_string(), "USDT".to_string()];
            match price_service.batch_fetch_prices(&symbols).await {
                Ok(prices) => {
                    price_cache = prices;
                    println!("Successfully fetched prices\n");
                }
                Err(e) => {
                    eprintln!("Warning: Failed to fetch prices: {}", e);
                    eprintln!("Will attempt to fetch prices individually as needed.\n");
                }
            }
        }

    match &wallet.chain {
        Chain::Solana => {
            let client = SolanaClient::new(rpc_url);
            query_and_display_solana(&client, wallet, &price_service, &mut price_cache, no_prices).await?;
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
            query_and_display_evm(&client, wallet, &price_service, &mut price_cache, no_prices).await?;
        }
    }

        return Ok(());
    }

    // Try to find banking account
    let account = book.banking_accounts.iter().find(|a| a.name == identifier || a.account_id == identifier);

    if let Some(account) = account {
        println!("\nQuerying balance for '{}'...\n", account.name);

        match &account.service {
            BankingService::Mercury => {
                let client = MercuryClient::new()?;
                let balances = client.get_account_balance(&account.account_id).await?;
                ui::render_mercury_balances(&account.company, &account.name, &account.account_id, &balances, &account.service);
            }
            BankingService::Circle => {
                let client = CircleClient::new()?;
                let balances = client.get_balances().await?;
                ui::render_circle_balances(&account.company, &account.name, &balances, &account.service);
            }
        }

        return Ok(());
    }

    ui::render_error(&format!("No address or account found with identifier '{}'", identifier));
    Ok(())
}

async fn enrich_with_usd_prices(
    balances: &mut solana::AccountBalances,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<()> {
    // Enrich SOL price
    let sol_price = if let Some(&cached_price) = price_cache.get("SOL") {
        cached_price
    } else {
        let price = price_service.get_single_price("SOL").await?;
        price_cache.insert("SOL".to_string(), price);
        price
    };

    balances.sol_usd_price = Some(sol_price);
    balances.sol_usd_value = Some(balances.sol_balance * sol_price);

    // Enrich token balances
    let mut total_usd = balances.sol_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(symbol) = &token.symbol {
            let price = if let Some(&cached_price) = price_cache.get(symbol) {
                cached_price
            } else {
                match price_service.get_single_price(symbol).await {
                    Ok(p) => {
                        price_cache.insert(symbol.clone(), p);
                        p
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to fetch price for {} ({}): {}", symbol, token.mint, e);
                        continue;
                    }
                }
            };

            token.usd_price = Some(price);
            token.usd_value = Some(token.ui_amount * price);
            total_usd += token.usd_value.unwrap_or(0.0);
        }
    }

    if total_usd > 0.0 {
        balances.total_usd_value = Some(total_usd);
    }

    Ok(())
}

async fn enrich_with_eth_prices(
    balances: &mut evm::AccountBalances,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<()> {
    // Enrich ETH price
    let eth_price = if let Some(&cached_price) = price_cache.get("ETH") {
        cached_price
    } else {
        let price = price_service.get_single_price("ETH").await?;
        price_cache.insert("ETH".to_string(), price);
        price
    };

    balances.eth_usd_price = Some(eth_price);
    balances.eth_usd_value = Some(balances.eth_balance * eth_price);

    // Enrich token balances
    let mut total_usd = balances.eth_usd_value.unwrap_or(0.0);
    for token in &mut balances.token_balances {
        if let Some(symbol) = &token.symbol {
            let price = if let Some(&cached_price) = price_cache.get(symbol) {
                cached_price
            } else {
                match price_service.get_single_price(symbol).await {
                    Ok(p) => {
                        price_cache.insert(symbol.clone(), p);
                        p
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to fetch price for {}: {}", symbol, e);
                        continue;
                    }
                }
            };

            token.usd_price = Some(price);
            token.usd_value = Some(token.ui_amount * price);
            total_usd += token.usd_value.unwrap_or(0.0);
        }
    }

    if total_usd > 0.0 {
        balances.total_usd_value = Some(total_usd);
    }

    Ok(())
}

async fn query_and_display_solana(
    client: &SolanaClient,
    wallet: &WalletAddress,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<solana::AccountBalances> {
    match client.get_balances(&wallet.address) {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache (skip if --no-prices)
            if !no_prices {
                if let Err(e) = enrich_with_usd_prices(&mut balances, price_service, price_cache).await {
                    eprintln!("Warning: Failed to fetch USD prices: {}", e);
                }
            }

            // Use the new UI renderer
            ui::render_solana_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", wallet.name, wallet.address, e));
            anyhow::bail!("Failed to query Solana address")
        }
    }
}

async fn query_and_display_evm(
    client: &EvmClient,
    wallet: &WalletAddress,
    price_service: &PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<evm::AccountBalances> {
    match client.get_balances(&wallet.address).await {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache (skip if --no-prices)
            if !no_prices {
                if let Err(e) = enrich_with_eth_prices(&mut balances, price_service, price_cache).await {
                    eprintln!("Warning: Failed to fetch USD prices: {}", e);
                }
            }

            // Use the new UI renderer
            ui::render_evm_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", wallet.name, wallet.address, e));
            anyhow::bail!("Failed to query EVM address")
        }
    }
}

fn aggregate_solana_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &solana::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "SOL", balances.sol_balance, balances.sol_usd_value);

    for token in &balances.token_balances {
        if let Some(symbol) = &token.symbol {
            add_asset_to_portfolio(portfolio, company, symbol, token.ui_amount, token.usd_value);
        }
    }
}

fn aggregate_evm_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &evm::AccountBalances, chain: &Chain) {
    let native_symbol = match chain {
        Chain::BinanceSmartChain => "BNB",
        Chain::Polygon => "MATIC",
        Chain::Avalanche => "AVAX",
        _ => "ETH",
    };
    add_asset_to_portfolio(portfolio, company, native_symbol, balances.eth_balance, balances.eth_usd_value);

    for token in &balances.token_balances {
        if let Some(symbol) = &token.symbol {
            add_asset_to_portfolio(portfolio, company, symbol, token.ui_amount, token.usd_value);
        }
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


fn aggregate_mercury_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &mercury::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, "USD", balances.current_balance, Some(balances.current_balance));
}

fn aggregate_circle_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &circle::AccountBalances) {
    // Aggregate available balances
    for balance in &balances.available_balances {
        add_asset_to_portfolio(portfolio, company, &balance.currency, balance.amount, Some(balance.amount));
    }
}

fn add_banking_account(company: String, name: String, account_id: String, service: BankingService) -> Result<()> {
    let mut book = AddressBook::load()?;

    let account = BankingAccount {
        company,
        name,
        account_id,
        service,
    };

    book.banking_accounts.push(account);
    book.save()?;

    ui::render_success("Banking account added successfully");
    Ok(())
}

async fn setup_mercury_accounts(company: String) -> Result<()> {
    let client = MercuryClient::new()?;
    let accounts = client.list_accounts().await?;

    if accounts.is_empty() {
        println!("No Mercury accounts found.");
        return Ok(());
    }

    println!("\nFound {} Mercury account(s):", accounts.len());
    for account in &accounts {
        println!("  - {} ({})", account.name, account.id);
    }

    println!("\nAdding accounts to tracking...");
    let mut book = AddressBook::load()?;

    for account in accounts {
        // Check if account already exists
        if book.banking_accounts.iter().any(|a| a.account_id == account.id) {
            println!("  Skipping {} (already tracked)", account.name);
            continue;
        }

        let banking_account = BankingAccount {
            company: company.clone(),
            name: account.name.clone(),
            account_id: account.id.clone(),
            service: BankingService::Mercury,
        };

        book.banking_accounts.push(banking_account);
        println!("  Added {}", account.name);
    }

    book.save()?;
    ui::render_success("Mercury setup complete");
    Ok(())
}

async fn export_transactions(
    account_name: String,
    format: String,
    start: Option<String>,
    end: Option<String>,
    output: Option<String>,
) -> Result<()> {
    let book = AddressBook::load()?;

    let account = book.banking_accounts.iter()
        .find(|a| a.name == account_name || a.account_id == account_name)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", account_name))?;

    match &account.service {
        BankingService::Mercury => {
            let client = MercuryClient::new()?;
            let transactions = client.get_transactions(&account.account_id, start.as_deref(), end.as_deref()).await?;

            let output_data = export_mercury_transactions(&transactions, &format)?;

            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(&path)?;
                    file.write_all(output_data.as_bytes())?;
                    println!("Exported {} transactions to {}", transactions.len(), path);
                }
                None => {
                    println!("{}", output_data);
                }
            }
        }
        BankingService::Circle => {
            return Err(anyhow::anyhow!("Transaction export not yet supported for Circle accounts"));
        }
    }

    Ok(())
}

fn export_mercury_transactions(transactions: &[mercury::Transaction], format: &str) -> Result<String> {
    fn escape_csv(s: &str) -> String {
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }

    let output_data = match format.to_lowercase().as_str() {
        "json" => {
            serde_json::to_string_pretty(&transactions)?
        }
        _ => {
            let mut csv_output = String::new();
            csv_output.push_str("date,amount,status,counterparty,description,note,kind\n");

            for tx in transactions {
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

    Ok(output_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test PriceEnrichable trait default implementation
    #[test]
    fn test_extract_token_symbols_empty() {
        let balances: Vec<WalletBalances> = vec![];
        let symbols = extract_token_symbols(&balances);
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_extract_token_symbols_solana() {
        let wallet = WalletAddress {
            company: "Test".to_string(),
            name: "Test Wallet".to_string(),
            address: "test123".to_string(),
            chain: Chain::Solana,
        };

        let mut balances = solana::AccountBalances {
            sol_balance: 1.0,
            sol_usd_price: None,
            sol_usd_value: None,
            token_balances: vec![],
            total_usd_value: None,
        };

        // Add a token
        balances.token_balances.push(solana::TokenBalance {
            mint: "test_mint".to_string(),
            symbol: Some("USDC".to_string()),
            name: Some("USD Coin".to_string()),
            decimals: 6,
            ui_amount: 100.0,
            usd_price: None,
            usd_value: None,
        });

        let wallet_balances = vec![WalletBalances::Solana(wallet, balances)];
        let symbols = extract_token_symbols(&wallet_balances);

        assert!(symbols.contains("SOL"));
        assert!(symbols.contains("USDC"));
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn test_extract_token_symbols_multiple_chains() {
        let sol_wallet = WalletAddress {
            company: "Test".to_string(),
            name: "SOL Wallet".to_string(),
            address: "sol123".to_string(),
            chain: Chain::Solana,
        };

        let eth_wallet = WalletAddress {
            company: "Test".to_string(),
            name: "ETH Wallet".to_string(),
            address: "0x123".to_string(),
            chain: Chain::Ethereum,
        };

        let sol_balances = solana::AccountBalances {
            sol_balance: 1.0,
            sol_usd_price: None,
            sol_usd_value: None,
            token_balances: vec![],
            total_usd_value: None,
        };

        let eth_balances = evm::AccountBalances {
            eth_balance: 1.0,
            eth_usd_price: None,
            eth_usd_value: None,
            token_balances: vec![],
            total_usd_value: None,
        };

        let wallet_balances = vec![
            WalletBalances::Solana(sol_wallet, sol_balances),
            WalletBalances::Evm(eth_wallet, eth_balances),
        ];

        let symbols = extract_token_symbols(&wallet_balances);
        assert!(symbols.contains("SOL"));
        assert!(symbols.contains("ETH"));
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn test_add_asset_to_portfolio() {
        let mut portfolio = PortfolioSummary {
            companies: HashMap::new(),
            total_usd_value: 0.0,
        };

        add_asset_to_portfolio(&mut portfolio, "TestCo", "BTC", 1.0, Some(50000.0));

        assert_eq!(portfolio.companies.len(), 1);
        assert!(portfolio.companies.contains_key("TestCo"));
        assert_eq!(portfolio.total_usd_value, 50000.0);

        let company = portfolio.companies.get("TestCo").unwrap();
        assert_eq!(company.total_usd_value, 50000.0);
        assert!(company.assets.contains_key("BTC"));

        let btc = company.assets.get("BTC").unwrap();
        assert_eq!(btc.amount, 1.0);
        assert_eq!(btc.usd_value, Some(50000.0));
    }

    #[test]
    fn test_add_asset_to_portfolio_accumulation() {
        let mut portfolio = PortfolioSummary {
            companies: HashMap::new(),
            total_usd_value: 0.0,
        };

        // Add same asset twice
        add_asset_to_portfolio(&mut portfolio, "TestCo", "BTC", 1.0, Some(50000.0));
        add_asset_to_portfolio(&mut portfolio, "TestCo", "BTC", 0.5, Some(25000.0));

        let company = portfolio.companies.get("TestCo").unwrap();
        let btc = company.assets.get("BTC").unwrap();

        assert_eq!(btc.amount, 1.5);
        assert_eq!(btc.usd_value, Some(75000.0));
        assert_eq!(portfolio.total_usd_value, 75000.0);
    }

    #[test]
    fn test_add_asset_zero_balance_ignored() {
        let mut portfolio = PortfolioSummary {
            companies: HashMap::new(),
            total_usd_value: 0.0,
        };

        add_asset_to_portfolio(&mut portfolio, "TestCo", "BTC", 0.0, Some(0.0));

        assert_eq!(portfolio.companies.len(), 0);
    }

    #[test]
    fn test_price_enrichable_trait() {
        let mut balances = near::AccountBalances {
            near_balance: 10.0,
            near_usd_price: None,
            near_usd_value: None,
            token_balances: vec![],
            total_usd_value: None,
        };

        let mut price_cache = HashMap::new();
        price_cache.insert("NEAR".to_string(), 5.0);

        balances.enrich_from_cache(&price_cache);

        assert_eq!(balances.near_usd_price, Some(5.0));
        assert_eq!(balances.near_usd_value, Some(50.0));
        assert_eq!(balances.total_usd_value, Some(50.0));
    }

    #[test]
    fn test_price_enrichable_no_price_available() {
        let mut balances = near::AccountBalances {
            near_balance: 10.0,
            near_usd_price: None,
            near_usd_value: None,
            token_balances: vec![],
            total_usd_value: None,
        };

        let price_cache = HashMap::new(); // Empty cache

        balances.enrich_from_cache(&price_cache);

        assert_eq!(balances.near_usd_price, None);
        assert_eq!(balances.near_usd_value, None);
        assert_eq!(balances.total_usd_value, None);
    }

    #[test]
    fn test_storage_round_trip() {
        use std::fs;
        use std::path::PathBuf;

        let temp_file = PathBuf::from("/tmp/test_gringotts_addresses.json");

        // Clean up if exists
        let _ = fs::remove_file(&temp_file);

        // Create test address book
        let book = AddressBook {
            addresses: vec![
                WalletAddress {
                    company: "TestCo".to_string(),
                    name: "Test Wallet".to_string(),
                    address: "test123".to_string(),
                    chain: Chain::Solana,
                }
            ],
            banking_accounts: vec![],
        };

        // Save
        book.save_to_path(&temp_file).expect("Failed to save");

        // Load
        let loaded = AddressBook::load_from_path(&temp_file).expect("Failed to load");

        assert_eq!(loaded.addresses.len(), 1);
        assert_eq!(loaded.addresses[0].name, "Test Wallet");
        assert_eq!(loaded.addresses[0].chain, Chain::Solana);

        // Clean up
        let _ = fs::remove_file(&temp_file);
    }
}
