//! Verified recipient import into a NEW book. Never selects an active book.
use crate::{
    backup_service::{BackupServiceError, initialize_backup_identity},
    db::{open_database, verify_database_snapshot},
    encrypted_transfer::{EncryptedTransferError, import_attachment_object},
    keys::{DeviceKeyStore, KeyStoreError, create_device_key},
    recipient_package::{
        RecipientPackageError, inspect_recipient_package, read_recipient_envelope,
    },
    recipient_transfer::{
        RecipientKeyStore, RecipientRequest, RecipientTransferError, unwrap_transfer_key,
    },
    storage::{
        ensure_directory_still_matches, ensure_path_still_names_open_file,
        publish_private_directory_noreplace,
    },
};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::sync::Arc;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;
const PREFIX: &str = ".kAccounting-Extra-01-recipient-import-";
#[derive(Debug, Error)]
pub enum RecipientImportError {
    #[error("unsafe or existing import destination")]
    UnsafeDestination,
    #[error("recipient import integrity mismatch")]
    Integrity,
    #[error(
        "import was published at {path}, but final verification or durability is uncertain: {source}"
    )]
    PublishedUnverified {
        path: PathBuf,
        source: Box<RecipientImportError>,
    },
    #[error("recipient import worker failed; inspect retained private staging")]
    Worker,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Package(#[from] RecipientPackageError),
    #[error(transparent)]
    Recipient(#[from] RecipientTransferError),
    #[error(transparent)]
    Key(#[from] KeyStoreError),
    #[error(transparent)]
    Identity(#[from] BackupServiceError),
    #[error(transparent)]
    Database(#[from] crate::db::DatabaseError),
    #[error(transparent)]
    Snapshot(#[from] crate::db::BackupError),
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Attachment(#[from] EncryptedTransferError),
    #[error(transparent)]
    Manifest(#[from] crate::transfer_manifest::TransferManifestError),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecipientImportStatus {
    pub format: String,
    pub package_id: String,
    pub destination: PathBuf,
    pub phase: String,
}
#[derive(Debug)]
pub struct RecipientImportOutcome {
    pub path: PathBuf,
    pub package_id: String,
    pub attachment_count: usize,
    /// A verified publication remains successful if optional owned-work cleanup fails.
    pub cleanup_warning: Option<String>,
}
fn private_dir(path: &Path) -> Result<PathBuf, RecipientImportError> {
    let md = std::fs::symlink_metadata(path)?;
    if !md.is_dir() || md.file_type().is_symlink() || md.permissions().mode() & 0o077 != 0 {
        return Err(RecipientImportError::UnsafeDestination);
    }
    let absolute = path.canonicalize()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RecipientImportError::UnsafeDestination)?;
    if absolute != path || absolute.starts_with(source) || source.starts_with(&absolute) {
        return Err(RecipientImportError::UnsafeDestination);
    }
    Ok(absolute)
}
fn new_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().mode(0o700).create(path)
}
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn checkpoint(
    root: &Path,
    identity: &std::fs::Metadata,
    status: &RecipientImportStatus,
) -> Result<(), RecipientImportError> {
    ensure_directory_still_matches(root, identity)?;
    let bytes = serde_json::to_vec(status).map_err(|_| RecipientImportError::Integrity)?;
    let target = root.join(format!("{}.json", status.phase));
    write_new(&target, &bytes)?;
    File::open(root)?.sync_all()?;
    Ok(())
}
/// Recipient-envelope authentication precedes device-key creation. Keys are never
/// removed on failure. Cancellation detaches the owned task, retaining all work
/// and its lifetime until pending blocking file work has completed.
pub async fn import_recipient_package(
    package: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    request: &RecipientRequest,
    recipient_keys: &impl RecipientKeyStore,
    device_keys: &impl DeviceKeyStore,
) -> Result<RecipientImportOutcome, RecipientImportError> {
    let original_package = package.as_ref();
    let package = original_package.canonicalize()?;
    if package != original_package
        || std::fs::symlink_metadata(original_package)?
            .file_type()
            .is_symlink()
    {
        return Err(RecipientImportError::UnsafeDestination);
    }
    let requested = destination.as_ref();
    let parent = private_dir(
        requested
            .parent()
            .ok_or(RecipientImportError::UnsafeDestination)?,
    )?;
    let parent_identity = std::fs::symlink_metadata(&parent)?;
    let name = requested
        .file_name()
        .ok_or(RecipientImportError::UnsafeDestination)?;
    let destination = parent.join(name);
    if destination.exists()
        || std::fs::symlink_metadata(&destination).is_ok()
        || destination.starts_with(&package)
    {
        return Err(RecipientImportError::UnsafeDestination);
    }
    let envelope = read_recipient_envelope(&package)?;
    let package_id = envelope.package_id().to_owned();
    let transfer_key = unwrap_transfer_key(recipient_keys, request, &package_id, &envelope)?;
    let mut token = [0u8; 16];
    getrandom::fill(&mut token).map_err(|_| KeyStoreError::RandomUnavailable)?;
    let hex: String = token.iter().map(|b| format!("{b:02x}")).collect();
    let work = parent.join(format!("{PREFIX}{hex}"));
    new_dir(&work)?;
    File::open(&parent)?.sync_all()?;
    let work_identity = std::fs::symlink_metadata(&work)?;
    let work_lock_path = work.join("operation.lock");
    let work_lock = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&work_lock_path)?;
    work_lock
        .try_lock()
        .map_err(|_| RecipientImportError::Integrity)?;
    let work_lock = Arc::new(work_lock);
    let staged = work.join("book");
    new_dir(&staged)?;
    let mut status = RecipientImportStatus {
        format: "kAccounting-Extra-01.recipient-import.v1".into(),
        package_id: package_id.clone(),
        destination: destination.clone(),
        phase: "created".into(),
    };
    checkpoint(&work, &work_identity, &status)?;
    let (key_id, local_key) = create_device_key(device_keys)?;
    let context = ImportRecoveryContext {
        domain: "kAccounting-Extra-01/recipient-import-recovery/initial/v1".into(),
        operation_name: work
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or(RecipientImportError::Integrity)?
            .to_owned(),
        parent: parent.clone(),
        destination: destination.clone(),
        package_id: package_id.clone(),
        local_key_id: key_id.as_str().to_owned(),
        parent_device: parent_identity.dev(),
        parent_inode: parent_identity.ino(),
        work_device: work_identity.dev(),
        work_inode: work_identity.ino(),
        ready: None,
    };
    write_new(&work.join("recovery-key-id"), key_id.as_str().as_bytes())?;
    write_recovery_context(&work, "recovery.kae", &context, &local_key)?;
    let runtime_lock = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(staged.join(".runtime.lock"))?;
    runtime_lock
        .try_lock()
        .map_err(|_| RecipientImportError::Integrity)?;
    let runtime_lock = Arc::new(runtime_lock);
    let backup_id = initialize_backup_identity(&staged, &key_id, device_keys)?;
    let local_key = Arc::new(local_key);
    let binding = serde_json::json!({"format":"kAccounting-Extra-01.local-backup","version":1,"key_id":key_id.as_str()});
    let binding_bytes =
        serde_json::to_vec(&binding).map_err(|_| RecipientImportError::Integrity)?;
    write_new(&staged.join("device-key.json"), &binding_bytes)?;
    new_dir(&staged.join("attachments"))?;
    tokio::spawn(async move {
        let operation_lock = work_lock;
        let _runtime_lock = runtime_lock;
        let staged_identity = std::fs::symlink_metadata(&staged)?;
        let database = staged.join("book.sqlite3");
        let verified =
            inspect_recipient_package(&package, &database, &transfer_key, &package_id).await?;
        status.phase = "verified".into();
        checkpoint(&work, &work_identity, &status)?;
        let objects = verified.manifest.attachment_objects()?;
        let worker_staged = staged.clone();
        let worker_package = package.clone();
        let worker_key = local_key.clone();
        let worker_lock = operation_lock.clone();
        let replacements = tokio::task::spawn_blocking(move || {
            let _worker_lock = worker_lock;
            let mut replacements = Vec::new();
            for (id, descriptor) in objects {
                let filename = format!("{}.kae", id.as_str());
                let result = import_attachment_object(
                    &worker_package.join("attachments").join(&filename),
                    &worker_staged.join("attachments").join(filename),
                    descriptor,
                    &transfer_key,
                    &worker_key,
                )?;
                replacements.push((id, result.attachment.ciphertext));
            }
            Ok::<_, RecipientImportError>(replacements)
        })
        .await
        .map_err(|_| RecipientImportError::Worker)??;
        // The only guard exception is confined to this unpublished, independently
        // authenticated database copy. Never expose this operation as a general
        // live-book mutation helper. Preserve journal, plaintext and audit fields.
        ensure_directory_still_matches(&staged, &staged_identity)?;
        let pool = open_database(&database).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("DROP TRIGGER attachments_immutable_update")
            .execute(&mut *tx)
            .await?;
        for (id, descriptor) in &replacements {
            let result = sqlx::query(
                "UPDATE attachments SET ciphertext_sha256=?, ciphertext_byte_length=? WHERE id=?",
            )
            .bind(descriptor.digest().to_hex())
            .bind(
                i64::try_from(descriptor.byte_length())
                    .map_err(|_| RecipientImportError::Integrity)?,
            )
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(RecipientImportError::Integrity);
            }
        }
        sqlx::query(
            "CREATE TRIGGER attachments_immutable_update
BEFORE UPDATE ON attachments
BEGIN
    SELECT RAISE(ABORT, 'attachment metadata is immutable');
END",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        pool.close().await;
        let descriptors: std::collections::HashMap<_, _> = replacements.iter().cloned().collect();
        if descriptors.len() != replacements.len() {
            return Err(RecipientImportError::Integrity);
        }
        let mut expected_references = verified.references;
        for reference in &mut expected_references {
            reference.ciphertext = *descriptors
                .get(&reference.id)
                .ok_or(RecipientImportError::Integrity)?;
        }
        verify_local_book(
            &staged,
            local_key.clone(),
            &binding_bytes,
            key_id.as_str(),
            backup_id.as_str(),
            &expected_references,
            operation_lock.clone(),
        )
        .await?;
        let mut ready_context = context;
        ready_context.domain = "kAccounting-Extra-01/recipient-import-recovery/ready/v1".into();
        let worker_database = database.clone();
        let worker_lock = operation_lock.clone();
        let db_descriptor = tokio::task::spawn_blocking(move || {
            let _lock = worker_lock;
            crate::storage::describe_file(worker_database)
        })
        .await
        .map_err(|_| RecipientImportError::Worker)??;
        ready_context.ready = Some(ImportReadyContext {
            database: RecoveryDescriptor::from_descriptor(db_descriptor),
            catalog: RecoveryDescriptor::from_descriptor(catalog_descriptor(&expected_references)?),
            binding: String::from_utf8(binding_bytes.clone())
                .map_err(|_| RecipientImportError::Integrity)?,
            backup_id: backup_id.as_str().to_owned(),
        });
        write_recovery_context(&work, "ready.kae", &ready_context, &local_key)?;
        status.phase = "ready".into();
        checkpoint(&work, &work_identity, &status)?;
        File::open(staged.join("attachments"))?.sync_all()?;
        File::open(&staged)?.sync_all()?;
        ensure_directory_still_matches(&work, &work_identity)?;
        ensure_directory_still_matches(&staged, &staged_identity)?;
        ensure_directory_still_matches(&parent, &parent_identity)?;
        publish_private_directory_noreplace(&staged, &destination)
            .map_err(|_| RecipientImportError::UnsafeDestination)?;
        let finalization = async {
            File::open(&parent)?.sync_all()?;
            ensure_directory_still_matches(&destination, &staged_identity)?;
            verify_local_book(
                &destination,
                local_key.clone(),
                &binding_bytes,
                key_id.as_str(),
                backup_id.as_str(),
                &expected_references,
                operation_lock.clone(),
            )
            .await?;
            status.phase = "published".into();
            checkpoint(&work, &work_identity, &status)?;
            Ok::<_, RecipientImportError>(())
        }
        .await;
        if let Err(source) = finalization {
            return Err(RecipientImportError::PublishedUnverified {
                path: destination,
                source: Box::new(source),
            });
        }
        Ok(RecipientImportOutcome {
            path: destination,
            package_id,
            attachment_count: replacements.len(),
            cleanup_warning: None,
        })
    })
    .await
    .map_err(|_| RecipientImportError::Worker)?
}
/// Conservative recovery: list durable operation evidence, never publish,
/// delete, resume, select a book, or infer completion from a directory name.
pub fn inspect_interrupted_recipient_imports(
    parent: impl AsRef<Path>,
) -> Result<Vec<(PathBuf, RecipientImportStatus)>, RecipientImportError> {
    let parent = private_dir(parent.as_ref())?;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if !name.starts_with(PREFIX) {
            continue;
        }
        private_dir(&path)?;
        let suffix = &name[PREFIX.len()..];
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(RecipientImportError::Integrity);
        }
        let lock_path = path.join("operation.lock");
        let lock = crate::storage::open_regular_file_without_symlink(&lock_path)?;
        match lock.try_lock() {
            Ok(()) => (),
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(_) => return Err(RecipientImportError::Integrity),
        }
        ensure_path_still_names_open_file(&lock_path, &lock)?;
        let mut found = None;
        for phase in ["published", "ready", "verified", "created"] {
            let file = path.join(format!("{phase}.json"));
            if !file.try_exists()? {
                continue;
            }
            let mut reader = crate::storage::open_regular_file_without_symlink(&file)?;
            let mut bytes = Vec::new();
            use std::io::Read;
            std::io::Read::by_ref(&mut reader)
                .take(8193)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 8192 {
                return Err(RecipientImportError::Integrity);
            }
            ensure_path_still_names_open_file(&file, &reader)?;
            let status: RecipientImportStatus =
                serde_json::from_slice(&bytes).map_err(|_| RecipientImportError::Integrity)?;
            if status.format != "kAccounting-Extra-01.recipient-import.v1"
                || status.phase != phase
                || status.destination.parent() != path.parent()
            {
                return Err(RecipientImportError::Integrity);
            }
            found = Some(status);
            break;
        }
        result.push((path, found.ok_or(RecipientImportError::Integrity)?));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

// Independently open the published path, audit its complete database catalog,
// authenticate every local object and new backup identity, and compare the
// exact create-only binding. No status record is trusted for this verification.
async fn verify_local_book(
    root: &Path,
    key: Arc<crate::crypto::RootEncryptionKey>,
    binding: &[u8],
    key_id: &str,
    backup_id: &str,
    expected: &[crate::attachments::Attachment],
    work_lock: Arc<File>,
) -> Result<(), RecipientImportError> {
    private_dir(root)?;
    let identity = std::fs::symlink_metadata(root)?;
    let objects = private_dir(&root.join("attachments"))?;
    let objects_identity = std::fs::symlink_metadata(&objects)?;
    verify_database_snapshot(root.join("book.sqlite3")).await?;
    let pool = crate::db::connect_read_only_database(&root.join("book.sqlite3")).await?;
    let actual = crate::attachments::catalog_for_pool(&pool).await;
    pool.close().await;
    let actual = actual.map_err(|_| RecipientImportError::Integrity)?;
    if actual != expected {
        return Err(RecipientImportError::Integrity);
    }
    let root = root.to_path_buf();
    let binding = binding.to_vec();
    let key_id = key_id.to_owned();
    let backup_id = backup_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let _worker_lock = work_lock;
        ensure_directory_still_matches(&root, &identity)?;
        ensure_directory_still_matches(&objects, &objects_identity)?;
        let binding_path = root.join("device-key.json");
        let mut input = crate::storage::open_regular_file_without_symlink(&binding_path)?;
        use std::io::Read;
        let mut bytes=Vec::new(); Read::by_ref(&mut input).take(4097).read_to_end(&mut bytes)?;
        ensure_path_still_names_open_file(&binding_path,&input)?;
        if bytes != binding { return Err(RecipientImportError::Integrity); }
        let identity_path=root.join("backup-identity.kae");
        let mut input = crate::storage::open_regular_file_without_symlink(&identity_path)?;
        if input.metadata()?.len() > 4096 { return Err(RecipientImportError::Integrity); }
        let mut plaintext=zeroize::Zeroizing::new(Vec::new());
        crate::crypto::decrypt_object(&mut input, &mut *plaintext, &key, crate::crypto::EncryptedObjectPurpose::BackupManifest).map_err(|_|RecipientImportError::Integrity)?;
        ensure_path_still_names_open_file(&identity_path,&input)?;
        let document:serde_json::Value=serde_json::from_slice(&plaintext).map_err(|_|RecipientImportError::Integrity)?;
        if document != serde_json::json!({"format":"kAccounting-Extra-01.backup-identity.v1","book_id":backup_id,"key_id":key_id}) { return Err(RecipientImportError::Integrity); }
        let expected_names: std::collections::BTreeSet<String> = actual.iter().map(|reference| format!("{}.kae",reference.id.as_str())).collect();
        let actual_names: std::collections::BTreeSet<String> = std::fs::read_dir(&objects)?.map(|entry| {
            entry?.file_name().into_string().map_err(|_|std::io::Error::other("invalid object name"))
        }).collect::<Result<_,_>>()?;
        // Local object transcode can leave an explicitly unreferenced work
        // directory only when its cleanup reported a warning; reject that
        // uncertain layout instead of declaring a complete imported book.
        if actual_names != expected_names { return Err(RecipientImportError::Integrity); }
        for reference in actual {
            crate::attachments::verify_encrypted_file(&root.join("attachments").join(format!("{}.kae",reference.id.as_str())), reference.plaintext, reference.ciphertext, &key).map_err(|_|RecipientImportError::Integrity)?;
        }
        ensure_directory_still_matches(&objects,&objects_identity)?;
        ensure_directory_still_matches(&root,&identity)?;
        Ok::<_,RecipientImportError>(())
    }).await.map_err(|_|RecipientImportError::Worker)?
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RecoveryDescriptor {
    digest: String,
    byte_length: u64,
}
impl RecoveryDescriptor {
    fn from_descriptor(d: crate::storage::StoredObjectDescriptor) -> Self {
        Self {
            digest: d.digest().to_hex(),
            byte_length: d.byte_length(),
        }
    }
    fn decode(&self) -> Result<crate::storage::StoredObjectDescriptor, RecipientImportError> {
        Ok(crate::storage::StoredObjectDescriptor::new(
            self.digest
                .parse()
                .map_err(|_| RecipientImportError::Integrity)?,
            self.byte_length,
        ))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ImportRecoveryContext {
    domain: String,
    operation_name: String,
    parent: PathBuf,
    destination: PathBuf,
    package_id: String,
    local_key_id: String,
    parent_device: u64,
    parent_inode: u64,
    work_device: u64,
    work_inode: u64,
    ready: Option<ImportReadyContext>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ImportReadyContext {
    database: RecoveryDescriptor,
    catalog: RecoveryDescriptor,
    binding: String,
    backup_id: String,
}
fn catalog_descriptor(
    references: &[crate::attachments::Attachment],
) -> Result<crate::storage::StoredObjectDescriptor, RecipientImportError> {
    let records: Vec<_> = references
        .iter()
        .map(|a| {
            serde_json::json!([
                a.id.as_str(),
                a.entry_id.get(),
                a.display_name,
                a.plaintext.digest().to_hex(),
                a.plaintext.byte_length(),
                a.ciphertext.digest().to_hex(),
                a.ciphertext.byte_length(),
                a.created_at
            ])
        })
        .collect();
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(&records).map_err(|_| RecipientImportError::Integrity)?,
    );
    Ok(crate::storage::describe_reader(&mut std::io::Cursor::new(
        &*bytes,
    ))?)
}
fn write_recovery_context(
    work: &Path,
    filename: &str,
    context: &ImportRecoveryContext,
    key: &crate::crypto::RootEncryptionKey,
) -> Result<(), RecipientImportError> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(context).map_err(|_| RecipientImportError::Integrity)?,
    );
    if bytes.len() > 16 * 1024 {
        return Err(RecipientImportError::Integrity);
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(work.join(filename))?;
    crate::crypto::encrypt_object(
        &mut std::io::Cursor::new(&*bytes),
        &mut file,
        key,
        crate::crypto::EncryptedObjectPurpose::TransferRecovery,
    )
    .map_err(|_| RecipientImportError::Integrity)?;
    file.sync_all()?;
    File::open(work)?.sync_all()?;
    Ok(())
}
fn read_recovery_context(
    work: &Path,
    filename: &str,
    key: &crate::crypto::RootEncryptionKey,
) -> Result<ImportRecoveryContext, RecipientImportError> {
    let path = work.join(filename);
    let mut input = crate::storage::open_regular_file_without_symlink(&path)?;
    if input.metadata()?.len() > 17 * 1024 || input.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(RecipientImportError::Integrity);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    crate::crypto::decrypt_object(
        &mut input,
        &mut *bytes,
        key,
        crate::crypto::EncryptedObjectPurpose::TransferRecovery,
    )
    .map_err(|_| RecipientImportError::Integrity)?;
    ensure_path_still_names_open_file(&path, &input)?;
    if bytes.len() > 16 * 1024 {
        return Err(RecipientImportError::Integrity);
    }
    serde_json::from_slice(&bytes).map_err(|_| RecipientImportError::Integrity)
}
/// Authenticated recovery never publishes an incomplete stage, deletes a key,
/// changes an existing destination, or selects an active book. It discards only
/// owned unpublished work, or verifies a published ready book before cleanup.
#[derive(Debug, Default)]
pub struct RecipientImportRecovery {
    pub removed_count: usize,
    pub verified_destinations: Vec<PathBuf>,
    pub active_work: Vec<PathBuf>,
    pub unrecognized_work: Vec<PathBuf>,
}
pub async fn recover_interrupted_recipient_imports(
    parent: impl AsRef<Path>,
    device_keys: &impl DeviceKeyStore,
) -> Result<RecipientImportRecovery, RecipientImportError> {
    // All borrowed key-store calls happen before the first await. Owned tasks
    // retain key material and OS locks if the requesting future is cancelled.
    let parent = private_dir(parent.as_ref())?;
    let parent_identity = std::fs::symlink_metadata(&parent)?;
    let mut result = RecipientImportRecovery::default();
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&parent)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_prefix(PREFIX) else {
            continue;
        };
        if crate::attachments::AttachmentId::parse(id).is_err() || private_dir(&path).is_err() {
            result.unrecognized_work.push(path);
            continue;
        }
        let identity = std::fs::symlink_metadata(&path)?;
        let lock_path = path.join("operation.lock");
        let Ok(lock) = crate::storage::open_regular_file_without_symlink(&lock_path) else {
            result.unrecognized_work.push(path);
            continue;
        };
        match lock.try_lock() {
            Ok(()) => (),
            Err(std::fs::TryLockError::WouldBlock) => {
                result.active_work.push(path);
                continue;
            }
            Err(_) => {
                result.unrecognized_work.push(path);
                continue;
            }
        }
        let preflight = (|| {
            ensure_path_still_names_open_file(&lock_path, &lock)?;
            if lock.metadata()?.permissions().mode() & 0o077 != 0 {
                return Err(RecipientImportError::Integrity);
            }
            let locator = path.join("recovery-key-id");
            let mut file = crate::storage::open_regular_file_without_symlink(&locator)?;
            use std::io::Read;
            let mut id = String::new();
            Read::by_ref(&mut file).take(33).read_to_string(&mut id)?;
            ensure_path_still_names_open_file(&locator, &file)?;
            let key_id = crate::keys::DeviceKeyId::parse(&id)?;
            let key = Arc::new(crate::keys::load_device_key(device_keys, &key_id)?);
            let context = read_recovery_context(&path, "recovery.kae", &key)?;
            if context.domain != "kAccounting-Extra-01/recipient-import-recovery/initial/v1"
                || context.ready.is_some()
                || context.operation_name != name
                || context.parent != parent
                || context.destination.parent() != Some(parent.as_path())
                || context.local_key_id != key_id.as_str()
                || context.parent_device != parent_identity.dev()
                || context.parent_inode != parent_identity.ino()
                || context.work_device != identity.dev()
                || context.work_inode != identity.ino()
                || crate::attachments::AttachmentId::parse(&context.package_id).is_err()
                || context.destination == path
                || context.destination.file_name().is_none()
            {
                return Err(RecipientImportError::Integrity);
            }
            let ready = if path.join("ready.kae").symlink_metadata().is_ok() {
                let ready = read_recovery_context(&path, "ready.kae", &key)?;
                let mut base = ready.clone();
                base.domain = context.domain.clone();
                base.ready = None;
                if base != context
                    || ready.ready.is_none()
                    || ready.domain != "kAccounting-Extra-01/recipient-import-recovery/ready/v1"
                {
                    return Err(RecipientImportError::Integrity);
                }
                ready.ready
            } else {
                None
            };
            Ok::<_, RecipientImportError>((context, ready, key))
        })();
        match preflight {
            Ok((context, ready, key)) => {
                candidates.push((path, identity, Arc::new(lock), context, ready, key))
            }
            Err(_) => result.unrecognized_work.push(path),
        }
    }
    tokio::spawn(async move {
        for (path, identity, lock, context, ready, key) in candidates {
            let recovery = async {
                ensure_directory_still_matches(&parent, &parent_identity)?;
                ensure_directory_still_matches(&path, &identity)?;
                ensure_path_still_names_open_file(&path.join("operation.lock"), &lock)?;
                let published = match std::fs::symlink_metadata(&context.destination) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(e) => return Err(e.into()),
                    Ok(_) => {
                        let ready = ready.as_ref().ok_or(RecipientImportError::Integrity)?;
                        verify_recovered_destination(
                            &context.destination,
                            &context.local_key_id,
                            ready,
                            key.clone(),
                            lock.clone(),
                        )
                        .await?;
                        true
                    }
                };
                ensure_directory_still_matches(&parent, &parent_identity)?;
                ensure_directory_still_matches(&path, &identity)?;
                ensure_path_still_names_open_file(&path.join("operation.lock"), &lock)?;
                // Destination may have appeared during an unpublished cleanup
                // preflight. Preserve all work on uncertainty.
                if !published && std::fs::symlink_metadata(&context.destination).is_ok() {
                    return Err(RecipientImportError::Integrity);
                }
                std::fs::remove_dir_all(&path)?;
                File::open(&parent)?.sync_all()?;
                Ok::<_, RecipientImportError>(published)
            }
            .await;
            match recovery {
                Ok(published) => {
                    result.removed_count += 1;
                    if published {
                        result.verified_destinations.push(context.destination)
                    }
                }
                Err(_) => result.unrecognized_work.push(path),
            }
        }
        result.active_work.sort();
        result.unrecognized_work.sort();
        result.verified_destinations.sort();
        Ok(result)
    })
    .await
    .map_err(|_| RecipientImportError::Worker)?
}
async fn verify_recovered_destination(
    destination: &Path,
    key_id: &str,
    ready: &ImportReadyContext,
    key: Arc<crate::crypto::RootEncryptionKey>,
    work_lock: Arc<File>,
) -> Result<(), RecipientImportError> {
    private_dir(destination)?;
    let identity = std::fs::symlink_metadata(destination)?;
    let runtime_lock_path = destination.join(".runtime.lock");
    let runtime_lock = crate::storage::open_regular_file_without_symlink(&runtime_lock_path)?;
    runtime_lock
        .try_lock()
        .map_err(|_| RecipientImportError::Integrity)?;
    ensure_path_still_names_open_file(&runtime_lock_path, &runtime_lock)?;
    let runtime_lock = Arc::new(runtime_lock);
    let database = destination.join("book.sqlite3");
    let expected = ready.database.decode()?;
    let worker_database = database.clone();
    let worker_lock = work_lock.clone();
    let worker_runtime = runtime_lock.clone();
    tokio::task::spawn_blocking(move || {
        let _locks = (worker_lock, worker_runtime);
        crate::storage::verify_file(worker_database, expected)
            .map_err(|_| RecipientImportError::Integrity)
    })
    .await
    .map_err(|_| RecipientImportError::Worker)??;
    // A checkpointed imported book has no pending WAL (zero-length SQLite
    // sidecars left by read-only checks are harmless). Never verify a modified
    // live database by hashing only its older main SQLite file.
    for suffix in ["-wal", "-journal"] {
        match std::fs::symlink_metadata(format!("{}{suffix}", database.display())) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() == 0 => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(RecipientImportError::Integrity),
        }
    }
    let pool = crate::db::connect_read_only_database(&database).await?;
    let references = crate::attachments::catalog_for_pool(&pool).await;
    pool.close().await;
    let references = references.map_err(|_| RecipientImportError::Integrity)?;
    if RecoveryDescriptor::from_descriptor(catalog_descriptor(&references)?) != ready.catalog {
        return Err(RecipientImportError::Integrity);
    }
    verify_local_book(
        destination,
        key,
        ready.binding.as_bytes(),
        key_id,
        &ready.backup_id,
        &references,
        work_lock.clone(),
    )
    .await?;
    ensure_directory_still_matches(destination, &identity)?;
    ensure_path_still_names_open_file(&runtime_lock_path, &runtime_lock)?;
    let worker_database = database;
    let worker_runtime = runtime_lock;
    tokio::task::spawn_blocking(move || {
        let _locks = (work_lock, worker_runtime);
        crate::storage::verify_file(worker_database, expected)
            .map_err(|_| RecipientImportError::Integrity)
    })
    .await
    .map_err(|_| RecipientImportError::Worker)??;
    Ok(())
}
