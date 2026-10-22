use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
    attachment_maintenance::{MaintenanceError, StorePin, collect_unreferenced_ciphertext},
    attachments::AttachmentRepository,
    crypto::RootEncryptionKey,
    db::{open_database, open_memory_database},
};
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
use std::{path::PathBuf, sync::Arc};

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-gc-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn preserves_referenced_objects_and_reclaims_detached_ciphertext_only_after_readers_close() {
    let dir = Temporary::new();
    let pool = open_database(dir.0.join("book.sqlite3")).await.unwrap();
    let entry = JournalRepository::new(pool.clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let root = dir.0.join("attachments");
    let repo = AttachmentRepository::open(
        pool.clone(),
        &root,
        Arc::new(RootEncryptionKey::new([42; 32])),
    )
    .unwrap();
    let source = dir.0.join("receipt.pdf");
    std::fs::write(&source, b"%PDF-1.7\nreceipt").unwrap();
    let retained = repo
        .import_file(entry.id, &source, "first.pdf")
        .await
        .unwrap();
    let detached = repo
        .import_file(entry.id, &source, "second.pdf")
        .await
        .unwrap();
    repo.remove_from_draft(&detached.id, entry.revision)
        .await
        .unwrap();
    assert!(matches!(
        collect_unreferenced_ciphertext(&pool, &root).await,
        Err(MaintenanceError::Busy)
    ));
    let snapshot_pin = StorePin::acquire(&root).unwrap();
    let worker_pin = snapshot_pin.clone();
    drop(repo);
    drop(snapshot_pin);
    assert!(matches!(
        collect_unreferenced_ciphertext(&pool, &root).await,
        Err(MaintenanceError::Busy)
    ));
    drop(worker_pin);
    let outcome = collect_unreferenced_ciphertext(&pool, &root).await.unwrap();
    assert_eq!(outcome.removed_objects, 1);
    assert_eq!(outcome.removed_bytes, detached.ciphertext.byte_length());
    assert!(root.join(format!("{}.kae", retained.id.as_str())).is_file());
    assert!(!root.join(format!("{}.kae", detached.id.as_str())).exists());
    assert_eq!(
        collect_unreferenced_ciphertext(&pool, &root)
            .await
            .unwrap()
            .removed_objects,
        0
    );
}

#[tokio::test]
#[cfg(unix)]
async fn unknown_nonprivate_symlink_and_hardlink_entries_are_preserved() {
    let dir = Temporary::new();
    let pool = open_database(dir.0.join("book.sqlite3")).await.unwrap();
    let root = dir.0.join("attachments");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(root.join("notes.txt"), b"preserve").unwrap();
    std::fs::create_dir(root.join(format!("{}.kae", "1".repeat(32)))).unwrap();
    let target = dir.0.join("outside");
    std::fs::write(&target, b"private outside").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, root.join(format!("{}.kae", "2".repeat(32)))).unwrap();
    std::fs::hard_link(&target, root.join(format!("{}.kae", "3".repeat(32)))).unwrap();
    let public = root.join(format!("{}.kae", "4".repeat(32)));
    std::fs::write(&public, b"public").unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o644)).unwrap();
    let outcome = collect_unreferenced_ciphertext(&pool, &root).await.unwrap();
    assert_eq!(outcome.removed_objects, 0);
    assert_eq!(outcome.preserved_unknown_entries, 5);
    assert_eq!(std::fs::read(&target).unwrap(), b"private outside");
}

#[tokio::test]
#[cfg(unix)]
async fn unsafe_store_and_replaced_lock_fail_closed() {
    let dir = Temporary::new();
    let pool = open_database(dir.0.join("book.sqlite3")).await.unwrap();
    let root = dir.0.join("attachments");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        StorePin::acquire(&root),
        Err(MaintenanceError::UnsafeStore)
    ));
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pin = StorePin::acquire(&root).unwrap();
    std::fs::remove_file(root.join(".maintenance.lock")).unwrap();
    std::fs::write(root.join(".maintenance.lock"), b"replacement").unwrap();
    assert!(pin.validate().is_err());
    assert!(collect_unreferenced_ciphertext(&pool, &root).await.is_err());
}

#[test]
fn subprocess_shared_pin_helper() {
    let Some(root) = std::env::var_os("KACCOUNTING_TEST_PIN_ROOT") else {
        return;
    };
    let _pin = StorePin::acquire(PathBuf::from(root)).unwrap();
    use std::io::{Read, Write};
    println!("STORE_PIN_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte).unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn independent_process_snapshot_pin_blocks_collection_until_process_exits() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let dir = Temporary::new();
    let pool = open_database(dir.0.join("book.sqlite3")).await.unwrap();
    let root = dir.0.join("attachments");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("subprocess_shared_pin_helper")
        .arg("--nocapture")
        .env("KACCOUNTING_TEST_PIN_ROOT", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut ready = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.contains("STORE_PIN_READY") {
            ready = true;
            break;
        }
    }
    assert!(ready, "subprocess must hold the shared OS file lock");
    assert!(matches!(
        collect_unreferenced_ciphertext(&pool, &root).await,
        Err(MaintenanceError::Busy)
    ));
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    collect_unreferenced_ciphertext(&pool, &root).await.unwrap();
}

#[tokio::test]
async fn foreign_memory_database_and_wrong_store_layout_cannot_delete_another_books_objects() {
    let dir = Temporary::new();
    let pool = open_database(dir.0.join("book.sqlite3")).await.unwrap();
    let entry = JournalRepository::new(pool.clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let root = dir.0.join("attachments");
    let repo = AttachmentRepository::open(
        pool.clone(),
        &root,
        Arc::new(RootEncryptionKey::new([42; 32])),
    )
    .unwrap();
    let source = dir.0.join("receipt.pdf");
    std::fs::write(&source, b"%PDF-1.7\nreceipt").unwrap();
    let attachment = repo
        .import_file(entry.id, &source, "receipt.pdf")
        .await
        .unwrap();
    drop(repo);
    let object = root.join(format!("{}.kae", attachment.id.as_str()));
    let original = std::fs::read(&object).unwrap();
    let foreign = open_database(dir.0.join("unrelated.sqlite3"))
        .await
        .unwrap();
    let memory = open_memory_database().await.unwrap();
    for other in [&foreign, &memory] {
        assert!(matches!(
            collect_unreferenced_ciphertext(other, &root).await,
            Err(MaintenanceError::MismatchedDatabase)
        ));
        assert_eq!(std::fs::read(&object).unwrap(), original);
    }
    let renamed = dir.0.join("other-objects");
    std::fs::rename(&root, &renamed).unwrap();
    assert!(matches!(
        collect_unreferenced_ciphertext(&pool, &renamed).await,
        Err(MaintenanceError::MismatchedDatabase)
    ));
    assert_eq!(
        std::fs::read(renamed.join(object.file_name().unwrap())).unwrap(),
        original
    );
}
