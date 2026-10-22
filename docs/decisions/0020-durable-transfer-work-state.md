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
  lifecycle state, and absolute UTF-8 destination path.
- Create the initial `working` record with create-new semantics and owner-only
  permissions. Never replace an existing record during creation.
- Decode strict JSON, reject unknown fields, unsupported versions or operation
  kinds, relative destinations, symlinks, non-regular files, and documents over
  64 KiB.
- Require every update to name the expected current state and obey the existing
  monotonic lifecycle: `working` to `published_unverified` or `failed`, then
  `published_unverified` to `verified` or `failed`.
- Serialize writers with an owner-only create-new lock file. Write the new
  record to a synchronized private temporary file, atomically replace the old
  record, and synchronize the work directory.
- A successfully persisted state remains successful if later lock cleanup
  fails; surface that cleanup as a warning instead of encouraging a retry.

## Consequences

Interrupted work can now be classified without guessing from partial file
names, and stale or concurrent state writers cannot move a terminal operation
backward. A crash may intentionally leave the update lock or temporary file;
future recovery logic must treat those as evidence requiring inspection rather
than deleting them automatically.

The next step is integrating this record into the end-to-end export at the
actual publication boundaries and adding restart/recovery classification tests.
