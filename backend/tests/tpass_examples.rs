use std::collections::BTreeMap;

use kaccounting_backend::accounting::accounts::{
    AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::db::open_memory_database;
use serde_json::Value;

const ACCOUNT_SETUP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/01-account-setup.json"
));
const BUY_PASS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/02-buy-pass.json"
));
const TPASS_RIDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/03-tpass-transit-ride.json"
));
const AMORTIZATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/04-amortization-expected.json"
));
const EASYCARD_RIDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/05-easycard-transit-and-journal.json"
));
const DELETE_RIDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/examples/06-delete-tpass-ride.json"
));
const VERIFICATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../TPASS_EXAMPLE_PACKAGE/VERIFICATION.json"
));

fn json(source: &str) -> Value {
    serde_json::from_str(source).expect("the TPASS fixture must be valid JSON")
}

fn amount(value: &Value, field: &str) -> i64 {
    value[field]
        .as_i64()
        .unwrap_or_else(|| panic!("{field} must be an exact integer amount"))
}

fn line_totals(lines: &Value) -> (i64, i64) {
    lines
        .as_array()
        .expect("journal lines must be an array")
        .iter()
        .fold((0, 0), |(debits, credits), line| {
            (
                debits + line.get("debit").and_then(Value::as_i64).unwrap_or(0),
                credits + line.get("credit").and_then(Value::as_i64).unwrap_or(0),
            )
        })
}

fn fixture_element(code: &str) -> AccountingElement {
    match code {
        "A" => AccountingElement::Asset,
        "F" => AccountingElement::Equity,
        "E" => AccountingElement::Expense,
        other => panic!("unsupported fixture account element {other}"),
    }
}

#[tokio::test]
async fn account_fixture_loads_into_an_isolated_database() {
    let fixture = json(ACCOUNT_SETUP);
    assert_eq!(
        fixture["purpose"],
        "Isolated test book only. Run in order; capture response IDs."
    );

    let pool = open_memory_database()
        .await
        .expect("the disposable in-memory database should migrate");
    let repository = AccountRepository::new(pool);
    let mut captured_accounts = BTreeMap::new();

    for request in fixture["requests"]
        .as_array()
        .expect("setup requests must be an array")
    {
        if request["action"] != "account_save" {
            continue;
        }

        let input = &request["input"];
        let name = input["name"].as_str().expect("account name must be text");
        let element = fixture_element(
            input["type"]
                .as_str()
                .expect("account element must be text"),
        );
        let account = repository
            .create(NewAccount::new(None, name, element, AccountKind::Posting))
            .await
            .expect("the fixture account should satisfy current account invariants");

        repository
            .require_postable(account.id)
            .await
            .expect("fixture accounts used by journal lines must be postable");
        captured_accounts.insert(
            request["capture"]
                .as_str()
                .expect("account requests must capture their generated ID"),
            account,
        );
    }

    assert_eq!(captured_accounts.len(), 5);
    assert_eq!(
        captured_accounts["CASH_ID"].element,
        AccountingElement::Asset
    );
    assert_eq!(
        captured_accounts["CAPITAL_ID"].element,
        AccountingElement::Equity
    );
    assert_eq!(
        captured_accounts["TRA_EXPENSE_ID"].element,
        AccountingElement::Expense
    );

    let opening_request = fixture["requests"]
        .as_array()
        .expect("setup requests must be an array")
        .iter()
        .find(|request| request["action"] == "journal_save")
        .expect("the fixture must contain an opening entry");
    let (debits, credits) = line_totals(&opening_request["input"]["lines"]);
    assert_eq!((debits, credits), (11_000, 11_000));
}

#[test]
fn tpass_examples_define_balanced_nonduplicating_financial_behavior() {
    let purchase = json(BUY_PASS);
    let ride = json(TPASS_RIDE);
    let amortization = json(AMORTIZATION);
    let easycard = json(EASYCARD_RIDE);
    let deletion = json(DELETE_RIDE);

    let pass_cost = amount(&purchase["input"], "cost");
    assert_eq!(purchase["action"], "tpass_buy");
    assert_eq!(purchase["input"]["purchase_date"], "2024-09-29");
    assert_eq!(purchase["input"]["start_date"], "2024-09-29");
    assert_eq!(purchase["expected"]["end_date"], "2024-10-28");
    assert_eq!(pass_cost, 1_200);
    assert_eq!(amount(&purchase["expected"], "purchase_debit"), pass_cost);
    assert_eq!(amount(&purchase["expected"], "purchase_credit"), pass_cost);
    assert_eq!(
        purchase["expected"]["creates_purchase_and_period_atomically"],
        true
    );

    assert_eq!(ride["action"], "document_save");
    assert_eq!(ride["input"]["type"], "Transit Ride");
    assert_eq!(ride["input"]["mrt_pass_plan"], "基北北桃 1200");
    assert_eq!(amount(&ride["input"], "mrt_wallet_charge"), 0);
    assert_eq!(amount(&ride["input"], "mrt_reference_fare"), 35);
    assert_eq!(amount(&ride["expected"], "journal_count_delta"), 0);
    assert_eq!(amount(&ride["expected"], "prepaid_balance_delta"), 0);
    assert!(ride["input"].get("lines").is_none());

    let expected_amortizations = amortization["expected_after_through_2024_10_28"]
        .as_array()
        .expect("amortizations must be an array");
    assert_eq!(expected_amortizations.len(), 2);

    let amortized_total = expected_amortizations
        .iter()
        .map(|entry| {
            assert_eq!(entry["time"], "23:59:59");
            let (debits, credits) = line_totals(&entry["lines"]);
            assert_eq!(debits, credits);
            debits
        })
        .sum::<i64>();
    assert_eq!(amortized_total, pass_cost);
    assert_eq!(amount(&amortization, "expected_total"), pass_cost);
    assert_eq!(
        amount(
            &amortization,
            "expected_prepaid_balance_after_full_amortization"
        ),
        0
    );

    let rounded_first_segment = (pass_cost * 2 + 15) / 30;
    assert_eq!(rounded_first_segment, 80);
    assert_eq!(pass_cost - rounded_first_segment, 1_120);

    let easycard_requests = easycard["requests"]
        .as_array()
        .expect("EasyCard requests must be an array");
    let easycard_document = &easycard_requests[0];
    let easycard_journal = &easycard_requests[1];
    assert_eq!(easycard_document["input"]["mrt_pass_plan"], "EasyCard");
    assert_eq!(amount(&easycard_document["input"], "mrt_wallet_charge"), 64);
    assert_eq!(line_totals(&easycard_journal["input"]["lines"]), (64, 64));
    assert_eq!(amount(&easycard["expected"], "tpass_ride_count_delta"), 0);

    assert_eq!(deletion["input"]["action"], "delete_tpass_ride");
    assert_eq!(deletion["expected"]["purchase_journal_unchanged"], true);
    assert_eq!(
        deletion["expected"]["amortization_journals_unchanged"],
        true
    );
    assert_eq!(amount(&deletion["expected"], "prepaid_balance_delta"), 0);
    assert_eq!(amount(&deletion["expected"], "ride_count_delta"), -1);
    assert_eq!(amount(&deletion["expected"], "reference_total_delta"), -35);
}

#[test]
fn package_verification_metadata_marks_the_examples_as_isolated() {
    let verification = json(VERIFICATION);

    assert_eq!(verification["isolated_book"], true);
    assert_eq!(verification["result"], "passed");
    assert_eq!(verification["verified_date"], "2026-10-22");
    assert!(
        verification["checks"]
            .as_array()
            .expect("verification checks must be an array")
            .iter()
            .any(|check| check == "TPASS ride creates no additional financial journal")
    );
}
