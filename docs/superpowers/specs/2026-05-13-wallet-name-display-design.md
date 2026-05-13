# Wallet Name Display — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

The `WalletAddress.name` field already exists and captures where each wallet "lives" (e.g., "WalletA", "WalletB", "WalletC"). It is shown on the dashboard at `templates/index.html:129`, but:

1. On the dashboard, the name visually competes with the full address rendered below it.
2. In `templates/balances.html` (the aggregated Query All view), the wallet name is **absent entirely** — the web handler aggregates by `(company, symbol)` and the wallet that contributed each asset is lost.

The user cannot tell *where* a given balance is held when viewing aggregated balances.

## Goal

Make wallet `name` visible wherever balances or wallets are displayed, so the user can answer "where is my SOL located?" directly from the UI.

## Scope

- `src/types.rs` — extend `CompanyAssets` with a wallet-level breakdown alongside the existing aggregated rollup.
- `src/query.rs` — thread `wallet.name` / `account.name` through all `aggregate_*_balances` helpers; bump them to `pub(crate)` visibility.
- `src/display/web.rs` — replace the inline aggregation in `query_balances` with `PortfolioSummary` so `balances.html` gets the per-wallet breakdown. Update `BalancesTemplate` shape.
- `templates/balances.html` — restructure to render company → wallet → assets.
- `templates/index.html` — make `wallet.name` dominate the address visually.
- `src/main.rs` — update existing `add_asset_to_portfolio` tests to pass the new `wallet_name` argument.

**Out of scope:**

- Terminal display (`src/display/terminal.rs`) — unchanged.
- JSON API endpoints — they read from `cache.balances` directly, not `PortfolioSummary`; unaffected.
- Adding a new "location" field. The existing `name` field is the location identifier.
- Transactions page — already shows the name in its single-wallet context.
- Deduplicating `query_balances` and `refresh_all_balances` (both still re-query chains and update the cache). That cleanup is a follow-up.

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

`add_asset_to_portfolio` signature becomes:

```rust
pub fn add_asset_to_portfolio(
    portfolio: &mut PortfolioSummary,
    company: &str,
    wallet_name: &str,   // NEW
    symbol: &str,
    amount: f64,
    usd_value: Option<f64>,
)
```

It writes to both `company.assets[symbol]` (rollup, unchanged) and `company.wallets[wallet_name].assets[symbol]` (new). `total_usd_value` is updated at three levels: `WalletAssets`, `CompanyAssets`, `PortfolioSummary`. Banking accounts fit the same shape via `BankingAccount.name`.

**Why keep both representations:** `src/display/terminal.rs:447` (`render_portfolio_summary`) consumes `company.assets`. Leaving it untouched means zero regression risk on the terminal path; `wallets` is purely additive. An alternative — drop `assets` and derive rollups on demand from `wallets` — was considered but rejected because it forces terminal code changes for no user-visible benefit.

#### Call site inventory

Every site that calls `add_asset_to_portfolio` or an `aggregate_*_balances` helper must be updated:

**`src/query.rs`** — 8 aggregate helpers (each calls `add_asset_to_portfolio` internally, 1+ times):

| Function | Line | Caller location |
|---|---|---|
| `aggregate_solana_balances` | 589 | called at 368 from `enrich_and_display_balances` |
| `aggregate_evm_balances` | 599 | called at 373 |
| `aggregate_near_balances` | 631 | called at 378 |
| `aggregate_aptos_balances` | 657 | called at 383 |
| `aggregate_sui_balances` | 683 | called at 388 |
| `aggregate_starknet_balances` | 709 | called at 393 |
| `aggregate_mercury_balances` | 714 | called at 397 |
| `aggregate_circle_balances` | 718 | called at 401 |

Each helper gets a `wallet_name: &str` parameter; each call site passes `&wallet.name` (crypto) or `&account.name` (banking). Helpers are bumped from private `fn` to `pub(crate) fn` so `web.rs` can call them from the refactored `query_balances` handler.

**`src/main.rs`** — 3 existing unit tests that invoke `add_asset_to_portfolio` directly and will fail to compile after the signature change:

- `test_add_asset_to_portfolio` at line 224 (1 call at line 230)
- `test_add_asset_to_portfolio_accumulation` at line 246 (2 calls at lines 253-254)
- `test_add_asset_zero_balance_ignored` at line 265 (1 call at line 271)

Each call gains a `wallet_name` argument (e.g., `"TestWallet"`).

**`src/display/web.rs`** — the refactored `query_balances` handler (see Section 2) will call the now-`pub(crate)` aggregate helpers, threading wallet/account `name` through.

### Section 2 — Refactor `query_balances` + restructure `balances.html`

Currently `query_balances` (`web.rs:1900-2277`) duplicates aggregation logic in ~370 lines of per-chain inline code using `HashMap<String, HashMap<String, (f64, f64)>>` (line 1929). This bypasses `PortfolioSummary` entirely, which is why a `types.rs` change alone cannot fix `balances.html`.

**Refactor approach** — replace the inline aggregation with shared aggregation:

1. Keep the existing per-wallet/per-account query loop (it still does chain RPC calls and updates the cache via `state.cache.update_balance`).
2. After each successful chain query, call the corresponding now-`pub(crate)` aggregate helper, passing `&wallet.name` or `&account.name`, into a local `PortfolioSummary`.
3. After the loop, build `BalancesTemplate` from the `PortfolioSummary`'s `companies → wallets → assets`.

The 8 per-chain query branches in `query_balances` keep their cache-update blocks intact; they swap their inline `portfolio.entry(...)` / `entry.entry(...)` accumulation for a single call to `aggregate_<chain>_balances(&mut portfolio, &wallet.company, &wallet.name, &balances)`. Net diff: roughly -100 lines of inline accumulation, +1 call per branch.

**`BalancesTemplate` shape** (`src/display/web.rs:100-106`) — the existing `companies: Vec<(String, Vec<AssetView>)>` is **replaced** (not extended) with:

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

**Build logic** (after the query loop):
- Iterate `portfolio.companies` sorted by `CompanyAssets.total_usd_value` descending.
- For each company, iterate `company.wallets.values()` sorted by `WalletAssets.total_usd_value` descending.
- Within each wallet, sort assets by USD value descending.
- Filter out wallets with zero `total_usd_value` (consistent with the existing `amount == 0.0` skip in `add_asset_to_portfolio`).

**Template render** (`templates/balances.html`):

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

Outer loop over `companies`, inner loop over `wallets` (each emits a subheader row showing wallet name + USD subtotal), nested loop over `assets`. The existing 4-column layout collapses the Company column into a section header rather than repeating it per row.

**Edge cases:**
- Empty company (no contributing wallets): omit the section.
- Wallet with all-zero balances: omit.
- Banking accounts: treated identically to crypto wallets via `account.name`.
- Failed wallet query: continues to be skipped silently (current behavior preserved — chain RPC `Err(_)` does not push into the portfolio).
- Empty company string: the existing handler maps `wallet.company.is_empty()` to `"Uncategorized"` (`web.rs:1939-1943`). The refactor preserves this — the `&wallet.company` argument passed into each aggregate helper is replaced with the `is_empty()`-substituted value, matching prior behavior.

### Section 3 — `index.html` dashboard prominence

The wallet name is already rendered at `templates/index.html:129`. Change is purely CSS:

1. **Truncate the address visually via CSS.** The address div at line 130 already has `.truncate`. Add a stricter rule scoped to `.asset-info .asset-symbol`: a fixed `max-width` (starting around 120px) plus `text-overflow: ellipsis` and `overflow: hidden`. CSS-only — no Askama filter — to keep the change contained to the template/stylesheet. The exact `max-width` will be tuned by visual feedback in the running app.
2. **Add a `title="{{ wallet.address }}"` attribute** on the address div so hovering reveals the full string.

`.asset-name` font size stays as-is — it is already the dominant element; dialing down the address is sufficient.

### Section 4 — Terminal display & JSON APIs

- **`src/display/terminal.rs:447`** (`render_portfolio_summary`) is **unchanged**. It continues consuming `company.assets`. The new `wallets` field is ignored on this path.
- **JSON API endpoints** (`get_totals_json` at `web.rs:1252`, `get_company_totals_json` at `web.rs:1407`, `get_balances_json`, `get_company_balances_json`, `get_single_balance_json` at `web.rs:371-373`) read from `state.cache` and build their own `serde_json::Value` responses. They do **not** go through `PortfolioSummary`, so they are unaffected by this change. No API contract change.

If the per-wallet breakdown should eventually be exposed via the JSON API, that is a separate endpoint addition — out of scope.

### Section 5 — Testing

**Unit tests** (in `src/types.rs` or `src/main.rs` test module):

1. `add_asset_to_portfolio` writes to both maps. Two calls with the same `(company, symbol)` from different wallet names produce: one entry in `company.assets` with summed amount/value, and two entries in `company.wallets` each carrying their own amount/value.
2. `total_usd_value` is correct at three levels: wallet, company, portfolio.
3. Zero-amount asset is skipped at both levels.
4. Multiple symbols in a single wallet land under the same `WalletAssets`.

**Existing test updates:** Three tests in `src/main.rs:217-274` (`test_add_asset_to_portfolio`, `test_add_asset_to_portfolio_accumulation`, `test_add_asset_zero_balance_ignored`) currently invoke `add_asset_to_portfolio` with five arguments. Each gains a sixth `wallet_name` argument. These tests should also be extended to assert that the `wallets` map is populated correctly (or new tests added alongside, with the existing ones left as basic-signature regression tests).

**Manual verification:**
- `cargo build --release && ./target/release/gringotts serve --port 3000` — click Query All, confirm `balances.html` renders per-wallet within each company; click Query on a single wallet, confirm `single_balance.html` still works.
- `./target/release/gringotts query` — terminal output unchanged from prior.

**Quality gates before claiming done:**
- `cargo fmt`
- `cargo clippy` — no new warnings
- `cargo test` — full suite passes; report exact pass/fail count.
