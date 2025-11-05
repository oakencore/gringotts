use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;

// CoinMarketCap v1 API - compatible with free tier
// Docs: https://coinmarketcap.com/api/documentation/v1/
const CMC_QUOTE_API: &str = "https://pro-api.coinmarketcap.com/v1/cryptocurrency/quotes/latest";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const SOL_SYMBOL: &str = "SOL";
const ETH_SYMBOL: &str = "ETH";

// Lazy static map for Solana token mint addresses to symbols
static SOLANA_MINT_TO_SYMBOL: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::with_capacity(8);
    m.insert("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", "USDC");
    m.insert("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", "USDT");
    m.insert("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So", "MSOL");
    m.insert("7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj", "stSOL");
    m.insert("SW1TCHLmRGTfW5xZknqQdpdarB8PD95sJYWpNp9TbFx", "SWTCH");
    m.insert("jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL", "JTO");
    m.insert("GP2vH92rxSHWm2VzttZBZdeFnv9LyfFJYvPrAet6pump", "RAT");
    m
});

#[derive(Debug, Deserialize)]
struct CmcResponse {
    data: HashMap<String, CmcQuoteData>,
}

#[derive(Debug, Deserialize)]
struct CmcQuoteData {
    quote: HashMap<String, CmcPriceData>,
}

#[derive(Debug, Deserialize)]
struct CmcPriceData {
    price: f64,
}

pub struct PriceService {
    client: reqwest::Client,
}

impl PriceService {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { client }
    }

    fn get_api_key() -> Result<String> {
        env::var("COINMARKETCAP_API_KEY")
            .context("COINMARKETCAP_API_KEY environment variable not set. Get your free API key at https://coinmarketcap.com/api/")
    }

    /// Fetch USD prices for multiple token mints
    pub async fn get_prices(&self, mint_addresses: &[String]) -> Result<HashMap<String, f64>> {
        if mint_addresses.is_empty() {
            return Ok(HashMap::new());
        }

        let api_key = Self::get_api_key()?;
        let mut prices = HashMap::new();

        // Separate SOL from other tokens
        let (sol_mints, token_mints): (Vec<_>, Vec<_>) = mint_addresses
            .iter()
            .partition(|addr| *addr == SOL_MINT);

        // Fetch SOL price if needed
        if !sol_mints.is_empty() {
            match self.fetch_sol_price(&api_key).await {
                Ok(price) => {
                    prices.insert(SOL_MINT.to_string(), price);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to fetch SOL price: {}", e);
                }
            }
        }

        // Fetch SPL token prices from CoinMarketCap
        if !token_mints.is_empty() {
            match self.fetch_token_prices(&api_key, &token_mints).await {
                Ok(token_prices) => {
                    prices.extend(token_prices);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to fetch token prices: {}", e);
                }
            }
        }

        Ok(prices)
    }

    async fn fetch_sol_price(&self, api_key: &str) -> Result<f64> {
        let url = format!("{}?symbol={}&convert=USD", CMC_QUOTE_API, SOL_SYMBOL);

        eprintln!("Fetching SOL price from: {}", url);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to fetch SOL price from CoinMarketCap")?;

        eprintln!("Response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            eprintln!("Error response: {}", error_text);
            anyhow::bail!("CoinMarketCap API returned status with error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse CoinMarketCap response")?;

        // Extract SOL price (v1 API structure)
        let sol_data = cmc_response
            .data
            .get(SOL_SYMBOL)
            .ok_or_else(|| anyhow::anyhow!("SOL data not found in response"))?;

        let usd_quote = sol_data
            .quote
            .get("USD")
            .ok_or_else(|| anyhow::anyhow!("USD quote not found"))?;

        Ok(usd_quote.price)
    }

    async fn fetch_token_prices(&self, api_key: &str, mint_addresses: &[&String]) -> Result<HashMap<String, f64>> {
        // Note: CoinMarketCap free tier has limited support for querying by contract address
        // This is a best-effort approach - tokens may not have prices if they're not well-known

        let mut symbols_to_query = Vec::new();
        let mut symbol_to_address: HashMap<&str, String> = HashMap::new();

        for address in mint_addresses {
            if let Some(&symbol) = SOLANA_MINT_TO_SYMBOL.get(address.as_str()) {
                symbols_to_query.push(symbol);
                symbol_to_address.insert(symbol, address.to_string());
            }
        }

        if symbols_to_query.is_empty() {
            eprintln!("No known tokens to query prices for");
            return Ok(HashMap::new());
        }

        let symbols = symbols_to_query.join(",");
        let url = format!(
            "{}?symbol={}&convert=USD",
            CMC_QUOTE_API, symbols
        );

        eprintln!("Fetching token prices for symbols: {}", symbols);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to fetch token prices from CoinMarketCap")?;

        eprintln!("Token response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            eprintln!("Token error response: {}", error_text);
            anyhow::bail!("CoinMarketCap API returned status with error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse token prices response")?;

        let mut prices = HashMap::new();
        // v1 API returns data[symbol] directly, not data[symbol][0]
        for (symbol, quote_data) in cmc_response.data {
            if let Some(usd_quote) = quote_data.quote.get("USD") {
                // Map back to the address
                if let Some(address) = symbol_to_address.get(symbol.as_str()) {
                    prices.insert(address.clone(), usd_quote.price);
                }
            }
        }

        Ok(prices)
    }

    /// Get ETH price in USD
    pub async fn get_eth_price(&self) -> Result<f64> {
        let api_key = Self::get_api_key()?;
        let url = format!("{}?symbol={}&convert=USD", CMC_QUOTE_API, ETH_SYMBOL);

        eprintln!("Fetching ETH price from: {}", url);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to fetch ETH price from CoinMarketCap")?;

        eprintln!("Response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            eprintln!("Error response: {}", error_text);
            anyhow::bail!("CoinMarketCap API returned status with error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse CoinMarketCap response")?;

        // Extract ETH price
        let eth_data = cmc_response
            .data
            .get(ETH_SYMBOL)
            .ok_or_else(|| anyhow::anyhow!("ETH data not found in response"))?;

        let usd_quote = eth_data
            .quote
            .get("USD")
            .ok_or_else(|| anyhow::anyhow!("USD quote not found"))?;

        Ok(usd_quote.price)
    }

    /// Get prices for ERC20 tokens (USDC, USDT, DAI, etc.)
    pub async fn get_erc20_prices(&self, symbols: &[String]) -> Result<HashMap<String, f64>> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        let api_key = Self::get_api_key()?;
        let symbols_str = symbols.join(",");
        let url = format!("{}?symbol={}&convert=USD", CMC_QUOTE_API, symbols_str);

        eprintln!("Fetching ERC20 token prices for: {}", symbols_str);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to fetch ERC20 prices from CoinMarketCap")?;

        eprintln!("Response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            eprintln!("Error response: {}", error_text);
            anyhow::bail!("CoinMarketCap API returned status with error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse CoinMarketCap response")?;

        let mut prices = HashMap::new();
        for (symbol, quote_data) in cmc_response.data {
            if let Some(usd_quote) = quote_data.quote.get("USD") {
                prices.insert(symbol, usd_quote.price);
            }
        }

        Ok(prices)
    }

    /// Batch fetch prices for a specific list of symbols
    pub async fn batch_fetch_prices(&self, symbols: &[String]) -> Result<HashMap<String, f64>> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        let api_key = Self::get_api_key()?;
        let symbols_str = symbols.join(",");
        let url = format!("{}?symbol={}&convert=USD", CMC_QUOTE_API, symbols_str);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to batch fetch prices from CoinMarketCap")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            anyhow::bail!("CoinMarketCap API returned error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse batch fetch response")?;

        let mut prices = HashMap::new();
        for (symbol, quote_data) in cmc_response.data {
            if let Some(usd_quote) = quote_data.quote.get("USD") {
                prices.insert(symbol, usd_quote.price);
            }
        }

        Ok(prices)
    }

    /// Batch fetch all prices for known symbols in a single API call
    /// This is more efficient than making separate calls for SOL, ETH, and tokens
    pub async fn batch_fetch_all_known_prices(&self) -> Result<HashMap<String, f64>> {
        let api_key = Self::get_api_key()?;

        // Collect all known symbols (use HashSet to deduplicate)
        let mut symbol_set = std::collections::HashSet::new();
        symbol_set.insert(SOL_SYMBOL);
        symbol_set.insert(ETH_SYMBOL);

        // Add all Solana token symbols
        for &symbol in SOLANA_MINT_TO_SYMBOL.values() {
            symbol_set.insert(symbol);
        }

        // Add common ERC20 token symbols
        symbol_set.insert("USDC");
        symbol_set.insert("USDT");
        symbol_set.insert("DAI");

        let symbols: Vec<&str> = symbol_set.into_iter().collect();
        let symbols_str = symbols.join(",");
        let url = format!("{}?symbol={}&convert=USD", CMC_QUOTE_API, symbols_str);

        eprintln!("Batch fetching prices for all known symbols: {}", symbols_str);

        let response = self
            .client
            .get(&url)
            .header("X-CMC_PRO_API_KEY", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to batch fetch prices from CoinMarketCap")?;

        eprintln!("Batch fetch response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Could not read response".to_string());
            eprintln!("Batch fetch error response: {}", error_text);
            anyhow::bail!("CoinMarketCap API returned error: {}", error_text);
        }

        let cmc_response: CmcResponse = response
            .json()
            .await
            .context("Failed to parse batch fetch response")?;

        let mut prices = HashMap::new();

        // Map symbols back to their corresponding keys (for SOL, map to mint address)
        for (symbol, quote_data) in cmc_response.data {
            if let Some(usd_quote) = quote_data.quote.get("USD") {
                let price = usd_quote.price;

                // Store symbol-based prices
                prices.insert(symbol.clone(), price);

                // For SOL, also store by mint address for compatibility
                if symbol.as_str() == SOL_SYMBOL {
                    prices.insert(SOL_MINT.to_string(), price);
                }

                // For Solana tokens, map symbol back to mint address
                for (&mint, &token_symbol) in SOLANA_MINT_TO_SYMBOL.iter() {
                    if token_symbol == symbol.as_str() {
                        prices.insert(mint.to_string(), price);
                    }
                }
            }
        }

        Ok(prices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_sol_price() {
        let service = PriceService::new();
        let mints = vec![SOL_MINT.to_string()];
        let prices = service.get_prices(&mints).await;
        assert!(prices.is_ok());
        let prices = prices.unwrap();
        let price = prices.get(SOL_MINT).unwrap();
        assert!(*price > 0.0);
        println!("SOL price: ${}", price);
    }

    #[tokio::test]
    async fn test_get_multiple_prices() {
        let service = PriceService::new();
        let mints = vec![
            SOL_MINT.to_string(), // SOL
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(), // USDC
        ];
        let prices = service.get_prices(&mints).await;
        assert!(prices.is_ok());
        let prices = prices.unwrap();
        println!("Prices: {:?}", prices);
        assert!(!prices.is_empty());
    }
}
