# Private integration checkpoint — 2026-10-22

## Actual network result

Tailscale login, extension and VPN connectivity passed. The user enabled HTTPS
and deselected optional Funnel. A temporary foreground private Serve route reached
TLS successfully, but returned 502 because the standalone system extension's
sandbox denied the private Unix socket. This is a measured incompatibility, not
successful accounting access. The probe contained only a nonce; no books or
financial keys were involved. The route was stopped, Serve state returned `{}`,
and the owned probe socket was removed. Existing GUI installation remains intact.

## Completed code increments

- `private_runtime_config.rs`, production `serve-private-chooser`, six configuration
  tests: explicit opt-in Unix runtime, strict bounded private configuration,
  descriptor-based no-follow validation, generic identity-free errors. No Serve
  invocation or TCP fallback.
- `native_host.rs` and four headless integration tests: pinned helper, explicit
  typed selections, cancellation/reaping, shutdown and retained-router closure.
  Explicit `serve-native-chooser` and `serve-private-native-chooser` production
  commands now construct the trusted host without opening a panel. Runtime
  shutdown and startup-failure paths close the host. Actual GUI remains separate.
- Native helper builder: reject hardlinks before chmod; recheck parent and binary
  identity/permissions before publication. Thirteen builder tests passed, including
  two new regressions; three example argument tests passed. Real Swift app build
  passed; no new GUI operation was claimed.
- Bounded disposable provenance probe and six tests, including Darwin peer UID
  lookup. Its actual ordinary-user forged request received 403; real proxy
  connection was prevented by the measured sandbox denial.

## Integrated verification

The completed verifier at `/private/tmp/ka-private-integration-verify.log` passed
**643 Rust tests, zero failures, nine ignored; 64 Python tooling tests; 41 frontend
tests**, plus format/lint/documentation/type checking and production frontend build.
Subsequent complete verification at `/private/tmp/ka-private-final-verify.log`
passed **645 Rust tests, zero failures, nine ignored; 64 Python tests; 41 frontend
tests**, including host-listener wiring and a delayed-body shutdown regression.
The session now closes admission under its selection lock, so a request admitted
before shutdown cannot later reopen a book. Release was rebuilt after that fix.
`git diff --check` passed. A separate actual production
CLI smoke check passed all three forged HTTP methods, private socket mode, empty
books and graceful shutdown; its disposable fixture is retained outside source.

## Remaining

The installed extension also explicitly permits `/Library/Tailscale/` in its
signed sandbox entitlements. A dedicated new 0700 operator-owned socket directory
under that root-owned parent is a smaller compatibility candidate than replacing
the installed variant. Its preparation requires explicit operator administrator
authorization; no existing Tailscale configuration or keys should be inspected or
modified. Actual compatibility at this location is not yet established. A separate
official source build is staging-only fallback, not an installation or network change.
Re-run actual Unix peer/header,
private HTTPS and selected-book integration only after compatibility is established.
Do not fall back to trusting caller-provided headers or another local TCP port.
Trusted host startup/shutdown composition and its regression checks passed. Native GUI, signed
distribution/login qualification and Mac Safari end-to-end remain explicit gates.
Physical APFS/exFAT external-drive qualification remains Version 02; retain code.

The branch and existing uncommitted work are preserved; no commit or push occurred.
No real account/device identities, keys, databases or backups are included here.

## Later integrated checkpoint

The final verifier at `/private/tmp/ka-nonui-latest-verify.log` passed **648 Rust
tests, zero failures, nine ignored; 87 Python tooling tests; 41 frontend tests**,
plus all format/lint/documentation/type/build checks. Release also passed at
`/private/tmp/ka-nonui-latest-release.log`. This includes strict GitHub-style
Tailscale login compatibility (`alice@github`), preserving exact identity matching,
and the private HTTPS acceptance tooling. The actual loopback disposable test
passed frontend/token proof before any writes, explicit selection, exact balances,
chunked attachment upload/download hashes, atomic posting above JavaScript's safe
integer range, encrypted backup, export/container download/upload/import, preserved
current selection and explicit switch, and return to the chooser. This is loopback
evidence, not private HTTPS success.

The user created the new private system-folder parent and it was verified 0700
and operator-owned. Re-testing there still produced the same sandbox `operation
not permitted`; this candidate failed and is not a supported deployed fix. The
test socket was cleaned, the approved parent retained and Serve stopped.

Official pinned open-source binaries were built and stamped using the verified
release commit outside source. The user subsequently started an isolated,
foreground userspace-networking daemon with memory-only ephemeral state, DNS and
route acceptance disabled, no persistent job and no installed-GUI replacement.
Its bounded read-only status check reported `NeedsLogin`; the user is completing
the separate test-node login. Root-peer/HTTPS acceptance through that node remains
pending. Version-only verification and preparation do not establish network
acceptance. See `TAILSCALE_VARIANT_ACCEPTANCE.md` for provenance and review.

## Actual private HTTPS passed and test services stopped

At approximately 21:18 Taipei the user-authorized ephemeral root daemon and
foreground Unix Serve route passed the complete disposable HTTP acceptance over
normal certificate-verified HTTPS. All four check sets passed: read-only page and
exact reports/security envelope; draft/streaming attachment/atomic post/verified
encrypted backup; complete encrypted export/download/upload/new-book import with
source selection preserved and explicit switch; explicit close back to chooser.
The first client run stopped before any writes because it treated response header
names case-sensitively; canonicalization was fixed with success/error regressions.
Subsequent complete HTTPS acceptance passed. No accounting guard was relaxed.

The exact real ingress also denied same-user forged GET/POST/DELETE requests with
403 despite correct copied proxy identity fields. The successful real proxy
passed the unchanged kernel UID-0 proof and strict expected forwarding/login
policy. A separate wrong-user/tagged-device real-client trial was not performed.

The ordinary operator attempted to remove the foreground-owned Serve handler and
received `handler does not exist`; this attempt is not counted as cleanup.
The user then explicitly pressed Control-C in both root foreground windows.
Independent checks confirmed the ephemeral socket absent and no staged daemon or
Serve process. The original GUI was still Running/online and its Serve config
remained empty. The disposable backend stopped cleanly and removed its own test
book fixture. No permanent login/background service was installed. Retained
private staging/build/receipt material stays outside source for inspection.

Remaining work is permanent provisioning of a compatible daemon and startup
workflow, signed-production/login/Keychain qualification, actual native operator
cases, and Mac Safari end-to-end. The functional private transport is now proven
in the temporary configuration; the default installed GUI has not become compatible.

## Final wrap-up before the requested ten-minute stop

At 21:24 Taipei the last integrated verifier completed successfully:
**648 Rust tests passed, zero failed, nine ignored; 90 Python tooling tests passed;
41 frontend tests passed**, including response-header case regression and socket
construction-failure cleanup. Format, strict lints, documentation, typecheck and
production frontend build also passed. Log: `/private/tmp/ka-wrap-final-verify.log`.
The final backend Release build passed (23.96 seconds), logged at
`/private/tmp/ka-nonui-latest-release.log`; there were no subsequent Rust changes.
`git diff --check` passed. Branch remains `main` at `afba3de`; existing uncommitted
changes are preserved. No staging, commit or push occurred. No real financial
book, key, database or backup was placed in source or used by these workflows.
