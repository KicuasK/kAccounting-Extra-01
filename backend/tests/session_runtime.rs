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
    device_failure: AtomicI32,
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
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
#[tokio::test]
async fn actionable_key_errors_preserve_current_selection_without_replacement_keys() {
    let f = Fixture::new().await;
    let response = f
        .request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let key_count = f.keys.device.lock().unwrap().len();
    for (status, code) in [
        (-25308, "keychain_interaction_required"),
        (-128, "keychain_user_cancelled"),
        (-34018, "keychain_unavailable"),
    ] {
        f.keys.device_failure.store(status, Ordering::SeqCst);
        let response = f
            .request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let result = json(response).await;
        assert_eq!(result["error"]["code"], code);
        assert!(!result.to_string().contains(&status.to_string()));
        assert_eq!(
            json(f.request("GET", "/api/session", "").await).await["book_id"],
            "first"
        );
        assert_eq!(f.keys.device.lock().unwrap().len(), key_count);
    }
    f.keys.device_failure.store(0, Ordering::SeqCst);
    f.keys.device.lock().unwrap().clear();
    let response = f
        .request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json(response).await["error"]["code"],
        "missing_original_key"
    );
    assert!(f.keys.device.lock().unwrap().is_empty());
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    f.session.shutdown().await;
}
#[tokio::test]
async fn native_dialog_tokens_never_follow_a_changed_selection() {
    use kaccounting_backend::storage_operations::{OperationError, SelectionKind};
    let f = Fixture::new().await;
    assert!(f.session.native_selection_session().await.is_err());
    let response = f
        .request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let original = f.session.native_selection_session().await.unwrap();
    assert!(
        f.session
            .register_storage_selection_for(&original, SelectionKind::Book, f.root.join("first"))
            .await
            .is_ok()
    );
    let response = f
        .request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    assert_eq!(
        f.session
            .register_storage_selection_for(&original, SelectionKind::Book, f.root.join("second"))
            .await,
        Err(OperationError::InvalidSelection)
    );
    let response = f
        .request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    assert_eq!(
        f.session
            .register_storage_selection_for(&original, SelectionKind::Book, f.root.join("first"))
            .await,
        Err(OperationError::InvalidSelection)
    );
    let current = f.session.native_selection_session().await.unwrap();
    assert!(
        f.session
            .register_storage_selection_for(&current, SelectionKind::Book, f.root.join("first"))
            .await
            .is_ok()
    );
    f.session.shutdown().await;
}
#[tokio::test]
async fn fresh_session_requires_choice_and_discovery_never_reads_keys() {
    let f = Fixture::new().await;
    let before = f.keys.reads.load(Ordering::SeqCst);
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["selection_required"],
        true
    );
    let books = json(f.request("GET", "/api/session/books", "").await).await;
    assert_eq!(books["books"].as_array().unwrap().len(), 2);
    assert_eq!(
        f.request("GET", "/api/accounts", "").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(f.request("GET", "/", "").await.status(), StatusCode::OK);
    assert_eq!(f.keys.reads.load(Ordering::SeqCst), before);
    f.session.shutdown().await;
}
#[tokio::test]
async fn choice_opens_and_clear_returns_to_unselected() {
    let f = Fixture::new().await;
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/api/accounts", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    assert_eq!(
        f.request("DELETE", "/api/session/selection", "")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/api/accounts", "").await.status(),
        StatusCode::CONFLICT
    );
    let fresh =
        SessionRuntime::new(&f.root, f.root.join("dist"), f.keys.clone(), f.keys.clone()).unwrap();
    let body = fresh
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(body).await["selection_required"], true);
    f.session.shutdown().await;
}
#[tokio::test]
async fn invalid_paths_and_unlisted_books_do_not_read_keys() {
    let f = Fixture::new().await;
    let before = f.keys.reads.load(Ordering::SeqCst);
    for id in [
        "../first",
        "/first",
        "first/../second",
        ".hidden",
        "first\\second",
        "",
    ] {
        let response = f
            .request(
                "POST",
                "/api/session/selection",
                &serde_json::json!({"book_id":id}).to_string(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"missing"}"#)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.keys.reads.load(Ordering::SeqCst), before);
    f.session.shutdown().await;
}
#[tokio::test]
async fn response_body_holds_book_lock_and_failed_switch_preserves_selection() {
    let f = Fixture::new().await;
    drop(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    );
    let held = f.request("GET", "/api/accounts", "").await;
    assert_eq!(held.status(), StatusCode::OK);
    assert_eq!(
        f.request("DELETE", "/api/session/selection", "")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    drop(held);
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"missing"}"#)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "second"
    );
    f.session.shutdown().await;
}
#[tokio::test]
async fn boundary_guards_session_static_and_unknown_routes() {
    let f = Fixture::new().await;
    for path in [
        "/",
        "/assets/missing.js",
        "/api/session",
        "/api/session/books",
        "/unknown",
    ] {
        let response = f
            .session
            .router()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = f
        .session
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/session/selection")
                .header("host", "127.0.0.1:3000")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"book_id":"first"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    f.session.shutdown().await;
}
#[tokio::test]
async fn one_candidate_still_requires_selection_and_failed_key_read_preserves_current() {
    let f = Fixture::new().await;
    fs::remove_dir_all(f.root.join("second")).unwrap();
    let fresh =
        SessionRuntime::new(&f.root, f.root.join("dist"), f.keys.clone(), f.keys.clone()).unwrap();
    let response = fresh
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["selection_required"], true);
    RuntimeBook::init_new(f.root.join("second"), f.keys.as_ref())
        .await
        .unwrap();
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    f.keys.device.lock().unwrap().clear();
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    assert_eq!(
        f.request("GET", "/api/accounts", "").await.status(),
        StatusCode::OK
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn cancelled_shutdown_still_drains_response_lease_and_releases_selected_book() {
    use std::future::Future;
    let f = Fixture::new().await;
    drop(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    );
    let response = f.request("GET", "/api/accounts", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut closing = Box::pin(f.session.shutdown());
    std::future::poll_fn(|cx| {
        assert!(
            closing.as_mut().poll(cx).is_pending(),
            "response lease did not delay shutdown"
        );
        std::task::Poll::Ready(())
    })
    .await;
    // Simulate the native caller/disconnected coordinator cancelling its wait.
    drop(closing);
    drop(response);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while f.session.native_selection_session().await.is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "cancelled shutdown permanently retained the selected book and its process lock"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn shutdown_refuses_new_data_requests_while_existing_response_drains() {
    use std::future::Future;
    let f = Fixture::new().await;
    drop(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    );
    let held = f.request("GET", "/api/accounts", "").await;
    let mut closing = Box::pin(f.session.shutdown());
    std::future::poll_fn(|cx| {
        assert!(closing.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let mut late = Box::pin(f.request("GET", "/api/accounts", ""));
    let response = std::future::poll_fn(|cx| match late.as_mut().poll(cx) {
        std::task::Poll::Ready(response) => std::task::Poll::Ready(response),
        std::task::Poll::Pending => {
            panic!("shutdown admitted a new data request behind its draining lease")
        }
    })
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(late);
    drop(held);
    closing.await;
}
