# ADR 0026: Approved journal, account, and statement contracts

Status: accepted; implements the existing Version 01 decisions.

## Journal metadata

Migration `0005` adds an optional user reference and optional line memo. The
existing explanation column now participates in ordinary draft save, replace,
post, and read operations. Journal and ledger reports carry these fields.

- Text is preserved exactly; absence is `NULL`, and empty strings remain empty.
- Null characters are rejected by both domain validation and SQLite. References
  do not imply automatic numbering or uniqueness.
- A complete draft replacement explicitly supplies its header and line set.
  Omitting optional metadata clears it. Replacements, deletion, and posting
  retain expected-revision checks and atomicity.
- Posted metadata remains immutable under the existing posted-row triggers.
- A correcting request's explanation is authoritative; a conflicting explanation
  in its entry header is rejected instead of discarded.
- Generated reversals retain the existing exact financial line-copy rule; they
  start with empty optional reference, explanation, and memos. Original metadata
  remains available on the linked original entry.

## Accounts

Migration `0006` implements the approved sibling naming and activation policy.
Names compare after existing Unicode edge-whitespace trimming, using exact,
case-sensitive Unicode text. No case folding or Unicode composition is added.
Root nodes share the root sibling namespace. Inactive accounts retain names.
Creation order remains the sibling display order.

A category cannot be deactivated while any descendant remains active. Creation,
reactivation, and subtree moves also preserve that final-state invariant:
activate ancestors first, and deactivate descendants first. SQL recursive
triggers, domain errors, defensive tree/path reads, and startup integrity checks
apply this rule. An account with journal history already cannot change kind or
acquire children under the existing migration `0002` protections.

## As-of balance sheet

The statement reads the complete account hierarchy and posted amounts from one
SQLite snapshot. It exposes the explicit hierarchy without inferring
current/non-current categories from names or dates. Assets use debit-minus-
credit; liabilities and equity use credit-minus-debit. Contra balances remain
signed. Cumulative unclosed income minus expense is shown separately, so assets
reconcile to liabilities, recorded equity, and the unclosed result.

No closing entry, accounting-period lock, classification rewrite, or mutable
balance is introduced. Exact checked amounts and the existing trial-balance
reconciliation remain mandatory. This is personal accounting guided by IFRS
concepts, without a claim of complete IFRS compliance.

## Evidence

`journal_metadata`, `account_policy`, `balance_sheet`, existing accounting tests,
and API schema tests cover preservation, conflicts, SQL bypass attempts,
reconciliation, drafts, inactive history, exact large integers, and corruption.
