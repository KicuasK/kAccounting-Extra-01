# Database Schema

Status: approved account/journal contracts and encrypted attachment metadata
implemented and covered by integration tests.

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
| `revision` | `INTEGER` | Monotonic optimistic-concurrency version, starting at 1; revision-advancing mutations fail closed at `i64::MAX`. |
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
| Stale client cannot overwrite a newer edit | Expected-revision update; exhaustion is a distinct error | Revision starts at 1; every update must advance it by exactly one |

`journal_lines.account_id` uses a restricted foreign key, so an account referenced by any saved draft or posted line cannot be deleted. Deleting a draft removes its lines, after which an otherwise unused account may be deleted; posted lines remain immutable and preserve their accounts permanently. An account's accounting element and category/posting kind are frozen once any journal line refers to it. Renaming, deactivation, and reactivation remain available because lines keep permanent account identifiers and posting eligibility is rechecked for each new entry.

Repository reads also validate positive identifiers, normalized names, strict
boolean activation values, positive revisions, and known enum values instead of
trusting decoded SQLite rows. Tree and root-to-node path construction
iteratively rechecks missing or duplicate nodes, category parents,
same-element relationships, and cycles, so corrupted deep hierarchies fail
closed without recursive call-stack dependence.

## `journal_entries` and `journal_lines`

Migration `0002` adds the smallest policy-neutral schema needed to prove atomic
posting. Migration `0003` adds an integer draft revision used for optimistic
concurrency. Migration `0004` adds immutable reversal/correction
self-references and an optional whole-entry explanation. Migration `0005` adds
optional reference and line-memo fields, and repository reads/writes preserve
all approved journal metadata. Migration `0006` completes the sibling naming
and category activation policy. Migration `0007` adds immutable entry-owned
encrypted attachment metadata.

The forward migration gives every existing version-2 journal entry revision `1`.
Automated upgrade coverage verifies that incomplete drafts keep their lines,
that previously posted entries remain posted and immutable, and that their new
relationship fields start empty.

`journal_entries` stores a positive stable identifier, a validated `YYYY-MM-DD` calendar date, `draft` or `posted` state, a positive monotonic revision, and UTC creation/posting timestamps. `journal_lines` stores a stable identifier, one-based line number, permanent account identifier, exactly one `debit` or `credit` side, and a strictly positive `INTEGER` minor-unit amount. Both tables use SQLite `STRICT` mode. SQLite requires positive line numbers and uniqueness within an entry; repository writes assign the contiguous sequence `1..n`, and every draft or posted-entry read defensively rejects gaps or a sequence that does not start at one.

`JournalRepository::reverse_posted` and `post_correction` create and post their
new entries atomically. A unique partial index allows only one direct reversal
per original. Posting triggers require a posted relationship target, prohibit
an entry from being both reversal and correction, reject reversal-of-reversal
and an earlier reversal date, and compare every reversal line with the original
line at the same line number. Correcting entries require a non-blank
explanation. Relationship fields are immutable, and startup verification
rechecks both relationship headers and exact reversal lines.

The repository supports explicit persistent drafts independently of any UI. A
draft may be empty, incomplete, or unbalanced. Its date and complete line set
may be replaced repeatedly, and it may be deleted. Each replacement is one
SQLite transaction, so a failed account check or line insert leaves the prior
draft unchanged. Posting revalidates the saved lines and account eligibility in
one write transaction; only then does the entry become immutable. The separate
convenience posting path still creates its temporary draft and posts it in one
transaction. Version 01 uses explicit Save, with no timed autosave or implicit
crash recovery, as approved in `VERSION01_DECISIONS.md`.

Each draft read returns its current revision. Replace, delete, and post require
that revision. A stale caller receives the current revision instead of silently
overwriting a newer save. SQLite triggers require direct draft header updates to
advance exactly one revision, and concurrent writers are serialized by the
transactional repository. Replace and post reject a draft at `i64::MAX` rather
than overflowing its revision. Before replace or delete, the repository holds
the writer slot and validates the stored header and complete line set; it will
not erase or overwrite a malformed draft that should remain available for
diagnosis. Replacement checks that stored snapshot and expected revision before
validating the caller's new line set, so a stale client receives the current
revision rather than an error from a payload that can no longer be applied.

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

Integer overflow aborts validation instead of wrapping. Rust uses checked
arithmetic before a posting transition, within the same transaction for an
existing draft, while SQLite integer aggregation provides a second fail-closed
boundary for direct SQL attempts.

There is no directly editable balance, special opening-balance column, or
dedicated opening-balance workflow. The generic posting repository and its
tests represent an opening balance as one balanced posted entry. Any future UI
must preserve that journal invariant; its account selection, date, and
presentation policy remain undecided.

## Why the element is repeated on every node

Storing `element` on every account makes element filtering and report grouping explicit. Triggers require a child to match its parent, so the repeated value cannot silently diverge inside a tree. Moving a subtree across elements is rejected rather than implicitly reclassifying historical meaning.

## Deliberately deferred fields and policies

The following are not part of migration `0001` because their behavior is unresolved:

- Locked or system-managed category flags.
- User-visible account codes.
- Multi-currency and per-account currency.
- Normal-balance overrides and contra-account behavior.
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

This low-level snapshot primitive alone does not implement application backup
policy. The encrypted local backup/restore layer is documented in ADR 0029.
Explicit backup, newest-10 retention, encryption, and new-destination restore
are approved; automatic application orchestration remains to be completed.

See [ADR 0002](decisions/0002-sqlite-file-safety.md) for the rejected alternatives and operational consequences of this fail-closed policy.
See [ADR 0004](decisions/0004-consistent-snapshot-backup.md) for backup publication and verification behavior.

## Rust access layer

`AccountRepository` owns account persistence operations and converts stable trigger messages into domain errors. Its `require_postable` guard checks that an account is active, is a posting node, and has no children before later journal code may use it.

Every rename, move, element change, kind change, and activation change includes the revision last seen by the caller. Each update and delete first acquires SQLite's writer slot, decodes and validates the complete stored account row, and compares that revision before attempting the requested change. A stale caller therefore receives a revision conflict before a classification or reference diagnosis, while unrelated stored corruption cannot be silently repaired or erased by a field-specific mutation. The SQL update succeeds only when the revision still matches and is below `i64::MAX`, increments it atomically, and returns that exact written row through SQLite `RETURNING`. A database trigger also rejects direct SQL updates that fail to advance the revision by exactly one. An exhausted revision produces a distinct failure and leaves the row unchanged.

Within that same stable transaction, account deletion diagnoses child accounts,
removable draft references, and immutable posted history as distinct causes;
a concurrent rename or delete cannot change the diagnosis halfway through.

`build_account_tree` converts flat rows into ordered roots and descendants for the UI. It defensively rechecks duplicate IDs, missing parents, parent kind, accounting-element consistency, and cycles even though normal database reads should already satisfy those constraints.

`MinorUnits` is an exact signed `i64` wrapper that exposes checked arithmetic. `validate_journal_lines` applies deterministic line-count, positive-amount, overflow, and balance rules without making presentation decisions. `JournalRepository::post` repeats mutable account eligibility checks inside the same write transaction that creates and posts the entry. Draft and posted-entry reads validate stored status/timestamp consistency, dates, revisions, positive identifiers and amounts, sides, contiguous line numbering, and that every referenced account still exists as a posting leaf. A posted entry must have advanced beyond the initial draft revision. `JournalRepository::get_posted` loads a posted header and its lines in one read snapshot, defensively reruns full line validation, and validates any permanent reversal/correction relationship before returning it.

`ReportRepository` is read-only. `trial_balance` derives account turnover and net debit/credit balances directly from posted lines through an inclusive as-of date, excludes drafts, uses checked integer accumulation across entries, and verifies that the final debit and credit columns reconcile. Trial-balance and income/expense rows are emitted from account-ID-keyed maps, which provide stable output order without an additional SQLite temporary sort. `accounting_equation_summary` groups that reconciled result into all five accounting elements without calling it a balance sheet or assuming closing/current-classification policy. `general_ledger` reads account metadata and lines in one SQLite snapshot, orders posted movements by accounting date and stable identifiers, then calculates checked running balances. `period_general_ledger` additionally carries a derived opening balance into an inclusive range and separates in-period turnover from the closing balance. `general_journal` returns balanced posted entries in accounting-date and stable-ID order for an inclusive range, with drafts excluded and checked per-entry and report totals. `income_expense_report` calculates posted period activity for income and expense elements, including opposite-side adjustments, but deliberately creates no closing entry and performs no equity transfer. Its signed account results accumulate in exact `i128` totals before a checked narrowing to the public `i64` minor-unit range, so an in-range result does not depend on account order. Every report reparses stored calendar dates and defensively rejects invalid sides or non-positive amounts if database constraints were bypassed; reports that expose or group accounting elements also reject unknown stored elements. No editable balance column exists.

## Verification coverage

Automated tests exercise repository behavior, direct-SQL attempts to bypass constraints, exact-money and aggregate-overflow boundaries, incomplete and repeatedly replaced drafts, atomic draft rollback, draft deletion, posting an existing draft, account reactivation, balanced and compound entries, rollback after validation or account failure, posted immutability, concurrent posting/deactivation, stable tree and path reconstruction, deep nesting, stale and exhausted revisions, concurrent writers, delete/update races, file reopen and migration idempotence, every configured pool connection, non-reuse of deleted IDs, and reconciliation among the period journal, account ledgers, trial balance, five-element summary, and income/expense report. Corruption tests deliberately inject a changed application identity, orphan rows, bypassed checks, cycles, posting parents, cross-element relationships, invalid stored identifiers, names, booleans, dates, statuses, revisions, sides, or line-number sequences, a too-short or unbalanced posted entry, non-positive posted line amounts, invalid elements on referenced accounts, missing schema objects, wrong table or index signatures, wrong foreign keys, and migration checksum mismatches into disposable databases, then confirm opening, repository reads or mutations, integrity verification, or report generation fails closed without rewriting the evidence.

Wrong-path tests also prove that unrelated SQLite databases, databases carrying another application ID, and ordinary non-SQLite files are rejected without application schema writes or content changes. No test opens or modifies a production financial-data path.


## Version 01 completed contracts

- `journal_entries.reference` and `journal_lines.memo` are nullable text with no
  null characters. Draft header replacement is revisioned; posted metadata is
  immutable. Journal and ledger read models carry it unchanged.
- Partial unique indexes enforce exact normalized sibling names, including one
  root namespace. SQL triggers and tree reads require active ancestors, with
  descendant-first deactivation and ancestor-first reactivation.
- `attachments` uses random lowercase 32-hex object IDs; immutable journal-entry
  references; plain display filenames; plaintext and ciphertext SHA-256/length;
  and creation timestamps. Exact file framing, 250 MiB/file, and 20 GiB/book
  constraints are enforced independently in SQLite. Draft deletion cascades
  its metadata; posted attachment deletion and all attachment updates fail.
- Full startup audits include attachment table signatures, foreign keys,
  constraints, trigger ownership, quota totals, and account activation semantics.
  Physical object verification belongs to the encrypted repository and backup
  protocol. Database integrity alone cannot prove that an object file exists.
- Ciphertext no longer referenced by a live draft is conservatively retained for
  snapshot readers. Physical cleanup, backup retention, and automatic backup
  before migration remain runtime integration work.

See ADRs 0026–0029 for current contracts. Earlier descriptions of the original
snapshot primitive are historical foundations; encrypted local backups now
wrap database snapshots and all selected attachment objects. They are not yet
wired to application commands or automatic startup migrations.
