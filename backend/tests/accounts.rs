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
    let (_repository, pool) = repository_with_pool().await;
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
