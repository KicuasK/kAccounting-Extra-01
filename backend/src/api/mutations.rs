//! Explicit write routes. Domain repositories remain authoritative for writes.

use super::{
    AccountResponse, ApiError, ApiState, DraftResponse, ExactInteger, PostedEntryResponse,
    parse_api_date, parse_api_id, parse_journal_entry_id,
};
use crate::accounting::{
    accounts::{AccountError, AccountKind, AccountRepository, AccountingElement, NewAccount},
    journal::{
        JournalError, JournalLineInput, JournalRepository, JournalSide, JournalTime,
        PostCorrection, PostEntry, SaveDraftEntry,
    },
    money::MinorUnits,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State, rejection::JsonRejection},
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{patch, post, put},
};
use serde::Deserialize;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/accounts", post(create_account))
        .route(
            "/api/accounts/{account_id}",
            patch(update_account).delete(delete_account),
        )
        .route("/api/journal/drafts", post(create_draft))
        .route(
            "/api/journal/drafts/{entry_id}",
            put(replace_draft).delete(delete_draft),
        )
        .route("/api/journal/drafts/{entry_id}/post", post(post_draft))
        .route("/api/journal/posted", post(post_entry))
        .route(
            "/api/journal/posted/{entry_id}/reversal",
            post(reverse_entry),
        )
        .route(
            "/api/journal/posted/{entry_id}/correction",
            post(correct_entry),
        )
}

pub(super) async fn require_same_origin(
    State(state): State<ApiState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let headers = request.headers();
    let unique_origin = headers.get_all(header::ORIGIN).iter().count() == 1;
    let unique_host = headers.get_all(header::HOST).iter().count() == 1;
    let trusted = state
        .write_origin
        .as_ref()
        .is_some_and(|origin| headers.get(header::ORIGIN) == Some(origin));
    let loopback_host = headers
        .get(header::HOST)
        .is_some_and(|host| host == "127.0.0.1:3000");
    let fetch_site = headers.get_all("sec-fetch-site");
    let fetch_site_valid = fetch_site.iter().count() <= 1
        && fetch_site
            .iter()
            .all(|site| site == "same-origin" || site == "none");
    let read_method = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let read_origin_valid =
        headers.get_all(header::ORIGIN).iter().count() == 0 || (unique_origin && trusted);
    if !unique_host
        || !loopback_host
        || !fetch_site_valid
        || (read_method && !read_origin_valid)
        || (!read_method && (!unique_origin || !trusted))
    {
        return ApiError::Forbidden.into_response();
    }
    next.run(request).await
}

fn payload<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    value.map(|Json(value)| value).map_err(|error| match error.status() {
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::JsonRequired,
        _ => ApiError::BadRequest { code: "invalid_json", message: "Send a JSON object with the documented fields; integers must be decimal strings.".to_owned() },
    })
}
fn positive_revision(value: ExactInteger) -> Result<i64, ApiError> {
    if value.0 < 1 {
        return Err(ApiError::BadRequest {
            code: "invalid_revision",
            message: "expected_revision must be a positive integer string.".to_owned(),
        });
    }
    Ok(value.0)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ElementInput {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}
impl From<ElementInput> for AccountingElement {
    fn from(value: ElementInput) -> Self {
        match value {
            ElementInput::Asset => Self::Asset,
            ElementInput::Liability => Self::Liability,
            ElementInput::Equity => Self::Equity,
            ElementInput::Income => Self::Income,
            ElementInput::Expense => Self::Expense,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindInput {
    Category,
    Posting,
}
impl From<KindInput> for AccountKind {
    fn from(value: KindInput) -> Self {
        match value {
            KindInput::Category => Self::Category,
            KindInput::Posting => Self::Posting,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAccountInput {
    parent_id: Option<ExactInteger>,
    name: String,
    element: ElementInput,
    kind: KindInput,
}
async fn create_account(
    State(state): State<ApiState>,
    input: Result<Json<CreateAccountInput>, JsonRejection>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
    let input = payload(input)?;
    let parent_id = input
        .parent_id
        .map(|id| parse_api_id("parent_id", &id.0.to_string()))
        .transpose()?;
    let account = AccountRepository::new(state.pool)
        .create(NewAccount::new(
            parent_id,
            input.name,
            input.element.into(),
            input.kind.into(),
        ))
        .await
        .map_err(account_error)?;
    Ok((StatusCode::CREATED, Json(account.into())))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RequiredParentId {
    Root(()),
    Account(ExactInteger),
}
impl RequiredParentId {
    fn into_domain(self) -> Result<Option<crate::accounting::accounts::AccountId>, ApiError> {
        match self {
            Self::Root(()) => Ok(None),
            Self::Account(id) => parse_api_id("parent_id", &id.0.to_string()).map(Some),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum AccountChange {
    Rename {
        expected_revision: ExactInteger,
        name: String,
    },
    Move {
        expected_revision: ExactInteger,
        parent_id: RequiredParentId,
    },
    SetActive {
        expected_revision: ExactInteger,
        is_active: bool,
    },
    ChangeKind {
        expected_revision: ExactInteger,
        kind: KindInput,
    },
    ChangeElement {
        expected_revision: ExactInteger,
        element: ElementInput,
    },
}
async fn update_account(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<AccountChange>, JsonRejection>,
) -> Result<Json<AccountResponse>, ApiError> {
    let id = parse_api_id("account_id", &id)?;
    let repo = AccountRepository::new(state.pool);
    let account = match payload(input)? {
        AccountChange::Rename {
            expected_revision,
            name,
        } => {
            repo.rename(id, positive_revision(expected_revision)?, &name)
                .await
        }
        AccountChange::Move {
            expected_revision,
            parent_id,
        } => {
            repo.move_to(
                id,
                positive_revision(expected_revision)?,
                parent_id.into_domain()?,
            )
            .await
        }
        AccountChange::SetActive {
            expected_revision,
            is_active,
        } => {
            repo.set_active(id, positive_revision(expected_revision)?, is_active)
                .await
        }
        AccountChange::ChangeKind {
            expected_revision,
            kind,
        } => {
            repo.change_kind(id, positive_revision(expected_revision)?, kind.into())
                .await
        }
        AccountChange::ChangeElement {
            expected_revision,
            element,
        } => {
            repo.change_element(id, positive_revision(expected_revision)?, element.into())
                .await
        }
    }
    .map_err(account_error)?;
    Ok(Json(account.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedRevision {
    expected_revision: ExactInteger,
}
async fn delete_account(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<ExpectedRevision>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = parse_api_id("account_id", &id)?;
    AccountRepository::new(state.pool)
        .delete(id, positive_revision(payload(input)?.expected_revision)?)
        .await
        .map_err(account_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SideInput {
    Debit,
    Credit,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineInput {
    account_id: ExactInteger,
    side: SideInput,
    amount_minor: ExactInteger,
    memo: Option<String>,
}
impl LineInput {
    fn into_domain(self) -> Result<JournalLineInput, ApiError> {
        Ok(JournalLineInput {
            account_id: parse_api_id("account_id", &self.account_id.0.to_string())?,
            side: match self.side {
                SideInput::Debit => JournalSide::Debit,
                SideInput::Credit => JournalSide::Credit,
            },
            amount: MinorUnits::new(self.amount_minor.0),
            memo: self.memo,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryInput {
    entry_date: String,
    entry_time: Option<String>,
    reference: Option<String>,
    explanation: Option<String>,
    lines: Vec<LineInput>,
}
impl EntryInput {
    fn into_draft(self) -> Result<SaveDraftEntry, ApiError> {
        Ok(SaveDraftEntry::new(
            parse_api_date("entry_date", &self.entry_date)?,
            self.lines
                .into_iter()
                .map(LineInput::into_domain)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .with_time(
            self.entry_time
                .as_deref()
                .map(JournalTime::parse)
                .transpose()
                .map_err(journal_error)?,
        )
        .with_header(self.reference, self.explanation))
    }
    fn into_post(self) -> Result<PostEntry, ApiError> {
        let draft = self.into_draft()?;
        Ok(PostEntry::new(draft.entry_date, draft.lines)
            .with_time(draft.entry_time)
            .with_header(draft.reference, draft.explanation))
    }
}
async fn create_draft(
    State(state): State<ApiState>,
    input: Result<Json<EntryInput>, JsonRejection>,
) -> Result<(StatusCode, Json<DraftResponse>), ApiError> {
    let draft = JournalRepository::new(state.pool)
        .create_draft(payload(input)?.into_draft()?)
        .await
        .map_err(journal_error)?;
    Ok((StatusCode::CREATED, Json(draft.into())))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceDraftInput {
    expected_revision: ExactInteger,
    entry: EntryInput,
}
async fn replace_draft(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<ReplaceDraftInput>, JsonRejection>,
) -> Result<Json<DraftResponse>, ApiError> {
    let id = parse_journal_entry_id(&id)?;
    let input = payload(input)?;
    let draft = JournalRepository::new(state.pool)
        .replace_draft(
            id,
            positive_revision(input.expected_revision)?,
            input.entry.into_draft()?,
        )
        .await
        .map_err(journal_error)?;
    Ok(Json(draft.into()))
}
async fn delete_draft(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<ExpectedRevision>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = parse_journal_entry_id(&id)?;
    JournalRepository::new(state.pool)
        .delete_draft(id, positive_revision(payload(input)?.expected_revision)?)
        .await
        .map_err(journal_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn post_draft(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<ExpectedRevision>, JsonRejection>,
) -> Result<Json<PostedEntryResponse>, ApiError> {
    let id = parse_journal_entry_id(&id)?;
    let entry = JournalRepository::new(state.pool)
        .post_draft(id, positive_revision(payload(input)?.expected_revision)?)
        .await
        .map_err(journal_error)?;
    Ok(Json(entry.into()))
}
async fn post_entry(
    State(state): State<ApiState>,
    input: Result<Json<EntryInput>, JsonRejection>,
) -> Result<(StatusCode, Json<PostedEntryResponse>), ApiError> {
    let entry = JournalRepository::new(state.pool)
        .post(payload(input)?.into_post()?)
        .await
        .map_err(journal_error)?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReversalInput {
    entry_date: String,
    entry_time: Option<String>,
}
async fn reverse_entry(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<ReversalInput>, JsonRejection>,
) -> Result<(StatusCode, Json<PostedEntryResponse>), ApiError> {
    let id = parse_journal_entry_id(&id)?;
    let input = payload(input)?;
    let entry = JournalRepository::new(state.pool)
        .reverse_posted_at(
            id,
            parse_api_date("entry_date", &input.entry_date)?,
            input
                .entry_time
                .as_deref()
                .map(JournalTime::parse)
                .transpose()
                .map_err(journal_error)?,
        )
        .await
        .map_err(journal_error)?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}
async fn correct_entry(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    input: Result<Json<EntryInput>, JsonRejection>,
) -> Result<(StatusCode, Json<PostedEntryResponse>), ApiError> {
    let id = parse_journal_entry_id(&id)?;
    let mut entry = payload(input)?.into_post()?;
    let explanation = entry.explanation.take().ok_or(ApiError::Unprocessable {
        code: "correction_explanation_required",
        message: "A correction requires an explanation.".to_owned(),
    })?;
    let entry = JournalRepository::new(state.pool)
        .post_correction(PostCorrection::new(id, entry, explanation))
        .await
        .map_err(journal_error)?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}

fn account_error(error: AccountError) -> ApiError {
    let message = error.to_string();
    match error {
        AccountError::NotFound(_) | AccountError::ParentNotFound => ApiError::NotFound {
            code: "account_not_found",
            message,
        },
        AccountError::InvalidName => ApiError::BadRequest {
            code: "invalid_account_name",
            message,
        },
        AccountError::DuplicateSiblingName => ApiError::Conflict {
            code: "duplicate_account_name",
            message,
        },
        AccountError::RevisionConflict { .. } | AccountError::RevisionExhausted(_) => {
            ApiError::Conflict {
                code: "account_revision_conflict",
                message,
            }
        }
        AccountError::Database(_)
        | AccountError::InvalidStoredValue { .. }
        | AccountError::CorruptTree(_) => ApiError::Internal,
        _ => ApiError::Unprocessable {
            code: "account_rule_violation",
            message,
        },
    }
}
fn journal_error(error: JournalError) -> ApiError {
    let message = error.to_string();
    match error {
        JournalError::OpenItemAllocationsActive => ApiError::Conflict {
            code: "open_item_allocations_active", message,
        },
        JournalError::EntryNotFound(_) => ApiError::NotFound {
            code: "entry_not_found",
            message,
        },
        JournalError::AccountNotFound(_) => ApiError::NotFound {
            code: "account_not_found",
            message,
        },
        JournalError::StaleDraftRevision { .. } | JournalError::DraftRevisionExhausted(_) => {
            ApiError::Conflict {
                code: "draft_revision_conflict",
                message,
            }
        }
        JournalError::EntryNotDraft(_) | JournalError::EntryAlreadyReversed(_) => {
            ApiError::Conflict {
                code: "journal_state_conflict",
                message,
            }
        }
        JournalError::InvalidDate(_)
        | JournalError::InvalidTime(_)
        | JournalError::InvalidMetadata
        | JournalError::InvalidExplanation => ApiError::BadRequest {
            code: "invalid_journal_input",
            message,
        },
        JournalError::Database(ref error) if error.as_database_error().is_some_and(|e|e.message().contains("payment memo draft no longer matches its payment plan")) => ApiError::Conflict {
            code: "payment_memo_conflict", message: "The payment draft no longer matches its plan. Reload and review the draft before posting.".into()
        },
        JournalError::Database(_) | JournalError::InvalidStoredValue(_) => ApiError::Internal,
        _ => ApiError::Unprocessable {
            code: "journal_rule_violation",
            message,
        },
    }
}
