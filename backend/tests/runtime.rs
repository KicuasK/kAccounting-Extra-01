//! Runtime tests use disposable books and an in-memory key store only.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::{LOCAL_ADDRESS, RuntimeBook, RuntimeError, application_router},
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
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct MemoryKeys {
    values: RefCell<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
    inserts: AtomicUsize,
}
impl DeviceKeyStore for MemoryKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
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
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-runtime-{id}"));
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
async fn startup_recovers_only_recognized_nested_managed_backup_work() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let book = fixture.0.join("book");
    RuntimeBook::init_new(&book, &keys).await.unwrap();
    let backups = book.join("backups");
    let generated = backups.join(format!(
        "{}-{}-{}.managed",
        "1".repeat(32),
        "0".repeat(39),
        "2".repeat(32)
    ));
    let unrelated = backups.join("unrelated.managed");
    for path in [&backups, &generated, &unrelated] {
        fs::create_dir(path).unwrap();
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let token = "0123456789abcdef0123456789abcdef";
    let name = format!(".kAccounting-Extra-01-encrypted-backup-{token}.work");
    let recognized = generated.join(&name);
    let untouched = unrelated.join(&name);
    for path in [&recognized, &untouched] {
        fs::create_dir(path).unwrap();
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            path.join(".lock"),
            format!("kAccounting-Extra-01/encrypted-backup-work/v1/{token}\n"),
        )
        .unwrap();
        #[cfg(unix)]
        fs::set_permissions(path.join(".lock"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(path.join("snapshot.sqlite3"), b"invented interrupted work").unwrap();
    }
    let opened = RuntimeBook::open_existing(&book, &keys).await.unwrap();
    assert!(!recognized.exists());
    assert!(untouched.join("snapshot.sqlite3").exists());
    opened.close().await;
    drop(opened);
    // An unmarked directory in an owned checkpoint cannot authorize deletion.
    fs::create_dir(&recognized).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&recognized, fs::Permissions::from_mode(0o700)).unwrap();
    let database = fs::read(book.join("book.sqlite3")).unwrap();
    assert!(matches!(
        RuntimeBook::open_existing(&book, &keys).await,
        Err(RuntimeError::RecoveryRequired)
    ));
    assert!(recognized.exists());
    assert_eq!(fs::read(book.join("book.sqlite3")).unwrap(), database);
}
#[tokio::test]
async fn explicit_initialization_then_existing_open_never_regenerates_keys() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    assert!(RuntimeBook::open_existing(&path, &keys).await.is_err());
    assert!(!path.exists());
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 0);
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    assert_eq!(book.root(), path);
    assert_eq!(book.attachments(), path.join("attachments"));
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
    assert!(matches!(
        RuntimeBook::open_existing(&path, &keys).await,
        Err(RuntimeError::BookBusy)
    ));
    assert!(RuntimeBook::init_new(&path, &keys).await.is_err());
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
    book.close().await;
    drop(book);
    let reopened = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    reopened.close().await;
}
#[tokio::test]
async fn missing_key_or_binding_fails_without_replacing_or_creating_a_key() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let original = fs::read(path.join("device-key.json")).unwrap();
    keys.values.borrow_mut().clear();
    assert!(matches!(
        RuntimeBook::open_existing(&path, &keys).await,
        Err(RuntimeError::Key(KeyStoreError::Missing))
    ));
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(path.join("device-key.json")).unwrap(), original);
    fs::remove_file(path.join("device-key.json")).unwrap();
    assert!(RuntimeBook::open_existing(&path, &keys).await.is_err());
    assert!(!path.join("device-key.json").exists());
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn corrupted_database_is_rejected_before_writable_open() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let database = path.join("book.sqlite3");
    fs::write(&database, b"not a SQLite financial book").unwrap();
    let original = fs::read(&database).unwrap();
    assert!(RuntimeBook::open_existing(&path, &keys).await.is_err());
    assert_eq!(fs::read(database).unwrap(), original);
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_or_public_book_directory_is_rejected() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let link = fixture.0.join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(matches!(
        RuntimeBook::open_existing(&link, &keys).await,
        Err(RuntimeError::UnsafePath)
    ));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        RuntimeBook::open_existing(&path, &keys).await,
        Err(RuntimeError::UnsafePath)
    ));
}
#[tokio::test]
async fn loopback_guard_covers_static_assets_unknown_routes_and_api() {
    assert_eq!(LOCAL_ADDRESS, "127.0.0.1:3000");
    let fixture = Fixture::new();
    let assets = fixture.0.join("dist");
    fs::create_dir(&assets).unwrap();
    fs::create_dir(assets.join("assets")).unwrap();
    fs::write(
        assets.join("index.html"),
        "<!doctype html><title>kAccounting-Extra-01</title>",
    )
    .unwrap();
    fs::write(assets.join("assets/app.js"), "console.log('ready')").unwrap();
    let pool = kaccounting_backend::db::open_memory_database()
        .await
        .unwrap();
    let router = application_router(pool, &assets).unwrap();
    for path in ["/", "/assets/app.js", "/api/health", "/unknown"] {
        for bad_host in ["localhost:3000", "evil.example", "127.0.0.1:9999"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", bad_host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{path} {bad_host}"
            );
        }
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", LOCAL_ADDRESS)
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", LOCAL_ADDRESS)
                    .header("sec-fetch-site", "cross-site")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", LOCAL_ADDRESS)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers().contains_key("content-security-policy"));
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("kAccounting-Extra-01")
    );
    let duplicate = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", LOCAL_ADDRESS)
                .header("host", LOCAL_ADDRESS)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::FORBIDDEN);
    let write = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/accounts")
                .header("host", LOCAL_ADDRESS)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::FORBIDDEN);
    let escape = router
        .oneshot(
            Request::builder()
                .uri("/assets/%2e%2e%2findex.html")
                .header("host", LOCAL_ADDRESS)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(escape.status(), StatusCode::NOT_FOUND);
}
