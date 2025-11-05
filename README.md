# Gringotts

CLI interface for tracking cryptocurrency balances across multiple blockchains.

## Setup

Get a CoinMarketCap API key at https://coinmarketcap.com/api/ 

```bash
export COINMARKETCAP_API_KEY="your-api-key-here"
```

Build:
```bash
cargo build --release
# Binary: target/release/gringotts

# Or install:
cargo install --path .
```

## Usage

```bash
# Add addresses
gringotts add --name "Wallet" --address <address>
gringotts add --name "Wallet" --address <address> --chain solana
gringotts add --name "Wallet" --address <address> --chain aptos --company "ACMECORP"

# List tracked addresses
gringotts list

# Query all balances
gringotts query

# Query specific wallet
gringotts query-one "Wallet"

# Remove address (by name or address)
gringotts remove "Wallet"
```

### Organisation

You can use the `--company` flag to group wallets by 'organisation'. This can be useful if you want to subcategorise addresses in addition to giving them names.

```bash
gringotts add --name "Hot Wallet" --address <address> --company "ACMECORP"
gringotts add --name "Cold Storage" --address <address> --company "SALLYS"
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

Free RPC enpoints often have harsh rate limits, so you can use your own RPC endpoints for better rate limits or specific networks:

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
