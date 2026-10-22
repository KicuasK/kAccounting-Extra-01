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
    storage_operations::{SelectionKind, StorageOperations},
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
    operations: StorageOperations,
    router: Router,
    book: Arc<RuntimeBook>,
}
impl Fixture {
    async fn new() -> Self {
        Self::new_with_limits(None).await
    }
    async fn new_with_limits(
        limits: Option<kaccounting_backend::transfer_container::ContainerLimits>,
    ) -> Self {
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
        let operations = StorageOperations::new(device.clone(), recipient);
        let operations = if let Some(limits) = limits {
            operations.with_container_limits(limits).unwrap()
        } else {
            operations
        };
        let operations = operations.with_active_book(book.clone()).unwrap();
        let router = api::localhost_storage_router(book.pool().clone(), operations.clone());
        Self {
            root,
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
async fn browser_download_upload_import_is_authenticated_new_destination_only() {
    use kaccounting_backend::{
        accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
        attachments::AttachmentRepository,
    };
    let f = Fixture::new().await;
    let draft = JournalRepository::new(f.book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let source = f.root.join("binary-source");
    std::fs::write(&source, vec![51; 2 * 1024 * 1024]).unwrap();
    AttachmentRepository::open(f.book.pool().clone(), f.book.attachments(), f.book.key())
        .unwrap()
        .import_file(draft.id, &source, "binary-source")
        .await
        .unwrap();
    let (request, preview) = f.recipient().await;
    let book = f.select(SelectionKind::Book, "book");
    let destination = f.select(SelectionKind::ExportDestination, "export");
    let job=f.job("/api/storage/exports",json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":preview["request_id"],"confirmed_fingerprint":preview["fingerprint"]})).await;
    let response = f
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/storage/operations/{}/download",
                    job["operation_id"].as_str().unwrap()
                ))
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    assert!(bytes.len() > 1024 * 1024);
    let mut corrupted = bytes.to_vec();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    let bad_destination = f.select(SelectionKind::BookDestination, "tampered-import");
    assert_eq!(
        upload(
            &f,
            &request,
            &bad_destination,
            Body::from(corrupted),
            "application/octet-stream",
            "http://127.0.0.1:3000"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!f.root.join("tampered-import").exists());
    let destination = f.select(SelectionKind::BookDestination, "imported");
    let response = upload(
        &f,
        &request,
        &destination,
        Body::from(bytes),
        "application/octet-stream",
        "http://127.0.0.1:3000",
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(value["selection_required"], true);
    assert_eq!(value["attachment_count"], "1");
    assert!(f.root.join("imported").is_dir());
    assert_eq!(f.book.root(), f.root.join("book"));
    assert!(f.operations.is_idle());
    f.close().await;
}
async fn upload(
    f: &Fixture,
    request: &str,
    destination: &str,
    body: Body,
    kind: &str,
    origin: &str,
) -> axum::response::Response {
    f.router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/storage/container-imports?request={request}&destination={destination}"
                ))
                .header("host", "127.0.0.1:3000")
                .header("origin", origin)
                .header("content-type", kind)
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn browser_upload_refuses_bad_type_origin_capabilities_and_truncated_container() {
    let f = Fixture::new().await;
    let (request, _) = f.recipient().await;
    let destination = f.select(SelectionKind::BookDestination, "imported");
    assert_eq!(
        upload(
            &f,
            &request,
            &destination,
            Body::empty(),
            "application/json",
            "http://127.0.0.1:3000"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        upload(
            &f,
            &request,
            &destination,
            Body::empty(),
            "application/octet-stream",
            "http://evil.example"
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        upload(
            &f,
            "invalid",
            &destination,
            Body::empty(),
            "application/octet-stream",
            "http://127.0.0.1:3000"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        upload(
            &f,
            &request,
            &destination,
            Body::from("KAXFER01"),
            "application/octet-stream",
            "http://127.0.0.1:3000"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!f.root.join("imported").exists());
    assert!(f.operations.is_idle());
    f.close().await;
}
struct Interrupted(bool);
impl futures_core::Stream for Interrupted {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.0 {
            std::task::Poll::Ready(None)
        } else {
            self.0 = true;
            std::task::Poll::Ready(Some(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "disconnected test body",
            ))))
        }
    }
}
#[tokio::test]
async fn interrupted_http_body_releases_worker_without_publishing_destination() {
    let f = Fixture::new().await;
    let (request, _) = f.recipient().await;
    let destination = f.select(SelectionKind::BookDestination, "interrupted");
    assert_eq!(
        upload(
            &f,
            &request,
            &destination,
            Body::from_stream(Interrupted(false)),
            "application/octet-stream",
            "http://127.0.0.1:3000"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(f.operations.is_idle());
    assert!(!f.root.join("interrupted").exists());
    f.close().await;
}
#[tokio::test]
async fn configured_transport_cap_rejects_declared_and_streamed_oversize_consistently() {
    let f = Fixture::new_with_limits(Some(
        kaccounting_backend::transfer_container::ContainerLimits {
            max_entries: 3,
            max_object_bytes: 1024,
            max_total_bytes: 1024,
        },
    ))
    .await;
    let (request, _) = f.recipient().await;
    let destination = f.select(SelectionKind::BookDestination, "oversize");
    let uri = format!("/api/storage/container-imports?request={request}&destination={destination}");
    let response = f
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/octet-stream")
                .header("content-length", "2048")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(value["code"], "transfer_payload_too_large");
    assert!(!value["message"].as_str().unwrap().contains("JSON"));
    let response = upload(
        &f,
        &request,
        &destination,
        Body::from(vec![0; 2048]),
        "application/octet-stream",
        "http://127.0.0.1:3000",
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(f.operations.is_idle());
    assert!(!f.root.join("oversize").exists());
    f.close().await;
}
