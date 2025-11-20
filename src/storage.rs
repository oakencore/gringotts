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
    Core,
    Near,
    Aptos,
    Sui,
    Starknet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BankingService {
    Mercury,
    Circle,
}

impl BankingService {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "mercury" => Ok(BankingService::Mercury),
            "circle" => Ok(BankingService::Circle),
            _ => anyhow::bail!("Unknown banking service: {}", s),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            BankingService::Mercury => "Mercury Banking",
            BankingService::Circle => "Circle",
        }
    }
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
            "core" => Ok(Chain::Core),
            "near" => Ok(Chain::Near),
            "aptos" | "apt" => Ok(Chain::Aptos),
            "sui" => Ok(Chain::Sui),
            "starknet" | "stark" => Ok(Chain::Starknet),
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
            Chain::Core => "Core",
            Chain::Near => "NEAR Protocol",
            Chain::Aptos => "Aptos",
            Chain::Sui => "Sui",
            Chain::Starknet => "Starknet",
        }
    }

    #[allow(dead_code)]
    pub fn is_evm(&self) -> bool {
        matches!(
            self,
            Chain::Ethereum
                | Chain::Polygon
                | Chain::BinanceSmartChain
                | Chain::Arbitrum
                | Chain::Optimism
                | Chain::Avalanche
                | Chain::Base
                | Chain::Core
        )
    }

    /// Get the native token symbol for this chain
    pub fn native_token_symbol(&self) -> &str {
        match self {
            Chain::Solana => "SOL",
            Chain::Ethereum => "ETH",
            Chain::Polygon => "MATIC",
            Chain::BinanceSmartChain => "BNB",
            Chain::Arbitrum => "ETH",
            Chain::Optimism => "ETH",
            Chain::Avalanche => "AVAX",
            Chain::Base => "ETH",
            Chain::Core => "CORE",
            Chain::Near => "NEAR",
            Chain::Aptos => "APT",
            Chain::Sui => "SUI",
            Chain::Starknet => "STRK",
        }
    }
}

fn default_chain() -> Chain {
    Chain::Solana
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WalletAddress {
    #[serde(default)]
    pub company: String,
    pub name: String,
    pub address: String,
    #[serde(default = "default_chain")]
    pub chain: Chain,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BankingAccount {
    #[serde(default)]
    pub company: String,
    pub name: String,
    pub account_id: String,
    pub service: BankingService,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddressBook {
    pub addresses: Vec<WalletAddress>,
    #[serde(default)]
    pub banking_accounts: Vec<BankingAccount>,
}

impl AddressBook {
    pub fn new() -> Self {
        Self {
            addresses: Vec::new(),
            banking_accounts: Vec::new(),
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
            addr.company = addr.company.trim().to_string();
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

    pub fn add_address(&mut self, company: String, name: String, address: String, chain: Option<String>) -> Result<()> {
        // Trim whitespace from inputs
        let company = company.trim().to_string();
        let name = name.trim().to_string();
        let address = address.trim().to_string();

        // Check if name already exists
        if self.addresses.iter().any(|a| a.name == name) {
            anyhow::bail!("Address with name '{}' already exists", name);
        }

        // Detect or use specified chain
        let chain = Self::detect_chain(&address, chain.as_deref())?;

        self.addresses.push(WalletAddress {
            company,
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

    pub fn add_banking_account(&mut self, company: String, name: String, account_id: String, service: String) -> Result<()> {
        // Trim whitespace from inputs
        let company = company.trim().to_string();
        let name = name.trim().to_string();
        let account_id = account_id.trim().to_string();

        // Check if name already exists in either addresses or banking accounts
        if self.addresses.iter().any(|a| a.name == name) {
            anyhow::bail!("Address with name '{}' already exists", name);
        }
        if self.banking_accounts.iter().any(|a| a.name == name) {
            anyhow::bail!("Banking account with name '{}' already exists", name);
        }

        let service = BankingService::from_str(&service)?;

        self.banking_accounts.push(BankingAccount {
            company,
            name,
            account_id,
            service,
        });
        Ok(())
    }

    pub fn remove_banking_account_by_identifier(&mut self, identifier: &str) -> Result<()> {
        let initial_len = self.banking_accounts.len();
        // Remove by name or account_id
        self.banking_accounts.retain(|a| a.name != identifier && a.account_id != identifier);

        if self.banking_accounts.len() == initial_len {
            anyhow::bail!("Banking account with name or account ID '{}' not found", identifier);
        }

        Ok(())
    }
}
