# Private deployment operator acceptance — 2026-10-23 Taipei

Current scope (2026-10-23 user decision): one Mac hosts all runtime and books; remote clients use private browser access. Developer ID signing and notarization are not required for this private deployment. Physical cross-Mac migration is not a daily-use release gate. Backup/restore, kernel peer and identity checks, development-package integrity, dedicated Keychain and login/reboot acceptance remain required. Do not build or change UI in this non-UI phase.

This runbook is a review and evidence checklist. Persistent installation,
lifecycle and version-management tools are now provided; their existence does
not establish actual installation or qualification. See [runtime operations](PRIVATE_RUNTIME_OPERATIONS_2026-10-23.md),
[transport versioning](PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md) and the latest
[single-Mac preparation record](SINGLE_MAC_INSTALL_PREPARATION_2026-10-23.md).
Formal signing is out of scope; development packaging must remain identified as development. Use disposable books and separately authorized
test keys throughout. Never use an existing financial key or real book to fill
an acceptance gap.

## What is ready for review

The disabled-plan generator verifies runtime and staged transport identities,
binds both plist files into a retained review SHA-256, and checks inputs again
immediately before completion publication. Its offline verifier checks the saved
artifact, not installation readiness. Example-only review output has
`example_only=true`, `deployment_ready=false`, `installed=false`, `started=false`
and `qualified=false`. Illustrative identities are never production credentials.

Current reviewed transport build metadata declares version `1.104.2`, source
`https://github.com/tailscale/tailscale`, revision
`e26db7a5797683ec99ad1516c40b628c7f2dfd71`, daemon SHA-256
`d50d16f6d78ced0b767a73918aed27de3267d1dd5345241a56a896956200a700`
and CLI SHA-256
`61288a7ce502381b9a080c13f83fc5f3d235f9e6291336be7b3d46e7c56092cb`.
These are staged development artifacts, not a signed-release or reproducible
build attestation. Recheck the exact retained files, not just these strings,
before a new review package is generated.

Keep the complete `StoragePicker.app` when preparing a new runtime; preserve its
Info.plist identity and use the actual `Contents/MacOS/StoragePicker` child as
the trusted helper. The package's `startup.native_helper` records its verified
relative child path. Never pass the application directory as an executable, or
claim a panel is usable based solely on helper construction/compilation. Confirm
the visible panel and explicit user cancellation on Mac Safari.

## 1. Freeze inputs and record the review

1. Retain exact runtime manifest digest, source revision role, architecture,
   development signature result, complete helper bundle and transport provenance.
2. The operator supplies the actual canonical private hostname and exact own-user
   login; no inferred hostname, example login or copied GUI node identity is
   acceptable. Keep that configuration outside the checkout in a private file.
3. Choose an existing mode-700 disposable books collection and a new short
   accounting Unix socket under a private canonical parent. Keep it separate
   from the future root daemon control socket and state directory.
4. Generate a new non-example review package using all explicit expected hashes.
   Record the returned review digest independently; run the offline verifier.
5. Read both disabled plists and the review requirements. Any input change,
   source substitution or mismatch requires a new review directory and digest;
   existing output must remain intact. Do not turn `Disabled` off as a shortcut.

## 2. Administrator boundary — tools implemented, actual activation pending

Future transport binaries belong under the fixed new root
`/Library/Application Support/kAccounting-Extra-01/private-transport/` with an
explicit private `--statedir` and `--state`. The separate control parent is
`/private/var/run/kaccounting-private-transport/`; spelling out `/private/var`
avoids the `/var` symlink. Future roots/state/control names must be new and
root-owned, mode 700; binaries must be root-owned single-link regular files,
mode 700 and match the reviewed hashes. The existing GUI files, state, sockets,
login items and keys must remain untouched.

The reviewed `install-private-transport.py`, `manage-private-transport.py`,
`private-transport-guard.py` and `update-private-transport.py` now provide separate
no-replace file installation, registration/lifecycle, guarded control-parent
creation, version switching and interruption checks. Actual root installation,
registration/start and reboot qualification remain separate operator gates.
A transport-only bootstrap review can be prepared before new-node login with
`prepare-private-transport.py`; it does not guess the future hostname or login.
Review coexistence with the GUI and which exact newly created job owns each path
before any OS authorization. Do not bootstrap a disabled review plist directly.

New-node login and private Serve configuration require their own explicit
operator actions against the reviewed transport. Keep DNS/route acceptance and
tailnet policy within the approved scope. Never reuse GUI state, enable Funnel,
expose a public service or substitute a TCP listener for the root-peer Unix
boundary. This document provides no unattended login or activation command.

## 3. Disposable transport and Safari acceptance

Record actual timestamps, exact binary hashes, node identity source, URLs and
sanitized command results in a private evidence directory. Do not record private
keys, tokens, raw financial payloads or unrelated OS state.

| Gate | Required evidence | Failure behavior |
| --- | --- | --- |
| Root Unix peer | Actual proxy connects with kernel UID 0 to the configured app socket | Refuse same-user peers; keep private ingress disabled |
| Public envelope | Normal TLS verification, exact hostname, HTTPS forwarding and own-user identity accepted | Wrong Host/Origin, duplicate headers and cross-site requests rejected |
| Wrong identities | Actual second user and tagged/missing-user clients refused | Do not substitute forged-header-only testing for real identity evidence |
| Public exposure | Exact new node's Serve status remains private-only; no Funnel route | Stop the exact reviewed route and investigate |
| Safari chooser | Starts unselected; explicitly opens disposable book; another tab changing selection causes reconciliation | No silent adoption of another generation |
| Native panels | Complete helper app visibly opens only on explicit local action; cancellation has no side effects | Keep operation pending/failed; never accept browser raw paths |
| First book | New destination is a direct child of configured collection; completion adds a book without selecting it | Existing/hidden/outside destinations refused without overwriting |
| Accounting | Exact large integer balanced posting; posted entry immutable; reports use posted lines | No success claim on parse/reconciliation failure |
| Attachments/backup/transfer | Upload/download hashes, encrypted export/import, new destination and unchanged selection | Preserve ambiguous work for recovery; no silent overwrite |
| Direct forgery | Same-user Unix forged GET/POST/DELETE and wrong public envelopes rejected | Never weaken peer proof or add TCP fallback |

Local native routes exist only in explicit loopback `browser_router` composition:
`POST /api/native/selections` accepts a typed kind, while `POST /api/native/books`
accepts only `{}` and can initialize a first book after a genuine destination
panel. Both require exact local Origin and selection generation; creation also
accepts the explicit `unselected` guard. Neither accepts a path, helper or
capability supplied by the browser. Private Unix composition deliberately omits
these panel routes; remote Safari therefore cannot claim local OS panels.

## 4. Dedicated Keychain and lifecycle qualification

Use only newly created, uniquely identified test items permitted by the user.
Record their identifiers and retain them for inspection. Existing financial
keys, default Keychain/search list and unrelated items are outside scope. Any
OS authorization prompt is completed by the user. The product does not acquire
a password, PIN, passphrase or recovery phrase through this procedure.

For each reviewed development binary identity, measure successful dedicated-key
creation/read, user-cancelled authorization, locked/unavailable Keychain behavior
and clean error mapping without logging secret bytes. Repeat after logout/login
and an explicitly authorized restart only once the new lifecycle jobs and
rollback have been separately reviewed. Confirm the root control parent was
recreated safely, only the intended jobs start, chooser remains unselected,
native panel identity still works and no unrelated GUI state changed. Old
temporary foreground acceptance does not establish any of these results.

## 5. Stop and rollback

Stop only the exact newly reviewed Serve route and new job labels. Verify no
new helper child, node process or accounting listener remains. Preserve books,
keys, encrypted packages, new node state and failed staging for inspection.
Do not remove persistent state or touch the original GUI; any cleanup beyond
the exact new job requires an explicit reviewed scope. Record actual cleanup
results separately from permission to perform cleanup.

## Evidence record

For each gate record `not_run`, `passed`, `failed` or `blocked`, evidence path,
operator and Taipei timestamp. Never mark a gate passed from compilation,
configuration generation, a granted permission or a synthetic fixture alone.
The final production qualification stays false until every required real gate
has passed; a blocked wrong-user, lifecycle, Keychain or Safari check remains
visible. Developer ID signing/notarization are out of scope, not passed checks. Development identity and package integrity must still be recorded.
