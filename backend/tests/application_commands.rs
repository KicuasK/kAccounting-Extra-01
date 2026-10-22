use kaccounting_backend::{
    application_commands::{self, CommandError},
    attachment_maintenance::StorePin,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::{RecipientKeyStore, RecipientRequest},
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
async fn backup_is_verified_retained_and_restored_to_new_book_only() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let book = fixture.0.join("book");
    RuntimeBook::init_new(&book, &keys).await.unwrap();
    let binding = fs::read(book.join("device-key.json")).unwrap();
    let outcome = application_commands::backup_book(&book, &keys)
        .await
        .unwrap();
    assert!(outcome.backup.path.starts_with(book.join("backups")));
    assert!(outcome.retention_warning.is_none());
    assert_eq!(outcome.retention.unwrap().retained.len(), 1);
    let restored = fixture.0.join("restored");
    application_commands::restore_book(&outcome.backup.path, &restored, &keys)
        .await
        .unwrap();
    assert_eq!(fs::read(restored.join("device-key.json")).unwrap(), binding);
    assert_eq!(fs::read(book.join("device-key.json")).unwrap(), binding);
    let opened = RuntimeBook::open_existing(&restored, &keys).await.unwrap();
    assert!(opened.backup_id().is_none());
    opened.close().await;
    drop(opened);
    let identity = application_commands::setup_backup_identity(&restored, &keys)
        .await
        .unwrap();
    let identity_file = fs::read(restored.join("backup-identity.kae")).unwrap();
    assert!(matches!(
        application_commands::setup_backup_identity(&restored, &keys).await,
        Err(CommandError::ExistingBackupIdentity)
    ));
    assert_eq!(
        fs::read(restored.join("backup-identity.kae")).unwrap(),
        identity_file
    );
    let reopened = RuntimeBook::open_existing(&restored, &keys).await.unwrap();
    assert_eq!(reopened.backup_id().unwrap(), &identity);
    reopened.close().await;
    drop(reopened);
    let resumed = application_commands::backup_book(&restored, &keys)
        .await
        .unwrap();
    assert!(resumed.backup.path.exists());
    assert!(
        application_commands::restore_book(&outcome.backup.path, &restored, &keys)
            .await
            .is_err()
    );
    assert!(outcome.backup.path.exists());
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn backup_uses_exclusive_book_lock_and_missing_namespace_never_replaces_keys() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let book = fixture.0.join("book");
    RuntimeBook::init_new(&book, &keys).await.unwrap();
    let opened = RuntimeBook::open_existing(&book, &keys).await.unwrap();
    assert!(
        application_commands::backup_book(&book, &keys)
            .await
            .is_err()
    );
    opened.close().await;
    drop(opened);
    fs::remove_file(book.join("backup-identity.kae")).unwrap();
    assert!(matches!(
        application_commands::backup_book(&book, &keys).await,
        Err(CommandError::MissingBackupIdentity)
    ));
    assert!(!book.join("backup-identity.kae").exists());
    assert!(!book.join("backups").exists());
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn explicit_collection_refuses_pins_and_preserves_unknown_entries() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let book = fixture.0.join("book");
    RuntimeBook::init_new(&book, &keys).await.unwrap();
    let root = book.join("attachments");
    let orphan = root.join(format!("{}.kae", "a".repeat(32)));
    fs::write(&orphan, b"orphan ciphertext").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&orphan, fs::Permissions::from_mode(0o600)).unwrap();
    let unknown = root.join("keep.txt");
    fs::write(&unknown, b"preserve").unwrap();
    let pin = StorePin::acquire(&root).unwrap();
    assert!(
        application_commands::collect_attachments(&book, &keys)
            .await
            .is_err()
    );
    assert!(orphan.exists());
    drop(pin);
    let result = application_commands::collect_attachments(&book, &keys)
        .await
        .unwrap();
    assert_eq!(result.removed_objects, 1);
    assert_eq!(result.removed_bytes, 17);
    assert!(unknown.exists());
    assert!(!orphan.exists());
}
#[test]
fn recipient_request_publishes_only_public_json_and_never_overwrites() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let path = fixture.0.join("destination-request.json");
    let result = application_commands::write_recipient_request(&path, &keys).unwrap();
    let bytes = fs::read(&path).unwrap();
    let request = RecipientRequest::from_json(&bytes).unwrap();
    assert_eq!(request.request_id(), result.request_id);
    assert_eq!(request.fingerprint(), result.fingerprint);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 5);
    assert!(json.get("private_key").is_none());
    assert!(json.get("secret").is_none());
    assert!(application_commands::write_recipient_request(&path, &keys).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[cfg(unix)]
#[test]
fn recipient_request_rejects_symlink_parent_without_key_creation() {
    let fixture = Fixture::new();
    let keys = Keys::default();
    let link = fixture.0.join("link");
    std::os::unix::fs::symlink(&fixture.0, &link).unwrap();
    assert!(
        application_commands::write_recipient_request(link.join("request.json"), &keys).is_err()
    );
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 0);
}
