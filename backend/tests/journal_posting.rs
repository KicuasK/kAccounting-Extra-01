use kaccounting_backend::accounting::accounts::{
    Account, AccountError, AccountId, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalError, JournalLineInput, JournalRepository, JournalSide,
    JournalValidationError, PostEntry, SaveDraftEntry,
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
                line(cash.id, JournalSide::Debit, i64::MAX),
                line(capital.id, JournalSide::Credit, i64::MAX),
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
            (1, cash.id.get(), "debit".to_owned(), i64::MAX),
            (2, capital.id.get(), "credit".to_owned(), i64::MAX),
        ]
    );
}

#[tokio::test]
async fn large_compound_entry_preserves_exact_totals_and_line_order() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let mut lines = Vec::with_capacity(512);
    lines.extend((0..256).map(|_| line(cash.id, JournalSide::Debit, 1)));
    lines.extend((0..256).map(|_| line(revenue.id, JournalSide::Credit, 1)));

    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            lines,
        ))
        .await
        .expect("a large exactly balanced compound entry should post atomically");

    assert_eq!(posted.lines.len(), 512);
    for (index, stored) in posted.lines.iter().enumerate() {
        assert_eq!(stored.line_no, i64::try_from(index + 1).unwrap());
        assert_eq!(stored.amount, MinorUnits::new(1));
        let expected_side = if index < 256 {
            JournalSide::Debit
        } else {
            JournalSide::Credit
        };
        assert_eq!(stored.side, expected_side);
    }
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
}

#[tokio::test]
async fn opening_balances_use_one_balanced_posted_entry() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "期初現金", AccountingElement::Asset).await;
    let opening_equity = posting_account(&accounts, "期初權益", AccountingElement::Equity).await;

    let rejected = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 50_000),
                line(opening_equity.id, JournalSide::Credit, 49_999),
            ],
        ))
        .await
        .expect_err("an opening amount cannot bypass balanced-entry validation");
    assert!(matches!(rejected, JournalError::Validation(_)));
    assert_eq!(row_counts(&pool).await, (0, 0));

    let opening = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 50_000),
                line(opening_equity.id, JournalSide::Credit, 50_000),
            ],
        ))
        .await
        .expect("opening balances should post through the normal balanced journal boundary");

    assert_eq!(opening.lines.len(), 2);
    assert_eq!(row_counts(&pool).await, (1, 2));
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
async fn posted_entry_read_rejects_non_contiguous_stored_line_numbers() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "投入資本", AccountingElement::Equity).await;
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (line_no, account_id, side) in [(2_i64, cash.id, "debit"), (3_i64, capital.id, "credit")] {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, 100)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id.get())
        .bind(side)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
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
    .expect("test setup demonstrates the storage-level ordering gap");

    let error = journal
        .get_posted(kaccounting_backend::accounting::journal::JournalEntryId::new(entry_id))
        .await
        .expect_err("posted reads must fail closed on non-contiguous line numbers");
    assert!(
        matches!(error, JournalError::InvalidStoredValue(message) if message.contains("expected line number 1, found 2"))
    );
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
async fn every_pure_posting_guard_preserves_existing_database_state() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let existing = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-21").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 50),
                line(revenue.id, JournalSide::Credit, 50),
            ],
        ))
        .await
        .unwrap();

    let invalid_cases = [
        ("empty", vec![]),
        ("single", vec![line(cash.id, JournalSide::Debit, 100)]),
        (
            "zero",
            vec![
                line(cash.id, JournalSide::Debit, 0),
                line(revenue.id, JournalSide::Credit, 1),
            ],
        ),
        (
            "negative",
            vec![
                line(cash.id, JournalSide::Debit, -1),
                line(revenue.id, JournalSide::Credit, 1),
            ],
        ),
        (
            "unbalanced",
            vec![
                line(cash.id, JournalSide::Debit, 101),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ),
        (
            "debit-overflow",
            vec![
                line(cash.id, JournalSide::Debit, i64::MAX),
                line(cash.id, JournalSide::Debit, 1),
                line(revenue.id, JournalSide::Credit, i64::MAX),
            ],
        ),
        (
            "credit-overflow",
            vec![
                line(cash.id, JournalSide::Debit, i64::MAX),
                line(revenue.id, JournalSide::Credit, i64::MAX),
                line(revenue.id, JournalSide::Credit, 1),
            ],
        ),
    ];

    for (case, lines) in invalid_cases {
        let error = journal
            .post(PostEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                lines,
            ))
            .await
            .unwrap_err();
        match case {
            "empty" | "single" => {
                assert!(matches!(
                    error,
                    JournalError::Validation(JournalValidationError::TooFewLines)
                ));
            }
            "zero" | "negative" => {
                assert!(matches!(
                    error,
                    JournalError::Validation(JournalValidationError::NonPositiveAmount {
                        line_index: 0
                    })
                ));
            }
            "unbalanced" => {
                assert!(matches!(
                    error,
                    JournalError::Validation(JournalValidationError::Unbalanced {
                        debit: 101,
                        credit: 100,
                    })
                ));
            }
            "debit-overflow" => {
                assert!(matches!(
                    error,
                    JournalError::Validation(JournalValidationError::TotalOverflow {
                        side: JournalSide::Debit,
                    })
                ));
            }
            "credit-overflow" => {
                assert!(matches!(
                    error,
                    JournalError::Validation(JournalValidationError::TotalOverflow {
                        side: JournalSide::Credit,
                    })
                ));
            }
            _ => unreachable!(),
        }
        assert_eq!(row_counts(&pool).await, (1, 2), "case {case}");
        assert_eq!(journal.get_posted(existing.id).await.unwrap(), existing);
    }
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
async fn database_rejection_at_the_final_posting_transition_rolls_back_every_staged_row() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    sqlx::query(
        r#"
        CREATE TRIGGER test_reject_final_posting_transition
        BEFORE UPDATE OF status ON journal_entries
        WHEN OLD.status = 'draft' AND NEW.status = 'posted'
        BEGIN
            SELECT RAISE(ABORT, 'forced final posting failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test trigger should install");

    let error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("a final transition failure must abort the whole posting transaction");

    assert!(
        matches!(error, JournalError::Database(ref source) if source.to_string().contains("forced final posting failure"))
    );
    assert_eq!(
        row_counts(&pool).await,
        (0, 0),
        "neither the staged header nor any staged line may escape rollback"
    );

    sqlx::query("DROP TRIGGER test_reject_final_posting_transition")
        .execute(&pool)
        .await
        .expect("test trigger should be removable after rollback");
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("the repository should remain usable after a rolled-back posting attempt");
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
    assert_eq!(row_counts(&pool).await, (1, 2));
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
async fn posting_leaf_guard_rejects_a_corrupted_posting_parent_without_writes() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    sqlx::query("DROP TRIGGER accounts_validate_parent_insert")
        .execute(&pool)
        .await
        .expect("test setup should bypass only the parent-kind insertion guard");
    sqlx::query(
        r#"
        INSERT INTO accounts (parent_id, name, element, kind)
        VALUES (?, '損壞子節點', 'asset', 'category')
        "#,
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect("test setup should simulate a posting account with a child");

    let error = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("a posting parent must not receive a journal line");
    assert!(matches!(error, JournalError::AccountHasChildren(id) if id == cash.id.get()));
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
    let inactive = posting_account(&accounts, "停用收入", AccountingElement::Income).await;
    accounts
        .set_active(inactive.id, inactive.revision, false)
        .await
        .expect("test account should be inactive");
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

    let inactive_error = sqlx::query(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'credit', 100)
        "#,
    )
    .bind(entry_id)
    .bind(inactive.id.get())
    .execute(&pool)
    .await
    .expect_err("inactive line account should be blocked by SQLite");
    assert!(
        inactive_error
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
async fn sqlite_independently_rejects_a_corrupted_posting_account_with_children() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    sqlx::query("DROP TRIGGER accounts_validate_parent_insert")
        .execute(&pool)
        .await
        .expect("test setup should bypass only the parent-kind insertion guard");
    sqlx::query(
        r#"
        INSERT INTO accounts (parent_id, name, element, kind)
        VALUES (?, '損壞子節點', 'asset', 'category')
        "#,
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect("test setup should simulate a posting account with a child");
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");

    let error = sqlx::query(
        r#"
        INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
        VALUES (?, 1, ?, 'debit', 100)
        "#,
    )
    .bind(entry_id)
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect_err("a posting account with children should be blocked by SQLite");

    assert!(
        error
            .to_string()
            .contains("journal line account must be active and posting")
    );
    assert_eq!(row_counts(&pool).await, (1, 0));
}

#[tokio::test]
async fn sqlite_rechecks_active_leaf_eligibility_at_the_final_posting_transition() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");
    for (line_no, account_id, side) in [(1_i64, cash.id, "debit"), (2_i64, revenue.id, "credit")] {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, 100)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id.get())
        .bind(side)
        .execute(&pool)
        .await
        .expect("eligible draft line should be inserted");
    }

    let inactive = accounts
        .set_active(revenue.id, revenue.revision, false)
        .await
        .expect("line account should become inactive after the draft is saved");
    let inactive_error = sqlx::query(
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
    .expect_err("SQLite must recheck active state at the posting boundary");
    assert!(
        inactive_error
            .to_string()
            .contains("journal line account must be active and posting")
    );
    let state_after_inactive: (String, Option<String>, i64, i64) = sqlx::query_as(
        r#"
        SELECT entry.status, entry.posted_at, entry.revision, COUNT(line.id)
        FROM journal_entries AS entry
        LEFT JOIN journal_lines AS line ON line.entry_id = entry.id
        WHERE entry.id = ?
        GROUP BY entry.id
        "#,
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("rejected draft should remain intact");
    assert_eq!(state_after_inactive, ("draft".to_owned(), None, 1, 2));

    accounts
        .set_active(inactive.id, inactive.revision, true)
        .await
        .expect("line account should reactivate for the leaf-state check");
    sqlx::query("DROP TRIGGER accounts_validate_parent_insert")
        .execute(&pool)
        .await
        .expect("test setup should bypass only the parent-kind insertion guard");
    let child_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO accounts (parent_id, name, element, kind)
        VALUES (?, '損壞子節點', 'asset', 'category')
        RETURNING id
        "#,
    )
    .bind(cash.id.get())
    .fetch_one(&pool)
    .await
    .expect("test setup should make a referenced posting account non-leaf");

    let non_leaf_error = sqlx::query(
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
    .expect_err("SQLite must recheck leaf state at the posting boundary");
    assert!(
        non_leaf_error
            .to_string()
            .contains("journal line account must be active and posting")
    );
    assert_eq!(row_counts(&pool).await, (1, 2));

    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(child_id)
        .execute(&pool)
        .await
        .expect("removing the corrupt child should restore leaf eligibility");
    sqlx::query(
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
    .expect("the unchanged balanced draft should post after eligibility is restored");
    let posted_state: (String, Option<String>, i64) =
        sqlx::query_as("SELECT status, posted_at, revision FROM journal_entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .expect("posted state should be readable");
    assert_eq!(
        posted_state,
        (
            "posted".to_owned(),
            Some("2026-10-22T00:00:00.000Z".to_owned()),
            2,
        )
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
        sqlx::query("UPDATE journal_entries SET status = 'draft', posted_at = NULL WHERE id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_entries SET revision = revision + 1 WHERE id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET amount_minor = 101 WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET side = 'credit' WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET line_no = 3 WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query("UPDATE journal_lines SET account_id = ? WHERE id = ?")
            .bind(revenue.id.get())
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query("DELETE FROM journal_lines WHERE id = ?")
            .bind(posted.lines[0].id.get())
            .execute(&pool)
            .await,
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, 3, ?, 'debit', 1)
            "#,
        )
        .bind(posted.id.get())
        .bind(cash.id.get())
        .execute(&pool)
        .await,
    ];

    for attempt in attempts {
        assert!(attempt.is_err(), "posted data mutation must be rejected");
    }
    assert_eq!(row_counts(&pool).await, (1, 2));
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
}

#[tokio::test]
async fn a_later_opposite_entry_preserves_the_original_audit_record() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let opposite = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(revenue.id, JournalSide::Debit, 100),
                line(cash.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("a later ordinary balanced entry should create a separate audit record");

    assert_ne!(opposite.id, original.id);
    assert!(opposite.lines.iter().all(|line| {
        original
            .lines
            .iter()
            .all(|original_line| original_line.id != line.id)
    }));
    assert_eq!(journal.get_posted(original.id).await.unwrap(), original);
    assert_eq!(journal.get_posted(opposite.id).await.unwrap(), opposite);
    assert_eq!(row_counts(&pool).await, (2, 4));
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

#[tokio::test]
async fn posting_waits_for_a_committing_deactivation_then_rolls_back_cleanly() {
    let database_path = unique_database_path("post-waits-for-deactivation");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let mut deactivation = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("deactivation should acquire SQLite's writer slot");
    sqlx::query(
        r#"
        UPDATE accounts
        SET is_active = 0,
            revision = revision + 1
        WHERE id = ? AND revision = ?
        "#,
    )
    .bind(revenue.id.get())
    .bind(revenue.revision)
    .execute(&mut *deactivation)
    .await
    .expect("the uncommitted deactivation should be staged");

    let posting_journal = journal.clone();
    let posting = tokio::spawn(async move {
        posting_journal
            .post(PostEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 100),
                    line(revenue.id, JournalSide::Credit, 100),
                ],
            ))
            .await
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !posting.is_finished(),
        "posting must wait while the competing writer owns SQLite's writer slot"
    );

    deactivation
        .commit()
        .await
        .expect("the competing deactivation should commit");
    let error = posting
        .await
        .expect("posting task should not panic")
        .expect_err("posting must observe the committed inactive account");

    assert!(matches!(error, JournalError::AccountInactive(id) if id == revenue.id.get()));
    assert_eq!(
        row_counts(&pool).await,
        (0, 0),
        "the staged journal header must roll back with all of its lines"
    );
    let inactive: i64 = sqlx::query_scalar("SELECT is_active FROM accounts WHERE id = ?")
        .bind(revenue.id.get())
        .fetch_one(&pool)
        .await
        .expect("the committed account state should be readable");
    assert_eq!(inactive, 0);

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_balanced_posts_create_two_complete_distinct_entries() {
    let database_path = unique_database_path("two-concurrent-posts");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let first = journal.post(PostEntry::new(
        JournalDate::parse("2026-10-21").unwrap(),
        vec![
            line(cash.id, JournalSide::Debit, 100),
            line(revenue.id, JournalSide::Credit, 100),
        ],
    ));
    let second = journal.post(PostEntry::new(
        JournalDate::parse("2026-10-22").unwrap(),
        vec![
            line(cash.id, JournalSide::Debit, 200),
            line(revenue.id, JournalSide::Credit, 200),
        ],
    ));
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("the first balanced posting should serialize and succeed");
    let second = second.expect("the second balanced posting should serialize and succeed");

    assert_ne!(first.id, second.id);
    assert!(first.lines.iter().all(|first_line| {
        second
            .lines
            .iter()
            .all(|second_line| first_line.id != second_line.id)
    }));
    assert_eq!(journal.get_posted(first.id).await.unwrap(), first);
    assert_eq!(journal.get_posted(second.id).await.unwrap(), second);
    assert_eq!(row_counts(&pool).await, (2, 4));
    let drafts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE status = 'draft'")
            .fetch_one(&pool)
            .await
            .expect("no staged draft should remain after concurrent success");
    assert_eq!(drafts, 0);

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
