use std::path::{Path, PathBuf};

use kaccounting_backend::db::{backup_database, open_database};
use kaccounting_backend::storage::{
    LocalDirectoryPublishError, ManifestAttachment, TRANSFER_MANIFEST_FILE_NAME, TransferManifest,
    describe_file,
};
use kaccounting_backend::transfer::{
    LiveBookExportError, STAGED_DATABASE_PATH, TransferAttachmentSource, TransferExportError,
    TransferOperationState, TransferPackageAssemblyError, TransferPackagePublishError,
    TransferPackageVerificationError, TransferRecoveryError, TransferRecoveryOutcome,
    TransferStagingError, TransferStateTransitionError, TransferVerificationError,
    assemble_private_transfer_package, export_live_book_snapshot, export_staged_transfer_package,
    load_and_verify_transfer_package, prepare_transfer_staging, publish_private_transfer_package,
    recover_export_work_state, verify_staged_transfer,
};
use kaccounting_backend::transfer_state::{
    TRANSFER_WORK_STATE_FILE_NAME, TransferWorkStateError, create_export_work_state,
    load_transfer_work_state, transition_transfer_work_state,
};

#[test]
fn transfer_lifecycle_requires_publication_then_verification() {
    let published = TransferOperationState::Working
        .transition(TransferOperationState::PublishedUnverified)
        .expect("working output may be published but is not yet complete");
    assert!(!published.is_terminal());

    let verified = published
        .transition(TransferOperationState::Verified)
        .expect("published output may become verified");
    assert!(verified.is_terminal());
}

#[test]
fn transfer_lifecycle_rejects_skips_retries_and_terminal_changes() {
    for (current, next) in [
        (
            TransferOperationState::Working,
            TransferOperationState::Verified,
        ),
        (
            TransferOperationState::Working,
            TransferOperationState::Working,
        ),
        (
            TransferOperationState::Verified,
            TransferOperationState::Failed,
        ),
        (
            TransferOperationState::Failed,
            TransferOperationState::Working,
        ),
    ] {
        assert_eq!(
            current.transition(next).unwrap_err(),
            TransferStateTransitionError { current, next }
        );
    }

    assert!(
        TransferOperationState::Working
            .transition(TransferOperationState::Failed)
            .expect("working operation may fail")
            .is_terminal()
    );
    assert!(
        TransferOperationState::PublishedUnverified
            .transition(TransferOperationState::Failed)
            .expect("published but unverified operation may fail")
            .is_terminal()
    );
}

#[tokio::test]
async fn staged_transfer_verifies_database_and_every_attachment() {
    let fixture = TransferFixture::create("valid").await;

    verify_staged_transfer(&fixture.root, &fixture.manifest)
        .await
        .expect("complete staging directory should verify");

    fixture.remove();
}

#[tokio::test]
async fn staged_transfer_rejects_corrupt_and_missing_attachments() {
    let corrupt = TransferFixture::create("corrupt").await;
    std::fs::write(&corrupt.attachment_path, b"corrupted evidence")
        .expect("attachment should be corruptible in isolated fixture");
    assert!(matches!(
        verify_staged_transfer(&corrupt.root, &corrupt.manifest).await,
        Err(TransferVerificationError::Object { logical_path, .. })
            if logical_path == "attachments/evidence.bin"
    ));
    corrupt.remove();

    let missing = TransferFixture::create("missing").await;
    std::fs::remove_file(&missing.attachment_path)
        .expect("attachment should be removable in isolated fixture");
    assert!(matches!(
        verify_staged_transfer(&missing.root, &missing.manifest).await,
        Err(TransferVerificationError::Object { logical_path, .. })
            if logical_path == "attachments/evidence.bin"
    ));
    missing.remove();
}

#[tokio::test]
async fn staged_transfer_rejects_bytes_that_are_not_an_application_database() {
    let fixture = TransferFixture::create("wrong-database").await;
    std::fs::write(&fixture.database_path, b"not an application database")
        .expect("database fixture should be replaceable");
    let wrong_descriptor = describe_file(&fixture.database_path).expect("bytes should describe");
    let manifest = TransferManifest::new(wrong_descriptor, fixture.manifest.attachments().to_vec())
        .expect("logical manifest should remain valid");

    assert!(matches!(
        verify_staged_transfer(&fixture.root, &manifest).await,
        Err(TransferVerificationError::Database(_))
    ));

    fixture.remove();
}

#[cfg(unix)]
#[tokio::test]
async fn staged_transfer_rejects_symlink_objects() {
    use std::os::unix::fs::symlink;

    let fixture = TransferFixture::create("symlink").await;
    let external = unique_path("external-attachment");
    std::fs::write(&external, b"original evidence").expect("external fixture should be written");
    std::fs::remove_file(&fixture.attachment_path).expect("fixture attachment should be removed");
    symlink(&external, &fixture.attachment_path).expect("fixture symlink should be created");

    assert!(matches!(
        verify_staged_transfer(&fixture.root, &fixture.manifest).await,
        Err(TransferVerificationError::UnsafeObjectType { path })
            if path == fixture.attachment_path
    ));

    std::fs::remove_file(&fixture.attachment_path).expect("symlink should be removed");
    std::fs::remove_file(&external).expect("external fixture should be removed");
    fixture.remove_without_attachment();
}

#[cfg(unix)]
#[tokio::test]
async fn staged_transfer_rejects_symlinked_intermediate_attachment_directories() {
    use std::os::unix::fs::symlink;

    let fixture = TransferFixture::create("symlinked-attachment-directory").await;
    let external_attachments = unique_path("external-staged-attachments");
    std::fs::rename(&fixture.attachment_directory, &external_attachments)
        .expect("attachment directory should move outside staging");
    symlink(&external_attachments, &fixture.attachment_directory)
        .expect("attachment directory symlink should be created");

    assert!(matches!(
        verify_staged_transfer(&fixture.root, &fixture.manifest).await,
        Err(TransferVerificationError::UnsafeObjectType { path })
            if path == fixture.attachment_directory
    ));

    std::fs::remove_file(&fixture.attachment_directory)
        .expect("attachment directory symlink should be removed");
    std::fs::rename(&external_attachments, &fixture.attachment_directory)
        .expect("attachment directory should be restored");
    fixture.remove();
}

#[tokio::test]
async fn complete_package_loads_manifest_and_verifies_exact_layout() {
    let fixture = TransferFixture::create("complete-package").await;
    fixture.write_manifest();

    let loaded = load_and_verify_transfer_package(&fixture.root)
        .await
        .expect("complete physical package should verify");

    assert_eq!(loaded, fixture.manifest);
    fixture.remove();
}

#[tokio::test]
async fn package_manifest_rejects_missing_duplicate_and_unknown_fields() {
    let fixture = TransferFixture::create("strict-manifest-package").await;
    let canonical = fixture
        .manifest
        .to_json_bytes()
        .expect("fixture manifest should encode");
    let canonical_value: serde_json::Value =
        serde_json::from_slice(&canonical).expect("canonical manifest should parse as JSON");

    let mut missing = canonical_value.clone();
    missing
        .as_object_mut()
        .expect("manifest root should be an object")
        .remove("attachments");
    let mut unknown = canonical_value.clone();
    unknown
        .as_object_mut()
        .expect("manifest root should be an object")
        .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
    let canonical_text = String::from_utf8(canonical).expect("manifest should be UTF-8");
    let duplicate = canonical_text.replacen(
        "\"format_version\": 1,",
        "\"format_version\": 1,\n  \"format_version\": 1,",
        1,
    );

    for invalid in [
        serde_json::to_vec(&missing).expect("missing-field fixture should encode"),
        serde_json::to_vec(&unknown).expect("unknown-field fixture should encode"),
        duplicate.into_bytes(),
    ] {
        std::fs::write(fixture.manifest_path(), invalid)
            .expect("invalid package manifest should be written");
        assert!(matches!(
            load_and_verify_transfer_package(&fixture.root).await,
            Err(TransferPackageVerificationError::Manifest(_))
        ));
    }

    fixture.remove();
}

#[tokio::test]
async fn package_manifest_rejects_noncanonical_integrity_fields_and_paths() {
    let fixture = TransferFixture::create("invalid-manifest-values-package").await;
    let canonical = fixture
        .manifest
        .to_json_bytes()
        .expect("fixture manifest should encode");
    let canonical_value: serde_json::Value =
        serde_json::from_slice(&canonical).expect("canonical manifest should parse as JSON");
    let mut invalid_documents = Vec::new();

    for digest in [
        "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
        "not-a-sha256-digest",
    ] {
        let mut document = canonical_value.clone();
        document["database"]["sha256"] = serde_json::Value::String(digest.to_owned());
        invalid_documents.push(document);
    }
    for byte_length in ["01", "18446744073709551616", "-1"] {
        let mut document = canonical_value.clone();
        document["database"]["byte_length"] = serde_json::Value::String(byte_length.to_owned());
        invalid_documents.push(document);
    }
    for path in [
        "attachments/./evidence.bin",
        "attachments/../evidence.bin",
        "attachments//evidence.bin",
        "attachments\\evidence.bin",
    ] {
        let mut document = canonical_value.clone();
        document["attachments"][0]["path"] = serde_json::Value::String(path.to_owned());
        invalid_documents.push(document);
    }

    for invalid in invalid_documents {
        std::fs::write(
            fixture.manifest_path(),
            serde_json::to_vec(&invalid).expect("invalid manifest fixture should encode"),
        )
        .expect("invalid package manifest should be written");
        assert!(matches!(
            load_and_verify_transfer_package(&fixture.root).await,
            Err(TransferPackageVerificationError::Manifest(_))
        ));
    }

    fixture.remove();
}

#[cfg(unix)]
#[tokio::test]
async fn package_manifest_permission_denied_fails_closed_when_os_denies_read() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TransferFixture::create("unreadable-manifest-package").await;
    fixture.write_manifest();
    let manifest_path = fixture.manifest_path();
    std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o000))
        .expect("manifest permissions should be removable");

    match std::fs::File::open(&manifest_path) {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        Ok(_) => {
            std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o600))
                .expect("manifest permissions should be restorable");
            fixture.remove();
            return;
        }
        Err(error) => panic!("unexpected manifest open failure: {error}"),
    }

    let result = load_and_verify_transfer_package(&fixture.root).await;

    std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o600))
        .expect("manifest permissions should be restorable");
    assert!(matches!(
        result,
        Err(TransferPackageVerificationError::ReadManifest { path, source })
            if path == manifest_path && source.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert_eq!(
        std::fs::read(&fixture.attachment_path)
            .expect("package attachment should remain readable after denial"),
        b"original evidence"
    );

    fixture.remove();
}

#[tokio::test]
async fn package_round_trips_unicode_attachment_paths() {
    let mut fixture = TransferFixture::create("unicode-package").await;
    let unicode_directory = fixture.attachment_directory.join("收據");
    let unicode_attachment = unicode_directory.join("證明-2026.txt");
    std::fs::create_dir(&unicode_directory).expect("Unicode directory should be created");
    std::fs::rename(&fixture.attachment_path, &unicode_attachment)
        .expect("attachment should move to Unicode path");
    fixture.attachment_path = unicode_attachment;
    fixture.manifest = TransferManifest::new(
        fixture.manifest.database(),
        vec![ManifestAttachment::new(
            "attachments/收據/證明-2026.txt",
            describe_file(&fixture.attachment_path).expect("Unicode attachment should describe"),
        )],
    )
    .expect("Unicode attachment path should validate");
    fixture.write_manifest();

    let loaded = load_and_verify_transfer_package(&fixture.root)
        .await
        .expect("Unicode package should verify");
    assert_eq!(loaded, fixture.manifest);

    fixture.remove();
}

#[tokio::test]
async fn package_verifies_every_object_in_a_large_attachment_list() {
    const ATTACHMENT_COUNT: usize = 256;

    let mut fixture = TransferFixture::create("large-attachment-list").await;
    std::fs::remove_file(&fixture.attachment_path).expect("default attachment should be removed");
    let bulk_root = fixture.attachment_directory.join("bulk");
    std::fs::create_dir(&bulk_root).expect("bulk attachment directory should be created");
    let mut attachments = Vec::with_capacity(ATTACHMENT_COUNT);
    for index in 0..ATTACHMENT_COUNT {
        let logical_path = format!("attachments/bulk/{index:04}.bin");
        let path = fixture.root.join(&logical_path);
        std::fs::write(&path, (index as u32).to_le_bytes())
            .expect("bulk attachment should be written");
        attachments.push(ManifestAttachment::new(
            logical_path,
            describe_file(&path).expect("bulk attachment should describe"),
        ));
    }
    fixture.manifest = TransferManifest::new(fixture.manifest.database(), attachments)
        .expect("large attachment manifest should validate");
    fixture.write_manifest();

    let loaded = load_and_verify_transfer_package(&fixture.root)
        .await
        .expect("every object in the large package should verify");
    assert_eq!(loaded.attachments().len(), ATTACHMENT_COUNT);

    let final_logical_path = format!("attachments/bulk/{:04}.bin", ATTACHMENT_COUNT - 1);
    std::fs::write(fixture.root.join(&final_logical_path), [0_u8; 4])
        .expect("last attachment should be corruptible");
    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::Content(
            TransferVerificationError::Object { logical_path, .. }
        )) if logical_path == final_logical_path
    ));

    fixture.remove_without_attachment();
}

#[tokio::test]
async fn package_rejects_undeclared_top_level_and_attachment_objects() {
    let fixture = TransferFixture::create("undeclared-package").await;
    fixture.write_manifest();
    let top_level_extra = fixture.root.join("notes.txt");
    std::fs::write(&top_level_extra, b"not declared").expect("extra fixture should be written");

    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UndeclaredObject { path })
            if path == top_level_extra
    ));
    std::fs::remove_file(&top_level_extra).expect("top-level extra should be removed");

    let attachment_extra = fixture.attachment_directory.join("extra.bin");
    std::fs::write(&attachment_extra, b"not declared").expect("attachment extra should be written");
    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UndeclaredObject { path })
            if path == attachment_extra
    ));
    std::fs::remove_file(attachment_extra).expect("attachment extra should be removed");

    fixture.remove();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn package_rejects_non_utf8_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = TransferFixture::create("non-utf8-layout").await;
    fixture.write_manifest();
    let non_utf8 = fixture.root.join(OsString::from_vec(vec![0xff, 0xfe]));
    std::fs::write(&non_utf8, b"undeclared").expect("non-UTF-8 package entry should be creatable");

    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UndeclaredObject { path })
            if path == non_utf8
    ));

    std::fs::remove_file(non_utf8).expect("non-UTF-8 package entry should be removed");
    fixture.remove();
}

#[cfg(unix)]
#[tokio::test]
async fn package_rejects_symlinked_manifest_and_attachment_directory() {
    use std::os::unix::fs::symlink;

    let fixture = TransferFixture::create("package-symlinks").await;
    let external_manifest = unique_path("external-manifest.json");
    std::fs::write(
        &external_manifest,
        fixture
            .manifest
            .to_json_bytes()
            .expect("manifest should encode"),
    )
    .expect("external manifest should be written");
    symlink(&external_manifest, fixture.manifest_path())
        .expect("manifest symlink should be created");

    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UnsafeObjectType { path })
            if path == fixture.manifest_path()
    ));
    std::fs::remove_file(fixture.manifest_path()).expect("manifest symlink should be removed");
    std::fs::remove_file(external_manifest).expect("external manifest should be removed");

    fixture.write_manifest();
    let external_attachments = unique_path("external-attachments");
    std::fs::rename(&fixture.attachment_directory, &external_attachments)
        .expect("attachment directory should move outside package");
    symlink(&external_attachments, &fixture.attachment_directory)
        .expect("attachment directory symlink should be created");
    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UnsafeObjectType { path })
            if path == fixture.attachment_directory
    ));

    std::fs::remove_file(&fixture.attachment_directory)
        .expect("attachment directory symlink should be removed");
    std::fs::rename(&external_attachments, &fixture.attachment_directory)
        .expect("attachment directory should be restored");
    fixture.remove();
}

#[cfg(unix)]
#[tokio::test]
async fn package_rejects_special_files_in_attachment_layout() {
    let fixture = TransferFixture::create("special-file-layout").await;
    fixture.write_manifest();
    let special_path = fixture.attachment_directory.join("control.pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&special_path)
        .status()
        .expect("mkfifo should run for the isolated package fixture");
    assert!(status.success(), "named pipe fixture should be creatable");

    assert!(matches!(
        load_and_verify_transfer_package(&fixture.root).await,
        Err(TransferPackageVerificationError::UnsafeObjectType { path })
            if path == special_path
    ));
    assert_eq!(
        std::fs::read(&fixture.attachment_path)
            .expect("declared attachment should remain readable after layout rejection"),
        b"original evidence"
    );

    std::fs::remove_file(special_path).expect("named pipe fixture should be removed");
    fixture.remove();
}

#[tokio::test]
async fn assembly_copies_verified_objects_and_builds_a_complete_private_package() {
    let fixture = TransferFixture::create("assemble-package").await;
    let work_root = unique_path(".private-package-work");

    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("verified staging should assemble as a private package");

    assert!(
        fixture.database_path.exists() && fixture.attachment_path.exists(),
        "assembly must preserve every source object"
    );
    let loaded = load_and_verify_transfer_package(&work_root)
        .await
        .expect("assembled package should remain independently verifiable");
    assert_eq!(loaded, fixture.manifest);
    assert_eq!(
        std::fs::read(work_root.join("attachments/evidence.bin"))
            .expect("assembled attachment should be readable"),
        b"original evidence"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&work_root)
                .expect("private package metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(work_root.join(TRANSFER_MANIFEST_FILE_NAME))
                .expect("manifest metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    std::fs::remove_dir_all(work_root).expect("private package work should be removed");
    fixture.remove();
}

#[tokio::test]
async fn assembly_rejects_bad_source_before_creating_work() {
    let fixture = TransferFixture::create("assemble-bad-source").await;
    let work_root = unique_path(".private-bad-source-work");
    std::fs::write(&fixture.attachment_path, b"corrupted evidence")
        .expect("source attachment should be corruptible");

    assert!(matches!(
        assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest).await,
        Err(TransferPackageAssemblyError::Source(
            TransferVerificationError::Object { .. }
        ))
    ));
    assert!(
        !work_root.exists(),
        "invalid source must not create private package work"
    );

    fixture.remove();
}

#[tokio::test]
async fn assembly_never_reuses_or_changes_an_existing_work_path() {
    let fixture = TransferFixture::create("assemble-existing-work").await;
    let work_root = unique_path(".private-existing-work");
    std::fs::create_dir(&work_root).expect("pre-existing work path should be created");
    let sentinel = work_root.join("sentinel");
    std::fs::write(&sentinel, b"keep me").expect("sentinel should be written");

    assert!(matches!(
        assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest).await,
        Err(TransferPackageAssemblyError::WorkPathExists(path)) if path == work_root
    ));
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel should remain readable"),
        b"keep me"
    );

    std::fs::remove_file(sentinel).expect("sentinel should be removed");
    std::fs::remove_dir(work_root).expect("pre-existing work path should be removed");
    fixture.remove();
}

#[tokio::test]
async fn assembly_rejects_work_nested_inside_source_without_modifying_source() {
    let fixture = TransferFixture::create("assemble-nested-work").await;
    let work_root = fixture.root.join(".private-package-work");

    assert!(matches!(
        assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest).await,
        Err(TransferPackageAssemblyError::WorkPathInsideSource {
            source_root,
            work_root: rejected,
        }) if source_root == fixture.root && rejected == work_root
    ));
    assert!(
        !work_root.exists(),
        "rejected work must not add an undeclared object to source staging"
    );
    verify_staged_transfer(&fixture.root, &fixture.manifest)
        .await
        .expect("rejected nested work must leave source staging unchanged");

    fixture.remove();
}

#[tokio::test]
async fn assembly_reports_a_missing_work_parent_without_creating_paths() {
    let fixture = TransferFixture::create("assemble-missing-parent").await;
    let missing_parent = unique_path("missing-package-parent");
    let work_root = missing_parent.join(".private-package-work");

    assert!(matches!(
        assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest).await,
        Err(TransferPackageAssemblyError::MissingParent(path)) if path == missing_parent
    ));
    assert!(
        !missing_parent.exists(),
        "assembly must not create a missing caller-selected parent"
    );

    fixture.remove();
}

#[tokio::test]
async fn publication_syncs_moves_and_reverifies_a_complete_package() {
    let mut fixture = TransferFixture::create("publish-package").await;
    let nested_directory = fixture.attachment_directory.join("receipts/2026");
    let nested_attachment = nested_directory.join("evidence.bin");
    std::fs::create_dir_all(&nested_directory).expect("nested attachment path should be created");
    std::fs::rename(&fixture.attachment_path, &nested_attachment)
        .expect("attachment should move into nested path");
    fixture.attachment_path = nested_attachment;
    fixture.manifest = TransferManifest::new(
        fixture.manifest.database(),
        vec![ManifestAttachment::new(
            "attachments/receipts/2026/evidence.bin",
            describe_file(&fixture.attachment_path).expect("nested attachment should describe"),
        )],
    )
    .expect("nested manifest should validate");
    let publication_root = unique_path("publish-package-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");

    let published = publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("complete package should publish durably and verify");

    assert_eq!(published.destination(), destination);
    assert_eq!(published.manifest(), &fixture.manifest);
    assert_eq!(published.state(), TransferOperationState::Verified);
    assert!(
        !work_root.exists(),
        "atomic publication must move private work"
    );
    assert!(
        fixture.database_path.exists() && fixture.attachment_path.exists(),
        "publication must preserve the original staging source"
    );
    load_and_verify_transfer_package(&destination)
        .await
        .expect("published package should remain independently verifiable");

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn publication_preserves_work_and_existing_destination_on_conflict() {
    let fixture = TransferFixture::create("publish-conflict").await;
    let publication_root = unique_path("publish-conflict-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let sentinel = destination.join("sentinel");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    std::fs::create_dir(&destination).expect("existing destination should be created");
    std::fs::write(&sentinel, b"keep me").expect("destination sentinel should be written");

    assert!(matches!(
        publish_private_transfer_package(&work_root, &destination).await,
        Err(TransferPackagePublishError::Publication(
            LocalDirectoryPublishError::DestinationExists(path)
        )) if path == destination
    ));
    assert!(
        work_root.exists(),
        "publication conflict must preserve private work"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel should remain readable"),
        b"keep me"
    );

    std::fs::remove_dir_all(work_root).expect("private work should be removed");
    std::fs::remove_dir_all(destination).expect("existing destination should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn publication_rejects_corrupt_private_work_before_creating_destination() {
    let fixture = TransferFixture::create("publish-corrupt-work").await;
    let publication_root = unique_path("publish-corrupt-work-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    std::fs::write(
        work_root.join("attachments/evidence.bin"),
        b"corrupted evidence",
    )
    .expect("private attachment should be corruptible");

    assert!(matches!(
        publish_private_transfer_package(&work_root, &destination).await,
        Err(TransferPackagePublishError::PrivateVerification(_))
    ));
    assert!(
        work_root.exists(),
        "failed private work must remain available"
    );
    assert!(
        !destination.exists(),
        "invalid private work must never receive a public name"
    );

    std::fs::remove_dir_all(work_root).expect("private work should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn recovery_reverifies_published_work_before_advancing_to_verified() {
    let fixture = TransferFixture::create("recover-published").await;
    let publication_root = unique_path("recover-published-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let state_root = unique_path("recover-published-state");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("package fixture should publish");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");

    let recovered = recover_export_work_state(&state_root)
        .await
        .expect("published recovery should reverify and complete");

    assert!(matches!(recovered, TransferRecoveryOutcome::Verified(_)));
    assert_eq!(recovered.destination(), destination);
    assert_eq!(recovered.state(), TransferOperationState::Verified);
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("recovered state should load")
            .state(),
        TransferOperationState::Verified
    );
    assert!(matches!(
        recover_export_work_state(&state_root)
            .await
            .expect("verified recovery should be idempotent"),
        TransferRecoveryOutcome::Verified(_)
    ));

    std::fs::write(
        destination.join("attachments/evidence.bin"),
        b"damaged after verified recovery",
    )
    .expect("verified package fixture should be damageable");
    assert!(matches!(
        recover_export_work_state(&state_root).await,
        Err(TransferRecoveryError::PublishedVerification { destination: path, .. })
            if path == destination
    ));
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("terminal recovery state should remain readable")
            .state(),
        TransferOperationState::Verified,
        "terminal durable state must never be downgraded after later package damage"
    );

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn recovery_keeps_damaged_published_work_retryable_and_preserves_it() {
    let fixture = TransferFixture::create("recover-damaged").await;
    let publication_root = unique_path("recover-damaged-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let state_root = unique_path("recover-damaged-state");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("package fixture should publish");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");
    let damaged = destination.join("attachments/evidence.bin");
    std::fs::write(&damaged, b"damaged after publication")
        .expect("published fixture should be damageable");

    assert!(matches!(
        recover_export_work_state(&state_root).await,
        Err(TransferRecoveryError::PublishedVerification { destination: path, .. })
            if path == destination
    ));
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("failed verification state should remain readable")
            .state(),
        TransferOperationState::PublishedUnverified,
        "a possibly transient verification error must remain retryable"
    );
    assert!(state_root.exists() && destination.exists());

    std::fs::remove_dir_all(destination).expect("damaged package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn recovery_requires_destination_parent_synchronization_before_verification() {
    let state_root = unique_path("recover-missing-parent-state");
    let missing_parent = unique_path("recover-missing-parent");
    let destination = missing_parent.join("book.kagextra");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");

    assert!(matches!(
        recover_export_work_state(&state_root).await,
        Err(TransferRecoveryError::PublishedParentSynchronization {
            destination: path,
            parent,
            ..
        }) if path == destination && parent == missing_parent
    ));
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("failed durability recovery should remain readable")
            .state(),
        TransferOperationState::PublishedUnverified,
        "failed parent synchronization must never advance durable state"
    );

    std::fs::remove_dir_all(state_root).expect("state root should be removed");
}

#[tokio::test]
async fn recovery_preserves_published_work_when_a_crash_lock_remains() {
    let fixture = TransferFixture::create("recover-crash-lock").await;
    let publication_root = unique_path("recover-crash-lock-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let state_root = unique_path("recover-crash-lock-state");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("package fixture should publish");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");
    let lock_path = state_root.join(".operation-state.lock");
    std::fs::write(&lock_path, b"crash residue").expect("crash lock fixture should be written");

    assert!(matches!(
        recover_export_work_state(&state_root).await,
        Err(TransferRecoveryError::VerifiedState { source, .. })
            if matches!(&*source, TransferWorkStateError::UpdateLocked(path) if path == &lock_path)
    ));
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("locked state should remain readable")
            .state(),
        TransferOperationState::PublishedUnverified
    );
    load_and_verify_transfer_package(&destination)
        .await
        .expect("crash lock must not damage the published package");
    assert!(lock_path.exists(), "recovery must preserve crash evidence");

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn recovery_tolerates_and_preserves_a_crash_temporary_state_file() {
    let fixture = TransferFixture::create("recover-crash-temporary-state").await;
    let publication_root = unique_path("recover-crash-temporary-state-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let state_root = unique_path("recover-crash-temporary-state");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("package fixture should publish");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");
    let temporary_path = state_root.join(".operation-state-4242-123456789-0.tmp");
    std::fs::write(&temporary_path, b"partial state from interrupted writer")
        .expect("crash temporary file fixture should be written");

    let recovered = recover_export_work_state(&state_root)
        .await
        .expect("unambiguous published recovery should ignore unrelated crash residue");

    assert!(matches!(recovered, TransferRecoveryOutcome::Verified(_)));
    assert_eq!(recovered.destination(), destination);
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("recovered state should remain readable")
            .state(),
        TransferOperationState::Verified
    );
    assert_eq!(
        std::fs::read(&temporary_path).expect("crash evidence should remain readable"),
        b"partial state from interrupted writer",
        "recovery must not silently delete ambiguous crash evidence"
    );

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn concurrent_recovery_converges_on_one_verified_terminal_state() {
    let fixture = TransferFixture::create("concurrent-recovery").await;
    let publication_root = unique_path("concurrent-recovery-root");
    let work_root = publication_root.join(".private-package-work");
    let destination = publication_root.join("book.kagextra");
    let state_root = unique_path("concurrent-recovery-state");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    assemble_private_transfer_package(&fixture.root, &work_root, &fixture.manifest)
        .await
        .expect("private package should assemble");
    publish_private_transfer_package(&work_root, &destination)
        .await
        .expect("package fixture should publish");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )
    .expect("fixture should persist the post-rename boundary");

    let (left, right) = tokio::join!(
        recover_export_work_state(&state_root),
        recover_export_work_state(&state_root),
    );

    let mut verified = 0;
    for outcome in [left, right] {
        match outcome {
            Ok(TransferRecoveryOutcome::Verified(package)) => {
                verified += 1;
                assert_eq!(package.destination(), destination);
            }
            Err(TransferRecoveryError::VerifiedState { source, .. })
                if matches!(
                    &*source,
                    TransferWorkStateError::UpdateLocked(_)
                        | TransferWorkStateError::StaleState {
                            expected: TransferOperationState::PublishedUnverified,
                            actual: TransferOperationState::Verified,
                        }
                ) => {}
            other => panic!("unexpected concurrent recovery outcome: {other:?}"),
        }
    }
    assert!(verified >= 1, "at least one recovery caller must complete");
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("concurrent recovery state should remain readable")
            .state(),
        TransferOperationState::Verified
    );
    assert!(matches!(
        recover_export_work_state(&state_root)
            .await
            .expect("a retry after concurrent recovery should be idempotent"),
        TransferRecoveryOutcome::Verified(_)
    ));
    assert_eq!(
        std::fs::read_dir(&state_root)
            .expect("state root should remain readable")
            .count(),
        1,
        "successful competing recovery must not leave a lock or temporary state file"
    );

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn recovery_classifies_failed_state_without_touching_a_competing_destination() {
    let state_root = unique_path("recover-failed-state");
    let destination = unique_path("recover-failed-destination.kagextra");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    transition_transfer_work_state(
        &state_root,
        TransferOperationState::Working,
        TransferOperationState::Failed,
    )
    .expect("working state should become failed");
    std::fs::create_dir(&destination).expect("competing destination should be created");
    let sentinel = destination.join("sentinel");
    std::fs::write(&sentinel, b"unrelated destination")
        .expect("competing destination sentinel should be written");

    let recovered = recover_export_work_state(&state_root)
        .await
        .expect("failed state should classify without destination inspection");
    assert!(matches!(recovered, TransferRecoveryOutcome::Failed { .. }));
    assert_eq!(recovered.state(), TransferOperationState::Failed);
    assert_eq!(
        std::fs::read(&sentinel).expect("competing destination should remain readable"),
        b"unrelated destination"
    );

    std::fs::remove_dir_all(destination).expect("competing destination should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
}

#[tokio::test]
async fn recovery_never_adopts_a_destination_that_appears_while_working() {
    let state_root = unique_path("recover-working-state");
    let destination = unique_path("recover-working-destination.kagextra");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");

    let private = recover_export_work_state(&state_root)
        .await
        .expect("absent destination should classify as private work");
    assert!(matches!(
        private,
        TransferRecoveryOutcome::PrivateWorking { .. }
    ));

    std::fs::create_dir(&destination).expect("racing destination should be created");
    std::fs::write(destination.join("sentinel"), b"not this operation")
        .expect("racing destination sentinel should be written");
    let collided = recover_export_work_state(&state_root)
        .await
        .expect("working collision should be classified without adoption");
    assert!(matches!(
        collided,
        TransferRecoveryOutcome::DestinationPresentWhileWorking { .. }
    ));
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("working state should remain readable")
            .state(),
        TransferOperationState::Working
    );
    assert_eq!(
        std::fs::read(destination.join("sentinel"))
            .expect("racing destination should remain untouched"),
        b"not this operation"
    );

    std::fs::remove_dir_all(destination).expect("racing destination should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_never_follows_a_working_destination_symlink() {
    use std::os::unix::fs::symlink;

    let state_root = unique_path("recover-working-symlink-state");
    let destination = unique_path("recover-working-symlink-destination.kagextra");
    let external = unique_path("recover-working-symlink-external");
    let sentinel = external.join("sentinel");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("working recovery state should be created");
    std::fs::create_dir(&external).expect("external destination should be created");
    std::fs::write(&sentinel, b"outside recovery ownership")
        .expect("external sentinel should be written");
    symlink(&external, &destination).expect("racing destination symlink should be created");

    let recovered = recover_export_work_state(&state_root)
        .await
        .expect("working destination symlink should be classified without adoption");

    assert!(matches!(
        recovered,
        TransferRecoveryOutcome::DestinationPresentWhileWorking { .. }
    ));
    assert_eq!(recovered.destination(), destination);
    assert_eq!(
        load_transfer_work_state(&state_root)
            .expect("working state should remain readable")
            .state(),
        TransferOperationState::Working
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("external sentinel should remain readable"),
        b"outside recovery ownership"
    );
    assert!(
        std::fs::symlink_metadata(&destination)
            .expect("destination symlink should remain inspectable")
            .file_type()
            .is_symlink(),
        "recovery must preserve the competing symlink as evidence"
    );

    std::fs::remove_file(destination).expect("destination symlink should be removed");
    std::fs::remove_dir_all(external).expect("external destination should be removed");
    std::fs::remove_dir_all(state_root).expect("state root should be removed");
}

#[tokio::test]
async fn working_recovery_is_read_only_idempotent_and_preserves_unicode_destination() {
    let state_root = unique_path("recover-working-unicode-state");
    let destination_parent = unique_path("recover-working-unicode-destination");
    let destination = destination_parent.join("帳簿-📒-2026.kagextra");
    create_private_test_directory(&state_root);
    create_export_work_state(&state_root, &destination)
        .expect("Unicode recovery state should be created");
    let state_path = state_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let partial_database = state_root.join(STAGED_DATABASE_PATH);
    let interrupted_update = state_root.join(".operation-state-4242-123456789-0.tmp");
    std::fs::write(&partial_database, b"partial snapshot")
        .expect("partial snapshot crash fixture should be written");
    std::fs::write(&interrupted_update, b"partial state update")
        .expect("interrupted state update fixture should be written");
    let state_before = std::fs::read(&state_path).expect("working state bytes should be readable");
    let mut entries_before = std::fs::read_dir(&state_root)
        .expect("state root should be readable")
        .map(|entry| entry.expect("state entry should be readable").path())
        .collect::<Vec<_>>();
    entries_before.sort();

    for _ in 0..3 {
        let recovered = recover_export_work_state(&state_root)
            .await
            .expect("absent Unicode destination should remain private work");
        assert!(matches!(
            recovered,
            TransferRecoveryOutcome::PrivateWorking { .. }
        ));
        assert_eq!(recovered.destination(), destination);
        assert_eq!(recovered.state(), TransferOperationState::Working);
    }

    assert_eq!(
        std::fs::read(&state_path).expect("working state bytes should remain readable"),
        state_before,
        "read-only recovery must not rewrite a working state record"
    );
    assert_eq!(
        {
            let mut entries = std::fs::read_dir(&state_root)
                .expect("state root should remain readable")
                .map(|entry| entry.expect("state entry should be readable").path())
                .collect::<Vec<_>>();
            entries.sort();
            entries
        },
        entries_before,
        "idempotent classification must neither create nor delete crash evidence"
    );

    std::fs::remove_dir_all(state_root).expect("state root should be removed");
}

#[tokio::test]
async fn staged_export_runs_the_complete_safe_publication_workflow() {
    let fixture = TransferFixture::create("orchestrated-export").await;
    let publication_root = unique_path("orchestrated-export-root");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");

    let published = export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest)
        .await
        .expect("verified staging should export through one safe workflow");

    assert_eq!(published.destination(), destination);
    assert_eq!(published.manifest(), &fixture.manifest);
    assert_eq!(published.state(), TransferOperationState::Verified);
    assert!(
        fixture.database_path.exists() && fixture.attachment_path.exists(),
        "export must preserve every source object"
    );
    assert_eq!(
        private_export_work_paths(&publication_root),
        Vec::<PathBuf>::new(),
        "successful atomic publication must not leave private work behind"
    );

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn retrying_a_successful_staged_export_is_non_destructive() {
    let fixture = TransferFixture::create("orchestrated-idempotent-retry").await;
    let publication_root = unique_path("orchestrated-idempotent-retry-root");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");

    export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest)
        .await
        .expect("first export should publish and verify");
    let manifest_before = std::fs::read(destination.join(TRANSFER_MANIFEST_FILE_NAME))
        .expect("published manifest should be readable");
    let attachment_before = std::fs::read(destination.join("attachments/evidence.bin"))
        .expect("published attachment should be readable");

    assert!(matches!(
        export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest).await,
        Err(TransferExportError::DestinationExists(path)) if path == destination
    ));
    assert_eq!(
        std::fs::read(destination.join(TRANSFER_MANIFEST_FILE_NAME))
            .expect("published manifest should remain readable"),
        manifest_before
    );
    assert_eq!(
        std::fs::read(destination.join("attachments/evidence.bin"))
            .expect("published attachment should remain readable"),
        attachment_before
    );
    assert_eq!(
        private_export_work_paths(&publication_root),
        Vec::<PathBuf>::new(),
        "known retry conflicts must not allocate private export work"
    );
    load_and_verify_transfer_package(&destination)
        .await
        .expect("original verified package should survive the retry");

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn staged_export_rejects_an_existing_destination_before_creating_work() {
    let fixture = TransferFixture::create("orchestrated-conflict").await;
    let publication_root = unique_path("orchestrated-conflict-root");
    let destination = publication_root.join("book.kagextra");
    let sentinel = destination.join("sentinel");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    std::fs::create_dir(&destination).expect("existing destination should be created");
    std::fs::write(&sentinel, b"keep me").expect("destination sentinel should be written");

    assert!(matches!(
        export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest).await,
        Err(TransferExportError::DestinationExists(path)) if path == destination
    ));
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel should remain readable"),
        b"keep me"
    );
    assert_eq!(
        private_export_work_paths(&publication_root),
        Vec::<PathBuf>::new(),
        "known destination conflicts must be rejected before private assembly"
    );

    std::fs::remove_dir_all(destination).expect("existing destination should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn staged_export_rejects_a_destination_inside_source_staging() {
    let fixture = TransferFixture::create("orchestrated-nested-destination").await;
    let destination = fixture.root.join("nested/book.kagextra");
    std::fs::create_dir(fixture.root.join("nested"))
        .expect("nested destination parent fixture should be created");

    let error = export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest)
        .await
        .expect_err("destination inside source must be rejected");
    assert!(matches!(
        error,
        TransferExportError::Assembly { source, .. }
            if matches!(*source, TransferPackageAssemblyError::WorkPathInsideSource { .. })
    ));
    assert!(!destination.exists());
    assert_eq!(
        private_export_work_paths(&fixture.root.join("nested")),
        Vec::<PathBuf>::new(),
        "rejected nested destination must not create private package work"
    );

    std::fs::remove_dir(fixture.root.join("nested"))
        .expect("nested destination parent should be removed");
    fixture.remove();
}

#[tokio::test]
async fn staged_export_rejects_invalid_source_without_publishing() {
    let fixture = TransferFixture::create("orchestrated-invalid-source").await;
    let publication_root = unique_path("orchestrated-invalid-source-root");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");
    std::fs::write(&fixture.attachment_path, b"corrupted evidence")
        .expect("source attachment should be corruptible");

    let error = export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest)
        .await
        .expect_err("corrupt source must fail export");
    assert!(matches!(
        error,
        TransferExportError::Assembly { source, .. }
            if matches!(
                *source,
                TransferPackageAssemblyError::Source(
                    TransferVerificationError::Object { .. }
                )
            )
    ));
    assert!(
        !destination.exists(),
        "invalid source must never be published"
    );
    assert_eq!(
        private_export_work_paths(&publication_root),
        Vec::<PathBuf>::new(),
        "source verification must happen before private assembly"
    );

    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn concurrent_staged_exports_have_one_verified_winner() {
    let fixture = TransferFixture::create("orchestrated-concurrent").await;
    let publication_root = unique_path("orchestrated-concurrent-root");
    let destination = publication_root.join("book.kagextra");
    std::fs::create_dir(&publication_root).expect("publication root should be created");

    let (left, right) = tokio::join!(
        export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest),
        export_staged_transfer_package(&fixture.root, &destination, &fixture.manifest),
    );
    let mut successes = 0;
    let mut conflicts = 0;
    for outcome in [left, right] {
        match outcome {
            Ok(published) => {
                successes += 1;
                assert_eq!(published.destination(), destination);
                assert_eq!(published.state(), TransferOperationState::Verified);
            }
            Err(TransferExportError::Publication {
                work_root, source, ..
            }) if matches!(
                *source,
                TransferPackagePublishError::Publication(
                    LocalDirectoryPublishError::DestinationExists(_)
                )
            ) =>
            {
                conflicts += 1;
                assert!(
                    work_root.exists(),
                    "losing private work must remain explicit after a publication race"
                );
                std::fs::remove_dir_all(work_root)
                    .expect("losing private work should be removable by the test");
            }
            other => panic!("unexpected concurrent export outcome: {other:?}"),
        }
    }

    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
    load_and_verify_transfer_package(&destination)
        .await
        .expect("the winning public package should verify");
    assert!(
        fixture.database_path.exists() && fixture.attachment_path.exists(),
        "concurrent exports must preserve every source object"
    );

    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(publication_root).expect("publication root should be removed");
    fixture.remove();
}

#[tokio::test]
async fn live_objects_prepare_one_verified_private_staging_tree() {
    let source_database_path = unique_path("prepare-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let source_attachment = unique_path("prepare-source-attachment.bin");
    std::fs::write(&source_attachment, b"immutable attachment bytes")
        .expect("source attachment should be written");
    let expected = describe_file(&source_attachment).expect("source attachment should describe");
    let selected = [TransferAttachmentSource::new(
        &source_attachment,
        "attachments/receipts/2026/evidence.bin",
        expected,
    )];
    let staging_root = unique_path("prepared-staging");

    let prepared = prepare_transfer_staging(&source_pool, &staging_root, &selected)
        .await
        .expect("live objects should prepare as verified private staging");

    assert_eq!(prepared.root(), staging_root);
    assert_eq!(prepared.manifest().attachments().len(), 1);
    assert!(prepared.cleanup_warnings().is_empty());
    assert_eq!(
        std::fs::read(staging_root.join("attachments/receipts/2026/evidence.bin"))
            .expect("staged attachment should be readable"),
        b"immutable attachment bytes"
    );
    std::fs::write(&source_attachment, b"changed after snapshot preparation")
        .expect("source attachment should remain independently mutable");
    sqlx::query("CREATE TABLE after_snapshot_marker (id INTEGER PRIMARY KEY) STRICT")
        .execute(&source_pool)
        .await
        .expect("isolated source database should remain independently mutable");
    verify_staged_transfer(&staging_root, prepared.manifest())
        .await
        .expect("prepared staging should not depend on later source changes");
    assert_eq!(
        std::fs::read(staging_root.join("attachments/receipts/2026/evidence.bin"))
            .expect("staged attachment should remain readable"),
        b"immutable attachment bytes",
        "staging must retain the bytes selected at snapshot preparation"
    );
    assert!(
        source_attachment.exists(),
        "staging preparation must preserve source attachments"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(source_attachment).expect("source attachment should be removed");
    std::fs::remove_dir_all(staging_root).expect("private staging should be removed");
}

#[tokio::test]
async fn staging_preparation_rejects_changed_attachment_bytes() {
    let source_database_path = unique_path("prepare-changed-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let source_attachment = unique_path("prepare-changed-attachment.bin");
    std::fs::write(&source_attachment, b"original bytes")
        .expect("source attachment should be written");
    let expected = describe_file(&source_attachment).expect("source attachment should describe");
    std::fs::write(&source_attachment, b"changed bytes")
        .expect("source attachment should be changed");
    let selected = [TransferAttachmentSource::new(
        &source_attachment,
        "attachments/evidence.bin",
        expected,
    )];
    let staging_root = unique_path("prepare-changed-staging");

    assert!(matches!(
        prepare_transfer_staging(&source_pool, &staging_root, &selected).await,
        Err(TransferStagingError::AttachmentCopy { logical_path, .. })
            if logical_path == "attachments/evidence.bin"
    ));
    assert!(
        staging_root.join(STAGED_DATABASE_PATH).exists(),
        "verified database work should remain private after attachment failure"
    );
    assert!(
        !staging_root.join("attachments/evidence.bin").exists(),
        "mismatched attachment must not receive its staged destination name"
    );
    assert_eq!(
        std::fs::read(&source_attachment).expect("source attachment should remain readable"),
        b"changed bytes"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(source_attachment).expect("source attachment should be removed");
    std::fs::remove_dir_all(staging_root).expect("failed private staging should be removed");
}

#[tokio::test]
async fn staging_preparation_rejects_unsafe_attachment_paths_without_escape() {
    let source_database_path = unique_path("prepare-path-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let source_attachment = unique_path("prepare-path-attachment.bin");
    std::fs::write(&source_attachment, b"attachment bytes")
        .expect("source attachment should be written");
    let expected = describe_file(&source_attachment).expect("source attachment should describe");
    let selected = [TransferAttachmentSource::new(
        &source_attachment,
        "attachments/../escaped.bin",
        expected,
    )];
    let staging_root = unique_path("prepare-unsafe-path-staging");
    let escaped = staging_root
        .parent()
        .expect("staging should have a parent")
        .join("escaped.bin");

    assert!(matches!(
        prepare_transfer_staging(&source_pool, &staging_root, &selected).await,
        Err(TransferStagingError::Manifest(_))
    ));
    assert!(
        !escaped.exists(),
        "unsafe logical path must never escape staging"
    );
    assert!(
        source_attachment.exists(),
        "invalid manifest must preserve its source attachment"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(source_attachment).expect("source attachment should be removed");
    std::fs::remove_dir_all(staging_root).expect("failed private staging should be removed");
}

#[tokio::test]
async fn staging_preparation_never_reuses_an_existing_path() {
    let source_database_path = unique_path("prepare-existing-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let staging_root = unique_path("prepare-existing-staging");
    let sentinel = staging_root.join("sentinel");
    std::fs::create_dir(&staging_root).expect("existing staging should be created");
    std::fs::write(&sentinel, b"keep me").expect("sentinel should be written");

    assert!(matches!(
        prepare_transfer_staging(&source_pool, &staging_root, &[]).await,
        Err(TransferStagingError::StagingPathExists(path)) if path == staging_root
    ));
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel should remain readable"),
        b"keep me"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(sentinel).expect("sentinel should be removed");
    std::fs::remove_dir(staging_root).expect("existing staging should be removed");
}

#[tokio::test]
async fn live_book_export_prepares_publishes_verifies_and_cleans_local_staging() {
    let source_database_path = unique_path("live-export-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let source_attachment = unique_path("live-export-attachment.bin");
    std::fs::write(&source_attachment, b"live export attachment")
        .expect("source attachment should be written");
    let selected = [TransferAttachmentSource::new(
        &source_attachment,
        "attachments/evidence.bin",
        describe_file(&source_attachment).expect("source attachment should describe"),
    )];
    let local_work_parent = unique_path("live-export-local-work");
    let destination_parent = unique_path("live-export-destination");
    let destination = destination_parent.join("book.kagextra");
    std::fs::create_dir(&local_work_parent).expect("local work parent should be created");
    std::fs::create_dir(&destination_parent).expect("destination parent should be created");

    let outcome =
        export_live_book_snapshot(&source_pool, &local_work_parent, &selected, &destination)
            .await
            .expect("live objects should export through the complete workflow");

    assert_eq!(outcome.package().destination(), destination);
    assert_eq!(outcome.package().state(), TransferOperationState::Verified);
    assert!(outcome.preparation_cleanup_warnings().is_empty());
    assert!(outcome.staging_cleanup_warning().is_none());
    assert_eq!(
        private_snapshot_work_paths(&local_work_parent),
        Vec::<PathBuf>::new(),
        "verified export should remove its local staging"
    );
    load_and_verify_transfer_package(&destination)
        .await
        .expect("published package should independently verify");
    assert!(
        source_database_path.exists() && source_attachment.exists(),
        "complete export must preserve all live sources"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(source_attachment).expect("source attachment should be removed");
    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(destination_parent).expect("destination parent should be removed");
    std::fs::remove_dir(local_work_parent).expect("local work parent should be removed");
}

#[tokio::test]
async fn live_book_export_rejects_known_destination_before_snapshot_work() {
    let source_database_path = unique_path("live-conflict-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let local_work_parent = unique_path("live-conflict-local-work");
    let destination_parent = unique_path("live-conflict-destination");
    let destination = destination_parent.join("book.kagextra");
    let sentinel = destination.join("sentinel");
    std::fs::create_dir(&local_work_parent).expect("local work parent should be created");
    std::fs::create_dir(&destination_parent).expect("destination parent should be created");
    std::fs::create_dir(&destination).expect("existing destination should be created");
    std::fs::write(&sentinel, b"keep me").expect("sentinel should be written");

    assert!(matches!(
        export_live_book_snapshot(&source_pool, &local_work_parent, &[], &destination).await,
        Err(LiveBookExportError::Destination(
            TransferExportError::DestinationExists(path)
        )) if path == destination
    ));
    assert_eq!(
        private_snapshot_work_paths(&local_work_parent),
        Vec::<PathBuf>::new(),
        "known destination conflict must be rejected before SQLite snapshot work"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel should remain readable"),
        b"keep me"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_dir_all(destination).expect("existing destination should be removed");
    std::fs::remove_dir(destination_parent).expect("destination parent should be removed");
    std::fs::remove_dir(local_work_parent).expect("local work parent should be removed");
}

#[tokio::test]
async fn live_book_export_rejects_destination_inside_private_work_parent() {
    let source_database_path = unique_path("live-overlap-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let local_work_parent = unique_path("live-overlap-work");
    let destination = local_work_parent.join("book.kagextra");
    std::fs::create_dir(&local_work_parent).expect("local work parent should be created");

    assert!(matches!(
        export_live_book_snapshot(&source_pool, &local_work_parent, &[], &destination).await,
        Err(LiveBookExportError::DestinationInsideWorkParent {
            work_parent,
            destination: rejected,
        }) if work_parent == local_work_parent && rejected == destination
    ));
    assert_eq!(
        private_snapshot_work_paths(&local_work_parent),
        Vec::<PathBuf>::new(),
        "overlapping destination must be rejected before snapshot work"
    );
    assert!(!destination.exists());

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_dir(local_work_parent).expect("local work parent should be removed");
}

#[tokio::test]
async fn live_book_export_preserves_private_staging_after_preparation_failure() {
    let source_database_path = unique_path("live-failure-source.sqlite3");
    let source_pool = open_database(&source_database_path)
        .await
        .expect("isolated source database should open");
    let source_attachment = unique_path("live-failure-attachment.bin");
    std::fs::write(&source_attachment, b"original bytes")
        .expect("source attachment should be written");
    let expected = describe_file(&source_attachment).expect("source attachment should describe");
    std::fs::write(&source_attachment, b"changed bytes")
        .expect("source attachment should be changed");
    let selected = [TransferAttachmentSource::new(
        &source_attachment,
        "attachments/evidence.bin",
        expected,
    )];
    let local_work_parent = unique_path("live-failure-local-work");
    let destination_parent = unique_path("live-failure-destination");
    let destination = destination_parent.join("book.kagextra");
    std::fs::create_dir(&local_work_parent).expect("local work parent should be created");
    std::fs::create_dir(&destination_parent).expect("destination parent should be created");

    let error =
        export_live_book_snapshot(&source_pool, &local_work_parent, &selected, &destination)
            .await
            .expect_err("changed attachment must fail the live export");
    let staging_root = match error {
        LiveBookExportError::Preparation {
            staging_root,
            source,
        } if matches!(*source, TransferStagingError::AttachmentCopy { .. }) => staging_root,
        other => panic!("unexpected live export failure: {other:?}"),
    };
    assert!(
        staging_root.exists(),
        "failed local staging must remain available for diagnosis"
    );
    assert!(
        staging_root.join(STAGED_DATABASE_PATH).exists(),
        "successful SQLite snapshot work must not be hidden by a later attachment failure"
    );
    let persisted = load_transfer_work_state(&staging_root)
        .expect("failed preparation should retain its durable recovery record");
    assert_eq!(persisted.destination(), destination);
    assert_eq!(persisted.state(), TransferOperationState::Working);
    assert!(staging_root.join(TRANSFER_WORK_STATE_FILE_NAME).is_file());
    assert!(!destination.exists(), "failed preparation must not publish");
    assert_eq!(
        std::fs::read(&source_attachment).expect("source attachment should remain readable"),
        b"changed bytes"
    );

    source_pool.close().await;
    remove_sqlite_files(&source_database_path);
    std::fs::remove_file(source_attachment).expect("source attachment should be removed");
    std::fs::remove_dir_all(staging_root).expect("failed staging should be removed by the test");
    std::fs::remove_dir(destination_parent).expect("destination parent should be removed");
    std::fs::remove_dir(local_work_parent).expect("local work parent should be removed");
}

struct TransferFixture {
    root: PathBuf,
    source_database_path: PathBuf,
    database_path: PathBuf,
    attachment_directory: PathBuf,
    attachment_path: PathBuf,
    manifest: TransferManifest,
}

impl TransferFixture {
    async fn create(label: &str) -> Self {
        let root = unique_path(&format!("transfer-{label}"));
        let attachment_directory = root.join("attachments");
        std::fs::create_dir(&root).expect("staging root should be created");
        std::fs::create_dir(&attachment_directory).expect("attachment directory should be created");

        let source_database_path = unique_path(&format!("transfer-source-{label}.sqlite3"));
        let source_pool = open_database(&source_database_path)
            .await
            .expect("isolated source database should open");
        let database_path = root.join(STAGED_DATABASE_PATH);
        backup_database(&source_pool, &database_path)
            .await
            .expect("database snapshot should publish");
        source_pool.close().await;

        let attachment_path = attachment_directory.join("evidence.bin");
        std::fs::write(&attachment_path, b"original evidence")
            .expect("attachment fixture should be written");
        let manifest = TransferManifest::new(
            describe_file(&database_path).expect("database should describe"),
            vec![ManifestAttachment::new(
                "attachments/evidence.bin",
                describe_file(&attachment_path).expect("attachment should describe"),
            )],
        )
        .expect("fixture manifest should validate");

        Self {
            root,
            source_database_path,
            database_path,
            attachment_directory,
            attachment_path,
            manifest,
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join(TRANSFER_MANIFEST_FILE_NAME)
    }

    fn write_manifest(&self) {
        std::fs::write(
            self.manifest_path(),
            self.manifest
                .to_json_bytes()
                .expect("fixture manifest should encode"),
        )
        .expect("fixture manifest should be written");
    }

    fn remove(self) {
        if self.attachment_path.exists() {
            std::fs::remove_file(&self.attachment_path).expect("attachment should be removed");
        }
        self.remove_without_attachment();
    }

    fn remove_without_attachment(self) {
        remove_sqlite_files(&self.source_database_path);
        let manifest_path = self.manifest_path();
        if manifest_path.exists() {
            std::fs::remove_file(manifest_path).expect("manifest should be removed");
        }
        std::fs::remove_file(&self.database_path).expect("snapshot should be removed");
        std::fs::remove_dir_all(&self.attachment_directory)
            .expect("attachment directory should be removed");
        std::fs::remove_dir(&self.root).expect("staging root should be removed");
    }
}

fn unique_path(label: &str) -> PathBuf {
    let temporary_root = std::fs::canonicalize(std::env::temp_dir())
        .expect("system temporary directory should resolve to a real path");
    temporary_root.join(format!(
        "kAccounting-Extra-01-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos()
    ))
}

fn create_private_test_directory(path: &Path) {
    std::fs::create_dir(path).expect("private test directory should be created");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("private test directory should be owner-only");
    }
}

fn private_export_work_paths(parent: &Path) -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(parent)
        .expect("publication root should be readable")
        .map(|entry| entry.expect("publication entry should be readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(".kAccounting-Extra-01-export-") && name.ends_with(".work")
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn private_snapshot_work_paths(parent: &Path) -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(parent)
        .expect("local work parent should be readable")
        .map(|entry| entry.expect("local work entry should be readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(".kAccounting-Extra-01-snapshot-") && name.ends_with(".work")
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn remove_sqlite_files(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
        PathBuf::from(format!("{}-journal", path.display())),
    ] {
        if candidate.exists() {
            std::fs::remove_file(candidate).expect("isolated SQLite file should be removed");
        }
    }
}
