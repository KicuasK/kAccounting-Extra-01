//! Cross-service acceptance: disposable local books, fake keys, no network.
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry},
        money::MinorUnits,
    },
    attachments::AttachmentRepository,
    book_selection::BookSelectionSession,
    db::verify_database_snapshot,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_import::{
        import_recipient_package, inspect_interrupted_recipient_imports,
        recover_interrupted_recipient_imports,
    },
    recipient_package::export_recipient_package,
    recipient_transfer::{RecipientKeyStore, create_recipient_request},
    runtime::RuntimeBook,
};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
use zeroize::Zeroizing;

#[derive(Default)]
struct Keys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().unwrap();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.into(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), bytes)
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).unwrap();
        let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-independent-acceptance-{token}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn recipient_roundtrip_preserves_exact_posted_history_and_requires_explicit_selection() {
    let scratch = Scratch::new();
    let keys = Keys::default();
    let source = scratch.0.join("source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let mut session = BookSelectionSession::new();
    assert!(session.current().is_none());
    session.select(&source, &keys).await.unwrap();
    let book = session.current().unwrap();
    let accounts = AccountRepository::new(book.pool().clone());
    let exact = 9_007_199_254_740_993i64;
    let mut lines = Vec::new();
    for (name, side) in [
        ("Acceptance cash", JournalSide::Debit),
        ("Acceptance bank", JournalSide::Credit),
    ] {
        let account = accounts
            .create(NewAccount::new(
                None,
                name,
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await
            .unwrap();
        lines.push(JournalLineInput::new(
            account.id,
            side,
            MinorUnits::new(exact),
        ));
    }
    let journal = JournalRepository::new(book.pool().clone());
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            lines,
        ))
        .await
        .unwrap();
    let source_file = scratch.0.join("receipt.txt");
    let plaintext = b"Independent acceptance attachment: receipt evidence.";
    std::fs::write(&source_file, plaintext).unwrap();
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key()).unwrap();
    let attachment = repository
        .import_file(posted.id, &source_file, "receipt.txt")
        .await
        .unwrap();
    let reversal = journal
        .reverse_posted(posted.id, JournalDate::parse("2026-10-22").unwrap())
        .await
        .unwrap();
    let posted = journal.get_posted(posted.id).await.unwrap();
    let request = create_recipient_request(&keys).unwrap();
    let package = export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        scratch.0.join("transfer.kagextra"),
    )
    .await
    .unwrap();
    let destination = scratch.0.join("destination");
    import_recipient_package(&package.path, &destination, &request, &keys, &keys)
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().root(), source);
    assert_eq!(journal.get_posted(posted.id).await.unwrap(), posted);
    // A valid database descriptor alone must never authorize recovery cleanup
    // when even one published ciphertext object has changed.
    let work = inspect_interrupted_recipient_imports(&scratch.0).unwrap()[0]
        .0
        .clone();
    let object_path = destination
        .join("attachments")
        .join(format!("{}.kae", attachment.id.as_str()));
    let ciphertext = std::fs::read(&object_path).unwrap();
    let mut tampered = ciphertext.clone();
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(&object_path, tampered).unwrap();
    let rejected = recover_interrupted_recipient_imports(&scratch.0, &keys)
        .await
        .unwrap();
    assert_eq!(rejected.removed_count, 0);
    assert_eq!(rejected.unrecognized_work, vec![work.clone()]);
    assert!(work.exists());
    assert_eq!(session.current().unwrap().root(), source);
    std::fs::write(&object_path, ciphertext).unwrap();
    let recovered = recover_interrupted_recipient_imports(&scratch.0, &keys)
        .await
        .unwrap();
    assert_eq!(recovered.removed_count, 1);
    assert_eq!(recovered.verified_destinations, vec![destination.clone()]);
    assert!(!work.exists());
    assert_eq!(session.current().unwrap().root(), source);
    session.select(&destination, &keys).await.unwrap();
    let imported = session.current().unwrap();
    assert_eq!(
        JournalRepository::new(imported.pool().clone())
            .get_posted(posted.id)
            .await
            .unwrap(),
        posted
    );
    assert_eq!(
        JournalRepository::new(imported.pool().clone())
            .get_posted(reversal.id)
            .await
            .unwrap(),
        reversal
    );
    let sums: (i64,i64)=sqlx::query_as("SELECT SUM(CASE WHEN side='debit' THEN amount_minor ELSE 0 END),SUM(CASE WHEN side='credit' THEN amount_minor ELSE 0 END) FROM journal_lines").fetch_one(imported.pool()).await.unwrap();
    assert_eq!(sums, (exact * 2, exact * 2));
    for sql in [
        "UPDATE journal_entries SET explanation='forged' WHERE status='posted'",
        "DELETE FROM journal_entries WHERE status='posted'",
        "UPDATE journal_lines SET amount_minor=1",
        "UPDATE attachments SET display_name='forged'",
        "DELETE FROM attachments",
    ] {
        assert!(
            sqlx::query(sql).execute(imported.pool()).await.is_err(),
            "guard missing: {sql}"
        );
    }
    let repository = AttachmentRepository::open(
        imported.pool().clone(),
        imported.attachments(),
        imported.key(),
    )
    .unwrap();
    let (metadata, mut chunks) = repository.download_stream(&attachment.id).await.unwrap();
    assert_eq!(metadata.plaintext, attachment.plaintext);
    assert_ne!(metadata.ciphertext, attachment.ciphertext);
    let mut downloaded = Vec::new();
    while let Some(chunk) = chunks.recv().await {
        downloaded.extend(chunk.unwrap());
    }
    assert_eq!(downloaded, plaintext);
    verify_database_snapshot(destination.join("book.sqlite3"))
        .await
        .unwrap();
    session.close().await;
    // A new startup always has no selected book, even after a successful import.
    assert!(BookSelectionSession::new().current().is_none());
}

/// Full package capacity acceptance, separate from the HTTP-only capacity test.
/// Generate, encrypt, transfer, import and hash without a full plaintext buffer.
#[tokio::test]
#[ignore = "250 MiB complete recipient-package acceptance; run explicitly in release mode"]
async fn maximum_recipient_package_roundtrips_with_bounded_plaintext_memory() {
    use kaccounting_backend::{
        accounting::journal::SaveDraftEntry,
        attachments::MAX_ATTACHMENT_BYTES,
        storage::{ContentDigest, StoredObjectDescriptor, describe_file},
    };
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let scratch = Scratch::new();
    let keys = Keys::default();
    let source = scratch.0.join("capacity-source");
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    let entry = JournalRepository::new(book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let input_path = scratch.0.join("capacity-receipt.bin");
    let mut input = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&input_path)
        .unwrap();
    let block = [0x5au8; 64 * 1024];
    let mut remaining = MAX_ATTACHMENT_BYTES;
    while remaining > 0 {
        let count = remaining.min(block.len() as u64) as usize;
        input.write_all(&block[..count]).unwrap();
        remaining -= count as u64;
    }
    input.sync_all().unwrap();
    drop(input);
    let expected = describe_file(&input_path).unwrap();
    assert_eq!(expected.byte_length(), 250 * 1024 * 1024);
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key()).unwrap();
    let attachment = repository
        .import_file(entry.id, &input_path, "capacity-receipt.bin")
        .await
        .unwrap();
    assert_eq!(attachment.plaintext, expected);
    std::fs::remove_file(&input_path).unwrap();
    assert!(!input_path.exists());
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(book.pool())
        .await
        .unwrap();
    let source_database = source.join("book.sqlite3");
    let source_database_before = describe_file(&source_database).unwrap();
    let source_ciphertext_path = book
        .attachments()
        .join(format!("{}.kae", attachment.id.as_str()));
    let source_ciphertext_before = describe_file(&source_ciphertext_path).unwrap();
    let source_key = DeviceKeyStore::read(&keys, book.key_id()).unwrap();
    let request = create_recipient_request(&keys).unwrap();
    let package = export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        scratch.0.join("capacity.kagextra"),
    )
    .await
    .unwrap();
    assert_eq!(package.attachment_count, 1);
    assert_eq!(
        describe_file(&source_database).unwrap(),
        source_database_before
    );
    assert_eq!(
        describe_file(&source_ciphertext_path).unwrap(),
        source_ciphertext_before
    );
    let destination = scratch.0.join("capacity-imported");
    let imported = import_recipient_package(&package.path, &destination, &request, &keys, &keys)
        .await
        .unwrap();
    assert_eq!(imported.attachment_count, 1);
    assert_eq!(
        describe_file(&source_database).unwrap(),
        source_database_before
    );
    assert_eq!(
        describe_file(&source_ciphertext_path).unwrap(),
        source_ciphertext_before
    );
    assert!(package.path.exists());
    let imported_book = RuntimeBook::open_existing(&destination, &keys)
        .await
        .unwrap();
    assert_ne!(imported_book.key_id(), book.key_id());
    assert_ne!(
        DeviceKeyStore::read(&keys, imported_book.key_id()).unwrap(),
        source_key
    );
    let imported_repository = AttachmentRepository::open(
        imported_book.pool().clone(),
        imported_book.attachments(),
        imported_book.key(),
    )
    .unwrap();
    let (metadata, mut chunks) = imported_repository
        .download_stream(&attachment.id)
        .await
        .unwrap();
    assert_eq!(metadata.plaintext, expected);
    assert_ne!(metadata.ciphertext, attachment.ciphertext);
    let mut digest = Sha256::new();
    let mut downloaded_bytes = 0u64;
    while let Some(chunk) = chunks.recv().await {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 64 * 1024);
        digest.update(&chunk);
        downloaded_bytes += chunk.len() as u64;
    }
    let hash: [u8; 32] = digest.finalize().into();
    assert_eq!(
        StoredObjectDescriptor::new(ContentDigest::from_bytes(hash), downloaded_bytes),
        expected
    );
    assert_eq!(
        describe_file(&source_database).unwrap(),
        source_database_before
    );
    verify_database_snapshot(destination.join("book.sqlite3"))
        .await
        .unwrap();
    imported_book.close().await;
    book.close().await;
}

const PROCESS_DEATH_KEY: [u8; 32] = [73; 32]; // Fake test key, never persisted.
const PROCESS_DEATH_BYTES: u64 = 64 * 1024 * 1024;

#[tokio::test]
#[ignore = "child helper for the explicit process-death acceptance test"]
async fn process_death_recipient_export_child() {
    use kaccounting_backend::{
        accounting::journal::SaveDraftEntry, crypto::RootEncryptionKey, db::open_database,
        storage::describe_file,
    };
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        sync::Arc,
    };
    let Some(root) = std::env::var_os("KA_PROCESS_DEATH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let source = root.join("source");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&source)
        .unwrap();
    let pool = open_database(source.join("book.sqlite3")).await.unwrap();
    let key = Arc::new(RootEncryptionKey::new(PROCESS_DEATH_KEY));
    let repository =
        AttachmentRepository::open(pool.clone(), source.join("attachments"), key.clone()).unwrap();
    let draft = JournalRepository::new(pool.clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let plaintext = root.join("temporary-receipt.bin");
    let mut input = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&plaintext)
        .unwrap();
    let block = [0x6bu8; 64 * 1024];
    for _ in 0..PROCESS_DEATH_BYTES / block.len() as u64 {
        input.write_all(&block).unwrap();
    }
    input.sync_all().unwrap();
    drop(input);
    let attachment = repository
        .import_file(draft.id, &plaintext, "receipt.bin")
        .await
        .unwrap();
    std::fs::remove_file(&plaintext).unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
        .unwrap();
    let database = describe_file(source.join("book.sqlite3")).unwrap();
    let metadata = serde_json::json!({
        "database_sha256": database.digest().to_hex(),
        "database_bytes": database.byte_length(),
        "attachment_id": attachment.id.as_str(),
        "ciphertext_sha256": attachment.ciphertext.digest().to_hex(),
        "ciphertext_bytes": attachment.ciphertext.byte_length(),
        "plaintext_sha256": attachment.plaintext.digest().to_hex(),
        "plaintext_bytes": attachment.plaintext.byte_length(),
    });
    let mut metadata_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join("source-descriptors.json"))
        .unwrap();
    metadata_file
        .write_all(&serde_json::to_vec(&metadata).unwrap())
        .unwrap();
    metadata_file.sync_all().unwrap();
    drop(metadata_file);
    let keys = Keys::default();
    let request = create_recipient_request(&keys).unwrap();
    export_recipient_package(
        &pool,
        source.join("attachments"),
        &key,
        &request,
        root.join("published.kagextra"),
    )
    .await
    .unwrap();
    // Keep a missed publication window observable as a failed parent assertion,
    // rather than letting an otherwise successful child exit hide the race.
    loop {
        std::thread::park();
    }
}

#[tokio::test]
#[ignore = "actual SIGKILL and authenticated restart recovery; run explicitly in release mode"]
async fn process_death_recipient_export_recovers_published_package_and_preserves_source() {
    use kaccounting_backend::{
        attachments::AttachmentId,
        crypto::RootEncryptionKey,
        recipient_package::recover_interrupted_recipient_exports,
        storage::{StoredObjectDescriptor, describe_file},
    };
    use sha2::{Digest, Sha256};
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Command, Stdio},
        sync::Arc,
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let scratch = Scratch::new();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_death_recipient_export_child",
            "--ignored",
        ])
        .env("KA_PROCESS_DEATH_ROOT", &scratch.0)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let destination = scratch.0.join("published.kagextra");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !destination.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child must not exit before publication"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "publication deadline exceeded"
        );
        std::thread::yield_now();
    }
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    assert!(!scratch.0.join("temporary-receipt.bin").exists());
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(scratch.0.join("source-descriptors.json")).unwrap())
            .unwrap();
    let decode = |hash: &str, bytes: &str| {
        StoredObjectDescriptor::new(
            metadata[hash].as_str().unwrap().parse().unwrap(),
            metadata[bytes].as_u64().unwrap(),
        )
    };
    let database_before = decode("database_sha256", "database_bytes");
    let ciphertext_before = decode("ciphertext_sha256", "ciphertext_bytes");
    let plaintext_before = decode("plaintext_sha256", "plaintext_bytes");
    let source = scratch.0.join("source");
    let object_id = AttachmentId::parse(metadata["attachment_id"].as_str().unwrap()).unwrap();
    let object_path = source
        .join("attachments")
        .join(format!("{}.kae", object_id.as_str()));
    assert_eq!(
        describe_file(source.join("book.sqlite3")).unwrap(),
        database_before
    );
    assert_eq!(describe_file(&object_path).unwrap(), ciphertext_before);
    let recovery =
        recover_interrupted_recipient_exports(&source, &RootEncryptionKey::new(PROCESS_DEATH_KEY))
            .await
            .unwrap();
    assert_eq!(
        recovery.removed_count, 1,
        "test must interrupt the publication verification window"
    );
    assert_eq!(recovery.verified_destinations, vec![destination.clone()]);
    assert!(recovery.active_work.is_empty());
    assert!(recovery.unrecognized_work.is_empty());
    assert!(destination.exists());
    assert_eq!(
        describe_file(source.join("book.sqlite3")).unwrap(),
        database_before
    );
    assert_eq!(describe_file(&object_path).unwrap(), ciphertext_before);
    verify_database_snapshot(source.join("book.sqlite3"))
        .await
        .unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(source.join("book.sqlite3"))
                .read_only(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let repository = AttachmentRepository::open(
        pool.clone(),
        source.join("attachments"),
        Arc::new(RootEncryptionKey::new(PROCESS_DEATH_KEY)),
    )
    .unwrap();
    let attachment_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attachment_count, 1);
    let catalog = repository.get(&object_id).await.unwrap();
    assert_eq!(catalog.id, object_id);
    assert_eq!(catalog.plaintext, plaintext_before);
    assert_eq!(catalog.ciphertext, ciphertext_before);
    let (_, mut chunks) = repository.download_stream(&object_id).await.unwrap();
    let mut actual = Sha256::new();
    let mut count = 0u64;
    while let Some(chunk) = chunks.recv().await {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 64 * 1024);
        actual.update(&chunk);
        count += chunk.len() as u64;
    }
    let block = [0x6bu8; 64 * 1024];
    let mut expected = Sha256::new();
    for _ in 0..PROCESS_DEATH_BYTES / block.len() as u64 {
        expected.update(block);
    }
    assert_eq!(actual.finalize(), expected.finalize());
    assert_eq!(count, PROCESS_DEATH_BYTES);
    pool.close().await;
    assert_eq!(
        describe_file(source.join("book.sqlite3")).unwrap(),
        database_before
    );
}
