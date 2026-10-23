# Version 01 non-UI remaining work — 2026-10-22

2026-10-23 update: browser selection-generation guards protect new UI requests
from cross-tab switching. Native helper publication no longer leaves staging
hardlinks that violate runtime-packaging inputs. A development bundle and disabled
launch plan were verified; read-only Developer ID preflight is available. The user
has no signing certificate and selected keeping the development bundle. Permanent
proxy/login/reboot/production Keychain remain unqualified. See
[current evidence](BUILD_TAKEOVER_2026-10-23.md).

Source/document audit, refreshed after the evening integration. This checklist distinguishes missing production
wiring from acceptance that has not yet been exercised. It does not authorize OS
changes or accessing real books/keys. Existing uncommitted work is preserved.

## Current evidence

- Core accounting, encrypted package/attachment transfer, migration checkpoint,
  new-destination restore, explicit selection, owned jobs and bounded HTTP
  transport are implemented. The last completed whole-tree checkpoint passed
  **651 Rust tests (9 ignored), 112 Python tests and 41 frontend tests**, plus
  formatting, lints, documentation, typecheck and production build. The separate
  Release build passed. These are checkpoint results, not a claim that later
  changes have been reverified; see
  [latest 20-minute verification](BUILD_20MIN_2026-10-22.md) and
  [earlier network verification](PRIVATE_INTEGRATION_2026-10-22.md).
- Tailscale has since reached `Running`, `Self.Online: true`, extension `OK`, VPN
  `Connected`, and no health warnings. Authentication/connectivity is now passed;
  the older pending-extension/login-blocked paragraph in `PRIVATE_ACCESS.md` is
  historical. Actual full private HTTPS acceptance subsequently passed through
  a reviewed source-built ephemeral root daemon; both tested GUI Unix locations
  remained sandbox-incompatible. Test services were stopped and the original GUI
  stayed online. This establishes temporary authenticated transport, not permanent
  compatible deployment or Safari qualification.
- Real dedicated-item Keychain and isolated create/read/duplicate/lock/denied-read/
  unlock/read acceptance passed without reading existing financial items or
  changing default/search-list preferences. Transient launchd bootstrap, empty
  selection, zero fake-key calls and removal of the owned test job passed.
  True logout/login and a packaged production executable are separate cases.

## Concrete production gaps and bounded next increments

| Priority | Remaining increment | Code evidence and completion criterion |
| --- | --- | --- |
| 1 | Qualify the permanent compatible proxy setup; temporary positive transport passed | The reviewed source-built ephemeral root daemon passed complete real HTTPS through [`serve_private_session`](../backend/src/runtime.rs), strict identity/envelope and kernel-root proof. Same-user direct Unix forgery returned 403. Preserve these guards; the installed GUI still cannot use either tested Unix location. A permanent compatible daemon/lifecycle configuration and actual wrong-user/tagged-device cases remain unqualified. |
| 2 | Validate the now-wired opt-in production start path against the real proxy | [`main.rs`](../backend/src/main.rs) now exposes `serve-private-chooser <private-collection> <assets> <private-config.json>` through [`private_runtime_config.rs`](../backend/src/private_runtime_config.rs). Configuration is strict, bounded, private/owned and descriptor-opened without following symlinks. The command starts only the existing Unix ingress and never invokes Tailscale or a TCP fallback. Default `serve-chooser` is unchanged. Real proxy acceptance from increment 1 remains required; [`make-private-serve-config.py`](../scripts/make-private-serve-config.py) still deliberately emits a disabled planning policy with `serve_config: null`. |
| 3 | Native host wiring completed; qualify actual operator workflows | [`native_host.rs`](../backend/src/native_host.rs) now composes the trusted picker/session lifecycle; production `main.rs` constructs it in explicit local/private native-chooser commands. Construction opens no panel. Cancellation, reaping, bind failure, closed-router and delayed-body reopening regressions passed. Actual request/book save, cancel and recovery-parent panels passed in the 22:06–22:26 round; later application UI actions remain unqualified; no web-supplied paths or panel endpoint was added. |
| 4 | Completed integrated regression; revalidate future packaging changes | The final whole-tree verifier and Release passed after host/identity changes. Full disposable selected-book export/import, attachment streaming, exact posting, shutdown and private TLS transport passed. Future permanent installation/signed-production changes require their own tests; do not reuse the temporary-node result as proof of a packaged production startup. |

The measured proxy compatibility requires a compatible unsandboxed daemon
deployment; headers alone or another unauthenticated TCP port must not replace proof.
A real Tailscale login is not sufficient reason to enable app access early.

## Remaining OS/operator qualification, not missing accounting primitives

- Actual request/book save destinations, cancellation and recovery-parent selection
  passed; record additional package/backup/book workflows separately. Swift compilation, canonical-path tests and bridge
  tests alone do not pass the live operator flow. Any coordinated live result
  should be recorded precisely; a partially operated panel is not a pass.
- Persistent installation, actual logout/login or reboot startup and production
  signed-app Keychain/ACL behavior remain unqualified. The successful disposable
  transient launchd and isolated-Keychain cases are narrower. Preparing a
  disabled plan is not installation. Obtain operational approval before changing
  persistent login settings or touching an existing key.
- Mac Safari private HTTPS and full application workflow acceptance follows the
  relevant wiring/UI. HTTP router/TCP tests do not establish browser behavior.
- The nine ignored tests are intentionally outside the routine verifier; consult
  the previous explicit capacity/process-death results and rerun relevant ones
  if their implementation changes. Do not call them failed merely because they
  are ignored, or claim they ran in the 630-test checkpoint.

## Explicitly outside these remaining non-UI tasks

Physical external APFS/exFAT qualification moved to Version 02 by user decision;
all portable-media implementations/tests/tools remain. It no longer blocks
Version 01 or beginning UI. Ordinary private local Mac storage remains Version 01.

The approved Catalina account/journal/report/attachment/backup/transfer interfaces
remain to be built in the UI phase. Ambiguous handwritten account classification
blocks default seeding only; do not invent policy to mark the backend complete.
The approved policies remain: choose a book every launch; import/restore does not
auto-switch; Safari downloads to Downloads before manual external-media movement;
no replacement-key fallback. No real accounting data is part of development.


## Explicit private-runtime configuration (opt-in wiring)

The production command accepts an existing canonical absolute configuration file
outside source, under a user-owned mode-700 private parent. The file must be
user-owned, regular, mode 600, single-link and at most 16 KiB. Ancestors and the
leaf are opened with no-follow directory descriptors; file and path identity are
rechecked after bounded reading. Unknown/duplicate JSON fields, malformed policy,
unsafe or already occupied socket paths are refused with a sanitized error.
The exact schema is three strings:

```json
{"hostname":"book.example.ts.net","login":"owner@example.com","socket":"/absolute/private-socket-parent/proxy.sock"}
```

These are invented examples, not deployable account settings. A real config stays
outside Git. Loading it does not read a key, configure Serve, activate Tailscale,
create a socket, or print its hostname/login. Explicit execution of
`serve-private-chooser` requests the Unix listener and still starts without a
selected book. Unsupported/non-root proxy clients remain denied by the existing
kernel-peer boundary. No real positive HTTPS acceptance follows from this code
being available. Focused fake-file configuration tests cover the contract;
whole-tree/Release results after the wiring remain the integrating run's duty.
