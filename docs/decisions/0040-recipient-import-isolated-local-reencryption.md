# ADR 0040 — Recipient import into an isolated new local book

Date: 2026-10-22. Status: implemented import and conservative authenticated restart recovery.

Recipient import authenticates the exact recipient envelope using the destination
recipient Keychain boundary. A wrong request or missing private key fails before
new device-key creation. The whole encrypted package is then verified on an
unpublished private staging tree: exact layout, authenticated manifest and
SQLite snapshot, complete attachment catalog, and every encrypted attachment.
No source book, existing destination, journal record, or audit history is edited.

A fresh create-only device key and backup identity belong to each import attempt.
Attachments stream from transfer encryption into fresh local encryption. Only
on this isolated, authenticated, unpublished database copy may one SQLite
transaction drop `attachments_immutable_update`, replace **only** ciphertext
hash/length fields, and reinstall the exact migration trigger. The transaction
preserves IDs, ownership, plaintext hashes and lengths, dates, journal history,
and all audit fields. No general live-book exception or HTTP SQL bypass exists.
A full schema/data audit follows the transaction.

Before and independently after atomic no-replace publication, the importer
verifies the full database, complete catalog, every local attachment, exact
local key binding and authenticated backup identity. Uncertain post-publication
verification/durability returns `PublishedUnverified` with the destination and
preserves recovery evidence. The destination is never automatically selected.
User decision: every application start explicitly chooses a book; successful
import/restore keeps the current book until a separate explicit selection.

An owned Tokio operation task holds the OS work lock through all asynchronous
and blocking workers, including after caller cancellation. Private generated
work directories retain durable create-only `created`, `verified`, `ready`, and
`published` checkpoints. Directory identity checks guard publication. Work
inspection skips held OS locks and treats status JSON as advisory evidence only;
it never authorizes publication, deletion, switching, key creation or automatic
resume. No key material is serialized in status records.

Restart recovery uses separate create-only encrypted initial and READY contexts,
with distinct import domain strings under `TransferRecovery`. Each binds the
exact work name, parent, destination, package ID, existing local key ID and
parent/work device+inode identities. The public key locator only loads an
existing key. No raw key is serialized. Plain status records never authorize
mutation.

An authenticated initial context plus an absent destination authorizes removal
of only that operation's private unpublished work; the original source package
and all Keychain keys are retained. A present destination requires the
authenticated READY context, exact SQLite descriptor and canonical complete
catalog descriptor, runtime lock, authenticated binding/backup identity and every
local attachment. The database hash is checked before and after verification;
nonempty or unsafe WAL/journal sidecars are rejected. Only then may recovery
remove its work and report the destination verified. Missing keys, tampering,
copied contexts, active workers, changed books and uncertainty preserve all work
and destinations. Recovery itself runs as an owned task with locks retained on
caller cancellation.

This is conservative recovery, not byte-level resume: no old stage is published
after restart. An unpublished interrupted attempt is discarded only after
ownership authentication and can be retried from the preserved package. Work
predating these authenticated contexts remains unrecognized and retained.

Tests use disposable books and in-memory key stores only. Coverage includes
posted journal values above JavaScript's safe integer range, draft and posted
attachments, ciphertext rotation, runtime reopen, immutable metadata/delete
guards, wrong recipients, occupied destinations, tampered packages, advisory
recovery inspection, forged readiness, authenticated cleanup and published
verification, missing keys, context tampering, modified/open books and caller
cancellation.
