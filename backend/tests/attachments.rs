use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{
            JournalDate, JournalEntryId, JournalLineInput, JournalRepository, JournalSide,
            PostEntry, SaveDraftEntry,
        },
        money::MinorUnits,
    },
    attachments::{
        AttachmentError, AttachmentId, AttachmentRepository, MAX_ATTACHMENT_BYTES,
        MAX_BOOK_ATTACHMENT_BYTES,
    },
    crypto::RootEncryptionKey,
    db::{open_memory_database, verify_database_integrity},
    storage::describe_file,
};
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    objects: PathBuf,
    pool: sqlx::SqlitePool,
    repo: AttachmentRepository,
    entry: JournalEntryId,
}
impl Fixture {
    async fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root =
            std::env::temp_dir().join(format!("kAccounting-Extra-01-attachment-test-{name}"));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("receipt.pdf");
        std::fs::write(&source, b"%PDF-1.7\nPrivate receipt evidence\n").unwrap();
        let objects = root.join("objects");
        let pool = open_memory_database().await.unwrap();
        let entry = JournalRepository::new(pool.clone())
            .create_draft(SaveDraftEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                vec![],
            ))
            .await
            .unwrap()
            .id;
        let repo = AttachmentRepository::open(
            pool.clone(),
            &objects,
            Arc::new(RootEncryptionKey::new([42; 32])),
        )
        .unwrap();
        Self {
            root,
            source,
            objects,
            pool,
            repo,
            entry,
        }
    }
    fn object(&self, id: &AttachmentId) -> PathBuf {
        self.objects.join(format!("{}.kae", id.as_str()))
    }
    async fn post(&self) -> JournalEntryId {
        let accounts = AccountRepository::new(self.pool.clone());
        let mut lines = Vec::new();
        for (name, side) in [("Cash", JournalSide::Debit), ("Bank", JournalSide::Credit)] {
            let account = accounts
                .create(NewAccount::new(
                    None,
                    name,
                    AccountingElement::Asset,
                    AccountKind::Posting,
                ))
                .await
                .unwrap();
            lines.push(JournalLineInput::new(account.id, side, MinorUnits::new(10)));
        }
        JournalRepository::new(self.pool.clone())
            .post(PostEntry::new(
                JournalDate::parse("2026-10-22").unwrap(),
                lines,
            ))
            .await
            .unwrap()
            .id
    }
    async fn fake_bytes(&self, index: u32, length: u64) -> Result<(), sqlx::Error> {
        let encrypted_length = length + 72 + 20 * length.div_ceil(65536).max(1);
        sqlx::query("INSERT INTO attachments (id, entry_id, display_name, plaintext_sha256, byte_length, ciphertext_sha256, ciphertext_byte_length) VALUES (?, ?, 'quota fixture', ?, ?, ?, ?)")
            .bind(format!("{index:032x}")).bind(self.entry.get()).bind("0".repeat(64)).bind(length as i64).bind("1".repeat(64)).bind(encrypted_length as i64).execute(&self.pool).await.map(|_| ())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn imported_objects_are_encrypted_private_verified_and_entry_owned() {
    let f = Fixture::new().await;
    let a = f
        .repo
        .import_file(f.entry, &f.source, "收據 Bank 01.pdf")
        .await
        .unwrap();
    assert_eq!(a.plaintext, describe_file(&f.source).unwrap());
    assert_eq!(a.ciphertext, describe_file(f.object(&a.id)).unwrap());
    let bytes = std::fs::read(f.object(&a.id)).unwrap();
    assert!(bytes.starts_with(b"KAEOBJ01"));
    assert!(
        !bytes
            .windows(b"Private receipt evidence".len())
            .any(|w| w == b"Private receipt evidence")
    );
    assert_eq!(f.repo.get(&a.id).await.unwrap(), a);
    assert_eq!(
        f.repo.list_for_entry(f.entry).await.unwrap(),
        vec![a.clone()]
    );
    assert_eq!(f.repo.list_all().await.unwrap(), vec![a.clone()]);
    f.repo.verify(&a.id).await.unwrap();
    assert_eq!(
        std::fs::read_dir(&f.objects).unwrap().count(),
        2,
        "no work or plaintext residue"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&f.objects).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(f.object(&a.id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn identical_content_has_independent_immutable_objects_without_deduplication() {
    let f = Fixture::new().await;
    let a = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    let b = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.plaintext, b.plaintext);
    assert_ne!(a.ciphertext.digest(), b.ciphertext.digest());
    assert_eq!(f.repo.list_for_entry(f.entry).await.unwrap(), vec![a, b]);
}

#[tokio::test]
async fn empty_and_chunk_boundary_files_match_the_sqlite_framing_contract() {
    let f = Fixture::new().await;
    for length in [0, 1, 65535, 65536, 65537, 131072, 131073] {
        std::fs::write(&f.source, vec![0x5a; length]).unwrap();
        let a = f
            .repo
            .import_file(f.entry, &f.source, "evidence.bin")
            .await
            .unwrap();
        assert_eq!(a.plaintext.byte_length(), length as u64);
        f.repo.verify(&a.id).await.unwrap();
    }
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn posted_entries_allow_new_attachments_but_block_removal_replacement_and_owner_changes() {
    let f = Fixture::new().await;
    let posted = f.post().await;
    let a = f
        .repo
        .import_file(posted, &f.source, "receipt.pdf")
        .await
        .unwrap();
    assert!(matches!(
        f.repo.remove_from_draft(&a.id, 2).await,
        Err(AttachmentError::PostedImmutable)
    ));
    for query in [
        "DELETE FROM attachments WHERE id = ?",
        "UPDATE attachments SET display_name = 'changed' WHERE id = ?",
        "UPDATE attachments SET entry_id = 1 WHERE id = ?",
        "UPDATE attachments SET plaintext_sha256 = lower(hex(zeroblob(32))) WHERE id = ?",
    ] {
        assert!(
            sqlx::query(query)
                .bind(a.id.as_str())
                .execute(&f.pool)
                .await
                .is_err()
        );
    }
    assert_eq!(f.repo.get(&a.id).await.unwrap(), a);
    f.repo.verify(&a.id).await.unwrap();
}

#[tokio::test]
async fn removing_draft_attachment_uses_revision_and_keeps_bytes_for_snapshot_readers() {
    let f = Fixture::new().await;
    let a = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    assert!(matches!(
        f.repo.remove_from_draft(&a.id, 2).await,
        Err(AttachmentError::StaleDraft)
    ));
    assert_eq!(f.repo.get(&a.id).await.unwrap(), a);
    f.repo.remove_from_draft(&a.id, 1).await.unwrap();
    assert!(matches!(
        f.repo.get(&a.id).await,
        Err(AttachmentError::NotFound)
    ));
    assert_eq!(
        JournalRepository::new(f.pool.clone())
            .get_draft(f.entry)
            .await
            .unwrap()
            .revision,
        2
    );
    assert_eq!(describe_file(f.object(&a.id)).unwrap(), a.ciphertext);
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn draft_replacement_preserves_attachments_and_deletion_removes_only_its_references() {
    let f = Fixture::new().await;
    let journal = JournalRepository::new(f.pool.clone());
    let other = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![],
        ))
        .await
        .unwrap();
    let a = f
        .repo
        .import_file(f.entry, &f.source, "a.pdf")
        .await
        .unwrap();
    let b = f
        .repo
        .import_file(other.id, &f.source, "b.pdf")
        .await
        .unwrap();
    let draft = journal
        .replace_draft(
            f.entry,
            1,
            SaveDraftEntry::new(JournalDate::parse("2026-10-21").unwrap(), vec![]),
        )
        .await
        .unwrap();
    assert_eq!(
        f.repo.list_for_entry(f.entry).await.unwrap(),
        vec![a.clone()]
    );
    journal.delete_draft(f.entry, draft.revision).await.unwrap();
    assert!(matches!(
        f.repo.get(&a.id).await,
        Err(AttachmentError::NotFound)
    ));
    assert_eq!(f.repo.get(&b.id).await.unwrap(), b);
    f.repo.verify(&b.id).await.unwrap();
    assert!(f.object(&a.id).exists());
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn wrong_key_tampering_truncation_and_missing_objects_fail_without_plaintext_output() {
    let f = Fixture::new().await;
    let a = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    let wrong = AttachmentRepository::open(
        f.pool.clone(),
        &f.objects,
        Arc::new(RootEncryptionKey::new([41; 32])),
    )
    .unwrap();
    assert!(matches!(
        wrong.verify(&a.id).await,
        Err(AttachmentError::Crypto(_))
    ));
    let original = std::fs::read(f.object(&a.id)).unwrap();
    let mut modified = original.clone();
    modified[80] ^= 1;
    std::fs::write(f.object(&a.id), modified).unwrap();
    assert!(f.repo.verify(&a.id).await.is_err());
    std::fs::write(f.object(&a.id), &original[..original.len() - 1]).unwrap();
    assert!(f.repo.verify(&a.id).await.is_err());
    std::fs::remove_file(f.object(&a.id)).unwrap();
    assert!(f.repo.verify(&a.id).await.is_err());
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 1);
}

#[tokio::test]
async fn rejects_executables_packages_directories_and_unsafe_names_before_publication() {
    let f = Fixture::new().await;
    for name in [
        "",
        ".",
        "..",
        "../receipt",
        "folder/file",
        "folder\\file",
        "bad\0name",
        "bad\nname",
        "bad\u{85}name",
    ] {
        assert!(matches!(
            f.repo.import_file(f.entry, &f.source, name).await,
            Err(AttachmentError::InvalidName)
        ));
    }
    for name in ["app.EXE", "app.pkg", "run.command", "app.APP", "app.scpt"] {
        assert!(matches!(
            f.repo.import_file(f.entry, &f.source, name).await,
            Err(AttachmentError::UnsafeSource)
        ));
    }
    assert!(matches!(
        f.repo.import_file(f.entry, &f.root, "directory").await,
        Err(AttachmentError::UnsafeSource)
    ));
    for bytes in [
        b"#!/bin/zsh\n".as_slice(),
        b"MZbinary",
        b"\x7fELFbinary",
        b"\xcf\xfa\xed\xfebinary",
        b"\xca\xfe\xba\xbfbinary",
    ] {
        std::fs::write(&f.source, bytes).unwrap();
        assert!(matches!(
            f.repo.import_file(f.entry, &f.source, "innocent.pdf").await,
            Err(AttachmentError::UnsafeSource)
        ));
    }
    assert!(f.repo.list_all().await.unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 1);
}

#[tokio::test]
async fn rejects_oversize_sources_before_reading_and_enforces_exact_metadata_limits() {
    let f = Fixture::new().await;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&f.source)
        .unwrap()
        .set_len(MAX_ATTACHMENT_BYTES + 1)
        .unwrap();
    assert!(matches!(
        f.repo.import_file(f.entry, &f.source, "large.pdf").await,
        Err(AttachmentError::FileTooLarge)
    ));
    assert!(f.fake_bytes(1, MAX_ATTACHMENT_BYTES + 1).await.is_err());
    f.fake_bytes(2, MAX_ATTACHMENT_BYTES).await.unwrap();
    assert!(sqlx::query("INSERT INTO attachments SELECT '00000000000000000000000000000003', entry_id, display_name, plaintext_sha256, byte_length, ciphertext_sha256, ciphertext_byte_length + 1, created_at FROM attachments WHERE id = ?").bind(format!("{:032x}", 2)).execute(&f.pool).await.is_err());
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 1);
}

#[tokio::test]
async fn quota_is_exact_atomic_and_rejected_import_leaves_no_final_or_work_object() {
    let f = Fixture::new().await;
    let mut remaining = MAX_BOOK_ATTACHMENT_BYTES;
    let mut index = 0;
    while remaining != 0 {
        let length = remaining.min(MAX_ATTACHMENT_BYTES);
        f.fake_bytes(index, length).await.unwrap();
        remaining -= length;
        index += 1;
    }
    assert!(f.fake_bytes(index, 1).await.is_err());
    assert!(matches!(
        f.repo.import_file(f.entry, &f.source, "receipt.pdf").await,
        Err(AttachmentError::BookQuotaExceeded)
    ));
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 1);
    let total: i64 = sqlx::query_scalar("SELECT SUM(byte_length) FROM attachments")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(total, MAX_BOOK_ATTACHMENT_BYTES as i64);
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn unknown_entry_and_attachment_ids_do_not_create_or_modify_objects() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.repo
            .import_file(JournalEntryId::new(999), &f.source, "receipt.pdf")
            .await,
        Err(AttachmentError::EntryNotFound)
    ));
    assert!(matches!(
        f.repo.list_for_entry(JournalEntryId::new(999)).await,
        Err(AttachmentError::EntryNotFound)
    ));
    let id = AttachmentId::parse(&"0".repeat(32)).unwrap();
    assert!(matches!(
        f.repo.get(&id).await,
        Err(AttachmentError::NotFound)
    ));
    for invalid in [
        "../object",
        "",
        "A0000000000000000000000000000000",
        "0000000000000000000000000000000g",
    ] {
        assert!(AttachmentId::parse(invalid).is_err());
    }
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_executable_permissions_and_replaced_store_roots_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new().await;
    let link = f.root.join("link.pdf");
    symlink(&f.source, &link).unwrap();
    assert!(matches!(
        f.repo.import_file(f.entry, &link, "receipt.pdf").await,
        Err(AttachmentError::UnsafeSource)
    ));
    std::fs::set_permissions(&f.source, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        f.repo.import_file(f.entry, &f.source, "receipt.pdf").await,
        Err(AttachmentError::UnsafeSource)
    ));
    std::fs::set_permissions(&f.source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let a = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    std::fs::rename(&f.objects, f.root.join("original-objects")).unwrap();
    std::fs::create_dir(&f.objects).unwrap();
    assert!(matches!(
        f.repo.verify(&a.id).await,
        Err(AttachmentError::UnsafeStore)
    ));
    assert!(matches!(
        f.repo.import_file(f.entry, &f.source, "receipt.pdf").await,
        Err(AttachmentError::UnsafeStore)
    ));
}

#[tokio::test]
async fn hundreds_of_attachments_keep_stable_references_and_verify_independently() {
    let f = Fixture::new().await;
    let mut imported = Vec::new();
    for index in 0..128 {
        imported.push(
            f.repo
                .import_file(f.entry, &f.source, &format!("收據 {index:04}.pdf"))
                .await
                .unwrap(),
        );
    }
    assert_eq!(f.repo.list_for_entry(f.entry).await.unwrap(), imported);
    for a in &imported {
        f.repo.verify(&a.id).await.unwrap();
    }
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
#[ignore = "250 MiB filesystem capacity acceptance; run explicitly in release mode"]
async fn maximum_size_attachment_streams_encrypts_and_verifies_without_a_plaintext_intermediate() {
    use std::io::{Seek, Write};
    let f = Fixture::new().await;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&f.source)
        .unwrap();
    file.set_len(MAX_ATTACHMENT_BYTES).unwrap();
    file.rewind().unwrap();
    file.write_all(b"%PDF-1.7\n").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let attachment = f
        .repo
        .import_file(f.entry, &f.source, "large receipt.pdf")
        .await
        .unwrap();
    assert_eq!(attachment.plaintext.byte_length(), MAX_ATTACHMENT_BYTES);
    assert_eq!(
        attachment.ciphertext.byte_length(),
        MAX_ATTACHMENT_BYTES + 72 + 20 * MAX_ATTACHMENT_BYTES.div_ceil(65536)
    );
    f.repo.verify(&attachment.id).await.unwrap();
    assert_eq!(std::fs::read_dir(&f.objects).unwrap().count(), 2);
    verify_database_integrity(&f.pool).await.unwrap();
}

#[tokio::test]
async fn orphaned_attachment_metadata_fails_closed_even_if_foreign_keys_were_bypassed() {
    let f = Fixture::new().await;
    let a = f
        .repo
        .import_file(f.entry, &f.source, "receipt.pdf")
        .await
        .unwrap();
    let before = describe_file(f.object(&a.id)).unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM journal_entries WHERE id = ?")
        .bind(f.entry.get())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.repo.get(&a.id).await,
        Err(AttachmentError::Integrity)
    ));
    assert!(matches!(
        f.repo.list_all().await,
        Err(AttachmentError::Integrity)
    ));
    assert!(matches!(
        f.repo.verify(&a.id).await,
        Err(AttachmentError::Integrity)
    ));
    assert!(
        f.fake_bytes(99, 1).await.is_err(),
        "owner insert trigger remains independent of disabled foreign keys"
    );
    assert_eq!(describe_file(f.object(&a.id)).unwrap(), before);
}
