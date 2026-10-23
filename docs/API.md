# Backend API

AR/AP posted-line tracking endpoints are documented in [AR/AP API](ARAP_API_2026-10-23.md).

Status: private loopback integration preview with explicit accounting and
book-command write APIs. Version 01 and Tailscale deployment remain incomplete.

The default constructor remains read-only. The explicit private runtime merges
accounting and book-command routers with frontend assets, opens an explicitly
selected book and binds only `127.0.0.1:3000`. It does not initialize a book or
replace missing keys during startup. See `LOCAL_RUNTIME.md` for commands.
Tailscale origin/identity integration and deployment acceptance remain pending.

Axum default features remain disabled. JSON, query parsing, HTTP/1 and Tokio integration are
explicitly enabled; HTTP/2 is disabled.

## Wire conventions

Every SQLite `INTEGER` identifier, revision, line number, and monetary amount is
encoded as a base-10 JSON string. Safari and other JavaScript clients therefore
cannot silently round values above JavaScript's exact integer range. Boolean
activation state remains a JSON boolean, and absent parent identifiers remain
JSON `null`. The current response models emit no JSON number values.

Dates are strings containing validated, existing `YYYY-MM-DD` calendar dates.
UTC database timestamps are returned as stored strings. Enum values use the
lowercase database vocabulary documented in `DATABASE_SCHEMA.md`.

## Payment memos and explicit payment candidates (2026-10-23)

Payment memos are non-posting plans, not ledger balances or payable recognition.
All routes are scoped to the selected book; session serving applies the existing
selection-generation guard. Write routes additionally require the exact trusted
Origin. The default read-only router exposes only the collection GET.

| Route | Request | Success |
| --- | --- | --- |
| `GET /api/payment-memos?from=YYYY-MM-DD&through=YYYY-MM-DD` | Both inclusive date bounds required; unknown query fields rejected | `200 {items: Memo[], required_total_minor: string}` |
| `POST /api/payment-memos` | `MemoFields` | `201 Memo` |
| `PUT /api/payment-memos/{id}` | `MemoFields` plus `expected_revision` | `200 Memo` |
| `POST /api/payment-memos/{id}/cancel` | `{expected_revision: string}` | `200 Memo` |
| `POST /api/payment-memos/{id}/reopen` | `{expected_revision: string}` | `200 Memo` |
| `POST /api/payment-memos/{id}/draft` | Payment candidate fields below | `201 {entry_id, entry_revision, memo_id, amount_minor}`; all values strings |

`MemoFields` has exactly six required properties:

```json
{"due_date":"2026-10-23","title":"Water payment","payee":"Supplier","note":"","amount_minor":"1200","necessity":"required"}
```

`title`, `payee`, and `note` are trimmed. Title must contain 1–200 characters;
payee allows 0–200 and note 0–2000. Title/payee reject control characters; note
permits only newline and tab among control characters. Amount is a positive
canonical decimal integer string within SQLite i64. Necessity is `required` or
`optional`. Unknown JSON fields, JSON numbers, padded integer strings, nonexistent
dates and reversed ranges are rejected. IDs and positive revisions also use
canonical decimal strings.

`Memo` contains `id`, `revision`, the six fields, `status`, `created_at`,
`updated_at`, `paid_amount_minor`, `remaining_minor`, and `payment_draft_id`.
Status is `planned` or `cancelled`; it is never writable as `paid`.
`payment_draft_id` is a string or null. Collection rows include cancelled plans,
ordered by due date and ID. `required_total_minor` sums remaining amounts of
planned, required memos within the explicit date range, with checked overflow.
It is a planning subtotal, not cash availability or an accounting liability.

Updates require the exact current revision and a planned memo. Cancel applies
only to planned memos; reopen applies only to cancelled memos. Successful changes
increment revision and append an immutable full snapshot. An active payment draft
or effective posted payment prevents editing/cancelling the plan. Delete the
editable candidate, or explicitly reverse posted payment and review, before
changing that plan. No memo deletion or manual paid endpoint exists.

Payment candidate request:

```json
{"expected_revision":"1","entry_date":"2026-10-23","debit_account_id":"12","payment_asset_id":"18","amount_minor":"1200"}
```

The caller explicitly chooses the date, distinct active posting-leaf accounts,
and positive amount no greater than remaining. The credit/payment account must
be an asset; no debit account is guessed from memo text. Admission takes the
SQLite writer lock, creates one balanced two-line draft and its immutable memo
link in one transaction. At most one editable candidate exists per memo. The
returned entry starts at revision `"1"`; creating it neither posts nor pays.

Posting uses the existing journal API. A database posting trigger verifies that
the two lines still exactly match the recorded amount/accounts, the memo remains
at its admitted revision and planned state, and cumulative effective payments
cannot exceed the plan. All posting entrances enforce this atomically. Amount
changes in the general journal require deleting this candidate and creating a
new explicit one. Posting a matching candidate makes its link count as payment;
reversing that posted entry removes its effective amount from the derived total
while preserving both journal and link history. Partial amounts are supported by
the API, and the remaining amount can produce a subsequent candidate. Plans are
not automatically matched to unrelated AP items or journal entries.

Domain errors: `400 payment_memo_invalid`, `404 payment_memo_not_found`,
`409 payment_memo_conflict`, and `422 payment_memo_overflow`. A changed payment
candidate rejected during posting also returns `409 payment_memo_conflict`.
Existing boundary/body-size/authentication errors still apply. No automatic
retry or idempotency key is provided: after an unknown write outcome, refresh
and reconcile before explicitly submitting again.

## Read-only routes

- `GET /api/health`
- `GET /api/accounts`
- `GET /api/accounts/tree`
- `GET /api/accounts/{account_id}`
- `GET /api/accounts/{account_id}/path`
- `GET /api/journal/drafts`
- `GET /api/journal/drafts/{entry_id}`
- `GET /api/journal/posted/{entry_id}`
- `GET /api/reports/trial-balance/{as_of}`
- `GET /api/reports/balance-sheet/{as_of}`
- `GET /api/reports/accounting-equation/{as_of}`
- `GET /api/reports/ledger/{account_id}/{from}/{through}`
- `GET /api/reports/general-journal/{from}/{through}`
- `GET /api/reports/income-expense/{from}/{through}`

## Success response schemas

Successful registered reads return `200`. The route tests lock the complete
current field sets:

- Health: `{service, status}`.
- Account: `{id, parent_id, name, element, kind, is_active, revision,
  created_at, updated_at}`. Account list and path responses are arrays of this
  object; a tree node is `{account, children}` recursively.
- Draft: `{id, entry_date, entry_time, revision, created_at, reference, explanation, lines}`.
  Posted entry: `{id, entry_date, entry_time, created_at, posted_at, reference, explanation,
  relationship, original_entry_id, lines}`. Relationship is `ordinary`,
  `reversal`, or `correction`; the original ID is nullable. Their line shape is
  `{id, line_no, account_id, side, amount_minor, memo}`.
- Trial balance: `{as_of, rows, total_debit_minor, total_credit_minor}`; each row
  is `{account_id, account_name, element, debit_turnover_minor,
  credit_turnover_minor, debit_balance_minor, credit_balance_minor}`.
- Accounting equation: `{as_of, rows, total_debit_minor, total_credit_minor}`;
  each row is `{element, debit_balance_minor, credit_balance_minor}`.
- Period ledger: `{account_id, account_name, element, from, through,
  opening_debit_balance_minor, opening_credit_balance_minor, lines,
  period_debit_turnover_minor, period_credit_turnover_minor,
  closing_debit_balance_minor, closing_credit_balance_minor}`. Each line is
  `{entry_id, line_id, entry_date, entry_time, line_no, side, amount_minor,
  debit_balance_minor, credit_balance_minor, reference, explanation, memo}`.
- General journal: `{from, through, entries, total_debit_minor,
  total_credit_minor}`. Each entry is `{id, entry_date, entry_time, posted_at, lines,
  debit_total_minor, credit_total_minor, reference, explanation}`; each line is
  `{id, line_no, account_id, account_name, side, amount_minor, memo}`.
- Income/expense: `{from, through, rows, total_income_minor,
  total_expense_minor, net_result_minor}`; each row is `{account_id,
  account_name, element, debit_turnover_minor, credit_turnover_minor,
  net_activity_minor}`.

The account list is ordered by stable account ID. The tree preserves that order
among roots and siblings, and each path is returned from root through the
requested account. Tree and path reads defensively revalidate hierarchy
structure; detected stored corruption becomes `internal_error`, not a partial
hierarchy.

Drafts are returned separately from posted entries and are ordered by entry
date then stable entry ID; their lines are ordered by one-based line number.
Draft responses include `revision`, while posted-entry responses do not expose
the internal terminal revision. A draft may be empty or unbalanced. These are
read routes only; journal editor interaction is separately approved in
`VERSION01_DECISIONS.md`, and UI implementation remains pending.

Report ranges are inclusive. Every report route derives from posted journal
lines and excludes drafts. Trial-balance and income/expense rows are ordered by
account ID. The accounting-equation response always contains the five elements
in `asset`, `liability`, `equity`, `income`, `expense` order. Period-ledger
movements and general-journal entries are ordered by accounting date and stable
identifiers. Report arithmetic is checked exact integer arithmetic; overflow,
stored corruption, or a reconciliation failure returns `internal_error`.

Path identifiers must be positive signed 64-bit integers. Zero, negative,
nonnumeric, and out-of-range values are rejected before repository access.
Ranges with `from` after `through` are rejected.

The health route probes its SQLite pool and fails closed with `500` if the
database connection is unavailable; it does not claim readiness from process
state alone.

The default read-only constructor does not register mutation methods.
`HEAD` is the only non-`GET` method
accepted on registered routes; it returns the corresponding `GET` status and
headers without a response body. Other methods return `405 Method Not Allowed`
with `Allow: GET, HEAD`. Unknown and near-miss paths return `route_not_found`
and do not advertise an `Allow` header.

## Error envelope

Failure bodies, when permitted by the HTTP method, are JSON objects containing
exactly the stable `code` and human-readable `message` fields. Current public
codes are:

- `invalid_date` (`400`)
- `invalid_id` (`400`)
- `invalid_date_range` (`400`)
- `invalid_path` (`400`) for a path that is not valid UTF-8
- `account_not_found` (`404`)
- `entry_not_found` (`404`) when no journal entry has the requested ID
- `draft_not_found` (`404`) when the entry exists but is not an editable draft
- `posted_entry_not_found` (`404`) when the entry exists but is not posted
- `route_not_found` (`404`)
- `method_not_allowed` (`405`)
- `internal_error` (`500`)

Database details are not exposed in the response body.

All responses disable storage with `Cache-Control: no-store` and
`Pragma: no-cache`, add `X-Content-Type-Options: nosniff`, and use
`Referrer-Policy: no-referrer`. Responses do not add validators such as `ETag`
or `Last-Modified`, and the router does not add permissive CORS headers.

## Balance sheet response

`GET /api/reports/balance-sheet/{as_of}` returns `{as_of, hierarchy, rows,
total_assets_minor, total_liabilities_minor, total_equity_minor,
unclosed_net_result_minor, total_liabilities_and_equity_minor}`. Hierarchy is
the ordinary flat account schema; each row is `{account_id, account_name,
element, balance_minor}`. Signed balances use the element's natural direction.
Unclosed results are separate, without inferred current/non-current labels or
generated closing entries. All monetary fields are exact strings.

## Explicit localhost write constructor

Only `api::localhost_write_router` enables these routes. It requires exactly
`Host: 127.0.0.1:3000` on reads and writes, and exactly
`Origin: http://127.0.0.1:3000` on writes. Any Origin on a read must match too.
Cross-site fetch metadata and duplicate Host/Origin headers are rejected.
This is a localhost policy, not yet the Tailscale HTTPS/identity policy.
The default read-only constructor and both test suites remain listener-free.

All requests below use `Content-Type: application/json`, canonical decimal
string integers, strict known fields, and a 1 MiB body limit.

| Method/path | Body | Success |
|---|---|---|
| `POST /api/accounts` | `{parent_id?, name, element, kind}` | 201 Account |
| `PATCH /api/accounts/{id}` | Tagged account change below | 200 Account |
| `DELETE /api/accounts/{id}` | `{expected_revision}` | 204 |
| `POST /api/journal/drafts` | Entry document below | 201 Draft |
| `PUT /api/journal/drafts/{id}` | `{expected_revision, entry}` | 200 Draft |
| `DELETE /api/journal/drafts/{id}` | `{expected_revision}` | 204 |
| `POST /api/journal/drafts/{id}/post` | `{expected_revision}` | 200 Posted |
| `POST /api/journal/posted` | Entry document | 201 Posted |
| `POST /api/journal/posted/{id}/reversal` | `{entry_date, entry_time?}` | 201 Posted |
| `POST /api/journal/posted/{id}/correction` | Entry document with nonblank explanation | 201 Posted |

Account changes use `operation` plus `expected_revision` and the indicated
field: `rename` (`name`), `move` (`parent_id`, explicitly null for root),
`set_active` (`is_active`), `change_kind` (`kind`), or `change_element`
(`element`). Missing `parent_id` on a move is an error.

Entry example:

```json
{
  "entry_date": "2026-10-22",
  "entry_time": "09:30:15",
  "reference": "Opening 001",
  "explanation": "Opening entry",
  "lines": [
    {"account_id": "1", "side": "debit", "amount_minor": "1000", "memo": null},
    {"account_id": "2", "side": "credit", "amount_minor": "1000", "memo": "opening equity"}
  ]
}
```

`entry_time` is optional local civil time, accepted as `HH:mm:ss` (00:00:00–23:59:59)
or null; omitted values remain compatible with date-only clients and legacy
entries. Empty strings, fractional seconds and timezone suffixes are rejected.
It is independent of `created_at`/`posted_at`, does not shift the accounting date,
and does not change date-based report filters or date/ID ordering. Draft create,
replacement, posting and correction preserve it; reversal may explicitly supply
its own optional time, otherwise null. Replacing a draft without it clears it.
Time changes follow the same draft revision and posted-entry immutability rules.

Optional reference, explanation, and memo fields accept null. A complete draft
replacement clears omitted optional metadata. Drafts may be incomplete or
unbalanced, but saved effective lines must have positive amounts and valid
active posting accounts. Posting repeats all mutable checks atomically.

Writes never update/delete posted entries. Reversal/correction create new
permanently linked history. Expected-revision conflicts return 409; accounting
rule failures return 422; malformed requests return 400, missing objects 404,
forbidden origins/hosts 403, oversized JSON 413, and missing JSON content type
415. Errors retain `{code, message}` and never expose SQL details.

Book-owned attachment metadata, verification, draft detach and manual backups
have the additional localhost routes documented below. Transfer orchestration and explicit process-local book selection are documented
below. Deployment remains an explicit local operation.

## Book-owned attachment and backup commands

The private runtime additionally merges `api::localhost_book_commands_router`
with the accounting router. These routes require the same exact loopback Host,
write Origin and Fetch Metadata checks, apply `no-store`, and return stable
`{code, message}` JSON errors. The process-held encryption key is used; handlers
do not call Keychain or accept local filesystem paths.

| Method and path | Input | Result |
| --- | --- | --- |
| `GET /api/journal/entries/{id}/attachments` | None | Attachment metadata array for an existing draft or posted entry |
| `GET /api/attachments/{id}` | None | Attachment metadata |
| `POST /api/attachments/{id}/verify` | None | `{id, verified: true}` after full ciphertext and plaintext authentication/hash checks |
| `DELETE /api/attachments/{id}` | `{expected_revision: "1"}` | 204 after atomic draft revision check and reference removal |
| `POST /api/backups` | Empty JSON object `{}` | 201 after independently verified encrypted managed backup and retention attempt |

Attachment metadata is `{id, entry_id, display_name, byte_length, sha256,
created_at}`. Integer fields are decimal strings. Metadata omits storage paths,
ciphertext internals and key material. Draft removal increments the draft
revision; stale saves return 409 `stale_draft`. Posted references return 409
`posted_attachment_immutable`. Failed authentication returns 409
`attachment_integrity_failed` without returning plaintext. Removing a reference
retains encrypted bytes until coordinated garbage collection permits reclamation.

Manual backups use only the configured book's private `backups/` directory and
its existing authenticated backup identity. The input does not select a path,
key, book, retention count, or overwrite behavior. An older book lacking that
identity returns 409 `backup_identity_missing`; HTTP does not silently initialize
or repair it. A busy maintenance lock returns 409 `backup_busy`. Unrecognized
backup content and symbolic links are preserved.

### Explicit backup-identity setup in selected sessions

The session runtime adds the following routes. Both require an explicitly selected
book, a mandatory matching `X-kAccounting-Selection` generation, and the existing
Host/Origin/Fetch Metadata boundary. Requests cannot select paths, namespaces or keys.

| Method and path | Input | Result |
| --- | --- | --- |
| `GET /api/backups/identity` | No body | `200 {initialized: boolean}` |
| `POST /api/backups/identity` | Exact empty JSON object `{}` | `201 {initialized: true}` |

Initialization is a separate explicit action, not a side effect of opening,
restoring or attempting a backup. It creates a fresh authenticated namespace
without replacing an existing identity or changing the device key. The selected
runtime's shared identity cache is updated only after publication succeeds; a
subsequent explicit manual backup in the same selected session can use it.
No backup or selection change is triggered automatically.

`GET` authenticates an existing identity using the process-held key. Only a truly
missing identity with no cached identity returns `initialized:false`; corrupted,
replaced or inconsistent identity returns `409 backup_identity_invalid`, not false.
`POST` refuses existing identity with `409 backup_identity_exists` and active
maintenance with `409 backup_busy`. Other setup failure is reported as
`backup_identity_initialization_failed`; preserve uncertain files and use the status
read to reconcile before another explicit action. Generation mismatch returns
`409 selection_changed` before setup. Neither route returns identity/key IDs or paths.

The owned initialization worker retains the selected-book lease through completion,
including requester disconnection. It cannot publish against a replacement selection.
Unknown response outcomes must not be blindly retried; the UI explicitly reads
status and retains its block when authentication or response validation fails.

Backup success is `{backup_name, verified: true, attachment_count,
attachment_bytes, retention_warning, preserved_unrecognized}`. Counts and sizes
are decimal strings. `backup_name` identifies a managed directory below the
configured backup root, without exposing an absolute path. Maintenance failure
returns the successful checkpoint with a non-null `retention_warning` and null
`preserved_unrecognized`; it never disguises a successful verified backup as a
failed checkpoint. When retention succeeds it preserves unknown content and
retains the newest ten authenticated checkpoints belonging to this book.

Streaming binary upload is registered on the entry attachment collection, and
`GET /api/attachments/{id}/content` provides an authenticated streaming download.
See [attachment streaming](ATTACHMENT_STREAMING.md) for required metadata,
draft revisions, posted append, limits, cancellation and error behavior. These
routes use bounded crypto bridges, not whole-file plaintext buffers or files.

Attachment IDs are random **32-character lowercase hexadecimal strings**, not
SQLite integer identifiers. The browser attachment UI lists metadata, explicitly
confirms original-`File` streaming upload, verifies integrity, and offers detach
only for saved drafts. Draft upload/detach refresh the draft revision before
later writes. Posted append creates a new immutable attachment reference; it
never replaces an existing one. Unconfirmed write outcomes preserve the selected
file and require explicit list/revision reconciliation before another mutation.

## Explicit book sessions and storage transfer (Round 2)

`serve-chooser <private-book-collection> <frontend-dist>` starts a session without
opening a book. Selection is never persisted. The existing exact loopback
Host/Origin and Fetch Metadata protections apply to all routes, including static
and unknown paths. Session control JSON is limited to 4096 bytes.

| Method and path | Input / behavior |
| --- | --- |
| `GET /api/session` | `{book_id, selection_required, selection_id}`; no paths or key IDs |
| `GET /api/session/books` | Available direct-child book IDs; discovery does not read keys |
| `POST /api/session/selection` | `{book_id}`; exact listed direct-child ID only |
| `DELETE /api/session/selection` | Explicitly clear the current selection |

Accounting/storage access requires explicit selection even with one candidate.
`selection_id` (2026-10-23) is null without selection, otherwise an opaque
32-character lowercase hexadecimal generation. Returning to the same book after
a switch creates a fresh generation; selecting the already active book preserves
it. This is concurrency metadata, not an authentication credential.

New browser data requests and select/close actions send
`X-kAccounting-Selection: <selection_id>` (or `unselected` for an empty session).
The guard is checked under the dispatch/transition lease before accounting
dispatch, key reads or selection changes. Mismatch returns 409 `selection_changed`;
malformed/duplicate guards return 400 `invalid_selection_guard`. Earlier integration
clients may omit it for compatibility; all new browser financial controls must
supply it. Existing Host/Origin/identity checks still apply.

Active response bodies and owned storage jobs prevent selection changes (409
`book_busy`); a failed replacement preserves the current book. Shutdown drains
responses/jobs before closing its book. Import/restore never switch the session.

Storage JSON accepts typed random process-local capabilities registered by a
trusted local adapter after genuine selection, never paths, keys or overwrite
instructions. There is deliberately no arbitrary HTTP capability-registration
endpoint. The explicit loopback browser-native action below invokes a genuine
panel; private ingress excludes it. Capabilities bind existing ancestor
identities; destination capabilities are consumed on admission. Public recipient
request content is captured at selection and must be confirmed before export.

| Method and path | Input / result |
| --- | --- |
| `POST /api/storage/recipient-requests` | `{destination}`; 202 job |
| `GET /api/storage/selections/{id}/request` | Public `{request_id, fingerprint}` preview |
| `POST /api/storage/exports` | `{book, request, destination, confirmed_request_id, confirmed_fingerprint}`; 202 job |
| `POST /api/storage/imports` | `{package, request, destination}` directory-package capabilities; 202 job |
| `POST /api/storage/restores` | `{backup, destination}`; 202 job |
| `POST /api/storage/recoveries` | `{kind:"imports", parent}` or `{kind:"exports", book, parent}`; 202 job |
| `GET /api/storage/operations/{id}` | Process-local state `running`, `succeeded` or `failed` |
| `GET /api/storage/operations/{id}/download` | Verified successful export only; encrypted single-file binary download |
| `POST /api/storage/container-imports?request=CAP&destination=CAP` | `application/octet-stream` encrypted body; 201 verified import result after completion |

Storage commands admit one owned job at a time with no unbounded queue. JSON
rejects unknown fields, operations survive requester disconnection, and errors
omit local paths and raw key/SQL failures. Counts and byte sizes are exact decimal
strings. Completed operation IDs do not survive restart; authenticated persisted
work determines recovery. The binary import uses bounded 64 KiB bridges, enforces
transport capacity and any declared Content-Length, then invokes full recipient
verification. Framing/hash verification alone never authorizes publication.
New-destination import clears its owned internal encrypted staging only after
fully verified success; failed/interrupted or uncertain work remains for
inspection. Original downloaded/external packages remain untouched. Import does
not select the new book. An incomplete/truncated body must not publish a book; disconnection after the
complete input has been accepted does not promise cancellation or rollback.

Download responses use `application/octet-stream`, `Cache-Control: no-store`,
`X-Content-Type-Options: nosniff` and
`Content-Disposition: attachment; filename="kAccounting-Extra-01.kagextra"`.
Only producer-captured package descriptors authorize download; arbitrary folder
registration and replaced export content do not. Safari policy is download to
Downloads first, followed by manual movement to external media. Browser/UI and
physical APFS/exFAT acceptance remain separate from HTTP-router tests.

See [storage capability contract](decisions/0043-storage-http-selection-capabilities.md),
[session contract](decisions/0046-explicit-http-session-runtime.md),
[container import](decisions/0050-container-import-pipeline.md),
[local runtime](LOCAL_RUNTIME.md), and
[round-two audit](RELEASE_AUDIT_R2_2026-10-22.md).

## Explicit loopback browser native actions

`NativeHost.browser_router()` adds `POST /api/native/selections` only to the
local browser composition. Its strict JSON body is `{kind}`, limited to 4096
bytes. Accepted kinds are `book`, `request`, `package`, `backup`,
`request-destination`, `export-destination`, `book-destination`, and
`recovery-parent`. No path, helper, owner UID, key, arbitrary capability ID, or
overwrite instruction is accepted from the browser.

Existing exact loopback Host/Origin and fetch-site restrictions apply. This
native action additionally requires a canonical `X-kAccounting-Selection`
header and an explicitly selected matching book. The native action guard holds
the generation lease through panel admission and capability registration.
Success is `{capability_id: "<32 lowercase hex>"}`; cancellation returns
`{capability_id: null}` without admitting a storage job. Only one panel is
admitted at a time. Startup supplies the trusted pinned helper; a genuine user
panel selection determines the capability.

The private Unix/HTTPS ingress uses `NativeHost.router()` and excludes this
panel endpoint. Native-panel availability is not evidence that permanent private
deployment, Keychain authorization, or login/reboot acceptance has passed.

## Native download generation guard

New browser-native downloads use exactly one query field
`download_selection=<32 lowercase hex selection generation>` on only:

- `GET /api/attachments/{32-lowercase-hex-attachment-id}/content`
- `GET /api/storage/operations/{32-lowercase-hex-operation-id}/download`

This narrow query alternative allows Safari's native download action, which
cannot add a custom header. It is checked under the same selected-book read
lease as dispatch; the leased streaming body continues to prevent switching
until it is consumed or dropped. A stale generation returns 409
`selection_changed` before financial dispatch. Malformed/duplicate/extra query
fields, an unsupported route/method, and a conflicting selection header return
400 `invalid_selection_guard`. Existing origin/fetch-site/identity protections
remain; the generation is concurrency metadata, not authentication.

The ordinary header route remains available to direct streaming clients. New
browser download links always carry the query guard and use same-origin exact
binary routes; uploads pass the original `File` directly with the header guard.
The UI never buffers an entire download as a Blob and reports only handoff to
Safari. Completion must be checked in Safari's download list/Downloads.

See [browser native actions ADR](decisions/0053-browser-native-actions.md) and
[storage UI acceptance checklist](STORAGE_UI_ACCEPTANCE_2026-10-23.md).
