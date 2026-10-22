# Private local runtime

The runtime is an integration preview, not a Version 01 release. The frontend
still provides only the read-only account tree. The exact-money accounting
write API is available to same-origin requests; the journal editor and full
Safari acceptance remain pending. Do not select a real financial book for
development or acceptance tests.

## Build and explicit initialization

Build source and frontend without creating a financial book:

```sh
cargo build --release --locked
npm --prefix frontend run build
```

The approved runtime location is
`~/Library/Application Support/kAccounting-Extra-01/`. Its parent and new book
directories must be private. Initialization is an explicit command and creates
a new empty book plus a non-synchronizing macOS Keychain key. It does not seed
the unresolved handwritten chart of accounts. Never run initialization over an
existing book. Opening a book never creates a replacement key.

```sh
mkdir -p -m 700 "$HOME/Library/Application Support/kAccounting-Extra-01"
./target/release/kaccounting-backend init-new "$HOME/Library/Application Support/kAccounting-Extra-01/book"
./target/release/kaccounting-backend serve "$HOME/Library/Application Support/kAccounting-Extra-01/book" "$PWD/frontend/dist"
```

Open `http://127.0.0.1:3000/` in Mac Safari. Use that exact address; `localhost`
and other hosts are rejected. The listener is fixed to loopback and cannot be
changed through a command argument. Stop with Control-C. A process lock prevents
two application instances from opening the same book.

## Startup and recovery

Startup reads the existing public device-key binding and loads the exact
existing Keychain item. Missing, malformed or unavailable keys stop startup.
The current schema is audited before writable opening. An older schema receives
an independently verified encrypted pre-migration checkpoint before the live
schema is upgraded. Failed checkpoint verification stops writable opening.

Recognized abandoned private backup work is recovered conservatively. Unknown
work is preserved and may block startup. Do not delete private work or Keychain
items merely to make an error disappear. A failed initialization may leave
private staging and an existing Keychain key for diagnosis.

## Login launcher preparation

The generator validates and prepares a **disabled plan**, never installs or
starts it, modifies Tailscale, or contacts Keychain. Every start requires an
explicit book choice. With `--assets`, the plan describes `serve-chooser`, which
starts a loopback session with no selected book. Without assets it describes the
key-free `books` discovery command. The frontend does not yet offer the selection
controls, so this is readiness tooling rather than a complete graphical login
workflow.

Select an existing user-owned mode-700 staging directory outside this repository
and outside LaunchAgents/LaunchDaemons. The preparer does not create parents.
`PRIVATE_PLAN_DIRECTORY` below denotes that directory; a direct output under the
shared `/private/tmp` parent is rejected.

```sh
python3 scripts/make-launch-agent.py \
  --binary "$PWD/target/release/kaccounting-backend" \
  --books-root "$HOME/Library/Application Support/kAccounting-Extra-01" \
  --assets "$PWD/frontend/dist" \
  --output "$PRIVATE_PLAN_DIRECTORY/local.kAccounting-Extra-01.plist" \
  --preflight
```

`--preflight` prints the fixed label, launch mode, executable fingerprint and
`disabled: true`, `installed: false`, `started: false` without writing files or
executing the binary. Omit it to exclusively create a new mode-600 plist with
`Disabled: true`; existing output is preserved. The output parent is opened via
no-follow directory descriptors and the plan plus parent are synced. Existing
logs must be private user-owned regular files with one link. No log is opened or
created during preparation.

The deprecated fixed `--book` argument and option abbreviations are rejected;
`--assets` is supported for an unselected session. Keep the binary location
stable; its recorded fingerprint is a preflight snapshot, not an at-launch pin.
Installation, enabled login behavior and locked-Keychain acceptance remain
separate unverified work. This preview does not enable private Tailscale Serve.
See [installation readiness](INSTALLATION_READINESS.md) and
[explicit sessions](SESSION_RUNTIME.md).

### External-media release scope

Physical external APFS/exFAT qualification is deferred to Version 02; portable
transfer/media/recovery code remains in place with its automated test coverage.
The OS-denied disk-image attempt is not a pass. Version 01 still uses private
local Mac storage and preserves all accounting, encryption, no-overwrite and
explicit-selection safeguards. This does not authorize testing real books.

## Explicit backup, restore, attachment collection, and recipient requests

These commands access the selected book and local Keychain only when explicitly
invoked. They are not run by the build or tests. Stop the running application
before backup or collection; their exclusive book lock refuses a concurrently
open book.

```sh
./target/release/kaccounting-backend backup "$HOME/Library/Application Support/kAccounting-Extra-01/book"
./target/release/kaccounting-backend restore /absolute/path/to/backup.kabackup /absolute/private-parent/new-restored-book
./target/release/kaccounting-backend setup-backup-identity /absolute/private-parent/new-restored-book
./target/release/kaccounting-backend collect-attachments "$HOME/Library/Application Support/kAccounting-Extra-01/book"
./target/release/kaccounting-backend recipient-request /absolute/private-parent/new-recipient-request.json
```

`backup` creates and independently verifies an encrypted package beneath the
fixed `book/backups/` directory, then retains the newest ten verified backups
for this book's authenticated backup identity. Unknown, foreign, incomplete,
and corrupt items are preserved. The printed package path identifies the
`backup.kabackup` directory accepted by `restore`. A retention warning after
successful verification is reported separately and does not hide the finished
backup. A missing backup identity fails closed without creating a replacement.

`restore` requires a new destination and verifies the complete database and
attachment set. It preserves the source backup, original book, and active-book
selection. The local backup format currently restores the existing device-key
binding but does not carry the managed-backup identity. Such a restored book
can open. Run `setup-backup-identity <restored-book>` explicitly to create a new
managed-backup namespace, then `backup <restored-book>`. Setup uses the existing
bound device key, refuses an existing identity, and never replaces key material.

`collect-attachments` removes only recognized private ciphertext objects absent
from the selected book's committed attachment table. Unknown entries remain.
Cross-process store pins held by readers, snapshots, exports, or backup workers
prevent collection. It never deletes the active book or referenced attachments.

`recipient-request` creates a separate non-synchronizing recipient Keychain key
and writes only the public request JSON to a new exclusive `0600` file. It prints
the request ID and public fingerprint. Parent directories must already be
private, absolute, outside the source tree, and free of symlink components.
The request is a prerequisite for destination-bound transfer; this command
alone does not export or import a complete `.kagextra` package. Interrupted
request publication can retain a Keychain key; no secret is written to disk or
silently deleted.


## Explicit book discovery and recipient transfer commands

Each process startup requires an explicit book path. There is no last-book
setting or automatic choice, even if only one candidate exists. Discover books
without opening SQLite or accessing Keychain:

```sh
./target/release/kaccounting-backend books /absolute/private-books-parent
```

Output is JSON with exact string `id` and `status: "selection_required"`.
The ID is the existing direct-child directory name. Unknown, staging, symlink,
non-private or malformed candidates are excluded. Discovery does not prove that
the book's key, lock or schema will pass opening checks. It creates nothing.

After the destination Mac creates its public recipient request, the source Mac
can explicitly export a verified encrypted package:

```sh
./target/release/kaccounting-backend export-recipient /absolute/existing-book /absolute/private/request.json /absolute/private/new-transfer.kagextra
./target/release/kaccounting-backend import-recipient /absolute/private/new-transfer.kagextra /absolute/private/request.json /absolute/private/new-imported-book
```

The request file must be a private regular file, at most 4096 bytes, outside the
source checkout and without symlink components. Export displays the selected
request ID and public-key fingerprint before any export. This maintenance CLI
asks the operator to enter the displayed request ID after comparing the request
with the destination Mac; EOF or a mismatch cancels. This is a CLI confirmation
mechanism, not a password or a Version 01 UI code-entry policy. The future UI
should use its normal explicit confirmation action while showing both public
identifiers. The parsed immutable request is used directly, avoiding a second
file read after confirmation.

Import uses the destination Mac's existing recipient private key, creates a new
local device binding and managed-backup namespace, verifies the complete package
and publishes only to a new destination. An existing destination is never
replaced. Successful import or restore leaves the current book selected; opening
the new book requires a separate explicit `serve` selection. Source books,
recipient keys and original packages are retained.

Conservative recovery/status commands:

```sh
./target/release/kaccounting-backend recover-recipient-exports /absolute/existing-source-book /absolute/existing-source-book
./target/release/kaccounting-backend import-status /absolute/private-import-parent
./target/release/kaccounting-backend recover-recipient-imports /absolute/private-import-parent
./target/release/kaccounting-backend restore-pre-migration /absolute/existing-book /absolute/private/pre-migration-OPERATION_ID.kabackup /absolute/private/new-restored-book
```

`recover-recipient-exports` uses the source book key to inspect its durable export
work. In the current portable exporter layout, the second argument is the same
private source book root, where source-local work lives; it is not the external
destination folder. It reports completed cleanup, verified destinations, active work and
unrecognized preserved work separately. It never switches books.
`import-status` only reads retained durable import evidence; it does not resume,
publish or delete work. Preserve interrupted import work and keys for diagnosis.
`recover-recipient-imports` loads existing local keys and authenticates initial
and READY contexts. It cleans only verified application-owned unpublished work
or independently verified published imports. Active, modified, unrecognized or
uncertain work/destinations are preserved. It never selects or publishes a book.
Both export recovery and checkpoint restore load only the selected source's
existing binding/key under its process lock; a damaged live database or retained
migration scratch does not block recovery. No replacement key is generated.
The restore command authenticates the pre-migration checkpoint against the
explicit existing book's device binding and publishes a verified new book.
It does not replace the source or select the destination.

All commands opening an existing book hold its exclusive runtime lock and close
the database after success or failure. Stop the server before maintenance.
No command enables a public listener or Tailscale Funnel.
