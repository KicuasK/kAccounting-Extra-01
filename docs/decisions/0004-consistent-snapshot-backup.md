# ADR 0004: Verified no-overwrite SQLite snapshot backups

Status: accepted for the backend backup primitive

## Decision

- Create backups from an opened file-backed application database with SQLite `VACUUM INTO`, rather than copying a live database file and its WAL sidecars.
- Write the snapshot inside a newly created private (`0700`) temporary directory beside the requested destination.
- Restrict the snapshot to owner read/write (`0600`) and run the same read-only physical, relational, embedded-migration-checksum, schema, and semantic integrity checks used at application startup.
- Publish the verified file with a hard link. This is atomic on the destination filesystem and fails if any file or symlink already occupies the destination.
- Remove the private temporary file and directory after publishing. If this
  cleanup fails after publication, return a successful outcome with a cleanup
  warning rather than ambiguously reporting that the verified destination
  failed.

The destination must not already exist. The backend never overwrites a prior backup, because replacement and retention are operational policies rather than implicit database behavior.

## Rationale

Copying only the main file of a live WAL database can omit committed pages or produce an inconsistent backup. SQLite's snapshot operation produces a transactionally consistent standalone database while normal application access continues. Creating and verifying it behind a private directory prevents an incomplete or unverified file from being exposed as a finished backup.

## Consequences

- Callers must choose an existing destination directory outside the source repository and use a file-backed source pool; the primitive does not create an unapproved directory tree.
- A successful return proves that the published snapshot passed the current application integrity audit and retained owner-only permissions. Callers must inspect the outcome for a non-fatal cleanup warning.
- Existing destinations, including symlinks, are refused without content changes.
- Scheduling, naming, retention, encryption, and the final Application Support/backup directories remain unresolved operational policy.
- Restore is not implemented by this decision. A restore workflow must preserve the current database, verify the candidate, and define explicit replacement/rollback behavior before it is added.
- `verify_database_snapshot` may be used to inspect a restore candidate without enabling WAL, migrating, chmodding, or otherwise changing it; successful inspection alone does not authorize replacement of the live database.
