# Local installation readiness

2026-10-23: `scripts/check-macos-signature.py --binary <absolute-executable>
--identifier <expected-identifier> [--team-id <expected-team>]` performs read-only
codesign verification. With an expected team it also tests the Apple anchor,
Developer ID Application certificate marker, exact team and identifier. Ad-hoc
validity is distinct from Developer ID qualification; hardened runtime and the
expected identity/team are required. It does not sign, query keys, install, start,
notarize or establish Keychain/login/proxy acceptance. Exit 0 qualifies the
signature, 1 means unqualified, 2 means refused/unavailable. The user currently
has no certificate and selected keeping the development bundle. See
[latest build](BUILD_TAKEOVER_2026-10-23.md).

This project can prepare and validate a **disabled login-launcher plan**. It does
not install a LaunchAgent, modify login settings, execute the selected program,
open a browser, read keys, or open a book. Actual login acceptance remains pending
in a disposable macOS user account. Preparing a plist is not login acceptance.

## Read-only preflight

Use an already built Release executable and frontend assets. Choose an existing
private (mode 700), user-owned directory outside this repository for plan output.
For example, a new temporary directory created with `mktemp -d` is appropriate.
The output itself must be a new `.plist` file, and may not reside in a directory
named LaunchAgents or LaunchDaemons. The preparer does not create parents.

```sh
python3 scripts/make-launch-agent.py \
  --binary "$PWD/target/release/kaccounting-backend" \
  --books-root "$HOME/Library/Application Support/kAccounting-Extra-01" \
  --assets "$PWD/frontend/dist" \
  --output "$PRIVATE_PLAN_DIRECTORY/local.kAccounting-Extra-01.plist" \
  --preflight
```

`PRIVATE_PLAN_DIRECTORY` above denotes the private staging directory chosen for
this run. No files are written by `--preflight`, including logs. The JSON result
records the fixed label, `serve-chooser` or `books` mode, disabled state, and
SHA-256/size/device/inode of the executable inspected. It reports `installed:
false` and `started: false` and never runs the executable to identify it.

Omit `--preflight` with the same arguments to write the new mode-600 plan. The
parent is opened by no-follow directory descriptors, creation is exclusive and
relative to the verified parent descriptor, and file plus parent are synced.
Existing destinations, including dangling symlinks, are preserved. Partial output
from an I/O failure may remain for inspection; retry requires a new output name.

## Safety boundaries

- The label is fixed to `local.kAccounting-Extra-01`; no label or shell argument
  interpolation is accepted. Paths with spaces remain separate plist arguments.
- All prepared plans contain `Disabled: true`. `RunAtLoad: true` describes the
  eventual intended login behavior only; this disabled plan is not enabled or
  installed by the tool. Enabling/installing is a later explicit operation.
- `--assets` prepares `serve-chooser`, which starts with **no selected book**.
  Omitting assets retains key-free candidate discovery. Restart never remembers a
  book, and import/restore never auto-switch the selection.
- Books/output roots must be outside the source tree, privately owned, and mode
  700. Output within LaunchAgents/LaunchDaemons is rejected case-insensitively.
  Output parents and existing paths may not traverse symlinks or `..`.
- The executable must be user-owned, regular, executable by its owner, and free
  of group/world write and set-ID bits. Descriptor identity is checked while
  hashing. Built source-tree executable/assets are permitted; runtime books and
  prepared output are kept separate from source.
- Existing stdout/stderr logs must be user-owned private regular files with one
  link. Symlinks, hardlinks and public files are rejected. Missing logs are not
  created during preparation. No log contents are read.
- Existing frontend assets must contain a regular no-symlink `index.html`.
  Asset construction and runtime route guards retain their own validation.

The executable fingerprint is a readiness snapshot, not code signing or an
at-launch pin. A binary, directory, or log changed after preparation must be
validated again before a future installation. The tool does not claim to defend
an eventual installed job against arbitrary changes by the same local user.
Private network access, Safari workflows, and real Keychain/login behavior have
separate acceptance requirements; this tool grants no new access to them.

## Verification

```sh
python3 -m unittest discover -s scripts -p test_launch_agent.py -v
```

Thirteen tests use disposable invented directories and harmless executable
fixtures. They cover disabled plans, no-shell arguments, old-option rejection,
read-only executable fingerprinting, non-execution, public/symlink/source output
rejection, preserved existing destinations, log hardlinks/symlinks, executable
permissions, and assets validation. No login service or real book is exercised.

## Transient launchd acceptance harness

On 2026-10-22 the user authorized a temporary background acceptance job, with no
permanent login entry and no financial/key reads. The separate
`scripts/login-acceptance.py` implements this bounded check. Its default mode
only prepares a concrete reviewable plan; it never calls `launchctl`. The
existing normal installation-plan generator remains `Disabled: true`.

Build the reject-only fake-key example, then prepare:

```sh
cargo build --locked --manifest-path backend/Cargo.toml --example login_acceptance
python3 scripts/login-acceptance.py \
  --binary "$PWD/target/debug/examples/login_acceptance" \
  --assets "$PWD/frontend/dist"
```

Preparation creates a unique private temporary directory, an **empty** books
collection, copied bounded regular frontend assets, private stdout/stderr logs,
`job.plist`, and `plan.json`. The copied index begins with a unique fixture token;
the original frontend assets remain unchanged. Nothing is put in LaunchAgents or
LaunchDaemons. No example or service is run during preparation. The output lists
the exact generated label, arguments, and proposed command for review.

After explicit operational approval, the root coordinator can execute that exact
prepared plan:

```sh
python3 scripts/login-acceptance.py --run --plan /absolute/private/fixture/plan.json
```

Execution verifies the executable SHA-256/device/inode snapshot, no-follow owned
private fixture files, exact generated label/root binding, copied index token,
unchanged plist, and still-empty books directory. An occupied loopback port 3000
or an existing exact test label aborts before bootstrap. The harness uses only:

- `launchctl print gui/<current-uid>/<unique-test-label>`;
- `launchctl bootstrap gui/<current-uid> <own-private-fixture>/job.plist`;
- `launchctl bootout gui/<current-uid>/<unique-test-label>` for verified cleanup.

There is no broad domain unload, persistent installation, enable/disable command,
production-label action, Keychain operation, browser opening, or book selection.
The transient test plist is enabled only for this explicit run, with no keepalive.
It does not modify the permanently disabled installation plan.

A successful startup must match both the launchd program/plist/full argument
identity and the exact fixture-token index before its HTTP responses count as
owned. The session must report `book_id: null`, `selection_required: true`, and an
empty candidate list. HTTP checks disable proxies and redirects and use only the
fixed loopback address. An unrelated server cannot satisfy the fixture proof.

Cleanup runs after success or failure, including a partially successful bootstrap,
but only after `launchctl print` proves the exact generated label, executable,
arguments, token and plist path. Unverified ownership causes a fail-closed error
instead of stopping a possibly unrelated job. The retained unique label/path is
available for inspection. After verified cleanup the harness requires fake-key
startup and shutdown receipts showing **zero calls**. The example has reject-only
fake device/recipient stores and never invokes the real OS adapters.

The fixture is retained for review. The script does not recursively delete files
or remove other jobs. Real current-user launchd execution requires the appropriate
macOS GUI context; sandbox/permission errors are failures rather than acceptance.

This tests transient launchd startup, no-book/no-key initialization, HTTP identity,
and removal of the isolated job. It does **not** simulate logout/login or reboot,
prove locked-Keychain behavior, install a persistent login service, or complete
those remaining manual acceptance gates. The actual run result must be recorded
separately; having the harness or passing mocked tests is not an OS acceptance.

Harness verification (all `launchctl` and HTTP calls mocked):

```sh
python3 -m unittest discover -s scripts -p test_login_acceptance.py -v
cargo clippy --locked --manifest-path backend/Cargo.toml \
  --example login_acceptance -- -D warnings
```

### Actual transient result — ongoing round checkpoint, 2026-10-22 20:08 Taipei

The coordinating, user-authorized run passed with `own_job_removed: true`,
`fake_key_calls: 0`, an empty collection and no selected book. Startup and shutdown
receipts for fixture `6e3baf3b21cf3186c74020f0d8f8348e` both record zero calls.
The unique temporary job was removed; no permanent login entry was installed.
See [ongoing round checkpoint](BUILD_NON_UI_45MIN_2026-10-22.md) for the retained
fixture and exact evidence. This qualifies transient launchd execution only;
actual logout/login, reboot startup and production signed-app access remain
separate gates. The requested 45-minute round is still ongoing at this checkpoint.
