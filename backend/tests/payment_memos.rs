use kaccounting_backend::{
    accounting::payment_memos::{MemoError, MemoInput, PaymentMemoRepository},
    db::{open_memory_database, verify_database_integrity},
};
fn input(amount: i64) -> MemoInput {
    MemoInput {
        due_date: "2026-10-23".into(),
        title: " 水費 ".into(),
        payee: "供應商".into(),
        note: "只是計畫".into(),
        amount_minor: amount,
        necessity: "required".into(),
    }
}
#[tokio::test]
async fn lifecycle_is_revision_checked_audited_and_never_posts() {
    let pool = open_memory_database().await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    let memo = repo.create(input(9007199254740993)).await.unwrap();
    assert_eq!(memo.title, "水費");
    let (_, total) = repo.list("2026-10-01", "2026-10-31").await.unwrap();
    assert_eq!(total, 9007199254740993);
    let updated = repo.update(memo.id, 1, input(123)).await.unwrap();
    assert_eq!(updated.revision, 2);
    assert!(matches!(
        repo.update(memo.id, 1, input(999)).await,
        Err(MemoError::Conflict)
    ));
    let cancelled = repo.transition(memo.id, 2, true).await.unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(repo.list("2026-10-23", "2026-10-23").await.unwrap().1, 0);
    assert!(matches!(
        repo.update(memo.id, 3, input(999)).await,
        Err(MemoError::Conflict)
    ));
    repo.transition(memo.id, 3, false).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payment_memo_history")
            .fetch_one(&pool)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM journal_entries")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query("DELETE FROM payment_memos")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE payment_memo_history SET title='tamper'")
            .execute(&pool)
            .await
            .is_err()
    );
    verify_database_integrity(&pool).await.unwrap();
}
#[tokio::test]
async fn invalid_inputs_range_boundaries_and_total_overflow_are_checked() {
    let pool = open_memory_database().await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    for amount in [0, -1] {
        assert!(matches!(
            repo.create(input(amount)).await,
            Err(MemoError::Invalid(_))
        ));
    }
    let mut invalid = input(1);
    invalid.due_date = "2026-02-30".into();
    assert!(repo.create(invalid).await.is_err());
    assert!(repo.list("2026-11-01", "2026-10-01").await.is_err());
    repo.create(input(i64::MAX)).await.unwrap();
    repo.create(input(1)).await.unwrap();
    assert!(matches!(
        repo.list("2026-10-23", "2026-10-23").await,
        Err(MemoError::Overflow)
    ));
    assert_eq!(repo.list("2026-10-24", "2026-11-01").await.unwrap().1, 0);
    verify_database_integrity(&pool).await.unwrap();
}
#[tokio::test]
async fn integrity_rejects_disabled_audit_or_altered_history() {
    let pool = open_memory_database().await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    let memo = repo.create(input(123)).await.unwrap();
    sqlx::query("DROP TRIGGER payment_memo_history_no_update")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE payment_memo_history SET amount_minor=456 WHERE memo_id=?")
        .bind(memo.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER payment_memo_history_no_update BEFORE UPDATE ON payment_memo_history BEGIN SELECT RAISE(ABORT,'payment memo audit is immutable'); END").execute(&pool).await.unwrap();
    assert!(verify_database_integrity(&pool).await.is_err());
}

async fn payment_accounts(pool: &sqlx::SqlitePool) -> (i64, i64) {
    use kaccounting_backend::accounting::accounts::{
        AccountKind, AccountRepository, AccountingElement, NewAccount,
    };
    let repo = AccountRepository::new(pool.clone());
    let debit = repo
        .create(NewAccount::new(
            None,
            "費用",
            AccountingElement::Expense,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    let asset = repo
        .create(NewAccount::new(
            None,
            "銀行",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .unwrap();
    (debit.id.get(), asset.id.get())
}
#[tokio::test]
async fn payment_drafts_are_explicit_atomic_partial_and_reversal_restores_remaining() {
    use kaccounting_backend::accounting::{
        journal::{JournalDate, JournalEntryId, JournalRepository},
        payment_memo_drafts::create_payment_draft,
    };
    let pool = open_memory_database().await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    let memo = repo.create(input(1000)).await.unwrap();
    let (debit, asset) = payment_accounts(&pool).await;
    assert!(
        create_payment_draft(&pool, memo.id, 1, "2026-10-23", asset, asset, 500)
            .await
            .is_err()
    );
    let entry = create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 400)
        .await
        .unwrap();
    let (items, total) = repo.list("2026-10-23", "2026-10-23").await.unwrap();
    assert_eq!(total, 1000);
    assert_eq!(items[0].payment_draft_id, Some(entry));
    assert_eq!(items[0].paid_amount_minor, 0);
    assert!(
        create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 500)
            .await
            .is_err()
    );
    assert!(matches!(
        repo.transition(memo.id, 1, true).await,
        Err(MemoError::Conflict)
    ));
    let journal = JournalRepository::new(pool.clone());
    journal
        .post_draft(JournalEntryId::new(entry), 1)
        .await
        .unwrap();
    let (items, total) = repo.list("2026-10-23", "2026-10-23").await.unwrap();
    assert_eq!(items[0].paid_amount_minor, 400);
    assert_eq!(total, 600);
    assert!(
        create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 601)
            .await
            .is_err()
    );
    let next = create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 600)
        .await
        .unwrap();
    journal
        .delete_draft(JournalEntryId::new(next), 1)
        .await
        .unwrap();
    journal
        .reverse_posted(
            JournalEntryId::new(entry),
            JournalDate::parse("2026-10-24").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(repo.list("2026-10-23", "2026-10-23").await.unwrap().1, 1000);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payment_memo_drafts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    repo.update(memo.id, 1, input(123)).await.unwrap();
    verify_database_integrity(&pool).await.unwrap();
}
#[tokio::test]
async fn edited_payment_candidate_cannot_post_and_admission_failure_is_atomic() {
    use kaccounting_backend::accounting::{
        journal::{JournalEntryId, JournalRepository},
        payment_memo_drafts::create_payment_draft,
    };
    let pool = open_memory_database().await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    let memo = repo.create(input(1000)).await.unwrap();
    let (debit, asset) = payment_accounts(&pool).await;
    let entry = create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 1000)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_lines SET amount_minor=999 WHERE entry_id=?")
        .bind(entry)
        .execute(&pool)
        .await
        .unwrap();
    let journal = JournalRepository::new(pool.clone());
    assert!(
        journal
            .post_draft(JournalEntryId::new(entry), 1)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM journal_entries WHERE id=?")
            .bind(entry)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "draft"
    );
    assert_eq!(repo.list("2026-10-23", "2026-10-23").await.unwrap().1, 1000);
    verify_database_integrity(&pool).await.unwrap();
}
#[tokio::test]
async fn verified_snapshot_retains_payment_plan_audit_and_posted_link() {
    use kaccounting_backend::{
        accounting::{
            journal::{JournalEntryId, JournalRepository},
            payment_memo_drafts::create_payment_draft,
        },
        db::{backup_database, open_database, verify_database_snapshot},
    };
    let folder = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "ka-payment-snapshot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&folder).unwrap();
    let source = folder.join("source.sqlite");
    let pool = open_database(&source).await.unwrap();
    let repo = PaymentMemoRepository::new(pool.clone());
    let memo = repo.create(input(1000)).await.unwrap();
    let (debit, asset) = payment_accounts(&pool).await;
    let entry = create_payment_draft(&pool, memo.id, 1, "2026-10-23", debit, asset, 250)
        .await
        .unwrap();
    JournalRepository::new(pool.clone())
        .post_draft(JournalEntryId::new(entry), 1)
        .await
        .unwrap();
    let path = folder.join("snapshot.sqlite");
    backup_database(&pool, &path).await.unwrap();
    verify_database_snapshot(&path).await.unwrap();
    let restored = open_database(&path).await.unwrap();
    let (items, total) = PaymentMemoRepository::new(restored.clone())
        .list("2026-10-23", "2026-10-23")
        .await
        .unwrap();
    assert_eq!(total, 750);
    assert_eq!(items[0].paid_amount_minor, 250);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payment_memo_history")
            .fetch_one(&restored)
            .await
            .unwrap(),
        1
    );
    restored.close().await;
    pool.close().await;
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(&path).unwrap();
    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if sidecar.exists() {
            std::fs::remove_file(sidecar).unwrap();
        }
    }
    std::fs::remove_dir(&folder).unwrap();
}
