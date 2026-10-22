# Book selection foundation

Approved operation (2026-10-22): choose a book each time the application opens;
importing or restoring a new destination leaves the current book selected.

`book_selection::discover_books(private_collection)` lists only recognizable
existing direct-child books without opening databases or accessing Keychain.
The collection and candidates must be private paths outside the source checkout,
without symlinks. Candidate JSON contains exact string `id` and string `status`
(`selection_required`), with no key identifiers or financial metadata. Listing
is advisory: only selection checks key availability, exclusive lock, full database
verification and recoverability. Unrecognized/unsafe candidates are omitted;
a malformed collection path returns an error. Discovery never creates a book.

Use `BookSelectionSession::new()` for each application session. `current()` is
initially `None`, even for a collection containing one book. Call
`select(absolute_path, key_store).await` only after the explicit choice. Failure
preserves the current book and its lock. A successful change opens and verifies
the replacement before closing the previous book. `close().await` closes the
session and releases its lock. Selecting the same path is idempotent.

Do not persist or restore the current choice automatically. Do not connect
successful import/restore callbacks to `select`: offer a separate explicit
selection operation. No financial information is exposed before selection.

This is a backend model, not a picker UI or hot-swappable HTTP service. Drain a
running book-backed router before a session switch and create the next router
from the new current book. Existing explicit CLI path startup satisfies explicit
selection for that process; an application launcher must show its future picker
on every launch rather than remembering that path.

Tests use disposable books and in-memory device keys only. No real Keychain or
financial data is needed.
