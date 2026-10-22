# Non-UI installation and OS acceptance — ongoing checkpoint

Date: 2026-10-22. Checkpoint: **20:08 Asia/Taipei**.
Requested round: **19:49:19–20:34:19 Asia/Taipei**
(**11:49:19–12:34:19 UTC**, 45 minutes).

**This round is still in progress. This is a saved checkpoint, not a claim that
45 minutes have elapsed or that the round, Version 01, or all non-UI acceptance
is complete.** The user permits a reboot after work is saved; this checkpoint
does not initiate one or imply that one occurred.

Project: `~/Desktop/kAccounting-Extra-01`, branch `main`, baseline
`afba3de66661876e77c2e502f3b47dd71e2838ea`. Existing uncommitted development from
prior rounds remains preserved. No commit or push is part of this checkpoint.

## Verified increments so far

| Area | Actual evidence | Scope and limit |
| --- | --- | --- |
| Transient background startup | Coordinating run reports `passed: true`, `own_job_removed: true`, `fake_key_calls: 0`, `collection_empty: true`, `selected_book: false`. | Only the unique temporary acceptance job was bootstrapped and removed. No permanent login entry, production job or existing book was used. |
| Isolated real OS Keychain | Corrected ASCII-credential harness passed create/read equality, duplicate refusal, verified locking, locked-read denial, explicit unlock and subsequent read equality. | New isolated fixture only. Locked read returned -128 with helper interaction disabled and no secret bytes. This is framework cancellation, not evidence that a person clicked Cancel. |
| Keychain preferences preservation | Public receipt records identical before/after default/search-list SHA-256 fingerprints and `preferences_unchanged: true`. | No automatic preference repair, default-keychain change or search-list change was performed. Fixture credential remained memory-only. |
| Tailscale setup | Official Tailscale 1.104.2 installation verified; after full quit/reopen the GUI explicitly shows System Extension Granted and VPN Configuration Granted. | Earlier OS/CLI checks showed the exact state `[terminating for disable]` while the UI was ON, plus CLI Error 1. Even after the granted-permission GUI state, the login button still does nothing; login remains blocked. No Serve configuration or private HTTPS acceptance is complete. |

### Transient startup evidence

The tool is `scripts/login-acceptance.py` with the reject-only fake-key
`backend/examples/login_acceptance.rs`. Default execution prepares a reviewable
plan only; the separately authorized `--run` was executed by the coordinating
worker. It verifies exact job label/program/plist/arguments, a unique frontend
fixture token, an empty collection and an unselected HTTP session. Cleanup refuses
unverified ownership and acts only on the exact generated test target.

Retained fixture:

`/private/var/folders/q8/z5ntm6zj4vg6crvpdc8l7ls40000gn/T/ka-login-acceptance-6e3baf3b21cf3186c74020f0d8f8348e`

The startup and shutdown log receipts both record zero fake-key calls for token
`6e3baf3b21cf3186c74020f0d8f8348e`. The normal installation-plan generator still
produces `Disabled: true`; this run does not enable a persistent login service.
Real logout/login or reboot startup has not been exercised by this test.

### Isolated Keychain evidence and first-attempt distinction

Retained successful public receipt:

`/tmp/ka-isolated-keychain-c0e81dad-fc66-409f-945d-f498a7e3d98d/public-receipt.json`

Its recorded status is
`isolated_create_read_duplicate_lock_unlock_read_passed`, with
`locked_read_tested: true`, `locked_read_status: -128`,
`locked_read_denial_kind: framework_cancelled_with_helper_ui_disabled`,
`preferences_unchanged: true`, `secret_persisted: false`, and
`login_logout_tested: false`.

The **first live attempt stopped before unlock and was incomplete**. It is not
retroactively counted as passing. The corrected ASCII-credential harness produced
the separate successful result above. No secret is included in the report,
arguments or public receipt, and the newly created fixture is retained. Existing
financial key items, login/System keychains and real books were not queried.

## Changes and focused verification

- New transient harness and fake-key example: `scripts/login-acceptance.py`,
  `scripts/test_login_acceptance.py`, `backend/examples/login_acceptance.rs`.
- Isolated OS fixture and argument-boundary checks:
  `native/IsolatedKeychainAcceptance.swift`,
  `scripts/test_keychain_fixture_harness.py`.
- Native workflow validation and expanded acceptance modes: existing native
  selection coverage now has **11 tests passed**; the example supports
  `--mode recovery-parent|request-destination|book-destination` and
  `--cancel-expected`. No actual save-panel or cancellation operation has yet been
  performed in this round; those modes are preparation, not live acceptance.
- Readiness evidence: `docs/INSTALLATION_READINESS.md`,
  `docs/KEYCHAIN_READINESS.md`,
  `docs/PRIVATE_ACCESS.md` and this checkpoint.

Focused transient-harness verification passed **14 mocked Python tests**;
strict Clippy for the fake-key example, syntax compilation and diff checks passed.
These are separate from the actual transient launchd and isolated Keychain
receipts. Current-round `cargo build --release --locked` passed; coordinating
log: `/private/tmp/ka-r4-release.log` (reported duration 0.87 seconds). Whole-tree
verification subsequently completed successfully before the restart handoff:
**630 Rust tests passed, 0 failed, 9 ignored; 56 Python tooling tests passed;
41 frontend tests passed; TypeScript checking and production frontend build
passed.** Log: `/private/tmp/ka-r4-verify.log`. These are this round's actual
results; the ignored capacity/process-death cases were not rerun in this check.
`git diff --check` passed. Git remains `main` at the baseline above, with
133 status entries (29 tracked modified, 104 untracked entries), including
preserved prior-round work. No commit or push was made.

## Remaining work and restart boundary

1. Save and review the current work before the user-authorized restart. No restart
   is initiated by this document.
2. After restart, recheck the extension/CLI state. Complete Tailscale login and
   verify the required real root-proxy transport/header behavior before any
   private Serve configuration or positive HTTPS acceptance. Do not enable public
   service or Funnel.
3. Current-round full regression and Release passed as recorded above.
   Do not replace this ongoing checkpoint with a completed-round claim early.
4. Keep true logout/login startup, production signed-app/Keychain access behavior,
   broader native operation acceptance and remaining private-access gates explicit.
   Isolated locked/unlocked success does not establish those separate cases.
5. UI and full Mac Safari end-to-end acceptance remain later work. Physical
   APFS/exFAT external-drive qualification was deferred by the user to Version 02;
   no physical-media pass is claimed here.

Existing policies remain: explicit book choice on every launch; no automatic
switch after import/restore; no replacement-key fallback; exact integer money,
atomic balanced posting, posted immutability and complete audit history. No
ambiguous account classification was decided. No real financial book or key was
used, no permanent login entry was installed, and no source/runtime secret,
database or backup was committed.

## Follow-up: Tailscale login verified at 20:26 Asia/Taipei

After the user's completed network-extension authorization and login attempts,
read-only checks confirmed the extension is `[activated enabled]`, official
`configure sysext status` reports `OK`, and macOS reports the Tailscale VPN as
`Connected`. The official CLI's parsed JSON now reports `BackendState: Running`,
`Self.Online: true`, an empty `AuthURL`, and no health warnings. Authentication
and local Tailscale connectivity are therefore verified; the earlier Connecting
state is historical. No additional reset, reinstall, configuration deletion or
credential entry was necessary in this follow-up. Account/device identities and
credentials are omitted. This does not establish the separate private Serve,
root-proxy or Safari HTTPS acceptance gates. No public service was enabled.
