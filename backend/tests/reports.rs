use std::time::{SystemTime, UNIX_EPOCH};

use kaccounting_backend::accounting::accounts::{
    Account, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::accounting::reports::{ReportError, ReportRepository};
use kaccounting_backend::db::{open_database, open_memory_database};
use sqlx::SqlitePool;

async fn posting_account(
    accounts: &AccountRepository,
    name: &str,
    element: AccountingElement,
) -> Account {
    accounts
        .create(NewAccount::new(None, name, element, AccountKind::Posting))
        .await
        .expect("test posting account should be created")
}

fn line(account: &Account, side: JournalSide, amount: i64) -> JournalLineInput {
    JournalLineInput::new(account.id, side, MinorUnits::new(amount))
}

async fn create_balanced_draft(
    pool: &SqlitePool,
    date: &str,
    debit: &Account,
    credit: &Account,
    amount: i64,
) {
    let entry_id: i64 =
        sqlx::query_scalar("INSERT INTO journal_entries (entry_date) VALUES (?) RETURNING id")
            .bind(date)
            .fetch_one(pool)
            .await
            .expect("draft should be created");
    for (line_no, account_id, side) in
        [(1, debit.id.get(), "debit"), (2, credit.id.get(), "credit")]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .bind(amount)
        .execute(pool)
        .await
        .expect("balanced draft line should be created");
    }
}

fn assert_invalid_amount(error: ReportError, line_id: i64, amount: i64) {
    assert!(
        matches!(
            error,
            ReportError::InvalidAmount {
                line_id: actual_line_id,
                amount: actual_amount,
            } if actual_line_id == line_id && actual_amount == amount
        ),
        "expected invalid amount {amount} for line {line_id}, got {error:?}"
    );
}

fn assert_invalid_amount_from(error: ReportError, line_ids: &[i64], amount: i64) {
    assert!(
        matches!(
            error,
            ReportError::InvalidAmount {
                line_id: actual_line_id,
                amount: actual_amount,
            } if line_ids.contains(&actual_line_id) && actual_amount == amount
        ),
        "expected invalid amount {amount} for one of lines {line_ids:?}, got {error:?}"
    );
}

fn assert_invalid_date_range(error: ReportError, from: &str, through: &str) {
    assert!(
        matches!(
            error,
            ReportError::InvalidDateRange {
                from: ref actual_from,
                through: ref actual_through,
            } if actual_from == from && actual_through == through
        ),
        "expected invalid range {from} through {through}, got {error:?}"
    );
}

fn unique_database_path(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "kAccounting-Extra-01-reports-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

fn remove_sqlite_files(database_path: &std::path::Path) {
    for path in [
        database_path.to_path_buf(),
        database_path.with_extension("sqlite3-wal"),
        database_path.with_extension("sqlite3-shm"),
        database_path.with_extension("sqlite3-journal"),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .unwrap_or_else(|error| panic!("failed to remove {}: {error}", path.display()));
        }
    }
}

#[tokio::test]
async fn trial_balance_uses_only_posted_lines_through_the_inclusive_date() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let expense = posting_account(&accounts, "交通費", AccountingElement::Expense).await;

    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&capital, JournalSide::Credit, 1_000),
            ],
        ))
        .await
        .expect("opening entry should post");
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-01").unwrap(),
            vec![
                line(&expense, JournalSide::Debit, 300),
                line(&cash, JournalSide::Credit, 300),
            ],
        ))
        .await
        .expect("expense entry should post");

    let january = reports
        .trial_balance(JournalDate::parse("2026-01-31").unwrap())
        .await
        .expect("January balance should reconcile");
    assert_eq!(january.rows.len(), 2);
    assert_eq!(january.total_debit, MinorUnits::new(1_000));
    assert_eq!(january.total_credit, MinorUnits::new(1_000));

    let february = reports
        .trial_balance(JournalDate::parse("2026-02-01").unwrap())
        .await
        .expect("inclusive February balance should reconcile");
    assert_eq!(february.rows.len(), 3);
    assert_eq!(
        february
            .rows
            .iter()
            .map(|row| row.account_id)
            .collect::<Vec<_>>(),
        vec![cash.id, capital.id, expense.id],
        "trial-balance rows must retain stable account identifier order"
    );
    assert_eq!(february.total_debit, MinorUnits::new(1_000));
    assert_eq!(february.total_credit, MinorUnits::new(1_000));

    let cash_row = february
        .rows
        .iter()
        .find(|row| row.account_id == cash.id)
        .expect("cash row should be present");
    assert_eq!(cash_row.debit_turnover, MinorUnits::new(1_000));
    assert_eq!(cash_row.credit_turnover, MinorUnits::new(300));
    assert_eq!(cash_row.debit_balance, MinorUnits::new(700));
    assert_eq!(cash_row.credit_balance, MinorUnits::ZERO);

    let expense_row = february
        .rows
        .iter()
        .find(|row| row.account_id == expense.id)
        .expect("expense row should be present");
    assert_eq!(expense_row.debit_balance, MinorUnits::new(300));
    let capital_row = february
        .rows
        .iter()
        .find(|row| row.account_id == capital.id)
        .expect("capital row should be present");
    assert_eq!(capital_row.credit_balance, MinorUnits::new(1_000));

    let journal_report = reports
        .general_journal(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-02-01").unwrap(),
        )
        .await
        .expect("the same posted lines should form a general journal");
    let trial_debit_turnover: i64 = february
        .rows
        .iter()
        .map(|row| row.debit_turnover.get())
        .sum();
    let trial_credit_turnover: i64 = february
        .rows
        .iter()
        .map(|row| row.credit_turnover.get())
        .sum();
    assert_eq!(journal_report.total_debit.get(), trial_debit_turnover);
    assert_eq!(journal_report.total_credit.get(), trial_credit_turnover);

    let cash_ledger = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-02-01").unwrap(),
        )
        .await
        .expect("the same posted cash lines should form a general ledger");
    assert_eq!(cash_ledger.closing_debit_balance, cash_row.debit_balance);
    assert_eq!(cash_ledger.closing_credit_balance, cash_row.credit_balance);
}

#[tokio::test]
async fn five_element_summary_reconciles_without_assuming_closing_policy() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let payable = posting_account(&accounts, "應付", AccountingElement::Liability).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    for (date, lines) in [
        (
            "2026-01-01",
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&capital, JournalSide::Credit, 1_000),
            ],
        ),
        (
            "2026-01-02",
            vec![
                line(&expense, JournalSide::Debit, 300),
                line(&cash, JournalSide::Credit, 300),
            ],
        ),
        (
            "2026-01-03",
            vec![
                line(&cash, JournalSide::Debit, 500),
                line(&revenue, JournalSide::Credit, 500),
            ],
        ),
        (
            "2026-01-04",
            vec![
                line(&payable, JournalSide::Debit, 200),
                line(&cash, JournalSide::Credit, 200),
            ],
        ),
    ] {
        journal
            .post(PostEntry::new(JournalDate::parse(date).unwrap(), lines))
            .await
            .expect("summary fixture should post");
    }

    let summary = reports
        .accounting_equation_summary(JournalDate::parse("2026-01-04").unwrap())
        .await
        .expect("five-element summary should reconcile");

    assert_eq!(summary.rows.len(), 5);
    assert_eq!(summary.rows[0].element, AccountingElement::Asset);
    assert_eq!(summary.rows[0].debit_balance, MinorUnits::new(1_000));
    assert_eq!(summary.rows[1].element, AccountingElement::Liability);
    assert_eq!(summary.rows[1].debit_balance, MinorUnits::new(200));
    assert_eq!(summary.rows[2].element, AccountingElement::Equity);
    assert_eq!(summary.rows[2].credit_balance, MinorUnits::new(1_000));
    assert_eq!(summary.rows[3].element, AccountingElement::Income);
    assert_eq!(summary.rows[3].credit_balance, MinorUnits::new(500));
    assert_eq!(summary.rows[4].element, AccountingElement::Expense);
    assert_eq!(summary.rows[4].debit_balance, MinorUnits::new(300));
    assert_eq!(summary.total_debit, MinorUnits::new(1_500));
    assert_eq!(summary.total_credit, MinorUnits::new(1_500));
}

#[tokio::test]
async fn empty_and_single_element_reports_remain_structurally_complete() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let as_of = JournalDate::parse("2026-01-01").unwrap();

    let empty_trial = reports
        .trial_balance(as_of.clone())
        .await
        .expect("empty trial balance should reconcile");
    assert!(empty_trial.rows.is_empty());
    assert_eq!(empty_trial.total_debit, MinorUnits::ZERO);
    assert_eq!(empty_trial.total_credit, MinorUnits::ZERO);

    let empty_summary = reports
        .accounting_equation_summary(as_of.clone())
        .await
        .expect("empty five-element summary should be complete");
    assert_eq!(empty_summary.rows.len(), 5);
    assert!(empty_summary.rows.iter().all(|row| {
        row.debit_balance == MinorUnits::ZERO && row.credit_balance == MinorUnits::ZERO
    }));

    let empty_journal = reports
        .general_journal(as_of.clone(), as_of.clone())
        .await
        .expect("empty general journal should reconcile");
    assert!(empty_journal.entries.is_empty());
    assert_eq!(empty_journal.total_debit, MinorUnits::ZERO);
    assert_eq!(empty_journal.total_credit, MinorUnits::ZERO);

    let empty_income_expense = reports
        .income_expense_report(as_of.clone(), as_of.clone())
        .await
        .expect("empty income/expense report should calculate");
    assert!(empty_income_expense.rows.is_empty());
    assert_eq!(empty_income_expense.net_result, MinorUnits::ZERO);

    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let bank = posting_account(&accounts, "銀行", AccountingElement::Asset).await;
    journal
        .post(PostEntry::new(
            as_of.clone(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&bank, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("asset transfer should post");

    let summary = reports
        .accounting_equation_summary(as_of.clone())
        .await
        .expect("single-element summary should reconcile");
    assert_eq!(summary.rows[0].element, AccountingElement::Asset);
    assert_eq!(summary.rows[0].debit_balance, MinorUnits::new(100));
    assert_eq!(summary.rows[0].credit_balance, MinorUnits::new(100));
    assert!(summary.rows[1..].iter().all(|row| {
        row.debit_balance == MinorUnits::ZERO && row.credit_balance == MinorUnits::ZERO
    }));

    let income_expense = reports
        .income_expense_report(as_of.clone(), as_of)
        .await
        .expect("asset-only activity should not become income or expense");
    assert!(income_expense.rows.is_empty());
    assert_eq!(income_expense.net_result, MinorUnits::ZERO);
}

#[tokio::test]
async fn trial_balance_ignores_balanced_draft_lines() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    create_balanced_draft(&pool, "2026-01-01", &cash, &capital, 500).await;

    let report = reports
        .trial_balance(JournalDate::parse("2026-12-31").unwrap())
        .await
        .expect("empty posted report should reconcile");

    assert!(report.rows.is_empty());
    assert_eq!(report.total_debit, MinorUnits::ZERO);
    assert_eq!(report.total_credit, MinorUnits::ZERO);
}

#[tokio::test]
async fn trial_balance_follows_permanent_ids_after_rename_and_deactivation() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 500),
                line(&capital, JournalSide::Credit, 500),
            ],
        ))
        .await
        .expect("entry should post");
    let renamed = accounts
        .rename(cash.id, cash.revision, "手邊現金")
        .await
        .expect("rename should succeed");
    accounts
        .set_active(renamed.id, renamed.revision, false)
        .await
        .expect("deactivation should succeed");

    let report = reports
        .trial_balance(JournalDate::parse("2026-01-01").unwrap())
        .await
        .expect("historical report should remain available");
    let row = report
        .rows
        .iter()
        .find(|row| row.account_id == cash.id)
        .expect("renamed account should retain its permanent identity");

    assert_eq!(row.account_name, "手邊現金");
    assert_eq!(row.debit_balance, MinorUnits::new(500));

    let ledger = reports
        .general_ledger(cash.id, JournalDate::parse("2026-01-01").unwrap())
        .await
        .expect("deactivated account history should remain in its ledger");
    assert_eq!(ledger.account_name, "手邊現金");
    assert_eq!(ledger.lines.len(), 1);
    assert_eq!(ledger.debit_balance, MinorUnits::new(500));

    let general_journal = reports
        .general_journal(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-01").unwrap(),
        )
        .await
        .expect("deactivated accounts should remain in journal history");
    let cash_line = general_journal.entries[0]
        .lines
        .iter()
        .find(|line| line.account_id == cash.id)
        .expect("historical cash line should remain available");
    assert_eq!(cash_line.account_name, "手邊現金");
}

#[tokio::test]
async fn trial_balance_reports_cross_entry_integer_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    for date in ["2026-01-01", "2026-01-02"] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(&cash, JournalSide::Debit, i64::MAX),
                    line(&capital, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each boundary-sized entry is independently valid");
    }

    let error = reports
        .trial_balance(JournalDate::parse("2026-01-02").unwrap())
        .await
        .expect_err("cross-entry sum must not wrap");

    assert!(matches!(error, ReportError::AccountOverflow(id) if id == cash.id.get()));
}

#[tokio::test]
async fn trial_balance_reports_cross_account_total_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let first_cash = posting_account(&accounts, "現金一", AccountingElement::Asset).await;
    let second_cash = posting_account(&accounts, "現金二", AccountingElement::Asset).await;
    let first_capital = posting_account(&accounts, "資本一", AccountingElement::Equity).await;
    let second_capital = posting_account(&accounts, "資本二", AccountingElement::Equity).await;

    for (date, debit, credit) in [
        ("2026-01-01", &first_cash, &first_capital),
        ("2026-01-02", &second_cash, &second_capital),
    ] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(debit, JournalSide::Debit, i64::MAX),
                    line(credit, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each account balance should fit independently");
    }

    let error = reports
        .trial_balance(JournalDate::parse("2026-01-02").unwrap())
        .await
        .expect_err("cross-account total must not wrap");

    assert!(matches!(error, ReportError::TotalOverflow));
}

#[tokio::test]
async fn general_ledger_orders_posted_lines_by_accounting_date_and_tracks_balance() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let expense = posting_account(&accounts, "交通費", AccountingElement::Expense).await;

    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-01").unwrap(),
            vec![
                line(&expense, JournalSide::Debit, 300),
                line(&cash, JournalSide::Credit, 300),
            ],
        ))
        .await
        .expect("later entry should post first in storage");
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&capital, JournalSide::Credit, 1_000),
            ],
        ))
        .await
        .expect("earlier entry should post later in storage");

    let ledger = reports
        .general_ledger(cash.id, JournalDate::parse("2026-02-01").unwrap())
        .await
        .expect("cash ledger should be derived from posted lines");

    assert_eq!(ledger.account_id, cash.id);
    assert_eq!(ledger.account_name, "現金");
    assert_eq!(ledger.element, AccountingElement::Asset);
    assert_eq!(ledger.lines.len(), 2);
    assert_eq!(ledger.lines[0].entry_date.as_str(), "2026-01-01");
    assert_eq!(ledger.lines[0].side, JournalSide::Debit);
    assert_eq!(ledger.lines[0].debit_balance, MinorUnits::new(1_000));
    assert_eq!(ledger.lines[1].entry_date.as_str(), "2026-02-01");
    assert_eq!(ledger.lines[1].side, JournalSide::Credit);
    assert_eq!(ledger.lines[1].debit_balance, MinorUnits::new(700));
    assert_eq!(ledger.debit_turnover, MinorUnits::new(1_000));
    assert_eq!(ledger.credit_turnover, MinorUnits::new(300));
    assert_eq!(ledger.debit_balance, MinorUnits::new(700));
    assert_eq!(ledger.credit_balance, MinorUnits::ZERO);

    let january = reports
        .general_ledger(cash.id, JournalDate::parse("2026-01-31").unwrap())
        .await
        .expect("as-of filter should be inclusive and bounded");
    assert_eq!(january.lines.len(), 1);
    assert_eq!(january.debit_balance, MinorUnits::new(1_000));
}

#[tokio::test]
async fn period_ledger_carries_opening_balance_and_limits_turnover_to_the_range() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let revenue = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    for (date, lines) in [
        (
            "2026-01-01",
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&capital, JournalSide::Credit, 1_000),
            ],
        ),
        (
            "2026-01-05",
            vec![
                line(&expense, JournalSide::Debit, 300),
                line(&cash, JournalSide::Credit, 300),
            ],
        ),
        (
            "2026-01-31",
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&revenue, JournalSide::Credit, 100),
            ],
        ),
        (
            "2026-02-01",
            vec![
                line(&cash, JournalSide::Debit, 500),
                line(&revenue, JournalSide::Credit, 500),
            ],
        ),
    ] {
        journal
            .post(PostEntry::new(JournalDate::parse(date).unwrap(), lines))
            .await
            .expect("period-ledger fixture should post");
    }

    let ledger = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2026-01-05").unwrap(),
            JournalDate::parse("2026-01-31").unwrap(),
        )
        .await
        .expect("period ledger should calculate");

    assert_eq!(ledger.opening_debit_balance, MinorUnits::new(1_000));
    assert_eq!(ledger.opening_credit_balance, MinorUnits::ZERO);
    assert_eq!(ledger.lines.len(), 2);
    assert_eq!(ledger.lines[0].entry_date.as_str(), "2026-01-05");
    assert_eq!(ledger.lines[0].debit_balance, MinorUnits::new(700));
    assert_eq!(ledger.lines[1].entry_date.as_str(), "2026-01-31");
    assert_eq!(ledger.lines[1].debit_balance, MinorUnits::new(800));
    assert_eq!(ledger.period_debit_turnover, MinorUnits::new(100));
    assert_eq!(ledger.period_credit_turnover, MinorUnits::new(300));
    assert_eq!(ledger.closing_debit_balance, MinorUnits::new(800));
    assert_eq!(ledger.closing_credit_balance, MinorUnits::ZERO);
}

#[tokio::test]
async fn period_ledger_flips_a_debit_opening_position_to_a_credit_closing_position() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&capital, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("opening entry should post");
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-05").unwrap(),
            vec![
                line(&expense, JournalSide::Debit, 250),
                line(&cash, JournalSide::Credit, 250),
            ],
        ))
        .await
        .expect("overdraft entry should post");

    let ledger = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2026-01-05").unwrap(),
            JournalDate::parse("2026-01-05").unwrap(),
        )
        .await
        .expect("balance direction should flip exactly");

    assert_eq!(ledger.opening_debit_balance, MinorUnits::new(100));
    assert_eq!(ledger.opening_credit_balance, MinorUnits::ZERO);
    assert_eq!(ledger.period_debit_turnover, MinorUnits::ZERO);
    assert_eq!(ledger.period_credit_turnover, MinorUnits::new(250));
    assert_eq!(ledger.lines.len(), 1);
    assert_eq!(ledger.lines[0].debit_balance, MinorUnits::ZERO);
    assert_eq!(ledger.lines[0].credit_balance, MinorUnits::new(150));
    assert_eq!(ledger.closing_debit_balance, MinorUnits::ZERO);
    assert_eq!(ledger.closing_credit_balance, MinorUnits::new(150));
}

#[tokio::test]
async fn period_ledger_racing_atomic_posts_observes_only_complete_snapshots() {
    let database_path = unique_database_path("period-ledger-post-race");
    let pool = open_database(&database_path)
        .await
        .expect("multi-connection test database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&capital, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("opening entry should post");

    let day = JournalDate::parse("2026-01-02").unwrap();
    for posted_count in 0_i64..32 {
        let report_future = reports.period_general_ledger(cash.id, day.clone(), day.clone());
        let post_future = journal.post(PostEntry::new(
            day.clone(),
            vec![
                line(&expense, JournalSide::Debit, 1),
                line(&cash, JournalSide::Credit, 1),
            ],
        ));
        let (report_result, post_result) = tokio::join!(report_future, post_future);
        let report = report_result.expect("racing ledger read should remain coherent");
        post_result.expect("racing entry should post atomically");

        let observed_count = report.period_credit_turnover.get();
        assert!(
            observed_count == posted_count || observed_count == posted_count + 1,
            "snapshot must contain either the complete pre-post or post-post state"
        );
        assert_eq!(report.lines.len() as i64, observed_count);
        assert_eq!(
            report.closing_debit_balance,
            MinorUnits::new(100 - observed_count)
        );
        assert_eq!(report.closing_credit_balance, MinorUnits::ZERO);
    }

    let final_ledger = reports
        .period_general_ledger(cash.id, day.clone(), day)
        .await
        .expect("final ledger should contain every completed posting");
    assert_eq!(final_ledger.lines.len(), 32);
    assert_eq!(final_ledger.period_credit_turnover, MinorUnits::new(32));
    assert_eq!(final_ledger.closing_debit_balance, MinorUnits::new(68));

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn general_ledger_returns_an_empty_existing_account() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;

    let empty = reports
        .general_ledger(cash.id, JournalDate::parse("2026-12-31").unwrap())
        .await
        .expect("existing account without history should have an empty ledger");
    assert!(empty.lines.is_empty());
    assert_eq!(empty.debit_balance, MinorUnits::ZERO);
    assert_eq!(empty.credit_balance, MinorUnits::ZERO);
}

#[tokio::test]
async fn general_ledger_reports_cross_entry_integer_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    for date in ["2026-01-01", "2026-01-02"] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(&cash, JournalSide::Debit, i64::MAX),
                    line(&capital, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each entry should post independently");
    }

    let error = reports
        .general_ledger(cash.id, JournalDate::parse("2026-01-02").unwrap())
        .await
        .expect_err("ledger accumulation must not wrap");
    assert!(matches!(error, ReportError::AccountOverflow(id) if id == cash.id.get()));

    let error = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-02").unwrap(),
        )
        .await
        .expect_err("period-ledger accumulation must not wrap");
    assert!(matches!(error, ReportError::AccountOverflow(id) if id == cash.id.get()));
}

#[tokio::test]
async fn general_journal_orders_entries_and_lines_and_excludes_drafts() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let expense = posting_account(&accounts, "交通費", AccountingElement::Expense).await;

    let february = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-01").unwrap(),
            vec![
                line(&expense, JournalSide::Debit, 300),
                line(&cash, JournalSide::Credit, 300),
            ],
        ))
        .await
        .expect("later entry should post first in storage");
    let january = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&expense, JournalSide::Debit, 200),
                line(&capital, JournalSide::Credit, 1_200),
            ],
        ))
        .await
        .expect("compound entry should post");

    create_balanced_draft(&pool, "2026-01-15", &cash, &capital, 50).await;

    let report = reports
        .general_journal(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-02-01").unwrap(),
        )
        .await
        .expect("general journal should reconcile");

    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.entries[0].id, january.id);
    assert_eq!(report.entries[0].entry_date.as_str(), "2026-01-01");
    assert_eq!(report.entries[0].lines.len(), 3);
    assert_eq!(report.entries[0].lines[0].line_no, 1);
    assert_eq!(report.entries[0].lines[1].line_no, 2);
    assert_eq!(report.entries[0].lines[2].line_no, 3);
    assert_eq!(report.entries[0].debit_total, MinorUnits::new(1_200));
    assert_eq!(report.entries[0].credit_total, MinorUnits::new(1_200));
    assert_eq!(report.entries[1].id, february.id);
    assert_eq!(report.total_debit, MinorUnits::new(1_500));
    assert_eq!(report.total_credit, MinorUnits::new(1_500));

    let january_only = reports
        .general_journal(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-31").unwrap(),
        )
        .await
        .expect("inclusive range should exclude February");
    assert_eq!(january_only.entries.len(), 1);
    assert_eq!(january_only.entries[0].id, january.id);
}

#[tokio::test]
async fn report_methods_use_consistent_range_and_account_error_variants() {
    let pool = open_memory_database().await.expect("database should open");
    let reports = ReportRepository::new(pool);
    let from = JournalDate::parse("2026-02-01").unwrap();
    let through = JournalDate::parse("2026-01-01").unwrap();
    let missing = kaccounting_backend::accounting::accounts::AccountId::new(999_999);

    let period_error = reports
        .period_general_ledger(missing, from.clone(), through.clone())
        .await
        .expect_err("period ledger should classify an inverted range first");
    let journal_error = reports
        .general_journal(from.clone(), through.clone())
        .await
        .expect_err("general journal should reject the same inverted range");
    let income_error = reports
        .income_expense_report(from, through)
        .await
        .expect_err("income/expense report should reject the same inverted range");
    for error in [period_error, journal_error, income_error] {
        assert_invalid_date_range(error, "2026-02-01", "2026-01-01");
    }

    let ledger_error = reports
        .general_ledger(missing, JournalDate::parse("2026-01-01").unwrap())
        .await
        .expect_err("general ledger should classify a missing account");
    let period_error = reports
        .period_general_ledger(
            missing,
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-01").unwrap(),
        )
        .await
        .expect_err("period ledger should classify the same missing account");
    assert!(matches!(
        ledger_error,
        ReportError::AccountNotFound(999_999)
    ));
    assert!(matches!(
        period_error,
        ReportError::AccountNotFound(999_999)
    ));
}

#[tokio::test]
async fn every_report_fails_closed_on_an_invalid_stored_calendar_date() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let income = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-28").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&income, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("valid fixture entry should post");

    let invalid_date = "2026-02-29";
    sqlx::query("DROP TRIGGER journal_entries_posted_immutable_update")
        .execute(&pool)
        .await
        .expect("disposable test database should allow date corruption injection");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test should temporarily bypass calendar checks");
    sqlx::query("UPDATE journal_entries SET entry_date = ? WHERE id = ?")
        .bind(invalid_date)
        .bind(posted.id.get())
        .execute(&pool)
        .await
        .expect("test should corrupt the stored calendar date");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .expect("test should restore calendar checks");

    let from = JournalDate::parse("2026-01-01").unwrap();
    let through = JournalDate::parse("2026-12-31").unwrap();
    let errors = [
        reports
            .trial_balance(through.clone())
            .await
            .expect_err("trial balance must reject an invalid stored date"),
        reports
            .accounting_equation_summary(through.clone())
            .await
            .expect_err("five-element summary must reject an invalid stored date"),
        reports
            .general_ledger(cash.id, through.clone())
            .await
            .expect_err("general ledger must reject an invalid stored date"),
        reports
            .period_general_ledger(cash.id, from.clone(), through.clone())
            .await
            .expect_err("period ledger must reject an invalid stored date"),
        reports
            .general_journal(from.clone(), through.clone())
            .await
            .expect_err("general journal must reject an invalid stored date"),
        reports
            .income_expense_report(from, through)
            .await
            .expect_err("income/expense report must reject an invalid stored date"),
    ];

    for error in errors {
        assert!(
            matches!(error, ReportError::InvalidStoredDate(ref date) if date == invalid_date),
            "expected invalid stored date {invalid_date}, got {error:?}"
        );
    }
}

#[tokio::test]
async fn reports_include_the_minimum_and_maximum_supported_calendar_dates() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let minimum = JournalDate::parse("0001-01-01").unwrap();
    let maximum = JournalDate::parse("9999-12-31").unwrap();

    let first = journal
        .post(PostEntry::new(
            minimum.clone(),
            vec![
                line(&cash, JournalSide::Debit, 1),
                line(&capital, JournalSide::Credit, 1),
            ],
        ))
        .await
        .expect("minimum-date entry should post");
    let last = journal
        .post(PostEntry::new(
            maximum.clone(),
            vec![
                line(&cash, JournalSide::Debit, 2),
                line(&capital, JournalSide::Credit, 2),
            ],
        ))
        .await
        .expect("maximum-date entry should post");

    let minimum_trial = reports
        .trial_balance(minimum.clone())
        .await
        .expect("minimum as-of date should be inclusive");
    assert_eq!(minimum_trial.total_debit, MinorUnits::new(1));
    assert_eq!(minimum_trial.total_credit, MinorUnits::new(1));

    let maximum_trial = reports
        .trial_balance(maximum.clone())
        .await
        .expect("maximum as-of date should be inclusive");
    assert_eq!(maximum_trial.total_debit, MinorUnits::new(3));
    assert_eq!(maximum_trial.total_credit, MinorUnits::new(3));

    let minimum_journal = reports
        .general_journal(minimum.clone(), minimum.clone())
        .await
        .expect("minimum single-day range should be inclusive");
    assert_eq!(minimum_journal.entries.len(), 1);
    assert_eq!(minimum_journal.entries[0].id, first.id);

    let maximum_journal = reports
        .general_journal(maximum.clone(), maximum.clone())
        .await
        .expect("maximum single-day range should be inclusive");
    assert_eq!(maximum_journal.entries.len(), 1);
    assert_eq!(maximum_journal.entries[0].id, last.id);

    let maximum_ledger = reports
        .period_general_ledger(cash.id, maximum.clone(), maximum)
        .await
        .expect("maximum-date period ledger should carry all earlier history");
    assert_eq!(maximum_ledger.opening_debit_balance, MinorUnits::new(1));
    assert_eq!(maximum_ledger.period_debit_turnover, MinorUnits::new(2));
    assert_eq!(maximum_ledger.closing_debit_balance, MinorUnits::new(3));
}

#[tokio::test]
async fn reports_include_cross_year_and_leap_day_period_boundaries() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let income = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    for (date, lines) in [
        (
            "2023-12-30",
            vec![
                line(&cash, JournalSide::Debit, 500),
                line(&capital, JournalSide::Credit, 500),
            ],
        ),
        (
            "2023-12-31",
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&income, JournalSide::Credit, 100),
            ],
        ),
        (
            "2024-02-29",
            vec![
                line(&expense, JournalSide::Debit, 40),
                line(&cash, JournalSide::Credit, 40),
            ],
        ),
        (
            "2024-03-01",
            vec![
                line(&cash, JournalSide::Debit, 25),
                line(&income, JournalSide::Credit, 25),
            ],
        ),
    ] {
        journal
            .post(PostEntry::new(JournalDate::parse(date).unwrap(), lines))
            .await
            .expect("calendar-boundary fixture should post");
    }

    let from = JournalDate::parse("2023-12-31").unwrap();
    let through = JournalDate::parse("2024-02-29").unwrap();
    let general_journal = reports
        .general_journal(from.clone(), through.clone())
        .await
        .expect("cross-year journal range should be inclusive");
    assert_eq!(general_journal.entries.len(), 2);
    assert_eq!(general_journal.entries[0].entry_date, from);
    assert_eq!(general_journal.entries[1].entry_date, through);
    assert_eq!(general_journal.total_debit, MinorUnits::new(140));

    let cash_ledger = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2023-12-31").unwrap(),
            JournalDate::parse("2024-02-29").unwrap(),
        )
        .await
        .expect("cross-year ledger should carry the prior opening balance");
    assert_eq!(cash_ledger.opening_debit_balance, MinorUnits::new(500));
    assert_eq!(cash_ledger.period_debit_turnover, MinorUnits::new(100));
    assert_eq!(cash_ledger.period_credit_turnover, MinorUnits::new(40));
    assert_eq!(cash_ledger.closing_debit_balance, MinorUnits::new(560));

    let income_expense = reports
        .income_expense_report(
            JournalDate::parse("2023-12-31").unwrap(),
            JournalDate::parse("2024-02-29").unwrap(),
        )
        .await
        .expect("cross-year income/expense period should calculate");
    assert_eq!(income_expense.total_income, MinorUnits::new(100));
    assert_eq!(income_expense.total_expense, MinorUnits::new(40));
    assert_eq!(income_expense.net_result, MinorUnits::new(60));
}

#[tokio::test]
async fn reports_handle_many_posted_entries_with_exact_totals_and_stable_order() {
    const ENTRY_COUNT: i64 = 512;

    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;

    sqlx::query(
        r#"
        WITH RECURSIVE sequence(value) AS (
            VALUES (1)
            UNION ALL
            SELECT value + 1 FROM sequence WHERE value < ?
        )
        INSERT INTO journal_entries (entry_date)
        SELECT printf('2026-01-%02d', ((value - 1) % 28) + 1)
        FROM sequence
        "#,
    )
    .bind(ENTRY_COUNT)
    .execute(&pool)
    .await
    .expect("many draft headers should be created together");
    for (line_no, account_id, side) in
        [(1, cash.id.get(), "debit"), (2, capital.id.get(), "credit")]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            SELECT id, ?, ?, ?, 1
            FROM journal_entries
            "#,
        )
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .execute(&pool)
        .await
        .expect("one valid line should be created for every draft");
    }
    sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-02-01T00:00:00.000Z',
            revision = revision + 1
        "#,
    )
    .execute(&pool)
    .await
    .expect("all complete drafts should post under database invariants");

    let through = JournalDate::parse("2026-01-28").unwrap();
    let trial = reports
        .trial_balance(through.clone())
        .await
        .expect("large trial balance should remain exact");
    assert_eq!(trial.total_debit, MinorUnits::new(ENTRY_COUNT));
    assert_eq!(trial.total_credit, MinorUnits::new(ENTRY_COUNT));

    let journal = reports
        .general_journal(JournalDate::parse("2026-01-01").unwrap(), through.clone())
        .await
        .expect("large general journal should remain exact");
    assert_eq!(journal.entries.len(), ENTRY_COUNT as usize);
    assert_eq!(journal.total_debit, MinorUnits::new(ENTRY_COUNT));
    assert!(journal.entries.windows(2).all(|pair| {
        pair[0].entry_date < pair[1].entry_date
            || (pair[0].entry_date == pair[1].entry_date && pair[0].id < pair[1].id)
    }));

    let ledger = reports
        .general_ledger(cash.id, through)
        .await
        .expect("large account ledger should remain exact");
    assert_eq!(ledger.lines.len(), ENTRY_COUNT as usize);
    assert_eq!(ledger.debit_turnover, MinorUnits::new(ENTRY_COUNT));
    assert_eq!(ledger.debit_balance, MinorUnits::new(ENTRY_COUNT));
}

#[tokio::test]
async fn general_journal_reports_cross_entry_total_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    for date in ["2026-01-01", "2026-01-02"] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(&cash, JournalSide::Debit, i64::MAX),
                    line(&capital, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each entry should post independently");
    }

    let error = reports
        .general_journal(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-02").unwrap(),
        )
        .await
        .expect_err("cross-entry journal total must not wrap");

    assert!(matches!(error, ReportError::TotalOverflow));
}

#[tokio::test]
async fn general_journal_reports_single_entry_side_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;

    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-01-01') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test draft should be created");
    for (line_no, account_id, side) in [
        (1, cash.id.get(), "debit"),
        (2, cash.id.get(), "debit"),
        (3, capital.id.get(), "credit"),
        (4, capital.id.get(), "credit"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .bind(i64::MAX)
        .execute(&pool)
        .await
        .expect("boundary-sized test line should be inserted");
    }

    sqlx::query("DROP TRIGGER journal_entries_validate_posting")
        .execute(&pool)
        .await
        .expect("disposable test database should allow overflow injection");
    sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-01-01T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect("test should inject a posted entry whose side totals overflow");

    let day = JournalDate::parse("2026-01-01").unwrap();
    let error = reports
        .general_journal(day.clone(), day)
        .await
        .expect_err("one entry's side total must not wrap");

    assert!(matches!(error, ReportError::EntryOverflow(id) if id == entry_id));
}

#[tokio::test]
async fn income_expense_report_uses_period_posted_activity_and_opposite_side_adjustments() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let revenue = posting_account(&accounts, "服務收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "交通費", AccountingElement::Expense).await;

    for (date, lines) in [
        (
            "2026-01-01",
            vec![
                line(&cash, JournalSide::Debit, 2_000),
                line(&revenue, JournalSide::Credit, 2_000),
            ],
        ),
        (
            "2026-01-02",
            vec![
                line(&revenue, JournalSide::Debit, 200),
                line(&cash, JournalSide::Credit, 200),
            ],
        ),
        (
            "2026-01-03",
            vec![
                line(&expense, JournalSide::Debit, 500),
                line(&cash, JournalSide::Credit, 500),
            ],
        ),
        (
            "2026-01-04",
            vec![
                line(&cash, JournalSide::Debit, 50),
                line(&expense, JournalSide::Credit, 50),
            ],
        ),
    ] {
        journal
            .post(PostEntry::new(JournalDate::parse(date).unwrap(), lines))
            .await
            .expect("period entry should post");
    }

    create_balanced_draft(&pool, "2026-01-05", &cash, &revenue, 999).await;

    let report = reports
        .income_expense_report(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-31").unwrap(),
        )
        .await
        .expect("period activity should calculate exactly");

    assert_eq!(report.rows.len(), 2, "asset accounts must be excluded");
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| row.account_id)
            .collect::<Vec<_>>(),
        vec![revenue.id, expense.id],
        "income/expense rows must retain stable account identifier order"
    );
    assert_eq!(report.total_income, MinorUnits::new(1_800));
    assert_eq!(report.total_expense, MinorUnits::new(450));
    assert_eq!(report.net_result, MinorUnits::new(1_350));
    let revenue_row = report
        .rows
        .iter()
        .find(|row| row.account_id == revenue.id)
        .expect("income row should be present");
    assert_eq!(revenue_row.debit_turnover, MinorUnits::new(200));
    assert_eq!(revenue_row.credit_turnover, MinorUnits::new(2_000));
    assert_eq!(revenue_row.net_activity, MinorUnits::new(1_800));
    let expense_row = report
        .rows
        .iter()
        .find(|row| row.account_id == expense.id)
        .expect("expense row should be present");
    assert_eq!(expense_row.debit_turnover, MinorUnits::new(500));
    assert_eq!(expense_row.credit_turnover, MinorUnits::new(50));
    assert_eq!(expense_row.net_activity, MinorUnits::new(450));

    let later_period = reports
        .income_expense_report(
            JournalDate::parse("2026-01-03").unwrap(),
            JournalDate::parse("2026-01-04").unwrap(),
        )
        .await
        .expect("inclusive subperiod should calculate");
    assert_eq!(later_period.total_income, MinorUnits::ZERO);
    assert_eq!(later_period.total_expense, MinorUnits::new(450));
    assert_eq!(later_period.net_result, MinorUnits::new(-450));
}

#[tokio::test]
async fn income_expense_report_rejects_total_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());

    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let first_income = posting_account(&accounts, "收入一", AccountingElement::Income).await;
    let second_income = posting_account(&accounts, "收入二", AccountingElement::Income).await;
    for (date, income) in [
        ("2026-01-01", &first_income),
        ("2026-01-02", &second_income),
    ] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(&cash, JournalSide::Debit, i64::MAX),
                    line(income, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each entry should post independently");
    }

    let overflow = reports
        .income_expense_report(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-02").unwrap(),
        )
        .await
        .expect_err("report total must not wrap");
    assert!(matches!(overflow, ReportError::TotalOverflow));
}

#[tokio::test]
async fn income_expense_report_rejects_net_result_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let first_cash = posting_account(&accounts, "現金一", AccountingElement::Asset).await;
    let second_cash = posting_account(&accounts, "現金二", AccountingElement::Asset).await;
    let income = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    for (date, debit, credit) in [
        ("2026-01-01", &first_cash, &income),
        ("2026-01-02", &second_cash, &expense),
    ] {
        journal
            .post(PostEntry::new(
                JournalDate::parse(date).unwrap(),
                vec![
                    line(debit, JournalSide::Debit, i64::MAX),
                    line(credit, JournalSide::Credit, i64::MAX),
                ],
            ))
            .await
            .expect("each boundary-sized entry should post independently");
    }

    let error = reports
        .income_expense_report(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-02").unwrap(),
        )
        .await
        .expect_err("income minus a negative expense total must not wrap");

    assert!(matches!(error, ReportError::TotalOverflow));
}

#[tokio::test]
async fn posted_period_reconciles_across_journal_ledgers_trial_and_income_expense_reports() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let income = posting_account(&accounts, "收入", AccountingElement::Income).await;
    let expense = posting_account(&accounts, "費用", AccountingElement::Expense).await;

    for (date, lines) in [
        (
            "2025-12-31",
            vec![
                line(&cash, JournalSide::Debit, 1_000),
                line(&capital, JournalSide::Credit, 1_000),
            ],
        ),
        (
            "2026-01-05",
            vec![
                line(&cash, JournalSide::Debit, 400),
                line(&income, JournalSide::Credit, 400),
            ],
        ),
        (
            "2026-01-10",
            vec![
                line(&expense, JournalSide::Debit, 150),
                line(&cash, JournalSide::Credit, 150),
            ],
        ),
        (
            "2026-01-15",
            vec![
                line(&income, JournalSide::Debit, 50),
                line(&cash, JournalSide::Credit, 50),
            ],
        ),
        (
            "2026-01-20",
            vec![
                line(&cash, JournalSide::Debit, 20),
                line(&expense, JournalSide::Credit, 20),
            ],
        ),
    ] {
        journal
            .post(PostEntry::new(JournalDate::parse(date).unwrap(), lines))
            .await
            .expect("cross-report fixture should post");
    }

    create_balanced_draft(&pool, "2026-01-25", &cash, &income, 999).await;

    let from = JournalDate::parse("2026-01-01").unwrap();
    let through = JournalDate::parse("2026-01-31").unwrap();
    let general_journal = reports
        .general_journal(from.clone(), through.clone())
        .await
        .expect("period journal should reconcile");
    assert_eq!(general_journal.entries.len(), 4, "draft must be excluded");
    assert_eq!(general_journal.total_debit, MinorUnits::new(620));
    assert_eq!(general_journal.total_credit, MinorUnits::new(620));

    let cash_ledger = reports
        .period_general_ledger(cash.id, from.clone(), through.clone())
        .await
        .expect("cash period ledger should calculate");
    let capital_ledger = reports
        .period_general_ledger(capital.id, from.clone(), through.clone())
        .await
        .expect("capital period ledger should calculate");
    let income_ledger = reports
        .period_general_ledger(income.id, from.clone(), through.clone())
        .await
        .expect("income period ledger should calculate");
    let expense_ledger = reports
        .period_general_ledger(expense.id, from.clone(), through.clone())
        .await
        .expect("expense period ledger should calculate");

    let ledger_debit_total = [
        cash_ledger.period_debit_turnover,
        capital_ledger.period_debit_turnover,
        income_ledger.period_debit_turnover,
        expense_ledger.period_debit_turnover,
    ]
    .into_iter()
    .try_fold(MinorUnits::ZERO, MinorUnits::checked_add)
    .expect("fixture debit turnover should remain exact");
    let ledger_credit_total = [
        cash_ledger.period_credit_turnover,
        capital_ledger.period_credit_turnover,
        income_ledger.period_credit_turnover,
        expense_ledger.period_credit_turnover,
    ]
    .into_iter()
    .try_fold(MinorUnits::ZERO, MinorUnits::checked_add)
    .expect("fixture credit turnover should remain exact");
    assert_eq!(ledger_debit_total, general_journal.total_debit);
    assert_eq!(ledger_credit_total, general_journal.total_credit);
    assert_eq!(cash_ledger.opening_debit_balance, MinorUnits::new(1_000));
    assert_eq!(
        capital_ledger.opening_credit_balance,
        MinorUnits::new(1_000)
    );
    assert_eq!(cash_ledger.closing_debit_balance, MinorUnits::new(1_220));
    assert_eq!(income_ledger.closing_credit_balance, MinorUnits::new(350));
    assert_eq!(expense_ledger.closing_debit_balance, MinorUnits::new(130));

    let trial = reports
        .trial_balance(through.clone())
        .await
        .expect("trial balance should reconcile");
    let cash_trial = trial
        .rows
        .iter()
        .find(|row| row.account_id == cash.id)
        .expect("cash should appear in trial balance");
    let income_trial = trial
        .rows
        .iter()
        .find(|row| row.account_id == income.id)
        .expect("income should appear in trial balance");
    let expense_trial = trial
        .rows
        .iter()
        .find(|row| row.account_id == expense.id)
        .expect("expense should appear in trial balance");
    assert_eq!(cash_trial.debit_balance, cash_ledger.closing_debit_balance);
    assert_eq!(
        income_trial.credit_balance,
        income_ledger.closing_credit_balance
    );
    assert_eq!(
        expense_trial.debit_balance,
        expense_ledger.closing_debit_balance
    );
    assert_eq!(trial.total_debit, MinorUnits::new(1_350));
    assert_eq!(trial.total_credit, MinorUnits::new(1_350));

    let summary = reports
        .accounting_equation_summary(through.clone())
        .await
        .expect("five-element summary should reconcile");
    assert_eq!(summary.total_debit, trial.total_debit);
    assert_eq!(summary.total_credit, trial.total_credit);

    let income_expense = reports
        .income_expense_report(from, through)
        .await
        .expect("income/expense report should calculate");
    let income_row = income_expense
        .rows
        .iter()
        .find(|row| row.account_id == income.id)
        .expect("income should appear in period report");
    let expense_row = income_expense
        .rows
        .iter()
        .find(|row| row.account_id == expense.id)
        .expect("expense should appear in period report");
    assert_eq!(
        income_row.debit_turnover,
        income_ledger.period_debit_turnover
    );
    assert_eq!(
        income_row.credit_turnover,
        income_ledger.period_credit_turnover
    );
    assert_eq!(
        expense_row.debit_turnover,
        expense_ledger.period_debit_turnover
    );
    assert_eq!(
        expense_row.credit_turnover,
        expense_ledger.period_credit_turnover
    );
    assert_eq!(income_expense.total_income, MinorUnits::new(350));
    assert_eq!(income_expense.total_expense, MinorUnits::new(130));
    assert_eq!(income_expense.net_result, MinorUnits::new(220));
}

#[tokio::test]
async fn equal_and_opposite_posted_entry_nets_reports_without_mutating_original() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool);
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    let income = posting_account(&accounts, "收入", AccountingElement::Income).await;

    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-31").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 500),
                line(&capital, JournalSide::Credit, 500),
            ],
        ))
        .await
        .expect("opening entry should post");
    let original = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 200),
                line(&income, JournalSide::Credit, 200),
            ],
        ))
        .await
        .expect("original entry should post");
    let opposite = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-01").unwrap(),
            vec![
                line(&income, JournalSide::Debit, 200),
                line(&cash, JournalSide::Credit, 200),
            ],
        ))
        .await
        .expect("equal and opposite entry should post independently");
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-02-02").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 50),
                line(&income, JournalSide::Credit, 50),
            ],
        ))
        .await
        .expect("later entry should post");

    let day = JournalDate::parse("2026-02-01").unwrap();
    let general_journal = reports
        .general_journal(day.clone(), day.clone())
        .await
        .expect("single-day journal should include both same-day entries");
    assert_eq!(
        general_journal
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![original.id, opposite.id],
        "same-day entries must retain stable identifier order"
    );
    assert_eq!(general_journal.total_debit, MinorUnits::new(400));
    assert_eq!(general_journal.total_credit, MinorUnits::new(400));

    let cash_ledger = reports
        .period_general_ledger(cash.id, day.clone(), day.clone())
        .await
        .expect("cash period ledger should calculate");
    assert_eq!(cash_ledger.opening_debit_balance, MinorUnits::new(500));
    assert_eq!(cash_ledger.lines.len(), 2);
    assert_eq!(cash_ledger.lines[0].entry_id, original.id);
    assert_eq!(cash_ledger.lines[1].entry_id, opposite.id);
    assert_eq!(cash_ledger.period_debit_turnover, MinorUnits::new(200));
    assert_eq!(cash_ledger.period_credit_turnover, MinorUnits::new(200));
    assert_eq!(cash_ledger.closing_debit_balance, MinorUnits::new(500));

    let income_expense = reports
        .income_expense_report(day.clone(), day.clone())
        .await
        .expect("single-day income activity should calculate");
    let income_row = income_expense
        .rows
        .iter()
        .find(|row| row.account_id == income.id)
        .expect("zero-net income activity should retain its audit row");
    assert_eq!(income_row.debit_turnover, MinorUnits::new(200));
    assert_eq!(income_row.credit_turnover, MinorUnits::new(200));
    assert_eq!(income_row.net_activity, MinorUnits::ZERO);
    assert_eq!(income_expense.total_income, MinorUnits::ZERO);
    assert_eq!(income_expense.net_result, MinorUnits::ZERO);

    let trial = reports
        .trial_balance(day.clone())
        .await
        .expect("same-day trial balance should reconcile");
    let cash_row = trial
        .rows
        .iter()
        .find(|row| row.account_id == cash.id)
        .expect("cash should remain in the trial balance");
    let income_row = trial
        .rows
        .iter()
        .find(|row| row.account_id == income.id)
        .expect("zero-net income turnover should remain auditable");
    assert_eq!(cash_row.debit_balance, MinorUnits::new(500));
    assert_eq!(income_row.debit_turnover, MinorUnits::new(200));
    assert_eq!(income_row.credit_turnover, MinorUnits::new(200));
    assert_eq!(income_row.debit_balance, MinorUnits::ZERO);
    assert_eq!(income_row.credit_balance, MinorUnits::ZERO);
    assert_eq!(trial.total_debit, MinorUnits::new(500));
    assert_eq!(trial.total_credit, MinorUnits::new(500));
}

#[tokio::test]
async fn reports_reject_non_positive_stored_journal_amounts() {
    for invalid_amount in [0, -1, i64::MIN] {
        let pool = open_memory_database().await.expect("database should open");
        let accounts = AccountRepository::new(pool.clone());
        let reports = ReportRepository::new(pool.clone());
        let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
        let income = posting_account(&accounts, "收入", AccountingElement::Income).await;

        let entry_id: i64 = sqlx::query_scalar(
            "INSERT INTO journal_entries (entry_date) VALUES ('2026-01-01') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("test draft should be created");
        sqlx::query("PRAGMA ignore_check_constraints = ON")
            .execute(&pool)
            .await
            .expect("test should temporarily bypass amount checks");
        let cash_line_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, 1, ?, 'debit', ?)
            RETURNING id
            "#,
        )
        .bind(entry_id)
        .bind(cash.id.get())
        .bind(invalid_amount)
        .fetch_one(&pool)
        .await
        .expect("test should inject a non-positive debit");
        let income_line_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, 2, ?, 'credit', ?)
            RETURNING id
            "#,
        )
        .bind(entry_id)
        .bind(income.id.get())
        .bind(invalid_amount)
        .fetch_one(&pool)
        .await
        .expect("test should inject a non-positive credit");
        sqlx::query("PRAGMA ignore_check_constraints = OFF")
            .execute(&pool)
            .await
            .expect("test should restore amount checks");
        sqlx::query(
            r#"
            UPDATE journal_entries
            SET status = 'posted',
                posted_at = '2026-01-01T00:00:00.000Z',
                revision = revision + 1
            WHERE id = ?
            "#,
        )
        .bind(entry_id)
        .execute(&pool)
        .await
        .expect("test should inject a balanced but invalid posted entry");

        let as_of = JournalDate::parse("2026-01-01").unwrap();
        let error = reports
            .trial_balance(as_of.clone())
            .await
            .expect_err("trial balance must reject a non-positive stored amount");
        assert_invalid_amount_from(error, &[cash_line_id, income_line_id], invalid_amount);

        let error = reports
            .accounting_equation_summary(as_of.clone())
            .await
            .expect_err("five-element summary must reject a non-positive stored amount");
        assert_invalid_amount_from(error, &[cash_line_id, income_line_id], invalid_amount);

        let error = reports
            .general_ledger(cash.id, as_of.clone())
            .await
            .expect_err("general ledger must reject a non-positive stored amount");
        assert_invalid_amount(error, cash_line_id, invalid_amount);

        let error = reports
            .period_general_ledger(cash.id, as_of.clone(), as_of.clone())
            .await
            .expect_err("period ledger must reject a non-positive stored amount");
        assert_invalid_amount(error, cash_line_id, invalid_amount);

        let error = reports
            .general_journal(as_of.clone(), as_of.clone())
            .await
            .expect_err("general journal must reject a non-positive stored amount");
        assert_invalid_amount(error, cash_line_id, invalid_amount);

        let error = reports
            .income_expense_report(as_of.clone(), as_of)
            .await
            .expect_err("income/expense report must reject a non-positive stored amount");
        assert_invalid_amount(error, income_line_id, invalid_amount);
    }
}

#[tokio::test]
async fn balance_reports_reject_an_invalid_referenced_account_element() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());
    let cash = posting_account(&accounts, "現金", AccountingElement::Asset).await;
    let capital = posting_account(&accounts, "資本", AccountingElement::Equity).await;
    journal
        .post(PostEntry::new(
            JournalDate::parse("2026-01-01").unwrap(),
            vec![
                line(&cash, JournalSide::Debit, 100),
                line(&capital, JournalSide::Credit, 100),
            ],
        ))
        .await
        .expect("fixture entry should post");

    sqlx::query("DROP TRIGGER accounts_preserve_journal_classification")
        .execute(&pool)
        .await
        .expect("disposable test database should allow corruption injection");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test should temporarily bypass element checks");
    sqlx::query("UPDATE accounts SET element = 'invalid', revision = revision + 1 WHERE id = ?")
        .bind(cash.id.get())
        .execute(&pool)
        .await
        .expect("test should corrupt the referenced account element");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .expect("test should restore element checks");

    let as_of = JournalDate::parse("2026-01-01").unwrap();
    let trial_error = reports
        .trial_balance(as_of.clone())
        .await
        .expect_err("trial balance must reject an invalid account element");
    assert!(matches!(trial_error, ReportError::InvalidElement(element) if element == "invalid"));

    let summary_error = reports
        .accounting_equation_summary(as_of.clone())
        .await
        .expect_err("five-element summary must reject an invalid account element");
    assert!(matches!(summary_error, ReportError::InvalidElement(element) if element == "invalid"));

    let ledger_error = reports
        .general_ledger(cash.id, as_of)
        .await
        .expect_err("general ledger must reject an invalid account element");
    assert!(matches!(ledger_error, ReportError::InvalidElement(element) if element == "invalid"));
}
