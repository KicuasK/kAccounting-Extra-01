//! Book-owned attachment commands. Paths and key material never cross HTTP.
use super::{ApiError, ExactInteger, parse_journal_entry_id};
use crate::attachments::{Attachment, AttachmentError, AttachmentId, AttachmentRepository};
use axum::{
    Json, Router,
    body::{Body, HttpBody},
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

pub(super) fn attachment_routes(repository: AttachmentRepository) -> Router {
    Router::new()
        .route(
            "/api/journal/entries/{entry_id}/attachments",
            get(list).post(upload),
        )
        .route("/api/attachments/{attachment_id}/content", get(download))
        .route(
            "/api/attachments/{attachment_id}",
            get(metadata).delete(detach),
        )
        .route("/api/attachments/{attachment_id}/verify", post(verify))
        .with_state(repository)
}

// The request body is read as frames, bypassing JSON buffering. Both the
// bridge and synchronous codec use at most two 64 KiB payload chunks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadInput {
    display_name: String,
    expected_revision: Option<String>,
    source_kind: String,
}
async fn upload(
    State(repository): State<AttachmentRepository>,
    Path(entry_id): Path<String>,
    input: Result<Query<UploadInput>, QueryRejection>,
    headers: HeaderMap,
    mut body: Body,
) -> Result<(StatusCode, Json<AttachmentResponse>), ApiError> {
    let entry_id = parse_journal_entry_id(&entry_id)?;
    let Query(input) = input.map_err(|_| ApiError::BadRequest { code: "invalid_upload_metadata", message: "Send display_name, source_kind=regular_file, and the current expected_revision for a draft.".to_owned() })?;
    if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/octet-stream")
    {
        return Err(ApiError::BadRequest {
            code: "binary_body_required",
            message: "Send the regular file bytes as application/octet-stream.".to_owned(),
        });
    }
    if input.source_kind != "regular_file" {
        return Err(ApiError::BadRequest {
            code: "unsafe_attachment_source",
            message: "Select a regular file; directories and packages are not supported."
                .to_owned(),
        });
    }
    let revision = input.expected_revision.as_deref().map(|value| {
        value.parse::<i64>().ok().filter(|n| *n > 0 && n.to_string() == value).ok_or(ApiError::BadRequest { code: "invalid_revision", message: "Use a canonical positive revision string for a draft; omit it for a posted entry.".to_owned() })
    }).transpose()?;
    let declared = headers
        .get("content-length")
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or(ApiError::BadRequest {
                    code: "invalid_content_length",
                    message: "Invalid Content-Length.".to_owned(),
                })
        })
        .transpose()?;
    if declared.is_some_and(|n| n > crate::attachments::MAX_ATTACHMENT_BYTES) {
        return Err(ApiError::AttachmentPayloadTooLarge);
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let reader = UploadReader {
        receiver,
        current: std::io::Cursor::new(Vec::new()),
        finished: false,
    };
    let consume = repository.import_stream(entry_id, &input.display_name, revision, reader);
    let pump = async move {
        let mut total = 0u64;
        while let Some(frame) =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await
        {
            let frame = frame.map_err(|_| ApiError::BadRequest {
                code: "attachment_upload_interrupted",
                message: "The upload did not complete. Retry the whole file.".to_owned(),
            })?;
            if let Ok(bytes) = frame.into_data() {
                total = total
                    .checked_add(bytes.len() as u64)
                    .ok_or(ApiError::AttachmentPayloadTooLarge)?;
                if total > crate::attachments::MAX_ATTACHMENT_BYTES {
                    return Err(ApiError::AttachmentPayloadTooLarge);
                }
                for chunk in bytes.chunks(64 * 1024) {
                    if sender.send(Some(chunk.to_vec())).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
        if declared.is_some_and(|n| n != total) {
            return Err(ApiError::BadRequest {
                code: "attachment_length_mismatch",
                message: "The upload length did not match Content-Length.".to_owned(),
            });
        }
        let _ = sender.send(None).await;
        Ok(())
    };
    let (result, pumped) = tokio::join!(consume, pump);
    // Prefer actual transport/size failures to the crypto worker's interrupted
    // input error. In either case there is no published database reference.
    pumped?;
    Ok((
        StatusCode::CREATED,
        Json(result.map_err(attachment_error)?.into()),
    ))
}
struct UploadReader {
    receiver: tokio::sync::mpsc::Receiver<Option<Vec<u8>>>,
    current: std::io::Cursor<Vec<u8>>,
    finished: bool,
}
impl std::io::Read for UploadReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.current.read(bytes)?;
            if n > 0 || self.finished {
                return Ok(n);
            }
            match self.receiver.blocking_recv() {
                Some(Some(chunk)) => self.current = std::io::Cursor::new(chunk),
                Some(None) => self.finished = true,
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "upload disconnected",
                    ));
                }
            }
        }
    }
}
struct DownloadStream(tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>);
impl futures_core::Stream for DownloadStream {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
async fn download(
    State(repository): State<AttachmentRepository>,
    Path(attachment_id): Path<String>,
) -> Result<Response, ApiError> {
    let (attachment, receiver) = repository
        .download_stream(&parse_id(&attachment_id)?)
        .await
        .map_err(attachment_error)?;
    let mut response = Response::new(Body::from_stream(DownloadStream(receiver)));
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        "content-length",
        HeaderValue::from_str(&attachment.plaintext.byte_length().to_string())
            .map_err(|_| ApiError::Internal)?,
    );
    // An ASCII fallback plus RFC 5987 encoding preserves Chinese filenames and
    // prevents names from injecting header syntax or enabling inline scripts.
    let encoded: String = attachment
        .display_name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    headers.insert(
        "content-disposition",
        HeaderValue::from_str(&format!(
            "attachment; filename=\"attachment\"; filename*=UTF-8''{encoded}"
        ))
        .map_err(|_| ApiError::Internal)?,
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(Serialize)]
struct AttachmentResponse {
    id: String,
    entry_id: ExactInteger,
    display_name: String,
    byte_length: String,
    sha256: String,
    created_at: String,
}
impl From<Attachment> for AttachmentResponse {
    fn from(value: Attachment) -> Self {
        Self {
            id: value.id.as_str().to_owned(),
            entry_id: value.entry_id.get().into(),
            display_name: value.display_name,
            byte_length: value.plaintext.byte_length().to_string(),
            sha256: value.plaintext.digest().to_hex(),
            created_at: value.created_at,
        }
    }
}
fn parse_id(value: &str) -> Result<AttachmentId, ApiError> {
    AttachmentId::parse(value).map_err(attachment_error)
}
async fn list(
    State(repository): State<AttachmentRepository>,
    Path(entry_id): Path<String>,
) -> Result<Json<Vec<AttachmentResponse>>, ApiError> {
    let attachments = repository
        .list_for_entry(parse_journal_entry_id(&entry_id)?)
        .await
        .map_err(attachment_error)?;
    Ok(Json(attachments.into_iter().map(Into::into).collect()))
}
async fn metadata(
    State(repository): State<AttachmentRepository>,
    Path(attachment_id): Path<String>,
) -> Result<Json<AttachmentResponse>, ApiError> {
    Ok(Json(
        repository
            .get(&parse_id(&attachment_id)?)
            .await
            .map_err(attachment_error)?
            .into(),
    ))
}
#[derive(Serialize)]
struct VerificationResponse {
    id: String,
    verified: bool,
}
async fn verify(
    State(repository): State<AttachmentRepository>,
    Path(attachment_id): Path<String>,
) -> Result<Json<VerificationResponse>, ApiError> {
    let id = parse_id(&attachment_id)?;
    repository.verify(&id).await.map_err(attachment_error)?;
    Ok(Json(VerificationResponse {
        id: attachment_id,
        verified: true,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetachInput {
    expected_revision: ExactInteger,
}
async fn detach(
    State(repository): State<AttachmentRepository>,
    Path(attachment_id): Path<String>,
    input: Result<Json<DetachInput>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let input = input
        .map(|Json(value)| value)
        .map_err(|error| match error.status() {
            StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
            StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::JsonRequired,
            _ => ApiError::BadRequest {
                code: "invalid_json",
                message: "Send expected_revision as a canonical decimal string.".to_owned(),
            },
        })?;
    if input.expected_revision.0 < 1 {
        return Err(ApiError::BadRequest {
            code: "invalid_revision",
            message: "expected_revision must be a positive integer string.".to_owned(),
        });
    }
    repository
        .remove_from_draft(&parse_id(&attachment_id)?, input.expected_revision.0)
        .await
        .map_err(attachment_error)?;
    Ok(StatusCode::NO_CONTENT)
}
fn attachment_error(error: AttachmentError) -> ApiError {
    match error {
        AttachmentError::InvalidName => ApiError::BadRequest { code: "invalid_attachment_name", message: "Use a plain display filename of 1 to 255 non-control characters.".to_owned() },
        AttachmentError::UnsafeSource => ApiError::BadRequest { code: "unsafe_attachment_source", message: "Executables and packages are not supported.".to_owned() },
        AttachmentError::FileTooLarge => ApiError::AttachmentPayloadTooLarge,
        AttachmentError::BookQuotaExceeded => ApiError::Conflict { code: "attachment_book_quota_exceeded", message: "The book attachment limit is 20 GiB.".to_owned() },
        AttachmentError::InvalidId => ApiError::BadRequest { code: "invalid_attachment_id", message: "Use a 32 character lowercase hexadecimal attachment identifier.".to_owned() },
        AttachmentError::NotFound => ApiError::NotFound { code: "attachment_not_found", message: "The attachment was not found.".to_owned() },
        AttachmentError::EntryNotFound => ApiError::NotFound { code: "entry_not_found", message: "The journal entry was not found.".to_owned() },
        AttachmentError::StaleDraft => ApiError::Conflict { code: "stale_draft", message: "The draft changed or the attachment revision is invalid. Reload the entry; include the current revision for drafts and omit it for posted entries.".to_owned() },
        AttachmentError::PostedImmutable => ApiError::Conflict { code: "posted_attachment_immutable", message: "Posted attachment references cannot be removed.".to_owned() },
        AttachmentError::RevisionExhausted => ApiError::Conflict { code: "revision_exhausted", message: "The draft revision is exhausted.".to_owned() },
        AttachmentError::Integrity | AttachmentError::Crypto(_) | AttachmentError::Verification(_) => ApiError::Conflict { code: "attachment_integrity_failed", message: "Attachment integrity verification failed. Preserve the book and restore a verified backup.".to_owned() },
        _ => ApiError::Internal,
    }
}

use crate::{
    backup_service::{self, BackupReason, BackupServiceError, BookBackupId},
    crypto::RootEncryptionKey,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::RuntimeBook,
};
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use zeroize::Zeroizing;

#[derive(Clone)]
struct BackupState {
    pool: sqlx::SqlitePool,
    attachment_root: PathBuf,
    backup_root: PathBuf,
    book_id: Arc<OnceLock<BookBackupId>>,
    keys: CachedKey,
}
/// Read-only process-held key material; HTTP never generates or stores keys.
#[derive(Clone)]
struct CachedKey {
    id: DeviceKeyId,
    key: Arc<RootEncryptionKey>,
}
impl DeviceKeyStore for CachedKey {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        if *id != self.id {
            return Err(KeyStoreError::Missing);
        }
        Ok(Zeroizing::new(self.key.as_bytes().to_vec()))
    }
    fn insert_new(&self, _id: &DeviceKeyId, _bytes: &[u8]) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::Unavailable(-1))
    }
}
pub(super) fn backup_routes(book: &RuntimeBook) -> Router {
    Router::new()
        .route("/api/backups", post(create_backup))
        .with_state(BackupState {
            pool: book.pool().clone(),
            attachment_root: book.attachments(),
            backup_root: book.root().join("backups"),
            book_id: book.backup_identity_cell(),
            keys: CachedKey {
                id: book.key_id().clone(),
                key: book.key(),
            },
        })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyBackupInput {}
#[derive(Serialize)]
struct BackupResponse {
    backup_name: String,
    verified: bool,
    attachment_count: String,
    attachment_bytes: String,
    retention_warning: Option<&'static str>,
    preserved_unrecognized: Option<String>,
}
async fn create_backup(
    State(state): State<BackupState>,
    input: Result<Json<EmptyBackupInput>, JsonRejection>,
) -> Result<(StatusCode, Json<BackupResponse>), ApiError> {
    let Json(_) = input.map_err(|error| match error.status() {
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::JsonRequired,
        _ => ApiError::BadRequest {
            code: "invalid_json",
            message: "Send an empty JSON object to create a local backup.".to_owned(),
        },
    })?;
    let book_id=state.book_id.get().ok_or(ApiError::Conflict{code:"backup_identity_missing",message:"This book requires an explicit backup identity setup before managed backups can be created.".to_owned()})?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(&state.backup_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ApiError::Internal),
    }
    // Persist a newly created backup directory before publishing a checkpoint.
    std::fs::File::open(state.backup_root.parent().ok_or(ApiError::Internal)?)
        .and_then(|file| file.sync_all())
        .map_err(|_| ApiError::Internal)?;
    let outcome = backup_service::create_managed_backup(
        &state.pool,
        &state.attachment_root,
        &state.backup_root,
        book_id,
        &state.keys.id,
        &state.keys,
        BackupReason::Manual,
    )
    .await
    .map_err(backup_error)?;
    let backup_name = outcome
        .path
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .ok_or(ApiError::Internal)?
        .to_owned();
    // Backup success is independent of maintenance. Retention failure remains
    // visible while the newly authenticated checkpoint remains available.
    let (retention_warning, preserved) = match backup_service::retain_latest_verified_backups(
        &state.backup_root,
        book_id,
        &state.keys.id,
        &state.keys,
    )
    .await
    {
        Ok(result) => (None, Some(result.preserved_unrecognized.len().to_string())),
        Err(_) => (
            Some("The backup was verified, but older-backup maintenance did not complete."),
            None,
        ),
    };
    Ok((
        StatusCode::CREATED,
        Json(BackupResponse {
            backup_name,
            verified: true,
            attachment_count: outcome.attachment_count.to_string(),
            attachment_bytes: outcome.attachment_bytes.to_string(),
            retention_warning,
            preserved_unrecognized: preserved,
        }),
    ))
}
fn backup_error(error: BackupServiceError) -> ApiError {
    match error {
        BackupServiceError::Busy=>ApiError::Conflict{code:"backup_busy",message:"Another backup operation is active. Try again after it finishes.".to_owned()},
        BackupServiceError::InvalidIdentity=>ApiError::Conflict{code:"backup_identity_invalid",message:"The backup directory or identity is invalid. Preserve existing files for inspection.".to_owned()},
        _=>ApiError::Internal,
    }
}
