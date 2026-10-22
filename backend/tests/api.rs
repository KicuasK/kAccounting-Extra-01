use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
            SaveDraftEntry,
        },
        money::MinorUnits,
    },
    api,
    db::open_memory_database,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn response_json(response: axum::response::Response) -> Value {
    assert_private_json_headers(&response);
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be bounded and readable");
    let value: Value = serde_json::from_slice(&bytes).expect("response should be valid JSON");
    assert_no_json_numbers(&value);
    value
}

async fn get_json(app: &Router, path: &str) -> Value {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET {path} should succeed"
    );
    response_json(response).await
}

fn assert_object_keys(value: &Value, expected: &[&str]) {
    let object = value.as_object().expect("schema node should be an object");
    assert_eq!(
        object.len(),
        expected.len(),
        "unexpected object fields in {value}"
    );
    for key in expected {
        assert!(object.contains_key(*key), "missing field {key} in {value}");
    }
}

fn assert_string_fields(value: &Value, fields: &[&str]) {
    for field in fields {
        assert!(
            value[*field].is_string(),
            "{field} should be a string in {value}"
        );
    }
}

fn assert_account_schema(value: &Value) {
    assert_object_keys(
        value,
        &[
            "id",
            "parent_id",
            "name",
            "element",
            "kind",
            "is_active",
            "revision",
            "created_at",
            "updated_at",
        ],
    );
    assert!(value["id"].is_string());
    assert!(value["parent_id"].is_null() || value["parent_id"].is_string());
    assert!(value["name"].is_string());
    assert!(value["element"].is_string());
    assert!(value["kind"].is_string());
    assert!(value["is_active"].is_boolean());
    assert!(value["revision"].is_string());
    assert!(value["created_at"].is_string());
    assert!(value["updated_at"].is_string());
}

fn assert_journal_line_schema(value: &Value) {
    assert_object_keys(
        value,
        &[
            "id",
            "line_no",
            "account_id",
            "side",
            "amount_minor",
            "memo",
        ],
    );
    assert_string_fields(
        value,
        &["id", "line_no", "account_id", "side", "amount_minor"],
    );
}

fn assert_error_schema(value: &Value, expected_code: &str) {
    assert_object_keys(value, &["code", "message"]);
    assert_eq!(value["code"], expected_code);
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "public error message should be a non-empty string"
    );
}

fn assert_get_head_allow_header(response: &axum::response::Response) {
    let mut methods: Vec<_> = response.headers()["allow"]
        .to_str()
        .expect("Allow should be valid ASCII")
        .split(',')
        .map(str::trim)
        .collect();
    methods.sort_unstable();
    assert_eq!(methods, ["GET", "HEAD"]);
}

fn assert_no_json_numbers(value: &Value) {
    match value {
        Value::Number(number) => {
            panic!("API integer escaped exact string encoding: {number}")
        }
        Value::Array(values) => values.iter().for_each(assert_no_json_numbers),
        Value::Object(fields) => fields.values().for_each(assert_no_json_numbers),
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

fn assert_private_json_headers(response: &axum::response::Response) {
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["pragma"], "no-cache");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    for absent in [
        "access-control-allow-origin",
        "access-control-allow-credentials",
        "etag",
        "last-modified",
    ] {
        assert!(
            response.headers().get(absent).is_none(),
            "private API responses must omit {absent}"
        );
    }
}

#[tokio::test]
async fn health_route_reports_the_exact_private_application_name() {
    let pool = open_memory_database().await.expect("database should open");
    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["pragma"], "no-cache");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    assert_eq!(
        response_json(response).await,
        json!({"service": "kAccounting-Extra-01", "status": "ok"})
    );
}

#[tokio::test]
async fn private_response_headers_cover_success_and_every_public_error_class() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool);
    for (method, path, expected_status) in [
        (Method::GET, "/api/health", StatusCode::OK),
        (
            Method::GET,
            "/api/accounts/not-an-id",
            StatusCode::BAD_REQUEST,
        ),
        (Method::GET, "/api/does-not-exist", StatusCode::NOT_FOUND),
        (
            Method::OPTIONS,
            "/api/accounts",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (Method::GET, "/api/accounts/%FF", StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            expected_status,
            "unexpected status for {path}"
        );
        assert_private_json_headers(&response);
        let _ = response_json(response).await;
    }
}

#[tokio::test]
async fn read_only_routes_fail_closed_without_leaking_database_errors() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool.clone());
    pool.close().await;

    for path in [
        "/api/health",
        "/api/accounts",
        "/api/accounts/tree",
        "/api/accounts/1",
        "/api/accounts/1/path",
        "/api/journal/drafts",
        "/api/journal/drafts/1",
        "/api/journal/posted/1",
        "/api/reports/trial-balance/2026-01-01",
        "/api/reports/accounting-equation/2026-01-01",
        "/api/reports/ledger/1/2026-01-01/2026-01-31",
        "/api/reports/general-journal/2026-01-01/2026-01-31",
        "/api/reports/income-expense/2026-01-01/2026-01-31",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_private_json_headers(&response);
        let body = response_json(response).await;
        assert_error_schema(&body, "internal_error");
        assert_eq!(body["message"], "The request could not be completed.");
        let rendered = body.to_string().to_ascii_lowercase();
        for secret in ["sqlite", "database", "pool", "closed", "connection", "sqlx"] {
            assert!(
                !rendered.contains(secret),
                "database errors must not expose {secret:?} details for {path}: {body}"
            );
        }
    }
}

#[tokio::test]
async fn account_route_returns_stable_identifiers_revisions_and_activation_state() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let inactive = accounts
        .set_active(cash.id, cash.revision, false)
        .await
        .unwrap();

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/accounts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], inactive.id.get().to_string());
    assert_eq!(body[0]["name"], "現金");
    assert_eq!(body[0]["element"], "asset");
    assert_eq!(body[0]["kind"], "posting");
    assert_eq!(body[0]["is_active"], false);
    assert_eq!(body[0]["revision"], inactive.revision.to_string());
}

#[tokio::test]
async fn account_detail_and_tree_routes_preserve_the_validated_hierarchy() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let category = accounts
        .create(NewAccount::new(
            None,
            "銀行 \"A\" \\ 本店",
            AccountingElement::Asset,
            AccountKind::Category,
        ))
        .await
        .unwrap();
    let child = accounts
        .create(NewAccount::new(
            Some(category.id),
            "主要帳戶",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let app = api::router(pool);

    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/accounts/{}", child.id.get()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = response_json(detail).await;
    assert_eq!(detail["id"], child.id.get().to_string());
    assert_eq!(detail["parent_id"], category.id.get().to_string());

    let tree = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/accounts/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tree.status(), StatusCode::OK);
    assert_eq!(tree.headers()["cache-control"], "no-store");
    assert_eq!(tree.headers()["pragma"], "no-cache");
    let tree = response_json(tree).await;
    assert_eq!(tree.as_array().unwrap().len(), 1);
    assert_eq!(tree[0]["account"]["id"], category.id.get().to_string());
    assert_eq!(tree[0]["account"]["name"], "銀行 \"A\" \\ 本店");
    assert_eq!(
        tree[0]["children"][0]["account"]["id"],
        child.id.get().to_string()
    );

    let path = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/accounts/{}/path", child.id.get()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(path.status(), StatusCode::OK);
    let path = response_json(path).await;
    assert_eq!(path.as_array().unwrap().len(), 2);
    assert_eq!(path[0]["id"], category.id.get().to_string());
    assert_eq!(path[1]["id"], child.id.get().to_string());

    for (path, expected_status, code) in [
        (
            "/api/accounts/not-an-id",
            StatusCode::BAD_REQUEST,
            "invalid_id",
        ),
        ("/api/accounts/0", StatusCode::BAD_REQUEST, "invalid_id"),
        ("/api/accounts/-1", StatusCode::BAD_REQUEST, "invalid_id"),
        (
            "/api/accounts/9223372036854775808",
            StatusCode::BAD_REQUEST,
            "invalid_id",
        ),
        (
            "/api/accounts/999999",
            StatusCode::NOT_FOUND,
            "account_not_found",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn account_tree_serialization_does_not_depend_on_recursive_call_stack_depth() {
    const ACCOUNT_COUNT: i64 = 5_000;

    let pool = open_memory_database().await.expect("database should open");
    let mut transaction = pool.begin().await.expect("transaction should begin");
    for id in 1..=ACCOUNT_COUNT {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, parent_id, name, element, kind)
            VALUES (?, ?, ?, 'asset', ?)
            "#,
        )
        .bind(id)
        .bind((id > 1).then_some(id - 1))
        .bind(format!("Account {id}"))
        .bind(if id == ACCOUNT_COUNT {
            "posting"
        } else {
            "category"
        })
        .execute(&mut *transaction)
        .await
        .expect("deep account should insert");
    }
    transaction
        .commit()
        .await
        .expect("transaction should commit");

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/accounts/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_private_json_headers(&response);
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("deep tree response should remain bounded and readable");
    let text = std::str::from_utf8(&bytes).expect("tree JSON should be UTF-8");

    assert!(text.starts_with("[{\"account\":"));
    let expected_end = "]}".repeat(ACCOUNT_COUNT as usize) + "]";
    assert!(text.ends_with(&expected_end));
    assert_eq!(
        text.matches("\"children\":[").count(),
        ACCOUNT_COUNT as usize
    );
    assert!(text.contains(&format!(r#""id":"{ACCOUNT_COUNT}""#)));
    assert!(!text.contains(&format!(r#""id":{ACCOUNT_COUNT}"#)));
}

#[tokio::test]
async fn account_tree_serialization_handles_a_wide_ordered_response() {
    const ACCOUNT_COUNT: i64 = 5_000;

    let pool = open_memory_database().await.expect("database should open");
    let mut transaction = pool.begin().await.expect("transaction should begin");
    for id in 1..=ACCOUNT_COUNT {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, name, element, kind)
            VALUES (?, ?, 'asset', 'posting')
            "#,
        )
        .bind(id)
        .bind(format!("Account {id}"))
        .execute(&mut *transaction)
        .await
        .expect("wide account should insert");
    }
    transaction
        .commit()
        .await
        .expect("transaction should commit");

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/accounts/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_private_json_headers(&response);
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("wide tree response should remain bounded and readable");
    let body: Value = serde_json::from_slice(&bytes).expect("wide tree should be valid JSON");
    let roots = body.as_array().expect("tree response should be an array");

    assert_eq!(roots.len(), ACCOUNT_COUNT as usize);
    assert_eq!(roots[0]["account"]["id"], "1");
    assert_eq!(roots[ACCOUNT_COUNT as usize - 1]["account"]["id"], "5000");
    assert!(roots.iter().all(|node| node["children"] == json!([])));
    assert_no_json_numbers(&body);
}

#[tokio::test]
async fn api_serializes_large_sqlite_identifiers_as_strings() {
    let pool = open_memory_database().await.expect("database should open");
    let large_id = 9_007_199_254_740_993_i64;
    sqlx::query(
        r#"
        INSERT INTO accounts (id, name, element, kind)
        VALUES (?, 'Large identifier', 'asset', 'posting')
        "#,
    )
    .bind(large_id)
    .execute(&pool)
    .await
    .expect("valid large SQLite identifier should be stored exactly");

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri(format!("/api/accounts/{large_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["id"], large_id.to_string());
}

#[tokio::test]
async fn draft_route_exposes_exact_integer_lines_and_excludes_posted_entries() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let revenue = accounts
        .create(NewAccount::new(
            None,
            "收入",
            AccountingElement::Income,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![JournalLineInput::new(
                cash.id,
                JournalSide::Debit,
                MinorUnits::new(9_007_199_254_740_993),
            )],
        ))
        .await
        .unwrap();
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(100)),
                JournalLineInput::new(revenue.id, JournalSide::Credit, MinorUnits::new(100)),
            ],
        ))
        .await
        .unwrap();
    let app = api::router(pool);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/journal/drafts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], draft.id.get().to_string());
    assert_eq!(body[0]["revision"], draft.revision.to_string());
    assert_eq!(body[0]["lines"][0]["side"], "debit");
    assert_eq!(body[0]["lines"][0]["amount_minor"], "9007199254740993");

    let draft_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/journal/drafts/{}", draft.id.get()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(draft_response.status(), StatusCode::OK);
    assert_eq!(
        response_json(draft_response).await["id"],
        draft.id.get().to_string()
    );

    let posted_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/journal/posted/{}", posted.id.get()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(posted_response.status(), StatusCode::OK);
    let posted_body = response_json(posted_response).await;
    assert_eq!(posted_body["id"], posted.id.get().to_string());
    assert_eq!(posted_body["lines"][0]["amount_minor"], "100");

    for path in [
        "/api/journal/drafts/0",
        "/api/journal/posted/-1",
        "/api/journal/drafts/9223372036854775808",
    ] {
        let invalid_id = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(invalid_id.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(invalid_id).await["code"], "invalid_id");
    }

    let wrong_state = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/journal/posted/{}", draft.id.get()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_state.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(wrong_state).await["code"],
        "posted_entry_not_found"
    );
}

#[tokio::test]
async fn report_routes_validate_paths_and_serialize_money_as_exact_strings() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let capital = accounts
        .create(NewAccount::new(
            None,
            "資本",
            AccountingElement::Equity,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                JournalLineInput::new(
                    cash.id,
                    JournalSide::Debit,
                    MinorUnits::new(9_007_199_254_740_993),
                ),
                JournalLineInput::new(
                    capital.id,
                    JournalSide::Credit,
                    MinorUnits::new(9_007_199_254_740_993),
                ),
            ],
        ))
        .await
        .unwrap();
    let app = api::router(pool);

    let trial_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reports/trial-balance/2026-12-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(trial_response.status(), StatusCode::OK);
    let trial = response_json(trial_response).await;
    assert_eq!(trial["total_debit_minor"], "9007199254740993");
    assert_eq!(trial["total_credit_minor"], "9007199254740993");

    let equation_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reports/accounting-equation/2026-12-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(equation_response.status(), StatusCode::OK);
    let equation = response_json(equation_response).await;
    assert_eq!(equation["rows"].as_array().unwrap().len(), 5);
    assert_eq!(equation["rows"][0]["element"], "asset");
    assert_eq!(
        equation["rows"][0]["debit_balance_minor"],
        "9007199254740993"
    );

    let ledger_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/reports/ledger/{}/2026-01-01/2026-12-31",
                    cash.id.get()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ledger_response.status(), StatusCode::OK);
    let ledger = response_json(ledger_response).await;
    assert_eq!(ledger["lines"][0]["amount_minor"], "9007199254740993");
    assert_eq!(ledger["closing_debit_balance_minor"], "9007199254740993");

    let journal_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reports/general-journal/2026-01-01/2026-12-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(journal_response.status(), StatusCode::OK);
    let general_journal = response_json(journal_response).await;
    assert_eq!(general_journal["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        general_journal["entries"][0]["lines"][0]["amount_minor"],
        "9007199254740993"
    );

    let income_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reports/income-expense/2026-01-01/2026-12-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(income_response.status(), StatusCode::OK);
    let income = response_json(income_response).await;
    assert!(income["rows"].as_array().unwrap().is_empty());
    assert_eq!(income["net_result_minor"], "0");

    for (uri, status, code) in [
        (
            "/api/reports/trial-balance/2026-02-30".to_owned(),
            StatusCode::BAD_REQUEST,
            "invalid_date",
        ),
        (
            "/api/reports/ledger/not-an-id/2026-01-01/2026-12-31".to_owned(),
            StatusCode::BAD_REQUEST,
            "invalid_id",
        ),
        (
            "/api/reports/ledger/999999/2026-01-01/2026-12-31".to_owned(),
            StatusCode::NOT_FOUND,
            "account_not_found",
        ),
        (
            format!(
                "/api/reports/ledger/{}/2026-12-31/2026-01-01",
                cash.id.get()
            ),
            StatusCode::BAD_REQUEST,
            "invalid_date_range",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn income_expense_api_maps_nonzero_posted_activity_without_drafts() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let income_account = accounts
        .create(NewAccount::new(
            None,
            "收入",
            AccountingElement::Income,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let expense_account = accounts
        .create(NewAccount::new(
            None,
            "費用",
            AccountingElement::Expense,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(1_000)),
                JournalLineInput::new(
                    income_account.id,
                    JournalSide::Credit,
                    MinorUnits::new(1_000),
                ),
            ],
        ))
        .await
        .unwrap();
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-02").unwrap(),
            vec![
                JournalLineInput::new(expense_account.id, JournalSide::Debit, MinorUnits::new(300)),
                JournalLineInput::new(cash.id, JournalSide::Credit, MinorUnits::new(300)),
            ],
        ))
        .await
        .unwrap();
    journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-01-03").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(9_999)),
                JournalLineInput::new(
                    income_account.id,
                    JournalSide::Credit,
                    MinorUnits::new(9_999),
                ),
            ],
        ))
        .await
        .unwrap();

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/reports/income-expense/2026-01-01/2026-01-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["rows"].as_array().unwrap().len(), 2);
    assert_eq!(body["total_income_minor"], "1000");
    assert_eq!(body["total_expense_minor"], "300");
    assert_eq!(body["net_result_minor"], "700");
    assert!(body["rows"].as_array().unwrap().iter().all(|row| {
        row["account_id"].is_string()
            && row["debit_turnover_minor"].is_string()
            && row["credit_turnover_minor"].is_string()
            && row["net_activity_minor"].is_string()
    }));
}

#[tokio::test]
async fn every_success_response_has_the_complete_stable_json_schema() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let assets = accounts
        .create(NewAccount::new(
            None,
            "Assets",
            AccountingElement::Asset,
            AccountKind::Category,
        ))
        .await
        .unwrap();
    let cash = accounts
        .create(NewAccount::new(
            Some(assets.id),
            "Cash",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let revenue = accounts
        .create(NewAccount::new(
            None,
            "Revenue",
            AccountingElement::Income,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![JournalLineInput::new(
                cash.id,
                JournalSide::Debit,
                MinorUnits::new(25),
            )],
        ))
        .await
        .unwrap();
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-02").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(100)),
                JournalLineInput::new(revenue.id, JournalSide::Credit, MinorUnits::new(100)),
            ],
        ))
        .await
        .unwrap();
    let app = api::router(pool);

    let health = get_json(&app, "/api/health").await;
    assert_object_keys(&health, &["service", "status"]);
    assert_string_fields(&health, &["service", "status"]);

    let account_list = get_json(&app, "/api/accounts").await;
    let account_list = account_list
        .as_array()
        .expect("account list should be an array");
    assert_eq!(account_list.len(), 3);
    account_list.iter().for_each(assert_account_schema);

    let account = get_json(&app, &format!("/api/accounts/{}", cash.id.get())).await;
    assert_account_schema(&account);

    let path = get_json(&app, &format!("/api/accounts/{}/path", cash.id.get())).await;
    let path = path.as_array().expect("account path should be an array");
    assert_eq!(path.len(), 2);
    path.iter().for_each(assert_account_schema);

    let tree = get_json(&app, "/api/accounts/tree").await;
    let roots = tree.as_array().expect("account tree should be an array");
    assert_eq!(roots.len(), 2);
    for root in roots {
        assert_object_keys(root, &["account", "children"]);
        assert_account_schema(&root["account"]);
        let children = root["children"]
            .as_array()
            .expect("tree children should be an array");
        for child in children {
            assert_object_keys(child, &["account", "children"]);
            assert_account_schema(&child["account"]);
            assert!(
                child["children"]
                    .as_array()
                    .expect("leaf children should be an array")
                    .is_empty()
            );
        }
    }

    let drafts = get_json(&app, "/api/journal/drafts").await;
    let drafts = drafts.as_array().expect("draft list should be an array");
    assert_eq!(drafts.len(), 1);
    assert_object_keys(
        &drafts[0],
        &[
            "id",
            "entry_date",
            "revision",
            "created_at",
            "lines",
            "reference",
            "explanation",
        ],
    );
    assert_string_fields(&drafts[0], &["id", "entry_date", "revision", "created_at"]);
    assert!(drafts[0]["lines"].is_array());
    assert_journal_line_schema(&drafts[0]["lines"][0]);

    let draft = get_json(&app, &format!("/api/journal/drafts/{}", draft.id.get())).await;
    assert_object_keys(
        &draft,
        &[
            "id",
            "entry_date",
            "revision",
            "created_at",
            "lines",
            "reference",
            "explanation",
        ],
    );
    assert_string_fields(&draft, &["id", "entry_date", "revision", "created_at"]);
    assert!(draft["lines"].is_array());
    assert_journal_line_schema(&draft["lines"][0]);

    let posted = get_json(&app, &format!("/api/journal/posted/{}", posted.id.get())).await;
    assert_object_keys(
        &posted,
        &[
            "id",
            "entry_date",
            "created_at",
            "posted_at",
            "lines",
            "reference",
            "explanation",
            "relationship",
            "original_entry_id",
        ],
    );
    assert_string_fields(&posted, &["id", "entry_date", "created_at", "posted_at"]);
    for line in posted["lines"]
        .as_array()
        .expect("posted lines should be an array")
    {
        assert_journal_line_schema(line);
    }

    let trial = get_json(&app, "/api/reports/trial-balance/2026-01-31").await;
    assert_object_keys(
        &trial,
        &["as_of", "rows", "total_debit_minor", "total_credit_minor"],
    );
    assert_string_fields(
        &trial,
        &["as_of", "total_debit_minor", "total_credit_minor"],
    );
    for row in trial["rows"]
        .as_array()
        .expect("trial rows should be an array")
    {
        assert_object_keys(
            row,
            &[
                "account_id",
                "account_name",
                "element",
                "debit_turnover_minor",
                "credit_turnover_minor",
                "debit_balance_minor",
                "credit_balance_minor",
            ],
        );
        assert_string_fields(
            row,
            &[
                "account_id",
                "account_name",
                "element",
                "debit_turnover_minor",
                "credit_turnover_minor",
                "debit_balance_minor",
                "credit_balance_minor",
            ],
        );
    }

    let equation = get_json(&app, "/api/reports/accounting-equation/2026-01-31").await;
    assert_object_keys(
        &equation,
        &["as_of", "rows", "total_debit_minor", "total_credit_minor"],
    );
    assert_string_fields(
        &equation,
        &["as_of", "total_debit_minor", "total_credit_minor"],
    );
    for row in equation["rows"]
        .as_array()
        .expect("equation rows should be an array")
    {
        assert_object_keys(
            row,
            &["element", "debit_balance_minor", "credit_balance_minor"],
        );
        assert_string_fields(
            row,
            &["element", "debit_balance_minor", "credit_balance_minor"],
        );
    }

    let ledger = get_json(
        &app,
        &format!(
            "/api/reports/ledger/{}/2026-01-01/2026-01-31",
            cash.id.get()
        ),
    )
    .await;
    assert_object_keys(
        &ledger,
        &[
            "account_id",
            "account_name",
            "element",
            "from",
            "through",
            "opening_debit_balance_minor",
            "opening_credit_balance_minor",
            "lines",
            "period_debit_turnover_minor",
            "period_credit_turnover_minor",
            "closing_debit_balance_minor",
            "closing_credit_balance_minor",
        ],
    );
    assert_string_fields(
        &ledger,
        &[
            "account_id",
            "account_name",
            "element",
            "from",
            "through",
            "opening_debit_balance_minor",
            "opening_credit_balance_minor",
            "period_debit_turnover_minor",
            "period_credit_turnover_minor",
            "closing_debit_balance_minor",
            "closing_credit_balance_minor",
        ],
    );
    for line in ledger["lines"]
        .as_array()
        .expect("ledger lines should be an array")
    {
        assert_object_keys(
            line,
            &[
                "entry_id",
                "line_id",
                "entry_date",
                "line_no",
                "side",
                "amount_minor",
                "debit_balance_minor",
                "credit_balance_minor",
                "memo",
                "reference",
                "explanation",
            ],
        );
        assert_string_fields(
            line,
            &[
                "entry_id",
                "line_id",
                "entry_date",
                "line_no",
                "side",
                "amount_minor",
                "debit_balance_minor",
                "credit_balance_minor",
            ],
        );
    }

    let general_journal =
        get_json(&app, "/api/reports/general-journal/2026-01-01/2026-01-31").await;
    assert_object_keys(
        &general_journal,
        &[
            "from",
            "through",
            "entries",
            "total_debit_minor",
            "total_credit_minor",
        ],
    );
    assert_string_fields(
        &general_journal,
        &["from", "through", "total_debit_minor", "total_credit_minor"],
    );
    for entry in general_journal["entries"]
        .as_array()
        .expect("general journal entries should be an array")
    {
        assert_object_keys(
            entry,
            &[
                "id",
                "entry_date",
                "posted_at",
                "lines",
                "debit_total_minor",
                "credit_total_minor",
                "reference",
                "explanation",
            ],
        );
        assert_string_fields(
            entry,
            &[
                "id",
                "entry_date",
                "posted_at",
                "debit_total_minor",
                "credit_total_minor",
            ],
        );
        for line in entry["lines"]
            .as_array()
            .expect("general journal lines should be an array")
        {
            assert_object_keys(
                line,
                &[
                    "id",
                    "line_no",
                    "account_id",
                    "account_name",
                    "side",
                    "amount_minor",
                    "memo",
                ],
            );
            assert_string_fields(
                line,
                &[
                    "id",
                    "line_no",
                    "account_id",
                    "account_name",
                    "side",
                    "amount_minor",
                ],
            );
        }
    }

    let income_expense = get_json(&app, "/api/reports/income-expense/2026-01-01/2026-01-31").await;
    assert_object_keys(
        &income_expense,
        &[
            "from",
            "through",
            "rows",
            "total_income_minor",
            "total_expense_minor",
            "net_result_minor",
        ],
    );
    assert_string_fields(
        &income_expense,
        &[
            "from",
            "through",
            "total_income_minor",
            "total_expense_minor",
            "net_result_minor",
        ],
    );
    for row in income_expense["rows"]
        .as_array()
        .expect("income/expense rows should be an array")
    {
        assert_object_keys(
            row,
            &[
                "account_id",
                "account_name",
                "element",
                "debit_turnover_minor",
                "credit_turnover_minor",
                "net_activity_minor",
            ],
        );
        assert_string_fields(
            row,
            &[
                "account_id",
                "account_name",
                "element",
                "debit_turnover_minor",
                "credit_turnover_minor",
                "net_activity_minor",
            ],
        );
    }
}

#[tokio::test]
async fn api_serializes_large_negative_report_values_as_exact_strings() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "Cash",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let expense = accounts
        .create(NewAccount::new(
            None,
            "Expense",
            AccountingElement::Expense,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                JournalLineInput::new(expense.id, JournalSide::Debit, MinorUnits::new(i64::MAX)),
                JournalLineInput::new(cash.id, JournalSide::Credit, MinorUnits::new(i64::MAX)),
            ],
        ))
        .await
        .unwrap();

    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/reports/income-expense/2026-01-01/2026-01-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["total_expense_minor"], i64::MAX.to_string());
    assert_eq!(body["net_result_minor"], (-i64::MAX).to_string());
    assert_eq!(body["rows"][0]["net_activity_minor"], i64::MAX.to_string());
}

#[tokio::test]
async fn read_only_routes_reject_mutating_methods() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool);
    for path in [
        "/api/health",
        "/api/accounts",
        "/api/accounts/tree",
        "/api/accounts/1",
        "/api/accounts/1/path",
        "/api/journal/drafts",
        "/api/journal/drafts/1",
        "/api/journal/posted/1",
        "/api/reports/trial-balance/2026-01-01",
        "/api/reports/accounting-equation/2026-01-01",
        "/api/reports/ledger/1/2026-01-01/2026-01-31",
        "/api/reports/general-journal/2026-01-01/2026-01-31",
        "/api/reports/income-expense/2026-01-01/2026-01-31",
    ] {
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::CONNECT,
            Method::OPTIONS,
            Method::TRACE,
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} must not be registered for {path}"
            );
            assert_get_head_allow_header(&response);
            let body = response_json(response).await;
            assert_error_schema(&body, "method_not_allowed");
            assert_eq!(
                body["message"],
                "The requested method is not allowed for this route."
            );
        }
    }

    let pool = open_memory_database().await.expect("database should open");
    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response_json(response).await["code"], "route_not_found");
}

#[tokio::test]
async fn head_is_the_only_non_get_method_accepted_by_read_only_routes() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool);
    for path in [
        "/api/health",
        "/api/accounts",
        "/api/accounts/tree",
        "/api/accounts/1",
        "/api/accounts/1/path",
        "/api/journal/drafts",
        "/api/journal/drafts/1",
        "/api/journal/posted/1",
        "/api/reports/trial-balance/2026-01-01",
        "/api/reports/accounting-equation/2026-01-01",
        "/api/reports/ledger/1/2026-01-01/2026-01-31",
        "/api/reports/general-journal/2026-01-01/2026-01-31",
        "/api/reports/income-expense/2026-01-01/2026-01-31",
    ] {
        let get_response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let get_status = get_response.status();
        let get_content_type = get_response.headers()["content-type"].clone();
        let get_content_length = get_response.headers().get("content-length").cloned();

        let head_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::HEAD)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            head_response.status(),
            get_status,
            "HEAD should select the same handler as GET for {path}"
        );
        assert_eq!(head_response.headers()["content-type"], get_content_type);
        assert_eq!(
            head_response.headers().get("content-length"),
            get_content_length.as_ref()
        );
        assert_private_json_headers(&head_response);
        assert!(
            to_bytes(head_response.into_body(), 1024)
                .await
                .expect("HEAD body should be readable")
                .is_empty(),
            "HEAD must not expose a response body for {path}"
        );
    }
}

#[tokio::test]
async fn malformed_percent_encoded_paths_return_the_stable_json_error_envelope() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool);
    for path in [
        "/api/accounts/%FF",
        "/api/accounts/%C3%28",
        "/api/accounts/%FF/path",
        "/api/journal/drafts/%FF",
        "/api/reports/trial-balance/%FF",
        "/api/reports/ledger/1/2026-01-01/%FF",
        "/api/reports/general-journal/%FF/2026-01-31",
        "/api/does-not-exist/%FF",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "invalid UTF-8 path parameter should be rejected for {path}"
        );
        let body = response_json(response).await;
        assert_error_schema(&body, "invalid_path");
        assert_eq!(body["message"], "The request path is not valid UTF-8.");
    }
}

#[tokio::test]
async fn near_miss_paths_do_not_alias_registered_routes() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool);
    for path in [
        "/api",
        "/api/",
        "/api//health",
        "/api/health/",
        "/API/health",
        "/api/accounts%2Ftree",
        "/api/accounts/1/extra",
        "/api/journal/posted",
        "/api/reports/ledger/1/2026-01-01",
        "/api/reports/general-journal/2026-01-01/2026-01-31/extra",
        "/api/does-not-exist/%",
        "/api/does-not-exist/%GG",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "near-miss path must not alias a registered route: {path}"
        );
        assert!(
            response.headers().get("allow").is_none(),
            "unregistered paths must not advertise route methods"
        );
        let body = response_json(response).await;
        assert_error_schema(&body, "route_not_found");
        assert_eq!(body["message"], "The requested API route was not found.");
    }
}
