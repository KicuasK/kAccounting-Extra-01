# Development Plan

Status: draft; sequencing may change after the journal-entry interaction review.

## Phase 1 — Foundation

Status: complete. The verified initial source baseline is commit `067df40`.

- Verify the Mac development environment.
- Initialize Git and the repository structure.
- Record requirements, accounting invariants, UI direction, and open policy questions.
- Create minimal Rust and TypeScript buildable projects.

Exit gate: clean builds and tests run locally; no accounting behavior is implied by scaffolding.

## Phase 2 — Accounts and database design

Status: in progress. The generic hierarchy, database constraints, Rust repository, startup integrity audit, and integration tests are implemented. The handwritten hierarchy and unresolved policies still require user decisions.

- Review the handwritten account hierarchy.
- Define account node types, posting eligibility, parent rules, activation state, and deletion restrictions.
- Design and review the SQLite schema and migrations.
- Test cycle prevention and invalid moves.

Exit gate: schema review completed and account-domain tests pass.

## Phase 3 — Accounting core

Status: in progress. Exact integer minor units, persistent editable/deletable
drafts with optimistic concurrency, policy-neutral journal-line validation,
SQLite persistence, atomic posting, and posted immutability are implemented.
Reversal and explicit correction behavior remain to be designed and tested
without depending on the editor.

- Implement exact money handling.
- Implement draft persistence, validation, atomic posting, and immutability rules.
- Add reversal and correction behavior.
- Test balanced and unbalanced entries, invalid accounts, rollback, and concurrency-sensitive cases.

Exit gate: accounting invariants are proven by automated tests.

## Phase 4 — API and account management UI

Status: read-only backend route foundation complete. Health, account detail/tree,
draft and posted-entry reads, general journal, trial balance, five-element
summary, period ledger, and income/expense routes are tested in-process with no
network listener. Mutation routes, authentication, serving, and all UI work
remain pending.

- Add backend endpoints with structured validation errors.
- Build the Catalina-style application shell and account tree.
- Verify keyboard, touch, Safari layout, and font behavior.

Exit gate: nested accounts can be safely maintained through the UI.

## Phase 5 — Journal interaction

- Review and approve the General Journal editing workflow.
- Implement only after the interaction specification is accepted.

Exit gate: entry creation, balancing feedback, posting, and correction are usable on Mac and iPad.

## Phase 6 — Ledgers and reports

Status: backend general-journal, full and period general-ledger, trial-balance,
five-element summary, and period income/expense read models are implemented
early because they are policy-neutral proofs that reports derive only from
posted lines. General journal presentation and balance sheet remain pending; no
closing or current/non-current policy has been assumed.

- Journal and general ledger.
- Trial balance.
- Balance sheet.
- Income/expense report.
- Reconciliation tests across all reports.

## Phase 7 — Operations

Status: the backend now provides a verified, owner-only, no-overwrite SQLite
snapshot primitive. Scheduling, destination/retention/encryption policy,
restore, service startup, and private network configuration remain
pending.

- Consistent backup and restore.
- User LaunchAgent.
- Localhost binding and Tailscale Serve.
- End-to-end verification and operating documentation.

## Storage safety workstream — required before write API or UI

Status: in progress. Normal use is confirmed to keep the live book on local Mac
storage. `.kagextra` is an application-managed transfer artifact used when
moving to another computer; it is not the live-book location. Implemented
foundations now include SHA-256 plus exact-length descriptors, a versioned
logical manifest model, strict deterministic `manifest.json` encoding, bounded
copy failure handling, read-only
database-plus-attachment staging verification, strict physical package-layout
verification, a monotonic transfer lifecycle, verified no-overwrite publication
for local stored objects, and atomic no-replace publication of complete private
directories. An already verified staging tree can now be copied into a new
owner-only private package directory, given its deterministic manifest, and the
assembled result is independently reverified. Conservative publication now
synchronizes every declared file and every package directory, publishes with an
atomic no-replace rename, synchronizes the destination parent, and reverifies
the public package before reporting success. A single staged-export operation
now allocates same-file-system private work, rejects known destination
conflicts before copying, and runs assembly through verified publication while
preserving the source and actionable failure state. The backend can also create
verified owner-only staging from a SQLite point-in-time snapshot plus an
explicit attachment set whose source bytes must match caller-supplied SHA-256
and exact-length descriptors; unsafe paths and changed objects fail before any
public package exists. An end-to-end backend operation now allocates that local
staging, rejects known destination conflicts before snapshot work, runs the
complete conservative package export, and cleans local staging only after the
public package verifies; failed work is retained and post-success cleanup is a
warning rather than an ambiguous failure. The physical
`.kagextra` container is now confirmed as a macOS directory package.
Strict private `operation-state.json` persistence now supports owner-only
create-new records, bounded decoding, locked atomic updates, directory
synchronization, and monotonic stale-safe transitions. Integration of those
records at the live export publication boundaries, automatic
database-referenced attachment selection, removable-drive compatibility
validation, and restart recovery remain pending.

1. Complete the live-book and attachment-store contract, directory-package
   orchestration and persistent work-state
   without treating the package as a true single file internally.
2. Extend the implemented interrupted-read, capacity-style write, invalid
   package, symlink, corruption, and post-publish cleanup failure coverage to
   process termination and physical publication failures.
3. Build a consistent database-plus-attachments export and verified import that
   never overwrites the only valid book.
4. Test interrupted removable-drive copies, incomplete packages, incompatible
   versions, and destination conflicts.

Exit gate: every operation leaves either the previous complete state or a new
verified complete state; the source is never deleted automatically and no
ambiguous failure can cause duplicate accounting writes.

## Version 02

- CSV export.

## Later phase — TPASS tracking

Status: confirmed future feature; not yet scheduled for implementation.

- Resolve the policy and interaction questions in `docs/TPASS_FUTURE_FEATURE.md`.
- Design TPASS purchase, pass-period, ride-evidence, and amortization models for this project's Rust/SQLite architecture.
- Add invariant tests before or with implementation, including atomic purchase posting, non-financial ride recording, exact amortization, idempotency, and protection of posted entries.
- Build the TPASS and Journal integrations only after the general journal interaction design is approved.
- Validate the complete flow with isolated test books; do not modify real financial data.

Exit gate: TPASS purchases post exactly once, rides affect only evidence and statistics, amortization reconciles exactly to the pass cost, and all related accounting invariants pass automated tests.

