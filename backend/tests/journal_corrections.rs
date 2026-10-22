use kaccounting_backend::accounting::accounts::{
    Account, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalError, JournalLineInput, JournalRepository, JournalSide, PostCorrection,
    PostEntry, PostedEntryRelationship,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::db::{open_memory_database, verify_database_integrity};

async fn posting_account(
    accounts: &AccountRepository,
    name: &str,
    element: AccountingElement,
) -> Account {
    accounts
        .create(NewAccount::new(None, name, element, AccountKind::Posting))
        .await
        .expect("posting account should be created")
}

fn line(account: &Account, side: JournalSide, amount: i64) -> JournalLineInput {
    JournalLineInput::new(account.id, side, MinorUnits::new(amount))
}

#[tokio::test]
async fn reversal_is_an_exact_immutable_side_exchange() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 12_345),
                line(&revenue, JournalSide::Credit, 12_345),
            ],
        ))
        .await
        .unwrap();

    let reversal = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-22").unwrap())
        .await
        .expect("valid reversal should post");

    assert_eq!(
        reversal.relationship,
        PostedEntryRelationship::Reversal {
            original_entry_id: original.id
        }
    );
    assert_eq!(reversal.explanation, None);
    assert_eq!(reversal.lines.len(), original.lines.len());
    for (reversal_line, original_line) in reversal.lines.iter().zip(&original.lines) {
        assert_eq!(reversal_line.line_no, original_line.line_no);
        assert_eq!(reversal_line.account_id, original_line.account_id);
        assert_eq!(reversal_line.amount, original_line.amount);
        assert_ne!(reversal_line.side, original_line.side);
    }
    assert_eq!(journal.get_posted(reversal.id).await.unwrap(), reversal);

    let mutation = sqlx::query("UPDATE journal_entries SET reverses_entry_id = NULL WHERE id = ?")
        .bind(reversal.id.get())
        .execute(&pool)
        .await
        .expect_err("posted relationship must be immutable");
    let message = mutation.to_string();
    assert!(
        message.contains("posted journal entry is immutable")
            || message.contains("journal entry relationship is immutable")
    );
}

#[tokio::test]
async fn reversal_policy_failures_leave_no_partial_entry() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&capital, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let early = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-19").unwrap())
        .await
        .expect_err("reversal cannot predate the original");
    assert!(matches!(
        early,
        JournalError::ReversalDateBeforeOriginal { .. }
    ));

    let reversal = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-20").unwrap())
        .await
        .unwrap();
    let duplicate = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-21").unwrap())
        .await
        .expect_err("only one direct reversal is allowed");
    assert!(matches!(
        duplicate,
        JournalError::EntryAlreadyReversed(id) if id == original.id.get()
    ));
    let nested = journal
        .reverse_posted(reversal.id, JournalDate::parse("2026-10-21").unwrap())
        .await
        .expect_err("a reversal cannot itself be reversed");
    assert!(matches!(
        nested,
        JournalError::CannotReverseReversal(id) if id == reversal.id.get()
    ));

    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM journal_entries), (SELECT COUNT(*) FROM journal_lines)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (2, 4));
}

#[tokio::test]
async fn reversal_requires_explicit_account_reactivation() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&revenue, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let inactive = accounts
        .set_active(revenue.id, revenue.revision, false)
        .await
        .unwrap();

    let blocked = journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-21").unwrap())
        .await
        .expect_err("inactive original account must block reversal");
    assert!(matches!(
        blocked,
        JournalError::AccountInactive(id) if id == revenue.id.get()
    ));
    let entry_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(entry_count, 1);

    accounts
        .set_active(inactive.id, inactive.revision, true)
        .await
        .unwrap();
    journal
        .reverse_posted(original.id, JournalDate::parse("2026-10-21").unwrap())
        .await
        .expect("explicit reactivation should allow reversal");
}

#[tokio::test]
async fn correction_requires_an_explanation_and_keeps_a_permanent_link() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&revenue, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let correction_entry = || {
        PostEntry::new(
            JournalDate::parse("2026-10-18").unwrap(),
            vec![
                line(&cash, JournalSide::Credit, 25),
                line(&revenue, JournalSide::Debit, 25),
            ],
        )
    };

    for invalid in ["", "  \n "] {
        let error = journal
            .post_correction(PostCorrection::new(
                original.id,
                correction_entry(),
                invalid,
            ))
            .await
            .expect_err("blank correction explanation must be rejected");
        assert!(matches!(error, JournalError::CorrectionExplanationRequired));
    }
    let correction = journal
        .post_correction(PostCorrection::new(
            original.id,
            correction_entry(),
            "將誤記金額減少 NT$25",
        ))
        .await
        .expect("balanced correction should post");
    assert_eq!(
        correction.relationship,
        PostedEntryRelationship::Correction {
            original_entry_id: original.id
        }
    );
    assert_eq!(
        correction.explanation.as_deref(),
        Some("將誤記金額減少 NT$25")
    );
    assert_eq!(journal.get_posted(correction.id).await.unwrap(), correction);
}

#[tokio::test]
async fn sqlite_independently_rejects_malformed_relationships() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&revenue, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let both = sqlx::query(
        "INSERT INTO journal_entries (entry_date, reverses_entry_id, corrects_entry_id, explanation) VALUES ('2026-10-21', ?, ?, 'x')",
    )
    .bind(original.id.get())
    .bind(original.id.get())
    .execute(&pool)
    .await
    .expect_err("one entry cannot carry both relationships");
    assert!(
        both.to_string()
            .contains("both a reversal and a correction")
    );

    let blank = sqlx::query(
        "INSERT INTO journal_entries (entry_date, corrects_entry_id, explanation) VALUES ('2026-10-21', ?, '   ')",
    )
    .bind(original.id.get())
    .execute(&pool)
    .await
    .expect_err("SQLite must require correction explanation");
    assert!(blank.to_string().contains("requires an explanation"));

    let malformed_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date, reverses_entry_id) VALUES ('2026-10-21', ?) RETURNING id",
    )
    .bind(original.id.get())
    .fetch_one(&pool)
    .await
    .unwrap();
    for (line_no, account, side) in [(1_i64, &cash, "debit"), (2, &revenue, "credit")] {
        sqlx::query(
            "INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor) VALUES (?, ?, ?, ?, 100)",
        )
        .bind(malformed_id)
        .bind(line_no)
        .bind(account.id.get())
        .bind(side)
        .execute(&pool)
        .await
        .unwrap();
    }
    let malformed = sqlx::query(
        "UPDATE journal_entries SET status = 'posted', posted_at = '2026-10-21T00:00:00.000Z', revision = revision + 1 WHERE id = ?",
    )
    .bind(malformed_id)
    .execute(&pool)
    .await
    .expect_err("SQLite must reject a reversal that does not exchange sides");
    assert!(malformed.to_string().contains("exactly exchange"));
}

#[tokio::test]
async fn integrity_audit_rejects_a_committed_relationship_draft() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-20").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&capital, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let stranded_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date, reverses_entry_id) VALUES ('2026-10-21', ?) RETURNING id",
    )
    .bind(original.id.get())
    .fetch_one(&pool)
    .await
    .unwrap();

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a committed relationship may not remain an editable draft");
    assert!(error.to_string().contains(&format!(
        "journal entry {stranded_id} has an invalid reversal or correction relationship"
    )));
}

#[tokio::test]
async fn sqlite_versions_ordinary_draft_explanation_updates() {
    let pool = open_memory_database().await.expect("database should open");
    let draft_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let unversioned =
        sqlx::query("UPDATE journal_entries SET explanation = '草稿說明' WHERE id = ?")
            .bind(draft_id)
            .execute(&pool)
            .await
            .expect_err("draft metadata changes must advance the revision");
    assert!(
        unversioned
            .to_string()
            .contains("revision must increase by one")
    );

    sqlx::query(
        "UPDATE journal_entries SET explanation = '草稿說明', revision = revision + 1 WHERE id = ?",
    )
    .bind(draft_id)
    .execute(&pool)
    .await
    .expect("a versioned ordinary draft explanation change should remain possible");
    let stored: (Option<String>, i64) =
        sqlx::query_as("SELECT explanation, revision FROM journal_entries WHERE id = ?")
            .bind(draft_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, (Some("草稿說明".to_owned()), 2));
}
