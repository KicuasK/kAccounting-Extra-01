use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalError, JournalLineInput, JournalRepository, JournalSide, PostEntry,
            SaveDraftEntry,
        },
        money::MinorUnits,
        reports::ReportRepository,
    },
    db::{open_memory_database, verify_database_integrity},
};

async fn fixture() -> (sqlx::SqlitePool, Vec<JournalLineInput>) {
    let pool = open_memory_database().await.unwrap();
    let accounts = AccountRepository::new(pool.clone());
    let mut lines = Vec::new();
    for (name, element, side) in [
        ("銀行 Bank 01", AccountingElement::Asset, JournalSide::Debit),
        (
            "Opening equity",
            AccountingElement::Equity,
            JournalSide::Credit,
        ),
    ] {
        let account = accounts
            .create(NewAccount::new(None, name, element, AccountKind::Posting))
            .await
            .unwrap();
        lines.push(
            JournalLineInput::new(account.id, side, MinorUnits::new(9007199254740993))
                .with_memo(format!("{name} 明細\nsecond line")),
        );
    }
    (pool, lines)
}

fn date() -> JournalDate {
    JournalDate::parse("2026-10-22").unwrap()
}

#[tokio::test]
async fn explicit_save_replace_post_and_reports_preserve_metadata() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(
            SaveDraftEntry::new(date(), lines.clone())
                .with_header(Some("收據 01-A".into()), Some("開帳\n說明".into())),
        )
        .await
        .unwrap();
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);
    assert_eq!(journal.list_drafts().await.unwrap(), vec![draft.clone()]);
    let replacement = journal
        .replace_draft(
            draft.id,
            draft.revision,
            SaveDraftEntry::new(date(), lines.clone())
                .with_header(Some("ref 02".into()), Some("revised explanation".into())),
        )
        .await
        .unwrap();
    assert_eq!(replacement.revision, 2);
    assert!(matches!(
        journal
            .replace_draft(draft.id, 1, SaveDraftEntry::new(date(), vec![]))
            .await,
        Err(JournalError::StaleDraftRevision { .. })
    ));
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), replacement);
    let posted = journal
        .post_draft(draft.id, replacement.revision)
        .await
        .unwrap();
    assert_eq!(posted.reference.as_deref(), Some("ref 02"));
    assert_eq!(posted.explanation.as_deref(), Some("revised explanation"));
    assert_eq!(posted.lines[0].memo, lines[0].memo);
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
    let report = ReportRepository::new(pool.clone())
        .general_journal(date(), date())
        .await
        .unwrap();
    assert_eq!(report.entries[0].reference, posted.reference);
    assert_eq!(report.entries[0].explanation, posted.explanation);
    assert_eq!(report.entries[0].lines[0].memo, posted.lines[0].memo);
    assert_eq!(report.total_debit.get(), 9007199254740993);
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn clearing_optional_metadata_is_explicit_and_atomic() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(
            SaveDraftEntry::new(date(), lines.clone())
                .with_header(Some("A".into()), Some("B".into())),
        )
        .await
        .unwrap();
    let mut bad = lines.clone();
    bad[0].memo = Some("bad\0memo".into());
    assert!(matches!(
        journal
            .replace_draft(draft.id, 1, SaveDraftEntry::new(date(), bad))
            .await,
        Err(JournalError::InvalidMetadata)
    ));
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);
    let cleared = journal
        .replace_draft(draft.id, 1, SaveDraftEntry::new(date(), lines))
        .await
        .unwrap();
    assert_eq!(cleared.reference, None);
    assert_eq!(cleared.explanation, None);
}

#[tokio::test]
async fn metadata_nulls_fail_without_partial_writes() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    for (reference, explanation) in [
        (Some("bad\0ref".into()), None),
        (None, Some("bad\0explanation".into())),
    ] {
        assert!(
            journal
                .create_draft(
                    SaveDraftEntry::new(date(), lines.clone())
                        .with_header(reference.clone(), explanation.clone())
                )
                .await
                .is_err()
        );
        assert!(
            journal
                .post(PostEntry::new(date(), lines.clone()).with_header(reference, explanation))
                .await
                .is_err()
        );
    }
    let mut bad = lines;
    bad[1].memo = Some("bad\0memo".into());
    assert!(journal.post(PostEntry::new(date(), bad)).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn database_blocks_posted_metadata_mutation_and_unversioned_draft_changes() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(date(), lines))
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE journal_entries SET reference = 'new' WHERE id = ?")
            .bind(draft.id.get())
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "UPDATE journal_entries SET reference = ? , revision = revision + 1 WHERE id = ?"
        )
        .bind("bad\0reference")
        .bind(draft.id.get())
        .execute(&pool)
        .await
        .is_err()
    );
    let posted = journal.post_draft(draft.id, 1).await.unwrap();
    assert!(
        sqlx::query(
            "UPDATE journal_entries SET reference = 'new', revision = revision + 1 WHERE id = ?"
        )
        .bind(posted.id.get())
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("UPDATE journal_lines SET memo = 'changed' WHERE entry_id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
}

#[tokio::test]
async fn conflicting_correction_explanations_are_rejected_instead_of_silently_discarded() {
    use kaccounting_backend::accounting::journal::PostCorrection;
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let original = journal
        .post(PostEntry::new(date(), lines.clone()))
        .await
        .unwrap();
    let request = PostCorrection::new(
        original.id,
        PostEntry::new(date(), lines).with_header(None, Some("header explanation".into())),
        "different correction explanation",
    );
    assert!(matches!(
        journal.post_correction(request).await,
        Err(JournalError::ConflictingExplanation)
    ));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(journal.get_posted(original.id).await.unwrap(), original);
}
