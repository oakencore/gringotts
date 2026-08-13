# Wallet and Banking Account Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename crypto wallets and banking accounts inline from the web dashboard, keeping the balance cache keyed correctly.

**Architecture:** A `rename_account` method on `AddressBook` owns all validation (trim, empty, collision, not-found). A `rename_balance` method on the cache re-keys the cached entry. A thin `PATCH /accounts/:name` Axum handler wires them together and answers HTMX. The UI is an inline label/form toggle in the dashboard row templates; success triggers a full page refresh via the `HX-Refresh` response header because the account name is baked into row ids, hx-targets, and detail ids all over the page.

**Tech Stack:** Rust, Axum, Askama templates, HTMX 1.9.10 (vendored via CDN in `templates/base.html`).

**Spec:** `docs/superpowers/specs/2026-08-13-wallet-rename-design.md`

## Global Constraints

- No emojis, no em dashes in any output (project CLAUDE.md).
- Name comparison is exact and case-sensitive (names are map keys everywhere).
- Lookup order: wallets first, then banking accounts (matches `remove_account`).
- Renaming to the same name is a no-op success.
- Persistence order: address book first, then cache; cache failure is logged, not fatal.
- Run `cargo fmt` before every commit; `cargo clippy -- -D warnings` must stay clean.

---

### Task 1: `AddressBook::rename_account` (storage layer, all validation)

**Files:**
- Modify: `src/storage.rs` (new `RenameError` enum after the `BankingAccount` struct ~line 141; new method inside `impl AddressBook` next to `remove_by_identifier` ~line 270; new `#[cfg(test)] mod tests` at end of file - the file has none yet)

**Interfaces:**
- Consumes: existing `AddressBook { addresses: Vec<WalletAddress>, banking_accounts: Vec<BankingAccount> }`.
- Produces: `pub enum RenameError { EmptyName, NameTaken, NotFound }` (derives `Debug, PartialEq, Eq`, implements `Display`) and `pub fn rename_account(&mut self, old_name: &str, new_name: &str) -> Result<String, RenameError>` returning the trimmed new name. Task 3 depends on both.

- [ ] **Step 1: Write the failing tests**

Append to the end of `src/storage.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> AddressBook {
        AddressBook {
            addresses: vec![WalletAddress {
                company: "Acme".to_string(),
                name: "Hot Wallet".to_string(),
                address: "abc123".to_string(),
                chain: Chain::Solana,
            }],
            banking_accounts: vec![BankingAccount {
                company: "Acme".to_string(),
                name: "Mercury Checking".to_string(),
                account_id: "m-1".to_string(),
                service: BankingService::Mercury,
            }],
        }
    }

    #[test]
    fn rename_wallet_succeeds() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Hot Wallet", "Cold Wallet"),
            Ok("Cold Wallet".to_string())
        );
        assert_eq!(b.addresses[0].name, "Cold Wallet");
    }

    #[test]
    fn rename_banking_account_succeeds() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Mercury Checking", "Mercury Ops"),
            Ok("Mercury Ops".to_string())
        );
        assert_eq!(b.banking_accounts[0].name, "Mercury Ops");
    }

    #[test]
    fn rename_trims_whitespace() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Hot Wallet", "  Cold Wallet  "),
            Ok("Cold Wallet".to_string())
        );
        assert_eq!(b.addresses[0].name, "Cold Wallet");
    }

    #[test]
    fn rename_rejects_empty_name() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Hot Wallet", "   "),
            Err(RenameError::EmptyName)
        );
        assert_eq!(b.addresses[0].name, "Hot Wallet");
    }

    #[test]
    fn rename_rejects_collision_with_wallet() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Mercury Checking", "Hot Wallet"),
            Err(RenameError::NameTaken)
        );
    }

    #[test]
    fn rename_rejects_collision_with_banking_account() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Hot Wallet", "Mercury Checking"),
            Err(RenameError::NameTaken)
        );
    }

    #[test]
    fn rename_to_same_name_is_noop_success() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Hot Wallet", "Hot Wallet"),
            Ok("Hot Wallet".to_string())
        );
    }

    #[test]
    fn rename_unknown_name_is_not_found() {
        let mut b = book();
        assert_eq!(
            b.rename_account("Nope", "Whatever"),
            Err(RenameError::NotFound)
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --bin gringotts storage::tests -- --nocapture`
Expected: compile error - `rename_account` and `RenameError` do not exist.

- [ ] **Step 3: Write the implementation**

After the `BankingAccount` struct in `src/storage.rs` (before `pub struct AddressBook`), add:

```rust
/// Why a rename was rejected. The web handler maps each variant to an
/// HTTP status, so keep this exhaustive rather than stringly-typed.
#[derive(Debug, PartialEq, Eq)]
pub enum RenameError {
    EmptyName,
    NameTaken,
    NotFound,
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::EmptyName => write!(f, "Name cannot be empty"),
            RenameError::NameTaken => write!(f, "An account with that name already exists"),
            RenameError::NotFound => write!(f, "Account not found"),
        }
    }
}
```

Inside `impl AddressBook`, next to `remove_by_identifier`, add:

```rust
/// Rename a wallet or banking account, matched by exact name (wallets
/// first, mirroring the web UI's delete handler). Trims the new name and
/// returns the trimmed value so callers can re-key derived state (the
/// balance cache) with exactly what was stored.
pub fn rename_account(
    &mut self,
    old_name: &str,
    new_name: &str,
) -> Result<String, RenameError> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(RenameError::EmptyName);
    }
    if new_name != old_name
        && (self.addresses.iter().any(|a| a.name == new_name)
            || self.banking_accounts.iter().any(|a| a.name == new_name))
    {
        return Err(RenameError::NameTaken);
    }
    if let Some(w) = self.addresses.iter_mut().find(|a| a.name == old_name) {
        w.name = new_name.to_string();
    } else if let Some(b) = self
        .banking_accounts
        .iter_mut()
        .find(|a| a.name == old_name)
    {
        b.name = new_name.to_string();
    } else {
        return Err(RenameError::NotFound);
    }
    Ok(new_name.to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin gringotts storage::tests`
Expected: 8 passed, 0 failed.

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt && cargo clippy -- -D warnings
git add src/storage.rs
git commit -m "feat: add AddressBook::rename_account with validation"
```

---

### Task 2: Cache re-key (`rename_balance`)

**Files:**
- Modify: `src/services/cache.rs` (method on `impl BalanceCache` next to `set_balance` ~line 129; method on `impl SharedCache` next to `update_balance` ~line 235; test in the existing `mod tests` ~line 269)

**Interfaces:**
- Consumes: existing `BalanceCache { balances: HashMap<String, CacheEntry<CachedBalance>>, .. }`.
- Produces: `pub fn rename_balance(&mut self, old_name: &str, new_name: &str)` on `BalanceCache`, and `pub async fn rename_balance(&self, old_name: &str, new_name: &str) -> Result<()>` on `SharedCache` (re-keys then persists). Task 3 calls the `SharedCache` one.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `src/services/cache.rs`:

```rust
#[test]
fn test_rename_balance_rekeys_entry() {
    let mut cache = BalanceCache::new();
    let balance = CachedBalance {
        name: "Old Name".to_string(),
        address_or_id: "test123".to_string(),
        chain_or_service: "Solana".to_string(),
        native_symbol: "SOL".to_string(),
        native_balance: 10.0,
        native_usd_value: Some(1000.0),
        tokens: vec![],
        total_usd_value: Some(1000.0),
    };
    cache.set_balance("Old Name", balance);
    let original_ts = cache.balances["Old Name"].timestamp;

    cache.rename_balance("Old Name", "New Name");

    assert!(cache.balances.get("Old Name").is_none());
    let entry = cache.balances.get("New Name").expect("entry re-keyed");
    assert_eq!(entry.data.name, "New Name");
    assert_eq!(entry.data.native_balance, 10.0);
    // Rename must not reset freshness.
    assert_eq!(entry.timestamp, original_ts);

    // Renaming a missing key is a silent no-op.
    cache.rename_balance("Ghost", "Anything");
    assert!(cache.balances.get("Anything").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin gringotts cache::tests::test_rename_balance_rekeys_entry`
Expected: compile error - `rename_balance` does not exist.

- [ ] **Step 3: Write the implementation**

On `impl BalanceCache`, after `set_balance`:

```rust
/// Move a cached balance to a new key after an account rename. Keeps the
/// entry's timestamp (a rename is not a refresh) and updates the embedded
/// name. No-op if the old key is missing or the names are equal.
pub fn rename_balance(&mut self, old_name: &str, new_name: &str) {
    if old_name == new_name {
        return;
    }
    if let Some(mut entry) = self.balances.remove(old_name) {
        entry.data.name = new_name.to_string();
        self.balances.insert(new_name.to_string(), entry);
    }
}
```

On `impl SharedCache`, after `update_balance`:

```rust
/// Re-key a cached balance after an account rename, then persist.
pub async fn rename_balance(&self, old_name: &str, new_name: &str) -> Result<()> {
    {
        let mut cache = self.inner.write().await;
        cache.rename_balance(old_name, new_name);
    }
    self.persist().await
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin gringotts cache::tests`
Expected: all cache tests pass, including `test_rename_balance_rekeys_entry`.

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt && cargo clippy -- -D warnings
git add src/services/cache.rs
git commit -m "feat: re-key cached balance on account rename"
```

---

### Task 3: `PATCH /accounts/:name` handler and route

**Files:**
- Modify: `src/display/web.rs`:
  - imports (top of file): add `patch` to the `axum::routing` import; add `RenameError` to the `crate::storage` import
  - new `RenameForm` struct and `rename_account` handler directly below `remove_account` (~line 2904)
  - route registration ~line 1000: extend the existing `/accounts/:name` route

**Interfaces:**
- Consumes: `AddressBook::rename_account` (Task 1), `SharedCache::rename_balance` (Task 2), existing `AppState { cache: SharedCache, .. }`.
- Produces: `PATCH /accounts/:name` with form field `new_name`. Responses: 200 + `HX-Refresh: true` header on success; 422/409/404 with a plain-text message body for EmptyName/NameTaken/NotFound; 500 on load/save failure. Task 4's UI depends on these exact status codes and the plain-text bodies.

- [ ] **Step 1: Extend the route**

Change line ~1000 from:

```rust
.route("/accounts/:name", delete(remove_account))
```

to:

```rust
.route("/accounts/:name", delete(remove_account).patch(rename_account))
```

- [ ] **Step 2: Write the handler**

Below `remove_account`:

```rust
#[derive(Deserialize)]
struct RenameForm {
    new_name: String,
}

async fn rename_account(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Form(form): Form<RenameForm>,
) -> Response {
    let mut book = match AddressBook::load() {
        Ok(b) => b,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response()
        }
    };

    let new_name = match book.rename_account(&name, &form.new_name) {
        Ok(n) => n,
        Err(e) => {
            let status = match e {
                RenameError::EmptyName => StatusCode::UNPROCESSABLE_ENTITY,
                RenameError::NameTaken => StatusCode::CONFLICT,
                RenameError::NotFound => StatusCode::NOT_FOUND,
            };
            return (status, e.to_string()).into_response();
        }
    };

    if let Err(e) = book.save() {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response();
    }

    // Address book is saved; re-key the cached balance so the dashboard keeps
    // showing it under the new name. If this fails the entry is refetched
    // under the new name on the next refresh - degraded, not broken.
    if let Err(e) = state.cache.rename_balance(&name, &new_name).await {
        eprintln!(
            "Warning: failed to re-key cached balance after rename: {}",
            e
        );
    }

    // The name is baked into row ids, hx-targets, and detail ids across the
    // page, so a full refresh is the only swap that stays consistent.
    (StatusCode::OK, [("HX-Refresh", "true")], String::new()).into_response()
}
```

Note: `Response` here is `axum::response::Response`. If it is not already imported at the top of `web.rs`, add it to the existing `axum::response` import; if the mixed-tuple returns fail type inference with `impl IntoResponse`, the explicit `Response` + `.into_response()` on every arm (as written) compiles cleanly.

- [ ] **Step 3: Build and lint**

Run: `cargo build --release && cargo clippy -- -D warnings`
Expected: clean build. Fix any missing-import errors (`patch`, `RenameError`, `Response`).

- [ ] **Step 4: Smoke test the endpoint against the running server**

Restart the server (`./target/release/gringotts serve --port 3000`, kill the old process first), then run a self-contained create/rename/delete cycle so no real account is touched:

```bash
B=http://localhost:3000
# create a throwaway account
curl -s -X POST $B/accounts -d 'company=Test&name=RenameSmoke&address=0x1111111111111111111111111111111111111111&chain=ethereum'
# happy path: expect HTTP 200 and an HX-Refresh: true header
curl -si -X PATCH $B/accounts/RenameSmoke -d 'new_name=RenameSmoke2' | grep -i 'HTTP\|hx-refresh'
# empty name: expect 422
curl -s -o /dev/null -w '%{http_code}\n' -X PATCH $B/accounts/RenameSmoke2 -d 'new_name=   '
# collision with an existing account name: expect 409
curl -s -X POST $B/accounts -d 'company=Test&name=RenameSmoke3&address=0x2222222222222222222222222222222222222222&chain=ethereum'
curl -s -o /dev/null -w '%{http_code}\n' -X PATCH $B/accounts/RenameSmoke2 -d 'new_name=RenameSmoke3'
# unknown account: expect 404
curl -s -o /dev/null -w '%{http_code}\n' -X PATCH $B/accounts/DoesNotExist -d 'new_name=x'
# clean up
curl -s -X DELETE $B/accounts/RenameSmoke2
curl -s -X DELETE $B/accounts/RenameSmoke3
```

Expected: 200 with `HX-Refresh: true`, then 422, 409, 404.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/display/web.rs
git commit -m "feat: PATCH /accounts/:name renames wallets and banking accounts"
```

---

### Task 4: Inline rename UI

**Files:**
- Modify: `templates/index.html` (wallet name cell ~line 152, banking name cell ~line 209, `<style>` block ~line 357, `<script>` block ~line 463)
- Modify: `templates/account_row.html` (name cell, line 8 - rows inserted right after adding an account get the same affordance)

**Interfaces:**
- Consumes: `PATCH /accounts/:name` from Task 3 (200 + HX-Refresh on success, 4xx plain-text bodies on validation errors).
- Produces: user-facing rename control; no code depends on this task.

- [ ] **Step 1: Replace the name markup in all three row templates**

In `templates/index.html`, replace line 152:

```html
<div class="asset-name">{{ wallet.name }}</div>
```

with:

```html
<div class="asset-name rename-label">
    <span>{{ wallet.name }}</span>
    <button class="rename-btn" title="Rename" onclick="startRename(this)">
        <i data-lucide="pencil" style="width: 12px; height: 12px;"></i>
    </button>
</div>
<form class="rename-form" hidden hx-patch="/accounts/{{ wallet.name }}" hx-swap="none">
    <input type="text" name="new_name" value="{{ wallet.name }}" class="rename-input">
    <span class="rename-error"></span>
</form>
```

Apply the same replacement to the banking cell (line ~209, `{{ account.name }}` instead of `{{ wallet.name }}`) and to `templates/account_row.html` line 8 (`{{ name }}`).

- [ ] **Step 2: Add the CSS**

In the `<style>` block of `templates/index.html` (near `.row-actions`):

```css
.rename-label { display: flex; align-items: center; gap: 6px; }
.rename-btn {
    background: none; border: none; cursor: pointer; padding: 2px;
    color: var(--text-muted, #888); display: inline-flex; opacity: 0;
}
tr:hover .rename-btn { opacity: 1; }
.rename-input { font: inherit; width: 12em; }
.rename-error { color: #e5484d; font-size: 0.8em; margin-left: 6px; }
```

- [ ] **Step 3: Add the JS**

In the existing `<script>` section of `templates/index.html`:

```javascript
function startRename(btn) {
    const label = btn.closest('.rename-label');
    const form = label.nextElementSibling;
    label.hidden = true;
    form.hidden = false;
    const input = form.querySelector('input');
    input.focus();
    input.select();
}
function cancelRename(form) {
    form.hidden = true;
    form.previousElementSibling.hidden = false;
    form.querySelector('.rename-error').textContent = '';
}
document.body.addEventListener('keydown', function(evt) {
    if (evt.key === 'Escape' && evt.target.matches('.rename-form input')) {
        cancelRename(evt.target.closest('.rename-form'));
    }
});
document.body.addEventListener('focusout', function(evt) {
    if (evt.target.matches && evt.target.matches('.rename-form input')) {
        // Let a just-submitted PATCH finish; only cancel if no request is in flight.
        const form = evt.target.closest('.rename-form');
        if (!form.classList.contains('htmx-request')) cancelRename(form);
    }
});
document.body.addEventListener('htmx:responseError', function(evt) {
    const form = evt.detail.elt.closest && evt.detail.elt.closest('.rename-form');
    if (form) {
        form.hidden = false;
        form.previousElementSibling.hidden = true;
        form.querySelector('.rename-error').textContent = evt.detail.xhr.responseText;
    }
});
```

(HTMX 1.9 does not swap 4xx bodies; the `htmx:responseError` listener is how the 422/409 messages from Task 3 land in `.rename-error`.)

- [ ] **Step 4: Build, restart, verify in the UI**

```bash
cargo build --release
# restart the serve process
```

Then in the browser at `http://localhost:3000`: hover a row, click the pencil, rename a throwaway account (create one first as in Task 3 Step 4), confirm the page refreshes with the new name and the row still shows its cached balance. Try an empty name and a colliding name; confirm the error text appears inline. Press Escape; confirm the label returns. Screenshot the result (project convention for UI changes).

- [ ] **Step 5: Full test suite and commit**

```bash
cargo test
```

Expected: all tests pass (report exact counts).

```bash
cargo fmt
git add templates/index.html templates/account_row.html
git commit -m "feat: inline rename control on dashboard account rows"
```
