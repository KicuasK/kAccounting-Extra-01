# kAccounting-Extra-01

Private, single-user double-entry accounting software for a Mac Studio, with a Rust backend and a Safari-oriented web interface.

## Current status

The project has a buildable Rust/TypeScript foundation, a tested SQLite chart-of-accounts schema, backend-only versioned drafts and atomic posting using exact integer money, posted-lines-only journal/ledger/trial-balance/five-element/income-expense read models, a tested read-only Axum route tree with exact JSON integer strings, fail-closed startup integrity checks, and a verified no-overwrite database snapshot primitive. Version 01 is single-currency TWD and requires attachments plus application-managed `.kagextra` transfer between computers. Storage foundations now provide streaming SHA-256 descriptors, a strict deterministic `manifest.json`, fail-closed exact-layout package verification, owner-only private package assembly, transfer lifecycle rules, and conservative synchronized no-replace publication followed by public-package reverification. No production financial database, attachment repository, live-book export orchestration, network listener, journal UI, or deployment has been created.

## Planned stack

- Rust, Axum, Tokio, and SQLx
- SQLite
- TypeScript and Vite
- Mac Safari and iPad Safari
- macOS user LaunchAgent
- Tailscale Serve for private HTTPS access

## Product principles

- Enforce balanced double-entry posting in the backend.
- Preserve an immutable audit trail for posted entries.
- Calculate ledgers and reports from posted journal lines.
- Support a user-managed hierarchical chart of accounts.
- Keep the interface compact and consistent with the macOS Catalina light Aqua style.

## Documentation

- [Requirements](docs/REQUIREMENTS.md)
- [Accounting policy](docs/ACCOUNTING_POLICY.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Database schema](docs/DATABASE_SCHEMA.md)
- [Backend API](docs/API.md)
- [UI style guide](docs/UI_STYLE_GUIDE.md)
- [Development plan](docs/DEVELOPMENT_PLAN.md)
- [Open questions](docs/OPEN_QUESTIONS.md)
- [ADR 0001: account hierarchy](docs/decisions/0001-account-hierarchy.md)
- [ADR 0002: SQLite file safety](docs/decisions/0002-sqlite-file-safety.md)
- [ADR 0003: exact money and journal-line validation](docs/decisions/0003-exact-money-and-line-validation.md)
- [ADR 0004: consistent snapshot backup](docs/decisions/0004-consistent-snapshot-backup.md)
- [ADR 0005: editable drafts and account reactivation](docs/decisions/0005-editable-drafts-and-account-reactivation.md)
- [ADR 0006: read-only API and exact JSON integers](docs/decisions/0006-read-only-api-and-exact-json-integers.md)

## Safety

The service must listen on localhost by default. Remote access will use Tailscale Serve; public Tailscale Funnel is prohibited.

## Local verification

```sh
./scripts/verify.sh
```

The script rejects runtime databases, `.kagextra` transfer artifacts, and private storage work state inside the source tree; checks Rust formatting; treats every Clippy warning as an error; runs all backend tests using the locked dependency graph; and builds the production frontend bundle.

For backend-only work that must not build the UI bundle:

```sh
./scripts/verify-backend.sh
```

The backend-only verifier also rejects floating-point accounting types, public
bind or Funnel patterns, network-listener code, HTTP server features, and panic
shortcuts in production Rust. It also requires unsafe Rust to remain forbidden
and release integer-overflow checks to stay enabled while the bind and
authentication policy remains undecided.

