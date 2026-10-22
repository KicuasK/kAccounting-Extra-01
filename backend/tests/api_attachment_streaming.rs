//! HTTP streaming acceptance uses disposable books and in-memory keys only.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalEntryId, JournalRepository, SaveDraftEntry},
    api::localhost_book_commands_router,
    attachments::{AttachmentRepository, MAX_ATTACHMENT_BYTES},
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::RuntimeBook,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct MemoryKeys {
    values: RefCell<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
    reads: AtomicUsize,
    inserts: AtomicUsize,
}
impl DeviceKeyStore for MemoryKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
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
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-book-http-{id}"));
        std::fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn book() -> (Fixture, RuntimeBook, JournalEntryId) {
    let fixture = Fixture::new();
    let keys = MemoryKeys::default();
    let path = fixture.0.join("book");
    RuntimeBook::init_new(&path, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&path, &keys).await.unwrap();
    let entry = JournalRepository::new(book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    (fixture, book, entry.id)
}
async fn send(
    book: &RuntimeBook,
    method: &str,
    uri: &str,
    body: Body,
    length: Option<u64>,
    origin: Option<&str>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/octet-stream");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(length) = length {
        request = request.header("content-length", length.to_string());
    }
    localhost_book_commands_router(book)
        .unwrap()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}
fn upload_uri(id: JournalEntryId, name: &str, revision: &str) -> String {
    format!(
        "/api/journal/entries/{}/attachments?display_name={name}&source_kind=regular_file{revision}",
        id.get()
    )
}
async fn json(response: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
struct Parts(VecDeque<Result<Vec<u8>, std::io::Error>>);
impl futures_core::Stream for Parts {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.0.pop_front())
    }
}
fn parts(chunks: impl IntoIterator<Item = Result<Vec<u8>, std::io::Error>>) -> Body {
    Body::from_stream(Parts(chunks.into_iter().collect()))
}
async fn counts(book: &RuntimeBook) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM attachments), (SELECT revision FROM journal_entries LIMIT 1)",
    )
    .fetch_one(book.pool())
    .await
    .unwrap()
}
fn objects(book: &RuntimeBook) -> usize {
    std::fs::read_dir(book.attachments())
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_name() != ".maintenance.lock")
        .count()
}
#[tokio::test]
async fn upload_crosses_json_limit_and_roundtrips_unicode_filename_encrypted_at_rest() {
    let (_fixture, book, entry) = book().await;
    let data: Vec<u8> = (0..(2 * 1024 * 1024 + 19))
        .map(|n| (n % 251) as u8)
        .collect();
    let response = send(
        &book,
        "POST",
        &upload_uri(
            entry,
            "%E6%94%B6%E6%93%9A%20%22.pdf",
            "&expected_revision=1",
        ),
        parts(data.chunks(997).map(|c| Ok(c.to_vec()))),
        Some(data.len() as u64),
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value = json(response).await;
    assert_eq!(value["display_name"], "收據 \".pdf");
    assert_eq!(value["byte_length"], data.len().to_string());
    assert_eq!(counts(&book).await, (1, 2));
    let id = value["id"].as_str().unwrap();
    let encrypted = std::fs::read(book.attachments().join(format!("{id}.kae"))).unwrap();
    assert!(encrypted.starts_with(b"KAEOBJ01"));
    assert!(!encrypted.windows(1024).any(|w| w == &data[500..1524]));
    let response = send(
        &book,
        "GET",
        &format!("/api/attachments/{id}/content"),
        Body::empty(),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["content-length"], data.len().to_string());
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename*=UTF-8''%E6%94%B6%E6%93%9A%20%22.pdf")
    );
    assert_eq!(
        to_bytes(response.into_body(), data.len() + 1)
            .await
            .unwrap()
            .as_ref(),
        data
    );
    book.close().await;
}
#[tokio::test]
async fn empty_upload_is_valid_and_explicit_eof_required() {
    let (_fixture, book, entry) = book().await;
    let response = send(
        &book,
        "POST",
        &upload_uri(entry, "empty.txt", "&expected_revision=1"),
        Body::empty(),
        Some(0),
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json(response).await["byte_length"], "0");
    book.close().await;
}
#[tokio::test]
async fn interrupted_upload_and_length_mismatch_publish_nothing_and_clean_ciphertext_work() {
    let (_fixture, book, entry) = book().await;
    let uri = upload_uri(entry, "receipt.pdf", "&expected_revision=1");
    let response = send(
        &book,
        "POST",
        &uri,
        parts([
            Ok(vec![42; 140_000]),
            Err(std::io::Error::other("connection reset")),
        ]),
        None,
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json(response).await["code"],
        "attachment_upload_interrupted"
    );
    assert_eq!(counts(&book).await, (0, 1));
    assert_eq!(objects(&book), 0);
    let response = send(
        &book,
        "POST",
        &uri,
        Body::from(vec![42; 140_000]),
        Some(140_001),
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json(response).await["code"], "attachment_length_mismatch");
    assert_eq!(counts(&book).await, (0, 1));
    assert_eq!(objects(&book), 0);
    book.close().await;
}
#[tokio::test]
async fn oversize_declared_length_and_unsafe_sources_are_rejected_without_mutation() {
    let (_fixture, book, entry) = book().await;
    let uri = upload_uri(entry, "receipt.pdf", "&expected_revision=1");
    let response = send(
        &book,
        "POST",
        &uri,
        Body::empty(),
        Some(MAX_ATTACHMENT_BYTES + 1),
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    for (name, content) in [
        ("a.pdf", b"#!/bin/zsh".as_slice()),
        ("a.pdf", b"MZfile"),
        ("A.APP", b"innocent"),
        ("a.pdf", b"\x7fELFbad"),
        ("bad%2Fname.pdf", b"data"),
    ] {
        let response = send(
            &book,
            "POST",
            &upload_uri(entry, name, "&expected_revision=1"),
            parts(content.chunks(1).map(|c| Ok(c.to_vec()))),
            None,
            Some("http://127.0.0.1:3000"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{name}");
    }
    assert_eq!(counts(&book).await, (0, 1));
    assert_eq!(objects(&book), 0);
    book.close().await;
}
#[tokio::test]
async fn stale_or_missing_revision_and_cross_origin_upload_cannot_publish() {
    let (_fixture, book, entry) = book().await;
    for revision in ["", "&expected_revision=2"] {
        let response = send(
            &book,
            "POST",
            &upload_uri(entry, "a.pdf", revision),
            Body::from("receipt"),
            None,
            Some("http://127.0.0.1:3000"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
    for origin in [None, Some("http://evil.example")] {
        let response = send(
            &book,
            "POST",
            &upload_uri(entry, "a.pdf", "&expected_revision=1"),
            Body::from("receipt"),
            None,
            origin,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(counts(&book).await, (0, 1));
    assert_eq!(objects(&book), 0);
    book.close().await;
}
#[tokio::test]
async fn tampered_download_fails_before_plaintext_or_success_headers() {
    let (fixture, book, entry) = book().await;
    let source = fixture.0.join("receipt");
    std::fs::write(&source, vec![42; 140_000]).unwrap();
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key()).unwrap();
    let attachment = repository
        .import_file(entry, &source, "receipt.pdf")
        .await
        .unwrap();
    let path = book
        .attachments()
        .join(format!("{}.kae", attachment.id.as_str()));
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(path, bytes).unwrap();
    let response = send(
        &book,
        "GET",
        &format!("/api/attachments/{}/content", attachment.id.as_str()),
        Body::empty(),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.headers().get("content-disposition").is_none());
    assert_eq!(json(response).await["code"], "attachment_integrity_failed");
    book.close().await;
}

async fn posted(book: &RuntimeBook) -> JournalEntryId {
    use kaccounting_backend::accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{JournalLineInput, JournalSide, PostEntry},
        money::MinorUnits,
    };
    let accounts = AccountRepository::new(book.pool().clone());
    let mut lines = Vec::new();
    for (name, side) in [("Cash", JournalSide::Debit), ("Bank", JournalSide::Credit)] {
        let account = accounts
            .create(NewAccount::new(
                None,
                name,
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await
            .unwrap();
        lines.push(JournalLineInput::new(account.id, side, MinorUnits::new(10)));
    }
    JournalRepository::new(book.pool().clone())
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            lines,
        ))
        .await
        .unwrap()
        .id
}
#[tokio::test]
async fn posted_append_is_explicit_and_never_changes_the_entry_or_existing_attachments() {
    let (_fixture, book, _draft) = book().await;
    let id = posted(&book).await;
    let before: (String, i64, String) =
        sqlx::query_as("SELECT status, revision, posted_at FROM journal_entries WHERE id = ?")
            .bind(id.get())
            .fetch_one(book.pool())
            .await
            .unwrap();
    let mut ids = Vec::new();
    for data in ["first receipt", "second receipt"] {
        let response = send(
            &book,
            "POST",
            &upload_uri(id, "receipt.pdf", ""),
            Body::from(data),
            None,
            Some("http://127.0.0.1:3000"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        ids.push(json(response).await["id"].as_str().unwrap().to_owned());
    }
    assert_ne!(ids[0], ids[1]);
    let after: (String, i64, String) =
        sqlx::query_as("SELECT status, revision, posted_at FROM journal_entries WHERE id = ?")
            .bind(id.get())
            .fetch_one(book.pool())
            .await
            .unwrap();
    assert_eq!(before, after);
    let response = send(
        &book,
        "POST",
        &upload_uri(id, "receipt.pdf", "&expected_revision=2"),
        Body::from("third"),
        None,
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/api/attachments/{}", ids[0]))
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"expected_revision":"2"}"#))
        .unwrap();
    let response = localhost_book_commands_router(&book)
        .unwrap()
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json(response).await["code"], "posted_attachment_immutable");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments")
            .fetch_one(book.pool())
            .await
            .unwrap(),
        2
    );
    book.close().await;
}
#[tokio::test]
async fn book_quota_failure_rolls_back_draft_revision_and_ciphertext_candidate() {
    use kaccounting_backend::attachments::MAX_BOOK_ATTACHMENT_BYTES;
    let (_fixture, book, entry) = book().await;
    let mut remaining = MAX_BOOK_ATTACHMENT_BYTES;
    let mut index = 0;
    while remaining != 0 {
        let length = remaining.min(MAX_ATTACHMENT_BYTES);
        let encrypted_length = length + 72 + 20 * length.div_ceil(65536).max(1);
        sqlx::query("INSERT INTO attachments (id, entry_id, display_name, plaintext_sha256, byte_length, ciphertext_sha256, ciphertext_byte_length) VALUES (?, ?, 'quota fixture', ?, ?, ?, ?)")
            .bind(format!("{index:032x}")).bind(entry.get()).bind("0".repeat(64)).bind(length as i64).bind("1".repeat(64)).bind(encrypted_length as i64).execute(book.pool()).await.unwrap();
        remaining -= length;
        index += 1;
    }
    let response = send(
        &book,
        "POST",
        &upload_uri(entry, "a.pdf", "&expected_revision=1"),
        Body::from("receipt"),
        None,
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json(response).await["code"],
        "attachment_book_quota_exceeded"
    );
    assert_eq!(counts(&book).await, (index, 1));
    assert_eq!(objects(&book), 0);
    book.close().await;
}
#[tokio::test]
async fn simultaneous_draft_uploads_have_one_winner_for_the_same_revision() {
    let (_fixture, book, entry) = book().await;
    let uri = upload_uri(entry, "receipt.pdf", "&expected_revision=1");
    let (a, b) = tokio::join!(
        send(
            &book,
            "POST",
            &uri,
            Body::from("first"),
            None,
            Some("http://127.0.0.1:3000")
        ),
        send(
            &book,
            "POST",
            &uri,
            Body::from("second"),
            None,
            Some("http://127.0.0.1:3000")
        ),
    );
    let mut statuses = [a.status(), b.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::CONFLICT]);
    assert_eq!(counts(&book).await, (1, 2));
    assert_eq!(objects(&book), 1);
    book.close().await;
}
struct NeverFinish(Option<Vec<u8>>);
impl futures_core::Stream for NeverFinish {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.0.take() {
            Some(bytes) => std::task::Poll::Ready(Some(Ok(bytes))),
            None => std::task::Poll::Pending,
        }
    }
}
#[tokio::test]
async fn dropping_http_request_cancels_upload_without_publishing_partial_content() {
    let (_fixture, book, entry) = book().await;
    let request = Request::builder()
        .method("POST")
        .uri(upload_uri(entry, "receipt.pdf", "&expected_revision=1"))
        .header("host", "127.0.0.1:3000")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "application/octet-stream")
        .body(Body::from_stream(NeverFinish(Some(vec![42; 140_000]))))
        .unwrap();
    let task = tokio::spawn(
        localhost_book_commands_router(&book)
            .unwrap()
            .oneshot(request),
    );
    let mut started = false;
    for _ in 0..100 {
        if objects(&book) > 0 {
            started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        started,
        "encrypted candidate should be created while stream waits"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    for _ in 0..100 {
        if objects(&book) == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(objects(&book), 0);
    assert_eq!(counts(&book).await, (0, 1));
    book.close().await;
}

struct GeneratedFile(u64);
impl futures_core::Stream for GeneratedFile {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.0 == 0 {
            return std::task::Poll::Ready(None);
        }
        let length = self.0.min(64 * 1024) as usize;
        self.0 -= length as u64;
        std::task::Poll::Ready(Some(Ok(vec![42; length])))
    }
}
#[tokio::test]
#[ignore = "250 MiB HTTP streaming capacity acceptance; run explicitly in release mode"]
async fn maximum_http_stream_roundtrips_and_undeclared_oversize_cleans_candidate() {
    use axum::body::HttpBody;
    use sha2::{Digest, Sha256};
    let (_fixture, book, entry) = book().await;
    let response = send(
        &book,
        "POST",
        &upload_uri(entry, "large.pdf", "&expected_revision=1"),
        Body::from_stream(GeneratedFile(MAX_ATTACHMENT_BYTES)),
        Some(MAX_ATTACHMENT_BYTES),
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let value = json(response).await;
    let response = send(
        &book,
        "GET",
        &format!("/api/attachments/{}/content", value["id"].as_str().unwrap()),
        Body::empty(),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut total = 0u64;
    let mut hash = Sha256::new();
    while let Some(frame) =
        std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await
    {
        if let Ok(bytes) = frame.unwrap().into_data() {
            assert!(bytes.len() <= 64 * 1024);
            assert!(bytes.iter().all(|byte| *byte == 42));
            total += bytes.len() as u64;
            hash.update(&bytes);
        }
    }
    assert_eq!(total, MAX_ATTACHMENT_BYTES);
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(digest, value["sha256"].as_str().unwrap());
    let response = send(
        &book,
        "POST",
        &upload_uri(entry, "too-large.pdf", "&expected_revision=2"),
        Body::from_stream(GeneratedFile(MAX_ATTACHMENT_BYTES + 1)),
        None,
        Some("http://127.0.0.1:3000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(counts(&book).await, (1, 2));
    assert_eq!(objects(&book), 1);
    book.close().await;
}
