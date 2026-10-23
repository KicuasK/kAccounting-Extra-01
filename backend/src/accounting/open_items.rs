//! Explicit, append-only AR/AP annotations of immutable posted journal lines.
use super::journal::JournalDate;
use sqlx::{Sqlite, SqlitePool, Transaction};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingKind {
    Receivable,
    Payable,
}
impl TrackingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Receivable => "receivable",
            Self::Payable => "payable",
        }
    }
    fn source_side(self) -> &'static str {
        match self {
            Self::Receivable => "debit",
            Self::Payable => "credit",
        }
    }
    fn element(self) -> &'static str {
        match self {
            Self::Receivable => "asset",
            Self::Payable => "liability",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationKind {
    Payment,
    Adjustment,
}
impl AllocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Payment => "payment",
            Self::Adjustment => "adjustment",
        }
    }
}
#[derive(Clone, Debug)]
pub struct NewOpenItem {
    pub amount_minor: i64,
    pub description: String,
    pub counterparty: Option<String>,
    pub due_date: Option<String>,
}
#[derive(Clone, Debug)]
pub struct NewAllocation {
    pub item_id: i64,
    pub amount_minor: i64,
    pub kind: AllocationKind,
}
#[derive(Debug, Clone)]
pub struct OpenItem {
    pub id: i64,
    pub source_line_id: i64,
    pub source_entry_id: i64,
    pub account_id: i64,
    pub kind: String,
    pub entry_date: String,
    pub amount_minor: i64,
    pub description: String,
    pub counterparty: Option<String>,
    pub due_date: Option<String>,
    pub paid_minor: i64,
    pub adjusted_minor: i64,
    pub outstanding_minor: i64,
    pub reversed: bool,
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Allocation {
    pub id: i64,
    pub settlement_line_id: i64,
    pub settlement_entry_id: i64,
    pub entry_date: String,
    pub item_id: i64,
    pub amount_minor: i64,
    pub kind: String,
    pub created_at: String,
    pub void_reason: Option<String>,
    pub voided_at: Option<String>,
}
#[derive(Debug, Error)]
pub enum OpenItemError {
    #[error("invalid open item input")]
    InvalidInput,
    #[error("posted source line not found")]
    LineNotFound,
    #[error("source line is reversed or is a reversal")]
    ReversedLine,
    #[error("wrong account or direction")]
    WrongAccount,
    #[error("source line already registered")]
    AlreadyRegistered,
    #[error("item split must equal the complete source line")]
    SplitMismatch,
    #[error("open item not found")]
    ItemNotFound,
    #[error("allocation exceeds available capacity")]
    Overallocated,
    #[error("settlement precedes source entry")]
    DateOrder,
    #[error("allocation not found or already voided")]
    AllocationNotActive,
    #[error("exact integer overflow")]
    Overflow,
    #[error("stored open item data is invalid")]
    InvalidStored,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
#[derive(Clone)]
pub struct OpenItemRepository {
    pool: SqlitePool,
}
#[derive(sqlx::FromRow)]
struct SourceLine {
    account_id: i64,
    side: String,
    amount_minor: i64,
    entry_date: String,
    element: String,
    status: String,
    reverses_entry_id: Option<i64>,
    reversed: bool,
}
#[derive(sqlx::FromRow)]
struct StoredItem {
    id: i64,
    source_line_id: i64,
    source_entry_id: i64,
    account_id: i64,
    kind: String,
    entry_date: String,
    amount_minor: i64,
    description: String,
    counterparty: Option<String>,
    due_date: Option<String>,
    reversed: bool,
}
const ITEMS: &str = "SELECT i.id,b.source_line_id,l.entry_id source_entry_id,l.account_id,b.kind,e.entry_date,i.amount_minor,i.description,i.counterparty,i.due_date,EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted') reversed FROM open_items i JOIN open_item_batches b ON b.id=i.batch_id JOIN journal_lines l ON l.id=b.source_line_id JOIN journal_entries e ON e.id=l.entry_id";
impl OpenItemRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn register(
        &self,
        line_id: i64,
        kind: TrackingKind,
        items: Vec<NewOpenItem>,
    ) -> Result<Vec<i64>, OpenItemError> {
        if line_id <= 0 || items.is_empty() || items.len() > 100 {
            return Err(OpenItemError::InvalidInput);
        }
        let mut total = 0i64;
        for i in &items {
            if i.amount_minor <= 0 {
                return Err(OpenItemError::InvalidInput);
            }
            text(&i.description, 1000)?;
            if let Some(c) = &i.counterparty {
                text(c, 200)?;
            }
            if let Some(d) = &i.due_date {
                JournalDate::parse(d).map_err(|_| OpenItemError::InvalidInput)?;
            }
            total = total
                .checked_add(i.amount_minor)
                .ok_or(OpenItemError::Overflow)?;
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let source = load_line(&mut tx, line_id).await?;
        if source.side != kind.source_side() || source.element != kind.element() {
            return Err(OpenItemError::WrongAccount);
        }
        if total != source.amount_minor {
            return Err(OpenItemError::SplitMismatch);
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM open_item_batches WHERE source_line_id=?)",
        )
        .bind(line_id)
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            return Err(OpenItemError::AlreadyRegistered);
        }
        let batch: i64 = sqlx::query_scalar(
            "INSERT INTO open_item_batches(source_line_id,kind) VALUES(?,?) RETURNING id",
        )
        .bind(line_id)
        .bind(kind.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let mut ids = Vec::new();
        for i in items {
            let id:i64=sqlx::query_scalar("INSERT INTO open_items(batch_id,amount_minor,description,counterparty,due_date) VALUES(?,?,?,?,?) RETURNING id").bind(batch).bind(i.amount_minor).bind(i.description.trim()).bind(i.counterparty.as_deref().map(str::trim)).bind(i.due_date).fetch_one(&mut *tx).await?;
            ids.push(id);
        }
        audit(&mut tx,"registered",batch,serde_json::json!({"source_line_id":line_id.to_string(),"kind":kind.as_str(),"item_ids":ids.iter().map(ToString::to_string).collect::<Vec<_>>(),"amount_minor":total.to_string()})).await?;
        tx.commit().await?;
        Ok(ids)
    }
    pub async fn allocate(
        &self,
        line_id: i64,
        allocations: Vec<NewAllocation>,
    ) -> Result<Vec<i64>, OpenItemError> {
        if line_id <= 0
            || allocations.is_empty()
            || allocations.len() > 100
            || allocations
                .iter()
                .any(|x| x.amount_minor <= 0 || x.item_id <= 0)
        {
            return Err(OpenItemError::InvalidInput);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let payment = load_line(&mut tx, line_id).await?;
        let used = used_by_line(&mut tx, line_id).await?;
        let mut remaining = payment
            .amount_minor
            .checked_sub(used)
            .filter(|x| *x >= 0)
            .ok_or(OpenItemError::InvalidStored)?;
        let mut ids = Vec::new();
        for a in allocations {
            let row: StoredItem = sqlx::QueryBuilder::<Sqlite>::new(ITEMS)
                .push(" WHERE i.id=")
                .push_bind(a.item_id)
                .build_query_as()
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(OpenItemError::ItemNotFound)?;
            let kind = stored_kind(&row.kind)?;
            let source = load_line(&mut tx, row.source_line_id).await?;
            if source.account_id != payment.account_id
                || payment.side == kind.source_side()
                || payment.element != kind.element()
            {
                return Err(OpenItemError::WrongAccount);
            }
            if payment.entry_date < source.entry_date {
                return Err(OpenItemError::DateOrder);
            }
            let (paid, adjusted) = used_by_item(&mut tx, row.id).await?;
            let outstanding = row
                .amount_minor
                .checked_sub(paid)
                .and_then(|n| n.checked_sub(adjusted))
                .filter(|x| *x >= 0)
                .ok_or(OpenItemError::InvalidStored)?;
            if a.amount_minor > outstanding || a.amount_minor > remaining {
                return Err(OpenItemError::Overallocated);
            }
            remaining -= a.amount_minor;
            let id:i64=sqlx::query_scalar("INSERT INTO open_item_allocations(settlement_line_id,item_id,amount_minor,kind) VALUES(?,?,?,?) RETURNING id").bind(line_id).bind(a.item_id).bind(a.amount_minor).bind(a.kind.as_str()).fetch_one(&mut *tx).await?;
            audit(&mut tx,"allocated",id,serde_json::json!({"settlement_line_id":line_id.to_string(),"item_id":a.item_id.to_string(),"amount_minor":a.amount_minor.to_string(),"kind":a.kind.as_str()})).await?;
            ids.push(id);
        }
        tx.commit().await?;
        Ok(ids)
    }
    /// Only cancels tracking. Financial payment and source entries stay posted.
    pub async fn void_allocation(&self, id: i64, reason: &str) -> Result<(), OpenItemError> {
        if id <= 0 {
            return Err(OpenItemError::InvalidInput);
        }
        text(reason, 1000)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM open_item_allocations a WHERE a.id=? AND NOT EXISTS(SELECT 1 FROM open_item_allocation_voids v WHERE v.allocation_id=a.id))").bind(id).fetch_one(&mut *tx).await?;
        if !active {
            return Err(OpenItemError::AllocationNotActive);
        }
        sqlx::query("INSERT INTO open_item_allocation_voids(allocation_id,reason) VALUES(?,?)")
            .bind(id)
            .bind(reason.trim())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            "allocation_voided",
            id,
            serde_json::json!({"reason":reason.trim()}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn list(&self) -> Result<Vec<OpenItem>, OpenItemError> {
        Ok(self.snapshot().await?.0)
    }
    /// Item amounts and allocation history come from the same SQLite snapshot.
    pub async fn snapshot(&self) -> Result<(Vec<OpenItem>, Vec<Allocation>), OpenItemError> {
        let mut tx = self.pool.begin().await?;
        let rows: Vec<StoredItem> = sqlx::QueryBuilder::<Sqlite>::new(ITEMS)
            .push(" ORDER BY e.entry_date,i.id")
            .build_query_as()
            .fetch_all(&mut *tx)
            .await?;
        let mut items = Vec::new();
        for r in rows {
            stored_kind(&r.kind)?;
            let (paid, adjusted) = used_by_item(&mut tx, r.id).await?;
            let outstanding = r
                .amount_minor
                .checked_sub(paid)
                .and_then(|n| n.checked_sub(adjusted))
                .filter(|x| *x >= 0)
                .ok_or(OpenItemError::InvalidStored)?;
            if r.reversed && (paid != 0 || adjusted != 0) {
                return Err(OpenItemError::InvalidStored);
            }
            items.push(OpenItem {
                id: r.id,
                source_line_id: r.source_line_id,
                source_entry_id: r.source_entry_id,
                account_id: r.account_id,
                kind: r.kind,
                entry_date: r.entry_date,
                amount_minor: r.amount_minor,
                description: r.description,
                counterparty: r.counterparty,
                due_date: r.due_date,
                paid_minor: paid,
                adjusted_minor: adjusted,
                outstanding_minor: if r.reversed { 0 } else { outstanding },
                reversed: r.reversed,
            });
        }
        let allocations=sqlx::query_as("SELECT a.id,a.settlement_line_id,l.entry_id settlement_entry_id,e.entry_date,a.item_id,a.amount_minor,a.kind,a.created_at,v.reason void_reason,v.created_at voided_at FROM open_item_allocations a JOIN journal_lines l ON l.id=a.settlement_line_id JOIN journal_entries e ON e.id=l.entry_id LEFT JOIN open_item_allocation_voids v ON v.allocation_id=a.id ORDER BY e.entry_date,a.id").fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok((items, allocations))
    }
    pub async fn allocations(&self) -> Result<Vec<Allocation>, OpenItemError> {
        Ok(sqlx::query_as("SELECT a.id,a.settlement_line_id,l.entry_id settlement_entry_id,e.entry_date,a.item_id,a.amount_minor,a.kind,a.created_at,v.reason void_reason,v.created_at voided_at FROM open_item_allocations a JOIN journal_lines l ON l.id=a.settlement_line_id JOIN journal_entries e ON e.id=l.entry_id LEFT JOIN open_item_allocation_voids v ON v.allocation_id=a.id ORDER BY e.entry_date,a.id").fetch_all(&self.pool).await?)
    }
    pub async fn line_capacity(&self, line_id: i64) -> Result<i64, OpenItemError> {
        let mut tx = self.pool.begin().await?;
        let line = load_line(&mut tx, line_id).await?;
        let used = used_by_line(&mut tx, line_id).await?;
        let n = line
            .amount_minor
            .checked_sub(used)
            .filter(|n| *n >= 0)
            .ok_or(OpenItemError::InvalidStored)?;
        tx.commit().await?;
        Ok(n)
    }
}
fn text(s: &str, max: usize) -> Result<(), OpenItemError> {
    if s.trim().is_empty() || s.chars().count() > max || s.contains('\0') {
        Err(OpenItemError::InvalidInput)
    } else {
        Ok(())
    }
}
fn stored_kind(s: &str) -> Result<TrackingKind, OpenItemError> {
    match s {
        "receivable" => Ok(TrackingKind::Receivable),
        "payable" => Ok(TrackingKind::Payable),
        _ => Err(OpenItemError::InvalidStored),
    }
}
async fn load_line(tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<SourceLine, OpenItemError> {
    let line:SourceLine=sqlx::query_as("SELECT l.entry_id,l.account_id,l.side,l.amount_minor,e.entry_date,a.element,e.status,e.reverses_entry_id,EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted') reversed FROM journal_lines l JOIN journal_entries e ON e.id=l.entry_id JOIN accounts a ON a.id=l.account_id WHERE l.id=?").bind(id).fetch_optional(&mut **tx).await?.ok_or(OpenItemError::LineNotFound)?;
    if line.status != "posted" {
        return Err(OpenItemError::LineNotFound);
    }
    if line.reversed || line.reverses_entry_id.is_some() {
        return Err(OpenItemError::ReversedLine);
    }
    if line.amount_minor <= 0
        || !matches!(line.side.as_str(), "debit" | "credit")
        || JournalDate::parse(&line.entry_date).is_err()
    {
        return Err(OpenItemError::InvalidStored);
    }
    Ok(line)
}
async fn used_by_item(
    tx: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> Result<(i64, i64), OpenItemError> {
    let rows:Vec<(i64,String)>=sqlx::query_as("SELECT a.amount_minor,a.kind FROM open_item_allocations a WHERE a.item_id=? AND NOT EXISTS(SELECT 1 FROM open_item_allocation_voids v WHERE v.allocation_id=a.id)").bind(id).fetch_all(&mut **tx).await?;
    let (mut paid, mut adjusted) = (0i64, 0i64);
    for (n, k) in rows {
        if n <= 0 {
            return Err(OpenItemError::InvalidStored);
        }
        let slot = match k.as_str() {
            "payment" => &mut paid,
            "adjustment" => &mut adjusted,
            _ => return Err(OpenItemError::InvalidStored),
        };
        *slot = slot.checked_add(n).ok_or(OpenItemError::Overflow)?;
    }
    Ok((paid, adjusted))
}
async fn used_by_line(tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<i64, OpenItemError> {
    let values:Vec<i64>=sqlx::query_scalar("SELECT a.amount_minor FROM open_item_allocations a WHERE a.settlement_line_id=? AND NOT EXISTS(SELECT 1 FROM open_item_allocation_voids v WHERE v.allocation_id=a.id)").bind(id).fetch_all(&mut **tx).await?;
    values.into_iter().try_fold(0i64, |sum, n| {
        if n > 0 {
            sum.checked_add(n).ok_or(OpenItemError::Overflow)
        } else {
            Err(OpenItemError::InvalidStored)
        }
    })
}
async fn audit(
    tx: &mut Transaction<'_, Sqlite>,
    kind: &str,
    id: i64,
    details: serde_json::Value,
) -> Result<(), OpenItemError> {
    sqlx::query("INSERT INTO open_item_events(kind,entity_id,details) VALUES(?,?,?)")
        .bind(kind)
        .bind(id)
        .bind(details.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
