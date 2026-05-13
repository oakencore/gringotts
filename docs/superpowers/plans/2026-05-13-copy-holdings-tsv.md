# Copy Holdings as TSV Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-click button on `/balances` and on per-wallet detail cards that copies the holdings to the clipboard as TSV, ready to paste into a spreadsheet with numeric cells parsing as numbers.

**Architecture:** Server pre-renders the TSV body and embeds it as a `data-tsv` attribute on the copy button. A single delegated `click` listener on `document.body` in `base.html` handles all `[data-tsv]` buttons across the app, reading the attribute and calling `navigator.clipboard.writeText`. No new endpoints, no fetch round-trip.

**Tech Stack:** Rust, Askama, Axum, HTMX, vanilla JS. Tests via `cargo test`. Format/lint via `cargo fmt`/`cargo clippy`.

**Spec:** `docs/superpowers/specs/2026-05-13-copy-holdings-tsv-design.md`

---

## File Structure

**Modified files:**
- `src/display/web.rs` — adds `escape_tsv`, `build_balances_tsv`, `build_single_balance_tsv` free functions; extends `BalancesTemplate` and `SingleBalanceTemplate` with `tsv_export: String`; updates 1 + 9 construction sites; adds 5 new unit tests.
- `templates/balances.html` — adds a "Copy as TSV" button in the existing `.balances-footer` div.
- `templates/single_balance.html` — adds a copy button in the existing `.single-balance-header`.
- `templates/base.html` — extends the existing `<script>` block at the bottom with one delegated `click` listener for `[data-tsv]` buttons.

**No new files.**

---

## Chunk 1: TSV building blocks

### Task 1: `escape_tsv` helper

**Files:**
- Modify: `src/display/web.rs` — add helper near other top-level utilities (e.g., near `format_duration` around line 350) and unit test in the `#[cfg(test)] mod tests {}` block.

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)] mod tests {}` block of `src/display/web.rs` (after the existing tests), add:

```rust
#[test]
fn test_escape_tsv_strips_control_chars() {
    assert_eq!(escape_tsv("foo\tbar\nbaz\rqux"), "foo bar baz qux");
    assert_eq!(escape_tsv("clean"), "clean");
    assert_eq!(escape_tsv(""), "");
    // Adjacent control chars collapse to one space each, not deduplicated
    assert_eq!(escape_tsv("a\t\tb"), "a  b");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test test_escape_tsv -- --test-threads=1 2>&1 | tail -15
```

Expected: compilation error — `escape_tsv` not defined.

- [ ] **Step 3: Implement `escape_tsv`**

Add a free function near `format_duration` in `src/display/web.rs`:

```rust
/// Replace tab, newline, and CR with a single space each so the value
/// is safe to embed in a TSV cell. Other characters pass through.
fn escape_tsv(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo test test_escape_tsv -- --test-threads=1 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Commit**

Write commit message to `/tmp/task1-msg.txt`:

```
feat: escape_tsv helper for spreadsheet-safe TSV cells

Replace tab/newline/CR with single space so user-controlled strings
(company, wallet, symbol, chain, address) remain safe to embed in
TSV cells exported for spreadsheet paste.
```

Commit:
```bash
git add src/display/web.rs
git commit -F /tmp/task1-msg.txt && rm -f /tmp/task1-msg.txt
```

---

### Task 2: `build_balances_tsv` helper

**Files:**
- Modify: `src/display/web.rs` — add helper near `escape_tsv` and unit test.

- [ ] **Step 1: Write the failing test**

In the test module, add:

```rust
#[test]
fn test_build_balances_tsv_flattens_company_wallet_asset() {
    let companies: Vec<(String, Vec<WalletGroup>)> = vec![(
        "Acme".to_string(),
        vec![
            WalletGroup {
                name: "WalletA".to_string(),
                total_usd: 400.0,
                assets: vec![
                    AssetView { symbol: "SOL".to_string(), amount: 3.0, usd_value: 300.0 },
                    AssetView { symbol: "USDC".to_string(), amount: 100.0, usd_value: 100.0 },
                ],
            },
            WalletGroup {
                name: "WalletB".to_string(),
                total_usd: 0.0,
                assets: vec![
                    AssetView { symbol: "UNPRICED".to_string(), amount: 5.5, usd_value: 0.0 },
                ],
            },
        ],
    )];

    let tsv = build_balances_tsv(&companies);
    let lines: Vec<&str> = tsv.lines().collect();

    assert_eq!(lines[0], "Company\tWallet\tSymbol\tAmount\tUSD Value");
    assert_eq!(lines[1], "Acme\tWalletA\tSOL\t3.000000\t300.00");
    assert_eq!(lines[2], "Acme\tWalletA\tUSDC\t100.000000\t100.00");
    // Empty USD cell for value <= 0.0 (two adjacent tabs at end)
    assert_eq!(lines[3], "Acme\tWalletB\tUNPRICED\t5.500000\t");
    assert_eq!(lines.len(), 4);
}

#[test]
fn test_build_balances_tsv_escapes_control_chars_in_strings() {
    let companies: Vec<(String, Vec<WalletGroup>)> = vec![(
        "Ac\tme".to_string(),
        vec![WalletGroup {
            name: "Wal\nletA".to_string(),
            total_usd: 100.0,
            assets: vec![AssetView { symbol: "S\rOL".to_string(), amount: 1.0, usd_value: 100.0 }],
        }],
    )];
    let tsv = build_balances_tsv(&companies);
    let lines: Vec<&str> = tsv.lines().collect();
    // String cells get control chars replaced with spaces; numeric cells unaffected
    assert_eq!(lines[1], "Ac me\tWal letA\tS OL\t1.000000\t100.00");
}

#[test]
fn test_build_balances_tsv_empty_input_returns_header_only() {
    let tsv = build_balances_tsv(&[]);
    assert_eq!(tsv, "Company\tWallet\tSymbol\tAmount\tUSD Value\n");
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test test_build_balances_tsv -- --test-threads=1 2>&1 | tail -10
```

Expected: compilation error — `build_balances_tsv` not defined.

- [ ] **Step 3: Implement `build_balances_tsv`**

Add a free function in `src/display/web.rs` (place after `escape_tsv`):

```rust
/// Build a TSV string from the balances template's `companies` data.
/// Flattens (company, wallet, asset) into one row per asset. Header
/// row is always included. USD values <= 0.0 render as an empty cell
/// so the spreadsheet column stays numeric.
fn build_balances_tsv(companies: &[(String, Vec<WalletGroup>)]) -> String {
    let mut tsv = String::from("Company\tWallet\tSymbol\tAmount\tUSD Value\n");
    for (company, wallets) in companies {
        let company_clean = escape_tsv(company);
        for wallet in wallets {
            let wallet_clean = escape_tsv(&wallet.name);
            for asset in &wallet.assets {
                let symbol_clean = escape_tsv(&asset.symbol);
                let usd_cell = if asset.usd_value > 0.0 {
                    format!("{:.2}", asset.usd_value)
                } else {
                    String::new()
                };
                tsv.push_str(&format!(
                    "{}\t{}\t{}\t{:.6}\t{}\n",
                    company_clean, wallet_clean, symbol_clean, asset.amount, usd_cell
                ));
            }
        }
    }
    tsv
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo test test_build_balances_tsv -- --test-threads=1 2>&1 | tail -10
```

Expected: all three tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/display/web.rs
git commit -m "feat: build_balances_tsv flattens company/wallet/asset to TSV"
```

---

### Task 3: `build_single_balance_tsv` helper

**Files:**
- Modify: `src/display/web.rs`.

- [ ] **Step 1: Write the failing test**

Add to the test module:

```rust
#[test]
fn test_build_single_balance_tsv_includes_native_and_tokens() {
    let tokens = vec![
        TokenView { symbol: "USDC".to_string(), balance: 100.0, usd_value: 100.0 },
        TokenView { symbol: "UNPRICED".to_string(), balance: 5.5, usd_value: 0.0 },
    ];
    let tsv = build_single_balance_tsv(
        "WalletA",
        "Solana",
        "So11111111111111111111111111111111111111112",
        "SOL",
        3.0,
        300.0,
        &tokens,
    );
    let lines: Vec<&str> = tsv.lines().collect();

    assert_eq!(lines[0], "Wallet\tChain\tAddress\tSymbol\tAmount\tUSD Value");
    assert_eq!(
        lines[1],
        "WalletA\tSolana\tSo11111111111111111111111111111111111111112\tSOL\t3.000000\t300.00"
    );
    assert_eq!(
        lines[2],
        "WalletA\tSolana\tSo11111111111111111111111111111111111111112\tUSDC\t100.000000\t100.00"
    );
    // Empty USD cell for unpriced token
    assert_eq!(
        lines[3],
        "WalletA\tSolana\tSo11111111111111111111111111111111111111112\tUNPRICED\t5.500000\t"
    );
    assert_eq!(lines.len(), 4);
}

#[test]
fn test_build_single_balance_tsv_native_only_no_tokens() {
    let tsv = build_single_balance_tsv("BankA", "Mercury", "acc_123", "USD", 1500.0, 1500.0, &[]);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1], "BankA\tMercury\tacc_123\tUSD\t1500.000000\t1500.00");
}

#[test]
fn test_build_single_balance_tsv_zero_native_omits_native_row() {
    // If the wallet has no native balance (e.g., zeroed-out), skip the native row.
    // We still want the header and any token rows.
    let tokens = vec![TokenView {
        symbol: "USDC".to_string(),
        balance: 100.0,
        usd_value: 100.0,
    }];
    let tsv = build_single_balance_tsv("WalletA", "Solana", "addr", "SOL", 0.0, 0.0, &tokens);
    let lines: Vec<&str> = tsv.lines().collect();
    // Header + 1 token row only (native row omitted because amount is 0)
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1], "WalletA\tSolana\taddr\tUSDC\t100.000000\t100.00");
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test test_build_single_balance_tsv -- --test-threads=1 2>&1 | tail -10
```

Expected: compilation error — function not defined.

- [ ] **Step 3: Implement `build_single_balance_tsv`**

Add after `build_balances_tsv`:

```rust
/// Build a TSV string for a single-wallet detail view. Includes the
/// native balance as the first asset row (skipped if native_balance
/// == 0.0), followed by each token in `tokens`. Each row carries the
/// full address so pasted rows are self-contained.
#[allow(clippy::too_many_arguments)]
fn build_single_balance_tsv(
    wallet_name: &str,
    chain: &str,
    address: &str,
    native_symbol: &str,
    native_balance: f64,
    native_usd: f64,
    tokens: &[TokenView],
) -> String {
    let mut tsv = String::from("Wallet\tChain\tAddress\tSymbol\tAmount\tUSD Value\n");
    let wallet_clean = escape_tsv(wallet_name);
    let chain_clean = escape_tsv(chain);
    let address_clean = escape_tsv(address);

    let emit_row = |tsv: &mut String, symbol: &str, amount: f64, usd: f64| {
        let usd_cell = if usd > 0.0 {
            format!("{:.2}", usd)
        } else {
            String::new()
        };
        tsv.push_str(&format!(
            "{}\t{}\t{}\t{}\t{:.6}\t{}\n",
            wallet_clean,
            chain_clean,
            address_clean,
            escape_tsv(symbol),
            amount,
            usd_cell
        ));
    };

    if native_balance != 0.0 {
        emit_row(&mut tsv, native_symbol, native_balance, native_usd);
    }
    for token in tokens {
        emit_row(&mut tsv, &token.symbol, token.balance, token.usd_value);
    }
    tsv
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo test test_build_single_balance_tsv -- --test-threads=1 2>&1 | tail -10
```

Expected: all three tests pass.

- [ ] **Step 5: Verify full test suite still passes**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: `test result: ok. 50 passed; 0 failed` (44 baseline + 1 escape + 3 balances + 3 single_balance — wait, recount: 1 + 3 + 3 = 7 new, so 51. Let me sanity-check: baseline after PR #2 merge was 44; this chunk adds 1 + 3 + 3 = 7 new tests → 51 total).

Adjust the expected count to whatever `cargo test` actually reports.

- [ ] **Step 6: Commit**

```bash
git add src/display/web.rs
git commit -m "feat: build_single_balance_tsv for per-wallet TSV export"
```

---

## Chunk 2: Wire balances.html

### Task 4: Add `tsv_export` to `BalancesTemplate` and the handler

**Files:**
- Modify: `src/display/web.rs` — `BalancesTemplate` struct around line 113; `query_balances` handler (~lines 2925-2969); the two error-path `BalancesTemplate` constructions earlier in `query_balances`.

- [ ] **Step 1: Add `tsv_export` field to `BalancesTemplate`**

Edit the struct at `src/display/web.rs` around line 113:

```rust
#[derive(Template)]
#[template(path = "balances.html")]
struct BalancesTemplate {
    total_usd: f64,
    companies: Vec<(String, Vec<WalletGroup>)>,
    tsv_export: String,
    error: String,
}
```

- [ ] **Step 2: Wire the success-path construction**

In `query_balances`, find the success-path `Html(BalancesTemplate { ... })` construction at the end of the function (around line 2962).

Because the existing literal moves `companies_view` into the `companies` field, the TSV builder (which borrows `&companies_view`) must run **before** the literal. Compute the TSV into a local first:

```rust
let tsv = build_balances_tsv(&companies_view);

Html(
    BalancesTemplate {
        total_usd: portfolio.total_usd_value,
        companies: companies_view,
        tsv_export: tsv,
        error: String::new(),
    }
    .render()
    .unwrap_or_default(),
)
```

This is the same "local-first" pattern used for all the `SingleBalanceTemplate` success paths in Task 6.

- [ ] **Step 3: Wire the two error-path constructions**

`query_balances` has two early-return `BalancesTemplate` constructions (the `Err` branch on `AddressBook::load` and the empty-book branch). Both currently look like:

```rust
return Html(
    BalancesTemplate {
        total_usd: 0.0,
        companies: vec![],
        error: format!("Failed to load accounts: {}", e),
    }
    .render()
    .unwrap_or_default(),
);
```

Add `tsv_export: String::new(),` before `error` in both:

```rust
return Html(
    BalancesTemplate {
        total_usd: 0.0,
        companies: vec![],
        tsv_export: String::new(),
        error: format!("Failed to load accounts: {}", e),
    }
    .render()
    .unwrap_or_default(),
);
```

Use grep to find all `BalancesTemplate {` construction sites and verify each carries a `tsv_export`:

```bash
grep -n "BalancesTemplate {" src/display/web.rs
```

Expected: 3 sites in `query_balances` plus the struct definition. All three constructions should have the new field.

- [ ] **Step 4: Verify build**

```bash
cargo build 2>&1 | tail -5
```

Expected: clean build. The Rust compiler may emit a `dead_code` warning on `BalancesTemplate.tsv_export` until Task 5 wires the field into the Askama template; that warning is expected and disappears in the next task. Do not silence it with `#[allow(dead_code)]` — it's signal that Task 5 is still pending.

- [ ] **Step 5: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: same test count as Chunk 1 final (e.g., 51 passed).

- [ ] **Step 6: Commit**

```bash
git add src/display/web.rs
git commit -m "feat: thread tsv_export through BalancesTemplate"
```

---

### Task 5: Add "Copy as TSV" button to `balances.html`

**Files:**
- Modify: `templates/balances.html` — add button in the existing `.balances-footer`.

- [ ] **Step 1: Insert the button**

Find the `.balances-footer` div (around line 69-75). Currently:

```html
<div class="balances-footer">
    <div class="portfolio-total">
        <span class="total-label">Total Portfolio Value</span>
        <span class="total-value">${{ total_usd|format_usd }}</span>
    </div>
    <span class="timestamp">Updated: <span id="balance-time"></span></span>
</div>
```

The footer uses `display: flex; justify-content: space-between;`, so the two existing children (`.portfolio-total` and the timestamp span) sit on opposite ends. To preserve that layout while adding a third child, wrap the timestamp AND the new button into a single right-side container. The existing timestamp span is **moved** from being a direct child of `.balances-footer` to being a child of the new `.balances-footer-right` wrapper:

```html
<div class="balances-footer">
    <div class="portfolio-total">
        <span class="total-label">Total Portfolio Value</span>
        <span class="total-value">${{ total_usd|format_usd }}</span>
    </div>
    <div class="balances-footer-right">
        <button class="btn btn-secondary btn-sm" data-tsv="{{ tsv_export }}">
            <i data-lucide="copy" style="width: 14px; height: 14px;"></i>
            <span>Copy as TSV</span>
        </button>
        <span class="timestamp">Updated: <span id="balance-time"></span></span>
    </div>
</div>
```

Note: wrapping the button and timestamp in `.balances-footer-right` keeps the footer's existing flex layout (`justify-content: space-between` between left and right groups).

Add a tiny CSS rule to the existing `<style>` block at the bottom of the file (after the `.balances-footer` rule):

```css
.balances-footer-right {
    display: flex;
    align-items: center;
    gap: 16px;
}
```

- [ ] **Step 2: Verify build**

```bash
cargo build 2>&1 | tail -5
```

Expected: clean build. The Askama macro now references `tsv_export`, so the `dead_code` warning from Task 4 Step 4 disappears.

- [ ] **Step 3: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: same count as before.

- [ ] **Step 4: Commit**

```bash
git add templates/balances.html
git commit -m "feat: Copy as TSV button on balances.html footer"
```

---

## Chunk 3: Wire single_balance.html

### Task 6: Add `tsv_export` to `SingleBalanceTemplate` and all construction sites

**Files:**
- Modify: `src/display/web.rs` — `SingleBalanceTemplate` struct at line 136; 9 construction sites (use grep to locate).

`SingleBalanceTemplate` is constructed in many places: error paths in `query_single_balance` (2 sites), `query_wallet_balance` success path (1 site), `query_bank_balance` Mercury/Circle success and error paths (~6 sites). All need to set `tsv_export`.

- [ ] **Step 1: Add `tsv_export` field to the struct**

Edit `SingleBalanceTemplate` at line 136:

```rust
#[derive(Template)]
#[template(path = "single_balance.html")]
struct SingleBalanceTemplate {
    name: String,
    address: String,
    chain: String,
    native_symbol: String,
    native_balance: f64,
    native_usd: f64,
    tokens: Vec<TokenView>,
    total_usd: f64,
    tsv_export: String,
    error: String,
}
```

- [ ] **Step 2: Enumerate construction sites and update each**

```bash
grep -n "SingleBalanceTemplate {" src/display/web.rs
```

Expected: 1 struct definition + 9 construction sites. Each construction site needs a `tsv_export` field added before `error`.

For **error-path constructions** (where the template carries no asset data), use:

```rust
tsv_export: String::new(),
```

For **success-path constructions** (in `query_wallet_balance` and `query_bank_balance` success branches), the existing literals move `chain_name`/`service_name` and `native_symbol` into struct fields. The TSV builder needs to **borrow** those values. Calling `build_single_balance_tsv(&chain_name, ...)` inline within the struct literal **fails to compile** (borrow-after-move) because Rust evaluates struct fields top-to-bottom and `chain: chain_name` moves before `tsv_export` reads it.

**Pattern: compute the TSV into a local first, then move into the struct literal.** This mirrors the pattern from Task 4 Step 2 (`let tsv = build_balances_tsv(&companies_view);` before the `BalancesTemplate { ... }` literal).

Each construction site is rewritten as:

```rust
let tsv = build_single_balance_tsv(
    &wallet.name,
    &chain_name,
    &wallet.address,
    &native_symbol,
    native_balance,
    native_usd,
    &tokens,
);
Html(
    SingleBalanceTemplate {
        // ... existing fields ...
        tsv_export: tsv,
        error,
    }
    .render()
    .unwrap_or_default(),
)
```

**Site-by-site enumeration** (use `grep -n "SingleBalanceTemplate {" src/display/web.rs` to locate exact lines, which may have shifted):

| Site | Path | Action |
|---|---|---|
| `query_single_balance` AddressBook load error (~2977) | Error path | `tsv_export: String::new()` inline (no moves to worry about) |
| `query_single_balance` not-found (~3005) | Error path | `tsv_export: String::new()` inline |
| `query_wallet_balance` success (~3168) | Crypto success | Compute `let tsv = build_single_balance_tsv(&wallet.name, &chain_name, &wallet.address, &native_symbol, native_balance, native_usd, &tokens);` BEFORE the `Html(SingleBalanceTemplate { ... })` literal |
| `query_bank_balance` Mercury success (~3191) | Mercury success | Compute `let tsv = build_single_balance_tsv(&account.name, &service_name, &account.account_id, "USD", balances.current_balance, balances.current_balance, &[]);` BEFORE the literal |
| `query_bank_balance` Mercury fetch error (~3206) | Error path | `tsv_export: String::new()` inline |
| `query_bank_balance` Mercury client-init error (~3222) | Error path | `tsv_export: String::new()` inline |
| `query_bank_balance` Circle success (~3256) | Circle success | Variables in scope at this site: `tokens: Vec<TokenView>`, `total: f64`, `service_name: String` (will be moved), `account.name`, `account.account_id`. Compute `let tsv = build_single_balance_tsv(&account.name, &service_name, &account.account_id, "USD", total, total, &tokens);` BEFORE the literal |
| `query_bank_balance` Circle fetch error (~3272) | Error path | `tsv_export: String::new()` inline |
| `query_bank_balance` Circle client-init error (~3288) | Error path | `tsv_export: String::new()` inline |

- [ ] **Step 3: Verify build**

```bash
cargo build 2>&1 | tail -10
```

Expected: clean build. If you missed a construction site, the compiler will name it. Like Task 4 Step 4, a `dead_code` warning on `SingleBalanceTemplate.tsv_export` is expected until Task 7 wires `single_balance.html` to read it. Don't silence it with `#[allow(dead_code)]`.

Sanity check the site count: `grep -n "SingleBalanceTemplate {" src/display/web.rs | wc -l` should return `10` (1 struct definition + 9 constructions).

- [ ] **Step 4: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: same count.

- [ ] **Step 5: Commit**

```bash
git add src/display/web.rs
git commit -m "feat: thread tsv_export through SingleBalanceTemplate"
```

---

### Task 7: Add copy button to `single_balance.html`

**Files:**
- Modify: `templates/single_balance.html` — add button in the existing `.single-balance-header`.

- [ ] **Step 1: Insert the button**

Find `.single-balance-header` (lines 2-13). The current close button is at line 10. Add a copy button immediately before the close button:

```html
<div class="single-balance-header">
    <div class="balance-info">
        <h3>{{ name }}</h3>
        <div class="balance-meta">
            <span class="chain-badge">{{ chain }}</span>
            <span class="address-display">{{ address }}</span>
        </div>
    </div>
    <div class="single-balance-actions">
        <button class="btn btn-secondary btn-sm" data-tsv="{{ tsv_export }}" title="Copy as TSV">
            <i data-lucide="copy" style="width: 14px; height: 14px;"></i>
        </button>
        <button class="btn btn-secondary btn-sm" onclick="this.closest('.single-balance-card').remove()">
            <i data-lucide="x" style="width: 14px; height: 14px;"></i>
        </button>
    </div>
</div>
```

Add a small CSS rule to the existing `<style>` block at the bottom of `single_balance.html`:

```css
.single-balance-actions {
    display: flex;
    gap: 6px;
}
```

- [ ] **Step 2: Verify build**

```bash
cargo build 2>&1 | tail -5
```

Expected: clean build.

- [ ] **Step 3: Run tests**

```bash
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: same count.

- [ ] **Step 4: Commit**

```bash
git add templates/single_balance.html
git commit -m "feat: Copy as TSV button on single_balance card"
```

---

## Chunk 4: Wire the JS handler

### Task 8: Delegated click listener in `base.html`

**Files:**
- Modify: `templates/base.html` — extend the existing `<script>` block at lines 591-599.

- [ ] **Step 1: Add the delegated listener**

In `templates/base.html`, find the existing `<script>` block at lines 591-599:

```html
<script>
    // Initialize Lucide icons
    lucide.createIcons();

    // Re-initialize icons after HTMX swaps
    document.body.addEventListener('htmx:afterSwap', function() {
        lucide.createIcons();
    });
</script>
```

Add a new listener inside the same `<script>` block (after the `htmx:afterSwap` listener):

```html
<script>
    // Initialize Lucide icons
    lucide.createIcons();

    // Re-initialize icons after HTMX swaps
    document.body.addEventListener('htmx:afterSwap', function() {
        lucide.createIcons();
    });

    // Delegated click handler for any button with data-tsv: copy to clipboard.
    document.body.addEventListener('click', async function(e) {
        const btn = e.target.closest('[data-tsv]');
        if (!btn) return;
        try {
            await navigator.clipboard.writeText(btn.dataset.tsv);
        } catch (err) {
            console.error('Clipboard write failed', err);
        }
    });
</script>
```

No visual feedback beyond the OS's normal clipboard-copy behavior. Adding a check-icon swap was considered but rejected: `lucide.createIcons()` replaces `<i data-lucide="...">` with an `<svg>` element that no longer carries `data-lucide`, so subsequent attempts to toggle the icon attribute would no-op. A working feedback affordance would require either a separate non-Lucide indicator (e.g., a temporary `.copied` class swap on the button) or a toast — both out of scope for this PR per the spec.

- [ ] **Step 2: Verify build and tests**

```bash
cargo build 2>&1 | tail -5
cargo test -- --test-threads=1 2>&1 | tail -3
```

Expected: clean build, same test count.

- [ ] **Step 3: Commit**

```bash
git add templates/base.html
git commit -m "feat: delegated click listener for data-tsv copy buttons"
```

---

## Chunk 5: Verification and PR

### Task 9: Quality gates and PR

- [ ] **Step 1: Format**

```bash
cargo fmt
```

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets 2>&1 | tail -10
```

Expected: no NEW warnings vs. baseline (13 pre-existing dead-code warnings on legacy modules).

- [ ] **Step 3: Full test suite**

```bash
cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: 7 new tests added (1 escape_tsv + 3 balances + 3 single_balance), so total = baseline + 7 = 51 passing.

- [ ] **Step 4: Manual smoke test**

```bash
cargo build --release
./target/release/gringotts serve --port 3000
```

In a browser at `http://localhost:3000`:

1. Click **Query All**. Click the "Copy as TSV" button in the footer.
2. Paste into a Google Sheets or Excel sheet. Verify:
   - Cells split correctly across Company / Wallet / Symbol / Amount / USD Value columns.
   - Amount and USD Value cells are right-aligned and sortable as numbers.
   - Rows with no USD value have an empty USD column (not `--` or `0`).
3. On the dashboard, click **Query** on one wallet row to load a single-balance card. Click the copy button (the icon with the dual-page glyph) in the card header.
4. Paste into the same sheet. Verify the per-wallet rows include the full address.
5. Open DevTools console. Click each copy button again. Confirm no `Clipboard write failed` error is logged. (There is no in-UI feedback by design — the OS clipboard is the source of truth.)

- [ ] **Step 5: Commit any fmt/clippy fixes**

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: cargo fmt and clippy cleanup"
```

- [ ] **Step 6: Open the PR**

```bash
git push -u origin feat/copy-holdings-tsv
```

Write PR body to `/tmp/pr-body.md`:

```markdown
## Summary
- One-click "Copy as TSV" button on `/balances` (full table) and on each single-wallet detail card
- Output is tab-separated with raw decimal amounts and USD values, designed to paste straight into Excel or Google Sheets without an import dialog
- All wiring is server-side: TSV is pre-rendered into a `data-tsv` attribute on the button; a single delegated JS click listener in `base.html` covers every `[data-tsv]` button across the app
- 7 new unit tests cover `escape_tsv`, `build_balances_tsv`, and `build_single_balance_tsv`

Spec: `docs/superpowers/specs/2026-05-13-copy-holdings-tsv-design.md`

## Design notes
- Used `data-tsv` attribute on the button rather than a hidden `<textarea>` keyed by id, so user-controlled wallet/company names are safely Askama-escaped without any custom id sanitization
- Empty cell (not `--`) when `usd_value <= 0.0`, matching the existing template's `> 0.0` sentinel and keeping the spreadsheet column numeric
- `escape_tsv` strips `\t`, `\n`, `\r` from every string cell

## Test plan
- [x] `cargo fmt --check` clean
- [x] `cargo clippy --all-targets` no new warnings
- [x] `cargo test` — 51 passing (44 baseline + 7 new)
- [ ] Manual smoke: paste into Sheets, confirm numeric cells, confirm full addresses on per-wallet rows

## Out of scope (per spec)
- "Copy as CSV" alternative
- Toast or in-UI feedback (no visual confirmation; OS clipboard is the source of truth)
- Dashboard table copy (rows are placeholders until Query is clicked)
- Per-wallet section buttons on `balances.html`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Create the PR:

```bash
gh pr create --base main --title "feat: copy holdings to clipboard as TSV" --body-file /tmp/pr-body.md
rm -f /tmp/pr-body.md
```

---
