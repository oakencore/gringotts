use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::io::Write;

use crate::banking::circle::CircleClient;
use crate::banking::mercury::MercuryClient;
use crate::banking::{circle, mercury};
use crate::chains::aptos::AptosClient;
use crate::chains::evm::EvmClient;
use crate::chains::near::NearClient;
use crate::chains::solana::SolanaClient;
use crate::chains::starknet::StarknetClient;
use crate::chains::sui::SuiClient;
use crate::chains::{aptos, evm, near, solana, starknet, sui};
use crate::display::terminal as ui;
use crate::services::price::PriceService;
use crate::storage::{AddressBook, BankingService, Chain, WalletAddress};
use crate::types::{
    add_asset_to_portfolio, FetchAllResult, FetchFailure, PortfolioSummary, PriceEnrichable,
    PriceFetchResult, WalletBalances,
};

// Helper function to fetch all balances from wallets and banking accounts
// Returns both successful balances and tracked failures for partial failure handling
pub async fn fetch_all_balances(book: &AddressBook, rpc_url: Option<String>) -> FetchAllResult {
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
    let mut failures: Vec<FetchFailure> = Vec::new();

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
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
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
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
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
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
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
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
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
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
                    }
                }
            }
            // All EVM chains
            Chain::Ethereum
            | Chain::Polygon
            | Chain::BinanceSmartChain
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::Core => match EvmClient::new(rpc_url.clone(), wallet.chain.clone()) {
                Ok(client) => match client.get_balances(&wallet.address).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Evm(wallet.clone(), balances));
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            wallet.name, wallet.address, e
                        ));
                        failures.push(FetchFailure {
                            name: wallet.name.clone(),
                            address_or_id: wallet.address.clone(),
                            chain_or_service: wallet.chain.display_name().to_string(),
                            error: error_msg,
                        });
                    }
                },
                Err(e) => {
                    let error_msg = format!("Failed to create client: {}", e);
                    pb.println(format!(
                        "Warning: Failed to create EVM client for {} ({}): {}",
                        wallet.name, wallet.address, e
                    ));
                    failures.push(FetchFailure {
                        name: wallet.name.clone(),
                        address_or_id: wallet.address.clone(),
                        chain_or_service: wallet.chain.display_name().to_string(),
                        error: error_msg,
                    });
                }
            },
        }
        pb.inc(1);
    }

    // Query banking accounts
    for account in book.banking_accounts.iter() {
        match &account.service {
            BankingService::Mercury => match MercuryClient::new() {
                Ok(client) => match client.get_account_balance(&account.account_id).await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Mercury(account.clone(), balances));
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} ({}): {}",
                            account.name, account.account_id, e
                        ));
                        failures.push(FetchFailure {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: account.service.display_name().to_string(),
                            error: error_msg,
                        });
                    }
                },
                Err(e) => {
                    let error_msg = format!("Failed to initialize client: {}", e);
                    pb.println(format!(
                        "Warning: Failed to initialize Mercury client: {}",
                        e
                    ));
                    failures.push(FetchFailure {
                        name: account.name.clone(),
                        address_or_id: account.account_id.clone(),
                        chain_or_service: account.service.display_name().to_string(),
                        error: error_msg,
                    });
                }
            },
            BankingService::Circle => match CircleClient::new() {
                Ok(client) => match client.get_balances().await {
                    Ok(balances) => {
                        all_balances.push(WalletBalances::Circle(account.clone(), balances));
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        pb.println(format!(
                            "Warning: Failed to query {} Circle balances: {}",
                            account.name, e
                        ));
                        failures.push(FetchFailure {
                            name: account.name.clone(),
                            address_or_id: account.account_id.clone(),
                            chain_or_service: account.service.display_name().to_string(),
                            error: error_msg,
                        });
                    }
                },
                Err(e) => {
                    let error_msg = format!("Failed to initialize client: {}", e);
                    pb.println(format!(
                        "Warning: Failed to initialize Circle client: {}",
                        e
                    ));
                    failures.push(FetchFailure {
                        name: account.name.clone(),
                        address_or_id: account.account_id.clone(),
                        chain_or_service: account.service.display_name().to_string(),
                        error: error_msg,
                    });
                }
            },
        }
        pb.inc(1);
    }

    // Show summary with success/failure counts
    let success_count = all_balances.len();
    let failure_count = failures.len();
    if failure_count > 0 {
        pb.finish_with_message(format!(
            "Fetched {} of {} items ({} failed)",
            success_count, total_items, failure_count
        ));
    } else {
        pb.finish_with_message(format!(
            "Successfully fetched balances from {} items",
            success_count
        ));
    }
    println!();

    FetchAllResult {
        balances: all_balances,
        failures,
    }
}

// Helper function to extract unique token symbols from all balances
pub fn extract_token_symbols(all_balances: &[WalletBalances]) -> HashSet<String> {
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
                symbols.insert(balances.native_symbol.clone());
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
// Uses cache fallback if API fails
pub async fn fetch_prices_for_symbols(symbols: HashSet<String>) -> Result<PriceFetchResult> {
    if symbols.is_empty() {
        return Ok(PriceFetchResult {
            prices: HashMap::new(),
            from_cache: false,
            cache_age: None,
        });
    }

    let mut price_service = PriceService::new()?;
    let price_pb = ProgressBar::new_spinner();
    price_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("valid spinner template"),
    );
    price_pb.set_message(format!(
        "Fetching USD prices for {} unique tokens...",
        symbols.len()
    ));
    price_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let symbols_vec: Vec<String> = symbols.into_iter().collect();
    let (prices, from_cache) = price_service
        .fetch_prices_with_fallback(&symbols_vec)
        .await?;

    let cache_age = if from_cache || !prices.is_empty() {
        Some(price_service.cache_age())
    } else {
        None
    };

    if from_cache {
        price_pb.finish_with_message(format!(
            "✓ Using cached prices for {} symbols (updated {})",
            prices.len(),
            cache_age.as_ref().unwrap_or(&"unknown".to_string())
        ));
    } else if !prices.is_empty() {
        price_pb.finish_with_message(format!(
            "✓ Successfully fetched prices for {} symbols",
            prices.len()
        ));
    } else {
        price_pb.finish_with_message("⚠ Failed to fetch prices and no cache available");
        price_pb.println("Balances will be displayed without USD values.");
    }
    println!();

    Ok(PriceFetchResult {
        prices,
        from_cache,
        cache_age,
    })
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
                ui::render_solana_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_solana_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Evm(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_evm_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_evm_balances(&mut portfolio, &wallet.company, &balances, &wallet.chain);
            }
            WalletBalances::Near(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_near_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_near_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Aptos(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_aptos_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_aptos_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Sui(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_sui_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_sui_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Starknet(wallet, mut balances) => {
                balances.enrich_from_cache(price_cache);
                ui::render_starknet_balances(
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &balances,
                    &wallet.chain,
                );
                aggregate_starknet_balances(&mut portfolio, &wallet.company, &balances);
            }
            WalletBalances::Mercury(account, balances) => {
                ui::render_mercury_balances(
                    &account.company,
                    &account.name,
                    &account.account_id,
                    &balances,
                    &account.service,
                );
                aggregate_mercury_balances(&mut portfolio, &account.company, &balances);
            }
            WalletBalances::Circle(account, balances) => {
                ui::render_circle_balances(
                    &account.company,
                    &account.name,
                    &balances,
                    &account.service,
                );
                aggregate_circle_balances(&mut portfolio, &account.company, &balances);
            }
        }
    }

    portfolio
}

pub async fn query_all(rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        println!("No addresses or accounts tracked yet.");
        println!("Use 'gringotts add' to add blockchain addresses.");
        println!("Use 'gringotts add-bank' to add banking accounts.");
        return Ok(());
    }

    if no_prices {
        println!(
            "\nQuerying balances for all tracked addresses and accounts (without prices)...\n"
        );
    } else {
        println!("\nQuerying balances for all tracked addresses and accounts...\n");
    }

    // Fetch all balances (includes partial failure handling)
    let fetch_result = fetch_all_balances(&book, rpc_url).await;

    // Extract symbols and fetch prices (skip if --no-prices)
    let (price_cache, price_info) = if !no_prices {
        let symbols = extract_token_symbols(&fetch_result.balances);
        let result = fetch_prices_for_symbols(symbols).await?;
        let info = if result.from_cache {
            result
                .cache_age
                .map(|age| format!("Prices from cache (updated {})", age))
        } else {
            None
        };
        (result.prices, info)
    } else {
        (HashMap::new(), None)
    };

    // Enrich balances with prices and display
    let portfolio = enrich_and_display_balances(fetch_result.balances, &price_cache);

    // Display portfolio summary with cache info
    ui::render_portfolio_summary(&portfolio, price_info.as_deref());

    // Display failure summary if any
    if !fetch_result.failures.is_empty() {
        ui::render_fetch_failures(&fetch_result.failures);
    }

    Ok(())
}

pub async fn query_one(identifier: String, rpc_url: Option<String>, no_prices: bool) -> Result<()> {
    let book = AddressBook::load()?;

    // Try to find wallet by name or address
    let wallet = book
        .addresses
        .iter()
        .find(|w| w.name == identifier || w.address == identifier);

    if let Some(wallet) = wallet {
        println!("\nQuerying balance for '{}'...\n", wallet.name);

        let mut price_service = PriceService::new()?;
        let mut price_cache: HashMap<String, f64> = HashMap::new();

        // Pre-fetch prices for common symbols and the wallet's native token
        if !no_prices {
            println!("Fetching cryptocurrency prices...");
            let native = wallet.chain.native_token_symbol().to_string();
            let mut symbols = vec![
                "SOL".to_string(),
                "ETH".to_string(),
                "USDC".to_string(),
                "USDT".to_string(),
            ];
            if !symbols.contains(&native) {
                symbols.push(native);
            }
            let (prices, from_cache) = price_service.fetch_prices_with_fallback(&symbols).await?;
            price_cache = prices;
            if from_cache {
                println!(
                    "Using cached prices (updated {})\n",
                    price_service.cache_age()
                );
            } else if !price_cache.is_empty() {
                println!("Successfully fetched prices\n");
            } else {
                eprintln!("Warning: Failed to fetch prices and no cache available.\n");
            }
        }

        match &wallet.chain {
            Chain::Solana => {
                let client = SolanaClient::new(rpc_url);
                query_and_display_solana(
                    &client,
                    wallet,
                    &mut price_service,
                    &mut price_cache,
                    no_prices,
                )
                .await?;
            }
            Chain::Near => {
                let client = NearClient::new(rpc_url);
                query_and_display_near(
                    &client,
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &wallet.chain,
                    &mut price_service,
                    &mut price_cache,
                )
                .await?;
            }
            Chain::Aptos => {
                let client = AptosClient::new(rpc_url);
                query_and_display_aptos(
                    &client,
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &wallet.chain,
                    &mut price_service,
                    &mut price_cache,
                )
                .await?;
            }
            Chain::Sui => {
                let client = SuiClient::new(rpc_url);
                query_and_display_sui(
                    &client,
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &wallet.chain,
                    &mut price_service,
                    &mut price_cache,
                )
                .await?;
            }
            Chain::Starknet => {
                let client = StarknetClient::new(rpc_url);
                query_and_display_starknet(
                    &client,
                    &wallet.company,
                    &wallet.name,
                    &wallet.address,
                    &wallet.chain,
                    &mut price_service,
                    &mut price_cache,
                )
                .await?;
            }
            // All EVM chains
            Chain::Ethereum
            | Chain::Polygon
            | Chain::BinanceSmartChain
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::Core => {
                let client = EvmClient::new(rpc_url, wallet.chain.clone())?;
                query_and_display_evm(
                    &client,
                    wallet,
                    &mut price_service,
                    &mut price_cache,
                    no_prices,
                )
                .await?;
            }
        }

        return Ok(());
    }

    // Try to find banking account
    let account = book
        .banking_accounts
        .iter()
        .find(|a| a.name == identifier || a.account_id == identifier);

    if let Some(account) = account {
        println!("\nQuerying balance for '{}'...\n", account.name);

        match &account.service {
            BankingService::Mercury => {
                let client = MercuryClient::new()?;
                let balances = client.get_account_balance(&account.account_id).await?;
                ui::render_mercury_balances(
                    &account.company,
                    &account.name,
                    &account.account_id,
                    &balances,
                    &account.service,
                );
            }
            BankingService::Circle => {
                let client = CircleClient::new()?;
                let balances = client.get_balances().await?;
                ui::render_circle_balances(
                    &account.company,
                    &account.name,
                    &balances,
                    &account.service,
                );
            }
        }

        return Ok(());
    }

    ui::render_error(&format!(
        "No address or account found with identifier '{}'",
        identifier
    ));
    Ok(())
}

async fn query_and_display_solana(
    client: &SolanaClient,
    wallet: &WalletAddress,
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<solana::AccountBalances> {
    match client.get_balances(&wallet.address) {
        Ok(mut balances) => {
            if !no_prices {
                balances.enrich_from_cache(price_cache);
            }
            ui::render_solana_balances(
                &wallet.company,
                &wallet.name,
                &wallet.address,
                &balances,
                &wallet.chain,
            );
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!(
                "Error querying '{}' ({}): {}",
                wallet.name, wallet.address, e
            ));
            anyhow::bail!("Failed to query Solana address")
        }
    }
}

async fn query_and_display_evm(
    client: &EvmClient,
    wallet: &WalletAddress,
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
    no_prices: bool,
) -> Result<evm::AccountBalances> {
    match client.get_balances(&wallet.address).await {
        Ok(mut balances) => {
            if !no_prices {
                balances.enrich_from_cache(price_cache);
            }
            ui::render_evm_balances(
                &wallet.company,
                &wallet.name,
                &wallet.address,
                &balances,
                &wallet.chain,
            );
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!(
                "Error querying '{}' ({}): {}",
                wallet.name, wallet.address, e
            ));
            anyhow::bail!("Failed to query EVM address")
        }
    }
}

fn aggregate_solana_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &solana::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "SOL",
        balances.sol_balance,
        balances.sol_usd_value,
    );

    for token in &balances.token_balances {
        if let Some(symbol) = &token.symbol {
            add_asset_to_portfolio(portfolio, company, symbol, token.ui_amount, token.usd_value);
        }
    }
}

fn aggregate_evm_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &evm::AccountBalances,
    _chain: &Chain,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        &balances.native_symbol,
        balances.eth_balance,
        balances.eth_usd_value,
    );

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
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<near::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            balances.enrich_from_cache(price_cache);
            ui::render_near_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query NEAR address")
        }
    }
}

fn aggregate_near_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &near::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "NEAR",
        balances.near_balance,
        balances.near_usd_value,
    );
}

async fn query_and_display_aptos(
    client: &AptosClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<aptos::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            balances.enrich_from_cache(price_cache);
            ui::render_aptos_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Aptos address")
        }
    }
}

fn aggregate_aptos_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &aptos::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "APT",
        balances.apt_balance,
        balances.apt_usd_value,
    );
}

async fn query_and_display_sui(
    client: &SuiClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<sui::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            balances.enrich_from_cache(price_cache);
            ui::render_sui_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Sui address")
        }
    }
}

fn aggregate_sui_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &sui::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "SUI",
        balances.sui_balance,
        balances.sui_usd_value,
    );
}

async fn query_and_display_starknet(
    client: &StarknetClient,
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    _price_service: &mut PriceService,
    price_cache: &mut HashMap<String, f64>,
) -> Result<starknet::AccountBalances> {
    match client.get_balances(address).await {
        Ok(mut balances) => {
            balances.enrich_from_cache(price_cache);
            ui::render_starknet_balances(company, name, address, &balances, chain);
            Ok(balances)
        }
        Err(e) => {
            ui::render_error(&format!("Error querying '{}' ({}): {}", name, address, e));
            anyhow::bail!("Failed to query Starknet address")
        }
    }
}

fn aggregate_starknet_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &starknet::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "ETH",
        balances.eth_balance,
        balances.eth_usd_value,
    );
}

fn aggregate_mercury_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &mercury::AccountBalances,
) {
    add_asset_to_portfolio(
        portfolio,
        company,
        "USD",
        balances.current_balance,
        Some(balances.current_balance),
    );
}

fn aggregate_circle_balances(
    portfolio: &mut PortfolioSummary,
    company: &str,
    balances: &circle::AccountBalances,
) {
    // Aggregate available balances - only treat USD-denominated balances as USD value
    for balance in &balances.available_balances {
        let usd_value = if balance.currency == "USD" {
            Some(balance.amount)
        } else {
            // Non-USD currencies need conversion - don't assume 1:1 with USD
            None
        };
        add_asset_to_portfolio(
            portfolio,
            company,
            &balance.currency,
            balance.amount,
            usd_value,
        );
    }
}

pub async fn export_transactions(
    account_name: String,
    format: String,
    start: Option<String>,
    end: Option<String>,
    output: Option<String>,
) -> Result<()> {
    let book = AddressBook::load()?;

    let account = book
        .banking_accounts
        .iter()
        .find(|a| a.name == account_name || a.account_id == account_name)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", account_name))?;

    match &account.service {
        BankingService::Mercury => {
            let client = MercuryClient::new()?;
            let transactions = client
                .get_transactions(&account.account_id, start.as_deref(), end.as_deref())
                .await?;

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
            return Err(anyhow::anyhow!(
                "Transaction export not yet supported for Circle accounts"
            ));
        }
    }

    Ok(())
}

fn export_mercury_transactions(
    transactions: &[mercury::Transaction],
    format: &str,
) -> Result<String> {
    fn escape_csv(s: &str) -> String {
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }

    let output_data = match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(&transactions)?,
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
                    escape_csv(&date),
                    escape_csv(&format!("{}", tx.amount)),
                    escape_csv(&tx.status),
                    escape_csv(counterparty),
                    escape_csv(description),
                    escape_csv(note),
                    escape_csv(&tx.kind)
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
    use crate::storage::Chain;

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
            native_symbol: "ETH".to_string(),
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
}
