use std::io::{Cursor, Read};
use std::str::FromStr;

use kaccounting_backend::storage::{
    CONTENT_DIGEST_ALGORITHM, CURRENT_TRANSFER_MANIFEST_VERSION, ContentDigest,
    ContentDigestParseError, LocalDirectoryPublishError, LocalObjectPublishError,
    ManifestAttachment, StoredObjectDescriptor, StoredObjectVerificationError,
    TRANSFER_DATABASE_PATH, TRANSFER_MANIFEST_FILE_NAME, TransferManifest, TransferManifestError,
    TransferManifestJsonError, copy_and_describe, describe_file, describe_reader,
    publish_private_directory_noreplace, publish_verified_local_copy, verify_file,
};

#[test]
fn descriptor_uses_stable_sha256_and_exact_byte_length() {
    let mut bytes = Cursor::new(b"abc");
    let descriptor = describe_reader(&mut bytes).expect("in-memory bytes should hash");

    assert_eq!(CONTENT_DIGEST_ALGORITHM, "sha256");
    assert_eq!(descriptor.byte_length(), 3);
    assert_eq!(
        descriptor.digest().to_string(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn descriptor_streams_large_content_in_bounded_reads() {
    let bytes = vec![0x5a; 256 * 1024 + 17];
    let mut reader = BoundedReader::new(Cursor::new(bytes), 64 * 1024);

    let descriptor = describe_reader(&mut reader).expect("bounded reader should hash");

    assert_eq!(descriptor.byte_length(), 256 * 1024 + 17);
    assert!(reader.read_calls > 1, "large input must be streamed");
}

#[test]
fn copy_stream_produces_exact_bytes_and_descriptor() {
    let source = b"copied attachment bytes";
    let mut reader = Cursor::new(source);
    let mut output = Vec::new();

    let copied = copy_and_describe(&mut reader, &mut output).expect("copy should succeed");
    let expected = describe_reader(&mut Cursor::new(source)).expect("source should describe");

    assert_eq!(output, source);
    assert_eq!(copied, expected);
}

#[test]
fn copy_stream_propagates_interrupted_reads_without_claiming_success() {
    let mut reader = FailingReader {
        first_chunk: Some(b"partial".to_vec()),
    };
    let mut output = Vec::new();

    let error = copy_and_describe(&mut reader, &mut output)
        .expect_err("injected source interruption must fail");

    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(output, b"partial");
}

#[test]
fn copy_stream_propagates_capacity_style_write_failures() {
    let mut reader = Cursor::new(b"larger than capacity");
    let mut writer = CapacityWriter {
        remaining: 4,
        bytes: Vec::new(),
    };

    let error = copy_and_describe(&mut reader, &mut writer)
        .expect_err("injected capacity exhaustion must fail");

    assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(writer.bytes, b"larg");
}

#[test]
fn copy_stream_propagates_final_flush_failure_without_returning_a_descriptor() {
    let mut reader = Cursor::new(b"fully written but not durably accepted");
    let mut writer = FlushFailingWriter { bytes: Vec::new() };

    let error = copy_and_describe(&mut reader, &mut writer)
        .expect_err("a final flush failure must prevent descriptor success");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(writer.bytes, b"fully written but not durably accepted");
}

#[test]
fn digest_parser_round_trips_and_normalizes_uppercase() {
    let uppercase = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
    let parsed = ContentDigest::from_str(uppercase).expect("uppercase hex should be accepted");

    assert_eq!(
        parsed.to_string(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn digest_parser_rejects_wrong_length_and_non_hex_input() {
    assert_eq!(
        ContentDigest::from_str("00").unwrap_err(),
        ContentDigestParseError::WrongLength { actual: 2 }
    );
    let invalid = format!("{}g", "0".repeat(63));
    assert_eq!(
        ContentDigest::from_str(&invalid).unwrap_err(),
        ContentDigestParseError::InvalidHex { index: 63 }
    );
}

#[test]
fn verifier_accepts_exact_content_and_rejects_same_length_corruption() {
    let path = unique_storage_path("verify-corruption");
    std::fs::write(&path, b"attachment evidence").expect("fixture should be written");
    let expected = describe_file(&path).expect("fixture should be described");

    verify_file(&path, expected).expect("unchanged bytes should verify");
    std::fs::write(&path, b"attachment evidencf").expect("fixture should be corrupted");

    assert!(matches!(
        verify_file(&path, expected),
        Err(StoredObjectVerificationError::DigestMismatch { .. })
    ));
    std::fs::remove_file(path).expect("fixture should be removed");
}

#[test]
fn verifier_rejects_truncation_before_digest_comparison() {
    let path = unique_storage_path("verify-truncation");
    std::fs::write(&path, b"complete attachment").expect("fixture should be written");
    let expected = describe_file(&path).expect("fixture should be described");
    std::fs::write(&path, b"short").expect("fixture should be truncated");

    assert!(matches!(
        verify_file(&path, expected),
        Err(StoredObjectVerificationError::ByteLengthMismatch {
            expected: 19,
            actual: 5,
            ..
        })
    ));
    std::fs::remove_file(path).expect("fixture should be removed");
}

#[test]
fn verifier_rejects_a_missing_object_without_creating_it() {
    let path = unique_storage_path("verify-missing");
    let digest =
        ContentDigest::from_str("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .expect("known empty digest should parse");
    let expected = StoredObjectDescriptor::new(digest, 0);

    assert!(matches!(
        verify_file(&path, expected),
        Err(StoredObjectVerificationError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound
    ));
    assert!(
        !path.exists(),
        "verification must not create a missing object"
    );
}

#[cfg(unix)]
#[test]
fn descriptor_and_verifier_reject_symlinks_without_trusting_their_targets() {
    use std::os::unix::fs::symlink;

    let root = unique_storage_path("verify-symlink");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let target = root.join("target");
    let link = root.join("link");
    std::fs::write(&target, b"target bytes").expect("target should be written");
    let expected = describe_file(&target).expect("regular target should describe");
    symlink(&target, &link).expect("symlink should be created");

    let describe_error = describe_file(&link).expect_err("describing a symlink must fail closed");
    assert_eq!(describe_error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(matches!(
        verify_file(&link, expected),
        Err(StoredObjectVerificationError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::InvalidInput
    ));

    std::fs::remove_file(link).expect("symlink should be removed");
    std::fs::remove_file(target).expect("target should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn descriptor_rejects_directories_and_device_special_files_without_opening_them() {
    let root = unique_storage_path("verify-special-file");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let directory = root.join("directory");
    let destination = root.join("destination");
    std::fs::create_dir(&directory).expect("fixture directory should be created");

    for path in [&directory, std::path::Path::new("/dev/null")] {
        let error = describe_file(path).expect_err("special files must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(matches!(
            verify_file(path, empty_descriptor()),
            Err(StoredObjectVerificationError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            publish_verified_local_copy(path, &destination, empty_descriptor()),
            Err(LocalObjectPublishError::UnsafeSource(rejected)) if rejected == path
        ));
        assert!(!destination.exists());
    }

    std::fs::remove_dir(directory).expect("fixture directory should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn verifier_detects_each_independent_descriptor_mutation() {
    let path = unique_storage_path("verify-mutated-descriptor");
    std::fs::write(&path, b"stable bytes").expect("fixture should be written");
    let actual = describe_file(&path).expect("fixture should describe");
    let other_digest = describe_reader(&mut Cursor::new(b"other bytes"))
        .expect("alternate bytes should describe")
        .digest();

    assert!(matches!(
        verify_file(
            &path,
            StoredObjectDescriptor::new(actual.digest(), actual.byte_length() + 1),
        ),
        Err(StoredObjectVerificationError::ByteLengthMismatch {
            expected: 13,
            actual: 12,
            ..
        })
    ));
    assert!(matches!(
        verify_file(
            &path,
            StoredObjectDescriptor::new(other_digest, actual.byte_length()),
        ),
        Err(StoredObjectVerificationError::DigestMismatch {
            expected,
            actual: observed,
            ..
        }) if expected == other_digest && observed == actual.digest()
    ));

    std::fs::remove_file(path).expect("fixture should be removed");
}

#[cfg(unix)]
#[test]
fn descriptor_rejects_path_replacement_while_content_is_read() {
    const SOURCE_LENGTH: u64 = 16 * 1024 * 1024;

    let root = unique_storage_path("describe-path-replacement");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let replacement = root.join("replacement");
    let source_file = std::fs::File::create(&source).expect("sparse source should be created");
    source_file
        .set_len(SOURCE_LENGTH)
        .expect("sparse source should be sized");
    drop(source_file);
    std::fs::write(&replacement, b"replacement inode").expect("replacement should be written");
    let read_started = std::sync::atomic::AtomicBool::new(false);

    let result = std::thread::scope(|scope| {
        let source_ref = &source;
        let started = &read_started;
        let descriptor_task = scope.spawn(move || {
            started.store(true, std::sync::atomic::Ordering::Release);
            describe_file(source_ref)
        });
        while !read_started.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::yield_now();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::rename(&replacement, &source)
            .expect("replacement should atomically take the source name");
        descriptor_task
            .join()
            .expect("descriptor task should not panic")
    });

    let error = result.expect_err("a changed pathname must not return a descriptor");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        error
            .to_string()
            .contains("changed while it was being read"),
        "post-read identity check should reject the replacement: {error}"
    );

    std::fs::remove_file(source).expect("replacement should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn manifest_is_versioned_and_orders_safe_attachment_paths() {
    let empty = empty_descriptor();
    let manifest = TransferManifest::new(
        empty,
        vec![
            ManifestAttachment::new("attachments/z-last", empty),
            ManifestAttachment::new("attachments/a-first", empty),
        ],
    )
    .expect("safe manifest should validate");

    assert_eq!(manifest.format_version(), CURRENT_TRANSFER_MANIFEST_VERSION);
    assert_eq!(manifest.database(), empty);
    assert_eq!(
        manifest
            .attachments()
            .iter()
            .map(ManifestAttachment::relative_path)
            .collect::<Vec<_>>(),
        vec!["attachments/a-first", "attachments/z-last"]
    );
}

#[test]
fn manifest_rejects_unsupported_versions_before_file_access() {
    let error = TransferManifest::from_version(2, empty_descriptor(), Vec::new())
        .expect_err("unknown manifest version must fail closed");

    assert_eq!(
        error,
        TransferManifestError::UnsupportedVersion { actual: 2 }
    );
}

#[test]
fn manifest_rejects_unsafe_and_duplicate_attachment_paths() {
    let descriptor = empty_descriptor();
    for path in [
        "",
        "attachment",
        "attachments",
        "../attachments/item",
        "/attachments/item",
        "attachments/../item",
        "attachments\\item",
        "attachments/./item",
        "attachments//item",
        "attachments/item/",
        "attachments/\0item",
    ] {
        assert!(matches!(
            TransferManifest::new(descriptor, vec![ManifestAttachment::new(path, descriptor)]),
            Err(TransferManifestError::UnsafePath { .. })
        ));
    }

    assert_eq!(
        TransferManifest::new(
            descriptor,
            vec![
                ManifestAttachment::new("attachments/same", descriptor),
                ManifestAttachment::new("attachments/same", descriptor),
            ],
        )
        .unwrap_err(),
        TransferManifestError::DuplicatePath {
            path: "attachments/same".to_owned()
        }
    );
}

#[test]
fn manifest_json_round_trips_deterministically_with_exact_string_lengths() {
    let empty = empty_descriptor();
    let huge = StoredObjectDescriptor::new(empty.digest(), u64::MAX);
    let manifest = TransferManifest::new(
        huge,
        vec![
            ManifestAttachment::new("attachments/z-last", huge),
            ManifestAttachment::new("attachments/a-first", empty),
        ],
    )
    .expect("logical manifest should validate");

    let encoded = manifest.to_json_bytes().expect("manifest should encode");
    let document: serde_json::Value =
        serde_json::from_slice(&encoded).expect("encoded bytes should be JSON");

    assert_eq!(TRANSFER_MANIFEST_FILE_NAME, "manifest.json");
    assert_eq!(TRANSFER_DATABASE_PATH, "book.sqlite3");
    assert_eq!(
        document["database"]["byte_length"],
        serde_json::Value::String(u64::MAX.to_string())
    );
    assert_eq!(document["attachments"][0]["path"], "attachments/a-first");
    assert!(encoded.ends_with(b"\n"));

    let decoded = TransferManifest::from_json_bytes(&encoded).expect("manifest should decode");
    assert_eq!(decoded, manifest);
    assert_eq!(
        decoded.to_json_bytes().expect("manifest should re-encode"),
        encoded
    );
}

#[test]
fn manifest_json_decoding_canonicalizes_field_order_whitespace_and_attachment_order() {
    let input = br#"{
        "attachments": [
            {"byte_length":"0","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","path":"attachments/z-last"},
            {"path":"attachments/a-first","byte_length":"0","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}
        ],
        "database": {"byte_length":"0","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","path":"book.sqlite3"},
        "format_version": 1
    }"#;
    let expected = concat!(
        "{\n",
        "  \"format_version\": 1,\n",
        "  \"database\": {\n",
        "    \"path\": \"book.sqlite3\",\n",
        "    \"sha256\": \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\n",
        "    \"byte_length\": \"0\"\n",
        "  },\n",
        "  \"attachments\": [\n",
        "    {\n",
        "      \"path\": \"attachments/a-first\",\n",
        "      \"sha256\": \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\n",
        "      \"byte_length\": \"0\"\n",
        "    },\n",
        "    {\n",
        "      \"path\": \"attachments/z-last\",\n",
        "      \"sha256\": \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\n",
        "      \"byte_length\": \"0\"\n",
        "    }\n",
        "  ]\n",
        "}\n",
    );

    let decoded = TransferManifest::from_json_bytes(input)
        .expect("equivalent non-canonical JSON should decode");
    assert_eq!(
        decoded
            .attachments()
            .iter()
            .map(ManifestAttachment::relative_path)
            .collect::<Vec<_>>(),
        vec!["attachments/a-first", "attachments/z-last"]
    );
    assert_eq!(decoded.to_json_bytes().unwrap(), expected.as_bytes());
}

#[test]
fn manifest_json_rejects_unknown_fields_and_wrong_database_path() {
    let unknown = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0"}"#,
        r#"[],"unexpected":true"#,
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(unknown.as_bytes()),
        Err(TransferManifestJsonError::Json(_))
    ));

    let nested_unknown = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0","bytes":0}"#,
        "[]",
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(nested_unknown.as_bytes()),
        Err(TransferManifestJsonError::Json(_))
    ));

    let duplicate_field = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0","byte_length":"0"}"#,
        "[]",
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(duplicate_field.as_bytes()),
        Err(TransferManifestJsonError::Json(_))
    ));

    let wrong_path = manifest_json_with(
        r#"{"path":"other.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0"}"#,
        "[]",
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(wrong_path.as_bytes()),
        Err(TransferManifestJsonError::UnexpectedDatabasePath { actual })
            if actual == "other.sqlite3"
    ));
}

#[test]
fn manifest_json_rejects_noncanonical_digest_and_byte_length() {
    let uppercase_digest = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855","byte_length":"0"}"#,
        "[]",
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(uppercase_digest.as_bytes()),
        Err(TransferManifestJsonError::NonCanonicalDigest { .. })
    ));

    let leading_zero_length = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"01"}"#,
        "[]",
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(leading_zero_length.as_bytes()),
        Err(TransferManifestJsonError::InvalidByteLength { .. })
    ));

    for value in ["", "+1", "-1", " 1", "1 ", "18446744073709551616"] {
        let invalid_length = manifest_json_with(
            &format!(
                r#"{{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"{value}"}}"#
            ),
            "[]",
        );
        assert!(matches!(
            TransferManifest::from_json_bytes(invalid_length.as_bytes()),
            Err(TransferManifestJsonError::InvalidByteLength { .. })
        ));
    }

    let noncanonical_attachment_digest = manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0"}"#,
        r#"[{"path":"attachments/item","sha256":"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855","byte_length":"0"}]"#,
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(noncanonical_attachment_digest.as_bytes()),
        Err(TransferManifestJsonError::NonCanonicalDigest { path, .. })
            if path == "attachments/item"
    ));
}

#[test]
fn manifest_json_reapplies_version_and_attachment_path_validation() {
    let unsupported = manifest_json_with_version(2, "[]");
    assert!(matches!(
        TransferManifest::from_json_bytes(unsupported.as_bytes()),
        Err(TransferManifestJsonError::Manifest(
            TransferManifestError::UnsupportedVersion { actual: 2 }
        ))
    ));

    let unsafe_attachment = manifest_json_with_version(
        CURRENT_TRANSFER_MANIFEST_VERSION,
        r#"[{"path":"attachments/../escape","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0"}]"#,
    );
    assert!(matches!(
        TransferManifest::from_json_bytes(unsafe_attachment.as_bytes()),
        Err(TransferManifestJsonError::Manifest(
            TransferManifestError::UnsafePath { .. }
        ))
    ));
}

fn manifest_json_with(database: &str, attachments_and_tail: &str) -> String {
    format!(r#"{{"format_version":1,"database":{database},"attachments":{attachments_and_tail}}}"#)
}

fn manifest_json_with_version(version: u32, attachments: &str) -> String {
    manifest_json_with(
        r#"{"path":"book.sqlite3","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","byte_length":"0"}"#,
        attachments,
    )
    .replacen(r#""format_version":1"#, &format!(r#""format_version":{version}"#), 1)
}

fn empty_descriptor() -> StoredObjectDescriptor {
    let digest =
        ContentDigest::from_str("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .expect("known empty digest should parse");
    StoredObjectDescriptor::new(digest, 0)
}

fn unique_storage_path(label: &str) -> std::path::PathBuf {
    let temporary_root = std::fs::canonicalize(std::env::temp_dir())
        .expect("temporary root should resolve to a real directory");
    temporary_root.join(format!(
        "kAccounting-Extra-01-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos()
    ))
}

#[test]
fn local_publish_copies_verifies_and_never_overwrites() {
    let root = unique_storage_path("local-publish");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let destination = root.join("destination");
    std::fs::write(&source, b"verified local object").expect("source should be written");
    let expected = describe_file(&source).expect("source should describe");

    let outcome = publish_verified_local_copy(&source, &destination, expected)
        .expect("verified local copy should publish");
    assert!(outcome.cleanup_warning().is_none());
    assert_eq!(
        std::fs::read(&destination).expect("destination should be readable"),
        b"verified local object"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        assert_eq!(
            std::fs::metadata(&destination)
                .expect("destination metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "published private objects must remain owner-only"
        );
        assert_ne!(
            std::fs::metadata(&source)
                .expect("source metadata should be readable")
                .ino(),
            std::fs::metadata(&destination)
                .expect("destination metadata should be readable")
                .ino(),
            "publication must copy source bytes instead of linking live source storage"
        );
        assert_eq!(
            std::fs::metadata(&destination)
                .expect("destination metadata should be readable")
                .nlink(),
            1,
            "successful cleanup must remove the private work-file hard link"
        );
    }
    verify_file(&destination, expected).expect("published object should verify");

    std::fs::write(&source, b"replacement attempt").expect("source should change");
    let replacement = describe_file(&source).expect("replacement should describe");
    assert!(matches!(
        publish_verified_local_copy(&source, &destination, replacement),
        Err(LocalObjectPublishError::DestinationExists(path)) if path == destination
    ));
    assert_eq!(
        std::fs::read(&destination).expect("destination should remain readable"),
        b"verified local object"
    );

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_file(destination).expect("destination should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn local_publish_rejects_descriptor_mismatch_without_destination() {
    let root = unique_storage_path("local-mismatch");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let destination = root.join("destination");
    std::fs::write(&source, b"actual bytes").expect("source should be written");
    let expected = empty_descriptor();

    assert!(matches!(
        publish_verified_local_copy(&source, &destination, expected),
        Err(LocalObjectPublishError::Verification(_))
    ));
    assert!(!destination.exists());
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        1,
        "failed work should be cleaned while preserving the source"
    );

    let actual = describe_file(&source).expect("unchanged source should describe for retry");
    let retry = publish_verified_local_copy(&source, &destination, actual)
        .expect("a clean retry with the right descriptor should publish");
    assert!(retry.cleanup_warning().is_none());
    verify_file(&destination, actual).expect("retried destination should verify");
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        2,
        "successful retry must leave only source and destination"
    );

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_file(destination).expect("destination should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn local_publish_rejects_the_source_path_as_its_destination_without_changes() {
    let root = unique_storage_path("local-same-path");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let original = b"must remain untouched";
    std::fs::write(&source, original).expect("source should be written");
    let expected = describe_file(&source).expect("source should describe");

    assert!(matches!(
        publish_verified_local_copy(&source, &source, expected),
        Err(LocalObjectPublishError::DestinationExists(path)) if path == source
    ));
    assert_eq!(
        std::fs::read(&source).expect("source should remain readable"),
        original
    );
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        1,
        "same-path rejection must occur before private work is created"
    );

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn local_publish_rejects_an_existing_hard_link_alias_without_changes() {
    use std::os::unix::fs::MetadataExt;

    let root = unique_storage_path("local-hard-link-alias");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let alias = root.join("alias");
    let original = b"one inode";
    std::fs::write(&source, original).expect("source should be written");
    std::fs::hard_link(&source, &alias).expect("fixture alias should be linked");
    let expected = describe_file(&source).expect("source should describe");

    assert!(matches!(
        publish_verified_local_copy(&source, &alias, expected),
        Err(LocalObjectPublishError::DestinationExists(path)) if path == alias
    ));
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read(&alias).unwrap(), original);
    assert_eq!(
        std::fs::metadata(&source).unwrap().ino(),
        std::fs::metadata(&alias).unwrap().ino()
    );
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        2,
        "alias rejection must not create private work"
    );

    std::fs::remove_file(alias).expect("alias should be removed");
    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn local_publish_rejects_source_changed_after_description_without_destination() {
    let root = unique_storage_path("local-changed-source");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let destination = root.join("destination");
    std::fs::write(&source, b"original bytes").expect("source should be written");
    let expected = describe_file(&source).expect("original source should describe");
    std::fs::write(&source, b"modified bytes").expect("source should change at the same length");

    assert!(matches!(
        publish_verified_local_copy(&source, &destination, expected),
        Err(LocalObjectPublishError::Verification(
            StoredObjectVerificationError::DigestMismatch { .. }
        ))
    ));
    assert!(!destination.exists());
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        1,
        "failed changed-source work must be cleaned"
    );

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn failed_local_publish_cleanup_preserves_unrelated_private_work() {
    let root = unique_storage_path("local-cleanup-scope");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let destination = root.join("destination");
    let unrelated = root.join(".kAccounting-Extra-01-object-unrelated");
    std::fs::write(&source, b"changed bytes").expect("source should be written");
    std::fs::create_dir(&unrelated).expect("unrelated work should be created");
    std::fs::write(unrelated.join("recovery-evidence"), b"preserve")
        .expect("recovery evidence should be written");

    assert!(matches!(
        publish_verified_local_copy(&source, &destination, empty_descriptor()),
        Err(LocalObjectPublishError::Verification(_))
    ));
    assert!(!destination.exists());
    assert_eq!(
        std::fs::read(unrelated.join("recovery-evidence"))
            .expect("unrelated recovery evidence should remain readable"),
        b"preserve"
    );
    let entries = std::fs::read_dir(&root)
        .expect("fixture root should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2, "only source and unrelated work remain");
    assert!(entries.contains(&source));
    assert!(entries.contains(&unrelated));

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_dir_all(unrelated).expect("unrelated work should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn successful_local_publish_reports_cleanup_residue_without_losing_the_destination() {
    const SOURCE_LENGTH: u64 = 4 * 1024 * 1024;

    let root = unique_storage_path("local-cleanup-warning");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let destination = root.join("destination");
    let source_file = std::fs::File::create(&source).expect("sparse source should be created");
    source_file
        .set_len(SOURCE_LENGTH)
        .expect("sparse source should be sized");
    drop(source_file);
    let expected = describe_file(&source).expect("source should describe");
    let publication_finished = std::sync::atomic::AtomicBool::new(false);
    let observer_ready = std::sync::Barrier::new(2);

    let (outcome, residue_directory) = std::thread::scope(|scope| {
        let finished = &publication_finished;
        let root_ref = &root;
        let ready = &observer_ready;
        let residue_task = scope.spawn(move || {
            ready.wait();
            loop {
                let work = std::fs::read_dir(root_ref)
                    .expect("fixture root should remain readable")
                    .filter_map(Result::ok)
                    .find_map(|entry| {
                        let name = entry.file_name();
                        (name
                            .to_string_lossy()
                            .starts_with(".kAccounting-Extra-01-object-"))
                        .then(|| entry.path())
                    });
                if let Some(work) = work {
                    std::fs::write(work.join("recovery-evidence"), b"preserve")
                        .expect("cleanup residue should be injected");
                    break Some(work);
                }
                if finished.load(std::sync::atomic::Ordering::Acquire) {
                    break None;
                }
                std::thread::yield_now();
            }
        });

        observer_ready.wait();
        let outcome = publish_verified_local_copy(&source, &destination, expected);
        publication_finished.store(true, std::sync::atomic::Ordering::Release);
        let residue_directory = residue_task
            .join()
            .expect("residue observer should not panic");
        (outcome, residue_directory)
    });

    let outcome = outcome.expect("verified destination should still be successful");
    let warning = outcome
        .cleanup_warning()
        .expect("non-empty private work should produce a cleanup warning");
    let residue_directory =
        residue_directory.expect("observer should find private work before publication finishes");
    assert_eq!(warning.operation, "removing private object directory");
    assert_eq!(warning.path, residue_directory);
    assert_eq!(warning.source.kind(), std::io::ErrorKind::DirectoryNotEmpty);
    verify_file(&destination, expected).expect("published destination should remain verified");
    assert_eq!(
        std::fs::read(residue_directory.join("recovery-evidence"))
            .expect("cleanup residue should remain available"),
        b"preserve"
    );
    assert!(!residue_directory.join("object").exists());

    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_file(destination).expect("destination should be removed");
    std::fs::remove_dir_all(residue_directory).expect("cleanup residue should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn concurrent_local_publishers_cannot_overwrite_each_other() {
    let root = unique_storage_path("local-concurrent");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source_a = root.join("source-a");
    let source_b = root.join("source-b");
    let destination = root.join("destination");
    std::fs::write(&source_a, b"publisher a").expect("source A should be written");
    std::fs::write(&source_b, b"publisher b").expect("source B should be written");
    let descriptor_a = describe_file(&source_a).expect("source A should describe");
    let descriptor_b = describe_file(&source_b).expect("source B should describe");

    let destination_a = destination.clone();
    let destination_b = destination.clone();
    let source_a_task = source_a.clone();
    let source_b_task = source_b.clone();
    let barrier = std::sync::Barrier::new(2);
    let (result_a, result_b) = std::thread::scope(|scope| {
        let barrier_ref = &barrier;
        let task_a = scope.spawn(move || {
            barrier_ref.wait();
            publish_verified_local_copy(&source_a_task, &destination_a, descriptor_a)
        });
        let barrier_ref = &barrier;
        let task_b = scope.spawn(move || {
            barrier_ref.wait();
            publish_verified_local_copy(&source_b_task, &destination_b, descriptor_b)
        });
        (
            task_a.join().expect("publisher A should not panic"),
            task_b.join().expect("publisher B should not panic"),
        )
    });

    let success_count = usize::from(result_a.is_ok()) + usize::from(result_b.is_ok());
    let conflict_count = usize::from(matches!(
        result_a,
        Err(LocalObjectPublishError::DestinationExists(_))
    )) + usize::from(matches!(
        result_b,
        Err(LocalObjectPublishError::DestinationExists(_))
    ));
    assert_eq!(success_count, 1);
    assert_eq!(conflict_count, 1);

    let published = std::fs::read(&destination).expect("winner should be readable");
    assert!(published == b"publisher a" || published == b"publisher b");
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("fixture root should be readable")
            .count(),
        3,
        "both private work directories should be removed"
    );

    std::fs::remove_file(source_a).expect("source A should be removed");
    std::fs::remove_file(source_b).expect("source B should be removed");
    std::fs::remove_file(destination).expect("destination should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn concurrent_local_publications_use_distinct_private_work_and_clean_up() {
    const PUBLISHER_COUNT: usize = 16;

    let root = unique_storage_path("local-concurrent-distinct");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let mut fixtures = Vec::with_capacity(PUBLISHER_COUNT);
    for index in 0..PUBLISHER_COUNT {
        let source = root.join(format!("source-{index}"));
        let destination = root.join(format!("destination-{index}"));
        std::fs::write(&source, format!("publisher {index}")).expect("source should be written");
        let expected = describe_file(&source).expect("source should describe");
        fixtures.push((source, destination, expected));
    }

    let barrier = std::sync::Barrier::new(PUBLISHER_COUNT);
    let results = std::thread::scope(|scope| {
        let handles = fixtures
            .iter()
            .map(|(source, destination, expected)| {
                let barrier_ref = &barrier;
                scope.spawn(move || {
                    barrier_ref.wait();
                    publish_verified_local_copy(source, destination, *expected)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("publisher should not panic"))
            .collect::<Vec<_>>()
    });

    assert!(results.iter().all(Result::is_ok));
    let remaining = std::fs::read_dir(&root)
        .expect("fixture root should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").file_name())
        .collect::<Vec<_>>();
    assert_eq!(remaining.len(), PUBLISHER_COUNT * 2);
    assert!(remaining.iter().all(|name| {
        !name
            .to_string_lossy()
            .starts_with(".kAccounting-Extra-01-object-")
    }));

    for (source, destination, expected) in fixtures {
        verify_file(&destination, expected).expect("destination should verify");
        std::fs::remove_file(source).expect("source should be removed");
        std::fs::remove_file(destination).expect("destination should be removed");
    }
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn local_publish_rejects_symlink_sources() {
    use std::os::unix::fs::symlink;

    let root = unique_storage_path("local-symlink");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let real_source = root.join("real-source");
    let source_link = root.join("source-link");
    let destination = root.join("destination");
    std::fs::write(&real_source, b"real bytes").expect("source should be written");
    symlink(&real_source, &source_link).expect("source symlink should be created");
    let expected = describe_file(&real_source).expect("source should describe");

    assert!(matches!(
        publish_verified_local_copy(&source_link, &destination, expected),
        Err(LocalObjectPublishError::UnsafeSource(path)) if path == source_link
    ));
    assert!(!destination.exists());

    std::fs::remove_file(source_link).expect("symlink should be removed");
    std::fs::remove_file(real_source).expect("source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn local_publish_does_not_follow_destination_or_parent_symlinks() {
    use std::os::unix::fs::symlink;

    let root = unique_storage_path("local-destination-symlink");
    std::fs::create_dir(&root).expect("fixture root should be created");
    let source = root.join("source");
    let target = root.join("target");
    let destination_link = root.join("destination-link");
    std::fs::write(&source, b"new bytes").expect("source should be written");
    std::fs::write(&target, b"preserve target").expect("target should be written");
    symlink(&target, &destination_link).expect("destination symlink should be created");
    let expected = describe_file(&source).expect("source should describe");

    assert!(matches!(
        publish_verified_local_copy(&source, &destination_link, expected),
        Err(LocalObjectPublishError::DestinationExists(path)) if path == destination_link
    ));
    assert_eq!(
        std::fs::read(&target).expect("target should remain readable"),
        b"preserve target"
    );

    let real_parent = root.join("real-parent");
    let parent_link = root.join("parent-link");
    std::fs::create_dir(&real_parent).expect("real parent should be created");
    symlink(&real_parent, &parent_link).expect("parent symlink should be created");
    let nested_destination = parent_link.join("object");
    assert!(matches!(
        publish_verified_local_copy(&source, &nested_destination, expected),
        Err(LocalObjectPublishError::MissingParent(path)) if path == parent_link
    ));
    assert!(!real_parent.join("object").exists());

    std::fs::remove_file(destination_link).expect("destination symlink should be removed");
    std::fs::remove_file(parent_link).expect("parent symlink should be removed");
    std::fs::remove_dir(real_parent).expect("real parent should be removed");
    std::fs::remove_file(source).expect("source should be removed");
    std::fs::remove_file(target).expect("target should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn directory_publish_is_atomic_and_never_replaces_an_existing_destination() {
    let root = unique_storage_path("directory-publish");
    let source = root.join(".private-work");
    let destination = root.join("book.kagextra");
    std::fs::create_dir(&root).expect("fixture root should be created");
    std::fs::create_dir(&source).expect("private source should be created");
    std::fs::write(source.join("complete"), b"first package")
        .expect("source content should be written");

    publish_private_directory_noreplace(&source, &destination)
        .expect("complete private directory should publish");
    assert!(
        !source.exists(),
        "successful publication moves private work"
    );
    assert_eq!(
        std::fs::read(destination.join("complete")).expect("published content should be readable"),
        b"first package"
    );

    std::fs::create_dir(&source).expect("second private source should be created");
    std::fs::write(source.join("complete"), b"replacement attempt")
        .expect("second source content should be written");
    assert!(matches!(
        publish_private_directory_noreplace(&source, &destination),
        Err(LocalDirectoryPublishError::DestinationExists(path)) if path == destination
    ));
    assert!(source.exists(), "failed private work must remain available");
    assert_eq!(
        std::fs::read(destination.join("complete")).expect("destination should remain readable"),
        b"first package"
    );

    std::fs::remove_dir_all(source).expect("second private source should be removed");
    std::fs::remove_dir_all(destination).expect("published package should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn directory_publish_rejects_the_source_path_as_its_destination() {
    let root = unique_storage_path("directory-same-path");
    let source = root.join(".private-work");
    std::fs::create_dir(&root).expect("fixture root should be created");
    std::fs::create_dir(&source).expect("private source should be created");
    std::fs::write(source.join("complete"), b"complete package")
        .expect("source content should be written");

    assert!(matches!(
        publish_private_directory_noreplace(&source, &source),
        Err(LocalDirectoryPublishError::DestinationExists(path)) if path == source
    ));
    assert_eq!(
        std::fs::read(source.join("complete")).expect("source should remain readable"),
        b"complete package"
    );

    std::fs::remove_dir_all(source).expect("private source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[test]
fn concurrent_directory_publishers_have_exactly_one_winner() {
    let root = unique_storage_path("directory-concurrent");
    let source_a = root.join(".private-a");
    let source_b = root.join(".private-b");
    let destination = root.join("book.kagextra");
    std::fs::create_dir(&root).expect("fixture root should be created");
    std::fs::create_dir(&source_a).expect("private source A should be created");
    std::fs::create_dir(&source_b).expect("private source B should be created");
    std::fs::write(source_a.join("publisher"), b"a").expect("source A should be written");
    std::fs::write(source_b.join("publisher"), b"b").expect("source B should be written");

    let source_a_task = source_a.clone();
    let source_b_task = source_b.clone();
    let destination_a = destination.clone();
    let destination_b = destination.clone();
    let barrier = std::sync::Barrier::new(2);
    let (result_a, result_b) = std::thread::scope(|scope| {
        let barrier_ref = &barrier;
        let task_a = scope.spawn(move || {
            barrier_ref.wait();
            publish_private_directory_noreplace(&source_a_task, &destination_a)
        });
        let barrier_ref = &barrier;
        let task_b = scope.spawn(move || {
            barrier_ref.wait();
            publish_private_directory_noreplace(&source_b_task, &destination_b)
        });
        (
            task_a.join().expect("publisher A should not panic"),
            task_b.join().expect("publisher B should not panic"),
        )
    });

    assert_eq!(
        usize::from(result_a.is_ok()) + usize::from(result_b.is_ok()),
        1
    );
    assert_eq!(
        usize::from(matches!(
            result_a,
            Err(LocalDirectoryPublishError::DestinationExists(_))
        )) + usize::from(matches!(
            result_b,
            Err(LocalDirectoryPublishError::DestinationExists(_))
        )),
        1
    );
    let published = std::fs::read(destination.join("publisher"))
        .expect("the winner's complete directory should be visible");
    assert!(published == b"a" || published == b"b");
    assert_ne!(source_a.exists(), source_b.exists());

    if source_a.exists() {
        std::fs::remove_dir_all(source_a).expect("losing source A should be removed");
    }
    if source_b.exists() {
        std::fs::remove_dir_all(source_b).expect("losing source B should be removed");
    }
    std::fs::remove_dir_all(destination).expect("published destination should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn directory_publish_rejects_symlink_sources() {
    use std::os::unix::fs::symlink;

    let root = unique_storage_path("directory-symlink");
    let real_source = root.join("real-source");
    let source_link = root.join("source-link");
    let destination = root.join("book.kagextra");
    std::fs::create_dir(&root).expect("fixture root should be created");
    std::fs::create_dir(&real_source).expect("real source should be created");
    symlink(&real_source, &source_link).expect("source symlink should be created");

    assert!(matches!(
        publish_private_directory_noreplace(&source_link, &destination),
        Err(LocalDirectoryPublishError::UnsafeSource(path)) if path == source_link
    ));
    assert!(!destination.exists());
    assert!(real_source.exists());

    std::fs::remove_file(source_link).expect("source symlink should be removed");
    std::fs::remove_dir(real_source).expect("real source should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

#[cfg(unix)]
#[test]
fn directory_publish_does_not_follow_a_destination_parent_symlink() {
    use std::os::unix::fs::symlink;

    let root = unique_storage_path("directory-parent-symlink");
    let source = root.join(".private-work");
    let real_parent = root.join("real-parent");
    let parent_link = root.join("parent-link");
    std::fs::create_dir(&root).expect("fixture root should be created");
    std::fs::create_dir(&source).expect("private source should be created");
    std::fs::create_dir(&real_parent).expect("real parent should be created");
    std::fs::write(source.join("complete"), b"complete package")
        .expect("source content should be written");
    symlink(&real_parent, &parent_link).expect("parent symlink should be created");
    let destination = parent_link.join("book.kagextra");

    assert!(matches!(
        publish_private_directory_noreplace(&source, &destination),
        Err(LocalDirectoryPublishError::MissingParent(path)) if path == parent_link
    ));
    assert!(
        source.exists(),
        "failed publication must preserve private work"
    );
    assert!(!real_parent.join("book.kagextra").exists());

    std::fs::remove_file(parent_link).expect("parent symlink should be removed");
    std::fs::remove_dir_all(source).expect("private source should be removed");
    std::fs::remove_dir(real_parent).expect("real parent should be removed");
    std::fs::remove_dir(root).expect("fixture root should be removed");
}

struct BoundedReader<R> {
    inner: R,
    maximum_read: usize,
    read_calls: usize,
}

impl<R> BoundedReader<R> {
    fn new(inner: R, maximum_read: usize) -> Self {
        Self {
            inner,
            maximum_read,
            read_calls: 0,
        }
    }
}

impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        assert!(
            buffer.len() <= self.maximum_read,
            "reader requested an unexpectedly large buffer"
        );
        self.read_calls += 1;
        self.inner.read(buffer)
    }
}

struct FailingReader {
    first_chunk: Option<Vec<u8>>,
}

impl Read for FailingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(chunk) = self.first_chunk.take() {
            buffer[..chunk.len()].copy_from_slice(&chunk);
            return Ok(chunk.len());
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "injected removable-media interruption",
        ))
    }
}

struct CapacityWriter {
    remaining: usize,
    bytes: Vec<u8>,
}

struct FlushFailingWriter {
    bytes: Vec<u8>,
}

impl std::io::Write for FlushFailingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("injected final flush failure"))
    }
}

impl std::io::Write for CapacityWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected storage capacity exhaustion",
            ));
        }
        let written = buffer.len().min(self.remaining);
        self.bytes.extend_from_slice(&buffer[..written]);
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
