use kaccounting_backend::accounting::accounts::{
    Account, AccountId, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalEntryId, JournalError, JournalLineInput, JournalRepository, JournalSide,
    SaveDraftEntry,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::db::{open_database, open_memory_database};
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

#[tokio::test]
async fn draft_can_be_empty_or_unbalanced_and_round_trips() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;

    let empty = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .expect("an empty work-in-progress draft should save");
    assert!(empty.lines.is_empty());

    let unbalanced = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 125)],
        ))
        .await
        .expect("an incomplete draft should save");
    assert_eq!(journal.get_draft(unbalanced.id).await.unwrap(), unbalanced);
}

#[tokio::test]
async fn draft_can_be_replaced_repeatedly_as_one_atomic_document() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let original = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();
    let first_line_id = original.lines[0].id;

    let replaced = journal
        .replace_draft(
            original.id,
            original.revision,
            SaveDraftEntry::new(
                JournalDate::parse("2026-10-24").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 250),
                    line(revenue.id, JournalSide::Credit, 250),
                ],
            ),
        )
        .await
        .expect("draft should be replaceable");
    assert_eq!(replaced.id, original.id);
    assert_eq!(replaced.revision, original.revision + 1);
    assert_eq!(replaced.created_at, original.created_at);
    assert_eq!(replaced.entry_date.as_str(), "2026-10-24");
    assert_eq!(replaced.lines.len(), 2);
    assert!(replaced.lines[0].id > first_line_id);

    let replaced_again = journal
        .replace_draft(
            original.id,
            replaced.revision,
            SaveDraftEntry::new(JournalDate::parse("2026-10-25").unwrap(), vec![]),
        )
        .await
        .expect("there is no artificial replacement limit");
    assert!(replaced_again.lines.is_empty());
    assert_eq!(
        journal.get_draft(original.id).await.unwrap(),
        replaced_again
    );
}

#[tokio::test]
async fn replacing_a_draft_releases_accounts_removed_from_its_line_set() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let bank = posting_account(&accounts, "銀行", AccountingElement::Asset).await;
    let original = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .expect("draft should initially reference cash");

    let replaced = journal
        .replace_draft(
            original.id,
            original.revision,
            SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![line(bank.id, JournalSide::Debit, 100)],
            ),
        )
        .await
        .expect("complete line replacement should succeed");

    accounts
        .delete(cash.id, cash.revision)
        .await
        .expect("an account removed from every draft line should be deletable");
    journal
        .delete_draft(replaced.id, replaced.revision)
        .await
        .expect("replacement draft should remain freely deletable");
    accounts
        .delete(bank.id, bank.revision)
        .await
        .expect("deleting the draft should release its remaining account reference");
}

#[tokio::test]
async fn failed_replacement_preserves_the_previous_draft() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let inactive = posting_account(&accounts, "停用收入", AccountingElement::Income).await;
    let original = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();
    accounts
        .set_active(inactive.id, inactive.revision, false)
        .await
        .unwrap();

    let error = journal
        .replace_draft(
            original.id,
            original.revision,
            SaveDraftEntry::new(
                JournalDate::parse("2026-11-01").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 200),
                    line(inactive.id, JournalSide::Credit, 200),
                ],
            ),
        )
        .await
        .expect_err("a later inactive account must reject the whole replacement");
    assert!(matches!(error, JournalError::AccountInactive(id) if id == inactive.id.get()));
    assert_eq!(journal.get_draft(original.id).await.unwrap(), original);

    let stored_line_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines WHERE entry_id = ?")
            .bind(original.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored_line_count, 1,
        "the valid new first line must roll back with the rejected second line"
    );
}

#[tokio::test]
async fn failed_draft_creation_leaves_no_partial_header_or_lines() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let inactive = posting_account(&accounts, "停用收入", AccountingElement::Income).await;
    accounts
        .set_active(inactive.id, inactive.revision, false)
        .await
        .unwrap();

    let error = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(inactive.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect_err("a later invalid line must roll back the whole draft creation");
    assert!(matches!(error, JournalError::AccountInactive(id) if id == inactive.id.get()));

    let entry_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    let line_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((entry_count, line_count), (0, 0));
}

#[tokio::test]
async fn draft_reads_and_posting_fail_closed_on_a_non_positive_stored_amount() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_lines SET amount_minor = 0 WHERE id = ?")
        .bind(draft.lines[0].id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate a bypassed database constraint");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .unwrap();

    let posting_error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("an invalid stored amount must never cross the posting boundary");
    assert!(
        matches!(posting_error, JournalError::InvalidStoredValue(message) if message.contains("non-positive amount 0"))
    );
    let state: (String, i64) =
        sqlx::query_as("SELECT status, revision FROM journal_entries WHERE id = ?")
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, ("draft".to_owned(), draft.revision));

    let error = journal
        .get_draft(draft.id)
        .await
        .expect_err("the repository must not expose an invalid stored amount");
    assert!(
        matches!(error, JournalError::InvalidStoredValue(message) if message.contains("non-positive amount 0"))
    );
    let list_error = journal
        .list_drafts()
        .await
        .expect_err("draft listing must apply the same defensive validation");
    assert!(matches!(list_error, JournalError::InvalidStoredValue(_)));
}

#[tokio::test]
async fn draft_reads_and_posting_fail_closed_on_non_contiguous_line_numbers() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    sqlx::query("UPDATE journal_lines SET line_no = 3 WHERE id = ?")
        .bind(draft.lines[1].id.get())
        .execute(&pool)
        .await
        .expect("test setup demonstrates the storage-level ordering gap");

    let posting_error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("a malformed line sequence must never be posted");
    assert!(
        matches!(posting_error, JournalError::InvalidStoredValue(message) if message.contains("expected line number 2, found 3"))
    );
    let state: (String, i64) =
        sqlx::query_as("SELECT status, revision FROM journal_entries WHERE id = ?")
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, ("draft".to_owned(), draft.revision));

    assert!(matches!(
        journal.get_draft(draft.id).await,
        Err(JournalError::InvalidStoredValue(message))
            if message.contains("expected line number 2, found 3")
    ));
    assert!(matches!(
        journal.list_drafts().await,
        Err(JournalError::InvalidStoredValue(message))
            if message.contains("expected line number 2, found 3")
    ));
}

#[tokio::test]
async fn malformed_stored_date_and_side_never_cross_the_posting_boundary() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let malformed_date = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let malformed_side = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ))
        .await
        .unwrap();

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER journal_entries_require_next_draft_revision")
        .execute(&pool)
        .await
        .expect("test setup should bypass revision enforcement while injecting corruption");
    sqlx::query("UPDATE journal_entries SET entry_date = '2026-02-30' WHERE id = ?")
        .bind(malformed_date.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate an invalid stored calendar date");
    sqlx::query("UPDATE journal_lines SET side = 'neither' WHERE id = ?")
        .bind(malformed_side.lines[0].id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate an invalid stored journal side");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .unwrap();

    for error in [
        journal.get_draft(malformed_date.id).await.unwrap_err(),
        journal
            .post_draft(malformed_date.id, malformed_date.revision)
            .await
            .unwrap_err(),
        journal
            .replace_draft(
                malformed_date.id,
                malformed_date.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-11-01").unwrap(), vec![]),
            )
            .await
            .unwrap_err(),
        journal
            .delete_draft(malformed_date.id, malformed_date.revision)
            .await
            .unwrap_err(),
    ] {
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid date 2026-02-30"))
        );
    }
    for error in [
        journal.get_draft(malformed_side.id).await.unwrap_err(),
        journal
            .post_draft(malformed_side.id, malformed_side.revision)
            .await
            .unwrap_err(),
        journal
            .replace_draft(
                malformed_side.id,
                malformed_side.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-11-02").unwrap(), vec![]),
            )
            .await
            .unwrap_err(),
        journal
            .delete_draft(malformed_side.id, malformed_side.revision)
            .await
            .unwrap_err(),
    ] {
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid side neither"))
        );
    }

    let states: Vec<(i64, String, i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, status, revision, posted_at
        FROM journal_entries
        WHERE id IN (?, ?)
        ORDER BY id
        "#,
    )
    .bind(malformed_date.id.get())
    .bind(malformed_side.id.get())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        states,
        vec![
            (
                malformed_date.id.get(),
                "draft".to_owned(),
                malformed_date.revision,
                None,
            ),
            (
                malformed_side.id.get(),
                "draft".to_owned(),
                malformed_side.revision,
                None,
            ),
        ],
        "defensive validation must not partially post either corrupt draft"
    );
    let malformed_date_value: String =
        sqlx::query_scalar("SELECT entry_date FROM journal_entries WHERE id = ?")
            .bind(malformed_date.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    let malformed_side_value: String =
        sqlx::query_scalar("SELECT side FROM journal_lines WHERE id = ?")
            .bind(malformed_side.lines[0].id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(malformed_date_value, "2026-02-30");
    assert_eq!(malformed_side_value, "neither");
}

#[tokio::test]
async fn corrupted_entry_status_fails_closed_for_every_draft_operation() {
    let pool = open_memory_database().await.expect("database should open");
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_entries SET status = 'corrupt' WHERE id = ?")
        .bind(draft.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate a bypassed status constraint");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .unwrap();

    for error in [
        journal
            .get_draft(draft.id)
            .await
            .expect_err("a corrupt status must not masquerade as a non-draft"),
        journal
            .get_posted(draft.id)
            .await
            .expect_err("a corrupt status must not masquerade as a non-posted entry"),
        journal
            .replace_draft(
                draft.id,
                draft.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await
            .expect_err("replacement must fail closed on a corrupt status"),
        journal
            .delete_draft(draft.id, draft.revision)
            .await
            .expect_err("deletion must fail closed on a corrupt status"),
        journal
            .post_draft(draft.id, draft.revision)
            .await
            .expect_err("posting must fail closed on a corrupt status"),
    ] {
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid status corrupt"))
        );
    }
    let list_error = journal
        .list_drafts()
        .await
        .expect_err("draft listing must not silently omit a corrupt entry status");
    assert!(
        matches!(list_error, JournalError::InvalidStoredValue(message) if message.contains("invalid status corrupt"))
    );
}

#[tokio::test]
async fn draft_header_with_a_posting_timestamp_fails_closed() {
    let pool = open_memory_database().await.expect("database should open");
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_entries SET posted_at = '2026-10-22T00:00:00.000Z' WHERE id = ?")
        .bind(draft.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate an internally inconsistent draft header");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .unwrap();

    let error = journal
        .get_draft(draft.id)
        .await
        .expect_err("a draft carrying a posting timestamp must not be returned");
    assert!(
        matches!(error, JournalError::InvalidStoredValue(message) if message.contains("draft entry") && message.contains("posting timestamp"))
    );
    for mutation_error in [
        journal
            .replace_draft(
                draft.id,
                draft.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await
            .expect_err("replacement must not rewrite a corrupt draft header"),
        journal
            .delete_draft(draft.id, draft.revision)
            .await
            .expect_err("deletion must not erase a corrupt draft header"),
        journal
            .post_draft(draft.id, draft.revision)
            .await
            .expect_err("posting must not accept a corrupt draft header"),
    ] {
        assert!(matches!(
            mutation_error,
            JournalError::InvalidStoredValue(_)
        ));
    }
    let list_error = journal
        .list_drafts()
        .await
        .expect_err("draft listing must validate header state");
    assert!(matches!(list_error, JournalError::InvalidStoredValue(_)));
}

#[tokio::test]
async fn non_positive_draft_revision_fails_closed() {
    let pool = open_memory_database().await.expect("database should open");
    let journal = JournalRepository::new(pool.clone());
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();

    sqlx::query("DROP TRIGGER journal_entries_require_next_draft_revision")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_entries SET revision = 0 WHERE id = ?")
        .bind(draft.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate a bypassed revision constraint");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .unwrap();

    for error in [
        journal.get_draft(draft.id).await.unwrap_err(),
        journal
            .replace_draft(
                draft.id,
                0,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await
            .unwrap_err(),
        journal.delete_draft(draft.id, 0).await.unwrap_err(),
        journal.post_draft(draft.id, 0).await.unwrap_err(),
    ] {
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid revision 0"))
        );
    }
    assert!(matches!(
        journal.list_drafts().await,
        Err(JournalError::InvalidStoredValue(message)) if message.contains("invalid revision 0")
    ));
}

#[tokio::test]
async fn posting_checks_balance_and_preserves_failed_draft_for_more_editing() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();

    let error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("an incomplete draft must not post");
    assert!(matches!(error, JournalError::Validation(_)));
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);

    let balanced = journal
        .replace_draft(
            draft.id,
            draft.revision,
            SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 100),
                    line(revenue.id, JournalSide::Credit, 100),
                ],
            ),
        )
        .await
        .unwrap();
    let posted = journal
        .post_draft(draft.id, balanced.revision)
        .await
        .expect("balanced draft should post");
    assert_eq!(posted.id, balanced.id);
    assert_eq!(posted.lines.len(), 2);
    let posted_revision: i64 =
        sqlx::query_scalar("SELECT revision FROM journal_entries WHERE id = ?")
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(posted_revision, balanced.revision + 1);
    assert!(matches!(
        journal.get_draft(draft.id).await,
        Err(JournalError::EntryNotDraft(id)) if id == draft.id.get()
    ));
}

#[tokio::test]
async fn failed_posting_uses_stable_error_priority_and_remains_retryable() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let inactive = accounts
        .set_active(revenue.id, revenue.revision, false)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_lines SET amount_minor = 101 WHERE id = ?")
        .bind(draft.lines[0].id.get())
        .execute(&pool)
        .await
        .unwrap();

    let unbalanced = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("pure line validation should run before mutable account checks");
    assert!(matches!(
        unbalanced,
        JournalError::Validation(
            kaccounting_backend::accounting::journal::JournalValidationError::Unbalanced {
                debit: 101,
                credit: 100,
            }
        )
    ));

    sqlx::query("UPDATE journal_lines SET amount_minor = 100 WHERE id = ?")
        .bind(draft.lines[0].id.get())
        .execute(&pool)
        .await
        .unwrap();
    let inactive_error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("the next retry should reach account eligibility validation");
    assert!(matches!(inactive_error, JournalError::AccountInactive(id) if id == revenue.id.get()));

    accounts
        .set_active(inactive.id, inactive.revision, true)
        .await
        .unwrap();
    let posted = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect("failed posting attempts must leave the same draft retryable");
    assert_eq!(posted.lines.len(), 2);
}

#[tokio::test]
async fn database_failure_during_posting_rolls_back_and_leaves_the_draft_retryable() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    sqlx::query(
        r#"
        CREATE TRIGGER test_reject_draft_post
        BEFORE UPDATE OF status ON journal_entries
        WHEN NEW.status = 'posted'
        BEGIN
            SELECT RAISE(ABORT, 'injected posting failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test setup should install a one-entry posting failure");

    let error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("the injected database failure must abort posting");
    assert!(
        matches!(error, JournalError::Database(source) if source.to_string().contains("injected posting failure"))
    );
    assert_eq!(
        journal.get_draft(draft.id).await.unwrap(),
        draft,
        "the aborted posting transaction must preserve the complete draft"
    );

    sqlx::query("DROP TRIGGER test_reject_draft_post")
        .execute(&pool)
        .await
        .expect("test failure trigger should be removable");
    let posted = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect("the same draft revision should remain retryable");
    assert_eq!(posted.id, draft.id);
    assert_eq!(posted.lines.len(), draft.lines.len());
}

#[tokio::test]
async fn draft_deletion_is_unlimited_but_posted_entries_stay_immutable() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    for _ in 0..3 {
        let draft = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![],
            ))
            .await
            .unwrap();
        journal
            .delete_draft(draft.id, draft.revision)
            .await
            .unwrap();
        assert!(matches!(
            journal.get_draft(draft.id).await,
            Err(JournalError::EntryNotFound(id)) if id == draft.id.get()
        ));
    }

    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    journal.post_draft(draft.id, draft.revision).await.unwrap();
    assert!(matches!(
        journal.delete_draft(draft.id, draft.revision).await,
        Err(JournalError::EntryNotDraft(id)) if id == draft.id.get()
    ));
    assert!(matches!(
        journal
            .replace_draft(
                draft.id,
                draft.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await,
        Err(JournalError::EntryNotDraft(id)) if id == draft.id.get()
    ));
    journal
        .get_posted(draft.id)
        .await
        .expect("rejected mutations must leave the posted entry intact");
}

#[tokio::test]
async fn reactivating_an_account_makes_it_available_for_new_drafts_again() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let inactive = accounts
        .set_active(revenue.id, revenue.revision, false)
        .await
        .unwrap();

    let rejected = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(inactive.id, JournalSide::Credit, 100)],
        ))
        .await;
    assert!(matches!(rejected, Err(JournalError::AccountInactive(_))));

    let active = accounts
        .set_active(inactive.id, inactive.revision, true)
        .await
        .expect("account should reactivate");
    assert!(active.is_active);
    journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(active.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("reactivated account should be usable again");
}

#[tokio::test]
async fn reactivating_an_account_allows_an_existing_draft_to_post() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let inactive = accounts
        .set_active(revenue.id, revenue.revision, false)
        .await
        .unwrap();
    let error = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect_err("an inactive line account must block posting");
    assert!(matches!(error, JournalError::AccountInactive(id) if id == revenue.id.get()));
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);

    accounts
        .set_active(inactive.id, inactive.revision, true)
        .await
        .expect("the account should reactivate");
    let posted = journal
        .post_draft(draft.id, draft.revision)
        .await
        .expect("the preserved draft should post after reactivation");
    assert_eq!(posted.id, draft.id);
    assert_eq!(posted.lines.len(), 2);
}

#[tokio::test]
async fn missing_draft_operations_report_not_found() {
    let pool = open_memory_database().await.expect("database should open");
    let journal = JournalRepository::new(pool);
    let missing = JournalEntryId::new(999_999);

    assert!(matches!(
        journal.delete_draft(missing, 1).await,
        Err(JournalError::EntryNotFound(999_999))
    ));
    assert!(matches!(
        journal.post_draft(missing, 1).await,
        Err(JournalError::EntryNotFound(999_999))
    ));
}

#[tokio::test]
async fn stale_revision_cannot_replace_delete_or_post_a_newer_draft() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let original = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let newer = journal
        .replace_draft(
            original.id,
            original.revision,
            SaveDraftEntry::new(
                JournalDate::parse("2026-10-23").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 200),
                    line(revenue.id, JournalSide::Credit, 200),
                ],
            ),
        )
        .await
        .unwrap();

    for error in [
        journal
            .replace_draft(
                original.id,
                original.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-24").unwrap(), vec![]),
            )
            .await
            .expect_err("stale replacement must fail"),
        journal
            .delete_draft(original.id, original.revision)
            .await
            .expect_err("stale deletion must fail"),
        journal
            .post_draft(original.id, original.revision)
            .await
            .expect_err("stale posting must fail"),
    ] {
        assert!(matches!(
            error,
            JournalError::StaleDraftRevision {
                id,
                expected_revision,
                actual_revision,
            } if id == original.id.get()
                && expected_revision == original.revision
                && actual_revision == newer.revision
        ));
    }
    assert_eq!(journal.get_draft(original.id).await.unwrap(), newer);
}

#[tokio::test]
async fn concurrent_draft_replacements_cannot_silently_overwrite_each_other() {
    let database_path = unique_database_path("draft-replace-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();

    let first = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ),
    );
    let second = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-24").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ),
    );
    let (first, second) = tokio::join!(first, second);

    let successes = [&first, &second]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let stale_failures = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| matches!(result, Err(JournalError::StaleDraftRevision { .. })))
        .count();
    assert_eq!(successes, 1);
    assert_eq!(stale_failures, 1);

    let stored = journal.get_draft(draft.id).await.unwrap();
    assert_eq!(stored.revision, draft.revision + 1);
    assert!(
        stored.entry_date.as_str() == "2026-10-23" || stored.entry_date.as_str() == "2026-10-24"
    );
    assert_eq!(stored.lines.len(), 2);

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_delete_and_replace_leave_one_complete_outcome() {
    let database_path = unique_database_path("draft-delete-replace-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let delete = journal.delete_draft(draft.id, draft.revision);
    let replace = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ),
    );
    let (delete_result, replace_result) = tokio::join!(delete, replace);

    match (delete_result, replace_result) {
        (Ok(()), Err(JournalError::EntryNotFound(id))) => {
            assert_eq!(id, draft.id.get());
            assert!(matches!(
                journal.get_draft(draft.id).await,
                Err(JournalError::EntryNotFound(id)) if id == draft.id.get()
            ));
            let remaining_lines: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines WHERE entry_id = ?")
                    .bind(draft.id.get())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(remaining_lines, 0, "draft deletion must cascade its lines");
        }
        (
            Err(JournalError::StaleDraftRevision {
                id,
                expected_revision,
                actual_revision,
            }),
            Ok(replaced),
        ) => {
            assert_eq!(id, draft.id.get());
            assert_eq!(expected_revision, draft.revision);
            assert_eq!(actual_revision, replaced.revision);
            assert_eq!(replaced.lines.len(), 2);
            assert_eq!(replaced.lines[0].amount, MinorUnits::new(200));
            assert_eq!(journal.get_draft(draft.id).await.unwrap(), replaced);
        }
        (delete, replace) => panic!("unexpected race results: {delete:?}, {replace:?}"),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_post_and_replace_choose_one_complete_version() {
    let database_path = unique_database_path("draft-post-replace-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let post = journal.post_draft(draft.id, draft.revision);
    let replace = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ),
    );
    let (post_result, replace_result) = tokio::join!(post, replace);

    match (post_result, replace_result) {
        (Ok(posted), Err(JournalError::EntryNotDraft(id))) => {
            assert_eq!(id, draft.id.get());
            assert_eq!(posted.lines[0].amount, MinorUnits::new(100));
            assert_eq!(journal.get_posted(draft.id).await.unwrap(), posted);
        }
        (
            Err(JournalError::StaleDraftRevision {
                id,
                expected_revision,
                actual_revision,
            }),
            Ok(replaced),
        ) => {
            assert_eq!(id, draft.id.get());
            assert_eq!(expected_revision, draft.revision);
            assert_eq!(actual_revision, replaced.revision);
            assert_eq!(replaced.lines[0].amount, MinorUnits::new(200));
            assert_eq!(journal.get_draft(draft.id).await.unwrap(), replaced);
        }
        (post, replace) => panic!("unexpected race results: {post:?}, {replace:?}"),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_failed_post_and_replace_leave_the_complete_replacement() {
    let database_path = unique_database_path("draft-failed-post-replace-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();

    let post = journal.post_draft(draft.id, draft.revision);
    let replace = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ),
    );
    let (post_result, replace_result) = tokio::join!(post, replace);

    let replaced = replace_result.expect("the valid replacement should eventually win");
    match post_result {
        Err(JournalError::Validation(
            kaccounting_backend::accounting::journal::JournalValidationError::TooFewLines,
        )) => {}
        Err(JournalError::StaleDraftRevision {
            id,
            expected_revision,
            actual_revision,
        }) => {
            assert_eq!(id, draft.id.get());
            assert_eq!(expected_revision, draft.revision);
            assert_eq!(actual_revision, replaced.revision);
        }
        result => panic!("unexpected failed-post race result: {result:?}"),
    }

    assert_eq!(replaced.revision, draft.revision + 1);
    assert_eq!(replaced.entry_date.as_str(), "2026-10-23");
    assert_eq!(replaced.lines.len(), 2);
    assert_eq!(replaced.lines[0].amount, MinorUnits::new(200));
    assert_eq!(replaced.lines[1].amount, MinorUnits::new(200));
    assert_eq!(journal.get_draft(draft.id).await.unwrap(), replaced);
    assert!(matches!(
        journal.get_posted(draft.id).await,
        Err(JournalError::EntryNotPosted(id)) if id == draft.id.get()
    ));

    let stored: (String, i64, i64) = sqlx::query_as(
        r#"
        SELECT entry.status, entry.revision, COUNT(line.id)
        FROM journal_entries AS entry
        LEFT JOIN journal_lines AS line ON line.entry_id = entry.id
        WHERE entry.id = ?
        GROUP BY entry.id
        "#,
    )
    .bind(draft.id.get())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, ("draft".to_owned(), replaced.revision, 2));

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_posts_of_one_revision_create_exactly_one_posting() {
    let database_path = unique_database_path("draft-double-post-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let first = journal.post_draft(draft.id, draft.revision);
    let second = journal.post_draft(draft.id, draft.revision);
    let (first, second) = tokio::join!(first, second);

    let successes = [&first, &second]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let already_posted = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| {
            matches!(result, Err(JournalError::EntryNotDraft(id)) if *id == draft.id.get())
        })
        .count();
    assert_eq!(successes, 1);
    assert_eq!(already_posted, 1);
    let posted = journal.get_posted(draft.id).await.unwrap();
    assert_eq!(posted.lines.len(), 2);

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_post_and_delete_choose_one_atomic_outcome() {
    let database_path = unique_database_path("draft-post-delete-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let post = journal.post_draft(draft.id, draft.revision);
    let delete = journal.delete_draft(draft.id, draft.revision);
    let (post_result, delete_result) = tokio::join!(post, delete);

    match (post_result, delete_result) {
        (Ok(posted), Err(JournalError::EntryNotDraft(id))) => {
            assert_eq!(id, draft.id.get());
            assert_eq!(journal.get_posted(draft.id).await.unwrap(), posted);
        }
        (Err(JournalError::EntryNotFound(id)), Ok(())) => {
            assert_eq!(id, draft.id.get());
            let counts: (i64, i64) = sqlx::query_as(
                r#"
                SELECT (SELECT COUNT(*) FROM journal_entries WHERE id = ?),
                       (SELECT COUNT(*) FROM journal_lines WHERE entry_id = ?)
                "#,
            )
            .bind(draft.id.get())
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(counts, (0, 0));
        }
        (post, delete) => panic!("unexpected race results: {post:?}, {delete:?}"),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_post_delete_and_replace_have_exactly_one_winner() {
    let database_path = unique_database_path("draft-three-way-mutation-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let post = journal.post_draft(draft.id, draft.revision);
    let delete = journal.delete_draft(draft.id, draft.revision);
    let replace = journal.replace_draft(
        draft.id,
        draft.revision,
        SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ),
    );
    let (post_result, delete_result, replace_result) = tokio::join!(post, delete, replace);

    match (post_result, delete_result, replace_result) {
        (
            Ok(posted),
            Err(JournalError::EntryNotDraft(delete_id)),
            Err(JournalError::EntryNotDraft(replace_id)),
        ) => {
            assert_eq!(delete_id, draft.id.get());
            assert_eq!(replace_id, draft.id.get());
            assert_eq!(journal.get_posted(draft.id).await.unwrap(), posted);
        }
        (
            Err(JournalError::EntryNotFound(post_id)),
            Ok(()),
            Err(JournalError::EntryNotFound(replace_id)),
        ) => {
            assert_eq!(post_id, draft.id.get());
            assert_eq!(replace_id, draft.id.get());
            let remaining: (i64, i64) = sqlx::query_as(
                r#"
                SELECT (SELECT COUNT(*) FROM journal_entries WHERE id = ?),
                       (SELECT COUNT(*) FROM journal_lines WHERE entry_id = ?)
                "#,
            )
            .bind(draft.id.get())
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(remaining, (0, 0));
        }
        (
            Err(JournalError::StaleDraftRevision {
                id: post_id,
                expected_revision: post_expected,
                actual_revision: post_actual,
            }),
            Err(JournalError::StaleDraftRevision {
                id: delete_id,
                expected_revision: delete_expected,
                actual_revision: delete_actual,
            }),
            Ok(replaced),
        ) => {
            assert_eq!(post_id, draft.id.get());
            assert_eq!(delete_id, draft.id.get());
            assert_eq!(post_expected, draft.revision);
            assert_eq!(delete_expected, draft.revision);
            assert_eq!(post_actual, replaced.revision);
            assert_eq!(delete_actual, replaced.revision);
            assert_eq!(replaced.lines[0].amount, MinorUnits::new(200));
            assert_eq!(journal.get_draft(draft.id).await.unwrap(), replaced);
        }
        (post, delete, replace) => {
            panic!("unexpected three-way race results: {post:?}, {delete:?}, {replace:?}")
        }
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_draft_post_and_account_deactivation_serialize_cleanly() {
    let database_path = unique_database_path("draft-post-deactivate-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let post = journal.post_draft(draft.id, draft.revision);
    let deactivate = accounts.set_active(revenue.id, revenue.revision, false);
    let (post_result, deactivate_result) = tokio::join!(post, deactivate);

    let inactive = deactivate_result.expect("deactivation should serialize and succeed");
    assert!(!inactive.is_active);
    match post_result {
        Ok(posted) => assert_eq!(journal.get_posted(draft.id).await.unwrap(), posted),
        Err(JournalError::AccountInactive(id)) if id == revenue.id.get() => {
            assert_eq!(journal.get_draft(draft.id).await.unwrap(), draft);
        }
        Err(error) => panic!("unexpected posting result: {error}"),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn post_draft_waits_for_an_existing_writer_and_then_posts_once() {
    let database_path = unique_database_path("draft-post-busy-writer");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let blocker = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("test writer should acquire SQLite's writer slot");
    let draft_id = draft.id;
    let draft_revision = draft.revision;
    let posting_journal = journal.clone();
    let posting =
        tokio::spawn(async move { posting_journal.post_draft(draft_id, draft_revision).await });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !posting.is_finished(),
        "posting should wait rather than claim success while another writer owns the slot"
    );
    blocker
        .rollback()
        .await
        .expect("releasing the test writer should succeed");

    let posted = posting
        .await
        .expect("posting task should not panic")
        .expect("posting should proceed after the writer slot is released");
    assert_eq!(posted.id, draft_id);
    assert_eq!(journal.get_posted(draft_id).await.unwrap(), posted);

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn waiting_replace_and_delete_reject_corruption_committed_by_the_prior_writer() {
    let database_path = unique_database_path("draft-wait-for-corrupting-writer");
    let pool = open_database(&database_path)
        .await
        .expect("temporary database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let replace_target = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 100)],
        ))
        .await
        .unwrap();
    let delete_target = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 200)],
        ))
        .await
        .unwrap();

    let mut blocker = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("test writer should acquire SQLite's writer slot");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("UPDATE journal_lines SET side = 'neither' WHERE entry_id IN (?, ?)")
        .bind(replace_target.id.get())
        .bind(delete_target.id.get())
        .execute(&mut *blocker)
        .await
        .expect("test writer should stage corrupt line sides");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let replacing_journal = journal.clone();
    let replace_id = replace_target.id;
    let replace_revision = replace_target.revision;
    let replacing = tokio::spawn(async move {
        replacing_journal
            .replace_draft(
                replace_id,
                replace_revision,
                SaveDraftEntry::new(JournalDate::parse("2026-11-01").unwrap(), vec![]),
            )
            .await
    });
    let deleting_journal = journal.clone();
    let delete_id = delete_target.id;
    let delete_revision = delete_target.revision;
    let deleting = tokio::spawn(async move {
        deleting_journal
            .delete_draft(delete_id, delete_revision)
            .await
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!replacing.is_finished());
    assert!(!deleting.is_finished());

    blocker
        .commit()
        .await
        .expect("the corrupting writer should commit its simulated damage");

    for error in [
        replacing
            .await
            .expect("replacement task should not panic")
            .expect_err("replacement must not hide newly committed corruption"),
        deleting
            .await
            .expect("deletion task should not panic")
            .expect_err("deletion must not erase newly committed corruption"),
    ] {
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid side neither"))
        );
    }

    let stored: Vec<(i64, String, i64, String)> = sqlx::query_as(
        r#"
        SELECT entry.id, entry.status, entry.revision, line.side
        FROM journal_entries AS entry
        JOIN journal_lines AS line ON line.entry_id = entry.id
        WHERE entry.id IN (?, ?)
        ORDER BY entry.id
        "#,
    )
    .bind(replace_target.id.get())
    .bind(delete_target.id.get())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![
            (
                replace_target.id.get(),
                "draft".to_owned(),
                replace_target.revision,
                "neither".to_owned(),
            ),
            (
                delete_target.id.get(),
                "draft".to_owned(),
                delete_target.revision,
                "neither".to_owned(),
            ),
        ]
    );

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn sqlite_requires_monotonic_revision_for_direct_draft_updates() {
    let pool = open_memory_database().await.expect("database should open");

    let invalid_insert =
        sqlx::query("INSERT INTO journal_entries (entry_date, revision) VALUES ('2026-10-22', 2)")
            .execute(&pool)
            .await
            .expect_err("a draft revision must start at one");
    assert!(
        invalid_insert
            .to_string()
            .contains("journal entry revision must start at one")
    );

    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for statement in [
        "UPDATE journal_entries SET entry_date = '2026-10-23' WHERE id = ?",
        "UPDATE journal_entries SET entry_date = '2026-10-23', revision = revision + 2 WHERE id = ?",
    ] {
        let error = sqlx::query(statement)
            .bind(entry_id)
            .execute(&pool)
            .await
            .expect_err("a direct update must advance exactly one revision");
        assert!(
            error
                .to_string()
                .contains("draft journal entry revision must increase by one")
        );
    }

    sqlx::query(
        "UPDATE journal_entries SET entry_date = '2026-10-23', revision = revision + 1 WHERE id = ?",
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect("a one-step revision update should succeed");
    let stored: (String, i64) =
        sqlx::query_as("SELECT entry_date, revision FROM journal_entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, ("2026-10-23".to_owned(), 2));
}

#[tokio::test]
async fn sqlite_rejects_posting_without_advancing_the_draft_revision() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();

    let error = sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z'
        WHERE id = ?
        "#,
    )
    .bind(draft.id.get())
    .execute(&pool)
    .await
    .expect_err("posting must advance the revision exactly once");
    assert!(
        error
            .to_string()
            .contains("posted journal entry revision is invalid")
    );

    let stored: (String, i64) =
        sqlx::query_as("SELECT status, revision FROM journal_entries WHERE id = ?")
            .bind(draft.id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, ("draft".to_owned(), draft.revision));
}

#[tokio::test]
async fn revision_exhaustion_cannot_replace_or_post_a_draft() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let replace_target = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    let post_target = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 200),
                line(revenue.id, JournalSide::Credit, 200),
            ],
        ))
        .await
        .unwrap();
    let ordinary = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-24").unwrap(),
            vec![],
        ))
        .await
        .unwrap();

    let stale_error = journal
        .replace_draft(
            ordinary.id,
            i64::MAX,
            SaveDraftEntry::new(JournalDate::parse("2026-10-25").unwrap(), vec![]),
        )
        .await
        .expect_err("a mismatched maximum revision remains a stale-write error");
    assert!(matches!(
        stale_error,
        JournalError::StaleDraftRevision {
            id,
            expected_revision: i64::MAX,
            actual_revision: 1,
        } if id == ordinary.id.get()
    ));

    sqlx::query("DROP TRIGGER journal_entries_require_next_draft_revision")
        .execute(&pool)
        .await
        .expect("test setup should bypass only the normal increment trigger");
    sqlx::query("UPDATE journal_entries SET revision = ? WHERE id IN (?, ?)")
        .bind(i64::MAX)
        .bind(replace_target.id.get())
        .bind(post_target.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate an exhausted revision");
    let exhausted_replace = journal.get_draft(replace_target.id).await.unwrap();
    let exhausted_post = journal.get_draft(post_target.id).await.unwrap();
    assert_eq!(exhausted_replace.revision, i64::MAX);
    assert_eq!(exhausted_post.revision, i64::MAX);

    let replace_error = journal
        .replace_draft(
            replace_target.id,
            i64::MAX,
            SaveDraftEntry::new(
                JournalDate::parse("2026-11-01").unwrap(),
                vec![
                    line(cash.id, JournalSide::Debit, 300),
                    line(revenue.id, JournalSide::Credit, 300),
                ],
            ),
        )
        .await
        .expect_err("revision overflow must reject replacement");
    assert!(matches!(
        replace_error,
        JournalError::DraftRevisionExhausted(id) if id == replace_target.id.get()
    ));
    assert_eq!(
        journal.get_draft(replace_target.id).await.unwrap(),
        exhausted_replace
    );

    let post_error = journal
        .post_draft(post_target.id, i64::MAX)
        .await
        .expect_err("revision overflow must reject posting");
    assert!(matches!(
        post_error,
        JournalError::DraftRevisionExhausted(id) if id == post_target.id.get()
    ));
    assert_eq!(
        journal.get_draft(post_target.id).await.unwrap(),
        exhausted_post
    );

    journal
        .delete_draft(replace_target.id, i64::MAX)
        .await
        .expect("deletion does not need to advance an exhausted revision");
    assert!(matches!(
        journal.get_draft(replace_target.id).await,
        Err(JournalError::EntryNotFound(id)) if id == replace_target.id.get()
    ));
}

#[tokio::test]
async fn draft_list_is_stable_complete_and_excludes_posted_entries() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;

    let later = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-11-02").unwrap(),
            vec![line(cash.id, JournalSide::Debit, 50)],
        ))
        .await
        .unwrap();
    let earlier_first = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-11-01").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let earlier_second = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-11-01").unwrap(),
            vec![
                line(cash.id, JournalSide::Debit, 100),
                line(revenue.id, JournalSide::Credit, 100),
            ],
        ))
        .await
        .unwrap();
    journal
        .post_draft(earlier_second.id, earlier_second.revision)
        .await
        .unwrap();

    let drafts = journal.list_drafts().await.unwrap();
    assert_eq!(drafts, vec![earlier_first, later]);
}

fn unique_database_path(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "kAccounting-Extra-01-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

fn remove_sqlite_files(database_path: &std::path::Path) {
    for path in [
        database_path.to_path_buf(),
        std::path::PathBuf::from(format!("{}-wal", database_path.display())),
        std::path::PathBuf::from(format!("{}-shm", database_path.display())),
        std::path::PathBuf::from(format!("{}-journal", database_path.display())),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .unwrap_or_else(|error| panic!("failed to remove {}: {error}", path.display()));
        }
    }
}
