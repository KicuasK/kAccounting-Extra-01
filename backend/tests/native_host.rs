//! Headless host smoke tests with private fake helper processes and memory keys.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    native_host::{NativeHost, NativeHostError, NativeHostEvent},
    recipient_transfer::RecipientKeyStore,
    runtime::RuntimeBook,
    session_runtime::SessionRuntime,
    storage_operations::SelectionKind,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys(
    Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    std::sync::atomic::AtomicUsize,
);
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, value: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().unwrap();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.into(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, value: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), value)
    }
}
struct Fixture {
    root: PathBuf,
    host: NativeHost,
    keys: Arc<Keys>,
}
impl Fixture {
    async fn selection_id(&self) -> String {
        let response = self
            .host
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
        let raw = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["selection_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    async fn new(body: &str) -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "ka-native-host-{:032x}",
            u128::from_be_bytes(random)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let helper = root.join("fake-helper");
        fs::write(
            &helper,
            format!(
                "#!/usr/bin/python3\nimport json,sys,os,time\nr=json.load(sys.stdin)\ninitial=r.pop('initial_directory',None)\n{body}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let keys = Arc::new(Keys::default());
        RuntimeBook::init_new(root.join("book"), keys.as_ref())
            .await
            .unwrap();
        let assets = root.join("assets");
        fs::create_dir(&assets).unwrap();
        fs::write(assets.join("index.html"), "<!doctype html>fake host").unwrap();
        let session = SessionRuntime::new(&root, &assets, keys.clone(), keys.clone()).unwrap();
        let owner = fs::metadata(&helper).unwrap().uid();
        let host = NativeHost::new(session, &helper, owner).unwrap();
        Self { root, host, keys }
    }
    async fn select_book(&self) {
        let response = self
            .host
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/selection")
                    .header("host", "127.0.0.1:3000")
                    .header("origin", "http://127.0.0.1:3000")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"book_id":"book"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    async fn wait_marker(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.root.join("started").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fake helper did not start"
            );
            tokio::task::yield_now().await;
        }
    }
}
fn browser_request(
    body: impl Into<Body>,
    selection: Option<&str>,
    origin: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/native/selections")
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/json");
    if let Some(selection) = selection {
        request = request.header("x-kaccounting-selection", selection);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    request.body(body.into()).unwrap()
}
fn create_book_request(body: &'static str, selection: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/native/books")
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "application/json");
    if let Some(selection) = selection {
        request = request.header("x-kaccounting-selection", selection);
    }
    request.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn browser_creates_first_book_only_from_real_picker_and_does_not_select_it() {
    let f = Fixture::new("assert initial==os.getcwd()\nprint(json.dumps(dict(r,result={'outcome':'selected','path':os.path.join(initial,'new-book')})))").await;
    // Remove only this disposable fixture's pre-created book to exercise an empty collection.
    fs::remove_dir_all(f.root.join("book")).unwrap();
    let response = f
        .host
        .browser_router()
        .oneshot(create_book_request("{}", Some("unselected")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let raw = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
        serde_json::json!({"book_id":"new-book","selection_required":true})
    );
    assert!(f.root.join("new-book/book.sqlite3").is_file());
    let response = f
        .host
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
    let raw = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["book_id"],
        serde_json::Value::Null
    );
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_create_rejects_http_paths_stale_guards_and_cancels_without_keys() {
    let f = Fixture::new(
        "open('started','w').close()\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
    )
    .await;
    let before = f.keys.0.lock().unwrap().len();
    for body in [
        r#"{"path":"/private/tmp/new"}"#,
        r#"{"book_id":"new"}"#,
        r#"{"kind":"book-destination"}"#,
        r#"{"capability_id":"fake"}"#,
        r#"{"initial_directory":"/private/tmp"}"#,
        "null",
    ] {
        assert_eq!(
            f.host
                .browser_router()
                .oneshot(create_book_request(body, Some("unselected")))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.host
            .browser_router()
            .oneshot(create_book_request("{}", None))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.host
            .browser_router()
            .oneshot(create_book_request(
                "{}",
                Some("00000000000000000000000000000000")
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert!(!f.root.join("started").exists());
    let response = f
        .host
        .browser_router()
        .oneshot(create_book_request("{}", Some("unselected")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
        serde_json::json!({"book_id":null,"selection_required":true})
    );
    assert_eq!(f.keys.0.lock().unwrap().len(), before);
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_create_refuses_existing_and_outside_collection_destinations_before_keys() {
    for destination in [
        "os.path.join(os.getcwd(),'book')",
        "os.path.join(os.getcwd(),'assets','nested')",
        "os.path.join(os.getcwd(),'.hidden-book')",
    ] {
        let f = Fixture::new(&format!(
            "print(json.dumps(dict(r,result={{'outcome':'selected','path':{destination}}})))"
        ))
        .await;
        let before = f.keys.0.lock().unwrap().len();
        let binding = fs::read(f.root.join("book/device-key.json")).unwrap();
        let response = f
            .host
            .browser_router()
            .oneshot(create_book_request("{}", Some("unselected")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(f.keys.0.lock().unwrap().len(), before);
        assert_eq!(
            fs::read(f.root.join("book/device-key.json")).unwrap(),
            binding
        );
        assert!(!f.root.join("assets/nested").exists());
        assert!(!f.root.join(".hidden-book").exists());
        f.host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn browser_create_preserves_selected_book_and_private_router_omits_endpoint() {
    let f = Fixture::new("print(json.dumps(dict(r,result={'outcome':'selected','path':os.path.join(os.getcwd(),'second-book')})))").await;
    f.select_book().await;
    let generation = f.selection_id().await;
    let response = f
        .host
        .router()
        .oneshot(create_book_request("{}", Some(&generation)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    drop(response);
    let response = f
        .host
        .browser_router()
        .oneshot(create_book_request("{}", Some(&generation)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    drop(response);
    assert_eq!(f.selection_id().await, generation);
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_create_refuses_collection_replaced_while_native_panel_is_open() {
    let f = Fixture::new("old=os.getcwd()\nopen('started','w').close()\ntime.sleep(0.4)\nprint(json.dumps(dict(r,result={'outcome':'selected','path':os.path.join(old,'new-book')})))").await;
    let before = f.keys.0.lock().unwrap().len();
    let router = f.host.browser_router();
    let pending = tokio::spawn(async move {
        router
            .oneshot(create_book_request("{}", Some("unselected")))
            .await
            .unwrap()
    });
    f.wait_marker().await;
    let moved = f.root.with_extension("moved-private-fixture");
    fs::rename(&f.root, &moved).unwrap();
    fs::create_dir(&f.root).unwrap();
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o700)).unwrap();
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(f.keys.0.lock().unwrap().len(), before);
    assert!(!f.root.join("new-book").exists());
    f.host.shutdown().await.unwrap();
    fs::remove_dir_all(moved).unwrap();
}

#[tokio::test]
async fn browser_panel_route_is_opt_in_and_requires_origin_and_exact_selected_generation() {
    let f = Fixture::new(
        "open('started','w').close()\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
    )
    .await;
    let origin = Some("http://127.0.0.1:3000");
    let response = f
        .host
        .browser_router()
        .oneshot(browser_request(
            r#"{"kind":"recovery-parent"}"#,
            Some("unselected"),
            origin,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    f.select_book().await;
    let generation = f.selection_id().await;
    for (guard, origin, expected) in [
        (None, origin, StatusCode::BAD_REQUEST),
        (Some("invalid"), origin, StatusCode::BAD_REQUEST),
        (
            Some("00000000000000000000000000000000"),
            origin,
            StatusCode::CONFLICT,
        ),
        (Some(generation.as_str()), None, StatusCode::FORBIDDEN),
        (
            Some(generation.as_str()),
            Some("https://foreign.example"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = f
            .host
            .browser_router()
            .oneshot(browser_request(
                r#"{"kind":"recovery-parent"}"#,
                guard,
                origin,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let request = browser_request(r#"{"kind":"recovery-parent"}"#, Some(&generation), origin);
    let response = f.host.router().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "router used by private Unix ingress must omit panel route"
    );
    drop(response);
    assert!(!f.root.join("started").exists());
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_panel_accepts_only_eight_kinds_and_never_web_paths_or_capabilities() {
    let f = Fixture::new(
        "open('started','w').close()\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
    )
    .await;
    f.select_book().await;
    let generation = f.selection_id().await;
    for body in [
        r#"{"kind":"unknown"}"#,
        r#"{"kind":"book","path":"/private/tmp"}"#,
        r#"{"kind":"book","capability_id":"fake"}"#,
        r#"{"kind":"book","helper":"fake"}"#,
        r#"{"kind":"book","kind":"request"}"#,
    ] {
        let response = f
            .host
            .browser_router()
            .oneshot(browser_request(
                body,
                Some(&generation),
                Some("http://127.0.0.1:3000"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = f
        .host
        .browser_router()
        .oneshot(browser_request(
            format!("{{\"kind\":\"{}\"}}", "x".repeat(5000)),
            Some(&generation),
            Some("http://127.0.0.1:3000"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!f.root.join("started").exists());
    for kind in [
        "book",
        "request",
        "package",
        "backup",
        "request-destination",
        "export-destination",
        "book-destination",
        "recovery-parent",
    ] {
        let response = f
            .host
            .browser_router()
            .oneshot(browser_request(
                format!("{{\"kind\":\"{kind}\"}}"),
                Some(&generation),
                Some("http://127.0.0.1:3000"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let raw = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
            serde_json::json!({"capability_id":null})
        );
    }
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_panel_pins_selection_and_refuses_book_switch_until_completion() {
    let f = Fixture::new("open('started','w').close()\ntime.sleep(0.4)\nprint(json.dumps(dict(r,result={'outcome':'selected','path':os.getcwd()})))").await;
    f.select_book().await;
    let generation = f.selection_id().await;
    let router = f.host.browser_router();
    let request = browser_request(
        r#"{"kind":"recovery-parent"}"#,
        Some(&generation),
        Some("http://127.0.0.1:3000"),
    );
    let pending = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    f.wait_marker().await;
    let router = f.host.router();
    let request = Request::builder()
        .method("DELETE")
        .uri("/api/session/selection")
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("x-kaccounting-selection", &generation)
        .body(Body::empty())
        .unwrap();
    let clearing = router.oneshot(request).await.unwrap();
    assert_eq!(clearing.status(), StatusCode::CONFLICT);
    let raw = axum::body::to_bytes(clearing.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["error"]["code"],
        "book_busy"
    );
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert!(
        result["capability_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && !id.contains('/'))
    );
    assert!(!String::from_utf8_lossy(&raw).contains(f.root.to_str().unwrap()));
    let request = Request::builder()
        .method("DELETE")
        .uri("/api/session/selection")
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("x-kaccounting-selection", &generation)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        f.host.router().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_panel_closed_host_and_explicit_cancel_are_bounded() {
    let f = Fixture::new("open('started','w').close()\ntime.sleep(30)").await;
    f.select_book().await;
    let generation = f.selection_id().await;
    let router = f.host.browser_router();
    let request = browser_request(
        r#"{"kind":"recovery-parent"}"#,
        Some(&generation),
        Some("http://127.0.0.1:3000"),
    );
    let pending = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    f.wait_marker().await;
    f.host.cancel_selection().await.unwrap();
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
        serde_json::json!({"capability_id":null})
    );
    f.host.shutdown().await.unwrap();
    let response = f
        .host
        .browser_router()
        .oneshot(browser_request(
            r#"{"kind":"book"}"#,
            Some(&generation),
            Some("http://127.0.0.1:3000"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn construction_and_unselected_action_never_run_helper() {
    let f = Fixture::new(
        "open('started','w').close()\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
    )
    .await;
    assert!(!f.root.join("started").exists());
    assert!(f.host.select(SelectionKind::RecoveryParent).await.is_err());
    assert!(!f.root.join("started").exists());
    f.host.shutdown().await.unwrap();
}
#[tokio::test]
async fn explicit_native_action_returns_only_typed_capability_or_cancel() {
    let f =
        Fixture::new("print(json.dumps(dict(r,result={'outcome':'selected','path':os.getcwd()})))")
            .await;
    f.select_book().await;
    let event = f.host.select(SelectionKind::RecoveryParent).await.unwrap();
    match event {
        NativeHostEvent::Selected { capability_id } => {
            assert!(!capability_id.is_empty());
            assert!(!capability_id.contains('/'));
        }
        _ => panic!("expected selection"),
    };
    f.host.shutdown().await.unwrap();
    let f = Fixture::new("print(json.dumps(dict(r,result={'outcome':'cancelled'})))").await;
    f.select_book().await;
    assert_eq!(
        f.host.select(SelectionKind::RecoveryParent).await.unwrap(),
        NativeHostEvent::Cancelled
    );
    f.host.shutdown().await.unwrap();
}
#[tokio::test]
async fn closing_cancels_and_reaps_owned_helper_then_prevents_reopening() {
    let f = Fixture::new("open('started','w').close()\ntime.sleep(30)").await;
    f.select_book().await;
    let host = f.host.clone();
    let pending = tokio::spawn(async move { host.select(SelectionKind::RecoveryParent).await });
    f.wait_marker().await;
    f.host.shutdown().await.unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(NativeHostError::Closed)
    ));
    assert!(matches!(
        f.host.select(SelectionKind::RecoveryParent).await,
        Err(NativeHostError::Closed)
    ));
    let response = f
        .host
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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = f
        .host
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "foreign.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    f.host.shutdown().await.unwrap();
}
#[tokio::test]
async fn explicit_cancel_reaps_only_active_panel_and_keeps_host_usable() {
    let f = Fixture::new("open('started','w').close()\ntime.sleep(30)").await;
    f.select_book().await;
    let host = f.host.clone();
    let pending = tokio::spawn(async move { host.select(SelectionKind::RecoveryParent).await });
    f.wait_marker().await;
    f.host.cancel_selection().await.unwrap();
    assert_eq!(pending.await.unwrap().unwrap(), NativeHostEvent::Cancelled);
    let response = f
        .host
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
    assert_eq!(response.status(), StatusCode::OK);
    f.host.shutdown().await.unwrap();
}

#[tokio::test]
async fn private_native_startup_failure_still_closes_host_without_running_helper() {
    let f = Fixture::new(
        "open('started','w').close()\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
    )
    .await;
    f.select_book().await;
    let retained = f.host.router();
    let policy = kaccounting_backend::private_access::PrivateAccessPolicy::new(
        "book.example.ts.net",
        "owner@example.com",
    )
    .unwrap();
    // Missing parent fails during private path validation, before listener bind.
    let result = kaccounting_backend::runtime::serve_private_native_host(
        f.host.clone(),
        policy,
        f.root.join("does-not-exist/ingress.sock"),
    )
    .await;
    assert!(result.is_err());
    assert!(!f.root.join("started").exists());
    let response = retained
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(matches!(
        f.host.select(SelectionKind::RecoveryParent).await,
        Err(NativeHostError::Closed)
    ));
}

#[tokio::test]
async fn delayed_selection_body_cannot_reopen_book_after_host_shutdown() {
    use axum::body::Bytes;
    use std::{
        pin::Pin,
        sync::atomic::Ordering,
        task::{Context, Poll},
    };
    struct Delayed {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        receiver: tokio::sync::mpsc::Receiver<Bytes>,
    }
    impl futures_core::Stream for Delayed {
        type Item = Result<Bytes, std::io::Error>;
        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
            }
            self.receiver.poll_recv(cx).map(|item| item.map(Ok))
        }
    }
    let f = Fixture::new("open('started','w').close()").await;
    f.select_book().await;
    let before = f.keys.1.load(Ordering::SeqCst);
    let (started, polled) = tokio::sync::oneshot::channel();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let router = f.host.router();
    let request = Request::builder()
        .method("POST")
        .uri("/api/session/selection")
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "application/json")
        .body(Body::from_stream(Delayed {
            started: Some(started),
            receiver,
        }))
        .unwrap();
    let pending = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    // Body polling proves host middleware admitted the request while it was open.
    polled.await.unwrap();
    f.host.shutdown().await.unwrap();
    sender
        .send(Bytes::from_static(br#"{"book_id":"book"}"#))
        .await
        .unwrap();
    drop(sender);
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        f.keys.1.load(Ordering::SeqCst),
        before,
        "late request accessed a key after shutdown"
    );
    assert!(!f.root.join("started").exists());
}

#[tokio::test]
async fn cancelled_host_shutdown_still_reaps_panel_and_releases_book_lock() {
    use std::future::Future;
    let f = Fixture::new("open('started','w').close()\ntime.sleep(30)").await;
    f.select_book().await;
    let host = f.host.clone();
    let pending = tokio::spawn(async move { host.select(SelectionKind::RecoveryParent).await });
    f.wait_marker().await;
    let mut closing = Box::pin(f.host.shutdown());
    std::future::poll_fn(|cx| {
        assert!(closing.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(closing);
    assert!(matches!(
        pending.await.unwrap(),
        Err(NativeHostError::Closed)
    ));
    // A retained closed host must not keep its old book's process lock forever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match RuntimeBook::open_existing(f.root.join("book"), f.keys.as_ref()).await {
            Ok(book) => {
                book.close().await;
                break;
            }
            Err(kaccounting_backend::runtime::RuntimeError::BookBusy) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "cancelled host shutdown stranded the book lock after panel reaping"
                );
                tokio::task::yield_now().await;
            }
            Err(error) => panic!("unexpected disposable book reopen failure: {error}"),
        }
    }
}
