use kaccounting_backend::accounting::accounts::{
    AccountError, AccountKind, AccountRepository, AccountTreeError, AccountingElement, NewAccount,
    build_account_tree,
};
use kaccounting_backend::db::{open_memory_database, verify_database_integrity};
use sqlx::SqlitePool;

async fn repository() -> AccountRepository {
    repository_with_pool().await.0
}

async fn repository_with_pool() -> (AccountRepository, SqlitePool) {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should migrate");
    (AccountRepository::new(pool.clone()), pool)
}

async fn create_account(
    repository: &AccountRepository,
    parent_id: Option<kaccounting_backend::accounting::accounts::AccountId>,
    name: &str,
    element: AccountingElement,
    kind: AccountKind,
) -> kaccounting_backend::accounting::accounts::Account {
    repository
        .create(NewAccount::new(parent_id, name, element, kind))
        .await
        .expect("account should be created")
}

#[tokio::test]
async fn creates_a_nested_account_tree_and_normalizes_names() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "\u{3000}資產\u{a0}",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let bank = create_account(
        &repository,
        Some(assets.id),
        "銀行",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let checking = create_account(
        &repository,
        Some(bank.id),
        "某銀行活期存款",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    assert_eq!(assets.name, "資產");
    assert_eq!(bank.parent_id, Some(assets.id));
    assert_eq!(checking.parent_id, Some(bank.id));
    assert_eq!(checking.kind, AccountKind::Posting);
    assert!(checking.is_active);

    let all_accounts = repository.list_all().await.expect("tree should load");
    assert_eq!(all_accounts.len(), 3);
}

#[tokio::test]
async fn round_trips_every_accounting_element() {
    let repository = repository().await;
    let cases = [
        ("資產科目", AccountingElement::Asset),
        ("負債科目", AccountingElement::Liability),
        ("權益科目", AccountingElement::Equity),
        ("收入科目", AccountingElement::Income),
        ("費用科目", AccountingElement::Expense),
    ];

    for (name, element) in cases {
        let account = create_account(&repository, None, name, element, AccountKind::Posting).await;
        assert_eq!(account.element, element);
        assert_eq!(account.element.as_str(), element.to_string());
    }

    assert_eq!(
        repository
            .list_all()
            .await
            .expect("all elements should remain readable")
            .len(),
        5
    );
}

#[test]
fn rejects_unknown_stored_enum_values() {
    assert!(matches!(
        AccountingElement::try_from("cash"),
        Err(AccountError::InvalidStoredValue {
            field: "element",
            value
        }) if value == "cash"
    ));
    assert!(matches!(
        AccountKind::try_from("leaf"),
        Err(AccountError::InvalidStoredValue {
            field: "kind",
            value
        }) if value == "leaf"
    ));
}

#[tokio::test]
async fn repository_rejects_a_non_boolean_stored_activation_state() {
    let (repository, pool) = repository_with_pool().await;
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test setup should bypass CHECK constraints");
    let account_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO accounts (name, element, kind, is_active)
        VALUES ('損壞狀態', 'asset', 'posting', 2)
        RETURNING id
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("STRICT INTEGER storage should still permit the corrupted value");

    let error = repository
        .get(kaccounting_backend::accounting::accounts::AccountId::new(
            account_id,
        ))
        .await
        .expect_err("repository must not decode a non-boolean state as active");

    assert!(matches!(
        error,
        AccountError::InvalidStoredValue {
            field: "is_active",
            value,
        } if value == "2"
    ));
}

#[tokio::test]
async fn repository_rejects_corrupted_stored_ids_names_and_revisions() {
    let (repository, pool) = repository_with_pool().await;
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test setup should bypass CHECK constraints");
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("test setup should bypass foreign keys");
    for drop_trigger_sql in [
        "DROP TRIGGER accounts_require_initial_revision",
        "DROP TRIGGER accounts_validate_parent_insert",
    ] {
        sqlx::query(drop_trigger_sql)
            .execute(&pool)
            .await
            .expect("test setup should bypass the matching insert trigger");
    }

    sqlx::query(
        "INSERT INTO accounts (id, name, element, kind) VALUES (0, '錯誤識別碼', 'asset', 'posting')",
    )
    .execute(&pool)
    .await
    .expect("test setup should insert a non-positive id");
    let invalid_name_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (name, element, kind) VALUES (' 未修剪 ', 'asset', 'posting') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test setup should insert an unnormalized name");
    let invalid_revision_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (name, element, kind, revision) VALUES ('錯誤版本', 'asset', 'posting', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test setup should insert a non-positive revision");
    let invalid_parent_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (-1, '錯誤父節點', 'asset', 'posting') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test setup should insert a non-positive parent id");

    for (id, field, value) in [
        (0, "id", "0"),
        (invalid_name_id, "name", " 未修剪 "),
        (invalid_revision_id, "revision", "0"),
        (invalid_parent_id, "parent_id", "-1"),
    ] {
        let error = repository
            .get(kaccounting_backend::accounting::accounts::AccountId::new(
                id,
            ))
            .await
            .expect_err("repository must reject a corrupted stored scalar");
        assert!(matches!(
            error,
            AccountError::InvalidStoredValue {
                field: actual_field,
                value: actual_value,
            } if actual_field == field && actual_value == value
        ));
    }
}

#[tokio::test]
async fn rejects_blank_and_oversized_names_before_writing() {
    let repository = repository().await;

    let blank_error = repository
        .create(NewAccount::new(
            None,
            "  ",
            AccountingElement::Asset,
            AccountKind::Category,
        ))
        .await
        .expect_err("blank names must be rejected");
    assert!(matches!(blank_error, AccountError::InvalidName));

    let oversized_error = repository
        .create(NewAccount::new(
            None,
            "科".repeat(121),
            AccountingElement::Asset,
            AccountKind::Category,
        ))
        .await
        .expect_err("names over 120 characters must be rejected");
    assert!(matches!(oversized_error, AccountError::InvalidName));

    let control_error = repository
        .create(NewAccount::new(
            None,
            "銀行\n活期存款",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect_err("control characters must be rejected");
    assert!(matches!(control_error, AccountError::InvalidName));

    assert!(
        repository
            .list_all()
            .await
            .expect("list should load")
            .is_empty()
    );
}

#[tokio::test]
async fn accepts_a_name_at_the_120_character_boundary() {
    let repository = repository().await;
    let boundary_name = "科".repeat(120);

    let account = repository
        .create(NewAccount::new(
            None,
            boundary_name.as_str(),
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("a 120-character account name should be valid");

    assert_eq!(account.name, boundary_name);
    assert_eq!(account.name.chars().count(), 120);
}

#[tokio::test]
async fn rust_and_sqlite_reject_every_c0_and_c1_control_character() {
    let (repository, pool) = repository_with_pool().await;

    for codepoint in 0..=159 {
        let character = char::from_u32(codepoint).expect("C0/C1 code point should be valid");
        if !character.is_control() {
            continue;
        }

        let name = format!("前{character}後");
        let rust_error = repository
            .create(NewAccount::new(
                None,
                name.as_str(),
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await
            .expect_err("Rust validation must reject every control character");
        assert!(matches!(rust_error, AccountError::InvalidName));

        let sqlite_result = sqlx::query(
            "INSERT INTO accounts (name, element, kind) VALUES (?, 'asset', 'posting')",
        )
        .bind(name)
        .execute(&pool)
        .await;
        assert!(
            sqlite_result.is_err(),
            "SQLite must reject control code point U+{codepoint:04X}"
        );
    }
}

#[tokio::test]
async fn rust_and_sqlite_agree_on_non_control_edge_whitespace() {
    let (repository, pool) = repository_with_pool().await;
    let edge_whitespace_codepoints = [
        160, 5760, 8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199, 8200, 8201, 8202, 8232, 8233,
        8239, 8287, 12288,
    ];

    for codepoint in edge_whitespace_codepoints {
        let character = char::from_u32(codepoint).expect("whitespace code point should be valid");
        assert!(character.is_whitespace());
        assert!(!character.is_control());

        let name = format!("{character}科目{character}");
        assert_eq!(name.trim(), "科目");
        let normalized = repository
            .create(NewAccount::new(
                None,
                name.as_str(),
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await
            .expect("repository should normalize every supported edge whitespace character");
        assert_eq!(normalized.name, "科目");
        repository.delete(normalized.id, normalized.revision).await.expect("remove this isolated fixture so SQLite rejection tests whitespace rather than uniqueness");

        let sqlite_result = sqlx::query(
            "INSERT INTO accounts (name, element, kind) VALUES (?, 'asset', 'posting')",
        )
        .bind(name)
        .execute(&pool)
        .await;
        assert!(
            sqlite_result.is_err(),
            "SQLite must reject edge whitespace U+{codepoint:04X}"
        );
    }
}

#[tokio::test]
async fn stores_sql_punctuation_in_account_names_as_plain_data() {
    let repository = repository().await;
    let requested_name = "Owner's cash; DROP TABLE accounts; --";

    let account = repository
        .create(NewAccount::new(
            None,
            requested_name,
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("SQL punctuation in a bound name should be stored as data");
    let all_accounts = repository
        .list_all()
        .await
        .expect("accounts table must remain intact");

    assert_eq!(account.name, requested_name);
    assert_eq!(all_accounts, [account]);
}

#[tokio::test]
async fn rejects_a_posting_account_as_a_parent() {
    let repository = repository().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let error = repository
        .create(NewAccount::new(
            Some(cash.id),
            "零用金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect_err("posting accounts cannot have children");

    assert!(matches!(error, AccountError::ParentNotCategory));

    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let move_error = repository
        .move_to(assets.id, assets.revision, Some(cash.id))
        .await
        .expect_err("existing accounts cannot move below a posting account");
    assert!(matches!(move_error, AccountError::ParentNotCategory));
}

#[tokio::test]
async fn rejects_cross_element_parent_relationships() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    let error = repository
        .create(NewAccount::new(
            Some(assets.id),
            "信用卡",
            AccountingElement::Liability,
            AccountKind::Posting,
        ))
        .await
        .expect_err("a child cannot cross accounting elements");

    assert!(matches!(error, AccountError::ElementMismatch));

    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let liabilities = create_account(
        &repository,
        None,
        "負債",
        AccountingElement::Liability,
        AccountKind::Category,
    )
    .await;

    let move_error = repository
        .move_to(cash.id, cash.revision, Some(liabilities.id))
        .await
        .expect_err("an existing account cannot move across accounting elements");
    assert!(matches!(move_error, AccountError::ElementMismatch));
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("failed move must remain atomic")
            .revision,
        cash.revision
    );
}

#[tokio::test]
async fn rejects_missing_parents() {
    let repository = repository().await;

    let error = repository
        .create(NewAccount::new(
            Some(kaccounting_backend::accounting::accounts::AccountId::new(
                404,
            )),
            "孤立科目",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect_err("missing parent must be rejected");

    assert!(matches!(error, AccountError::ParentNotFound));

    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let move_error = repository
        .move_to(
            cash.id,
            cash.revision,
            Some(kaccounting_backend::accounting::accounts::AccountId::new(
                405,
            )),
        )
        .await
        .expect_err("moving to a missing parent must be rejected");
    assert!(matches!(move_error, AccountError::ParentNotFound));
}

#[tokio::test]
async fn rejects_hierarchy_cycles() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let payment_accounts = create_account(
        &repository,
        Some(current_assets.id),
        "支付型",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    let direct_cycle = repository
        .move_to(
            current_assets.id,
            current_assets.revision,
            Some(current_assets.id),
        )
        .await
        .expect_err("self-parenting must be rejected");
    assert!(matches!(direct_cycle, AccountError::Cycle));

    let descendant_cycle = repository
        .move_to(assets.id, assets.revision, Some(payment_accounts.id))
        .await
        .expect_err("moving below a descendant must be rejected");
    assert!(matches!(descendant_cycle, AccountError::Cycle));

    let unchanged_current_assets = repository
        .get(current_assets.id)
        .await
        .expect("failed self-cycle must leave the row unchanged");
    let unchanged_assets = repository
        .get(assets.id)
        .await
        .expect("failed descendant cycle must leave the row unchanged");
    assert_eq!(unchanged_current_assets.parent_id, Some(assets.id));
    assert_eq!(unchanged_current_assets.revision, current_assets.revision);
    assert_eq!(unchanged_assets.parent_id, None);
    assert_eq!(unchanged_assets.revision, assets.revision);
}

#[tokio::test]
async fn failed_moves_preserve_the_complete_snapshot_and_stale_conflicts_take_precedence() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let renamed = repository
        .rename(cash.id, cash.revision, "手邊現金")
        .await
        .expect("rename should produce a newer snapshot");
    let missing_parent = kaccounting_backend::accounting::accounts::AccountId::new(999);

    let stale_error = repository
        .move_to(cash.id, cash.revision, Some(missing_parent))
        .await
        .expect_err("a stale move must not evaluate an invalid destination against newer state");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2,
        }
    ));
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("stale move must leave the newer snapshot unchanged"),
        renamed
    );

    let invalid_destination_error = repository
        .move_to(renamed.id, renamed.revision, Some(missing_parent))
        .await
        .expect_err("the current snapshot must reject a missing destination");
    assert!(matches!(
        invalid_destination_error,
        AccountError::ParentNotFound
    ));
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("failed current move must preserve every stored field"),
        renamed
    );
}

#[tokio::test]
async fn refuses_to_turn_a_parent_into_a_posting_account() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let error = repository
        .change_kind(assets.id, assets.revision, AccountKind::Posting)
        .await
        .expect_err("a parent cannot become posting");

    assert!(matches!(error, AccountError::PostingAccountHasChildren));
    let unchanged = repository
        .get(assets.id)
        .await
        .expect("failed kind change must leave the account unchanged");
    assert_eq!(unchanged.kind, AccountKind::Category);
    assert_eq!(unchanged.revision, assets.revision);
}

#[tokio::test]
async fn refuses_element_changes_that_would_split_a_tree() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let parent_error = repository
        .change_element(assets.id, assets.revision, AccountingElement::Liability)
        .await
        .expect_err("parent element cannot diverge from child");
    assert!(matches!(parent_error, AccountError::ElementMismatch));

    let child_error = repository
        .change_element(cash.id, cash.revision, AccountingElement::Liability)
        .await
        .expect_err("child element cannot diverge from parent");
    assert!(matches!(child_error, AccountError::ElementMismatch));

    let unchanged_assets = repository
        .get(assets.id)
        .await
        .expect("failed parent reclassification must be atomic");
    let unchanged_cash = repository
        .get(cash.id)
        .await
        .expect("failed child reclassification must be atomic");
    assert_eq!(unchanged_assets.element, AccountingElement::Asset);
    assert_eq!(unchanged_assets.revision, assets.revision);
    assert_eq!(unchanged_cash.element, AccountingElement::Asset);
    assert_eq!(unchanged_cash.revision, cash.revision);
}

#[tokio::test]
async fn posting_eligibility_requires_an_active_posting_leaf() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let category_error = repository
        .require_postable(assets.id)
        .await
        .expect_err("categories must not be postable");
    assert!(matches!(category_error, AccountError::NotPosting));

    assert_eq!(
        repository
            .require_postable(cash.id)
            .await
            .expect("active posting leaf should be eligible")
            .id,
        cash.id
    );

    repository
        .set_active(cash.id, cash.revision, false)
        .await
        .expect("account should deactivate");
    let inactive_error = repository
        .require_postable(cash.id)
        .await
        .expect_err("inactive accounts must not be postable");
    assert!(matches!(inactive_error, AccountError::Inactive));

    let missing_error = repository
        .require_postable(kaccounting_backend::accounting::accounts::AccountId::new(
            999,
        ))
        .await
        .expect_err("missing accounts must not be considered postable");
    assert!(matches!(missing_error, AccountError::NotFound(999)));
}

#[tokio::test]
async fn posting_eligibility_fails_closed_when_storage_contains_a_posting_parent() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    sqlx::query("DROP TRIGGER accounts_validate_parent_insert")
        .execute(&pool)
        .await
        .expect("test setup should bypass posting-parent validation");
    sqlx::query(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (?, '損壞子節點', 'asset', 'posting')",
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect("test setup should create a posting parent only in the disposable database");

    let error = repository
        .require_postable(cash.id)
        .await
        .expect_err("a corrupted posting parent must never become eligible for journal lines");

    assert!(matches!(error, AccountError::PostingAccountHasChildren));
}

#[tokio::test]
async fn a_parent_cannot_be_deleted_before_its_children() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let error = repository
        .delete(assets.id, assets.revision)
        .await
        .expect_err("parent deletion must be restricted");
    assert!(matches!(error, AccountError::HasChildren));

    repository
        .delete(cash.id, cash.revision)
        .await
        .expect("leaf should delete before journal tables exist");
    repository
        .delete(assets.id, assets.revision)
        .await
        .expect("empty parent should delete");

    let replacement = create_account(
        &repository,
        None,
        "替代科目",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    assert!(
        replacement.id.get() > cash.id.get(),
        "deleted account identifiers must never be reused"
    );
}

#[tokio::test]
async fn delete_prioritizes_revision_conflicts_and_preserves_a_parent_with_children() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let renamed_assets = repository
        .rename(assets.id, assets.revision, "資產總類")
        .await
        .expect("parent rename should advance its revision");

    let stale_error = repository
        .delete(assets.id, assets.revision)
        .await
        .expect_err("a stale delete must report its conflict before relationship constraints");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2,
        }
    ));

    let current_error = repository
        .delete(renamed_assets.id, renamed_assets.revision)
        .await
        .expect_err("the current parent revision must still be protected by its child");
    assert!(matches!(current_error, AccountError::HasChildren));
    assert_eq!(
        repository
            .get(renamed_assets.id)
            .await
            .expect("both rejected deletes must preserve the parent"),
        renamed_assets
    );
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("both rejected deletes must preserve the child"),
        cash
    );
}

#[tokio::test]
async fn journal_references_do_not_mask_revision_conflicts_or_allow_destructive_changes() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test draft should be created");
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
    .expect("test draft should reference the posting account");
    let renamed = repository
        .rename(cash.id, cash.revision, "手邊現金")
        .await
        .expect("a journal reference must not freeze descriptive account fields");

    let stale_delete = repository
        .delete(cash.id, cash.revision)
        .await
        .expect_err("stale deletion should report its revision conflict first");
    assert!(matches!(
        stale_delete,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2,
        }
    ));
    let current_delete = repository
        .delete(renamed.id, renamed.revision)
        .await
        .expect_err("a current deletion must preserve an account used by a draft");
    assert!(matches!(current_delete, AccountError::HasDraftReferences));

    let stale_classification = repository
        .change_kind(cash.id, cash.revision, AccountKind::Category)
        .await
        .expect_err("stale classification should report its revision conflict first");
    assert!(matches!(
        stale_classification,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2,
        }
    ));
    let current_classification = repository
        .change_kind(renamed.id, renamed.revision, AccountKind::Category)
        .await
        .expect_err("a current classification change must preserve referenced semantics");
    assert!(matches!(
        current_classification,
        AccountError::JournalHistoryLocksClassification
    ));
    assert_eq!(
        repository
            .get(renamed.id)
            .await
            .expect("all rejected destructive changes must preserve the latest snapshot"),
        renamed
    );
}

#[tokio::test]
async fn database_constraints_reject_bypassing_the_repository() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let raw_error = sqlx::query(
        r#"
        INSERT INTO accounts (parent_id, name, element, kind)
        VALUES (?, '零用金', 'asset', 'posting')
        "#,
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect_err("database trigger must reject child under posting account");

    let message = match raw_error {
        sqlx::Error::Database(database_error) => database_error.message().to_owned(),
        other => panic!("expected a database constraint error, got {other}"),
    };
    assert!(message.contains("account parent must be a category"));

    for invalid_sql in [
        "INSERT INTO accounts (id, name, element, kind) VALUES (0, '錯誤識別碼', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('錯誤要素', 'cash', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('錯誤類型', 'asset', 'leaf')",
        "INSERT INTO accounts (name, element, kind, is_active) VALUES ('錯誤狀態', 'asset', 'posting', 2)",
        "INSERT INTO accounts (name, element, kind, revision) VALUES ('錯誤版本', 'asset', 'posting', 2)",
        "INSERT INTO accounts (name, element, kind) VALUES (printf('%0121d', 0), 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('   ', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES (' 未修剪 ', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('換行' || char(10) || '科目', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('空' || char(0) || '字元', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('控制' || char(1) || '字元', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('控制' || char(133) || '字元', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES (char(160) || '不換行空白', 'asset', 'posting')",
        "INSERT INTO accounts (name, element, kind) VALUES ('全形空白' || char(12288), 'asset', 'posting')",
    ] {
        sqlx::query(invalid_sql)
            .execute(&pool)
            .await
            .expect_err("SQLite CHECK constraint must reject invalid raw data");
    }
}

#[tokio::test]
async fn database_requires_every_raw_update_to_advance_one_revision() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    for invalid_sql in [
        "UPDATE accounts SET name = '零用金' WHERE id = ?",
        "UPDATE accounts SET name = '零用金', revision = revision + 2 WHERE id = ?",
    ] {
        let error = sqlx::query(invalid_sql)
            .bind(cash.id.get())
            .execute(&pool)
            .await
            .expect_err("raw updates must advance the revision by exactly one");
        let message = match error {
            sqlx::Error::Database(database_error) => database_error.message().to_owned(),
            other => panic!("expected a database constraint error, got {other}"),
        };
        assert!(message.contains("account revision must increase by one"));
    }

    sqlx::query(
        "UPDATE accounts SET name = '錯誤' || char(10) || '名稱', revision = revision + 1 WHERE id = ?",
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect_err("name constraints must also reject invalid raw updates");

    let unchanged = repository
        .get(cash.id)
        .await
        .expect("rejected updates must leave the account unchanged");
    assert_eq!(unchanged.name, "現金");
    assert_eq!(unchanged.revision, 1);
}

#[tokio::test]
async fn repository_mutations_fail_closed_when_revision_is_exhausted() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let ordinary = create_account(
        &repository,
        None,
        "零用金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let stale_error = repository
        .rename(ordinary.id, i64::MAX, "新零用金")
        .await
        .expect_err("a mismatched maximum revision is a conflict, not exhaustion");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: i64::MAX,
            actual: 1,
        }
    ));

    sqlx::query("DROP TRIGGER accounts_require_next_revision")
        .execute(&pool)
        .await
        .expect("test setup should temporarily bypass revision sequencing");
    sqlx::query("UPDATE accounts SET revision = ? WHERE id = ?")
        .bind(i64::MAX)
        .bind(cash.id.get())
        .execute(&pool)
        .await
        .expect("test setup should exhaust revision only in the disposable database");
    sqlx::query(
        r#"
        CREATE TRIGGER accounts_require_next_revision
        BEFORE UPDATE ON accounts
        WHEN NEW.revision <> OLD.revision + 1
        BEGIN
            SELECT RAISE(ABORT, 'account revision must increase by one');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test setup should restore revision sequencing");

    let exhausted = repository
        .get(cash.id)
        .await
        .expect("exhausted account should remain readable");
    assert_eq!(exhausted.revision, i64::MAX);

    sqlx::query("UPDATE accounts SET name = '溢位', revision = revision + 1 WHERE id = ?")
        .bind(cash.id.get())
        .execute(&pool)
        .await
        .expect_err("SQLite must independently reject an overflowing raw revision update");
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("failed raw update must preserve the exhausted account"),
        exhausted
    );

    let errors = [
        repository
            .rename(cash.id, i64::MAX, "手邊現金")
            .await
            .expect_err("rename must fail before revision overflow"),
        repository
            .move_to(cash.id, i64::MAX, None)
            .await
            .expect_err("move must fail before revision overflow"),
        repository
            .change_element(cash.id, i64::MAX, AccountingElement::Expense)
            .await
            .expect_err("element change must fail before revision overflow"),
        repository
            .change_kind(cash.id, i64::MAX, AccountKind::Category)
            .await
            .expect_err("kind change must fail before revision overflow"),
        repository
            .set_active(cash.id, i64::MAX, false)
            .await
            .expect_err("activation change must fail before revision overflow"),
    ];

    assert!(
        errors.iter().all(
            |error| matches!(error, AccountError::RevisionExhausted(id) if *id == cash.id.get())
        )
    );
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("failed mutations must preserve the exhausted account"),
        exhausted
    );

    repository
        .delete(cash.id, i64::MAX)
        .await
        .expect("deletion need not advance an exhausted revision");
    assert!(matches!(
        repository.get(cash.id).await,
        Err(AccountError::NotFound(id)) if id == cash.id.get()
    ));
}

#[tokio::test]
async fn database_keeps_account_ids_immutable() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    for (invalid_sql, expected_message) in [
        (
            "UPDATE accounts SET id = id + 100, revision = revision + 1 WHERE id = ?",
            Some("account id is immutable"),
        ),
        (
            "UPDATE accounts SET id = NULL, revision = revision + 1 WHERE id = ?",
            None,
        ),
    ] {
        let error = sqlx::query(invalid_sql)
            .bind(cash.id.get())
            .execute(&pool)
            .await
            .expect_err("stable account ids must not be rewritten");
        let message = match error {
            sqlx::Error::Database(database_error) => database_error.message().to_owned(),
            other => panic!("expected a database constraint error, got {other}"),
        };
        if let Some(expected_message) = expected_message {
            assert!(message.contains(expected_message));
        }
    }

    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("original id should still exist")
            .revision,
        1
    );
}

#[tokio::test]
async fn database_keeps_account_creation_timestamps_immutable() {
    let (repository, pool) = repository_with_pool().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let error = sqlx::query(
        "UPDATE accounts SET created_at = '2000-01-01T00:00:00.000Z', revision = revision + 1 WHERE id = ?",
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect_err("account creation timestamps must not be rewritten");
    let message = match error {
        sqlx::Error::Database(database_error) => database_error.message().to_owned(),
        other => panic!("expected a database constraint error, got {other}"),
    };

    assert!(message.contains("account creation timestamp is immutable"));
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("rejected update must leave the timestamp unchanged")
            .created_at,
        cash.created_at
    );
}

#[tokio::test]
async fn sqlite_foreign_keys_are_enabled_for_every_test_connection() {
    let (_repository, pool) = repository_with_pool().await;
    let foreign_keys_enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("pragma should be readable");

    assert_eq!(foreign_keys_enabled, 1);
}

#[tokio::test]
async fn builds_a_stable_nested_tree_for_the_ui() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(current_assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let bank = create_account(
        &repository,
        Some(current_assets.id),
        "銀行存款",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let liabilities = create_account(
        &repository,
        None,
        "負債",
        AccountingElement::Liability,
        AccountKind::Category,
    )
    .await;

    let tree = repository
        .list_tree()
        .await
        .expect("valid rows should form a tree");

    assert_eq!(tree.len(), 2);
    assert_eq!(tree[0].account.id, assets.id);
    assert_eq!(tree[0].children[0].account.id, current_assets.id);
    assert_eq!(tree[0].children[0].children[0].account.id, cash.id);
    assert_eq!(tree[0].children[0].children[1].account.id, bank.id);
    assert_eq!(tree[1].account.id, liabilities.id);
}

#[tokio::test]
async fn tree_builder_preserves_supplied_order_when_children_precede_parents() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(current_assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let bank = create_account(
        &repository,
        Some(current_assets.id),
        "銀行存款",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let liabilities = create_account(
        &repository,
        None,
        "負債",
        AccountingElement::Liability,
        AccountKind::Category,
    )
    .await;

    let supplied = [
        liabilities.clone(),
        bank.clone(),
        cash.clone(),
        current_assets.clone(),
        assets.clone(),
    ];
    let tree = build_account_tree(&supplied)
        .expect("input order must not require parents to appear before children");

    assert_eq!(tree[0].account.id, liabilities.id);
    assert_eq!(tree[1].account.id, assets.id);
    assert_eq!(tree[1].children[0].account.id, current_assets.id);
    assert_eq!(tree[1].children[0].children[0].account.id, bank.id);
    assert_eq!(tree[1].children[0].children[1].account.id, cash.id);
}

#[tokio::test]
async fn tree_builder_rejects_duplicate_and_missing_ids() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    let duplicate_error = build_account_tree(&[assets.clone(), assets.clone()])
        .expect_err("duplicate ids must be rejected");
    assert_eq!(
        duplicate_error,
        AccountTreeError::DuplicateId(assets.id.get())
    );

    let mut orphan = assets;
    orphan.parent_id = Some(kaccounting_backend::accounting::accounts::AccountId::new(
        999,
    ));
    let missing_error =
        build_account_tree(&[orphan.clone()]).expect_err("missing parent ids must be rejected");
    assert_eq!(
        missing_error,
        AccountTreeError::MissingParent {
            account_id: orphan.id.get(),
            parent_id: 999,
        }
    );
}

#[tokio::test]
async fn repository_tree_fails_closed_on_a_late_corrupted_orphan() {
    let (repository, pool) = repository_with_pool().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let bank = create_account(
        &repository,
        Some(assets.id),
        "銀行存款",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    sqlx::query("DROP TRIGGER accounts_validate_parent_update")
        .execute(&pool)
        .await
        .expect("test setup should bypass repository-independent parent validation");
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("test setup should bypass the parent foreign key");
    sqlx::query("UPDATE accounts SET parent_id = 999, revision = revision + 1 WHERE id = ?")
        .bind(bank.id.get())
        .execute(&pool)
        .await
        .expect("test setup should corrupt only the final row in the disposable database");

    let error = repository
        .list_tree()
        .await
        .expect_err("tree loading must reject the whole result rather than return valid prefixes");

    assert!(matches!(
        error,
        AccountError::CorruptTree(AccountTreeError::MissingParent {
            account_id,
            parent_id: 999,
        }) if account_id == bank.id.get()
    ));
    assert_eq!(
        repository
            .get(cash.id)
            .await
            .expect("unrelated valid rows should remain individually readable"),
        cash
    );
}

#[tokio::test]
async fn tree_builder_defends_against_corrupted_cycles() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    let mut corrupted_assets = assets.clone();
    corrupted_assets.parent_id = Some(current_assets.id);
    let cycle_error = build_account_tree(&[corrupted_assets, current_assets])
        .expect_err("defensive tree construction must reject cycles");

    assert!(matches!(cycle_error, AccountTreeError::Cycle(_)));
}

#[tokio::test]
async fn tree_builder_rechecks_parent_kind_and_element() {
    let repository = repository().await;
    let posting_parent = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let mut child = posting_parent.clone();
    child.id = kaccounting_backend::accounting::accounts::AccountId::new(100);
    child.name = "零用金".to_owned();
    child.parent_id = Some(posting_parent.id);

    let kind_error = build_account_tree(&[posting_parent.clone(), child.clone()])
        .expect_err("posting parent must be rejected defensively");
    assert_eq!(
        kind_error,
        AccountTreeError::PostingParent(posting_parent.id.get())
    );

    let mut category_parent = posting_parent;
    category_parent.kind = AccountKind::Category;
    child.element = AccountingElement::Liability;
    let element_error = build_account_tree(&[category_parent.clone(), child.clone()])
        .expect_err("cross-element child must be rejected defensively");
    assert_eq!(
        element_error,
        AccountTreeError::ElementMismatch {
            account_id: child.id.get(),
            parent_id: category_parent.id.get(),
        }
    );
}

#[tokio::test]
async fn tree_builder_handles_a_very_deep_child_first_hierarchy_without_using_the_call_stack() {
    const ACCOUNT_COUNT: i64 = 50_000;

    let repository = repository().await;
    let template = create_account(
        &repository,
        None,
        "深層科目",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let mut accounts: Vec<_> = (1..=ACCOUNT_COUNT)
        .map(|id| {
            let mut account = template.clone();
            account.id = kaccounting_backend::accounting::accounts::AccountId::new(id);
            account.parent_id =
                (id > 1).then(|| kaccounting_backend::accounting::accounts::AccountId::new(id - 1));
            account.kind = if id == ACCOUNT_COUNT {
                AccountKind::Posting
            } else {
                AccountKind::Category
            };
            account
        })
        .collect();
    accounts.reverse();

    let tree = build_account_tree(&accounts)
        .expect("a child-first deep hierarchy should not depend on recursive call-stack space");
    let mut node = &tree[0];
    let mut observed_count = 1;
    while let Some(child) = node.children.first() {
        observed_count += 1;
        node = child;
    }

    assert_eq!(observed_count, ACCOUNT_COUNT);
    assert_eq!(node.account.id.get(), ACCOUNT_COUNT);

    // Drain the recursive DTO iteratively as well, so this intentionally extreme
    // fixture does not make the test harness recursively drop 50,000 nodes.
    let mut pending = tree;
    while let Some(mut node) = pending.pop() {
        pending.append(&mut node.children);
    }
}

#[tokio::test]
async fn tree_builder_handles_a_very_wide_child_first_hierarchy_in_supplied_order() {
    const CHILD_COUNT: i64 = 50_000;

    let repository = repository().await;
    let root = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let template = create_account(
        &repository,
        Some(root.id),
        "明細科目",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let mut accounts = Vec::with_capacity((CHILD_COUNT + 1) as usize);
    accounts.push(root.clone());
    accounts.extend((1..=CHILD_COUNT).map(|offset| {
        let mut account = template.clone();
        account.id = kaccounting_backend::accounting::accounts::AccountId::new(offset + 1);
        account
    }));
    accounts.rotate_left(1);

    let tree = build_account_tree(&accounts).expect("wide child-first hierarchy should build");

    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].children.len(), CHILD_COUNT as usize);
    assert_eq!(tree[0].children[0].account.id.get(), 2);
    assert_eq!(
        tree[0]
            .children
            .last()
            .expect("last child")
            .account
            .id
            .get(),
        CHILD_COUNT + 1
    );
}

#[tokio::test]
async fn tree_builder_rejects_a_very_deep_cycle_without_using_the_call_stack() {
    const ACCOUNT_COUNT: i64 = 50_000;

    let repository = repository().await;
    let template = create_account(
        &repository,
        None,
        "循環科目",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let accounts: Vec<_> = (1..=ACCOUNT_COUNT)
        .map(|id| {
            let mut account = template.clone();
            account.id = kaccounting_backend::accounting::accounts::AccountId::new(id);
            account.parent_id = Some(kaccounting_backend::accounting::accounts::AccountId::new(
                if id == 1 { ACCOUNT_COUNT } else { id - 1 },
            ));
            account
        })
        .collect();

    let error = build_account_tree(&accounts)
        .expect_err("a deep cycle must return a structural error instead of overflowing the stack");

    assert!(matches!(error, AccountTreeError::Cycle(_)));
}

#[tokio::test]
async fn mutations_increment_revision_and_reject_stale_clients() {
    let repository = repository().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    assert_eq!(cash.revision, 1);

    let renamed = repository
        .rename(cash.id, cash.revision, "  手邊現金  ")
        .await
        .expect("current revision should rename");
    assert_eq!(renamed.name, "手邊現金");
    assert_eq!(renamed.revision, 2);

    let stale_error = repository
        .set_active(cash.id, cash.revision, false)
        .await
        .expect_err("stale revision must not overwrite a newer edit");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2
        }
    ));

    let deactivated = repository
        .set_active(renamed.id, renamed.revision, false)
        .await
        .expect("latest revision should update");
    assert!(!deactivated.is_active);
    assert_eq!(deactivated.revision, 3);

    let reactivated = repository
        .set_active(deactivated.id, deactivated.revision, true)
        .await
        .expect("latest revision should reactivate");
    assert!(reactivated.is_active);
    assert_eq!(reactivated.revision, 4);
}

#[tokio::test]
async fn same_value_mutations_are_versioned_updates_not_silent_noops() {
    let repository = repository().await;
    let original = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let renamed = repository
        .rename(original.id, original.revision, original.name.as_str())
        .await
        .expect("same-name update should still consume its expected revision");
    let moved = repository
        .move_to(renamed.id, renamed.revision, renamed.parent_id)
        .await
        .expect("same-parent update should still consume its expected revision");
    let reelemented = repository
        .change_element(moved.id, moved.revision, moved.element)
        .await
        .expect("same-element update should still consume its expected revision");
    let rekinded = repository
        .change_kind(reelemented.id, reelemented.revision, reelemented.kind)
        .await
        .expect("same-kind update should still consume its expected revision");
    let reactivated = repository
        .set_active(rekinded.id, rekinded.revision, rekinded.is_active)
        .await
        .expect("same-activation update should still consume its expected revision");

    assert_eq!(reactivated.revision, original.revision + 5);
    assert_eq!(reactivated.parent_id, original.parent_id);
    assert_eq!(reactivated.name, original.name);
    assert_eq!(reactivated.element, original.element);
    assert_eq!(reactivated.kind, original.kind);
    assert_eq!(reactivated.is_active, original.is_active);

    let stale_error = repository
        .set_active(original.id, original.revision, false)
        .await
        .expect_err("the initial snapshot must be stale after same-value updates");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 6,
        }
    ));
}

#[tokio::test]
async fn valid_moves_and_leaf_reclassification_are_versioned() {
    let repository = repository().await;
    let first_asset_group = create_account(
        &repository,
        None,
        "第一組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let second_asset_group = create_account(
        &repository,
        None,
        "第二組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(first_asset_group.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let moved = repository
        .move_to(cash.id, cash.revision, Some(second_asset_group.id))
        .await
        .expect("same-element move should succeed");
    assert_eq!(moved.parent_id, Some(second_asset_group.id));
    assert_eq!(moved.revision, 2);

    let detached = repository
        .move_to(moved.id, moved.revision, None)
        .await
        .expect("posting leaf may become a root");
    let reclassified = repository
        .change_element(detached.id, detached.revision, AccountingElement::Expense)
        .await
        .expect("detached leaf may be explicitly reclassified");

    assert_eq!(reclassified.element, AccountingElement::Expense);
    assert_eq!(reclassified.revision, 4);
}

#[tokio::test]
async fn moves_a_same_element_subtree_without_rewriting_descendants() {
    let repository = repository().await;
    let first_group = create_account(
        &repository,
        None,
        "第一組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let second_group = create_account(
        &repository,
        None,
        "第二組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let payment_group = create_account(
        &repository,
        Some(first_group.id),
        "支付型",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(payment_group.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let moved_group = repository
        .move_to(
            payment_group.id,
            payment_group.revision,
            Some(second_group.id),
        )
        .await
        .expect("a valid same-element subtree move should succeed");
    let unchanged_cash = repository
        .get(cash.id)
        .await
        .expect("descendant should remain readable after moving its subtree");
    let path = repository
        .path_to(cash.id)
        .await
        .expect("descendant path should reflect the moved subtree");
    let path_names: Vec<&str> = path.iter().map(|account| account.name.as_str()).collect();

    assert_eq!(moved_group.parent_id, Some(second_group.id));
    assert_eq!(moved_group.revision, payment_group.revision + 1);
    assert_eq!(unchanged_cash.parent_id, Some(payment_group.id));
    assert_eq!(unchanged_cash.revision, cash.revision);
    assert_eq!(path_names, ["第二組資產", "支付型", "現金"]);
}

#[tokio::test]
async fn moving_an_inactive_posting_account_preserves_eligibility_state_and_path() {
    let repository = repository().await;
    let first_group = create_account(
        &repository,
        None,
        "第一組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let second_group = create_account(
        &repository,
        None,
        "第二組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(first_group.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let inactive = repository
        .set_active(cash.id, cash.revision, false)
        .await
        .expect("posting account should deactivate");
    let moved = repository
        .move_to(inactive.id, inactive.revision, Some(second_group.id))
        .await
        .expect("inactive posting account should still support a valid structural move");
    let path = repository
        .path_to(moved.id)
        .await
        .expect("path should reflect the move");

    assert!(!moved.is_active);
    assert_eq!(moved.revision, 3);
    assert_eq!(
        path.iter()
            .map(|account| account.name.as_str())
            .collect::<Vec<_>>(),
        ["第二組資產", "現金"]
    );
    assert!(matches!(
        repository.require_postable(moved.id).await,
        Err(AccountError::Inactive)
    ));

    let reactivated = repository
        .set_active(moved.id, moved.revision, true)
        .await
        .expect("moved posting account should reactivate");
    assert_eq!(reactivated.revision, 4);
    assert_eq!(
        repository
            .require_postable(reactivated.id)
            .await
            .expect("reactivated posting leaf should be eligible"),
        reactivated
    );
}

#[tokio::test]
async fn moving_an_older_account_preserves_creation_order_under_its_new_parent() {
    let repository = repository().await;
    let first_group = create_account(
        &repository,
        None,
        "第一組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let second_group = create_account(
        &repository,
        None,
        "第二組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let older_cash = create_account(
        &repository,
        Some(first_group.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let newer_bank = create_account(
        &repository,
        Some(second_group.id),
        "銀行存款",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    repository
        .move_to(older_cash.id, older_cash.revision, Some(second_group.id))
        .await
        .expect("same-element move should succeed");
    let tree = repository.list_tree().await.expect("tree should rebuild");
    let destination = tree
        .iter()
        .find(|node| node.account.id == second_group.id)
        .expect("destination root should remain present");

    assert_eq!(
        destination
            .children
            .iter()
            .map(|node| node.account.id)
            .collect::<Vec<_>>(),
        [older_cash.id, newer_bank.id]
    );
    assert_eq!(
        repository
            .path_to(older_cash.id)
            .await
            .expect("path should agree with the rebuilt tree")
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>(),
        [second_group.id, older_cash.id]
    );
}

#[tokio::test]
async fn repeated_valid_moves_keep_revision_tree_and_path_consistent() {
    const MOVE_COUNT: i64 = 512;

    let repository = repository().await;
    let first_group = create_account(
        &repository,
        None,
        "第一組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let second_group = create_account(
        &repository,
        None,
        "第二組資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let mut cash = create_account(
        &repository,
        Some(first_group.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    for move_number in 1..=MOVE_COUNT {
        let destination = if move_number % 2 == 1 {
            second_group.id
        } else {
            first_group.id
        };
        cash = repository
            .move_to(cash.id, cash.revision, Some(destination))
            .await
            .expect("every alternating same-element move should succeed");
    }

    assert_eq!(cash.revision, MOVE_COUNT + 1);
    assert_eq!(cash.parent_id, Some(first_group.id));
    assert_eq!(
        repository
            .path_to(cash.id)
            .await
            .expect("path should reflect the final move")
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>(),
        [first_group.id, cash.id]
    );
    let tree = repository
        .list_tree()
        .await
        .expect("tree should remain valid");
    assert_eq!(
        tree.iter()
            .find(|node| node.account.id == first_group.id)
            .expect("final parent should remain present")
            .children[0]
            .account
            .id,
        cash.id
    );
}

#[tokio::test]
async fn an_empty_leaf_can_change_between_category_and_posting() {
    let repository = repository().await;
    let leaf = create_account(
        &repository,
        None,
        "尚未使用的明細",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    let posting = repository
        .change_kind(leaf.id, leaf.revision, AccountKind::Posting)
        .await
        .expect("an empty category leaf may become a posting account");
    assert_eq!(posting.kind, AccountKind::Posting);
    assert_eq!(posting.revision, 2);

    let category = repository
        .change_kind(posting.id, posting.revision, AccountKind::Category)
        .await
        .expect("an unused posting leaf may return to a category before journals exist");
    assert_eq!(category.kind, AccountKind::Category);
    assert_eq!(category.revision, 3);
}

#[tokio::test]
async fn supports_deep_user_defined_account_hierarchies() {
    let (repository, pool) = repository_with_pool().await;
    let root = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let mut parent = root;

    for depth in 1..=32 {
        parent = create_account(
            &repository,
            Some(parent.id),
            &format!("第 {depth} 層"),
            AccountingElement::Asset,
            AccountKind::Category,
        )
        .await;
    }

    let leaf = create_account(
        &repository,
        Some(parent.id),
        "最末層可過帳科目",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let tree = repository
        .list_tree()
        .await
        .expect("deep hierarchy should remain valid");
    let mut node = &tree[0];
    let mut observed_depth = 0;
    while let Some(child) = node.children.first() {
        observed_depth += 1;
        node = child;
    }

    assert_eq!(observed_depth, 33);
    assert_eq!(node.account.id, leaf.id);
    verify_database_integrity(&pool)
        .await
        .expect("startup audit should accept a valid deep hierarchy");
}

#[tokio::test]
async fn stale_revision_cannot_delete_a_newer_account_version() {
    let repository = repository().await;
    let cash = create_account(
        &repository,
        None,
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    let renamed = repository
        .rename(cash.id, cash.revision, "手邊現金")
        .await
        .expect("rename should succeed");

    let stale_error = repository
        .delete(cash.id, cash.revision)
        .await
        .expect_err("stale delete must be rejected");
    assert!(matches!(
        stale_error,
        AccountError::RevisionConflict {
            expected: 1,
            actual: 2
        }
    ));

    repository
        .delete(renamed.id, renamed.revision)
        .await
        .expect("latest revision may delete an unreferenced leaf");

    let missing_error = repository
        .delete(renamed.id, renamed.revision)
        .await
        .expect_err("deleting an already removed account should return not found");
    assert!(matches!(missing_error, AccountError::NotFound(id) if id == renamed.id.get()));
}

#[tokio::test]
async fn returns_an_account_path_from_root_to_leaf() {
    let repository = repository().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(current_assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    let path = repository.path_to(cash.id).await.expect("path should load");
    let names: Vec<&str> = path.iter().map(|account| account.name.as_str()).collect();
    assert_eq!(names, ["資產", "流動資產", "現金"]);

    let root_path = repository
        .path_to(assets.id)
        .await
        .expect("a root account should have a one-item path");
    assert_eq!(root_path.as_slice(), [assets]);

    let missing_error = repository
        .path_to(kaccounting_backend::accounting::accounts::AccountId::new(
            999,
        ))
        .await
        .expect_err("missing account path should return not found");
    assert!(matches!(missing_error, AccountError::NotFound(999)));
}

#[tokio::test]
async fn account_path_rejects_a_corrupted_missing_parent() {
    let (repository, pool) = repository_with_pool().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;

    sqlx::query("DROP TRIGGER accounts_validate_parent_update")
        .execute(&pool)
        .await
        .expect("test setup should bypass the parent trigger");
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("test setup should bypass the foreign key");
    sqlx::query("UPDATE accounts SET parent_id = 999, revision = revision + 1 WHERE id = ?")
        .bind(cash.id.get())
        .execute(&pool)
        .await
        .expect("test setup should create an orphan only in the disposable database");

    let error = repository
        .path_to(cash.id)
        .await
        .expect_err("a partial lineage must not be returned as a valid account path");

    assert!(matches!(
        error,
        AccountError::CorruptTree(AccountTreeError::MissingParent {
            account_id,
            parent_id: 999,
        }) if account_id == cash.id.get()
    ));
}

#[tokio::test]
async fn account_path_rejects_a_corrupted_cycle_without_recursive_sql() {
    let (repository, pool) = repository_with_pool().await;
    let assets = create_account(
        &repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let current_assets = create_account(
        &repository,
        Some(assets.id),
        "流動資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;

    sqlx::query("DROP TRIGGER accounts_prevent_cycle_update")
        .execute(&pool)
        .await
        .expect("test setup should bypass the cycle trigger");
    sqlx::query("UPDATE accounts SET parent_id = ?, revision = revision + 1 WHERE id = ?")
        .bind(current_assets.id.get())
        .bind(assets.id.get())
        .execute(&pool)
        .await
        .expect("test setup should create a cycle only in the disposable database");

    let error = repository
        .path_to(current_assets.id)
        .await
        .expect_err("a cyclic lineage must fail closed instead of recursively expanding");

    assert!(matches!(
        error,
        AccountError::CorruptTree(AccountTreeError::Cycle(_))
    ));
}

#[tokio::test]
async fn account_path_rechecks_parent_kind_and_element() {
    let (kind_repository, kind_pool) = repository_with_pool().await;
    let assets = create_account(
        &kind_repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let cash = create_account(
        &kind_repository,
        Some(assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    sqlx::query("DROP TRIGGER accounts_posting_kind_requires_leaf")
        .execute(&kind_pool)
        .await
        .expect("test setup should bypass the posting-parent trigger");
    sqlx::query("UPDATE accounts SET kind = 'posting', revision = revision + 1 WHERE id = ?")
        .bind(assets.id.get())
        .execute(&kind_pool)
        .await
        .expect("test setup should create a posting parent only in the disposable database");

    let kind_error = kind_repository
        .path_to(cash.id)
        .await
        .expect_err("a posting parent must invalidate an account path");
    assert!(matches!(
        kind_error,
        AccountError::CorruptTree(AccountTreeError::PostingParent(id))
            if id == assets.id.get()
    ));

    let (element_repository, element_pool) = repository_with_pool().await;
    let other_assets = create_account(
        &element_repository,
        None,
        "資產",
        AccountingElement::Asset,
        AccountKind::Category,
    )
    .await;
    let other_cash = create_account(
        &element_repository,
        Some(other_assets.id),
        "現金",
        AccountingElement::Asset,
        AccountKind::Posting,
    )
    .await;
    sqlx::query("DROP TRIGGER accounts_preserve_child_elements")
        .execute(&element_pool)
        .await
        .expect("test setup should bypass the child-element trigger");
    sqlx::query("UPDATE accounts SET element = 'liability', revision = revision + 1 WHERE id = ?")
        .bind(other_assets.id.get())
        .execute(&element_pool)
        .await
        .expect("test setup should split elements only in the disposable database");

    let element_error = element_repository
        .path_to(other_cash.id)
        .await
        .expect_err("an element mismatch must invalidate an account path");
    assert!(matches!(
        element_error,
        AccountError::CorruptTree(AccountTreeError::ElementMismatch {
            account_id,
            parent_id,
        }) if account_id == other_cash.id.get() && parent_id == other_assets.id.get()
    ));
}
