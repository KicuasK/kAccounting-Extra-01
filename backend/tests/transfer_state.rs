use std::path::PathBuf;

use kaccounting_backend::transfer::TransferOperationState;
use kaccounting_backend::transfer_state::{
    TRANSFER_WORK_STATE_FILE_NAME, TransferWorkStateError, create_export_work_state,
    load_transfer_work_state, transition_transfer_work_state,
};

#[test]
fn export_work_state_is_strict_private_and_durable_to_read_back() {
    let root = unique_root("create");
    create_private_root(&root);
    let destination = PathBuf::from("/Volumes/Transfer/book.kagextra");

    let created = create_export_work_state(&root, &destination)
        .expect("initial work state should be created");
    let loaded = load_transfer_work_state(&root).expect("created work state should load");

    assert_eq!(created, loaded);
    assert_eq!(loaded.destination(), destination);
    assert_eq!(loaded.state(), TransferOperationState::Working);
    assert_eq!(
        std::fs::read_to_string(root.join(TRANSFER_WORK_STATE_FILE_NAME))
            .expect("state JSON should be readable"),
        "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/Volumes/Transfer/book.kagextra\"\n}\n"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(root.join(TRANSFER_WORK_STATE_FILE_NAME))
                .expect("state metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn work_state_transitions_atomically_and_only_forward() {
    let root = unique_root("transitions");
    create_private_root(&root);
    create_export_work_state(&root, "/tmp/book.kagextra")
        .expect("initial work state should be created");

    let published = transition_transfer_work_state(
        &root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("working state should advance to published-unverified");
    assert_eq!(
        published.state().state(),
        TransferOperationState::PublishedUnverified
    );
    assert!(published.cleanup_warning().is_none());

    let verified = transition_transfer_work_state(
        &root,
        TransferOperationState::PublishedUnverified,
        TransferOperationState::Verified,
    )
    .expect("published-unverified state should advance to verified");
    assert_eq!(verified.state().state(), TransferOperationState::Verified);
    assert_eq!(
        load_transfer_work_state(&root)
            .expect("verified state should load")
            .state(),
        TransferOperationState::Verified
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(root.join(TRANSFER_WORK_STATE_FILE_NAME))
                .expect("transitioned state metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "atomic replacement must retain owner-only state permissions"
        );
    }

    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Verified,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::Transition(_))
    ));
    assert_eq!(
        load_transfer_work_state(&root)
            .expect("terminal state should remain readable")
            .state(),
        TransferOperationState::Verified
    );

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn stale_or_locked_updates_never_replace_the_current_state() {
    let root = unique_root("stale-locked");
    create_private_root(&root);
    create_export_work_state(&root, "/tmp/book.kagextra")
        .expect("initial work state should be created");
    transition_transfer_work_state(
        &root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("newer state should be persisted before the stale update");
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let original_state = std::fs::read(&state_path).expect("newer state should be readable");

    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::StaleState {
            expected: TransferOperationState::Working,
            actual: TransferOperationState::PublishedUnverified,
        })
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("stale state should remain readable"),
        original_state,
        "a stale update must preserve the exact durable state bytes"
    );
    let lock_path = root.join(".operation-state.lock");
    assert!(
        !lock_path.exists(),
        "a stale update must release the lock it acquired"
    );
    std::fs::write(&lock_path, b"held").expect("lock fixture should be written");
    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::PublishedUnverified,
            TransferOperationState::Verified,
        ),
        Err(TransferWorkStateError::UpdateLocked(path)) if path == lock_path
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("locked state should remain readable"),
        original_state,
        "a locked update must preserve the exact durable state bytes"
    );
    assert_eq!(
        std::fs::read(&lock_path).expect("existing lock should remain readable"),
        b"held",
        "a rejected updater must not alter another writer's lock"
    );
    assert_eq!(
        load_transfer_work_state(&root)
            .expect("unchanged state should load")
            .state(),
        TransferOperationState::PublishedUnverified
    );

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn concurrent_state_writers_persist_exactly_one_complete_transition() {
    let root = unique_root("concurrent-writers");
    create_private_root(&root);
    create_export_work_state(&root, "/tmp/book.kagextra")
        .expect("initial work state should be created");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let published_root = root.clone();
    let published_barrier = barrier.clone();
    let published = std::thread::spawn(move || {
        published_barrier.wait();
        (
            TransferOperationState::PublishedUnverified,
            transition_transfer_work_state(
                published_root,
                TransferOperationState::Working,
                TransferOperationState::PublishedUnverified,
            ),
        )
    });
    let failed_root = root.clone();
    let failed_barrier = barrier.clone();
    let failed = std::thread::spawn(move || {
        failed_barrier.wait();
        (
            TransferOperationState::Failed,
            transition_transfer_work_state(
                failed_root,
                TransferOperationState::Working,
                TransferOperationState::Failed,
            ),
        )
    });

    barrier.wait();
    let results = [
        published.join().expect("published writer should not panic"),
        failed.join().expect("failed writer should not panic"),
    ];
    let mut winner = None;
    let lock_path = root.join(".operation-state.lock");
    for (attempted, result) in results {
        match result {
            Ok(outcome) => {
                assert!(
                    winner.replace(attempted).is_none(),
                    "only one writer may win"
                );
                assert_eq!(outcome.state().state(), attempted);
                assert!(outcome.cleanup_warning().is_none());
            }
            Err(TransferWorkStateError::UpdateLocked(path)) => assert_eq!(path, lock_path),
            Err(TransferWorkStateError::StaleState {
                expected: TransferOperationState::Working,
                actual,
            }) => assert!(matches!(
                actual,
                TransferOperationState::PublishedUnverified | TransferOperationState::Failed
            )),
            Err(error) => panic!("unexpected concurrent-writer error: {error:?}"),
        }
    }

    let winner = winner.expect("exactly one concurrent writer should persist its transition");
    assert_eq!(
        load_transfer_work_state(&root)
            .expect("winning state should remain readable")
            .state(),
        winner
    );
    assert_no_transition_artifacts(&root);

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn failed_transition_from_corrupt_state_preserves_evidence_and_releases_its_lock() {
    let root = unique_root("corrupt-transition");
    create_private_root(&root);
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let corrupt = b"{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n";
    std::fs::write(&state_path, corrupt).expect("corrupt state fixture should be written");
    set_private_file_permissions(&state_path);

    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::Json { path, .. }) if path == state_path
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("corrupt evidence should remain readable"),
        corrupt,
        "a failed transition must not rewrite corrupt recovery evidence"
    );
    assert_no_transition_artifacts(&root);

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn transition_from_missing_state_fails_closed_and_releases_its_lock() {
    let root = unique_root("missing-transition-state");
    create_private_root(&root);
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);

    let error = transition_transfer_work_state(
        &root,
        TransferOperationState::Working,
        TransferOperationState::Failed,
    )
    .expect_err("a missing durable state must not be synthesized by an update");
    assert!(matches!(
        error,
        TransferWorkStateError::Io {
            operation: "inspecting transfer work-state path",
            path,
            source,
        } if path == state_path && source.kind() == std::io::ErrorKind::NotFound
    ));
    assert!(!state_path.exists());
    assert_no_transition_artifacts(&root);

    std::fs::remove_dir(root).expect("work root should be removed");
}

#[test]
fn oversized_state_transition_preserves_all_bytes_and_releases_its_lock() {
    let root = unique_root("oversized-transition-state");
    create_private_root(&root);
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let oversized = vec![b'x'; 64 * 1024 + 1];
    std::fs::write(&state_path, &oversized).expect("oversized state fixture should be written");
    set_private_file_permissions(&state_path);

    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::StateTooLarge {
            path,
            actual: 65_537,
            maximum: 65_536,
        }) if path == state_path
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("oversized evidence should remain readable"),
        oversized
    );
    assert_no_transition_artifacts(&root);

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn state_directory_is_never_replaced_or_removed_by_create_load_or_transition() {
    let root = unique_root("state-directory");
    create_private_root(&root);
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    std::fs::create_dir(&state_path).expect("state directory fixture should be created");
    let sentinel = state_path.join("keep.txt");
    std::fs::write(&sentinel, b"recovery evidence")
        .expect("state-directory sentinel should be written");

    assert!(matches!(
        create_export_work_state(&root, "/tmp/book.kagextra"),
        Err(TransferWorkStateError::StateExists(path)) if path == state_path
    ));
    assert!(matches!(
        load_transfer_work_state(&root),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_path
    ));
    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_path
    ));
    assert_eq!(
        std::fs::read(&sentinel).expect("state-directory sentinel should remain readable"),
        b"recovery evidence"
    );
    assert_no_transition_artifacts(&root);

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn every_disallowed_transition_preserves_the_exact_current_record() {
    let states = [
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
        TransferOperationState::Verified,
        TransferOperationState::Failed,
    ];

    for current in states {
        let root = unique_root(&format!("invalid-transition-{current:?}"));
        create_private_root(&root);
        seed_work_state(&root, current);
        let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
        let before = std::fs::read(&state_path).expect("current state should be readable");

        for next in states {
            let allowed = matches!(
                (current, next),
                (
                    TransferOperationState::Working,
                    TransferOperationState::PublishedUnverified | TransferOperationState::Failed
                ) | (
                    TransferOperationState::PublishedUnverified,
                    TransferOperationState::Verified | TransferOperationState::Failed
                )
            );
            if allowed {
                continue;
            }

            assert!(matches!(
                transition_transfer_work_state(&root, current, next),
                Err(TransferWorkStateError::Transition(error))
                    if error.current == current && error.next == next
            ));
            assert_eq!(
                std::fs::read(&state_path).expect("rejected state should remain readable"),
                before,
                "rejected transition from {current:?} to {next:?} changed durable state"
            );
            assert_no_transition_artifacts(&root);
        }

        std::fs::remove_dir_all(root).expect("work root should be removed");
    }
}

#[test]
fn both_failure_transitions_are_durable_and_terminal() {
    for (label, first) in [
        ("working-failed", None),
        (
            "published-failed",
            Some(TransferOperationState::PublishedUnverified),
        ),
    ] {
        let root = unique_root(label);
        create_private_root(&root);
        create_export_work_state(&root, "/tmp/book.kagextra")
            .expect("initial work state should be created");
        let expected = if let Some(intermediate) = first {
            transition_transfer_work_state(&root, TransferOperationState::Working, intermediate)
                .expect("intermediate state should be persisted");
            intermediate
        } else {
            TransferOperationState::Working
        };

        transition_transfer_work_state(&root, expected, TransferOperationState::Failed)
            .expect("failure state should be persisted");
        assert_eq!(
            load_transfer_work_state(&root)
                .expect("failed state should load")
                .state(),
            TransferOperationState::Failed
        );
        assert_no_transition_artifacts(&root);

        std::fs::remove_dir_all(root).expect("work root should be removed");
    }
}

#[test]
fn existing_or_invalid_state_is_never_silently_replaced() {
    let existing_root = unique_root("existing");
    create_private_root(&existing_root);
    create_export_work_state(&existing_root, "/tmp/first.kagextra")
        .expect("initial work state should be created");
    let existing_state_path = existing_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let existing_before =
        std::fs::read(&existing_state_path).expect("initial work state should be readable");
    assert!(matches!(
        create_export_work_state(&existing_root, "/tmp/second.kagextra"),
        Err(TransferWorkStateError::StateExists(_))
    ));
    assert_eq!(
        std::fs::read(&existing_state_path).expect("original state should remain readable"),
        existing_before,
        "create-new semantics must preserve the exact existing state bytes"
    );
    assert_eq!(
        load_transfer_work_state(&existing_root)
            .expect("original state should remain readable")
            .destination(),
        PathBuf::from("/tmp/first.kagextra")
    );
    std::fs::remove_dir_all(existing_root).expect("work root should be removed");

    let invalid_root = unique_root("invalid-json");
    create_private_root(&invalid_root);
    std::fs::write(
        invalid_root.join(TRANSFER_WORK_STATE_FILE_NAME),
        br#"{
  "format_version": 1,
  "operation": "export",
  "state": "working",
  "destination": "/tmp/book.kagextra",
  "unexpected": true
}
"#,
    )
    .expect("invalid state fixture should be written");
    set_private_file_permissions(&invalid_root.join(TRANSFER_WORK_STATE_FILE_NAME));
    assert!(matches!(
        load_transfer_work_state(&invalid_root),
        Err(TransferWorkStateError::Json { .. })
    ));
    std::fs::remove_dir_all(invalid_root).expect("work root should be removed");
}

#[test]
fn work_state_rejects_unsupported_version_operation_and_corrupt_json_without_changes() {
    let cases = [
        (
            "unsupported-version",
            "{\n  \"format_version\": 2,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/book.kagextra\"\n}\n",
            "version",
        ),
        (
            "unsupported-operation",
            "{\n  \"format_version\": 1,\n  \"operation\": \"import\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/book.kagextra\"\n}\n",
            "operation",
        ),
        (
            "invalid-state",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"complete\",\n  \"destination\": \"/tmp/book.kagextra\"\n}\n",
            "json",
        ),
        (
            "duplicate-state",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"state\": \"failed\",\n  \"destination\": \"/tmp/book.kagextra\"\n}\n",
            "json",
        ),
        (
            "truncated-json",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n",
            "json",
        ),
        (
            "trailing-json",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/book.kagextra\"\n}\ntrue\n",
            "json",
        ),
        (
            "relative-destination",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"relative/book.kagextra\"\n}\n",
            "destination",
        ),
        (
            "nul-destination",
            "{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/book\\u0000.kagextra\"\n}\n",
            "nul-destination",
        ),
    ];

    for (label, document, expected) in cases {
        let root = unique_root(label);
        create_private_root(&root);
        let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
        std::fs::write(&state_path, document).expect("state fixture should be written");
        set_private_file_permissions(&state_path);

        let error = load_transfer_work_state(&root).expect_err("invalid state must fail closed");
        match expected {
            "version" => assert!(matches!(
                error,
                TransferWorkStateError::UnsupportedVersion { actual: 2 }
            )),
            "operation" => assert!(matches!(
                error,
                TransferWorkStateError::UnsupportedOperation(operation)
                    if operation == "import"
            )),
            "json" => assert!(matches!(error, TransferWorkStateError::Json { .. })),
            "destination" => assert!(matches!(
                error,
                TransferWorkStateError::InvalidDestination(path)
                    if path.as_path() == std::path::Path::new("relative/book.kagextra")
            )),
            "nul-destination" => assert!(matches!(
                error,
                TransferWorkStateError::InvalidDestination(path)
                    if path.as_path() == std::path::Path::new("/tmp/book\0.kagextra")
            )),
            other => panic!("unexpected fixture expectation: {other}"),
        }
        assert_eq!(
            std::fs::read_to_string(&state_path).expect("invalid state should remain readable"),
            document,
            "failed decoding must never rewrite recovery evidence"
        );
        std::fs::remove_dir_all(root).expect("work root should be removed");
    }
}

#[test]
fn work_state_rejects_non_utf8_json_without_changing_recovery_evidence() {
    let root = unique_root("non-utf8-json");
    create_private_root(&root);
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let invalid_utf8 = b"{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/\xff.kagextra\"\n}\n";
    std::fs::write(&state_path, invalid_utf8).expect("non-UTF-8 fixture should be written");
    set_private_file_permissions(&state_path);

    assert!(matches!(
        load_transfer_work_state(&root),
        Err(TransferWorkStateError::Json { path, .. }) if path == state_path
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("invalid state should remain readable"),
        invalid_utf8
    );

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn work_state_requires_an_absolute_destination_and_bounded_document() {
    let relative_root = unique_root("relative-destination");
    create_private_root(&relative_root);
    assert!(matches!(
        create_export_work_state(&relative_root, "relative/book.kagextra"),
        Err(TransferWorkStateError::InvalidDestination(path))
            if path.as_path() == std::path::Path::new("relative/book.kagextra")
    ));
    assert!(!relative_root.join(TRANSFER_WORK_STATE_FILE_NAME).exists());
    std::fs::remove_dir(relative_root).expect("work root should be removed");

    let nul_root = unique_root("nul-destination-create");
    create_private_root(&nul_root);
    let nul_destination = std::path::Path::new("/tmp/book\0.kagextra");
    assert!(matches!(
        create_export_work_state(&nul_root, nul_destination),
        Err(TransferWorkStateError::InvalidDestination(path))
            if path.as_path() == nul_destination
    ));
    assert!(!nul_root.join(TRANSFER_WORK_STATE_FILE_NAME).exists());
    std::fs::remove_dir(nul_root).expect("work root should be removed");

    let oversized_root = unique_root("oversized");
    create_private_root(&oversized_root);
    std::fs::write(
        oversized_root.join(TRANSFER_WORK_STATE_FILE_NAME),
        vec![b' '; 64 * 1024 + 1],
    )
    .expect("oversized fixture should be written");
    set_private_file_permissions(&oversized_root.join(TRANSFER_WORK_STATE_FILE_NAME));
    assert!(matches!(
        load_transfer_work_state(&oversized_root),
        Err(TransferWorkStateError::StateTooLarge {
            actual: 65_537,
            maximum: 65_536,
            ..
        })
    ));
    std::fs::remove_dir_all(oversized_root).expect("work root should be removed");
}

#[cfg(unix)]
#[test]
fn work_state_loader_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let root = unique_root("symlink");
    create_private_root(&root);
    let external = unique_root("external-state.json");
    std::fs::write(&external, b"{}\n").expect("external state should be written");
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    symlink(&external, &state_path).expect("state symlink should be created");

    assert!(matches!(
        load_transfer_work_state(&root),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_path
    ));

    std::fs::remove_file(state_path).expect("state symlink should be removed");
    std::fs::remove_file(external).expect("external state should be removed");
    std::fs::remove_dir(root).expect("work root should be removed");
}

#[cfg(unix)]
#[test]
fn transitions_do_not_follow_state_or_lock_symlinks() {
    use std::os::unix::fs::symlink;

    let external_root = unique_root("external-state-root");
    create_private_root(&external_root);
    create_export_work_state(&external_root, "/tmp/external.kagextra")
        .expect("external state fixture should be created");
    let external_state = external_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let external_before =
        std::fs::read(&external_state).expect("external state should be readable");

    let state_link_root = unique_root("transition-state-symlink");
    create_private_root(&state_link_root);
    let state_link = state_link_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    symlink(&external_state, &state_link).expect("state symlink should be created");
    assert!(matches!(
        create_export_work_state(&state_link_root, "/tmp/replacement.kagextra"),
        Err(TransferWorkStateError::StateExists(path)) if path == state_link
    ));
    assert!(matches!(
        transition_transfer_work_state(
            &state_link_root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_link
    ));
    assert_eq!(
        std::fs::read(&external_state).expect("external state should remain readable"),
        external_before,
        "a state symlink must never redirect an atomic transition"
    );
    assert_no_transition_artifacts(&state_link_root);
    std::fs::remove_dir_all(&state_link_root).expect("state-link root should be removed");

    let lock_link_root = unique_root("transition-lock-symlink");
    create_private_root(&lock_link_root);
    create_export_work_state(&lock_link_root, "/tmp/local.kagextra")
        .expect("local state fixture should be created");
    let local_state = lock_link_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let local_before = std::fs::read(&local_state).expect("local state should be readable");
    let external_lock_target = unique_root("external-lock-target");
    std::fs::write(&external_lock_target, b"do not touch")
        .expect("external lock target should be written");
    let lock_link = lock_link_root.join(".operation-state.lock");
    symlink(&external_lock_target, &lock_link).expect("lock symlink should be created");
    assert!(matches!(
        transition_transfer_work_state(
            &lock_link_root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::UpdateLocked(path)) if path == lock_link
    ));
    assert_eq!(
        std::fs::read(&local_state).expect("local state should remain readable"),
        local_before,
        "a lock symlink must block rather than redirect an update"
    );
    assert_eq!(
        std::fs::read(&external_lock_target).expect("external lock target should be readable"),
        b"do not touch"
    );
    assert!(
        std::fs::symlink_metadata(&lock_link)
            .expect("lock symlink metadata should remain readable")
            .file_type()
            .is_symlink()
    );

    std::fs::remove_dir_all(lock_link_root).expect("lock-link root should be removed");
    std::fs::remove_file(external_lock_target).expect("external lock target should be removed");
    std::fs::remove_dir_all(external_root).expect("external root should be removed");
}

#[cfg(unix)]
#[test]
fn work_root_symlinks_are_rejected_without_touching_the_target() {
    use std::os::unix::fs::symlink;

    let real_root = unique_root("real-root");
    create_private_root(&real_root);
    create_export_work_state(&real_root, "/tmp/book.kagextra")
        .expect("real state fixture should be created");
    let state_path = real_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let before = std::fs::read(&state_path).expect("real state should be readable");
    let linked_root = unique_root("linked-root");
    symlink(&real_root, &linked_root).expect("work-root symlink should be created");

    assert!(matches!(
        load_transfer_work_state(&linked_root),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == linked_root
    ));
    assert!(matches!(
        create_export_work_state(&linked_root, "/tmp/replacement.kagextra"),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == linked_root
    ));
    assert!(matches!(
        transition_transfer_work_state(
            &linked_root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == linked_root
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("real state should remain readable"),
        before
    );

    std::fs::remove_file(linked_root).expect("work-root symlink should be removed");
    std::fs::remove_dir_all(real_root).expect("real root should be removed");
}

#[cfg(unix)]
#[test]
fn work_state_rejects_shared_roots_and_state_files() {
    use std::os::unix::fs::PermissionsExt;

    let shared_root = unique_root("shared-root");
    std::fs::create_dir(&shared_root).expect("shared root should be created");
    std::fs::set_permissions(&shared_root, std::fs::Permissions::from_mode(0o755))
        .expect("shared root permissions should be set");
    let shared_state_path = shared_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let shared_document = b"{\n  \"format_version\": 1,\n  \"operation\": \"export\",\n  \"state\": \"working\",\n  \"destination\": \"/tmp/existing.kagextra\"\n}\n";
    std::fs::write(&shared_state_path, shared_document)
        .expect("shared-root state fixture should be written");
    set_private_file_permissions(&shared_state_path);
    assert!(matches!(
        create_export_work_state(&shared_root, "/tmp/book.kagextra"),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == shared_root
    ));
    assert!(matches!(
        load_transfer_work_state(&shared_root),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == shared_root
    ));
    assert!(matches!(
        transition_transfer_work_state(
            &shared_root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::InvalidRoot(path)) if path == shared_root
    ));
    assert_eq!(
        std::fs::read(&shared_state_path).expect("shared-root state should remain readable"),
        shared_document
    );
    assert!(!shared_root.join(".operation-state.lock").exists());
    std::fs::remove_dir_all(shared_root).expect("shared root should be removed");

    let root = unique_root("shared-state-file");
    create_private_root(&root);
    create_export_work_state(&root, "/tmp/book.kagextra")
        .expect("private state fixture should be created");
    let state_path = root.join(TRANSFER_WORK_STATE_FILE_NAME);
    std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o644))
        .expect("state fixture permissions should be weakened");
    let before = std::fs::read(&state_path).expect("shared state fixture should be readable");
    assert!(matches!(
        load_transfer_work_state(&root),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_path
    ));
    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::UnsafeStateFile(path)) if path == state_path
    ));
    assert_eq!(
        std::fs::read(&state_path).expect("shared state fixture should remain readable"),
        before
    );
    assert_no_transition_artifacts(&root);
    std::fs::remove_dir_all(root).expect("work root should be removed");
}

fn unique_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "kAccounting-Extra-01-transfer-state-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos()
    ))
}

fn create_private_root(path: &std::path::Path) {
    std::fs::create_dir(path).expect("work root should be created");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("work root should be owner-only");
    }
}

fn set_private_file_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .expect("state fixture should be owner-only");
    }
}

fn seed_work_state(root: &std::path::Path, state: TransferOperationState) {
    create_export_work_state(root, "/tmp/book.kagextra")
        .expect("initial work state should be created");
    match state {
        TransferOperationState::Working => {}
        TransferOperationState::PublishedUnverified => {
            transition_transfer_work_state(
                root,
                TransferOperationState::Working,
                TransferOperationState::PublishedUnverified,
            )
            .expect("published-unverified state should be created");
        }
        TransferOperationState::Verified => {
            transition_transfer_work_state(
                root,
                TransferOperationState::Working,
                TransferOperationState::PublishedUnverified,
            )
            .expect("published-unverified state should be created");
            transition_transfer_work_state(
                root,
                TransferOperationState::PublishedUnverified,
                TransferOperationState::Verified,
            )
            .expect("verified state should be created");
        }
        TransferOperationState::Failed => {
            transition_transfer_work_state(
                root,
                TransferOperationState::Working,
                TransferOperationState::Failed,
            )
            .expect("failed state should be created");
        }
    }
}

fn assert_no_transition_artifacts(root: &std::path::Path) {
    let unexpected: Vec<_> = std::fs::read_dir(root)
        .expect("work root should be readable")
        .map(|entry| {
            entry
                .expect("work-root entry should be readable")
                .file_name()
        })
        .filter(|name| name != TRANSFER_WORK_STATE_FILE_NAME)
        .collect();
    assert!(
        unexpected.is_empty(),
        "failed transition left unexpected artifacts: {unexpected:?}"
    );
}
