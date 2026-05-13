# Wallet Name Display Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show wallet `name` everywhere balances are listed in the web UI, so the user can see *where* each balance is held.

**Architecture:** Extend `PortfolioSummary` with a per-wallet breakdown (`CompanyAssets.wallets`) alongside the existing aggregated rollup. Refactor the web `query_balances` handler to use the shared `aggregate_*_balances` helpers (eliminating ~370 lines of duplicated inline aggregation). Restructure `templates/balances.html` to render company → wallet → assets. Visually subordinate the address on the dashboard so `wallet.name` dominates.

**Tech Stack:** Rust, Askama templates, Axum, HTMX. Tests via `cargo test`. Format/lint via `cargo fmt` / `cargo clippy`.

**Spec:** `docs/superpowers/specs/2026-05-13-wallet-name-display-design.md`

---

## File Structure

**Modified files:**
- `src/types.rs` — add `WalletAssets`, extend `CompanyAssets`, change `add_asset_to_portfolio` signature
- `src/main.rs` — update 3 existing tests, add new tests asserting wallet-level behavior
- `src/query.rs` — bump 8 aggregate helpers to `pub(crate)`, add `wallet_name` parameter, update 8 call sites
- `src/display/web.rs` — refactor `query_balances` to build a `PortfolioSummary` via aggregate helpers; replace `BalancesTemplate` shape
- `templates/balances.html` — render company → wallet → assets
- `templates/index.html` — CSS rule to constrain address width

**No new files.**

---

## Chunk 1: Data Model & Tests

### Task 1: Extend data model with per-wallet breakdown

**Files:**
- Modify: `src/types.rs:7-58`

- [ ] **Step 1: Write failing tests in `src/main.rs`**

Replace the three existing tests at `src/main.rs:223-274` and append two new tests. Final test block:

```rust
#[test]
fn test_add_asset_to_portfolio() {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "BTC", 1.0, Some(50000.0));

    assert_eq!(portfolio.companies.len(), 1);
    assert!(portfolio.companies.contains_key("TestCo"));
    assert_eq!(portfolio.total_usd_value, 50000.0);

    let company = portfolio.companies.get("TestCo").unwrap();
    assert_eq!(company.total_usd_value, 50000.0);
    assert!(company.assets.contains_key("BTC"));
    assert!(company.wallets.contains_key("WalletA"));

    let btc = company.assets.get("BTC").unwrap();
    assert_eq!(btc.amount, 1.0);
    assert_eq!(btc.usd_value, Some(50000.0));

    let wallet_a = company.wallets.get("WalletA").unwrap();
    assert_eq!(wallet_a.name, "WalletA");
    assert_eq!(wallet_a.total_usd_value, 50000.0);
    let wallet_btc = wallet_a.assets.get("BTC").unwrap();
    assert_eq!(wallet_btc.amount, 1.0);
    assert_eq!(wallet_btc.usd_value, Some(50000.0));
}

#[test]
fn test_add_asset_to_portfolio_accumulation() {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "BTC", 1.0, Some(50000.0));
    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "BTC", 0.5, Some(25000.0));

    let company = portfolio.companies.get("TestCo").unwrap();
    let btc = company.assets.get("BTC").unwrap();
    assert_eq!(btc.amount, 1.5);
    assert_eq!(btc.usd_value, Some(75000.0));
    assert_eq!(portfolio.total_usd_value, 75000.0);

    let wallet_a = company.wallets.get("WalletA").unwrap();
    let wallet_btc = wallet_a.assets.get("BTC").unwrap();
    assert_eq!(wallet_btc.amount, 1.5);
    assert_eq!(wallet_btc.usd_value, Some(75000.0));
    assert_eq!(wallet_a.total_usd_value, 75000.0);
}

#[test]
fn test_add_asset_zero_balance_ignored() {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "BTC", 0.0, Some(0.0));

    assert_eq!(portfolio.companies.len(), 0);
}

#[test]
fn test_add_asset_disaggregates_by_wallet() {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "SOL", 3.0, Some(300.0));
    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletB", "SOL", 7.0, Some(700.0));

    let company = portfolio.companies.get("TestCo").unwrap();

    // Rollup: 10 SOL @ $1000
    let sol_rollup = company.assets.get("SOL").unwrap();
    assert_eq!(sol_rollup.amount, 10.0);
    assert_eq!(sol_rollup.usd_value, Some(1000.0));

    // Disaggregated: two wallets, each with their own SOL line
    assert_eq!(company.wallets.len(), 2);

    let walleta = company.wallets.get("WalletA").unwrap();
    assert_eq!(walleta.assets.get("SOL").unwrap().amount, 3.0);
    assert_eq!(walleta.total_usd_value, 300.0);

    let acme = company.wallets.get("WalletB").unwrap();
    assert_eq!(acme.assets.get("SOL").unwrap().amount, 7.0);
    assert_eq!(acme.total_usd_value, 700.0);

    // Company total still aggregates correctly
    assert_eq!(company.total_usd_value, 1000.0);
    assert_eq!(portfolio.total_usd_value, 1000.0);
}

#[test]
fn test_add_asset_multiple_symbols_in_one_wallet() {
    let mut portfolio = PortfolioSummary {
        companies: HashMap::new(),
        total_usd_value: 0.0,
    };

    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "SOL", 3.0, Some(300.0));
    add_asset_to_portfolio(&mut portfolio, "TestCo", "WalletA", "USDC", 100.0, Some(100.0));

    let company = portfolio.companies.get("TestCo").unwrap();
    let walleta = company.wallets.get("WalletA").unwrap();
    assert_eq!(walleta.assets.len(), 2);
    assert_eq!(walleta.assets.get("SOL").unwrap().amount, 3.0);
    assert_eq!(walleta.assets.get("USDC").unwrap().amount, 100.0);
    assert_eq!(walleta.total_usd_value, 400.0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --lib add_asset
```

Expected: compilation failure (`add_asset_to_portfolio` has wrong arity, `WalletAssets` does not exist, `company.wallets` does not exist).

- [ ] **Step 3: Implement the data model changes**

In `src/types.rs`, replace lines 7-58 with:

```rust
use std::collections::HashMap;

use crate::chains::{solana, evm, near, aptos, sui, starknet};
use crate::banking::{mercury, circle};
use crate::storage::{WalletAddress, BankingAccount};

// Portfolio summary structure
pub struct PortfolioSummary {
    pub companies: HashMap<String, CompanyAssets>,
    pub total_usd_value: f64,
}

pub struct CompanyAssets {
    pub assets: HashMap<String, AssetSummary>,
    pub wallets: HashMap<String, WalletAssets>,
    pub total_usd_value: f64,
}

pub struct WalletAssets {
    pub name: String,
    pub assets: HashMap<String, AssetSummary>,
    pub total_usd_value: f64,
}

pub struct AssetSummary {
    pub symbol: String,
    pub amount: f64,
    pub usd_value: Option<f64>,
}

pub fn add_asset_to_portfolio(
    portfolio: &mut PortfolioSummary,
    company: &str,
    wallet_name: &str,
    symbol: &str,
    amount: f64,
    usd_value: Option<f64>,
) {
    if amount == 0.0 {
        return;
    }

    let company_assets = portfolio
        .companies
        .entry(company.to_string())
        .or_insert_with(|| CompanyAssets {
            assets: HashMap::new(),
            wallets: HashMap::new(),
            total_usd_value: 0.0,
        });

    // Update aggregated rollup
    let asset = company_assets
        .assets
        .entry(symbol.to_string())
        .or_insert_with(|| AssetSummary {
            symbol: symbol.to_string(),
            amount: 0.0,
            usd_value: None,
        });
    asset.amount += amount;
    if let Some(value) = usd_value {
        asset.usd_value = Some(asset.usd_value.unwrap_or(0.0) + value);
        company_assets.total_usd_value += value;
        portfolio.total_usd_value += value;
    }

    // Update per-wallet breakdown
    let wallet = company_assets
        .wallets
        .entry(wallet_name.to_string())
        .or_insert_with(|| WalletAssets {
            name: wallet_name.to_string(),
            assets: HashMap::new(),
            total_usd_value: 0.0,
        });
    let wallet_asset = wallet
        .assets
        .entry(symbol.to_string())
        .or_insert_with(|| AssetSummary {
            symbol: symbol.to_string(),
            amount: 0.0,
            usd_value: None,
        });
    wallet_asset.amount += amount;
    if let Some(value) = usd_value {
        wallet_asset.usd_value = Some(wallet_asset.usd_value.unwrap_or(0.0) + value);
        wallet.total_usd_value += value;
    }
}
```

- [ ] **Step 4: Verify the data-model tests still fail (signatures wrong elsewhere)**

```bash
cargo build 2>&1 | head -40
```

Expected: build fails. `query.rs` and `web.rs` callers of `add_asset_to_portfolio` still pass 5 args instead of 6. That is fixed in Tasks 2 and 3 — do not try to fix them yet.

- [ ] **Step 5: Commit**

```bash
git add src/types.rs src/main.rs
git commit -m "feat: extend CompanyAssets with per-wallet breakdown"
```

---

## Chunk 2: Thread wallet_name through aggregation

### Task 2: Update aggregate helpers in `src/query.rs`

**Files:**
- Modify: `src/query.rs:589-728` (helper definitions)
- Modify: `src/query.rs:363-403` (call sites in `enrich_and_display_balances`)

- [ ] **Step 1: Bump visibility and add `wallet_name` param to all 8 aggregate helpers**

For each helper at `src/query.rs:589`, `599`, `631`, `657`, `683`, `709`, `714`, `718`:

1. Change `fn` → `pub(crate) fn`.
2. Add `wallet_name: &str` as the third parameter (after `company`).
3. Pass `wallet_name` through every internal `add_asset_to_portfolio` call.

Exact rewrites:

```rust
pub(crate) fn aggregate_solana_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &solana::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "SOL", balances.sol_balance, balances.sol_usd_value);

    for token in &balances.token_balances {
        if let Some(symbol) = &token.symbol {
            add_asset_to_portfolio(portfolio, company, wallet_name, symbol, token.ui_amount, token.usd_value);
        }
    }
}

pub(crate) fn aggregate_evm_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &evm::AccountBalances, _chain: &Chain) {
    add_asset_to_portfolio(portfolio, company, wallet_name, &balances.native_symbol, balances.eth_balance, balances.eth_usd_value);

    for token in &balances.token_balances {
        if let Some(symbol) = &token.symbol {
            add_asset_to_portfolio(portfolio, company, wallet_name, symbol, token.ui_amount, token.usd_value);
        }
    }
}

pub(crate) fn aggregate_near_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &near::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "NEAR", balances.near_balance, balances.near_usd_value);
}

pub(crate) fn aggregate_aptos_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &aptos::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "APT", balances.apt_balance, balances.apt_usd_value);
}

pub(crate) fn aggregate_sui_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &sui::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "SUI", balances.sui_balance, balances.sui_usd_value);
}

pub(crate) fn aggregate_starknet_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &starknet::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "ETH", balances.eth_balance, balances.eth_usd_value);
}

pub(crate) fn aggregate_mercury_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &mercury::AccountBalances) {
    add_asset_to_portfolio(portfolio, company, wallet_name, "USD", balances.current_balance, Some(balances.current_balance));
}

pub(crate) fn aggregate_circle_balances(portfolio: &mut PortfolioSummary, company: &str, wallet_name: &str, balances: &circle::AccountBalances) {
    for balance in &balances.available_balances {
        let usd_value = if balance.currency == "USD" {
            Some(balance.amount)
        } else {
            None
        };
        add_asset_to_portfolio(portfolio, company, wallet_name, &balance.currency, balance.amount, usd_value);
    }
}
```

- [ ] **Step 2: Update the 8 call sites in `enrich_and_display_balances`**

In `src/query.rs:363-403`, each `aggregate_*_balances(...)` call gets `&wallet.name` (crypto) or `&account.name` (banking) inserted as the third argument:

```rust
WalletBalances::Solana(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_solana_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_solana_balances(&mut portfolio, &wallet.company, &wallet.name, &balances);
}
WalletBalances::Evm(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_evm_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_evm_balances(&mut portfolio, &wallet.company, &wallet.name, &balances, &wallet.chain);
}
WalletBalances::Near(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_near_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_near_balances(&mut portfolio, &wallet.company, &wallet.name, &balances);
}
WalletBalances::Aptos(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_aptos_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_aptos_balances(&mut portfolio, &wallet.company, &wallet.name, &balances);
}
WalletBalances::Sui(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_sui_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_sui_balances(&mut portfolio, &wallet.company, &wallet.name, &balances);
}
WalletBalances::Starknet(wallet, mut balances) => {
    balances.enrich_from_cache(price_cache);
    ui::render_starknet_balances(&wallet.company, &wallet.name, &wallet.address, &balances, &wallet.chain);
    aggregate_starknet_balances(&mut portfolio, &wallet.company, &wallet.name, &balances);
}
WalletBalances::Mercury(account, balances) => {
    ui::render_mercury_balances(&account.company, &account.name, &account.account_id, &balances, &account.service);
    aggregate_mercury_balances(&mut portfolio, &account.company, &account.name, &balances);
}
WalletBalances::Circle(account, balances) => {
    ui::render_circle_balances(&account.company, &account.name, &balances, &account.service);
    aggregate_circle_balances(&mut portfolio, &account.company, &account.name, &balances);
}
```

- [ ] **Step 3: Run cargo build to confirm `query.rs` now compiles**

```bash
cargo build 2>&1 | grep -E "(error|warning: unused)" | head -20
```

Expected: only errors remaining should be in `src/display/web.rs` (the web handler still calls the helpers with the old signature). `query.rs` and `types.rs` errors should be gone.

- [ ] **Step 4: Run the data-model tests from Task 1 — they should now pass**

```bash
cargo test --lib test_add_asset 2>&1 | tail -20
```

Expected: all 5 `test_add_asset_*` tests pass. (`cargo test` may still fail overall because `web.rs` is broken; that is fixed in Task 3.)

If the lib tests can't even compile because `web.rs` is broken in the same crate, skip this step and verify after Task 3.

- [ ] **Step 5: Commit**

```bash
git add src/query.rs
git commit -m "feat: thread wallet_name through aggregate helpers"
```

---

## Chunk 3: Refactor web handler

### Task 3: Refactor `query_balances` to use shared aggregation

**Files:**
- Modify: `src/display/web.rs:85-178` (template structs)
- Modify: `src/display/web.rs:1900-2277` (`query_balances` handler — replace inline aggregation)

This is the biggest change. The current handler does its own inline aggregation per chain. We replace that aggregation with calls to the now-`pub(crate)` aggregate helpers, while preserving the cache-update side-effects.

- [ ] **Step 1: Update `BalancesTemplate` and helper view structs**

In `src/display/web.rs`, replace the `BalancesTemplate` definition at lines 100-106 and add a new `WalletGroup` struct. Final state:

```rust
#[derive(Template)]
#[template(path = "balances.html")]
struct BalancesTemplate {
    total_usd: f64,
    companies: Vec<(String, Vec<WalletGroup>)>,
    error: String,
}

struct WalletGroup {
    name: String,
    total_usd: f64,
    assets: Vec<AssetView>,
}
```

`AssetView` (around `web.rs:174`) is reused unchanged. The old `companies: Vec<(String, Vec<AssetView>)>` shape goes away.

- [ ] **Step 2: Add imports for the shared aggregation in `web.rs`**

At the top of `src/display/web.rs`, ensure these items are imported (add to the existing `use` block, or add a new one):

```rust
use crate::query::{
    aggregate_solana_balances, aggregate_evm_balances, aggregate_near_balances,
    aggregate_aptos_balances, aggregate_sui_balances, aggregate_starknet_balances,
    aggregate_mercury_balances, aggregate_circle_balances,
};
use crate::types::PortfolioSummary;
```

- [ ] **Step 3: Replace the `query_balances` handler body**

Replace `query_balances` at `src/display/web.rs:1900-2277` with the following. Two structural changes from current:

1. The local `portfolio: HashMap<String, HashMap<String, (f64, f64)>>` (line 1929) is replaced by a `PortfolioSummary`.
2. Each per-chain branch keeps its cache-update logic (the `CachedBalance { ... }` block and `state.cache.update_balance(...)` call), but replaces its inline accumulation with a call to the shared `aggregate_*_balances` helper.

For the **company string**, preserve the existing `is_empty() → "Uncategorized"` behavior by computing the substituted value once per wallet/account at the top of each branch.

Concrete rewrite for each branch (Solana shown — apply the same shape to Near, Aptos, Sui, Starknet, EVM, Mercury, Circle):

```rust
Chain::Solana => {
    let client = SolanaClient::new(None);
    if let Ok(balances) = client.get_balances(&wallet.address) {
        let company = if wallet.company.is_empty() {
            "Uncategorized"
        } else {
            wallet.company.as_str()
        };

        aggregate_solana_balances(&mut portfolio, company, &wallet.name, &balances);

        // Cache update (unchanged behavior)
        let mut cached_tokens = vec![];
        for token in &balances.token_balances {
            if let Some(symbol) = &token.symbol {
                cached_tokens.push(CachedToken {
                    symbol: symbol.clone(),
                    balance: token.ui_amount,
                    usd_value: token.usd_value,
                });
            }
        }
        let cached_balance = CachedBalance {
            name: wallet.name.clone(),
            address_or_id: wallet.address.clone(),
            chain_or_service: wallet.chain.display_name().to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: balances.sol_balance,
            native_usd_value: balances.sol_usd_value,
            tokens: cached_tokens,
            total_usd_value: balances.total_usd_value,
        };
        let _ = state.cache.update_balance(&wallet.name, cached_balance).await;
    }
}
```

Notes:
- `all_symbols: HashSet<String>` was used in the old code to track which symbols were seen for later price fetch. Inspect whether it is still needed downstream in `query_balances`; if it was only used for inline accumulation, remove it. Otherwise, leave the `all_symbols.insert(...)` calls in place inside each branch.
- For EVM, the native symbol comes from `balances.native_symbol` (not a hardcoded "ETH"). The helper handles this — but the cache update's `native_symbol` field must also reflect this.
- For Mercury: the existing branch reads `balances.current_balance`; keep that unchanged in the cache update and let `aggregate_mercury_balances` handle the portfolio side.
- For Circle: the existing branch iterates `balances.available_balances`; keep that unchanged for cache, and let `aggregate_circle_balances` handle the portfolio side.

After the per-wallet / per-account loops, build the template data from `portfolio`:

```rust
// Build template data from PortfolioSummary
let mut companies_view: Vec<(String, Vec<WalletGroup>)> = Vec::new();
let mut sorted_companies: Vec<(&String, &CompanyAssets)> = portfolio.companies.iter().collect();
sorted_companies.sort_by(|a, b| b.1.total_usd_value.total_cmp(&a.1.total_usd_value));

for (company_name, company) in sorted_companies {
    let mut wallets_view: Vec<WalletGroup> = Vec::new();
    let mut sorted_wallets: Vec<&WalletAssets> = company.wallets.values().collect();
    sorted_wallets.sort_by(|a, b| b.total_usd_value.total_cmp(&a.total_usd_value));

    for wallet_assets in sorted_wallets {
        if wallet_assets.total_usd_value == 0.0 && wallet_assets.assets.is_empty() {
            continue;
        }
        let mut asset_views: Vec<AssetView> = wallet_assets.assets.values().map(|a| AssetView {
            symbol: a.symbol.clone(),
            amount: a.amount,
            usd_value: a.usd_value.unwrap_or(0.0),
        }).collect();
        asset_views.sort_by(|a, b| b.usd_value.total_cmp(&a.usd_value));

        wallets_view.push(WalletGroup {
            name: wallet_assets.name.clone(),
            total_usd: wallet_assets.total_usd_value,
            assets: asset_views,
        });
    }

    if !wallets_view.is_empty() {
        companies_view.push((company_name.clone(), wallets_view));
    }
}

Html(
    BalancesTemplate {
        total_usd: portfolio.total_usd_value,
        companies: companies_view,
        error: String::new(),
    }
    .render()
    .unwrap_or_default(),
)
```

Add the import for `WalletAssets` from `crate::types` near the existing `PortfolioSummary` import.

- [ ] **Step 4: Run `cargo build`**

```bash
cargo build 2>&1 | tail -40
```

Expected: compiles. Address any remaining errors (likely unused imports or type mismatches in untouched branches).

- [ ] **Step 5: Run `cargo test`**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing + new tests pass. The Askama template will not have been updated yet, so `balances.html` will render with the old shape and may fail to compile — that is fixed in Task 4. If `cargo test` fails specifically with a template compile error, defer fixing it until Task 4.

- [ ] **Step 6: Commit**

```bash
git add src/display/web.rs
git commit -m "refactor: route query_balances through shared aggregation"
```

---

## Chunk 4: Template restructure

### Task 4: Restructure `templates/balances.html`

**Files:**
- Modify: `templates/balances.html`

- [ ] **Step 1: Replace the table body with company → wallet → asset structure**

Replace the `<tbody>...</tbody>` and surrounding `<table>` in `templates/balances.html` (lines 22-56) with a sectioned layout. Final balances rendering:

```html
{% for (company, wallets) in companies %}
<div class="company-section">
    <div class="company-header">
        <span class="company-name">{{ company }}</span>
    </div>

    {% for wallet in wallets %}
    <div class="wallet-section">
        <div class="wallet-header">
            <span class="wallet-name">{{ wallet.name }}</span>
            <span class="wallet-total amount positive">${{ wallet.total_usd|format_usd }}</span>
        </div>

        <table class="data-table wallet-assets">
            <tbody>
                {% for asset in wallet.assets %}
                <tr>
                    <td>
                        <div class="asset-info">
                            <div class="asset-icon">{{ asset.symbol.chars().next().unwrap_or('?') }}</div>
                            <div class="asset-name">{{ asset.symbol }}</div>
                        </div>
                    </td>
                    <td class="amount">{{ asset.amount|format_amount }}</td>
                    <td class="amount positive">
                        {% if asset.usd_value > 0.0 %}
                        ${{ asset.usd_value|format_usd }}
                        {% else %}
                        --
                        {% endif %}
                    </td>
                </tr>
                {% endfor %}
            </tbody>
        </table>
    </div>
    {% endfor %}
</div>
{% endfor %}
```

Add basic CSS for the new sections at the bottom `<style>` block:

```css
.company-section {
    margin-bottom: 24px;
}

.company-header {
    padding: 12px 20px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
}

.company-name {
    font-size: 14px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-disabled);
}

.wallet-section {
    margin-left: 16px;
    border-left: 2px solid var(--border-subtle);
    padding-left: 12px;
    margin-bottom: 16px;
}

.wallet-header {
    display: flex;
    justify-content: space-between;
    padding: 8px 12px;
    background: var(--bg-surface);
}

.wallet-name {
    font-weight: 500;
    font-size: 13px;
}

.wallet-total {
    font-family: var(--font-mono);
    font-weight: 500;
}

.wallet-assets {
    width: 100%;
}
```

- [ ] **Step 2: Run `cargo build` to catch template errors**

```bash
cargo build 2>&1 | tail -30
```

Expected: clean build. Askama compiles the template at build time, so any field reference typo will surface here.

- [ ] **Step 3: Run `cargo test`**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 4: Manual smoke test**

```bash
cargo build --release && ./target/release/gringotts serve --port 3000
```

In a browser at `http://localhost:3000`, click **Query All**. Verify the result table groups by company, with each wallet's assets nested under a wallet subheader showing the wallet name and per-wallet USD subtotal.

- [ ] **Step 5: Commit**

```bash
git add templates/balances.html
git commit -m "feat: render balances grouped by company and wallet"
```

---

## Chunk 5: Dashboard visual tweak

### Task 5: Subordinate the address on the dashboard

**Files:**
- Modify: `templates/index.html:121-168` (wallet/account row rendering)

- [ ] **Step 1: Add a title attribute and a CSS rule**

In `templates/index.html` at line 130, change the address div to include a `title` attribute:

```html
<div class="asset-symbol truncate" title="{{ wallet.address }}">{{ wallet.address }}</div>
```

And at line 178 (banking account):

```html
<div class="asset-symbol truncate" title="{{ account.account_id }}">{{ account.account_id }}</div>
```

Then in the page's `<style>` block (at the bottom of `templates/index.html`, or in `templates/base.html`'s shared CSS — pick the first that already defines `.asset-symbol`), add a scoped rule:

```css
.asset-info .asset-symbol {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
```

If the existing `.asset-symbol` style already sets `max-width`, just tighten the value to `120px`.

- [ ] **Step 2: Manual smoke test**

```bash
cargo build --release && ./target/release/gringotts serve --port 3000
```

In a browser, confirm:
- Each wallet row in the dashboard table shows the wallet `name` as the prominent label.
- The address is truncated with an ellipsis.
- Hovering the truncated address reveals the full string via the browser tooltip.

Adjust `max-width` between 96px and 160px if the truncation looks too aggressive or too loose.

- [ ] **Step 3: Commit**

```bash
git add templates/index.html
git commit -m "feat: subordinate address on dashboard so wallet name dominates"
```

---

## Chunk 6: Final verification

### Task 6: Quality gates and final smoke test

- [ ] **Step 1: Format**

```bash
cargo fmt
```

- [ ] **Step 2: Lint**

```bash
cargo clippy --all-targets 2>&1 | tail -30
```

Expected: no new warnings. Fix any introduced.

- [ ] **Step 3: Full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. Report exact count (e.g., "N passing, 0 failing") in the final summary.

- [ ] **Step 4: Terminal-path regression check**

```bash
./target/release/gringotts query 2>&1 | tail -40
```

Expected: terminal output unchanged from prior — companies and their aggregated assets list, same shape as before.

- [ ] **Step 5: Web-path regression check**

```bash
./target/release/gringotts serve --port 3000
```

Browser: dashboard wallets list shows `name` prominently with truncated address. Clicking **Query All** shows balances grouped by company → wallet → assets. Single-wallet **Query** button on a row still loads `single_balance.html`. Transactions button still loads `transactions.html`.

- [ ] **Step 6: Commit any fmt/clippy fixes**

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: cargo fmt and clippy cleanup"
```

- [ ] **Step 7: Open a PR**

```bash
git push -u origin HEAD && gh pr create --title "feat: show wallet name on balances and dashboard" --body "$(cat <<'EOF'
## Summary
- Extend `CompanyAssets` with a per-wallet breakdown (`wallets: HashMap<String, WalletAssets>`) alongside the existing aggregated rollup
- Refactor the web `query_balances` handler to use the shared `aggregate_*_balances` helpers, eliminating ~370 lines of duplicated inline aggregation
- Restructure `templates/balances.html` to render company → wallet → assets
- Visually subordinate the address on the dashboard so `wallet.name` dominates

Spec: `docs/superpowers/specs/2026-05-13-wallet-name-display-design.md`

## Test plan
- [x] `cargo test` passes
- [x] `cargo clippy` clean
- [x] Manual: Query All shows per-wallet attribution
- [x] Manual: dashboard shows wallet name prominently, address truncated
- [x] Manual: terminal `gringotts query` unchanged

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---
