# Independent non-UI security and accounting review

Date: 2026-10-22. Scope: this development increment, not a security certification.

The review used disposable temporary books, in-memory fake device/recipient key
stores and no network. It did not access real books or the macOS Keychain.

## Findings addressed during the increment

- Recipient import now independently verifies the staged and published database,
  complete attachment catalog, every reencrypted attachment, exact key binding
  and authenticated backup identity. Uncertain publication preserves evidence
  and reports its destination instead of pretending the destination is absent.
- Recipient import holds an OS operation lock through an owned asynchronous
  task. Advisory restart inspection skips active work and cannot authorize
  deletion, publication or automatic book selection. Source-tree ancestors and
  package path aliases are rejected.
- Authenticated import restart recovery was subsequently added and reviewed.
  Separate encrypted initial/ready contexts bind the existing local key ID,
  operation, package, destination and parent/work filesystem identities. Only
  authenticated abandoned work may be removed; a present destination requires
  the authenticated ready database hash, complete catalog, all local objects,
  binding and backup identity to verify. A destination process lock and rejection
  of pending WAL/journal content protect against verifying a running/modified
  book through its stale main database file. Owned tasks retain operation locks
  through worker completion, including requester cancellation.
- Runtime opening now owns its process lock through the full migration/SQLite
  operation even if the selecting caller disconnects.
- Export recovery previously held its work lock in the requesting future while
  nested blocking verification could outlive cancellation. Recovery now runs in
  an owned task, retaining that lock until verification workers complete. The
  independent review added the wrapper and a cancellation regression test.
- The new pre-migration restore command initially required successfully opening
  its damaged source database. It now loads only the validated existing binding
  and key under a source process lock; interrupted scratch is preserved. The
  implementation owner's six command tests passed, including recovery with a
  corrupt source database and interrupted migration work, exact source/evidence
  preservation and successful opening of the independently restored new book.

## Independent cross-service acceptance

`backend/tests/non_ui_acceptance.rs` creates a real disposable runtime book,
posts an amount of `9007199254740993` minor units, adds an attachment and posts
an explicit reversal. It exports and imports the complete encrypted package,
then checks:

- Exact posted entries, timestamps and reversal relationship survive transfer.
- SQLite debit and credit sums remain exact above JavaScript's safe integer
  range.
- Posted entry/line updates and deletes, attachment metadata updates and posted
  attachment deletes remain rejected by the restored database triggers.
- Imported attachment ciphertext changes while downloaded plaintext matches.
- A bit-flipped published local attachment prevents authenticated recovery
  cleanup even when the database, catalog and ready context still match. Work
  and the selected source remain intact. Restoring the exact original
  ciphertext permits full independent destination verification and work cleanup.
- Import retains the selected source book; selecting the destination requires a
  separate explicit action, and a new session always begins without a book.

The independent test passed. The targeted combined run also passed all 30 tests
in `api_attachment_streaming`, `book_selection`, `migration_backup`,
`non_ui_acceptance`, `recipient_import` and `recipient_package`. One separate
250 MiB streaming acceptance test was intentionally ignored by that ordinary
debug run; this review does not count it as executed. Subsequent full verifier,
Release/capacity and real-socket results belong in the main progress record.

The new unit regression
`cancelled_recovery_request_keeps_its_owned_operation_alive` passed: dispatching
recovery and dropping the requesting future still completes authenticated
abandoned-work cleanup without creating a publication.

After the authenticated import recovery and damaged-source restore fixes, the
reviewer independently reran `recipient_import` (9 passed),
`non_ui_acceptance` (1 passed) and `transfer_commands` (6 passed). These include
missing-key/tampered-context/forged-status preservation, authenticated abandoned
cleanup with source/key retention, published-book verification, modified/open
book preservation and damaged-source checkpoint restore.

## Full 250 MiB recipient-package capacity acceptance

The reviewer added and executed the separate ignored test
`maximum_recipient_package_roundtrips_with_bounded_plaintext_memory`:

```text
cargo test --release --locked -p kaccounting-backend --test non_ui_acceptance maximum_recipient_package -- --ignored
1 passed; 0 failed; test execution 11.62 seconds
```

It generated the exact 250 MiB attachment in 64 KiB writes, encrypted it into
the disposable source book, and removed the source plaintext fixture before
package export. It exported and imported the complete recipient package using
fresh device-key IDs and key bytes, checked ciphertext descriptor rotation,
then incrementally hashed the downloaded plaintext without accumulating a full
attachment buffer. Every received chunk was at most 64 KiB. Final SHA-256 and
byte length matched the original input. Exact source database and source
ciphertext SHA-256/length descriptors remained unchanged after export, import
and download. All fixtures used fake in-memory keys and were removed afterward.

## Actual process-death restart acceptance

The ignored parent test
`process_death_recipient_export_recovers_published_package_and_preserves_source`
launches the same test executable with an exact ignored child-helper filter.
The child creates a disposable source book with a 64 MiB attachment generated
in 64 KiB chunks, removes its plaintext input, checkpoints the database, records
only source descriptors and starts export. The parent waits for the actual
published package name, kills only that spawned child, confirms SIGKILL (signal
9), and waits for its exit. A guard kills/waits on failure as well. A separate
caller then runs authenticated export restart recovery under the same fixed
fake test key, which was never serialized or placed in Keychain.

Recovery must independently verify the published package, clean exactly one
interrupted work directory and preserve the package. Source database and local
ciphertext SHA-256/length descriptors must remain unchanged. A read-only source
catalog and incremental download hash/length must match the original generated
64 MiB receipt. The test has a 30-second publication deadline and is excluded
from ordinary test runs.

```text
cargo test --release --locked -p kaccounting-backend --test non_ui_acceptance process_death_recipient_export_recovers -- --ignored
Three consecutive actual process-death runs passed: 1.84 s, 1.78 s, 1.78 s.
```

## Remaining boundaries

- Import status JSON remains advisory and cannot authorize mutation. Explicit
  authenticated restart recovery verifies unchanged published books or removes
  abandoned owned staging; it does not resume publishing an incomplete stage.
  Retry the unchanged original package to a fresh destination after cleanup.
- Interrupted migration work deliberately blocks normal book opening and can
  contain owner-only plaintext SQLite scratch. It must not be silently removed.
- Filesystem identity checks and exclusive publication reduce path replacement
  risks; unsupported concurrent modification by another process with the same
  user's filesystem/Keychain authority is not prevented universally.
- Streaming code uses bounded channels and authenticated object chunks; it
  creates no plaintext attachment intermediate. Four genuinely stalled uploads
  may occupy the four configured crypto workers until clients disconnect. Early
  metadata/signature rejection releases its reader and permit. Inactivity policy
  and browser interaction acceptance remain separate product/runtime work.
- macOS Keychain behavior, external-drive filesystem compatibility and Mac
  Safari end-to-end operation were not independently exercised by this review.

No accounting classification decision was inferred. No commits or pushes were
made by the reviewing agent.
