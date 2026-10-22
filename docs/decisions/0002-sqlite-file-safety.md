# ADR 0002: Fail-closed SQLite identity, durability, and startup verification

- Status: accepted for the initial database layer
- Date: 2026-10-22
- Scope: opening and validating local SQLite databases

## Context

The application will eventually open a user-specific database outside the source repository, but its final macOS Application Support path has not yet been approved. Passing the wrong path to an auto-migrating SQLite connection could modify an unrelated database. A structurally readable file may also be unsafe to use if constraints, triggers, foreign keys, or account-tree relationships have been damaged.

This is personal financial data. The expected database is small, and protecting confirmed writes and refusing ambiguous files are more important than minimizing startup latency.

## Decision

- Reserve SQLite `application_id` `1799434289`, whose hexadecimal bytes represent `kA01`.
- When the configured path does not exist on macOS/Unix, atomically reserve it
  as a `0600` file before SQLite first opens it. Do not alter an existing path
  until identity preflight accepts it. Subsequent SQLite connections may not
  recreate a path that disappears between stages; they fail closed instead.
- Before WAL or migrations are enabled, inspect the target with a single connection that does not change its journal mode.
- Accept only an empty SQLite database or one already carrying the expected application ID. Reject non-SQLite files, databases with another application ID, and unmarked databases that already contain schema objects.
- Claim an accepted empty database with the application ID before SQLx creates migration metadata, so an interrupted first migration can be retried safely.
- After identity succeeds, restrict the database and existing or newly created SQLite sidecars to owner read/write permissions (`0600`) on macOS/Unix; never change permissions on a rejected file.
- Use WAL journaling with `synchronous=FULL`, foreign-key enforcement, a five-second busy timeout, and four pooled connections for file databases.
- After migrations, use one read transaction to run full SQLite integrity and
  foreign-key checks, verify the exact migration-table signature and required
  schema whitelist (including object ownership and critical constraints), and
  audit semantic account-tree and posted-journal relationships.
- Fail closed. Never silently repair a database that fails verification.

## Alternatives considered

### Trust the configured filename

Rejected. Filenames and extensions do not prove that a SQLite file belongs to this application, and an incorrect path could receive migrations before the mistake is noticed.

### Use only SQLx migration history

Rejected. Migration history detects known migration versions and checksum changes, but it does not by itself prove that live tables, indexes, triggers, and stored relationships remain intact.

### Use `synchronous=NORMAL`

Rejected for the current personal workload. It can reduce write latency in WAL mode, but the database is expected to be small enough that `FULL` synchronization is the safer default for confirmed financial writes.

### Automatically repair failed checks

Rejected. Automatic repair can discard or reinterpret financial records. The original file must be preserved and diagnosis or restoration performed against copies through an explicitly reviewed procedure.

## Consequences

- A mistaken existing path is rejected before application schema, WAL settings, or file permissions are changed.
- Startup performs more I/O than a minimal open, in exchange for earlier detection of corruption and schema drift.
- Intentional future schema changes must update both forward-only migrations and the required-schema audit in the same release.
- The application ID is only a collision guard. It is not authentication, encryption, or proof that a file is trustworthy.
- Backup and restore procedures must still be designed and tested separately.
