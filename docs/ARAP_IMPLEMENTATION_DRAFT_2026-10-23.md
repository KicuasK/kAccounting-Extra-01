# AR/AP implementation and approved policies — 2026-10-23

Status: implemented, with user-approved historical adoption and explicit
unallocation policies. The filename retains `DRAFT` for continuity with the
initial design note; this document now records the resulting implementation.
The user approved explicit selection of posted lines, manual historical adoption,
no compulsory classification of every account transaction, and explicit
unallocation before reversing a source with allocations. Unallocation retains
the original financial payment. Existing posted entries remain unchanged.

The initial proposal text was written before that decision. Sections below now
describe the implemented slice; the current HTTP contract is recorded separately
in [ARAP_API_2026-10-23.md](ARAP_API_2026-10-23.md).

## Scope and existing foundations

Extra-01 already exposes permanent posted journal-line IDs. Its SQLite triggers
prevent changing or deleting posted headers and lines. Reversals create a new
entry linked by `reverses_entry_id`; a correcting entry is additional accounting
history, not a replacement of its target.

Super supplies useful validation patterns: one source line can establish several
items; receipts/payments and adjustments are separate allocation kinds; all
amounts are positive integers; allocations must use the same account and cannot
precede their source entry or exceed an outstanding amount. Its naming rules,
restrictions against adopting accounts with history, and deletion-based journal
recalculation must not be imported automatically.

## Implemented smallest vertical slice

1. A user explicitly selects a posted line and splits its full amount into
   receivable or payable items. AR uses an asset debit; AP uses a liability
   credit. The ordinary journal remains authoritative for account balances.
2. A user explicitly selects an opposite-side posted line in the same account
   and allocates some or all of its capacity to existing items. Classification
   distinguishes payment from adjustment. The UI displays unused line capacity.
3. A read model derives original, paid, adjusted, outstanding, source entry,
   allocation history, optional counterparty text and optional due date.
4. Removing an erroneous tracking relationship appends a reasoned
   cancellation event. It does not delete history or reverse financial money.

Selecting existing lines only annotates accounting that already exists. It must
not create a balancing line, change a posted amount, or imply that an unlinked
line has been accounted for in the subledger.

## Implemented persisted model

- `open_item_batches`: immutable source-line registration with tracking kind,
  timestamp and a unique source-line constraint.
- `open_items`: immutable batch members with positive original integer amount,
  description, optional counterparty and due date.
- `open_item_allocations`: immutable source settlement-line, target item,
  positive amount and explicit payment/adjustment classification.
- `open_item_allocation_voids`: immutable, reason-required cancellation records;
  each allocation can be cancelled once.
- `open_item_events`: append-only audit events for registration, allocation and
  cancellation. Schema integrity checks validate their record linkage and payload.

Each repository mutation obtains `BEGIN IMMEDIATE`, reads its source and current
effective allocations inside that transaction, validates all requested members,
then inserts business records and audit together. Checked integer arithmetic
must report overflow; a SQLite `SUM` overflow must never be interpreted as zero.
Source-batch totals must equal their posted source line. Effective allocations
must not exceed either item outstanding or the settlement-line amount, including
other requests in the same batch and concurrent submissions.

Records use foreign keys to permanent journal lines. Sidecar update/delete
triggers preserve history. The backup/schema integrity checker must explicitly
recognize and validate the new tables, indexes and triggers; adding a migration
alone is insufficient because `db.rs` rejects unexpected schema objects.

## Implemented HTTP contract

All integer IDs and amounts use canonical decimal strings. Mutation DTOs reject
unknown fields and use the existing exact-origin and selection-generation
boundary. Implemented route names:

- `GET /api/open-items` returns both kinds and their allocation history.
- `GET /api/open-items/lines/{line_id}/capacity`
- `POST /api/open-items` with `source_line_id`, `kind`, and `items`.
- `POST /api/open-items/allocations` with `settlement_line_id` and allocations
  containing `item_id`, `amount_minor`, and `kind`.
- `POST /api/open-items/allocations/{allocation_id}/void` with `reason`.

The backend must return distinct missing-line, wrong-account/direction,
unposted/reversed-source, allocation-capacity, date, duplicate registration,
invalid-input and stale-selection errors. Allocation failure writes no partial
batch and no misleading success event.

## User-approved policies and implementation boundaries

### Historical coverage

Tracking adopts explicitly selected historical lines and leaves other lines
unclassified. It does not require every historical or future account line to be
classified. The response labels coverage `explicit-lines-only`; a partial
subledger must not be presented as the complete account balance.

### Reversals and allocation cancellation

When an item has effective receipts/payments or adjustments, reversing its source
entry is blocked until its allocations are explicitly cancelled with a reason.
The same protection applies to reversing a settlement entry. There is no
automatic cascade. Cancelling allocation only changes tracking: the payment
entry remains posted and its line capacity becomes available for reallocation.

This is enforced in `reverse_posted` and in a SQLite posting trigger, covering
the existing general-journal path. A correcting entry has no
automatic cancellation semantics: it is additional history and needs explicit
tracking decisions.

### Classification and automation

No automatic FIFO allocation, arbitrary account naming requirement, automatic
account creation, credit-card classification or write-off account is established
by this proposal. Payment and adjustment classifications annotate a valid posted
line; they cannot make an otherwise unbalanced entry acceptable.

## Verification performed

`cargo test --locked --test open_items`: **7 passed**. These include AR/AP splits,
partial payments, adjustments, reason-required cancellation, unchanged posted
history, large exact integers, atomic rejection, concurrent allocation attempts,
origin/JSON boundaries, source/payment reversal guards and schema corruption.
This is targeted backend evidence, not a claim of full verification or live
Safari acceptance.

## Invariant coverage checklist

- Full source split, partial allocation, multi-item allocation and final closure.
- AR/AP directions, same-account constraint, date ordering and exact arithmetic
  above JavaScript's safe-integer range.
- Over-item and over-line allocations, duplicates in a request, repeated requests
  and concurrent writer attempts; failure must leave no partial records.
- Draft, reversed source, missing source and metadata corruption rejection.
- Approved reversal/cancellation behavior through both repository and ordinary
  journal API routes, while posted rows remain byte-for-byte unchanged.
- Audit persistence, immutable sidecars, wrong selection and wrong origin.
- Backup/restore validation of populated sidecars remains a separate acceptance
  item; exact schema validation is integrated into normal book integrity checks.

## Evidence reviewed

Read-only review of Extra-01 `accounting/journal.rs`, `api.rs`, journal migrations,
`db.rs`, and Super `Receivables.swift` plus `Journal.swift` liability allocation
logic. No Super program was executed and no real financial data was opened.
