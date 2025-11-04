use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gringotts")]
#[command(about = "A CLI tool for querying cryptocurrency account balances", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add a blockchain address to track (Solana, Ethereum, Polygon, etc.)
    Add {
        /// Name/label for this address
        #[arg(short, long)]
        name: String,

        /// The blockchain address
        #[arg(short, long)]
        address: String,

        /// Blockchain chain (solana, ethereum, polygon, bsc, arbitrum, optimism, avalanche, base)
        /// If not specified, chain is auto-detected based on address format
        #[arg(short, long)]
        chain: Option<String>,
    },

    /// List all tracked addresses
    List,

    /// Remove an address by name or address
    Remove {
        /// Name or address to remove
        identifier: String,
    },

    /// Query balances for all tracked addresses
    Query {
        /// Optional RPC URL (defaults to mainnet)
        #[arg(short, long)]
        rpc_url: Option<String>,
    },

    /// Query balances for a specific address by name
    QueryOne {
        /// Name of the address to query
        name: String,

        /// Optional RPC URL (defaults to mainnet)
        #[arg(short, long)]
        rpc_url: Option<String>,
    },
}
