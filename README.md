# Gringotts

Multi-chain cryptocurrency portfolio tracker with banking integration. Track balances across 13+ blockchains and multiple banking accounts with real-time USD pricing.

## Features

- **Multi-chain support**: Solana, Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Avalanche, Core, NEAR, Aptos, Sui, Starknet
- **Banking integration**: Mercury, Circle
- **Real-time pricing**: USD values via Switchboard Surge
- **Portfolio aggregation**: Group assets by company/organization
- **Web interface**: HTMX-powered dashboard
- **Transaction export**: CSV/JSON export for banking transactions
- **Auto-detection**: Automatically detects chain from address format
- **Premium RPC support**: Auto-detects Helius (Solana) and Alchemy (EVM) API keys

### Roadmap

- Add support for Dakota.xyz, altitude.squads.xyz, Stripe
- Add support for additional price providers (redundancy)
- Add support for exchange accounts (Coinbase, Binance, etc.)

## Setup

### Prerequisites

**Node.js 18+** is required for price fetching (used by the Switchboard Surge integration).

```bash
npm install  # Install dependencies for price feeds
```

### API Keys

Gringotts supports environment variables via `.env` file or shell exports:

```bash
# Create .env file in project root (recommended)
cat > .env <<EOF
# Price Feeds (required for USD values)
SURGE_API_KEY="your-wallet-address"           # Switchboard Surge (preferred)

# RPC Providers (optional - improves rate limits)
HELIUS_API_KEY="your-helius-key"              # Solana premium RPC
ALCHEMY_API_KEY="your-alchemy-key"            # EVM chains premium RPC

# Banking Integrations (optional)
MERCURY_API_KEY="your-mercury-key"            # Mercury banking
CIRCLE_API_KEY="your-circle-key"              # Circle banking
EOF
```

#### Required API Keys

**Switchboard Surge** (for USD pricing): Subscribe at https://explorer.switchboardlabs.xyz/subscriptions and use your Solana wallet address as the API key.

#### Optional API Keys

**Helius** (Solana RPC): Get a free API key from https://helius.dev for better Solana rate limits

**Alchemy** (EVM RPC): Get a free API key from https://alchemy.com for Ethereum, Polygon, Arbitrum, Optimism, and Base

**Mercury** (Banking): Get an API key from https://mercury.com/settings/tokens

**Circle** (Banking): Get an API key from https://developers.circle.com

### Build

```bash
cargo build --release
# Binary: target/release/gringotts

# Or install globally:
cargo install --path .
```

### Testing

Gringotts has comprehensive test coverage:

```bash
# Run all tests
cargo test

# Run tests with price service integration (requires SURGE_API_KEY)
SURGE_API_KEY="your-wallet-address" cargo test

# Run specific test
cargo test test_extract_token_symbols
```

Test coverage includes:

- Token symbol extraction across multiple chains
- Portfolio aggregation and asset accumulation
- Price enrichment with caching
- Storage persistence (save/load)
- Helper functions and utilities

### Help

```bash
gringotts --help              # Show all commands
gringotts add --help          # Show options for a specific command
```

## Usage

### Cryptocurrency Addresses

```bash
# Add blockchain addresses
gringotts add --name "Wallet" --address <address>
gringotts add --name "Wallet" --address <address> --chain solana
gringotts add --name "Wallet" --address <address> --chain aptos --company "CompanyName"

# List tracked addresses and accounts
gringotts list

# Query all balances
gringotts query

# Query specific wallet or account
gringotts query-one "Wallet"

# Remove address or account (by name)
gringotts remove "Wallet"
```

### Banking Accounts

#### Mercury Setup Wizard

```bash
# Automatically discover and add all Mercury accounts
gringotts setup-mercury --company "CompanyName"

# List Mercury accounts
gringotts list-mercury-accounts
```

#### Manual Banking Account Management

```bash
# Add Mercury account manually
gringotts add-bank --name "Operating" --account-id <mercury-account-id> --service mercury --company "CompanyName"

# Add Circle account
gringotts add-bank --name "Circle USD" --account-id <circle-account-id> --service circle

# Query all balances (includes all banking accounts)
gringotts query

# Query specific banking account
gringotts query-one "Operating"

# Remove banking account
gringotts remove "Operating"
```

#### Transaction Export

Export banking transactions to CSV or JSON:

```bash
# Export to CSV (default)
gringotts export-transactions "Operating Account"

# Export to JSON
gringotts export-transactions "Operating Account" --format json

# Export with date range
gringotts export-transactions "Operating Account" --start 2025-01-01 --end 2025-01-31

# Export to file
gringotts export-transactions "Operating Account" --output transactions.csv
```

### Organisation

You can use the `--company` flag to group wallets by 'organisation'. This can be useful if you want to subcategorise addresses in addition to giving them names.

```bash
gringotts add --name "Hot Wallet" --address <address> --company "CompanyA"
gringotts add --name "Cold Storage" --address <address> --company "CompanyB"
```

Portfolio summary displays assets grouped by the company flag.

### Web Interface

Launch a web dashboard to view your portfolio:

```bash
# Start web server (default port 3000)
gringotts serve

# Custom port
gringotts serve --port 8080
```

Access the dashboard at `http://localhost:3000` for an interactive HTMX-powered interface.

### Supported Chains

**Layer 1**
- Solana: `solana`, `sol`
- Ethereum: `ethereum`, `eth` (This is the default for 0x addresses currently)
- NEAR: `near`
- Aptos: `aptos`, `apt`
- Sui: `sui`
- Core: `core`

**Layer 2 / EVM**
- Polygon: `polygon`, `matic`
- BSC: `bsc`, `binance`, `bnb`
- Arbitrum: `arbitrum`, `arb`
- Optimism: `optimism`, `op`
- Avalanche: `avalanche`, `avax`
- Base: `base`
- Starknet: `starknet`, `stark`


### RPC Configuration

Gringotts automatically uses premium RPC endpoints when API keys are detected:

**Auto-detected RPC providers:**

- **Helius** (Solana): Automatically used when `HELIUS_API_KEY` is set
- **Alchemy** (EVM): Automatically used when `ALCHEMY_API_KEY` is set for Ethereum, Polygon, Arbitrum, Optimism, and Base

**Fallback behavior:**

- Without API keys, Gringotts falls back to free public RPC endpoints
- You'll see a warning message with links to get free API keys

**Override RPC endpoints:**

You can override the RPC endpoint for any query:

```bash
# Solana custom RPC
gringotts query --rpc-url https://api.mainnet-beta.solana.com
gringotts query --rpc-url https://api.devnet.solana.com

# EVM custom RPC
gringotts query --rpc-url https://eth.llamarpc.com
gringotts query --rpc-url https://polygon-rpc.com
gringotts query --rpc-url https://arb1.arbitrum.io/rpc
```

### Performance Optimization

Skip USD price lookups for faster queries:

```bash
# Query without fetching prices
gringotts query --no-prices

# Useful for quick balance checks or when price service is down
gringotts query-one "Wallet" --no-prices
```
## Storage

Addresses and banking accounts are stored in: `~/.gringotts/addresses.json`

## Architecture

### Core Modules

- **cli.rs** - Command-line interface definitions using Clap
- **main.rs** - Command dispatch, orchestration, and business logic
- **storage.rs** - Persistence layer for addresses and accounts
- **ui.rs** - Terminal rendering with box-drawing characters

### Blockchain Clients

Each blockchain module implements `get_balances(address)` returning chain-specific `AccountBalances`:

- **solana.rs** - SOL + SPL tokens via `solana-client`
- **evm.rs** - Ethereum and EVM-compatible chains (Polygon, Arbitrum, Optimism, Base, BSC, Avalanche, Core)
- **aptos.rs** - Aptos native token via REST API
- **sui.rs** - Sui native token via JSON-RPC
- **near.rs** - NEAR native token via JSON-RPC
- **starknet.rs** - Starknet ETH via JSON-RPC

### Banking Integrations

- **mercury.rs** - Mercury API client for balances and transactions
- **circle.rs** - Circle API client for USD balances

### Price Service

- **price.rs** - Switchboard Surge integration with rate limiting and caching
- Uses `i-am-surging` crate which wraps Node.js dependencies

### Web Server

- **web.rs** - Axum-based server with HTMX frontend

### Key Design Patterns

- **PriceEnrichable trait**: Unified interface for enriching balance data with USD prices
- **Chain auto-detection**: Automatically detects blockchain from address format (0x = Ethereum, base58 = Solana)
- **Premium RPC auto-detection**: Automatically uses Helius/Alchemy when API keys are present
- **Portfolio aggregation**: Groups assets by company tag for organizational reporting

## Development

### Running from Source

```bash
# With .env file
cargo run -- query

# With inline environment variables
SURGE_API_KEY="..." cargo run -- query

# Debug logging
RUST_LOG=debug cargo run -- query
```

### Contributing

Contributions are welcome! The codebase has comprehensive test coverage - please add tests for new features.

## License

MIT
