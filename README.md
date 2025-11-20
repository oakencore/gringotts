# Gringotts

CLI interface for tracking cryptocurrency balances across multiple blockchains and banking accounts.

Please note that this is a work in progress. 
### Todo:
- Add Support for Dakota.xyz, altitude.squads.xyz, Stripe
- Add support for additional price providers (redundancy)
- Add support for additional RPC providers (redundancy)
- Add support for exchange accounts

## Setup

### Prerequisites

**Node.js 18+** is required for price fetching (used by the Switchboard Surge integration).

```bash
npm install  # Install dependencies for price feeds
```

### API Keys

#### Price Feeds (choose one)

**Switchboard Surge** (preferred): Subscribe at https://explorer.switchboardlabs.xyz/subscriptions and use your Solana wallet address as the API key.

```bash
export SURGE_API_KEY="your-wallet-address-here"
```

**CoinMarketCap** (alternative): Get a free API key from https://coinmarketcap.com/api/

```bash
export COINMARKETCAP_API_KEY="your-coinmarketcap-api-key-here"
```

#### Banking Integrations (optional)

**Mercury**: Get an API key from https://mercury.com/settings/tokens

```bash
export MERCURY_API_KEY="your-mercury-api-key-here"
```

### Build

```bash
cargo build --release
# Binary: target/release/gringotts

# Or install:
cargo install --path .
```

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

#### Mercury

```bash
# Add Mercury banking account
gringotts add-bank --name "Operating Account" --account-id <mercury-account-id> --service mercury
gringotts add-bank --name "Savings" --account-id <mercury-account-id> --service mercury --company "CompanyName"
```

#### General Commands

```bash
# Query all balances (includes all banking accounts)
gringotts query

# Query specific banking account
gringotts query-one "Operating Account"

# Remove banking account
gringotts remove "Operating Account"
```

### Organisation

You can use the `--company` flag to group wallets by 'organisation'. This can be useful if you want to subcategorise addresses in addition to giving them names.

```bash
gringotts add --name "Hot Wallet" --address <address> --company "CompanyA"
gringotts add --name "Cold Storage" --address <address> --company "CompanyB"
```

Portfolio summary displays assets grouped by the company flag.

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


### Custom RPC

Free RPC endpoints often have harsh rate limits, so you can use your own RPC endpoints for better rate limits or specific networks:

```bash
# Solana
gringotts query --rpc-url https://api.mainnet-beta.solana.com
gringotts query --rpc-url https://api.devnet.solana.com

# EVM chains
gringotts query --rpc-url https://eth.llamarpc.com
gringotts query --rpc-url https://polygon-rpc.com
gringotts query --rpc-url https://arb1.arbitrum.io/rpc
```
## Storage

Addresses: `~/.gringotts/addresses.json`

## Licence

MIT
