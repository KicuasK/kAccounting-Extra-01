//! Disposable runtime books and fake keys; never accesses the user's Keychain.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    api::{localhost_book_commands_router, localhost_write_router},
    book_backup::verify_local_backup,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::RuntimeBook,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct MemoryKeys {
    values: RefCell<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
    reads: AtomicUsize,
    inserts: AtomicUsize,
}
impl DeviceKeyStore for MemoryKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.values
            .borrow()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.values.borrow_mut();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.clone(), Zeroizing::new(bytes.to_vec()));
        self.inserts.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-book-http-{id}"));
        std::fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn request(book: &RuntimeBook, body: &str) -> (StatusCode, serde_json::Value) {
    let router = localhost_write_router(book.pool().clone())
        .merge(localhost_book_commands_router(book).unwrap());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/backups")
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn manual_backup_is_verified_at_fixed_destination_without_keychain_calls() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    let draft =
        kaccounting_backend::accounting::journal::JournalRepository::new(book.pool().clone())
            .create_draft(
                kaccounting_backend::accounting::journal::SaveDraftEntry::new(
                    kaccounting_backend::accounting::journal::JournalDate::parse("2026-10-22")
                        .unwrap(),
                    vec![],
                ),
            )
            .await
            .unwrap();
    let source = fixture.0.join("receipt.pdf");
    std::fs::write(&source, b"%PDF receipt").unwrap();
    kaccounting_backend::attachments::AttachmentRepository::open(
        book.pool().clone(),
        book.attachments(),
        book.key(),
    )
    .unwrap()
    .import_file(draft.id, &source, "receipt.pdf")
    .await
    .unwrap();
    let reads = keys.reads.load(Ordering::Relaxed);
    let inserts = keys.inserts.load(Ordering::Relaxed);
    let (status, value) = request(&book, "{}").await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    assert_eq!(value["verified"], true);
    assert_eq!(value["attachment_count"], "1");
    assert_eq!(value["attachment_bytes"], "12");
    assert!(value["retention_warning"].is_null());
    assert_eq!(value["preserved_unrecognized"], "0");
    assert_eq!(keys.reads.load(Ordering::Relaxed), reads);
    assert_eq!(keys.inserts.load(Ordering::Relaxed), inserts);
    let name = value["backup_name"].as_str().unwrap();
    assert!(!name.contains('/'));
    let backup = path.join("backups").join(name).join("backup.kabackup");
    assert!(backup.is_dir());
    verify_local_backup(&backup, &keys).await.unwrap();
    assert!(value.get("key_id").is_none());
    assert!(value.get("path").is_none());
    book.close().await;
}
#[tokio::test]
async fn backup_rejects_client_selected_paths_without_writing() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    let (status, value) = request(&book, r#"{"destination":"/tmp/arbitrary.kabackup"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["code"], "invalid_json");
    assert!(!path.join("backups").exists());
    book.close().await;
}
#[tokio::test]
async fn missing_backup_identity_is_explicit_and_never_created_by_http() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    std::fs::remove_file(path.join("backup-identity.kae")).unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    let (status, value) = request(&book, "{}").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "backup_identity_missing");
    assert!(!path.join("backup-identity.kae").exists());
    assert!(!path.join("backups").exists());
    book.close().await;
}
#[cfg(unix)]
#[tokio::test]
async fn backup_symlink_is_preserved_and_never_used() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    let unrelated = fixture.0.join("unrelated");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::set_permissions(&unrelated, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&unrelated, path.join("backups")).unwrap();
    let (status, value) = request(&book, "{}").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "backup_identity_invalid");
    assert_eq!(std::fs::read_dir(&unrelated).unwrap().count(), 0);
    assert!(
        std::fs::symlink_metadata(path.join("backups"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    book.close().await;
}
