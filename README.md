# kAccounting-Extra-01

最新非 UI 增量：[私人代理版本升級與回復工具](docs/PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md)；只完成隔離驗證，未套用真實管理員安裝，試用服務保留。

最新非 UI 工具：[安裝、啟停、更新與復原](docs/PRIVATE_RUNTIME_OPERATIONS_2026-10-23.md) 已完成隔離測試；未套用系統安裝，含 root 工作控制與選填登入啟動設定，但實際安裝、私人連線及登入／重開機驗收仍待完成。試用服務保留。

本輪非 UI 工具與驗證結果：[單主機非 UI 增量](docs/SINGLE_HOST_NON_UI_2026-10-23.md)。

## Current deployment scope — 2026-10-23 (supersedes older gates)

The user selected **one Mac host with private remote-browser access**. The Mac
keeps the books and runs the backend; remote devices use authenticated private
HTTPS. Public exposure and Tailscale Funnel remain forbidden. Developer ID
signing and notarization are **out of scope**, rather than paused prerequisites.
Physical cross-Mac qualification is optional recovery/future work, not a gate for
this single-host deployment; encrypted backup and restore remain required.

Do not change UI code or run frontend/UI builds in this round. Candidate6 is the
latest verified development runtime, including the no-PR journal layout; reuse
its existing assets and external receipt
`deliverables/candidate6-verification/DELIVERY.json`. The user's candidate6-asset,
fake-data trial page is currently running: preserve its process and port, and do
not replace it with a deployment or stop it as test cleanup. It is not the
permanent service or real-Keychain acceptance.

Next, shortest independent non-UI work first:

1. Verify existing deployment-plan/package integrity and tool refusal cases with
   disposable data; retain the current package and lockfiles.
2. Complete reviewable installation/start/stop/update/recovery tooling and
   operator instructions, preserving existing binaries, data and unknown files.
   Preparation is not installation; test offline before any persistent action.
3. Run backend/tool verification and locked Release only; record the actual
   results separately from historical full-stack counts below.
4. Arrange private-proxy identity/configuration, real Keychain, persistent startup,
   logout/login and reboot acceptance only when the required operator action is
   concrete. Explain that action then; no permanent proxy or login service is
   claimed installed or qualified by these preparatory checks.

Older UI/cross-Mac/signing gates and time estimates below are retained history;
this scope takes precedence. The remaining operational checks are still open.

Private, single-user double-entry accounting software for a Mac Studio, with a Rust backend and a Safari-oriented web interface.

## Current status

Latest journal layout: Safari verified the date/account-narrative/debit/credit structure without PR, explicit draft save and posted text display. See [journal layout acceptance](docs/JOURNAL_LAYOUT_SAFARI_2026-10-23.md). The new candidate6 development package includes this frontend change; package integrity, 16 packaging tests and packaged startup/asset delivery passed. Candidate5 is preserved. See `deliverables/candidate6-verification/DELIVERY.json` for the external digest and receipt.

Previous increment (2026-10-23): disposable Safari acceptance passed for draft
attachment cancellation, upload, verification, removal and unsaved-edit protection,
and native directory-package import followed by explicit reopening. Imported
posted amounts remain exactly 9,007,199,254,740,993; attachment verification and
independent source/package/request preservation checks passed. See
[draft attachment and directory-import evidence](docs/DRAFT_FOLDER_SAFARI_2026-10-23.md).
Complete verification: **684 backend passed (9 intentionally ignored), 141 tool
and 185 frontend passed**. Production code, frontend and lockfiles are unchanged;
retain the verified candidate5 development package. This increment's source and
evidence are recorded externally in `deliverables/draft-folder-verification/DELIVERY.json`.
Formal signing remains paused. Permanent private deployment, real Keychain/
login/reboot and physical cross-Mac qualification remain open; this same-Mac
fake-key acceptance does not qualify them. Older checkpoints below are history.

Previous increment (2026-10-23): explicit partial-payment input for payment memos,
plus multi-item AR/AP source-line splits and type/status/keyword filtering are
implemented. Amounts remain exact integers; payment links appear only after
separate atomic posting. Safari verified a 1,000 TWD plan paid as 400 + 600,
AR 500 split into 300 + 200, AP 300 split into 100 + 200, filtering, and a
75 TWD AP payment allocation. Complete verification: **684 backend passed
(9 intentionally ignored), 141 tool and 185 frontend passed; locked Release
succeeded**. See [partial-payment/split evidence](docs/PARTIAL_SPLIT_UI_2026-10-23.md).
The new development package is candidate5; exact source binding, manifest hash
and logs belong to external `deliverables/partial-split-verification/DELIVERY.json`.
Formal signing remains paused. Permanent private deployment, real Keychain/
login/reboot and physical cross-Mac qualification remain open. Credit-card,
TPASS, ageing reports and background notifications are not completed by this increment.

Previous increment (2026-10-23): payment reminders with explicit journal drafts and
posted payment links, plus explicitly tracked AR/AP items, partial allocations,
reasoned allocation release and reversal protection are implemented.
**684 backend tests passed (9 intentionally ignored), 141 tool tests, 171 frontend
tests, and locked Release passed**. Safari exercised reminder creation → draft →
explicit posting → paid balance, and exact large-integer AR registration,
allocation and historical release. See [payment/ARAP evidence](docs/PAYMENT_ARAP_2026-10-23.md).
Final source/package binding is external `deliverables/payment-arap-verification/DELIVERY.json`.

Previous increment (2026-10-23): **669 backend tests passed (9 intentionally ignored),
141 tool tests, 141 frontend tests, and locked Release passed**. Safari verified
legacy-restored book reopening, explicit backup setup followed by a separately
confirmed backup in the same selected book, and authenticated import/export
recovery with protected source/unknown files preserved. A separate Release
128 MiB SIGKILL recovery test passed. See
[backup setup and recovery evidence](docs/BACKUP_IDENTITY_RECOVERY_2026-10-23.md).
Final source/package binding uses external `deliverables/ui4-verification/DELIVERY.json`.
Formal signing, permanent deployment, real Keychain and physical cross-Mac
qualification remain open. The preceding UI3 checkpoint below is historical.

**Changing development computers:** start with
[新電腦開始](docs/START_ON_NEW_MAC.md), then
[完整換機交接](docs/HANDOFF_NEXT_MAC.md) and
[另一個對話的 UI 設計、13 張原圖與待決事項](docs/UI_DESIGN_HANDOFF_2026-10-22.md).
All design images are included in this repository; no local chat access is needed.

Previous UI3 checkpoint, superseded by UI4 above (2026-10-23): **664 backend tests passed (9 intentionally ignored),
141 tool tests, 132 frontend tests, and locked Release build passed**. Final
release-candidate verification and package/source binding are recorded externally
in `deliverables/ui3-verification/DELIVERY.json`. See
[attachment, storage and native UI build](docs/BUILD_STORAGE_UI_2026-10-23.md) and
[preceding journal/report build](docs/BUILD_UI_WORKSPACES_2026-10-23.md).
Temporary authenticated private HTTPS
and four native-panel cases passed; permanent compatible private-proxy installation,
signed production/Keychain/login qualification and full product acceptance remain open.
The account browser now implements the selected 02 compact layout with explicit
book selection, as-of debit/credit balances, search/filter and bottom summary.
Only category labels use color. The user removed account codes on 2026-10-23.
Account maintenance, explicit draft editing/posting, immutable corrections/reversals
and five report workspaces are implemented with disposable Safari acceptance.
The frontend build targets Safari 14; the 13 mockups remain design references.
Attachment and eight storage workflows, localhost-only typed native selection,
first-book creation and guarded native downloads are implemented. Disposable
Safari backup, posted attachment upload/verify/download and native first-book
creation without automatic selection passed. A same-Mac, fake-key recipient request,
export, Safari download, single-file import and explicit reopening chain passed,
preserving the exact 9,007,199,254,740,993 trial balance. Physical cross-Mac,
large-attachment transfer, directory-package import and recovery live acceptance
remain open. Native backup restore also passed without switching books; opening
the restored book and checking its reports remains unverified.

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

**This is an integration preview, not a Version 01 release.** Remaining work includes
private Tailscale integration,
installation/login acceptance and complete Mac Safari/product acceptance.
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
- Keep the latest 02 compact macOS layout, black/white/gray except category colors.

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

