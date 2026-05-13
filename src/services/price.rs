use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use i_am_surging::SurgeClient;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Cached price data with timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceCache {
    pub prices: HashMap<String, f64>,
    pub updated_at: DateTime<Utc>,
}

impl PriceCache {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
            updated_at: Utc::now(),
        }
    }

    fn get_cache_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Failed to get home directory")?;
        Ok(home.join(".gringotts").join("price_cache.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::get_cache_path()?;

        if !path.exists() {
            return Ok(Self::new());
        }

        let content = fs::read_to_string(&path).context("Failed to read price cache")?;

        let cache: PriceCache =
            serde_json::from_str(&content).context("Failed to parse price cache")?;

        Ok(cache)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::get_cache_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create cache directory")?;
        }

        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize price cache")?;

        fs::write(&path, content).context("Failed to write price cache")?;

        Ok(())
    }

    /// Get a human-readable string describing how old the cache is
    pub fn age_string(&self) -> String {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.updated_at);

        if duration.num_seconds() < 60 {
            "just now".to_string()
        } else if duration.num_minutes() < 60 {
            let mins = duration.num_minutes();
            format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" })
        } else if duration.num_hours() < 24 {
            let hours = duration.num_hours();
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        } else {
            let days = duration.num_days();
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        }
    }

    /// Check if the cache is considered stale (older than the given duration in minutes)
    pub fn is_stale(&self, max_age_minutes: i64) -> bool {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.updated_at);
        duration.num_minutes() >= max_age_minutes
    }
}

/// PriceService using Switchboard Surge for cryptocurrency prices
/// Provides efficient price queries for 2,266+ trading pairs
/// Includes caching to persist prices across program restarts
pub struct PriceService {
    surge_client: SurgeClient,
    cache: PriceCache,
}

// Global rate limiter shared across all PriceService instances
static LAST_REQUEST_MS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

// Minimum delay between API requests to avoid rate limiting (1 second)
const MIN_REQUEST_DELAY_MS: u64 = 1000;

impl PriceService {
    pub fn new() -> Result<Self> {
        // Check for API key and warn if not set
        if Self::get_api_key().is_err() {
            eprintln!("Warning: SURGE_API_KEY not set. Price queries may fail.");
            eprintln!("Get your API key (Solana wallet address) from https://switchboard.xyz");
        }

        // Create the Surge client (reads API key from environment internally)
        let surge_client = SurgeClient::new().context("Failed to create SurgeClient")?;

        // Load cached prices
        let cache = PriceCache::load().unwrap_or_else(|_| PriceCache::new());

        Ok(Self {
            surge_client,
            cache,
        })
    }

    /// Get a reference to the price cache
    pub fn get_cache(&self) -> &PriceCache {
        &self.cache
    }

    /// Save the current cache to disk
    pub fn save_cache(&self) -> Result<()> {
        self.cache.save()
    }

    /// Get cached prices without fetching from API
    pub fn get_cached_prices(&self) -> &HashMap<String, f64> {
        &self.cache.prices
    }

    /// Get the cache age as a human-readable string
    pub fn cache_age(&self) -> String {
        self.cache.age_string()
    }

    /// Check if cache has prices and return them with age info
    pub fn has_cached_prices(&self) -> bool {
        !self.cache.prices.is_empty()
    }

    fn current_time_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }

    /// Rate limit API requests to avoid 429 errors (uses global state)
    /// Uses compare_exchange to atomically claim the next request slot
    async fn rate_limit() {
        loop {
            let last = LAST_REQUEST_MS.load(Ordering::SeqCst);
            let now = Self::current_time_ms();
            let next_allowed = last + MIN_REQUEST_DELAY_MS;

            let target_time = if now >= next_allowed {
                now
            } else {
                next_allowed
            };

            // Atomically try to claim this time slot
            match LAST_REQUEST_MS.compare_exchange(
                last,
                target_time,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    // We claimed the slot - sleep if needed
                    if target_time > now {
                        tokio::time::sleep(Duration::from_millis(target_time - now)).await;
                    }
                    break;
                }
                Err(_) => {
                    // Another task claimed the slot, retry
                    continue;
                }
            }
        }
    }

    fn get_api_key() -> Result<String> {
        env::var("SURGE_API_KEY")
            .context("SURGE_API_KEY environment variable not set. Get your API key (Solana wallet address) from https://switchboard.xyz")
    }

    /// Internal method to fetch price without updating cache (used in fallback)
    async fn get_single_price_internal(&self, symbol: &str) -> Result<f64> {
        Self::rate_limit().await;

        // Convert symbol to trading pair format (e.g., "SOL" -> "SOL/USD")
        let trading_pair = format!("{}/USD", symbol);

        match self.surge_client.get_price(&trading_pair).await {
            Ok(price_data) => Ok(price_data.value),
            Err(e) => {
                anyhow::bail!("Failed to get price for {}: {}", symbol, e)
            }
        }
    }

    /// Get price for a single token symbol (e.g., "SOL", "ETH", "BTC")
    /// Returns price in USD and updates cache
    pub async fn get_single_price(&mut self, symbol: &str) -> Result<f64> {
        let price = self.get_single_price_internal(symbol).await?;

        // Update cache
        self.cache.prices.insert(symbol.to_string(), price);
        self.cache.updated_at = Utc::now();
        let _ = self.cache.save();

        Ok(price)
    }

    /// Fetch USD prices for multiple token mints (Solana-specific)
    /// This maintains backward compatibility with existing Solana code
    pub async fn get_prices(&mut self, mint_addresses: &[String]) -> Result<HashMap<String, f64>> {
        if mint_addresses.is_empty() {
            return Ok(HashMap::new());
        }

        const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

        let mut prices = HashMap::new();

        // Map known Solana mints to symbols
        for mint in mint_addresses {
            let symbol = match mint.as_str() {
                SOL_MINT => "SOL",
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC",
                "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT",
                "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So" => "MSOL",
                "7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj" => "stSOL",
                "SW1TCHLmRGTfW5xZknqQdpdarB8PD95sJYWpNp9TbFx" => "SWTCH",
                "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL" => "JTO",
                "GP2vH92rxSHWm2VzttZBZdeFnv9LyfFJYvPrAet6pump" => "RAT",
                _ => {
                    eprintln!("Warning: Unknown mint address {}, skipping", mint);
                    continue;
                }
            };

            match self.get_single_price(symbol).await {
                Ok(price) => {
                    prices.insert(mint.clone(), price);
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to fetch price for {} ({}): {}",
                        symbol, mint, e
                    );
                }
            }
        }

        Ok(prices)
    }

    /// Get ETH price in USD
    pub async fn get_eth_price(&mut self) -> Result<f64> {
        self.get_single_price("ETH").await
    }

    /// Get prices for ERC20 tokens (USDC, USDT, DAI, etc.)
    pub async fn get_erc20_prices(&mut self, symbols: &[String]) -> Result<HashMap<String, f64>> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        let mut prices = HashMap::new();

        for symbol in symbols {
            match self.get_single_price(symbol).await {
                Ok(price) => {
                    prices.insert(symbol.clone(), price);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to fetch price for {}: {}", symbol, e);
                }
            }
        }

        Ok(prices)
    }

    /// Batch fetch prices for a specific list of symbols
    /// More efficient than individual queries
    /// Updates the cache with fetched prices
    pub async fn batch_fetch_prices(&mut self, symbols: &[String]) -> Result<HashMap<String, f64>> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        Self::rate_limit().await;

        // Convert symbols to trading pairs
        let trading_pairs: Vec<String> = symbols.iter().map(|s| format!("{}/USD", s)).collect();

        let trading_pair_refs: Vec<&str> = trading_pairs.iter().map(|s| s.as_str()).collect();

        match self
            .surge_client
            .get_multiple_prices(&trading_pair_refs)
            .await
        {
            Ok(price_list) => {
                let mut prices = HashMap::new();

                for price_data in price_list {
                    // Extract symbol from trading pair (e.g., "BTC/USD" -> "BTC")
                    if let Some(base_symbol) = price_data.symbol.split('/').next() {
                        prices.insert(base_symbol.to_string(), price_data.value);
                    }
                }

                // Update cache with new prices
                self.update_cache(&prices);

                Ok(prices)
            }
            Err(e) => {
                eprintln!("Warning: Batch fetch failed: {}", e);
                // Fall back to individual queries with rate limiting
                let mut prices = HashMap::new();
                for symbol in symbols {
                    match self.get_single_price_internal(symbol).await {
                        Ok(price) => {
                            prices.insert(symbol.clone(), price);
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to get price for {}/USD: {}", symbol, e);
                        }
                    }
                }

                // Update cache if we got any prices
                if !prices.is_empty() {
                    self.update_cache(&prices);
                }

                Ok(prices)
            }
        }
    }

    /// Update the cache with new prices and save to disk
    fn update_cache(&mut self, prices: &HashMap<String, f64>) {
        for (symbol, price) in prices {
            self.cache.prices.insert(symbol.clone(), *price);
        }
        self.cache.updated_at = Utc::now();

        // Save cache to disk (ignore errors)
        if let Err(e) = self.cache.save() {
            eprintln!("Warning: Failed to save price cache: {}", e);
        }
    }

    /// Try to fetch fresh prices, fall back to cache if API fails
    /// Returns (prices, is_cached) - is_cached is true if using cached data
    pub async fn fetch_prices_with_fallback(
        &mut self,
        symbols: &[String],
    ) -> Result<(HashMap<String, f64>, bool)> {
        if symbols.is_empty() {
            return Ok((HashMap::new(), false));
        }

        // Try to fetch fresh prices
        match self.batch_fetch_prices(symbols).await {
            Ok(prices) if !prices.is_empty() => Ok((prices, false)),
            Ok(_) | Err(_) => {
                // Fall back to cached prices
                if self.has_cached_prices() {
                    let cached_prices: HashMap<String, f64> = symbols
                        .iter()
                        .filter_map(|s| self.cache.prices.get(s).map(|p| (s.clone(), *p)))
                        .collect();

                    if !cached_prices.is_empty() {
                        return Ok((cached_prices, true));
                    }
                }
                // No cache available either
                Ok((HashMap::new(), false))
            }
        }
    }

    /// Batch fetch all prices for known symbols in a single API call
    /// This is more efficient than making separate calls for SOL, ETH, and tokens
    pub async fn batch_fetch_all_known_prices(&mut self) -> Result<HashMap<String, f64>> {
        // Only include symbols that have feeds on Switchboard
        let known_symbols = vec![
            "SOL", "ETH", "BTC", "USDC", "USDT", "NEAR", "APT", "SUI", "AVAX", "BNB",
        ];

        self.batch_fetch_prices(
            &known_symbols
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_sol_price() {
        // This test requires SURGE_API_KEY environment variable
        if env::var("SURGE_API_KEY").is_ok() {
            let mut service = PriceService::new().expect("Failed to create price service");
            let price = service.get_single_price("SOL").await;
            assert!(price.is_ok());
            let price = price.unwrap();
            assert!(price > 0.0);
            println!("SOL price: ${}", price);
        }
    }

    #[tokio::test]
    async fn test_batch_fetch() {
        // This test requires SURGE_API_KEY environment variable
        if env::var("SURGE_API_KEY").is_ok() {
            let mut service = PriceService::new().expect("Failed to create price service");
            let symbols = vec!["BTC".to_string(), "ETH".to_string(), "SOL".to_string()];
            let prices = service.batch_fetch_prices(&symbols).await;
            assert!(prices.is_ok());
            let prices = prices.unwrap();
            println!("Prices: {:?}", prices);
            assert!(!prices.is_empty());
        }
    }

    #[test]
    fn test_price_cache_age_string() {
        let mut cache = PriceCache::new();

        // Test "just now"
        assert_eq!(cache.age_string(), "just now");

        // Test minutes
        cache.updated_at = Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(cache.age_string(), "5 minutes ago");

        // Test hours
        cache.updated_at = Utc::now() - chrono::Duration::hours(2);
        assert_eq!(cache.age_string(), "2 hours ago");

        // Test days
        cache.updated_at = Utc::now() - chrono::Duration::days(3);
        assert_eq!(cache.age_string(), "3 days ago");
    }

    #[test]
    fn test_price_cache_is_stale() {
        let mut cache = PriceCache::new();

        // Fresh cache should not be stale
        assert!(!cache.is_stale(5));

        // Old cache should be stale
        cache.updated_at = Utc::now() - chrono::Duration::minutes(10);
        assert!(cache.is_stale(5));
    }
}
