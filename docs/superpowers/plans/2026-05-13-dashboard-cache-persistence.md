# Dashboard Cache Persistence Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the dashboard show cached balances after navigation, so the Balance / Value cells and the Total Portfolio Value / Last Refresh metric cards no longer reset to `--` when the user revisits the dashboard.

**Architecture:** The `index` handler reads `state.cache` on every render. `WalletView` / `BankingView` gain three optional cached fields populated via small free-function helpers (`populate_wallet_view`, `populate_banking_view`) that take a `&BalanceCache` and are unit-testable in isolation. `IndexTemplate` carries `total_portfolio_usd` and `last_refresh_human` as `Option<…>` so the existing metric cards render server-side initial values that JS/HTMX overwrite after Refresh All.

**Tech Stack:** Rust, Askama, Axum, vanilla JS. Tests via `cargo test`. Format/lint via `cargo fmt`/`cargo clippy`.

**Spec:** `docs/superpowers/specs/2026-05-13-dashboard-cache-persistence-design.md`

---

## File Structure

**Modified files:**
- `src/display/web.rs` — extend `WalletView` (line 270), `BankingView` (line 278), `IndexTemplate` (line 96); add `populate_wallet_view` / `populate_banking_view` / `compute_dashboard_metrics` helpers; rewire the `index` handler (line 2280) to read cache and use the helpers; 4-5 new unit tests.
- `templates/index.html` — conditional rendering on Balance/Value cells (lines 147-149 and 197-199); conditional rendering on Total Portfolio Value (line 43) and Last Refresh (line 55); JS update at line 414 to write `"just now"` instead of wall-clock.

**No new files.**

---

## Chunk 1: View struct fields + populate helpers

### Task 1: Add cached fields to `WalletView` and `BankingView`

**Files:**
- Modify: `src/display/web.rs:270-282` (`WalletView` and `BankingView` definitions)

- [ ] **Step 1: Edit `WalletView`**

In `src/display/web.rs` at line 270:

```rust
struct WalletView {
    name: String,
    #[allow(dead_code)]
    company: String,
    address: String,
    chain: String,
    cached_total_usd: Option<f64>,
    cached_native_symbol: Option<String>,
    cached_native_balance: Option<f64>,
}
```

- [ ] **Step 2: Edit `BankingView`**

Around line 278:

```rust
struct BankingView {
    name: String,
    #[allow(dead_code)]
    company: String,
    account_id: String,
    service: String,
    cached_total_usd: Option<f64>,
    cached_native_symbol: Option<String>,
    cached_native_balance: Option<f64>,
}
```

- [ ] **Step 3: Verify build**

```bash
cargo build 2>&1 | tail -20
```

Expected: build fails. The `index` handler at `src/display/web.rs:2280` constructs both views without the new fields. Errors will name the missing fields.

Do NOT fix the handler yet — that's Task 3.

- [ ] **Step 4: Hold off on commit**

Don't commit yet. This task and Task 2 together produce a meaningful, build-green commit. After Task 2 the build returns to green.

---

### Task 2: Add `populate_wallet_view` and `populate_banking_view` helpers with tests

**Files:**
- Modify: `src/display/web.rs` — add two free functions near other utility helpers (e.g., after `resolve_dashboard_filter`); add unit tests to the `#[cfg(test)] mod tests {}` block.

- [ ] **Step 1: Add the helpers**

Place near `resolve_dashboard_filter` (use `grep -n "fn resolve_dashboard_filter" src/display/web.rs` to locate):

```rust
/// Build a WalletView for a wallet, populating cached_* fields from
/// the cache if an entry exists under wallet.name. Cache age is not
/// checked - if there's an entry, we render it.
fn populate_wallet_view(
    wallet: &crate::storage::WalletAddress,
    cache: &crate::services::cache::BalanceCache,
) -> WalletView {
    let (total, sym, bal) = match cache.get_balance(&wallet.name, u64::MAX) {
        Some(c) => (
            c.total_usd_value,
            Some(c.native_symbol.clone()),
            Some(c.native_balance),
        ),
        None => (None, None, None),
    };
    WalletView {
        name: wallet.name.clone(),
        company: wallet.company.clone(),
        address: wallet.address.clone(),
        chain: wallet.chain.display_name().to_string(),
        cached_total_usd: total,
        cached_native_symbol: sym,
        cached_native_balance: bal,
    }
}

/// Build a BankingView for an account, same caching shape as wallets.
fn populate_banking_view(
    account: &crate::storage::BankingAccount,
    cache: &crate::services::cache::BalanceCache,
) -> BankingView {
    let (total, sym, bal) = match cache.get_balance(&account.name, u64::MAX) {
        Some(c) => (
            c.total_usd_value,
            Some(c.native_symbol.clone()),
            Some(c.native_balance),
        ),
        None => (None, None, None),
    };
    BankingView {
        name: account.name.clone(),
        company: account.company.clone(),
        account_id: account.account_id.clone(),
        service: account.service.display_name().to_string(),
        cached_total_usd: total,
        cached_native_symbol: sym,
        cached_native_balance: bal,
    }
}
```

- [ ] **Step 2: Write unit tests**

In the `#[cfg(test)] mod tests {}` block at the bottom of `src/display/web.rs`, after the existing tests, add:

```rust
#[test]
fn test_populate_wallet_view_pulls_from_cache_when_present() {
    use crate::services::cache::{BalanceCache, CachedBalance};
    use crate::storage::{Chain, WalletAddress};

    let mut cache = BalanceCache::new();
    cache.set_balance(
        "WalletA",
        CachedBalance {
            name: "WalletA".to_string(),
            address_or_id: "addr".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 3.5,
            native_usd_value: Some(350.0),
            tokens: vec![],
            total_usd_value: Some(350.0),
        },
    );

    let wallet = WalletAddress {
        name: "WalletA".to_string(),
        company: "Acme".to_string(),
        address: "addr".to_string(),
        chain: Chain::Solana,
    };

    let view = populate_wallet_view(&wallet, &cache);
    assert_eq!(view.name, "WalletA");
    assert_eq!(view.cached_total_usd, Some(350.0));
    assert_eq!(view.cached_native_symbol, Some("SOL".to_string()));
    assert_eq!(view.cached_native_balance, Some(3.5));
}

#[test]
fn test_populate_wallet_view_returns_none_fields_when_no_cache_entry() {
    use crate::services::cache::BalanceCache;
    use crate::storage::{Chain, WalletAddress};

    let cache = BalanceCache::new();
    let wallet = WalletAddress {
        name: "AbsentWallet".to_string(),
        company: "Acme".to_string(),
        address: "addr".to_string(),
        chain: Chain::Solana,
    };

    let view = populate_wallet_view(&wallet, &cache);
    assert_eq!(view.cached_total_usd, None);
    assert_eq!(view.cached_native_symbol, None);
    assert_eq!(view.cached_native_balance, None);
}

#[test]
fn test_populate_banking_view_pulls_from_cache_when_present() {
    use crate::services::cache::{BalanceCache, CachedBalance};
    use crate::storage::{BankingAccount, BankingService};

    let mut cache = BalanceCache::new();
    cache.set_balance(
        "BankA",
        CachedBalance {
            name: "BankA".to_string(),
            address_or_id: "acc_123".to_string(),
            chain_or_service: "Mercury Banking".to_string(),
            native_symbol: "USD".to_string(),
            native_balance: 1500.0,
            native_usd_value: Some(1500.0),
            tokens: vec![],
            total_usd_value: Some(1500.0),
        },
    );

    let account = BankingAccount {
        name: "BankA".to_string(),
        company: "Acme".to_string(),
        account_id: "acc_123".to_string(),
        service: BankingService::Mercury,
    };

    let view = populate_banking_view(&account, &cache);
    assert_eq!(view.cached_total_usd, Some(1500.0));
    assert_eq!(view.cached_native_symbol, Some("USD".to_string()));
    assert_eq!(view.cached_native_balance, Some(1500.0));
}

#[test]
fn test_populate_view_handles_cached_balance_with_none_total() {
    // Wallet has a balance but no USD price (total_usd_value is None).
    // The view should reflect that: native fields populated, total None.
    use crate::services::cache::{BalanceCache, CachedBalance};
    use crate::storage::{Chain, WalletAddress};

    let mut cache = BalanceCache::new();
    cache.set_balance(
        "WalletNoPriced",
        CachedBalance {
            name: "WalletNoPriced".to_string(),
            address_or_id: "addr".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 5.0,
            native_usd_value: None,
            tokens: vec![],
            total_usd_value: None,
        },
    );

    let wallet = WalletAddress {
        name: "WalletNoPriced".to_string(),
        company: "Acme".to_string(),
        address: "addr".to_string(),
        chain: Chain::Solana,
    };

    let view = populate_wallet_view(&wallet, &cache);
    assert_eq!(view.cached_total_usd, None);
    assert_eq!(view.cached_native_symbol, Some("SOL".to_string()));
    assert_eq!(view.cached_native_balance, Some(5.0));
}
```

- [ ] **Step 3: Verify build (still broken in the handler, but helpers compile)**

```bash
cargo build 2>&1 | tail -20
```

Expected: still failing at the `index` handler (Task 3 fixes it), but the new helpers and tests compile correctly.

- [ ] **Step 4: Hold off on commit**

Task 3 brings the build back to green. Commit then.

---

## Chunk 2: Wire `index` handler

### Task 3: Use helpers from `index` and add `compute_dashboard_metrics`

**Files:**
- Modify: `src/display/web.rs` — extend `IndexTemplate` at line 96; rewrite `index` handler at line 2280; add `compute_dashboard_metrics` helper.

- [ ] **Step 1: Extend `IndexTemplate`**

Find `struct IndexTemplate` around line 96 and add two fields:

```rust
struct IndexTemplate {
    // ... existing fields ...
    total_portfolio_usd: Option<f64>,
    last_refresh_human: Option<String>,
}
```

- [ ] **Step 2: Add `compute_dashboard_metrics` helper**

After `populate_banking_view`, add:

```rust
/// Sum cached total_usd_value across every wallet/account in the cache,
/// and produce a human-readable "Xm ago" for cache.last_full_refresh.
/// Returns (total_portfolio_usd, last_refresh_human). Both are Option:
/// total is None if no cached entry has a usd value; last_refresh is
/// None if the cache has never been refreshed.
fn compute_dashboard_metrics(
    cache: &crate::services::cache::BalanceCache,
) -> (Option<f64>, Option<String>) {
    let mut total_portfolio_usd: Option<f64> = None;
    for entry in cache.balances.values() {
        if let Some(v) = entry.data.total_usd_value {
            *total_portfolio_usd.get_or_insert(0.0) += v;
        }
    }

    let last_refresh_human = cache.last_full_refresh.map(|ts| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let elapsed = now.saturating_sub(ts);
        format!("{} ago", format_duration(Duration::from_secs(elapsed)))
    });

    (total_portfolio_usd, last_refresh_human)
}
```

- [ ] **Step 3: Add a unit test for `compute_dashboard_metrics`**

```rust
#[test]
fn test_compute_dashboard_metrics_sums_cached_totals() {
    use crate::services::cache::{BalanceCache, CachedBalance};

    let mut cache = BalanceCache::new();
    cache.set_balance(
        "A",
        CachedBalance {
            name: "A".to_string(),
            address_or_id: "a".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 1.0,
            native_usd_value: Some(100.0),
            tokens: vec![],
            total_usd_value: Some(100.0),
        },
    );
    cache.set_balance(
        "B",
        CachedBalance {
            name: "B".to_string(),
            address_or_id: "b".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 2.0,
            native_usd_value: Some(200.0),
            tokens: vec![],
            total_usd_value: Some(200.0),
        },
    );
    // One entry with no price - excluded from the sum
    cache.set_balance(
        "C",
        CachedBalance {
            name: "C".to_string(),
            address_or_id: "c".to_string(),
            chain_or_service: "Solana".to_string(),
            native_symbol: "SOL".to_string(),
            native_balance: 3.0,
            native_usd_value: None,
            tokens: vec![],
            total_usd_value: None,
        },
    );

    let (total, _last) = compute_dashboard_metrics(&cache);
    assert_eq!(total, Some(300.0));
}

#[test]
fn test_compute_dashboard_metrics_returns_none_for_empty_cache() {
    use crate::services::cache::BalanceCache;
    let cache = BalanceCache::new();
    let (total, last) = compute_dashboard_metrics(&cache);
    assert_eq!(total, None);
    assert_eq!(last, None);
}
```

- [ ] **Step 4: Rewrite the `index` handler**

Find `async fn index(` at `src/display/web.rs:2280`. Make three changes:

1. Drop the underscore on `State(_state)` so the extractor is used:

```rust
async fn index(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
```

2. Acquire a read lock on the cache near the top of the function, right after `book` is loaded:

```rust
let cache = state.cache.read().await;
```

3. Replace the two inline `entry.0.push(WalletView { ... })` and `entry.1.push(BankingView { ... })` constructions (in the loops over `book.addresses` and `book.banking_accounts`) with calls to the helpers:

```rust
for w in &book.addresses {
    let company_name = if w.company.is_empty() {
        "Uncategorized".to_string()
    } else {
        w.company.clone()
    };
    let entry = company_map.entry(company_name).or_insert((vec![], vec![]));
    entry.0.push(populate_wallet_view(w, &cache));
}

for a in &book.banking_accounts {
    let company_name = if a.company.is_empty() {
        "Uncategorized".to_string()
    } else {
        a.company.clone()
    };
    let entry = company_map.entry(company_name).or_insert((vec![], vec![]));
    entry.1.push(populate_banking_view(a, &cache));
}
```

4. Compute the new template fields and pass them into the `IndexTemplate` literal at the end:

```rust
let (total_portfolio_usd, last_refresh_human) = compute_dashboard_metrics(&cache);

Html(
    IndexTemplate {
        companies,
        wallet_count,
        bank_count,
        filter,
        active_nav,
        has_visible_rows,
        total_portfolio_usd,
        last_refresh_human,
    }
    .render()
    .unwrap_or_else(|e| format!("Template error: {}", e)),
)
```

- [ ] **Step 5: Verify build**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build (or only a `dead_code` warning on `total_portfolio_usd` / `last_refresh_human` if Askama hasn't been told to use them yet — that warning clears in Task 4 when the template references them). Don't silence with `#[allow(dead_code)]`.

- [ ] **Step 6: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: 53 passing (52 baseline + 5 new tests: 3 from Task 2 + 2 from Task 3 Step 3 — but actually 4 from Task 2 and 2 from Task 3 = 6, so 58. Run and confirm the actual count, then commit.).

Wait — Task 2 has 4 tests, Task 3 has 2 tests. Total new = 6. Baseline = 52. Expected = 58. Adjust the commit message in the next step to match the real count.

- [ ] **Step 7: Commit (covers Tasks 1, 2, and 3 together since they're a single bisect-safe unit)**

```bash
cargo fmt
git add src/display/web.rs
git commit -m "feat: dashboard reads cached balances from state.cache

WalletView and BankingView gain cached_total_usd, cached_native_symbol,
cached_native_balance (all Option). IndexTemplate gains
total_portfolio_usd and last_refresh_human. The index handler now reads
state.cache and threads the cached values into the template via two
small helpers (populate_wallet_view, populate_banking_view) plus a
compute_dashboard_metrics helper. Helpers are unit-tested in isolation.

Build is green again; template wiring in the next commit."
```

---

## Chunk 3: Template + JS wiring

### Task 4: Wire `templates/index.html` to consume new fields

**Files:**
- Modify: `templates/index.html` — lines 43 (Total Portfolio Value card), 55 (Last Refresh card), 147-149 (wallet Balance/Price/Value), 197-199 (banking Balance/Price/Value), 414-417 (`updateRefreshTime` function).

- [ ] **Step 1: Total Portfolio Value card (line 43)**

Replace:

```html
<div class="metric-value" id="total-balance">--</div>
```

With:

```html
<div class="metric-value" id="total-balance">
    {% if let Some(v) = total_portfolio_usd %}${{ v|format_usd }}{% else %}--{% endif %}
</div>
```

The Askama `{% if let Some(x) = expr %}` syntax is supported in Askama 0.12 (per `Cargo.toml`).

- [ ] **Step 2: Last Refresh card (line 55)**

Replace:

```html
<div class="metric-value" id="last-refresh">--</div>
```

With:

```html
<div class="metric-value" id="last-refresh">
    {% if let Some(human) = last_refresh_human %}{{ human }}{% else %}--{% endif %}
</div>
```

- [ ] **Step 3: Wallet row Balance/Price/Value cells (lines 147-149)**

Currently:

```html
<td class="amount">--</td>
<td class="amount">--</td>
<td class="amount">--</td>
```

Replace with:

```html
<td class="amount">
    {% if let Some(bal) = wallet.cached_native_balance %}
        {{ bal|format_amount }}
        {% if let Some(sym) = wallet.cached_native_symbol %}{{ sym }}{% endif %}
    {% else %}--{% endif %}
</td>
<td class="amount">--</td>
<td class="amount {% if wallet.cached_total_usd.is_some() %}positive{% endif %}">
    {% if let Some(v) = wallet.cached_total_usd %}${{ v|format_usd }}{% else %}--{% endif %}
</td>
```

The middle "Price" cell stays `--` (out of scope per spec).

- [ ] **Step 4: Banking row Balance/Price/Value cells (lines 197-199)**

Apply the symmetric change, swapping `wallet` for `account` in the field paths.

- [ ] **Step 5: Update `updateRefreshTime()` JS (lines 414-418)**

Replace:

```js
function updateRefreshTime() {
    var now = new Date();
    var timeStr = now.toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'});
    document.getElementById('last-refresh').textContent = timeStr;
}
```

With:

```js
function updateRefreshTime() {
    // Match the server-rendered "Xm ago" format. After a manual refresh
    // we've just refreshed, so write "just now".
    document.getElementById('last-refresh').textContent = 'just now';
}
```

This keeps the format consistent across server-rendered initial state and post-Refresh-All updates.

- [ ] **Step 6: Verify build**

```bash
cargo build 2>&1 | tail -10
```

Expected: clean build. Askama compiles templates at build time so any field-reference typo surfaces here. The `dead_code` warning on `total_portfolio_usd`/`last_refresh_human` (if it appeared in Task 3) is now gone.

- [ ] **Step 7: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: 58 passing (no new tests in this task; template-only).

- [ ] **Step 8: Commit**

```bash
git add templates/index.html
git commit -m "feat: render cached balances in dashboard rows and metric cards

The wallet/account rows show cached Balance + Value when state.cache
has data for that name; otherwise '--'. The Total Portfolio Value and
Last Refresh metric cards are populated server-side from the same cache.

The existing JS updateRefreshTime() now writes 'just now' to
#last-refresh after Refresh All, replacing the previous wall-clock
toLocaleTimeString output so the format matches the server-rendered
'Xm ago' on page-load-from-navigation."
```

---

## Chunk 4: Verification + PR

### Task 5: Quality gates and PR

- [ ] **Step 1: Format**

```bash
cargo fmt
```

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets 2>&1 | tail -10
```

Expected: warning count matches baseline (13 pre-existing dead-code on legacy modules). No new warnings.

- [ ] **Step 3: Full test suite**

```bash
cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: 58 passing (52 baseline + 6 new). Lock the actual count once you run; the plan's prediction may drift.

- [ ] **Step 4: Manual smoke test**

```bash
cargo build --release
./target/release/gringotts serve --port 3000
```

In a browser at `http://localhost:3000`:

1. Click **Refresh All**. Confirm the holdings table shows live numbers AND the wallet/account rows above also show Balance/Value matching the rendered table.
2. Click **Settings** in the sidebar. Confirm Settings loads.
3. Click **Dashboard** in the sidebar (or browser back). Verify the wallet/account rows STILL show their balance/value (not `--`), and the Total Portfolio Value / Last Refresh cards still show their values.
4. Restart the server (`Ctrl+C` and re-launch). Reload the dashboard. Verify persistence across restart — the values still display because `SharedCache::persist` writes to disk and `BalanceCache::load()` reads on startup.
5. Click Refresh All again. Confirm `#last-refresh` flips to `"just now"` (matching the format, not a wall-clock).

- [ ] **Step 5: Commit any fmt/clippy fixes**

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: cargo fmt / clippy cleanup"
```

- [ ] **Step 6: Open the PR**

```bash
git push -u origin feat/dashboard-cache-persistence
```

Write PR body to `/tmp/pr-body.md`:

```markdown
## Summary
- Dashboard now reads `state.cache` on every render and populates wallet/account row Balance + Value cells with whatever the cache holds
- The Total Portfolio Value and Last Refresh metric cards are now server-rendered from cached data, so they survive navigation and server restart
- `updateRefreshTime()` JS rewritten to write `"just now"` so the format matches the server-rendered `"Xm ago"` on page-load-from-navigation

Spec: `docs/superpowers/specs/2026-05-13-dashboard-cache-persistence-design.md`

## How it works
- `index` handler reads `state.cache` and uses two new helpers (`populate_wallet_view`, `populate_banking_view`) to build views with optional cached fields, plus `compute_dashboard_metrics` for the portfolio total and "Xm ago" timestamp.
- Template conditionally renders cached values when present, falls back to `--` when not (i.e., wallet hasn't been queried yet).
- Refresh All still overwrites the cache and re-renders the holdings card; the dashboard rows ALSO refresh on the next navigation back (the rendered initial values change).
- Staleness policy: no max-age. The Last Refresh card communicates how fresh the numbers are.

## Test plan
- [x] `cargo fmt --check` clean
- [x] `cargo clippy --all-targets` no new warnings
- [x] `cargo test` — 58 passing (52 baseline + 6 new)
- [ ] Manual smoke: navigate dashboard ⇄ Settings, verify rows persist; server restart, verify rows still display

## Out of scope (per spec)
- Per-asset price column (still `--`)
- Showing the full token list on the dashboard
- Replacing Refresh All with a passive background-only model
- Pruning stale entries from the cache

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

```bash
gh pr create --base main --title "feat: dashboard reads cached balances; persists across navigation" --body-file /tmp/pr-body.md
rm -f /tmp/pr-body.md
```

---
