use kaccounting_backend::{
    application_commands,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    runtime::RuntimeBook,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys {
    values: RefCell<HashMap<String, Zeroizing<Vec<u8>>>>,
    inserts: AtomicUsize,
}
impl Keys {
    fn load(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.values
            .borrow()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.values.borrow_mut();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.into(), Zeroizing::new(bytes.to_vec()));
        self.inserts.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.load(id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.insert(id.as_str(), bytes)
    }
}
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.load(id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.insert(id, bytes)
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-command-{id}"));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn recipient_command_roundtrip_preserves_source_and_releases_locks() {
    let fixture = Fixture::new();
    let source_keys = Keys::default();
    let destination_keys = Keys::default();
    let source = fixture.0.join("source");
    RuntimeBook::init_new(&source, &source_keys).await.unwrap();
    let original_binding = fs::read(source.join("device-key.json")).unwrap();
    let request_file = fixture.0.join("request.json");
    application_commands::write_recipient_request(&request_file, &destination_keys).unwrap();
    let request = application_commands::read_recipient_request(&request_file).unwrap();
    let package = fixture.0.join("transfer.kagextra");
    let exported =
        application_commands::export_recipient_book(&source, &request, &package, &source_keys)
            .await
            .unwrap();
    assert_eq!(exported.path, package);
    let reopened = RuntimeBook::open_existing(&source, &source_keys)
        .await
        .unwrap();
    reopened.close().await;
    drop(reopened);
    let imported_path = fixture.0.join("new-book");
    let imported = application_commands::import_recipient_book(
        &package,
        &request,
        &imported_path,
        &destination_keys,
        &destination_keys,
    )
    .await
    .unwrap();
    assert_eq!(imported.package_id, exported.package_id);
    assert_eq!(imported.path, imported_path);
    let imported_book = RuntimeBook::open_existing(&imported_path, &destination_keys)
        .await
        .unwrap();
    assert!(imported_book.backup_id().is_some());
    imported_book.close().await;
    drop(imported_book);
    assert_eq!(
        fs::read(source.join("device-key.json")).unwrap(),
        original_binding
    );
    let recovered =
        application_commands::recover_recipient_exports(&source, &fixture.0, &source_keys)
            .await
            .unwrap();
    assert!(recovered.unrecognized_work.is_empty());
    assert!(
        application_commands::import_recipient_book(
            &package,
            &request,
            &imported_path,
            &destination_keys,
            &destination_keys
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn failed_export_and_recovery_release_book_and_preserve_existing_destination() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let source = fixture.0.join("source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let request_file = fixture.0.join("request.json");
    application_commands::write_recipient_request(&request_file, &keys).unwrap();
    let request = application_commands::read_recipient_request(&request_file).unwrap();
    let existing = fixture.0.join("existing.kagextra");
    fs::write(&existing, b"preserve").unwrap();
    assert!(
        application_commands::export_recipient_book(&source, &request, &existing, &keys)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&existing).unwrap(), b"preserve");
    let book = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    book.close().await;
    drop(book);
    assert!(
        application_commands::recover_recipient_exports(&source, fixture.0.join("missing"), &keys)
            .await
            .is_err()
    );
    let book = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    book.close().await;
}

#[test]
fn request_reader_rejects_oversize_links_public_mode_and_source_paths() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let request = fixture.0.join("request.json");
    application_commands::write_recipient_request(&request, &keys).unwrap();
    assert!(application_commands::read_recipient_request(&request).is_ok());
    #[cfg(unix)]
    {
        let alias = fixture.0.join("alias.json");
        std::os::unix::fs::symlink(&request, &alias).unwrap();
        assert!(application_commands::read_recipient_request(&alias).is_err());
        fs::set_permissions(&request, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(application_commands::read_recipient_request(&request).is_err());
        fs::set_permissions(&request, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(&request, vec![b' '; 4097]).unwrap();
    assert!(application_commands::read_recipient_request(&request).is_err());
    assert!(
        application_commands::read_recipient_request(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
        )
        .is_err()
    );
}

#[tokio::test]
async fn export_recovery_uses_existing_key_even_when_live_database_is_damaged() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let source = fixture.0.join("source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let binding = fs::read(source.join("device-key.json")).unwrap();
    fs::write(
        source.join("book.sqlite3"),
        b"invented damaged live database",
    )
    .unwrap();
    fs::create_dir(source.join(".migration-work-preserve-evidence")).unwrap();
    let ambiguous = fixture.0.join(format!(
        ".kAccounting-Extra-01-recipient-export-{}.work",
        "1".repeat(32)
    ));
    fs::create_dir(&ambiguous).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&ambiguous, fs::Permissions::from_mode(0o700)).unwrap();
    let result = application_commands::recover_recipient_exports(&source, &fixture.0, &keys)
        .await
        .unwrap();
    assert_eq!(result.unrecognized_work, vec![ambiguous.clone()]);
    assert!(ambiguous.exists());
    assert_eq!(
        fs::read(source.join("book.sqlite3")).unwrap(),
        b"invented damaged live database"
    );
    assert_eq!(fs::read(source.join("device-key.json")).unwrap(), binding);
    assert!(source.join(".migration-work-preserve-evidence").exists());
    let missing = Keys::default();
    assert!(
        application_commands::recover_recipient_exports(&source, &fixture.0, &missing)
            .await
            .is_err()
    );
    assert_eq!(missing.inserts.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn failed_checkpoint_restore_closes_source_and_leaves_destination_absent() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let source = fixture.0.join("source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let destination = fixture.0.join("restored");
    assert!(
        application_commands::restore_pre_migration(
            &source,
            fixture.0.join("missing-checkpoint"),
            &destination,
            &keys
        )
        .await
        .is_err()
    );
    assert!(!destination.exists());
    let reopened = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    reopened.close().await;
}

#[tokio::test]
async fn checkpoint_restore_command_preserves_selection_and_source_and_opens_new_book() {
    use kaccounting_backend::book_selection::BookSelectionSession;
    use sqlx::migrate::Migrator;
    static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");
    let fixture = Fixture::new();
    let keys = Keys::default();
    let source = fixture.0.join("source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    // Replace only this disposable empty fixture with a known legacy schema.
    fs::remove_file(source.join("book.sqlite3")).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(source.join("book.sqlite3"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let legacy = Migrator {
        migrations: std::borrow::Cow::Owned(MIGRATIONS.migrations[..6].to_vec()),
        ..Migrator::DEFAULT
    };
    legacy.run(&pool).await.unwrap();
    pool.close().await;
    #[cfg(unix)]
    fs::set_permissions(
        source.join("book.sqlite3"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut selected = BookSelectionSession::new();
    selected.select(&source, &keys).await.unwrap();
    let checkpoint = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pre-migration-")
        })
        .unwrap();
    let destination = fixture.0.join("restored");
    assert!(
        application_commands::restore_pre_migration(&source, &checkpoint, &destination, &keys)
            .await
            .is_err()
    );
    assert_eq!(selected.current().unwrap().root(), source);
    assert!(!destination.exists());
    selected.close().await;
    let original_binding = fs::read(source.join("device-key.json")).unwrap();
    let original_database = fs::read(source.join("book.sqlite3")).unwrap();
    assert!(
        application_commands::restore_pre_migration(
            &source,
            &checkpoint,
            &destination,
            &Keys::default()
        )
        .await
        .is_err()
    );
    assert!(!destination.exists());
    let outcome =
        application_commands::restore_pre_migration(&source, &checkpoint, &destination, &keys)
            .await
            .unwrap();
    assert_eq!(outcome.path, destination);
    assert_eq!(
        fs::read(source.join("device-key.json")).unwrap(),
        original_binding
    );
    assert_eq!(
        fs::read(source.join("book.sqlite3")).unwrap(),
        original_database
    );
    assert!(selected.current().is_none());
    let restored = RuntimeBook::open_existing(&destination, &keys)
        .await
        .unwrap();
    restored.close().await;
    let source_book = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    source_book.close().await;
}

#[tokio::test]
async fn corrupt_live_database_and_interrupted_work_do_not_block_checkpoint_recovery() {
    use kaccounting_backend::{keys::create_device_key, migration_backup::prepare_upgrade};
    use sqlx::migrate::Migrator;
    static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");
    let fixture = Fixture::new();
    let keys = Keys::default();
    let source = fixture.0.join("damaged-source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("attachments")).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            source.join("attachments"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let (id, key) = create_device_key(&keys).unwrap();
    let binding = serde_json::to_vec(&serde_json::json!({"format":"kAccounting-Extra-01.local-backup","version":1,"key_id":id.as_str()})).unwrap();
    fs::write(source.join("device-key.json"), &binding).unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        source.join("device-key.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(source.join("book.sqlite3"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let legacy = Migrator {
        migrations: std::borrow::Cow::Owned(MIGRATIONS.migrations[..6].to_vec()),
        ..Migrator::DEFAULT
    };
    legacy.run(&pool).await.unwrap();
    pool.close().await;
    #[cfg(unix)]
    fs::set_permissions(
        source.join("book.sqlite3"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let checkpoint = prepare_upgrade(&source, &id, &key).await.unwrap().unwrap();
    fs::write(
        source.join("book.sqlite3"),
        b"damaged original database preserve exactly",
    )
    .unwrap();
    let scratch = source.join(".migration-work-interrupted");
    fs::create_dir(&scratch).unwrap();
    fs::write(scratch.join("evidence"), b"preserve interruption evidence").unwrap();
    let destination = fixture.0.join("recovered");
    assert!(RuntimeBook::open_existing(&source, &keys).await.is_err());
    assert!(
        application_commands::restore_pre_migration(
            &source,
            &checkpoint,
            &destination,
            &Keys::default()
        )
        .await
        .is_err()
    );
    assert!(!destination.exists());
    let outcome =
        application_commands::restore_pre_migration(&source, &checkpoint, &destination, &keys)
            .await
            .unwrap();
    assert_eq!(outcome.path, destination);
    assert_eq!(
        fs::read(source.join("book.sqlite3")).unwrap(),
        b"damaged original database preserve exactly"
    );
    assert_eq!(fs::read(source.join("device-key.json")).unwrap(), binding);
    assert_eq!(
        fs::read(scratch.join("evidence")).unwrap(),
        b"preserve interruption evidence"
    );
    let restored = RuntimeBook::open_existing(&destination, &keys)
        .await
        .unwrap();
    restored.close().await;
}
