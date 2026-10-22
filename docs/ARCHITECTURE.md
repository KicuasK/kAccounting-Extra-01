# Architecture

Status: draft

## Deployment model

```text
Mac/iPad Safari
      |
 private Tailscale network (HTTPS)
      |
 Tailscale Serve
      |
 127.0.0.1:3000
      |
 Rust/Axum application
      |
 SQLite runtime database
```

The application is available only while the Mac Studio is powered on and the user is logged in. Public Funnel access is prohibited.

## Components

- `backend/`: Rust service, accounting domain logic, API, database access, and migrations.
- `frontend/`: TypeScript/Vite web interface optimized for Safari.
- `deploy/`: LaunchAgent and Tailscale Serve configuration documentation or templates.
- `scripts/`: repeatable development, verification, backup, and restore helpers.
- `docs/`: requirements, policies, decisions, and operating documentation.

## Backend boundaries

Planned modules:

- API: request parsing, authentication boundary if added, validation reporting, and response mapping.
- Accounting domain: account-tree rules, journal-entry validation, posting, reversals, and report calculations.
- Database: SQLx queries, migrations, transactions, and integrity checks.

HTTP handlers must not contain the authoritative accounting rules. Domain operations and database constraints should defend the invariants independently.

The current Axum router is read-only and is exercised in-process by tests. It
does not open a listener. JSON encodes SQLite identifiers, revisions, line
numbers, and monetary amounts as decimal strings so JavaScript cannot round
exact values. Authentication and the localhost-only serving boundary must be
decided before mutation routes or a listener are added.

The account implementation currently uses four integrity layers:

1. Rust input validation and structured repository errors.
2. SQLite checks, foreign keys, and triggers.
3. Defensive tree reconstruction before hierarchical data reaches the UI.
4. Startup checks for SQLite integrity, schema shape, foreign keys, and stored tree semantics.

Before enabling WAL or migrating a file, the database layer accepts only an empty SQLite database or one carrying this application's `kA01` application ID; unrelated SQLite files are refused without journal-mode or schema writes. Opening then runs embedded migrations and checks SQLite page/index consistency, foreign-key consistency, required schema objects and column signatures, and the semantic validity of the stored account tree. Repositories are not returned when any check fails.

## Data model status

- `accounts`: initial migration and Rust repository implemented; see `DATABASE_SCHEMA.md`.
- `journal_entries`: minimal date/state/timestamp header plus monotonic draft revision implemented for safe draft editing and atomic posting; presentation-specific header fields remain deferred.
- `journal_lines`: exact integer amount, debit/credit side, stable account reference, and immutable posted-line behavior implemented.
- `accounting_periods`
- `settings`

The account model, persistent user-editable drafts, and policy-neutral posting subset of the journal model have entered implementation. Reversals/corrections, periods, and settings remain candidates whose fields, state transitions, and policies require dedicated review.

## Data locations

- Source: this repository.
- Live runtime book: outside the repository on the Mac's local storage under
  macOS Application Support. It contains the runtime database and attachment
  store and is not opened directly from removable storage in Version 01.
- Runtime database and SQLite sidecars: owner read/write only (`0600`) after application identity is accepted.
- Proposed directory name: `kAccounting-Extra-01`; the final path must be confirmed before deployment.
- Backups and `.kagextra` transfer exports: outside the source tree and never
  committed.

The application owns the supported transfer workflow. The `.kagextra` artifact
is a macOS directory package that Finder may present as one document, not a
true single file. Its top level contains strict UTF-8 `manifest.json`, the
verified `book.sqlite3` snapshot, and `attachments/`. The old Mac creates a
consistent export, copies it to a
user-selected removable destination, and
verifies the published result before reporting success. The new Mac validates
the exact package layout, rejects undeclared objects and symlinks, and validates
integrity and format compatibility before importing to local storage. A failed
or interrupted operation must preserve the source and any pre-existing valid
destination. See `decisions/0007-local-book-and-transfer-boundary.md`.

The implemented package builder first verifies a staging tree, copies each
declared object into a new owner-only private directory, writes deterministic
`manifest.json`, and verifies the complete result. It does not publish that
private directory by itself. The publication step resynchronizes every package
file, synchronizes nested directories from deepest to shallowest, atomically
publishes without replacement, synchronizes the destination parent, and
reverifies the public package before returning a verified result. A file system
that rejects a required synchronization step is not treated as successful.

## Integrity strategy

- Use SQLite foreign keys and explicit transactions.
- Validate account eligibility and balance before posting.
- Re-check mutable prerequisites inside the posting transaction.
- Derive balances and reports from posted lines.
- Back up through a database-consistent mechanism rather than copying a live database unsafely.

