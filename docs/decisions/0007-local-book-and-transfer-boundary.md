# ADR 0007: Local live book and application-managed transfer boundary

Status: accepted

## Context

Normal accounting work happens on one Mac. Large-volume attachments are a
Version 01 requirement and must remain consistent with the SQLite book. An
external drive is used only when moving the book to another computer. The
supported copy must be performed by the application so that integrity,
compatibility, and destination conflicts can be checked.

The user selected a macOS directory package for the physical `.kagextra`
container. Finder may present the package as one document after the future app
registers the extension, but storage and recovery code must continue treating
it as a directory containing multiple files.

## Decision

- The live book resides on local Mac storage outside the source repository.
- The live-book boundary contains one SQLite database, an attachment store,
  and private work state needed to recover or discard incomplete operations.
- Removable storage is not a supported live-book location in Version 01.
- `.kagextra` is the user-visible transfer artifact produced and consumed by
  the application. It is a versioned macOS directory package; Finder copying is
  not the supported transfer workflow.
- Export must take a consistent database snapshot, bind every referenced
  attachment to a versioned manifest, publish without overwriting an existing
  destination, synchronize every package file and directory, synchronize the
  destination parent after publication, and verify the published artifact
  before reporting success.
- Import must validate format compatibility, database identity and integrity,
  manifest completeness, attachment sizes and content hashes before publishing
  a new local live book.
- Interrupted, capacity-limited, or failed work remains distinguishable from a
  published artifact and may be cleaned up later without deleting a valid
  source or destination.
- The application never automatically deletes the old computer's live book
  after transfer.

## Required observable states

An export or import is one of:

1. `working`: private and never offered as a valid book;
2. `published_unverified`: not yet reportable as success;
3. `verified`: safe to report as complete;
4. `failed`: carries an actionable error while preserving valid data.

The lifecycle is monotonic: `working` may become `published_unverified` or
`failed`; `published_unverified` may become `verified` or `failed`; both final
states are terminal. An operation may not skip verification or return to
working after a terminal result.

If final cleanup fails after verification, the operation remains successful and
records a cleanup warning. A caller must not retry a completed transfer merely
because cleanup of private work state failed.

## Deferred decisions

- attachment identifier format (content integrity uses SHA-256 per ADR 0008);
- attachment ownership, allowed types, limits, deduplication, and retention;
- cryptographic format and key management (encryption at rest is required by
  the accepted Version 01 product decision);
- supported removable-drive file systems;
- backup schedule and retention.

## Consequences

Storage code must separate creation, publication, verification, and cleanup so
that post-publication cleanup cannot turn a successful result into an ambiguous
failure. Tests must inject failures at each boundary before the transfer flow is
used by an API or UI.

ADRs 0010–0021 implement the read-only package verifier, conservative export,
durable export state, and restart classification for an explicit attachment
set. They do not yet implement live-book import replacement, prove attachment
reference completeness, select removable-drive compatibility, or expose a
transfer API or UI.
