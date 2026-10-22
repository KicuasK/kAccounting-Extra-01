# Backend API

Status: read-only route foundation; not served or deployed.

The backend currently constructs an Axum router for in-process verification. It
does not bind a TCP listener, choose a runtime database path, configure
authentication, or enable Tailscale. Those deployment decisions remain required
before any route may be served.

Axum is compiled with default features disabled and only its JSON feature
enabled. HTTP/1 and HTTP/2 serving support are therefore not enabled by this
foundation.

## Exact integer encoding

Every SQLite `INTEGER` identifier, revision, line number, and monetary amount is
encoded as a base-10 JSON string. Safari and other JavaScript clients therefore
cannot silently round values above JavaScript's exact integer range. Boolean
activation state remains a JSON boolean.

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
- `GET /api/reports/accounting-equation/{as_of}`
- `GET /api/reports/ledger/{account_id}/{from}/{through}`
- `GET /api/reports/general-journal/{from}/{through}`
- `GET /api/reports/income-expense/{from}/{through}`

Dates use validated `YYYY-MM-DD` calendar values. The period-ledger range is
inclusive. The trial balance and ledger derive only from posted journal lines;
the draft route returns drafts separately. Path identifiers must be positive
signed 64-bit integers; zero, negative, nonnumeric, and out-of-range values are
rejected before repository access.

The health route probes its SQLite pool and fails closed with `500` if the
database connection is unavailable; it does not claim readiness from process
state alone.

Mutation methods are not registered. Axum responds with `405 Method Not
Allowed` when a caller tries to use `POST`, `PUT`, `PATCH`, or `DELETE` on any
registered read-only path.

## Error envelope

Input and lookup failures return JSON with stable `code` and human-readable
`message` fields. Current public codes are:

- `invalid_date` (`400`)
- `invalid_id` (`400`)
- `invalid_date_range` (`400`)
- `account_not_found` (`404`)
- `entry_not_found`, `draft_not_found`, or `posted_entry_not_found` (`404`)
- `route_not_found` (`404`)
- `method_not_allowed` (`405`)
- `internal_error` (`500`)

Database details are not exposed in the response body.

All responses disable storage with `Cache-Control: no-store` and
`Pragma: no-cache`, add `X-Content-Type-Options: nosniff`, and use
`Referrer-Policy: no-referrer`. The router does not add permissive CORS headers.
