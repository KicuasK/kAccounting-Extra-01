# ADR 0027: Immutable encrypted attachment repository

Status: accepted engineering implementation of Version 01 decision 4.

Migration `0007` gives every attachment a permanent journal-entry owner, random
128-bit identifier, original display filename, exact plaintext SHA-256/length,
exact ciphertext SHA-256/length, and creation timestamp. Each attachment owns
one object; identical content is intentionally not deduplicated.

## Publication and lifecycle

- Import only regular non-symlink files. Reject directory/package selections,
  executable permission bits, recognized executable signatures and executable
  filename extensions. Display names are plain filenames with no control
  characters or separators, and never become storage paths.
- Stream encryption through ADR 0023 in a private work directory. No plaintext
  attachment intermediate is written. Check the source identity and length,
  synchronize ciphertext, then authenticate/decrypt it into a hashing sink.
- Acquire SQLite's writer slot, insert metadata with the book quota enforced,
  publish exclusively, synchronize the object directory, then commit. A crash
  or uncertain commit can leave unreferenced ciphertext; it cannot justify
  deleting bytes that might have a committed reference.
- Enforce 250 MiB per regular file and 20 GiB of referenced plaintext per book,
  with exact integer arithmetic and independent SQL constraints/triggers.
- Posted entries permit appending attachments. Existing attachment metadata
  cannot be updated, and a posted attachment cannot be removed.
- Removing a draft reference advances its expected draft revision. Replacing a
  draft preserves its attachments. Deleting a draft cascades only its metadata.

## Conservative physical retention

Unreferenced ciphertext is retained for now. A live database deletion must not
invalidate an already-created backup/export snapshot. Physical garbage
collection needs coordinated snapshot pins before it may be enabled. The
20 GiB limit currently measures referenced content, and retained ciphertext and
private work can consume additional physical disk space. This is an explicit
remaining release task, not a claim of finished disk-space management.

The repository checks owner-only store permissions and directory identity.
Keys are supplied through the key boundary; no key file is created. The source
checkout rejects `.kae` objects and private storage work as runtime artifacts.

## Evidence

Attachment integration tests cover empty/chunk-boundary objects, independent
identical uploads, immutable posted ownership, expected-revision removal,
draft lifecycle, incorrect/missing/tampered bytes, wrong keys, symlinks, root
replacement, unsafe uploads, quotas, and 128 independently verified objects.
All fixtures are isolated and contain invented financial data.
