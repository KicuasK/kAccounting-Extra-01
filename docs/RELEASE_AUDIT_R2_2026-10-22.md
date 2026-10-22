# Round 2 independent release integration audit — 2026-10-22

Project: `kAccounting-Extra-01`. Reviewed working tree remains on `main`,
`afba3de`; prior changes are preserved, with no commit or push by this worker.
This audit covers non-UI integration. It is not a Version 01 completion claim.

## Executed independent acceptance

Command: `cargo test --manifest-path backend/Cargo.toml --test r2_release_acceptance`.
Result on 2026-10-22 (Asia/Taipei): **3 passed, 0 failed**, using only disposable
private temporary directories and in-memory fake device/recipient keys.
Formatting was applied to this new test file. The integrating worker owns the
full verifier, frontend tests and Release build; their final results belong in
the session report, not inferred from this focused test run.

`backend/tests/r2_release_acceptance.rs` exercises these boundaries together:

1. A fresh `SessionRuntime` has no selected book and rejects accounting HTTP
   access until explicit selection. A temporary source book contains an exact
   integer amount of `9_007_199_254_740_993` minor units, an explicit correction,
   reversal and attachment. Storage capability registration precedes a real
   localhost-router HTTP export. Its completed job provides the browser download
   route with a single encrypted container and attachment download headers.
   Holding the download response body prevents switching or deselection. The
   transport is independently extracted, then consumed through
   `import_recipient_container`, which invokes the full recipient importer with
   a separate fake recipient/device keystore. The imported key and key ID differ
   from the source. Original, correction and reversal compare exactly; attachment
   plaintext agrees while ciphertext differs. SQL mutation/deletion of posted
   lines, entries and attachment records is refused. Import leaves the source
   selected; clearing selection explicitly returns the session to unselected.
   Replacing an exported ciphertext descriptor causes subsequent download to
   return 404 rather than transporting the replacement.
2. Replacing a selected export destination's parent directory with another
   directory at the same pathname makes the HTTP command return 400 before
   publication. Both the original and replacement parent remain empty, and the
   current book remains selected.
3. A deterministic gate inside a fake recipient keystore blocks an admitted
   storage job after its HTTP response has been consumed. Both switching and
   deselection return 409. Shutdown remains pending and the source book's process
   lock remains exclusive. Releasing the gate completes the request and shutdown;
   the book then opens again and the session requires explicit selection.

Initial focused-test failures were fixture errors: a directly opened source
still owned its process lock after pool close, and a manually written public
request lacked the private mode required by the application command. The test
now drops the source runtime before selection and sets request mode to 0600.
Neither required an implementation change.

## Actual loopback TCP transfer acceptance

A subsequent run used the compiled disposable `local_acceptance` example with
`--chooser-transfer` and the HTTP script with `--writes --choose-book
--transfer-capabilities <fixture-capability-file>`. The trusted fixture adapter
registers storage capabilities only after explicit HTTP book selection and
writes a token-bound 0600 capability document inside its disposable root. It
never opens real books or OS Keychain items, and starts no new listener outside
the existing loopback runtime.

Actual TCP acceptance **passed**: initial unselected session, explicit selection,
exact invented chart/report checks, chunked attachment upload/download and hash,
posting beyond JavaScript's safe integer range, verified encrypted backup, 202
export job completion, encrypted single-file download, chunked binary 201
recipient import, unchanged source selection, explicit switch to imported book,
identical exact report and attachment hash, then explicit deselection. All
Host/Origin/fetch-site boundary checks in the script passed too.

The temporary server was stopped gracefully, its disposable root was removed,
and port 3000 had no remaining listener. Strict Clippy for the example passed:
`cargo clippy --manifest-path backend/Cargo.toml --example local_acceptance --
-D warnings`. `git diff --check` also passed. Initial sandbox TCP restrictions
were retried through approved escalation; no network boundary was weakened.
This establishes actual TCP behavior, not Safari rendering or physical media.

## Source-review conclusions and remaining gates

The tested integration does not reveal a release-blocking implementation bug in
these paths. Producer-captured download descriptors are distinct from arbitrary
folder registration, selected-path device/inode checks reject observed ancestor
replacement, and owned jobs plus response-body leases preserve book lifetimes.
These are bounded findings, not proof against every possible filesystem race.
The security audit records the remaining advantage of descriptor-rooted system
calls against an actively racing local process.

Remaining acceptance gates are explicit:

- Actual APFS and exFAT external-media publication, removal, flush and interrupted
  recovery behavior. A temporary directory or simulated mode is not physical
  filesystem acceptance; an OS-denied disk-image attempt does not pass this gate.
- Mac Safari's actual download-to-Downloads flow, selection controls and complete
  operator workflow. HTTP headers and router tests do not establish browser UI
  acceptance. The approved policy is Downloads first, followed by manual movement
  to external media.
- Production private HTTPS ingress/proxy provenance, private Tailscale access and
  login-session launch behavior. No public service or Funnel was enabled here.
- The parent worker's separately authorized real Keychain acceptance and retained
  public item metadata. This audit neither repeats OS keychain calls nor reads
  existing items; fake-key tests must not be reported as real Keychain evidence.
- Full verifier and Release build after all concurrent changes, plus truthful
  operator documentation of retained encrypted container-import staging and
  explicit cleanup/recovery limitations.

No real accounting book, existing financial key, backup, public listener, login
agent installation or UI implementation was touched by this audit. This worker
changed only its new test and this report.
