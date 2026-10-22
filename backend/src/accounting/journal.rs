use std::collections::HashMap;

use thiserror::Error;

use super::accounts::AccountId;
use super::money::{MinorUnits, MoneyError};
use sqlx::SqlitePool;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Stable identifier for one journal entry.
pub struct JournalEntryId(i64);

impl JournalEntryId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Stable identifier for one journal line.
pub struct JournalLineId(i64);

impl JournalLineId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Validated proleptic-Gregorian calendar date stored as `YYYY-MM-DD`.
pub struct JournalDate(String);

impl JournalDate {
    /// Parses a four-digit year and rejects nonexistent calendar dates.
    pub fn parse(value: &str) -> Result<Self, JournalError> {
        let bytes = value.as_bytes();
        if bytes.len() != 10
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || bytes
                .iter()
                .enumerate()
                .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
        {
            return Err(JournalError::InvalidDate(value.to_owned()));
        }

        let year = parse_date_part(&bytes[0..4]);
        let month = parse_date_part(&bytes[5..7]);
        let day = parse_date_part(&bytes[8..10]);
        let maximum_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(year) => 29,
            2 => 28,
            _ => 0,
        };

        if year == 0 || day == 0 || day > maximum_day {
            return Err(JournalError::InvalidDate(value.to_owned()));
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which side of the general journal receives a line amount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalSide {
    Debit,
    Credit,
}

impl JournalSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debit => "debit",
            Self::Credit => "credit",
        }
    }
}

/// Policy-neutral journal line input used by the accounting core.
///
/// This type intentionally says nothing about the eventual editor layout. It
/// models the accounting fact that one positive amount belongs to exactly one
/// side and one account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalLineInput {
    pub account_id: AccountId,
    pub side: JournalSide,
    pub amount: MinorUnits,
}

impl JournalLineInput {
    pub const fn new(account_id: AccountId, side: JournalSide, amount: MinorUnits) -> Self {
        Self {
            account_id,
            side,
            amount,
        }
    }
}

/// Result of validating the arithmetic invariants for a journal entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedJournalLines {
    pub line_count: usize,
    pub debit_total: MinorUnits,
    pub credit_total: MinorUnits,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum JournalValidationError {
    #[error("a posted journal entry requires at least two effective lines")]
    TooFewLines,
    #[error("journal line {line_index} must contain a positive amount")]
    NonPositiveAmount { line_index: usize },
    #[error("the {side:?} total exceeds SQLite's signed 64-bit integer range")]
    TotalOverflow { side: JournalSide },
    #[error("journal entry is not balanced: debit {debit}, credit {credit}")]
    Unbalanced { debit: i64, credit: i64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Minimal posting request; presentation-specific header fields are deferred.
pub struct PostEntry {
    pub entry_date: JournalDate,
    pub lines: Vec<JournalLineInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A user-editable draft. Draft lines may be incomplete or unbalanced.
pub struct SaveDraftEntry {
    pub entry_date: JournalDate,
    pub lines: Vec<JournalLineInput>,
}

impl SaveDraftEntry {
    pub fn new(entry_date: JournalDate, lines: Vec<JournalLineInput>) -> Self {
        Self { entry_date, lines }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One persisted line belonging to an editable draft.
pub struct DraftJournalLine {
    pub id: JournalLineId,
    pub line_no: i64,
    pub account_id: AccountId,
    pub side: JournalSide,
    pub amount: MinorUnits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Persisted draft returned by the repository.
pub struct DraftJournalEntry {
    pub id: JournalEntryId,
    pub entry_date: JournalDate,
    pub revision: i64,
    pub created_at: String,
    pub lines: Vec<DraftJournalLine>,
}

impl PostEntry {
    pub fn new(entry_date: JournalDate, lines: Vec<JournalLineInput>) -> Self {
        Self { entry_date, lines }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Immutable line returned after its parent transaction commits.
pub struct PostedJournalLine {
    pub id: JournalLineId,
    pub line_no: i64,
    pub account_id: AccountId,
    pub side: JournalSide,
    pub amount: MinorUnits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Immutable posted journal entry returned by the repository.
pub struct PostedJournalEntry {
    pub id: JournalEntryId,
    pub entry_date: JournalDate,
    pub created_at: String,
    pub posted_at: String,
    pub lines: Vec<PostedJournalLine>,
}

#[derive(Debug, Error)]
/// Domain or persistence failure from an atomic posting attempt.
pub enum JournalError {
    #[error("invalid journal date {0}; expected an existing date in YYYY-MM-DD form")]
    InvalidDate(String),
    #[error(transparent)]
    Validation(#[from] JournalValidationError),
    #[error("journal line refers to missing account {0}")]
    AccountNotFound(i64),
    #[error("journal line account {0} is not a posting account")]
    AccountNotPosting(i64),
    #[error("journal line account {0} is inactive")]
    AccountInactive(i64),
    #[error("journal line account {0} is not a leaf")]
    AccountHasChildren(i64),
    #[error("journal entry contains more lines than SQLite can number")]
    TooManyLines,
    #[error("journal entry {0} was not found")]
    EntryNotFound(i64),
    #[error("journal entry {0} is not posted")]
    EntryNotPosted(i64),
    #[error("journal entry {0} is not an editable draft")]
    EntryNotDraft(i64),
    #[error(
        "journal draft {id} changed since revision {expected_revision}; current revision is {actual_revision}"
    )]
    StaleDraftRevision {
        id: i64,
        expected_revision: i64,
        actual_revision: i64,
    },
    #[error("stored journal entry is invalid: {0}")]
    InvalidStoredValue(String),
    #[error("journal database operation failed: {0}")]
    Database(#[source] sqlx::Error),
}

#[derive(Clone)]
/// Transactional persistence boundary for journal posting.
pub struct JournalRepository {
    pool: SqlitePool,
}

impl JournalRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Creates a persistent draft without requiring it to be balanced.
    pub async fn create_draft(
        &self,
        input: SaveDraftEntry,
    ) -> Result<DraftJournalEntry, JournalError> {
        validate_draft_lines(&input.lines)?;

        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let (entry_id, revision, created_at): (i64, i64, String) = sqlx::query_as(
            r#"
            INSERT INTO journal_entries (entry_date)
            VALUES (?)
            RETURNING id, revision, created_at
            "#,
        )
        .bind(input.entry_date.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let lines = insert_draft_lines(&mut transaction, entry_id, &input.lines).await?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(DraftJournalEntry {
            id: JournalEntryId::new(entry_id),
            entry_date: input.entry_date,
            revision,
            created_at,
            lines,
        })
    }

    /// Atomically replaces an editable draft's date and complete line set.
    ///
    /// A failure leaves the previously saved draft unchanged.
    pub async fn replace_draft(
        &self,
        id: JournalEntryId,
        expected_revision: i64,
        input: SaveDraftEntry,
    ) -> Result<DraftJournalEntry, JournalError> {
        validate_draft_lines(&input.lines)?;

        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let updated: Option<(i64, String)> = sqlx::query_as(
            r#"
            UPDATE journal_entries
            SET entry_date = ?,
                revision = revision + 1
            WHERE id = ? AND status = 'draft' AND revision = ?
            RETURNING revision, created_at
            "#,
        )
        .bind(input.entry_date.as_str())
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let Some((revision, created_at)) = updated else {
            return Err(
                entry_not_draft_error(&mut transaction, id, Some(expected_revision)).await?,
            );
        };

        sqlx::query("DELETE FROM journal_lines WHERE entry_id = ?")
            .bind(id.get())
            .execute(&mut *transaction)
            .await
            .map_err(JournalError::Database)?;
        let lines = insert_draft_lines(&mut transaction, id.get(), &input.lines).await?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(DraftJournalEntry {
            id,
            entry_date: input.entry_date,
            revision,
            created_at,
            lines,
        })
    }

    /// Loads one editable draft and its current lines in one read snapshot.
    pub async fn get_draft(&self, id: JournalEntryId) -> Result<DraftJournalEntry, JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let header: Option<StoredJournalHeader> = sqlx::query_as(
            r#"
            SELECT entry_date, status, created_at, posted_at, revision
            FROM journal_entries
            WHERE id = ?
            "#,
        )
        .bind(id.get())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let Some(header) = header else {
            return Err(JournalError::EntryNotFound(id.get()));
        };
        if header.status != "draft" {
            return Err(JournalError::EntryNotDraft(id.get()));
        }
        let entry_date = parse_stored_date(id, &header.entry_date)?;
        let lines = load_draft_lines(&mut transaction, id).await?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(DraftJournalEntry {
            id,
            entry_date,
            revision: header.revision,
            created_at: header.created_at,
            lines,
        })
    }

    /// Lists all editable drafts in accounting-date and stable-ID order.
    ///
    /// Headers and lines are read from one SQLite snapshot, so callers never
    /// receive a header from one revision paired with lines from another.
    pub async fn list_drafts(&self) -> Result<Vec<DraftJournalEntry>, JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let headers: Vec<(i64, String, i64, String)> = sqlx::query_as(
            r#"
            SELECT id, entry_date, revision, created_at
            FROM journal_entries
            WHERE status = 'draft'
            ORDER BY entry_date, id
            "#,
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;

        let stored_lines: Vec<StoredDraftListLine> = sqlx::query_as(
            r#"
            SELECT line.entry_id,
                   line.id,
                   line.line_no,
                   line.account_id,
                   line.side,
                   line.amount_minor
            FROM journal_lines AS line
            JOIN journal_entries AS entry ON entry.id = line.entry_id
            WHERE entry.status = 'draft'
            ORDER BY line.entry_id, line.line_no, line.id
            "#,
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let mut lines_by_entry: HashMap<i64, Vec<StoredJournalLine>> = HashMap::new();
        for stored in stored_lines {
            lines_by_entry
                .entry(stored.entry_id)
                .or_default()
                .push(StoredJournalLine {
                    id: stored.id,
                    line_no: stored.line_no,
                    account_id: stored.account_id,
                    side: stored.side,
                    amount_minor: stored.amount_minor,
                });
        }

        let mut drafts = Vec::with_capacity(headers.len());
        for (stored_id, stored_date, revision, created_at) in headers {
            let id = JournalEntryId::new(stored_id);
            let stored_lines = lines_by_entry.remove(&stored_id).unwrap_or_default();
            let lines = stored_lines
                .into_iter()
                .map(|stored| {
                    let (account_id, side, amount) = stored_line_values(id, &stored)?;
                    Ok(DraftJournalLine {
                        id: JournalLineId::new(stored.id),
                        line_no: stored.line_no,
                        account_id,
                        side,
                        amount,
                    })
                })
                .collect::<Result<Vec<_>, JournalError>>()?;
            drafts.push(DraftJournalEntry {
                id,
                entry_date: parse_stored_date(id, &stored_date)?,
                revision,
                created_at,
                lines,
            });
        }
        if let Some(orphan_entry_id) = lines_by_entry.keys().next() {
            return Err(JournalError::InvalidStoredValue(format!(
                "draft line group refers to missing draft entry {orphan_entry_id}"
            )));
        }
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(drafts)
    }

    /// Permanently deletes a draft and its lines. Posted entries are rejected.
    pub async fn delete_draft(
        &self,
        id: JournalEntryId,
        expected_revision: i64,
    ) -> Result<(), JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let deleted: Option<i64> = sqlx::query_scalar(
            "DELETE FROM journal_entries WHERE id = ? AND status = 'draft' AND revision = ? RETURNING id",
        )
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        if deleted.is_none() {
            return Err(
                entry_not_draft_error(&mut transaction, id, Some(expected_revision)).await?,
            );
        }
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(())
    }

    /// Validates and atomically posts an existing draft.
    pub async fn post_draft(
        &self,
        id: JournalEntryId,
        expected_revision: i64,
    ) -> Result<PostedJournalEntry, JournalError> {
        // Acquire SQLite's writer slot before reading so a concurrent
        // replace/delete cannot slip between validation and posting.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(JournalError::Database)?;
        let header: Option<(String, String)> = sqlx::query_as(
            r#"
            SELECT entry_date, created_at
            FROM journal_entries
            WHERE id = ? AND status = 'draft' AND revision = ?
            "#,
        )
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let Some((stored_date, created_at)) = header else {
            return Err(
                entry_not_draft_error(&mut transaction, id, Some(expected_revision)).await?,
            );
        };
        let entry_date = parse_stored_date(id, &stored_date)?;
        let stored_lines = load_stored_lines(&mut transaction, id).await?;
        let line_inputs = stored_lines_to_inputs(id, &stored_lines)?;
        validate_journal_lines(&line_inputs)?;
        for line in &line_inputs {
            require_postable_account(&mut transaction, line.account_id).await?;
        }

        let posted_at: String = sqlx::query_scalar(
            r#"
            UPDATE journal_entries
            SET status = 'posted',
                posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                revision = revision + 1
            WHERE id = ? AND status = 'draft'
            RETURNING posted_at
            "#,
        )
        .bind(id.get())
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let lines = stored_lines_to_posted(id, stored_lines)?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(PostedJournalEntry {
            id,
            entry_date,
            created_at,
            posted_at,
            lines,
        })
    }

    /// Validates and posts an entry as one SQLite transaction.
    ///
    /// This convenience path creates and posts without leaving a saved draft.
    pub async fn post(&self, input: PostEntry) -> Result<PostedJournalEntry, JournalError> {
        validate_journal_lines(&input.lines)?;

        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let (entry_id, created_at): (i64, String) = sqlx::query_as(
            r#"
            INSERT INTO journal_entries (entry_date)
            VALUES (?)
            RETURNING id, created_at
            "#,
        )
        .bind(input.entry_date.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;

        let mut posted_lines = Vec::with_capacity(input.lines.len());
        for (line_index, line) in input.lines.iter().enumerate() {
            require_postable_account(&mut transaction, line.account_id).await?;
            let line_no = i64::try_from(line_index + 1).map_err(|_| JournalError::TooManyLines)?;
            let line_id: i64 = sqlx::query_scalar(
                r#"
                INSERT INTO journal_lines (
                    entry_id, line_no, account_id, side, amount_minor
                )
                VALUES (?, ?, ?, ?, ?)
                RETURNING id
                "#,
            )
            .bind(entry_id)
            .bind(line_no)
            .bind(line.account_id.get())
            .bind(line.side.as_str())
            .bind(line.amount.get())
            .fetch_one(&mut *transaction)
            .await
            .map_err(JournalError::Database)?;

            posted_lines.push(PostedJournalLine {
                id: JournalLineId::new(line_id),
                line_no,
                account_id: line.account_id,
                side: line.side,
                amount: line.amount,
            });
        }

        let posted_at: String = sqlx::query_scalar(
            r#"
            UPDATE journal_entries
            SET status = 'posted',
                posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                revision = revision + 1
            WHERE id = ? AND status = 'draft'
            RETURNING posted_at
            "#,
        )
        .bind(entry_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;

        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(PostedJournalEntry {
            id: JournalEntryId::new(entry_id),
            entry_date: input.entry_date,
            created_at,
            posted_at,
            lines: posted_lines,
        })
    }

    /// Loads one immutable posted entry and all of its lines in one read snapshot.
    pub async fn get_posted(&self, id: JournalEntryId) -> Result<PostedJournalEntry, JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let header: Option<StoredJournalHeader> = sqlx::query_as(
            r#"
            SELECT entry_date, status, created_at, posted_at, revision
            FROM journal_entries
            WHERE id = ?
            "#,
        )
        .bind(id.get())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let Some(header) = header else {
            return Err(JournalError::EntryNotFound(id.get()));
        };
        if header.status != "posted" {
            return Err(JournalError::EntryNotPosted(id.get()));
        }
        let posted_at = header.posted_at.ok_or_else(|| {
            JournalError::InvalidStoredValue(format!(
                "posted entry {} has no posting timestamp",
                id.get()
            ))
        })?;
        let entry_date = parse_stored_date(id, &header.entry_date)?;
        let stored_lines = load_stored_lines(&mut transaction, id).await?;
        transaction.commit().await.map_err(JournalError::Database)?;
        let validation_lines = stored_lines_to_inputs(id, &stored_lines)?;
        validate_journal_lines(&validation_lines).map_err(|error| {
            JournalError::InvalidStoredValue(format!(
                "entry {} failed line validation: {error}",
                id.get()
            ))
        })?;

        let lines = stored_lines_to_posted(id, stored_lines)?;
        Ok(PostedJournalEntry {
            id,
            entry_date,
            created_at: header.created_at,
            posted_at,
            lines,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct StoredJournalHeader {
    entry_date: String,
    status: String,
    created_at: String,
    posted_at: Option<String>,
    revision: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredJournalLine {
    id: i64,
    line_no: i64,
    account_id: i64,
    side: String,
    amount_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredDraftListLine {
    entry_id: i64,
    id: i64,
    line_no: i64,
    account_id: i64,
    side: String,
    amount_minor: i64,
}

fn validate_draft_lines(lines: &[JournalLineInput]) -> Result<(), JournalError> {
    i64::try_from(lines.len()).map_err(|_| JournalError::TooManyLines)?;
    for (line_index, line) in lines.iter().enumerate() {
        if line.amount <= MinorUnits::ZERO {
            return Err(JournalValidationError::NonPositiveAmount { line_index }.into());
        }
    }
    Ok(())
}

async fn insert_draft_lines(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry_id: i64,
    lines: &[JournalLineInput],
) -> Result<Vec<DraftJournalLine>, JournalError> {
    let mut saved = Vec::with_capacity(lines.len());
    for (line_index, line) in lines.iter().enumerate() {
        require_postable_account(transaction, line.account_id).await?;
        let line_no = i64::try_from(line_index + 1).map_err(|_| JournalError::TooManyLines)?;
        let line_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(line.account_id.get())
        .bind(line.side.as_str())
        .bind(line.amount.get())
        .fetch_one(&mut **transaction)
        .await
        .map_err(JournalError::Database)?;
        saved.push(DraftJournalLine {
            id: JournalLineId::new(line_id),
            line_no,
            account_id: line.account_id,
            side: line.side,
            amount: line.amount,
        });
    }
    Ok(saved)
}

async fn load_stored_lines(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
) -> Result<Vec<StoredJournalLine>, JournalError> {
    sqlx::query_as(
        r#"
        SELECT id, line_no, account_id, side, amount_minor
        FROM journal_lines
        WHERE entry_id = ?
        ORDER BY line_no, id
        "#,
    )
    .bind(id.get())
    .fetch_all(&mut **transaction)
    .await
    .map_err(JournalError::Database)
}

async fn load_draft_lines(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
) -> Result<Vec<DraftJournalLine>, JournalError> {
    load_stored_lines(transaction, id)
        .await?
        .into_iter()
        .map(|stored| {
            let (account_id, side, amount) = stored_line_values(id, &stored)?;
            Ok(DraftJournalLine {
                id: JournalLineId::new(stored.id),
                line_no: stored.line_no,
                account_id,
                side,
                amount,
            })
        })
        .collect()
}

fn stored_lines_to_inputs(
    id: JournalEntryId,
    lines: &[StoredJournalLine],
) -> Result<Vec<JournalLineInput>, JournalError> {
    lines
        .iter()
        .map(|stored| {
            let (account_id, side, amount) = stored_line_values(id, stored)?;
            Ok(JournalLineInput::new(account_id, side, amount))
        })
        .collect()
}

fn stored_lines_to_posted(
    id: JournalEntryId,
    lines: Vec<StoredJournalLine>,
) -> Result<Vec<PostedJournalLine>, JournalError> {
    lines
        .into_iter()
        .map(|stored| {
            let (account_id, side, amount) = stored_line_values(id, &stored)?;
            Ok(PostedJournalLine {
                id: JournalLineId::new(stored.id),
                line_no: stored.line_no,
                account_id,
                side,
                amount,
            })
        })
        .collect()
}

fn stored_line_values(
    entry_id: JournalEntryId,
    stored: &StoredJournalLine,
) -> Result<(AccountId, JournalSide, MinorUnits), JournalError> {
    let side = match stored.side.as_str() {
        "debit" => JournalSide::Debit,
        "credit" => JournalSide::Credit,
        _ => {
            return Err(JournalError::InvalidStoredValue(format!(
                "entry {} line {} has invalid side {}",
                entry_id.get(),
                stored.id,
                stored.side
            )));
        }
    };
    Ok((
        AccountId::new(stored.account_id),
        side,
        MinorUnits::new(stored.amount_minor),
    ))
}

fn parse_stored_date(id: JournalEntryId, stored: &str) -> Result<JournalDate, JournalError> {
    JournalDate::parse(stored).map_err(|_| {
        JournalError::InvalidStoredValue(format!("entry {} has invalid date {stored}", id.get()))
    })
}

async fn entry_not_draft_error(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
    expected_revision: Option<i64>,
) -> Result<JournalError, JournalError> {
    let state: Option<(String, i64)> =
        sqlx::query_as("SELECT status, revision FROM journal_entries WHERE id = ?")
            .bind(id.get())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(JournalError::Database)?;
    Ok(match (state, expected_revision) {
        (Some((status, actual_revision)), Some(expected_revision))
            if status == "draft" && expected_revision != actual_revision =>
        {
            JournalError::StaleDraftRevision {
                id: id.get(),
                expected_revision,
                actual_revision,
            }
        }
        (Some(_), _) => JournalError::EntryNotDraft(id.get()),
        (None, _) => JournalError::EntryNotFound(id.get()),
    })
}

/// Validates the arithmetic invariants required before posting.
///
/// Account existence and posting eligibility are mutable database facts and
/// must be checked again inside the posting transaction. This function
/// therefore validates only the pure, deterministic line rules.
pub fn validate_journal_lines(
    lines: &[JournalLineInput],
) -> Result<ValidatedJournalLines, JournalValidationError> {
    if lines.len() < 2 {
        return Err(JournalValidationError::TooFewLines);
    }

    let mut debit_total = MinorUnits::ZERO;
    let mut credit_total = MinorUnits::ZERO;

    for (line_index, line) in lines.iter().enumerate() {
        if line.amount <= MinorUnits::ZERO {
            return Err(JournalValidationError::NonPositiveAmount { line_index });
        }

        let total = match line.side {
            JournalSide::Debit => &mut debit_total,
            JournalSide::Credit => &mut credit_total,
        };
        *total = total
            .checked_add(line.amount)
            .map_err(
                |MoneyError::Overflow| JournalValidationError::TotalOverflow { side: line.side },
            )?;
    }

    if debit_total != credit_total {
        return Err(JournalValidationError::Unbalanced {
            debit: debit_total.get(),
            credit: credit_total.get(),
        });
    }

    Ok(ValidatedJournalLines {
        line_count: lines.len(),
        debit_total,
        credit_total,
    })
}

async fn require_postable_account(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: AccountId,
) -> Result<(), JournalError> {
    let account: Option<(String, bool, bool)> = sqlx::query_as(
        r#"
        SELECT kind,
               is_active,
               EXISTS(SELECT 1 FROM accounts AS child WHERE child.parent_id = accounts.id)
        FROM accounts
        WHERE id = ?
        "#,
    )
    .bind(account_id.get())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;

    let Some((kind, is_active, has_children)) = account else {
        return Err(JournalError::AccountNotFound(account_id.get()));
    };
    if kind != "posting" {
        return Err(JournalError::AccountNotPosting(account_id.get()));
    }
    if !is_active {
        return Err(JournalError::AccountInactive(account_id.get()));
    }
    if has_children {
        return Err(JournalError::AccountHasChildren(account_id.get()));
    }

    Ok(())
}

fn parse_date_part(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100))
}
