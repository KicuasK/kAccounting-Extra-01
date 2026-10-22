# Requirements

Status: confirmed Version 01 scope with remaining policy decisions recorded in
`OPEN_QUESTIONS.md`

## Objective

Build a personal double-entry accounting MVP that guarantees balanced posting, provides an auditable journal and ledger, produces core financial reports, and supports consistent backup and restore.

The system uses IFRS concepts as accounting guidance but does not claim complete enterprise IFRS compliance.

## Confirmed scope

- One personal reporting entity and one primary user.
- Accrual-basis accounting, with optional cash-flow observation derived from transactions.
- Single currency: TWD. Multi-currency accounting is not part of Version 01.
- Hierarchical chart of accounts with user-created nested accounts.
- Draft and posted journal-entry states.
- Journal, general ledger, trial balance, balance sheet, and income/expense reporting.
- Consistent backup and restore.
- High-volume attachments stored and verified together with the book.
- Application-managed encryption at rest for live attachment objects,
  application-managed backups, and `.kagextra` transfer exports.
- Do not create an application password, backup password, transfer password,
  passphrase, PIN, or recovery phrase. Use operating-system key storage and
  application-generated cryptographic keys instead.
- Application-managed computer-to-computer transfer through a versioned
  `.kagextra` macOS directory package placed on user-selected removable
  storage.
- Actionable error reporting.
- Mac Safari desktop interface.
- Private remote access while the Mac is powered on and the user is logged in.

## Delivery constraints

- Complete non-UI accounting, storage, API, and runtime behavior before the
  remaining UI work.
- Implement the Mac Safari UI as the final Version 01 product phase.
- Keep high-volume attachments in Version 01.
- Time-box remaining Version 01 development according to
  `DEVELOPMENT_PLAN.md`; scope changes require an explicit decision and revised
  estimate.

## Required behavior

### Accounts

- Represent the five accounting elements: assets, liabilities, equity, income, and expenses.
- Separate grouping/category nodes from posting accounts.
- Allow nested child accounts through `parent_id`.
- By default, allow posting only to active leaf posting accounts.
- Support expanding, collapsing, renaming, deactivating, balance viewing, and transaction-history viewing.
- Reject parent cycles and invalid cross-element moves.
- Do not directly delete accounts that have transaction history.

### Journal entries

- Require at least two effective lines for posting.
- Require debit and credit totals to be exactly equal.
- Post all lines atomically.
- Prevent silent mutation of posted entries.
- Correct posted entries through reversal or explicit correction.
- Present entries in a traditional General Journal structure, subject to a separate interaction-design review.

### Reports

- Calculate reports from posted journal lines only.
- Never expose an editable stored account balance as the source of truth.
- Represent opening balances with a balanced opening entry.

### Operations

- Keep source code and runtime data in separate locations.
- Keep the live book and its attachments on the Mac's local storage during
  normal use. Removable storage is a transfer destination, not the live-book
  location.
- Export, copy, verify, and import `.kagextra` data through the application;
  do not depend on a manual Finder copy as the supported workflow.
- Do not expose plaintext attachment, backup, or `.kagextra` payloads as
  completed artifacts or leave plaintext intermediate files after failure.
- Preserve the source book and every pre-existing valid destination when an
  export or import is interrupted, runs out of space, fails verification, or
  encounters an incompatible version.
- Never overwrite the only known-valid book and never automatically delete the
  old computer's data after transfer.
- Bind the backend to `127.0.0.1:3000` by default.
- Use Tailscale Serve for private HTTPS access.
- Never enable Tailscale Funnel.
- Run as a user LaunchAgent after login when deployment is configured.

## Out of scope for the first version

- CSV export (deferred to Version 02).
- Responsive, touch-friendly iPad Safari interface (deferred to Version 02).
- iPhone-specific layout.
- Docker, Linux/systemd, or cloud-server deployment.
- Public internet exposure.
- Claims of full IFRS compliance.
- Direct editing of posted balances or posted entries.
- Multi-currency accounting.

## Confirmed future feature: TPASS tracking

TPASS tracking is a frequently used workflow and is confirmed for a later phase after the core journal interaction is approved and implemented. It is not part of the first-version acceptance baseline.

- Record a pass purchase once as a balanced journal entry: debit prepaid expense and credit the selected payment account.
- Track an inclusive 30-day pass period from its actual activation date.
- Record individual TPASS transit rides as supporting evidence linked to the purchase, without creating a new journal entry or reducing prepaid expense for each ride.
- Store the fare that would otherwise have been paid as a reference amount for ride and break-even statistics only.
- Recognize the pass cost by calendar-month segments based on valid days, with the final segment absorbing any exact-integer rounding remainder.
- Keep ride edits and deletion from changing the purchase or amortization journal entries.
- Preserve permanent-ID relationships when account display names change.
- Provide a compact TPASS view and Journal integration that follow the Catalina-style UI direction on Mac Safari and iPad Safari.

The detailed future specification and unresolved policy choices are recorded in [TPASS_FUTURE_FEATURE.md](TPASS_FUTURE_FEATURE.md).

## Acceptance baseline

- Automated tests prove posting invariants and transaction atomicity.
- A user can create and maintain nested posting accounts safely.
- Journal, ledger, and reports reconcile from the same posted data.
- Backup, transfer, and restore reproduce a consistent database together with
  every referenced attachment and reject incomplete or damaged input.
- The UI is usable with keyboard input in Mac Safari.

## Version 02

- Responsive, touch-friendly iPad Safari interface.
- CSV export.

