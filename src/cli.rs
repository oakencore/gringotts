use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gringotts")]
#[command(about = "CLI for tracking cryptocurrency and banking account balances", long_about = None)]
#[command(after_help = "Examples:
  gringotts add -n \"My Wallet\" -a 0x742d35Cc6634C0532925a3b844Bc9e7595f5bE5B
  gringotts add -c CompanyName -n \"Hot Wallet\" -a 5FHneW46... --chain solana
  gringotts add-bank -c CompanyName -n \"Checking\" -i 87c9c4a4-... -s mercury
  gringotts list
  gringotts list -c CompanyName
  gringotts query
  gringotts query-one \"My Wallet\"
  gringotts setup-mercury -c CompanyName
  gringotts export-transactions \"Checking\" --start 2025-01-01 --end 2025-01-31
  gringotts export-transactions \"Checking\" -f json -o transactions.json")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add a blockchain address to track (Solana, Ethereum, Polygon, etc.)
    Add {
        /// Company/organization for this address
        #[arg(short, long, default_value = "")]
        company: String,

        /// Name/label for this address
        #[arg(short, long)]
        name: String,

        /// The blockchain address
        #[arg(short, long)]
        address: String,

        /// Blockchain chain (solana, ethereum, polygon, bsc, arbitrum, optimism, avalanche, base, core, near, aptos, sui, starknet)
        /// If not specified, chain is auto-detected based on address format
        #[arg(long)]
        chain: Option<String>,
    },

    /// Add a banking account to track (Mercury)
    AddBank {
        /// Company/organization for this account
        #[arg(short, long, default_value = "")]
        company: String,

        /// Name/label for this account
        #[arg(short, long)]
        name: String,

        /// The account ID
        #[arg(short = 'i', long)]
        account_id: String,

        /// Banking service (mercury)
        #[arg(short, long)]
        service: String,
    },

    /// List tracked addresses and accounts (optionally filter by company)
    List {
        /// Filter by company name (case-insensitive, partial match)
        #[arg(short, long)]
        company: Option<String>,
    },

    /// Remove an address or banking account by name
    Remove {
        /// Name to remove
        identifier: String,
    },

    /// Query balances for all tracked addresses and banking accounts
    Query {
        /// Optional RPC URL (defaults to mainnet)
        #[arg(short, long)]
        rpc_url: Option<String>,

        /// Skip price lookups (faster, no USD values)
        #[arg(long)]
        no_prices: bool,
    },

    /// Query balances for a specific address or banking account by name
    QueryOne {
        /// Name of the address or account to query
        name: String,

        /// Optional RPC URL (defaults to mainnet)
        #[arg(short, long)]
        rpc_url: Option<String>,

        /// Skip price lookups (faster, no USD values)
        #[arg(long)]
        no_prices: bool,
    },

    /// List all accounts from Mercury
    ListMercuryAccounts,

    /// Set up Mercury integration - list accounts and add to tracking
    SetupMercury {
        /// Company/organization for these accounts
        #[arg(short, long, default_value = "")]
        company: String,
    },

    /// Export transactions from a Mercury banking account
    ExportTransactions {
        /// Name of the Mercury account to export from
        name: String,

        /// Output format (csv or json)
        #[arg(short, long, default_value = "csv")]
        format: String,

        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,

        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,

        /// Output file path (defaults to stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
}
