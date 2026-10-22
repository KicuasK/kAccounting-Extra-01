# ADR 0003: Exact money and policy-neutral journal-line validation

Status: accepted for the accounting-core foundation

## Decision

- Represent monetary values as signed 64-bit integer minor units in Rust and SQLite `INTEGER` columns.
- Expose checked addition, subtraction, and negation only; arithmetic overflow is an explicit error.
- Represent a candidate journal line as one account, one debit-or-credit side, and one strictly positive amount.
- Require at least two lines and exactly equal debit and credit totals before an entry can be posted.
- Keep mutable account checks out of the pure validator. Account existence, active state, posting kind, and leaf status are checked again inside each SQLite posting transaction.

For the current single-currency MVP, values are exact whole New Taiwan dollars:
one stored unit displays as `NT$1`, with no fractional Version 01 amount. This
scale was approved in `VERSION01_DECISIONS.md` on 2026-10-22. This decision does
not add multi-currency support or settle a future currency-table design.

## Rationale

Binary floating point cannot represent many decimal values exactly and is prohibited by the project invariants. SQLite's exact signed `INTEGER` range maps directly to Rust `i64`; checked arithmetic prevents release-mode wrapping at its boundaries.

Release builds also keep Rust integer-overflow checks enabled as a secondary
guard against programmer mistakes outside the explicit monetary operations.
This does not replace the domain's checked arithmetic and structured overflow
errors.

Keeping the line model independent from the eventual editor avoids deciding whether the UI uses two amount columns, a side selector, or a responsive hybrid. Likewise, the pure validation layer does not decide reference numbering, explanation fields, autosave, or account-search behavior.

## Consequences

- All journal, ledger, and report calculations must use `MinorUnits` or another reviewed exact representation.
- Zero and negative candidate line amounts are rejected. Reversals will swap debit and credit sides rather than storing negative lines.
- Migration `0002` and `JournalRepository::post` provide the base transaction boundary, independent database constraints, immutability triggers, and posting/deactivation concurrency coverage. The subsequently approved Version 01 reversal and correction policy is implemented by migration `0004`, the atomic repository operations, and ADR 0025.
- This ADR does not authorize implementation of the journal-entry editor.
