# Private access implementation status

The ordinary integrated preview remains `http://127.0.0.1:3000`. Private HTTPS
is **disabled**. No Tailscale configuration, login, account lookup, network
listener or Funnel change is performed by these scripts or the policy module.

A planning document can be generated with fake values:

```sh
python3 scripts/make-private-serve-config.py --hostname book.example.ts.net --login owner@example.com
python3 scripts/test_private_serve_config.py
```

The JSON is intentionally not an executable Serve configuration. Keep any plan
containing the real hostname or owner identity outside the source checkout.
No actual login information or private token should be committed.

`PrivateAccessPolicy::authorize` requires an exact canonical Host, exact own-user
`Tailscale-User-Login`, exact Origin for mutations and rejects duplicate security
headers, cross-site requests and forwarding origin overrides. Safe GET/HEAD
navigation can omit Origin. This policy accepts only an independently verified
transport marker. Passing the marker based on request headers or loopback peer
address is forbidden. The ordinary listener must continue to use its local-origin
policy; it cannot promote caller-supplied Tailscale headers into authentication.

The dedicated Unix ingress is now implemented with kernel-reported root peer
credentials before public HTTPS/identity authorization. An outer per-connection
boundary normalizes authorized proxy requests for the unchanged local guards;
the ordinary loopback listener cannot obtain that proof. It is available only
through an explicit library runtime entry point, without a default CLI enable.
See [ingress design and tests](PRIVATE_INGRESS_DESIGN.md).

Remaining work: verify the installed macOS Tailscale distribution can reach the
private socket with root peer credentials, verify its real Serve header behavior,
then validate private HTTPS on Mac Safari with a disposable book.
A second TCP loopback port without authentication does not resolve spoofing.
Do not enable remote service before those checks. See ADR 0044 for reasoning.

## Installed-client checkpoint — 2026-10-22 20:08 Taipei

Historical checkpoint: superseded by the actual login and probe results below.

Official Tailscale 1.104.2 installation has been verified. After full quit/reopen,
the GUI explicitly reports System Extension Granted and VPN Configuration
Granted. Earlier OS/CLI checks showed the exact state `[terminating for disable]`
while the UI was ON, plus CLI Error 1;
the login button still does nothing despite the granted GUI permissions, so
login remains blocked. This is not a
completed Tailscale login or private-access acceptance. No Serve configuration or
public/Funnel service has been enabled. The user permits a reboot after work is
saved; a restart and post-restart verification are not performed or claimed by
this documentation checkpoint. See [ongoing round checkpoint](BUILD_NON_UI_45MIN_2026-10-22.md).

## Actual installed-client compatibility — 2026-10-22 evening

Login/connectivity passed: the official CLI reports Running, Self.Online true,
no health warnings, extension OK and macOS VPN Connected. The user enabled HTTPS
certificates and explicitly deselected Funnel on the official consent page.

A foreground Serve route targeted only the disposable nonce probe, never the
accounting router. A normal TLS-verifying HTTPS client reached the service but
received HTTP 502. Logs restricted to that exact random socket reported:
`dial unix .../p.sock: connect: operation not permitted`. The installed standalone
system extension runs as root but its sandbox prevents reaching this private Unix
endpoint. No accepted proxy peer or identity-envelope positive result occurred.
The probe timed out and confirmed owned socket cleanup. Foreground Serve was
stopped; a fresh `serve status --json` returned `{}`. No public service was enabled.

This installed variant is therefore **not qualified at the tested temporary-socket location**.
Do not weaken the root-peer requirement or substitute unauthenticated loopback
HTTP. A separately reviewed unsandboxed official open-source daemon variant is
being prepared outside the checkout as a fallback; installation or switching has
not occurred. The signed extension also explicitly allows `/Library/Tailscale/`.
A new dedicated 0700 socket parent there is the preferred smaller compatibility
experiment, subject to operator administrator authorization and actual re-testing;
existing Tailscale contents must remain untouched. No success is yet claimed.
See [official macOS variants](https://tailscale.com/docs/concepts/macos-variants).

The explicit production `serve-private-chooser` command now loads a bounded,
private, no-follow configuration and starts only the Unix listener; it does not
invoke Serve. A disposable production CLI check passed GET/POST/DELETE forged
header refusal, 0600 socket permissions, unchanged empty collection and graceful
shutdown. This negative test does not replace real proxy positive acceptance.

## Positive ephemeral-daemon acceptance — 2026-10-22 21:18 Taipei

Actual private HTTPS acceptance subsequently **passed** using reviewed official
v1.104.2 source-built binaries and a user-started, root-owned foreground
userspace-networking ephemeral test node. It used memory-only node state,
accepted no DNS/routes and did not replace the installed GUI. The operator
authorized Unix Serve configuration; no permanent daemon/login item was installed.

Normal TLS verification reached the exact nonce-bound disposable accounting fixture
through the existing kernel-root-only ingress. The strict expected Host, HTTPS
forwarding and exact GitHub-style own-user identity policy accepted the real proxy.
The full HTTPS chooser/write/attachment/backup/container-transfer check passed:
chunked upload, exact download hashes, atomic balanced posting beyond JavaScript's
safe integer range, verified new-book import and unchanged source selection.
Direct same-user Unix forged GET/POST/DELETE requests still returned 403. Wrong
Host, Origin and cross-site metadata were refused. No real book or financial
Keychain was used. The acceptance client's mixed-case HTTP response header handling
was corrected and regression-tested; this did not weaken the server's guards.

After the operator stopped both foreground windows, independent checks confirmed
no staged daemon/Serve process or ephemeral control socket. The original GUI stayed
Running/online with empty Serve configuration. Its earlier sandbox failures at
both tested locations remain valid; the system-folder candidate did not solve them.
**Temporary positive acceptance does not qualify the GUI variant or establish
permanent installation, production signing/login/Keychain ACL, real wrong-user or
tagged-device behavior, or Mac Safari operator acceptance.**
