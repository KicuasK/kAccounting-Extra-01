# ADR 0036 — Encrypted transfer manifest contract

Date: 2026-10-22. Status: implemented foundation, package orchestration pending.

`transfer_manifest.rs` adds a separate versioned manifest for the encrypted
recipient-transfer workflow. It does not reuse or upgrade the plaintext
`storage::TransferManifest` format silently. No accounting policy changes.

The manifest uses the existing `TransferManifest` encryption purpose and binds
the canonical package ID and exact serialized recipient-envelope digest/length.
It records plaintext and transfer-ciphertext descriptors for the database and
each attachment, plus original local-ciphertext descriptors for attachment
reference reconciliation against the source SQLite snapshot. The encrypted
database itself binds entry ownership, display names and all other metadata.

Attachment paths must be derived from validated canonical IDs. No package path
or original filename is accepted by this contract. IDs are sorted and unique;
the complete reference set must match the verified decrypted database, including
drafts. Missing, extra, duplicated or changed references are errors. Per-file
250 MiB and per-book 20 GiB limits are checked with integer arithmetic. Descriptor
lengths are canonical decimal strings and digests canonical lowercase hex.

Manifest plaintext is held only in zeroizing, bounded metadata memory (64 MiB),
never a plaintext manifest file. Parsing occurs only after complete encrypted
object authentication. Unknown fields, incompatible versions, invalid IDs,
noncanonical descriptors and wrong package/envelope contexts fail closed.

This is not a complete package implementation. Before publication the future
orchestrator must verify the SQLite snapshot, reconcile its full catalog,
authenticate and hash every object, reject unexpected package files, publish
without replacement, and durably track verification/recovery state. Import must
re-encrypt into a new local book, update ciphertext descriptors atomically and
verify the restored book before active-book switching. No HTTP/CLI transfer
command or claim of completed cross-Mac transfer is introduced here.

Tests cover authenticated round trips, empty books, ordering, complete reference
matching, duplicate references, changed descriptors, context/key mismatches,
tampering, truncation, trailing bytes and strict authenticated JSON rejection.
