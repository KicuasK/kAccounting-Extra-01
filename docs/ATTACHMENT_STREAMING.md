# HTTP attachment streaming

Implemented in the private book-command router. These endpoints inherit its
loopback Host, Origin, and fetch-site validation and `Cache-Control: no-store`.
No request accepts a server filesystem path or key material.

## Upload

`POST /api/journal/entries/{entry_id}/attachments?display_name={url_encoded_filename}&source_kind=regular_file&expected_revision={draft_revision}`

Send raw file bytes as `Content-Type: application/octet-stream`. Use standard
UTF-8 percent encoding for the display filename. `source_kind=regular_file` is
required; directory/package selections must never be converted into a zip by a
client. No multipart or base64 wrapper is used. Unknown or duplicate query
fields are rejected. File names must be plain names of 1–255 non-control
characters without slash/backslash. Content-Length is optional, but when supplied
must match the bytes actually received. Chunked HTTP uploads are supported.

For a **draft**, supply the current canonical positive `expected_revision`.
Publication checks that revision under `BEGIN IMMEDIATE` and increments it
atomically with attachment insertion. Reload the draft after success. Two
uploads using the same draft revision cannot both publish. For a **posted
entry**, omit `expected_revision`: this is an explicit append of a new immutable
attachment, allowed by the approved Version 01 policy. Posted journal fields and
existing attachment references are never changed; replacement/removal remains
forbidden. Supplying a revision for a posted append is rejected.

Success is 201 with the existing attachment metadata JSON (exact integers remain
strings). Failures use the API error envelope. Limits remain 250 MiB/file and
20 GiB/book; quota and draft revision updates share the same transaction.
413 indicates an oversized request. Malformed filenames, executable signatures,
package names, or unsupported source kind return 400; stale/missing draft
revision and book quota return 409.

The browser cannot expose trustworthy Unix execute-mode bits or prove a selected
file was not reached through a symlink. The HTTP boundary therefore validates
explicit regular-file selection metadata, filename/package extensions, and the
first eight content bytes, including signatures split across HTTP frames. It
rejects shebang, PE, ELF, and Mach-O signatures using the same policy as local
imports. This is file-type screening, not an antivirus claim. Local filesystem
imports retain their stricter mode, symlink, and identity checks.

## Download

`GET /api/attachments/{attachment_id}/content`

The complete stored ciphertext digest and plaintext digest/authentication are
checked before a successful response is created. A second authenticated decrypt
pass streams from the same opened encrypted file handle. Responses use
`application/octet-stream`, exact Content-Length, `nosniff`, and forced
`Content-Disposition: attachment` with an RFC 5987 encoded UTF-8 filename. This
prevents active attachment content from executing in the application origin.

Corruption present before download returns 409 with no attachment bytes or
success headers. A later filesystem/I/O failure aborts the body stream; a client
must discard partial bytes. Cancellation closes the bounded output channel and
stops the decryption worker. No plaintext intermediate file is created.

## Resource and failure behavior

Upload producers and download consumers communicate with blocking crypto workers
through two 64 KiB payload slots. A repository shares a four-worker semaphore
across HTTP uploads/downloads. Upload signature inspection retains eight bytes;
crypto uses its existing 64 KiB authenticated framing. Memory does not grow with
attachment length. Socket/body implementations may separately retain transport
buffers; no application path collects a complete upload/download into memory.

Only ciphertext work files are created in private, leased attachment storage.
An interrupted body, mismatched length, oversized actual stream, encryption
failure, stale draft, or failed quota transaction cannot publish an attachment
reference. Private work is removed on ordinary failure/cancellation. The existing
crash rule remains: a process/power failure around durable publication can leave
unreferenced encrypted objects for exclusive offline maintenance, never a
committed reference to an unpublished object. After a lost success response,
list entry attachments before retrying; Version 01 deliberately does not deduplicate
or infer that two receipts are identical.

## Acceptance

`cargo test --locked --offline --manifest-path backend/Cargo.toml --test api_attachment_streaming`

Capacity acceptance generates bytes incrementally and consumes download frames
without collecting a plaintext file or whole-body buffer:

`cargo test --release --locked --offline --manifest-path backend/Cargo.toml --test api_attachment_streaming maximum_http_stream_roundtrips_and_undeclared_oversize_cleans_candidate -- --ignored`

Tests use disposable temporary books and fake in-memory keys, never a real
accounting book or the user's Keychain. Actual socket/runtime acceptance and Mac
Safari interaction acceptance remain separate integration checks.
