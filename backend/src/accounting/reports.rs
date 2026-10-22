use std::collections::BTreeMap;

use sqlx::SqlitePool;
use thiserror::Error;

use super::accounts::{AccountId, AccountingElement};
use super::journal::{JournalDate, JournalEntryId, JournalLineId, JournalSide};
use super::money::{MinorUnits, MoneyError};

/// One account's activity and net position in an as-of trial balance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrialBalanceRow {
    pub account_id: AccountId,
    pub account_name: String,
    pub element: AccountingElement,
    pub debit_turnover: MinorUnits,
    pub credit_turnover: MinorUnits,
    pub debit_balance: MinorUnits,
    pub credit_balance: MinorUnits,
}

/// Exact trial balance derived only from posted journal lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrialBalance {
    pub as_of: JournalDate,
    pub rows: Vec<TrialBalanceRow>,
    pub total_debit: MinorUnits,
    pub total_credit: MinorUnits,
}

/// Net trial-balance columns grouped under one of the five accounting elements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElementBalanceRow {
    pub element: AccountingElement,
    pub debit_balance: MinorUnits,
    pub credit_balance: MinorUnits,
}

/// Policy-neutral five-element summary derived from a reconciled trial balance.
///
/// This is not labeled a balance sheet: it does not assume current/non-current
/// classification, closing entries, or retained-earnings policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountingEquationSummary {
    pub as_of: JournalDate,
    pub rows: Vec<ElementBalanceRow>,
    pub total_debit: MinorUnits,
    pub total_credit: MinorUnits,
}

/// One posted movement in a single-account general ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralLedgerLine {
    pub entry_id: JournalEntryId,
    pub line_id: JournalLineId,
    pub entry_date: JournalDate,
    pub line_no: i64,
    pub side: JournalSide,
    pub amount: MinorUnits,
    pub debit_balance: MinorUnits,
    pub credit_balance: MinorUnits,
}

/// Chronological posted-line ledger for one permanent account identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralLedger {
    pub account_id: AccountId,
    pub account_name: String,
    pub element: AccountingElement,
    pub through: JournalDate,
    pub lines: Vec<GeneralLedgerLine>,
    pub debit_turnover: MinorUnits,
    pub credit_turnover: MinorUnits,
    pub debit_balance: MinorUnits,
    pub credit_balance: MinorUnits,
}

/// Inclusive period ledger with an opening position carried from earlier
/// posted lines and running balances after every in-period movement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeriodGeneralLedger {
    pub account_id: AccountId,
    pub account_name: String,
    pub element: AccountingElement,
    pub from: JournalDate,
    pub through: JournalDate,
    pub opening_debit_balance: MinorUnits,
    pub opening_credit_balance: MinorUnits,
    pub lines: Vec<GeneralLedgerLine>,
    pub period_debit_turnover: MinorUnits,
    pub period_credit_turnover: MinorUnits,
    pub closing_debit_balance: MinorUnits,
    pub closing_credit_balance: MinorUnits,
}

/// One line in the posted general-journal read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralJournalLine {
    pub id: JournalLineId,
    pub line_no: i64,
    pub account_id: AccountId,
    pub account_name: String,
    pub side: JournalSide,
    pub amount: MinorUnits,
}

/// One balanced posted entry in chronological general-journal order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralJournalEntry {
    pub id: JournalEntryId,
    pub entry_date: JournalDate,
    pub posted_at: String,
    pub lines: Vec<GeneralJournalLine>,
    pub debit_total: MinorUnits,
    pub credit_total: MinorUnits,
}

/// Posted general journal for one inclusive accounting-date range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralJournal {
    pub from: JournalDate,
    pub through: JournalDate,
    pub entries: Vec<GeneralJournalEntry>,
    pub total_debit: MinorUnits,
    pub total_credit: MinorUnits,
}

/// One income or expense account's exact activity for a report period.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomeExpenseRow {
    pub account_id: AccountId,
    pub account_name: String,
    pub element: AccountingElement,
    pub debit_turnover: MinorUnits,
    pub credit_turnover: MinorUnits,
    /// Credit-minus-debit for income; debit-minus-credit for expenses.
    pub net_activity: MinorUnits,
}

/// Period activity for income and expense elements, without closing entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomeExpenseReport {
    pub from: JournalDate,
    pub through: JournalDate,
    pub rows: Vec<IncomeExpenseRow>,
    pub total_income: MinorUnits,
    pub total_expense: MinorUnits,
    pub net_result: MinorUnits,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("account {0} was not found for reporting")]
    AccountNotFound(i64),
    #[error("trial-balance arithmetic overflowed for account {0}")]
    AccountOverflow(i64),
    #[error("trial-balance total arithmetic overflowed")]
    TotalOverflow,
    #[error("five-element summary arithmetic overflowed for {0}")]
    ElementOverflow(AccountingElement),
    #[error("journal entry {0} arithmetic overflowed while reporting")]
    EntryOverflow(i64),
    #[error("invalid report date range: {from} is after {through}")]
    InvalidDateRange { from: String, through: String },
    #[error("stored journal line has invalid side {0}")]
    InvalidSide(String),
    #[error("stored account has invalid element {0}")]
    InvalidElement(String),
    #[error("stored journal entry has invalid date {0}")]
    InvalidStoredDate(String),
    #[error("stored general-journal rows are inconsistent: {0}")]
    InvalidStoredJournal(String),
    #[error("posted journal data does not reconcile: debit {debit}, credit {credit}")]
    DoesNotReconcile { debit: i64, credit: i64 },
    #[error("posted journal entry {entry_id} does not reconcile: debit {debit}, credit {credit}")]
    PostedEntryDoesNotReconcile {
        entry_id: i64,
        debit: i64,
        credit: i64,
    },
    #[error("report database operation failed: {0}")]
    Database(#[source] sqlx::Error),
}

#[derive(Clone)]
/// Read-only reporting boundary backed by posted journal lines.
pub struct ReportRepository {
    pool: SqlitePool,
}

impl ReportRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Builds a stable account-order trial balance through `as_of`, inclusive.
    ///
    /// Draft entries are excluded in SQL. Account balances are never read from
    /// or written to a mutable balance column.
    pub async fn trial_balance(&self, as_of: JournalDate) -> Result<TrialBalance, ReportError> {
        let lines: Vec<ReportLineRow> = sqlx::query_as(
            r#"
            SELECT account.id AS account_id,
                   account.name AS account_name,
                   account.element,
                   line.side,
                   line.amount_minor
            FROM journal_lines AS line
            JOIN journal_entries AS entry ON entry.id = line.entry_id
            JOIN accounts AS account ON account.id = line.account_id
            WHERE entry.status = 'posted'
              AND entry.entry_date <= ?
            ORDER BY account.id, line.id
            "#,
        )
        .bind(as_of.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(ReportError::Database)?;

        let mut accumulators: BTreeMap<AccountId, TrialBalanceAccumulator> = BTreeMap::new();
        for line in lines {
            let account_id = AccountId::new(line.account_id);
            let element = AccountingElement::try_from(line.element.as_str())
                .map_err(|_| ReportError::InvalidElement(line.element.clone()))?;
            let side = match line.side.as_str() {
                "debit" => JournalSide::Debit,
                "credit" => JournalSide::Credit,
                _ => return Err(ReportError::InvalidSide(line.side)),
            };
            let accumulator =
                accumulators
                    .entry(account_id)
                    .or_insert_with(|| TrialBalanceAccumulator {
                        account_name: line.account_name,
                        element,
                        debit: MinorUnits::ZERO,
                        credit: MinorUnits::ZERO,
                    });
            let total = match side {
                JournalSide::Debit => &mut accumulator.debit,
                JournalSide::Credit => &mut accumulator.credit,
            };
            *total = total
                .checked_add(MinorUnits::new(line.amount_minor))
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?;
        }

        let mut rows = Vec::with_capacity(accumulators.len());
        let mut total_debit = MinorUnits::ZERO;
        let mut total_credit = MinorUnits::ZERO;
        for (account_id, accumulator) in accumulators {
            let (debit_balance, credit_balance) =
                net_balance(accumulator.debit, accumulator.credit, account_id)?;
            total_debit = total_debit
                .checked_add(debit_balance)
                .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;
            total_credit = total_credit
                .checked_add(credit_balance)
                .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;
            rows.push(TrialBalanceRow {
                account_id,
                account_name: accumulator.account_name,
                element: accumulator.element,
                debit_turnover: accumulator.debit,
                credit_turnover: accumulator.credit,
                debit_balance,
                credit_balance,
            });
        }

        if total_debit != total_credit {
            return Err(ReportError::DoesNotReconcile {
                debit: total_debit.get(),
                credit: total_credit.get(),
            });
        }

        Ok(TrialBalance {
            as_of,
            rows,
            total_debit,
            total_credit,
        })
    }

    /// Groups a reconciled trial balance into the five accounting elements.
    pub async fn accounting_equation_summary(
        &self,
        as_of: JournalDate,
    ) -> Result<AccountingEquationSummary, ReportError> {
        let trial_balance = self.trial_balance(as_of.clone()).await?;
        let mut rows = vec![
            ElementBalanceRow {
                element: AccountingElement::Asset,
                debit_balance: MinorUnits::ZERO,
                credit_balance: MinorUnits::ZERO,
            },
            ElementBalanceRow {
                element: AccountingElement::Liability,
                debit_balance: MinorUnits::ZERO,
                credit_balance: MinorUnits::ZERO,
            },
            ElementBalanceRow {
                element: AccountingElement::Equity,
                debit_balance: MinorUnits::ZERO,
                credit_balance: MinorUnits::ZERO,
            },
            ElementBalanceRow {
                element: AccountingElement::Income,
                debit_balance: MinorUnits::ZERO,
                credit_balance: MinorUnits::ZERO,
            },
            ElementBalanceRow {
                element: AccountingElement::Expense,
                debit_balance: MinorUnits::ZERO,
                credit_balance: MinorUnits::ZERO,
            },
        ];

        for account in trial_balance.rows {
            let row = &mut rows[element_index(account.element)];
            row.debit_balance = row
                .debit_balance
                .checked_add(account.debit_balance)
                .map_err(|MoneyError::Overflow| ReportError::ElementOverflow(account.element))?;
            row.credit_balance = row
                .credit_balance
                .checked_add(account.credit_balance)
                .map_err(|MoneyError::Overflow| ReportError::ElementOverflow(account.element))?;
        }

        let total_debit = checked_element_total(&rows, |row| row.debit_balance)?;
        let total_credit = checked_element_total(&rows, |row| row.credit_balance)?;
        if total_debit != trial_balance.total_debit || total_credit != trial_balance.total_credit {
            return Err(ReportError::DoesNotReconcile {
                debit: total_debit.get(),
                credit: total_credit.get(),
            });
        }

        Ok(AccountingEquationSummary {
            as_of,
            rows,
            total_debit,
            total_credit,
        })
    }

    /// Builds one account's general ledger from posted lines through `through`.
    pub async fn general_ledger(
        &self,
        account_id: AccountId,
        through: JournalDate,
    ) -> Result<GeneralLedger, ReportError> {
        let mut transaction = self.pool.begin().await.map_err(ReportError::Database)?;
        let account: Option<(String, String)> =
            sqlx::query_as("SELECT name, element FROM accounts WHERE id = ?")
                .bind(account_id.get())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(ReportError::Database)?;
        let Some((account_name, stored_element)) = account else {
            return Err(ReportError::AccountNotFound(account_id.get()));
        };
        let element = AccountingElement::try_from(stored_element.as_str())
            .map_err(|_| ReportError::InvalidElement(stored_element))?;

        let stored_lines: Vec<LedgerLineRow> = sqlx::query_as(
            r#"
            SELECT entry.id AS entry_id,
                   line.id AS line_id,
                   entry.entry_date,
                   line.line_no,
                   line.side,
                   line.amount_minor
            FROM journal_lines AS line
            JOIN journal_entries AS entry ON entry.id = line.entry_id
            WHERE line.account_id = ?
              AND entry.status = 'posted'
              AND entry.entry_date <= ?
            ORDER BY entry.entry_date, entry.id, line.line_no, line.id
            "#,
        )
        .bind(account_id.get())
        .bind(through.as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(ReportError::Database)?;
        transaction.commit().await.map_err(ReportError::Database)?;

        let mut lines = Vec::with_capacity(stored_lines.len());
        let mut debit_turnover = MinorUnits::ZERO;
        let mut credit_turnover = MinorUnits::ZERO;
        for stored in stored_lines {
            let side = parse_side(stored.side)?;
            let amount = MinorUnits::new(stored.amount_minor);
            let total = match side {
                JournalSide::Debit => &mut debit_turnover,
                JournalSide::Credit => &mut credit_turnover,
            };
            *total = total
                .checked_add(amount)
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?;
            let (debit_balance, credit_balance) =
                net_balance(debit_turnover, credit_turnover, account_id)?;
            let entry_date = JournalDate::parse(&stored.entry_date)
                .map_err(|_| ReportError::InvalidStoredDate(stored.entry_date))?;
            lines.push(GeneralLedgerLine {
                entry_id: JournalEntryId::new(stored.entry_id),
                line_id: JournalLineId::new(stored.line_id),
                entry_date,
                line_no: stored.line_no,
                side,
                amount,
                debit_balance,
                credit_balance,
            });
        }
        let (debit_balance, credit_balance) =
            net_balance(debit_turnover, credit_turnover, account_id)?;

        Ok(GeneralLedger {
            account_id,
            account_name,
            element,
            through,
            lines,
            debit_turnover,
            credit_turnover,
            debit_balance,
            credit_balance,
        })
    }

    /// Builds an inclusive period ledger with a derived opening balance.
    pub async fn period_general_ledger(
        &self,
        account_id: AccountId,
        from: JournalDate,
        through: JournalDate,
    ) -> Result<PeriodGeneralLedger, ReportError> {
        validate_date_range(&from, &through)?;
        let mut transaction = self.pool.begin().await.map_err(ReportError::Database)?;
        let account: Option<(String, String)> =
            sqlx::query_as("SELECT name, element FROM accounts WHERE id = ?")
                .bind(account_id.get())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(ReportError::Database)?;
        let Some((account_name, stored_element)) = account else {
            return Err(ReportError::AccountNotFound(account_id.get()));
        };
        let element = AccountingElement::try_from(stored_element.as_str())
            .map_err(|_| ReportError::InvalidElement(stored_element))?;
        let stored_lines: Vec<LedgerLineRow> = sqlx::query_as(
            r#"
            SELECT entry.id AS entry_id,
                   line.id AS line_id,
                   entry.entry_date,
                   line.line_no,
                   line.side,
                   line.amount_minor
            FROM journal_lines AS line
            JOIN journal_entries AS entry ON entry.id = line.entry_id
            WHERE line.account_id = ?
              AND entry.status = 'posted'
              AND entry.entry_date <= ?
            ORDER BY entry.entry_date, entry.id, line.line_no, line.id
            "#,
        )
        .bind(account_id.get())
        .bind(through.as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(ReportError::Database)?;
        transaction.commit().await.map_err(ReportError::Database)?;

        let mut cumulative_debit = MinorUnits::ZERO;
        let mut cumulative_credit = MinorUnits::ZERO;
        let mut period_debit_turnover = MinorUnits::ZERO;
        let mut period_credit_turnover = MinorUnits::ZERO;
        let mut lines = Vec::new();
        let mut opening_turnover = (MinorUnits::ZERO, MinorUnits::ZERO);

        for stored in stored_lines {
            let entry_date = JournalDate::parse(&stored.entry_date)
                .map_err(|_| ReportError::InvalidStoredDate(stored.entry_date.clone()))?;
            let side = parse_side(stored.side)?;
            let amount = MinorUnits::new(stored.amount_minor);
            let cumulative = match side {
                JournalSide::Debit => &mut cumulative_debit,
                JournalSide::Credit => &mut cumulative_credit,
            };
            *cumulative = cumulative
                .checked_add(amount)
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?;

            if entry_date < from {
                opening_turnover = (cumulative_debit, cumulative_credit);
                continue;
            }

            let period_total = match side {
                JournalSide::Debit => &mut period_debit_turnover,
                JournalSide::Credit => &mut period_credit_turnover,
            };
            *period_total = period_total
                .checked_add(amount)
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?;
            let (debit_balance, credit_balance) =
                net_balance(cumulative_debit, cumulative_credit, account_id)?;
            lines.push(GeneralLedgerLine {
                entry_id: JournalEntryId::new(stored.entry_id),
                line_id: JournalLineId::new(stored.line_id),
                entry_date,
                line_no: stored.line_no,
                side,
                amount,
                debit_balance,
                credit_balance,
            });
        }

        let (opening_debit_balance, opening_credit_balance) =
            net_balance(opening_turnover.0, opening_turnover.1, account_id)?;
        let (closing_debit_balance, closing_credit_balance) =
            net_balance(cumulative_debit, cumulative_credit, account_id)?;
        Ok(PeriodGeneralLedger {
            account_id,
            account_name,
            element,
            from,
            through,
            opening_debit_balance,
            opening_credit_balance,
            lines,
            period_debit_turnover,
            period_credit_turnover,
            closing_debit_balance,
            closing_credit_balance,
        })
    }

    /// Builds a chronological posted general journal for an inclusive date range.
    pub async fn general_journal(
        &self,
        from: JournalDate,
        through: JournalDate,
    ) -> Result<GeneralJournal, ReportError> {
        validate_date_range(&from, &through)?;

        let stored_lines: Vec<GeneralJournalLineRow> = sqlx::query_as(
            r#"
            SELECT entry.id AS entry_id,
                   entry.entry_date,
                   entry.posted_at,
                   line.id AS line_id,
                   line.line_no,
                   account.id AS account_id,
                   account.name AS account_name,
                   line.side,
                   line.amount_minor
            FROM journal_entries AS entry
            JOIN journal_lines AS line ON line.entry_id = entry.id
            JOIN accounts AS account ON account.id = line.account_id
            WHERE entry.status = 'posted'
              AND entry.entry_date >= ?
              AND entry.entry_date <= ?
            ORDER BY entry.entry_date, entry.id, line.line_no, line.id
            "#,
        )
        .bind(from.as_str())
        .bind(through.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(ReportError::Database)?;

        let mut entries = Vec::new();
        let mut current: Option<GeneralJournalAccumulator> = None;
        for stored in stored_lines {
            if current
                .as_ref()
                .is_some_and(|entry| entry.id.get() != stored.entry_id)
            {
                let previous = current.take().ok_or_else(|| {
                    ReportError::InvalidStoredJournal(
                        "entry group disappeared while changing identifiers".to_owned(),
                    )
                })?;
                entries.push(finish_general_journal_entry(previous)?);
            }

            if current.is_none() {
                let entry_date = JournalDate::parse(&stored.entry_date)
                    .map_err(|_| ReportError::InvalidStoredDate(stored.entry_date.clone()))?;
                current = Some(GeneralJournalAccumulator {
                    id: JournalEntryId::new(stored.entry_id),
                    entry_date,
                    posted_at: stored.posted_at.clone(),
                    lines: Vec::new(),
                    debit_total: MinorUnits::ZERO,
                    credit_total: MinorUnits::ZERO,
                });
            }

            let entry = current.as_mut().ok_or_else(|| {
                ReportError::InvalidStoredJournal(
                    "line could not be assigned to an entry group".to_owned(),
                )
            })?;
            let side = parse_side(stored.side)?;
            let amount = MinorUnits::new(stored.amount_minor);
            let total = match side {
                JournalSide::Debit => &mut entry.debit_total,
                JournalSide::Credit => &mut entry.credit_total,
            };
            *total = total
                .checked_add(amount)
                .map_err(|MoneyError::Overflow| ReportError::EntryOverflow(entry.id.get()))?;
            entry.lines.push(GeneralJournalLine {
                id: JournalLineId::new(stored.line_id),
                line_no: stored.line_no,
                account_id: AccountId::new(stored.account_id),
                account_name: stored.account_name,
                side,
                amount,
            });
        }
        if let Some(entry) = current {
            entries.push(finish_general_journal_entry(entry)?);
        }

        let mut total_debit = MinorUnits::ZERO;
        let mut total_credit = MinorUnits::ZERO;
        for entry in &entries {
            total_debit = total_debit
                .checked_add(entry.debit_total)
                .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;
            total_credit = total_credit
                .checked_add(entry.credit_total)
                .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;
        }
        if total_debit != total_credit {
            return Err(ReportError::DoesNotReconcile {
                debit: total_debit.get(),
                credit: total_credit.get(),
            });
        }

        Ok(GeneralJournal {
            from,
            through,
            entries,
            total_debit,
            total_credit,
        })
    }

    /// Derives income and expense activity for one inclusive date range.
    ///
    /// This read model does not create closing entries or move the result into
    /// equity. Those state transitions remain separate accounting-policy work.
    pub async fn income_expense_report(
        &self,
        from: JournalDate,
        through: JournalDate,
    ) -> Result<IncomeExpenseReport, ReportError> {
        validate_date_range(&from, &through)?;

        let lines: Vec<ReportLineRow> = sqlx::query_as(
            r#"
            SELECT account.id AS account_id,
                   account.name AS account_name,
                   account.element,
                   line.side,
                   line.amount_minor
            FROM journal_lines AS line
            JOIN journal_entries AS entry ON entry.id = line.entry_id
            JOIN accounts AS account ON account.id = line.account_id
            WHERE entry.status = 'posted'
              AND entry.entry_date >= ?
              AND entry.entry_date <= ?
              AND account.element IN ('income', 'expense')
            ORDER BY account.id, line.id
            "#,
        )
        .bind(from.as_str())
        .bind(through.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(ReportError::Database)?;

        let mut accumulators: BTreeMap<AccountId, TrialBalanceAccumulator> = BTreeMap::new();
        for line in lines {
            let account_id = AccountId::new(line.account_id);
            let element = AccountingElement::try_from(line.element.as_str())
                .map_err(|_| ReportError::InvalidElement(line.element.clone()))?;
            if !matches!(
                element,
                AccountingElement::Income | AccountingElement::Expense
            ) {
                return Err(ReportError::InvalidElement(line.element));
            }
            let side = parse_side(line.side)?;
            let accumulator =
                accumulators
                    .entry(account_id)
                    .or_insert_with(|| TrialBalanceAccumulator {
                        account_name: line.account_name,
                        element,
                        debit: MinorUnits::ZERO,
                        credit: MinorUnits::ZERO,
                    });
            let total = match side {
                JournalSide::Debit => &mut accumulator.debit,
                JournalSide::Credit => &mut accumulator.credit,
            };
            *total = total
                .checked_add(MinorUnits::new(line.amount_minor))
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?;
        }

        let mut rows = Vec::with_capacity(accumulators.len());
        let mut total_income = MinorUnits::ZERO;
        let mut total_expense = MinorUnits::ZERO;
        for (account_id, accumulator) in accumulators {
            let (net_activity, report_total) = match accumulator.element {
                AccountingElement::Income => (
                    accumulator.credit.checked_sub(accumulator.debit).map_err(
                        |MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()),
                    )?,
                    &mut total_income,
                ),
                AccountingElement::Expense => (
                    accumulator.debit.checked_sub(accumulator.credit).map_err(
                        |MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()),
                    )?,
                    &mut total_expense,
                ),
                _ => {
                    return Err(ReportError::InvalidElement(
                        accumulator.element.as_str().to_owned(),
                    ));
                }
            };
            *report_total = report_total
                .checked_add(net_activity)
                .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;
            rows.push(IncomeExpenseRow {
                account_id,
                account_name: accumulator.account_name,
                element: accumulator.element,
                debit_turnover: accumulator.debit,
                credit_turnover: accumulator.credit,
                net_activity,
            });
        }

        let net_result = total_income
            .checked_sub(total_expense)
            .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)?;

        Ok(IncomeExpenseReport {
            from,
            through,
            rows,
            total_income,
            total_expense,
            net_result,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ReportLineRow {
    account_id: i64,
    account_name: String,
    element: String,
    side: String,
    amount_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct LedgerLineRow {
    entry_id: i64,
    line_id: i64,
    entry_date: String,
    line_no: i64,
    side: String,
    amount_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct GeneralJournalLineRow {
    entry_id: i64,
    entry_date: String,
    posted_at: String,
    line_id: i64,
    line_no: i64,
    account_id: i64,
    account_name: String,
    side: String,
    amount_minor: i64,
}

struct TrialBalanceAccumulator {
    account_name: String,
    element: AccountingElement,
    debit: MinorUnits,
    credit: MinorUnits,
}

struct GeneralJournalAccumulator {
    id: JournalEntryId,
    entry_date: JournalDate,
    posted_at: String,
    lines: Vec<GeneralJournalLine>,
    debit_total: MinorUnits,
    credit_total: MinorUnits,
}

fn parse_side(side: String) -> Result<JournalSide, ReportError> {
    match side.as_str() {
        "debit" => Ok(JournalSide::Debit),
        "credit" => Ok(JournalSide::Credit),
        _ => Err(ReportError::InvalidSide(side)),
    }
}

fn net_balance(
    debit: MinorUnits,
    credit: MinorUnits,
    account_id: AccountId,
) -> Result<(MinorUnits, MinorUnits), ReportError> {
    if debit >= credit {
        Ok((
            debit
                .checked_sub(credit)
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?,
            MinorUnits::ZERO,
        ))
    } else {
        Ok((
            MinorUnits::ZERO,
            credit
                .checked_sub(debit)
                .map_err(|MoneyError::Overflow| ReportError::AccountOverflow(account_id.get()))?,
        ))
    }
}

fn finish_general_journal_entry(
    entry: GeneralJournalAccumulator,
) -> Result<GeneralJournalEntry, ReportError> {
    if entry.debit_total != entry.credit_total {
        return Err(ReportError::PostedEntryDoesNotReconcile {
            entry_id: entry.id.get(),
            debit: entry.debit_total.get(),
            credit: entry.credit_total.get(),
        });
    }

    Ok(GeneralJournalEntry {
        id: entry.id,
        entry_date: entry.entry_date,
        posted_at: entry.posted_at,
        lines: entry.lines,
        debit_total: entry.debit_total,
        credit_total: entry.credit_total,
    })
}

fn validate_date_range(from: &JournalDate, through: &JournalDate) -> Result<(), ReportError> {
    if from > through {
        Err(ReportError::InvalidDateRange {
            from: from.as_str().to_owned(),
            through: through.as_str().to_owned(),
        })
    } else {
        Ok(())
    }
}

const fn element_index(element: AccountingElement) -> usize {
    match element {
        AccountingElement::Asset => 0,
        AccountingElement::Liability => 1,
        AccountingElement::Equity => 2,
        AccountingElement::Income => 3,
        AccountingElement::Expense => 4,
    }
}

fn checked_element_total(
    rows: &[ElementBalanceRow],
    value: impl Fn(&ElementBalanceRow) -> MinorUnits,
) -> Result<MinorUnits, ReportError> {
    rows.iter().try_fold(MinorUnits::ZERO, |total, row| {
        total
            .checked_add(value(row))
            .map_err(|MoneyError::Overflow| ReportError::TotalOverflow)
    })
}
