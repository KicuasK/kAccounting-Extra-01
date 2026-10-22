# Dedicated private ingress boundary — 2026-10-22

Status: boundary implementation and disposable tests; deployment remains disabled.
Latest evidence: real private TLS and full disposable application transport passed
with the reviewed source-built v1.104.2 root daemon in ephemeral userspace mode.
The installed standalone GUI's sandbox denied both tested Unix socket locations.
No root-proof/TCP fallback was introduced. The temporary daemon/proxy was stopped;
permanent provisioning and Safari qualification remain open. See
[actual integration evidence](PRIVATE_INTEGRATION_2026-10-22.md) and
[updated private-access status](PRIVATE_ACCESS.md).
No Tailscale install, login, configuration, remote listener, or Funnel action is
performed by this module or its tests. Real macOS proxy/Safari acceptance is a
separate release gate.

## Provenance before identity headers

The dedicated ingress uses a Unix-domain socket under a canonical private 0700
parent, outside the source checkout. Runtime must create a new socket, set it to
0600, and validate parent/socket metadata. Socket paths are conservatively limited to 103 bytes for macOS Unix sockets;
use a short private parent rather than a long application-support pathname.
Existing pathnames are refused and
never automatically removed, including apparently stale sockets.

The runtime obtains the Unix peer credentials from the kernel, using Tokio's
`UnixStream::peer_cred`. Only UID 0 is accepted. `RootProxyConnection` has private
fields and a crate-private factory that takes the actual accepted stream; no HTTP
header, loopback address, supplied numeric UID, or ordinary user process can
construct its proof. Bind the proof to that connection's router service, never a
shared router reachable from another listener. Administrator/root access is a
trusted boundary because root can already inspect the user's financial process
and private files. This does not identify an executable cryptographically.

A same-user Tailscale peer is deliberately refused. A standalone or App Store
variant that cannot reach the socket, or connects without root credentials, must
remain unsupported until a different independently authenticated boundary is
implemented. There is no automatic fallback to loopback TCP or a caller-supplied
bearer/identity header.

## HTTP envelope and unchanged book guards

After the kernel check, the connection proof validates the expected Unix-target
proxy envelope: `Host: localhost`, exactly one canonical public
`X-Forwarded-Host`, `X-Forwarded-Proto: https`, the exact approved Tailscale login,
and the original public HTTPS Origin on mutations. Duplicates, unexpected
forwarding-origin overrides, a Funnel marker, other users, tagged/missing-user
requests, wrong origins, and cross-site metadata are rejected. The forwarded
source IP does not establish authentication.

After the complete public policy succeeds, the adapter removes forwarding
headers and rewrites Host and any present Origin to the existing local constants.
The unchanged session/book HTTP routers still enforce their ordinary guards.
The ordinary loopback listener never calls this adapter and cannot promote
forged Tailscale headers into proxy authentication. Safe navigation may omit
Origin; writes must carry the exact public origin before adaptation.

No real hostname, login, token or local socket location belongs in this source
checkout. Any future executable Serve plan must be reviewed against the
installed Tailscale release and remain private. No public/Funnel route is allowed.

## Evidence and compatibility limits

The official current CLI source documents Unix socket targets for Linux/macOS/BSD.
[Serve CLI source](https://github.com/tailscale/tailscale/blob/main/cmd/tailscale/cli/serve_v2.go)

The current proxy source connects to Unix targets, changes Host to localhost,
and sets forwarding/identity headers. Its identity handling strips incoming
identity values and does not provide user identity for Funnel or tagged clients.
[Serve proxy source](https://github.com/tailscale/tailscale/blob/main/ipn/ipnlocal/serve.go)

The user identity header is useful only after proving the transport boundary.
[Official Serve documentation](https://tailscale.com/docs/features/tailscale-serve)

macOS distributions differ and sandbox restrictions can limit filesystem access.
These documents do not prove the installed distribution can connect to our
private socket with the required UID; that must be measured with a disposable
runtime before enabling any private service.
[Official macOS variants](https://tailscale.com/docs/concepts/macos-variants),
[Official Serve examples](https://tailscale.com/docs/reference/examples/serve)

The referenced main-branch implementation is evidence checked on 2026-10-22,
not a pinned installed-version acceptance. Current macOS Tailscale requirements
are separate from the app's Catalina-inspired appearance.

## Remaining acceptance

- Verify actual installed Tailscale release, socket target behavior, peer UID,
  sandbox reachability and exact forwarded headers without using a real book.
- Exercise same-user forged HTTP headers against both ordinary and Unix listeners.
- Exercise approved and wrong private users, tagged/missing users, malformed or
  duplicated headers, exact public origins, streaming attachment uploads and
  downloads, selected-book transitions and disconnect/shutdown behavior.
- Confirm no public listener/Funnel and Mac Safari private HTTPS end-to-end.
- Consider descriptor-rooted socket setup for stronger protection from a local
  process racing pathname changes. Current identity checks fail on observable
  replacement but do not atomically eliminate every same-account syscall race.

## Verified disposable checks on this Mac

- Six module tests passed: real kernel peer-credential refusal for an unprivileged
  Unix socket pair; expected Serve-envelope normalization; strict public policy
  before local adaptation; duplicate identity/forwarding headers; wrong origin,
  identity, Funnel and forwarding overrides; navigation versus mutation Origin.
- Five filesystem tests passed: private socket mode, no adoption/deletion of
  existing paths, symlink parents, changed directory identity/permissions,
  ambient/relative locations and overlong socket paths. The actual Unix bind
  required the normal tool approval outside the sandbox; it did not start a
  TCP service or configure Tailscale.
- Targeted Clippy passed with warnings denied.
- At the earlier checkpoint, root-peer acceptance through a real Tailscale distribution was **not** verified.
  No root helper or real private service was started by these tests.
