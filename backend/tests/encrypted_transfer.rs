use kaccounting_backend::{
    attachment_maintenance::StorePin,
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, decrypt_object, encrypt_object},
    encrypted_transfer::{
        TransferredAttachment, export_attachment_object, import_attachment_object,
    },
    keys::KeyStoreError,
    recipient_transfer::{
        RecipientKeyStore, create_recipient_request, create_transfer_key, unwrap_transfer_key,
    },
    storage::{ContentDigest, StoredObjectDescriptor, describe_file, describe_reader},
};
use std::{
    collections::HashMap,
    fs::File,
    io::{Cursor, Write},
    path::PathBuf,
    sync::Mutex,
};
use zeroize::Zeroizing;

struct Fixture {
    root: PathBuf,
    objects: PathBuf,
    stage: PathBuf,
    pin: StorePin,
    local: RootEncryptionKey,
    transfer: RootEncryptionKey,
}
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("kAccounting-Extra-01-transcode-test-{name}"));
        std::fs::create_dir(&root).unwrap();
        let objects = root.join("objects");
        let stage = root.join("stage");
        std::fs::create_dir(&objects).unwrap();
        std::fs::create_dir(&stage).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&root, &objects, &stage] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let pin = StorePin::acquire(&objects).unwrap();
        Self {
            root,
            objects,
            stage,
            pin,
            local: RootEncryptionKey::new([31; 32]),
            transfer: RootEncryptionKey::new([83; 32]),
        }
    }
    fn source(&self, bytes: &[u8]) -> (PathBuf, TransferredAttachment) {
        let path = self.objects.join("fixture.kae");
        let mut file = File::create(&path).unwrap();
        encrypt_object(
            &mut Cursor::new(bytes),
            &mut file,
            &self.local,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        (
            path.clone(),
            TransferredAttachment {
                plaintext: describe_reader(&mut Cursor::new(bytes)).unwrap(),
                ciphertext: describe_file(path).unwrap(),
            },
        )
    }
    fn assert_no_stage(&self) {
        assert_eq!(
            std::fs::read_dir(&self.stage).unwrap().count(),
            0,
            "failed operation must leave no stage or published file"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn reencrypts_empty_chunk_boundaries_and_large_objects_without_plaintext_files() {
    for length in [0, 1, 65535, 65536, 65537, 131072, 131073, 5 * 1024 * 1024] {
        let f = Fixture::new();
        let bytes: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        let (source, expected) = f.source(&bytes);
        let exported = f.stage.join("export.kae");
        let outcome =
            export_attachment_object(&f.pin, &source, &exported, expected, &f.local, &f.transfer)
                .unwrap();
        assert!(outcome.cleanup_warning.is_none());
        assert_eq!(outcome.attachment.plaintext, expected.plaintext);
        assert_ne!(outcome.attachment.ciphertext, expected.ciphertext);
        let imported = f.stage.join("import.kae");
        let destination_key = RootEncryptionKey::new([117; 32]);
        let imported_outcome = import_attachment_object(
            &exported,
            &imported,
            outcome.attachment,
            &f.transfer,
            &destination_key,
        )
        .unwrap();
        assert_eq!(imported_outcome.attachment.plaintext, expected.plaintext);
        let mut actual = Vec::new();
        decrypt_object(
            &mut File::open(&imported).unwrap(),
            &mut actual,
            &destination_key,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        assert_eq!(actual, bytes);
        assert_eq!(std::fs::read_dir(&f.stage).unwrap().count(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&exported).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&imported).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn authentication_descriptor_and_late_stream_failures_never_publish() {
    for case in 0..7 {
        let f = Fixture::new();
        let (source, mut expected) = f.source(&vec![0x5a; 3 * 65536 + 29]);
        let wrong = RootEncryptionKey::new([99; 32]);
        let key = if case == 0 { &wrong } else { &f.local };
        match case {
            1 => {
                let mut bytes = std::fs::read(&source).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                std::fs::write(&source, &bytes).unwrap();
                expected.ciphertext = describe_file(&source).unwrap();
            }
            2 => {
                expected.plaintext = StoredObjectDescriptor::new(
                    ContentDigest::from_bytes([0; 32]),
                    expected.plaintext.byte_length(),
                )
            }
            3 => {
                expected.plaintext = StoredObjectDescriptor::new(
                    expected.plaintext.digest(),
                    expected.plaintext.byte_length() - 1,
                )
            }
            4 => {
                expected.ciphertext = StoredObjectDescriptor::new(
                    ContentDigest::from_bytes([0; 32]),
                    expected.ciphertext.byte_length(),
                )
            }
            5 => {
                File::options()
                    .append(true)
                    .open(&source)
                    .unwrap()
                    .write_all(b"unexpected trailing bytes")
                    .unwrap();
                expected.ciphertext = describe_file(&source).unwrap();
            }
            6 => {
                expected.plaintext =
                    StoredObjectDescriptor::new(expected.plaintext.digest(), 250 * 1024 * 1024 + 1)
            }
            _ => {}
        }
        assert!(
            export_attachment_object(
                &f.pin,
                &source,
                &f.stage.join("failed.kae"),
                expected,
                key,
                &f.transfer
            )
            .is_err()
        );
        f.assert_no_stage();
    }
}

#[test]
fn destination_is_never_overwritten_and_wrong_purpose_is_rejected() {
    let f = Fixture::new();
    let (source, expected) = f.source(b"private receipt");
    let destination = f.stage.join("existing.kae");
    std::fs::write(&destination, b"existing valid destination").unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &source,
            &destination,
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"existing valid destination"
    );
    std::fs::remove_file(&destination).unwrap();
    assert!(
        import_attachment_object(&source, &destination, expected, &f.local, &f.transfer).is_err(),
        "a local attachment cannot be substituted for transfer purpose"
    );
    f.assert_no_stage();
}

#[test]
fn source_must_be_in_the_pinned_store() {
    let f = Fixture::new();
    let (source, expected) = f.source(b"receipt");
    let copied = f.stage.join("outside.kae");
    std::fs::copy(&source, &copied).unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &copied,
            &f.stage.join("output.kae"),
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&f.stage).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symlink_source_and_destination_are_rejected_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let (source, expected) = f.source(b"receipt");
    let alias = f.objects.join("alias.kae");
    symlink(&source, &alias).unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &alias,
            &f.stage.join("output.kae"),
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    f.assert_no_stage();
    let target = f.root.join("existing");
    std::fs::write(&target, b"preserve me").unwrap();
    let destination = f.stage.join("symlink.kae");
    symlink(&target, &destination).unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &source,
            &destination,
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"preserve me");
}

#[cfg(unix)]
#[test]
fn destination_staging_must_be_private_and_not_a_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let (source, expected) = f.source(b"receipt");
    std::fs::set_permissions(&f.stage, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &source,
            &f.stage.join("output.kae"),
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    f.assert_no_stage();
    std::fs::set_permissions(&f.stage, std::fs::Permissions::from_mode(0o700)).unwrap();
    let alias = f.root.join("stage-alias");
    symlink(&f.stage, &alias).unwrap();
    assert!(
        export_attachment_object(
            &f.pin,
            &source,
            &alias.join("output.kae"),
            expected,
            &f.local,
            &f.transfer
        )
        .is_err()
    );
    f.assert_no_stage();
}

#[derive(Default)]
struct MemoryRecipient(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for MemoryRecipient {
    fn read(&self, request_id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(request_id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, request_id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        let mut map = self.0.lock().unwrap();
        if map.contains_key(request_id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        map.insert(request_id.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }
}

#[test]
fn hpke_destination_key_reencrypts_transfer_object_into_new_local_key() {
    let f = Fixture::new();
    let store = MemoryRecipient::default();
    let request = create_recipient_request(&store).unwrap();
    let package_id = "2b6b34fb9bb447058b00c74313964026";
    let (transfer_key, envelope) = create_transfer_key(&request, package_id).unwrap();
    let (source, expected) = f.source(b"destination-bound attachment evidence");
    let exported = f.stage.join("export.kae");
    let outcome = export_attachment_object(
        &f.pin,
        &source,
        &exported,
        expected,
        &f.local,
        &transfer_key,
    )
    .unwrap();
    let recovered = unwrap_transfer_key(&store, &request, package_id, &envelope).unwrap();
    let imported = f.stage.join("import.kae");
    let destination_key = RootEncryptionKey::new([181; 32]);
    let imported_outcome = import_attachment_object(
        &exported,
        &imported,
        outcome.attachment,
        &recovered,
        &destination_key,
    )
    .unwrap();
    assert_eq!(imported_outcome.attachment.plaintext, expected.plaintext);
    let mut plaintext = Vec::new();
    decrypt_object(
        &mut File::open(&imported).unwrap(),
        &mut plaintext,
        &destination_key,
        EncryptedObjectPurpose::Attachment,
    )
    .unwrap();
    assert_eq!(plaintext, b"destination-bound attachment evidence");
}
