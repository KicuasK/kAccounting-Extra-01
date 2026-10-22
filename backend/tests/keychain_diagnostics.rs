//! Pure fake-error acceptance; never reads or writes the operating-system key store.
use kaccounting_backend::{keychain_diagnostics::classify, keys::KeyStoreError};

#[test]
fn missing_and_malformed_keys_require_preservation_not_regeneration() {
    for error in [KeyStoreError::Missing, KeyStoreError::Unavailable(-25300)] {
        let problem = classify(&error);
        assert_eq!(problem.code, "missing_original_key");
        assert!(problem.operator_guidance.contains("Preserve this book"));
        assert!(problem.operator_guidance.contains("original key"));
        assert!(
            problem
                .operator_guidance
                .contains("Do not create a replacement key")
        );
    }
    let problem = classify(&KeyStoreError::MalformedSecret);
    assert_eq!(problem.code, "malformed_key");
    assert!(problem.operator_guidance.contains("key item unchanged"));
    assert!(
        problem
            .operator_guidance
            .contains("Do not repair or replace")
    );
}

#[test]
fn cancellation_and_interaction_are_distinct_and_do_not_claim_locked_state() {
    let cancelled = classify(&KeyStoreError::Unavailable(-128));
    assert_eq!(cancelled.code, "keychain_user_cancelled");
    assert!(cancelled.operator_guidance.contains("when ready"));
    for status in [-25308, -25315] {
        let problem = classify(&KeyStoreError::Unavailable(status));
        assert_eq!(problem.code, "keychain_interaction_required");
        assert!(
            problem
                .operator_guidance
                .contains("does not prove it is locked")
        );
        assert!(
            problem
                .operator_guidance
                .contains("local logged-in session")
        );
        assert!(
            problem
                .operator_guidance
                .contains("do not create a replacement key")
        );
    }
}

#[test]
fn generic_authentication_decode_and_unknown_errors_remain_unavailable() {
    for status in [
        -25291,
        -25293,
        -25294,
        -25295,
        -26275,
        -34018,
        0,
        i32::MIN,
        i32::MAX,
    ] {
        let problem = classify(&KeyStoreError::Unavailable(status));
        assert_eq!(problem.code, "keychain_unavailable");
        assert!(
            problem
                .operator_guidance
                .contains("Preserve the book and existing key items")
        );
        assert!(
            problem
                .operator_guidance
                .contains("Do not create a replacement key")
        );
        assert!(!problem.operator_guidance.contains(&status.to_string()));
    }
}

#[test]
fn invalid_binding_collision_and_random_failure_never_authorize_key_replacement() {
    let invalid = classify(&KeyStoreError::InvalidIdentifier);
    assert_eq!(invalid.code, "invalid_key_binding");
    assert!(invalid.operator_guidance.contains("Do not guess"));
    for error in [
        KeyStoreError::AlreadyExists,
        KeyStoreError::Unavailable(-25299),
    ] {
        let problem = classify(&error);
        assert_eq!(problem.code, "key_already_exists");
        assert!(
            problem
                .operator_guidance
                .contains("Do not overwrite or delete")
        );
    }
    let random = classify(&KeyStoreError::RandomUnavailable);
    assert_eq!(random.code, "secure_random_unavailable");
    assert!(
        random
            .operator_guidance
            .contains("Do not substitute a predictable key")
    );
}

#[test]
fn serialized_problem_is_static_bounded_and_contains_no_raw_status_or_identifier() {
    let problem = classify(&KeyStoreError::Unavailable(-25308));
    let value = serde_json::to_value(problem).unwrap();
    let fields = value.as_object().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields["code"], "keychain_interaction_required");
    assert!(fields.contains_key("operator_guidance"));
    let output = value.to_string();
    assert!(output.len() < 1024);
    assert!(!output.contains("-25308"));
    assert!(!output.contains("key_id"));
    assert!(!output.contains("system status"));
}
