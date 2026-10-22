# Storage HTTP integration

This is a backend API for the later local/native selection adapter and Safari
UI. It does not create an operating-system file picker. There is no browser
endpoint that registers or enumerates filesystem paths.

Create one `StorageOperations::new(Arc<DeviceKeyStore>,
Arc<RecipientKeyStore>)` per application process. To export the active book,
configure `.with_active_book(Arc<RuntimeBook>)` before cloning the service.
Mount `api::localhost_storage_router(pool, operations.clone())` alongside the
existing localhost application routes. Do not instantiate a fresh service per
request; admission and capabilities must be shared.

The trusted adapter calls `register_selection(SelectionKind, absolute_path)`
after the user chooses a file/directory. It supplies only the resulting opaque
identifier to the browser. Public request selections retain the parsed request
in memory. Re-selecting a request creates a new capability. Do not expose
registration as an unauthenticated IPC or HTTP convenience method.

All routes use the existing localhost Host/Origin policy and no-store headers.
POST bodies require `application/json`, reject unknown fields, and accept only
opaque capability strings. No request accepts paths, passwords, private keys,
book selection changes or overwrite options.

| Route | JSON fields / result |
| --- | --- |
| `POST /api/storage/recipient-requests` | `destination` (RequestDestination) |
| `GET /api/storage/selections/{id}/request` | Request capability; returns `request_id`, `fingerprint` |
| `POST /api/storage/exports` | `book`, `request`, `destination` (ExportDestination), `confirmed_request_id`, `confirmed_fingerprint` |
| `POST /api/storage/imports` | `package`, `request`, `destination` (BookDestination) |
| `POST /api/storage/restores` | `backup`, `destination` (BookDestination) |
| `POST /api/storage/recoveries` | `kind: "imports"`, `parent` (RecoveryParent); or `kind: "exports"`, `book`, `parent` |
| `GET /api/storage/operations/{id}` | Operation status |

Example export after the adapter supplies capabilities and the user confirms
the previewed recipient identity:

```json
{
  "book": "<Book capability>",
  "request": "<Request capability>",
  "destination": "<ExportDestination capability>",
  "confirmed_request_id": "<preview request_id>",
  "confirmed_fingerprint": "<preview fingerprint>"
}
```

Accepted POSTs return 202 with `{operation_id, state: "running"}`. Poll the
status route until `state` becomes `succeeded` or `failed`. Failures contain the
sanitized `error_code: "storage_operation_failed"`; preserve source data and
use explicit recovery rather than blindly retrying a consumed destination.
Detailed operation errors must be inspected locally without exposing keys or
absolute paths over HTTP. There is currently no persistent HTTP job history.

A successful result includes only relevant metadata. Recipient request results
provide the public request ID/fingerprint. Export/import results include the
package ID and attachment counts; all counts and byte sizes are decimal
strings. Cleanup warnings are booleans. Import/restore set `selection_required:
true`: the user keeps the current book and explicitly chooses whether to open
the new one. They do not create a live-book backup, because the live book is
not mutated.

Recovery results contain string counts `removed_count`, `verified_count`,
`active_count`, `unrecognized_count`, without filesystem paths. For exports
staged by the portable exporter, register the private source book root as the
recovery parent. Imports recover in their chosen private destination parent.
Unknown work is preserved by the existing recovery implementations.

The service admits one job at a time; concurrent submissions receive 409
`storage_busy`. Its tasks retain ownership after HTTP disconnection. At most
256 selections and 256 statuses are retained; 409 `storage_capacity` requires
the trusted adapter to call `revoke_selection`/`forget_completed` after user
acknowledgement. Destinations are consumed on admission. Source capabilities
can be reused or revoked by that adapter. A failed job cannot silently replace
an existing file or book; select a fresh destination explicitly.

API tests: `cargo test -p kaccounting-backend --test api_storage_commands`.
These use disposable books and fake keys. Native picker integration, physical
APFS/exFAT, Keychain, Safari and authenticated private HTTPS acceptance must be
verified separately. The basic library router alone is not a network exposure
authorization.
