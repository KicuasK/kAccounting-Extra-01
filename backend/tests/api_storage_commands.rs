use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    api,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    runtime::RuntimeBook,
    storage_operations::{OperationError, RequestInput, SelectionKind, StorageOperations},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl Keys {
    fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn put(&self, id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        let mut map = self.0.lock().unwrap();
        if map.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        map.insert(id.into(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.get(id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, secret: &[u8]) -> Result<(), KeyStoreError> {
        self.put(id.as_str(), secret)
    }
}
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.get(id)
    }
    fn insert_new(&self, id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        self.put(id, secret)
    }
}
struct Fixture {
    root: PathBuf,
    device: Arc<Keys>,
    operations: StorageOperations,
    router: Router,
    book: Arc<RuntimeBook>,
}
impl Fixture {
    async fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-http-storage-{id}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let device = Arc::new(Keys::default());
        let recipient = Arc::new(Keys::default());
        let path = root.join("book");
        RuntimeBook::init_new(&path, device.as_ref()).await.unwrap();
        let book = Arc::new(
            RuntimeBook::open_existing(&path, device.as_ref())
                .await
                .unwrap(),
        );
        let operations = StorageOperations::new(device.clone(), recipient)
            .with_active_book(book.clone())
            .unwrap();
        let router = api::localhost_storage_router(book.pool().clone(), operations.clone());
        Self {
            root,
            device,
            operations,
            router,
            book,
        }
    }
    fn select(&self, kind: SelectionKind, name: &str) -> String {
        self.operations
            .register_selection(kind, self.root.join(name))
            .unwrap()
    }
    async fn job(&self, route: &str, value: Value) -> Value {
        let (status, value) = send(
            self.router.clone(),
            "POST",
            route,
            Some(value),
            true,
            "127.0.0.1:3000",
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{value}");
        self.finished(value["operation_id"].as_str().unwrap()).await
    }
    async fn finished(&self, id: &str) -> Value {
        let start = std::time::Instant::now();
        loop {
            let (status, value) = send(
                self.router.clone(),
                "GET",
                &format!("/api/storage/operations/{id}"),
                None,
                false,
                "127.0.0.1:3000",
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            if value["state"] != "running" {
                return value;
            }
            assert!(start.elapsed().as_secs() < 30, "storage job timed out");
            tokio::task::yield_now().await;
        }
    }
    async fn recipient(&self) -> (String, Value) {
        let destination = self.select(SelectionKind::RequestDestination, "request.json");
        let value = self
            .job(
                "/api/storage/recipient-requests",
                json!({"destination":destination}),
            )
            .await;
        assert_eq!(value["state"], "succeeded", "{value}");
        let request = self.select(SelectionKind::Request, "request.json");
        (request, value["result"].clone())
    }
    async fn close(self) {
        self.book.close().await;
        drop(self.router);
        drop(self.operations);
        drop(self.book);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
async fn send(
    router: Router,
    method: &str,
    path: &str,
    value: Option<Value>,
    origin: bool,
    host: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host);
    if origin {
        request = request.header("origin", "http://127.0.0.1:3000");
    }
    let body = if let Some(value) = value {
        request = request.header("content-type", "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    let response = router.oneshot(request.body(body).unwrap()).await.unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn storage_routes_reject_cross_origin_paths_unknown_fields_and_unknown_capabilities() {
    let f = Fixture::new().await;
    for (value, origin, host, status) in [
        (
            json!({"destination":"/private/tmp/arbitrary"}),
            true,
            "127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"destination":"a".repeat(32),"path":"/private/tmp/arbitrary"}),
            true,
            "127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"destination":"a".repeat(32)}),
            false,
            "127.0.0.1:3000",
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"destination":"a".repeat(32)}),
            true,
            "evil.example",
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"destination":42}),
            true,
            "127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            send(
                f.router.clone(),
                "POST",
                "/api/storage/recipient-requests",
                Some(value),
                origin,
                host
            )
            .await
            .0,
            status
        );
    }
    assert_eq!(
        send(
            f.router.clone(),
            "GET",
            "/api/storage/operations/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            None,
            false,
            "evil.example"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            f.router.clone(),
            "GET",
            "/api/storage/operations/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            None,
            false,
            "127.0.0.1:3000"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(f.device.0.lock().unwrap().len() == 1);
    f.close().await;
}
#[tokio::test]
async fn selected_transfer_exports_active_book_imports_new_book_and_keeps_current_book() {
    let f = Fixture::new().await;
    let (request, preview) = f.recipient().await;
    let book = f.select(SelectionKind::Book, "book");
    let destination = f.select(SelectionKind::ExportDestination, "transfer.kagextra");
    let input = json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":preview["request_id"],"confirmed_fingerprint":"wrong"});
    assert_eq!(
        send(
            f.router.clone(),
            "POST",
            "/api/storage/exports",
            Some(input),
            true,
            "127.0.0.1:3000"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let result = f.job("/api/storage/exports", json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":preview["request_id"],"confirmed_fingerprint":preview["fingerprint"]})).await;
    assert_eq!(result["state"], "succeeded", "{result}");
    assert_eq!(result["result"]["attachment_count"], "0");
    assert_eq!(result["result"]["attachment_bytes"], "0");
    assert!(!result.to_string().contains(f.root.to_str().unwrap()));
    let package = f.select(SelectionKind::Package, "transfer.kagextra");
    let destination = f.select(SelectionKind::BookDestination, "imported");
    let binding = std::fs::read(f.book.root().join("device-key.json")).unwrap();
    let result = f
        .job(
            "/api/storage/imports",
            json!({"package":package,"request":request,"destination":destination}),
        )
        .await;
    assert_eq!(result["state"], "succeeded", "{result}");
    assert_eq!(result["result"]["selection_required"], true);
    assert_eq!(
        std::fs::read(f.book.root().join("device-key.json")).unwrap(),
        binding
    );
    assert!(
        !f.book.root().join("backups").exists(),
        "new-destination import must not write a live-book backup"
    );
    let imported = RuntimeBook::open_existing(f.root.join("imported"), f.device.as_ref())
        .await
        .unwrap();
    imported.close().await;
    drop(imported);
    f.close().await;
}
#[tokio::test]
async fn jobs_are_owned_after_http_response_and_destination_capabilities_are_single_use() {
    let f = Fixture::new().await;
    let destination = f.select(SelectionKind::RequestDestination, "request.json");
    let started = f
        .operations
        .request(RequestInput {
            destination: destination.clone(),
        })
        .unwrap();
    // No await between admissions: the first owned task still holds its permit.
    let other = f.select(SelectionKind::RequestDestination, "other.json");
    assert_eq!(
        f.operations
            .request(RequestInput { destination: other })
            .unwrap_err(),
        OperationError::Busy
    );
    assert_eq!(
        f.operations.forget_completed(&started.operation_id),
        Err(OperationError::Busy)
    );
    let result = f.finished(&started.operation_id).await;
    assert_eq!(result["state"], "succeeded");
    assert_eq!(
        f.operations
            .request(RequestInput { destination })
            .unwrap_err(),
        OperationError::InvalidSelection
    );
    f.operations
        .forget_completed(&started.operation_id)
        .unwrap();
    assert_eq!(
        f.operations.status(&started.operation_id).unwrap_err(),
        OperationError::NotFound
    );
    f.close().await;
}
#[tokio::test]
async fn wrong_capability_type_existing_destination_and_symlink_selections_fail_closed() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new().await;
    assert_eq!(
        f.operations
            .register_selection(SelectionKind::BookDestination, f.root.join("book"))
            .unwrap_err(),
        OperationError::InvalidSelection
    );
    symlink(f.root.join("book"), f.root.join("alias")).unwrap();
    assert_eq!(
        f.operations
            .register_selection(SelectionKind::Book, f.root.join("alias"))
            .unwrap_err(),
        OperationError::InvalidSelection
    );
    let book = f.select(SelectionKind::Book, "book");
    assert_eq!(
        send(
            f.router.clone(),
            "POST",
            "/api/storage/recipient-requests",
            Some(json!({"destination":book})),
            true,
            "127.0.0.1:3000"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    f.close().await;
}
#[tokio::test]
async fn restore_is_new_destination_only_and_does_not_change_active_book() {
    let f = Fixture::new().await;
    let source = f.root.join("backup-source");
    RuntimeBook::init_new(&source, f.device.as_ref())
        .await
        .unwrap();
    let backup = kaccounting_backend::application_commands::backup_book(&source, f.device.as_ref())
        .await
        .unwrap();
    let capability = f
        .operations
        .register_selection(SelectionKind::Backup, &backup.backup.path)
        .unwrap();
    let destination = f.select(SelectionKind::BookDestination, "collision");
    std::fs::create_dir(f.root.join("collision")).unwrap();
    std::fs::write(
        f.root.join("collision/keep"),
        b"preserve existing destination",
    )
    .unwrap();
    let (status, value) = send(
        f.router.clone(),
        "POST",
        "/api/storage/restores",
        Some(json!({"backup":capability,"destination":destination})),
        true,
        "127.0.0.1:3000",
    )
    .await;
    // A destination created after selection is rejected at admission, before
    // scheduling a restore against a now-stale native selection capability.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["code"], "invalid_storage_selection");
    assert!(!value.to_string().contains(f.root.to_str().unwrap()));
    assert_eq!(
        std::fs::read(f.root.join("collision/keep")).unwrap(),
        b"preserve existing destination"
    );
    let binding = std::fs::read(f.book.root().join("device-key.json")).unwrap();
    let destination = f.select(SelectionKind::BookDestination, "restored");
    let value = f
        .job(
            "/api/storage/restores",
            json!({"backup":capability,"destination":destination}),
        )
        .await;
    assert_eq!(value["state"], "succeeded", "{value}");
    assert_eq!(value["result"]["selection_required"], true);
    assert_eq!(value["result"]["attachment_count"], "0");
    assert_eq!(
        std::fs::read(f.book.root().join("device-key.json")).unwrap(),
        binding
    );
    let restored = RuntimeBook::open_existing(f.root.join("restored"), f.device.as_ref())
        .await
        .unwrap();
    restored.close().await;
    drop(restored);
    f.close().await;
}
#[tokio::test]
async fn recovery_uses_selected_parent_and_returns_counts_without_paths() {
    let f = Fixture::new().await;
    let parent = f.select(SelectionKind::RecoveryParent, "");
    let book = f.select(SelectionKind::Book, "book");
    for input in [
        json!({"kind":"imports","parent":parent}),
        json!({"kind":"exports","book":book,"parent":parent}),
    ] {
        let value = f.job("/api/storage/recoveries", input).await;
        assert_eq!(value["state"], "succeeded", "{value}");
        for key in [
            "removed_count",
            "verified_count",
            "active_count",
            "unrecognized_count",
        ] {
            assert_eq!(value["result"][key], "0");
        }
        assert!(!value.to_string().contains(f.root.to_str().unwrap()));
    }
    f.close().await;
}
#[tokio::test]
async fn recipient_confirmation_is_bound_to_immutable_selected_document() {
    let f = Fixture::new().await;
    let (request, original) = f.recipient().await;
    // Another public request replaces the selected file after local registration.
    // The capability must keep the originally previewed recipient document.
    let replacement = f.select(SelectionKind::RequestDestination, "replacement.json");
    let second = f
        .job(
            "/api/storage/recipient-requests",
            json!({"destination":replacement}),
        )
        .await;
    std::fs::rename(f.root.join("replacement.json"), f.root.join("request.json")).unwrap();
    let (status, preview) = send(
        f.router.clone(),
        "GET",
        &format!("/api/storage/selections/{request}/request"),
        None,
        false,
        "127.0.0.1:3000",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["request_id"], original["request_id"]);
    assert_ne!(preview["request_id"], second["result"]["request_id"]);
    let book = f.select(SelectionKind::Book, "book");
    let destination = f.select(SelectionKind::ExportDestination, "bound.kagextra");
    let result = f.job("/api/storage/exports", json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":original["request_id"],"confirmed_fingerprint":original["fingerprint"]})).await;
    assert_eq!(result["state"], "succeeded", "{result}");
    // Import with the same immutable original request proves envelope binding.
    let package = f.select(SelectionKind::Package, "bound.kagextra");
    let destination = f.select(SelectionKind::BookDestination, "bound-import");
    let result = f
        .job(
            "/api/storage/imports",
            json!({"package":package,"request":request,"destination":destination}),
        )
        .await;
    assert_eq!(result["state"], "succeeded", "{result}");
    f.close().await;
}
#[tokio::test]
async fn tampered_package_failure_never_publishes_or_changes_current_book() {
    let f = Fixture::new().await;
    let (request, preview) = f.recipient().await;
    let book = f.select(SelectionKind::Book, "book");
    let destination = f.select(SelectionKind::ExportDestination, "tampered.kagextra");
    let value = f.job("/api/storage/exports", json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":preview["request_id"],"confirmed_fingerprint":preview["fingerprint"]})).await;
    assert_eq!(value["state"], "succeeded");
    let package = f.select(SelectionKind::Package, "tampered.kagextra");
    std::fs::write(f.root.join("tampered.kagextra/unknown.txt"), b"tamper").unwrap();
    let destination = f.select(SelectionKind::BookDestination, "must-not-publish");
    let binding = std::fs::read(f.book.root().join("device-key.json")).unwrap();
    let value = f
        .job(
            "/api/storage/imports",
            json!({"package":package,"request":request,"destination":destination}),
        )
        .await;
    assert_eq!(value["state"], "failed");
    assert!(!f.root.join("must-not-publish").exists());
    assert_eq!(
        std::fs::read(f.book.root().join("device-key.json")).unwrap(),
        binding
    );
    f.close().await;
}
