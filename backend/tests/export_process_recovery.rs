//! Real subprocess death during export: invented disposable book, memory keys only.
#![cfg(unix)]
use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
    attachments::AttachmentRepository,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_package::{export_recipient_package, recover_interrupted_recipient_exports},
    recipient_transfer::{RecipientKeyStore, create_recipient_request},
    runtime::RuntimeBook,
    storage::describe_file,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
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
    fn insert_new(&self, id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        let mut map = self.0.lock().unwrap();
        if map.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        map.insert(id.into(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, secret: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), secret)
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let mut id = [0; 16];
        getrandom::fill(&mut id).unwrap();
        let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ka-export-process-recovery-{id}"));
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
fn export_work(source: &Path) -> Option<PathBuf> {
    std::fs::read_dir(source)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".kAccounting-Extra-01-recipient-export-")
        })
}

#[tokio::test]
#[ignore = "subprocess helper; launched only by the explicit SIGKILL regression"]
async fn private_source_export_child() {
    use std::io::Read;
    let Some(root) = std::env::var_os("KA_EXPORT_KILL_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mut secret = Zeroizing::new([0u8; 32]);
    std::io::stdin().read_exact(secret.as_mut()).unwrap();
    let binding: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("source/device-key.json")).unwrap())
            .unwrap();
    let keys = Keys::default();
    RecipientKeyStore::insert_new(&keys, binding["key_id"].as_str().unwrap(), secret.as_ref())
        .unwrap();
    let book = RuntimeBook::open_existing(root.join("source"), &keys)
        .await
        .unwrap();
    let request = create_recipient_request(&keys).unwrap();
    export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        root.join("media/output.kagextra"),
    )
    .await
    .unwrap();
    loop {
        std::thread::park();
    }
}

#[tokio::test]
#[ignore = "actual SIGKILL regression with 128 MiB invented attachment; run explicitly"]
async fn killed_export_recovers_private_source_work_without_plaintext_on_media() {
    use std::{
        io::Write,
        os::unix::{fs::OpenOptionsExt, process::ExitStatusExt},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let scratch = Scratch::new();
    let source = scratch.0.join("source");
    let media = scratch.0.join("media");
    std::fs::create_dir(&media).unwrap();
    let keys = Keys::default();
    RuntimeBook::init_new(&source, &keys).await.unwrap();
    let book = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    let draft = JournalRepository::new(book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let input = scratch.0.join("invented-input.bin");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&input)
        .unwrap();
    let block = [0x4d; 64 * 1024];
    for _ in 0..2048 {
        file.write_all(&block).unwrap();
    }
    file.sync_all().unwrap();
    drop(file);
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key()).unwrap();
    let attachment = repository
        .import_file(draft.id, &input, "invented.bin")
        .await
        .unwrap();
    std::fs::remove_file(input).unwrap();
    drop(repository);
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(book.pool())
        .await
        .unwrap();
    let database = describe_file(source.join("book.sqlite3")).unwrap();
    let object = source
        .join("attachments")
        .join(format!("{}.kae", attachment.id.as_str()));
    let ciphertext = describe_file(&object).unwrap();
    let child_key = DeviceKeyStore::read(&keys, book.key_id()).unwrap();
    book.close().await;
    drop(book);
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "private_source_export_child", "--ignored"])
        .env("KA_EXPORT_KILL_TEST_ROOT", &scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    // Only the owned child's anonymous pipe receives the disposable memory key.
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(child_key.as_ref())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let work = loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before export encryption"
        );
        assert!(
            Instant::now() < deadline,
            "export staging deadline exceeded"
        );
        if let Some(work) = export_work(&source) {
            // Attachment export itself uses a same-directory exclusive temporary
            // object; the final .kae name appears only after complete encryption.
            let encrypting = std::fs::read_dir(work.join("package/attachments"))
                .ok()
                .is_some_and(|entries| {
                    entries.filter_map(Result::ok).any(|entry| {
                        let object = entry.path().join("object.kae");
                        std::fs::metadata(object).is_ok_and(|m| {
                            m.is_file() && m.len() > 0 && m.len() < ciphertext.byte_length() / 2
                        })
                    })
                });
            if encrypting {
                break work;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(work.join("snapshot.sqlite3").exists());
    assert!(
        std::fs::read_dir(&media).unwrap().next().is_none(),
        "plaintext/source recovery must not touch transfer media"
    );
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    assert!(work.exists());
    assert!(!media.join("output.kagextra").exists());
    assert_eq!(
        describe_file(source.join("book.sqlite3")).unwrap(),
        database
    );
    assert_eq!(describe_file(&object).unwrap(), ciphertext);
    // A new runtime acquires the prior process lock and authenticates the book.
    let reopened = RuntimeBook::open_existing(&source, &keys).await.unwrap();
    let recovery = recover_interrupted_recipient_exports(&source, &reopened.key())
        .await
        .unwrap();
    assert_eq!(recovery.removed_count, 1);
    assert!(recovery.active_work.is_empty());
    assert!(recovery.unrecognized_work.is_empty());
    assert!(recovery.verified_destinations.is_empty());
    assert!(!work.exists());
    assert!(export_work(&source).is_none());
    assert!(std::fs::read_dir(&media).unwrap().next().is_none());
    assert_eq!(
        describe_file(source.join("book.sqlite3")).unwrap(),
        database
    );
    assert_eq!(describe_file(&object).unwrap(), ciphertext);
    assert_eq!(
        JournalRepository::new(reopened.pool().clone())
            .get_draft(draft.id)
            .await
            .unwrap()
            .id,
        draft.id
    );
    reopened.close().await;
}
