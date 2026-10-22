# Round 2 independent security review — 2026-10-22

Scope: non-UI source review of portable encrypted export, process-local storage
capabilities, selected-book HTTP runtime, and private access policy. Only
throwaway directories and in-memory fake keys are used in regression tests.
This is an engineering review, not a physical-media or real-Keychain acceptance.

## Findings and follow-up

1. **P1: native selected path identity must survive until execution.** At initial
   review `Selection` retained only a `PathBuf`. After registering a destination,
   renaming its parent away and creating another directory at the same path
   redirected a request write to the replacement. The same boundary applies to
   source books/packages/backups and recovery parents. Symlink checks alone do
   not identify an operator-selected directory/volume. Sent to integration owner;
   new regression coverage is in `backend/tests/r2_security_acceptance.rs`.
   Fixed in `storage_operations.rs`: record device/inode/type of every existing
   path component, then revalidate on capability lookup, admission, and owned
   job execution. The immutable recipient request leaf remains exempt because
   its parsed content, not later filesystem content, is the selected object.
   The three new security tests and all eight existing storage HTTP tests pass.
   These checks narrow replacement windows; descriptor-rooted system calls
   would provide a stronger boundary against an actively racing local process.

2. **Physical APFS/exFAT behavior remains unverified.** Source-local plaintext
   scratch and ciphertext-only external staging address the Unix-permission
   mismatch, but disposable-directory tests do not establish actual exFAT flush,
   device removal, case-folded name collisions, or power-loss durability.

## Reviewed boundaries

- Recipient request content is captured on selection; replacing the public file
  does not change the confirmed recipient fingerprint.
- Storage destination capabilities are consumed on admission; jobs retain an
  owned worker/supervisor and permit after their HTTP response is dropped.
- HTTP JSON rejects arbitrary paths, keys, overwrite and switch instructions;
  status errors do not return filesystem paths or underlying key failures.
- Export makes the SQLite snapshot and verification databases under private
  source-book work. Only envelope and encrypted package objects go to media.
  Stage copy and published output receive independent package authentication.
- Media recovery authenticates destination and device/inode identity. Missing or
  substituted media leaves private recovery work and external ciphertext intact.
- Atomic no-replace publication remains the final collision boundary, including
  names the filesystem considers equivalent. Physical filesystem acceptance is
  separate.
- The proposed session runtime holds selected-book read leases through response
  bodies, checks owned storage jobs before switching, and owns selection work
  across HTTP disconnects. Full integration tests are still required.
- `PrivateAccessPolicy` rejects even correct-looking Tailscale identity headers
  on `RegularLoopback`; its authenticated-proxy enum is a server assertion and
  currently not wired into an enabled production ingress. Do not treat header
  validation alone as proxy provenance.

## Verification log

- Initial regression execution blocked by expected in-progress integration:
  `runtime.rs` referred to `crate::session_runtime` before the module export was
  added. No completed test claim is made for that attempt.

- Final targeted verification: `cargo test -p kaccounting-backend --test
  r2_security_acceptance --test api_storage_commands`: **11 passed, 0 failed**.
  A destination created after selection is now rejected before job admission;
  destination parent identities are also checked before execution and writes.

## Final transport review

- A valid zero-attachment container originally omitted the required empty
  `attachments` directory during extraction. Fixed by creating the private
  directory unconditionally; an empty-book export/container/import/runtime-open
  regression now passes alongside the existing attached-book round trip.
- Downloads originally bounded chunks but not concurrent blocking workers.
  A separate two-permit download semaphore now covers preflight hashing and
  streaming. A third unconsumed download receives 409; disconnection releases
  capacity after its worker exits. The endpoint regression covers this lifecycle.
- HTTP container imports use configurable deployment resource limits, by default
  64 GiB total and 32 GiB per object, separate from accounting attachment policy.
  Declared length, actual received bytes and extraction share the configuration.
- Upload cancellation closes the bounded reader bridge; the owned import job
  retains its storage permit until extraction/import finishes. Transport hashes
  never replace recipient authentication or database and attachment verification.
- Output remains tentative until the download finishes. An actively racing
  same-account file mutation can emit partial invalid bytes before the final
  descriptor check rejects the stream; the recipient importer rejects such a
  container. No claim of descriptor-rooted race elimination is made.

## Approved successful-import cleanup

The user subsequently selected automatic removal of internal encrypted container
copies after fully verified import. A further read-only review found no new
blocker: cleanup authority stays in the live worker, checks exact entries and
recorded directory/object identities, avoids recursive deletion and preserves
unknown or replaced objects. The original input is never modified. Optional
cleanup errors leave the verified publication successful and expose only a
boolean warning over HTTP. Five focused cleanup tests and seven import
integration tests passed, including file/directory symlinks and unexpected
entries at every stage level. The known same-account syscall race limitation
remains. See [ADR 0051](decisions/0051-successful-container-import-staging-cleanup.md).
