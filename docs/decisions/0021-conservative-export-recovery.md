# ADR 0021: Conservative durable export recovery

Status: accepted engineering decision and implemented

## Context

The durable transfer record from ADR 0020 made lifecycle changes persistent,
but the end-to-end export created that record only after snapshot preparation
had completed. An interruption during the database snapshot or attachment copy
could therefore leave a private directory without machine-readable ownership
or destination information. Restart behavior also needed to distinguish a
destination published by this operation from one created by a concurrent
publisher.

## Decision

- Create and synchronize the initial `working` record immediately after the
  private snapshot root is allocated and before creating the SQLite snapshot or
  copying attachments.
- Synchronize both the work root and its parent directory when creating the
  initial record so the private root's directory entry is included in the
  durability boundary.
- On Unix platforms, reject work roots with any group or world permissions and
  reject state files that are not owner-only. Recovery state is authoritative
  and must not live in a directory writable or readable as shared work.
- Recovery is read-only for `working`, `verified`, and `failed` states except
  for the one unambiguous resumed transition described below.
- A `working` record with no destination remains private work. A `working`
  record whose destination name identifies any filesystem object is classified
  as a collision requiring explicit inspection. This check does not follow a
  destination symlink, and recovery never adopts the object because a competing
  exporter may have created that path.
- A `published_unverified` record proves that this operation persisted its
  state after the exclusive rename. Recovery first repeats synchronization of
  the destination parent, then re-runs the complete public package audit, and
  advances atomically to `verified` only when both steps succeed. This covers
  interruption after rename but before the original parent synchronization.
- Failed public verification leaves the record `published_unverified`, keeps
  all paths, and remains retryable. A transient read failure must not force a
  terminal `failed` state or delete evidence.
- An already `verified` record is independently rechecked before recovery
  reports a verified package. A failed recheck never downgrades that terminal
  state. A `failed` record is classified without inspecting or adopting its
  destination. Recovery never deletes a public destination.
- A leftover update lock blocks the resumed state transition and is preserved
  for inspection. An unrelated temporary state file is also preserved, but it
  does not by itself prevent classification or an otherwise safe transition.
- Concurrent recovery callers may observe lock contention or a stale expected
  state, but at least one can advance an intact `published_unverified` record;
  later retries converge on the independently reverified `verified` result.

## Consequences

Every end-to-end export snapshot root is classifiable even when preparation is
interrupted. Restart code cannot silently claim a valid package created by a
racing operation, and it cannot report success from a durable state alone when
the public package is no longer verifiable.

Automatic deletion of interrupted or failed private work remains deferred.
Cleanup requires a separately reviewed ownership policy so an arbitrary
directory containing a copied state file can never be recursively removed.
