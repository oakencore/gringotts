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

**Data carrier:** A `data-tsv="..."` attribute on the copy button itself. Askama HTML-escapes the value automatically, so embedded tab characters survive as `&#9;` in source and decode to literal tabs on `dataset.tsv` read. Strings inside the TSV are passed through `escape_tsv` first to remove any raw tab/newline/CR (see Section 3) before assembly, so no double-escaping is required at the template layer.

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

**JS handler:** Shared with Section 2 via a single delegated listener on `document.body`. See Section 2 for the full snippet — it covers both this button and the single-balance buttons.

### Section 2 — `single_balance.html` (per-wallet detail)

**Button placement:** Copy button inside the existing `single-balance-header`, next to the close button. Same Lucide `copy` icon plus label.

**TSV shape** (one row per asset, full address per row for self-containment):

```
Wallet	Chain	Address	Symbol	Amount	USD Value
WalletA	Solana	So11111111111111111111111111111111111111112	SOL	3.500000	350.00
WalletA	Solana	So11111111111111111111111111111111111111112	USDC	100.000000	100.00
```

**Server-side generation:** `SingleBalanceTemplate` gains a `tsv_export: String` field. The `query_single_balance` handler builds it from the same `tokens` + native data the template renders.

**Data carrier:** A `data-tsv="..."` attribute on the button itself rather than a hidden textarea keyed by id. Askama auto-escapes attribute values, so user-controlled `wallet.name` strings (which may contain spaces or other characters that would break a naive `id=`) are handled safely with zero extra logic. Multiple single-balance cards on the same page work without unique-id juggling — each card's button carries its own `data-tsv`.

**JS handler:** A delegated listener on `document.body` for `click` events targeting `[data-tsv]` reads the attribute and copies. One inline `<script>` in `base.html` (or in each template) handles both balances and single-balance buttons:

```js
document.body.addEventListener('click', async (e) => {
    const btn = e.target.closest('[data-tsv]');
    if (!btn) return;
    try {
        await navigator.clipboard.writeText(btn.dataset.tsv);
    } catch (err) {
        console.error('Clipboard write failed', err);
    }
});
```

Implementation note: `data-tsv` is read via `dataset.tsv` (JS auto-converts kebab to camel). The same delegated listener works for the balances button in Section 1 — Section 1's JS snippet is therefore unnecessary; the listener defined here covers both. Move it to `base.html` so it's loaded once.

### Section 3 — Format details

- **TSV format** with tab between cells, `\n` between rows.
- **Header row always present.**
- **Amount precision:** crypto amounts as `{:.6}` (matches the cache's stored precision); USD-denominated currencies as `{:.2}`. No `$`, no thousand separators.
- **Zero or missing USD value:** the data model carries `usd_value: f64` (not `Option<f64>`). The visible template already treats `usd_value > 0.0` as "has price" and renders `--` otherwise (`templates/balances.html` line 46 in the merged tree). The TSV mirrors this: when `usd_value > 0.0`, emit `{:.2}`; otherwise emit an empty cell (two adjacent tabs). Empty keeps the spreadsheet column numeric.
- **Address column** (single_balance only): full address, untruncated. Display truncation is visual only; the export should be paste-friendly for further lookups.
- **Escaping:** asset symbols and company names rarely contain tab/newline/CR characters but the export must be robust. A small `escape_tsv(&str) -> String` helper replaces `\t`, `\n`, and `\r` each with a single space. Apply uniformly to every string cell (company, wallet, symbol, chain, address). Numeric cells (amount, USD value) bypass the helper since they are formatted from `f64` and cannot contain the offending characters.

### Section 4 — Browser compatibility

`navigator.clipboard.writeText` requires either a secure context (HTTPS) or `localhost`. Since gringotts is local-only, this works out of the box. The `try`/`catch` in the handler logs to console on failure; no UI fallback is added because there is no realistic failure mode on `localhost`.

### Section 5 — Testing

**Unit tests** (in the `src/display/web.rs` test module):

1. `escape_tsv("foo\tbar\nbaz\rqux")` returns `"foo bar baz qux"` (tab, newline, and CR each become a single space).
2. A helper that constructs the balances TSV from a `Vec<(String, Vec<WalletGroup>)>` produces a header line and N+1 lines total for N assets. The first non-header line carries the expected `company\twallet\tsymbol\tamount\tvalue` shape.
3. A helper that constructs the single-balance TSV from a `SingleBalanceTemplate`-equivalent input produces the expected layout, including the empty cell for an asset with no USD value.

**Manual smoke:**

- Click "Copy as TSV" on `/balances`, paste into a Google Sheets or Excel sheet. Confirm the cells split correctly and Amount / USD Value land as numeric (right-aligned, sortable).
- Click the per-wallet copy on a single-balance card, paste, confirm the same.
- Confirm the `display: none` textarea is not visible.

**Quality gates** before opening a PR: `cargo fmt`, `cargo clippy` (no new warnings), `cargo test`.
