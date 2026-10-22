# ADR 0019: End-to-end conservative live-book export

Status: accepted by the conservative transfer policy and implemented for an
explicit attachment set

## Context

Verified live-object staging and durable staged-package publication existed as
separate safe primitives. A future service or UI still needed one operation
that composed them without losing the distinction between private work,
published-but-unverified output, verified success, and cleanup warnings.

## Decision

- Require separate caller-selected locations for local private work and the
  public `.kagextra` destination.
- Resolve both locations before snapshot work and reject a public destination
  whose parent is the local private work parent or any directory below it.
  This keeps recoverable private state outside the public artifact boundary.
- Reject a known destination conflict before taking a database snapshot or
  copying an attachment.
- Allocate a unique owner-only snapshot staging directory under the local work
  parent. Never reuse existing work.
- Prepare and verify the SQLite snapshot and explicit immutable attachment set,
  then pass that exact manifest and tree to the conservative staged-export
  operation.
- On preparation or package-export failure, return the local staging path and
  retain all private work. Never remove the source or a public destination.
- Only after the public package has passed its post-publication verification,
  attempt to remove local snapshot staging.
- Treat failure to remove local staging after verified publication as a
  non-fatal warning. A verified export must not be retried because cleanup
  failed.

## Consequences

Backend callers now have one operation that runs from an opened SQLite book and
an explicitly selected immutable attachment set to a verified `.kagextra`
package. Destination conflicts, changed attachments, and private-work recovery
have deterministic outcomes.

The remaining consistency integration is the future attachment repository's
responsibility: it must select the database-referenced attachment set under the
same logical live-book boundary. ADR 0020 and ADR 0021 add durable operation
state and restart classification. Physical removable-drive interruption
testing remains pending.
