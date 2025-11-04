use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Chain {
    Solana,
    Ethereum,
    Polygon,
    BinanceSmartChain,
    Arbitrum,
    Optimism,
    Avalanche,
    Base,
}

impl Chain {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "solana" | "sol" => Ok(Chain::Solana),
            "ethereum" | "eth" => Ok(Chain::Ethereum),
            "polygon" | "matic" => Ok(Chain::Polygon),
            "bsc" | "binance" | "bnb" => Ok(Chain::BinanceSmartChain),
            "arbitrum" | "arb" => Ok(Chain::Arbitrum),
            "optimism" | "op" => Ok(Chain::Optimism),
            "avalanche" | "avax" => Ok(Chain::Avalanche),
            "base" => Ok(Chain::Base),
            _ => anyhow::bail!("Unknown chain: {}", s),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Chain::Solana => "Solana",
            Chain::Ethereum => "Ethereum",
            Chain::Polygon => "Polygon",
            Chain::BinanceSmartChain => "Binance Smart Chain",
            Chain::Arbitrum => "Arbitrum",
            Chain::Optimism => "Optimism",
            Chain::Avalanche => "Avalanche C-Chain",
            Chain::Base => "Base",
        }
    }

    pub fn is_evm(&self) -> bool {
        !matches!(self, Chain::Solana)
    }
}

fn default_chain() -> Chain {
    Chain::Solana
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WalletAddress {
    pub name: String,
    pub address: String,
    #[serde(default = "default_chain")]
    pub chain: Chain,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddressBook {
    pub addresses: Vec<WalletAddress>,
}

impl AddressBook {
    pub fn new() -> Self {
        Self {
            addresses: Vec::new(),
        }
    }

    fn detect_chain(address: &str, specified_chain: Option<&str>) -> Result<Chain> {
        // If chain is specified, use it
        if let Some(chain_str) = specified_chain {
            return Chain::from_str(chain_str);
        }

        // Auto-detect based on address format
        if address.len() == 42 && address.starts_with("0x") {
            if address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                // EVM address, default to Ethereum
                return Ok(Chain::Ethereum);
            }
        }

        // Default to Solana for base58-encoded addresses
        Ok(Chain::Solana)
    }

    pub fn load() -> Result<Self> {
        let path = Self::get_storage_path()?;

        if !path.exists() {
            return Ok(Self::new());
        }

        let content = fs::read_to_string(&path)
            .context("Failed to read address book")?;

        let mut book: AddressBook = serde_json::from_str(&content)
            .context("Failed to parse address book")?;

        // Clean up any whitespace
        for addr in &mut book.addresses {
            addr.name = addr.name.trim().to_string();
            addr.address = addr.address.trim().to_string();
            // Chain is already stored in JSON, trust it
        }

        Ok(book)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::get_storage_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .context("Failed to create storage directory")?;
        }

        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize address book")?;

        fs::write(&path, content)
            .context("Failed to write address book")?;

        Ok(())
    }

    pub fn add_address(&mut self, name: String, address: String, chain: Option<String>) -> Result<()> {
        // Trim whitespace from inputs
        let name = name.trim().to_string();
        let address = address.trim().to_string();

        // Check if name already exists
        if self.addresses.iter().any(|a| a.name == name) {
            anyhow::bail!("Address with name '{}' already exists", name);
        }

        // Detect or use specified chain
        let chain = Self::detect_chain(&address, chain.as_deref())?;

        self.addresses.push(WalletAddress {
            name,
            address,
            chain,
        });
        Ok(())
    }

    pub fn remove_by_identifier(&mut self, identifier: &str) -> Result<()> {
        let initial_len = self.addresses.len();
        // Remove by name or address
        self.addresses.retain(|a| a.name != identifier && a.address != identifier);

        if self.addresses.len() == initial_len {
            anyhow::bail!("Address with name or address '{}' not found", identifier);
        }

        Ok(())
    }

    fn get_storage_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .context("Failed to get home directory")?;

        Ok(home.join(".gringotts").join("addresses.json"))
    }
}
