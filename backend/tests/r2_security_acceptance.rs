//! Independent second-round security regressions: fake keys and disposable paths only.
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    storage_operations::{OperationError, RequestInput, SelectionKind, StorageOperations},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct Keys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl Keys {
    fn read_key(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut keys = self.0.lock().unwrap();
        if keys.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        keys.insert(id.into(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.read_key(id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.put(id.as_str(), bytes)
    }
}
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.read_key(id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.put(id, bytes)
    }
}
struct Fixture {
    root: PathBuf,
    operations: StorageOperations,
}
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|v| format!("{v:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-r2-security-{name}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("chosen")).unwrap();
        Self {
            root,
            operations: StorageOperations::new(
                Arc::new(Keys::default()),
                Arc::new(Keys::default()),
            ),
        }
    }
    async fn rejected(&self, token: String) {
        match self.operations.request(RequestInput { destination: token }) {
            Err(OperationError::InvalidSelection) => (),
            Ok(job) => loop {
                let status = self.operations.status(&job.operation_id).unwrap();
                if status.state != "running" {
                    assert_eq!(
                        status.state, "failed",
                        "a replaced native selection must never redirect a write"
                    );
                    break;
                }
                tokio::task::yield_now().await;
            },
            other => panic!("unexpected admission: {other:?}"),
        }
        assert!(!self.root.join("chosen/request.json").exists());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn destination_capability_cannot_follow_replaced_selected_directory() {
    let f = Fixture::new();
    let token = f
        .operations
        .register_selection(
            SelectionKind::RequestDestination,
            f.root.join("chosen/request.json"),
        )
        .unwrap();
    std::fs::rename(f.root.join("chosen"), f.root.join("original")).unwrap();
    std::fs::create_dir(f.root.join("chosen")).unwrap();
    f.rejected(token).await;
}

#[cfg(unix)]
#[tokio::test]
async fn destination_capability_cannot_follow_symlink_replacement() {
    let f = Fixture::new();
    let token = f
        .operations
        .register_selection(
            SelectionKind::RequestDestination,
            f.root.join("chosen/request.json"),
        )
        .unwrap();
    std::fs::rename(f.root.join("chosen"), f.root.join("original")).unwrap();
    std::os::unix::fs::symlink(f.root.join("original"), f.root.join("chosen")).unwrap();
    f.rejected(token).await;
    assert!(!f.root.join("original/request.json").exists());
}

#[test]
fn forged_tailscale_identity_never_authorizes_regular_listener() {
    use axum::http::{HeaderMap, HeaderValue, Method};
    use kaccounting_backend::private_access::{
        PrivateAccessError, PrivateAccessPolicy, TrustedIngress,
    };
    let policy = PrivateAccessPolicy::new("book.example.ts.net", "owner@example.com").unwrap();
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("host", "book.example.ts.net"),
        ("origin", "https://book.example.ts.net"),
        ("tailscale-user-login", "owner@example.com"),
        ("sec-fetch-site", "same-origin"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    for method in [Method::GET, Method::POST, Method::DELETE, Method::OPTIONS] {
        assert_eq!(
            policy.authorize(&headers, &method, TrustedIngress::RegularLoopback),
            Err(PrivateAccessError::UntrustedIngress)
        );
    }
}
