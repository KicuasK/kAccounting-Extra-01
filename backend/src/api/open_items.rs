use super::{ApiError, ApiState, ExactInteger};
use crate::accounting::open_items::{
    AllocationKind, NewAllocation, NewOpenItem, OpenItemError, OpenItemRepository, TrackingKind,
};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
pub(super) fn read_routes() -> Router<ApiState> {
    Router::new()
        .route("/api/open-items", get(list))
        .route("/api/open-items/lines/{id}/capacity", get(capacity))
}
pub(super) fn write_routes() -> Router<ApiState> {
    Router::new()
        .route("/api/open-items", post(register))
        .route("/api/open-items/allocations", post(allocate))
        .route("/api/open-items/allocations/{id}/void", post(void))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemInput {
    amount_minor: ExactInteger,
    description: String,
    counterparty: Option<String>,
    due_date: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    source_line_id: ExactInteger,
    kind: String,
    items: Vec<ItemInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationInput {
    item_id: ExactInteger,
    amount_minor: ExactInteger,
    kind: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocate {
    settlement_line_id: ExactInteger,
    allocations: Vec<AllocationInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Void {
    reason: String,
}
#[derive(Serialize)]
struct Item {
    id: ExactInteger,
    source_line_id: ExactInteger,
    source_entry_id: ExactInteger,
    account_id: ExactInteger,
    kind: String,
    entry_date: String,
    amount_minor: ExactInteger,
    description: String,
    counterparty: Option<String>,
    due_date: Option<String>,
    paid_minor: ExactInteger,
    adjusted_minor: ExactInteger,
    outstanding_minor: ExactInteger,
    reversed: bool,
}
#[derive(Serialize)]
struct Allocation {
    id: ExactInteger,
    settlement_line_id: ExactInteger,
    settlement_entry_id: ExactInteger,
    entry_date: String,
    item_id: ExactInteger,
    amount_minor: ExactInteger,
    kind: String,
    created_at: String,
    void_reason: Option<String>,
    voided_at: Option<String>,
}
#[derive(Serialize)]
struct List {
    items: Vec<Item>,
    allocations: Vec<Allocation>,
    coverage: &'static str,
}
#[derive(Serialize)]
struct ItemIds {
    item_ids: Vec<ExactInteger>,
}
#[derive(Serialize)]
struct AllocationIds {
    allocation_ids: Vec<ExactInteger>,
}
#[derive(Serialize)]
struct Voided {
    voided: bool,
}
#[derive(Serialize)]
struct Capacity {
    remaining_minor: ExactInteger,
}
fn error(e: OpenItemError) -> ApiError {
    let message = e.to_string();
    match e {
        OpenItemError::InvalidInput => ApiError::BadRequest {
            code: "open_item_invalid",
            message,
        },
        OpenItemError::LineNotFound | OpenItemError::ItemNotFound => ApiError::NotFound {
            code: "open_item_not_found",
            message,
        },
        OpenItemError::Overflow => ApiError::Unprocessable {
            code: "open_item_overflow",
            message,
        },
        OpenItemError::Database(_) | OpenItemError::InvalidStored => ApiError::Internal,
        _ => ApiError::Conflict {
            code: match e {
                OpenItemError::ReversedLine => "open_item_reversed",
                OpenItemError::WrongAccount => "open_item_wrong_account",
                OpenItemError::AlreadyRegistered => "open_item_exists",
                OpenItemError::SplitMismatch => "open_item_split_mismatch",
                OpenItemError::Overallocated => "open_item_overallocated",
                OpenItemError::DateOrder => "open_item_date_order",
                OpenItemError::AllocationNotActive => "open_item_allocation_inactive",
                _ => unreachable!(),
            },
            message,
        },
    }
}
fn payload<T>(v: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    v.map(|Json(v)| v)
        .map_err(|_| error(OpenItemError::InvalidInput))
}
fn id(s: &str) -> Result<i64, ApiError> {
    s.parse::<i64>()
        .ok()
        .filter(|i| *i > 0 && i.to_string() == s)
        .ok_or_else(|| error(OpenItemError::InvalidInput))
}
async fn list(State(s): State<ApiState>) -> Result<Json<List>, ApiError> {
    let r = OpenItemRepository::new(s.pool);
    let (items, allocations) = r.snapshot().await.map_err(error)?;
    let items = items
        .into_iter()
        .map(|i| Item {
            id: i.id.into(),
            source_line_id: i.source_line_id.into(),
            source_entry_id: i.source_entry_id.into(),
            account_id: i.account_id.into(),
            kind: i.kind,
            entry_date: i.entry_date,
            amount_minor: i.amount_minor.into(),
            description: i.description,
            counterparty: i.counterparty,
            due_date: i.due_date,
            paid_minor: i.paid_minor.into(),
            adjusted_minor: i.adjusted_minor.into(),
            outstanding_minor: i.outstanding_minor.into(),
            reversed: i.reversed,
        })
        .collect();
    let allocations = allocations
        .into_iter()
        .map(|a| Allocation {
            id: a.id.into(),
            settlement_line_id: a.settlement_line_id.into(),
            settlement_entry_id: a.settlement_entry_id.into(),
            entry_date: a.entry_date,
            item_id: a.item_id.into(),
            amount_minor: a.amount_minor.into(),
            kind: a.kind,
            created_at: a.created_at,
            void_reason: a.void_reason,
            voided_at: a.voided_at,
        })
        .collect();
    Ok(Json(List {
        items,
        allocations,
        coverage: "explicit-lines-only",
    }))
}
async fn register(
    State(s): State<ApiState>,
    v: Result<Json<Register>, JsonRejection>,
) -> Result<(StatusCode, Json<ItemIds>), ApiError> {
    let v = payload(v)?;
    let kind = match v.kind.as_str() {
        "receivable" => TrackingKind::Receivable,
        "payable" => TrackingKind::Payable,
        _ => return Err(error(OpenItemError::InvalidInput)),
    };
    let ids = OpenItemRepository::new(s.pool)
        .register(
            v.source_line_id.0,
            kind,
            v.items
                .into_iter()
                .map(|i| NewOpenItem {
                    amount_minor: i.amount_minor.0,
                    description: i.description,
                    counterparty: i.counterparty,
                    due_date: i.due_date,
                })
                .collect(),
        )
        .await
        .map_err(error)?;
    Ok((
        StatusCode::CREATED,
        Json(ItemIds {
            item_ids: ids.into_iter().map(Into::into).collect(),
        }),
    ))
}
async fn allocate(
    State(s): State<ApiState>,
    v: Result<Json<Allocate>, JsonRejection>,
) -> Result<(StatusCode, Json<AllocationIds>), ApiError> {
    let v = payload(v)?;
    let allocations = v
        .allocations
        .into_iter()
        .map(|a| {
            Ok(NewAllocation {
                item_id: a.item_id.0,
                amount_minor: a.amount_minor.0,
                kind: match a.kind.as_str() {
                    "payment" => AllocationKind::Payment,
                    "adjustment" => AllocationKind::Adjustment,
                    _ => return Err(error(OpenItemError::InvalidInput)),
                },
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let ids = OpenItemRepository::new(s.pool)
        .allocate(v.settlement_line_id.0, allocations)
        .await
        .map_err(error)?;
    Ok((
        StatusCode::CREATED,
        Json(AllocationIds {
            allocation_ids: ids.into_iter().map(Into::into).collect(),
        }),
    ))
}
async fn void(
    State(s): State<ApiState>,
    Path(raw): Path<String>,
    v: Result<Json<Void>, JsonRejection>,
) -> Result<Json<Voided>, ApiError> {
    let v = payload(v)?;
    OpenItemRepository::new(s.pool)
        .void_allocation(id(&raw)?, &v.reason)
        .await
        .map_err(error)?;
    Ok(Json(Voided { voided: true }))
}
async fn capacity(
    State(s): State<ApiState>,
    Path(raw): Path<String>,
) -> Result<Json<Capacity>, ApiError> {
    let n = OpenItemRepository::new(s.pool)
        .line_capacity(id(&raw)?)
        .await
        .map_err(error)?;
    Ok(Json(Capacity {
        remaining_minor: n.into(),
    }))
}
