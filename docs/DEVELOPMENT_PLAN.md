# Development Plan

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
