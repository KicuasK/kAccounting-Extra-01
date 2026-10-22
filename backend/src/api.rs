use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::accounting::{
    accounts::{Account, AccountError, AccountId, AccountNode, AccountRepository},
    journal::{
        DraftJournalEntry, DraftJournalLine, JournalDate, JournalEntryId, JournalError,
        JournalRepository, PostedJournalEntry,
    },
    reports::{
        AccountingEquationSummary, GeneralJournal, IncomeExpenseReport, PeriodGeneralLedger,
        ReportError, ReportRepository, TrialBalance,
    },
};

const SERVICE_NAME: &str = "kAccounting-Extra-01";

#[derive(Clone)]
struct ApiState {
    pool: SqlitePool,
}

/// Builds the current read-only API route tree without opening a network listener.
///
/// Authentication, bind address, and deployment are intentionally outside this
/// function and remain required before the router may be served.
pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/accounts", get(list_accounts))
        .route("/api/accounts/tree", get(account_tree))
        .route("/api/accounts/{account_id}", get(get_account))
        .route("/api/accounts/{account_id}/path", get(account_path))
        .route("/api/journal/drafts", get(list_drafts))
        .route("/api/journal/drafts/{entry_id}", get(get_draft))
        .route("/api/journal/posted/{entry_id}", get(get_posted))
        .route("/api/reports/trial-balance/{as_of}", get(trial_balance))
        .route(
            "/api/reports/accounting-equation/{as_of}",
            get(accounting_equation),
        )
        .route(
            "/api/reports/ledger/{account_id}/{from}/{through}",
            get(period_general_ledger),
        )
        .route(
            "/api/reports/general-journal/{from}/{through}",
            get(general_journal),
        )
        .route(
            "/api/reports/income-expense/{from}/{through}",
            get(income_expense),
        )
        .fallback(route_not_found)
        .layer(middleware::from_fn(private_response_headers))
        .with_state(ApiState { pool })
}

async fn route_not_found() -> ApiError {
    ApiError::NotFound {
        code: "route_not_found",
        message: "The requested API route was not found.".to_owned(),
    }
}

async fn private_response_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        let allow = response.headers().get(header::ALLOW).cloned();
        response = (
            StatusCode::METHOD_NOT_ALLOWED,
            Json(ApiErrorBody {
                code: "method_not_allowed",
                message: "The requested method is not allowed for this route.".to_owned(),
            }),
        )
            .into_response();
        if let Some(allow) = allow {
            response.headers_mut().insert(header::ALLOW, allow);
        }
    }
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

#[derive(Serialize)]
struct HealthResponse {
    service: &'static str,
    status: &'static str,
}

async fn health(State(state): State<ApiState>) -> Result<Json<HealthResponse>, ApiError> {
    let probe: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    if probe != 1 {
        return Err(ApiError::Internal);
    }
    Ok(Json(HealthResponse {
        service: SERVICE_NAME,
        status: "ok",
    }))
}

#[derive(Serialize)]
struct AccountResponse {
    id: String,
    parent_id: Option<String>,
    name: String,
    element: &'static str,
    kind: &'static str,
    is_active: bool,
    revision: String,
    created_at: String,
    updated_at: String,
}

impl From<Account> for AccountResponse {
    fn from(account: Account) -> Self {
        Self {
            id: account.id.get().to_string(),
            parent_id: account.parent_id.map(|id| id.get().to_string()),
            name: account.name,
            element: account.element.as_str(),
            kind: account.kind.as_str(),
            is_active: account.is_active,
            revision: account.revision.to_string(),
            created_at: account.created_at,
            updated_at: account.updated_at,
        }
    }
}

async fn list_accounts(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    let accounts = AccountRepository::new(state.pool)
        .list_all()
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(
        accounts.into_iter().map(AccountResponse::from).collect(),
    ))
}

async fn get_account(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
) -> Result<Json<AccountResponse>, ApiError> {
    let account_id = parse_api_id("account_id", &account_id)?;
    let account = AccountRepository::new(state.pool)
        .get(account_id)
        .await
        .map_err(ApiError::from_account)?;
    Ok(Json(account.into()))
}

#[derive(Serialize)]
struct AccountNodeResponse {
    account: AccountResponse,
    children: Vec<AccountNodeResponse>,
}

impl From<AccountNode> for AccountNodeResponse {
    fn from(node: AccountNode) -> Self {
        Self {
            account: node.account.into(),
            children: node
                .children
                .into_iter()
                .map(AccountNodeResponse::from)
                .collect(),
        }
    }
}

async fn account_tree(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AccountNodeResponse>>, ApiError> {
    let tree = AccountRepository::new(state.pool)
        .list_tree()
        .await
        .map_err(ApiError::from_account)?;
    Ok(Json(
        tree.into_iter().map(AccountNodeResponse::from).collect(),
    ))
}

async fn account_path(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    let account_id = parse_api_id("account_id", &account_id)?;
    let path = AccountRepository::new(state.pool)
        .path_to(account_id)
        .await
        .map_err(ApiError::from_account)?;
    Ok(Json(path.into_iter().map(AccountResponse::from).collect()))
}

#[derive(Serialize)]
struct DraftLineResponse {
    id: String,
    line_no: String,
    account_id: String,
    side: &'static str,
    amount_minor: String,
}

impl From<DraftJournalLine> for DraftLineResponse {
    fn from(line: DraftJournalLine) -> Self {
        Self {
            id: line.id.get().to_string(),
            line_no: line.line_no.to_string(),
            account_id: line.account_id.get().to_string(),
            side: line.side.as_str(),
            amount_minor: line.amount.get().to_string(),
        }
    }
}

#[derive(Serialize)]
struct DraftResponse {
    id: String,
    entry_date: String,
    revision: String,
    created_at: String,
    lines: Vec<DraftLineResponse>,
}

impl From<DraftJournalEntry> for DraftResponse {
    fn from(draft: DraftJournalEntry) -> Self {
        Self {
            id: draft.id.get().to_string(),
            entry_date: draft.entry_date.as_str().to_owned(),
            revision: draft.revision.to_string(),
            created_at: draft.created_at,
            lines: draft
                .lines
                .into_iter()
                .map(DraftLineResponse::from)
                .collect(),
        }
    }
}

async fn list_drafts(State(state): State<ApiState>) -> Result<Json<Vec<DraftResponse>>, ApiError> {
    let drafts = JournalRepository::new(state.pool)
        .list_drafts()
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(drafts.into_iter().map(DraftResponse::from).collect()))
}

async fn get_draft(
    State(state): State<ApiState>,
    Path(entry_id): Path<String>,
) -> Result<Json<DraftResponse>, ApiError> {
    let entry_id = parse_journal_entry_id(&entry_id)?;
    let draft = JournalRepository::new(state.pool)
        .get_draft(entry_id)
        .await
        .map_err(ApiError::from_journal)?;
    Ok(Json(draft.into()))
}

#[derive(Serialize)]
struct PostedLineResponse {
    id: String,
    line_no: String,
    account_id: String,
    side: &'static str,
    amount_minor: String,
}

#[derive(Serialize)]
struct PostedEntryResponse {
    id: String,
    entry_date: String,
    created_at: String,
    posted_at: String,
    lines: Vec<PostedLineResponse>,
}

impl From<PostedJournalEntry> for PostedEntryResponse {
    fn from(entry: PostedJournalEntry) -> Self {
        Self {
            id: entry.id.get().to_string(),
            entry_date: entry.entry_date.as_str().to_owned(),
            created_at: entry.created_at,
            posted_at: entry.posted_at,
            lines: entry
                .lines
                .into_iter()
                .map(|line| PostedLineResponse {
                    id: line.id.get().to_string(),
                    line_no: line.line_no.to_string(),
                    account_id: line.account_id.get().to_string(),
                    side: line.side.as_str(),
                    amount_minor: line.amount.get().to_string(),
                })
                .collect(),
        }
    }
}

async fn get_posted(
    State(state): State<ApiState>,
    Path(entry_id): Path<String>,
) -> Result<Json<PostedEntryResponse>, ApiError> {
    let entry_id = parse_journal_entry_id(&entry_id)?;
    let entry = JournalRepository::new(state.pool)
        .get_posted(entry_id)
        .await
        .map_err(ApiError::from_journal)?;
    Ok(Json(entry.into()))
}

#[derive(Serialize)]
struct TrialBalanceRowResponse {
    account_id: String,
    account_name: String,
    element: &'static str,
    debit_turnover_minor: String,
    credit_turnover_minor: String,
    debit_balance_minor: String,
    credit_balance_minor: String,
}

#[derive(Serialize)]
struct TrialBalanceResponse {
    as_of: String,
    rows: Vec<TrialBalanceRowResponse>,
    total_debit_minor: String,
    total_credit_minor: String,
}

impl From<TrialBalance> for TrialBalanceResponse {
    fn from(report: TrialBalance) -> Self {
        Self {
            as_of: report.as_of.as_str().to_owned(),
            rows: report
                .rows
                .into_iter()
                .map(|row| TrialBalanceRowResponse {
                    account_id: row.account_id.get().to_string(),
                    account_name: row.account_name,
                    element: row.element.as_str(),
                    debit_turnover_minor: row.debit_turnover.get().to_string(),
                    credit_turnover_minor: row.credit_turnover.get().to_string(),
                    debit_balance_minor: row.debit_balance.get().to_string(),
                    credit_balance_minor: row.credit_balance.get().to_string(),
                })
                .collect(),
            total_debit_minor: report.total_debit.get().to_string(),
            total_credit_minor: report.total_credit.get().to_string(),
        }
    }
}

async fn trial_balance(
    State(state): State<ApiState>,
    Path(as_of): Path<String>,
) -> Result<Json<TrialBalanceResponse>, ApiError> {
    let as_of = parse_api_date("as_of", &as_of)?;
    let report = ReportRepository::new(state.pool)
        .trial_balance(as_of)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(report.into()))
}

#[derive(Serialize)]
struct ElementBalanceResponse {
    element: &'static str,
    debit_balance_minor: String,
    credit_balance_minor: String,
}

#[derive(Serialize)]
struct AccountingEquationResponse {
    as_of: String,
    rows: Vec<ElementBalanceResponse>,
    total_debit_minor: String,
    total_credit_minor: String,
}

impl From<AccountingEquationSummary> for AccountingEquationResponse {
    fn from(summary: AccountingEquationSummary) -> Self {
        Self {
            as_of: summary.as_of.as_str().to_owned(),
            rows: summary
                .rows
                .into_iter()
                .map(|row| ElementBalanceResponse {
                    element: row.element.as_str(),
                    debit_balance_minor: row.debit_balance.get().to_string(),
                    credit_balance_minor: row.credit_balance.get().to_string(),
                })
                .collect(),
            total_debit_minor: summary.total_debit.get().to_string(),
            total_credit_minor: summary.total_credit.get().to_string(),
        }
    }
}

async fn accounting_equation(
    State(state): State<ApiState>,
    Path(as_of): Path<String>,
) -> Result<Json<AccountingEquationResponse>, ApiError> {
    let as_of = parse_api_date("as_of", &as_of)?;
    let summary = ReportRepository::new(state.pool)
        .accounting_equation_summary(as_of)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(summary.into()))
}

#[derive(Serialize)]
struct LedgerLineResponse {
    entry_id: String,
    line_id: String,
    entry_date: String,
    line_no: String,
    side: &'static str,
    amount_minor: String,
    debit_balance_minor: String,
    credit_balance_minor: String,
}

#[derive(Serialize)]
struct PeriodLedgerResponse {
    account_id: String,
    account_name: String,
    element: &'static str,
    from: String,
    through: String,
    opening_debit_balance_minor: String,
    opening_credit_balance_minor: String,
    lines: Vec<LedgerLineResponse>,
    period_debit_turnover_minor: String,
    period_credit_turnover_minor: String,
    closing_debit_balance_minor: String,
    closing_credit_balance_minor: String,
}

impl From<PeriodGeneralLedger> for PeriodLedgerResponse {
    fn from(report: PeriodGeneralLedger) -> Self {
        Self {
            account_id: report.account_id.get().to_string(),
            account_name: report.account_name,
            element: report.element.as_str(),
            from: report.from.as_str().to_owned(),
            through: report.through.as_str().to_owned(),
            opening_debit_balance_minor: report.opening_debit_balance.get().to_string(),
            opening_credit_balance_minor: report.opening_credit_balance.get().to_string(),
            lines: report
                .lines
                .into_iter()
                .map(|line| LedgerLineResponse {
                    entry_id: line.entry_id.get().to_string(),
                    line_id: line.line_id.get().to_string(),
                    entry_date: line.entry_date.as_str().to_owned(),
                    line_no: line.line_no.to_string(),
                    side: line.side.as_str(),
                    amount_minor: line.amount.get().to_string(),
                    debit_balance_minor: line.debit_balance.get().to_string(),
                    credit_balance_minor: line.credit_balance.get().to_string(),
                })
                .collect(),
            period_debit_turnover_minor: report.period_debit_turnover.get().to_string(),
            period_credit_turnover_minor: report.period_credit_turnover.get().to_string(),
            closing_debit_balance_minor: report.closing_debit_balance.get().to_string(),
            closing_credit_balance_minor: report.closing_credit_balance.get().to_string(),
        }
    }
}

async fn period_general_ledger(
    State(state): State<ApiState>,
    Path((account_id, from, through)): Path<(String, String, String)>,
) -> Result<Json<PeriodLedgerResponse>, ApiError> {
    let account_id = parse_api_id("account_id", &account_id)?;
    let from = parse_api_date("from", &from)?;
    let through = parse_api_date("through", &through)?;
    let report = ReportRepository::new(state.pool)
        .period_general_ledger(account_id, from, through)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(report.into()))
}

#[derive(Serialize)]
struct GeneralJournalLineResponse {
    id: String,
    line_no: String,
    account_id: String,
    account_name: String,
    side: &'static str,
    amount_minor: String,
}

#[derive(Serialize)]
struct GeneralJournalEntryResponse {
    id: String,
    entry_date: String,
    posted_at: String,
    lines: Vec<GeneralJournalLineResponse>,
    debit_total_minor: String,
    credit_total_minor: String,
}

#[derive(Serialize)]
struct GeneralJournalResponse {
    from: String,
    through: String,
    entries: Vec<GeneralJournalEntryResponse>,
    total_debit_minor: String,
    total_credit_minor: String,
}

impl From<GeneralJournal> for GeneralJournalResponse {
    fn from(report: GeneralJournal) -> Self {
        Self {
            from: report.from.as_str().to_owned(),
            through: report.through.as_str().to_owned(),
            entries: report
                .entries
                .into_iter()
                .map(|entry| GeneralJournalEntryResponse {
                    id: entry.id.get().to_string(),
                    entry_date: entry.entry_date.as_str().to_owned(),
                    posted_at: entry.posted_at,
                    lines: entry
                        .lines
                        .into_iter()
                        .map(|line| GeneralJournalLineResponse {
                            id: line.id.get().to_string(),
                            line_no: line.line_no.to_string(),
                            account_id: line.account_id.get().to_string(),
                            account_name: line.account_name,
                            side: line.side.as_str(),
                            amount_minor: line.amount.get().to_string(),
                        })
                        .collect(),
                    debit_total_minor: entry.debit_total.get().to_string(),
                    credit_total_minor: entry.credit_total.get().to_string(),
                })
                .collect(),
            total_debit_minor: report.total_debit.get().to_string(),
            total_credit_minor: report.total_credit.get().to_string(),
        }
    }
}

async fn general_journal(
    State(state): State<ApiState>,
    Path((from, through)): Path<(String, String)>,
) -> Result<Json<GeneralJournalResponse>, ApiError> {
    let from = parse_api_date("from", &from)?;
    let through = parse_api_date("through", &through)?;
    let report = ReportRepository::new(state.pool)
        .general_journal(from, through)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(report.into()))
}

#[derive(Serialize)]
struct IncomeExpenseRowResponse {
    account_id: String,
    account_name: String,
    element: &'static str,
    debit_turnover_minor: String,
    credit_turnover_minor: String,
    net_activity_minor: String,
}

#[derive(Serialize)]
struct IncomeExpenseResponse {
    from: String,
    through: String,
    rows: Vec<IncomeExpenseRowResponse>,
    total_income_minor: String,
    total_expense_minor: String,
    net_result_minor: String,
}

impl From<IncomeExpenseReport> for IncomeExpenseResponse {
    fn from(report: IncomeExpenseReport) -> Self {
        Self {
            from: report.from.as_str().to_owned(),
            through: report.through.as_str().to_owned(),
            rows: report
                .rows
                .into_iter()
                .map(|row| IncomeExpenseRowResponse {
                    account_id: row.account_id.get().to_string(),
                    account_name: row.account_name,
                    element: row.element.as_str(),
                    debit_turnover_minor: row.debit_turnover.get().to_string(),
                    credit_turnover_minor: row.credit_turnover.get().to_string(),
                    net_activity_minor: row.net_activity.get().to_string(),
                })
                .collect(),
            total_income_minor: report.total_income.get().to_string(),
            total_expense_minor: report.total_expense.get().to_string(),
            net_result_minor: report.net_result.get().to_string(),
        }
    }
}

async fn income_expense(
    State(state): State<ApiState>,
    Path((from, through)): Path<(String, String)>,
) -> Result<Json<IncomeExpenseResponse>, ApiError> {
    let from = parse_api_date("from", &from)?;
    let through = parse_api_date("through", &through)?;
    let report = ReportRepository::new(state.pool)
        .income_expense_report(from, through)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(report.into()))
}

fn parse_api_date(field: &'static str, value: &str) -> Result<JournalDate, ApiError> {
    JournalDate::parse(value).map_err(|_| ApiError::BadRequest {
        code: "invalid_date",
        message: format!("{field} must be an existing date in YYYY-MM-DD form"),
    })
}

fn parse_api_id(field: &'static str, value: &str) -> Result<AccountId, ApiError> {
    let id = value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| ApiError::BadRequest {
            code: "invalid_id",
            message: format!("{field} must be a positive integer"),
        })?;
    Ok(AccountId::new(id))
}

fn parse_journal_entry_id(value: &str) -> Result<JournalEntryId, ApiError> {
    let id = value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| ApiError::BadRequest {
            code: "invalid_id",
            message: "entry_id must be a positive integer".to_owned(),
        })?;
    Ok(JournalEntryId::new(id))
}

enum ApiError {
    BadRequest { code: &'static str, message: String },
    NotFound { code: &'static str, message: String },
    Internal,
}

impl ApiError {
    fn from_account(error: AccountError) -> Self {
        match error {
            AccountError::NotFound(id) => Self::NotFound {
                code: "account_not_found",
                message: format!("Account {id} was not found."),
            },
            _ => Self::Internal,
        }
    }

    fn from_journal(error: JournalError) -> Self {
        match error {
            JournalError::EntryNotFound(id) => Self::NotFound {
                code: "entry_not_found",
                message: format!("Journal entry {id} was not found."),
            },
            JournalError::EntryNotDraft(id) => Self::NotFound {
                code: "draft_not_found",
                message: format!("Editable draft {id} was not found."),
            },
            JournalError::EntryNotPosted(id) => Self::NotFound {
                code: "posted_entry_not_found",
                message: format!("Posted journal entry {id} was not found."),
            },
            _ => Self::Internal,
        }
    }

    fn from_report(error: ReportError) -> Self {
        match error {
            ReportError::AccountNotFound(id) => Self::NotFound {
                code: "account_not_found",
                message: format!("Account {id} was not found."),
            },
            ReportError::InvalidDateRange { from, through } => Self::BadRequest {
                code: "invalid_date_range",
                message: format!("Date range {from} through {through} is invalid."),
            },
            _ => Self::Internal,
        }
    }
}

#[derive(Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest { code, message } => (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorBody { code, message }),
            )
                .into_response(),
            Self::NotFound { code, message } => {
                (StatusCode::NOT_FOUND, Json(ApiErrorBody { code, message })).into_response()
            }
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorBody {
                    code: "internal_error",
                    message: "The request could not be completed.".to_owned(),
                }),
            )
                .into_response(),
        }
    }
}
