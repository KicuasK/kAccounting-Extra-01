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

/// Optional local civil time metadata; never changes the accounting date or implies a timezone.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JournalTime(String);
impl JournalTime {
    pub fn parse(value: &str) -> Result<Self, JournalError> {
        let bytes = value.as_bytes();
        if bytes.len() != 8
            || bytes[2] != b':'
            || bytes[5] != b':'
            || bytes
                .iter()
                .enumerate()
                .any(|(i, b)| i != 2 && i != 5 && !b.is_ascii_digit())
            || parse_date_part(&bytes[0..2]) > 23
            || parse_date_part(&bytes[3..5]) > 59
            || parse_date_part(&bytes[6..8]) > 59
        {
            return Err(JournalError::InvalidTime(value.to_owned()));
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalLineInput {
    pub account_id: AccountId,
    pub side: JournalSide,
    pub amount: MinorUnits,
    pub memo: Option<String>,
}

impl JournalLineInput {
    pub fn new(account_id: AccountId, side: JournalSide, amount: MinorUnits) -> Self {
        Self {
            account_id,
            side,
            amount,
            memo: None,
        }
    }

    pub fn with_memo(mut self, memo: impl Into<String>) -> Self {
        self.memo = Some(memo.into());
        self
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
/// Posting request with the approved optional journal header fields.
pub struct PostEntry {
    pub entry_date: JournalDate,
    pub entry_time: Option<JournalTime>,
    pub lines: Vec<JournalLineInput>,
    pub reference: Option<String>,
    pub explanation: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Permanent relationship carried by an immutable posted entry.
pub enum PostedEntryRelationship {
    Ordinary,
    Reversal { original_entry_id: JournalEntryId },
    Correction { original_entry_id: JournalEntryId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Request to post an explicit correcting entry linked to existing history.
pub struct PostCorrection {
    pub original_entry_id: JournalEntryId,
    pub entry: PostEntry,
    pub explanation: String,
}

impl PostCorrection {
    pub fn new(
        original_entry_id: JournalEntryId,
        entry: PostEntry,
        explanation: impl Into<String>,
    ) -> Self {
        Self {
            original_entry_id,
            entry,
            explanation: explanation.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A user-editable draft. Draft lines may be incomplete or unbalanced.
pub struct SaveDraftEntry {
    pub entry_date: JournalDate,
    pub entry_time: Option<JournalTime>,
    pub lines: Vec<JournalLineInput>,
    pub reference: Option<String>,
    pub explanation: Option<String>,
}

impl SaveDraftEntry {
    pub fn new(entry_date: JournalDate, lines: Vec<JournalLineInput>) -> Self {
        Self {
            entry_date,
            entry_time: None,
            lines,
            reference: None,
            explanation: None,
        }
    }

    pub fn with_time(mut self, time: Option<JournalTime>) -> Self {
        self.entry_time = time;
        self
    }

    pub fn with_header(mut self, reference: Option<String>, explanation: Option<String>) -> Self {
        self.reference = reference;
        self.explanation = explanation;
        self
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
    pub memo: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Persisted draft returned by the repository.
pub struct DraftJournalEntry {
    pub id: JournalEntryId,
    pub entry_date: JournalDate,
    pub entry_time: Option<JournalTime>,
    pub revision: i64,
    pub created_at: String,
    pub reference: Option<String>,
    pub explanation: Option<String>,
    pub lines: Vec<DraftJournalLine>,
}

impl PostEntry {
    pub fn new(entry_date: JournalDate, lines: Vec<JournalLineInput>) -> Self {
        Self {
            entry_date,
            entry_time: None,
            lines,
            reference: None,
            explanation: None,
        }
    }

    pub fn with_time(mut self, time: Option<JournalTime>) -> Self {
        self.entry_time = time;
        self
    }

    pub fn with_header(mut self, reference: Option<String>, explanation: Option<String>) -> Self {
        self.reference = reference;
        self.explanation = explanation;
        self
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
    pub memo: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Immutable posted journal entry returned by the repository.
pub struct PostedJournalEntry {
    pub id: JournalEntryId,
    pub entry_date: JournalDate,
    pub entry_time: Option<JournalTime>,
    pub created_at: String,
    pub posted_at: String,
    pub relationship: PostedEntryRelationship,
    pub reference: Option<String>,
    pub explanation: Option<String>,
    pub lines: Vec<PostedJournalLine>,
}

#[derive(Debug, Error)]
/// Domain or persistence failure from an atomic posting attempt.
pub enum JournalError {
    #[error("explicitly void open-item allocations before reversing this entry")]
    OpenItemAllocationsActive,
    #[error("invalid journal date {0}; expected an existing date in YYYY-MM-DD form")]
    InvalidDate(String),
    #[error("invalid journal time {0}; expected HH:mm:ss")]
    InvalidTime(String),
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
    #[error("journal entry {0} is already directly reversed")]
    EntryAlreadyReversed(i64),
    #[error("journal entry {0} is itself a reversal and cannot be directly reversed")]
    CannotReverseReversal(i64),
    #[error("reversal date {reversal_date} cannot precede original date {original_date}")]
    ReversalDateBeforeOriginal {
        reversal_date: String,
        original_date: String,
    },
    #[error("a correcting journal entry requires a non-blank explanation")]
    CorrectionExplanationRequired,
    #[error("journal entry explanation contains a null character")]
    InvalidExplanation,
    #[error("journal reference or line memo contains a null character")]
    InvalidMetadata,
    #[error("correction header and request contain different explanations")]
    ConflictingExplanation,
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
    #[error("journal draft {0} revision is exhausted and cannot be advanced")]
    DraftRevisionExhausted(i64),
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
        validate_metadata(input.reference.as_deref(), input.explanation.as_deref())?;
        validate_draft_lines(&input.lines)?;

        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let (entry_id, revision, created_at): (i64, i64, String) = sqlx::query_as(
            r#"
            INSERT INTO journal_entries (entry_date, entry_time, reference, explanation)
            VALUES (?, ?, ?, ?)
            RETURNING id, revision, created_at
            "#,
        )
        .bind(input.entry_date.as_str())
        .bind(input.entry_time.as_ref().map(JournalTime::as_str))
        .bind(input.reference.as_deref())
        .bind(input.explanation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let lines = insert_draft_lines(&mut transaction, entry_id, &input.lines).await?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(DraftJournalEntry {
            id: JournalEntryId::new(entry_id),
            entry_date: input.entry_date,
            entry_time: input.entry_time,
            reference: input.reference,
            explanation: input.explanation,
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
        // Lock the writer before inspecting the existing document. Besides
        // preserving optimistic-concurrency semantics, this lets us reject a
        // malformed stored draft without another writer changing it between
        // validation and replacement.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(JournalError::Database)?;
        validate_stored_draft_for_mutation(&mut transaction, id, expected_revision, true).await?;
        validate_metadata(input.reference.as_deref(), input.explanation.as_deref())?;
        validate_draft_lines(&input.lines)?;
        let updated: Option<(i64, String)> = sqlx::query_as(
            r#"
            UPDATE journal_entries
            SET entry_date = ?,
                entry_time = ?,
                reference = ?,
                explanation = ?,
                revision = revision + 1
            WHERE id = ?
              AND status = 'draft'
              AND posted_at IS NULL
              AND revision = ?
              AND revision BETWEEN 1 AND ?
            RETURNING revision, created_at
            "#,
        )
        .bind(input.entry_date.as_str())
        .bind(input.entry_time.as_ref().map(JournalTime::as_str))
        .bind(input.reference.as_deref())
        .bind(input.explanation.as_deref())
        .bind(id.get())
        .bind(expected_revision)
        .bind(i64::MAX - 1)
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
            entry_time: input.entry_time,
            reference: input.reference,
            explanation: input.explanation,
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
            SELECT entry_date, entry_time, status, created_at, posted_at, revision, reference, explanation
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
        validate_stored_header_state(
            id,
            &header.status,
            header.revision,
            header.posted_at.as_deref(),
        )?;
        if header.status != "draft" {
            return Err(JournalError::EntryNotDraft(id.get()));
        }
        validate_metadata(header.reference.as_deref(), header.explanation.as_deref())?;
        let entry_date = parse_stored_date(id, &header.entry_date)?;
        let entry_time = parse_stored_time(header.entry_time.as_deref())?;
        let lines = load_draft_lines(&mut transaction, id).await?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(DraftJournalEntry {
            id,
            entry_date,
            entry_time,
            revision: header.revision,
            created_at: header.created_at,
            reference: header.reference,
            explanation: header.explanation,
            lines,
        })
    }

    /// Lists all editable drafts in accounting-date and stable-ID order.
    ///
    /// Headers and lines are read from one SQLite snapshot, so callers never
    /// receive a header from one revision paired with lines from another.
    pub async fn list_drafts(&self) -> Result<Vec<DraftJournalEntry>, JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let invalid_header: Option<(i64, String, i64, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, status, revision, posted_at
            FROM journal_entries
            WHERE status NOT IN ('draft', 'posted')
               OR revision < 1
               OR (status = 'posted' AND revision < 2)
               OR (status = 'draft' AND posted_at IS NOT NULL)
               OR (status = 'posted' AND posted_at IS NULL)
            ORDER BY id
            LIMIT 1
            "#,
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        if let Some((stored_id, status, revision, posted_at)) = invalid_header {
            let id = JournalEntryId::new(stored_id);
            validate_stored_header_state(id, &status, revision, posted_at.as_deref())?;
        }
        let headers: Vec<StoredDraftListHeader> = sqlx::query_as(
            r#"
            SELECT id, entry_date, entry_time, revision, created_at, reference, explanation
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
                   line.amount_minor,
                   line.memo,
                   EXISTS(
                       SELECT 1 FROM accounts AS account
                       WHERE account.id = line.account_id
                   ) AS account_exists,
                   EXISTS(
                       SELECT 1
                       FROM accounts AS account
                       WHERE account.id = line.account_id
                         AND account.kind = 'posting'
                         AND NOT EXISTS (
                             SELECT 1 FROM accounts AS child
                             WHERE child.parent_id = account.id
                         )
                   ) AS account_is_posting_leaf
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
                    memo: stored.memo,
                    account_exists: stored.account_exists,
                    account_is_posting_leaf: stored.account_is_posting_leaf,
                });
        }

        let mut drafts = Vec::with_capacity(headers.len());
        for header in headers {
            let StoredDraftListHeader {
                id: stored_id,
                entry_date: stored_date,
                entry_time: stored_time,
                revision,
                created_at,
                reference,
                explanation,
            } = header;
            let id = JournalEntryId::new(stored_id);
            validate_metadata(reference.as_deref(), explanation.as_deref())?;
            let stored_lines = lines_by_entry.remove(&stored_id).unwrap_or_default();
            validate_stored_line_order(id, &stored_lines)?;
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
                        memo: stored.memo,
                    })
                })
                .collect::<Result<Vec<_>, JournalError>>()?;
            drafts.push(DraftJournalEntry {
                id,
                entry_date: parse_stored_date(id, &stored_date)?,
                entry_time: parse_stored_time(stored_time.as_deref())?,
                revision,
                created_at,
                reference,
                explanation,
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
        // Deleting a damaged draft would hide corruption. Inspect it while
        // holding SQLite's writer slot, then delete only the exact revision
        // that was validated.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(JournalError::Database)?;
        validate_stored_draft_for_mutation(&mut transaction, id, expected_revision, false).await?;
        let deleted: Option<i64> = sqlx::query_scalar(
            r#"
            DELETE FROM journal_entries
            WHERE id = ?
              AND status = 'draft'
              AND posted_at IS NULL
              AND revision = ?
              AND revision >= 1
            RETURNING id
            "#,
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
        let header: Option<StoredPostedHeader> = sqlx::query_as(
            r#"
            SELECT entry_date, entry_time, created_at, status, revision, posted_at,
                   reverses_entry_id, corrects_entry_id, explanation, reference
            FROM journal_entries
            WHERE id = ? AND status = 'draft' AND revision = ?
            "#,
        )
        .bind(id.get())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let Some(header) = header else {
            return Err(
                entry_not_draft_error(&mut transaction, id, Some(expected_revision)).await?,
            );
        };
        validate_stored_header_state(
            id,
            &header.status,
            header.revision,
            header.posted_at.as_deref(),
        )?;
        validate_metadata(header.reference.as_deref(), header.explanation.as_deref())?;
        let relationship = decode_stored_relationship(
            id,
            header.reverses_entry_id,
            header.corrects_entry_id,
            header.explanation.as_deref(),
        )?;
        if expected_revision == i64::MAX {
            return Err(JournalError::DraftRevisionExhausted(id.get()));
        }
        let entry_date = parse_stored_date(id, &header.entry_date)?;
        let entry_time = parse_stored_time(header.entry_time.as_deref())?;
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
            WHERE id = ?
              AND status = 'draft'
              AND posted_at IS NULL
              AND revision = ?
            RETURNING posted_at
            "#,
        )
        .bind(id.get())
        .bind(expected_revision)
        .fetch_one(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        let lines = stored_lines_to_posted(id, stored_lines)?;
        transaction.commit().await.map_err(JournalError::Database)?;

        Ok(PostedJournalEntry {
            id,
            entry_date,
            entry_time,
            created_at: header.created_at,
            posted_at,
            relationship,
            reference: header.reference,
            explanation: header.explanation,
            lines,
        })
    }

    /// Validates and posts an entry as one SQLite transaction.
    ///
    /// This convenience path creates and posts without leaving a saved draft.
    pub async fn post(&self, input: PostEntry) -> Result<PostedJournalEntry, JournalError> {
        validate_journal_lines(&input.lines)?;

        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let posted = insert_and_post(
            &mut transaction,
            input,
            PostedEntryRelationship::Ordinary,
            None,
        )
        .await?;
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(posted)
    }

    /// Atomically posts an exact debit/credit-side reversal of one posted entry.
    pub async fn reverse_posted(
        &self,
        original_id: JournalEntryId,
        reversal_date: JournalDate,
    ) -> Result<PostedJournalEntry, JournalError> {
        self.reverse_posted_at(original_id, reversal_date, None)
            .await
    }

    pub async fn reverse_posted_at(
        &self,
        original_id: JournalEntryId,
        reversal_date: JournalDate,
        entry_time: Option<JournalTime>,
    ) -> Result<PostedJournalEntry, JournalError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(JournalError::Database)?;
        let original = load_posted_for_relationship(&mut transaction, original_id).await?;
        let allocated:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM open_item_allocations x JOIN journal_lines payment ON payment.id=x.settlement_line_id JOIN open_items item ON item.id=x.item_id JOIN open_item_batches batch ON batch.id=item.batch_id JOIN journal_lines source ON source.id=batch.source_line_id WHERE NOT EXISTS(SELECT 1 FROM open_item_allocation_voids v WHERE v.allocation_id=x.id) AND (source.entry_id=? OR payment.entry_id=?))").bind(original_id.get()).bind(original_id.get()).fetch_one(&mut *transaction).await.map_err(JournalError::Database)?;
        if allocated {
            return Err(JournalError::OpenItemAllocationsActive);
        }
        if matches!(
            original.relationship,
            PostedEntryRelationship::Reversal { .. }
        ) {
            return Err(JournalError::CannotReverseReversal(original_id.get()));
        }
        if reversal_date < original.entry_date {
            return Err(JournalError::ReversalDateBeforeOriginal {
                reversal_date: reversal_date.as_str().to_owned(),
                original_date: original.entry_date.as_str().to_owned(),
            });
        }
        let existing_reversal: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM journal_entries WHERE reverses_entry_id = ? LIMIT 1",
        )
        .bind(original_id.get())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JournalError::Database)?;
        if existing_reversal.is_some() {
            return Err(JournalError::EntryAlreadyReversed(original_id.get()));
        }

        let lines = original
            .lines
            .iter()
            .map(|line| {
                JournalLineInput::new(
                    line.account_id,
                    match line.side {
                        JournalSide::Debit => JournalSide::Credit,
                        JournalSide::Credit => JournalSide::Debit,
                    },
                    line.amount,
                )
            })
            .collect();
        let posted = insert_and_post(
            &mut transaction,
            PostEntry::new(reversal_date, lines).with_time(entry_time),
            PostedEntryRelationship::Reversal {
                original_entry_id: original_id,
            },
            None,
        )
        .await?;
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(posted)
    }

    /// Atomically posts a balanced correcting entry linked to existing history.
    pub async fn post_correction(
        &self,
        request: PostCorrection,
    ) -> Result<PostedJournalEntry, JournalError> {
        if request
            .entry
            .explanation
            .as_ref()
            .is_some_and(|text| text != &request.explanation)
        {
            return Err(JournalError::ConflictingExplanation);
        }
        validate_explanation(&request.explanation)?;
        validate_journal_lines(&request.entry.lines)?;

        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(JournalError::Database)?;
        load_posted_for_relationship(&mut transaction, request.original_entry_id).await?;
        let posted = insert_and_post(
            &mut transaction,
            request.entry,
            PostedEntryRelationship::Correction {
                original_entry_id: request.original_entry_id,
            },
            Some(request.explanation),
        )
        .await?;
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(posted)
    }

    /// Loads one immutable posted entry and all of its lines in one read snapshot.
    pub async fn get_posted(&self, id: JournalEntryId) -> Result<PostedJournalEntry, JournalError> {
        let mut transaction = self.pool.begin().await.map_err(JournalError::Database)?;
        let header: Option<StoredPostedHeader> = sqlx::query_as(
            r#"
            SELECT entry_date, entry_time, status, created_at, posted_at, revision,
                   reverses_entry_id, corrects_entry_id, explanation, reference
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
        validate_stored_header_state(
            id,
            &header.status,
            header.revision,
            header.posted_at.as_deref(),
        )?;
        if header.status != "posted" {
            return Err(JournalError::EntryNotPosted(id.get()));
        }
        let posted_at = header.posted_at.ok_or_else(|| {
            JournalError::InvalidStoredValue(format!(
                "posted entry {} has no posting timestamp",
                id.get()
            ))
        })?;
        validate_metadata(header.reference.as_deref(), header.explanation.as_deref())?;
        let entry_date = parse_stored_date(id, &header.entry_date)?;
        let entry_time = parse_stored_time(header.entry_time.as_deref())?;
        let stored_lines = load_stored_lines(&mut transaction, id).await?;
        let validation_lines = stored_lines_to_inputs(id, &stored_lines)?;
        validate_journal_lines(&validation_lines).map_err(|error| {
            JournalError::InvalidStoredValue(format!(
                "entry {} failed line validation: {error}",
                id.get()
            ))
        })?;

        let lines = stored_lines_to_posted(id, stored_lines)?;
        let relationship = decode_stored_relationship(
            id,
            header.reverses_entry_id,
            header.corrects_entry_id,
            header.explanation.as_deref(),
        )?;
        let posted = PostedJournalEntry {
            id,
            entry_date,
            entry_time,
            created_at: header.created_at,
            posted_at,
            relationship,
            reference: header.reference,
            explanation: header.explanation,
            lines,
        };
        validate_loaded_relationship(&mut transaction, &posted).await?;
        transaction.commit().await.map_err(JournalError::Database)?;
        Ok(posted)
    }
}

#[derive(Debug, sqlx::FromRow)]
struct StoredJournalHeader {
    entry_date: String,
    entry_time: Option<String>,
    status: String,
    created_at: String,
    posted_at: Option<String>,
    revision: i64,
    reference: Option<String>,
    explanation: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredPostedHeader {
    entry_date: String,
    entry_time: Option<String>,
    status: String,
    created_at: String,
    posted_at: Option<String>,
    revision: i64,
    reverses_entry_id: Option<i64>,
    corrects_entry_id: Option<i64>,
    explanation: Option<String>,
    reference: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredJournalLine {
    id: i64,
    line_no: i64,
    account_id: i64,
    side: String,
    amount_minor: i64,
    memo: Option<String>,
    account_exists: bool,
    account_is_posting_leaf: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredDraftListHeader {
    id: i64,
    entry_date: String,
    entry_time: Option<String>,
    revision: i64,
    created_at: String,
    reference: Option<String>,
    explanation: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredDraftListLine {
    entry_id: i64,
    id: i64,
    line_no: i64,
    account_id: i64,
    side: String,
    amount_minor: i64,
    memo: Option<String>,
    account_exists: bool,
    account_is_posting_leaf: bool,
}

async fn insert_and_post(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    input: PostEntry,
    relationship: PostedEntryRelationship,
    explanation: Option<String>,
) -> Result<PostedJournalEntry, JournalError> {
    validate_draft_lines(&input.lines)?;
    validate_journal_lines(&input.lines)?;
    let explanation = explanation.or(input.explanation);
    validate_metadata(input.reference.as_deref(), explanation.as_deref())?;
    let (reverses_entry_id, corrects_entry_id) = match relationship {
        PostedEntryRelationship::Ordinary => (None, None),
        PostedEntryRelationship::Reversal { original_entry_id } => {
            (Some(original_entry_id.get()), None)
        }
        PostedEntryRelationship::Correction { original_entry_id } => {
            (None, Some(original_entry_id.get()))
        }
    };
    let (entry_id, created_at): (i64, String) = sqlx::query_as(
        r#"
        INSERT INTO journal_entries (
            entry_date, entry_time, reverses_entry_id, corrects_entry_id, explanation, reference
        )
        VALUES (?, ?, ?, ?, ?, ?)
        RETURNING id, created_at
        "#,
    )
    .bind(input.entry_date.as_str())
    .bind(input.entry_time.as_ref().map(JournalTime::as_str))
    .bind(reverses_entry_id)
    .bind(corrects_entry_id)
    .bind(explanation.as_deref())
    .bind(input.reference.as_deref())
    .fetch_one(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;

    let mut posted_lines = Vec::with_capacity(input.lines.len());
    for (line_index, line) in input.lines.iter().enumerate() {
        require_postable_account(transaction, line.account_id).await?;
        let line_no = i64::try_from(line_index + 1).map_err(|_| JournalError::TooManyLines)?;
        let line_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO journal_lines (
                entry_id, line_no, account_id, side, amount_minor, memo
            )
            VALUES (?, ?, ?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(line.account_id.get())
        .bind(line.side.as_str())
        .bind(line.amount.get())
        .bind(line.memo.as_deref())
        .fetch_one(&mut **transaction)
        .await
        .map_err(JournalError::Database)?;
        posted_lines.push(PostedJournalLine {
            id: JournalLineId::new(line_id),
            line_no,
            account_id: line.account_id,
            side: line.side,
            amount: line.amount,
            memo: line.memo.clone(),
        });
    }

    let posted_at: String = sqlx::query_scalar(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            revision = revision + 1
        WHERE id = ?
          AND status = 'draft'
          AND posted_at IS NULL
          AND revision = 1
        RETURNING posted_at
        "#,
    )
    .bind(entry_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;

    Ok(PostedJournalEntry {
        id: JournalEntryId::new(entry_id),
        entry_date: input.entry_date,
        entry_time: input.entry_time,
        created_at,
        posted_at,
        relationship,
        reference: input.reference,
        explanation,
        lines: posted_lines,
    })
}

async fn load_posted_for_relationship(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
) -> Result<PostedJournalEntry, JournalError> {
    let header: Option<StoredPostedHeader> = sqlx::query_as(
        r#"
        SELECT entry_date, entry_time, status, created_at, posted_at, revision,
               reverses_entry_id, corrects_entry_id, explanation, reference
        FROM journal_entries
        WHERE id = ?
        "#,
    )
    .bind(id.get())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;
    let Some(header) = header else {
        return Err(JournalError::EntryNotFound(id.get()));
    };
    validate_stored_header_state(
        id,
        &header.status,
        header.revision,
        header.posted_at.as_deref(),
    )?;
    if header.status != "posted" {
        return Err(JournalError::EntryNotPosted(id.get()));
    }
    let posted_at = header.posted_at.ok_or_else(|| {
        JournalError::InvalidStoredValue(format!(
            "posted entry {} has no posting timestamp",
            id.get()
        ))
    })?;
    validate_metadata(header.reference.as_deref(), header.explanation.as_deref())?;
    let stored_lines = load_stored_lines(transaction, id).await?;
    let validation_lines = stored_lines_to_inputs(id, &stored_lines)?;
    validate_journal_lines(&validation_lines).map_err(|error| {
        JournalError::InvalidStoredValue(format!(
            "entry {} failed line validation: {error}",
            id.get()
        ))
    })?;
    let relationship = decode_stored_relationship(
        id,
        header.reverses_entry_id,
        header.corrects_entry_id,
        header.explanation.as_deref(),
    )?;
    let posted = PostedJournalEntry {
        id,
        entry_date: parse_stored_date(id, &header.entry_date)?,
        entry_time: parse_stored_time(header.entry_time.as_deref())?,
        created_at: header.created_at,
        posted_at,
        relationship,
        reference: header.reference,
        explanation: header.explanation,
        lines: stored_lines_to_posted(id, stored_lines)?,
    };
    validate_loaded_relationship(transaction, &posted).await?;
    Ok(posted)
}

fn decode_stored_relationship(
    id: JournalEntryId,
    reverses_entry_id: Option<i64>,
    corrects_entry_id: Option<i64>,
    explanation: Option<&str>,
) -> Result<PostedEntryRelationship, JournalError> {
    if explanation.is_some_and(|value| value.contains('\0')) {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} explanation contains a null character",
            id.get()
        )));
    }
    match (reverses_entry_id, corrects_entry_id) {
        (None, None) => Ok(PostedEntryRelationship::Ordinary),
        (Some(original), None) if original > 0 && original != id.get() => {
            Ok(PostedEntryRelationship::Reversal {
                original_entry_id: JournalEntryId::new(original),
            })
        }
        (None, Some(original)) if original > 0 && original != id.get() => {
            if explanation.is_none_or(|value| value.trim().is_empty()) {
                return Err(JournalError::InvalidStoredValue(format!(
                    "correcting entry {} has no explanation",
                    id.get()
                )));
            }
            Ok(PostedEntryRelationship::Correction {
                original_entry_id: JournalEntryId::new(original),
            })
        }
        (Some(_), Some(_)) => Err(JournalError::InvalidStoredValue(format!(
            "entry {} is both a reversal and a correction",
            id.get()
        ))),
        _ => Err(JournalError::InvalidStoredValue(format!(
            "entry {} has an invalid related entry identifier",
            id.get()
        ))),
    }
}

async fn validate_loaded_relationship(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry: &PostedJournalEntry,
) -> Result<(), JournalError> {
    let original_id = match entry.relationship {
        PostedEntryRelationship::Ordinary => return Ok(()),
        PostedEntryRelationship::Reversal { original_entry_id }
        | PostedEntryRelationship::Correction { original_entry_id } => original_entry_id,
    };
    let original: Option<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT entry_date, status, reverses_entry_id FROM journal_entries WHERE id = ?",
    )
    .bind(original_id.get())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;
    let Some((original_date, status, original_reverses)) = original else {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} refers to missing entry {}",
            entry.id.get(),
            original_id.get()
        )));
    };
    if status != "posted" {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} refers to non-posted entry {}",
            entry.id.get(),
            original_id.get()
        )));
    }
    if !matches!(entry.relationship, PostedEntryRelationship::Reversal { .. }) {
        return Ok(());
    }
    if original_reverses.is_some() {
        return Err(JournalError::InvalidStoredValue(format!(
            "reversal {} reverses another reversal {}",
            entry.id.get(),
            original_id.get()
        )));
    }
    let original_date = parse_stored_date(original_id, &original_date)?;
    if entry.entry_date < original_date {
        return Err(JournalError::InvalidStoredValue(format!(
            "reversal {} predates original entry {}",
            entry.id.get(),
            original_id.get()
        )));
    }
    let original_lines = load_stored_lines(transaction, original_id).await?;
    let original_lines = stored_lines_to_inputs(original_id, &original_lines)?;
    if entry.lines.len() != original_lines.len()
        || entry
            .lines
            .iter()
            .zip(original_lines)
            .any(|(reversal, original)| {
                reversal.account_id != original.account_id
                    || reversal.amount != original.amount
                    || reversal.side == original.side
            })
    {
        return Err(JournalError::InvalidStoredValue(format!(
            "reversal {} does not exactly exchange original entry {} sides",
            entry.id.get(),
            original_id.get()
        )));
    }
    Ok(())
}

fn validate_explanation(explanation: &str) -> Result<(), JournalError> {
    if explanation.contains('\0') {
        return Err(JournalError::InvalidExplanation);
    }
    if explanation.trim().is_empty() {
        return Err(JournalError::CorrectionExplanationRequired);
    }
    Ok(())
}

fn validate_metadata(
    reference: Option<&str>,
    explanation: Option<&str>,
) -> Result<(), JournalError> {
    if reference.is_some_and(|text| text.contains('\0')) {
        return Err(JournalError::InvalidMetadata);
    }
    if explanation.is_some_and(|text| text.contains('\0')) {
        return Err(JournalError::InvalidExplanation);
    }
    Ok(())
}

fn validate_draft_lines(lines: &[JournalLineInput]) -> Result<(), JournalError> {
    if lines
        .iter()
        .any(|line| line.memo.as_deref().is_some_and(|text| text.contains('\0')))
    {
        return Err(JournalError::InvalidMetadata);
    }
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
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor, memo)
            VALUES (?, ?, ?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(line.account_id.get())
        .bind(line.side.as_str())
        .bind(line.amount.get())
        .bind(line.memo.as_deref())
        .fetch_one(&mut **transaction)
        .await
        .map_err(JournalError::Database)?;
        saved.push(DraftJournalLine {
            id: JournalLineId::new(line_id),
            line_no,
            account_id: line.account_id,
            side: line.side,
            amount: line.amount,
            memo: line.memo.clone(),
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
        SELECT line.id,
               line.line_no,
               line.account_id,
               line.side,
               line.amount_minor,
               line.memo,
               EXISTS(
                   SELECT 1 FROM accounts AS account
                   WHERE account.id = line.account_id
               ) AS account_exists,
               EXISTS(
                   SELECT 1
                   FROM accounts AS account
                   WHERE account.id = line.account_id
                     AND account.kind = 'posting'
                     AND NOT EXISTS (
                         SELECT 1 FROM accounts AS child
                         WHERE child.parent_id = account.id
                     )
               ) AS account_is_posting_leaf
        FROM journal_lines AS line
        WHERE line.entry_id = ?
        ORDER BY line.line_no, line.id
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
    let stored_lines = load_stored_lines(transaction, id).await?;
    validate_stored_line_order(id, &stored_lines)?;
    stored_lines
        .into_iter()
        .map(|stored| {
            let (account_id, side, amount) = stored_line_values(id, &stored)?;
            Ok(DraftJournalLine {
                id: JournalLineId::new(stored.id),
                line_no: stored.line_no,
                account_id,
                side,
                amount,
                memo: stored.memo,
            })
        })
        .collect()
}

fn stored_lines_to_inputs(
    id: JournalEntryId,
    lines: &[StoredJournalLine],
) -> Result<Vec<JournalLineInput>, JournalError> {
    validate_stored_line_order(id, lines)?;
    lines
        .iter()
        .map(|stored| {
            let (account_id, side, amount) = stored_line_values(id, stored)?;
            Ok(JournalLineInput {
                account_id,
                side,
                amount,
                memo: stored.memo.clone(),
            })
        })
        .collect()
}

fn stored_lines_to_posted(
    id: JournalEntryId,
    lines: Vec<StoredJournalLine>,
) -> Result<Vec<PostedJournalLine>, JournalError> {
    validate_stored_line_order(id, &lines)?;
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
                memo: stored.memo,
            })
        })
        .collect()
}

fn validate_stored_line_order(
    entry_id: JournalEntryId,
    lines: &[StoredJournalLine],
) -> Result<(), JournalError> {
    for (index, line) in lines.iter().enumerate() {
        let expected = i64::try_from(index + 1).map_err(|_| {
            JournalError::InvalidStoredValue(format!(
                "entry {} has more lines than SQLite can number",
                entry_id.get()
            ))
        })?;
        if line.line_no != expected {
            return Err(JournalError::InvalidStoredValue(format!(
                "entry {} line {} expected line number {expected}, found {}",
                entry_id.get(),
                line.id,
                line.line_no
            )));
        }
    }
    Ok(())
}

fn stored_line_values(
    entry_id: JournalEntryId,
    stored: &StoredJournalLine,
) -> Result<(AccountId, JournalSide, MinorUnits), JournalError> {
    if stored
        .memo
        .as_deref()
        .is_some_and(|text| text.contains('\0'))
    {
        return Err(JournalError::InvalidStoredValue(
            "line memo contains a null character".to_owned(),
        ));
    }

    if stored.id <= 0 {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} has non-positive line identifier {}",
            entry_id.get(),
            stored.id
        )));
    }
    if stored.account_id <= 0 {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} line {} has non-positive account identifier {}",
            entry_id.get(),
            stored.id,
            stored.account_id
        )));
    }
    if !stored.account_exists {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} line {} refers to missing account {}",
            entry_id.get(),
            stored.id,
            stored.account_id
        )));
    }
    if !stored.account_is_posting_leaf {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} line {} refers to non-posting-leaf account {}",
            entry_id.get(),
            stored.id,
            stored.account_id
        )));
    }
    if stored.amount_minor <= 0 {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} line {} has non-positive amount {}",
            entry_id.get(),
            stored.id,
            stored.amount_minor
        )));
    }
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

fn parse_stored_time(value: Option<&str>) -> Result<Option<JournalTime>, JournalError> {
    value
        .map(JournalTime::parse)
        .transpose()
        .map_err(|error| JournalError::InvalidStoredValue(error.to_string()))
}

fn parse_stored_date(id: JournalEntryId, stored: &str) -> Result<JournalDate, JournalError> {
    JournalDate::parse(stored).map_err(|_| {
        JournalError::InvalidStoredValue(format!("entry {} has invalid date {stored}", id.get()))
    })
}

fn validate_stored_header_state(
    id: JournalEntryId,
    status: &str,
    revision: i64,
    posted_at: Option<&str>,
) -> Result<(), JournalError> {
    if !matches!(status, "draft" | "posted") {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} has invalid status {status}",
            id.get()
        )));
    }
    if revision < 1 {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} has invalid revision {revision}",
            id.get()
        )));
    }
    if status == "posted" && revision < 2 {
        return Err(JournalError::InvalidStoredValue(format!(
            "entry {} has invalid posted revision {revision}",
            id.get()
        )));
    }
    match (status, posted_at) {
        ("draft", Some(_)) => Err(JournalError::InvalidStoredValue(format!(
            "draft entry {} unexpectedly has a posting timestamp",
            id.get()
        ))),
        ("posted", None) => Err(JournalError::InvalidStoredValue(format!(
            "posted entry {} has no posting timestamp",
            id.get()
        ))),
        _ => Ok(()),
    }
}

async fn entry_not_draft_error(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
    expected_revision: Option<i64>,
) -> Result<JournalError, JournalError> {
    let state: Option<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT status, revision, posted_at FROM journal_entries WHERE id = ?")
            .bind(id.get())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(JournalError::Database)?;
    if let Some((status, revision, posted_at)) = &state
        && let Err(error) =
            validate_stored_header_state(id, status, *revision, posted_at.as_deref())
    {
        return Ok(error);
    }
    Ok(match (state, expected_revision) {
        (Some((status, actual_revision, _)), Some(expected_revision))
            if status == "draft"
                && expected_revision == actual_revision
                && actual_revision == i64::MAX =>
        {
            JournalError::DraftRevisionExhausted(id.get())
        }
        (Some((status, actual_revision, _)), Some(expected_revision))
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

async fn validate_stored_draft_for_mutation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: JournalEntryId,
    expected_revision: i64,
    advances_revision: bool,
) -> Result<(), JournalError> {
    let header: Option<StoredPostedHeader> = sqlx::query_as(
        r#"
        SELECT entry_date, entry_time, status, created_at, revision, posted_at, reference, explanation, reverses_entry_id, corrects_entry_id
        FROM journal_entries
        WHERE id = ?
        "#,
    )
    .bind(id.get())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(JournalError::Database)?;
    let Some(header) = header else {
        return Err(JournalError::EntryNotFound(id.get()));
    };
    let revision = header.revision;
    validate_stored_header_state(id, &header.status, revision, header.posted_at.as_deref())?;
    if header.status != "draft" {
        return Err(JournalError::EntryNotDraft(id.get()));
    }
    if revision != expected_revision {
        return Err(JournalError::StaleDraftRevision {
            id: id.get(),
            expected_revision,
            actual_revision: revision,
        });
    }
    if advances_revision && revision == i64::MAX {
        return Err(JournalError::DraftRevisionExhausted(id.get()));
    }

    validate_metadata(header.reference.as_deref(), header.explanation.as_deref())?;
    parse_stored_date(id, &header.entry_date)?;
    parse_stored_time(header.entry_time.as_deref())?;
    load_draft_lines(transaction, id).await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory_database;

    #[tokio::test]
    async fn replace_and_delete_do_not_hide_a_corrupt_stored_draft() {
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
        sqlx::query(
            "UPDATE journal_entries SET entry_date = 'not-a-date', revision = revision + 1 WHERE id = ?",
        )
        .bind(draft.id.get())
        .execute(&pool)
        .await
        .expect("test setup should simulate a bypassed date constraint");
        sqlx::query("PRAGMA ignore_check_constraints = OFF")
            .execute(&pool)
            .await
            .unwrap();
        let corrupted_revision = draft.revision + 1;

        let replace_error = journal
            .replace_draft(
                draft.id,
                corrupted_revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await
            .expect_err("replacement must not silently repair a corrupt stored date");
        assert!(
            matches!(replace_error, JournalError::InvalidStoredValue(message) if message.contains("invalid date not-a-date"))
        );

        let delete_error = journal
            .delete_draft(draft.id, corrupted_revision)
            .await
            .expect_err("deletion must not silently erase a corrupt stored draft");
        assert!(
            matches!(delete_error, JournalError::InvalidStoredValue(message) if message.contains("invalid date not-a-date"))
        );

        let stored: (String, String, i64) =
            sqlx::query_as("SELECT entry_date, status, revision FROM journal_entries WHERE id = ?")
                .bind(draft.id.get())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stored,
            (
                "not-a-date".to_owned(),
                "draft".to_owned(),
                corrupted_revision
            )
        );

        let account_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, element, kind) VALUES ('測試帳戶', 'asset', 'posting') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let malformed_lines = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-24").unwrap(),
                vec![JournalLineInput::new(
                    AccountId::new(account_id),
                    JournalSide::Debit,
                    MinorUnits::new(100),
                )],
            ))
            .await
            .unwrap();
        sqlx::query("UPDATE journal_lines SET line_no = 2 WHERE id = ?")
            .bind(malformed_lines.lines[0].id.get())
            .execute(&pool)
            .await
            .expect("test setup should create a gap in the stored line sequence");

        for mutation_error in [
            journal
                .replace_draft(
                    malformed_lines.id,
                    malformed_lines.revision,
                    SaveDraftEntry::new(JournalDate::parse("2026-10-25").unwrap(), vec![]),
                )
                .await
                .expect_err("replacement must not hide malformed stored lines"),
            journal
                .delete_draft(malformed_lines.id, malformed_lines.revision)
                .await
                .expect_err("deletion must not erase malformed stored lines"),
        ] {
            assert!(
                matches!(mutation_error, JournalError::InvalidStoredValue(message) if message.contains("expected line number 1, found 2"))
            );
        }
        let malformed_line_no: i64 =
            sqlx::query_scalar("SELECT line_no FROM journal_lines WHERE entry_id = ?")
                .bind(malformed_lines.id.get())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(malformed_line_no, 2);
    }

    #[tokio::test]
    async fn posted_read_rejects_an_impossible_initial_revision() {
        let pool = open_memory_database().await.expect("database should open");
        let journal = JournalRepository::new(pool.clone());
        let cash_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, element, kind) VALUES ('現金', 'asset', 'posting') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let revenue_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, element, kind) VALUES ('收入', 'income', 'posting') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let posted = journal
            .post(PostEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![
                    JournalLineInput::new(
                        AccountId::new(cash_id),
                        JournalSide::Debit,
                        MinorUnits::new(100),
                    ),
                    JournalLineInput::new(
                        AccountId::new(revenue_id),
                        JournalSide::Credit,
                        MinorUnits::new(100),
                    ),
                ],
            ))
            .await
            .unwrap();

        sqlx::query("DROP TRIGGER journal_entries_posted_immutable_update")
            .execute(&pool)
            .await
            .expect("test setup should bypass posted immutability");
        sqlx::query("UPDATE journal_entries SET revision = 1 WHERE id = ?")
            .bind(posted.id.get())
            .execute(&pool)
            .await
            .expect("test setup should simulate an impossible posted revision");

        let error = journal
            .get_posted(posted.id)
            .await
            .expect_err("a posted entry cannot still have its initial draft revision");
        assert!(
            matches!(error, JournalError::InvalidStoredValue(message) if message.contains("invalid posted revision 1"))
        );
        assert!(
            matches!(journal.list_drafts().await, Err(JournalError::InvalidStoredValue(message)) if message.contains("invalid posted revision 1"))
        );
    }

    #[tokio::test]
    async fn draft_operations_reject_an_orphaned_stored_account_reference() {
        let pool = open_memory_database().await.expect("database should open");
        let journal = JournalRepository::new(pool.clone());
        let account_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, element, kind) VALUES ('現金', 'asset', 'posting') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let draft = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![JournalLineInput::new(
                    AccountId::new(account_id),
                    JournalSide::Debit,
                    MinorUnits::new(100),
                )],
            ))
            .await
            .unwrap();

        sqlx::query("DROP TRIGGER journal_lines_validate_account_update")
            .execute(&pool)
            .await
            .expect("test setup should bypass account eligibility validation");
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE journal_lines SET account_id = 999999 WHERE id = ?")
            .bind(draft.lines[0].id.get())
            .execute(&pool)
            .await
            .expect("test setup should simulate an orphaned line account");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();

        let read_error = journal
            .get_draft(draft.id)
            .await
            .expect_err("an orphaned account reference must not be returned");
        assert!(
            matches!(read_error, JournalError::InvalidStoredValue(message) if message.contains("missing account 999999"))
        );
        for mutation_error in [
            journal
                .replace_draft(
                    draft.id,
                    draft.revision,
                    SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
                )
                .await
                .expect_err("replacement must not hide an orphaned account reference"),
            journal
                .delete_draft(draft.id, draft.revision)
                .await
                .expect_err("deletion must not erase an orphaned account reference"),
        ] {
            assert!(
                matches!(mutation_error, JournalError::InvalidStoredValue(message) if message.contains("missing account 999999"))
            );
        }
    }

    #[tokio::test]
    async fn draft_operations_reject_a_corrupted_non_posting_account_reference() {
        let pool = open_memory_database().await.expect("database should open");
        let journal = JournalRepository::new(pool.clone());
        let account_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, element, kind) VALUES ('現金', 'asset', 'posting') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let draft = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![JournalLineInput::new(
                    AccountId::new(account_id),
                    JournalSide::Debit,
                    MinorUnits::new(100),
                )],
            ))
            .await
            .unwrap();

        sqlx::query("DROP TRIGGER accounts_preserve_journal_classification")
            .execute(&pool)
            .await
            .expect("test setup should bypass journal classification immutability");
        sqlx::query("UPDATE accounts SET kind = 'category', revision = revision + 1 WHERE id = ?")
            .bind(account_id)
            .execute(&pool)
            .await
            .expect("test setup should simulate a reclassified referenced account");

        for error in [
            journal.get_draft(draft.id).await.unwrap_err(),
            journal
                .replace_draft(
                    draft.id,
                    draft.revision,
                    SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
                )
                .await
                .unwrap_err(),
            journal
                .delete_draft(draft.id, draft.revision)
                .await
                .unwrap_err(),
        ] {
            assert!(
                matches!(error, JournalError::InvalidStoredValue(message) if message.contains("non-posting-leaf account"))
            );
        }
    }

    #[tokio::test]
    async fn stale_replace_reports_the_current_revision_before_validating_new_lines() {
        let pool = open_memory_database().await.expect("database should open");
        let journal = JournalRepository::new(pool);
        let original = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![],
            ))
            .await
            .unwrap();
        let current = journal
            .replace_draft(
                original.id,
                original.revision,
                SaveDraftEntry::new(JournalDate::parse("2026-10-23").unwrap(), vec![]),
            )
            .await
            .unwrap();

        let error = journal
            .replace_draft(
                original.id,
                original.revision,
                SaveDraftEntry::new(
                    JournalDate::parse("2026-10-24").unwrap(),
                    vec![JournalLineInput::new(
                        AccountId::new(999_999),
                        JournalSide::Debit,
                        MinorUnits::ZERO,
                    )],
                ),
            )
            .await
            .expect_err("a stale client must receive the current revision first");
        assert!(matches!(
            error,
            JournalError::StaleDraftRevision {
                id,
                expected_revision,
                actual_revision,
            } if id == original.id.get()
                && expected_revision == original.revision
                && actual_revision == current.revision
        ));
    }
}
