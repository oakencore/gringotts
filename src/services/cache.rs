use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Cache entry with timestamp for staleness tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry<T> {
    pub data: T,
    pub timestamp: u64,
}

impl<T> CacheEntry<T> {
    pub fn new(data: T) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self { data, timestamp }
    }

    pub fn age_secs(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.timestamp)
    }

    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        self.age_secs() > max_age_secs
    }
}

/// Cached balance data for a wallet or account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedBalance {
    pub name: String,
    pub address_or_id: String,
    pub chain_or_service: String,
    pub native_symbol: String,
    pub native_balance: f64,
    pub native_usd_value: Option<f64>,
    pub tokens: Vec<CachedToken>,
    pub total_usd_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedToken {
    pub symbol: String,
    pub balance: f64,
    pub usd_value: Option<f64>,
}

/// Cached price data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPrices {
    pub prices: HashMap<String, f64>,
}

/// The main cache structure persisted to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceCache {
    /// Cached balances keyed by account name
    pub balances: HashMap<String, CacheEntry<CachedBalance>>,
    /// Cached prices keyed by symbol
    pub prices: CacheEntry<HashMap<String, f64>>,
    /// Last full refresh timestamp
    pub last_full_refresh: Option<u64>,
}

impl Default for BalanceCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BalanceCache {
    pub fn new() -> Self {
        Self {
            balances: HashMap::new(),
            prices: CacheEntry::new(HashMap::new()),
            last_full_refresh: None,
        }
    }

    fn get_cache_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Failed to get home directory")?;
        Ok(home.join(".gringotts").join("cache.json"))
    }

    /// Load cache from disk
    pub fn load() -> Result<Self> {
        let path = Self::get_cache_path()?;

        if !path.exists() {
            return Ok(Self::new());
        }

        let content = fs::read_to_string(&path).context("Failed to read cache file")?;

        let cache: BalanceCache =
            serde_json::from_str(&content).context("Failed to parse cache file")?;

        Ok(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::get_cache_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create cache directory")?;
        }

        let content = serde_json::to_string_pretty(self).context("Failed to serialize cache")?;

        fs::write(&path, content).context("Failed to write cache file")?;

        Ok(())
    }

    /// Update a single balance entry
    pub fn set_balance(&mut self, name: &str, balance: CachedBalance) {
        self.balances
            .insert(name.to_string(), CacheEntry::new(balance));
    }

    /// Move a cached balance to a new key after an account rename. Keeps the
    /// entry's timestamp (a rename is not a refresh) and updates the embedded
    /// name. No-op if the old key is missing or the names are equal.
    pub fn rename_balance(&mut self, old_name: &str, new_name: &str) {
        if old_name == new_name {
            return;
        }
        if let Some(mut entry) = self.balances.remove(old_name) {
            entry.data.name = new_name.to_string();
            self.balances.insert(new_name.to_string(), entry);
        }
    }

    /// Get a cached balance if not stale
    pub fn get_balance(&self, name: &str, max_age_secs: u64) -> Option<&CachedBalance> {
        self.balances.get(name).and_then(|entry| {
            if entry.is_stale(max_age_secs) {
                None
            } else {
                Some(&entry.data)
            }
        })
    }

    /// Return the cached balance regardless of age. Used by surfaces
    /// (like the dashboard) that prefer to render stale data over
    /// rendering nothing, signaling freshness via a separate timestamp.
    pub fn get_balance_unchecked(&self, name: &str) -> Option<&CachedBalance> {
        self.balances.get(name).map(|e| &e.data)
    }

    /// Update prices
    pub fn set_prices(&mut self, prices: HashMap<String, f64>) {
        self.prices = CacheEntry::new(prices);
    }

    /// Get cached prices if not stale
    pub fn get_prices(&self, max_age_secs: u64) -> Option<&HashMap<String, f64>> {
        if self.prices.is_stale(max_age_secs) {
            None
        } else {
            Some(&self.prices.data)
        }
    }

    /// Mark a full refresh
    pub fn mark_full_refresh(&mut self) {
        self.last_full_refresh = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
    }

    /// Get human-readable age of cache
    pub fn cache_age_string(&self) -> String {
        match self.last_full_refresh {
            Some(ts) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let age = now.saturating_sub(ts);
                if age < 60 {
                    format!("{}s ago", age)
                } else if age < 3600 {
                    format!("{}m ago", age / 60)
                } else if age < 86400 {
                    format!("{}h ago", age / 3600)
                } else {
                    format!("{}d ago", age / 86400)
                }
            }
            None => "never".to_string(),
        }
    }
}

/// Thread-safe cache wrapper for use in async contexts
#[derive(Clone)]
pub struct SharedCache {
    inner: Arc<RwLock<BalanceCache>>,
}

impl SharedCache {
    /// Create a new shared cache, loading from disk if available
    pub fn new() -> Self {
        let cache = BalanceCache::load().unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load cache: {}. Starting fresh.", e);
            BalanceCache::new()
        });
        Self {
            inner: Arc::new(RwLock::new(cache)),
        }
    }

    /// Get read access to the cache
    pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, BalanceCache> {
        self.inner.read().await
    }

    /// Get write access to the cache
    pub async fn write(&self) -> tokio::sync::RwLockWriteGuard<'_, BalanceCache> {
        self.inner.write().await
    }

    /// Save the cache to disk
    pub async fn persist(&self) -> Result<()> {
        let cache = self.inner.read().await;
        cache.save()
    }

    /// Update a balance and persist
    pub async fn update_balance(&self, name: &str, balance: CachedBalance) -> Result<()> {
        {
            let mut cache = self.inner.write().await;
            cache.set_balance(name, balance);
        }
        self.persist().await
    }

    /// Re-key a cached balance after an account rename, then persist.
    pub async fn rename_balance(&self, old_name: &str, new_name: &str) -> Result<()> {
        {
            let mut cache = self.inner.write().await;
            cache.rename_balance(old_name, new_name);
        }
        self.persist().await
    }

    /// Update prices and persist
    pub async fn update_prices(&self, prices: HashMap<String, f64>) -> Result<()> {
        {
            let mut cache = self.inner.write().await;
            cache.set_prices(prices);
        }
        self.persist().await
    }

    /// Mark full refresh and persist
    pub async fn mark_refresh(&self) -> Result<()> {
        {
            let mut cache = self.inner.write().await;
            cache.mark_full_refresh();
        }
        self.persist().await
    }
}

impl Default for SharedCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_age() {
        let entry = CacheEntry::new("test data");
        assert!(entry.age_secs() < 2);
        assert!(!entry.is_stale(10));
    }

    #[test]
    fn test_cache_entry_staleness() {
        let mut entry = CacheEntry::new("test data");
        // Manually set old timestamp
        entry.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 100;

        assert!(entry.is_stale(60));
        assert!(!entry.is_stale(120));
    }

    #[test]
    fn test_balance_cache_operations() {
        let mut cache = BalanceCache::new();

        let balance = CachedBalance {
            name: "Test Wallet".to_string(),
            address_or_id: "test123".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 10.0,
            native_usd_value: Some(1000.0),
            tokens: vec![],
            total_usd_value: Some(1000.0),
        };

        cache.set_balance("Test Wallet", balance.clone());

        // Should retrieve when not stale
        let retrieved = cache.get_balance("Test Wallet", 3600);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().native_balance, 10.0);

        // Manually make the entry stale by backdating the timestamp
        if let Some(entry) = cache.balances.get_mut("Test Wallet") {
            entry.timestamp = entry.timestamp.saturating_sub(100); // 100 seconds old
        }
        let stale = cache.get_balance("Test Wallet", 50); // 50 second max age
        assert!(stale.is_none());
    }

    #[test]
    fn test_price_cache_operations() {
        let mut cache = BalanceCache::new();

        let mut prices = HashMap::new();
        prices.insert("SOL".to_string(), 100.0);
        prices.insert("ETH".to_string(), 3000.0);

        cache.set_prices(prices);

        let retrieved = cache.get_prices(3600);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().get("SOL"), Some(&100.0));
    }

    #[test]
    fn test_cache_age_string() {
        let mut cache = BalanceCache::new();

        // No refresh yet
        assert_eq!(cache.cache_age_string(), "never");

        // Mark refresh
        cache.mark_full_refresh();
        let age_str = cache.cache_age_string();
        assert!(age_str.contains("s ago") || age_str.contains("0s"));
    }

    #[test]
    fn test_rename_balance_rekeys_entry() {
        let mut cache = BalanceCache::new();
        let balance = CachedBalance {
            name: "Old Name".to_string(),
            address_or_id: "test123".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 10.0,
            native_usd_value: Some(1000.0),
            tokens: vec![],
            total_usd_value: Some(1000.0),
        };
        cache.set_balance("Old Name", balance);
        let original_ts = cache.balances["Old Name"].timestamp;

        cache.rename_balance("Old Name", "New Name");

        assert!(cache.balances.get("Old Name").is_none());
        let entry = cache.balances.get("New Name").expect("entry re-keyed");
        assert_eq!(entry.data.name, "New Name");
        assert_eq!(entry.data.native_balance, 10.0);
        // Rename must not reset freshness.
        assert_eq!(entry.timestamp, original_ts);

        // Renaming a missing key is a silent no-op.
        cache.rename_balance("Ghost", "Anything");
        assert!(cache.balances.get("Anything").is_none());
    }
}
