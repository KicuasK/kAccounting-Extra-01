# One-hour development session — 2026-10-22

Project: `kAccounting-Extra-01`.
Start: 13:49 Asia/Taipei. Requested time-box: approximately one hour.

## Delivered in this worktree

- Forward-only migrations `0005`–`0007`: journal metadata, remaining approved
  account naming/activation rules, and permanent attachment ownership/integrity.
- Reference, explanation, and line memo preservation through draft save,
  replacement, posting, API reads, journal, and ledger reports.
- Exact sibling-name uniqueness, ancestor-first activation, descendant-first
  deactivation, and defensive hierarchy validation.
- Posted-lines-only as-of balance sheet with explicit hierarchy, contra balances,
  separate unclosed result, and exact reconciliation.
- Explicit localhost account/journal mutation API with revision checks,
  correction relationships, canonical integer strings, strict JSON, body limits,
  Host/Origin protection, and complete in-process workflow tests.
- Encrypted immutable attachment repository with regular-file validation,
  active ownership, exact file/book quotas, ciphertext publication, and full
  authentication/content verification.
- Create-only/read-only device-key contract, compiled macOS Keychain adapter,
  and fake-store tests for missing, unavailable, malformed, and duplicate keys.
- Encrypted local database-plus-attachments backup and new-destination restore.
  Selection comes from a consistent database snapshot, not a manual file list.
  Private and published results are reverified; existing destinations remain
  intact. Temporary plaintext is private and removed on completion/failure.
- Marked, locked private backup work recovery that skips live workers and
  preserves unrecognized paths for diagnosis.
- Updated source/runtime separation checks and design/API documentation.

## Validation record

See the final session update below for completed commands and totals. Fixtures
use in-memory databases or random disposable temporary directories, generated
keys, and invented accounting entries. No real financial book or user Keychain
item was opened or changed. No listener, Tailscale configuration, LaunchAgent,
public service, push, or deployment was enabled.

## Remaining before everyday use

1. Wire active-book/key binding, startup recovery, real Keychain acceptance,
   backup-before-migration/import, and newest-10 local backup retention.
2. Coordinate snapshot pins and physical ciphertext reclamation. Removed draft
   references currently retain encrypted bytes, so physical disk use may exceed
   the 20 GiB referenced-content quota.
3. Finish destination public-recipient requests, HPKE wrapping, encrypted
   `.kagextra` export, and verified cross-Mac import.
4. Implement the approved localhost runtime and Tailscale identity/origin
   integration, then deployment.
5. After non-UI behavior is complete, implement the approved account/journal,
   report, attachment, backup/transfer interfaces and Mac Safari acceptance.

The current frontend still shows the read-only account tree. This is a tested
backend completion increment, not a claim that Version 01 is ready for real
financial use. The original handwritten account hierarchy and category meaning
questions remain documented in `OPEN_QUESTIONS.md`; this session does not seed
or invent them.

## Change map

| Area | Files |
|---|---|
| Accounting and reports | `backend/src/accounting/{accounts,journal,reports}.rs` |
| HTTP contracts | `backend/src/api.rs`, `backend/src/api/mutations.rs` |
| Encrypted storage | `backend/src/{attachments,book_backup,keys,crypto,storage}.rs` |
| Database contracts | `backend/src/db.rs`, migrations `0005`–`0007` |
| Tests | new account-policy, journal-metadata, balance-sheet, API-mutation, attachment, and book-backup suites; existing schema/fixture updates |
| Dependencies | `backend/Cargo.toml`, `Cargo.lock`; macOS Security/Core Foundation only |
| Verification/docs | `.gitignore`, `scripts/verify-backend.sh`, README, architecture/schema/API/development plan, ADRs `0026`–`0029` |
| Entrypoint status | `backend/src/main.rs` now accurately explains that runtime integration is unfinished rather than asking again for an already approved policy |

Git changes are intentionally left uncommitted for review; the branch is `main`.

## Final verification update

Verified around 14:47 Asia/Taipei, following approximately one hour of focused
implementation and checks from baseline commit `2e6930e`.

| Check | Result |
|---|---|
| `./scripts/verify.sh` | Passed: formatting, Clippy with warnings denied, production panic-shortcut lints, 420 backend tests, documentation checks, 41 frontend tests, TypeScript, and production Vite build |
| Default capacity-test handling | One 250 MiB acceptance test is intentionally excluded from routine runs; explicitly executed below |
| Release tests | Full release suite plus final affected storage/crypto/key and backup regression suites passed |
| `cargo test --release --test attachments maximum_size_attachment --locked -- --ignored` | Passed with an actual 250 MiB source file, complete encryption and verification, and no plaintext attachment intermediate |
| `cargo build --release --locked` | Passed; release backend executable builds |
| `cargo check --workspace --all-targets --locked` | Passed after the final entrypoint status correction |
| `git diff --check` | Passed |
| Documentation relative links | 41 Markdown files checked, no missing relative targets |

The release executable still reports that private runtime startup is not yet
implemented. A successful build is not presented as a running accounting app.
No Mac Safari end-to-end acceptance is claimed, because the recording UI and
listener were deliberately not enabled ahead of their remaining integration.

Git: `main`, changes present and uncommitted; no push or deployment. Real
financial data and the actual user Keychain were untouched.
