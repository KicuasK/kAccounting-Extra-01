# ADR 0012: Strict JSON transfer manifest

Status: accepted by user decision

## Decision

Every Version 01 `.kagextra` directory package stores its manifest at the
top-level path `manifest.json`. The file is human-readable UTF-8 JSON with this
shape:

```json
{
  "format_version": 1,
  "database": {
    "path": "book.sqlite3",
    "sha256": "<64 lowercase hexadecimal characters>",
    "byte_length": "<canonical unsigned decimal string>"
  },
  "attachments": [
    {
      "path": "attachments/<safe relative path>",
      "sha256": "<64 lowercase hexadecimal characters>",
      "byte_length": "<canonical unsigned decimal string>"
    }
  ]
}
```

`format_version` is a JSON integer because its supported range is small and
explicitly checked. Every `byte_length` is a string so Safari, JavaScript, or
another JSON consumer cannot round a valid `u64` value above `2^53 - 1`.
Canonical decimal strings contain ASCII digits only and have no leading zero
unless the complete value is `0`.

The decoder rejects malformed JSON, duplicate or unknown fields, an unexpected
database path, non-lowercase or invalid SHA-256 text, non-canonical or
out-of-range byte lengths, unsupported versions, unsafe attachment paths, and
duplicate attachment paths. These checks run before any referenced object is
opened.

The encoder orders attachments by validated logical path, emits fields in the
documented order with stable pretty printing, and ends the file with one line
feed. Monetary amounts do not belong in this manifest.

## Rationale

JSON is easy to inspect during recovery and has direct support in the Rust
backend and future Safari clients. Strict decoding prevents misspelled or
future fields from being silently ignored, while decimal strings preserve
exact large lengths across Rust and JavaScript.

## Consequences

Changing field meaning or accepting a structurally incompatible document
requires a new manifest format version. Import still verifies the actual size
and SHA-256 of every declared object and independently validates the SQLite
snapshot; successfully parsing `manifest.json` is not enough to trust a
package.
