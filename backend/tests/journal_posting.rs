use kaccounting_backend::accounting::accounts::{
    Account, AccountError, AccountId, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalError, JournalLineInput, JournalRepository, JournalSide, PostEntry,
    SaveDraftEntry,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::db::{open_database, open_memory_database};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

fn line(account_id: AccountId, side: JournalSide, amount: i64) -> JournalLineInput {
    JournalLineInput::new(account_id, side, MinorUnits::new(amount))
}

async fn posting_account(
    accounts: &AccountRepository,
    name: &str,
    element: AccountingElement,
) -> Account {
    accounts
        .create(NewAccount::new(None, name, element, AccountKind::Posting))
        .await
        .expect("test posting account should be created")
}

async fn row_counts(pool: &SqlitePool) -> (i64, i64) {
    let entries = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(pool)
        .await
        .expect("journal entry count should be readable");
    let lines = sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines")
        .fetch_one(pool)
        .await
        .expect("journal line count should be readable");
    (entries, lines)
}

#[test]
fn journal_date_accepts_only_real_four_digit_calendar_dates() {
    for valid in ["0001-01-01", "2000-02-29", "2026-10-22", "9999-12-31"] {
        assert_eq!(
            JournalDate::parse(valid)
                .expect("date should be valid")
                .as_str(),
            valid
        );
    }

    for invalid in [
        "",
        "2026-1-01",
        "0000-01-01",
        "1900-02-29",
        "2025-02-29",
        "2024-02-30",
        "2026-04-31",
        "2026-13-01",
        "2026-00-10",
        "2026-01-00",
        "２０２６-01-01",
    ] {
        assert!(
            matches!(JournalDate::parse(invalid), Err(JournalError::InvalidDate(value)) if value == invalid),
            "{invalid} should be rejected"
        );
    }
}

#[tokio::test]
async fn balanced_entry_posts_atomically_with_exact_minor_units() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "投入資本", AccountingElement::Equity).await;

    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 9_007_199_254_740_993),
                line(capital.id, JournalSide::Credit, 9_007_199_254_740_993),
            ],
        ))
        .await
        .expect("balanced entry should post");

    let stored_entry: (String, String, Option<String>) =
        sqlx::query_as("SELECT entry_date, status, posted_at FROM journal_entries WHERE id = ?")
            .bind(posted.id.get())
            .fetch_one(&pool)
            .await
            .expect("posted entry should be stored");
    let stored_lines: Vec<(i64, i64, String, i64)> = sqlx::query_as(
        r#"
        SELECT line_no, account_id, side, amount_minor
        FROM journal_lines
        WHERE entry_id = ?
        ORDER BY line_no
        "#,
    )
    .bind(posted.id.get())
    .fetch_all(&pool)
    .await
    .expect("posted lines should be stored");

    assert_eq!(posted.lines.len(), 2);
    assert_eq!(posted.lines[0].line_no, 1);
    assert_eq!(posted.lines[1].line_no, 2);
    assert_eq!(stored_entry.0, "2026-10-22");
    assert_eq!(stored_entry.1, "posted");
    assert_eq!(stored_entry.2.as_deref(), Some(posted.posted_at.as_str()));
    assert_eq!(
        stored_lines,
        vec![
            (1, cash.id.get(), "debit".to_owned(), 9_007_199_254_740_993,),
            (
                2,
                capital.id.get(),
                "credit".to_owned(),
                9_007_199_254_740_993,
            ),
        ]
    );
}

#[tokio::test]
async fn posted_entry_round_trips_through_a_consistent_read_snapshot() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "投入資本", AccountingElement::Equity).await;
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 12_345),
                line(capital.id, JournalSide::Credit, 12_345),
            ],
        ))
        .await
        .expect("entry should post");

    let reloaded = journal
        .get_posted(posted.id)
        .await
        .expect("posted entry should reload");

    assert_eq!(reloaded, posted);
}

#[tokio::test]
async fn posted_entry_lookup_distinguishes_missing_and_draft_rows() {
    let pool = open_memory_database().await.expect("database should open");
    let journal = JournalRepository::new(pool.clone());
    let missing = journal
        .get_posted(kaccounting_backend::accounting::journal::JournalEntryId::new(999_999))
        .await
        .expect_err("missing entry should be explicit");
    assert!(matches!(missing, JournalError::EntryNotFound(999_999)));

    let draft_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be created");
    let draft = journal
        .get_posted(kaccounting_backend::accounting::journal::JournalEntryId::new(draft_id))
        .await
        .expect_err("draft should not masquerade as a posted entry");
    assert!(matches!(draft, JournalError::EntryNotPosted(id) if id == draft_id));
}

#[tokio::test]
async fn arithmetic_validation_failure_writes_nothing() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 101),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("unbalanced entry must fail");

    assert!(matches!(error, JournalError::Validation(_)));
    assert_eq!(row_counts(&pool).await, (0, 0));
}

#[tokio::test]
async fn account_eligibility_failure_rolls_back_the_staged_draft() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let inactive = posting_account(&accounts, "停用收入", AccountingElement::Income).await;
    accounts
        .set_active(inactive.id, inactive.revision, false)
        .await
        .expect("test account should be deactivated");

    let error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(inactive.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("inactive account must reject posting");

    assert!(matches!(error, JournalError::AccountInactive(id) if id == inactive.id.get()));
    assert_eq!(row_counts(&pool).await, (0, 0));
}

#[tokio::test]
async fn missing_and_category_accounts_are_rejected_without_partial_rows() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let category = accounts
        .create(NewAccount::new(
            None,
            "收入分類",
            AccountingElement::Income,
            AccountKind::Category,
        ))
        .await
        .expect("category should be created");

    let category_error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(category.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("category account must reject posting");
    assert!(
        matches!(category_error, JournalError::AccountNotPosting(id) if id == category.id.get())
    );
    assert_eq!(row_counts(&pool).await, (0, 0));

    let missing_id = AccountId::new(999_999);
    let missing_error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(missing_id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("missing account must reject posting");
    assert!(matches!(
        missing_error,
        JournalError::AccountNotFound(999_999)
    ));
    assert_eq!(row_counts(&pool).await, (0, 0));
}

#[tokio::test]
async fn sqlite_rejects_invalid_dates_and_direct_posted_inserts() {
    let pool = open_memory_database().await.expect("database should open");

    for invalid_date in ["2025-02-29", "2026-04-31", "2026-1-01"] {
        let error = sqlx::query("INSERT INTO journal_entries (entry_date) VALUES (?)")
            .bind(invalid_date)
            .execute(&pool)
            .await
            .expect_err("SQLite date constraint must reject invalid date");
        assert!(error.to_string().contains("CHECK constraint failed"));
    }

    let error = sqlx::query(
        r#"
        INSERT INTO journal_entries (entry_date, status, posted_at)
        VALUES ('2026-10-22', 'posted', '2026-10-22T00:00:00.000Z')
        "#,
    )
    .execute(&pool)
    .await
    .expect_err("direct posted insert must be rejected");
    assert!(
        error
            .to_string()
            .contains("journal entry must be inserted as a draft")
    );
    assert_eq!(row_counts(&pool).await, (0, 0));
}

#[tokio::test]
async fn sqlite_constraints_independently_reject_ineligible_and_unbalanced_lines() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let category = accounts
        .create(NewAccount::new(
            None,
            "分類",
            AccountingElement::Expense,
            AccountKind::Category,
        ))
        .await
        .expect("category should be created");
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");

    let category_error = sqlx::query(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        "#,
    )
    .bind(entry_id)
    .bind(category.id.get())
    .execute(&pool)
    .await
    .expect_err("category line should be blocked by SQLite");
    assert!(
        category_error
            .to_string()
            .contains("journal line account must be active and posting")
    );

    for (line_no, account_id, side, amount) in
        [(1, cash.id, "debit", 101), (2, revenue.id, "credit", 100)]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id.get())
        .bind(side)
        .bind(amount)
        .execute(&pool)
        .await
        .expect("eligible draft line should be inserted");
    }

    let post_error = sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect_err("SQLite must reject an unbalanced transition");
    assert!(
        post_error
            .to_string()
            .contains("journal entry is not balanced")
    );

    let status: String = sqlx::query_scalar("SELECT status FROM journal_entries WHERE id = ?")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("draft status should remain readable");
    assert_eq!(status, "draft");
}

#[tokio::test]
async fn sqlite_independently_requires_two_effective_lines() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");
    sqlx::query(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        "#,
    )
    .bind(entry_id)
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect("eligible draft line should be inserted");

    let error = sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect_err("one-line posting must be rejected by SQLite");

    assert!(
        error
            .to_string()
            .contains("posted journal entry requires at least two lines")
    );
}

#[tokio::test]
async fn sqlite_rejects_posting_when_a_side_total_overflows() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");

    for (line_no, account_id, side) in [
        (1_i64, cash.id, "debit"),
        (2, cash.id, "debit"),
        (3, capital.id, "credit"),
        (4, capital.id, "credit"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id.get())
        .bind(side)
        .bind(i64::MAX)
        .execute(&pool)
        .await
        .expect("each exact integer line should fit independently");
    }

    let error = sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect_err("an overflowing side total must not be posted");
    assert!(error.to_string().contains("integer overflow"));

    let state: (String, Option<String>, i64) =
        sqlx::query_as("SELECT status, posted_at, revision FROM journal_entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .expect("rejected draft should remain readable");
    assert_eq!(state, ("draft".to_owned(), None, 1));
    assert_eq!(row_counts(&pool).await, (1, 4));
}

#[tokio::test]
async fn posted_entries_and_lines_cannot_be_updated_or_deleted() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("entry should post");

    let attempts = [
        sqlx::query("UPDATE journal_entries SET entry_date = '2026-10-23' WHERE id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await,
        sqlx::query("DELETE FROM journal_entries WHERE id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET amount_minor = 101 WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query("DELETE FROM journal_lines WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
    ];

    for attempt in attempts {
        assert!(attempt.is_err(), "posted data mutation must be rejected");
    }
    assert_eq!(row_counts(&pool).await, (1, 2));
}

#[tokio::test]
async fn sqlite_keeps_draft_entry_and_line_identity_fields_immutable() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let first_entry: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("first draft should be inserted");
    let second_entry: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-23') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("second draft should be inserted");
    let line_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        RETURNING id
        "#,
    )
    .bind(first_entry)
    .bind(cash.id.get())
    .fetch_one(&pool)
    .await
    .expect("draft line should be inserted");

    let attempts = [
        sqlx::query("UPDATE journal_entries SET id = id + 100 WHERE id = ?")
            .bind(first_entry)
            .execute(&pool)
            .await,
        sqlx::query(
            "UPDATE journal_entries SET created_at = '2000-01-01T00:00:00.000Z' WHERE id = ?",
        )
        .bind(first_entry)
        .execute(&pool)
        .await,
        sqlx::query("UPDATE journal_lines SET id = id + 100 WHERE id = ?")
            .bind(line_id)
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET entry_id = ? WHERE id = ?")
            .bind(second_entry)
            .bind(line_id)
            .execute(&pool)
            .await,
        sqlx::query(
            "UPDATE journal_lines SET created_at = '2000-01-01T00:00:00.000Z' WHERE id = ?",
        )
        .bind(line_id)
        .execute(&pool)
        .await,
    ];

    for attempt in attempts {
        let error = attempt.expect_err("journal identity mutation must be rejected");
        assert!(error.to_string().contains("identity is immutable"));
    }
}

#[tokio::test]
async fn deleting_a_draft_cascades_lines_without_reusing_identifiers() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let first_entry: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("first draft should be inserted");
    let first_line: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        RETURNING id
        "#,
    )
    .bind(first_entry)
    .bind(cash.id.get())
    .fetch_one(&pool)
    .await
    .expect("first draft line should be inserted");
    sqlx::query("DELETE FROM journal_entries WHERE id = ?")
        .bind(first_entry)
        .execute(&pool)
        .await
        .expect("draft should be deletable");
    assert_eq!(row_counts(&pool).await, (0, 0));

    let second_entry: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-23') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("second draft should be inserted");
    let second_line: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        RETURNING id
        "#,
    )
    .bind(second_entry)
    .bind(cash.id.get())
    .fetch_one(&pool)
    .await
    .expect("second draft line should be inserted");

    assert!(second_entry > first_entry);
    assert!(second_line > first_line);
}

#[tokio::test]
async fn posting_history_prevents_account_deletion() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("entry should post");
    journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 50)],
        ))
        .await
        .expect("a later editable draft may reference the same account");

    let error = accounts
        .delete(cash.id, cash.revision)
        .await
        .expect_err("immutable posting history must remain the primary deletion reason");

    assert!(matches!(error, AccountError::HasJournalHistory));
}

#[tokio::test]
async fn draft_reference_prevents_account_deletion_until_the_draft_is_deleted() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .expect("an intentionally incomplete draft should be saved");

    let referenced_error = accounts
        .delete(cash.id, cash.revision)
        .await
        .expect_err("an account referenced by a draft must be preserved");
    assert!(matches!(referenced_error, AccountError::HasDraftReferences));

    journal
        .delete_draft(draft.id, draft.revision)
        .await
        .expect("the user may delete the editable draft");
    accounts
        .delete(cash.id, cash.revision)
        .await
        .expect("the unused account may be deleted after its draft reference is gone");
}

#[tokio::test]
async fn history_freezes_classification_but_allows_rename_and_deactivation() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("entry should post");

    let kind_error = accounts
        .change_kind(cash.id, cash.revision, AccountKind::Category)
        .await
        .expect_err("journal history should freeze account kind");
    assert!(matches!(
        kind_error,
        AccountError::JournalHistoryLocksClassification
    ));

    let element_error = accounts
        .change_element(cash.id, cash.revision, AccountingElement::Expense)
        .await
        .expect_err("journal history should freeze account element");
    assert!(matches!(
        element_error,
        AccountError::JournalHistoryLocksClassification
    ));

    let renamed = accounts
        .rename(cash.id, cash.revision, "手邊現金")
        .await
        .expect("display name may change without changing permanent references");
    let inactive = accounts
        .set_active(renamed.id, renamed.revision, false)
        .await
        .expect("historical account may be closed to new posting");
    assert_eq!(inactive.name, "手邊現金");
    assert!(!inactive.is_active);
}

#[tokio::test]
async fn concurrent_deactivation_and_posting_serialize_without_partial_entries() {
    let database_path = unique_database_path("post-deactivate-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let post = journal.post(PostEntry::new(
        JournalDate::parse("2026-10-22").unwrap(),
        vec![
            line(cash.id, JournalSide::Debit, 100),
            line(revenue.id, JournalSide::Credit, 100),
        ],
    ));
    let deactivate = accounts.set_active(revenue.id, revenue.revision, false);
    let (post_result, deactivate_result) = tokio::join!(post, deactivate);

    let deactivated = deactivate_result.expect("deactivation should serialize and succeed");
    assert!(!deactivated.is_active);
    match post_result {
        Ok(_) => assert_eq!(row_counts(&pool).await, (1, 2)),
        Err(JournalError::AccountInactive(id)) if id == revenue.id.get() => {
            assert_eq!(row_counts(&pool).await, (0, 0));
        }
        Err(error) => panic!("unexpected posting result: {error}"),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

fn unique_database_path(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "kAccounting-Extra-01-journal-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

fn remove_sqlite_files(database_path: &std::path::Path) {
    for path in [
        database_path.to_path_buf(),
        database_path.with_extension("sqlite3-wal"),
        database_path.with_extension("sqlite3-shm"),
        database_path.with_extension("sqlite3-journal"),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .unwrap_or_else(|error| panic!("failed to remove {}: {error}", path.display()));
        }
    }
}
