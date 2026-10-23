use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    api::{localhost_write_router, router},
    db::open_memory_database,
};
use serde_json::{Value, json};
use tower::ServiceExt;
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    origin: bool,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/json");
    if origin {
        request = request.header("origin", "http://127.0.0.1:3000");
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&data).unwrap())
}
fn input() -> Value {
    json!({"due_date":"2026-10-23","title":"水費","payee":"供應商","note":"","amount_minor":"9007199254740993","necessity":"required"})
}
#[tokio::test]
async fn exact_contract_strict_fields_revision_and_origin_are_enforced() {
    let pool = open_memory_database().await.unwrap();
    let app = localhost_write_router(pool);
    assert_eq!(
        send(&app, "POST", "/api/payment-memos", input(), false)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for invalid in [json!(1), json!("01"), json!("1.5"), json!("0")] {
        let mut value = input();
        value["amount_minor"] = invalid;
        assert_eq!(
            send(&app, "POST", "/api/payment-memos", value, true)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut invalid = input();
    invalid["status"] = json!("paid");
    assert_eq!(
        send(&app, "POST", "/api/payment-memos", invalid, true)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, memo) = send(&app, "POST", "/api/payment-memos", input(), true).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(memo["amount_minor"], "9007199254740993");
    assert_eq!(memo["revision"], "1");
    assert_eq!(memo["paid_amount_minor"], "0");
    let id = memo["id"].as_str().unwrap();
    let path = format!("/api/payment-memos/{id}/cancel");
    assert_eq!(
        send(&app, "POST", &path, json!({"expected_revision":"1"}), true)
            .await
            .0,
        StatusCode::OK
    );
    let (status, error) = send(&app, "POST", &path, json!({"expected_revision":"1"}), true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "payment_memo_conflict");
    let (status, list) = send(
        &app,
        "GET",
        "/api/payment-memos?from=2026-10-23&through=2026-10-23",
        Value::Null,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["required_total_minor"], "0");
    assert_eq!(list["items"][0]["status"], "cancelled");
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/payment-memos?from=2026-10-23",
            Value::Null,
            false
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn readonly_router_exposes_plans_without_mutation_routes() {
    let pool = open_memory_database().await.unwrap();
    let app = router(pool);
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/payment-memos?from=2026-01-01&through=2026-12-31",
            Value::Null,
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/payment-memos")
                .header("content-type", "application/json")
                .body(Body::from(input().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
