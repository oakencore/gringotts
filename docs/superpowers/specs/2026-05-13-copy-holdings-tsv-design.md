# Copy Holdings as TSV — Design

**Date:** 2026-05-13
**Status:** Approved (pending implementation plan)

## Problem

The web UI has no quick way to export holdings to a spreadsheet. A user looking at the `/balances` aggregated view or the per-wallet expansion has to retype or manually copy each cell to move data into Excel or Google Sheets.

## Goal

Let a user copy holdings to the clipboard as **tab-separated values** with one click, paste into a spreadsheet, and have the values land in numeric cells (no further import dialogs, no string cells where numbers should be).

## Scope

- `templates/balances.html` — one "Copy as TSV" button in the footer. Copies the full aggregated table (every company / wallet / asset).
- `templates/single_balance.html` — one "Copy as TSV" button in the card header. Copies the per-wallet detail.
- `src/display/web.rs` — `BalancesTemplate` and `SingleBalanceTemplate` each gain a `tsv_export: String` field; the handlers build the TSV alongside the existing view data.

**Out of scope:**

- Dashboard table (`/`). Rows are placeholder `--` until Query is clicked; nothing to copy.
- A "Copy as CSV" alternative. TSV is strictly better for spreadsheet paste (auto-split without an import dialog).
- A "Copied!" toast confirmation. Optional polish; can be added in a follow-up by extending the JS handler with a one-line `aria-live` swap.
- A standalone `/balances/export.tsv` endpoint. Would double the rendering paths to keep in sync. Embedding the TSV in the page is sufficient.
- Per-wallet section buttons on `balances.html`. The single top-of-table button is sufficient — users can filter in the spreadsheet after pasting.

## Design

### Section 1 — `balances.html` (Query All view)

**Button placement:** One copy button inside the existing `<div class="balances-footer">`, next to the portfolio total. Lucide `copy` icon plus the label "Copy as TSV".

**Hidden data:** A `<textarea id="balances-tsv-data" style="display:none">` rendered server-side at the top of the body, containing the pre-formatted TSV. Using a hidden textarea (rather than a `data-` attribute) keeps any whitespace, including tabs and newlines, intact through HTML serialization without needing manual escaping.

**TSV shape:**

```
Company	Wallet	Symbol	Amount	USD Value
Acme	WalletA	SOL	3.500000	350.00
Acme	WalletA	USDC	100.000000	100.00
Acme	WalletB	SOL	7.000000	700.00
```

- Tab between cells, `\n` between rows.
- Single header row.
- One row per `(company, wallet, asset)` triple. The visible UI groups company → wallet → assets; the export flattens it so each row is self-contained.

**Server-side generation:** In the `query_balances` handler, after `companies_view` is built, walk it once more to assemble a `String` for `BalancesTemplate.tsv_export`. Same nested loop shape that produces the visible view, emitting one line per asset.

**JS handler:** A small inline `<script>` at the bottom of `balances.html` (next to the existing `<script>` that sets the timestamp):

```js
document.getElementById('copy-balances-tsv')?.addEventListener('click', async () => {
    const tsv = document.getElementById('balances-tsv-data').value;
    try {
        await navigator.clipboard.writeText(tsv);
    } catch (e) {
        console.error('Clipboard write failed', e);
    }
});
```

### Section 2 — `single_balance.html` (per-wallet detail)

**Button placement:** Copy button inside the existing `single-balance-header`, next to the close button. Same Lucide `copy` icon plus label.

**Hidden data:** A `<textarea>` inside the card carrying TSV with one row per asset:

```
Wallet	Chain	Address	Symbol	Amount	USD Value
WalletA	Solana	So11111111111111111111111111111111111111112	SOL	3.500000	350.00
WalletA	Solana	So11111111111111111111111111111111111111112	USDC	100.000000	100.00
```

Includes the full address so each row is self-contained when pasted alongside rows from other wallets.

**Server-side generation:** `SingleBalanceTemplate` gains a `tsv_export: String` field. The `query_single_balance` handler builds it from the same `tokens` + native data the template renders.

**JS handler:** Same pattern, but scoped to the single-balance card so multiple cards on the same page (if the user opens detail for several wallets) each have their own working button. The textarea is given a unique id (`single-balance-tsv-{name}` or similar) and the script wires its own button.

### Section 3 — Format details

- **TSV format** with tab between cells, `\n` between rows.
- **Header row always present.**
- **Amount precision:** crypto amounts as `{:.6}` (matches the cache's stored precision); USD-denominated currencies as `{:.2}`. No `$`, no thousand separators.
- **Missing USD value:** empty cell (two adjacent tabs), not `--`. Keeps the cell numeric in the spreadsheet.
- **Address column** (single_balance only): full address, untruncated. Display truncation is visual only; the export should be paste-friendly for further lookups.
- **Escaping:** asset symbols and company names that contain a tab character are rare but should be sanitized via a small `escape_tsv(&str) -> String` helper that replaces `\t` and `\n` with a single space. Apply uniformly to all string cells.

### Section 4 — Browser compatibility

`navigator.clipboard.writeText` requires either a secure context (HTTPS) or `localhost`. Since gringotts is local-only, this works out of the box. The `try`/`catch` in the handler logs to console on failure; no UI fallback is added because there is no realistic failure mode on `localhost`.

### Section 5 — Testing

**Unit tests** (in the `src/display/web.rs` test module):

1. `escape_tsv("foo\tbar\nbaz")` returns `"foo bar baz"`.
2. A helper that constructs the balances TSV from a `Vec<(String, Vec<WalletGroup>)>` produces a header line and N+1 lines total for N assets. The first non-header line carries the expected `company\twallet\tsymbol\tamount\tvalue` shape.
3. A helper that constructs the single-balance TSV from a `SingleBalanceTemplate`-equivalent input produces the expected layout, including the empty cell for an asset with no USD value.

**Manual smoke:**

- Click "Copy as TSV" on `/balances`, paste into a Google Sheets or Excel sheet. Confirm the cells split correctly and Amount / USD Value land as numeric (right-aligned, sortable).
- Click the per-wallet copy on a single-balance card, paste, confirm the same.
- Confirm the `display: none` textarea is not visible.

**Quality gates** before opening a PR: `cargo fmt`, `cargo clippy` (no new warnings), `cargo test`.
