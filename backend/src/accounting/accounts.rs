use std::collections::{HashMap, HashSet};
use std::fmt;

use sqlx::SqlitePool;
use thiserror::Error;

const MAX_ACCOUNT_NAME_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Stable identifier for one account row.
pub struct AccountId(i64);

impl AccountId {
    /// Wraps a numeric identifier for lookup or relationship input.
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the underlying SQLite integer value.
    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// One of the five accounting elements used to classify an account tree.
pub enum AccountingElement {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

impl AccountingElement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Equity => "equity",
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }
}

impl fmt::Display for AccountingElement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AccountingElement {
    type Error = AccountError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "asset" => Ok(Self::Asset),
            "liability" => Ok(Self::Liability),
            "equity" => Ok(Self::Equity),
            "income" => Ok(Self::Income),
            "expense" => Ok(Self::Expense),
            _ => Err(AccountError::InvalidStoredValue {
                field: "element",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether a node only groups children or may receive journal lines.
pub enum AccountKind {
    Category,
    Posting,
}

impl AccountKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Posting => "posting",
        }
    }
}

impl fmt::Display for AccountKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AccountKind {
    type Error = AccountError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "category" => Ok(Self::Category),
            "posting" => Ok(Self::Posting),
            _ => Err(AccountError::InvalidStoredValue {
                field: "kind",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Snapshot of one stored account revision.
pub struct Account {
    pub id: AccountId,
    pub parent_id: Option<AccountId>,
    pub name: String,
    pub element: AccountingElement,
    pub kind: AccountKind,
    pub is_active: bool,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Validated-on-write input for a new active account.
pub struct NewAccount {
    pub parent_id: Option<AccountId>,
    pub name: String,
    pub element: AccountingElement,
    pub kind: AccountKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Account snapshot with its recursively nested children.
pub struct AccountNode {
    pub account: Account,
    pub children: Vec<AccountNode>,
}

#[derive(Debug, Error, Eq, PartialEq)]
/// Structural error found while reconstructing a defensive account tree.
pub enum AccountTreeError {
    #[error("account tree traversal could not find account {0}")]
    MissingNode(i64),
    #[error("account tree contains duplicate id {0}")]
    DuplicateId(i64),
    #[error("account {account_id} refers to missing parent {parent_id}")]
    MissingParent { account_id: i64, parent_id: i64 },
    #[error("posting account {0} cannot be a parent")]
    PostingParent(i64),
    #[error("account {account_id} does not match parent {parent_id}'s accounting element")]
    ElementMismatch { account_id: i64, parent_id: i64 },
    #[error("account tree contains a cycle at account {0}")]
    Cycle(i64),
}

/// Reconstructs roots and descendants in the order supplied by `accounts`.
pub fn build_account_tree(accounts: &[Account]) -> Result<Vec<AccountNode>, AccountTreeError> {
    let mut accounts_by_id = HashMap::with_capacity(accounts.len());
    let mut children_by_parent: HashMap<AccountId, Vec<AccountId>> = HashMap::new();
    let mut root_ids = Vec::new();

    for account in accounts {
        if accounts_by_id.insert(account.id, account).is_some() {
            return Err(AccountTreeError::DuplicateId(account.id.get()));
        }
    }

    for account in accounts {
        if let Some(parent_id) = account.parent_id {
            let parent = accounts_by_id
                .get(&parent_id)
                .ok_or(AccountTreeError::MissingParent {
                    account_id: account.id.get(),
                    parent_id: parent_id.get(),
                })?;

            if parent.kind != AccountKind::Category {
                return Err(AccountTreeError::PostingParent(parent_id.get()));
            }
            if parent.element != account.element {
                return Err(AccountTreeError::ElementMismatch {
                    account_id: account.id.get(),
                    parent_id: parent_id.get(),
                });
            }

            children_by_parent
                .entry(parent_id)
                .or_default()
                .push(account.id);
        } else {
            root_ids.push(account.id);
        }
    }

    let mut visiting = HashSet::new();
    let mut finished = HashSet::new();
    let mut roots = Vec::with_capacity(root_ids.len());

    for root_id in root_ids {
        roots.push(build_account_node(
            root_id,
            &accounts_by_id,
            &children_by_parent,
            &mut visiting,
            &mut finished,
        )?);
    }

    for account in accounts {
        if !finished.contains(&account.id) {
            build_account_node(
                account.id,
                &accounts_by_id,
                &children_by_parent,
                &mut visiting,
                &mut finished,
            )?;
        }
    }

    Ok(roots)
}

impl NewAccount {
    /// Creates account input; persistence performs normalization and structural validation.
    pub fn new(
        parent_id: Option<AccountId>,
        name: impl Into<String>,
        element: AccountingElement,
        kind: AccountKind,
    ) -> Self {
        Self {
            parent_id,
            name: name.into(),
            element,
            kind,
        }
    }
}

#[derive(Debug, Error)]
/// Domain or persistence error returned by account operations.
pub enum AccountError {
    #[error("account name must contain 1 to {MAX_ACCOUNT_NAME_CHARS} non-control characters")]
    InvalidName,
    #[error("account {0} was not found")]
    NotFound(i64),
    #[error("parent account was not found")]
    ParentNotFound,
    #[error("a posting account cannot be used as a parent")]
    ParentNotCategory,
    #[error("parent and child accounts must use the same accounting element")]
    ElementMismatch,
    #[error("the requested parent would create an account hierarchy cycle")]
    Cycle,
    #[error("a posting account cannot have child accounts")]
    PostingAccountHasChildren,
    #[error("the account has child accounts")]
    HasChildren,
    #[error("the account is used by an editable journal draft")]
    HasDraftReferences,
    #[error("the account has journal history")]
    HasJournalHistory,
    #[error("an account with journal lines cannot change element or node kind")]
    JournalHistoryLocksClassification,
    #[error("the account is not a posting account")]
    NotPosting,
    #[error("the posting account is inactive")]
    Inactive,
    #[error("account revision conflict: expected {expected}, current revision is {actual}")]
    RevisionConflict { expected: i64, actual: i64 },
    #[error("an account update must increase its revision by exactly one")]
    InvalidRevisionAdvance,
    #[error("invalid value in stored account field {field}: {value}")]
    InvalidStoredValue { field: &'static str, value: String },
    #[error("stored account tree is invalid: {0}")]
    CorruptTree(#[from] AccountTreeError),
    #[error("account database operation failed: {0}")]
    Database(#[source] sqlx::Error),
}

#[derive(Clone)]
/// Persistence boundary for validated chart-of-accounts operations.
pub struct AccountRepository {
    pool: SqlitePool,
}

impl AccountRepository {
    /// Wraps a configured application database pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Creates an active account after normalizing its name and validating its parent.
    pub async fn create(&self, input: NewAccount) -> Result<Account, AccountError> {
        let name = normalized_name(&input.name)?;

        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            INSERT INTO accounts (parent_id, name, element, kind)
            VALUES (?, ?, ?, ?)
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(input.parent_id.map(AccountId::get))
        .bind(name)
        .bind(input.element.as_str())
        .bind(input.kind.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_database_error)?;

        row.try_into()
    }

    /// Loads one account snapshot by stable identifier.
    pub async fn get(&self, id: AccountId) -> Result<Account, AccountError> {
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            SELECT id, parent_id, name, element, kind, is_active, revision,
                   created_at, updated_at
            FROM accounts
            WHERE id = ?
            "#,
        )
        .bind(id.get())
        .fetch_optional(&self.pool)
        .await
        .map_err(AccountError::Database)?
        .ok_or(AccountError::NotFound(id.get()))?;

        row.try_into()
    }

    /// Lists all accounts in stable creation/identifier order.
    pub async fn list_all(&self) -> Result<Vec<Account>, AccountError> {
        let rows = sqlx::query_as::<_, AccountRow>(
            r#"
            SELECT id, parent_id, name, element, kind, is_active, revision,
                   created_at, updated_at
            FROM accounts
            ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AccountError::Database)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// Lists the defensive nested representation used by account-tree views.
    pub async fn list_tree(&self) -> Result<Vec<AccountNode>, AccountError> {
        let accounts = self.list_all().await?;
        build_account_tree(&accounts).map_err(AccountError::from)
    }

    /// Returns the root-to-node lineage for one account.
    pub async fn path_to(&self, id: AccountId) -> Result<Vec<Account>, AccountError> {
        let rows = sqlx::query_as::<_, AccountRow>(
            r#"
            WITH RECURSIVE lineage(
                id, parent_id, name, element, kind, is_active, revision,
                created_at, updated_at, depth
            ) AS (
                SELECT id, parent_id, name, element, kind, is_active, revision,
                       created_at, updated_at, 0
                FROM accounts
                WHERE id = ?

                UNION ALL

                SELECT parent.id, parent.parent_id, parent.name, parent.element,
                       parent.kind, parent.is_active, parent.revision,
                       parent.created_at, parent.updated_at, child.depth + 1
                FROM accounts AS parent
                JOIN lineage AS child ON parent.id = child.parent_id
            )
            SELECT id, parent_id, name, element, kind, is_active, revision,
                   created_at, updated_at
            FROM lineage
            ORDER BY depth DESC
            "#,
        )
        .bind(id.get())
        .fetch_all(&self.pool)
        .await
        .map_err(AccountError::Database)?;

        if rows.is_empty() {
            return Err(AccountError::NotFound(id.get()));
        }

        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// Renames an account only if `expected_revision` is still current.
    pub async fn rename(
        &self,
        id: AccountId,
        expected_revision: i64,
        requested_name: &str,
    ) -> Result<Account, AccountError> {
        let name = normalized_name(requested_name)?;
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            UPDATE accounts
            SET name = ?,
                revision = revision + 1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND revision = ?
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_database_error)?;

        finish_account_mutation(&self.pool, row, id, expected_revision).await
    }

    /// Moves an account or subtree only if the parent and revision remain valid.
    pub async fn move_to(
        &self,
        id: AccountId,
        expected_revision: i64,
        parent_id: Option<AccountId>,
    ) -> Result<Account, AccountError> {
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            UPDATE accounts
            SET parent_id = ?,
                revision = revision + 1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND revision = ?
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(parent_id.map(AccountId::get))
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_database_error)?;

        finish_account_mutation(&self.pool, row, id, expected_revision).await
    }

    /// Reclassifies an account only when its parent and children remain consistent.
    pub async fn change_element(
        &self,
        id: AccountId,
        expected_revision: i64,
        element: AccountingElement,
    ) -> Result<Account, AccountError> {
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            UPDATE accounts
            SET element = ?,
                revision = revision + 1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND revision = ?
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(element.as_str())
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_database_error)?;

        finish_account_mutation(&self.pool, row, id, expected_revision).await
    }

    /// Changes category/posting kind without allowing a posting parent.
    pub async fn change_kind(
        &self,
        id: AccountId,
        expected_revision: i64,
        kind: AccountKind,
    ) -> Result<Account, AccountError> {
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            UPDATE accounts
            SET kind = ?,
                revision = revision + 1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND revision = ?
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(kind.as_str())
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_database_error)?;

        finish_account_mutation(&self.pool, row, id, expected_revision).await
    }

    /// Activates or deactivates an account at the expected revision.
    pub async fn set_active(
        &self,
        id: AccountId,
        expected_revision: i64,
        is_active: bool,
    ) -> Result<Account, AccountError> {
        let row = sqlx::query_as::<_, AccountRow>(
            r#"
            UPDATE accounts
            SET is_active = ?,
                revision = revision + 1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND revision = ?
            RETURNING id, parent_id, name, element, kind, is_active, revision,
                      created_at, updated_at
            "#,
        )
        .bind(is_active)
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_database_error)?;

        finish_account_mutation(&self.pool, row, id, expected_revision).await
    }

    /// Confirms that an account is currently an active posting leaf.
    ///
    /// Journal posting repeats this check inside its own write transaction; this
    /// read guard alone is not a posting authorization.
    pub async fn require_postable(&self, id: AccountId) -> Result<Account, AccountError> {
        let account = self.get(id).await?;

        if account.kind != AccountKind::Posting {
            return Err(AccountError::NotPosting);
        }
        if !account.is_active {
            return Err(AccountError::Inactive);
        }

        let has_children: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accounts WHERE parent_id = ?)")
                .bind(id.get())
                .fetch_one(&self.pool)
                .await
                .map_err(AccountError::Database)?;

        if has_children {
            return Err(AccountError::PostingAccountHasChildren);
        }

        Ok(account)
    }

    /// Deletes an unreferenced leaf at the expected revision.
    ///
    /// Journal foreign keys also prevent deletion when any draft or posted line exists.
    pub async fn delete(&self, id: AccountId, expected_revision: i64) -> Result<(), AccountError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AccountError::Database)?;
        let result = sqlx::query("DELETE FROM accounts WHERE id = ? AND revision = ?")
            .bind(id.get())
            .bind(expected_revision)
            .execute(&mut *transaction)
            .await;

        let result = match result {
            Ok(result) => result,
            Err(error) if database_message(&error).contains("FOREIGN KEY constraint failed") => {
                let has_children: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accounts WHERE parent_id = ?)")
                        .bind(id.get())
                        .fetch_one(&mut *transaction)
                        .await
                        .map_err(AccountError::Database)?;
                let (has_journal_history, has_draft_references): (bool, bool) = sqlx::query_as(
                    r#"
                        SELECT EXISTS(
                                   SELECT 1
                                   FROM journal_lines AS line
                                   JOIN journal_entries AS entry ON entry.id = line.entry_id
                                   WHERE line.account_id = ? AND entry.status = 'posted'
                               ),
                               EXISTS(
                                   SELECT 1
                                   FROM journal_lines AS line
                                   JOIN journal_entries AS entry ON entry.id = line.entry_id
                                   WHERE line.account_id = ? AND entry.status = 'draft'
                               )
                        "#,
                )
                .bind(id.get())
                .bind(id.get())
                .fetch_one(&mut *transaction)
                .await
                .map_err(AccountError::Database)?;
                // Report immutable posted history before a removable draft
                // reference when both prevent deletion.
                let domain_error = if has_children {
                    AccountError::HasChildren
                } else if has_journal_history {
                    AccountError::HasJournalHistory
                } else if has_draft_references {
                    AccountError::HasDraftReferences
                } else {
                    AccountError::Database(error)
                };
                transaction
                    .rollback()
                    .await
                    .map_err(AccountError::Database)?;
                return Err(domain_error);
            }
            Err(error) => {
                let domain_error = map_database_error(error);
                transaction
                    .rollback()
                    .await
                    .map_err(AccountError::Database)?;
                return Err(domain_error);
            }
        };

        if result.rows_affected() > 0 {
            transaction.commit().await.map_err(AccountError::Database)?;
            Ok(())
        } else {
            let actual_revision: Option<i64> =
                sqlx::query_scalar("SELECT revision FROM accounts WHERE id = ?")
                    .bind(id.get())
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(AccountError::Database)?;
            let domain_error = match actual_revision {
                Some(actual) => AccountError::RevisionConflict {
                    expected: expected_revision,
                    actual,
                },
                None => AccountError::NotFound(id.get()),
            };
            transaction
                .rollback()
                .await
                .map_err(AccountError::Database)?;
            Err(domain_error)
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct AccountRow {
    id: i64,
    parent_id: Option<i64>,
    name: String,
    element: String,
    kind: String,
    is_active: bool,
    revision: i64,
    created_at: String,
    updated_at: String,
}

impl TryFrom<AccountRow> for Account {
    type Error = AccountError;

    fn try_from(row: AccountRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: AccountId::new(row.id),
            parent_id: row.parent_id.map(AccountId::new),
            name: row.name,
            element: AccountingElement::try_from(row.element.as_str())?,
            kind: AccountKind::try_from(row.kind.as_str())?,
            is_active: row.is_active,
            revision: row.revision,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn normalized_name(requested_name: &str) -> Result<&str, AccountError> {
    let name = requested_name.trim();
    let character_count = name.chars().count();

    if character_count == 0
        || character_count > MAX_ACCOUNT_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(AccountError::InvalidName);
    }

    Ok(name)
}

async fn finish_account_mutation(
    pool: &SqlitePool,
    row: Option<AccountRow>,
    id: AccountId,
    expected_revision: i64,
) -> Result<Account, AccountError> {
    match row {
        Some(row) => row.try_into(),
        None => Err(mutation_error(pool, id, expected_revision).await),
    }
}

async fn mutation_error(pool: &SqlitePool, id: AccountId, expected_revision: i64) -> AccountError {
    let actual_revision: Option<i64> =
        match sqlx::query_scalar("SELECT revision FROM accounts WHERE id = ?")
            .bind(id.get())
            .fetch_optional(pool)
            .await
        {
            Ok(revision) => revision,
            Err(error) => return AccountError::Database(error),
        };

    match actual_revision {
        Some(actual) => AccountError::RevisionConflict {
            expected: expected_revision,
            actual,
        },
        None => AccountError::NotFound(id.get()),
    }
}

fn map_database_error(error: sqlx::Error) -> AccountError {
    let message = database_message(&error);

    if message.contains("account parent not found") {
        AccountError::ParentNotFound
    } else if message.contains("account parent must be a category") {
        AccountError::ParentNotCategory
    } else if message.contains("account parent must use the same element")
        || message.contains("account element must match its children")
    {
        AccountError::ElementMismatch
    } else if message.contains("account hierarchy cycle") {
        AccountError::Cycle
    } else if message.contains("posting account cannot have children") {
        AccountError::PostingAccountHasChildren
    } else if message.contains("account revision must increase by one") {
        AccountError::InvalidRevisionAdvance
    } else if message.contains("account journal classification is immutable") {
        AccountError::JournalHistoryLocksClassification
    } else {
        AccountError::Database(error)
    }
}

fn database_message(error: &sqlx::Error) -> &str {
    match error {
        sqlx::Error::Database(database_error) => database_error.message(),
        _ => "",
    }
}

fn build_account_node(
    id: AccountId,
    accounts_by_id: &HashMap<AccountId, &Account>,
    children_by_parent: &HashMap<AccountId, Vec<AccountId>>,
    visiting: &mut HashSet<AccountId>,
    finished: &mut HashSet<AccountId>,
) -> Result<AccountNode, AccountTreeError> {
    if !visiting.insert(id) {
        return Err(AccountTreeError::Cycle(id.get()));
    }

    let mut children = Vec::new();
    if let Some(child_ids) = children_by_parent.get(&id) {
        children.reserve(child_ids.len());
        for child_id in child_ids {
            children.push(build_account_node(
                *child_id,
                accounts_by_id,
                children_by_parent,
                visiting,
                finished,
            )?);
        }
    }

    visiting.remove(&id);
    finished.insert(id);

    let account = accounts_by_id
        .get(&id)
        .ok_or(AccountTreeError::MissingNode(id.get()))?;

    Ok(AccountNode {
        account: (**account).clone(),
        children,
    })
}
