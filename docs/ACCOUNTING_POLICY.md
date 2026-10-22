# Accounting Policy

Status: draft; unresolved policies are listed separately and must not be inferred.

## Reporting basis

- Reporting entity: one individual.
- Primary basis: accrual accounting.
- Default currency: TWD.
- Monetary values must use exact storage. The initial design should prefer integer minor units when all supported currencies have a known fixed minor unit.

## Elements and UI terminology

The accounting elements are assets, liabilities, equity, income, and expenses. The UI may label the expense area as 「支出」, but cash outflow must not automatically be classified as an expense.

## Posting rules

- Every posted entry has at least two valid lines.
- Total debits equal total credits exactly.
- Each line references an active posting account.
- Posting is one database transaction.
- Posted entries are immutable in ordinary workflows.
- Reversals and correcting entries retain the audit trail.
- Opening balances are entered through a balanced opening journal entry.

## Classification examples

- Credit-card repayment reduces cash and the card liability; it does not recognize the expense again.
- Moving cash into a time deposit is an asset-to-asset transfer and is not automatically an expense.
- A contract is not automatically a liability. Recognition depends on the actual present obligation and performance status.
- Time deposits are current or non-current based on maturity and withdrawal conditions, not their name alone.

## Initial account-category direction

This is a default high-level structure, not a locked detailed chart of accounts:

- Assets
  - Current assets
    - Payment accounts
      - Banks
      - Cash
    - Prepayments
    - Receivables
  - Non-current assets
    - Time deposits, subject to classification review
- Liabilities
  - Immediate
    - Credit cards
    - Purchase contracts, subject to recognition policy
  - Non-immediate
    - Contracts
    - Payables
- Expenses / 「支出」
  - Food, clothing, housing, transportation, education, and entertainment
- Income
  - 「正」 and 「偏」, pending definitions
- Equity
  - Retained earnings
  - 「工作」 and 「資助」, pending definitions

The handwritten source image must be checked before this hierarchy is treated as final.

