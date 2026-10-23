use super::{ApiError, ApiState, ExactInteger};
use crate::accounting::payment_memos::{MemoError, MemoInput, PaymentMemo, PaymentMemoRepository};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
pub(super) fn read_routes() -> Router<ApiState> {
    Router::new().route("/api/payment-memos", get(list))
}
pub(super) fn write_routes() -> Router<ApiState> {
    Router::new()
        .route("/api/payment-memos", post(create))
        .route("/api/payment-memos/{id}", put(update))
        .route("/api/payment-memos/{id}/cancel", post(cancel))
        .route("/api/payment-memos/{id}/reopen", post(reopen))
        .route("/api/payment-memos/{id}/draft", post(create_draft))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    from: String,
    through: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    due_date: String,
    title: String,
    payee: String,
    note: String,
    amount_minor: ExactInteger,
    necessity: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    due_date: String,
    title: String,
    payee: String,
    note: String,
    amount_minor: ExactInteger,
    necessity: String,
    expected_revision: ExactInteger,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: ExactInteger,
}
#[derive(Serialize)]
struct Item {
    id: ExactInteger,
    revision: ExactInteger,
    due_date: String,
    title: String,
    payee: String,
    note: String,
    amount_minor: ExactInteger,
    necessity: String,
    status: String,
    created_at: String,
    updated_at: String,
    paid_amount_minor: ExactInteger,
    remaining_minor: ExactInteger,
    payment_draft_id: Option<ExactInteger>,
}
impl From<PaymentMemo> for Item {
    fn from(m: PaymentMemo) -> Self {
        Self {
            id: m.id.into(),
            revision: m.revision.into(),
            due_date: m.due_date,
            title: m.title,
            payee: m.payee,
            note: m.note,
            amount_minor: m.amount_minor.into(),
            necessity: m.necessity,
            status: m.status,
            created_at: m.created_at,
            updated_at: m.updated_at,
            paid_amount_minor: m.paid_amount_minor.into(),
            remaining_minor: (m.amount_minor - m.paid_amount_minor).into(),
            payment_draft_id: m.payment_draft_id.map(Into::into),
        }
    }
}
#[derive(Serialize)]
struct List {
    items: Vec<Item>,
    required_total_minor: ExactInteger,
}
fn error(e: MemoError) -> ApiError {
    match e {
        MemoError::Invalid(_) => ApiError::BadRequest {
            code: "payment_memo_invalid",
            message: e.to_string(),
        },
        MemoError::NotFound => ApiError::NotFound {
            code: "payment_memo_not_found",
            message: e.to_string(),
        },
        MemoError::Conflict => ApiError::Conflict {
            code: "payment_memo_conflict",
            message: e.to_string(),
        },
        MemoError::Overflow => ApiError::Unprocessable {
            code: "payment_memo_overflow",
            message: e.to_string(),
        },
        MemoError::Database(_) => ApiError::Internal,
    }
}
fn payload<T>(v: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    v.map(|Json(v)| v).map_err(|_| ApiError::BadRequest {
        code: "payment_memo_invalid",
        message: "Send exactly the documented JSON fields with canonical integer strings.".into(),
    })
}
fn identity(id: String, revision: i64) -> Result<i64, ApiError> {
    let parsed = id
        .parse::<i64>()
        .ok()
        .filter(|i| *i > 0 && i.to_string() == id);
    if revision < 1 || parsed.is_none() {
        return Err(error(MemoError::Invalid("identity")));
    }
    parsed.ok_or_else(|| error(MemoError::Invalid("identity")))
}
async fn list(
    State(s): State<ApiState>,
    range: Result<Query<Range>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<List>, ApiError> {
    let Query(r) = range.map_err(|_| error(MemoError::Invalid("range")))?;
    let (items, total) = PaymentMemoRepository::new(s.pool)
        .list(&r.from, &r.through)
        .await
        .map_err(error)?;
    Ok(Json(List {
        items: items.into_iter().map(Into::into).collect(),
        required_total_minor: total.into(),
    }))
}
async fn create(
    State(s): State<ApiState>,
    v: Result<Json<Input>, JsonRejection>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    let i = payload(v)?;
    let item = PaymentMemoRepository::new(s.pool)
        .create(MemoInput {
            due_date: i.due_date,
            title: i.title,
            payee: i.payee,
            note: i.note,
            amount_minor: i.amount_minor.0,
            necessity: i.necessity,
        })
        .await
        .map_err(error)?;
    Ok((StatusCode::CREATED, Json(item.into())))
}
async fn update(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    v: Result<Json<Update>, JsonRejection>,
) -> Result<Json<Item>, ApiError> {
    let i = payload(v)?;
    let id = identity(id, i.expected_revision.0)?;
    Ok(Json(
        PaymentMemoRepository::new(s.pool)
            .update(
                id,
                i.expected_revision.0,
                MemoInput {
                    due_date: i.due_date,
                    title: i.title,
                    payee: i.payee,
                    note: i.note,
                    amount_minor: i.amount_minor.0,
                    necessity: i.necessity,
                },
            )
            .await
            .map_err(error)?
            .into(),
    ))
}
async fn transition(
    s: ApiState,
    id: String,
    v: Result<Json<Revision>, JsonRejection>,
    cancel: bool,
) -> Result<Json<Item>, ApiError> {
    let i = payload(v)?;
    let id = identity(id, i.expected_revision.0)?;
    Ok(Json(
        PaymentMemoRepository::new(s.pool)
            .transition(id, i.expected_revision.0, cancel)
            .await
            .map_err(error)?
            .into(),
    ))
}
async fn cancel(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    v: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Item>, ApiError> {
    transition(s, id, v, true).await
}
async fn reopen(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    v: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Item>, ApiError> {
    transition(s, id, v, false).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftInput {
    expected_revision: ExactInteger,
    entry_date: String,
    debit_account_id: ExactInteger,
    payment_asset_id: ExactInteger,
    amount_minor: ExactInteger,
}
#[derive(Serialize)]
struct DraftOutput {
    entry_id: ExactInteger,
    entry_revision: ExactInteger,
    memo_id: ExactInteger,
    amount_minor: ExactInteger,
}
async fn create_draft(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    v: Result<Json<DraftInput>, JsonRejection>,
) -> Result<(StatusCode, Json<DraftOutput>), ApiError> {
    let i = payload(v)?;
    let id = identity(id, i.expected_revision.0)?;
    let entry = crate::accounting::payment_memo_drafts::create_payment_draft(
        &s.pool,
        id,
        i.expected_revision.0,
        &i.entry_date,
        i.debit_account_id.0,
        i.payment_asset_id.0,
        i.amount_minor.0,
    )
    .await
    .map_err(error)?;
    Ok((
        StatusCode::CREATED,
        Json(DraftOutput {
            entry_id: entry.into(),
            entry_revision: 1.into(),
            memo_id: id.into(),
            amount_minor: i.amount_minor.0.into(),
        }),
    ))
}
