# Pre-migration safety backups

Opening an explicitly selected recognized older book first creates a private,
encrypted `pre-migration-<random>.kabackup` directory beside `book.sqlite3`.
It retains the original schema, every referenced encrypted attachment, and an
authenticated encrypted descriptor catalog. The existing device key is required;
this is not a cross-Mac transfer and does not move Keychain material.

The application verifies an upgraded disposable copy before creating the backup,
then verifies the complete package before and after durable publication. No live
schema migration begins until all checks succeed. Existing destinations are not
replaced. Safety backups are retained without automatic pruning.

Current-schema books receive their existing full integrity audit without a new
pre-migration backup. Unknown or altered legacy migration histories and schemas
are rejected. Missing attachments, wrong keys, catalog/ciphertext corruption or
storage errors stop the upgrade. A failed live migration leaves the safety backup
available, but is not silently declared a successful upgrade.

Failed or interrupted `.migration-work-*` directories block opening and are preserved for
inspection. They may contain private plaintext SQLite scratch. Do not copy them
into the source tree or delete them as an automated workaround. Automatic scratch
cleanup/resumption is not implemented. Use the explicit `restore-pre-migration`
command to restore a verified checkpoint to a NEW destination. It loads only the
source book's existing key binding, so a corrupt source database or retained
migration work cannot block recovery. The restored copy is upgraded and audited
privately; exact database bytes, key binding and every attachment are verified
again after publication. Source corruption evidence and the checkpoint remain
untouched. Select the restored book explicitly; no automatic switch occurs.

The checkpoint uses the existing device key and requires the original book's
public `device-key.json` binding. It is not portable recovery if that Keychain
key is unavailable. The restored book can explicitly initialize its own backup
namespace through `setup-backup-identity`, just like ordinary local restores.

Runtime also performs conservative existing backup-work recovery in `backups/`
and one level of precisely named managed backup directories. Symlinks and
unrecognized interrupted backup work stop opening rather than being traversed
or removed. Completed packages and unrelated directories are preserved.

Tests use disposable temporary books and fixed test keys only. They do not access
real books or the user Keychain.
