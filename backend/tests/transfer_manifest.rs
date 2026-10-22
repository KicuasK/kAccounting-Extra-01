use kaccounting_backend::{
    accounting::journal::JournalEntryId,
    attachments::{Attachment, AttachmentId},
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, encrypt_object},
    encrypted_transfer::TransferredAttachment,
    storage::{StoredObjectDescriptor, describe_reader},
    transfer_manifest::EncryptedTransferManifest,
};
use std::io::Cursor;

const PACKAGE: &str = "1234567890abcdef1234567890abcdef";
fn descriptor(bytes: &[u8]) -> StoredObjectDescriptor {
    describe_reader(&mut Cursor::new(bytes)).unwrap()
}
fn reference(id: &str) -> Attachment {
    Attachment {
        id: AttachmentId::parse(id).unwrap(),
        entry_id: JournalEntryId::new(1),
        display_name: "receipt.pdf".into(),
        plaintext: descriptor(b"receipt"),
        ciphertext: descriptor(b"local encrypted receipt"),
        created_at: "2026-10-22".into(),
    }
}
fn manifest(references: &[Attachment]) -> EncryptedTransferManifest {
    EncryptedTransferManifest::new(
        PACKAGE,
        descriptor(b"recipient envelope"),
        TransferredAttachment {
            plaintext: descriptor(b"database"),
            ciphertext: descriptor(b"encrypted database"),
        },
        references
            .iter()
            .map(|source| {
                (
                    source.clone(),
                    TransferredAttachment {
                        plaintext: source.plaintext,
                        ciphertext: descriptor(source.id.as_str().as_bytes()),
                    },
                )
            })
            .collect(),
    )
    .unwrap()
}
fn roundtrip(manifest: &EncryptedTransferManifest) -> EncryptedTransferManifest {
    let key = RootEncryptionKey::new([42; 32]);
    let mut encrypted = Vec::new();
    manifest.encrypt(&mut encrypted, &key).unwrap();
    assert!(!encrypted.windows(b"receipt".len()).any(|w| w == b"receipt"));
    EncryptedTransferManifest::decrypt(
        &mut Cursor::new(encrypted),
        &key,
        PACKAGE,
        descriptor(b"recipient envelope"),
    )
    .unwrap()
}

#[test]
fn encrypted_catalog_roundtrips_and_checks_complete_snapshot_set() {
    let first = reference("00000000000000000000000000000001");
    let second = reference("00000000000000000000000000000002");
    let references = vec![second, first];
    let decoded = roundtrip(&manifest(&references));
    decoded.verify_references(&references).unwrap();
    assert_eq!(
        decoded.database().unwrap().plaintext,
        descriptor(b"database")
    );
    let objects = decoded.attachment_objects().unwrap();
    assert_eq!(objects[0].0.as_str(), "00000000000000000000000000000001");
    assert_eq!(objects[0].1.plaintext, descriptor(b"receipt"));
    assert!(decoded.verify_references(&references[..1]).is_err());
    let mut extra = references.clone();
    extra.push(reference("00000000000000000000000000000003"));
    assert!(decoded.verify_references(&extra).is_err());
    let mut duplicate = references.clone();
    duplicate[1] = duplicate[0].clone();
    assert!(decoded.verify_references(&duplicate).is_err());
    let mut changed = references.clone();
    changed[0].plaintext = descriptor(b"changed receipt");
    assert!(decoded.verify_references(&changed).is_err());
    changed = references;
    changed[0].ciphertext = descriptor(b"replaced local object");
    assert!(decoded.verify_references(&changed).is_err());
}

#[test]
fn empty_book_and_duplicate_source_references() {
    roundtrip(&manifest(&[])).verify_references(&[]).unwrap();
    let item = reference("00000000000000000000000000000001");
    assert!(
        EncryptedTransferManifest::new(
            PACKAGE,
            descriptor(b"envelope"),
            TransferredAttachment {
                plaintext: descriptor(b"db"),
                ciphertext: descriptor(b"cipher")
            },
            vec![
                (
                    item.clone(),
                    TransferredAttachment {
                        plaintext: item.plaintext,
                        ciphertext: descriptor(b"cipher")
                    }
                );
                2
            ],
        )
        .is_err()
    );
}

#[test]
fn rejects_wrong_context_key_tamper_truncation_and_trailing_bytes() {
    let key = RootEncryptionKey::new([42; 32]);
    let mut bytes = Vec::new();
    manifest(&[]).encrypt(&mut bytes, &key).unwrap();
    let read = |data: &[u8], key: &RootEncryptionKey, package: &str, envelope| {
        EncryptedTransferManifest::decrypt(&mut Cursor::new(data), key, package, envelope)
    };
    assert!(
        read(
            &bytes,
            &RootEncryptionKey::new([43; 32]),
            PACKAGE,
            descriptor(b"recipient envelope")
        )
        .is_err()
    );
    assert!(
        read(
            &bytes,
            &key,
            "00000000000000000000000000000000",
            descriptor(b"recipient envelope")
        )
        .is_err()
    );
    assert!(read(&bytes, &key, PACKAGE, descriptor(b"other envelope")).is_err());
    for length in [0, 1, bytes.len() - 1] {
        assert!(
            read(
                &bytes[..length],
                &key,
                PACKAGE,
                descriptor(b"recipient envelope")
            )
            .is_err()
        );
    }
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(read(&tampered, &key, PACKAGE, descriptor(b"recipient envelope")).is_err());
    bytes.push(0);
    assert!(read(&bytes, &key, PACKAGE, descriptor(b"recipient envelope")).is_err());
}

#[test]
fn strict_authenticated_json_rejects_unsafe_and_noncanonical_metadata() {
    let original =
        serde_json::to_value(manifest(&[reference("00000000000000000000000000000001")])).unwrap();
    let mut cases = Vec::new();
    let mut value = original.clone();
    value["version"] = 2.into();
    cases.push(value);
    let mut value = original.clone();
    value["unknown"] = true.into();
    cases.push(value);
    let mut value = original.clone();
    value["attachments"][0]["id"] = "../escape".into();
    cases.push(value);
    let mut value = original.clone();
    value["database_plaintext"]["byte_length"] = "01".into();
    cases.push(value);
    let mut value = original.clone();
    value["database_plaintext"]["sha256"] = "A".repeat(64).into();
    cases.push(value);
    let mut value = original.clone();
    value["attachments"][0]["plaintext"]["byte_length"] = "262144001".into();
    cases.push(value);
    let mut value = original.clone();
    value["attachments"][0]["plaintext"]["byte_length"] = 7.into();
    cases.push(value);
    let mut value = original.clone();
    let duplicate = value["attachments"][0].clone();
    value["attachments"].as_array_mut().unwrap().push(duplicate);
    cases.push(value);
    let key = RootEncryptionKey::new([42; 32]);
    for value in cases {
        let mut bytes = Vec::new();
        encrypt_object(
            &mut Cursor::new(serde_json::to_vec(&value).unwrap()),
            &mut bytes,
            &key,
            EncryptedObjectPurpose::TransferManifest,
        )
        .unwrap();
        assert!(
            EncryptedTransferManifest::decrypt(
                &mut Cursor::new(bytes),
                &key,
                PACKAGE,
                descriptor(b"recipient envelope")
            )
            .is_err()
        );
    }
}
