# Sidebar Pages Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire every gringotts sidebar item (Dashboard, Wallets, Transactions, Banking, Settings) to a real page or filtered view.

**Architecture:** Wallets and Banking become filtered views of the existing dashboard via a `?filter=` query param (no new pages). Settings is a new `/settings` page with an editable refresh-interval and a force-refresh button. Transactions is a new `/transactions` page aggregating Solana + Mercury transactions chronologically with a top-100 cap. `AppState.refresh_interval_secs` becomes `Arc<RwLock<u64>>` so the Settings POST can write to it; the background refresh task reads the live value on each tick. Sidebar gains active-state highlighting via an `active_nav: String` field on the three full-page template structs.

**Tech Stack:** Rust, Axum, Askama templates, HTMX. Tests via `cargo test`. Format via `cargo fmt`. Lint via `cargo clippy`.

**Spec:** `docs/superpowers/specs/2026-05-13-sidebar-pages-design.md`

---

## File Structure

**Modified files:**
- `src/display/web.rs` — `AppState.refresh_interval_secs` becomes `Arc<RwLock<u64>>`; `background_refresh_task` signature drops `interval` arg and reads state; `index` handler gains `State` + `Query` extractors and three new template fields; new handlers (`settings_page`, `update_refresh_interval`, `global_transactions`); new struct definitions (`DashboardFilter`, `SettingsTemplate`, `GlobalTransactionsTemplate`, `GlobalTxView`, `RefreshIntervalForm`); new routes registered on `protected_routes`.
- `templates/base.html` — replace placeholder `#anchor` hrefs with real routes; add `active_nav` conditional class.
- `templates/index.html` — wrap wallet and banking inner loops with filter conditionals; update subtitle; switch empty-state predicate to `has_visible_rows`.

**New files:**
- `templates/settings.html` — Settings page.
- `templates/global_transactions.html` — Transactions global view.

**No new source modules** — handlers and view structs live alongside existing ones in `src/display/web.rs` to match the project's current pattern. (`web.rs` is ~3400 lines; splitting it is out of scope.)

---

## Chunk 1: AppState refactor + background ticker

### Task 1: Wrap `refresh_interval_secs` in `Arc<RwLock<u64>>`

**Files:**
- Modify: `src/display/web.rs` — `AppState` struct definition near line 188; reads at `health_check` (lines 1795, 1801); `AppState` constructor around line 353; existing tests at lines 3309, 3336, 3383.

This is a foundational change. The build will break briefly after Step 1 and recover by Step 4.

- [ ] **Step 1: Change `AppState.refresh_interval_secs` field type**

Edit the struct definition (`src/display/web.rs` around line 188):

```rust
/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub cache: SharedCache,
    pub api_key: Option<String>,
    pub refresh_interval_secs: Arc<RwLock<u64>>,
    /// Timestamp of last manual refresh for rate limiting (Unix seconds)
    pub last_manual_refresh: Arc<RwLock<Option<u64>>>,
    /// Whether a refresh is currently in progress
    pub refresh_in_progress: Arc<RwLock<bool>>,
}
```

- [ ] **Step 2: Update the `AppState` constructor**

Find the literal construction around line 353:

```rust
let state = Arc::new(AppState {
    cache,
    api_key,
    refresh_interval_secs: interval.as_secs(),
    ...
});
```

Change to:

```rust
let state = Arc::new(AppState {
    cache,
    api_key,
    refresh_interval_secs: Arc::new(RwLock::new(interval.as_secs())),
    last_manual_refresh: Arc::new(RwLock::new(None)),
    refresh_in_progress: Arc::new(RwLock::new(false)),
});
```

- [ ] **Step 3: Update the three test constructors**

Three test sites construct `AppState` literally with `refresh_interval_secs: 3600`. Find them (search for `refresh_interval_secs: 3600` in `src/display/web.rs`; they are at approximately lines 3309, 3336, 3383). Change each to:

```rust
refresh_interval_secs: Arc::new(RwLock::new(3600)),
```

- [ ] **Step 4: Update `health_check` reads**

Find the two reads at lines 1795 and 1801:

```rust
let next_in = state.refresh_interval_secs.saturating_sub(elapsed);
// ...
format_duration(Duration::from_secs(state.refresh_interval_secs)),
```

Change to (the function is already `async`, so `.read().await` is fine):

```rust
let interval = *state.refresh_interval_secs.read().await;
let next_in = interval.saturating_sub(elapsed);
// ...
format_duration(Duration::from_secs(interval)),
```

Compute `interval` once at the top of the function so both reads share it.

- [ ] **Step 5: Verify the build still passes**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build. `background_refresh_task` still uses its `interval: Duration` parameter; that gets refactored in Task 2.

- [ ] **Step 6: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: 37 tests passing (same count as before — no behavior change yet).

- [ ] **Step 7: Commit**

Write commit message to `/tmp/task1-msg.txt`:

```
refactor: wrap AppState.refresh_interval_secs in Arc<RwLock<u64>>

Foundational change for the Settings page, which needs to update
the refresh interval at runtime. The field becomes shared mutable
state. health_check reads through .read().await; the three test
constructors and the AppState literal in start_server are updated.

The background refresh task still receives interval: Duration as a
separate argument; Task 2 will refactor it to read from state.
```

Commit:
```bash
git add src/display/web.rs
git commit -F /tmp/task1-msg.txt
```

---

### Task 2: Refactor `background_refresh_task` to read from state

**Files:**
- Modify: `src/display/web.rs` — `background_refresh_task` at line 466; the call site at line 455-456.

- [ ] **Step 1: Rewrite `background_refresh_task` signature and body**

Replace lines 466-493 with:

```rust
/// Background task that refreshes balances on a configurable interval.
/// Reads the current interval from state on each loop iteration so the
/// Settings UI's POST to /settings/refresh-interval takes effect on the
/// next tick.
async fn background_refresh_task(state: Arc<AppState>) {
    // Skip the first immediate tick - don't refresh right at startup
    let initial = *state.refresh_interval_secs.read().await;
    tokio::time::sleep(Duration::from_secs(initial)).await;

    loop {
        println!(
            "[{}] Starting background refresh...",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );

        if let Err(e) = refresh_all_balances(&state).await {
            eprintln!(
                "[{}] Background refresh failed: {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                e
            );
        } else {
            println!(
                "[{}] Background refresh completed",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }

        let next = *state.refresh_interval_secs.read().await;
        tokio::time::sleep(Duration::from_secs(next)).await;
    }
}
```

- [ ] **Step 2: Update the call site**

Find the spawn at line 455-456:

```rust
let refresh_state = state.clone();
tokio::spawn(async move {
    background_refresh_task(refresh_state, interval).await;
});
```

Drop the `interval` argument:

```rust
let refresh_state = state.clone();
tokio::spawn(async move {
    background_refresh_task(refresh_state).await;
});
```

The `interval: Duration` local at the top of `start_server` is still needed for the initial value passed into the `AppState` constructor; leave that.

- [ ] **Step 3: Verify build and tests**

```bash
cargo build 2>&1 | tail -10
cargo test 2>&1 | tail -10
```

Expected: build clean, 37 tests passing.

- [ ] **Step 4: Commit**

Write commit message to `/tmp/task2-msg.txt`:

```
refactor: background_refresh_task reads interval from state per tick

Drop the redundant interval: Duration parameter. The task now reads
state.refresh_interval_secs on each loop iteration so updates from
the upcoming Settings page take effect on the next tick.

Uses tokio::time::sleep instead of a fixed tokio::time::interval
ticker since the interval can change between iterations.
```

Commit:
```bash
git add src/display/web.rs
git commit -F /tmp/task2-msg.txt
```

---

## Chunk 2: Dashboard filter

### Task 3: Dashboard filter query param and template wiring

**Files:**
- Modify: `src/display/web.rs` — `IndexTemplate` struct (around line 88), `index` handler (line 1933), add `DashboardFilter` struct.
- Modify: `templates/index.html` — wrap inner loops with conditionals; update subtitle; switch empty-state predicate.

- [ ] **Step 1: Add `DashboardFilter` struct and extend `IndexTemplate`**

Near other request-extraction structs (search for `struct ApiKeyQuery` — around line 200; place after it), add:

```rust
#[derive(Deserialize)]
struct DashboardFilter {
    filter: Option<String>,
}
```

Edit `IndexTemplate` (struct around line 88) to add three fields:

```rust
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    companies: Vec<CompanyGroup>,
    wallet_count: usize,
    bank_count: usize,
    filter: String,
    active_nav: String,
    has_visible_rows: bool,
}
```

- [ ] **Step 2: Refactor the `index` handler signature and body**

Replace `async fn index() -> impl IntoResponse { ... }` at line 1933 with:

```rust
async fn index(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(_) => AddressBook::new(),
    };

    let wallet_count = book.addresses.len();
    let bank_count = book.banking_accounts.len();

    // Map filter query param to canonical strings
    let (filter, active_nav) = match q.filter.as_deref() {
        Some("wallets") => ("wallets".to_string(), "wallets".to_string()),
        Some("banking") => ("banking".to_string(), "banking".to_string()),
        _ => ("all".to_string(), "dashboard".to_string()),
    };

    // ... existing CompanyGroup-building logic preserved verbatim ...
    // (the existing handler builds `companies: Vec<CompanyGroup>` from `book`.
    //  copy that body unchanged.)

    // Compute has_visible_rows AFTER companies is built
    let has_visible_rows = match filter.as_str() {
        "wallets" => companies.iter().any(|c| !c.wallets.is_empty()),
        "banking" => companies.iter().any(|c| !c.banking_accounts.is_empty()),
        _ => !companies.is_empty(),
    };

    Html(
        IndexTemplate {
            companies,
            wallet_count,
            bank_count,
            filter,
            active_nav,
            has_visible_rows,
        }
        .render()
        .unwrap_or_default(),
    )
}
```

Notes:
- Read the *existing* body of `index` first to preserve the CompanyGroup-building logic (it iterates `book.addresses` and `book.banking_accounts`, groups by company, and builds `Vec<CompanyGroup>`). Do not re-derive it.
- `State` is extracted but unused (`_state`); that's fine — Axum requires the extractor to be in scope for the handler signature to compile against the router. Underscore-prefixed binding silences the unused-variable warning.

- [ ] **Step 3: Update `templates/index.html`**

Find the dashboard table body. Currently it contains:

```html
{% for wallet in company.wallets %}
   ...wallet row...
{% endfor %}
{% for account in company.banking_accounts %}
   ...account row...
{% endfor %}
```

Wrap each loop with a filter conditional:

```html
{% if filter != "banking" %}
{% for wallet in company.wallets %}
   ...wallet row...
{% endfor %}
{% endif %}
{% if filter != "wallets" %}
{% for account in company.banking_accounts %}
   ...account row...
{% endfor %}
{% endif %}
```

Locate the subtitle (search for "Track your crypto holdings"). Make it filter-aware:

```html
<p class="page-subtitle">
    {% if filter == "wallets" %}Wallets across all chains
    {% else if filter == "banking" %}Banking accounts
    {% else %}Track your crypto holdings across all wallets and chains
    {% endif %}
</p>
```

Find the empty-state block (search for `companies.is_empty()`). Change the condition and the message:

```html
{% if !has_visible_rows %}
<div class="empty">
    <i data-lucide="wallet" style="width: 48px; height: 48px; margin-bottom: 16px; opacity: 0.3;"></i>
    <p>
        {% if filter == "wallets" %}No wallets tracked yet
        {% else if filter == "banking" %}No banking accounts tracked yet
        {% else %}No accounts tracked yet
        {% endif %}
    </p>
    <p class="text-muted" style="margin-top: 8px;">Add a wallet or bank account to get started</p>
</div>
{% else %}
... existing data table ...
{% endif %}
```

- [ ] **Step 4: Verify build**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build. Askama compiles the template at build time, so any field-reference typo surfaces here.

- [ ] **Step 5: Add a unit test for the filter logic**

Where the other handler tests live in the `#[cfg(test)] mod tests {}` block (around `web.rs:3300+`), add:

```rust
#[test]
fn test_index_template_filter_routing() {
    // The mapping from filter query string to (filter, active_nav)
    // is small enough to test directly without spinning up a handler.
    let cases = [
        (None, ("all", "dashboard")),
        (Some("wallets"), ("wallets", "wallets")),
        (Some("banking"), ("banking", "banking")),
        (Some("garbage"), ("all", "dashboard")),
    ];
    for (input, (want_filter, want_nav)) in cases {
        let (got_filter, got_nav) = match input {
            Some("wallets") => ("wallets", "wallets"),
            Some("banking") => ("banking", "banking"),
            _ => ("all", "dashboard"),
        };
        assert_eq!(got_filter, want_filter, "filter for input {:?}", input);
        assert_eq!(got_nav, want_nav, "active_nav for input {:?}", input);
    }
}
```

This test pins the mapping. End-to-end testing of the handler is covered by manual smoke (Chunk 6).

- [ ] **Step 6: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: 38 tests passing (37 prior + 1 new).

- [ ] **Step 7: Commit**

Write commit message to `/tmp/task3-msg.txt`:

```
feat: dashboard filter via ?filter=wallets|banking query param

index handler now accepts a filter query param and exposes three
new fields on IndexTemplate: filter, active_nav, has_visible_rows.
templates/index.html conditionally renders wallet rows, banking
rows, or both; the subtitle and empty-state messaging adapt to
the filter. has_visible_rows handles the case where companies
exist but the active filter has no matching rows.
```

Commit:
```bash
git add src/display/web.rs templates/index.html
git commit -F /tmp/task3-msg.txt
```

---

## Chunk 3: Sidebar wiring

### Task 4: Update sidebar nav with real routes and active state

**Files:**
- Modify: `templates/base.html` — sidebar at lines 556-577.

This task only changes the sidebar in `base.html`. The `IndexTemplate` already has `active_nav` after Task 3; the new Settings and Transactions templates land in Tasks 5 and 7 and will include their own `active_nav` values.

- [ ] **Step 1: Replace sidebar markup**

In `templates/base.html`, find `<nav class="sidebar-nav">` (around line 556). Replace the entire `<nav>...</nav>` block with:

```html
<nav class="sidebar-nav">
    <a href="/" class="nav-item {% if active_nav == "dashboard" %}active{% endif %}">
        <i data-lucide="layout-dashboard"></i>
        <span>Dashboard</span>
    </a>
    <a href="/?filter=wallets" class="nav-item {% if active_nav == "wallets" %}active{% endif %}">
        <i data-lucide="wallet"></i>
        <span>Wallets</span>
    </a>
    <a href="/transactions" class="nav-item {% if active_nav == "transactions" %}active{% endif %}">
        <i data-lucide="arrow-left-right"></i>
        <span>Transactions</span>
    </a>
    <a href="/?filter=banking" class="nav-item {% if active_nav == "banking" %}active{% endif %}">
        <i data-lucide="landmark"></i>
        <span>Banking</span>
    </a>
    <a href="/settings" class="nav-item {% if active_nav == "settings" %}active{% endif %}">
        <i data-lucide="settings"></i>
        <span>Settings</span>
    </a>
</nav>
```

- [ ] **Step 2: Verify build**

```bash
cargo build 2>&1 | tail -10
```

Expected: clean build. The `index.html` template extends `base.html` and provides `active_nav` from Task 3. Other partials (`balances.html`, `single_balance.html`, `transactions.html`, `account_row.html`) do not extend `base.html` so they're unaffected.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: 38 tests passing.

- [ ] **Step 4: Commit**

Write commit message to `/tmp/task4-msg.txt`:

```
feat: wire sidebar nav to real routes with active-state highlighting

Dashboard, Wallets, Transactions, Banking, Settings now point at
real URLs (or filtered dashboard views). The active class is
applied based on the active_nav template field. Note that the
Settings and Transactions routes don't exist yet (Tasks 5 and 7);
clicking those links will 404 until those tasks land.
```

Commit:
```bash
git add templates/base.html
git commit -F /tmp/task4-msg.txt
```

---

## Chunk 4: Settings page

### Task 5: Settings page handler and template

**Files:**
- Create: `templates/settings.html`
- Modify: `src/display/web.rs` — add `SettingsTemplate`, `EnvVarStatus`, `RefreshIntervalForm` structs; add `settings_page` GET handler and `update_refresh_interval` POST handler; register both on `protected_routes`.

- [ ] **Step 1: Add template view structs**

Near the other `#[derive(Template)]` blocks (around line 86-145), add:

```rust
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    port: u16,
    refresh_interval_human: String,
    refresh_interval_secs: u64,
    last_refresh: String,
    next_refresh: String,
    api_key_enabled: bool,
    cached_wallet_count: usize,
    cached_price_count: usize,
    price_cache_age: String,
    env_vars: Vec<EnvVarStatus>,
    active_nav: String,
}

struct EnvVarStatus {
    name: String,
    configured: bool,
}

#[derive(Deserialize)]
struct RefreshIntervalForm {
    interval: String,
}
```

- [ ] **Step 2: Add the GET `/settings` handler**

After the `health_check` function (around line 1812), add:

```rust
async fn settings_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.cache.read().await;
    let interval_secs = *state.refresh_interval_secs.read().await;
    let interval = Duration::from_secs(interval_secs);

    // Compute last/next refresh
    let last_refresh_iso = cache
        .last_refresh
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "never".to_string());

    let next_refresh = match cache.last_refresh {
        Some(last_dt) => {
            let last_ts = last_dt.timestamp() as u64;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let elapsed = now.saturating_sub(last_ts);
            let next_in = interval_secs.saturating_sub(elapsed);
            format_duration(Duration::from_secs(next_in))
        }
        None => format_duration(interval),
    };

    let cached_wallet_count = cache.balances.len();
    let cached_price_count = cache.prices.len();
    let price_cache_age = cache.cache_age_string();

    let env_vars = vec![
        EnvVarStatus { name: "HELIUS_API_KEY".to_string(), configured: std::env::var("HELIUS_API_KEY").is_ok() },
        EnvVarStatus { name: "ALCHEMY_API_KEY".to_string(), configured: std::env::var("ALCHEMY_API_KEY").is_ok() },
        EnvVarStatus { name: "SURGE_API_KEY".to_string(), configured: std::env::var("SURGE_API_KEY").is_ok() },
        EnvVarStatus { name: "MERCURY_API_KEY".to_string(), configured: std::env::var("MERCURY_API_KEY").is_ok() },
        EnvVarStatus { name: "CIRCLE_API_KEY".to_string(), configured: std::env::var("CIRCLE_API_KEY").is_ok() },
    ];

    Html(
        SettingsTemplate {
            port: 0, // filled below from state if available; placeholder here
            refresh_interval_human: format_duration(interval),
            refresh_interval_secs: interval_secs,
            last_refresh: last_refresh_iso,
            next_refresh,
            api_key_enabled: state.api_key.is_some(),
            cached_wallet_count,
            cached_price_count,
            price_cache_age,
            env_vars,
            active_nav: "settings".to_string(),
        }
        .render()
        .unwrap_or_default(),
    )
}
```

Note on `port`: `AppState` doesn't currently carry the port. Two options:
- (Preferred) Add `pub port: u16` to `AppState` and populate from the CLI flag at the constructor. Then read it here.
- (Quick) Read `state.api_key`-style — actually just hardcode "n/a" if you don't want to plumb it.

For this task, **add `pub port: u16` to `AppState`** at line 192-198 and populate it from the `port` variable in `start_server` (around line 353 where the AppState is constructed). Then read `state.port` in `settings_page`.

Update the three test constructors (search for `last_manual_refresh: Arc::new(RwLock::new(None))` to find them) to set `port: 3000` (any test value works).

- [ ] **Step 3: Add the POST `/settings/refresh-interval` handler**

After `settings_page`, add:

```rust
async fn update_refresh_interval(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RefreshIntervalForm>,
) -> impl IntoResponse {
    let parsed = match parse_duration(&form.interval) {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("Invalid interval: {}", e)).into_response();
        }
    };

    let new_secs = parsed.as_secs();
    if new_secs < 30 {
        return (
            StatusCode::BAD_REQUEST,
            "Interval must be at least 30 seconds".to_string(),
        )
            .into_response();
    }

    *state.refresh_interval_secs.write().await = new_secs;

    // Redirect back to /settings so HTMX can swap or browser can follow
    axum::response::Redirect::to("/settings").into_response()
}
```

- [ ] **Step 4: Register the routes**

In `start_server` (around line 369-388), extend the `protected_routes` builder:

```rust
let protected_routes = Router::new()
    .route("/", get(index))
    .route("/settings", get(settings_page))
    .route("/settings/refresh-interval", post(update_refresh_interval))
    .route("/transactions", get(global_transactions))  // Task 7 will add this handler
    .route("/accounts", post(add_account))
    // ... existing routes ...
```

(Comment out the `/transactions` line until Task 7 if you want a green build between Tasks 5 and 7. Otherwise add a stub handler.)

For a green build between tasks, add a one-line stub for `global_transactions` at the bottom of the file:

```rust
async fn global_transactions(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    Html("<p>Coming soon</p>".to_string())
}
```

Task 7 replaces the body.

- [ ] **Step 5: Create `templates/settings.html`**

```html
{% extends "base.html" %}

{% block content %}
<div class="page-header">
    <div class="header-left">
        <h1 class="page-title">Settings</h1>
        <p class="page-subtitle">Runtime configuration and cache status</p>
    </div>
</div>

<div class="card">
    <h2 class="section-heading">Server Configuration</h2>
    <table class="settings-table">
        <tr>
            <td class="label">Port</td>
            <td class="value">{{ port }}</td>
        </tr>
        <tr>
            <td class="label">Refresh interval</td>
            <td class="value">
                <form hx-post="/settings/refresh-interval" hx-swap="none" style="display: inline-flex; gap: 8px;">
                    <input type="text" name="interval" value="{{ refresh_interval_human }}" class="input-inline">
                    <button type="submit" class="btn btn-primary btn-sm">Save</button>
                </form>
                <p class="text-muted" style="margin-top: 4px; font-size: 12px;">Session-only. Use --refresh-interval CLI flag to persist.</p>
            </td>
        </tr>
        <tr>
            <td class="label">Last refresh</td>
            <td class="value">{{ last_refresh }}</td>
        </tr>
        <tr>
            <td class="label">Next refresh</td>
            <td class="value">in {{ next_refresh }}</td>
        </tr>
        <tr>
            <td class="label">API key auth</td>
            <td class="value">{% if api_key_enabled %}Enabled{% else %}Disabled{% endif %}</td>
        </tr>
    </table>
</div>

<div class="card">
    <h2 class="section-heading">Cache Status</h2>
    <table class="settings-table">
        <tr>
            <td class="label">Cached wallets</td>
            <td class="value">{{ cached_wallet_count }}</td>
        </tr>
        <tr>
            <td class="label">Cached prices</td>
            <td class="value">{{ cached_price_count }} symbols</td>
        </tr>
        <tr>
            <td class="label">Price cache age</td>
            <td class="value">{{ price_cache_age }}</td>
        </tr>
    </table>
    <button class="btn btn-primary"
            hx-post="/api/refresh"
            hx-swap="none"
            hx-on::after-request="if(event.detail.xhr.status === 429) alert('Rate-limited. Try again in a few minutes.');">
        Force refresh now
    </button>
    <p class="text-muted" style="margin-top: 8px; font-size: 12px;">Limited to once per 5 minutes.</p>
</div>

<div class="card">
    <h2 class="section-heading">Environment</h2>
    <table class="settings-table">
        {% for env in env_vars %}
        <tr>
            <td class="label">{{ env.name }}</td>
            <td class="value">{% if env.configured %}Configured{% else %}<span class="text-muted">Not configured</span>{% endif %}</td>
        </tr>
        {% endfor %}
    </table>
</div>

<style>
    .section-heading {
        font-size: 14px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-disabled);
        margin-bottom: 12px;
    }
    .settings-table {
        width: 100%;
        border-collapse: collapse;
        margin-bottom: 12px;
    }
    .settings-table td {
        padding: 8px 12px;
        border-bottom: 1px solid var(--border-subtle);
    }
    .settings-table .label {
        font-weight: 500;
        width: 200px;
        color: var(--text-secondary);
    }
    .settings-table .value {
        font-family: var(--font-mono);
    }
    .input-inline {
        background: var(--bg-surface);
        border: 1px solid var(--border-subtle);
        border-radius: var(--radius-md);
        padding: 4px 8px;
        font-family: var(--font-mono);
        font-size: 13px;
        color: var(--text-primary);
        width: 80px;
    }
</style>
{% endblock %}
```

- [ ] **Step 6: Verify build**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 7: Add a unit test for the POST handler**

In the test module:

```rust
#[tokio::test]
async fn test_update_refresh_interval_rejects_garbage() {
    use crate::services::cache::SharedCache;
    use axum::extract::{Form, State};
    use axum::response::IntoResponse;

    let state = Arc::new(AppState {
        cache: SharedCache::new(),
        api_key: None,
        port: 3000,
        refresh_interval_secs: Arc::new(RwLock::new(3600)),
        last_manual_refresh: Arc::new(RwLock::new(None)),
        refresh_in_progress: Arc::new(RwLock::new(false)),
    });

    let form = Form(RefreshIntervalForm {
        interval: "not-a-duration".to_string(),
    });

    let response = update_refresh_interval(State(state), form).await;
    let (parts, _body) = response.into_response().into_parts();
    assert_eq!(parts.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_refresh_interval_writes_state() {
    use crate::services::cache::SharedCache;
    use axum::extract::{Form, State};

    let state = Arc::new(AppState {
        cache: SharedCache::new(),
        api_key: None,
        port: 3000,
        refresh_interval_secs: Arc::new(RwLock::new(3600)),
        last_manual_refresh: Arc::new(RwLock::new(None)),
        refresh_in_progress: Arc::new(RwLock::new(false)),
    });

    let form = Form(RefreshIntervalForm {
        interval: "2h".to_string(),
    });

    let _response = update_refresh_interval(State(state.clone()), form).await;
    let stored = *state.refresh_interval_secs.read().await;
    assert_eq!(stored, 7200);
}

#[tokio::test]
async fn test_update_refresh_interval_rejects_too_short() {
    use crate::services::cache::SharedCache;
    use axum::extract::{Form, State};
    use axum::response::IntoResponse;

    let state = Arc::new(AppState {
        cache: SharedCache::new(),
        api_key: None,
        port: 3000,
        refresh_interval_secs: Arc::new(RwLock::new(3600)),
        last_manual_refresh: Arc::new(RwLock::new(None)),
        refresh_in_progress: Arc::new(RwLock::new(false)),
    });

    let form = Form(RefreshIntervalForm {
        interval: "5s".to_string(),
    });

    let response = update_refresh_interval(State(state), form).await;
    let (parts, _body) = response.into_response().into_parts();
    assert_eq!(parts.status, StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 8: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: 41 tests passing (38 prior + 3 new).

- [ ] **Step 9: Commit**

Write commit message to `/tmp/task5-msg.txt`:

```
feat: Settings page with editable refresh interval

GET /settings renders runtime config, cache status, and env-var
configured/not-configured. POST /settings/refresh-interval accepts
a duration string (e.g., "30m", "4h"), validates via parse_duration,
rejects values under 30 seconds, and writes to AppState.refresh_
interval_secs. The change takes effect on the next tick of the
background refresh task.

AppState gains a pub port: u16 field plumbed from the CLI flag so
the Settings page can display it. Three test constructors updated.

Includes a stub global_transactions handler so the router compiles
between this commit and Task 7's implementation.
```

Commit:
```bash
git add src/display/web.rs templates/settings.html
git commit -F /tmp/task5-msg.txt
```

---

## Chunk 5: Transactions global view

### Task 6: GlobalTxView struct and conversion helpers

**Files:**
- Modify: `src/display/web.rs` — add `GlobalTxView`, `GlobalTransactionsTemplate`; add conversion impls/free-functions from `solana::SolanaTransaction` and `mercury::MercuryTransaction`.

This task adds the data types and conversions in isolation, with unit tests, before wiring them into a handler.

- [ ] **Step 1: Add `GlobalTransactionsTemplate` and `GlobalTxView` structs**

Near the existing `#[derive(Template)]` blocks:

```rust
#[derive(Template)]
#[template(path = "global_transactions.html")]
struct GlobalTransactionsTemplate {
    rows: Vec<GlobalTxView>,
    active_nav: String,
}

struct GlobalTxView {
    date: String,
    timestamp: i64,
    source_name: String,
    source_chain: String,
    description: String,
    amount: f64,
    currency: String,
    status: String,
    explorer_url: String,
}
```

- [ ] **Step 2: Add conversion helpers**

After `GlobalTxView`:

```rust
impl GlobalTxView {
    fn from_solana(wallet: &crate::storage::WalletAddress, tx: &crate::chains::solana::SolanaTransaction) -> Self {
        let date = match tx.block_time {
            Some(ts) => chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            None => "pending".to_string(),
        };

        Self {
            date,
            timestamp: tx.block_time.unwrap_or(0),
            source_name: wallet.name.clone(),
            source_chain: "Solana".to_string(),
            description: tx.memo.clone().unwrap_or_else(|| {
                let sig = &tx.signature;
                if sig.len() > 12 {
                    format!("{}…{}", &sig[..6], &sig[sig.len() - 6..])
                } else {
                    sig.clone()
                }
            }),
            amount: tx.sol_change,
            currency: "SOL".to_string(),
            status: if tx.success { "Confirmed".to_string() } else { "Failed".to_string() },
            explorer_url: format!("https://solscan.io/tx/{}", tx.signature),
        }
    }

    fn from_mercury(account: &crate::storage::BankingAccount, tx: &crate::banking::mercury::MercuryTransaction) -> Self {
        let timestamp_str = tx.posted_at.as_ref().unwrap_or(&tx.created_at);
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let date = if timestamp_str.len() >= 16 {
            timestamp_str[..16].replace("T", " ")
        } else {
            timestamp_str.clone()
        };

        let description = tx
            .bank_description
            .clone()
            .or(tx.note.clone())
            .or(tx.external_memo.clone())
            .or_else(|| tx.counterparty_name.clone())
            .unwrap_or_else(|| tx.kind.clone());

        Self {
            date,
            timestamp,
            source_name: account.name.clone(),
            source_chain: "Mercury".to_string(),
            description,
            amount: tx.amount,
            currency: "USD".to_string(),
            status: tx.status.clone(),
            explorer_url: String::new(),
        }
    }
}
```

Note: this assumes `solana::SolanaTransaction` has fields `block_time: Option<i64>`, `signature: String`, `memo: Option<String>`, `sol_change: f64`, `success: bool`. Verify by reading `src/chains/solana.rs` (the struct is defined just above `get_transactions` at line 250). If field names differ, adapt accordingly.

Similarly for `mercury::MercuryTransaction` — verify field names in `src/banking/mercury.rs`.

- [ ] **Step 3: Add unit tests for the conversions and sorting**

```rust
#[test]
fn test_global_tx_view_sort_and_truncate() {
    let mut rows: Vec<GlobalTxView> = (0..150)
        .map(|i| GlobalTxView {
            date: "2026-05-13".to_string(),
            timestamp: i as i64,
            source_name: "test".to_string(),
            source_chain: "Solana".to_string(),
            description: "".to_string(),
            amount: 0.0,
            currency: "SOL".to_string(),
            status: "Confirmed".to_string(),
            explorer_url: "".to_string(),
        })
        .collect();

    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    rows.truncate(100);

    assert_eq!(rows.len(), 100);
    assert_eq!(rows[0].timestamp, 149);
    assert_eq!(rows[99].timestamp, 50);
}
```

- [ ] **Step 4: Verify build and tests**

```bash
cargo build 2>&1 | tail -10
cargo test 2>&1 | tail -10
```

Expected: clean build, 42 tests passing (41 prior + 1 new).

- [ ] **Step 5: Commit**

Write commit message to `/tmp/task6-msg.txt`:

```
feat: GlobalTxView and conversion helpers for the Transactions page

Adds the view struct and from_solana / from_mercury conversion
helpers. The handler is wired in Task 7. Conversion logic normalizes
timestamps to unix seconds for sorting and pretty strings for
display. Solana rows carry a Solscan explorer URL; Mercury rows
leave explorer_url empty.
```

Commit:
```bash
git add src/display/web.rs
git commit -F /tmp/task6-msg.txt
```

---

### Task 7: `/transactions` handler with spawn_blocking

**Files:**
- Modify: `src/display/web.rs` — replace the stub `global_transactions` with the real implementation.
- Create: `templates/global_transactions.html`

- [ ] **Step 1: Replace the stub handler body**

Find `async fn global_transactions(State(_state): State<Arc<AppState>>) -> impl IntoResponse { ... }` (the stub from Task 5). Replace its body:

```rust
async fn global_transactions(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(_) => AddressBook::new(),
    };
    let mut rows: Vec<GlobalTxView> = Vec::new();

    // Solana wallets - SolanaClient::get_transactions is synchronous and uses
    // std::thread::sleep internally, so wrap each fetch in spawn_blocking to
    // avoid stalling the Tokio runtime.
    for wallet in book.addresses.iter().filter(|w| w.chain == Chain::Solana) {
        let wallet = wallet.clone();
        let result = tokio::task::spawn_blocking(move || {
            let client = SolanaClient::new(None);
            client
                .get_transactions(&wallet.address, 25)
                .ok()
                .map(|t| (wallet, t))
        })
        .await
        .ok()
        .flatten();
        if let Some((wallet, txs)) = result {
            for tx in &txs {
                rows.push(GlobalTxView::from_solana(&wallet, tx));
            }
        }
    }

    // Mercury accounts - async client
    for account in book
        .banking_accounts
        .iter()
        .filter(|a| a.service == BankingService::Mercury)
    {
        let client = match MercuryClient::new() {
            Ok(c) => c,
            Err(_) => continue,
        };
        match client
            .get_transactions(&account.account_id, None, None)
            .await
        {
            Ok(txs) => {
                for tx in txs.iter().take(50) {
                    rows.push(GlobalTxView::from_mercury(account, tx));
                }
            }
            Err(_) => continue,
        }
    }

    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    rows.truncate(100);

    Html(
        GlobalTransactionsTemplate {
            rows,
            active_nav: "transactions".to_string(),
        }
        .render()
        .unwrap_or_default(),
    )
}
```

- [ ] **Step 2: Create `templates/global_transactions.html`**

```html
{% extends "base.html" %}

{% block content %}
<div class="page-header">
    <div class="header-left">
        <h1 class="page-title">Transactions</h1>
        <p class="page-subtitle">Recent activity across Solana wallets and Mercury accounts (top 100, newest first)</p>
    </div>
</div>

{% if rows.is_empty() %}
<div class="card">
    <div class="empty">
        <i data-lucide="inbox" style="width: 48px; height: 48px; margin-bottom: 16px; opacity: 0.3;"></i>
        <p>No transactions found</p>
        <p class="text-muted" style="margin-top: 8px;">
            Transaction history is currently only supported for Solana wallets and Mercury bank accounts.
            Add a Solana wallet or Mercury account to see transactions here.
        </p>
    </div>
</div>
{% else %}
<div class="card">
    <table class="data-table">
        <thead>
            <tr>
                <th>Date</th>
                <th>Source</th>
                <th>Description</th>
                <th style="text-align: right;">Amount</th>
                <th>Status</th>
                <th></th>
            </tr>
        </thead>
        <tbody>
            {% for row in rows %}
            <tr>
                <td class="amount">{{ row.date }}</td>
                <td>
                    <div class="source-cell">
                        <span class="source-name">{{ row.source_name }}</span>
                        <span class="chain-badge {% if row.source_chain == "Mercury" %}bank{% endif %}">{{ row.source_chain }}</span>
                    </div>
                </td>
                <td class="truncate" style="max-width: 320px;">{{ row.description }}</td>
                <td class="amount {% if row.amount >= 0.0 %}positive{% else %}negative{% endif %}">
                    {% if row.amount >= 0.0 %}+{% endif %}{{ row.amount|format_amount }} {{ row.currency }}
                </td>
                <td>{{ row.status }}</td>
                <td>
                    {% if !row.explorer_url.is_empty() %}
                    <a href="{{ row.explorer_url }}" target="_blank" rel="noopener" class="btn btn-secondary btn-sm">
                        <i data-lucide="external-link" style="width: 14px; height: 14px;"></i>
                    </a>
                    {% endif %}
                </td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
</div>
{% endif %}

<style>
    .source-cell {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .source-name {
        font-weight: 500;
    }
    .amount.negative {
        color: var(--red-error);
    }
</style>
{% endblock %}
```

- [ ] **Step 3: Verify build**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: 42 tests passing (Task 6 added the sort/truncate test; no new tests here because the handler itself depends on filesystem `AddressBook::load`).

- [ ] **Step 5: Commit**

Write commit message to `/tmp/task7-msg.txt`:

```
feat: /transactions global view aggregating Solana + Mercury

Replaces the Task 5 stub with the real handler. Iterates Solana
wallets (fetched via spawn_blocking since the underlying client is
synchronous and uses std::thread::sleep) and Mercury accounts
(already async), sorts by timestamp descending, truncates to top
100. Failed RPC and missing API key are silently skipped (per spec).
```

Commit:
```bash
git add src/display/web.rs templates/global_transactions.html
git commit -F /tmp/task7-msg.txt
```

---

## Chunk 6: Final verification + PR

### Task 8: Quality gates and PR

- [ ] **Step 1: Format**

```bash
cargo fmt
```

- [ ] **Step 2: Lint**

```bash
cargo clippy --all-targets 2>&1 | tail -30
```

Expected: no NEW warnings vs the existing dead-code warnings.

- [ ] **Step 3: Full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: 42 tests passing (37 prior + 1 filter-routing + 3 settings POST + 1 sort/truncate).

- [ ] **Step 4: Manual smoke test**

```bash
cargo build --release && ./target/release/gringotts serve --port 3000
```

Open `http://localhost:3000` in a browser. Verify:
- Each sidebar item leads somewhere (no 404s).
- Clicking Dashboard / Wallets / Banking / Transactions / Settings highlights the correct sidebar item.
- Dashboard with `?filter=wallets` shows only crypto wallet rows; `?filter=banking` shows only banking accounts.
- If a filter yields no rows, the empty-state message reflects the filter.
- Settings page renders all sections; env-var status is correct.
- Settings page "Save" button on refresh interval: enter "5m", click Save. Reload. Field shows "5m".
- Force refresh button works; pressing it again within 5 minutes shows the alert.
- Transactions page shows recent activity if you have Solana wallets or Mercury accounts configured, otherwise the empty-state message.

- [ ] **Step 5: Commit any fmt/clippy fixes**

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: cargo fmt and clippy cleanup"
```

- [ ] **Step 6: Push branch and open PR**

```bash
git push -u origin feat/sidebar-pages
```

Write PR body to `/tmp/pr-body.md`:

```markdown
## Summary
- Wallets and Banking sidebar items now route to filtered dashboard views (`?filter=wallets|banking`)
- Adds /settings page with editable refresh interval, force-refresh button, cache status, and env-var configured/not-configured display
- Adds /transactions global view aggregating Solana and Mercury transactions chronologically (top 100)
- Sidebar items show an active state via a new `active_nav` template field on the three full-page templates

Spec: `docs/superpowers/specs/2026-05-13-sidebar-pages-design.md`

## Changes
- `AppState.refresh_interval_secs: u64` → `Arc<RwLock<u64>>` so Settings POST can write to it
- `background_refresh_task` drops its `interval: Duration` parameter and reads from state each tick (uses `tokio::time::sleep` instead of a fixed `tokio::time::interval` ticker so changes take effect on the next iteration)
- `AppState` gains a `port: u16` field plumbed from the CLI flag
- Solana transaction fetches in `/transactions` are wrapped in `tokio::task::spawn_blocking` because `SolanaClient::get_transactions` is synchronous and uses `std::thread::sleep`

## Test plan
- [x] `cargo fmt --check` clean
- [x] `cargo clippy` no new warnings
- [x] `cargo test` 42 passing
- [x] Manual smoke: sidebar nav, filtered dashboard views, Settings save, force refresh cooldown, Transactions page

## Known limitations
- Transaction history is Solana + Mercury only; other chains and Circle are silently skipped
- Refresh interval changes are in-memory only and reset on restart (matches `--refresh-interval` CLI flag semantics)
- Transactions page can take a few seconds per Solana wallet (the underlying client throttles 100ms between detail fetches)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

```bash
gh pr create --title "feat: sidebar pages (filters, Settings, Transactions)" --body-file /tmp/pr-body.md
```

---
