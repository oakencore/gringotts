mod banking;
mod chains;
mod cli;
mod display;
mod query;
mod services;
mod storage;
mod types;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use display::terminal as ui;
use storage::{AddressBook, BankingAccount, BankingService};

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file if present
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    match cli.command {
        Commands::Add {
            company,
            name,
            address,
            chain,
        } => {
            let mut book = AddressBook::load()?;
            book.add_address(company, name, address, chain)?;
            book.save()?;
            ui::render_success("Address added successfully");
        }
        Commands::List { company } => {
            list_addresses(company)?;
        }
        Commands::Remove { identifier } => {
            remove_address(identifier)?;
        }
        Commands::Query { rpc_url, no_prices } => {
            query::query_all(rpc_url, no_prices).await?;
        }
        Commands::QueryOne {
            name,
            rpc_url,
            no_prices,
        } => {
            query::query_one(name, rpc_url, no_prices).await?;
        }
        Commands::AddBank {
            company,
            name,
            account_id,
            service,
        } => {
            let banking_service = BankingService::from_str(&service)?;
            add_banking_account(company, name, account_id, banking_service)?;
        }
        Commands::SetupMercury { company } => {
            setup_mercury_accounts(company).await?;
        }
        Commands::ListMercuryAccounts => {
            let client = banking::mercury::MercuryClient::new()?;
            let accounts = client.list_accounts().await?;
            if accounts.is_empty() {
                println!("No Mercury accounts found.");
            } else {
                println!("\nMercury Accounts ({}):\n", accounts.len());
                for account in &accounts {
                    println!("  {} (ID: {})", account.name, account.id);
                }
            }
        }
        Commands::ExportTransactions {
            name,
            format,
            start,
            end,
            output,
        } => {
            query::export_transactions(name, format, start, end, output).await?;
        }
        Commands::Serve {
            port,
            refresh_interval,
            eager,
            api_key,
        } => {
            display::web::start_server(port, refresh_interval, eager, api_key).await?;
        }
    }

    Ok(())
}

fn list_addresses(company_filter: Option<String>) -> Result<()> {
    let book = AddressBook::load()?;

    if book.addresses.is_empty() && book.banking_accounts.is_empty() {
        println!("No addresses or accounts tracked yet.");
        println!("Use 'gringotts add' to add blockchain addresses.");
        println!("Use 'gringotts add-bank' to add banking accounts.");
        return Ok(());
    }

    let filter = company_filter.as_deref().map(|s| s.to_lowercase());

    let filtered_addresses: Vec<_> = book
        .addresses
        .iter()
        .filter(|w| match &filter {
            Some(f) => w.company.to_lowercase().contains(f),
            None => true,
        })
        .collect();

    let filtered_accounts: Vec<_> = book
        .banking_accounts
        .iter()
        .filter(|a| match &filter {
            Some(f) => a.company.to_lowercase().contains(f),
            None => true,
        })
        .collect();

    if filtered_addresses.is_empty() && filtered_accounts.is_empty() {
        if filter.is_some() {
            println!(
                "No addresses or accounts match the company filter '{}'.",
                company_filter.unwrap()
            );
        } else {
            println!("No addresses or accounts tracked yet.");
        }
        return Ok(());
    }

    if !filtered_addresses.is_empty() {
        println!("\n=== Tracked Blockchain Addresses ===\n");
        for (i, wallet) in filtered_addresses.iter().enumerate() {
            println!(
                "{}. {} - {} ({})",
                i + 1,
                wallet.name,
                wallet.address,
                wallet.chain.display_name()
            );
            if !wallet.company.is_empty() {
                println!("   Company: {}", wallet.company);
            }
            println!();
        }
    }

    if !filtered_accounts.is_empty() {
        println!("\n=== Tracked Banking Accounts ===\n");
        for (i, account) in filtered_accounts.iter().enumerate() {
            println!(
                "{}. {} - {} ({})",
                i + 1,
                account.name,
                account.account_id,
                account.service.display_name()
            );
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
    book.addresses
        .retain(|w| w.name != identifier && w.address != identifier);

    if book.addresses.len() < initial_len {
        book.save()?;
        ui::render_success(&format!("Removed '{}'", identifier));
        return Ok(());
    }

    // Try to remove from banking accounts
    let initial_bank_len = book.banking_accounts.len();
    book.banking_accounts
        .retain(|a| a.name != identifier && a.account_id != identifier);

    if book.banking_accounts.len() < initial_bank_len {
        book.save()?;
        ui::render_success(&format!("Removed '{}'", identifier));
        return Ok(());
    }

    ui::render_error(&format!(
        "No address or account found with identifier '{}'",
        identifier
    ));
    Ok(())
}

fn add_banking_account(
    company: String,
    name: String,
    account_id: String,
    service: BankingService,
) -> Result<()> {
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
    let client = banking::mercury::MercuryClient::new()?;
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
        if book
            .banking_accounts
            .iter()
            .any(|a| a.account_id == account.id)
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Chain, WalletAddress};
    use crate::types::{add_asset_to_portfolio, PortfolioSummary, PriceEnrichable};
    use std::collections::HashMap;

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
        let mut balances = chains::near::AccountBalances {
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
        let mut balances = chains::near::AccountBalances {
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
            addresses: vec![WalletAddress {
                company: "TestCo".to_string(),
                name: "Test Wallet".to_string(),
                address: "test123".to_string(),
                chain: Chain::Solana,
            }],
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
