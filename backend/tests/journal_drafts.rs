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
    let journal = JournalRepository::new(pool);
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
                vec![line(inactive.id, JournalSide::Credit, 100)],
            ),
        )
        .await
        .expect_err("inactive account must reject the whole replacement");
    assert!(matches!(error, JournalError::AccountInactive(id) if id == inactive.id.get()));
    assert_eq!(journal.get_draft(original.id).await.unwrap(), original);
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
