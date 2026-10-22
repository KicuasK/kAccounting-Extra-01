# Encrypted recipient transfer

These non-UI functions provide encrypted package export and verified import into
an explicit **new** local book. They do not transfer an existing Mac's Keychain
or select the imported book. Use only disposable data for development validation.

The user requires APFS and exFAT transfer-media compatibility. Current tests use
the local Mac filesystem, not a physical removable drive. Existing export work
requires private directories and same-filesystem directory publication, so exFAT
must not yet be advertised as working. The remaining adapter must keep SQLite
plaintext/sensitive work on private local storage, copy only encrypted payloads
to removable-media staging, verify them independently there, and retain the
conservative no-overwrite/interruption guarantees. Do not weaken permission
checks globally or ask the user to reformat as a hidden workaround.

## Export and import

1. On the destination Mac create a recipient request using
   `recipient_transfer::create_recipient_request`. Only its public JSON travels
   to the source Mac. Preserve the destination's corresponding private key.
2. The source explicitly chooses that request and a new destination package
   path. `recipient_package::export_recipient_package` snapshots SQLite while
   pinning its complete attachment catalog. The database, manifest and every
   attachment are encrypted; the package contains only a public recipient
   envelope plus encrypted objects. Publication never replaces an existing name.
3. Move the completed package through a private channel. HPKE base mode binds
   the package and recipient context; it does not authenticate sender identity.
4. On the destination explicitly choose the request and an absent new-book path.
   `recipient_import::import_recipient_package` unwraps the transfer key through
   the recipient store, verifies the entire package, creates a fresh local device
   key/backup identity, and re-encrypts every attachment locally. It preserves all
   journal/audit history and only updates local attachment ciphertext descriptors
   on its isolated database copy. The result is independently verified before
   and after no-replace publication, and opens through `RuntimeBook`.
5. Keep the existing book selected. Select the newly imported book only through
   a separate explicit action. Application startup always asks for a book.

Public identifiers, ciphertext descriptors and operation statuses are not secret
keys. No transfer key, device key, database, backup or runtime staging belongs in
Git. Local book SQLite data is private runtime data; package database encryption
does not imply that local SQLite itself is encrypted.

## Interruption and failure

A cancelled caller does not interrupt the owned export/import operation task;
its work lock stays held while workers finish. An actual process or power loss
can leave private work and create-only keys. Preserve both.

`inspect_interrupted_recipient_imports(parent)` lists bounded advisory checkpoints
and skips active workers. It performs no recovery mutation. A `ready` or
`published` JSON record is not cryptographic proof of completion and cannot
authorize publication or active-book switching. `PublishedUnverified` identifies
a destination whose rename happened but final verification/durability was
uncertain; do not retry over that destination or delete it automatically.

`recover_interrupted_recipient_imports(parent, device_keys)` provides conservative
restart recovery. It uses encrypted initial and READY contexts bound to the exact
operation, parent/work identities, destination, package and existing local key.
The source package and Keychain keys are always retained.

- For an absent destination, an authenticated owned initial context authorizes
  removing only that private unpublished staging tree. Retry the original
  package into an absent destination.
- For a present destination, recovery requires an authenticated READY checkpoint
  plus exact database/catalog descriptors, the book's runtime lock, all local
  attachments, the binding and backup identity. Only a fully verified book
  permits work cleanup and a verified-destination result.
- Active workers are skipped. Missing keys, modified/open books, context
  tampering, unsafe paths, nonempty SQLite WAL/journal files and uncertainty
  preserve work and destination as unrecognized. Old operations without the new
  authenticated contexts also remain preserved.

There is no byte-level resume and recovery never publishes an old stage or
switches books. Plain `ready`/`published` status JSON cannot authorize cleanup.
Recovery retains its work/runtime locks through owned tasks if the caller
cancels. Never manually publish a staging tree based on a status record. The
exporter has its own authenticated recovery operation.

The import path is absent-only and outside the source tree; private parent
permissions, symlink/path aliases and directory identity are checked. Existing
books are never overwritten. A failed import may leave an unused Keychain key;
keys are never silently deleted, replaced or regenerated to repair missing keys.

## Verification

Run `cargo test --manifest-path backend/Cargo.toml --test recipient_package`,
`--test recipient_import`, and `--test non_ui_acceptance`, followed by the full
project verification script. Tests use temporary books and fake key stores;
real cross-Mac Keychain, power-loss and Mac Safari acceptance remain separate
manual acceptance work. This document does not claim Version 01 complete.
