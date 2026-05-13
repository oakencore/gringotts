# Wallet Name Display — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

The `WalletAddress.name` field already exists and captures where each wallet "lives" (e.g., "WalletA", "WalletB", "WalletC"). It is shown on the dashboard at `templates/index.html:129`, but:

1. On the dashboard, the name visually competes with the full address rendered below it.
2. In `templates/balances.html` (the aggregated Query All view), the wallet name is **absent entirely** — balances are aggregated by `(company, symbol)` in `src/query.rs` + `src/types.rs:24` and the wallet that contributed each asset is lost.

The user cannot tell *where* a given balance is held when viewing aggregated balances.

## Goal

Make wallet `name` visible wherever balances or wallets are displayed, so the user can answer "where is my SOL located?" directly from the UI.

## Scope

- `templates/balances.html` — restructure to show per-wallet breakdown inside each company.
- `templates/index.html` — make `wallet.name` dominant over the address visually.
- `src/types.rs` — extend `CompanyAssets` with a wallet-level breakdown alongside the existing aggregated rollup.
- `src/query.rs` — thread `wallet.name` / `account.name` through all `aggregate_*_balances` helpers.
- `src/display/web.rs` — update `BalancesTemplate` shape and its builder.

**Out of scope:**

- Terminal display (`src/display/terminal.rs`) — unchanged.
- JSON API contracts — new `wallets` field will serialize additively; no breaking changes.
- Adding a new "location" field. The existing `name` field is the location identifier.
- Transactions page (`templates/transactions.html`) — already shows the name in its single-wallet context.

## Design

### Section 1 — Data model

Extend `CompanyAssets` in `src/types.rs` so the wallet-level breakdown lives alongside the existing aggregated rollup. Both representations are populated together by `add_asset_to_portfolio`.

```rust
pub struct CompanyAssets {
    pub assets: HashMap<String, AssetSummary>,    // unchanged — aggregated rollup
    pub wallets: HashMap<String, WalletAssets>,   // NEW — per-wallet breakdown
    pub total_usd_value: f64,
}

pub struct WalletAssets {
    pub name: String,
    pub assets: HashMap<String, AssetSummary>,
    pub total_usd_value: f64,
}
```

`add_asset_to_portfolio` gains a `wallet_name: &str` parameter and writes to both maps. Banking accounts fit the same shape via `BankingAccount.name`.

**Why keep both representations:** Terminal display (`src/display/terminal.rs:447`) and JSON API endpoints currently consume `CompanyAssets.assets`. Leaving it untouched means zero regression risk in those paths; the new `wallets` field is purely additive.

### Section 2 — `balances.html` restructure

`BalancesTemplate` shape (`src/display/web.rs:100-106`):

```rust
struct BalancesTemplate {
    total_usd: f64,
    companies: Vec<(String /* company */, Vec<WalletGroup>)>,
    error: String,
}

struct WalletGroup {
    name: String,
    total_usd: f64,
    assets: Vec<AssetView>,
}
```

Builder logic in the balances handler (`web.rs:1905-1928`):
- Iterate `portfolio.companies` sorted by `total_usd_value` descending.
- For each company, iterate `company.wallets` sorted by `total_usd_value` descending.
- Within each wallet, sort assets by USD value descending.

Template render in `balances.html`:

```
Company Acme ─────────────────────────
  Wallet: WalletA                    $X
    SOL      3      $300
    USDC   100      $100
  Wallet: WalletB               $Y
    SOL      7      $700
─────────────────────────────────────
Total Portfolio Value:           $XXX
```

Structurally: outer loop over `companies`, inner loop over `wallets` (wallet subheader row), nested loop over `assets`. The existing 4-column layout (Asset / Company / Balance / Value) is preserved; the Company column becomes a section header rather than repeating per row, and a wallet subheader row is added inside each section.

**Edge cases:**
- Empty company (no contributing wallets): omit the section.
- Wallet with all-zero balances: omit the wallet subheader. Consistent with `add_asset_to_portfolio`'s existing `amount == 0.0` skip.
- Banking accounts: treated identically to crypto wallets. The 8 `aggregate_*_balances` helpers in `src/query.rs:589-718` start passing their wallet/account `name` argument; Mercury and Circle helpers pass `account.name`.

### Section 3 — `index.html` dashboard prominence

The wallet name is already rendered at `templates/index.html:129`. The change is purely visual subordination of the address so the name visually wins:

1. **Cap the address visual width** via stronger CSS truncation (e.g., `max-width: 12ch` with `text-overflow: ellipsis`) or a head/tail middle-ellipsis filter.
2. **Add a `title` attribute** carrying the full address for hover inspection.

`.asset-name` font size stays as-is — it is already the dominant element; dialing down the address is sufficient.

### Section 4 — Terminal display & JSON APIs

- `src/display/terminal.rs:447` (`render_portfolio_summary`) is **unchanged**. It continues consuming `company.assets`.
- JSON endpoints (`/api/balances`, `/api/balances/company/:company` in `web.rs:1252` and `web.rs:1407`) continue serializing `PortfolioSummary`. The new `wallets` field appears additively. Existing clients ignore unknown fields; future clients can opt into the per-wallet breakdown without a new endpoint.

### Section 5 — Testing

**Unit tests** in `src/types.rs`:

1. `add_asset_to_portfolio` writes to both `assets` and `wallets[name].assets`. Two calls with the same `(company, symbol)` from different wallet names produce one entry in `company.assets` with summed amount/value, and two entries in `company.wallets` each carrying their own amount/value.
2. `total_usd_value` is correct at three levels: company, wallet, portfolio.
3. Zero-amount asset is skipped at both levels.
4. Multiple symbols in a single wallet land under the same `WalletAssets`.

**Manual verification:**

- `cargo build --release && ./target/release/gringotts serve --port 3000` — click Query, confirm balances render per-wallet within each company.
- `./target/release/gringotts query` — terminal output unchanged.

**Quality gates before claiming done:**
- `cargo fmt`
- `cargo clippy` — no new warnings
- `cargo test` — full suite passes, report exact count
