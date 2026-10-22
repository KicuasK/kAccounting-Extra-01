# Portable encrypted export: APFS and exFAT

The export implementation no longer requires Unix private permissions on the
transfer disk. It builds and verifies the encrypted recipient package using
private source-local work, copies only ciphertext and the public recipient
envelope to the destination disk, verifies the copy using local private scratch,
then publishes with an exclusive same-disk rename and verifies once more.

The attachment-store parent is the private export work location. Recovery must
scan that source-local parent, not the external destination directory. It never
switches the selected book. A missing or changed destination disk preserves the
local operation for later inspection; unknown or unverified output is never
silently deleted or replaced.

An interrupted copy can leave a hidden directory named
`.kAccounting-Extra-01-ciphertext-<package-id>.partial` on the destination disk.
It contains encrypted objects and possibly the public recipient envelope only.
The software deliberately preserves such directories rather than deleting an
unplugged or replaced disk by a stale pathname. After confirming which disk and
operation they belong to, the operator can explicitly remove these incomplete
copies. Preserve the final `.kagextra` directory if uncertain and retry to a new
name. Export recovery is cleanup/verification, not automatic publication resume.

Physical APFS and exFAT testing is still required: real removable drive flush
support, exclusive rename, disconnection before/during copy and publication,
remount behavior, full-disk errors, and power interruption. Disposable-folder
tests on the current filesystem are not substitutes for that acceptance.
No disk formatting, genuine financial book, or actual Keychain key was used for
this implementation.

See [ADR 0042](decisions/0042-portable-encrypted-export-staging.md).
