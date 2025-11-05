mod aptos;
mod cli;
mod evm;
mod near;
mod price;
mod solana;
mod starknet;
mod storage;
mod sui;
mod ui;

use anyhow::Result;
use aptos::AptosClient;
use clap::Parser;
use cli::{Cli, Commands};
use evm::EvmClient;
use near::NearClient;
use price::PriceService;
use solana::SolanaClient;
use starknet::StarknetClient;
use storage::{AddressBook, Chain, WalletAddress};
use sui::SuiClient;
use std::collections::{HashMap, HashSet};
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

// Struct to hold wallet + balances during query phase
enum WalletBalances {
    Solana(WalletAddress, solana::AccountBalances),
    Evm(WalletAddress, evm::AccountBalances),
    Near(WalletAddress, near::AccountBalances),
    Aptos(WalletAddress, aptos::AccountBalances),
    Sui(WalletAddress, sui::AccountBalances),
    Starknet(WalletAddress, starknet::AccountBalances),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { company, name, address, chain } => {
            add_address(company, name, address, chain)?;
        }
        Commands::List => {
            list_addresses()?;
        }
        Commands::Remove { identifier } => {
            remove_address(identifier)?;
        }
        Commands::Query { rpc_url } => {
            query_all(rpc_url).await?;
        }
        Commands::QueryOne { name, rpc_url } => {
            query_one(name, rpc_url).await?;
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

fn list_addresses() -> Result<()> {
    let book = AddressBook::load()?;
    ui::render_addresses(&book.addresses);
    Ok(())
}

fn remove_address(identifier: String) -> Result<()> {
    let mut book = AddressBook::load()?;
    book.remove_by_identifier(&identifier)?;
    book.save()?;

    ui::render_success(&format!("Removed address '{}'", identifier));
    Ok(())
}

async fn query_all(rpc_url: Option<String>) -> Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() {
        println!("No addresses tracked yet. Use 'gringotts add' to add addresses.");
        return Ok(());
    }

    println!("\nQuerying balances for all tracked addresses...\n");

    // Phase 1: Query all balances (without prices)
    let total_wallets = book.addresses.len();
    let pb = ProgressBar::new(total_wallets as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} wallets ({eta})")
            .unwrap()
            .progress_chars("#>-")
    );
    pb.set_message("Fetching wallet balances...");

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
                let client = EvmClient::new(rpc_url.clone(), wallet.chain.clone());
                match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Evm(wallet.clone(), balances));
                    }
                    Err(e) => {
                        pb.println(format!("⚠ Warning: Failed to query {} ({}): {}", wallet.name, wallet.address, e));
                    }
                }
            }
        }
        pb.inc(1);
    }

    pb.finish_with_message(format!("✓ Successfully fetched balances from {} wallets", all_balances.len()));
    println!();

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
        }
    }

    // Phase 3: Batch fetch prices for all symbols
    let price_service = PriceService::new();
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    if !symbols.is_empty() {
        let price_pb = ProgressBar::new_spinner();
        price_pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap()
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
        }
    }

    // Display portfolio summary
    ui::render_portfolio_summary(&portfolio);

    Ok(())
}

async fn query_one(name: String, rpc_url: Option<String>) -> Result<()> {
    let book = AddressBook::load()?;

    let wallet = book
        .addresses
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("Address '{}' not found", name))?;

    println!("\nQuerying balance for '{}'...\n", name);

    // Create price service and cache
    let price_service = PriceService::new();
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    // Batch fetch all known prices in a single API call
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

    match &wallet.chain {
        Chain::Solana => {
            let client = SolanaClient::new(rpc_url);
            query_and_display_solana(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
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
            let client = EvmClient::new(rpc_url, wallet.chain.clone());
            query_and_display_evm(&client, &wallet.company, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
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
    price_cache: &mut HashMap<String, f64>
) -> Result<solana::AccountBalances> {
    match client.get_balances(address) {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache
            if let Err(e) = enrich_with_usd_prices(&mut balances, price_service, price_cache).await {
                eprintln!("Warning: Failed to fetch USD prices: {}", e);
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
    price_cache: &mut HashMap<String, f64>
) -> Result<evm::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            // Try to enrich with USD prices using cache
            if let Err(e) = enrich_with_eth_prices(&mut balances, price_service, price_cache).await {
                eprintln!("Warning: Failed to fetch USD prices: {}", e);
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
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    // Add SOL balance
    let sol_entry = company_summary.assets.entry("SOL".to_string()).or_insert(AssetBalance {
        symbol: "SOL".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    sol_entry.total_amount += balances.sol_balance;
    if let Some(usd_value) = balances.sol_usd_value {
        sol_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }

    // Add SPL token balances
    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        let entry = company_summary.assets.entry(symbol.to_string()).or_insert(AssetBalance {
            symbol: symbol.to_string(),
            total_amount: 0.0,
            total_usd_value: 0.0,
        });
        entry.total_amount += token.ui_amount;
        if let Some(usd_value) = token.usd_value {
            entry.total_usd_value += usd_value;
            company_summary.total_usd_value += usd_value;
            portfolio.total_usd_value += usd_value;
        }
    }
}

fn aggregate_evm_balances(portfolio: &mut PortfolioSummary, company: &str, balances: &evm::AccountBalances, chain: &Chain) {
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    // Add native token balance (ETH, CORE, MATIC, BNB, AVAX, etc.)
    let native_symbol = chain.native_token_symbol();
    let native_entry = company_summary.assets.entry(native_symbol.to_string()).or_insert(AssetBalance {
        symbol: native_symbol.to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    native_entry.total_amount += balances.eth_balance;
    if let Some(usd_value) = balances.eth_usd_value {
        native_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }

    // Add ERC20 token balances
    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        let entry = company_summary.assets.entry(symbol.to_string()).or_insert(AssetBalance {
            symbol: symbol.to_string(),
            total_amount: 0.0,
            total_usd_value: 0.0,
        });
        entry.total_amount += token.ui_amount;
        if let Some(usd_value) = token.usd_value {
            entry.total_usd_value += usd_value;
            company_summary.total_usd_value += usd_value;
            portfolio.total_usd_value += usd_value;
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
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    let near_entry = company_summary.assets.entry("NEAR".to_string()).or_insert(AssetBalance {
        symbol: "NEAR".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    near_entry.total_amount += balances.near_balance;
    if let Some(usd_value) = balances.near_usd_value {
        near_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }
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
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    let apt_entry = company_summary.assets.entry("APT".to_string()).or_insert(AssetBalance {
        symbol: "APT".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    apt_entry.total_amount += balances.apt_balance;
    if let Some(usd_value) = balances.apt_usd_value {
        apt_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }
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
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    let sui_entry = company_summary.assets.entry("SUI".to_string()).or_insert(AssetBalance {
        symbol: "SUI".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    sui_entry.total_amount += balances.sui_balance;
    if let Some(usd_value) = balances.sui_usd_value {
        sui_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }
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
    let company_key = if company.is_empty() { "Uncategorized" } else { company };

    let company_summary = portfolio.companies.entry(company_key.to_string()).or_insert(CompanySummary {
        company: company_key.to_string(),
        assets: HashMap::new(),
        total_usd_value: 0.0,
    });

    let eth_entry = company_summary.assets.entry("ETH".to_string()).or_insert(AssetBalance {
        symbol: "ETH".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    eth_entry.total_amount += balances.eth_balance;
    if let Some(usd_value) = balances.eth_usd_value {
        eth_entry.total_usd_value += usd_value;
        company_summary.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }
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
