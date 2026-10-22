use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalEntryId, JournalLineInput, JournalRepository, JournalSide,
            PostEntry, SaveDraftEntry,
        },
        money::MinorUnits,
    },
    api::{localhost_attachment_router, localhost_write_router},
    attachments::{Attachment, AttachmentRepository},
    crypto::RootEncryptionKey,
    db::open_memory_database,
};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;
struct Fixture {
    root: PathBuf,
    pool: sqlx::SqlitePool,
    repo: AttachmentRepository,
    entry: JournalEntryId,
    attachment: Attachment,
}
impl Fixture {
    async fn new(posted: bool) -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("ka-attachment-http-{id}"));
        std::fs::create_dir(&root).unwrap();
        let pool = open_memory_database().await.unwrap();
        let journal = JournalRepository::new(pool.clone());
        let date = JournalDate::parse("2026-10-22").unwrap();
        let entry = if posted {
            let accounts = AccountRepository::new(pool.clone());
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
                lines.push(JournalLineInput::new(account.id, side, MinorUnits::new(1)));
            }
            journal.post(PostEntry::new(date, lines)).await.unwrap().id
        } else {
            journal
                .create_draft(SaveDraftEntry::new(date, vec![]))
                .await
                .unwrap()
                .id
        };
        let repo = AttachmentRepository::open(
            pool.clone(),
            root.join("objects"),
            Arc::new(RootEncryptionKey::new([43; 32])),
        )
        .unwrap();
        let source = root.join("receipt.pdf");
        std::fs::write(&source, b"%PDF-1.7 private receipt").unwrap();
        let attachment = repo
            .import_file(entry, source, "收據 01.pdf")
            .await
            .unwrap();
        Self {
            root,
            pool,
            repo,
            entry,
            attachment,
        }
    }
    fn router(&self) -> Router {
        localhost_write_router(self.pool.clone()).merge(localhost_attachment_router(
            self.pool.clone(),
            self.repo.clone(),
        ))
    }
    fn path(&self) -> String {
        format!("/api/attachments/{}", self.attachment.id.as_str())
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        json: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:3000")
            .header("origin", "http://127.0.0.1:3000");
        if json.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = self
            .router()
            .oneshot(
                request
                    .body(Body::from(json.unwrap_or("").to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn metadata_and_list_use_exact_strings_and_hide_paths_and_keys() {
    let f = Fixture::new(false).await;
    let (status, value) = f.request("GET", &f.path(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        value["byte_length"],
        f.attachment.plaintext.byte_length().to_string()
    );
    assert_eq!(value["entry_id"], f.entry.get().to_string());
    assert_eq!(value["display_name"], "收據 01.pdf");
    assert_eq!(value["sha256"], f.attachment.plaintext.digest().to_hex());
    assert!(value.get("path").is_none());
    assert!(value.get("key_id").is_none());
    let (_, list) = f
        .request(
            "GET",
            &format!("/api/journal/entries/{}/attachments", f.entry.get()),
            None,
        )
        .await;
    assert_eq!(list, serde_json::json!([value]));
    let (status, value) = f
        .request("GET", "/api/journal/entries/999/attachments", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "entry_not_found");
}
#[tokio::test]
async fn detach_is_revision_checked_and_atomic() {
    let f = Fixture::new(false).await;
    let (status, value) = f
        .request("DELETE", &f.path(), Some(r#"{"expected_revision":"2"}"#))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "stale_draft");
    assert!(f.repo.get(&f.attachment.id).await.is_ok());
    let (status, _) = f
        .request("DELETE", &f.path(), Some(r#"{"expected_revision":"1"}"#))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM journal_entries WHERE id=?")
        .bind(f.entry.get())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(revision, 2);
    assert!(
        f.root
            .join("objects")
            .join(format!("{}.kae", f.attachment.id.as_str()))
            .exists()
    );
}
#[tokio::test]
async fn posted_attachment_cannot_be_detached() {
    let f = Fixture::new(true).await;
    let (status, value) = f
        .request("DELETE", &f.path(), Some(r#"{"expected_revision":"1"}"#))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "posted_attachment_immutable");
    assert!(f.repo.get(&f.attachment.id).await.is_ok());
}
#[tokio::test]
async fn verify_authenticates_ciphertext_and_reports_stable_error() {
    let f = Fixture::new(false).await;
    let path = format!("{}/verify", f.path());
    let (status, value) = f.request("POST", &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["verified"], true);
    std::fs::write(
        f.root
            .join("objects")
            .join(format!("{}.kae", f.attachment.id.as_str())),
        b"corrupt",
    )
    .unwrap();
    let (status, value) = f.request("POST", &path, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "attachment_integrity_failed");
}
#[tokio::test]
async fn all_attachment_routes_reject_host_origin_and_fetch_site_attacks() {
    let f = Fixture::new(false).await;
    for (method, headers) in [
        ("GET", vec![("host", "evil.example")]),
        (
            "GET",
            vec![
                ("host", "127.0.0.1:3000"),
                ("origin", "https://evil.example"),
            ],
        ),
        (
            "GET",
            vec![("host", "127.0.0.1:3000"), ("host", "127.0.0.1:3000")],
        ),
        (
            "GET",
            vec![("host", "127.0.0.1:3000"), ("sec-fetch-site", "cross-site")],
        ),
        ("DELETE", vec![("host", "127.0.0.1:3000")]),
        (
            "DELETE",
            vec![
                ("host", "127.0.0.1:3000"),
                ("origin", "http://127.0.0.1:3000"),
                ("origin", "http://127.0.0.1:3000"),
            ],
        ),
    ] {
        let mut request = Request::builder().method(method).uri(f.path());
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = f
            .router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["content-type"], "application/json");
    }
}
#[tokio::test]
async fn detach_rejects_noncanonical_revision_unknown_fields_and_path_import() {
    let f = Fixture::new(false).await;
    for json in [
        r#"{"expected_revision":1}"#,
        r#"{"expected_revision":"01"}"#,
        r#"{"expected_revision":"0"}"#,
        r#"{"expected_revision":"1","path":"/etc/passwd"}"#,
    ] {
        let (status, _) = f.request("DELETE", &f.path(), Some(json)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, value) = f
        .request(
            "POST",
            &format!("/api/journal/entries/{}/attachments", f.entry.get()),
            Some(r#"{"path":"/etc/passwd"}"#),
        )
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(value["code"], "method_not_allowed");
    assert!(f.repo.get(&f.attachment.id).await.is_ok());
}
