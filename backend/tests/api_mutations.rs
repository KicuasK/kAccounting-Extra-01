use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    api::localhost_write_router,
    db::{open_memory_database, verify_database_integrity},
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn send(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    send_raw(
        app,
        method,
        path,
        body.to_string(),
        &[
            ("host", "127.0.0.1:3000"),
            ("origin", "http://127.0.0.1:3000"),
            ("content-type", "application/json"),
        ],
    )
    .await
}
async fn send_raw(
    app: &Router,
    method: &str,
    path: &str,
    body: String,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
async fn create(
    app: &Router,
    name: &str,
    element: &str,
    kind: &str,
    parent: Option<&str>,
) -> Value {
    let (status, body) = send(
        app,
        "POST",
        "/api/accounts",
        json!({"name": name, "element": element, "kind": kind, "parent_id": parent}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}
fn entry(cash: &Value, equity: &Value, amount: &str) -> Value {
    json!({"entry_date": "2026-10-22", "reference": "ref 01", "explanation": "說明\nExplanation", "lines": [
        {"account_id": cash["id"], "side": "debit", "amount_minor": amount, "memo": "銀行 Bank 01"},
        {"account_id": equity["id"], "side": "credit", "amount_minor": amount, "memo": null}
    ]})
}
async fn setup() -> (sqlx::SqlitePool, Router, Value, Value) {
    let pool = open_memory_database().await.unwrap();
    let app = localhost_write_router(pool.clone());
    let cash = create(&app, "現金 Cash 01", "asset", "posting", None).await;
    let equity = create(&app, "Opening equity", "equity", "posting", None).await;
    (pool, app, cash, equity)
}

#[tokio::test]
async fn complete_account_draft_post_reverse_correct_and_report_workflow_uses_exact_strings() {
    let (pool, app, cash, equity) = setup().await;
    let mut input = entry(&cash, &equity, "9007199254740993");
    input["lines"].as_array_mut().unwrap().pop();
    let (status, draft) = send(&app, "POST", "/api/journal/drafts", input).await;
    assert_eq!(status, StatusCode::CREATED);
    let path = format!("/api/journal/drafts/{}", draft["id"].as_str().unwrap());
    let (status, error) = send(
        &app,
        "POST",
        &format!("{path}/post"),
        json!({"expected_revision": "1"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["code"], "journal_rule_violation");
    let (status, saved) = send(
        &app,
        "PUT",
        &path,
        json!({"expected_revision": "1", "entry": entry(&cash, &equity, "9007199254740993")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["revision"], "2");
    let (status, _) = send(&app, "DELETE", &path, json!({"expected_revision": "1"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, posted) = send(
        &app,
        "POST",
        &format!("{path}/post"),
        json!({"expected_revision": "2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(posted["relationship"], "ordinary");
    assert!(posted["original_entry_id"].is_null());
    assert_eq!(posted["lines"][0]["amount_minor"], "9007199254740993");
    assert_eq!(posted["lines"][0]["memo"], "銀行 Bank 01");
    assert_eq!(posted["explanation"], "說明\nExplanation");
    let posted_path = format!("/api/journal/posted/{}", posted["id"].as_str().unwrap());
    let (status, trial) = send(
        &app,
        "GET",
        "/api/reports/trial-balance/2026-10-22",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trial["total_debit_minor"], "9007199254740993");
    assert_eq!(
        send(&app, "PATCH", &posted_path, json!({"reference":"changed"}))
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let (status, reversal) = send(
        &app,
        "POST",
        &format!("{posted_path}/reversal"),
        json!({"entry_date":"2026-10-22"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(reversal["relationship"], "reversal");
    assert_eq!(reversal["original_entry_id"], posted["id"]);
    assert_eq!(
        send(
            &app,
            "POST",
            &format!("{posted_path}/reversal"),
            json!({"entry_date":"2026-10-22"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, correction) = send(
        &app,
        "POST",
        &format!("{posted_path}/correction"),
        entry(&cash, &equity, "10"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(correction["relationship"], "correction");
    assert_eq!(correction["original_entry_id"], posted["id"]);
    let (_, original) = send(&app, "GET", &posted_path, Value::Null).await;
    assert_eq!(original, posted);
    let (_, journal) = send(
        &app,
        "GET",
        "/api/reports/general-journal/2026-10-22/2026-10-22",
        Value::Null,
    )
    .await;
    assert_eq!(journal["entries"].as_array().unwrap().len(), 3);
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn account_mutations_preserve_normalization_hierarchy_and_optimistic_concurrency() {
    let (pool, app, cash, _) = setup().await;
    let root = create(&app, "資產", "asset", "category", None).await;
    let path = format!("/api/accounts/{}", cash["id"].as_str().unwrap());
    let (status, moved) = send(
        &app,
        "PATCH",
        &path,
        json!({"operation":"move", "expected_revision":"1", "parent_id":root["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["revision"], "2");
    assert_eq!(
        send(
            &app,
            "PATCH",
            &path,
            json!({"operation":"rename", "expected_revision":"1", "name":"stale"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, renamed) = send(
        &app,
        "PATCH",
        &path,
        json!({"operation":"rename", "expected_revision":"2", "name":"\u{3000}銀行 Bank 01\u{a0}"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "銀行 Bank 01");
    let (status, _) = send(
        &app,
        "POST",
        "/api/accounts",
        json!({"parent_id":root["id"],"name":"銀行 Bank 01", "kind":"posting", "element":"asset"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let root_path = format!("/api/accounts/{}", root["id"].as_str().unwrap());
    assert_eq!(
        send(
            &app,
            "PATCH",
            &root_path,
            json!({"operation":"set_active", "expected_revision":"1", "is_active":false})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(&app, "DELETE", &path, json!({"expected_revision":"3"}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "DELETE", &root_path, json!({"expected_revision":"1"}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn all_mutations_reject_cross_origin_missing_origin_duplicate_headers_and_dns_rebinding() {
    let pool = open_memory_database().await.unwrap();
    let app = localhost_write_router(pool.clone());
    let body = json!({"name":"Cash", "element":"asset", "kind":"posting"}).to_string();
    for headers in [
        vec![
            ("host", "127.0.0.1:3000"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "127.0.0.1:3000"),
            ("origin", "https://evil.example"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "evil.example"),
            ("origin", "http://127.0.0.1:3000"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "127.0.0.1:3000"),
            ("origin", "null"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "127.0.0.1:3000"),
            ("origin", "http://127.0.0.1:3000"),
            ("origin", "http://127.0.0.1:3000"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "127.0.0.1:3000"),
            ("host", "127.0.0.1:3000"),
            ("origin", "http://127.0.0.1:3000"),
            ("content-type", "application/json"),
        ],
        vec![
            ("host", "127.0.0.1:3000"),
            ("origin", "http://127.0.0.1:3000"),
            ("sec-fetch-site", "cross-site"),
            ("content-type", "application/json"),
        ],
    ] {
        let (status, error) = send_raw(&app, "POST", "/api/accounts", body.clone(), &headers).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{headers:?}");
        assert_eq!(error["code"], "origin_forbidden");
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        assert_eq!(
            send_raw(
                &app,
                method,
                "/api/journal/drafts/1",
                "{}".into(),
                &[("host", "127.0.0.1:3000")]
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn reads_also_reject_untrusted_host_and_origin_without_leaking_book_data() {
    let (_, app, _, _) = setup().await;
    assert_eq!(
        send_raw(
            &app,
            "GET",
            "/api/accounts",
            "".into(),
            &[("host", "evil.example")]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send_raw(
            &app,
            "GET",
            "/api/accounts",
            "".into(),
            &[
                ("host", "127.0.0.1:3000"),
                ("origin", "https://evil.example")
            ]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send_raw(
            &app,
            "GET",
            "/api/accounts",
            "".into(),
            &[("host", "127.0.0.1:3000")]
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn malformed_unknown_duplicate_numeric_and_noncanonical_fields_are_rejected_before_writes() {
    let (_, app, cash, equity) = setup().await;
    let valid = entry(&cash, &equity, "10");
    let mut invalid = Vec::new();
    for amount in [
        json!(10),
        json!(10.5),
        json!("01"),
        json!("+1"),
        json!("-0"),
        json!("1e2"),
        json!("9223372036854775808"),
        json!(true),
    ] {
        let mut input = valid.clone();
        input["lines"][0]["amount_minor"] = amount;
        invalid.push(input.to_string());
    }
    let mut unknown = valid.clone();
    unknown["posted_at"] = json!("injected");
    invalid.push(unknown.to_string());
    let mut unknown = valid.clone();
    unknown["lines"][0]["extra"] = json!("injected");
    invalid.push(unknown.to_string());
    invalid.extend([
        "{".into(),
        "null".into(),
        "[]".into(),
        "{\"entry_date\":\"2026-10-22\",\"entry_date\":\"2026-10-21\",\"lines\":[]}".into(),
    ]);
    for input in invalid {
        let (status, error) = send_raw(
            &app,
            "POST",
            "/api/journal/posted",
            input,
            &[
                ("host", "127.0.0.1:3000"),
                ("origin", "http://127.0.0.1:3000"),
                ("content-type", "application/json"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], "invalid_json");
    }
}

#[tokio::test]
async fn unbalanced_missing_inactive_and_category_accounts_leave_no_partial_journal() {
    let (pool, app, cash, equity) = setup().await;
    let mut unbalanced = entry(&cash, &equity, "10");
    unbalanced["lines"][1]["amount_minor"] = json!("9");
    assert_eq!(
        send(&app, "POST", "/api/journal/posted", unbalanced)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut missing = entry(&cash, &equity, "10");
    missing["lines"][1]["account_id"] = json!("9999");
    assert_eq!(
        send(&app, "POST", "/api/journal/posted", missing).await.0,
        StatusCode::NOT_FOUND
    );
    let category = create(&app, "Assets", "asset", "category", None).await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/journal/posted",
            entry(&category, &equity, "10")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let path = format!("/api/accounts/{}", cash["id"].as_str().unwrap());
    send(
        &app,
        "PATCH",
        &path,
        json!({"operation":"set_active", "expected_revision":"1", "is_active":false}),
    )
    .await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/journal/posted",
            entry(&cash, &equity, "10")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for query in [
        "SELECT COUNT(*) FROM journal_entries",
        "SELECT COUNT(*) FROM journal_lines",
    ] {
        let count: i64 = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn json_content_type_and_body_limit_return_actionable_stable_errors() {
    let (_, app, _, _) = setup().await;
    let headers = [
        ("host", "127.0.0.1:3000"),
        ("origin", "http://127.0.0.1:3000"),
    ];
    let (status, error) =
        send_raw(&app, "POST", "/api/journal/drafts", "{}".into(), &headers).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(error["code"], "json_required");
    let (status, error) = send(
        &app,
        "POST",
        "/api/journal/drafts",
        json!({"entry_date":"2026-10-22", "lines":[], "explanation":"x".repeat(1024*1024)}),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error["code"], "body_too_large");
}

#[tokio::test]
async fn concurrent_posting_has_one_winner_and_repeated_requests_do_not_duplicate_history() {
    let (pool, app, cash, equity) = setup().await;
    let (_, draft) = send(
        &app,
        "POST",
        "/api/journal/drafts",
        entry(&cash, &equity, "10"),
    )
    .await;
    let path = format!("/api/journal/drafts/{}/post", draft["id"].as_str().unwrap());
    let (a, b) = tokio::join!(
        send(&app, "POST", &path, json!({"expected_revision":"1"})),
        send(&app, "POST", &path, json!({"expected_revision":"1"}))
    );
    assert!(
        (a.0 == StatusCode::OK && b.0 == StatusCode::CONFLICT)
            || (b.0 == StatusCode::OK && a.0 == StatusCode::CONFLICT)
    );
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE status = 'posted'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn a_move_requires_an_explicit_parent_or_null_and_never_silently_moves_to_root() {
    let (_, app, cash, _) = setup().await;
    let path = format!("/api/accounts/{}", cash["id"].as_str().unwrap());
    assert_eq!(
        send(
            &app,
            "PATCH",
            &path,
            json!({"operation":"move","expected_revision":"1"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, unchanged) = send(&app, "GET", &path, Value::Null).await;
    assert_eq!(unchanged, cash);
    let root = create(&app, "Assets", "asset", "category", None).await;
    assert_eq!(
        send(
            &app,
            "PATCH",
            &path,
            json!({"operation":"move","expected_revision":"1","parent_id":root["id"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, moved) = send(
        &app,
        "PATCH",
        &path,
        json!({"operation":"move","expected_revision":"2","parent_id":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(moved["parent_id"].is_null());
}

#[tokio::test]
async fn optional_entry_time_is_strict_and_roundtrips_without_changing_accounting_date() {
    let (pool, app, cash, equity) = setup().await;
    for value in ["12:34", "24:00:00", "12:60:00", "12:34:60", "12:34:56Z", ""] {
        let mut input = entry(&cash, &equity, "100");
        input["entry_time"] = json!(value);
        let (status, _) = send(&app, "POST", "/api/journal/drafts", input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    }
    let mut input = entry(&cash, &equity, "100");
    input["entry_time"] = json!("23:59:59");
    let (status, draft) = send(&app, "POST", "/api/journal/drafts", input.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(draft["entry_time"], "23:59:59");
    let path = format!("/api/journal/drafts/{}", draft["id"].as_str().unwrap());
    input["entry_time"] = json!("00:00:00");
    let (status, saved) = send(
        &app,
        "PUT",
        &path,
        json!({"expected_revision":"1", "entry":input}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["entry_time"], "00:00:00");
    let (status, posted) = send(
        &app,
        "POST",
        &format!("{path}/post"),
        json!({"expected_revision":"2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(posted["entry_date"], "2026-10-22");
    assert_eq!(posted["entry_time"], "00:00:00");
    let posted_path = format!("/api/journal/posted/{}", posted["id"].as_str().unwrap());
    let (status, reversed) = send(
        &app,
        "POST",
        &format!("{posted_path}/reversal"),
        json!({"entry_date":"2026-10-22", "entry_time":"12:34:56"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(reversed["entry_time"], "12:34:56");
    let (status, corrected) = send(&app, "POST", &format!("{posted_path}/correction"), {
        let mut v = entry(&cash, &equity, "50");
        v["entry_time"] = json!("09:08:07");
        v
    })
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(corrected["entry_time"], "09:08:07");
    let (status, omitted) = send(
        &app,
        "POST",
        "/api/journal/drafts",
        entry(&cash, &equity, "100"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(omitted["entry_time"].is_null());
    verify_database_integrity(&pool).await.unwrap();
}
