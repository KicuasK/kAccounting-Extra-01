# Explicit local book session

The session server starts at `127.0.0.1:3000` with no selected book, even if one
candidate exists. Startup and discovery do not read Keychain. The locally
configured collection must be private, outside source, and contain only safe
direct-child candidates. Discovery checks the public key binding but does not
open SQLite or read any encryption key.

- `GET /api/session/books`: safe candidate IDs and selection-required status.
- `GET /api/session`: current ID or `null`, plus `selection_required`.
- `POST /api/session/selection`: JSON `{ "book_id": "direct-child-name" }`.
- `DELETE /api/session/selection`: explicitly close current book.

IDs are exact direct-child names, not filesystem paths. Missing or unsafe IDs
never trigger key access. The server rejects extra JSON fields. Opening and
verification failures preserve the previous selection. Successful import and
restore do not switch books. Restarting the process forgets the selection.

Accounting, attachment, backup and storage routes require selection and return
409 `selection_required` before one exists. Static assets and session routes
remain available. The exact Host/Origin/fetch-source guard covers every route,
including unknown paths and static assets. HTTP cannot register arbitrary paths:
only trusted native integration can register a capability after OS selection.
There is no file picker or book-selection UI in this increment.

## Switching safety

Every data response holds an owned read lease until its body is dropped or fully
consumed. Selection and deselection use a nonblocking write lease and return 409
`book_busy` while requests or streaming bodies hold the book. Owned storage jobs
also block switching after their admission response has ended. The process lock,
database pool and keys remain alive throughout. Replacement is opened and its
routers constructed before the old book is closed. Selection transitions run as
owned tasks so disconnecting a client cannot cancel migration/open/close halfway.

Graceful server shutdown waits for responses and storage jobs before closing the
pool. Capability IDs belong only to the selected session and must be selected
again after switching. No remembered book or auto-switch policy is introduced.

## Validation

`cargo test --locked --manifest-path backend/Cargo.toml --test session_runtime --test runtime`
uses disposable private temporary books and memory keys. Tests cover zero key
reads before selection, one-candidate startup, data-route gating, exact local
boundary, traversal rejection, explicit close, body lifetime busy behavior,
replacement failure preservation, and explicit switching. No real books,
Keychain data, or external services are accessed.

## Preparing a login launcher

`kaccounting-backend serve-chooser <private-collection> <frontend-dist>` starts
only the fixed loopback session. It neither opens a remembered book nor opens a
browser. The frontend currently remains read-only and does not yet provide the
selection controls; the session HTTP contract above is available for acceptance.

The launcher preparer creates a disabled plan in an existing user-owned mode-700
staging directory outside source and outside LaunchAgents/LaunchDaemons. It does
not create that directory. Choose a new output name. In the example below,
`PRIVATE_PLAN_DIRECTORY` means that already prepared private directory:


```sh
python3 scripts/make-launch-agent.py \
  --binary "$PWD/target/release/kaccounting-backend" \
  --books-root "$HOME/Library/Application Support/kAccounting-Extra-01" \
  --assets "$PWD/frontend/dist" \
  --output "$PRIVATE_PLAN_DIRECTORY/local.kAccounting-Extra-01.plist" \
  --preflight
```

`--preflight` prints readiness JSON and executable fingerprint without writing
anything or running the binary. Omit `--preflight` to create a new mode-600 plist
with `Disabled: true`; neither operation installs a LaunchAgent, invokes
`launchctl`, starts a server, or opens a book. A shared `/private/tmp` output
parent is not accepted; use its own private mode-700 staging subdirectory. `--assets` selects `serve-chooser`;
omitting it retains key-free `books` discovery. The deprecated `--book` option
and option abbreviations are rejected. Asset paths must be absolute, without
symlinks, and contain a regular `index.html`. Paths containing spaces remain
individual argument values, with no shell intermediary. Existing plist output
is never replaced. Actual login acceptance remains pending in a disposable
macOS account; preparing the plist is not proof of that acceptance.

Launcher tests: `python3 -m unittest discover -s scripts -p test_launch_agent.py -v`.


See [installation readiness](INSTALLATION_READINESS.md) for descriptor-based
output creation, private log checks and limitations. Actual installation,
login-session behavior and locked-Keychain acceptance remain unverified.

## External-media release scope

Physical external APFS/exFAT qualification is deferred to Version 02. Existing
portable encrypted transfer, integrity and conservative recovery implementations
remain available for controlled tests; their code is retained. This deferral
does not change Version 01's ordinary private local Mac storage, required exact
accounting invariants, or the rule that real financial books are outside
development tests. No physical-media qualification pass is claimed.
