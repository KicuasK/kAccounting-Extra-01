# ADR 0046: Explicit local HTTP book sessions

- Status: Accepted technical implementation of recorded user decisions
- Date: 2026-10-22

The user requires book selection on every launch and an explicit choice before
switching after import/restore. A server that opens a book before showing a
selector cannot implement that policy.

Start a loopback session without opening any book. Discover candidates only
inside a locally configured private collection. Accept exact direct-child IDs
through HTTP; never accept arbitrary filesystem paths or expose key IDs/paths.
Keep accounting/storage endpoints gated until explicit selection. Do not persist
selection, even when only one candidate exists.

Use owned read leases through the full HTTP response body, and nonblocking write
leases for book transitions. Active requests, streams and owned storage jobs
return `book_busy` instead of allowing forced close. Own transition tasks across
client disconnection; preserve the old book on replacement failure. Imports and
restores never select their output. All routes retain the same exact loopback
Host/Origin/fetch-source boundary.

The native file selector/UI remains future work; capability registration is
local-only. Legacy explicit-book server APIs remain for compatibility with
existing acceptance tests. The default launcher can adopt `serve_session`
without changing accounting invariants or real financial data.
