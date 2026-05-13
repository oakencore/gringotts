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

**Backend** (`src/display/web.rs::index_handler`):

```rust
#[derive(Deserialize)]
struct DashboardFilter {
    filter: Option<String>,  // "wallets" | "banking" | None
}

async fn index_handler(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<DashboardFilter>,
) -> impl IntoResponse {
    // existing handler logic, passing q.filter into the template
}
```

`IndexTemplate` gains a `filter: String` field (one of `"all"`, `"wallets"`, `"banking"`). The handler maps `q.filter.as_deref()` to that.

**Frontend** (`templates/index.html`):
- Wrap the existing inner loops with conditionals so the table reflects the filter:
  - `{% if filter != "banking" %}{% for wallet in company.wallets %}...{% endif %}`
  - `{% if filter != "wallets" %}{% for account in company.banking_accounts %}...{% endif %}`
- Update the page subtitle to reflect the active filter ("Wallets" / "Banking accounts" / "All accounts").
- Use the filter to set `active_nav` (see Section 4) so the correct sidebar item highlights.
- Empty state: reuse the existing empty-state block but with filter-aware copy ("No banking accounts tracked yet").

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

1. **Refresh interval** — text input accepting duration strings (e.g., `4h`, `30m`, `1d`) parsed by the existing `parse_duration` helper at `src/display/web.rs:250`. Posts a form to `/settings/refresh-interval` which updates `AppState.refresh_interval_secs`. The change takes effect at the next tick of the background refresh task (see implementation note below).
2. **Force refresh now** — HTMX-POSTs to the existing `/api/refresh` endpoint (`web.rs:445`), which already implements the 30-second cooldown.

**Read-only displays:**
- Port (passed from CLI args / config via `AppState`).
- Last refresh / Next refresh derived from `state.cache.last_refresh` plus the interval.
- Cache stats read from `state.cache`.
- API key auth: `state.api_key.is_some()` only (the key value is never shown).
- Env vars: server-side `env::var(name).is_ok()` checks. Display "Configured" / "Not configured" (the value is never shown).

**State plumbing:**

`AppState.refresh_interval_secs: u64` becomes `Arc<RwLock<u64>>` so the POST handler can write to it. Read sites are few. The `background_refresh_task` (`src/display/web.rs:435`) creates a `tokio::time::interval` from this value at startup; changing the value mid-flight doesn't automatically reset the ticker. Two acceptable implementations:

- **Simple:** the task reads the value on each tick and recomputes its sleep duration manually instead of using a fixed `interval`. Add a `mark` time and `sleep_until(mark + dynamic_interval)`.
- **Adequate:** keep the fixed `tokio::time::interval`, document in the UI that the change takes effect after the next refresh fires.

Implementation will pick one when the plan is written. For this spec, both are acceptable.

### Section 3 — Transactions global view

**Route:** `GET /transactions`. Template: new `templates/global_transactions.html` (does not collide with existing `templates/transactions.html`, which is the per-wallet detail view).

**Aggregation logic:**

```rust
async fn get_global_transactions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let book = AddressBook::load()?;
    let mut rows: Vec<GlobalTxView> = Vec::new();

    // Solana wallets: 25 txns per wallet
    for wallet in book.addresses.iter().filter(|w| matches!(w.chain, Chain::Solana)) {
        let client = SolanaClient::new(None);
        if let Ok(txs) = client.get_transactions(&wallet.address, 25) {
            for tx in txs {
                rows.push(GlobalTxView::from_solana(wallet, tx));
            }
        }
    }

    // Mercury accounts: 50 txns per account
    for account in book.banking_accounts.iter().filter(|a| a.service == BankingService::Mercury) {
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

    Html(GlobalTransactionsTemplate { rows, active_nav: "transactions".to_string() }.render().unwrap_or_default())
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

`solana::get_transactions` uses 100ms sleeps between detailed RPC calls (`src/chains/solana.rs:289-291`) and fetches up to 20 details per call. With N Solana wallets, the page load is roughly `N × 2s` worst case. Mercury is one HTTPS call per account. This is acceptable for v1 — the page is on-demand from a sidebar click, not background-fetched. If it becomes painful, follow-ups include parallelizing per-wallet fetches via `tokio::join_all`, or caching results server-side with TTL.

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

**`active_nav` plumbing:** Each top-level template struct (`IndexTemplate`, `SettingsTemplate`, `GlobalTransactionsTemplate`) gains an `active_nav: String` field. Five values: `"dashboard"`, `"wallets"`, `"banking"`, `"transactions"`, `"settings"`. The index handler maps `filter=wallets` → `"wallets"`, `filter=banking` → `"banking"`, default → `"dashboard"`. The Askama base template uses the value to render the `.active` class on the matching nav item.

Alternative considered: a base Askama struct with a `nav` field. Rejected for simplicity; five templates with one string field each is acceptable repetition.

### Section 5 — Testing

**Unit tests in `src/display/web.rs`:**

1. Dashboard filter: `index_handler` with `filter=wallets` returns a template payload where banking rows are absent and `active_nav == "wallets"`. Same for `banking`. None → `active_nav == "dashboard"`.
2. Settings POST: parsing of `refresh_interval` form input via `parse_duration` succeeds for `4h`, `30m`, `1d`; rejects garbage with a 400.
3. Global transactions: with a stubbed `AddressBook` containing a Solana wallet and a Mercury account, the sorted output is in descending timestamp order and capped at 100 rows.

**Manual smoke:**
- Click each sidebar item; verify active-state highlighting and that the page renders.
- Browser back/forward navigation between `/`, `/?filter=wallets`, `/?filter=banking`, `/transactions`, `/settings` works without re-fetch glitches.
- Change refresh interval on Settings; confirm it takes effect (long-form test, optional).
- Force refresh button works and respects the 30-second cooldown.
- Transactions page renders rows for any configured Solana wallets and Mercury accounts.

**Quality gates before opening a PR:** `cargo fmt`, `cargo clippy` (no new warnings), `cargo test` (exact pass count reported).
