# kAccounting-Extra-01

Private, single-user double-entry accounting software for a Mac Studio, with a Rust backend and a Safari-oriented web interface.

## Current status

**New Mac / developer handoff:** start with
[換電腦建置與開發交接](docs/HANDOFF_NEW_MAC.md) for clone/setup commands,
verified tool versions, current limitations, and the next implementation steps.

Latest non-UI increment: [30-minute pre-UI integration](docs/BUILD_30MIN_2026-10-22.md)
adds undeployed root-peer Unix ingress, disabled installation-readiness plans,
safe Keychain diagnostics and native-selection epoch protection. Actual non-root
Unix forgery refusal passed; real private Tailscale/TLS positive, enabled login
and complete native GUI acceptance remain open. Final integrated verification
for that increment is recorded in its report.

The project has a tested Rust/TypeScript accounting foundation with exact TWD
integers, balanced atomic posting, versioned explicit drafts, immutable
reversal/correction relationships, journal references/explanations/line memos,
and posted-lines-only journal, ledgers, trial balance, balance sheet, and
income/expense reports. Accounts enforce hierarchy, exact sibling-name
uniqueness, activation order, and optimistic concurrency.

The encrypted attachment repository enforces ownership, immutable posted
attachments, file/book quotas, authenticated streaming, and independent content
verification. Local device-key backup/restore primitives automatically select
attachments from consistent database snapshots and publish verified results
without replacement. A compiled macOS Keychain adapter and tested key-storage
boundary never introduce an application password. Encrypted `.kagextra`
export/import now connects strict public recipient requests, RFC 9180 HPKE,
full database/attachment verification, fresh local attachment encryption and
non-overwriting publication. Export restart recovery verifies persisted work;
import restart handling and its remaining limits are described in
[recipient transfer](docs/RECIPIENT_TRANSFER.md).

The private runtime now supports explicit new-book initialization and opening
an existing verified book with its existing Keychain key. It serves the frontend
and guarded write APIs only at `127.0.0.1:3000`, holds an exclusive book lock,
and fails closed on missing keys. Recognized older schemas receive an
independently verified encrypted checkpoint before migration; damaged databases
can be restored from those checkpoints into new destinations. Managed backups have
authenticated per-book identity and conservative newest-ten retention.

The current Catalina-style account-tree UI remains read-only. **This is an
integration preview, not a Version 01 release.** Remaining work includes
private Tailscale integration,
installation/login acceptance, write UI, and complete Mac Safari acceptance.
Physical APFS/exFAT external-drive acceptance is deferred to Version 02 by the
user (2026-10-22); all existing code, tests and media tooling are preserved.
Attachment upload/download uses bounded HTTP streaming. Each startup requires
an explicit book choice; import and restore never select a book automatically.
Development tests use
disposable books and fake key stores. No service is started automatically.

The second 45-minute session adds explicit HTTP book sessions, capability-only
storage jobs, ciphertext-only portable export staging, bounded single-file
transfer download/import, and independent cross-module acceptance. Two newly
created dedicated Keychain items passed actual production-adapter checks and
were retained as requested; locked-Keychain and login behavior remain untested.
No physical APFS/exFAT pass is claimed. See the
[twelve-agent session report](docs/BUILD_12_AGENTS_45MIN_2026-10-22.md) and
[independent release audit](docs/RELEASE_AUDIT_R2_2026-10-22.md).

See [private local runtime instructions](docs/LOCAL_RUNTIME.md) for explicit
commands and the launcher generator, which prepares a plist without installing
or starting it.

## Planned stack

- Rust, Axum, Tokio, and SQLx
- SQLite
- TypeScript and Vite
- Mac Safari for Version 01; touch-friendly iPad Safari for Version 02
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
- [Version 01 decision gate](docs/VERSION01_DECISIONS.md)
- [Open questions](docs/OPEN_QUESTIONS.md)
- [One-hour development handoff](docs/BUILD_SESSION_2026-10-22.md)
- [Private local runtime](docs/LOCAL_RUNTIME.md)
- [Six-Codex integration handoff](docs/PARALLEL_BUILD_2026-10-22.md)
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
bind or Funnel patterns, network startup outside the tested runtime module,
HTTP/2 features, and panic shortcuts in production Rust. It requires runtime
boundary tests, forbidden unsafe Rust, and release integer-overflow checks.

