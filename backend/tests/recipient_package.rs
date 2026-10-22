use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
    attachments::AttachmentRepository,
    crypto::RootEncryptionKey,
    db::{open_database, verify_database_snapshot},
    keys::KeyStoreError,
    recipient_package::{
        RecipientPackageError, export_recipient_package, inspect_recipient_package,
        read_recipient_envelope, recover_interrupted_recipient_exports,
    },
    recipient_transfer::{
        RecipientKeyStore, RecipientRequest, create_recipient_request, unwrap_transfer_key,
    },
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;
#[derive(Default)]
struct MemoryKeys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for MemoryKeys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        let mut map = self.0.lock().unwrap();
        if map.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        map.insert(id.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    objects: PathBuf,
    pool: sqlx::SqlitePool,
    key: RootEncryptionKey,
    request: RecipientRequest,
    keys: MemoryKeys,
}
impl Fixture {
    async fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("kAccounting-Extra-01-recipient-test-{id}"));
        std::fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let pool = open_database(root.join("live.sqlite3")).await.unwrap();
        let objects = root.join("live-attachments");
        let key = RootEncryptionKey::new([37; 32]);
        AttachmentRepository::open(
            pool.clone(),
            &objects,
            Arc::new(RootEncryptionKey::new([37; 32])),
        )
        .unwrap();
        let keys = MemoryKeys::default();
        let request = create_recipient_request(&keys).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
            objects,
            pool,
            key,
            request,
            keys,
        }
    }
    async fn attach(&self) -> kaccounting_backend::attachments::Attachment {
        let entry = JournalRepository::new(self.pool.clone())
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![],
            ))
            .await
            .unwrap();
        let file = self.root.join("receipt.txt");
        std::fs::write(&file, b"private receipt evidence").unwrap();
        AttachmentRepository::open(
            self.pool.clone(),
            &self.objects,
            Arc::new(RootEncryptionKey::new([37; 32])),
        )
        .unwrap()
        .import_file(entry.id, &file, "receipt.txt")
        .await
        .unwrap()
    }
    async fn export(&self) -> kaccounting_backend::recipient_package::RecipientPackageOutcome {
        export_recipient_package(
            &self.pool,
            &self.objects,
            &self.key,
            &self.request,
            self.root.join("transfer.kagextra"),
        )
        .await
        .unwrap()
    }
    fn transfer_key(&self, package: &std::path::Path, id: &str) -> RootEncryptionKey {
        unwrap_transfer_key(
            &self.keys,
            &self.request,
            id,
            &read_recipient_envelope(package).unwrap(),
        )
        .unwrap()
    }
    async fn finish(self) {
        self.pool.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
#[tokio::test]
async fn empty_book_is_encrypted_and_independently_verified() {
    let f = Fixture::new().await;
    let outcome = f.export().await;
    assert_eq!(outcome.attachment_count, 0);
    assert!(outcome.cleanup_warning.is_none());
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    let database = f.root.join("verify.sqlite3");
    let verified = inspect_recipient_package(&outcome.path, &database, &key, &outcome.package_id)
        .await
        .unwrap();
    assert!(verified.references.is_empty());
    verify_database_snapshot(&database).await.unwrap();
    assert!(
        !std::fs::read(outcome.path.join("book.kae"))
            .unwrap()
            .starts_with(b"SQLite format 3")
    );
    assert!(
        !std::fs::read(outcome.path.join("manifest.kae"))
            .unwrap()
            .starts_with(b"{")
    );
    assert_eq!(
        std::fs::read_dir(&f.root)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("recipient-export"))
            .count(),
        0
    );
    f.finish().await;
}
#[tokio::test]
async fn all_snapshot_attachments_are_reencrypted_and_source_unchanged() {
    let f = Fixture::new().await;
    let attachment = f.attach().await;
    let source = f.objects.join(format!("{}.kae", attachment.id.as_str()));
    let before = std::fs::read(&source).unwrap();
    let outcome = f.export().await;
    assert_eq!(outcome.attachment_count, 1);
    assert_eq!(outcome.attachment_bytes, 24);
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    let verified = inspect_recipient_package(
        &outcome.path,
        &f.root.join("verify.sqlite3"),
        &key,
        &outcome.package_id,
    )
    .await
    .unwrap();
    assert_eq!(verified.references, vec![attachment.clone()]);
    let exported = std::fs::read(
        outcome
            .path
            .join("attachments")
            .join(format!("{}.kae", attachment.id.as_str())),
    )
    .unwrap();
    assert_ne!(exported, before);
    assert_eq!(std::fs::read(source).unwrap(), before);
    f.finish().await;
}
#[tokio::test]
async fn wrong_key_context_and_tampered_manifest_are_rejected() {
    let f = Fixture::new().await;
    let outcome = f.export().await;
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("wrong-key.sqlite3"),
            &RootEncryptionKey::new([91; 32]),
            &outcome.package_id
        )
        .await
        .is_err()
    );
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("wrong-id.sqlite3"),
            &key,
            &"a".repeat(32)
        )
        .await
        .is_err()
    );
    let manifest = outcome.path.join("manifest.kae");
    let mut bytes = std::fs::read(&manifest).unwrap();
    let index = bytes.len() - 1;
    bytes[index] ^= 1;
    std::fs::write(manifest, bytes).unwrap();
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("tampered.sqlite3"),
            &key,
            &outcome.package_id
        )
        .await
        .is_err()
    );
    f.finish().await;
}
#[tokio::test]
async fn missing_extra_and_changed_attachment_fail_complete_verification() {
    let f = Fixture::new().await;
    let attachment = f.attach().await;
    let outcome = f.export().await;
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    let object = outcome
        .path
        .join("attachments")
        .join(format!("{}.kae", attachment.id.as_str()));
    let original = std::fs::read(&object).unwrap();
    std::fs::remove_file(&object).unwrap();
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("missing.sqlite3"),
            &key,
            &outcome.package_id
        )
        .await
        .is_err()
    );
    std::fs::write(&object, &original).unwrap();
    let extra = outcome.path.join("attachments/unexpected.kae");
    std::fs::write(&extra, b"unexpected").unwrap();
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("extra.sqlite3"),
            &key,
            &outcome.package_id
        )
        .await
        .is_err()
    );
    std::fs::remove_file(extra).unwrap();
    let mut changed = original;
    let index = changed.len() - 1;
    changed[index] ^= 1;
    std::fs::write(object, changed).unwrap();
    assert!(
        inspect_recipient_package(
            &outcome.path,
            &f.root.join("changed.sqlite3"),
            &key,
            &outcome.package_id
        )
        .await
        .is_err()
    );
    f.finish().await;
}
#[tokio::test]
async fn conflict_never_overwrites_destination_or_verification_database() {
    let f = Fixture::new().await;
    let destination = f.root.join("conflict.kagextra");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(matches!(
        export_recipient_package(&f.pool, &f.objects, &f.key, &f.request, &destination).await,
        Err(RecipientPackageError::DestinationExists)
    ));
    assert_eq!(
        std::fs::read(destination.join("sentinel")).unwrap(),
        b"keep"
    );
    let outcome = f.export().await;
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    let database = f.root.join("occupied.sqlite3");
    std::fs::write(&database, b"keep database").unwrap();
    assert!(matches!(
        inspect_recipient_package(&outcome.path, &database, &key, &outcome.package_id).await,
        Err(RecipientPackageError::DestinationExists)
    ));
    assert_eq!(std::fs::read(database).unwrap(), b"keep database");
    f.finish().await;
}
#[tokio::test]
async fn package_symlinks_are_rejected_and_unknown_work_is_preserved() {
    let f = Fixture::new().await;
    let outcome = f.export().await;
    let key = f.transfer_key(&outcome.path, &outcome.package_id);
    #[cfg(unix)]
    {
        let file = outcome.path.join("book.kae");
        let moved = f.root.join("moved.kae");
        std::fs::rename(&file, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &file).unwrap();
        assert!(
            inspect_recipient_package(
                &outcome.path,
                &f.root.join("symlink.sqlite3"),
                &key,
                &outcome.package_id
            )
            .await
            .is_err()
        );
    }
    let unknown = f.root.join(format!(
        ".kAccounting-Extra-01-recipient-export-{}.work",
        "b".repeat(32)
    ));
    std::fs::create_dir(&unknown).unwrap();
    std::fs::write(unknown.join("sentinel"), b"keep").unwrap();
    let result = recover_interrupted_recipient_exports(&f.root, &f.key)
        .await
        .unwrap();
    assert_eq!(result.unrecognized_work, vec![unknown.clone()]);
    assert_eq!(std::fs::read(unknown.join("sentinel")).unwrap(), b"keep");
    f.finish().await;
}

#[tokio::test]
async fn external_permissionless_destination_contains_only_encrypted_package() {
    let f = Fixture::new().await;
    f.attach().await;
    let media = f.root.join("removable-media");
    std::fs::create_dir(&media).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let destination = media.join("portable.kagextra");
    let outcome = export_recipient_package(&f.pool, &f.objects, &f.key, &f.request, &destination)
        .await
        .unwrap();
    assert_eq!(outcome.attachment_count, 1);
    assert!(outcome.cleanup_warning.is_none());
    assert_eq!(std::fs::read_dir(&media).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 4);
    assert!(
        !std::fs::read(destination.join("book.kae"))
            .unwrap()
            .starts_with(b"SQLite format 3")
    );
    assert!(!destination.join("recovery.kae").exists());
    assert!(!destination.join("operation-state.json").exists());
    f.finish().await;
}
