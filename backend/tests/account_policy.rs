use kaccounting_backend::{
    accounting::{
        accounts::{
            Account, AccountError, AccountKind, AccountRepository, AccountingElement, NewAccount,
        },
        journal::{JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry},
        money::MinorUnits,
    },
    db::{open_memory_database, verify_database_integrity},
};

async fn create(
    repo: &AccountRepository,
    parent: Option<&Account>,
    name: &str,
    kind: AccountKind,
) -> Account {
    repo.create(NewAccount::new(
        parent.map(|a| a.id),
        name,
        AccountingElement::Asset,
        kind,
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn sibling_names_use_normalized_exact_case_sensitive_unicode() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool.clone());
    let root = create(&repo, None, "資產", AccountKind::Category).await;
    let group = create(&repo, None, "other", AccountKind::Category).await;
    let cash = create(&repo, Some(&root), "現金 Bank 01", AccountKind::Posting).await;
    assert!(matches!(
        repo.create(NewAccount::new(
            Some(root.id),
            "\u{3000}現金 Bank 01\u{a0}",
            AccountingElement::Asset,
            AccountKind::Posting
        ))
        .await,
        Err(AccountError::DuplicateSiblingName)
    ));
    create(&repo, Some(&group), "現金 Bank 01", AccountKind::Posting).await;
    create(&repo, Some(&root), "現金 bank 01", AccountKind::Posting).await;
    create(&repo, Some(&root), "é", AccountKind::Posting).await;
    create(&repo, Some(&root), "e\u{301}", AccountKind::Posting).await;
    assert!(matches!(
        repo.create(NewAccount::new(
            None,
            "資產",
            AccountingElement::Equity,
            AccountKind::Category
        ))
        .await,
        Err(AccountError::DuplicateSiblingName)
    ));
    assert_eq!(
        repo.rename(cash.id, 1, &cash.name).await.unwrap().revision,
        2
    );
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn duplicate_rename_and_subtree_move_are_atomic_and_keep_revision() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool.clone());
    let first = create(&repo, None, "first", AccountKind::Category).await;
    let second = create(&repo, None, "second", AccountKind::Category).await;
    let cash = create(&repo, Some(&first), "Cash", AccountKind::Posting).await;
    create(&repo, Some(&second), "Cash", AccountKind::Posting).await;
    let bank = create(&repo, Some(&first), "Bank", AccountKind::Posting).await;
    assert!(matches!(
        repo.rename(bank.id, 1, "Cash").await,
        Err(AccountError::DuplicateSiblingName)
    ));
    assert!(matches!(
        repo.move_to(cash.id, 1, Some(second.id)).await,
        Err(AccountError::DuplicateSiblingName)
    ));
    assert_eq!(repo.get(cash.id).await.unwrap(), cash);
    assert_eq!(repo.get(bank.id).await.unwrap(), bank);
    repo.set_active(cash.id, 1, false).await.unwrap();
    assert!(
        matches!(
            repo.rename(bank.id, 1, "Cash").await,
            Err(AccountError::DuplicateSiblingName)
        ),
        "inactive siblings still retain their names"
    );
}

#[tokio::test]
async fn deactivate_descendants_first_then_reactivate_ancestors_first() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool.clone());
    let root = create(&repo, None, "root", AccountKind::Category).await;
    let group = create(&repo, Some(&root), "group", AccountKind::Category).await;
    let cash = create(&repo, Some(&group), "Cash", AccountKind::Posting).await;
    assert!(matches!(
        repo.set_active(root.id, 1, false).await,
        Err(AccountError::ActiveDescendants)
    ));
    assert!(matches!(
        repo.set_active(group.id, 1, false).await,
        Err(AccountError::ActiveDescendants)
    ));
    let cash = repo.set_active(cash.id, 1, false).await.unwrap();
    let group = repo.set_active(group.id, 1, false).await.unwrap();
    let root = repo.set_active(root.id, 1, false).await.unwrap();
    assert!(matches!(
        repo.set_active(cash.id, cash.revision, true).await,
        Err(AccountError::InactiveAncestor)
    ));
    assert!(matches!(
        repo.set_active(group.id, group.revision, true).await,
        Err(AccountError::InactiveAncestor)
    ));
    assert!(matches!(
        repo.create(NewAccount::new(
            Some(root.id),
            "new",
            AccountingElement::Asset,
            AccountKind::Posting
        ))
        .await,
        Err(AccountError::InactiveAncestor)
    ));
    repo.set_active(root.id, root.revision, true).await.unwrap();
    repo.set_active(group.id, group.revision, true)
        .await
        .unwrap();
    repo.set_active(cash.id, cash.revision, true).await.unwrap();
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn moving_active_subtrees_under_inactive_categories_fails_atomically() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool.clone());
    let root = create(&repo, None, "root", AccountKind::Category).await;
    let group = create(&repo, None, "group", AccountKind::Category).await;
    let cash = create(&repo, Some(&root), "Cash", AccountKind::Posting).await;
    let group = repo.set_active(group.id, 1, false).await.unwrap();
    assert!(matches!(
        repo.move_to(root.id, 1, Some(group.id)).await,
        Err(AccountError::InactiveAncestor)
    ));
    assert_eq!(repo.get(root.id).await.unwrap(), root);
    let cash = repo.set_active(cash.id, 1, false).await.unwrap();
    repo.move_to(cash.id, cash.revision, Some(group.id))
        .await
        .unwrap();
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn posting_history_already_prevents_conversion_to_a_category_or_children() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool.clone());
    let cash = create(&repo, None, "Cash", AccountKind::Posting).await;
    let bank = create(&repo, None, "Bank", AccountKind::Posting).await;
    JournalRepository::new(pool.clone())
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(10)),
                JournalLineInput::new(bank.id, JournalSide::Credit, MinorUnits::new(10)),
            ],
        ))
        .await
        .unwrap();
    assert!(matches!(
        repo.change_kind(cash.id, 1, AccountKind::Category).await,
        Err(AccountError::JournalHistoryLocksClassification)
    ));
    assert!(matches!(
        repo.create(NewAccount::new(
            Some(cash.id),
            "child",
            AccountingElement::Asset,
            AccountKind::Posting
        ))
        .await,
        Err(AccountError::ParentNotCategory)
    ));
    verify_database_integrity(&pool).await.unwrap();
}

#[tokio::test]
async fn tree_reconstruction_rejects_an_active_child_of_an_inactive_category() {
    let pool = open_memory_database().await.unwrap();
    let repo = AccountRepository::new(pool);
    let mut root = create(&repo, None, "root", AccountKind::Category).await;
    let child = create(&repo, Some(&root), "child", AccountKind::Posting).await;
    root.is_active = false;
    assert!(matches!(
        kaccounting_backend::accounting::accounts::build_account_tree(&[root, child]),
        Err(kaccounting_backend::accounting::accounts::AccountTreeError::InactiveParent { .. })
    ));
}
