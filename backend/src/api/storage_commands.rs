//! Capability-only storage HTTP routes. The parent wrapper applies Host/Origin,
//! request size and no-store controls to every route, including status reads.
use super::ApiError;
use crate::storage_operations::{
    ExportInput, ImportInput, OperationError, OperationStatus, RecoveryInput, RequestInput,
    RequestPreview, RestoreInput, StorageOperations,
};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
pub(super) fn routes(operations: StorageOperations) -> Router {
    Router::new()
        .route("/api/storage/recipient-requests", post(request))
        .route(
            "/api/storage/container-imports",
            post(super::container_upload::upload),
        )
        .route("/api/storage/exports", post(export))
        .route("/api/storage/imports", post(import))
        .route("/api/storage/restores", post(restore))
        .route("/api/storage/recoveries", post(recover))
        .route("/api/storage/operations/{operation_id}", get(status))
        .route(
            "/api/storage/operations/{operation_id}/download",
            get(super::transfer_download::download),
        )
        .route(
            "/api/storage/selections/{selection_id}/request",
            get(preview),
        )
        .with_state(operations)
}
fn input<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    value.map(|Json(value)| value).map_err(|error| match error.status() {
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::JsonRequired,
        _ => ApiError::BadRequest { code: "invalid_json", message: "Send only the documented selection capabilities; paths and keys are not accepted.".into() },
    })
}
fn error(error: OperationError) -> ApiError {
    match error {
        OperationError::InvalidSelection => ApiError::BadRequest {
            code: "invalid_storage_selection",
            message: "Choose the required item again using the local file selector.".into(),
        },
        OperationError::ConfirmationRequired => ApiError::Conflict {
            code: "recipient_confirmation_required",
            message: "Confirm the selected recipient request ID and fingerprint before exporting."
                .into(),
        },
        OperationError::Busy => ApiError::Conflict {
            code: "storage_busy",
            message: "Wait for the active storage operation to finish.".into(),
        },
        OperationError::Capacity => ApiError::Conflict {
            code: "storage_capacity",
            message: "Acknowledge completed operations in the local adapter before continuing."
                .into(),
        },
        OperationError::NotFound => ApiError::NotFound {
            code: "storage_operation_not_found",
            message: "This process does not have that storage operation.".into(),
        },
        OperationError::Internal => ApiError::Internal,
    }
}
macro_rules! command {
    ($name:ident, $input:ty) => {
        async fn $name(
            State(state): State<StorageOperations>,
            body: Result<Json<$input>, JsonRejection>,
        ) -> Result<(StatusCode, Json<OperationStatus>), ApiError> {
            Ok((
                StatusCode::ACCEPTED,
                Json(state.$name(input(body)?).map_err(error)?),
            ))
        }
    };
}
command!(request, RequestInput);
command!(export, ExportInput);
command!(import, ImportInput);
command!(restore, RestoreInput);
command!(recover, RecoveryInput);
async fn status(
    State(state): State<StorageOperations>,
    Path(id): Path<String>,
) -> Result<Json<OperationStatus>, ApiError> {
    Ok(Json(state.status(&id).map_err(error)?))
}
async fn preview(
    State(state): State<StorageOperations>,
    Path(id): Path<String>,
) -> Result<Json<RequestPreview>, ApiError> {
    Ok(Json(state.request_preview(&id).map_err(error)?))
}
