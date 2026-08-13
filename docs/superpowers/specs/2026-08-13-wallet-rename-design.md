# Wallet and Banking Account Rename - Design

Date: 2026-08-13
Status: Approved (verbal, this session)

## Goal

Allow renaming crypto wallets and banking accounts from the web UI. A wallet's
`name` is its identity across the system (storage key in `addresses.json`, URL
param on `/balances/:name`, key for cached balances in `cache.json`), so a
rename must update storage and re-key the cache atomically from the user's
point of view.

## Scope

- In: inline rename in the web UI for both wallets and banking accounts,
  backend endpoint, cache re-key, validation, tests.
- Out: CLI rename command, editing any other field (company, chain, address).

## API

`PATCH /accounts/:name` with form field `new_name`.

- Trim `new_name`. Reject empty with 422 and an inline error message.
- Reject if the trimmed name collides with any existing wallet or banking
  account name (409, inline error). Comparison is exact (case-sensitive),
  matching how names are used as keys everywhere else.
- Renaming to the same name is a no-op success.
- Lookup order: wallets first, then banking accounts - same as the existing
  `remove_account` handler.
- Unknown `:name` returns 404.

## Persistence

1. Update the entry's `name` in the `AddressBook` and save via the existing
   atomic write in `storage.rs`.
2. Move `cache.balances[old_name]` to `cache.balances[new_name]` (if present)
   and persist the cache, so the dashboard keeps showing the balance instead
   of treating it as an orphan entry.
3. Order: address book first, then cache. If the cache write fails the entry
   is refetched under the new name on the next refresh - degraded, not broken.

## UI

- Each row in the accounts list gets an edit affordance next to the name.
- Clicking it swaps the name for a text input pre-filled with the current
  name (HTMX, matching existing patterns in `display/web.rs`).
- Submit (Enter or button) sends the PATCH; success re-renders the affected
  section with the new name. Escape or blur cancels and restores the label.
- Validation errors (empty, collision) render inline next to the input.

## Testing

- Unit tests for the rename logic: success (wallet and banking account),
  cache entry re-keyed, name collision rejected, unknown name rejected,
  empty/whitespace name rejected, same-name no-op.
- Manual curl smoke test against the running server, then a UI check.
