//! Non-posting payment plans. No account balances or journal rows are changed.
use super::journal::JournalDate;
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, FromRow)]
pub struct PaymentMemo {
    pub id: i64,
    pub revision: i64,
    pub due_date: String,
    pub title: String,
    pub payee: String,
    pub note: String,
    pub amount_minor: i64,
    pub necessity: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    #[sqlx(default)]
    pub paid_amount_minor: i64,
    #[sqlx(default)]
    pub payment_draft_id: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct MemoInput {
    pub due_date: String,
    pub title: String,
    pub payee: String,
    pub note: String,
    pub amount_minor: i64,
    pub necessity: String,
}
#[derive(Debug, Error)]
pub enum MemoError {
    #[error("invalid payment memo: {0}")]
    Invalid(&'static str),
    #[error("payment memo not found")]
    NotFound,
    #[error("payment memo revision or state changed")]
    Conflict,
    #[error("payment memo total exceeds integer capacity")]
    Overflow,
    #[error(transparent)]
    Database(sqlx::Error),
}
impl From<sqlx::Error> for MemoError {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error().is_some_and(|e| {
            e.message()
                .contains("payment memo has a draft or effective payment")
        }) {
            Self::Conflict
        } else {
            Self::Database(e)
        }
    }
}
impl MemoInput {
    fn validate(mut self) -> Result<Self, MemoError> {
        JournalDate::parse(&self.due_date).map_err(|_| MemoError::Invalid("due_date"))?;
        self.title = self.title.trim().to_owned();
        self.payee = self.payee.trim().to_owned();
        self.note = self.note.trim().to_owned();
        if self.title.is_empty()
            || self.title.chars().count() > 200
            || self.payee.chars().count() > 200
            || self.note.chars().count() > 2000
            || self.amount_minor <= 0
            || !["required", "optional"].contains(&self.necessity.as_str())
            || self
                .title
                .chars()
                .chain(self.payee.chars())
                .any(char::is_control)
            || self
                .note
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(MemoError::Invalid("fields"));
        }
        Ok(self)
    }
}
#[derive(Clone)]
pub struct PaymentMemoRepository {
    pool: SqlitePool,
}
impl PaymentMemoRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list(
        &self,
        from: &str,
        through: &str,
    ) -> Result<(Vec<PaymentMemo>, i64), MemoError> {
        JournalDate::parse(from).map_err(|_| MemoError::Invalid("from"))?;
        JournalDate::parse(through).map_err(|_| MemoError::Invalid("through"))?;
        if from > through {
            return Err(MemoError::Invalid("date range"));
        }
        let items=sqlx::query_as::<_,PaymentMemo>("SELECT m.*,COALESCE((SELECT SUM(l.amount_minor) FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id WHERE l.memo_id=m.id AND e.status='posted' AND NOT EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted')),0) paid_amount_minor,(SELECT l.entry_id FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id WHERE l.memo_id=m.id AND e.status='draft') payment_draft_id FROM payment_memos m WHERE due_date BETWEEN ? AND ? ORDER BY due_date,id").bind(from).bind(through).fetch_all(&self.pool).await?;
        let mut total = 0_i64;
        for item in &items {
            if item.status == "planned" && item.necessity == "required" {
                total = total
                    .checked_add(
                        item.amount_minor
                            .checked_sub(item.paid_amount_minor)
                            .ok_or(MemoError::Overflow)?,
                    )
                    .ok_or(MemoError::Overflow)?;
            }
        }
        Ok((items, total))
    }
    pub async fn create(&self, input: MemoInput) -> Result<PaymentMemo, MemoError> {
        let i = input.validate()?;
        Ok(sqlx::query_as("INSERT INTO payment_memos(revision,due_date,title,payee,note,amount_minor,necessity,status) VALUES(1,?,?,?,?,?,?,'planned') RETURNING *").bind(i.due_date).bind(i.title).bind(i.payee).bind(i.note).bind(i.amount_minor).bind(i.necessity).fetch_one(&self.pool).await?)
    }
    pub async fn update(
        &self,
        id: i64,
        revision: i64,
        input: MemoInput,
    ) -> Result<PaymentMemo, MemoError> {
        let i = input.validate()?;
        let result=sqlx::query_as("UPDATE payment_memos SET revision=revision+1,due_date=?,title=?,payee=?,note=?,amount_minor=?,necessity=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND revision=? AND status='planned' AND revision<9223372036854775807 RETURNING *").bind(i.due_date).bind(i.title).bind(i.payee).bind(i.note).bind(i.amount_minor).bind(i.necessity).bind(id).bind(revision).fetch_optional(&self.pool).await?;
        self.resolve(id, result).await
    }
    pub async fn transition(
        &self,
        id: i64,
        revision: i64,
        cancel: bool,
    ) -> Result<PaymentMemo, MemoError> {
        let (old, new) = if cancel {
            ("planned", "cancelled")
        } else {
            ("cancelled", "planned")
        };
        let result=sqlx::query_as("UPDATE payment_memos SET revision=revision+1,status=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND revision=? AND status=? AND revision<9223372036854775807 RETURNING *").bind(new).bind(id).bind(revision).bind(old).fetch_optional(&self.pool).await?;
        self.resolve(id, result).await
    }
    async fn resolve(
        &self,
        id: i64,
        result: Option<PaymentMemo>,
    ) -> Result<PaymentMemo, MemoError> {
        if let Some(item) = result {
            return Ok(item);
        }
        if sqlx::query_scalar::<_, i64>("SELECT id FROM payment_memos WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .is_some()
        {
            Err(MemoError::Conflict)
        } else {
            Err(MemoError::NotFound)
        }
    }
}
