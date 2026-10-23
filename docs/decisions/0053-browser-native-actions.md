# Explicit browser native actions and bounded downloads

Status: accepted engineering decision, 2026-10-23. Implements the approved
private single-user storage/attachment UI; introduces no accounting policy,
password, public listener, or Funnel permission.

## Native selection

The local browser composition uses `NativeHost.browser_router()`. It adds only
`POST /api/native/selections` to the ordinary session/data router. The request
is strict JSON `{kind}` limited to 4096 bytes. The eight accepted kinds are
`book`, `request`, `package`, `backup`, `request-destination`,
`export-destination`, `book-destination`, and `recovery-parent`.

The listener remains loopback-only and applies the existing exact localhost
Host/Origin and fetch-site policy. Unlike legacy compatible data calls, this
native action requires `X-kAccounting-Selection` and a selected matching book.
The selection lease spans native panel admission and capability registration;
an outdated tab cannot register a selection for another book generation.

The browser supplies no helper executable, owner UID, filesystem path, key,
arbitrary capability identifier, or overwrite instruction. Trusted startup pins
the helper; the user selects through its native panel. The response is
`{capability_id: <opaque lower-hex capability>}` or `{capability_id: null}` on
cancellation. Admission remains one-panel-at-a-time, with cancellation/reaping
and host shutdown handled by the native host.

Private Unix/HTTPS ingress uses the existing `NativeHost.router()` composition,
which excludes browser-native panel actions. Enabling local browser panels does
not authorize remote callers to open panels on the Mac.

## Streaming upload and browser-native download

Attachment upload and encrypted-container import pass the original browser
`File` directly to `fetch` with `application/octet-stream` and the pinned
generation header. They do not read a whole `Blob`, `ArrayBuffer`, base64 string,
or plaintext temporary file. Accounting/storage writes retain ordinary single
admission and unknown-outcome protections.

Safari's native download action cannot set a custom generation header. A narrow
alternative permits the single query field
`download_selection=<32 lowercase hexadecimal generation>` only on these GETs:

- `/api/attachments/{32-lowercase-hex-attachment-id}/content`
- `/api/storage/operations/{32-lowercase-hex-operation-id}/download`

The query guard is concurrency metadata, not an authentication secret. Existing
Host/Origin/fetch-site/private-identity boundaries remain in force. Duplicate or
extra query fields, invalid generation, unsupported route or method, and a
conflicting selection header are refused. The generation is compared under the
same selected-book read lease used for financial dispatch; the response body
retains that lease through streaming completion/drop. A stale download cannot
silently serve another book. Legacy direct clients may still use the existing
header path; new browser-native downloads always use the query guard.

The frontend starts a same-origin native download and does not materialize the
response as a whole-file Blob. Server authentication/integrity checks and forced
attachment disposition remain authoritative. UI success means only that the
request was handed to Safari; the user confirms completion in Safari's download
list/Downloads. Exports are downloaded there first and manually moved to external
media under the existing approved workflow.

## Verification boundary

Use disposable fake-key route/session tests for malformed and stale guards,
panel admission, private-route exclusion, and body leases; use frontend tests
for original-File transport, exact allowed download URLs, confirmation, and
unknown results. Record real Safari/native-panel acceptance independently in
`STORAGE_UI_ACCEPTANCE_2026-10-23.md`. Neither automated tests nor native-download
handoff establish production deployment, Keychain, login/reboot, or completed
file-download acceptance.
