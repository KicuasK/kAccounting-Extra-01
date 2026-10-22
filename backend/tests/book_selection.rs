//! Disposable books and in-memory keys only.
use kaccounting_backend::{
    book_selection::{BookSelectionSession, discover_books},
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::{RuntimeBook, RuntimeError},
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
async fn discovery_is_keyless_read_only_and_every_session_starts_empty() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let first = RuntimeBook::init_new(fixture.0.join("9007199254740993"), &keys)
        .await
        .unwrap();
    let before = fs::read_dir(&first).unwrap().count();
    let candidates = discover_books(&fixture.0).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, "9007199254740993");
    assert_eq!(candidates[0].status, "selection_required");
    assert_eq!(fs::read_dir(&first).unwrap().count(), before);
    assert!(!first.join(".runtime.lock").exists());
    assert!(BookSelectionSession::new().current().is_none());
    let json = serde_json::to_string(&candidates).unwrap();
    assert!(!json.contains("key_id"));
    assert!(!json.contains("sqlite"));
    assert_eq!(keys.inserts.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn failed_selection_retains_current_and_success_releases_previous_lock() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let first = RuntimeBook::init_new(fixture.0.join("first"), &keys)
        .await
        .unwrap();
    let second = RuntimeBook::init_new(fixture.0.join("second"), &keys)
        .await
        .unwrap();
    let mut session = BookSelectionSession::new();
    session.select(&first, &keys).await.unwrap();
    session.select(&first, &keys).await.unwrap();
    let busy = RuntimeBook::open_existing(&second, &keys).await.unwrap();
    assert!(matches!(
        session.select(&second, &keys).await,
        Err(RuntimeError::BookBusy)
    ));
    assert_eq!(session.current().unwrap().root(), first);
    assert!(matches!(
        RuntimeBook::open_existing(&first, &keys).await,
        Err(RuntimeError::BookBusy)
    ));
    assert!(
        session
            .select(&second, &MemoryKeys::default())
            .await
            .is_err()
    );
    assert_eq!(session.current().unwrap().root(), first);
    busy.close().await;
    drop(busy);
    assert!(
        session
            .select(&second, &MemoryKeys::default())
            .await
            .is_err()
    );
    assert_eq!(session.current().unwrap().root(), first);
    session.select(&second, &keys).await.unwrap();
    assert_eq!(session.current().unwrap().root(), second);
    let reopened = RuntimeBook::open_existing(&first, &keys).await.unwrap();
    reopened.close().await;
    session.close().await;
    assert!(session.current().is_none());
    assert!(BookSelectionSession::new().current().is_none());
}

#[tokio::test]
async fn discovery_excludes_unknown_and_symlink_entries() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let first = RuntimeBook::init_new(fixture.0.join("first"), &keys)
        .await
        .unwrap();
    fs::create_dir(fixture.0.join("unknown")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&first, fixture.0.join("alias")).unwrap();
    let books = discover_books(&fixture.0).unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].id, "first");
    fs::write(first.join("device-key.json"), b"{}").unwrap();
    assert!(discover_books(&fixture.0).unwrap().is_empty());
    assert!(discover_books(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn unsafe_candidates_and_selection_paths_are_rejected_without_switching() {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let first = RuntimeBook::init_new(fixture.0.join("first"), &keys)
        .await
        .unwrap();
    let second = RuntimeBook::init_new(fixture.0.join("second"), &keys)
        .await
        .unwrap();
    let mut session = BookSelectionSession::new();
    session.select(&first, &keys).await.unwrap();
    let alias = fixture.0.join("alias");
    std::os::unix::fs::symlink(&second, &alias).unwrap();
    assert!(matches!(
        session.select(&alias, &keys).await,
        Err(RuntimeError::UnsafePath)
    ));
    assert!(
        session
            .select(fixture.0.join("second/../second"), &keys)
            .await
            .is_err()
    );
    assert_eq!(session.current().unwrap().root(), first);
    fs::set_permissions(
        second.join("device-key.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let books = discover_books(&fixture.0).unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].id, "first");
    assert!(session.select(&second, &keys).await.is_err());
    assert_eq!(session.current().unwrap().root(), first);
    session.close().await;
}
