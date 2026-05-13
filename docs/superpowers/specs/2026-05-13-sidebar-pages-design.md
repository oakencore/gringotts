# Sidebar Pages — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

Four sidebar items in `templates/base.html:561-577` (Wallets, Transactions, Banking, Settings) point at dead `#anchor` placeholders. They navigate nowhere. Only the Dashboard link (`/`) actually works. This creates a confusing UI: the chrome promises functionality that doesn't exist.

## Goal

Wire every sidebar item to a real page or filtered view, with the smallest reasonable surface area.

## Scope

- **Wallets** and **Banking** sidebar items: filtered views of the existing dashboard (no new pages).
- **Settings** sidebar item: new `/settings` page with editable runtime config.
- **Transactions** sidebar item: new `/transactions` page aggregating Solana + Mercury transactions chronologically.
- Sidebar component: add active-state highlighting.

**Out of scope:**
- Adding new pages for Wallets and Banking (a filtered dashboard view covers both).
- EVM/Near/Aptos/Sui/Starknet/Circle transaction history. Only Solana and Mercury currently implement transaction listing; the global Transactions page reflects that constraint.
- API key rotation from the UI. CLI flag / env var stays the source of truth.
- Pagination on Transactions beyond the top-100 cap.
- Caching the global transaction list.

## Design

### Section 1 — Dashboard filter

Replace the dead `#wallets` and `#banking` hrefs with `?filter=wallets` and `?filter=banking` query params on the existing dashboard route.

**Backend** (`src/display/web.rs:1933` — the existing `index` handler):

The current handler has signature `async fn index() -> impl IntoResponse` (no extractors). It gains two extractors: `State` for cache reads (if Settings UI needs them later) and `Query` for the new filter param. Axum will wire both automatically since `.with_state(state.clone())` is already applied to the router.

```rust
#[derive(Deserialize)]
struct DashboardFilter {
    filter: Option<String>,  // "wallets" | "banking" | None
}

async fn index(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
    // existing handler logic, plus computing `filter`, `active_nav`,
    // and predicate flags for the empty-state.
}
```

`IndexTemplate` gains three new fields: `filter: String` (one of `"all"`, `"wallets"`, `"banking"`), `active_nav: String` (see Section 4), and `has_visible_rows: bool` (true if any row in the filtered view will render). The handler maps `q.filter.as_deref()` to `filter` and computes `has_visible_rows` by walking `companies` and checking the relevant inner collection.

**Frontend** (`templates/index.html`):
- Wrap the existing inner loops with conditionals so the table reflects the filter:
  - `{% if filter != "banking" %}{% for wallet in company.wallets %}...{% endif %}`
  - `{% if filter != "wallets" %}{% for account in company.banking_accounts %}...{% endif %}`
- Update the page subtitle to reflect the active filter ("Wallets" / "Banking accounts" / "All accounts").
- **Empty-state predicate:** The existing empty state checks `companies.is_empty()`. That doesn't catch the case where `filter=banking` is applied but only crypto wallets exist (companies isn't empty; banking rows just aren't rendered). Use the new `has_visible_rows` field instead: show the empty state when `!has_visible_rows`. Copy varies by filter: "No wallets tracked yet" / "No banking accounts tracked yet" / "No accounts tracked yet".

### Section 2 — Settings page

**Route:** `GET /settings` and `POST /settings/refresh-interval`.

**Template:** new `templates/settings.html`.

**Sections rendered:**

```
Server Configuration
  Port                  3000
  Refresh interval      [4h ▼] [Save]
  Last refresh          2026-05-13 14:32 (3m ago)
  Next refresh          3h 57m
  API key auth          Enabled

Cache Status
  Cached wallets        7
  Cached prices         12 symbols
  Price cache age       2m
  [Force refresh now]

Environment
  HELIUS_API_KEY        Configured
  ALCHEMY_API_KEY       Configured
  SURGE_API_KEY         Configured
  MERCURY_API_KEY       Not configured
  CIRCLE_API_KEY        Configured
```

**Editable controls:**

1. **Refresh interval** — text input accepting duration strings (e.g., `4h`, `30m`, `1d`) parsed by the existing `parse_duration` helper at `src/display/web.rs:248`. Posts a form to `/settings/refresh-interval` which updates `AppState.refresh_interval_secs`. The change takes effect at the next tick of the background refresh task (see implementation note below).
2. **Force refresh now** — HTMX-POSTs to the existing `/api/refresh` endpoint (handler `manual_refresh` at `web.rs:897`, route registered around `web.rs:387`), which already implements a 5-minute cooldown (`MANUAL_REFRESH_RATE_LIMIT_SECS` at `web.rs:891`). The UI should surface the cooldown to the user: disable the button and show the remaining seconds when the endpoint returns 429.

**Read-only displays:**
- Port (passed from CLI args / config via `AppState`).
- Last refresh / Next refresh derived from `state.cache.last_refresh` plus the interval.
- Cache stats read from `state.cache`.
- API key auth: `state.api_key.is_some()` only (the key value is never shown).
- Env vars: server-side `env::var(name).is_ok()` checks. Display "Configured" / "Not configured" (the value is never shown).

**State plumbing:**

`AppState.refresh_interval_secs: u64` becomes `Arc<RwLock<u64>>` so the POST handler can write to it. Existing read sites that must be updated:

- `health_check` at `web.rs:1795` and `web.rs:1801` (both reads, async context, so `.read().await` is fine).
- The signature of `background_refresh_task` at `web.rs:466` currently accepts `interval: Duration` as a separate argument (computed at startup, passed by value). That argument becomes redundant once the task reads from state. The signature changes to drop `interval`, and the task pulls `state.refresh_interval_secs.read().await` at the top of each loop iteration.
- Three test sites at `web.rs:3309`, `web.rs:3336`, `web.rs:3383` that construct `AppState` literally; they need to wrap the value in `Arc::new(RwLock::new(...))`.
- The call site at `web.rs:455-456` that spawns `background_refresh_task(refresh_state, interval).await` must drop the `interval` argument when the signature changes (the task reads from state instead).

**Persistence:** Updates to `refresh_interval_secs` are in-memory only and reset on restart. This matches the existing CLI-flag / env-var semantics (the `--refresh-interval` flag at startup is the durable source). Document this in the UI (e.g., "Effective for this session only; set the `--refresh-interval` CLI flag to persist").

**Background task ticker behavior** (`background_refresh_task` at `web.rs:466`):

Decision: option **Simple** — the task reads the current interval on each loop iteration and uses `tokio::time::sleep_until(mark + Duration::from_secs(*interval))` instead of a fixed `tokio::time::interval` ticker. This is necessary because option "Adequate" (document the lag) is unhelpful in practice: a user changing the interval from `4h` to `30m` would wait up to 4 hours for the change to take effect, which is the opposite of what they pressed Save for. Option Simple makes the change take effect on the next tick (at most one current-period wait), which is what users expect.

Sketch:

```rust
async fn background_refresh_task(state: Arc<AppState>) {
    // Skip the first immediate tick: don't refresh at startup
    let initial = *state.refresh_interval_secs.read().await;
    tokio::time::sleep(Duration::from_secs(initial)).await;

    loop {
        if let Err(e) = refresh_all_balances(&state).await {
            eprintln!("[{}] Background refresh failed: {}", ts(), e);
        }
        let next = *state.refresh_interval_secs.read().await;
        tokio::time::sleep(Duration::from_secs(next)).await;
    }
}
```

### Section 3 — Transactions global view

**Route:** `GET /transactions`. Template: new `templates/global_transactions.html` (does not collide with existing `templates/transactions.html`, which is the per-wallet detail view).

**Aggregation logic:**

`SolanaClient::get_transactions` is **synchronous** and uses `std::thread::sleep(Duration::from_millis(100))` internally. Calling it from an `async fn` directly will block the Tokio worker thread for the whole `N × 2s` worst-case duration and starve other handlers. The Solana fetch must be wrapped in `tokio::task::spawn_blocking`.

```rust
async fn get_global_transactions(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let book = match AddressBook::load() {
        Ok(b) => b,
        Err(_) => AddressBook::new(),
    };
    let mut rows: Vec<GlobalTxView> = Vec::new();

    // Solana wallets: 25 txns per wallet, fetched on a blocking thread pool
    for wallet in book.addresses.iter().filter(|w| w.chain == Chain::Solana) {
        let address = wallet.address.clone();
        let wallet_for_view = wallet.clone();
        let txs = tokio::task::spawn_blocking(move || {
            let client = SolanaClient::new(None);
            client.get_transactions(&address, 25).ok()
        })
        .await
        .ok()
        .flatten();
        if let Some(txs) = txs {
            for tx in txs {
                rows.push(GlobalTxView::from_solana(&wallet_for_view, tx));
            }
        }
    }

    // Mercury accounts: 50 txns per account (already async)
    for account in book
        .banking_accounts
        .iter()
        .filter(|a| a.service == BankingService::Mercury)
    {
        if let Ok(client) = MercuryClient::new() {
            if let Ok(txs) = client.get_transactions(&account.account_id, None, None).await {
                for tx in txs.into_iter().take(50) {
                    rows.push(GlobalTxView::from_mercury(account, tx));
                }
            }
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

struct GlobalTxView {
    date: String,            // formatted YYYY-MM-DD HH:MM
    timestamp: i64,          // unix seconds; for sorting
    source_name: String,     // wallet.name / account.name
    source_chain: String,    // "Solana" / "Mercury"
    description: String,     // memo / counterparty / signature suffix
    amount: f64,             // signed: positive = inflow
    currency: String,        // "SOL" / "USD"
    status: String,
    explorer_url: String,    // Solscan link for chain txns, empty for Mercury
}
```

**Template** (`templates/global_transactions.html`):

A single table with columns Date / Source / Description / Amount / Status. The Source column shows the wallet/account name as a chip plus a chain badge (Solana / Mercury). Amount is color-coded (green for positive, red for negative). The explorer URL renders as an external-link icon on each Solana row.

**Empty state:** If there are no Solana wallets and no Mercury accounts in the book, render: "Transaction history is currently only supported for Solana wallets and Mercury bank accounts. Add a Solana wallet or Mercury account to see transactions here."

**Performance caveats:**

`SolanaClient::get_transactions` uses 100ms sleeps between detailed RPC calls (`src/chains/solana.rs:288-291`) and fetches up to 20 details per call. With N Solana wallets, the page load is roughly `N × 2s` worst case. The `spawn_blocking` wrapping above ensures these blocking sleeps don't starve the Tokio runtime, but the user-perceived latency is still real. Mercury is one HTTPS call per account. Acceptable for v1 — the page is on-demand from a sidebar click, not background-fetched. If it becomes painful, follow-ups include parallelizing per-wallet `spawn_blocking` calls via `tokio::join_all`, or caching results server-side with TTL.

### Section 4 — Sidebar wiring + active state

**Changes to `templates/base.html:556-577`:**

Replace placeholder hrefs with real routes and add active-state via a template variable:

```html
<nav class="sidebar-nav">
    <a href="/" class="nav-item {% if active_nav == "dashboard" %}active{% endif %}">
        <i data-lucide="layout-dashboard"></i><span>Dashboard</span>
    </a>
    <a href="/?filter=wallets" class="nav-item {% if active_nav == "wallets" %}active{% endif %}">
        <i data-lucide="wallet"></i><span>Wallets</span>
    </a>
    <a href="/transactions" class="nav-item {% if active_nav == "transactions" %}active{% endif %}">
        <i data-lucide="arrow-left-right"></i><span>Transactions</span>
    </a>
    <a href="/?filter=banking" class="nav-item {% if active_nav == "banking" %}active{% endif %}">
        <i data-lucide="landmark"></i><span>Banking</span>
    </a>
    <a href="/settings" class="nav-item {% if active_nav == "settings" %}active{% endif %}">
        <i data-lucide="settings"></i><span>Settings</span>
    </a>
</nav>
```

**`active_nav` plumbing:** Only one template (`templates/index.html`) currently extends `templates/base.html`. The other four (`balances.html`, `single_balance.html`, `transactions.html` (per-wallet detail), `account_row.html`) are HTMX partials returned to `hx-target` swaps, not full page loads, so they don't render the sidebar.

The full-page templates that need `active_nav: String` are: `IndexTemplate` (existing), `SettingsTemplate` (new), `GlobalTransactionsTemplate` (new). The HTMX-partial templates do not.

Five values: `"dashboard"`, `"wallets"`, `"banking"`, `"transactions"`, `"settings"`. The `index` handler maps `filter=wallets` → `"wallets"`, `filter=banking` → `"banking"`, default → `"dashboard"`. The base template uses the value to render the `.active` class on the matching nav item.

**Route registration:** Both new routes (`GET /settings`, `POST /settings/refresh-interval`, `GET /transactions`) go on the `protected_routes` Router at `web.rs:369-388` (alongside `/`, `/balances`, etc.), not `public_routes`, so the API-key middleware applies.

Alternative considered: a base Askama struct with a `nav` field shared via `{% include %}`. Rejected for simplicity; three full-page templates with one string field each is acceptable repetition.

### Section 5 — Testing

**Unit tests in `src/display/web.rs`:**

1. Dashboard filter: build an `IndexTemplate` directly (the handler is async and uses `AddressBook::load` which reads from `~/.gringotts/`; for unit tests, exercise the template-level `filter`/`active_nav`/`has_visible_rows` plumbing rather than the handler end-to-end). Assert: with `filter == "wallets"`, the rendered HTML does not contain `class="landmark"` (the banking icon); with `filter == "banking"`, the rendered HTML does not contain `class="wallet"` (the wallet icon); with `has_visible_rows = false`, the empty-state block renders.
2. Settings POST: `parse_duration` accepts `"4h"`, `"30m"`, `"1d"` (existing tests cover this — extend or reuse). The new POST handler returns a 400 on parse failure. Cover by calling the handler with a `Form<RefreshIntervalForm>` extractor carrying garbage and asserting the response status.
3. Global transactions: a unit test that constructs `Vec<GlobalTxView>` directly (bypassing the chain/Mercury fetch) and asserts the sort-and-truncate pipeline: descending `timestamp` order, capped at 100 rows. The fetching path is exercised via manual smoke instead.

**Manual smoke:**
- Click each sidebar item; verify active-state highlighting and that the page renders.
- Browser back/forward navigation between `/`, `/?filter=wallets`, `/?filter=banking`, `/transactions`, `/settings` works without re-fetch glitches.
- Change refresh interval on Settings; confirm it takes effect (long-form test, optional).
- Force refresh button works and respects the 5-minute cooldown (returns 429 if pressed twice within 5 minutes).
- Transactions page renders rows for any configured Solana wallets and Mercury accounts.

**Quality gates before opening a PR:** `cargo fmt`, `cargo clippy` (no new warnings), `cargo test` (exact pass count reported).
