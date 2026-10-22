# ADR 0017: One conservative staged-export workflow

Status: accepted by the previously selected conservative transfer policy and
implemented

## Context

The backend already had separate primitives to verify a logical staging tree,
assemble an owner-only directory package, synchronize it, publish it without
replacement, and verify the public result. A caller still had to select a
same-file-system private path and invoke those primitives in the correct order.
That left room for an API or future UI to skip a safety boundary or mishandle a
known destination conflict.

This decision does not yet define how a live attachment store is frozen with a
SQLite snapshot. Attachment ownership and retention remain separate policy
questions.

## Decision

- Provide one staged-export operation that accepts an already consistent
  staging tree and its validated logical manifest.
- Reject any existing destination object before creating private work. The
  later atomic no-replace rename remains authoritative against races.
- Require the destination parent to be an existing real directory, not a
  symlink.
- Allocate the owner-only work directory beside the destination so publication
  stays on one file system.
- Run complete source verification, private package assembly, conservative
  synchronization, atomic publication, parent synchronization, and independent
  public verification through that operation.
- Never modify or delete the source staging tree.
- Return the selected private work path with assembly or publication failures.
  Private residue is retained for explicit recovery or cleanup. A failure after
  the atomic rename remains an explicit published-but-not-verified outcome and
  does not delete the destination.

## Consequences

Future API and UI code has one safe entry point for publishing an already
consistent export instead of reimplementing the sequence. ADR 0018 creates the
verified staging tree from one SQLite snapshot and an explicit immutable
attachment set, ADR 0019 composes both workflows, and ADRs 0020–0021 add durable
state and restart classification. Selecting the database-referenced attachment
set atomically remains the future attachment repository's responsibility.
