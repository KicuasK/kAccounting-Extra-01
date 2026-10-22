use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalEntryId, JournalLineInput, JournalRepository, JournalSide,
            PostEntry, SaveDraftEntry,
        },
        money::MinorUnits,
    },
    attachments::{Attachment, AttachmentRepository},
    book_backup::{BookBackupError, create_local_backup, restore_local_backup},
    crypto::{EncryptedObjectPurpose, decrypt_object, encrypt_object},
    db::{open_database, verify_database_integrity},
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, create_device_key, load_device_key},
    storage::describe_file,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct MemoryKeys(Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>);
impl DeviceKeyStore for MemoryKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().unwrap();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.clone(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    objects: PathBuf,
    source: PathBuf,
    pool: sqlx::SqlitePool,
    keys: MemoryKeys,
    key_id: DeviceKeyId,
    attachments: AttachmentRepository,
    posted: JournalEntryId,
    draft: JournalEntryId,
}
impl Fixture {
    async fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("kAccounting-Extra-01-backup-test-{id}"));
        std::fs::create_dir(&root).unwrap();
        let pool = open_database(root.join("live.sqlite3")).await.unwrap();
        let keys = MemoryKeys::default();
        let (key_id, key) = create_device_key(&keys).unwrap();
        let accounts = AccountRepository::new(pool.clone());
        let mut lines = Vec::new();
        for (name, element, side) in [
            ("銀行 Bank 01", AccountingElement::Asset, JournalSide::Debit),
            (
                "Opening equity",
                AccountingElement::Equity,
                JournalSide::Credit,
            ),
        ] {
            let a = accounts
                .create(NewAccount::new(None, name, element, AccountKind::Posting))
                .await
                .unwrap();
            lines.push(
                JournalLineInput::new(a.id, side, MinorUnits::new(9007199254740993))
                    .with_memo("opening memo"),
            );
        }
        let journal = JournalRepository::new(pool.clone());
        let posted = journal
            .post(
                PostEntry::new(JournalDate::parse("2026-10-22").unwrap(), lines)
                    .with_header(Some("開帳 001".into()), Some("opening explanation".into())),
            )
            .await
            .unwrap()
            .id;
        let draft = journal
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![],
            ))
            .await
            .unwrap()
            .id;
        let objects = root.join("live-attachments");
        let attachments =
            AttachmentRepository::open(pool.clone(), &objects, Arc::new(key)).unwrap();
        let source = root.join("receipt.pdf");
        std::fs::write(&source, b"%PDF-1.7\nPrivate receipt bytes").unwrap();
        Self {
            root,
            objects,
            source,
            pool,
            keys,
            key_id,
            attachments,
            posted,
            draft,
        }
    }
    async fn attach(&self, entry: JournalEntryId, name: &str) -> Attachment {
        self.attachments
            .import_file(entry, &self.source, name)
            .await
            .unwrap()
    }
    async fn backup(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        create_local_backup(&self.pool, &self.objects, &path, &self.key_id, &self.keys)
            .await
            .unwrap();
        path
    }
    fn assert_no_work(&self) {
        for entry in std::fs::read_dir(&self.root).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name
                    .to_string_lossy()
                    .starts_with(".kAccounting-Extra-01-encrypted-backup-"),
                "no plaintext or private work may remain after completion/failure"
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn encrypted_backup_and_new_destination_restore_reproduce_posted_drafts_and_all_attachments()
{
    let f = Fixture::new().await;
    let a = f.attach(f.posted, "posted receipt.pdf").await;
    let b = f.attach(f.draft, "草稿 Bank 01.pdf").await;
    let before = JournalRepository::new(f.pool.clone())
        .get_posted(f.posted)
        .await
        .unwrap();
    let backup = f.root.join("first.kabackup");
    let created = create_local_backup(&f.pool, &f.objects, &backup, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert_eq!(created.attachment_count, 2);
    assert_eq!(
        created.attachment_bytes,
        a.plaintext.byte_length() + b.plaintext.byte_length()
    );
    let encrypted = std::fs::read(backup.join("book.kae")).unwrap();
    assert!(encrypted.starts_with(b"KAEOBJ01"));
    assert!(!encrypted.windows(16).any(|w| w == b"SQLite format 3\0"));
    assert!(!backup.join("book.sqlite3").exists());
    assert!(!backup.join("snapshot.sqlite3").exists());
    f.assert_no_work();
    let original_backup = describe_file(backup.join("book.kae")).unwrap();
    let destination = f.root.join("restored-book");
    let restored = restore_local_backup(&backup, &destination, &f.keys)
        .await
        .unwrap();
    assert_eq!(restored.attachment_count, 2);
    assert_eq!(restored.key_id, f.key_id);
    let pool = open_database(destination.join("book.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        JournalRepository::new(pool.clone())
            .get_posted(f.posted)
            .await
            .unwrap(),
        before
    );
    assert!(
        JournalRepository::new(pool.clone())
            .get_draft(f.draft)
            .await
            .is_ok()
    );
    let objects = AttachmentRepository::open(
        pool.clone(),
        destination.join("attachments"),
        Arc::new(load_device_key(&f.keys, &f.key_id).unwrap()),
    )
    .unwrap();
    for attachment in [&a, &b] {
        assert_eq!(objects.get(&attachment.id).await.unwrap(), *attachment);
        objects.verify(&attachment.id).await.unwrap();
    }
    verify_database_integrity(&pool).await.unwrap();
    pool.close().await;
    assert_eq!(
        describe_file(backup.join("book.kae")).unwrap(),
        original_backup
    );
    f.assert_no_work();
}

#[tokio::test]
async fn snapshot_selection_excludes_unreferenced_retained_objects_and_keeps_zero_attachment_books_valid()
 {
    let f = Fixture::new().await;
    let stale = f.attach(f.draft, "removed.pdf").await;
    f.attachments.remove_from_draft(&stale.id, 1).await.unwrap();
    assert!(
        f.objects
            .join(format!("{}.kae", stale.id.as_str()))
            .exists()
    );
    let backup = f.root.join("empty.kabackup");
    let outcome = create_local_backup(&f.pool, &f.objects, &backup, &f.key_id, &f.keys)
        .await
        .unwrap();
    assert_eq!(outcome.attachment_count, 0);
    assert_eq!(
        std::fs::read_dir(backup.join("attachments"))
            .unwrap()
            .count(),
        0
    );
    restore_local_backup(&backup, f.root.join("restored"), &f.keys)
        .await
        .unwrap();
    f.assert_no_work();
}

#[tokio::test]
async fn missing_live_attachment_aborts_backup_without_publishing_or_leaving_plaintext() {
    let f = Fixture::new().await;
    let a = f.attach(f.posted, "receipt.pdf").await;
    std::fs::remove_file(f.objects.join(format!("{}.kae", a.id.as_str()))).unwrap();
    let destination = f.root.join("missing.kabackup");
    assert!(
        create_local_backup(&f.pool, &f.objects, &destination, &f.key_id, &f.keys)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    f.assert_no_work();
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn existing_backup_restore_destinations_and_symlinks_are_preserved() {
    let f = Fixture::new().await;
    let backup = f.backup("existing.kabackup").await;
    let before = describe_file(backup.join("book.kae")).unwrap();
    assert!(matches!(
        create_local_backup(&f.pool, &f.objects, &backup, &f.key_id, &f.keys).await,
        Err(BookBackupError::DestinationExists)
    ));
    assert_eq!(describe_file(backup.join("book.kae")).unwrap(), before);
    let destination = f.root.join("existing-book");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(matches!(
        restore_local_backup(&backup, &destination, &f.keys).await,
        Err(BookBackupError::DestinationExists)
    ));
    assert_eq!(
        std::fs::read(destination.join("sentinel")).unwrap(),
        b"keep"
    );
    #[cfg(unix)]
    {
        let link = f.root.join("link");
        std::os::unix::fs::symlink(&destination, &link).unwrap();
        assert!(matches!(
            restore_local_backup(&backup, &link, &f.keys).await,
            Err(BookBackupError::DestinationExists)
        ));
    }
    f.assert_no_work();
}

#[tokio::test]
async fn missing_wrong_and_malformed_device_keys_fail_closed_without_creating_destinations() {
    let f = Fixture::new().await;
    let backup = f.backup("keys.kabackup").await;
    let destination = f.root.join("restored");
    let missing = MemoryKeys::default();
    assert!(matches!(
        restore_local_backup(&backup, &destination, &missing).await,
        Err(BookBackupError::Key(KeyStoreError::Missing))
    ));
    let wrong = MemoryKeys::default();
    wrong.insert_new(&f.key_id, &[99; 32]).unwrap();
    assert!(
        restore_local_backup(&backup, &destination, &wrong)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    let malformed = MemoryKeys::default();
    malformed.insert_new(&f.key_id, &[99; 31]).unwrap();
    assert!(matches!(
        restore_local_backup(&backup, &destination, &malformed).await,
        Err(BookBackupError::Key(KeyStoreError::MalformedSecret))
    ));
    f.assert_no_work();
}

#[tokio::test]
async fn corrupted_database_catalog_or_attachment_ciphertext_never_publish_a_restore() {
    let f = Fixture::new().await;
    let a = f.attach(f.posted, "receipt.pdf").await;
    for (i, file) in [
        PathBuf::from("book.kae"),
        PathBuf::from("catalog.kae"),
        PathBuf::from(format!("attachments/{}.kae", a.id.as_str())),
    ]
    .into_iter()
    .enumerate()
    {
        let backup = f.backup(&format!("corrupt-{i}.kabackup")).await;
        let path = backup.join(file);
        let mut bytes = std::fs::read(&path).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        let destination = f.root.join(format!("failed-{i}"));
        assert!(
            restore_local_backup(&backup, &destination, &f.keys)
                .await
                .is_err()
        );
        assert!(!destination.exists());
        f.assert_no_work();
    }
}

#[tokio::test]
async fn extra_missing_and_symlinked_objects_are_rejected_as_incomplete_or_unsafe() {
    let f = Fixture::new().await;
    let a = f.attach(f.posted, "receipt.pdf").await;
    let extra = f.backup("extra.kabackup").await;
    std::fs::write(extra.join("unlisted"), b"unexpected").unwrap();
    assert!(
        restore_local_backup(&extra, f.root.join("fail-extra"), &f.keys)
            .await
            .is_err()
    );
    let missing = f.backup("missing.kabackup").await;
    std::fs::remove_file(
        missing
            .join("attachments")
            .join(format!("{}.kae", a.id.as_str())),
    )
    .unwrap();
    assert!(
        restore_local_backup(&missing, f.root.join("fail-missing"), &f.keys)
            .await
            .is_err()
    );
    #[cfg(unix)]
    {
        let linked = f.backup("linked.kabackup").await;
        std::fs::remove_file(linked.join("catalog.kae")).unwrap();
        std::os::unix::fs::symlink(extra.join("catalog.kae"), linked.join("catalog.kae")).unwrap();
        assert!(
            restore_local_backup(&linked, f.root.join("fail-linked"), &f.keys)
                .await
                .is_err()
        );
    }
    f.assert_no_work();
}

#[tokio::test]
async fn unsupported_duplicate_and_unknown_envelope_fields_fail_before_restore_writes() {
    let f = Fixture::new().await;
    let backup = f.backup("format.kabackup").await;
    let envelope = backup.join("envelope.json");
    let valid = std::fs::read_to_string(&envelope).unwrap();
    let base: serde_json::Value = serde_json::from_str(&valid).unwrap();
    let mut future = base.clone();
    future["version"] = serde_json::json!(999);
    let mut unknown = base.clone();
    unknown["extra"] = serde_json::json!("no");
    for (index, value) in [
        future.to_string(),
        unknown.to_string(),
        valid.replacen("{", "{\"version\":1,", 1),
        "not JSON".into(),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(&envelope, value).unwrap();
        let destination = f.root.join(format!("rejected-{index}"));
        assert!(
            restore_local_backup(&backup, &destination, &f.keys)
                .await
                .is_err()
        );
        assert!(!destination.exists());
    }
    f.assert_no_work();
}

#[tokio::test]
async fn public_key_identifier_is_bound_to_the_authenticated_catalog() {
    let f = Fixture::new().await;
    let backup = f.backup("binding.kabackup").await;
    let alias = DeviceKeyId::parse(&"a".repeat(32)).unwrap();
    let bytes = f.keys.read(&f.key_id).unwrap();
    f.keys.insert_new(&alias, &bytes).unwrap();
    let path = backup.join("envelope.json");
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    envelope["key_id"] = serde_json::json!(alias.as_str());
    std::fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(
        restore_local_backup(&backup, f.root.join("rejected"), &f.keys)
            .await
            .is_err()
    );
    f.assert_no_work();
}

#[tokio::test]
async fn even_authenticated_catalog_cannot_omit_a_database_attachment_reference() {
    let f = Fixture::new().await;
    let a = f.attach(f.posted, "receipt.pdf").await;
    let backup = f.backup("omitted.kabackup").await;
    let key = load_device_key(&f.keys, &f.key_id).unwrap();
    let path = backup.join("catalog.kae");
    let mut plain = Vec::new();
    decrypt_object(
        &mut std::fs::File::open(&path).unwrap(),
        &mut plain,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )
    .unwrap();
    let mut catalog: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    catalog["attachments"] = serde_json::json!([]);
    let mut encrypted = Vec::new();
    encrypt_object(
        &mut std::io::Cursor::new(serde_json::to_vec(&catalog).unwrap()),
        &mut encrypted,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )
    .unwrap();
    std::fs::write(&path, encrypted).unwrap();
    std::fs::remove_file(
        backup
            .join("attachments")
            .join(format!("{}.kae", a.id.as_str())),
    )
    .unwrap();
    assert!(matches!(
        restore_local_backup(&backup, f.root.join("rejected"), &f.keys).await,
        Err(BookBackupError::CatalogMismatch)
    ));
    f.assert_no_work();
}

#[tokio::test]
async fn concurrent_backup_publication_has_one_winner_and_preserves_the_live_book() {
    let f = Fixture::new().await;
    f.attach(f.posted, "receipt.pdf").await;
    let path = f.root.join("race.kabackup");
    let (a, b) = tokio::join!(
        create_local_backup(&f.pool, &f.objects, &path, &f.key_id, &f.keys),
        create_local_backup(&f.pool, &f.objects, &path, &f.key_id, &f.keys)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    restore_local_backup(&path, f.root.join("restored"), &f.keys)
        .await
        .unwrap();
    verify_database_integrity(&f.pool).await.unwrap();
    f.assert_no_work();
}

#[tokio::test]
async fn high_volume_backup_restores_every_selected_object_without_manual_selection() {
    let f = Fixture::new().await;
    for i in 0..128 {
        f.attach(
            if i % 2 == 0 { f.draft } else { f.posted },
            &format!("收據 {i:04}.pdf"),
        )
        .await;
    }
    let backup = f.backup("many.kabackup").await;
    let outcome = restore_local_backup(&backup, f.root.join("restored"), &f.keys)
        .await
        .unwrap();
    assert_eq!(outcome.attachment_count, 128);
    assert_eq!(
        std::fs::read_dir(outcome.path.join("attachments"))
            .unwrap()
            .count(),
        128
    );
    f.assert_no_work();
}
