//! Trusted in-process native composition. There is no panel HTTP endpoint or
//! web-supplied helper/path input. Only an explicit host action invokes a panel.
use crate::{
    native_selection::{NativePicker, NativeSelectionError},
    session_runtime::SessionRuntime,
    storage_operations::SelectionKind,
};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
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
