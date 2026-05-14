# Dashboard Cache Persistence — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

The web UI's dashboard always renders `--` in three places until the user clicks **Refresh All**:

- Each wallet/account row's Balance / Price / Value cells.
- The "Total Portfolio Value" metric card.
- The "Last Refresh" metric card.

The Refresh All HTMX call writes results into `state.cache`, but when the user navigates away (Settings, Transactions) and back, the dashboard is re-rendered from scratch and all three surfaces show `--` again. The cache holds the data; the `index` handler at `src/display/web.rs:2280` simply doesn't read it (its `State(_state)` extractor is unused).

Result: the user has to re-run Refresh All every time they revisit the dashboard, even though the cached numbers are still available — and may even be authoritative if the background refresh task has run since the last manual refresh.

## Goal

Make the dashboard a stateful surface that reflects whatever the cache currently holds, with a small timestamp so the user knows how fresh the displayed numbers are.

## Scope

- `src/display/web.rs` — `index` handler reads `state.cache`; `WalletView` / `BankingView` gain three optional display fields; `IndexTemplate` gains `total_portfolio_usd: Option<f64>` and `last_refresh_human: Option<String>` fields.
- `templates/index.html` — conditionally render cached balance/value in wallet/account rows; populate the existing "Total Portfolio Value" and "Last Refresh" metric cards from the server-rendered template fields instead of letting JS be the only writer.

**Out of scope:**

- Per-asset price display in the dashboard's middle "Price" column. Would require additional lookups not currently performed; leaving as `--` for this PR.
- Showing the full token list on the dashboard. Tokens beyond the native asset live in the per-wallet detail view (`single_balance.html`) and are not surfaced on the dashboard table.
- Replacing Refresh All with a passive "refresh in background" model. Refresh All remains as an explicit user-triggered refresh.
- Pruning stale entries from the cache. Cache durability is the existing `SharedCache::persist` mechanism; no changes.

## Design

### Section 1 — Data flow

The `index` handler becomes:

```rust
async fn index(
    State(state): State<Arc<AppState>>,   // _state -> state (drop underscore)
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
    let book = ...;
    let cache = state.cache.read().await;
    // existing wallet/banking grouping, augmented to look up each name in the cache.
}
```

The cache lookup uses `cache.get_balance(&name, u64::MAX)` — `u64::MAX` as the staleness limit means "any age", which matches the chosen UX policy (show whatever the cache has, signal freshness via the timestamp).

A separate `last_refresh_human: Option<String>` is computed from `cache.last_full_refresh` to drive the page-level timestamp banner (see Section 4).

### Section 2 — `WalletView` / `BankingView` field additions

Both view structs (currently `WalletView` at `src/display/web.rs:270`, `BankingView` at `:278`) gain three optional fields populated from the cache. The existing `#[allow(dead_code)]` on the `company` field is preserved (cache lookup is by `name`, not `company`).

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

Handler populates each from `cache.get_balance(&name, u64::MAX)`:

```rust
let cached = cache.get_balance(&w.name, u64::MAX);
let (total, sym, bal) = match cached {
    Some(c) => (c.total_usd_value, Some(c.native_symbol.clone()), Some(c.native_balance)),
    None => (None, None, None),
};
entry.0.push(WalletView { /* ... */ cached_total_usd: total, cached_native_symbol: sym, cached_native_balance: bal });
```

`CachedBalance.total_usd_value` is already `Option<f64>` in `src/services/cache.rs:49`, so the `None`-when-no-price flow threads through naturally. Tokens beyond the native asset are not pulled into the dashboard view (out of scope per Scope section).

### Section 3 — `templates/index.html` cell rendering

The wallet row's Balance / Price / Value cells at `templates/index.html:147-149` currently render `--` (three single-line `<td class="amount">--</td>` rows). Swap them for conditional cells:

```html
<td class="amount">
    {% if let Some(bal) = wallet.cached_native_balance %}
        {{ bal|format_amount }}
        {% if let Some(sym) = wallet.cached_native_symbol %}{{ sym }}{% endif %}
    {% else %}
        --
    {% endif %}
</td>
<td class="amount">--</td>   <!-- Price stays placeholder; no per-asset price on dashboard -->
<td class="amount {% if wallet.cached_total_usd.is_some() %}positive{% endif %}">
    {% if let Some(v) = wallet.cached_total_usd %}
        ${{ v|format_usd }}
    {% else %}
        --
    {% endif %}
</td>
```

Banking rows (`templates/index.html:197-199`) get the symmetric treatment.

**Askama version check:** `{% if let Some(x) = expr %}` is supported in Askama 0.12+. The implementation plan will verify the project's Askama version and, if older, fall back to `cached_native_balance.is_some()` + `.unwrap()` pairs.

### Section 4 — Populate existing metric cards from the server

The dashboard already has two metric cards that render `--` until JS updates them after Refresh All:

- `templates/index.html:43` — Total Portfolio Value, `<div class="metric-value" id="total-balance">--</div>`.
- `templates/index.html:55` — Last Refresh, `<div class="metric-value" id="last-refresh">--</div>`.

Both `id`s are addressed by `updateRefreshTime()` (at line 414) which the existing `/balances` HTMX swap triggers via `hx-on::after-request`. JS remains the writer for the live post-Refresh-All update.

This task adds a **server-side initial value**: on page render, the handler populates each card with cached data so the dashboard "remembers" across navigation. After Refresh All, JS overwrites both — same as today.

**`IndexTemplate` adds two fields:**

```rust
struct IndexTemplate {
    // ... existing fields ...
    total_portfolio_usd: Option<f64>,
    last_refresh_human: Option<String>,
}
```

**Handler computation:**

```rust
// Sum cached total_usd_value across all wallets and banking accounts.
let mut total_portfolio_usd: Option<f64> = None;
for entry in cache.balances.values() {
    if let Some(v) = entry.data.total_usd_value {
        *total_portfolio_usd.get_or_insert(0.0) += v;
    }
}

// Format "Xm ago" / "Xh ago" / "Xd ago" via the existing format_duration helper
let last_refresh_human = cache.last_full_refresh.map(|ts| {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(ts);
    format!("{} ago", format_duration(Duration::from_secs(elapsed)))
});
```

`format_duration` is the existing helper at `src/display/web.rs:467` producing strings like `"3m"`, `"4h"`, `"2d"`. `cache.balances` is `HashMap<String, CacheEntry<CachedBalance>>` per `src/services/cache.rs:69`; the `.data.total_usd_value` access matches what `cache_status` uses today.

**Template changes** to `templates/index.html`:

Line 43 (Total Portfolio Value):
```html
<div class="metric-value" id="total-balance">
    {% if let Some(v) = total_portfolio_usd %}${{ v|format_usd }}{% else %}--{% endif %}
</div>
```

Line 55 (Last Refresh):
```html
<div class="metric-value" id="last-refresh">
    {% if let Some(human) = last_refresh_human %}{{ human }}{% else %}--{% endif %}
</div>
```

**JS update format alignment.** The existing `updateRefreshTime()` at `templates/index.html:414` writes a wall-clock string (`Date.prototype.toLocaleTimeString`) to `#last-refresh` after Refresh All. The server now renders `"3m ago"` style. Without changing the JS, the card would jump from `"3m ago"` to `"10:30:45 AM"` on click — visually jarring. Update `updateRefreshTime()` to write `"just now"` (since it fires immediately after a successful refresh) so the format stays consistent with the server's relative-time format.

```js
function updateRefreshTime() {
    document.getElementById('last-refresh').textContent = 'just now';
}
```

The polling that runs every minute can simply not update the text — relative time updates would need a server round-trip and aren't worth the complexity. The Last Refresh card stays at `"just now"` until either the next Refresh All fires or the user navigates and returns (server then renders `"Xm ago"`).

**`#total-balance` is updated via HTMX OOB, not JS.** `templates/balances.html:1` carries `<div id="total-balance" hx-swap-oob="innerHTML">${{ total_usd|format_usd }}</div>`, which overwrites the dashboard's `#total-balance` card whenever the `/balances` endpoint responds. Server-rendered initial value (`${{ v|format_usd }}`) uses the same `format_usd` filter, so the formatting matches byte-for-byte across the initial render and the post-Refresh-All update.

No new CSS rules required (both cards already use `.metric-value`).

### Section 5 — Testing

**Unit test** (in the `src/display/web.rs` test module):

The cleanest target is a small extracted helper `populate_wallet_view(wallet: &WalletAddress, cache: &BalanceCache) -> WalletView` (and a symmetric `populate_banking_view`). The test asserts:

1. Given a `WalletAddress { name: "WalletA", ... }` and a `BalanceCache` populated with a `CachedBalance` under key `"WalletA"`, the returned `WalletView` has `cached_total_usd = Some(...)`, `cached_native_symbol = Some(...)`, `cached_native_balance = Some(...)` matching the cached entry.
2. Given the same wallet but with NO cache entry under that name, the returned `WalletView` has all three `cached_*` fields as `None`.

End-to-end test of the handler itself is skipped because it depends on `AddressBook::load()` reading from `~/.gringotts/`.

**Manual smoke:**
- Run Refresh All; verify Balance and Value cells populate with cached numbers.
- Navigate to Settings, then back to dashboard; verify Balance/Value cells still show numbers (not `--`).
- Verify the Last Refresh metric card shows `"Xm ago"` on page load and `"just now"` immediately after Refresh All.
- Verify the Total Portfolio Value metric card shows the cached portfolio sum on page load and updates to the live value after Refresh All (via the existing HTMX OOB swap from `balances.html`).
- Restart the server (cache persists to disk via `SharedCache::persist`); reload dashboard; verify numbers still display.

**Quality gates** before opening a PR: `cargo fmt`, `cargo clippy` (no new warnings), `cargo test` (expect 53 passing — 52 baseline + 2 new tests, but the cardinality depends on how the helper is split; the plan task will lock the exact count).
