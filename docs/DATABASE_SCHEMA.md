# Database Schema

Status: account schema and minimal atomic-posting journal schema implemented and covered by integration tests.

## Design goals

- Keep the chart of accounts hierarchical and user-extensible.
- Separate grouping categories from accounts that may receive journal lines.
- Enforce structural invariants in both Rust and SQLite.
- Avoid encoding unresolved accounting policy as schema.
- Keep all test data in disposable in-memory or temporary file databases.

## `accounts`

| Column | SQLite type | Meaning |
|---|---|---|
| `id` | `INTEGER` | Positive, non-reused surrogate identifier; immutable after insertion. |
| `parent_id` | `INTEGER NULL` | Self-referencing parent; `NULL` means a root node. |
| `name` | `TEXT` | Trimmed display name, 1–120 Unicode characters, without control characters. |
| `element` | `TEXT` | `asset`, `liability`, `equity`, `income`, or `expense`. |
| `kind` | `TEXT` | `category` or `posting`. |
| `is_active` | `INTEGER` | Boolean activation state. |
| `revision` | `INTEGER` | Monotonic optimistic-concurrency version, starting at 1. |
| `created_at` | `TEXT` | Immutable UTC creation timestamp. |
| `updated_at` | `TEXT` | UTC last-change timestamp. |

The table uses SQLite `STRICT` mode. Boolean values are constrained to `0` or `1`, and enum-like values have explicit `CHECK` constraints. Name checks reject C0/C1 control characters and leading or trailing Unicode whitespace, matching Rust normalization even when SQL bypasses the repository.

## Enforced invariants

| Invariant | Rust | SQLite |
|---|---:|---:|
| Non-empty name of at most 120 characters | Yes | Yes |
| Stable identifier cannot be rewritten | Repository never updates it | Immutable-ID trigger |
| Creation timestamp cannot be rewritten | Repository never updates it | Immutable-timestamp trigger |
| Parent exists | Error mapping | Trigger and foreign key |
| Parent is a category | Error mapping | Trigger |
| Parent and child use the same element | Error mapping | Trigger |
| No self-parent or descendant cycle | Error mapping | Check and recursive trigger |
| Posting account has no children | Eligibility check | Insert/update triggers |
| Only active posting leaves are postable | Repository guard | Structure and state columns |
| Parent cannot be deleted before children | Repository error | Restricted foreign key |
| Stale client cannot overwrite a newer edit | Expected-revision update | Revision starts at 1; every update must advance it by exactly one |

`journal_lines.account_id` uses a restricted foreign key, so an account referenced by any saved draft or posted line cannot be deleted. Deleting a draft removes its lines, after which an otherwise unused account may be deleted; posted lines remain immutable and preserve their accounts permanently. An account's accounting element and category/posting kind are frozen once any journal line refers to it. Renaming, deactivation, and reactivation remain available because lines keep permanent account identifiers and posting eligibility is rechecked for each new entry.

## `journal_entries` and `journal_lines`

Migration `0002` adds the smallest policy-neutral schema needed to prove atomic posting. Migration `0003` adds an integer draft revision used for optimistic concurrency. They deliberately omit reference numbers, explanations, attachments, UI draft recovery fields, and other header choices that still require review.

The forward migration gives every existing version-2 journal entry revision `1`.
Automated upgrade coverage verifies that incomplete drafts keep their lines and
that previously posted entries remain posted and immutable.

`journal_entries` stores a positive stable identifier, a validated `YYYY-MM-DD` calendar date, `draft` or `posted` state, a positive monotonic revision, and UTC creation/posting timestamps. `journal_lines` stores a stable identifier, one-based line number, permanent account identifier, exactly one `debit` or `credit` side, and a strictly positive `INTEGER` minor-unit amount. Both tables use SQLite `STRICT` mode.

The repository supports explicit persistent drafts independently of any UI. A
draft may be empty, incomplete, or unbalanced. Its date and complete line set
may be replaced repeatedly, and it may be deleted. Each replacement is one
SQLite transaction, so a failed account check or line insert leaves the prior
draft unchanged. Posting revalidates the saved lines and account eligibility in
one write transaction; only then does the entry become immutable. The separate
convenience posting path still creates its temporary draft and posts it in one
transaction. Autosave timing and recovery behavior remain deferred until the
journal interaction is approved.

Each draft read returns its current revision. Replace, delete, and post require
that revision. A stale caller receives the current revision instead of silently
overwriting a newer save. SQLite triggers require direct draft header updates to
advance exactly one revision, and concurrent writers are serialized by the
transactional repository.

Database constraints and triggers independently enforce:

- entries can only be inserted as drafts;
- drafts may be incomplete and may be replaced or deleted;
- a draft header revision starts at one and every direct header update advances
  it by exactly one; the repository additionally compares the caller's expected
  revision for replace, delete, and post operations;
- a posted entry has at least two lines and exactly equal integer debit and credit totals;
- lines use active posting leaf accounts at insertion and posting time;
- zero or negative amounts and invalid calendar dates are rejected;
- lines cannot be added to a posted entry;
- posted headers and lines cannot be updated or deleted;
- account deletion cannot remove history; and
- account element/kind cannot be reclassified after any journal line exists.

Integer overflow aborts validation instead of wrapping. Rust performs checked arithmetic before opening a transaction, while SQLite integer aggregation provides a second fail-closed boundary for direct SQL attempts.

## Why the element is repeated on every node

Storing `element` on every account makes element filtering and report grouping explicit. Triggers require a child to match its parent, so the repeated value cannot silently diverge inside a tree. Moving a subtree across elements is rejected rather than implicitly reclassifying historical meaning.

## Deliberately deferred fields and policies

The following are not part of migration `0001` because their behavior is unresolved:

- Locked or system-managed category flags.
- User-visible account codes.
- Multi-currency and per-account currency.
- Normal-balance overrides and contra-account behavior.
- Same-parent name uniqueness and case-folding policy.
- Explicit sibling display order.
- Rules for deactivating a parent that still has active descendants.
- Any future policy that would permit safely converting an account with journal history into a parent category. The current conservative rule freezes `element` and `kind` after the first line reference.

Adding any of these requires a documented decision and a new forward-only migration. Existing migrations must not be silently rewritten after real data exists.

## Migration policy

- Migrations live in `backend/migrations/` and are embedded by SQLx; reopening rejects an applied migration whose recorded checksum no longer matches the embedded source.
- The database layer uses SQLite `application_id` `1799434289` (`kA01`), also declared by migration 1. A single-connection preflight runs before WAL is enabled or migrations execute, and accepts only an empty database or a database carrying that ID. After that check, an empty database is claimed with `kA01` before SQLx creates migration metadata, so an interrupted migration remains safely retryable. A mistaken path cannot add application schema or change the journal mode of an unrelated SQLite file; non-SQLite files are also rejected without modifying their contents.
- Application connections must enable SQLite foreign keys.
- Tests use a single-connection in-memory pool so the database is isolated and deterministic.
- File databases use WAL journaling, full synchronization, a five-second busy timeout, and a four-connection pool. The small personal workload favors confirmed-write durability over the lower latency of normal synchronization.
- On macOS/Unix, a missing database path is atomically reserved as an
  owner-only (`0600`) file before SQLite first opens it, eliminating a
  world-readable creation window. Existing files are not chmodded until the
  identity preflight accepts them. A recognized database and any SQLite
  WAL/SHM/journal sidecars are then restricted to `0600` before and after the
  production pool opens.
- Database opening runs full `PRAGMA integrity_check`,
  `PRAGMA foreign_key_check`, an exact embedded-migration
  version/success/checksum and table-signature audit, a required-schema
  whitelist with index/trigger owner checks, critical constraint/trigger SQL
  checks, and semantic account-tree and posted-journal audits after migrations.
  The complete audit uses one SQLite read transaction, so every check observes
  the same snapshot. A damaged file, migration-history mismatch, weakened or
  unexpected schema object, same-name decoy object, preexisting cycle, posting
  parent, cross-element relationship, short or unbalanced posted entry, or
  posted line referencing a category account is rejected before repositories
  are returned. The personal database is expected to remain small enough that
  full startup verification is preferable to the reduced coverage of
  `quick_check`.
- Production data paths remain outside the repository and are not created by tests.

Integrity failures are fail-closed. The application must not silently rewrite or auto-repair a database that fails these checks. Preserve the original file, diagnose against a copy, and use a verified backup or an explicitly reviewed repair procedure. When a future migration intentionally changes the account schema, its release must update the required-schema audit in the same reviewable change.

The application ID is a wrong-file guard, not authentication, encryption, or proof that a file is trustworthy. Private access and backup protection remain separate operational requirements.

## Snapshot backup primitive

`backup_database` uses SQLite `VACUUM INTO` against a file-backed application pool to produce a transactionally consistent standalone snapshot. The snapshot is created inside a same-parent private temporary directory, changed to `0600`, opened read-only, and subjected to the complete integrity audit—including embedded migration checksums—before publication. A hard link publishes it without replacing an existing file; the temporary link and directory are then removed. A failed audit leaves no published destination.

Any existing destination filesystem object, including a symbolic link, is
rejected before snapshot creation. Publication does not follow or replace a
symlink, and the final hard-link operation independently fails if a destination
appears concurrently.

`verify_database_snapshot` exposes the same identification and integrity audit for an existing standalone backup or future restore candidate. It opens the file read-only, does not enable WAL, does not migrate, does not change permissions, and closes its private connection pool before returning.

The caller remains responsible for choosing a destination outside the repository. Backup scheduling, naming, retention, encryption, and restore behavior are deliberately unresolved and are not implied by this primitive.

See [ADR 0002](decisions/0002-sqlite-file-safety.md) for the rejected alternatives and operational consequences of this fail-closed policy.
See [ADR 0004](decisions/0004-consistent-snapshot-backup.md) for backup publication and verification behavior.

## Rust access layer

`AccountRepository` owns account persistence operations and converts stable trigger messages into domain errors. Its `require_postable` guard checks that an account is active, is a posting node, and has no children before later journal code may use it.

Every rename, move, element change, kind change, and activation change includes the revision last seen by the caller. The SQL update succeeds only when that revision still matches, increments it atomically, and returns that exact written row through SQLite `RETURNING`. A database trigger also rejects direct SQL updates that fail to advance the revision by exactly one. This prevents an older Mac or iPad screen from silently overwriting a newer edit or receiving a later client's state as its own response.

Account deletion acquires SQLite's writer slot before attempting the delete and
diagnosing a restricted foreign key. The decision therefore comes from one
stable transaction: child accounts, removable draft references, and immutable
posted history are reported as distinct causes, while a concurrent rename or
delete cannot change the diagnosis halfway through.

`build_account_tree` converts flat rows into ordered roots and descendants for the UI. It defensively rechecks duplicate IDs, missing parents, parent kind, accounting-element consistency, and cycles even though normal database reads should already satisfy those constraints.

`MinorUnits` is an exact signed `i64` wrapper that exposes checked arithmetic. `validate_journal_lines` applies deterministic line-count, positive-amount, overflow, and balance rules without making presentation decisions. `JournalRepository::post` repeats mutable account eligibility checks inside the same write transaction that creates and posts the entry. `JournalRepository::get_posted` loads a posted header and its lines in one read snapshot, then defensively reruns line validation before returning it.

`ReportRepository` is read-only. `trial_balance` derives account turnover and net debit/credit balances directly from posted lines through an inclusive as-of date, excludes drafts, uses checked integer accumulation across entries, and verifies that the final debit and credit columns reconcile. `accounting_equation_summary` groups that reconciled result into all five accounting elements without calling it a balance sheet or assuming closing/current-classification policy. `general_ledger` reads account metadata and lines in one SQLite snapshot, orders posted movements by accounting date and stable identifiers, then calculates checked running balances. `period_general_ledger` additionally carries a derived opening balance into an inclusive range and separates in-period turnover from the closing balance. `general_journal` returns balanced posted entries in accounting-date and stable-ID order for an inclusive range, with drafts excluded and checked per-entry and report totals. `income_expense_report` calculates posted period activity for income and expense elements, including opposite-side adjustments, but deliberately creates no closing entry and performs no equity transfer. No editable balance column exists.

## Verification coverage

Automated tests exercise repository behavior, direct-SQL attempts to bypass constraints, exact-money and aggregate-overflow boundaries, incomplete and repeatedly replaced drafts, atomic draft rollback, draft deletion, posting an existing draft, account reactivation, balanced and compound entries, rollback after validation or account failure, posted immutability, concurrent posting/deactivation, stable tree and path reconstruction, deep nesting, stale revisions, concurrent writers, delete/update races, file reopen and migration idempotence, every configured pool connection, and non-reuse of deleted IDs. Corruption tests deliberately inject a changed application identity, orphan rows, bypassed checks, cycles, posting parents, cross-element relationships, an unbalanced posted entry, missing schema objects, wrong table or index signatures, wrong foreign keys, and migration checksum mismatches into disposable databases, then confirm opening or integrity verification fails closed.

Wrong-path tests also prove that unrelated SQLite databases, databases carrying another application ID, and ordinary non-SQLite files are rejected without application schema writes or content changes. No test opens or modifies a production financial-data path.

