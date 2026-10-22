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
The current schema is audited before writable opening. An older schema is
rejected until the backup-before-migration workflow is implemented; there is no
automatic migration without a verified checkpoint.

Recognized abandoned private backup work is recovered conservatively. Unknown
work is preserved and may block startup. Do not delete private work or Keychain
items merely to make an error disappear. A failed initialization may leave
private staging and an existing Keychain key for diagnosis.

## Login launcher preparation

The generator writes a new property list only. It does not install a LaunchAgent,
start a service, modify Tailscale, or contact Keychain. Run it after selecting a
verified book and building the release executable:

```sh
python3 scripts/make-launch-agent.py --binary "$PWD/target/release/kaccounting-backend" --book "$HOME/Library/Application Support/kAccounting-Extra-01/book" --assets "$PWD/frontend/dist" --output /private/tmp/local.kAccounting-Extra-01.plist
plutil -lint /private/tmp/local.kAccounting-Extra-01.plist
```

Installation and real login/locked-Keychain acceptance remain release work.
Keep the binary and asset locations stable before installing a launcher.
Private Tailscale identity/origin integration is still pending; a Tailscale
Serve deployment is not enabled by this preview.

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
