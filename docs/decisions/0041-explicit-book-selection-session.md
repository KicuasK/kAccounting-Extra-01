# ADR 0041: Explicit, non-persistent book selection

Status: implemented foundation, 2026-10-22.

The user requires a book choice whenever the application opens. A successful
import or restore must retain the current book until a separate explicit choice.
This refines the prior restore wording in VERSION01_DECISIONS section 7.

A fresh `BookSelectionSession` contains no book and does not load a remembered
path. There is no persistent active-book setting. Listing uses a supplied private
collection directory and inspects only direct children with a recognizable public
binding, private database file and private attachments directory. It does not
open SQLite, create files, inspect financial metadata or call any key store.
A candidate contains only the exact directory-name string and the status
`selection_required`; listing is not proof that a key, lock or valid schema is
available. Unknown, hidden, inaccessible and unsafe candidates are excluded.
The collection itself must be safe; collection errors are reported.

Explicit selection uses RuntimeBook's full startup verification, recovery,
migration-backup and exclusive lock boundary. The replacement must open
successfully before the previous connection pool is closed or its lock released.
Any failure retains the old selection. Selecting the already open canonical path
is idempotent. Import and restore services have no reference to the selection
session and cannot switch it implicitly.

The discovery identifier is an exact string, including digit-only names beyond
JavaScript's exact integer range. It is not a database identifier or a key ID.
Do not join an untrusted ID into a filesystem path without validating that it is
a single direct-child component; consumers can instead pass an explicitly chosen
absolute path to `select`.

No HTTP picker or UI is provided by this increment. CLI startup remains an
explicit path choice; future picker integration must not start a book-backed
HTTP router before selection. Runtime routers contain cloned database handles:
a caller must stop/drain an old router before switching its session, and build
a replacement router from the new current book. This session alone does not
perform hot switching of a running server.

The login generator now prepares only `books <private-collection>` discovery,
with no saved book, key loading or `serve` command. Legacy `--book`/`--assets`
arguments are rejected. Until the chooser UI exists this is a diagnostic
candidate listing, not a complete graphical login workflow. Nothing is installed
or started by the generator or its tests.
