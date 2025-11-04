mod cli;
mod evm;
mod price;
mod solana;
mod storage;
mod ui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use evm::EvmClient;
use price::PriceService;
use solana::SolanaClient;
use storage::{AddressBook, Chain};
use std::collections::HashMap;

#[derive(Debug)]
pub struct AssetBalance {
    pub symbol: String,
    pub total_amount: f64,
    pub total_usd_value: f64,
}

#[derive(Debug)]
pub struct PortfolioSummary {
    pub assets: HashMap<String, AssetBalance>,
    pub total_usd_value: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { name, address, chain } => {
            add_address(name, address, chain)?;
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

fn add_address(name: String, address: String, chain: Option<String>) -> Result<()> {
    let mut book = AddressBook::load()?;
    book.add_address(name.clone(), address.clone(), chain)?;
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

    let mut portfolio = PortfolioSummary {
        assets: HashMap::new(),
        total_usd_value: 0.0,
    };

    // Create price service and cache
    let price_service = PriceService::new();
    let mut price_cache: HashMap<String, f64> = HashMap::new();

    // Batch fetch all known prices in a single API call
    println!("Fetching cryptocurrency prices...");
    match price_service.batch_fetch_all_known_prices().await {
        Ok(prices) => {
            price_cache = prices;
            println!("Successfully fetched prices for {} symbols\n", price_cache.len());
        }
        Err(e) => {
            eprintln!("Warning: Failed to batch fetch prices: {}", e);
            eprintln!("Will attempt to fetch prices individually as needed.\n");
        }
    }

    for wallet in book.addresses.iter() {
        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(rpc_url.clone());
                let balances = query_and_display_solana(&client, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
                aggregate_solana_balances(&mut portfolio, &balances);
            }
            chain if chain.is_evm() => {
                let client = EvmClient::new(rpc_url.clone(), chain.clone());
                let balances = query_and_display_evm(&client, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
                aggregate_evm_balances(&mut portfolio, &balances);
            }
            _ => {}
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
            query_and_display_solana(&client, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        chain if chain.is_evm() => {
            let client = EvmClient::new(rpc_url, chain.clone());
            query_and_display_evm(&client, &wallet.name, &wallet.address, &wallet.chain, &price_service, &mut price_cache).await?;
        }
        _ => {}
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
            ui::render_solana_balances(name, address, &balances, chain);
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
            ui::render_evm_balances(name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query EVM address")
        }
    }
}

fn aggregate_solana_balances(portfolio: &mut PortfolioSummary, balances: &solana::AccountBalances) {
    // Add SOL balance
    let sol_entry = portfolio.assets.entry("SOL".to_string()).or_insert(AssetBalance {
        symbol: "SOL".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    sol_entry.total_amount += balances.sol_balance;
    if let Some(usd_value) = balances.sol_usd_value {
        sol_entry.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }

    // Add SPL token balances
    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        let entry = portfolio.assets.entry(symbol.to_string()).or_insert(AssetBalance {
            symbol: symbol.to_string(),
            total_amount: 0.0,
            total_usd_value: 0.0,
        });
        entry.total_amount += token.ui_amount;
        if let Some(usd_value) = token.usd_value {
            entry.total_usd_value += usd_value;
            portfolio.total_usd_value += usd_value;
        }
    }
}

fn aggregate_evm_balances(portfolio: &mut PortfolioSummary, balances: &evm::AccountBalances) {
    // Add ETH balance
    let eth_entry = portfolio.assets.entry("ETH".to_string()).or_insert(AssetBalance {
        symbol: "ETH".to_string(),
        total_amount: 0.0,
        total_usd_value: 0.0,
    });
    eth_entry.total_amount += balances.eth_balance;
    if let Some(usd_value) = balances.eth_usd_value {
        eth_entry.total_usd_value += usd_value;
        portfolio.total_usd_value += usd_value;
    }

    // Add ERC20 token balances
    for token in &balances.token_balances {
        let symbol = token.symbol.as_deref().unwrap_or("Unknown");
        let entry = portfolio.assets.entry(symbol.to_string()).or_insert(AssetBalance {
            symbol: symbol.to_string(),
            total_amount: 0.0,
            total_usd_value: 0.0,
        });
        entry.total_amount += token.ui_amount;
        if let Some(usd_value) = token.usd_value {
            entry.total_usd_value += usd_value;
            portfolio.total_usd_value += usd_value;
        }
    }
}
