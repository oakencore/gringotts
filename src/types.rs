use std::collections::HashMap;

use serde::Serialize;

use crate::banking::{circle, mercury};
use crate::chains::{aptos, evm, near, solana, starknet, sui};
use crate::storage::{BankingAccount, WalletAddress};

// Portfolio summary structure
#[derive(Serialize)]
pub struct PortfolioSummary {
    pub companies: HashMap<String, CompanyAssets>,
    pub total_usd_value: f64,
}

#[derive(Serialize)]
pub struct CompanyAssets {
    pub assets: HashMap<String, AssetSummary>,
    pub wallets: HashMap<String, WalletAssets>,
    pub total_usd_value: f64,
}

/// Per-wallet breakdown of holdings within a company.
///
/// Invariant: `name` mirrors the key under which this struct is stored in
/// `CompanyAssets.wallets`. The field exists for ergonomic iteration
/// (`for w in company.wallets.values() { use w.name }`), matching the
/// existing convention on `AssetSummary.symbol`.
#[derive(Serialize)]
pub struct WalletAssets {
    pub name: String,
    pub assets: HashMap<String, AssetSummary>,
    pub total_usd_value: f64,
}

#[derive(Serialize)]
pub struct AssetSummary {
    pub symbol: String,
    pub amount: f64,
    pub usd_value: Option<f64>,
}

/// Inserts or updates an asset entry in an assets map, accumulating amount and USD value
/// into both the entry and a running total. File-private helper for
/// [`add_asset_to_portfolio`] - shared between the rollup and per-wallet update paths.
fn upsert_asset(
    assets: &mut HashMap<String, AssetSummary>,
    running_total: &mut f64,
    symbol: &str,
    amount: f64,
    usd_value: Option<f64>,
) {
    let asset = assets
        .entry(symbol.to_string())
        .or_insert_with(|| AssetSummary {
            symbol: symbol.to_string(),
            amount: 0.0,
            usd_value: None,
        });
    asset.amount += amount;
    if let Some(value) = usd_value {
        asset.usd_value = Some(asset.usd_value.unwrap_or(0.0) + value);
        *running_total += value;
    }
}

/// Records an asset into the portfolio, updating both representations:
///
/// - **Aggregated rollup**: `company.assets[symbol]` - sum across all wallets in the company.
/// - **Per-wallet breakdown**: `company.wallets[wallet_name].assets[symbol]` - same data
///   keyed by wallet so callers can attribute each balance to its source.
///
/// Both maps are written in every call; they must remain consistent. Do not remove either
/// path without updating every consumer (terminal renderer reads `company.assets`; web
/// renderer reads `company.wallets`).
///
/// Zero-amount calls are skipped entirely (no entry created in either map).
pub fn add_asset_to_portfolio(
    portfolio: &mut PortfolioSummary,
    company: &str,
    wallet_name: &str,
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
            wallets: HashMap::new(),
            total_usd_value: 0.0,
        });

    // Aggregated rollup
    upsert_asset(
        &mut company_assets.assets,
        &mut company_assets.total_usd_value,
        symbol,
        amount,
        usd_value,
    );

    // Per-wallet breakdown
    let wallet = company_assets
        .wallets
        .entry(wallet_name.to_string())
        .or_insert_with(|| WalletAssets {
            name: wallet_name.to_string(),
            assets: HashMap::new(),
            total_usd_value: 0.0,
        });
    upsert_asset(
        &mut wallet.assets,
        &mut wallet.total_usd_value,
        symbol,
        amount,
        usd_value,
    );

    // Portfolio-level total - only incremented once per call (not once per upsert)
    // to avoid double-counting between the rollup and per-wallet paths.
    if let Some(value) = usd_value {
        portfolio.total_usd_value += value;
    }
}

// Trait for price enrichment - eliminates duplicate code across chains
pub trait PriceEnrichable {
    fn native_symbol(&self) -> &str;

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
        if let Some(&price) = price_cache.get(self.native_symbol()) {
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
pub enum WalletBalances {
    Solana(WalletAddress, solana::AccountBalances),
    Evm(WalletAddress, evm::AccountBalances),
    Near(WalletAddress, near::AccountBalances),
    Aptos(WalletAddress, aptos::AccountBalances),
    Sui(WalletAddress, sui::AccountBalances),
    Starknet(WalletAddress, starknet::AccountBalances),
    Mercury(BankingAccount, mercury::AccountBalances),
    Circle(BankingAccount, circle::AccountBalances),
    /// Manual accounts carry their balance on the account itself.
    Manual(BankingAccount),
}

/// Represents a failed wallet/account query
#[derive(Debug, Clone)]
pub struct FetchFailure {
    pub name: String,
    pub chain_or_service: String,
    pub error: String,
}

/// Result of fetching all balances - includes successes and failures
pub struct FetchAllResult {
    pub balances: Vec<WalletBalances>,
    pub failures: Vec<FetchFailure>,
}

/// Result of price fetching - includes whether prices came from cache
pub struct PriceFetchResult {
    pub prices: HashMap<String, f64>,
    pub from_cache: bool,
    pub cache_age: Option<String>,
}
