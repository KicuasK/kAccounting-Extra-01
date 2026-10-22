# ADR 0035: Streaming encrypted attachment transfer staging

Status: accepted engineering foundation for Version 01; package workflow pending

## Decision

`encrypted_transfer` converts local attachment ciphertext into ciphertext under
an OS-random recipient transfer root key, and converts transfer ciphertext into
ciphertext under a new destination-local root key. It uses the existing ADR 0023
object format with distinct `Attachment` and `TransferAttachment` purposes.

- Export requires a shared `StorePin`. The caller acquires this pin before its
  SQLite snapshot and holds it through all attachment reads. The operation
  validates the pinned store before and after conversion and rejects sources
  outside that exact store.
- The complete input ciphertext must authenticate and match the snapshot's
  exact ciphertext SHA-256 and byte count. Decrypted bytes must match the exact
  plaintext SHA-256 and byte count. Plaintext length is capped at the approved
  250 MiB per attachment before work starts, and enforced during streaming.
- Plaintext crosses a synchronous, capacity-one channel in at most 64 KiB
  zeroizing messages. Backpressure bounds memory independently of object size.
  Plaintext is never written to an intermediate file.
- The consumer encrypts into an exclusive private work file inside a private
  staging directory. Every output gets a new object salt and nonce. Late input
  authentication, descriptor, read, or output failures prevent publication.
- The complete staged ciphertext is independently decrypted and checked before
  publication. Publication uses the established verified, synchronized,
  no-replace local file publisher. The published name is then independently
  decrypted and checked again. Existing files and symlinks are never replaced.
- Source and output names are checked against open file identity. Source
  identity, size, and modification timestamp must remain stable across the
  conversion. Staging parent and work-directory identity and private permissions
  are rechecked before publication and cleanup.
- An output already published but failing final verification is reported with
  its path as `PublishedUnverified`, never as success. Successful publication
  and a subsequent work-cleanup warning are represented separately. Crashed
  work directories contain ciphertext only.

## Verification

Tests cover empty files, chunk boundaries, a 5 MiB object, source/destination
key separation, wrong key or purpose, late authentication failure, incorrect
plaintext and ciphertext descriptors, oversized plaintext declarations,
trailing ciphertext, no overwrite, symlink rejection, private staging, pinned
store containment, and no private work residue after failure. Injected output
exhaustion at the header and multiple chunk boundaries proves consumer shutdown
wakes a backpressured producer and returns failure without deadlocking. A recipient
fixture creates a request and HPKE envelope, exports under its transfer key,
unwraps on the destination fixture, and imports under an independent local key.
Fixtures use temporary files and an in-memory key store; no real financial data
or user Keychain items are accessed.

## Boundary and remaining integration

This module does **not** create or import a complete `.kagextra` package. It does
not serialize a package manifest, snapshot a database, switch active books,
manage removable-drive publication, or expose a user-facing transfer command.

The package orchestrator must bind the package identifier, recipient request
identifier and fingerprint, database descriptor, and complete attachment
reference set in an authenticated encrypted manifest. It must verify those
bindings and database references before publishing or activating any imported
book. A successful object conversion alone does not establish package validity.
The legacy plaintext transfer foundation must not be advertised as the final
encrypted cross-Mac workflow.
