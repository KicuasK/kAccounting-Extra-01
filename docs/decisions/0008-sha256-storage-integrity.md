# ADR 0008: SHA-256 storage-integrity descriptors

Status: accepted for attachment and transfer integrity

## Decision

- Identify stored byte content with a SHA-256 digest encoded as 64 lowercase
  hexadecimal characters.
- Record the exact unsigned byte length with every digest. Verification must
  match both fields.
- Hash content through bounded streaming reads so large attachments are not
  loaded wholly into memory.
- Use the same bounded stream to copy and describe bytes. A read interruption
  or capacity-style write failure is an error; partial output remains private
  work and is never a publishable object.
- Treat malformed digests, read failures, and byte-count overflow as hard
  validation failures.
- Use the stable algorithm label `sha256` when the future manifest format needs
  an explicit algorithm field.

This decision defines content integrity only. It does not decide attachment
ownership, deduplication, deletion, allowed media types, encryption, manifest
encoding, or the physical `.kagextra` container.

## Rationale

The transfer workflow needs a portable digest that can be independently
recomputed on local and removable storage. SHA-256 is widely implemented,
stable, and sufficient for detecting incomplete or damaged attachment copies.
Keeping byte length beside the digest gives clearer diagnostics and rejects
truncation before content comparison.

## Consequences

Changing the digest algorithm later requires an explicit manifest-version
migration. Human-facing filenames must not be trusted as integrity evidence.
The application must recompute descriptors after copying rather than trusting
source metadata.

For local live-book storage, a verified-copy primitive writes to a private
owner-only work file beside the destination, synchronizes and re-reads it,
publishes with no overwrite, and treats later private cleanup failure as a
warning. This local-filesystem primitive is not the undecided removable-drive
`.kagextra` publication mechanism.
