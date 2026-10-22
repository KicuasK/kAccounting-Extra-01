use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use kaccounting_backend::{
    accounting::{
        accounts::{Account, AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
            SaveDraftEntry,
        },
        money::MinorUnits,
        reports::ReportRepository,
    },
    api,
    db::{open_memory_database, verify_database_integrity},
};
use tower::ServiceExt;

fn date(value: &str) -> JournalDate {
    JournalDate::parse(value).unwrap()
}
async fn account(
    pool: &sqlx::SqlitePool,
    name: &str,
    element: AccountingElement,
    parent: Option<&Account>,
    kind: AccountKind,
) -> Account {
    AccountRepository::new(pool.clone())
        .create(NewAccount::new(parent.map(|a| a.id), name, element, kind))
        .await
        .unwrap()
}
async fn post(pool: &sqlx::SqlitePool, when: &str, debit: &Account, credit: &Account, amount: i64) {
    JournalRepository::new(pool.clone())
        .post(PostEntry::new(
            date(when),
            vec![
                JournalLineInput::new(debit.id, JournalSide::Debit, MinorUnits::new(amount)),
                JournalLineInput::new(credit.id, JournalSide::Credit, MinorUnits::new(amount)),
            ],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn balance_sheet_reconciles_opening_profit_loss_liabilities_and_contra_accounts_without_closing()
 {
    let pool = open_memory_database().await.unwrap();
    let root = account(
        &pool,
        "資產",
        AccountingElement::Asset,
        None,
        AccountKind::Category,
    )
    .await;
    let group = account(
        &pool,
        "User's chosen current group",
        AccountingElement::Asset,
        Some(&root),
        AccountKind::Category,
    )
    .await;
    let cash = account(
        &pool,
        "銀行 Bank 01",
        AccountingElement::Asset,
        Some(&group),
        AccountKind::Posting,
    )
    .await;
    let contra = account(
        &pool,
        "Contra",
        AccountingElement::Asset,
        Some(&root),
        AccountKind::Posting,
    )
    .await;
    let equity = account(
        &pool,
        "Equity",
        AccountingElement::Equity,
        None,
        AccountKind::Posting,
    )
    .await;
    let liability = account(
        &pool,
        "Liability",
        AccountingElement::Liability,
        None,
        AccountKind::Posting,
    )
    .await;
    let income = account(
        &pool,
        "Income",
        AccountingElement::Income,
        None,
        AccountKind::Posting,
    )
    .await;
    let expense = account(
        &pool,
        "Expense",
        AccountingElement::Expense,
        None,
        AccountKind::Posting,
    )
    .await;
    post(&pool, "2025-01-01", &cash, &equity, 100).await;
    post(&pool, "2026-01-01", &cash, &liability, 40).await;
    post(&pool, "2026-02-01", &cash, &income, 50).await;
    post(&pool, "2026-03-01", &expense, &cash, 20).await;
    post(&pool, "2026-04-01", &expense, &contra, 10).await;
    post(&pool, "2026-12-01", &expense, &cash, 60).await;
    JournalRepository::new(pool.clone())
        .create_draft(SaveDraftEntry::new(
            date("2026-01-01"),
            vec![JournalLineInput::new(
                cash.id,
                JournalSide::Debit,
                MinorUnits::new(999),
            )],
        ))
        .await
        .unwrap();
    let repo = ReportRepository::new(pool.clone());
    let report = repo.balance_sheet(date("2026-10-22")).await.unwrap();
    assert_eq!(report.total_assets.get(), 160);
    assert_eq!(report.total_liabilities.get(), 40);
    assert_eq!(report.total_equity.get(), 100);
    assert_eq!(report.unclosed_net_result.get(), 20);
    assert_eq!(report.total_liabilities_and_equity.get(), 160);
    assert_eq!(
        report
            .rows
            .iter()
            .find(|r| r.account_id == contra.id)
            .unwrap()
            .balance
            .get(),
        -10
    );
    assert_eq!(
        report
            .hierarchy
            .iter()
            .find(|a| a.id == cash.id)
            .unwrap()
            .parent_id,
        Some(group.id)
    );
    assert_eq!(
        report
            .hierarchy
            .iter()
            .find(|a| a.id == group.id)
            .unwrap()
            .name,
        group.name
    );
    let earlier = repo.balance_sheet(date("2025-12-31")).await.unwrap();
    assert_eq!(earlier.total_assets.get(), 100);
    assert_eq!(earlier.unclosed_net_result.get(), 0);
    let loss = repo.balance_sheet(date("2026-12-31")).await.unwrap();
    assert_eq!(loss.unclosed_net_result.get(), -40);
    assert_eq!(loss.total_assets.get(), 100);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 7, "report must not generate closing entries");
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn empty_statement_is_zero_and_inactive_history_is_retained() {
    let pool = open_memory_database().await.unwrap();
    let repo = ReportRepository::new(pool.clone());
    let empty = repo.balance_sheet(date("2026-10-22")).await.unwrap();
    assert_eq!(empty.total_assets, MinorUnits::ZERO);
    assert!(empty.rows.is_empty());
    let cash = account(
        &pool,
        "Cash",
        AccountingElement::Asset,
        None,
        AccountKind::Posting,
    )
    .await;
    let equity = account(
        &pool,
        "Equity",
        AccountingElement::Equity,
        None,
        AccountKind::Posting,
    )
    .await;
    post(&pool, "2026-10-22", &cash, &equity, 9007199254740993).await;
    AccountRepository::new(pool.clone())
        .set_active(cash.id, 1, false)
        .await
        .unwrap();
    let report = repo.balance_sheet(date("2026-10-22")).await.unwrap();
    assert_eq!(report.total_assets.get(), 9007199254740993);
    assert!(
        !report
            .hierarchy
            .iter()
            .find(|a| a.id == cash.id)
            .unwrap()
            .is_active
    );
    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/reports/balance-sheet/2026-10-22")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["total_assets_minor"], "9007199254740993");
    assert_eq!(
        body["total_liabilities_and_equity_minor"],
        "9007199254740993"
    );
}

#[tokio::test]
async fn statement_rejects_a_corrupted_account_hierarchy_and_impossible_dates() {
    let pool = open_memory_database().await.unwrap();
    let group = account(
        &pool,
        "Assets",
        AccountingElement::Asset,
        None,
        AccountKind::Category,
    )
    .await;
    let child = account(
        &pool,
        "Cash",
        AccountingElement::Asset,
        Some(&group),
        AccountKind::Posting,
    )
    .await;
    sqlx::query("DROP TRIGGER accounts_validate_parent_update")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE accounts SET parent_id = 999, revision = revision + 1 WHERE id = ?")
        .bind(child.id.get())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ReportRepository::new(pool.clone())
            .balance_sheet(date("2026-10-22"))
            .await
            .is_err()
    );
    let response = api::router(pool)
        .oneshot(
            Request::builder()
                .uri("/api/reports/balance-sheet/2026-02-30")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}
