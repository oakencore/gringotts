# Dashboard Cache Persistence — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

The web UI's dashboard always renders `--` in the Balance / Price / Value cells until the user clicks **Query All**. The Query All HTMX call writes results into `state.cache`, but when the user navigates away (Settings, Transactions) and back, the dashboard is re-rendered from scratch and shows `--` again. The cache holds the data; the `index` handler at `src/display/web.rs:2377` simply doesn't read it (its `State(_state)` extractor is unused).

Result: the user has to re-run Query All every time they revisit the dashboard, even though the cached numbers are still available — and may even be authoritative if the background refresh task has run since the last manual refresh.

## Goal

Make the dashboard a stateful surface that reflects whatever the cache currently holds, with a small timestamp so the user knows how fresh the displayed numbers are.

## Scope

- `src/display/web.rs` — `index` handler reads `state.cache`; `WalletView` / `BankingView` gain three optional display fields; `IndexTemplate` gains a `last_refresh_human: Option<String>` field.
- `templates/index.html` — conditionally render the cached balance/value cells; add a "Last refreshed: Xm ago" banner near the page subtitle.

**Out of scope:**

- Per-asset price display in the dashboard's middle "Price" column. Would require additional lookups not currently performed; leaving as `--` for this PR.
- Showing the full token list on the dashboard. Tokens beyond the native asset live in the per-wallet detail view (`single_balance.html`) and are not surfaced on the dashboard table.
- Replacing Query All with a passive "refresh in background" model. Query All remains as an explicit user-triggered refresh.
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

Both view structs gain three optional fields populated from the cache:

```rust
struct WalletView {
    name: String,
    company: String,
    address: String,
    chain: String,
    cached_total_usd: Option<f64>,
    cached_native_symbol: Option<String>,
    cached_native_balance: Option<f64>,
}

struct BankingView {
    name: String,
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

`CachedBalance.total_usd_value` is already `Option<f64>` in `src/services/cache.rs`, so the `None`-when-no-price flow threads through naturally. Tokens beyond the native asset are not pulled into the dashboard view (out of scope per Scope section).

### Section 3 — `templates/index.html` cell rendering

The wallet row's Balance / Price / Value cells at `templates/index.html:130-138` currently render `--`. Swap them for conditional cells:

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

Banking rows (around line 178) get the symmetric treatment.

**Askama version check:** `{% if let Some(x) = expr %}` is supported in Askama 0.12+. The implementation plan will verify the project's Askama version and, if older, fall back to `cached_native_balance.is_some()` + `.unwrap()` pairs.

### Section 4 — "Last refreshed" banner

`IndexTemplate` gains `last_refresh_human: Option<String>`. Handler:

```rust
let last_refresh_human = cache.last_full_refresh.map(|ts| {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(ts);
    format!("{} ago", format_duration(Duration::from_secs(elapsed)))
});
```

`format_duration` is the existing helper at `src/display/web.rs:498` that produces strings like `"3m"`, `"4h"`, `"2d"`.

Template: below the existing `.page-subtitle` (around `templates/index.html:8`):

```html
{% if let Some(human) = last_refresh_human %}
<span class="cache-timestamp">Last refreshed: {{ human }}</span>
{% endif %}
```

CSS (added to the existing `<style>` block):

```css
.cache-timestamp {
    font-size: 12px;
    color: var(--text-disabled);
    font-family: var(--font-mono);
    margin-left: 12px;
}
```

The banner renders nothing when the cache has never been refreshed (`cache.last_full_refresh.is_none()`) — fresh server starts fall through to the existing subtitle alone.

### Section 5 — Testing

**Unit test** (in the `src/display/web.rs` test module):

The cleanest target is a small extracted helper `populate_wallet_view(wallet: &WalletAddress, cache: &BalanceCache) -> WalletView` (and a symmetric `populate_banking_view`). The test asserts:

1. Given a `WalletAddress { name: "WalletA", ... }` and a `BalanceCache` populated with a `CachedBalance` under key `"WalletA"`, the returned `WalletView` has `cached_total_usd = Some(...)`, `cached_native_symbol = Some(...)`, `cached_native_balance = Some(...)` matching the cached entry.
2. Given the same wallet but with NO cache entry under that name, the returned `WalletView` has all three `cached_*` fields as `None`.

End-to-end test of the handler itself is skipped because it depends on `AddressBook::load()` reading from `~/.gringotts/`.

**Manual smoke:**
- Run Query All; verify Balance and Value cells populate with cached numbers.
- Navigate to Settings, then back to dashboard; verify cells still show numbers (not `--`).
- Verify "Last refreshed: Xm ago" banner appears below the subtitle and updates appropriately.
- Restart the server (cache persists to disk via `SharedCache::persist`); reload dashboard; verify numbers still display.

**Quality gates** before opening a PR: `cargo fmt`, `cargo clippy` (no new warnings), `cargo test` (expect 53 passing — 52 baseline + 2 new tests, but the cardinality depends on how the helper is split; the plan task will lock the exact count).
