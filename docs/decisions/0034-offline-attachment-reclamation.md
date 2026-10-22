# 0034 — Offline attachment reclamation and snapshot pins

Date: 2026-10-22

Status: Accepted engineering implementation under the approved storage scope.

## Contract

An attachment reference may be removed from a draft, while a snapshot selected
before that removal still requires the encrypted object. Physical reclamation
therefore requires a cross-process boundary shared by all store users.

`attachment_maintenance::StorePin::acquire` holds a shared OS file lock on the
private store's permanent `.maintenance.lock` file. `AttachmentRepository`
retains that pin for its entire lifetime, including all clones and blocking
workers. Encrypted local backup acquires a pin **before** its SQLite snapshot
and retains it through published verification. Its blocking copy worker owns a
clone so cancellation of an awaiting task does not release protection early.
Future export/snapshot callers must acquire the same pin before selecting the
database and keep it through their final object read. Package-only reads do not
require a live-store pin.

`collect_unreferenced_ciphertext` is an explicit offline maintenance primitive.
All open repositories and snapshot/export readers must close before it can
obtain an exclusive OS file lock. It also holds a SQLite `BEGIN IMMEDIATE`
transaction, validates all reference identifiers, rechecks each candidate's
absence in the matching live database, and checks directory/lock/file identity
before removing a candidate. It enforces the runtime layout by resolving the
transaction's `PRAGMA database_list` main filename to the private sibling
`book.sqlite3` beside the `attachments` directory. In-memory, foreign,
nonmatching or unsafe database paths fail before any removal. All reference
queries explicitly use `main.attachments` to avoid temporary-table shadowing.
The database path identity is checked again before every deletion.

Only private, single-link, regular files named exactly 32 lowercase hexadecimal
characters followed by `.kae` qualify. Referenced files, symlinks, directories,
hardlinks, nonprivate files, unknown names, work directories, and the permanent
lock file are preserved. Collection performs no recursive directory deletion,
never edits accounting rows, and syncs the object directory before reporting
success. An interrupted run may reclaim a safe subset; rerunning is idempotent.

## Operational limits

This conservative first implementation requires closing the book and all
readers for physical reclamation. It does not provide online per-snapshot
reference counting. No default background cleanup or application command is
wired yet. All processes must follow the pin protocol. External tools must not
remove/replace the lock marker or mutate the private store; detected identity
changes fail closed. OS locks coordinate cooperating processes, not arbitrary
same-user filesystem tampering.

The quota remains logical referenced plaintext bytes. Physical ciphertext use
can exceed that quota until maintenance is explicitly run. No real financial
book was used by development or tests.

## Validation

Tests cover retained live references, detached ciphertext reclamation,
repository and cloned snapshot pins, a separate process holding a shared OS
lock, repeated collection, unsafe stores/replaced lock files, and preservation
of unknown entries, symlinks, hardlinks, directories, and nonprivate files.
Foreign and in-memory databases and the wrong store layout cannot collect
another book's ciphertext.
Existing attachment and encrypted local-backup regression suites also pass.
