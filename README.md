# Gringotts

CLI for tracking cryptocurrency balances across Solana and EVM chains with USD pricing.

## Setup

Get a free CoinMarketCap API key at https://coinmarketcap.com/api/ (10,000 calls/month).

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
gringotts add --name "Wallet" --address <address> --chain polygon

# List tracked addresses
gringotts list

# Query all balances
gringotts query

# Query specific wallet
gringotts query-one "Wallet"

# Remove address (by name or address)
gringotts remove "Wallet"
```

### Supported Chains

**Solana**: `solana`, `sol`
**Ethereum**: `ethereum`, `eth` (default for 0x addresses)
**Polygon**: `polygon`, `matic`
**BSC**: `bsc`, `binance`, `bnb`
**Arbitrum**: `arbitrum`, `arb`
**Optimism**: `optimism`, `op`
**Avalanche**: `avalanche`, `avax`
**Base**: `base`

EVM addresses auto-detect as Ethereum. Specify `--chain` for other EVM networks.

### Custom RPC

Use your own RPC endpoints for better rate limits or specific networks:

```bash
# Solana
gringotts query --rpc-url https://api.mainnet-beta.solana.com
gringotts query --rpc-url https://api.devnet.solana.com

# EVM chains
gringotts query --rpc-url https://eth.llamarpc.com
gringotts query --rpc-url https://polygon-rpc.com
gringotts query --rpc-url https://arb1.arbitrum.io/rpc
```

## Features

**Solana**
- SOL balance with USD pricing
- SPL token detection via Metaplex metadata
- Supported tokens: USDC, USDT, mSOL, stSOL, SWTCH, JTO, RAT

**EVM Chains**
- Native balance (ETH, MATIC, BNB, etc.)
- ERC20 tokens: USDC, USDT, DAI (varies by chain)
- Contract-based token metadata

**Portfolio**
- Aggregated asset view across all chains
- Total portfolio value in USD
- Real-time pricing via CoinMarketCap API

## Storage

Addresses: `~/.gringotts/addresses.json`

## Licence

MIT
