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
async fn download(f: &Fixture, id: &str) -> axum::response::Response {
    f.router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storage/operations/{id}/download"))
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn successful_verified_export_downloads_single_ciphertext_container_and_tampering_is_refused()
{
    let f = Fixture::new().await;
    use kaccounting_backend::{
        accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
        attachments::AttachmentRepository,
    };
    let draft = JournalRepository::new(f.book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let source = f.root.join("large-test-attachment");
    std::fs::write(&source, vec![73u8; 2 * 1024 * 1024]).unwrap();
    AttachmentRepository::open(f.book.pool().clone(), f.book.attachments(), f.book.key())
        .unwrap()
        .import_file(draft.id, &source, "large-test-attachment")
        .await
        .unwrap();
    let (request, preview) = f.recipient().await;
    let book = f.select(SelectionKind::Book, "book");
    let destination = f.select(SelectionKind::ExportDestination, "export");
    let job = f.job("/api/storage/exports", json!({"book":book,"request":request,"destination":destination,"confirmed_request_id":preview["request_id"],"confirmed_fingerprint":preview["fingerprint"]})).await;
    assert_eq!(job["state"], "succeeded");
    let id = job["operation_id"].as_str().unwrap();
    let first = download(&f, id).await;
    let second = download(&f, id).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let busy = download(&f, id).await;
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    let busy: Value =
        serde_json::from_slice(&to_bytes(busy.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(busy["code"], "storage_download_busy");
    drop(first);
    let start = std::time::Instant::now();
    let response = loop {
        let response = download(&f, id).await;
        if response.status() == StatusCode::OK {
            break response;
        }
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(start.elapsed().as_secs() < 10);
        tokio::task::yield_now().await;
    };
    drop(second);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"kAccounting-Extra-01.kagextra\""
    );
    let mut stream = Box::pin(response.into_body().into_data_stream());
    let mut bytes = Vec::new();
    while let Some(chunk) =
        std::future::poll_fn(|cx| futures_core::Stream::poll_next(stream.as_mut(), cx)).await
    {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 64 * 1024);
        bytes.extend_from_slice(&chunk);
        assert!(bytes.len() <= 16 * 1024 * 1024);
    }
    assert!(bytes.len() > 2 * 1024 * 1024);
    assert_eq!(std::fs::metadata(&source).unwrap().len(), 2 * 1024 * 1024);
    assert!(bytes.starts_with(kaccounting_backend::transfer_container::CONTAINER_MAGIC));
    let extracted = f.root.join("extracted");
    kaccounting_backend::transfer_container::extract_container(
        std::io::Cursor::new(bytes),
        &extracted,
        Default::default(),
    )
    .unwrap();
    for name in ["envelope.json", "book.kae", "manifest.kae"] {
        assert_eq!(
            std::fs::read(extracted.join(name)).unwrap(),
            std::fs::read(f.root.join("export").join(name)).unwrap()
        );
    }
    std::fs::write(f.root.join("export/manifest.kae"), b"tampered").unwrap();
    assert_eq!(download(&f, id).await.status(), StatusCode::NOT_FOUND);
    f.close().await;
}
#[tokio::test]
async fn download_refuses_unknown_malformed_and_non_export_jobs_and_cross_origin() {
    let f = Fixture::new().await;
    for id in ["bad", "00000000000000000000000000000000"] {
        assert_eq!(download(&f, id).await.status(), StatusCode::NOT_FOUND);
    }
    let destination = f.select(SelectionKind::RequestDestination, "request.json");
    let job = f
        .job(
            "/api/storage/recipient-requests",
            json!({"destination":destination}),
        )
        .await;
    assert_eq!(
        download(&f, job["operation_id"].as_str().unwrap())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let response = f
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storage/operations/00000000000000000000000000000000/download")
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    f.close().await;
}
