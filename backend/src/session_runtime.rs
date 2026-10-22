//! Explicit process-local book selection. HTTP never supplies filesystem paths.
use crate::{
    api, book_selection,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    runtime::{self, RuntimeBook, RuntimeError},
    storage_operations::StorageOperations,
};
use axum::{
    Json, Router,
    body::{Body, BodyDataStream, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use tower::ServiceExt;
use zeroize::Zeroizing;

#[derive(Clone)]
struct Device(Arc<dyn DeviceKeyStore + Send + Sync>);
impl DeviceKeyStore for Device {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.read(id)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.0.insert_new(id, bytes)
    }
}
#[derive(Clone)]
struct Recipient(Arc<dyn RecipientKeyStore + Send + Sync>);
impl RecipientKeyStore for Recipient {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.read(id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.0.insert_new(id, bytes)
    }
}
struct Selected {
    id: String,
    native_epoch: Arc<()>,
    book: Arc<RuntimeBook>,
    operations: StorageOperations,
    routes: Router,
}
#[derive(Clone)]
pub struct SessionRuntime {
    parent: Arc<PathBuf>,
    device: Arc<Device>,
    recipient: Arc<Recipient>,
    selected: Arc<RwLock<Option<Selected>>>,
    closed: Arc<AtomicBool>,
    assets: Router,
}
/// Opaque, process-local selection generation captured before a native dialog.
/// Returning to the same book after a switch does not revive an old token.
#[derive(Clone)]
pub struct NativeSelectionSession(Arc<()>);
#[derive(Serialize)]
struct SessionStatus {
    book_id: Option<String>,
    selection_required: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionInput {
    book_id: String,
}
fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({"error": {"code": code}}))).into_response()
}
impl SessionRuntime {
    /// The collection is preconfigured locally. Construction/discovery never reads keys.
    pub fn new<D, R>(
        parent: impl AsRef<Path>,
        frontend_dist: impl AsRef<Path>,
        device: Arc<D>,
        recipient: Arc<R>,
    ) -> Result<Self, RuntimeError>
    where
        D: DeviceKeyStore + Send + Sync + 'static,
        R: RecipientKeyStore + Send + Sync + 'static,
    {
        book_selection::discover_books(parent.as_ref())?;
        Ok(Self {
            parent: Arc::new(parent.as_ref().canonicalize()?),
            device: Arc::new(Device(device)),
            recipient: Arc::new(Recipient(recipient)),
            selected: Arc::new(RwLock::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
            assets: runtime::static_router(frontend_dist)?,
        })
    }
    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/session", get(status))
            .route("/api/session/books", get(books))
            .route("/api/session/selection", post(select).delete(deselect))
            .fallback(dispatch)
            .layer(DefaultBodyLimit::max(4096))
            .with_state(self.clone())
            .layer(middleware::from_fn(runtime::local_boundary))
    }
    /// Native/local integration only, after an explicit OS selection. There is
    /// deliberately no corresponding HTTP path-registration endpoint.
    pub async fn register_storage_selection(
        &self,
        kind: crate::storage_operations::SelectionKind,
        path: impl AsRef<Path>,
    ) -> Result<String, crate::storage_operations::OperationError> {
        let lease = self.selected.read().await;
        let selected = lease
            .as_ref()
            .ok_or(crate::storage_operations::OperationError::InvalidSelection)?;
        selected.operations.register_selection(kind, path)
    }
    pub async fn native_selection_session(
        &self,
    ) -> Result<NativeSelectionSession, crate::storage_operations::OperationError> {
        let lease = self.selected.read().await;
        let active = lease
            .as_ref()
            .ok_or(crate::storage_operations::OperationError::InvalidSelection)?;
        Ok(NativeSelectionSession(active.native_epoch.clone()))
    }
    /// Atomically refuse native dialog results belonging to an older selection.
    pub async fn register_storage_selection_for(
        &self,
        session: &NativeSelectionSession,
        kind: crate::storage_operations::SelectionKind,
        path: impl AsRef<Path>,
    ) -> Result<String, crate::storage_operations::OperationError> {
        let lease = self.selected.read().await;
        let active = lease
            .as_ref()
            .ok_or(crate::storage_operations::OperationError::InvalidSelection)?;
        if !Arc::ptr_eq(&active.native_epoch, &session.0) {
            return Err(crate::storage_operations::OperationError::InvalidSelection);
        }
        active.operations.register_selection(kind, path)
    }
    /// Wait for responses and owned storage jobs before releasing the process lock.
    pub async fn shutdown(&self) {
        // Set admission state before waiting for leases. A request whose JSON
        // body completes later must not reopen this permanently closed session.
        self.closed.store(true, Ordering::Release);
        let session = self.clone();
        // Own the drain even if the caller abandons its shutdown wait while a
        // streaming response or storage job retains the selected book.
        if tokio::spawn(async move { session.finish_shutdown().await })
            .await
            .is_err()
        {
            // Preserve the existing infallible API; if a worker unexpectedly
            // panics, retry the same guarded close rather than reporting a drain
            // as complete while retaining the book indefinitely.
            self.finish_shutdown().await;
        }
    }
    async fn finish_shutdown(&self) {
        let mut selected = self.selected.write().await;
        if let Some(active) = selected.as_ref() {
            active.operations.wait_idle().await;
            active.book.close().await;
        }
        *selected = None;
    }
    async fn choose(&self, id: String) -> Response {
        // Reject all path syntax, hidden work names and unlisted candidates before key access.
        if id.is_empty()
            || id.starts_with('.')
            || id.contains(['/', '\\'])
            || Path::new(&id).components().count() != 1
        {
            return error(StatusCode::BAD_REQUEST, "invalid_book_selection");
        }
        let Ok(mut selected) = self.selected.clone().try_write_owned() else {
            return error(StatusCode::CONFLICT, "book_busy");
        };
        // Check while owning the transition lock. An already-opening book is
        // drained by shutdown; a delayed transition can never run after it.
        if self.closed.load(Ordering::Acquire) {
            return error(StatusCode::SERVICE_UNAVAILABLE, "session_closed");
        }
        if let Some(active) = selected.as_ref() {
            if !active.operations.is_idle() {
                return error(StatusCode::CONFLICT, "book_busy");
            }
            if active.id == id {
                return Json(SessionStatus {
                    book_id: Some(id),
                    selection_required: false,
                })
                .into_response();
            }
        }
        let candidates = match book_selection::discover_books(self.parent.as_ref()) {
            Ok(v) => v,
            Err(_) => return error(StatusCode::CONFLICT, "book_collection_unavailable"),
        };
        if !candidates.iter().any(|book| book.id == id) {
            return error(StatusCode::NOT_FOUND, "book_not_available");
        }
        let book =
            match RuntimeBook::open_existing(self.parent.join(&id), self.device.as_ref()).await {
                Ok(v) => Arc::new(v),
                Err(RuntimeError::Key(key_error)) => {
                    let problem = crate::keychain_diagnostics::classify(&key_error);
                    return (
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "error": { "code": problem.code, "message": problem.operator_guidance }
                        })),
                    )
                        .into_response();
                }
                Err(_) => return error(StatusCode::CONFLICT, "book_open_failed"),
            };
        let operations = match StorageOperations::new(self.device.clone(), self.recipient.clone())
            .with_active_book(book.clone())
        {
            Ok(v) => v,
            Err(_) => {
                book.close().await;
                return error(StatusCode::CONFLICT, "book_busy");
            }
        };
        let commands = match api::localhost_book_commands_router(&book) {
            Ok(v) => v,
            Err(_) => {
                book.close().await;
                return error(StatusCode::CONFLICT, "book_open_failed");
            }
        };
        let routes = api::localhost_write_router(book.pool().clone())
            .merge(commands)
            .merge(api::localhost_storage_router(
                book.pool().clone(),
                operations.clone(),
            ));
        let previous = selected.replace(Selected {
            id: id.clone(),
            native_epoch: Arc::new(()),
            book,
            operations,
            routes,
        });
        if let Some(previous) = previous {
            previous.book.close().await;
        }
        Json(SessionStatus {
            book_id: Some(id),
            selection_required: false,
        })
        .into_response()
    }
    async fn clear(&self) -> Response {
        let Ok(mut selected) = self.selected.clone().try_write_owned() else {
            return error(StatusCode::CONFLICT, "book_busy");
        };
        // Check while owning the transition lock. An already-opening book is
        // drained by shutdown; a delayed transition can never run after it.
        if self.closed.load(Ordering::Acquire) {
            return error(StatusCode::SERVICE_UNAVAILABLE, "session_closed");
        }
        if selected
            .as_ref()
            .is_some_and(|active| !active.operations.is_idle())
        {
            return error(StatusCode::CONFLICT, "book_busy");
        }
        if let Some(previous) = selected.take() {
            previous.book.close().await;
        }
        Json(SessionStatus {
            book_id: None,
            selection_required: true,
        })
        .into_response()
    }
}
async fn status(State(session): State<SessionRuntime>) -> Response {
    let selected = session.selected.read().await;
    Json(SessionStatus {
        book_id: selected.as_ref().map(|b| b.id.clone()),
        selection_required: selected.is_none(),
    })
    .into_response()
}
async fn books(State(session): State<SessionRuntime>) -> Response {
    match book_selection::discover_books(session.parent.as_ref()) {
        Ok(books) => Json(serde_json::json!({"books": books})).into_response(),
        Err(_) => error(StatusCode::CONFLICT, "book_collection_unavailable"),
    }
}
async fn select(
    State(session): State<SessionRuntime>,
    Json(input): Json<SelectionInput>,
) -> Response {
    // Own transitions despite disconnect/cancellation, including SQLite migrations.
    tokio::spawn(async move { session.choose(input.book_id).await })
        .await
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session_worker_failed"))
}
async fn deselect(State(session): State<SessionRuntime>) -> Response {
    tokio::spawn(async move { session.clear().await })
        .await
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session_worker_failed"))
}
async fn dispatch(State(session): State<SessionRuntime>, request: Request<Body>) -> Response {
    if request.uri().path() == "/" || request.uri().path().starts_with("/assets/") {
        return match session.assets.clone().oneshot(request).await {
            Ok(response) => response,
            Err(infallible) => match infallible {},
        };
    }
    if session.closed.load(Ordering::Acquire) {
        return error(StatusCode::SERVICE_UNAVAILABLE, "session_closed");
    }
    let lease = session.selected.clone().read_owned().await;
    if session.closed.load(Ordering::Acquire) {
        return error(StatusCode::SERVICE_UNAVAILABLE, "session_closed");
    }
    let Some(active) = lease.as_ref() else {
        return error(StatusCode::CONFLICT, "selection_required");
    };
    let response = match active.routes.clone().oneshot(request).await {
        Ok(response) => response,
        Err(infallible) => match infallible {},
    };
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::from_stream(LeasedBody {
            inner: body.into_data_stream(),
            _lease: Some(lease),
        }),
    )
}
// The owned read lease survives handler completion and is dropped only when the
// response body is consumed/dropped. A slow streaming client prevents switching.
struct LeasedBody {
    inner: BodyDataStream,
    _lease: Option<OwnedRwLockReadGuard<Option<Selected>>>,
}
impl Stream for LeasedBody {
    type Item = Result<Bytes, axum::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = Pin::new(&mut self.inner).poll_next(cx);
        if matches!(&result, Poll::Ready(None)) {
            self._lease.take();
        }
        result
    }
}
