// Opt-in real filesystem test. All keys are in memory; source/import stay in temporary private storage.
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
            media_root().join("transfer.kagextra"),
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

fn media_root() -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let root =
        PathBuf::from(std::env::var("KACCOUNTING_MEDIA_TEST_ROOT").expect("explicit test root"));
    let token = std::env::var("KACCOUNTING_MEDIA_TEST_TOKEN").expect("explicit ownership token");
    assert!(token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(root.canonicalize().unwrap(), root);
    assert!(
        root.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("kAccounting-Extra-01-media-test-")
    );
    let marker_path = root.join(".kAccounting-Extra-01-media-acceptance.json");
    assert!(
        !std::fs::symlink_metadata(&marker_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(marker_path).unwrap()).unwrap();
    let md = std::fs::metadata(&root).unwrap();
    assert_eq!(marker["format"], 1);
    assert_eq!(marker["token"], token);
    assert_eq!(marker["device"].as_u64(), Some(md.dev()));
    assert_eq!(marker["inode"].as_u64(), Some(md.ino()));
    root
}
#[tokio::test]
#[ignore = "requires explicit disposable APFS/exFAT root and token; use scripts/media-acceptance.py"]
async fn mounted_media_complete_encrypted_roundtrip() {
    let media = media_root();
    assert_eq!(std::fs::read_dir(&media).unwrap().count(), 1);
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
