# Backend API

Status: read and explicit localhost write route foundations; not served or deployed.

The backend constructs a default read-only router and an explicitly enabled
localhost write router for in-process verification. It
does not bind a TCP listener, choose a runtime database path, configure
the approved private runtime, or enable Tailscale. Runtime and Tailscale
origin/identity integration remain before serving.

Axum is compiled with default features disabled and only its JSON feature
enabled. HTTP/1 and HTTP/2 serving support are therefore not enabled by this
foundation.

## Wire conventions

Every SQLite `INTEGER` identifier, revision, line number, and monetary amount is
encoded as a base-10 JSON string. Safari and other JavaScript clients therefore
cannot silently round values above JavaScript's exact integer range. Boolean
activation state remains a JSON boolean, and absent parent identifiers remain
JSON `null`. The current response models emit no JSON number values.

Dates are strings containing validated, existing `YYYY-MM-DD` calendar dates.
UTC database timestamps are returned as stored strings. Enum values use the
lowercase database vocabulary documented in `DATABASE_SCHEMA.md`.

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
- Draft: `{id, entry_date, revision, created_at, reference, explanation, lines}`.
  Posted entry: `{id, entry_date, created_at, posted_at, reference, explanation,
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
  `{entry_id, line_id, entry_date, line_no, side, amount_minor,
  debit_balance_minor, credit_balance_minor, reference, explanation, memo}`.
- General journal: `{from, through, entries, total_debit_minor,
  total_credit_minor}`. Each entry is `{id, entry_date, posted_at, lines,
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
read routes only and do not constitute an approved journal-entry editor.

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

Mutation methods are not registered. `HEAD` is the only non-`GET` method
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
| `POST /api/journal/posted/{id}/reversal` | `{entry_date}` | 201 Posted |
| `POST /api/journal/posted/{id}/correction` | Entry document with nonblank explanation | 201 Posted |

Account changes use `operation` plus `expected_revision` and the indicated
field: `rename` (`name`), `move` (`parent_id`, explicitly null for root),
`set_active` (`is_active`), `change_kind` (`kind`), or `change_element`
(`element`). Missing `parent_id` on a move is an error.

Entry example:

```json
{
  "entry_date": "2026-10-22",
  "reference": "Opening 001",
  "explanation": "Opening entry",
  "lines": [
    {"account_id": "1", "side": "debit", "amount_minor": "1000", "memo": null},
    {"account_id": "2", "side": "credit", "amount_minor": "1000", "memo": "opening equity"}
  ]
}
```

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
have the additional localhost routes documented below. Transfer, active-book
switching and deployment commands remain outside the HTTP API.

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

Backup success is `{backup_name, verified: true, attachment_count,
attachment_bytes, retention_warning, preserved_unrecognized}`. Counts and sizes
are decimal strings. `backup_name` identifies a managed directory below the
configured backup root, without exposing an absolute path. Maintenance failure
returns the successful checkpoint with a non-null `retention_warning` and null
`preserved_unrecognized`; it never disguises a successful verified backup as a
failed checkpoint. When retention succeeds it preserves unknown content and
retains the newest ten authenticated checkpoints belonging to this book.

Streaming attachment upload and plaintext download, backup restore, encrypted
recipient transfer, and active-book switching are not implemented as HTTP
commands. Upload must first gain a bounded stream-to-encryption pipeline;
250 MiB plaintext buffering or plaintext temporary files are not substitutes.
Existing attachment import remains a local repository operation.
