# Browser transfer download

User decision (2026-10-22): Safari downloads one encrypted `.kagextra` file to Downloads; the user subsequently moves it to APFS or exFAT storage. Downloads do not switch the active book.

`GET /api/storage/operations/{operation_id}/download` returns only successful recipient-encrypted exports produced by this process. A private registry holds a producer-authenticated whitelist of ciphertext descriptors, never request-supplied paths. Generic directories, other operation types, unknown jobs and replaced/tampered artifacts are refused. Forgetting a completed operation revokes its download.

The response is `application/octet-stream`, `Content-Disposition: attachment` with a fixed safe filename, and `Cache-Control: no-store`. Existing localhost Host/Origin controls apply. Each download checks filesystem identity and expected ciphertext hashes. A worker streams the versioned transfer container through a two-item channel with chunks limited to 64 KiB. Browser disconnect stops the producer; failure midstream aborts the body rather than reporting complete success. Root keys and decrypted databases are never part of the container.

Container framing hashes provide transport integrity, not recipient authentication. The binary upload/import adapter extracts into a fresh private directory and performs full recipient package verification before import. The active book must remain unchanged until explicit selection.

Pre-UI limitation: export still needs locally registered book/request/destination capabilities. The HTTP endpoint does not accept native paths or implement the OS picker. Safari end-to-end download/save/upload interaction and native picker wiring remain acceptance work.

## Binary browser import

`POST /api/storage/container-imports?request=<opaque request selection>&destination=<opaque new-book destination selection>` accepts `application/octet-stream`. No paths or keys are accepted. It consumes the destination capability once admission succeeds, shares the existing storage-operation permit, and returns HTTP 201 only after the complete body, framing validation, recipient authentication, decrypted database validation and newly encrypted destination publication succeed. The response contains exact string counts and `selection_required: true`.

A two-item 64 KiB bridge provides backpressure. End-of-stream is explicit: body failure, length mismatch or request cancellation closes the bridge without a successful EOF marker. An owned supervisor holds the storage permit until the reader and import finish, even if the HTTP request is dropped. Cancellation after complete upload may allow an already authenticated import to finish; retry must select a fresh destination and never overwrites an existing book. Retained encrypted container work follows the container-import helper's inspection policy.

Deployment transport defaults are 64 GiB total encrypted object bytes, 32 GiB per object, and 100,003 entries. These are technical resource caps, separate from the approved per-attachment and per-book accounting policies. A native deployment may set `StorageOperations::with_container_limits` before sharing the service. The same configuration controls extraction plus declared and streamed HTTP lengths. Wire framing permits at most 128 additional bytes per entry plus 64 fixed bytes. Oversize HTTP bodies receive `transfer_payload_too_large`, distinct from the JSON request limit.

At most two download producers may run per storage service. Admission occurs before ciphertext preflight hashing and remains owned through streaming, including detached preflight workers. Additional downloads receive HTTP 409 `storage_download_busy`. Dropping a slow download closes its channel and releases admission when its producer exits.

After fully verified successful import, the application automatically removes
only its owned internal encrypted container copy. Original Downloads/external
packages and failed/interrupted staging remain intact. Unknown or replaced work
is preserved; optional cleanup failure leaves import successful and returns
`cleanup_warning: true`. See [ADR 0051](decisions/0051-successful-container-import-staging-cleanup.md).
