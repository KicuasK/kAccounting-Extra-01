# ADR 0042: Source-local export work, portable ciphertext publication

Status: implemented engineering decision, 2026-10-22. Physical removable-media
acceptance remains pending. The user requires APFS and exFAT support and has
delegated technical choices.

The prior exporter created its entire work tree beside the destination. That
placed the live SQLite snapshot, verification databases, operation state, and
source-key-wrapped recovery material on transfer media and relied on Unix
private modes there. exFAT cannot enforce that local-private-storage contract.

All new export work is now under the canonical source attachment-store parent.
That parent must pass the existing strict private directory checks. The runtime
must supply local source-book storage. No global private-permission check is
relaxed. Source pinning occurs before snapshot creation and all decrypted
verification databases remain in this private work tree.

The complete encrypted package is built and independently verified locally.
Only the bounded public recipient envelope and encrypted database, manifest,
and attachment files are copied using bounded I/O to a fresh `.partial` directory
beside the destination. The removable staging layer assumes no Unix mode
support. It never receives SQLite plaintext, a transfer key, source recovery
context, or operation-state files. The copied package is independently decrypted
and verified into private local scratch before publication. An exclusive atomic
same-media rename publishes it without replacing existing output, followed by
another independent full verification. Required synchronization errors fail
closed instead of being ignored.

The authenticated, source-key-wrapped recovery context binds the canonical
destination, the media parent device/inode identity, and deterministic encrypted
staging name. Recovery scans the private source parent, verifies the original
media identity, and preserves private work if the media is absent or replaced.
It never recursively deletes an external stage or uncertain published output.
Interrupted ciphertext-only `.partial` directories may remain on the media and
can be reviewed and removed explicitly by the operator. The bound identity is
conservative: remounting with changed identifiers can require a fresh export.
Old recovery contexts remain recognized only under their original beside-the-
destination contract.

Tests use disposable books and memory keys, permissive-mode destination
folders, changed/missing media identities, symlinks, failed copies, and existing
outputs. These simulate the permissionless and interruption contracts; they do
not establish real exFAT flush/rename behavior, APFS/exFAT device removal safety,
or power-loss durability on physical hardware.
