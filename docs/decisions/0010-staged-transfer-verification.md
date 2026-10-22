# ADR 0010: Read-only staged transfer verification

Status: accepted and implemented

## Decision

Before import publication, a decoded transfer candidate is held in a private
staging directory and verified read-only:

1. the staging root must be a real directory, not a symlink;
2. the database is the regular non-symlink file `book.sqlite3` and must match
   the manifest's SHA-256 and exact byte length;
3. the database must independently pass the existing application identity,
   migration, schema, foreign-key, physical, and accounting-semantic audit;
4. every declared attachment must be a regular non-symlink file and match its
   manifest descriptor.

Verification never creates a missing object, migrates a candidate, or publishes
it as a live book. Import replacement and rollback remain a separate workflow.

## Rationale

Content hashes alone cannot prove that the SQLite file belongs to this
application or preserves its accounting constraints. Conversely, a healthy
database does not prove that its attachments survived a removable-drive copy.
Both checks are required before an import can be considered publishable.
Rejecting symlinks prevents manifest paths from escaping private staging.

## Consequences

ADR 0014 adds the physical directory-package loader around this verifier. It
validates the exact package layout, rejects symlinked intermediate directories
and undeclared objects, then calls this verifier. Attachment and package size
limits remain open product decisions.
