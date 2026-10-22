# Tailscale distribution acceptance and staged fallback

Status: build staging only; no open-source daemon installation, startup, login,
GUI replacement or system/network change is authorized by this document.

## Measured GUI distribution boundary

The coordinating acceptance run used the installed GUI distribution v1.104.2.
HTTPS reached the Tailscale proxy and TLS verification succeeded, but its root
Network Extension could not connect to the disposable private Unix socket:
`connect: operation not permitted`. No Unix peer reached the probe. A subsequent
probe under the GUI extension's apparent `/Library/Tailscale/` filesystem exception
was also refused. These observations do not permit weakening the root Unix-peer
requirement or trusting identity headers on an ordinary loopback TCP socket.
The coordinating run owns the logs and Serve cleanup evidence.

Entitlement path exceptions made the second probe a reasonable empirical test,
but do not establish that Apple's sandbox permits Unix socket connections there.
Actual refusal takes precedence over inferred filesystem access permissions.
The existing GUI distribution remains preserved; no CLI-only operational switch
is implied by source preparation.

## Private source-build staging

The Go toolchain was downloaded from the official Go release API. At preparation
on 2026-10-22 the stable darwin/arm64 release was Go 1.27.3:

- Official archive: `go1.27.3.darwin-arm64.tar.gz`.
- Expected and observed SHA-256:
  `dd3908cf20959d798c1e06ebe5b72213f0505ce01373b40ce3110975a6cd99c2`.
- Tool reports: `go version go1.27.3 darwin/arm64`.
- Private staging root: `/private/tmp/ka-ts-source-stage-8876a10dbb362373`.
- GOPATH, GOCACHE, GOBIN and GOTMPDIR are dedicated children of that staging root.
  GOENV is disabled, GOTOOLCHAIN is local, and official Go proxy/checksum services
  are used. No global Go installation or user Go configuration is changed.

[Official Go release metadata](https://go.dev/dl/?mode=json),
[Official Tailscale v1.104.2 release](https://github.com/tailscale/tailscale/releases/tag/v1.104.2).

The pinned build request is `tailscale.com/cmd/tailscale@v1.104.2` and
`tailscale.com/cmd/tailscaled@v1.104.2`. Dependencies retain their upstream module
versions and are verified through Go's checksum database. Source checkout,
compiler/cache artifacts and executable binaries stay outside this repository.

The first sandbox build attempt failed DNS before downloading modules. Its
network approval remained pending and was subsequently interrupted, so no build
ran during that interval. A fresh approved network build started afterward.
This was a permission wait, not a completed or hung compilation.

Final binary hashes, embedded build info and version-only output will be recorded
once the pinned build completes. A successful compilation alone does not pass
private HTTPS, Unix-peer, login, reboot or Safari acceptance.

## Operational approval still needed before any switch

Review the produced executables and provenance first. The operator then needs to
approve the precise operational plan, including administrator steps if required,
any GUI-distribution pause/disable or replacement, and any separate Tailscale
login or node identity. Do not reuse or read the GUI extension's existing state,
authentication keys, certificates or financial storage for a new daemon.

Any future daemon must use an explicitly new private state directory and socket,
remain non-public, and be exercised only against the disposable provenance probe
before any application integration. No Funnel operation is allowed. Preserve a
concrete rollback plan to the existing GUI distribution before making changes.
This document does not itself authorize those changes.

## Completed staged build evidence

The approved pinned source build completed with exit status **0**. Both
version-only invocations succeeded; no daemon was started or installed.

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `bin/tailscale` | 24,071,346 bytes | `502f6a711315f7064c41cb20903804aac76cb8551fc1335f13a9580d636fb23d` |
| `bin/tailscaled` | 35,416,930 bytes | `94302ed81017dac64ae42510f1844e3c68aedb2825819f82cf17fe6583e1c636` |

Paths are relative to the private staging root identified above. The public
build receipt is `/private/tmp/ka-ts-source-stage-8876a10dbb362373/build-receipt.json`;
full embedded module/build information is saved beside it as
`tailscale-build-info.txt` and `tailscaled-build-info.txt`.

Both embedded Go build records identify `tailscale.com v1.104.2`, module checksum
`h1:zBbSW0tI4Xi2PhrZEl9yt6oWay3l8A+M8scsTnZpyAk=`, darwin/arm64, and Go 1.27.3.
The official module origin metadata binds `refs/tags/v1.104.2` to commit
`e26db7a5797683ec99ad1516c40b628c7f2dfd71` in
`https://github.com/tailscale/tailscale`.

Plain versioned-module `go install` has no VCS checkout metadata for Tailscale's
custom version formatter, so the version-only output says
`1.104.2-ERR-BuildInfo`. The source version is established by the embedded module
record and checksum; this suffix is not represented as an official distribution
binary. Upstream supports linker version stamps in `build_dist.sh` and
`version/version.go`; any stamped rebuild would need new hashes/receipt before
operational review. These binaries have not been silently rewritten or stamped.

No source/build artifacts are committed. No installer, launch daemon, root
helper, login, GUI replacement, Serve configuration or financial data operation
was performed during staging. The operational switch and subsequent real Unix
provenance/Safari acceptance remain pending the coordinating operator plan.

## Reviewable stamped build and temporary operator actions

A separate stamped rebuild now uses upstream's documented linker fields and
version grammar, with the same verified pinned commit. It reports short
`1.104.2`, long `1.104.2-te26db7a57`, and full commit
`e26db7a5797683ec99ad1516c40b628c7f2dfd71`; no version was invented.
Original unstamped artifacts remain unchanged for traceability.

| Stamped artifact under staging root | SHA-256 |
| --- | --- |
| `stamped-bin/tailscale` | `6ccc410e7681a816007ea58c041717275be0a3f68a84ae2e288ad0669118197d` |
| `stamped-bin/tailscaled` | `499a99eeef9efa06e288b2f38ab803acfd9e3c5335cad6c297325c5420f3f30c` |

Two operator-action `.command` files are staged only under
`/private/tmp/ka-ts-ephemeral-75e341f417b5`. `operator-review.json` records exact
commands, binary/script hashes and the new state/socket paths. `Start.command`
requires a real terminal, Chinese confirmation by pressing Return, then sudo.
It starts a foreground, userspace-networking, memory-only ephemeral node with
an explicitly new private state directory/socket, log upload disabled and empty
file-based syspolicy. Ctrl-C stops that foreground process. It does not install
a permanent job, request a TUN device or reuse GUI state.

`Login.command` separately requires a terminal and explicit confirmation, checks
the new socket, then offers the temporary-node login with a unique test hostname,
DNS/route acceptance disabled, and the current user as that node's operator.
Any authentication URL is for the user to review; no auth key or saved GUI
credential is supplied. This is a new temporary node, not a permanent production
variant switch. GUI coexistence and no host DNS/routing change still require
actual operator verification; staging alone does not establish them.

Both scripts verify directory device/inode/owner/0700 and exact executable hashes
before and after confirmation. Python isolation and explicit errors prevent
Python environment/optimization settings from disabling checks. Six read-only
guard checks passed (valid assets, altered digest refusal, wrong owner refusal
for each script), and shell syntax checks passed. These checks did not run the
daemon, `up`, sudo, or either launcher's operational path.

At staging completion neither launcher has been activated by the agent. Root
coordination owns review, user authorization, any actual administrator/login
operation, and subsequent disposable Unix provenance acceptance.
