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
        MinorUnits::new(i64::MIN).checked_add(MinorUnits::new(-1)),
        Err(MoneyError::Overflow)
    );
    assert_eq!(
        MinorUnits::new(i64::MAX).checked_sub(MinorUnits::new(-1)),
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
fn exact_total_boundary_matrix_preserves_compound_splits_on_both_sides() {
    let totals = [
        2,
        3,
        9_007_199_254_740_991,
        9_007_199_254_740_992,
        9_007_199_254_740_993,
        i64::MAX - 1,
        i64::MAX,
    ];

    for total in totals {
        for first_part in [1, total / 2, total - 1] {
            let second_part = total - first_part;

            for (compound_side, single_side) in [
                (JournalSide::Debit, JournalSide::Credit),
                (JournalSide::Credit, JournalSide::Debit),
            ] {
                let lines = [
                    line(1, compound_side, first_part),
                    line(2, compound_side, second_part),
                    line(3, single_side, total),
                ];

                let validated = validate_journal_lines(&lines).unwrap_or_else(|error| {
                    panic!(
                        "total {total}, split {first_part} + {second_part}, and side \
                         {compound_side:?} should validate: {error:?}"
                    )
                });

                assert_eq!(validated.line_count, 3);
                assert_eq!(validated.debit_total, MinorUnits::new(total));
                assert_eq!(validated.credit_total, MinorUnits::new(total));
            }
        }
    }
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
fn raw_line_count_does_not_allow_an_invalid_second_line_to_satisfy_the_minimum() {
    for amount in [0, -1, i64::MIN] {
        assert_eq!(
            validate_journal_lines(&[
                line(1, JournalSide::Debit, 1),
                line(2, JournalSide::Credit, amount),
            ]),
            Err(JournalValidationError::NonPositiveAmount { line_index: 1 }),
            "a second line with amount {amount} is not a valid journal line"
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
fn totals_beyond_binary64_precision_still_detect_a_one_unit_difference() {
    let debit = 9_007_199_254_740_993;
    let credit = 9_007_199_254_740_992;

    assert_eq!(
        validate_journal_lines(&[
            line(1, JournalSide::Debit, debit),
            line(2, JournalSide::Credit, credit),
        ]),
        Err(JournalValidationError::Unbalanced { debit, credit })
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
fn credit_total_overflow_is_rejected_symmetrically() {
    assert_eq!(
        validate_journal_lines(&[
            line(1, JournalSide::Debit, i64::MAX),
            line(2, JournalSide::Credit, i64::MAX),
            line(3, JournalSide::Credit, 1),
        ]),
        Err(JournalValidationError::TotalOverflow {
            side: JournalSide::Credit,
        })
    );
}

#[test]
fn total_overflow_boundary_matrix_is_side_and_order_symmetric() {
    for overflowing_side in [JournalSide::Debit, JournalSide::Credit] {
        let balancing_side = match overflowing_side {
            JournalSide::Debit => JournalSide::Credit,
            JournalSide::Credit => JournalSide::Debit,
        };

        for overflowing_parts in [[i64::MAX, 1], [1, i64::MAX], [i64::MAX - 1, 2]] {
            assert_eq!(
                validate_journal_lines(&[
                    line(1, overflowing_side, overflowing_parts[0]),
                    line(2, overflowing_side, overflowing_parts[1]),
                    line(3, balancing_side, i64::MAX),
                ]),
                Err(JournalValidationError::TotalOverflow {
                    side: overflowing_side,
                }),
                "{overflowing_side:?} total should reject overflowing parts {overflowing_parts:?}"
            );
        }
    }
}

#[test]
fn largest_exact_balanced_totals_remain_valid() {
    let validated = validate_journal_lines(&[
        line(1, JournalSide::Debit, i64::MAX),
        line(2, JournalSide::Credit, i64::MAX),
    ])
    .expect("the largest representable exact total must not be rejected");

    assert_eq!(validated.debit_total, MinorUnits::new(i64::MAX));
    assert_eq!(validated.credit_total, MinorUnits::new(i64::MAX));
}

#[test]
fn same_account_on_both_sides_is_not_a_presentation_policy_error() {
    let lines = [
        line(7, JournalSide::Debit, 500),
        line(7, JournalSide::Credit, 500),
    ];

    assert!(validate_journal_lines(&lines).is_ok());
}
