//! Trusted native composition. The explicit loopback browser router admits only
//! typed panel actions, never web-supplied helpers, paths or capability IDs.
use crate::{
    native_selection::{NativePicker, NativeSelectionError},
    session_runtime::SessionRuntime,
    storage_operations::SelectionKind,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;
use tokio::sync::watch;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeHostEvent {
    Selected { capability_id: String },
    Cancelled,
}
#[derive(Debug, Error)]
pub enum NativeHostError {
    #[error("native host is shutting down")]
    Closed,
    #[error(transparent)]
    Selection(#[from] NativeSelectionError),
    #[error("native host shutdown worker failed")]
    ShutdownWorker,
}
struct Inner {
    session: SessionRuntime,
    picker: NativePicker,
    cancellation: watch::Sender<u64>,
    closed: AtomicBool,
}
#[derive(Clone)]
pub struct NativeHost(Arc<Inner>);
impl NativeHost {
    /// Local browser composition only. Private Unix ingress deliberately uses
    /// router(), which never includes this native-panel endpoint.
    pub fn browser_router(&self) -> Router {
        Router::new()
            .route("/api/native/selections", post(browser_selection))
            .route("/api/native/books", post(browser_create_book))
            .layer(DefaultBodyLimit::max(4096))
            .with_state(self.clone())
            .fallback_service(self.router())
            .layer(middleware::from_fn_with_state(
                self.clone(),
                require_open_host,
            ))
            .layer(middleware::from_fn(crate::runtime::local_boundary))
    }
    /// `helper` and owner UID come only from trusted local startup configuration.
    /// Construction pins the helper without executing it or accessing OS keys.
    pub fn new(
        session: SessionRuntime,
        helper: &Path,
        expected_owner_uid: u32,
    ) -> Result<Self, NativeHostError> {
        let picker = NativePicker::pin(helper, expected_owner_uid)?;
        let (cancellation, _) = watch::channel(0);
        Ok(Self(Arc::new(Inner {
            session,
            picker,
            cancellation,
            closed: AtomicBool::new(false),
        })))
    }
    /// Existing session/data routes only. Closing this host also gates retained
    /// router clones so they cannot reopen a book after lifecycle shutdown.
    pub fn router(&self) -> Router {
        self.0
            .session
            .router()
            .layer(middleware::from_fn_with_state(
                self.clone(),
                require_open_host,
            ))
            .layer(middleware::from_fn(crate::runtime::local_boundary))
    }
    /// Invoke only for an explicit, authorized native user action. Epoch binding,
    /// one-panel admission, cancellation-on-drop and capability checks are
    /// delegated to the existing picker/session implementations.
    pub async fn select(&self, kind: SelectionKind) -> Result<NativeHostEvent, NativeHostError> {
        let mut cancellation = self.0.cancellation.subscribe();
        if self.0.closed.load(Ordering::Acquire) {
            return Err(NativeHostError::Closed);
        }
        tokio::select! {
            biased;
            _ = cancellation.changed() => {
                if self.0.closed.load(Ordering::Acquire) { Err(NativeHostError::Closed) }
                else { Ok(NativeHostEvent::Cancelled) }
            }
            selected = self.0.picker.select_and_register(&self.0.session, kind) => {
                Ok(match selected? { Some(capability_id) => NativeHostEvent::Selected { capability_id }, None => NativeHostEvent::Cancelled })
            }
        }
    }
    /// Cancel only this host's active panel and wait until its child is reaped.
    /// A subsequent explicit action can open a new panel while the host is open.
    pub async fn cancel_selection(&self) -> Result<(), NativeHostError> {
        self.0
            .cancellation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        self.0.picker.wait_idle().await?;
        Ok(())
    }
    /// Stop admitting native actions and HTTP book reopening, cancel/reap the
    /// panel, then drain response leases/storage jobs and close the selected book.
    pub async fn shutdown(&self) -> Result<(), NativeHostError> {
        self.0.closed.store(true, Ordering::Release);
        let host = self.clone();
        // Cancellation/reaping and subsequent book draining must survive a
        // caller dropping this future before the panel worker has exited.
        tokio::spawn(async move {
            host.cancel_selection().await?;
            host.0.session.shutdown().await;
            Ok(())
        })
        .await
        .map_err(|_| NativeHostError::ShutdownWorker)?
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BrowserSelectionKind {
    Book,
    Request,
    Package,
    Backup,
    RequestDestination,
    ExportDestination,
    BookDestination,
    RecoveryParent,
}
impl From<BrowserSelectionKind> for SelectionKind {
    fn from(value: BrowserSelectionKind) -> Self {
        match value {
            BrowserSelectionKind::Book => Self::Book,
            BrowserSelectionKind::Request => Self::Request,
            BrowserSelectionKind::Package => Self::Package,
            BrowserSelectionKind::Backup => Self::Backup,
            BrowserSelectionKind::RequestDestination => Self::RequestDestination,
            BrowserSelectionKind::ExportDestination => Self::ExportDestination,
            BrowserSelectionKind::BookDestination => Self::BookDestination,
            BrowserSelectionKind::RecoveryParent => Self::RecoveryParent,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSelectionInput {
    kind: BrowserSelectionKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCreateBookInput {}
async fn browser_create_book(
    State(host): State<NativeHost>,
    headers: HeaderMap,
    input: Result<Json<BrowserCreateBookInput>, JsonRejection>,
) -> Response {
    if input.is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":{"code":"invalid_native_book_creation"}})),
        )
            .into_response();
    }
    let guard = match host.0.session.native_creation_guard(&headers).await {
        Ok(guard) => guard,
        Err(response) => return *response,
    };
    let mut cancellation = host.0.cancellation.subscribe();
    if host.0.closed.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"native_host_closed"}})),
        )
            .into_response();
    }
    let selected = tokio::select! {
        biased;
        _ = cancellation.changed() => if host.0.closed.load(Ordering::Acquire) { Err(NativeHostError::Closed) } else { Ok(None) },
        selected = host.0.picker.choose_book_destination(host.0.session.native_collection_directory()) => selected.map_err(NativeHostError::from),
    };
    match selected {
        Ok(None) => {
            Json(serde_json::json!({"book_id":null,"selection_required":true})).into_response()
        }
        Ok(Some(path)) => match host.0.session.create_native_book(guard, path).await {
            Ok(book_id) => (
                StatusCode::CREATED,
                Json(serde_json::json!({"book_id":book_id,"selection_required":true})),
            )
                .into_response(),
            Err(crate::runtime::RuntimeError::UnsafePath) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error":{"code":"invalid_native_book_destination"}})),
            )
                .into_response(),
            Err(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error":{"code":"native_book_creation_failed"}})),
            )
                .into_response(),
        },
        Err(NativeHostError::Closed) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"native_host_closed"}})),
        )
            .into_response(),
        Err(NativeHostError::Selection(NativeSelectionError::Busy)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":{"code":"native_picker_busy"}})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":{"code":"native_selection_failed"}})),
        )
            .into_response(),
    }
}
async fn browser_selection(
    State(host): State<NativeHost>,
    headers: HeaderMap,
    input: Result<Json<BrowserSelectionInput>, JsonRejection>,
) -> Response {
    let Json(input) = match input {
        Ok(input) => input,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":{"code":"invalid_native_selection"}})),
            )
                .into_response();
        }
    };
    let guard = match host.0.session.native_action_guard(&headers).await {
        Ok(guard) => guard,
        Err(response) => return *response,
    };
    let mut cancellation = host.0.cancellation.subscribe();
    if host.0.closed.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"native_host_closed"}})),
        )
            .into_response();
    }
    let selected = tokio::select! {
        biased;
        _ = cancellation.changed() => if host.0.closed.load(Ordering::Acquire) { Err(NativeHostError::Closed) } else { Ok(None) },
        selected = host.0.picker.select_and_register_guarded(&guard, input.kind.into()) => selected.map_err(NativeHostError::from),
    };
    match selected {
        Ok(capability_id) => {
            Json(serde_json::json!({"capability_id":capability_id})).into_response()
        }
        Err(NativeHostError::Closed) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"native_host_closed"}})),
        )
            .into_response(),
        Err(NativeHostError::Selection(NativeSelectionError::Busy)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":{"code":"native_picker_busy"}})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":{"code":"native_selection_failed"}})),
        )
            .into_response(),
    }
}
async fn require_open_host(
    State(host): State<NativeHost>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if host.0.closed.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": {"code": "native_host_closed"}})),
        )
            .into_response();
    }
    next.run(request).await
}
