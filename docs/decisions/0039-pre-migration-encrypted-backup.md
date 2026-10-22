# ADR 0039: Backup-first legacy schema opening

Status: accepted engineering implementation, with interrupted upgrade work requiring inspection.

Before an existing legacy book receives a writable database handle, recognize its
exact successful migration prefix and recreate that historical schema in memory.
Reject missing/extra schema objects, modified trigger SQL, checksum mismatches,
physical corruption and foreign-key failures. Current-schema integrity requirements
remain unchanged.

Acquire a shared attachment-store pin before a consistent SQLite snapshot. Preserve
that original schema snapshot, migrate a disposable copy, run the existing complete
current-schema audit and select every attachment reference from that copy. Encrypt
the original snapshot under the existing device key with Backup purpose; copy and
authenticate every referenced attachment ciphertext. Encrypt a separate bounded
manifest with BackupManifest purpose, binding format, version, device key ID,
original snapshot hash/length, and the exact complete attachment descriptors.

Independently decrypt the completed candidate, validate the original history and
schema, migrate a fresh disposable copy, run the complete current audit, compare
the full attachment reference set, and authenticate every attachment. Synchronize
all package files and directories, publish without replacement, synchronize its
parent, and repeat independent verification of the published path. Only then may
runtime opening migrate the live database. Backup creation never generates or
changes a key. Backup verification or migration failure cannot remove the retained
pre-upgrade package. No automatic retention applies to these recovery points.

The dedicated `.kabackup` format is distinct from current-schema local backups;
using the current backup verifier against a historical schema would incorrectly
reject valid recovery material. The explicit `restore-pre-migration` command now
restores a verified checkpoint into a NEW current-schema local book. It loads
only the source binding/existing key under the process lock, so damaged source
SQLite and retained scratch do not prevent recovery. The checkpoint and source
evidence remain unchanged. Exact database bytes, binding and full attachment
content are checked after publication. No active-book selection is performed.

Success cleans private scratch after all database pools have closed. An error,
cancellation or crash preserves private plaintext
SQLite scratch; startup detects `.migration-work-*` and refuses opening pending
inspection. It never deletes unknown work or silently resumes migration. Automatic
validated recovery of interrupted upgrade work remains follow-up work. Book and
store process locks coordinate supported application clients; unsupported direct
SQLite writers must not modify a book while it is open.

Runtime startup runs migration/audit/open in an owned asynchronous task retaining
the process lock if its caller cancels. A regression blocks a legacy database
with an exclusive SQLite lock, drops the selecting future, verifies BookBusy,
then releases SQLite and checks eventual safe reopen. Backup catalog output is
bounded during serialization as well as parsing, and successful scratch cleanup
synchronizes its parent directory.
