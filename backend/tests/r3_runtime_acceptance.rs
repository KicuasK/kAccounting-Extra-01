//! Disposable process-local session acceptance; no real OS key access.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    runtime::RuntimeBook,
    session_runtime::SessionRuntime,
};
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys {
    device: Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
    recipient: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    reads: AtomicUsize,
    next_device_error: Mutex<Option<KeyStoreError>>,
    device_failure: AtomicI32,
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.next_device_error.lock().unwrap().take() {
            return Err(error);
        }
        let failure = self.device_failure.load(Ordering::SeqCst);
        if failure != 0 {
            return Err(KeyStoreError::Unavailable(failure));
        }
        self.device
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &DeviceKeyId, value: &[u8]) -> Result<(), KeyStoreError> {
        let mut keys = self.device.lock().unwrap();
        if keys.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        keys.insert(id.clone(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
}
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.recipient
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, value: &[u8]) -> Result<(), KeyStoreError> {
        let mut keys = self.recipient.lock().unwrap();
        if keys.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        keys.insert(id.into(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    keys: Arc<Keys>,
    session: SessionRuntime,
}
impl Fixture {
    async fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "ka-session-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let keys = Arc::new(Keys::default());
        RuntimeBook::init_new(root.join("first"), keys.as_ref())
            .await
            .unwrap();
        RuntimeBook::init_new(root.join("second"), keys.as_ref())
            .await
            .unwrap();
        let assets = root.join("dist");
        fs::create_dir(&assets).unwrap();
        fs::write(assets.join("index.html"), "<!doctype html>session test").unwrap();
        let session = SessionRuntime::new(&root, &assets, keys.clone(), keys.clone()).unwrap();
        Self {
            root,
            keys,
            session,
        }
    }
    async fn request(&self, method: &str, path: &str, body: &str) -> axum::response::Response {
        self.session
            .router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "127.0.0.1:3000")
                    .header("origin", "http://127.0.0.1:3000")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap()
}
use axum::http::{HeaderMap, HeaderValue, Method};
use kaccounting_backend::{
    private_access::{PrivateAccessPolicy, TrustedIngress},
    storage_operations::SelectionKind,
};

#[tokio::test]
async fn native_dialog_epoch_does_not_revive_after_switch_back_or_cross_sessions() {
    let f = Fixture::new().await;
    assert!(f.session.native_selection_session().await.is_err());
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    let first_epoch = f.session.native_selection_session().await.unwrap();
    // Choosing the already active book does not invalidate an in-flight dialog.
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    assert!(
        f.session
            .register_storage_selection_for(&first_epoch, SelectionKind::Book, f.root.join("first"))
            .await
            .is_ok()
    );
    let other_session =
        SessionRuntime::new(&f.root, f.root.join("dist"), f.keys.clone(), f.keys.clone()).unwrap();
    assert!(
        other_session
            .register_storage_selection_for(&first_epoch, SelectionKind::Book, f.root.join("first"))
            .await
            .is_err()
    );
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    assert!(
        f.session
            .register_storage_selection_for(&first_epoch, SelectionKind::Book, f.root.join("first"))
            .await
            .is_err()
    );
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    assert!(
        f.session
            .register_storage_selection_for(&first_epoch, SelectionKind::Book, f.root.join("first"))
            .await
            .is_err()
    );
    let fresh_epoch = f.session.native_selection_session().await.unwrap();
    assert!(
        f.session
            .register_storage_selection_for(&fresh_epoch, SelectionKind::Book, f.root.join("first"))
            .await
            .is_ok()
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn diagnostic_key_failures_preserve_current_selection_and_never_regenerate() {
    let f = Fixture::new().await;
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    let count = f.keys.device.lock().unwrap().len();
    for (failure, expected) in [
        (KeyStoreError::Missing, "missing_original_key"),
        (
            KeyStoreError::Unavailable(-25308),
            "keychain_interaction_required",
        ),
        (KeyStoreError::Unavailable(-128), "keychain_user_cancelled"),
        (KeyStoreError::Unavailable(-123456), "keychain_unavailable"),
    ] {
        *f.keys.next_device_error.lock().unwrap() = Some(failure);
        let response = f
            .request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = json(response).await;
        assert_eq!(body["error"]["code"], expected);
        let serialized = body.to_string();
        assert!(!serialized.contains(f.root.to_str().unwrap()));
        assert!(!serialized.contains("-123456"));
        assert_eq!(f.keys.device.lock().unwrap().len(), count);
        assert_eq!(
            json(f.request("GET", "/api/session", "").await).await["book_id"],
            "first"
        );
        assert_eq!(
            f.request("GET", "/api/accounts", "").await.status(),
            StatusCode::OK
        );
    }
    f.session.shutdown().await;
}

#[test]
fn private_policy_never_accepts_loopback_assertions_null_origins_or_duplicate_identity() {
    let policy = PrivateAccessPolicy::new("book.example.ts.net", "owner@example.com").unwrap();
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("host", "book.example.ts.net"),
        ("tailscale-user-login", "owner@example.com"),
        ("origin", "https://book.example.ts.net"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    for method in [Method::GET, Method::POST, Method::DELETE] {
        assert!(
            policy
                .authorize(&headers, &method, TrustedIngress::RegularLoopback)
                .is_err()
        );
        let mut null_origin = headers.clone();
        null_origin.insert("origin", HeaderValue::from_static("null"));
        assert!(
            policy
                .authorize(
                    &null_origin,
                    &method,
                    TrustedIngress::AuthenticatedPrivateProxy
                )
                .is_err()
        );
        let mut duplicate_identity = headers.clone();
        duplicate_identity.append(
            "tailscale-user-login",
            HeaderValue::from_static("owner@example.com"),
        );
        assert!(
            policy
                .authorize(
                    &duplicate_identity,
                    &method,
                    TrustedIngress::AuthenticatedPrivateProxy
                )
                .is_err()
        );
        let mut no_identity = headers.clone();
        no_identity.remove("tailscale-user-login");
        assert!(
            policy
                .authorize(
                    &no_identity,
                    &method,
                    TrustedIngress::AuthenticatedPrivateProxy
                )
                .is_err()
        );
    }
}
