use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
    attachments::AttachmentRepository,
    crypto::RootEncryptionKey,
    db::open_database,
    keys::KeyStoreError,
    keys::{DeviceKeyId, DeviceKeyStore},
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
    keys: Arc<MemoryKeys>,
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
            keys: Arc::new(keys),
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
            self.keys.as_ref(),
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
use kaccounting_backend::{container_import::*, storage::describe_file, transfer_container::*};
use std::io::{Cursor, Read};
fn encode(package: &std::path::Path) -> Vec<u8> {
    let mut entries = Vec::new();
    for name in ["book.kae", "envelope.json", "manifest.kae"] {
        entries
            .push(ContainerEntry::new(name, describe_file(package.join(name)).unwrap()).unwrap());
    }
    if let Ok(files) = std::fs::read_dir(package.join("attachments")) {
        for file in files {
            let file = file.unwrap();
            let name = format!("attachments/{}", file.file_name().to_str().unwrap());
            entries.push(ContainerEntry::new(name, describe_file(file.path()).unwrap()).unwrap());
        }
    }
    let mut output = Vec::new();
    write_container(package, &entries, &mut output).unwrap();
    output
}
struct Bounded(Cursor<Vec<u8>>);
impl Read for Bounded {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        assert!(bytes.len() <= COPY_BUFFER_BYTES);
        self.0.read(bytes)
    }
}
async fn run(
    f: &Fixture,
    bytes: Vec<u8>,
    name: &str,
) -> Result<kaccounting_backend::recipient_import::RecipientImportOutcome, ContainerImportError> {
    import_recipient_container(
        Bounded(Cursor::new(bytes)),
        f.root.clone(),
        f.root.join(name),
        f.request.clone(),
        f.keys.clone(),
        f.keys.clone(),
        ContainerLimits::default(),
    )
    .await
}
#[tokio::test]
async fn full_attachment_roundtrip_is_bounded_and_opens_with_fresh_keys() {
    let f = Fixture::new().await;
    f.attach().await;
    let exported = f.export().await;
    let bytes = encode(&exported.path);
    let outcome = run(&f, bytes, "imported").await.unwrap();
    assert_eq!(outcome.attachment_count, 1);
    assert!(outcome.cleanup_warning.is_none());
    assert!(!std::fs::read_dir(&f.root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".kAccounting-Extra-01-container-import-")
    }));
    assert!(exported.path.join("book.kae").is_file());
    let runtime = RuntimeBook::open_existing(&outcome.path, f.keys.as_ref())
        .await
        .unwrap();
    runtime.close().await;
    f.finish().await;
}
#[tokio::test]
async fn outer_tampering_and_incomplete_body_do_not_create_keys_or_book() {
    let f = Fixture::new().await;
    let exported = f.export().await;
    let original = encode(&exported.path);
    let before = f.keys.0.lock().unwrap().len();
    let mut corrupt = original.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(run(&f, corrupt, "tampered").await.is_err());
    assert!(
        run(&f, original[..original.len() - 1].to_vec(), "short")
            .await
            .is_err()
    );
    assert_eq!(f.keys.0.lock().unwrap().len(), before);
    assert!(!f.root.join("tampered").exists());
    assert!(!f.root.join("short").exists());
    assert!(std::fs::read_dir(&f.root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".kAccounting-Extra-01-container-import-")
    }));
    f.finish().await;
}
#[tokio::test]
async fn authenticated_inner_tampering_is_rejected_before_publication() {
    let f = Fixture::new().await;
    let exported = f.export().await;
    let path = exported.path.join("book.kae");
    let mut book = std::fs::read(&path).unwrap();
    let last = book.len() - 1;
    book[last] ^= 1;
    std::fs::write(&path, book).unwrap();
    assert!(run(&f, encode(&exported.path), "bad-inner").await.is_err());
    assert!(!f.root.join("bad-inner").exists());
    f.finish().await;
}
#[tokio::test]
async fn wrong_recipient_and_destination_conflict_preserve_existing_data() {
    let f = Fixture::new().await;
    let exported = f.export().await;
    let request = create_recipient_request(f.keys.as_ref()).unwrap();
    let before = f.keys.0.lock().unwrap().len();
    assert!(
        import_recipient_container(
            Cursor::new(encode(&exported.path)),
            f.root.clone(),
            f.root.join("wrong"),
            request,
            f.keys.clone(),
            f.keys.clone(),
            ContainerLimits::default()
        )
        .await
        .is_err()
    );
    assert_eq!(before, f.keys.0.lock().unwrap().len());
    std::fs::create_dir(f.root.join("existing")).unwrap();
    std::fs::write(f.root.join("existing/sentinel"), b"keep").unwrap();
    assert!(run(&f, encode(&exported.path), "existing").await.is_err());
    assert_eq!(
        std::fs::read(f.root.join("existing/sentinel")).unwrap(),
        b"keep"
    );
    f.finish().await;
}
#[tokio::test]
async fn disconnect_does_not_drop_reader_while_blocking_read_is_active() {
    struct BlockingReader {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        finished: Option<tokio::sync::oneshot::Sender<()>>,
    }
    impl Read for BlockingReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
            }
            self.release.recv().unwrap();
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "disconnected",
            ))
        }
    }
    impl Drop for BlockingReader {
        fn drop(&mut self) {
            if let Some(done) = self.finished.take() {
                let _ = done.send(());
            }
        }
    }
    let f = Fixture::new().await;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let future = import_recipient_container(
        BlockingReader {
            entered: Some(entered_tx),
            release: release_rx,
            finished: Some(done_tx),
        },
        f.root.clone(),
        f.root.join("cancelled"),
        f.request.clone(),
        f.keys.clone(),
        f.keys.clone(),
        ContainerLimits::default(),
    );
    let caller = tokio::spawn(future);
    entered_rx.await.unwrap();
    caller.abort();
    let _ = caller.await;
    release_tx.send(()).unwrap();
    done_rx.await.unwrap();
    assert!(!f.root.join("cancelled").exists());
    // The detached operation can still be finishing; retain its private root.
    f.pool.close().await;
}
#[tokio::test]
async fn caller_capacity_limit_rejects_before_any_key_creation() {
    let f = Fixture::new().await;
    let exported = f.export().await;
    let before = f.keys.0.lock().unwrap().len();
    let result = import_recipient_container(
        Cursor::new(encode(&exported.path)),
        f.root.clone(),
        f.root.join("capped"),
        f.request.clone(),
        f.keys.clone(),
        f.keys.clone(),
        ContainerLimits {
            max_entries: 2,
            max_object_bytes: 1,
            max_total_bytes: 1,
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(ContainerImportError::Container(ContainerError::Capacity))
    ));
    assert_eq!(before, f.keys.0.lock().unwrap().len());
    assert!(!f.root.join("capped").exists());
    f.finish().await;
}

#[tokio::test]
async fn empty_book_container_import_creates_attachment_directory_and_opens() {
    let f = Fixture::new().await;
    let exported = f.export().await;
    assert_eq!(exported.attachment_count, 0);
    let bytes = encode(&exported.path);
    let outcome = run(&f, bytes, "empty-book").await.unwrap();
    assert_eq!(outcome.attachment_count, 0);
    assert!(outcome.path.join("attachments").is_dir());
    let runtime = RuntimeBook::open_existing(&outcome.path, f.keys.as_ref())
        .await
        .unwrap();
    runtime.close().await;
    f.finish().await;
}
