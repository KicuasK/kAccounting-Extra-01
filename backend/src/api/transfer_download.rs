//! Downloads only descriptors captured by a successful verified export producer.
use super::ApiError;
use crate::storage_operations::StorageOperations;
use axum::{
    body::Body,
    extract::{Path, State},
    http::HeaderValue,
    response::Response,
};
use std::io::{self, Write};
struct Stream(tokio::sync::mpsc::Receiver<Result<Vec<u8>, io::Error>>);
impl futures_core::Stream for Stream {
    type Item = Result<Vec<u8>, io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
struct Sender(tokio::sync::mpsc::Sender<Result<Vec<u8>, io::Error>>);
impl Write for Sender {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(64 * 1024);
        self.0
            .blocking_send(Ok(bytes[..count].to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "download disconnected"))?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) async fn download(
    State(operations): State<StorageOperations>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let artifact = operations
        .download_artifact(&id)
        .map_err(|_| unavailable())?;
    let permit = operations
        .acquire_download()
        .map_err(|_| ApiError::Conflict {
            code: "storage_download_busy",
            message:
                "Wait for an active download to finish, or cancel it before downloading again."
                    .into(),
        })?;
    let (artifact, permit) = tokio::task::spawn_blocking(move || {
        // Moving admission into preflight also bounds detached hashing workers
        // when the HTTP request disappears before validation completes.
        let permit = permit;
        artifact.validate()?;
        Ok::<_, crate::storage_operations::OperationError>((artifact, permit))
    })
    .await
    .map_err(|_| ApiError::Internal)?
    .map_err(|_| unavailable())?;
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut output = Sender(sender.clone());
        // The codec checks directory identity and each streamed descriptor again.
        let result = crate::transfer_container::write_container(
            &artifact.root,
            &artifact.entries,
            &mut output,
        )
        .map(|_| ())
        .map_err(|_| io::Error::other("download interrupted"));
        if let Err(error) = result {
            let _ = sender.blocking_send(Err(error));
        }
    });
    let mut response = Response::new(Body::from_stream(Stream(receiver)));
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        "content-disposition",
        HeaderValue::from_static("attachment; filename=\"kAccounting-Extra-01.kagextra\""),
    );
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
fn unavailable() -> ApiError {
    ApiError::NotFound {
        code: "storage_download_not_found",
        message: "This verified export is unavailable. Export again to download.".into(),
    }
}
