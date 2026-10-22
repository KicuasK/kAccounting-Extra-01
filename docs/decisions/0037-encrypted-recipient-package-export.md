# ADR 0037 — Encrypted recipient-package export and recovery

Date: 2026-10-22. Status: implemented library workflow; CLI/HTTP and importer integration are separate increments.

The recipient package is a directory containing exactly `envelope.json`,
`manifest.kae`, `book.kae`, and `attachments/<canonical attachment ID>.kae`.
The public HPKE envelope contains no private key or accounting metadata. The
manifest, database, and every attachment use purpose-separated authenticated
streaming encryption under an independently generated recipient transfer root.
The old plaintext transfer implementation is never called or silently upgraded.
No sender-identity authentication is claimed by HPKE base mode.

A source `StorePin` is acquired before SQLite snapshot creation and retained
through object export. The snapshot determines the full catalog, including
attachments owned by drafts. Source database and attachment objects are not
modified. Every local attachment is authenticated and reencrypted through the
bounded-memory transfer pipeline; plaintext attachment bytes never reach disk.
The temporary database snapshot is owner-only inside locked private work.

The manifest binds package ID, exact serialized envelope digest/length,
plaintext and transfer ciphertext descriptors, and the original database
attachment references. Independent inspection verifies exact directory layout,
rejects symlinks and unexpected/missing files, authenticates and hashes all
objects, verifies SQLite integrity, and reconciles the complete catalog. Paths
come from canonical IDs, never user filenames. New database verification paths
must belong to a private directory outside the source tree; callers own their
cleanup even on error. Partial verification databases must never be activated.

Publication uses the existing atomic no-replace directory primitive and then
independently verifies the published name before reporting success. A public
package that cannot be verified is retained and reported as requiring recovery.
Successful cleanup failures are warnings, not permission to report a failed
publication as absent. Application callers may disconnect without cancelling
an active export: an owned task retains the source pin and OS work lock until
all workers finish. Process death instead invokes restart recovery.

Durable state records begin as `working`. Before publication, state advances to
`published_unverified`, conservatively covering both sides of an interrupted
rename. Only successful independent verification advances it to `verified`.
A dedicated `TransferRecovery` encrypted-object purpose wraps a private recovery
context under the source local key. That context binds the exact operation
name, destination, package ID, and transfer root. The root is never stored as
plaintext; transient serialized buffers and parsed key strings are zeroized.

Recovery acquires the OS lock and rejects unknown/malformed/symlinked work.
Live workers are preserved. Correct source local key and authenticated matching
context are required even for cleanup. An absent destination in a non-success
state permits abandoned private-work cleanup; a present destination requires
full independent package verification before state can advance and work can be
removed. Failed authentication or uncertain public output preserves both work
and destination and reports required investigation. Recovery does not delete
or overwrite published packages, access Keychain, switch the active book, or
infer accounting policy. No real financial data is used by tests.

Validation covers empty and attachment-bearing exports, independent database
inspection, no source mutation, wrong key/context, tampered manifest/object,
missing/extra files, destination conflicts, symlink rejection, unknown work,
live-worker locks, correct-key-only cleanup, and uncertain-publication retention.
