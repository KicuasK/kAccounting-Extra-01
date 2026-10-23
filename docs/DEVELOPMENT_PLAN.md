# Development Plan

最新非 UI 增量：[私人代理版本升級與回復工具](PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md)；只完成隔離驗證，未套用真實管理員安裝，試用服務保留。

最新非 UI 工具：[安裝、啟停、更新與復原](PRIVATE_RUNTIME_OPERATIONS_2026-10-23.md) 已完成隔離測試；未套用系統安裝，含 root 工作控制與選填登入啟動設定，但實際安裝、私人連線及登入／重開機驗收仍待完成。試用服務保留。

本輪非 UI 工具與驗證結果：[單主機非 UI 增量](SINGLE_HOST_NON_UI_2026-10-23.md)。

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

## Current increment — 2026-10-23 (draft attachment and directory-package Safari acceptance)

Disposable Safari checks passed for draft attachment cancellation, pending-file
save protection, explicit upload confirmation, integrity verification, reference
removal and unsaved-edit protection. Directory-package import passed genuine
native selections and explicit confirmation without switching the current book.
Explicit reopening verified exact 9,007,199,254,740,993 posted totals and attachment
integrity; independent receipt checks confirmed source/package/request preservation,
fresh local encryption and complete posted-entry equality. See
[acceptance evidence](DRAFT_FOLDER_SAFARI_2026-10-23.md).
Actual complete verification: **684 Rust passed, 9 intentionally ignored;
141 Python and 185 frontend passed; zero failures**.
Only acceptance fixtures and documentation changed; production code, frontend
and lockfiles did not. Retain the verified candidate5 development package.
This increment's source and evidence belong to external
`deliverables/draft-folder-verification/DELIVERY.json`.
Formal signing remains paused. Permanent private deployment, real Keychain/
login/reboot and physical cross-Mac qualification remain open. Same-Mac fake-key
acceptance does not qualify those operations. Older pending draft-attachment and
directory-import statements below are superseded; remaining large-Safari-file
and stale-tab transfer variants still need their own acceptance.

## Previous increment — 2026-10-23 (partial payments, AR/AP splits and filters, retained history)

Explicit partial-payment amounts and full-source-line multi-item AR/AP splits
are implemented, with exact BigInt validation, type/status/keyword filters and
filter-scoped summaries. Payment creation remains a draft; separate successful
posting creates the payment link. Safari verified a 1,000 TWD plan paid as
400 + 600, AR 500 split into 300 + 200, AP 300 split into 100 + 200, filtering,
and a 75 TWD AP payment allocation. See
[implementation and acceptance](PARTIAL_SPLIT_UI_2026-10-23.md).
Actual complete verification: **684 Rust passed, 9 intentionally ignored;
141 Python and 185 frontend passed; locked Release succeeded**.
The new development package is candidate5; its exact source binding, hash and
logs belong to external `deliverables/partial-split-verification/DELIVERY.json`.
No new migration is introduced; preserve protected migration 8–10 handling.
Formal signing remains paused. Permanent private deployment, real Keychain/
login/reboot and physical cross-Mac qualification remain open. Credit-card,
TPASS, ageing reports and background notifications remain future work.
Older checkpoints below are retained history.

## Previous increment — 2026-10-23 (payment memos and AR/AP, retained history)

Payment plans, explicit payment-draft creation and separate posting, plus
append-only AR/AP source splits, partial payment/adjustment allocations and
reason-required unallocation are implemented. Reversals with active allocations
are blocked. See [implementation and evidence](PAYMENT_ARAP_2026-10-23.md).
Actual complete verification: **684 Rust passed, 9 intentionally ignored;
141 Python and 171 frontend passed; locked Release succeeded**.
The development package is candidate4; its exact source binding, hash and logs
belong to external `deliverables/payment-arap-verification/DELIVERY.json`.

Migrations 8–10 require the existing runtime's protected migration and migration
backup workflow. Preserve that workflow for existing books; do not hand-edit the
schema, skip the backup or remove migration records to simulate a downgrade.
An older binary must not read a book upgraded to the new schema.
Formal signing remains paused. Private deployment, real Keychain/login/reboot
and physical cross-Mac qualification remain separate acceptance work.

## Previous checkpoint — 2026-10-23 (UI4, retained history)

Source checkpoint `35d6d0f35aea2962f1322967959d559cf09d1294` completes explicit
backup-identity setup after legacy restore and authenticated import/export
recovery UI acceptance. Safari reopened the restored book and verified its
exact large-integer account balances before separately confirming setup and backup.
Protected source and unknown files survived recovery. Complete verification:
**669 Rust passed, 9 intentionally ignored; 141 Python and 141 frontend passed;
locked Release succeeded**. An additional Release 128 MiB SIGKILL recovery test
passed separately. See [UI4 evidence](BACKUP_IDENTITY_RECOVERY_2026-10-23.md).
The verified development package is candidate3; exact source binding, manifest
hash and logs are in external `deliverables/ui4-verification/DELIVERY.json`.
These results replace the older UI3 counts and pending recovery/restored-report
statements below; they are not added to historical test totals.

Next work, shortest independent checks first:

1. Close remaining disposable Safari acceptance gaps: native cancellation,
   directory-package import and representative large-attachment workflows.
   Preserve failed/unknown work and independently verify protected contents.
2. Keep development packaging, update/recovery instructions and disabled private
   deployment plans reviewable; rerun verification after changes.
3. Permanent private-proxy installation and real Keychain/login/reboot
   qualification require the appropriate operator authorization or Mac action.
   Physical cross-Mac acceptance needs another Mac. These remain unverified.

Formal signing is paused by the user; retain development packaging. Physical
APFS/exFAT media acceptance and touch-friendly iPad UI remain Version 02.
Account codes are removed, balances retain debit/credit direction, and unresolved
credit-card policies must not be guessed. Version 01 is not yet fully accepted.

## Historical checkpoints and original schedule

The remainder records earlier checkpoints and the original forecast. Its
UI-last gates, unbuilt-UI descriptions and original day estimates are historical,
not the current remaining-work list or a new delivery estimate.

Latest increment (2026-10-23): account maintenance, draft/posting/correction/reversal,
five reports, attachment management and eight storage workspaces are implemented.
Typed localhost native selection, first-book creation, generation-guarded native
downloads, original File-body streaming, full helper-app packaging and offline
deployment review verification are integrated. User-requested UI/non-UI parallel
work supersedes the historical UI-last gates below. Visible account codes remain
removed and balances retain explicit debit/credit direction.

Complete verification after the trusted native initial-directory improvement:
664 Rust passed/9 intentionally ignored, 141 Python and 132 frontend passed,
locked Release succeeded. Safari disposable backup, posted 62-byte attachment
upload/verify/download and native first-book creation without automatic selection
passed. A same-Mac fake-key request/export/download/single-file import/reopen chain
also passed with exact large-integer balances; physical cross-Mac, large attachments,
folder import and recovery live flows remain pending. Restore to a new destination
also passed without selection changes; opening restored-book reports remains
unverified. Signing, permanent
private deployment and real Keychain/login/reboot qualification remain open.
See [current implementation and evidence](BUILD_STORAGE_UI_2026-10-23.md).
Native-helper packages now require `serve-native-chooser` with the exact helper
argument; verification explicitly reports `legacy_native_startup` for old bundles.
Final package hashes and source binding live in external
`deliverables/ui3-verification/DELIVERY.json` to avoid self-referential manifests.
All older baseline/pending statements below are historical and do not mean the
implemented journal, report, attachment or storage UI is still unbuilt.

Status: active Version 01 completion plan, based on clean Git baseline
`78d2924`.

Latest 2026-10-22 evening integration: real private HTTPS through the reviewed
source-built root Tailscale daemon passed the full disposable chooser, write,
streaming attachment, encrypted backup and container export/import workflow.
The installed GUI remained sandbox-incompatible at both tested Unix locations;
do not reuse its failed transport. Temporary test services were stopped and the
original GUI stayed online. Explicit private/native host start commands,
shutdown admission protection and GitHub-style exact identity support are wired.
Permanent provisioning, production signed-app/login/Keychain qualification,
native operator workflows and Mac Safari acceptance remain separate; this is not
Version 01 or all non-UI completion. See
[latest integration checkpoint](PRIVATE_INTEGRATION_2026-10-22.md).

Latest increment: the [30-minute pre-UI session](BUILD_30MIN_2026-10-22.md)
adds undeployed root-peer Unix runtime ingress, disabled installation-readiness
plans, sanitized key-access diagnostics and native-selection epoch protection.
Actual non-root Unix header-forgery refusal passed with zero key operations.
Real private Tailscale/TLS positive, enabled login/locked-Keychain and complete
native GUI acceptance are not established; final full-suite evidence is being
integrated. The approved journal UI remains unimplemented pending remaining
pre-UI OS gates. External physical APFS/exFAT qualification remains Version 02,
with all implementations preserved.

Previous execution update: the second 45-minute, twelve-agent non-UI session on
2026-10-22 adds private-source/portable-ciphertext export staging, selected-path
identity checks, capability-only storage HTTP jobs, explicit HTTP book sessions,
single-file encrypted download/import, and cross-module fake-key acceptance.
Actual OS checks passed for two newly created dedicated Keychain items, retained
at the user's request; locked/login behavior was not tested. The APFS/exFAT
disk-image attempt was blocked by the OS, so physical-media acceptance remains
open. Full-suite/Release results are recorded only after the integrating run.
See [session evidence](BUILD_12_AGENTS_45MIN_2026-10-22.md) and
[release audit](RELEASE_AUDIT_R2_2026-10-22.md).

User scope update (2026-10-22): actual external-drive APFS/exFAT acceptance,
including removal/interruption/recovery, is deferred to Version 02 because no
external drive is available. Preserve all existing implementations, simulated
tests and opt-in media tooling. This is no longer a Version 01 or UI-start gate;
it does not claim physical-media verification has passed.

Before starting the UI phase, finish private HTTPS/Tailscale provenance and
login/locked-Keychain acceptance; connect
the trusted local selection adapter to the eventual operator interface. Preserve
the approved operation policies: choose a book every launch; imports/restores
leave the current book selected; Safari export downloads to Downloads before the
user manually moves it to external media. Do not re-ask these decisions.

## Delivery constraints

- The user delegated security and efficiency engineering decisions on
  2026-10-22. Ask for a user decision only when a choice changes accounting
  behavior, the supported workflow, or another user-visible product contract;
  record security and performance choices in ADRs with tests and evidence.
- Version 01 remaining work is time-boxed to at most 10 focused development
  days from baseline `78d2924`, with a target of 8 days and days 9–10 reserved
  for integration and acceptance fixes.
- Waiting for required user decisions is calendar delay, not development time.
  Any unresolved blocking decision moves the delivery date by the same amount.
- Finish non-UI domain, storage, API, and runtime behavior before implementing
  the remaining UI. UI implementation is the final product phase, not the
  driver of accounting or storage policy.
- High-volume attachments remain part of Version 01. They may not be deferred
  merely to fit the time-box.
- Version 01 targets Mac Safari. Touch-friendly iPad Safari is Version 02;
  iPhone remains out of scope.
- Multi-currency, CSV export, and TPASS are not part of Version 01.
- A new feature or policy beyond the confirmed Version 01 scope requires an
  explicit decision and a revised estimate; it must not silently consume the
  10-day completion budget.

## Original implementation baseline (before the latest session)

Implemented and verified:

- Exact integer money, balanced atomic posting, persistent versioned drafts,
  posted-entry immutability, opening entries, and active posting-leaf checks.
- Hierarchical account repository, cycle and cross-element protection,
  optimistic concurrency, and fail-closed database integrity checks.
- Posted-lines-only general journal, full and period ledger, trial balance,
  five-element summary, and income/expense read models.
- Read-only Axum routes with exact JSON integer strings and stable error
  envelopes; no network listener or mutation routes are enabled.
- Consistent SQLite snapshots, deterministic `.kagextra` manifests, strict
  package verification, private assembly, no-replace publication, persistent
  operation state, and conservative interrupted-export recovery.
- Versioned bounded-memory encrypted-object streaming with purpose-separated
  keys and rejection of wrong keys, tampering, reordering, duplication,
  truncation, malformed framing, and trailing data.
- Catalina-style read-only account-tree UI with frontend parsing, request
  lifecycle, accessibility, and source-safety tests.
- Full verification baseline: 364 Rust tests and 41 frontend tests, plus
  formatting, Clippy, documentation, type checking, and production build.

Still required for Version 01:

- Completed in this session: journal reference/explanation/line-memo metadata,
  approved account-list rules, and the as-of balance sheet.
- Attachment ownership, encrypted repository, lifecycle metadata, and database
  integration are implemented; physical reclamation and runtime/UI integration remain.
- Encrypted local database-plus-attachments backup and new-destination restore
  primitives are implemented; application commands, retention, migration backup,
  recipient-key export, and verified cross-Mac import remain.
- Explicit localhost account/journal mutation APIs are implemented in-process;
  listener, Tailscale origin/identity integration, and deployment remain.
- Account management, journal, report, attachment, backup, and transfer UI.
- Mac Safari end-to-end acceptance and operating documentation.

### Six-Codex integration session, 2026-10-22

The subsequent approximately 30-minute session added private runtime startup,
explicit maintenance commands, authenticated managed-backup identity/retention,
offline attachment reclamation with snapshot pins, attachment metadata/detach/
verification and manual backup HTTP commands, recipient request/HPKE envelopes,
and bounded-memory cross-device attachment re-encryption. These are integrated
and verified without enabling real financial data or the user's Keychain.

The remaining release gates are now complete encrypted package export/import,
attachment upload/download, backup-before-legacy-migration orchestration,
automatic active-book selection, private Tailscale identity/origin integration,
installation/login acceptance, the full write/report/storage UI, and Mac Safari
end-to-end acceptance. No remaining UI was implemented ahead of those non-UI
gates. See `PARALLEL_BUILD_2026-10-22.md` for validation and limitations.

## Approved Version 01 decisions

The user approved every recommendation in `VERSION01_DECISIONS.md` on
2026-10-22 and separately confirmed that application-managed encryption at
rest is required for live attachments, application-managed backups, and
`.kagextra`. ADRs 0023–0024 define the encrypted-object and passwordless
recipient-key designs; accounting and journal-domain work may proceed
independently. No application password or passphrase is permitted; cross-Mac
transfer uses a destination-generated public recipient request.

- Exact stored and displayed TWD minor-unit scale.
- General Journal header fields, reference numbering, explanation fields,
  account search, debit/credit input, autosave, and crash-recovery behavior.
- Reversal and correction rules, including inactive accounts and reversal
  relationships.
- Attachment ownership, allowed file types, size limits, retention,
  deduplication, and encryption requirements.
- Initial chart-of-accounts content and remaining account naming/order policy.
- Balance-sheet classification and whether period closing/locking is included.
- Backup frequency, retention, encryption, and restore-validation policy.
- Final macOS Application Support path, Tailscale activation timing, and
  whether application authentication is required in addition to Tailscale.

## Remaining execution order

### Step 1 — Close accounting and data contracts

Planned effort: days 1–2.

- Record the required decisions above.
- Completed: reversal and explicit correction domain behavior with invariant
  tests (ADR 0025).
- Finalize the Version 01 account, settings, attachment, and journal metadata
  contracts without depending on UI behavior.
- Add forward-only migrations and integrity checks for the accepted contracts.

Exit gate: every Version 01 accounting and attachment write has an approved
domain rule, schema, and automated invariant test.

Progress: reversal and explicit correction domain behavior, forward migration,
SQLite enforcement, integrity audit, and invariant tests are complete in ADR
0025. Journal/account metadata, attachment metadata/repository, and statement
contracts are now implemented in ADRs 0026–0029. Active-book settings and
application integration remain. See `BUILD_SESSION_2026-10-22.md` for the
current handoff.

### Step 2 — Complete attachments, backup, import, and transfer

Planned effort: days 3–5.

- Implement the local attachment repository and permanent database references.
- Select every referenced attachment automatically for snapshots and exports.
- Complete verified `.kagextra` import into new local storage without
  overwriting the only valid book.
- Complete consistent database-plus-attachments backup and restore.
- Encrypt live attachment objects, application-managed backups, and
  `.kagextra` payloads without publishing or retaining plaintext intermediate
  artifacts.
- Exercise capacity failure, interrupted removable-drive work, incompatible
  versions, destination conflicts, and restart recovery.

Exit gate: export, import, backup, and restore leave either the previous
complete state or a new verified complete state, with source data preserved.

### Step 3 — Add write APIs and the private runtime

Planned effort: day 6.

- Add structured mutation routes for accounts, drafts, posting, correction,
  attachments, backup, import, and export.
- Serve only on the approved localhost boundary and keep Funnel prohibited.
- Integrate the frontend bundle, final runtime data path, authentication
  decision, LaunchAgent, and Tailscale Serve configuration.
- Add API and runtime tests before exposing any mutation route.

Exit gate: every supported write is authorized by the domain layer, atomic
where required, returns actionable errors, and is reachable only through the
approved private boundary.

### Step 4 — Build the Version 01 UI last

Planned effort: days 7–9.

- Finish account creation, rename, move, activation, balance, and history UI.
- Implement the separately approved General Journal editor, drafts, balancing
  feedback, posting, reversal, and correction flows.
- Add journal, ledger, trial-balance, balance-sheet, and income/expense views.
- Add attachment, backup, export, import, and recovery status interfaces.
- Preserve the Catalina light Aqua direction, Mac keyboard workflow, exact
  amount display, and account-name mixed-script font rules.

Exit gate: one user can complete the full Version 01 workflow in Mac Safari
without database, terminal, or Finder intervention.

### Step 5 — Acceptance and release baseline

Planned effort: day 10, or days 9–10 when contingency is needed.

- Run the complete automated verifier and Mac Safari end-to-end scenarios.
- Test a large representative account tree and high-volume attachment set with
  isolated data, never real financial data.
- Verify create/open, account maintenance, draft, posting, correction, every
  report, backup, export, import, restore, restart, and failure recovery.
- Finish operating, backup, restore, update, and troubleshooting documentation.
- Commit a clean Version 01 release candidate; do not push or deploy without
  separate authorization.

Exit gate: all Version 01 acceptance criteria pass from a clean checkout and
the Git worktree is clean.

## Forecast

- Best case: 8 focused development days.
- Planned case: 10 focused development days.
- Calendar equivalent: about 1.5–2 working weeks if required decisions are
  answered immediately and the confirmed scope does not change.
- Main schedule risks: passwordless recipient-key and Keychain integration,
  removable-drive behavior on the target file system,
  authentication/Tailscale integration, and defects discovered during real
  Mac Safari acceptance.

This forecast is for the confirmed Version 01 scope only. It includes
high-volume attachments and private Mac operation, but excludes Version 02 and
later work below.

## Version 02

- Touch-friendly iPad Safari interface and iPad-specific package workflow.
- CSV export.

## Later phase — TPASS tracking

Status: confirmed future feature; not scheduled for Version 01.

- Resolve the policy and interaction questions in `TPASS_FUTURE_FEATURE.md`.
- Design TPASS purchase, pass-period, ride-evidence, and amortization models.
- Add invariant tests before or with implementation.
- Build TPASS and Journal integrations only after the General Journal
  interaction design is approved and Version 01 is complete.
- Validate with isolated test books; never modify real financial data.
