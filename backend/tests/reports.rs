use kaccounting_backend::accounting::accounts::{
    Account, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::accounting::reports::{ReportError, ReportRepository};
use kaccounting_backend::db::open_memory_database;

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
async fn trial_balance_ignores_balanced_draft_lines() {
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
    .expect("draft should be created");
    for (line_no, account_id, side) in
        [(1, cash.id.get(), "debit"), (2, capital.id.get(), "credit")]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, 500)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .execute(&pool)
        .await
        .expect("draft line should be created");
    }

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

    let range_error = reports
        .period_general_ledger(
            cash.id,
            JournalDate::parse("2026-02-01").unwrap(),
            JournalDate::parse("2026-01-01").unwrap(),
        )
        .await
        .expect_err("inverted period must fail before querying");
    assert!(matches!(range_error, ReportError::InvalidDateRange { .. }));
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
async fn general_ledger_returns_an_empty_existing_account_and_rejects_a_missing_one() {
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

    let error = reports
        .general_ledger(
            kaccounting_backend::accounting::accounts::AccountId::new(999_999),
            JournalDate::parse("2026-12-31").unwrap(),
        )
        .await
        .expect_err("missing account should be explicit");
    assert!(matches!(error, ReportError::AccountNotFound(999_999)));
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

    let draft_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-01-15') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");
    for (line_no, account_id, side) in
        [(1, cash.id.get(), "debit"), (2, capital.id.get(), "credit")]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, 50)
            "#,
        )
        .bind(draft_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .execute(&pool)
        .await
        .expect("draft line should be inserted");
    }

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
async fn general_journal_rejects_an_inverted_date_range() {
    let pool = open_memory_database().await.expect("database should open");
    let reports = ReportRepository::new(pool.clone());

    let error = reports
        .general_journal(
            JournalDate::parse("2026-02-01").unwrap(),
            JournalDate::parse("2026-01-01").unwrap(),
        )
        .await
        .expect_err("inverted range should be explicit");

    assert!(matches!(
        error,
        ReportError::InvalidDateRange { from, through }
            if from == "2026-02-01" && through == "2026-01-01"
    ));
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

    let draft_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-01-05') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");
    for (line_no, account_id, side) in
        [(1, cash.id.get(), "debit"), (2, revenue.id.get(), "credit")]
    {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, 999)
            "#,
        )
        .bind(draft_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .execute(&pool)
        .await
        .expect("draft line should be inserted");
    }

    let report = reports
        .income_expense_report(
            JournalDate::parse("2026-01-01").unwrap(),
            JournalDate::parse("2026-01-31").unwrap(),
        )
        .await
        .expect("period activity should calculate exactly");

    assert_eq!(report.rows.len(), 2, "asset accounts must be excluded");
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
async fn income_expense_report_rejects_ranges_and_total_overflow() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let reports = ReportRepository::new(pool.clone());

    let range_error = reports
        .income_expense_report(
            JournalDate::parse("2026-02-01").unwrap(),
            JournalDate::parse("2026-01-01").unwrap(),
        )
        .await
        .expect_err("inverted range should be rejected");
    assert!(matches!(range_error, ReportError::InvalidDateRange { .. }));

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
