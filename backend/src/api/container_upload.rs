//! Binary encrypted transport, followed by full authenticated recipient import.
use super::ApiError;
use crate::storage_operations::{OperationError, OperationResult, StorageOperations};
use axum::{
    Json,
    body::{Body, HttpBody},
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    request: String,
    destination: String,
}
struct Reader {
    receiver: tokio::sync::mpsc::Receiver<Option<Vec<u8>>>,
    current: std::io::Cursor<Vec<u8>>,
    done: bool,
}
impl std::io::Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.current.read(bytes)?;
            if n > 0 || self.done {
                return Ok(n);
            }
            match self.receiver.blocking_recv() {
                Some(Some(bytes)) => self.current = std::io::Cursor::new(bytes),
                Some(None) => self.done = true,
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "upload interrupted",
                    ));
                }
            }
        }
    }
}
fn bad() -> ApiError {
    ApiError::BadRequest { code: "invalid_container_upload", message: "Send a complete encrypted package with valid request and new-book destination selections.".into() }
}
pub(super) async fn upload(
    State(operations): State<StorageOperations>,
    input: Result<Query<Input>, QueryRejection>,
    headers: HeaderMap,
    mut body: Body,
) -> Result<(StatusCode, Json<OperationResult>), ApiError> {
    let Query(input) = input.map_err(|_| bad())?;
    if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/octet-stream")
    {
        return Err(bad());
    }
    let maximum = operations.container_upload_maximum();
    let declared = headers
        .get("content-length")
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(bad)
        })
        .transpose()?;
    if declared.is_some_and(|n| n > maximum) {
        return Err(ApiError::TransferPayloadTooLarge);
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let handle = operations
        .begin_container_import(
            &input.request,
            &input.destination,
            Reader {
                receiver,
                current: std::io::Cursor::new(Vec::new()),
                done: false,
            },
        )
        .map_err(|e| match e {
            OperationError::Busy => ApiError::Conflict {
                code: "storage_busy",
                message: "Wait for the active storage operation to finish.".into(),
            },
            _ => bad(),
        })?;
    let pump = async move {
        let mut total = 0u64;
        while let Some(frame) =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await
        {
            let frame = frame.map_err(|_| bad())?;
            if let Ok(bytes) = frame.into_data() {
                total = total
                    .checked_add(bytes.len() as u64)
                    .ok_or(ApiError::TransferPayloadTooLarge)?;
                if total > maximum {
                    return Err(ApiError::TransferPayloadTooLarge);
                }
                for chunk in bytes.chunks(64 * 1024) {
                    if sender.send(Some(chunk.to_vec())).await.is_err() {
                        return Err(bad());
                    }
                }
            }
        }
        if declared.is_some_and(|n| n != total) {
            return Err(bad());
        }
        sender.send(None).await.map_err(|_| bad())?;
        Ok(())
    };
    let (result, pumped) = tokio::join!(handle, pump);
    pumped?;
    Ok((
        StatusCode::CREATED,
        Json(result.map_err(|_| ApiError::Internal)?.map_err(|_| bad())?),
    ))
}
