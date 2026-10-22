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
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys {
    device: Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
    recipient: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    reads: AtomicUsize,
    recipient_insert_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    recipient_insert_started: std::sync::atomic::AtomicBool,
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
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
        if let Some(gate) = self.recipient_insert_gate.lock().unwrap().take() {
            self.recipient_insert_started.store(true, Ordering::SeqCst);
            gate.recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|_| KeyStoreError::Missing)?;
        }
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
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, PostCorrection,
            PostEntry,
        },
        money::MinorUnits,
    },
    attachments::AttachmentRepository,
    container_import::import_recipient_container,
    recipient_transfer::create_recipient_request,
    storage_operations::SelectionKind,
    transfer_container::extract_container,
};
use serde_json::{Value, json as value};

async fn completed(f: &Fixture, id: &str) -> Value {
    let start = std::time::Instant::now();
    loop {
        let response = f
            .request("GET", &format!("/api/storage/operations/{id}"), "")
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let status = json(response).await;
        if status["state"] != "running" {
            return status;
        }
        assert!(start.elapsed().as_secs() < 30, "job did not terminate");
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn session_http_download_recipient_import_preserves_exact_history_and_never_auto_selects() {
    let f = Fixture::new().await;
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["selection_required"],
        true
    );
    assert_eq!(
        f.request("GET", "/api/accounts", "").await.status(),
        StatusCode::CONFLICT
    );
    // Build a source using exact integers outside JavaScript's safe integer range.
    let source = RuntimeBook::open_existing(f.root.join("first"), f.keys.as_ref())
        .await
        .unwrap();
    let accounts = AccountRepository::new(source.pool().clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "Audit cash",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let bank = accounts
        .create(NewAccount::new(
            None,
            "Audit bank",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let exact = 9_007_199_254_740_993;
    let entry = |amount| {
        PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(amount)),
                JournalLineInput::new(bank.id, JournalSide::Credit, MinorUnits::new(amount)),
            ],
        )
    };
    let journal = JournalRepository::new(source.pool().clone());
    let original = journal.post(entry(exact)).await.unwrap();
    let receipt = f.root.join("receipt.txt");
    let plaintext = b"independent release acceptance receipt";
    fs::write(&receipt, plaintext).unwrap();
    let attachments =
        AttachmentRepository::open(source.pool().clone(), source.attachments(), source.key())
            .unwrap();
    let attachment = attachments
        .import_file(original.id, &receipt, "receipt.txt")
        .await
        .unwrap();
    let correction = journal
        .post_correction(PostCorrection::new(
            original.id,
            entry(17),
            "Explicit acceptance correction",
        ))
        .await
        .unwrap();
    let reversal = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-22").unwrap())
        .await
        .unwrap();
    let original = journal.get_posted(original.id).await.unwrap();
    let old_key_id = source.key_id().clone();
    source.close().await;
    drop(attachments);
    drop(journal);
    drop(accounts);
    drop(source);

    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    let recipient_keys = Arc::new(Keys::default());
    let request = create_recipient_request(recipient_keys.as_ref()).unwrap();
    fs::write(f.root.join("request.json"), request.to_json().unwrap()).unwrap();
    fs::set_permissions(
        f.root.join("request.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let book = f
        .session
        .register_storage_selection(SelectionKind::Book, f.root.join("first"))
        .await
        .unwrap();
    let request_cap = f
        .session
        .register_storage_selection(SelectionKind::Request, f.root.join("request.json"))
        .await
        .unwrap();
    let destination = f
        .session
        .register_storage_selection(SelectionKind::ExportDestination, f.root.join("export"))
        .await
        .unwrap();
    let response = f.request("POST", "/api/storage/exports", &value!({"book":book,"request":request_cap,"destination":destination,"confirmed_request_id":request.request_id(),"confirmed_fingerprint":request.fingerprint()}).to_string()).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let id = json(response).await["operation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let job = completed(&f, &id).await;
    assert_eq!(job["state"], "succeeded", "{job}");
    let download = f
        .request("GET", &format!("/api/storage/operations/{id}/download"), "")
        .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["cache-control"], "no-store");
    assert_eq!(
        download.headers()["content-disposition"],
        "attachment; filename=\"kAccounting-Extra-01.kagextra\""
    );
    // The live download holds the book lease until its body is drained/dropped.
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.request("DELETE", "/api/session/selection", "")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let bytes = to_bytes(download.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let extracted = f.root.join("download-extracted");
    extract_container(
        std::io::Cursor::new(bytes.clone()),
        &extracted,
        Default::default(),
    )
    .unwrap();
    let imported_path = f.root.join("imported");
    import_recipient_container(
        std::io::Cursor::new(bytes),
        f.root.clone(),
        imported_path.clone(),
        request.clone(),
        recipient_keys.clone(),
        recipient_keys.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    let imported = RuntimeBook::open_existing(&imported_path, recipient_keys.as_ref())
        .await
        .unwrap();
    assert_ne!(imported.key_id(), &old_key_id);
    assert_ne!(
        DeviceKeyStore::read(recipient_keys.as_ref(), imported.key_id()).unwrap(),
        DeviceKeyStore::read(f.keys.as_ref(), &old_key_id).unwrap()
    );
    let imported_journal = JournalRepository::new(imported.pool().clone());
    for expected in [original, correction, reversal] {
        assert_eq!(
            imported_journal.get_posted(expected.id).await.unwrap(),
            expected
        );
    }
    for sql in [
        "UPDATE journal_lines SET amount_minor=1",
        "DELETE FROM journal_entries WHERE status='posted'",
        "DELETE FROM attachments",
    ] {
        assert!(
            sqlx::query(sql).execute(imported.pool()).await.is_err(),
            "immutable guard missing: {sql}"
        );
    }
    let imported_attachments = AttachmentRepository::open(
        imported.pool().clone(),
        imported.attachments(),
        imported.key(),
    )
    .unwrap();
    let (metadata, mut chunks) = imported_attachments
        .download_stream(&attachment.id)
        .await
        .unwrap();
    assert_eq!(metadata.plaintext, attachment.plaintext);
    assert_ne!(metadata.ciphertext, attachment.ciphertext);
    let mut actual = Vec::new();
    while let Some(chunk) = chunks.recv().await {
        actual.extend(chunk.unwrap());
    }
    assert_eq!(actual, plaintext);
    imported.close().await;
    // Producer provenance includes the captured descriptors. A valid export job
    // does not grant future downloads of files replaced at that same pathname.
    fs::write(f.root.join("export/manifest.kae"), b"replaced ciphertext").unwrap();
    assert_eq!(
        f.request("GET", &format!("/api/storage/operations/{id}/download"), "")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.request("DELETE", "/api/session/selection", "")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["selection_required"],
        true
    );
    f.session.shutdown().await;
}

#[tokio::test]
async fn session_export_rejects_replaced_destination_ancestor_before_any_publication() {
    let f = Fixture::new().await;
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    let request = create_recipient_request(f.keys.as_ref()).unwrap();
    fs::write(f.root.join("request.json"), request.to_json().unwrap()).unwrap();
    fs::set_permissions(
        f.root.join("request.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let parent = f.root.join("selected-parent");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let book = f
        .session
        .register_storage_selection(SelectionKind::Book, f.root.join("first"))
        .await
        .unwrap();
    let request_cap = f
        .session
        .register_storage_selection(SelectionKind::Request, f.root.join("request.json"))
        .await
        .unwrap();
    let destination = f
        .session
        .register_storage_selection(SelectionKind::ExportDestination, parent.join("export"))
        .await
        .unwrap();
    // A replacement directory at the same textual pathname cannot inherit a capability.
    let original_parent = f.root.join("original-parent");
    fs::rename(&parent, &original_parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let response = f.request("POST", "/api/storage/exports", &value!({"book":book,"request":request_cap,"destination":destination,"confirmed_request_id":request.request_id(),"confirmed_fingerprint":request.fingerprint()}).to_string()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    drop(response);
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    assert_eq!(fs::read_dir(original_parent).unwrap().count(), 0);
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["book_id"],
        "first"
    );
    f.session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_session_job_blocks_switch_and_shutdown_drains_before_releasing_book() {
    let f = Fixture::new().await;
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"first"}"#)
            .await
            .status(),
        StatusCode::OK
    );
    let (release, blocked) = std::sync::mpsc::channel();
    *f.keys.recipient_insert_gate.lock().unwrap() = Some(blocked);
    let destination = f
        .session
        .register_storage_selection(
            SelectionKind::RequestDestination,
            f.root.join("owned-request.json"),
        )
        .await
        .unwrap();
    let response = f
        .request(
            "POST",
            "/api/storage/recipient-requests",
            &value!({"destination":destination}).to_string(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    // Drain the HTTP response: only the owned background job keeps the session busy.
    let job = json(response).await;
    let started = std::time::Instant::now();
    while !f.keys.recipient_insert_started.load(Ordering::SeqCst) {
        assert!(started.elapsed().as_secs() < 5);
        tokio::task::yield_now().await;
    }
    assert_eq!(
        f.request("POST", "/api/session/selection", r#"{"book_id":"second"}"#)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.request("DELETE", "/api/session/selection", "")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        completed_state(&f, job["operation_id"].as_str().unwrap()).await,
        "running"
    );
    let session = f.session.clone();
    let shutdown = tokio::spawn(async move {
        session.shutdown().await;
    });
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        !shutdown.is_finished(),
        "shutdown released a live owned job"
    );
    // Book lock remains exclusive while the operation is blocked.
    assert!(
        RuntimeBook::open_existing(f.root.join("first"), f.keys.as_ref())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    shutdown.await.unwrap();
    assert!(f.root.join("owned-request.json").exists());
    assert_eq!(
        json(f.request("GET", "/api/session", "").await).await["selection_required"],
        true
    );
    let reopened = RuntimeBook::open_existing(f.root.join("first"), f.keys.as_ref())
        .await
        .unwrap();
    reopened.close().await;
}
async fn completed_state(f: &Fixture, id: &str) -> String {
    json(
        f.request("GET", &format!("/api/storage/operations/{id}"), "")
            .await,
    )
    .await["state"]
        .as_str()
        .unwrap()
        .to_owned()
}
