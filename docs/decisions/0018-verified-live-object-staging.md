# ADR 0018: Verified private staging from live-book objects

Status: accepted by the conservative transfer policy and implemented as a
backend primitive

## Context

The staged-export workflow requires a private tree containing one verified
SQLite snapshot and every attachment named by a validated manifest. Until now,
tests and callers had to construct that tree manually.

Attachment ownership, identifiers, retention, and the database query that will
select an attachment set remain unresolved product decisions. This primitive
must not silently decide them.

## Decision

- Create a new owner-only staging directory on caller-selected local private
  storage. Never reuse or replace an existing path.
- Produce `book.sqlite3` with SQLite's verified point-in-time snapshot
  primitive instead of copying the live WAL database file.
- Require the caller to provide each selected attachment's safe package path,
  source path, exact byte length, and SHA-256 digest.
- Validate the complete logical manifest before using any attachment path as a
  destination.
- Copy each attachment through verified no-overwrite local publication. Missing,
  changed, symlinked, or otherwise unsafe sources fail closed.
- Independently verify the completed staging database and every declared
  attachment against the resulting manifest before returning success.
- Preserve the live database and all source attachments. On failure, retain any
  private staging work for explicit diagnosis or cleanup.
- Surface post-publication cleanup residue as warnings without converting a
  verified object into an ambiguous failure.

## Consistency boundary

This primitive guarantees that the returned tree contains one valid SQLite
point-in-time snapshot and the exact attachment bytes explicitly selected by
the caller. The future attachment repository must select that attachment list
under its live-book consistency boundary so the database references and list
belong to the same logical point in time. Until that repository exists, this
primitive must not be described as proving database-reference completeness.

## Consequences

The backend can now safely prepare real transfer input without adopting an
attachment ownership policy. ADR 0019 composes it with conservative export,
and ADR 0020–0021 preserve and recover the durable operation record across
interruption.
