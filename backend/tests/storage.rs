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
    std::env::temp_dir().join(format!(
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

    std::fs::remove_file(source).expect("source should be removed");
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
    let (result_a, result_b) = std::thread::scope(|scope| {
        let task_a = scope.spawn(move || {
            publish_verified_local_copy(&source_a_task, &destination_a, descriptor_a)
        });
        let task_b = scope.spawn(move || {
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
    let (result_a, result_b) = std::thread::scope(|scope| {
        let task_a = scope
            .spawn(move || publish_private_directory_noreplace(&source_a_task, &destination_a));
        let task_b = scope
            .spawn(move || publish_private_directory_noreplace(&source_b_task, &destination_b));
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
