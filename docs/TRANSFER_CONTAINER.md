# Single-file recipient transfer transport

`backend::transfer_container` transports a recipient-encrypted package as one
file suitable for Safari Downloads, then manual movement to APFS or exFAT.
Suggested extension: `.kagextra-download`; package-directory formats are unchanged.

All integers are unsigned, fixed-width big-endian. Layout:

- 12 magic bytes: `KAXFER01\r\n\x1a\n` (the `01` is the format version).
- Four-byte entry count.
- For each entry in strictly increasing UTF-8 byte/name order: two-byte name
  length, UTF-8 name, eight-byte payload length, 32 raw SHA-256 bytes, payload.
- EOF immediately after the final payload; no padding or trailers.

Allowed names are exactly `book.kae`, `envelope.json`, `manifest.kae`, and
`attachments/{32 lowercase hexadecimal attachment ID}.kae`. All three fixed
entries are mandatory. No directory entries, aliases, compression, absolute
paths, slash variants or symlinks exist in this format. Headers are bounded to
128 bytes per name; copy buffers are at most 64 KiB. Capacity is configurable
through `ContainerLimits` and checked before writing each extracted object.

`ContainerEntry::new` validates names; `write_container` receives explicit
expected `StoredObjectDescriptor`s, never scans an arbitrary folder, verifies
source lengths/digests and source identities while streaming, and returns the
exact encoded length. The caller must obtain descriptors from a verified
recipient-package context. Failed output may contain incomplete bytes and must
be discarded, never marked complete.

`extract_container` accepts a stream and creates a NEW owner-only staging
directory, using exclusive owner-only files. It never overwrites a destination
or deletes an uncertain stage. On failure, the caller reports the known private
stage and retains it for explicit recovery. Successful parsing is **not recipient
authentication**: call full recipient package verification and SQLite/reference
validation before importing or publishing a book. Extraction never switches the
current book. Container checksums are integrity diagnostics, not signatures.

The original source package is preserved on success and failure. Real financial
data, real production keys and external media were not used in unit tests.
