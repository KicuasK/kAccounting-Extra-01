# ADR 0038: bounded encrypted HTTP attachment streams

Status: accepted engineering implementation, 2026-10-22.

## Context

Version 01 accepts regular attachments up to 250 MiB and 20 GiB/book, including
explicit new attachments on posted entries. The private HTTP layer previously
exposed metadata, verification, and draft detachment only. Collecting attachment
bodies into JSON or a plaintext temporary file would defeat the existing bounded
crypto design and encryption-at-rest boundary.

## Decision

Use raw octet-stream uploads with strict URL query metadata, a two-slot 64 KiB
channel, and the existing synchronous authenticated encryption codec on a blocking
worker. An explicit channel EOF distinguishes complete input from a disconnected
request. The actual body limit is checked independently of Content-Length, and
publication reuses private ciphertext staging, full verification, SQLite quota
triggers, `BEGIN IMMEDIATE`, and exclusive durable rename.

Draft HTTP uploads require their current revision and bump it in the same
transaction. Explicit posted appends omit the draft revision and preserve the
posted entry and existing attachments. This implements the approved policy;
it creates no new accounting workflow or classification decision.

Downloads completely verify the selected open file before success headers, then
rewind and decrypt through a bounded channel from that same file handle.
Content-Disposition forces download; filenames are percent encoded and the
response is octet-stream/no-store/nosniff. A four-worker semaphore shared by
repository clones bounds the streaming crypto work, and disconnects terminate
channel work. There is no plaintext temporary file or 250 MiB application buffer.

Browser source metadata cannot certify Unix mode bits or original symlink state.
HTTP validates regular-file selection metadata, safe filename/package extensions,
and executable content signatures; local path imports retain mode/identity
checks. This limitation is explicit rather than claiming unobservable browser
metadata was authenticated.

## Evidence and limitations

HTTP tests exercise >1 MiB input, fragmented signatures, Unicode/header safety,
zero-length input, incomplete input and cancellation cleanup, declared-length
mismatch, actual/declared limits, stale concurrent drafts, exact quota rollback,
posted append immutability, and corruption before download. Separate ignored
Release acceptance incrementally generates and downloads an exact 250 MiB file,
then rejects an undeclared oversized stream without retaining a candidate.

Transport disconnection after a committed upload may hide a successful outcome;
clients should list attachments before retrying. Existing offline maintenance
handles crash-orphaned ciphertext conservatively. Corruption occurring after
preverification aborts the body; partial client bytes must be discarded. This
increment supplies the HTTP/storage contract; Safari UI and real-socket acceptance
are separate from in-process route tests.

Integration subsequently passed actual loopback chunked upload, incremental
download SHA-256/length verification, draft revision/posting and encrypted backup
including the streamed attachment. Mac Safari interaction remains pending.
