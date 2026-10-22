# ADR 0006: Read-only API foundation and exact JSON integers

Status: accepted for the backend foundation on 2026-10-22.

## Context

The Rust domain stores identifiers, revisions, line numbers, and money as signed
64-bit SQLite integers. JavaScript represents ordinary JSON numbers with binary
floating point and cannot preserve every 64-bit integer exactly. The project's
authentication and final localhost serving policy are also unresolved.

## Decision

- Encode every SQLite integer exposed by the API as a base-10 JSON string.
- Keep booleans as JSON booleans and dates/timestamps as validated strings.
- Initially register only health and read-only account, draft, journal, ledger,
  trial-balance, five-element, and income/expense routes.
- Compile Axum without default server features and enable only JSON support.
- Do not open a network listener from the executable yet.
- Do not add mutation routes until their authorization boundary and request
  contracts are reviewed.
- Disable response caching, omit permissive CORS headers, and return stable JSON
  error codes without database details.

## Consequences

Safari clients must parse exact integer strings deliberately, using `BigInt` or
validated decimal-string handling rather than implicit `Number` conversion.
The in-process router can be tested now without exposing private accounting data
on any interface. A later serving decision must still enforce localhost-only
binding and must never enable Tailscale Funnel.
