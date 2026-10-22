use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry,
            SaveDraftEntry,
        },
        money::MinorUnits,
    },
    attachments::AttachmentRepository,
    crypto::RootEncryptionKey,
    db::{open_database, verify_database_snapshot},
    keys::KeyStoreError,
    keys::{DeviceKeyId, DeviceKeyStore},
    recipient_import::{import_recipient_package, inspect_interrupted_recipient_imports},
    recipient_package::{export_recipient_package, read_recipient_envelope},
    recipient_transfer::{
        RecipientKeyStore, RecipientRequest, create_recipient_request, unwrap_transfer_key,
    },
    runtime::RuntimeBook,
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
    #[allow(dead_code)]
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

impl DeviceKeyStore for MemoryKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), bytes)
    }
}
#[tokio::test]
async fn import_reencrypts_complete_book_and_runtime_opens_with_fresh_key() {
    let f = Fixture::new().await;
    let original = f.attach().await;
    let accounts = AccountRepository::new(f.pool.clone());
    let mut lines = Vec::new();
    for (name, element, side) in [
        ("Asset", AccountingElement::Asset, JournalSide::Debit),
        ("Equity", AccountingElement::Equity, JournalSide::Credit),
    ] {
        let account = accounts
            .create(NewAccount::new(None, name, element, AccountKind::Posting))
            .await
            .unwrap();
        lines.push(JournalLineInput::new(
            account.id,
            side,
            MinorUnits::new(9007199254740993),
        ));
    }
    let posted = JournalRepository::new(f.pool.clone())
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            lines,
        ))
        .await
        .unwrap();
    let file = f.root.join("posted.txt");
    std::fs::write(&file, b"posted proof").unwrap();
    let posted_attachment = AttachmentRepository::open(
        f.pool.clone(),
        &f.objects,
        Arc::new(RootEncryptionKey::new([37; 32])),
    )
    .unwrap()
    .import_file(posted.id, &file, "posted.txt")
    .await
    .unwrap();
    let package = f.export().await;
    let dest = f.root.join("imported");
    let imported = import_recipient_package(&package.path, &dest, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    assert_eq!(imported.attachment_count, 2);
    let book = RuntimeBook::open_existing(&dest, &f.keys).await.unwrap();
    assert_ne!(
        DeviceKeyStore::read(&f.keys, book.key_id())
            .unwrap()
            .as_slice(),
        &[37; 32]
    );
    let catalog = kaccounting_backend::attachments::AttachmentRepository::open(
        book.pool().clone(),
        book.attachments(),
        book.key(),
    )
    .unwrap();
    // Full audit validates the exact immutable guard after isolated descriptor rewrite.
    verify_database_snapshot(dest.join("book.sqlite3"))
        .await
        .unwrap();
    let cipher: String = sqlx::query_scalar(
        "SELECT ciphertext_sha256 FROM attachments WHERE display_name='receipt.txt'",
    )
    .fetch_one(book.pool())
    .await
    .unwrap();
    assert_ne!(cipher, original.ciphertext.digest().to_hex());
    let source_cipher: String = sqlx::query_scalar(
        "SELECT ciphertext_sha256 FROM attachments WHERE display_name='receipt.txt'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(source_cipher, original.ciphertext.digest().to_hex());
    assert!(
        sqlx::query("UPDATE attachments SET display_name='changed'")
            .execute(book.pool())
            .await
            .is_err()
    );
    let states = inspect_interrupted_recipient_imports(&f.root).unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].1.phase, "published");
    assert_eq!(
        JournalRepository::new(book.pool().clone())
            .get_posted(posted.id)
            .await
            .unwrap(),
        posted
    );
    let (_, mut stream) = catalog
        .download_stream(&posted_attachment.id)
        .await
        .unwrap();
    let mut received = Vec::new();
    while let Some(frame) = stream.recv().await {
        received.extend(frame.unwrap());
    }
    assert_eq!(received, b"posted proof");
    assert!(
        sqlx::query("DELETE FROM attachments WHERE display_name='posted.txt'")
            .execute(book.pool())
            .await
            .is_err()
    );
    drop(catalog);
    book.close().await;
    drop(book);
    f.finish().await;
}
#[tokio::test]
async fn wrong_recipient_and_existing_destination_fail_without_mutation() {
    let f = Fixture::new().await;
    let package = f.export().await;
    let wrong = create_recipient_request(&f.keys).unwrap();
    let dest = f.root.join("imported");
    assert!(
        import_recipient_package(&package.path, &dest, &wrong, &f.keys, &f.keys)
            .await
            .is_err()
    );
    assert!(!dest.exists());
    assert!(
        inspect_interrupted_recipient_imports(&f.root)
            .unwrap()
            .is_empty()
    );
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join("keep"), b"unchanged").unwrap();
    assert!(
        import_recipient_package(&package.path, &dest, &f.request, &f.keys, &f.keys)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(dest.join("keep")).unwrap(), b"unchanged");
    f.finish().await;
}
#[tokio::test]
async fn tampered_package_retains_private_interruption_evidence_without_publication() {
    let f = Fixture::new().await;
    let package = f.export().await;
    let file = package.path.join("book.kae");
    let mut bytes = std::fs::read(&file).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 1;
    std::fs::write(file, bytes).unwrap();
    let dest = f.root.join("imported");
    assert!(
        import_recipient_package(&package.path, &dest, &f.request, &f.keys, &f.keys)
            .await
            .is_err()
    );
    assert!(!dest.exists());
    let states = inspect_interrupted_recipient_imports(&f.root).unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].1.phase, "created");
    let before = std::fs::read_dir(&states[0].0).unwrap().count();
    assert_eq!(
        inspect_interrupted_recipient_imports(&f.root).unwrap(),
        states
    );
    assert_eq!(std::fs::read_dir(&states[0].0).unwrap().count(), before);
    f.finish().await;
}

#[tokio::test]
async fn cancelled_caller_does_not_drop_owned_import_or_its_work_lock() {
    let f = Fixture::new().await;
    f.attach().await;
    let package = f.export().await;
    let dest = f.root.join("cancelled-caller-import");
    let mut operation = Box::pin(import_recipient_package(
        &package.path,
        &dest,
        &f.request,
        &f.keys,
        &f.keys,
    ));
    tokio::select! { biased; result=&mut operation => panic!("operation completed before cancellation injection: {result:?}"), _=tokio::task::yield_now()=>() }
    drop(operation);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let states = inspect_interrupted_recipient_imports(&f.root).unwrap();
        if states.iter().any(|(_, s)| s.phase == "published") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "detached worker must finish and release lock"
        );
        tokio::task::yield_now().await;
    }
    let book = RuntimeBook::open_existing(&dest, &f.keys).await.unwrap();
    book.close().await;
    drop(book);
    f.finish().await;
}
#[tokio::test]
async fn status_skips_live_workers_and_never_publishes_a_forged_ready_record() {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let f = Fixture::new().await;
    let work = f
        .root
        .join(".kAccounting-Extra-01-recipient-import-11111111111111111111111111111111");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&work)
        .unwrap();
    let lock = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(work.join("operation.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let destination = f.root.join("must-not-publish");
    let record = serde_json::json!({"format":"kAccounting-Extra-01.recipient-import.v1","package_id":"11111111111111111111111111111111","destination":destination,"phase":"ready"});
    std::fs::write(
        work.join("ready.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    assert!(
        inspect_interrupted_recipient_imports(&f.root)
            .unwrap()
            .is_empty()
    );
    drop(lock);
    let result = inspect_interrupted_recipient_imports(&f.root).unwrap();
    assert_eq!(result[0].1.phase, "ready");
    assert!(!destination.exists());
    f.finish().await;
}

#[tokio::test]
async fn authenticated_restart_cleans_only_unpublished_owned_work_and_retains_keys_and_source() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    let f = Fixture::new().await;
    let package = f.export().await;
    let encrypted = package.path.join("book.kae");
    let mut bytes = std::fs::read(&encrypted).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&encrypted, bytes).unwrap();
    let destination = f.root.join("incomplete");
    assert!(
        import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
            .await
            .is_err()
    );
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    let key_count = f.keys.0.lock().unwrap().len();
    // Plain checkpoints can neither authorize nor prevent authentic ownership verification.
    std::fs::write(work.join("created.json"), b"forged advisory data").unwrap();
    let recovered = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(recovered.removed_count, 1);
    assert!(recovered.unrecognized_work.is_empty());
    assert!(!work.exists());
    assert!(!destination.exists());
    assert!(package.path.exists());
    assert_eq!(f.keys.0.lock().unwrap().len(), key_count);
    f.finish().await;
}
#[tokio::test]
async fn authenticated_restart_verifies_published_book_before_cleanup() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    let f = Fixture::new().await;
    f.attach().await;
    let package = f.export().await;
    let destination = f.root.join("published");
    import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    // Simulate process loss between rename and published marker: authentic READY remains.
    std::fs::remove_file(work.join("published.json")).unwrap();
    let recovered = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(recovered.removed_count, 1);
    assert_eq!(recovered.verified_destinations, vec![destination.clone()]);
    assert!(!work.exists());
    assert!(destination.exists());
    assert!(package.path.exists());
    let book = RuntimeBook::open_existing(&destination, &f.keys)
        .await
        .unwrap();
    book.close().await;
    drop(book);
    f.finish().await;
}
#[tokio::test]
async fn restart_rejects_missing_key_tampered_context_and_forged_status_without_cleanup() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    let f = Fixture::new().await;
    let package = f.export().await;
    let destination = f.root.join("published");
    import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    let wrong = MemoryKeys::default();
    let result = recover_interrupted_recipient_imports(&f.root, &wrong)
        .await
        .unwrap();
    assert_eq!(result.removed_count, 0);
    assert_eq!(result.unrecognized_work, vec![work.clone()]);
    let context = work.join("ready.kae");
    let mut bytes = std::fs::read(&context).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&context, bytes).unwrap();
    let result = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(result.removed_count, 0);
    assert_eq!(result.unrecognized_work, vec![work.clone()]);
    assert!(work.exists());
    assert!(destination.exists());
    assert!(package.path.exists());
    f.finish().await;
}
#[tokio::test]
async fn restart_preserves_modified_or_currently_open_published_book() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    let f = Fixture::new().await;
    let package = f.export().await;
    let destination = f.root.join("published");
    import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    let book = RuntimeBook::open_existing(&destination, &f.keys)
        .await
        .unwrap();
    let result = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(result.removed_count, 0);
    assert!(work.exists());
    JournalRepository::new(book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    book.close().await;
    drop(book);
    let result = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(result.removed_count, 0);
    assert_eq!(result.unrecognized_work, vec![work.clone()]);
    assert!(work.exists());
    assert!(destination.exists());
    f.finish().await;
}

#[tokio::test]
async fn restart_rejects_copied_ownership_context_even_at_the_same_work_name() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    use std::os::unix::fs::DirBuilderExt;
    let f = Fixture::new().await;
    let package = f.export().await;
    let destination = f.root.join("published");
    import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    let preserved = f.root.join("preserved-original-work");
    std::fs::rename(&work, &preserved).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&work)
        .unwrap();
    for name in [
        "operation.lock",
        "recovery-key-id",
        "recovery.kae",
        "ready.kae",
    ] {
        std::fs::copy(preserved.join(name), work.join(name)).unwrap();
    }
    let recovered = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(recovered.removed_count, 0);
    assert_eq!(recovered.unrecognized_work, vec![work.clone()]);
    assert!(work.exists());
    assert!(preserved.exists());
    assert!(destination.exists());
    f.finish().await;
}
#[tokio::test]
async fn restart_skips_active_work_and_plain_ready_status_cannot_authorize_cleanup() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let f = Fixture::new().await;
    let work = f
        .root
        .join(".kAccounting-Extra-01-recipient-import-22222222222222222222222222222222");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&work)
        .unwrap();
    let lock = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(work.join("operation.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    std::fs::write(work.join("ready.json"), b"{\"phase\":\"ready\"}").unwrap();
    let recovered = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(recovered.active_work, vec![work.clone()]);
    assert_eq!(recovered.removed_count, 0);
    drop(lock);
    let recovered = recover_interrupted_recipient_imports(&f.root, &f.keys)
        .await
        .unwrap();
    assert_eq!(recovered.unrecognized_work, vec![work.clone()]);
    assert_eq!(recovered.removed_count, 0);
    assert!(work.exists());
    f.finish().await;
}
#[tokio::test]
async fn cancelled_recovery_caller_keeps_owned_verification_and_cleanup_alive() {
    use kaccounting_backend::recipient_import::recover_interrupted_recipient_imports;
    let f = Fixture::new().await;
    f.attach().await;
    let package = f.export().await;
    let destination = f.root.join("published");
    import_recipient_package(&package.path, &destination, &f.request, &f.keys, &f.keys)
        .await
        .unwrap();
    let work = inspect_interrupted_recipient_imports(&f.root).unwrap()[0]
        .0
        .clone();
    let mut operation = Box::pin(recover_interrupted_recipient_imports(&f.root, &f.keys));
    tokio::select! {biased; result=&mut operation=>panic!("recovery finished before cancellation injection: {result:?}"), _=tokio::task::yield_now()=>()}
    drop(operation);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while work.exists() {
        assert!(std::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    let book = RuntimeBook::open_existing(&destination, &f.keys)
        .await
        .unwrap();
    book.close().await;
    drop(book);
    f.finish().await;
}
