use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod book_commands;
mod mutations;
use sqlx::SqlitePool;

use crate::accounting::{
    accounts::{Account, AccountError, AccountId, AccountNode, AccountRepository},
    journal::{
        DraftJournalEntry, DraftJournalLine, JournalDate, JournalEntryId, JournalError,
        JournalRepository, PostedEntryRelationship, PostedJournalEntry,
    },
    reports::{
        AccountingEquationSummary, BalanceSheet, GeneralJournal, IncomeExpenseReport,
        PeriodGeneralLedger, ReportError, ReportRepository, TrialBalance,
    },
};

const SERVICE_NAME: &str = "kAccounting-Extra-01";

/// An exact SQLite integer at the JSON boundary.
///
/// Keeping this distinct from presentation strings makes it impossible for a
/// response field declared as an integer to silently serialize as a JSON
/// number, which JavaScript cannot represent exactly across the full i64
/// range.
struct ExactInteger(i64);

impl<'de> Deserialize<'de> for ExactInteger {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let value = text.parse::<i64>().map_err(serde::de::Error::custom)?;
        if value.to_string() != text {
            return Err(serde::de::Error::custom(
                "integer must be a canonical decimal string",
            ));
        }
        Ok(Self(value))
    }
}

impl From<i64> for ExactInteger {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl Serialize for ExactInteger {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

#[derive(Clone)]
struct ApiState {
    pool: SqlitePool,
    write_origin: Option<HeaderValue>,
}

/// Builds the current read-only API route tree without opening a network listener.
///
/// Authentication, bind address, and deployment are intentionally outside this
/// function and remain required before the router may be served.
pub fn router(pool: SqlitePool) -> Router {
    read_routes()
        .layer(middleware::from_fn(private_response_headers))
        .with_state(ApiState {
            pool,
            write_origin: None,
        })
}

fn read_routes() -> Router<ApiState> {
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
        .route("/api/reports/balance-sheet/{as_of}", get(balance_sheet))
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
}

/// Builds the explicitly enabled localhost write API without opening a listener.
/// Writes require the loopback Host and exact Origin. Remote serving requires
/// its own reviewed origin/identity policy rather than trusting forwarded headers.
pub fn localhost_write_router(pool: SqlitePool) -> Router {
    read_routes()
        .merge(mutations::routes())
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            ApiState {
                pool: pool.clone(),
                write_origin: Some(HeaderValue::from_static("http://127.0.0.1:3000")),
            },
            mutations::require_same_origin,
        ))
        .layer(middleware::from_fn(private_response_headers))
        .with_state(ApiState {
            pool,
            write_origin: Some(HeaderValue::from_static("http://127.0.0.1:3000")),
        })
}

/// Adds book-owned encrypted attachment commands to the loopback application.
/// It receives an already configured repository; HTTP never accepts filesystem paths.
pub fn localhost_attachment_router(
    pool: SqlitePool,
    repository: crate::attachments::AttachmentRepository,
) -> Router {
    protect_local_book_commands(pool, book_commands::attachment_routes(repository))
}

/// All commands use the process-held book key; requests never access Keychain.
pub fn localhost_book_commands_router(
    book: &crate::runtime::RuntimeBook,
) -> Result<Router, crate::attachments::AttachmentError> {
    let repository = crate::attachments::AttachmentRepository::open(
        book.pool().clone(),
        book.attachments(),
        book.key(),
    )?;
    Ok(protect_local_book_commands(
        book.pool().clone(),
        book_commands::attachment_routes(repository).merge(book_commands::backup_routes(book)),
    ))
}

fn protect_local_book_commands(pool: SqlitePool, routes: Router) -> Router {
    routes
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            ApiState {
                pool,
                write_origin: Some(HeaderValue::from_static("http://127.0.0.1:3000")),
            },
            mutations::require_same_origin,
        ))
        .layer(middleware::from_fn(private_response_headers))
}

async fn route_not_found(request: Request<Body>) -> ApiError {
    if !is_valid_percent_encoded_utf8_path(request.uri().path()) {
        return ApiError::invalid_path();
    }

    ApiError::NotFound {
        code: "route_not_found",
        message: "The requested API route was not found.".to_owned(),
    }
}

/// Validates the single percent-decoding pass used for HTTP path matching.
///
/// Axum rejects invalid UTF-8 when a dynamic route matches, but an invalid
/// segment in an otherwise unknown path reaches the fallback unchanged. The
/// fallback must classify both cases consistently without treating encoded
/// separators as a second routing pass.
fn is_valid_percent_encoded_utf8_path(path: &str) -> bool {
    let encoded = path.as_bytes();
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;

    while index < encoded.len() {
        if encoded[index] != b'%' {
            decoded.push(encoded[index]);
            index += 1;
            continue;
        }

        if let (Some(high), Some(low)) = (
            encoded.get(index + 1).and_then(|byte| hex_value(*byte)),
            encoded.get(index + 2).and_then(|byte| hex_value(*byte)),
        ) {
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            // Match the router's percent-decoder: a percent sign without two
            // hexadecimal digits remains a literal byte.
            decoded.push(b'%');
            index += 1;
        }
    }

    std::str::from_utf8(&decoded).is_ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn private_response_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    // The current router has no body or query extractors that can produce a
    // framework 400. Its only non-JSON framework 400 is an invalid UTF-8 path.
    let framework_bad_request = response.status() == StatusCode::BAD_REQUEST
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(';').next() == Some("application/json"));
    if framework_bad_request {
        response = ApiError::invalid_path().into_response();
    } else if response.status() == StatusCode::METHOD_NOT_ALLOWED {
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
    id: ExactInteger,
    parent_id: Option<ExactInteger>,
    name: String,
    element: &'static str,
    kind: &'static str,
    is_active: bool,
    revision: ExactInteger,
    created_at: String,
    updated_at: String,
}

impl From<Account> for AccountResponse {
    fn from(account: Account) -> Self {
        Self {
            id: account.id.get().into(),
            parent_id: account.parent_id.map(|id| id.get().into()),
            name: account.name,
            element: account.element.as_str(),
            kind: account.kind.as_str(),
            is_active: account.is_active,
            revision: account.revision.into(),
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

async fn account_tree(State(state): State<ApiState>) -> Result<Response, ApiError> {
    let tree = AccountRepository::new(state.pool)
        .list_tree()
        .await
        .map_err(ApiError::from_account)?;
    let bytes = serialize_account_tree(tree)?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

enum AccountTreeToken {
    Node(AccountNode),
    Bytes(&'static [u8]),
}

/// Encodes the recursive account-tree schema with an explicit heap stack.
/// Valid account hierarchies have no product depth limit, so neither response
/// conversion nor JSON serialization may depend on the process call stack.
fn serialize_account_tree(tree: Vec<AccountNode>) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::new();
    bytes.push(b'[');

    let mut pending = vec![AccountTreeToken::Bytes(b"]")];
    push_account_nodes(&mut pending, tree);
    while let Some(token) = pending.pop() {
        match token {
            AccountTreeToken::Bytes(fragment) => bytes.extend_from_slice(fragment),
            AccountTreeToken::Node(node) => {
                bytes.extend_from_slice(b"{\"account\":");
                serde_json::to_writer(&mut bytes, &AccountResponse::from(node.account))
                    .map_err(|_| ApiError::Internal)?;
                bytes.extend_from_slice(b",\"children\":[");
                pending.push(AccountTreeToken::Bytes(b"]}"));
                push_account_nodes(&mut pending, node.children);
            }
        }
    }

    Ok(bytes)
}

fn push_account_nodes(pending: &mut Vec<AccountTreeToken>, nodes: Vec<AccountNode>) {
    for (index, node) in nodes.into_iter().enumerate().rev() {
        pending.push(AccountTreeToken::Node(node));
        if index > 0 {
            pending.push(AccountTreeToken::Bytes(b","));
        }
    }
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
    id: ExactInteger,
    line_no: ExactInteger,
    account_id: ExactInteger,
    side: &'static str,
    amount_minor: ExactInteger,
    memo: Option<String>,
}

impl From<DraftJournalLine> for DraftLineResponse {
    fn from(line: DraftJournalLine) -> Self {
        Self {
            id: line.id.get().into(),
            line_no: line.line_no.into(),
            account_id: line.account_id.get().into(),
            side: line.side.as_str(),
            amount_minor: line.amount.get().into(),
            memo: line.memo,
        }
    }
}

#[derive(Serialize)]
struct DraftResponse {
    id: ExactInteger,
    entry_date: String,
    revision: ExactInteger,
    created_at: String,
    lines: Vec<DraftLineResponse>,
    reference: Option<String>,
    explanation: Option<String>,
}

impl From<DraftJournalEntry> for DraftResponse {
    fn from(draft: DraftJournalEntry) -> Self {
        Self {
            id: draft.id.get().into(),
            entry_date: draft.entry_date.as_str().to_owned(),
            revision: draft.revision.into(),
            created_at: draft.created_at,
            reference: draft.reference,
            explanation: draft.explanation,
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
    id: ExactInteger,
    line_no: ExactInteger,
    account_id: ExactInteger,
    side: &'static str,
    amount_minor: ExactInteger,
    memo: Option<String>,
}

#[derive(Serialize)]
struct PostedEntryResponse {
    id: ExactInteger,
    entry_date: String,
    created_at: String,
    posted_at: String,
    lines: Vec<PostedLineResponse>,
    reference: Option<String>,
    explanation: Option<String>,
    relationship: &'static str,
    original_entry_id: Option<ExactInteger>,
}

impl From<PostedJournalEntry> for PostedEntryResponse {
    fn from(entry: PostedJournalEntry) -> Self {
        let (relationship, original_entry_id) = match entry.relationship {
            PostedEntryRelationship::Ordinary => ("ordinary", None),
            PostedEntryRelationship::Reversal { original_entry_id } => {
                ("reversal", Some(original_entry_id.get().into()))
            }
            PostedEntryRelationship::Correction { original_entry_id } => {
                ("correction", Some(original_entry_id.get().into()))
            }
        };
        Self {
            relationship,
            original_entry_id,
            id: entry.id.get().into(),
            entry_date: entry.entry_date.as_str().to_owned(),
            created_at: entry.created_at,
            posted_at: entry.posted_at,
            reference: entry.reference,
            explanation: entry.explanation,
            lines: entry
                .lines
                .into_iter()
                .map(|line| PostedLineResponse {
                    id: line.id.get().into(),
                    line_no: line.line_no.into(),
                    account_id: line.account_id.get().into(),
                    side: line.side.as_str(),
                    amount_minor: line.amount.get().into(),
                    memo: line.memo,
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
    account_id: ExactInteger,
    account_name: String,
    element: &'static str,
    debit_turnover_minor: ExactInteger,
    credit_turnover_minor: ExactInteger,
    debit_balance_minor: ExactInteger,
    credit_balance_minor: ExactInteger,
}

#[derive(Serialize)]
struct TrialBalanceResponse {
    as_of: String,
    rows: Vec<TrialBalanceRowResponse>,
    total_debit_minor: ExactInteger,
    total_credit_minor: ExactInteger,
}

impl From<TrialBalance> for TrialBalanceResponse {
    fn from(report: TrialBalance) -> Self {
        Self {
            as_of: report.as_of.as_str().to_owned(),
            rows: report
                .rows
                .into_iter()
                .map(|row| TrialBalanceRowResponse {
                    account_id: row.account_id.get().into(),
                    account_name: row.account_name,
                    element: row.element.as_str(),
                    debit_turnover_minor: row.debit_turnover.get().into(),
                    credit_turnover_minor: row.credit_turnover.get().into(),
                    debit_balance_minor: row.debit_balance.get().into(),
                    credit_balance_minor: row.credit_balance.get().into(),
                })
                .collect(),
            total_debit_minor: report.total_debit.get().into(),
            total_credit_minor: report.total_credit.get().into(),
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
struct BalanceSheetRowResponse {
    account_id: ExactInteger,
    account_name: String,
    element: &'static str,
    balance_minor: ExactInteger,
}

#[derive(Serialize)]
struct BalanceSheetResponse {
    as_of: String,
    hierarchy: Vec<AccountResponse>,
    rows: Vec<BalanceSheetRowResponse>,
    total_assets_minor: ExactInteger,
    total_liabilities_minor: ExactInteger,
    total_equity_minor: ExactInteger,
    unclosed_net_result_minor: ExactInteger,
    total_liabilities_and_equity_minor: ExactInteger,
}
impl From<BalanceSheet> for BalanceSheetResponse {
    fn from(report: BalanceSheet) -> Self {
        Self {
            as_of: report.as_of.as_str().to_owned(),
            hierarchy: report
                .hierarchy
                .into_iter()
                .map(AccountResponse::from)
                .collect(),
            rows: report
                .rows
                .into_iter()
                .map(|row| BalanceSheetRowResponse {
                    account_id: row.account_id.get().into(),
                    account_name: row.account_name,
                    element: row.element.as_str(),
                    balance_minor: row.balance.get().into(),
                })
                .collect(),
            total_assets_minor: report.total_assets.get().into(),
            total_liabilities_minor: report.total_liabilities.get().into(),
            total_equity_minor: report.total_equity.get().into(),
            unclosed_net_result_minor: report.unclosed_net_result.get().into(),
            total_liabilities_and_equity_minor: report.total_liabilities_and_equity.get().into(),
        }
    }
}
async fn balance_sheet(
    State(state): State<ApiState>,
    Path(as_of): Path<String>,
) -> Result<Json<BalanceSheetResponse>, ApiError> {
    let report = ReportRepository::new(state.pool)
        .balance_sheet(parse_api_date("as_of", &as_of)?)
        .await
        .map_err(ApiError::from_report)?;
    Ok(Json(report.into()))
}

#[derive(Serialize)]
struct ElementBalanceResponse {
    element: &'static str,
    debit_balance_minor: ExactInteger,
    credit_balance_minor: ExactInteger,
}

#[derive(Serialize)]
struct AccountingEquationResponse {
    as_of: String,
    rows: Vec<ElementBalanceResponse>,
    total_debit_minor: ExactInteger,
    total_credit_minor: ExactInteger,
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
                    debit_balance_minor: row.debit_balance.get().into(),
                    credit_balance_minor: row.credit_balance.get().into(),
                })
                .collect(),
            total_debit_minor: summary.total_debit.get().into(),
            total_credit_minor: summary.total_credit.get().into(),
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
    entry_id: ExactInteger,
    line_id: ExactInteger,
    entry_date: String,
    line_no: ExactInteger,
    side: &'static str,
    amount_minor: ExactInteger,
    debit_balance_minor: ExactInteger,
    credit_balance_minor: ExactInteger,
    memo: Option<String>,
    reference: Option<String>,
    explanation: Option<String>,
}

#[derive(Serialize)]
struct PeriodLedgerResponse {
    account_id: ExactInteger,
    account_name: String,
    element: &'static str,
    from: String,
    through: String,
    opening_debit_balance_minor: ExactInteger,
    opening_credit_balance_minor: ExactInteger,
    lines: Vec<LedgerLineResponse>,
    period_debit_turnover_minor: ExactInteger,
    period_credit_turnover_minor: ExactInteger,
    closing_debit_balance_minor: ExactInteger,
    closing_credit_balance_minor: ExactInteger,
}

impl From<PeriodGeneralLedger> for PeriodLedgerResponse {
    fn from(report: PeriodGeneralLedger) -> Self {
        Self {
            account_id: report.account_id.get().into(),
            account_name: report.account_name,
            element: report.element.as_str(),
            from: report.from.as_str().to_owned(),
            through: report.through.as_str().to_owned(),
            opening_debit_balance_minor: report.opening_debit_balance.get().into(),
            opening_credit_balance_minor: report.opening_credit_balance.get().into(),
            lines: report
                .lines
                .into_iter()
                .map(|line| LedgerLineResponse {
                    entry_id: line.entry_id.get().into(),
                    line_id: line.line_id.get().into(),
                    entry_date: line.entry_date.as_str().to_owned(),
                    line_no: line.line_no.into(),
                    side: line.side.as_str(),
                    amount_minor: line.amount.get().into(),
                    memo: line.memo,
                    debit_balance_minor: line.debit_balance.get().into(),
                    credit_balance_minor: line.credit_balance.get().into(),
                    reference: line.reference,
                    explanation: line.explanation,
                })
                .collect(),
            period_debit_turnover_minor: report.period_debit_turnover.get().into(),
            period_credit_turnover_minor: report.period_credit_turnover.get().into(),
            closing_debit_balance_minor: report.closing_debit_balance.get().into(),
            closing_credit_balance_minor: report.closing_credit_balance.get().into(),
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
    id: ExactInteger,
    line_no: ExactInteger,
    account_id: ExactInteger,
    account_name: String,
    side: &'static str,
    amount_minor: ExactInteger,
    memo: Option<String>,
}

#[derive(Serialize)]
struct GeneralJournalEntryResponse {
    id: ExactInteger,
    entry_date: String,
    posted_at: String,
    lines: Vec<GeneralJournalLineResponse>,
    debit_total_minor: ExactInteger,
    credit_total_minor: ExactInteger,
    reference: Option<String>,
    explanation: Option<String>,
}

#[derive(Serialize)]
struct GeneralJournalResponse {
    from: String,
    through: String,
    entries: Vec<GeneralJournalEntryResponse>,
    total_debit_minor: ExactInteger,
    total_credit_minor: ExactInteger,
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
                    id: entry.id.get().into(),
                    entry_date: entry.entry_date.as_str().to_owned(),
                    posted_at: entry.posted_at,
                    reference: entry.reference,
                    explanation: entry.explanation,
                    lines: entry
                        .lines
                        .into_iter()
                        .map(|line| GeneralJournalLineResponse {
                            id: line.id.get().into(),
                            line_no: line.line_no.into(),
                            account_id: line.account_id.get().into(),
                            account_name: line.account_name,
                            side: line.side.as_str(),
                            amount_minor: line.amount.get().into(),
                            memo: line.memo,
                        })
                        .collect(),
                    debit_total_minor: entry.debit_total.get().into(),
                    credit_total_minor: entry.credit_total.get().into(),
                })
                .collect(),
            total_debit_minor: report.total_debit.get().into(),
            total_credit_minor: report.total_credit.get().into(),
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
    account_id: ExactInteger,
    account_name: String,
    element: &'static str,
    debit_turnover_minor: ExactInteger,
    credit_turnover_minor: ExactInteger,
    net_activity_minor: ExactInteger,
}

#[derive(Serialize)]
struct IncomeExpenseResponse {
    from: String,
    through: String,
    rows: Vec<IncomeExpenseRowResponse>,
    total_income_minor: ExactInteger,
    total_expense_minor: ExactInteger,
    net_result_minor: ExactInteger,
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
                    account_id: row.account_id.get().into(),
                    account_name: row.account_name,
                    element: row.element.as_str(),
                    debit_turnover_minor: row.debit_turnover.get().into(),
                    credit_turnover_minor: row.credit_turnover.get().into(),
                    net_activity_minor: row.net_activity.get().into(),
                })
                .collect(),
            total_income_minor: report.total_income.get().into(),
            total_expense_minor: report.total_expense.get().into(),
            net_result_minor: report.net_result.get().into(),
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
    Unprocessable { code: &'static str, message: String },
    Conflict { code: &'static str, message: String },
    Forbidden,
    PayloadTooLarge,
    JsonRequired,
    Internal,
}

impl ApiError {
    fn invalid_path() -> Self {
        Self::BadRequest {
            code: "invalid_path",
            message: "The request path is not valid UTF-8.".to_owned(),
        }
    }

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
            Self::Unprocessable { code, message } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiErrorBody { code, message }),
            )
                .into_response(),
            Self::Conflict { code, message } => {
                (StatusCode::CONFLICT, Json(ApiErrorBody { code, message })).into_response()
            }
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                Json(ApiErrorBody {
                    code: "origin_forbidden",
                    message: "Use this application's localhost page to make changes.".to_owned(),
                }),
            )
                .into_response(),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(ApiErrorBody {
                    code: "body_too_large",
                    message: "JSON requests must not exceed 1 MiB.".to_owned(),
                }),
            )
                .into_response(),
            Self::JsonRequired => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(ApiErrorBody {
                    code: "json_required",
                    message: "Use Content-Type: application/json.".to_owned(),
                }),
            )
                .into_response(),
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
