//! Disposable legacy books only; no user Keychain or accounting data.
use kaccounting_backend::{
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, decrypt_object},
    db,
    keys::DeviceKeyId,
    migration_backup::{
        MigrationBackupError, prepare_upgrade, restore_pre_migration_backup,
        verify_pre_migration_backup,
    },
};
use sqlx::{migrate::Migrator, sqlite::SqlitePoolOptions};
use std::os::unix::fs::PermissionsExt;
use std::{borrow::Cow, fs, future::Future, path::PathBuf, task::Poll, time::Duration};
static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");
struct Fixture(PathBuf);
impl Fixture {
    async fn legacy(version: usize) -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-migration-{token}"));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(root.join("attachments")).unwrap();
        fs::set_permissions(root.join("attachments"), fs::Permissions::from_mode(0o700)).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(root.join("book.sqlite3"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let prefix = Migrator {
            migrations: Cow::Owned(MIGRATIONS.migrations[..version].to_vec()),
            ..Migrator::DEFAULT
        };
        prefix.run(&pool).await.unwrap();
        pool.close().await;
        fs::set_permissions(root.join("book.sqlite3"), fs::Permissions::from_mode(0o600)).unwrap();
        Self(root)
    }
    fn id() -> DeviceKeyId {
        DeviceKeyId::parse("11111111111111111111111111111111").unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn cancelling_startup_keeps_the_book_lock_until_owned_sqlite_work_finishes() {
    use kaccounting_backend::{
        keys::{DeviceKeyStore, KeyStoreError},
        runtime::{RuntimeBook, RuntimeError},
    };
    struct Keys;
    impl DeviceKeyStore for Keys {
        fn read(&self, id: &DeviceKeyId) -> Result<zeroize::Zeroizing<Vec<u8>>, KeyStoreError> {
            if id != &Fixture::id() {
                return Err(KeyStoreError::Missing);
            }
            Ok(zeroize::Zeroizing::new(vec![61; 32]))
        }
        fn insert_new(&self, _: &DeviceKeyId, _: &[u8]) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::AlreadyExists)
        }
    }
    let fixture = Fixture::legacy(6).await;
    let binding = fixture.0.join("device-key.json");
    fs::write(&binding, serde_json::to_vec(&serde_json::json!({
        "format":"kAccounting-Extra-01.local-backup", "version":1, "key_id":Fixture::id().as_str(),
    })).unwrap()).unwrap();
    fs::set_permissions(binding, fs::Permissions::from_mode(0o600)).unwrap();
    // An exclusive lock on this legacy DELETE-journal fixture prevents the
    // owned startup task from finishing until the test explicitly releases it.
    let blocker = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(fixture.0.join("book.sqlite3")),
        )
        .await
        .unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&blocker)
        .await
        .unwrap();
    let mut pending = Box::pin(RuntimeBook::open_existing(&fixture.0, &Keys));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(pending);
    assert!(matches!(
        RuntimeBook::open_existing(&fixture.0, &Keys).await,
        Err(RuntimeError::BookBusy)
    ));
    sqlx::query("ROLLBACK").execute(&blocker).await.unwrap();
    blocker.close().await;
    let reopened = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match RuntimeBook::open_existing(&fixture.0, &Keys).await {
                Err(RuntimeError::BookBusy) => tokio::time::sleep(Duration::from_millis(5)).await,
                other => break other,
            }
        }
    })
    .await
    .unwrap()
    .unwrap();
    reopened.close().await;
}
#[tokio::test]
async fn legacy_snapshot_is_encrypted_verified_and_source_unchanged() {
    for version in 1..7 {
        let fixture = Fixture::legacy(version).await;
        let original = fs::read(fixture.0.join("book.sqlite3")).unwrap();
        let key = RootEncryptionKey::new([91; 32]);
        let package = prepare_upgrade(&fixture.0, &Fixture::id(), &key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original, fs::read(fixture.0.join("book.sqlite3")).unwrap());
        assert!(
            db::pending_schema_migrations(fixture.0.join("book.sqlite3"))
                .await
                .unwrap()
        );
        verify_pre_migration_backup(&package, &Fixture::id(), &key)
            .await
            .unwrap();
        let mut recovered = Vec::new();
        decrypt_object(
            &mut fs::File::open(package.join("database.kae")).unwrap(),
            &mut recovered,
            &key,
            EncryptedObjectPurpose::Backup,
        )
        .unwrap();
        assert!(recovered.starts_with(b"SQLite format 3"));
        assert!(
            !fs::read(package.join("database.kae"))
                .unwrap()
                .starts_with(b"SQLite format 3")
        );
        let pool = db::open_database(fixture.0.join("book.sqlite3"))
            .await
            .unwrap();
        pool.close().await;
        db::verify_database_snapshot(fixture.0.join("book.sqlite3"))
            .await
            .unwrap();
        assert!(
            prepare_upgrade(&fixture.0, &Fixture::id(), &key)
                .await
                .unwrap()
                .is_none()
        );
    }
}
#[tokio::test]
async fn unrecognized_history_and_removed_legacy_trigger_fail_without_source_changes() {
    for sql in [
        "UPDATE _sqlx_migrations SET checksum = x'00' WHERE version = 1",
        "DROP TRIGGER accounts_prevent_cycle_update",
    ] {
        let fixture = Fixture::legacy(6).await;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new().filename(fixture.0.join("book.sqlite3")),
            )
            .await
            .unwrap();
        // Choose a known existing trigger rather than relying on a specific name.
        if sql.starts_with("DROP") {
            sqlx::query("DROP TRIGGER accounts_prevent_cycle_update")
                .execute(&pool)
                .await
                .unwrap();
        } else {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        pool.close().await;
        let original = fs::read(fixture.0.join("book.sqlite3")).unwrap();
        assert!(
            prepare_upgrade(&fixture.0, &Fixture::id(), &RootEncryptionKey::new([1; 32]))
                .await
                .is_err()
        );
        assert_eq!(original, fs::read(fixture.0.join("book.sqlite3")).unwrap());
        assert!(!fs::read_dir(&fixture.0).unwrap().any(|p| {
            p.unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".kabackup")
        }));
    }
}
#[tokio::test]
async fn wrong_key_tamper_and_interrupted_work_fail_closed() {
    let fixture = Fixture::legacy(6).await;
    let key = RootEncryptionKey::new([41; 32]);
    let package = prepare_upgrade(&fixture.0, &Fixture::id(), &key)
        .await
        .unwrap()
        .unwrap();
    assert!(
        verify_pre_migration_backup(&package, &Fixture::id(), &RootEncryptionKey::new([42; 32]))
            .await
            .is_err()
    );
    let file = package.join("database.kae");
    let mut bytes = fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(file, bytes).unwrap();
    assert!(
        verify_pre_migration_backup(&package, &Fixture::id(), &key)
            .await
            .is_err()
    );
    let interrupted = fixture.0.join(".migration-work-interrupted");
    fs::create_dir(&interrupted).unwrap();
    let original = fs::read(fixture.0.join("book.sqlite3")).unwrap();
    assert!(matches!(
        prepare_upgrade(&fixture.0, &Fixture::id(), &key).await,
        Err(MigrationBackupError::RecoveryRequired)
    ));
    assert!(interrupted.exists());
    assert_eq!(original, fs::read(fixture.0.join("book.sqlite3")).unwrap());
}
#[tokio::test]
async fn invalid_legacy_data_cannot_be_hidden_by_upgrade() {
    let fixture = Fixture::legacy(1).await;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(fixture.0.join("book.sqlite3")),
        )
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO accounts(name,element,kind,is_active) VALUES('Bad','asset','posting',2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let original = fs::read(fixture.0.join("book.sqlite3")).unwrap();
    assert!(
        prepare_upgrade(&fixture.0, &Fixture::id(), &RootEncryptionKey::new([1; 32]))
            .await
            .is_err()
    );
    assert_eq!(original, fs::read(fixture.0.join("book.sqlite3")).unwrap());
}

#[tokio::test]
async fn restores_historical_checkpoints_only_into_new_verified_destinations() {
    for version in 1..7 {
        let fixture = Fixture::legacy(version).await;
        let key = RootEncryptionKey::new([71; 32]);
        let package = prepare_upgrade(&fixture.0, &Fixture::id(), &key)
            .await
            .unwrap()
            .unwrap();
        let source_database = fs::read(fixture.0.join("book.sqlite3")).unwrap();
        let encrypted_original = fs::read(package.join("database.kae")).unwrap();
        let destination = fixture.0.join("restored");
        let outcome = restore_pre_migration_backup(&package, &destination, &Fixture::id(), &key)
            .await
            .unwrap();
        assert_eq!(outcome.path, destination);
        assert!(outcome.cleanup_warning.is_none());
        db::verify_database_snapshot(destination.join("book.sqlite3"))
            .await
            .unwrap();
        assert!(
            !db::pending_schema_migrations(destination.join("book.sqlite3"))
                .await
                .unwrap()
        );
        let binding: serde_json::Value =
            serde_json::from_slice(&fs::read(destination.join("device-key.json")).unwrap())
                .unwrap();
        assert_eq!(binding["key_id"], Fixture::id().as_str());
        assert_eq!(binding["format"], "kAccounting-Extra-01.local-backup");
        assert_eq!(
            source_database,
            fs::read(fixture.0.join("book.sqlite3")).unwrap()
        );
        assert_eq!(
            encrypted_original,
            fs::read(package.join("database.kae")).unwrap()
        );
        let restored = fs::read(destination.join("book.sqlite3")).unwrap();
        assert!(
            restore_pre_migration_backup(&package, &destination, &Fixture::id(), &key)
                .await
                .is_err()
        );
        assert_eq!(
            restored,
            fs::read(destination.join("book.sqlite3")).unwrap()
        );
        let wrong_key_destination = fixture.0.join("wrong-key-destination");
        assert!(
            restore_pre_migration_backup(
                &package,
                &wrong_key_destination,
                &Fixture::id(),
                &RootEncryptionKey::new([72; 32])
            )
            .await
            .is_err()
        );
        assert!(!wrong_key_destination.exists());
    }
}

#[tokio::test]
async fn package_extra_files_are_rejected_and_success_leaves_no_scratch() {
    let fixture = Fixture::legacy(6).await;
    let key = RootEncryptionKey::new([51; 32]);
    let package = prepare_upgrade(&fixture.0, &Fixture::id(), &key)
        .await
        .unwrap()
        .unwrap();
    assert!(!fs::read_dir(&fixture.0).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".migration-work-")
    }));
    fs::write(package.join("unexpected"), b"extra").unwrap();
    assert!(
        verify_pre_migration_backup(&package, &Fixture::id(), &key)
            .await
            .is_err()
    );
}
