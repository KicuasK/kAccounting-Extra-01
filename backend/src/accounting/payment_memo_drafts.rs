//! Explicitly requested payment candidates; posting remains a separate action.
use super::{journal::JournalDate, payment_memos::MemoError};
use sqlx::SqlitePool;
#[derive(Clone, Debug)]
pub struct PaymentProgress {
    pub paid_amount_minor: i64,
    pub remaining_minor: i64,
    pub payment_draft_id: Option<i64>,
}
pub async fn progress(pool: &SqlitePool, id: i64) -> Result<PaymentProgress, MemoError> {
    let mut tx = pool.begin().await?;
    let amount: Option<i64> =
        sqlx::query_scalar("SELECT amount_minor FROM payment_memos WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let amount = amount.ok_or(MemoError::NotFound)?;
    let (paid, draft) = progress_tx(&mut tx, id).await?;
    tx.commit().await?;
    Ok(PaymentProgress {
        paid_amount_minor: paid,
        remaining_minor: amount.checked_sub(paid).ok_or(MemoError::Overflow)?,
        payment_draft_id: draft,
    })
}
async fn progress_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
) -> Result<(i64, Option<i64>), MemoError> {
    let payments:Vec<i64>=sqlx::query_scalar("SELECT l.amount_minor FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id WHERE l.memo_id=? AND e.status='posted' AND NOT EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted')").bind(id).fetch_all(&mut **tx).await?;
    let mut paid = 0_i64;
    for amount in payments {
        paid = paid.checked_add(amount).ok_or(MemoError::Overflow)?;
    }
    let draft=sqlx::query_scalar("SELECT l.entry_id FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id WHERE l.memo_id=? AND e.status='draft'").bind(id).fetch_optional(&mut **tx).await?;
    Ok((paid, draft))
}
pub async fn create_payment_draft(
    pool: &SqlitePool,
    id: i64,
    revision: i64,
    date: &str,
    debit: i64,
    asset: i64,
    amount: i64,
) -> Result<i64, MemoError> {
    JournalDate::parse(date).map_err(|_| MemoError::Invalid("entry_date"))?;
    if debit <= 0 || asset <= 0 || debit == asset || amount <= 0 {
        return Err(MemoError::Invalid("payment fields"));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let memo: Option<(i64, i64, String, String)> =
        sqlx::query_as("SELECT revision,amount_minor,status,title FROM payment_memos WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (current, total, status, title) = memo.ok_or(MemoError::NotFound)?;
    if current != revision || status != "planned" {
        return Err(MemoError::Conflict);
    }
    let (paid, draft) = progress_tx(&mut tx, id).await?;
    if draft.is_some() || amount > total.checked_sub(paid).ok_or(MemoError::Overflow)? {
        return Err(MemoError::Conflict);
    }
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM accounts a WHERE a.id IN (?,?) AND a.kind='posting' AND a.is_active=1 AND NOT EXISTS(SELECT 1 FROM accounts c WHERE c.parent_id=a.id) AND (a.id<>? OR a.element='asset')").bind(debit).bind(asset).bind(asset).fetch_one(&mut *tx).await?;
    if count != 2 {
        return Err(MemoError::Invalid(
            "active posting accounts and payment asset required",
        ));
    }
    let entry: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries(entry_date,explanation) VALUES(?,?) RETURNING id",
    )
    .bind(date)
    .bind(format!("付款備忘錄：{title}"))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO journal_lines(entry_id,line_no,account_id,side,amount_minor) VALUES(?,1,?,'debit',?),(?,2,?,'credit',?)").bind(entry).bind(debit).bind(amount).bind(entry).bind(asset).bind(amount).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO payment_memo_drafts(entry_id,memo_id,memo_revision,amount_minor,debit_account_id,payment_asset_id) VALUES(?,?,?,?,?,?)").bind(entry).bind(id).bind(revision).bind(amount).bind(debit).bind(asset).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(entry)
}
