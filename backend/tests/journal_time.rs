use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, JournalTime,
            PostCorrection, PostEntry, SaveDraftEntry,
        },
        money::MinorUnits,
        reports::ReportRepository,
    },
    db::{open_memory_database, verify_database_integrity},
};
fn date() -> JournalDate {
    JournalDate::parse("2026-10-22").unwrap()
}
fn time(value: &str) -> Option<JournalTime> {
    Some(JournalTime::parse(value).unwrap())
}
async fn fixture() -> (sqlx::SqlitePool, Vec<JournalLineInput>) {
    let pool = open_memory_database().await.unwrap();
    let accounts = AccountRepository::new(pool.clone());
    let mut lines = vec![];
    for (name, element, side) in [
        ("Cash", AccountingElement::Asset, JournalSide::Debit),
        ("Equity", AccountingElement::Equity, JournalSide::Credit),
    ] {
        let account = accounts
            .create(NewAccount::new(None, name, element, AccountKind::Posting))
            .await
            .unwrap();
        lines.push(JournalLineInput::new(
            account.id,
            side,
            MinorUnits::new(100),
        ));
    }
    (pool, lines)
}
#[test]
fn journal_time_requires_exact_existing_hours_minutes_seconds() {
    for value in ["00:00:00", "23:59:59", "12:34:56"] {
        assert_eq!(JournalTime::parse(value).unwrap().as_str(), value);
    }
    for value in [
        "",
        "1:02:03",
        "12:34",
        "24:00:00",
        "23:60:00",
        "23:59:60",
        "12:30:00Z",
        "12:30:00+08:00",
        "12:30:00.1",
        "12:30:00\0",
        "１２:30:00",
        "ab:cd:ef",
    ] {
        assert!(JournalTime::parse(value).is_err(), "{value}");
    }
}
#[tokio::test]
async fn time_roundtrips_draft_replace_post_reports_correction_and_reversal() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(date(), lines.clone()).with_time(time("23:59:59")))
        .await
        .unwrap();
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);
    assert_eq!(journal.list_drafts().await.unwrap(), vec![draft.clone()]);
    let replacement = journal
        .replace_draft(
            draft.id,
            draft.revision,
            SaveDraftEntry::new(date(), lines.clone()).with_time(time("00:00:00")),
        )
        .await
        .unwrap();
    assert_eq!(replacement.entry_time, time("00:00:00"));
    let posted = journal
        .post_draft(draft.id, replacement.revision)
        .await
        .unwrap();
    assert_eq!(posted.entry_time, time("00:00:00"));
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
    let reports = ReportRepository::new(pool.clone());
    let report = reports.general_journal(date(), date()).await.unwrap();
    assert_eq!(report.entries[0].entry_time, posted.entry_time);
    let ledger = reports
        .general_ledger(lines[0].account_id, date())
        .await
        .unwrap();
    assert_eq!(ledger.lines[0].entry_time, posted.entry_time);
    let ledger = reports
        .period_general_ledger(lines[0].account_id, date(), date())
        .await
        .unwrap();
    assert_eq!(ledger.lines[0].entry_time, posted.entry_time);
    let corrected = journal
        .post_correction(PostCorrection::new(
            posted.id,
            PostEntry::new(date(), lines.clone()).with_time(time("12:34:56")),
            "explicit correction",
        ))
        .await
        .unwrap();
    assert_eq!(corrected.entry_time, time("12:34:56"));
    let reversed = journal
        .reverse_posted_at(posted.id, date(), time("13:02:01"))
        .await
        .unwrap();
    assert_eq!(reversed.entry_time, time("13:02:01"));
    assert_eq!(
        journal
            .reverse_posted(corrected.id, date())
            .await
            .unwrap()
            .entry_time,
        None
    );
    assert_eq!(
        journal
            .post(PostEntry::new(date(), lines))
            .await
            .unwrap()
            .entry_time,
        None
    );
    verify_database_integrity(&pool).await.unwrap();
}
#[tokio::test]
async fn sqlite_time_validation_versioning_and_posted_immutability_are_enforced() {
    let (pool, lines) = fixture().await;
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(date(), lines.clone()).with_time(time("10:11:12")))
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE journal_entries SET entry_time = '11:12:13' WHERE id = ?")
            .bind(draft.id.get())
            .execute(&pool)
            .await
            .is_err()
    );
    for value in [
        "24:00:00",
        "12:60:00",
        "12:00:60",
        "12:30",
        "12:30:00Z",
        "12:30:00\0x",
    ] {
        assert!(
            sqlx::query(
                "UPDATE journal_entries SET entry_time = ?, revision = revision + 1 WHERE id = ?"
            )
            .bind(value)
            .bind(draft.id.get())
            .execute(&pool)
            .await
            .is_err(),
            "{value}"
        );
    }
    let cleared = journal
        .replace_draft(draft.id, draft.revision, SaveDraftEntry::new(date(), lines))
        .await
        .unwrap();
    assert_eq!(cleared.entry_time, None);
    let posted = journal
        .post_draft(draft.id, cleared.revision)
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE journal_entries SET entry_time = '11:12:13', revision = revision + 1 WHERE id = ?").bind(posted.id.get()).execute(&pool).await.is_err());
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
}
