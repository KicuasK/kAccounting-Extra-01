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

async fn select_legacy_without_identity(fixture: &Fixture) -> String {
    fs::remove_file(fixture.root.join("first/backup-identity.kae")).unwrap();
    let selected = json(
        fixture
            .request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    )
    .await;
    selected["selection_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn explicit_backup_identity_setup_updates_existing_routes_without_keys_or_selection_change() {
    let f = Fixture::new().await;
    let generation = select_legacy_without_identity(&f).await;
    let reads = f.keys.reads.load(Ordering::SeqCst);
    let keys = f.keys.device.lock().unwrap().len();
    let binding = fs::read(f.root.join("first/device-key.json")).unwrap();
    assert_eq!(
        json(guarded(&f, "GET", "/api/backups/identity", "", &[&generation]).await).await,
        serde_json::json!({"initialized":false})
    );
    let response = guarded(&f, "POST", "/api/backups", "{}", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json(response).await["code"], "backup_identity_missing");
    let response = guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        json(response).await,
        serde_json::json!({"initialized":true})
    );
    let identity = fs::read(f.root.join("first/backup-identity.kae")).unwrap();
    assert_eq!(
        json(guarded(&f, "GET", "/api/backups/identity", "", &[&generation]).await).await,
        serde_json::json!({"initialized":true})
    );
    let repeated = guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation]).await;
    assert_eq!(repeated.status(), StatusCode::CONFLICT);
    assert_eq!(
        json(repeated).await["error"]["code"],
        "backup_identity_exists"
    );
    let response = guarded(&f, "POST", "/api/backups", "{}", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json(response).await["verified"], true);
    assert_eq!(f.keys.reads.load(Ordering::SeqCst), reads);
    assert_eq!(f.keys.device.lock().unwrap().len(), keys);
    assert_eq!(
        fs::read(f.root.join("first/device-key.json")).unwrap(),
        binding
    );
    assert_eq!(
        fs::read(f.root.join("first/backup-identity.kae")).unwrap(),
        identity
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["selection_id"],
        generation
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn backup_identity_requires_exact_generation_origin_and_empty_input() {
    let f = Fixture::new().await;
    for method in ["GET", "POST"] {
        let response = guarded(&f, method, "/api/backups/identity", "{}", &["unselected"]).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(json(response).await["error"]["code"], "selection_required");
    }
    let generation = select_legacy_without_identity(&f).await;
    for method in ["GET", "POST"] {
        for guards in [
            vec![],
            vec!["malformed"],
            vec![generation.as_str(), generation.as_str()],
        ] {
            assert_eq!(
                guarded(&f, method, "/api/backups/identity", "{}", &guards)
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            guarded(
                &f,
                method,
                "/api/backups/identity",
                "{}",
                &["00000000000000000000000000000000"]
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
    }
    for body in [
        r#"{"path":"/private/tmp/other"}"#,
        r#"{"key_id":"fake"}"#,
        r#"{"overwrite":true}"#,
        "null",
    ] {
        assert_eq!(
            guarded(&f, "POST", "/api/backups/identity", body, &[&generation])
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = f
        .session
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/backups/identity")
                .header("host", "127.0.0.1:3000")
                .header("origin", "https://wrong.example")
                .header("x-kaccounting-selection", &generation)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!f.root.join("first/backup-identity.kae").exists());
    f.session.shutdown().await;
    assert_eq!(
        guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation])
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn backup_identity_never_treats_corrupt_or_substituted_file_as_missing_or_overwrites_it() {
    let f = Fixture::new().await;
    let generation = select_legacy_without_identity(&f).await;
    let file = f.root.join("first/backup-identity.kae");
    fs::write(&file, b"preserve invalid identity").unwrap();
    let response = guarded(&f, "GET", "/api/backups/identity", "", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json(response).await["error"]["code"],
        "backup_identity_invalid"
    );
    assert_eq!(
        guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fs::read(&file).unwrap(), b"preserve invalid identity");
    fs::remove_file(&file).unwrap();
    let unrelated = f.root.join("unrelated-identity");
    fs::write(&unrelated, b"unrelated bytes").unwrap();
    std::os::unix::fs::symlink(&unrelated, &file).unwrap();
    assert_eq!(
        guarded(&f, "GET", "/api/backups/identity", "", &[&generation])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fs::read(unrelated).unwrap(), b"unrelated bytes");
    let previous = f.root.join("first-retained");
    fs::rename(f.root.join("first"), &previous).unwrap();
    fs::create_dir(f.root.join("first")).unwrap();
    fs::set_permissions(f.root.join("first"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        guarded(&f, "GET", "/api/backups/identity", "", &[&generation])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let response = guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json(response).await["error"]["code"],
        "backup_identity_invalid"
    );
    assert!(!f.root.join("first/backup-identity.kae").exists());
    f.session.shutdown().await;
}

#[tokio::test]
async fn backup_identity_setup_refuses_held_response_lease_then_succeeds_explicitly() {
    let f = Fixture::new().await;
    let generation = select_legacy_without_identity(&f).await;
    let held = guarded(&f, "GET", "/api/accounts", "", &[&generation]).await;
    let response = guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json(response).await["error"]["code"], "backup_busy");
    assert!(!f.root.join("first/backup-identity.kae").exists());
    drop(held);
    assert_eq!(
        guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation])
            .await
            .status(),
        StatusCode::CREATED
    );
    f.session.shutdown().await;
}

#[test]
fn cancelled_identity_request_keeps_owned_worker_and_generation_pinned_until_reconciled_or_shutdown()
 {
    use std::future::Future;
    for close_during_setup in [false, true] {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let f = Fixture::new().await;
            let generation = select_legacy_without_identity(&f).await;
            let reads = f.keys.reads.load(Ordering::SeqCst);
            let key_count = f.keys.device.lock().unwrap().len();
            let (release, wait) = std::sync::mpsc::channel();
            let (entered, ready) = tokio::sync::oneshot::channel();
            // Occupy the sole blocking worker so setup's owned operation can be
            // deterministically cancelled by its caller before filesystem work starts.
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                let _ = wait.recv_timeout(std::time::Duration::from_secs(15));
            });
            ready.await.unwrap();
            let request = Request::builder()
                .method("POST")
                .uri("/api/backups/identity")
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://127.0.0.1:3000")
                .header("x-kaccounting-selection", &generation)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            let mut pending = Box::pin(f.session.router().oneshot(request));
            std::future::poll_fn(|cx| {
                assert!(pending.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let busy = guarded(&f, "POST", "/api/backups/identity", "{}", &[&generation]).await;
            assert_eq!(busy.status(), StatusCode::CONFLICT);
            assert_eq!(json(busy).await["error"]["code"], "backup_busy");
            assert!(!f.root.join("first/backup-identity.kae").exists());
            drop(pending); // The client has no confirmed outcome; the worker still owns admission.
            if close_during_setup {
                let mut closing = Box::pin(f.session.shutdown());
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(30), &mut closing)
                        .await
                        .is_err()
                );
                release.send(()).unwrap();
                blocker.await.unwrap();
                closing.await;
                assert_eq!(f.keys.reads.load(Ordering::SeqCst), reads);
                let reopened = RuntimeBook::open_existing(f.root.join("first"), f.keys.as_ref())
                    .await
                    .unwrap();
                assert!(reopened.backup_id().is_some());
                reopened.close().await;
            } else {
                let guards = [&generation[..]];
                let mut status = Box::pin(guarded(&f, "GET", "/api/backups/identity", "", &guards));
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(30), &mut status)
                        .await
                        .is_err(),
                    "in-flight setup must never be reported missing"
                );
                release.send(()).unwrap();
                blocker.await.unwrap();
                let response = status.await;
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(
                    json(response).await,
                    serde_json::json!({"initialized":true})
                );
                assert_eq!(f.keys.reads.load(Ordering::SeqCst), reads);
                f.session.shutdown().await;
            }
            assert_eq!(f.keys.device.lock().unwrap().len(), key_count);
        });
    }
}

async fn guarded(
    fixture: &Fixture,
    method: &str,
    path: &str,
    body: &str,
    guards: &[&str],
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "application/json");
    for guard in guards {
        request = request.header("x-kaccounting-selection", *guard);
    }
    fixture
        .session
        .router()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn generation_guards_bind_reads_writes_and_transitions_before_key_access() {
    let f = Fixture::new().await;
    let empty = json(f.request("GET", "/api/session", "").await).await;
    assert!(empty["selection_id"].is_null());
    let first = json(
        guarded(
            &f,
            "POST",
            "/api/session/selection",
            r#"{"book_id":"first"}"#,
            &["unselected"],
        )
        .await,
    )
    .await;
    let generation = first["selection_id"].as_str().unwrap();
    assert_eq!(generation.len(), 32);
    let current = guarded(&f, "GET", "/api/accounts/tree", "", &[generation]).await;
    assert_eq!(current.status(), StatusCode::OK);
    drop(current);
    let reads = f.keys.reads.load(Ordering::SeqCst);
    for (method, path, body) in [
        ("GET", "/api/accounts/tree", ""),
        (
            "POST",
            "/api/accounts",
            r#"{"name":"must not create","element":"asset","kind":"posting"}"#,
        ),
        ("POST", "/api/session/selection", r#"{"book_id":"second"}"#),
        ("DELETE", "/api/session/selection", ""),
    ] {
        let response = guarded(&f, method, path, body, &["unselected"]).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(json(response).await["error"]["code"], "selection_changed");
    }
    assert_eq!(f.keys.reads.load(Ordering::SeqCst), reads);
    let accounts = json(guarded(&f, "GET", "/api/accounts", "", &[generation]).await).await;
    assert_eq!(accounts, serde_json::json!([]));
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn returning_to_same_book_never_revives_an_old_generation() {
    let f = Fixture::new().await;
    let first = json(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    )
    .await;
    let original = first["selection_id"].as_str().unwrap();
    let same = json(
        guarded(
            &f,
            "POST",
            "/api/session/selection",
            r#"{"book_id":"first"}"#,
            &[original],
        )
        .await,
    )
    .await;
    assert_eq!(same["selection_id"], original);
    drop(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await,
    );
    let returned = json(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    )
    .await;
    assert_ne!(returned["selection_id"], original);
    assert_eq!(
        guarded(&f, "GET", "/api/accounts/tree", "", &[original])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let current = returned["selection_id"].as_str().unwrap();
    assert_eq!(
        guarded(&f, "DELETE", "/api/session/selection", "", &[current])
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        guarded(
            &f,
            "POST",
            "/api/session/selection",
            r#"{"book_id":"first"}"#,
            &[current]
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn native_download_generation_is_checked_before_binary_dispatch() {
    let f = Fixture::new().await;
    let selected = json(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await,
    )
    .await;
    let generation = selected["selection_id"].as_str().unwrap();
    let path = format!(
        "/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content?download_selection={generation}"
    );
    // Missing attachment is a domain error, rather than a guard error.
    let valid = json(f.request("GET", &path, "").await).await;
    assert_ne!(valid["error"]["code"], "invalid_selection_guard");
    assert_ne!(valid["error"]["code"], "selection_changed");
    for suffix in ["&download_selection=bad", "&x=1"] {
        assert_eq!(
            f.request("GET", &format!("{path}{suffix}"), "")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.request("POST", &path, "").await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.request(
            "GET",
            &format!("/api/accounts?download_selection={generation}"),
            ""
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    drop(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await,
    );
    assert_eq!(
        f.request("GET", &path, "").await.status(),
        StatusCode::CONFLICT
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn malformed_or_duplicate_generation_headers_fail_without_key_access() {
    let f = Fixture::new().await;
    let reads = f.keys.reads.load(Ordering::SeqCst);
    for guards in [
        vec![""],
        vec!["bad"],
        vec!["unselected", "unselected"],
        vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"],
    ] {
        for (method, path, body) in [
            ("GET", "/api/accounts/tree", ""),
            ("POST", "/api/session/selection", r#"{"book_id":"first"}"#),
            ("DELETE", "/api/session/selection", ""),
        ] {
            let response = guarded(&f, method, path, body, &guards).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                json(response).await["error"]["code"],
                "invalid_selection_guard"
            );
        }
    }
    assert_eq!(f.keys.reads.load(Ordering::SeqCst), reads);
    f.session.shutdown().await;
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
