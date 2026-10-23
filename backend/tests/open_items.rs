use kaccounting_backend::{
    accounting::{
        accounts::{AccountId, AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
            PostedJournalEntry,
        },
        money::MinorUnits,
        open_items::{
            AllocationKind, NewAllocation, NewOpenItem, OpenItemError, OpenItemRepository,
            TrackingKind,
        },
    },
    db::{open_memory_database, verify_database_integrity},
};
use sqlx::SqlitePool;
async fn fixture() -> (
    SqlitePool,
    OpenItemRepository,
    AccountId,
    AccountId,
    AccountId,
) {
    let p = open_memory_database().await.unwrap();
    let a = AccountRepository::new(p.clone());
    let mut ids = Vec::new();
    for (name, element) in [
        ("應收", AccountingElement::Asset),
        ("應付", AccountingElement::Liability),
        ("現金", AccountingElement::Asset),
    ] {
        ids.push(
            a.create(NewAccount::new(None, name, element, AccountKind::Posting))
                .await
                .unwrap()
                .id,
        )
    }
    (
        p.clone(),
        OpenItemRepository::new(p),
        ids[0],
        ids[1],
        ids[2],
    )
}
async fn entry(
    p: &SqlitePool,
    debit: AccountId,
    credit: AccountId,
    n: i64,
    date: &str,
) -> PostedJournalEntry {
    JournalRepository::new(p.clone())
        .post(PostEntry::new(
            JournalDate::parse(date).unwrap(),
            vec![
                JournalLineInput::new(debit, JournalSide::Debit, MinorUnits::new(n)),
                JournalLineInput::new(credit, JournalSide::Credit, MinorUnits::new(n)),
            ],
        ))
        .await
        .unwrap()
}
fn item(n: i64) -> NewOpenItem {
    NewOpenItem {
        amount_minor: n,
        description: "發票款項".into(),
        counterparty: Some("對象".into()),
        due_date: Some("2026-11-01".into()),
    }
}
fn allocation(id: i64, n: i64, kind: AllocationKind) -> NewAllocation {
    NewAllocation {
        item_id: id,
        amount_minor: n,
        kind,
    }
}
#[tokio::test]
async fn explicit_ar_split_partial_payment_adjustment_void_preserves_gl_and_audit() {
    let (p, r, ar, _, cash) = fixture().await;
    let source = entry(&p, ar, cash, 100, "2026-10-01").await;
    let payment = entry(&p, cash, ar, 100, "2026-10-02").await;
    let ids = r
        .register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(60), item(40)],
        )
        .await
        .unwrap();
    let a = r
        .allocate(
            payment.lines[1].id.get(),
            vec![
                allocation(ids[0], 30, AllocationKind::Payment),
                allocation(ids[0], 20, AllocationKind::Adjustment),
            ],
        )
        .await
        .unwrap();
    let rows = r.list().await.unwrap();
    assert_eq!(
        (
            rows[0].paid_minor,
            rows[0].adjusted_minor,
            rows[0].outstanding_minor
        ),
        (30, 20, 10)
    );
    assert_eq!(
        r.line_capacity(payment.lines[1].id.get()).await.unwrap(),
        50
    );
    r.void_allocation(a[1], "重新分配").await.unwrap();
    assert_eq!(r.list().await.unwrap()[0].outstanding_minor, 30);
    assert!(matches!(
        r.void_allocation(a[1], "again").await,
        Err(OpenItemError::AllocationNotActive)
    ));
    let original = JournalRepository::new(p.clone())
        .get_posted(source.id)
        .await
        .unwrap();
    assert_eq!(original, source);
    assert_eq!(
        JournalRepository::new(p.clone())
            .get_posted(payment.id)
            .await
            .unwrap(),
        payment
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM open_item_events")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(count, 4);
    assert!(
        sqlx::query("DELETE FROM open_item_events")
            .execute(&p)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE open_item_allocations SET amount_minor=1")
            .execute(&p)
            .await
            .is_err()
    );
    verify_database_integrity(&p).await.unwrap();
}
#[tokio::test]
async fn payable_and_large_exact_amounts_work_without_account_name_policy() {
    let (p, r, _, ap, cash) = fixture().await;
    let n = 9_007_199_254_740_993;
    let source = entry(&p, cash, ap, n, "2026-10-01").await;
    let payment = entry(&p, ap, cash, n, "2026-10-02").await;
    let ids = r
        .register(
            source.lines[1].id.get(),
            TrackingKind::Payable,
            vec![item(n)],
        )
        .await
        .unwrap();
    r.allocate(
        payment.lines[0].id.get(),
        vec![allocation(ids[0], n, AllocationKind::Payment)],
    )
    .await
    .unwrap();
    assert_eq!(r.list().await.unwrap()[0].paid_minor, n);
    assert_eq!(r.list().await.unwrap()[0].outstanding_minor, 0);
}
#[tokio::test]
async fn failed_batches_overallocation_and_wrong_account_are_atomic() {
    let (p, r, ar, ap, cash) = fixture().await;
    let source = entry(&p, ar, cash, 100, "2026-10-02").await;
    assert!(matches!(
        r.register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(99)]
        )
        .await,
        Err(OpenItemError::SplitMismatch)
    ));
    assert!(matches!(
        r.register(
            source.lines[0].id.get(),
            TrackingKind::Payable,
            vec![item(100)]
        )
        .await,
        Err(OpenItemError::WrongAccount)
    ));
    assert!(matches!(
        r.register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(0)]
        )
        .await,
        Err(OpenItemError::InvalidInput)
    ));
    let id = r
        .register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(100)],
        )
        .await
        .unwrap()[0];
    assert!(matches!(
        r.register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(100)]
        )
        .await,
        Err(OpenItemError::AlreadyRegistered)
    ));
    let payment = entry(&p, cash, ar, 60, "2026-10-03").await;
    assert!(matches!(
        r.allocate(
            payment.lines[1].id.get(),
            vec![
                allocation(id, 40, AllocationKind::Payment),
                allocation(id, 30, AllocationKind::Payment)
            ]
        )
        .await,
        Err(OpenItemError::Overallocated)
    ));
    assert_eq!(r.list().await.unwrap()[0].outstanding_minor, 100);
    assert!(r.allocations().await.unwrap().is_empty());
    let early = entry(&p, cash, ar, 100, "2026-10-01").await;
    assert!(matches!(
        r.allocate(
            early.lines[1].id.get(),
            vec![allocation(id, 1, AllocationKind::Payment)]
        )
        .await,
        Err(OpenItemError::DateOrder)
    ));
    let wrong = entry(&p, ap, cash, 100, "2026-10-03").await;
    assert!(matches!(
        r.allocate(
            wrong.lines[0].id.get(),
            vec![allocation(id, 1, AllocationKind::Payment)]
        )
        .await,
        Err(OpenItemError::WrongAccount)
    ));
    r.allocate(
        payment.lines[1].id.get(),
        vec![allocation(id, 60, AllocationKind::Payment)],
    )
    .await
    .unwrap();
    let second = entry(&p, cash, ar, 100, "2026-10-04").await;
    assert!(matches!(
        r.allocate(
            second.lines[1].id.get(),
            vec![allocation(id, 41, AllocationKind::Payment)]
        )
        .await,
        Err(OpenItemError::Overallocated)
    ));
}
#[tokio::test]
async fn source_and_payment_reversal_require_explicit_unallocation_and_keep_history() {
    let (p, r, ar, _, cash) = fixture().await;
    let source = entry(&p, ar, cash, 100, "2026-10-01").await;
    let payment = entry(&p, cash, ar, 100, "2026-10-02").await;
    let id = r
        .register(
            source.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(100)],
        )
        .await
        .unwrap()[0];
    let aid = r
        .allocate(
            payment.lines[1].id.get(),
            vec![allocation(id, 100, AllocationKind::Payment)],
        )
        .await
        .unwrap()[0];
    let j = JournalRepository::new(p.clone());
    let date = JournalDate::parse("2026-10-03").unwrap();
    assert!(j.reverse_posted(source.id, date.clone()).await.is_err());
    assert!(j.reverse_posted(payment.id, date.clone()).await.is_err());
    assert!(matches!(
        r.void_allocation(aid, " ").await,
        Err(OpenItemError::InvalidInput)
    ));
    r.void_allocation(aid, "來源更正前解除分配").await.unwrap();
    assert_eq!(j.get_posted(payment.id).await.unwrap(), payment);
    j.reverse_posted(source.id, date.clone()).await.unwrap();
    assert!(r.list().await.unwrap()[0].reversed);
    assert_eq!(r.list().await.unwrap()[0].outstanding_minor, 0);
    assert_eq!(r.allocations().await.unwrap().len(), 1);
    assert!(matches!(
        r.allocate(
            payment.lines[1].id.get(),
            vec![allocation(id, 1, AllocationKind::Payment)]
        )
        .await,
        Err(OpenItemError::ReversedLine)
    ));
    let reversal = j.reverse_posted(payment.id, date).await.unwrap();
    assert!(matches!(
        r.register(
            reversal.lines[1].id.get(),
            TrackingKind::Receivable,
            vec![item(100)]
        )
        .await,
        Err(OpenItemError::ReversedLine)
    ));
    verify_database_integrity(&p).await.unwrap();
}

#[tokio::test]
async fn competing_allocations_cannot_overallocate_and_void_releases_line_capacity() {
    let (p, r, ar, _, cash) = fixture().await;
    let s = entry(&p, ar, cash, 100, "2026-10-01").await;
    let pay = entry(&p, cash, ar, 100, "2026-10-02").await;
    let id = r
        .register(
            s.lines[0].id.get(),
            TrackingKind::Receivable,
            vec![item(100)],
        )
        .await
        .unwrap()[0];
    let line = pay.lines[1].id.get();
    let (a, b) = tokio::join!(
        r.allocate(line, vec![allocation(id, 60, AllocationKind::Payment)]),
        r.allocate(line, vec![allocation(id, 60, AllocationKind::Payment)])
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(r.list().await.unwrap()[0].outstanding_minor, 40);
    let aid = r.allocations().await.unwrap()[0].id;
    r.void_allocation(aid, "重選").await.unwrap();
    r.allocate(line, vec![allocation(id, 100, AllocationKind::Payment)])
        .await
        .unwrap();
    assert_eq!(r.line_capacity(line).await.unwrap(), 0);
}

#[tokio::test]
async fn api_requires_origin_exact_integer_payloads_and_preserves_large_response_values() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;
    let (p, _, ar, _, cash) = fixture().await;
    let n = 9_007_199_254_740_993;
    let s = entry(&p, ar, cash, n, "2026-10-01").await;
    let app = kaccounting_backend::api::localhost_write_router(p);
    let input = json!({"source_line_id":s.lines[0].id.get().to_string(),"kind":"receivable","items":[{"amount_minor":n.to_string(),"description":"明細","counterparty":null,"due_date":null}]});
    let request = |body: serde_json::Value, origin: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/open-items")
            .header("host", "127.0.0.1:3000")
            .header("origin", origin)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(request(input.clone(), "https://evil.invalid"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut wrong = input.clone();
    wrong["items"][0]["amount_minor"] = json!(n);
    assert_eq!(
        app.clone()
            .oneshot(request(wrong, "http://127.0.0.1:3000"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let mut unknown = input.clone();
    unknown["overwrite"] = json!(true);
    assert_eq!(
        app.clone()
            .oneshot(request(unknown, "http://127.0.0.1:3000"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.clone()
            .oneshot(request(input, "http://127.0.0.1:3000"))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/open-items")
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100000).await.unwrap()).unwrap();
    assert_eq!(value["coverage"], "explicit-lines-only");
    assert_eq!(value["items"][0]["amount_minor"], n.to_string());
    assert_eq!(value["items"][0]["outstanding_minor"], n.to_string());
}

#[tokio::test]
async fn schema_tamper_and_inconsistent_inserted_rows_fail_integrity() {
    let (p, _, _, _, _) = fixture().await;
    sqlx::query("DROP TRIGGER open_item_reversal_requires_unallocation")
        .execute(&p)
        .await
        .unwrap();
    assert!(verify_database_integrity(&p).await.is_err());
    let (p, _, ar, _, cash) = fixture().await;
    let s = entry(&p, ar, cash, 100, "2026-10-01").await;
    sqlx::query("INSERT INTO open_item_batches(source_line_id,kind) VALUES(?,'receivable')")
        .bind(s.lines[0].id.get())
        .execute(&p)
        .await
        .unwrap();
    assert!(verify_database_integrity(&p).await.is_err());
}
