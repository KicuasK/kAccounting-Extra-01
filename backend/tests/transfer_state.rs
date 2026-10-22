use std::path::PathBuf;

use kaccounting_backend::transfer::TransferOperationState;
use kaccounting_backend::transfer_state::{
    TRANSFER_WORK_STATE_FILE_NAME, TransferWorkStateError, create_export_work_state,
    load_transfer_work_state, transition_transfer_work_state,
};

#[test]
fn export_work_state_is_strict_private_and_durable_to_read_back() {
    let root = unique_root("create");
    std::fs::create_dir(&root).expect("work root should be created");
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
    std::fs::create_dir(&root).expect("work root should be created");
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
    std::fs::create_dir(&root).expect("work root should be created");
    create_export_work_state(&root, "/tmp/book.kagextra")
        .expect("initial work state should be created");

    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::PublishedUnverified,
            TransferOperationState::Verified,
        ),
        Err(TransferWorkStateError::StaleState {
            expected: TransferOperationState::PublishedUnverified,
            actual: TransferOperationState::Working,
        })
    ));
    let lock_path = root.join(".operation-state.lock");
    std::fs::write(&lock_path, b"held").expect("lock fixture should be written");
    assert!(matches!(
        transition_transfer_work_state(
            &root,
            TransferOperationState::Working,
            TransferOperationState::Failed,
        ),
        Err(TransferWorkStateError::UpdateLocked(path)) if path == lock_path
    ));
    assert_eq!(
        load_transfer_work_state(&root)
            .expect("unchanged state should load")
            .state(),
        TransferOperationState::Working
    );

    std::fs::remove_dir_all(root).expect("work root should be removed");
}

#[test]
fn existing_or_invalid_state_is_never_silently_replaced() {
    let existing_root = unique_root("existing");
    std::fs::create_dir(&existing_root).expect("work root should be created");
    create_export_work_state(&existing_root, "/tmp/first.kagextra")
        .expect("initial work state should be created");
    assert!(matches!(
        create_export_work_state(&existing_root, "/tmp/second.kagextra"),
        Err(TransferWorkStateError::StateExists(_))
    ));
    assert_eq!(
        load_transfer_work_state(&existing_root)
            .expect("original state should remain readable")
            .destination(),
        PathBuf::from("/tmp/first.kagextra")
    );
    std::fs::remove_dir_all(existing_root).expect("work root should be removed");

    let invalid_root = unique_root("invalid-json");
    std::fs::create_dir(&invalid_root).expect("work root should be created");
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
    assert!(matches!(
        load_transfer_work_state(&invalid_root),
        Err(TransferWorkStateError::Json { .. })
    ));
    std::fs::remove_dir_all(invalid_root).expect("work root should be removed");
}

#[test]
fn work_state_requires_an_absolute_destination_and_bounded_document() {
    let relative_root = unique_root("relative-destination");
    std::fs::create_dir(&relative_root).expect("work root should be created");
    assert!(matches!(
        create_export_work_state(&relative_root, "relative/book.kagextra"),
        Err(TransferWorkStateError::InvalidDestination(path))
            if path.as_path() == std::path::Path::new("relative/book.kagextra")
    ));
    assert!(!relative_root.join(TRANSFER_WORK_STATE_FILE_NAME).exists());
    std::fs::remove_dir(relative_root).expect("work root should be removed");

    let oversized_root = unique_root("oversized");
    std::fs::create_dir(&oversized_root).expect("work root should be created");
    std::fs::write(
        oversized_root.join(TRANSFER_WORK_STATE_FILE_NAME),
        vec![b' '; 64 * 1024 + 1],
    )
    .expect("oversized fixture should be written");
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
    std::fs::create_dir(&root).expect("work root should be created");
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
