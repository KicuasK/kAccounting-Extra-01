# ADR 0020: Durable private transfer work state

Status: accepted by the conservative transfer policy and implemented as a
backend primitive

## Context

Transfer lifecycle states were enforced in memory, but process termination
could leave private snapshot or package work without a machine-readable record.
Recovery code needs a strict durable record that cannot move backward or be
silently overwritten by a stale caller.

## Decision

- Store a private `operation-state.json` with format version, operation kind,
  lifecycle state, and an absolute, NUL-free UTF-8 destination path.
- Require its work root to be a real non-symlink directory. ADR 0021 adds the
  owner-only Unix permission checks used when this record becomes authoritative
  for restart recovery.
- Create the initial `working` record with create-new semantics and owner-only
  permissions. Never replace an existing record during creation. Synchronize
  both the work root and its parent so the new root and record are inside the
  initial durability boundary.
- Decode strict JSON, reject unknown fields, unsupported versions or operation
  kinds, relative destinations, symlinks, non-regular files, and documents over
  64 KiB. Read through one opened file and, on Unix, require that descriptor and
  the immediately re-inspected pathname to have the same device and inode
  identity, so replacement after opening fails closed.
- Require every update to name the expected current state and obey the existing
  monotonic lifecycle: `working` to `published_unverified` or `failed`, then
  `published_unverified` to `verified` or `failed`.
- Serialize writers with an owner-only create-new lock file and synchronize the
  directory entry before updating. Write the new record to a synchronized
  private temporary file, atomically replace the old record, and synchronize
  the work directory. If replacement succeeds but this synchronization fails,
  report persistence as uncertain instead of claiming a durable transition.
- A successfully persisted state remains successful if later lock cleanup
  fails; surface that cleanup as a warning instead of encouraging a retry.
- Rejected creation, decoding, or transition must preserve the existing root
  and state bytes. A failed updater may remove only the lock it created; it
  never rewrites corrupt or oversized evidence and never removes a lock that
  prevented it from starting.

## Consequences

Interrupted work can now be classified without guessing from partial file
names, and stale or concurrent state writers cannot move a terminal operation
backward. A crash may intentionally leave the update lock or temporary file;
ADR 0021 preserves those objects as evidence rather than deleting them
automatically.

ADR 0021 integrates this record into the end-to-end export at the actual
publication boundaries and defines restart classification. Automatic cleanup
of interrupted or failed work remains deferred until private-directory
ownership can be proven without trusting a copied state file.

Version 1 can classify a persisted `failed` operation but does not store its
diagnostic. A caller that advances to `failed` must preserve an actionable
error separately; restart classification alone does not yet satisfy ADR 0007's
requirement that a failed operation carry such an error. The durable failure
detail format and the points that should persist it require a separate reviewed
decision.
