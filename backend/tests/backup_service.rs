use kaccounting_backend::{
    backup_service::{
        BackupReason, BackupServiceError, BookBackupId, create_managed_backup,
        initialize_backup_identity, load_backup_identity, retain_latest_verified_backups,
        with_verified_checkpoint,
    },
    book_backup::verify_local_backup,
    db::open_database,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, create_device_key},
};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
use zeroize::Zeroizing;

#[tokio::test]
async fn new_backup_stays_newest_when_clock_is_behind_an_authenticated_receipt() {
    use kaccounting_backend::{
        crypto::{EncryptedObjectPurpose, decrypt_object, encrypt_object},
        keys::load_device_key,
    };
    use std::io::Cursor;
    let f = Fixture::new().await;
    let package = f.backup().await;
    let directory = package.parent().unwrap();
    let old_name = directory.file_name().unwrap().to_str().unwrap();
    let suffix = old_name.split('-').nth(2).unwrap();
    let key = load_device_key(&f.keys, &f.key_id).unwrap();
    let mut receipt = Vec::new();
    decrypt_object(
        &mut Cursor::new(std::fs::read(directory.join("receipt.kae")).unwrap()),
        &mut receipt,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )
    .unwrap();
    let mut receipt: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
    let future = receipt["created_nanos"]
        .as_str()
        .unwrap()
        .parse::<u128>()
        .unwrap()
        + 1_000_000_000_000_000_000;
    let new_name = format!("{}-{future:039}-{suffix}", f.book.as_str());
    receipt["created_nanos"] = serde_json::json!(future.to_string());
    receipt["name"] = serde_json::json!(new_name);
    let mut encrypted = Vec::new();
    encrypt_object(
        &mut Cursor::new(serde_json::to_vec(&receipt).unwrap()),
        &mut encrypted,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )
    .unwrap();
    std::fs::write(directory.join("receipt.kae"), encrypted).unwrap();
    std::fs::rename(directory, f.backups.join(new_name)).unwrap();
    let current = f.backup().await;
    let current_time = current
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .split('-')
        .nth(1)
        .unwrap()
        .parse::<u128>()
        .unwrap();
    assert!(
        current_time > future,
        "a new checkpoint must sort newer even after clock rollback"
    );
}

#[derive(Default)]
struct Keys(Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>);
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().unwrap();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.clone(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    objects: PathBuf,
    backups: PathBuf,
    pool: sqlx::SqlitePool,
    keys: Keys,
    key_id: DeviceKeyId,
    book: BookBackupId,
}
fn private_dir(path: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path).unwrap();
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-backup-service-{}",
            BookBackupId::generate().unwrap().as_str()
        ));
        private_dir(&root);
        let root = std::fs::canonicalize(root).unwrap();
        let objects = root.join("objects");
        private_dir(&objects);
        let backups = root.join("backups");
        private_dir(&backups);
        let pool = open_database(root.join("book.sqlite3")).await.unwrap();
        let keys = Keys::default();
        let (key_id, _) = create_device_key(&keys).unwrap();
        let book = initialize_backup_identity(&root, &key_id, &keys).unwrap();
        Self {
            root,
            objects,
            backups,
            pool,
            keys,
            key_id,
            book,
        }
    }
    async fn backup(&self) -> PathBuf {
        create_managed_backup(
            &self.pool,
            &self.objects,
            &self.backups,
            &self.book,
            &self.key_id,
            &self.keys,
            BackupReason::Manual,
        )
        .await
        .unwrap()
        .path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn identity_is_encrypted_create_only_and_wrong_keys_or_missing_files_fail_closed() {
    let f = Fixture::new().await;
    assert_eq!(
        load_backup_identity(&f.root, &f.key_id, &f.keys).unwrap(),
        f.book
    );
    assert!(initialize_backup_identity(&f.root, &f.key_id, &f.keys).is_err());
    let bytes = std::fs::read(f.root.join("backup-identity.kae")).unwrap();
    assert!(
        !bytes
            .windows(32)
            .any(|window| window == f.book.as_str().as_bytes())
    );
    assert!(load_backup_identity(&f.root, &f.key_id, &Keys::default()).is_err());
    std::fs::remove_file(f.root.join("backup-identity.kae")).unwrap();
    assert!(load_backup_identity(&f.root, &f.key_id, &f.keys).is_err());
    assert!(!f.root.join("backup-identity.kae").exists());
}

#[tokio::test]
async fn newest_ten_verified_backups_retained_and_unknown_corrupt_and_foreign_preserved() {
    let f = Fixture::new().await;
    let mut paths = Vec::new();
    for _ in 0..12 {
        paths.push(f.backup().await);
    }
    let unknown = f.backups.join("my-important-folder");
    private_dir(&unknown);
    std::fs::write(unknown.join("keep.txt"), b"preserve").unwrap();
    let incomplete = f.backups.join(format!(
        "{}-{}-{}.managed",
        f.book.as_str(),
        "0".repeat(39),
        "a".repeat(32)
    ));
    private_dir(&incomplete);
    let foreign = create_managed_backup(
        &f.pool,
        &f.objects,
        &f.backups,
        &BookBackupId::generate().unwrap(),
        &f.key_id,
        &f.keys,
        BackupReason::Manual,
    )
    .await
    .unwrap()
    .path;
    let corrupt = f.backup().await;
    std::fs::write(corrupt.parent().unwrap().join("receipt.kae"), b"broken").unwrap();
    let outcome = retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert_eq!(outcome.retained.len(), 10);
    assert_eq!(outcome.removed.len(), 2);
    assert!(!paths[0].exists());
    assert!(!paths[1].exists());
    for path in paths.iter().skip(2) {
        assert!(path.exists());
    }
    assert!(foreign.exists());
    assert!(unknown.join("keep.txt").exists());
    assert!(incomplete.exists());
    assert!(corrupt.exists());
    assert_eq!(outcome.preserved_unrecognized.len(), 4);
    verify_local_backup(paths.last().unwrap(), &f.keys)
        .await
        .unwrap();
    assert!(f.root.join("book.sqlite3").exists());
}

#[tokio::test]
async fn unavailable_key_never_prunes_and_last_valid_backup_is_retained() {
    let f = Fixture::new().await;
    let path = f.backup().await;
    assert!(
        retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &Keys::default())
            .await
            .is_err()
    );
    assert!(path.exists());
    let result = retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert_eq!(result.retained.len(), 1);
    assert!(result.removed.is_empty());
}

#[tokio::test]
async fn checkpoint_failure_prevents_operation_and_operation_failure_preserves_backup() {
    let result = with_verified_checkpoint::<(), (), _, _>(
        Err(BackupServiceError::InvalidIdentity),
        |_| async { panic!("operation must not run") },
    )
    .await;
    assert!(result.is_err());
    let f = Fixture::new().await;
    let checkpoint = create_managed_backup(
        &f.pool,
        &f.objects,
        &f.backups,
        &f.book,
        &f.key_id,
        &f.keys,
        BackupReason::BeforeImport,
    )
    .await;
    let path = checkpoint.as_ref().unwrap().path.clone();
    let result = with_verified_checkpoint(checkpoint, |_| async { Err::<(), _>("import failed") })
        .await
        .unwrap();
    assert_eq!(result, Err("import failed"));
    assert!(path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_are_preserved_and_unprivate_root_is_rejected() {
    let f = Fixture::new().await;
    let path = f.backup().await;
    let link = f.backups.join(format!(
        "{}-{}-{}.managed",
        f.book.as_str(),
        "1".repeat(39),
        "b".repeat(32)
    ));
    std::os::unix::fs::symlink(path.parent().unwrap(), &link).unwrap();
    let result = retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert!(result.preserved_unrecognized.contains(&link));
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::set_permissions(&f.backups, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn modified_catalog_and_extra_files_are_never_retention_candidates() {
    let f = Fixture::new().await;
    let path = f.backup().await;
    std::fs::write(path.parent().unwrap().join("extra"), b"unknown user file").unwrap();
    let other = f.backup().await;
    std::fs::write(other.join("catalog.kae"), b"corrupt").unwrap();
    let result = retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert!(result.retained.is_empty());
    assert!(result.removed.is_empty());
    assert_eq!(result.preserved_unrecognized.len(), 2);
    assert!(path.exists());
    assert!(other.exists());
}

#[tokio::test]
async fn renamed_receipts_and_foreign_device_keys_cannot_claim_book_ownership() {
    let f = Fixture::new().await;
    let path = f.backup().await;
    let renamed = f.backups.join(format!(
        "{}-{}-{}.managed",
        f.book.as_str(),
        "2".repeat(39),
        "c".repeat(32)
    ));
    std::fs::rename(path.parent().unwrap(), &renamed).unwrap();
    let (foreign_key_id, _) = create_device_key(&f.keys).unwrap();
    let foreign = create_managed_backup(
        &f.pool,
        &f.objects,
        &f.backups,
        &f.book,
        &foreign_key_id,
        &f.keys,
        BackupReason::BeforeMigration,
    )
    .await
    .unwrap()
    .path;
    let result = retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert!(result.retained.is_empty());
    assert!(result.removed.is_empty());
    assert!(renamed.exists());
    assert!(foreign.exists());
    assert_eq!(result.preserved_unrecognized.len(), 2);
}

#[tokio::test]
async fn active_maintenance_lock_blocks_backup_and_pruning() {
    let f = Fixture::new().await;
    f.backup().await;
    let lock = std::fs::File::open(f.backups.join(".backup-service.lock")).unwrap();
    lock.try_lock().unwrap();
    assert!(matches!(
        retain_latest_verified_backups(&f.backups, &f.book, &f.key_id, &f.keys).await,
        Err(BackupServiceError::Busy)
    ));
    assert!(matches!(
        create_managed_backup(
            &f.pool,
            &f.objects,
            &f.backups,
            &f.book,
            &f.key_id,
            &f.keys,
            BackupReason::Manual
        )
        .await,
        Err(BackupServiceError::Busy)
    ));
}
