# Credit-card core/API review — pending policy, 2026-10-22

2026-10-23 amendment: user removed account codes and selected explicit debit/credit
balance direction for now. Card subtype, limits/capacity, prepayment, outstanding
debt and the other policies below remain unapproved. No card formula was added.

## Scope and authority

The UI conversation relayed the user's concern that credit-card logic may need
changing. This authorizes assessment, not a specific accounting policy or copying
all legacy rules. No core, API, UI, real book or migration was changed by this review.

## Source findings

- Current `backend/src/accounting/accounts.rs`: Account has element, category/posting
  kind, active status, revision and names; no explicit credit-card subtype, credit
  limit, effective-dated limit history or card-specific metadata.
- Current `backend/src/api.rs` AccountResponse mirrors generic account metadata.
  Account listing has no balance or card metrics. Report responses expose exact
  integer amounts as strings and separate debit/credit balances. Existing posted-line
  reports remain the accounting source of truth.
- Current core/API source search found no credit-card, available-credit, outstanding,
  prepaid or credit-limit implementation. Generic liability journal accounting is
  supported; a credit-card management contract is not yet supported.
- Legacy reference (read-only): `/Users/kicuashsu/Desktop/KAccountingSuper/`.
  `Sources/KAccounting/Web/features/accounts.js` creditCardAccountRow shows available
  credit in the list; creditCardDetails shows limit/prepayment/outstanding separately.
  `Sources/AccountingCore/Accounts.swift` derives balance as debit minus credit,
  available_credit = limit + balance, outstanding = max(0, -balance),
  prepaid = max(0, balance). Legacy UI also describes blocking debt above limit;
  that restriction is evidence of legacy behavior, not an approved new policy.

## Contract boundary and engineering recommendations

Available credit is capacity metadata plus posted activity, never a trial-balance
or balance-sheet account balance. Keep report derivation and journal posting
invariants unchanged. Card identity must be explicit metadata rather than inferred
from its name or four-digit display code. The internal AccountId is not itself an
approved four-digit account-code schema.

If approved, add card settings and derived metrics through a versioned migration
and explicit API fields. Limits and amounts must use exact minor-unit integers,
checked arithmetic and existing exact-string serialization. Record settings changes
in the audit trail; do not mutate posted lines to change a limit. Derive metrics
from posted lines at a clearly stated as-of date. Do not maintain independently
editable prepaid/outstanding balances. Preserve generic report debit/credit semantics
until the account-list sign policy is decided.

## User decisions still required

1. Account-list signed balance convention for all five elements, including cards;
   whether available credit has a separate column/detail rather than replacing balance.
2. Whether positive debit card balances stay shown on that liability account or
   require an explicit report reclassification; do not silently auto-reclassify.
3. Limit behavior: informational warning or posting prohibition; permit actual
   over-limit debt/fees? Allow zero limit? Prepayment may exceed the stated limit?
4. Changes of limit: current-only or effective-dated history; how to handle a new
   limit below existing debt and whether historical postings use historical limits.
5. Refunds, overpayments, reversals and backdated corrections: capacity behavior and
   whether a protective limit rule can ever prevent a necessary correcting entry.
6. Whether pending authorizations/installments/shared card limits are in Version 01;
   posted journals alone cannot claim to reproduce bank real-time available credit.
7. Existing generic liability conversion, subtype changes after use, deactivation
   and whether old books receive card metadata only after explicit user selection.

No decision here may be treated as an approved accounting policy. UI color,
layout and four-digit-code requests relayed from the design conversation are context;
this review does not implement them or settle the signed-amount convention.

## Verification and Git

Read-only source/schema/API comparison and documentation review only. No executable
behavior changed, so no new runtime tests were run. Last verified source checkpoint
remains 651 backend / 112 tool / 41 frontend tests, 9 intentionally ignored; those
counts do not constitute acceptance of credit-card functionality. This assessment
is a new local follow-up after GitHub commit 59d57f1; no automatic push is implied.
