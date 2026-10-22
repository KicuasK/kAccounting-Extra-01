# APFS / exFAT disposable-media acceptance

Scope update (user decision, 2026-10-22): physical external-drive acceptance is
deferred to Version 02 because no external drive is available. Keep all existing
implementation, simulated tests and this opt-in harness. Do not run physical
media checks now or treat them as a Version 01 release/UI gate. No physical-media
pass is claimed.

The portable package implementation is covered by ordinary simulated-media tests. Actual mounted APFS and exFAT acceptance remains a separate opt-in gate. A green unit suite does **not** establish physical-volume acceptance.

## Safe test procedure

Use an already mounted disposable disk image or an explicitly designated empty test directory on a mounted test volume. This harness never creates or formats a volume, never detaches one, never deletes existing contents, and never uses Keychain. Do not point it at a real book. The directory must be empty, canonical, outside the source tree, and named `kAccounting-Extra-01-media-test-*`.

1. Create that empty directory on the mounted test volume.
2. Run `python3 scripts/media-acceptance.py prepare --root /Volumes/TEST/kAccounting-Extra-01-media-test-example --filesystem exfat` (or `apfs`). Record the returned random ownership token.
3. Run `python3 scripts/media-acceptance.py run --root /Volumes/TEST/kAccounting-Extra-01-media-test-example --filesystem exfat --token TOKEN`.
4. Inspect the retained encrypted `transfer.kagextra` fixture. Remove the disposable test directory manually when finished. The harness does not attempt automatic cleanup or detach.

Run separately for APFS and exFAT. The marker binds the token to directory device/inode and expected filesystem. The launcher checks `diskutil info -plist`, refuses unmounted/mismatched filesystems, a system-root mount, symlinks, source paths, unexpected existing files, and token mismatch. A run refuses any contents other than its own marker, so reruns require a fresh empty test directory. This protects against accidental selection; concurrent malicious filesystem mutation is outside the harness threat model.

The ignored Rust test creates a source book and import target only in local temporary private storage. It exports a recipient-encrypted package onto the selected media, imports it back under a fresh in-memory device key, verifies exact posted amounts above JavaScript's integer range, full database integrity, attachments, immutable guards, and unchanged source ciphertext. No source database or clear attachment is written to the media. The successful ciphertext fixture remains on media. The test requires the ownership marker even when invoked directly.

## Current machine result (2026-10-22)

- Seven Python safety/parser tests pass; Rust acceptance target compiles.
- Real mounted APFS/exFAT acceptance: **NOT RUN** in this environment.
- Earlier disposable-image creation via `hdiutil create` failed with `Device not configured`; its approved escalated retry failed with `Operation not permitted`.
- Read-only `diskutil image create help` inspection also failed: `Unable to run because unable to use the DiskManagement framework` (DiskArbitration unavailable). No image was created. No alternate privilege mechanism, physical disk format, or approval bypass was attempted.
- To complete the gate, run the explicit procedure in a normal macOS login session with an already mounted disposable image or selected empty test root. Native filesystem compatibility and physical unplug/power-loss durability remain unverified until actual media runs succeed.

## Automated harness checks

`python3 scripts/test_media_acceptance.py`

`cargo test --manifest-path backend/Cargo.toml --test media_acceptance` reports the real-media test ignored by design. Use the launcher above to opt in. A default ignored result is not a pass for the physical-media gate.
