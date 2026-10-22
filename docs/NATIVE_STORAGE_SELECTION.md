# Trusted native storage selection bridge

Actual host evidence (2026-10-22 19:44:39): the user authorized the panel and
asked the integrating worker to operate it. Exact disposable-folder selection
and capability registration passed after `finishLaunching` and Darwin `realpath`
fixes. Direct path-field entry was used; automated clipboard paste remained
ineffective and is not claimed fixed. Save dialogs and broader operator flows
remain unverified. Earlier compile-only descriptions below are historical.

`native_selection::NativePicker` is a reusable native-host boundary. It creates
no HTTP route, listener, persistent IPC token, shell command, or raw-path API.
The existing Safari API continues to receive process-local capabilities only.

## Deployment and invocation

1. Compile `native/StoragePicker.swift` using
   `sh scripts/build-native-picker.sh /absolute/private/build-dir/StoragePicker.app`.
   This creates a complete regular AppKit bundle. Its trusted child executable is
   `StoragePicker.app/Contents/MacOS/StoragePicker`; use that path with `NativePicker`.
   A trusted optional `--bundle-id` distinguishes isolated acceptance builds.
   This compiles only; it never opens a panel and refuses an existing output.
2. Install the binary in an owner-controlled private location outside the source
   checkout. No installation or panel invocation has been performed by this work.
3. The trusted native host supplies that canonical executable path and its
   expected numeric owner UID to `NativePicker::pin`. Never source either value
   from browser requests. The executable must be regular, executable by its
   owner, at most 16 MiB, not group/world writable; its immediate parent must be
   an owner/root-owned directory without group/world write access. Symlink paths
   and source-checkout executables are rejected. Pinning records parent identity,
   executable device/inode/mode/owner and a streaming content digest. Parent owner,
   mode and canonical executable path are rechecked on every invocation.
4. After an explicit user action and permission to open an OS panel, call
   `picker.select_and_register(&session, kind).await`. Only this trusted native
   action invokes the child. There is intentionally no web-triggered picker.

The adapter snapshots the selected-book epoch before opening a panel and calls
`register_storage_selection_for` afterward. Switching away and back, deselecting,
or otherwise replacing that epoch invalidates the result. The selected path
stays inside the native process; successful registration returns only the opaque
capability string. User cancellation returns `None` and registers nothing.

## Protocol and lifecycle

Rust launches the pinned executable directly with an empty environment, fixed
working directory, private piped stdin/stdout and suppressed stderr. It sends one
JSON request containing only the exact format, a fresh 256-bit random nonce and
a typed kind. The child accepts at most 1024 bytes. It returns one JSON response
with matching format/nonce/kind and either `selected` with one absolute canonical
path or `cancelled`. Rust caps output at 16 KiB and rejects unknown fields,
wrong kinds/nonces, nonzero exit, unsafe paths and changed helper identity.
No secret keys or accounting records cross this protocol.

The child uses AppKit `NSOpenPanel` for existing requests/folders and
`NSSavePanel` for new destinations. It never creates destination files or
folders, resolves aliases into silently different targets, or permits multiple
selection. A save destination must not already exist. Directory-package
selection supports the existing lower-level operations; single-file `.kagextra`
packages retain their browser upload flow. Browser exports still download to
Downloads for manual movement by the user.

The host retains one pinned picker and reuses its clones. They share one
non-queued admission permit: another active dialog returns `Busy` before a
blocking worker or process is created. The permit remains with the actual worker
until child termination/reaping finishes, including caller cancellation.

Dropping the Rust future signals cancellation. The owned worker checks at most
every 50 ms while waiting for child output or exit, kills and reaps its child,
and never registers a result from that dropped future. No financial operation
starts merely by showing or cancelling a picker.

## Verification and limits

Fake private child-process fixtures test valid/cancelled responses, bounded
output, nonce/kind mismatch, unknown fields, unsafe path/owner/mode, executable
replacement or mutation, failed child exit, and cancellation with process reaping.
The actual Swift source was compiled successfully without running a real panel.
Interactive OS-panel behavior still requires a deliberate user-operated native
host acceptance run. The adapter does not yet add a menu-bar application or
connect native UI controls; it is ready for that integration.

Helper pinning protects against accidental replacement and untrusted path input;
it is not a code-signing/sandbox boundary against an attacker already controlling
the same macOS account. Path-based executable launch retains the normal local
filesystem check/launch race; the host must protect its installation directory.

## Disposable operator acceptance

Build with `cargo build --manifest-path backend/Cargo.toml --example native_picker_acceptance`.
The executable takes the canonical compiled helper path and optional
`--mode recovery-parent|request-destination|book-destination` and
`--cancel-expected`. Save modes print their private navigation parent and expected
new name; selecting a destination only registers it and must not create it. Launching it deliberately opens a real panel, so the native
host/operator must coordinate the run. It creates its disposable fixture only
at launch, selects a fake-key test book through the in-process router (no socket),
and prints the sole allowed target folder. Use Command-Shift-G to paste that
folder into the panel; select it, or cancel for the cancellation run.
`select_and_register_exact` rejects any other selection before registration.
Successful selection and expected cancellation print separate PASS messages;
unexpected cancellation/selection fails that acceptance run. The example closes
the session and cleans its identity-owned disposable fixture on normal exit.
It never connects to macOS Keychain.

The compile-only script now requires a canonical owner-private (0700) parent
outside the checkout. Bare executable output uses an exclusive hard link;
`.app` output uses macOS `renameatx_np(RENAME_EXCL)` to publish a complete bundle
atomically without replacing even an empty pre-existing directory. A destination
appearing during compilation is preserved, rather than overwritten. Private
compile intermediates remain for inspection. Eleven no-GUI Python tests verify
preflight, symlink rejection, bundle metadata and publication conflict safety.

The operator example handles Ctrl-C by dropping its pending selection future,
waiting on `NativePicker::wait_idle` until the owned child is killed/reaped,
then closing the session and cleaning the disposable fixture. The helper uses
a regular AppKit activation policy so a temporary test app bundle can appear
in the normal running-app inventory for authorized UI automation.

The actual panel run exposed a macOS path-normalization difference: Foundation
shortens `/private/var` and `/private/tmp` to `/var` and `/tmp`, while Rust's
canonical paths retain the POSIX `realpath` spelling. The helper now emits
Darwin `realpath` after its existing alias/symlink checks; new destinations use
the canonical parent plus the selected new leaf. The no-GUI regression in
`test_native_picker_canonical.py` compiles the actual helper canonicalizer and
checks it against POSIX paths after Foundation normalization. Operator-run
selection acceptance remains separately recorded by the coordinating session.


The follow-up build makes the standard Edit menu's responder target and Command
shortcuts explicit, without installing a custom clipboard reader or bypassing
AppKit validation. Earlier automated `paste` calls produced no field change;
this alone does not identify an AppKit menu defect. The next operator run must
compare focused-field Command-V or Edit > Paste with the automation paste helper
and record results separately. Direct accessibility field entry remains a tested
operator technique; clipboard paste is not claimed fixed without observation.
