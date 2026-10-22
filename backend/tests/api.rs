use axum::{
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
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be bounded and readable");
    let value: Value = serde_json::from_slice(&bytes).expect("response should be valid JSON");
    assert_no_json_numbers(&value);
    value
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
async fn health_route_fails_closed_when_its_database_pool_is_closed() {
    let pool = open_memory_database().await.expect("database should open");
    let app = api::router(pool.clone());
    pool.close().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response_json(response).await["code"], "internal_error");
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
            "銀行",
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
    let tree = response_json(tree).await;
    assert_eq!(tree.as_array().unwrap().len(), 1);
    assert_eq!(tree[0]["account"]["id"], category.id.get().to_string());
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
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
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
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(
                response.headers()["allow"]
                    .to_str()
                    .unwrap()
                    .contains("GET")
            );
            assert_eq!(response_json(response).await["code"], "method_not_allowed");
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
