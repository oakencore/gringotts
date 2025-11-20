use anyhow::{Context, Result};
use i_am_surging::SurgeClient;
use std::collections::HashMap;
use std::env;

/// PriceService using Switchboard Surge for cryptocurrency prices
/// Provides efficient price queries for 2,266+ trading pairs
pub struct PriceService {
    surge_client: SurgeClient,
}

impl PriceService {
    pub fn new() -> Result<Self> {
        // Get the API key from environment
        let api_key = Self::get_api_key().unwrap_or_else(|_| {
            eprintln!("Warning: SURGE_API_KEY not set. Price queries will fail.");
            eprintln!("Get your API key (Solana wallet address) from https://switchboard.xyz");
            String::new()
        });

        // Create the Surge client
        let surge_client = SurgeClient::new(&api_key)
            .context("Failed to create SurgeClient. Ensure feedIds.json is present.")?;

        Ok(Self { surge_client })
    }

    fn get_api_key() -> Result<String> {
        env::var("SURGE_API_KEY")
            .context("SURGE_API_KEY environment variable not set. Get your API key (Solana wallet address) from https://switchboard.xyz")
    }

    /// Get price for a single token symbol (e.g., "SOL", "ETH", "BTC")
    /// Returns price in USD
    async fn get_single_price(&self, symbol: &str) -> Result<f64> {
        // Convert symbol to trading pair format (e.g., "SOL" -> "SOL/USD")
        let trading_pair = format!("{}/USD", symbol);

        match self.surge_client.get_price(&trading_pair).await {
            Ok(price_data) => Ok(price_data.value),
            Err(e) => {
                // Try alternate quote currencies if USD fails
                for quote in &["USDT", "USDC"] {
                    let alt_pair = format!("{}/{}", symbol, quote);
                    if let Ok(price_data) = self.surge_client.get_price(&alt_pair).await {
                        return Ok(price_data.value);
                    }
                }
                anyhow::bail!("Failed to get price for {}: {}", symbol, e)
            }
        }
    }

    /// Fetch USD prices for multiple token mints (Solana-specific)
    /// This maintains backward compatibility with existing Solana code
    pub async fn get_prices(&self, mint_addresses: &[String]) -> Result<HashMap<String, f64>> {
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
                    eprintln!("Warning: Failed to fetch price for {} ({}): {}", symbol, mint, e);
                }
            }
        }

        Ok(prices)
    }

    /// Get ETH price in USD
    pub async fn get_eth_price(&self) -> Result<f64> {
        self.get_single_price("ETH").await
    }

    /// Get prices for ERC20 tokens (USDC, USDT, DAI, etc.)
    pub async fn get_erc20_prices(&self, symbols: &[String]) -> Result<HashMap<String, f64>> {
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
    pub async fn batch_fetch_prices(&self, symbols: &[String]) -> Result<HashMap<String, f64>> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        // Convert symbols to trading pairs
        let trading_pairs: Vec<String> = symbols
            .iter()
            .map(|s| format!("{}/USD", s))
            .collect();

        let trading_pair_refs: Vec<&str> = trading_pairs.iter().map(|s| s.as_str()).collect();

        match self.surge_client.get_multiple_prices(&trading_pair_refs).await {
            Ok(price_list) => {
                let mut prices = HashMap::new();

                for price_data in price_list {
                    // Extract symbol from trading pair (e.g., "BTC/USD" -> "BTC")
                    if let Some(base_symbol) = price_data.symbol.split('/').next() {
                        prices.insert(base_symbol.to_string(), price_data.value);
                    }
                }

                Ok(prices)
            }
            Err(e) => {
                eprintln!("Warning: Batch fetch failed: {}", e);
                // Fall back to individual queries
                let mut prices = HashMap::new();
                for symbol in symbols {
                    if let Ok(price) = self.get_single_price(symbol).await {
                        prices.insert(symbol.clone(), price);
                    }
                }
                Ok(prices)
            }
        }
    }

    /// Batch fetch all prices for known symbols in a single API call
    /// This is more efficient than making separate calls for SOL, ETH, and tokens
    pub async fn batch_fetch_all_known_prices(&self) -> Result<HashMap<String, f64>> {
        let known_symbols = vec![
            "SOL", "ETH", "BTC", "USDC", "USDT", "DAI",
            "MSOL", "stSOL", "SWTCH", "JTO", "RAT",
            "NEAR", "APT", "SUI", "AVAX", "MATIC", "BNB"
        ];

        self.batch_fetch_prices(&known_symbols.iter().map(|s| s.to_string()).collect::<Vec<_>>()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_sol_price() {
        // This test requires SURGE_API_KEY environment variable
        if env::var("SURGE_API_KEY").is_ok() {
            let service = PriceService::new();
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
            let service = PriceService::new();
            let symbols = vec!["BTC".to_string(), "ETH".to_string(), "SOL".to_string()];
            let prices = service.batch_fetch_prices(&symbols).await;
            assert!(prices.is_ok());
            let prices = prices.unwrap();
            println!("Prices: {:?}", prices);
            assert!(!prices.is_empty());
        }
    }
}
