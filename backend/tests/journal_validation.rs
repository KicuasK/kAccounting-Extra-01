use kaccounting_backend::accounting::accounts::AccountId;
use kaccounting_backend::accounting::journal::{
    JournalLineInput, JournalSide, JournalValidationError, validate_journal_lines,
};
use kaccounting_backend::accounting::money::{MinorUnits, MoneyError};

fn line(account_id: i64, side: JournalSide, amount: i64) -> JournalLineInput {
    JournalLineInput::new(AccountId::new(account_id), side, MinorUnits::new(amount))
}

#[test]
fn minor_units_preserve_exact_integer_values() {
    let first = MinorUnits::new(9_007_199_254_740_993);
    let second = MinorUnits::new(7);

    assert_eq!(
        first.checked_add(second),
        Ok(MinorUnits::new(9_007_199_254_741_000))
    );
    assert_eq!(
        first.checked_sub(second),
        Ok(MinorUnits::new(9_007_199_254_740_986))
    );
    assert_eq!(second.checked_neg(), Ok(MinorUnits::new(-7)));
}

#[test]
fn minor_unit_arithmetic_reports_every_signed_boundary_overflow() {
    assert_eq!(
        MinorUnits::new(i64::MAX).checked_add(MinorUnits::new(1)),
        Err(MoneyError::Overflow)
    );
    assert_eq!(
        MinorUnits::new(i64::MIN).checked_sub(MinorUnits::new(1)),
        Err(MoneyError::Overflow)
    );
    assert_eq!(
        MinorUnits::new(i64::MIN).checked_neg(),
        Err(MoneyError::Overflow)
    );
}

#[test]
fn balanced_two_line_entry_is_valid() {
    let validated = validate_journal_lines(&[
        line(1, JournalSide::Debit, 12_345),
        line(2, JournalSide::Credit, 12_345),
    ])
    .expect("equal exact totals should validate");

    assert_eq!(validated.line_count, 2);
    assert_eq!(validated.debit_total, MinorUnits::new(12_345));
    assert_eq!(validated.credit_total, MinorUnits::new(12_345));
}

#[test]
fn balanced_compound_entry_is_valid() {
    let validated = validate_journal_lines(&[
        line(1, JournalSide::Debit, 10_000),
        line(2, JournalSide::Debit, 2_500),
        line(3, JournalSide::Credit, 12_500),
    ])
    .expect("compound entry should validate by exact totals");

    assert_eq!(validated.line_count, 3);
    assert_eq!(validated.debit_total, MinorUnits::new(12_500));
}

#[test]
fn fewer_than_two_lines_are_rejected() {
    assert_eq!(
        validate_journal_lines(&[]),
        Err(JournalValidationError::TooFewLines)
    );
    assert_eq!(
        validate_journal_lines(&[line(1, JournalSide::Debit, 100)]),
        Err(JournalValidationError::TooFewLines)
    );
}

#[test]
fn zero_and_negative_lines_are_not_effective() {
    for amount in [0, -1] {
        assert_eq!(
            validate_journal_lines(&[
                line(1, JournalSide::Debit, amount),
                line(2, JournalSide::Credit, 1),
            ]),
            Err(JournalValidationError::NonPositiveAmount { line_index: 0 })
        );
    }
}

#[test]
fn unequal_exact_totals_are_rejected() {
    assert_eq!(
        validate_journal_lines(&[
            line(1, JournalSide::Debit, 1_001),
            line(2, JournalSide::Credit, 1_000),
        ]),
        Err(JournalValidationError::Unbalanced {
            debit: 1_001,
            credit: 1_000,
        })
    );
}

#[test]
fn side_total_overflow_is_rejected_before_balance_comparison() {
    assert_eq!(
        validate_journal_lines(&[
            line(1, JournalSide::Debit, i64::MAX),
            line(2, JournalSide::Debit, 1),
            line(3, JournalSide::Credit, i64::MAX),
        ]),
        Err(JournalValidationError::TotalOverflow {
            side: JournalSide::Debit,
        })
    );
}

#[test]
fn same_account_on_both_sides_is_not_a_presentation_policy_error() {
    let lines = [
        line(7, JournalSide::Debit, 500),
        line(7, JournalSide::Credit, 500),
    ];

    assert!(validate_journal_lines(&lines).is_ok());
}
