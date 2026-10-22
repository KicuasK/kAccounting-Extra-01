# ADR 0028: Explicit localhost mutation API

Status: accepted engineering implementation; no network listener is enabled.

The original `api::router` remains an in-process read-only route tree.
`api::localhost_write_router` explicitly adds account maintenance, draft CRUD,
posting, reversal, and correction routes. Repositories and SQLite remain the
authoritative accounting boundary.

- Require the exact `Host: 127.0.0.1:3000` for every request, including reads,
  to reject DNS rebinding. Reject duplicate Host headers.
- Writes require exactly one `Origin: http://127.0.0.1:3000`. An Origin on reads
  must match too. Reject cross-site fetch metadata and duplicate origins.
- Do not trust forwarded-host/identity headers and do not enable CORS. This
  constructor does not implement the separate Tailscale HTTPS origin and
  identity boundary; that integration remains before private remote serving.
- Require JSON, reject unknown/duplicate fields, and cap request bodies at
  1 MiB. IDs, revisions, line numbers, and amounts remain canonical decimal
  strings; JSON numbers cannot enter the money model.
- Require an explicit nullable parent for moves, so an omitted field cannot
  accidentally move an account to the root.
- Use expected revisions for account edits/deletion and draft replacement,
  deletion, and posting. Repeated posting cannot create duplicate history.
- Return stable JSON errors, appropriate 400/403/404/409/413/415/422 statuses,
  and existing private/no-cache response headers. Never expose SQL errors.
- Expose permanent correction/reversal relationships in posted-entry reads.

A future server must deliberately choose its approved access policy. The
read-only constructor must not be mistaken for a complete private server.
No listener, LaunchAgent, public exposure, Funnel, or real-book startup occurs
as a result of constructing these routers or running their tests.

## Evidence

`api_mutations` exercises the complete account/draft/post/reverse/correct/report
workflow, large integers, failed-post rollback, revisions, concurrency, JSON
validation, body limits, origin/Host rejection, and explicit account moves.
