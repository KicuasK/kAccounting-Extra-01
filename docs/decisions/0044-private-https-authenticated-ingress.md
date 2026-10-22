# ADR 0044 — Private HTTPS needs authenticated proxy provenance

Status: accepted foundation; private deployment remains blocked.

## Decision

Keep ordinary local service fixed at `http://127.0.0.1:3000`. Private HTTPS will
use one explicit lowercase device hostname under `.ts.net`, the exact HTTPS
origin and one explicit owner login. Do not enable Funnel or public access.
Private identity header checks must execute for reads and writes, before any
book data is accessed. Mutation requests require the exact Origin; cross-site
and same-site Fetch Metadata are rejected. Header duplicates and forwarding
origin overrides are rejected rather than normalized.

Tailscale identity headers alone do not prove that a request passed through
Tailscale Serve. The local listener can be called directly by local processes.
A caller can spoof every identity and forwarding header. Consequently regular
loopback ingress always fails the private-access policy, even with a perfect
set of identity headers. A second loopback port alone is also insufficient.

A future transport must establish proxy provenance separately, for example an
OS-permission-protected dedicated Unix socket with supported proxy access, or
an authenticated proxy bridge with a private capability unavailable to ordinary
clients. Its integration must prove that untrusted request data cannot choose
`AuthenticatedPrivateProxy`. No such boundary is provided in this increment.
No private serving flags, listener or functional Serve configuration are emitted.

## Scope and validation

The pure Rust policy and Python planning generator are testable with fake
hostnames and identities. The generator prints a disabled planning JSON document,
not a document accepted by the Tailscale CLI. It does not discover account
information, contact the network, write config, execute Tailscale or alter a tailnet.
This is a foundation, not a completed private remote-access implementation.

Official upstream context: [Tailscale Serve identity headers](https://tailscale.com/docs/features/tailscale-serve)
explains header injection and that localhost limits tampering to local services.
The stricter authenticated-ingress requirement here addresses that remaining
local spoofing boundary. Reviewed 2026-10-22.
