# ADR 0031 — Managed backup identity, checkpoints, and conservative retention

Status: Implemented foundation, 2026-10-22. Runtime integration and real Keychain acceptance remain explicit release gates.

Integration update: explicit CLI and guarded localhost backup commands are
implemented. New creation-order timestamps advance beyond authenticated existing
receipts under the maintenance lock, so a system clock rollback cannot demote a
new checkpoint. Reading those ordering receipts does not rehash old attachment
payloads; retention still verifies each complete candidate before deletion.
Real Keychain and representative large-book acceptance remain release gates.

2026-10-23 selected-session integration: restored/imported old-format books may
explicitly initialize their missing namespace through `POST /api/backups/identity {}`.
The UI offers this separate checkbox-confirmed action only after a manual backup
reports `backup_identity_missing`. Opening, restore and ordinary backup never
initialize it implicitly. Setup returns exact `201 {initialized:true}` and does
not start a backup, switch books or replace a device key.

`GET /api/backups/identity` returns exact `{initialized:boolean}` for explicit
unknown-outcome reconciliation. It authenticates existing metadata with the cached
process key; false means genuinely absent metadata and an empty identity cache.
Corrupt/replaced/inconsistent metadata fails with `backup_identity_invalid`.
Existing identity is create-only (`backup_identity_exists`), maintenance admission
is non-queued (`backup_busy`), and other failure preserves uncertain data.
Both routes enforce the selected session generation. An owned worker keeps that
book's lease across publication even if the HTTP caller disconnects; the same
runtime's shared, single-assignment identity cache is populated only after success.
Manual backup then works in that same session after a separate explicit action.
These checks do not grant authority to prune any previous namespace.

## Decision

A local device key does not identify an accounting book. Generate a random
128-bit `BookBackupId` exactly once on explicit new-book initialization and
persist it in authenticated encrypted `backup-identity.kae`, bound to the device
key ID. Opening an existing book never creates or replaces missing identity.
Recovery/import of a book without this file must explicitly initialize a new
backup namespace; it cannot inherit authority to prune another namespace.

Managed backup directories live outside the source tree in a private 0700 root.
The root has a 0600 OS maintenance lock. A managed child contains exactly
`backup.kabackup` and encrypted `receipt.kae`; interrupted children lacking a
valid receipt are preserved. The authenticated receipt binds its exact directory
name, per-book ID, key ID, creation timestamp, purpose, and encrypted catalog
SHA-256/length. The catalog already authenticates the database and attachment
objects. No editable filesystem timestamp determines backup age.

`verify_local_backup` authenticates every object, audits a private decrypted
SQLite snapshot, compares the complete attachment reference set, and removes
verification plaintext before returning success. It publishes no restored book.

`retain_latest_verified_backups` verifies candidates before deleting anything,
keeps the ten newest authenticated backups for the supplied persistent book ID
and existing key, and rechecks the newest survivor and each deletion candidate.
Directories that are foreign, incomplete, corrupt, renamed, have unknown files,
or involve symlinks are preserved. Missing keys fail the operation. Source data
is never a candidate. Retention is separate from checkpoint creation, so a
retention error cannot disguise a successfully created safety backup.

`create_managed_backup` plus `with_verified_checkpoint` supplies an explicit
backup-before-mutation flow. Failed backup creation prevents the operation;
failed operations preserve the checkpoint. This supports current audited-schema
books. A compatible pre-migration snapshot reader for older schemas remains
necessary; these functions do not weaken current schema auditing to accept one.

## Limits and verification

The caller must load the book's persisted identity rather than choose a different
identity arbitrarily. The service is a local private-user boundary and assumes
other cooperating processes honor its maintenance lock. It does not implement
scheduling, backup UI, automatic active-book switching, or encryption of the live
SQLite database. The new identity is deliberately not part of the old backup
package format; restoring that package cannot accidentally gain pruning rights.

Tests use only temporary SQLite books and an in-memory key store. They cover
newest-ten retention, the last valid backup, unknown/incomplete/corrupt/foreign
entries, symlink preservation, encrypted create-only identity, unavailable keys,
receipt-name binding, shared maintenance locks, and backup-before-operation
failure behavior. No real book or user Keychain is accessed.
