# ADR 0022: Application-managed at-rest encryption boundary

Status: accepted as a Version 01 product requirement; cryptographic design
pending review

## Context

Version 01 stores many private financial attachments on the Mac, creates
application-managed backups, and transfers a complete book through a portable
`.kagextra` package. File permissions, removable-drive permissions, and
SHA-256 integrity descriptors do not protect those bytes when storage is read
outside the running application.

The user confirmed on 2026-10-22 that application-managed at-rest encryption
is required for all three classes of data.

## Decision

- Encrypt live attachment objects, application-managed backups, and
  `.kagextra` payloads at rest through the application.
- Do not publish a plaintext payload as a completed attachment, backup, or
  transfer artifact, and do not intentionally retain plaintext intermediate
  files after success or failure.
- Preserve bounded-memory streaming for large attachments and exports.
- Use authenticated encryption and fail closed when authentication,
  truncation, format, version, or key validation fails.
- Continue recording and checking exact byte lengths and SHA-256 descriptors
  as storage and transfer integrity evidence. Hashing is distinct from
  encryption and does not replace it.
- Keep the live SQLite database outside this ADR's encryption boundary. Its
  existing application identity, integrity, file-permission, backup, and
  transactional rules continue to apply.

## Follow-up design

Implementation must not begin merely by choosing an algorithm name. ADR 0023
defines the Version 01 streaming encrypted-object primitive. Integration work
must additionally define:

- local macOS key storage and recovery behavior;
- passwordless `.kagextra` recipient-key wrapping and new-computer recovery;
- key loss, rotation, cancellation, interruption, and plaintext-residue
  behavior;
- dependency maintenance and compatibility policy.

The design must include known-answer or interoperable test vectors, tamper and
truncation tests, interrupted-write tests, large bounded-stream tests, and
missing, wrong, and unavailable key tests before use with any real book.

## Consequences

The attachment repository, backup/restore workflow, and `.kagextra`
export/import format must be designed together around this boundary. Existing
plaintext transfer primitives remain foundations for integrity and failure
semantics, but they are not a complete Version 01 storage format.

Adding live SQLite database encryption later would be a separate product and
migration decision; it must not be implied by this ADR.
