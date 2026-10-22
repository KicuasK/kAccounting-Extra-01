//! Complete encrypted recipient-package export and independent verification.
//! No operation selects a book or accesses Keychain. Private export work is
//! created beneath the source book parent; accounting data is never modified.
use crate::{
    attachment_maintenance::StorePin,
    attachments::{Attachment, AttachmentError, catalog_for_pool},
    crypto::{
        EncryptedObjectError, EncryptedObjectPurpose, RootEncryptionKey, decrypt_object,
        encrypt_object,
    },
    db::{
        BackupError, DatabaseError, backup_database, connect_read_only_database,
        verify_database_snapshot,
    },
    encrypted_transfer::{EncryptedTransferError, TransferredAttachment, export_attachment_object},
    portable_media::{CiphertextStage, MediaIdentity},
    recipient_transfer::{
        RecipientEnvelope, RecipientRequest, RecipientTransferError, create_transfer_key,
    },
    storage::{
        ContentDigest, LocalDirectoryPublishError, StoredObjectDescriptor,
        StoredObjectVerificationError, describe_file, describe_reader,
        ensure_directory_still_matches, ensure_path_still_names_open_file,
        open_regular_file_without_symlink, publish_private_directory_noreplace, verify_file,
    },
    transfer::TransferOperationState,
    transfer_manifest::{EncryptedTransferManifest, TransferManifestError},
    transfer_state::{
        TransferWorkStateError, create_export_work_state, load_transfer_work_state,
        transition_transfer_work_state,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use zeroize::Zeroizing;

pub const ENVELOPE_FILE: &str = "envelope.json";
pub const MANIFEST_FILE: &str = "manifest.kae";
pub const DATABASE_FILE: &str = "book.kae";
const PREFIX: &str = ".kAccounting-Extra-01-recipient-export-";

#[derive(Debug, Error)]
pub enum RecipientPackageError {
    #[error("recipient package destination already exists")]
    DestinationExists,
    #[error("recipient package path is unsafe or inside source code/store")]
    UnsafePath,
    #[error("recipient package layout, descriptor, or recovery context does not match")]
    Integrity,
    #[error("recipient package worker failed")]
    Worker,
    #[error("published recipient package requires verification at {path}: {source}")]
    PublishedUnverified {
        path: PathBuf,
        source: Box<RecipientPackageError>,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Snapshot(#[from] BackupError),
    #[error(transparent)]
    Attachment(#[from] AttachmentError),
    #[error(transparent)]
    Crypto(#[from] EncryptedObjectError),
    #[error(transparent)]
    Transfer(#[from] EncryptedTransferError),
    #[error(transparent)]
    Recipient(#[from] RecipientTransferError),
    #[error(transparent)]
    Manifest(#[from] TransferManifestError),
    #[error(transparent)]
    Verify(#[from] StoredObjectVerificationError),
    #[error(transparent)]
    Publish(#[from] LocalDirectoryPublishError),
    #[error(transparent)]
    State(#[from] TransferWorkStateError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientPackageOutcome {
    pub path: PathBuf,
    pub package_id: String,
    pub attachment_count: usize,
    pub attachment_bytes: u64,
    pub cleanup_warning: Option<String>,
    pub(crate) verified_download_entries: Vec<crate::transfer_container::ContainerEntry>,
}
#[derive(Clone, Debug)]
pub struct VerifiedRecipientPackage {
    pub manifest: EncryptedTransferManifest,
    pub references: Vec<Attachment>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryContext {
    operation_id: String,
    package_id: String,
    transfer_key: String,
    destination: String,
    #[serde(default)]
    media: Option<MediaIdentity>,
    #[serde(default)]
    staging_name: Option<String>,
}

impl Drop for RecoveryContext {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.transfer_key.zeroize();
    }
}

/// Protects the source store BEFORE snapshot selection, and includes the entire
/// snapshot catalog. Publication is atomic and never replaces an existing name.
/// The attachment-store parent must be private local book storage. All SQLite
/// and key-wrapped recovery scratch stays there; removable media receives only
/// encrypted package objects and the public envelope. Recover exports by scanning
/// that source-local parent, rather than the destination media directory.
pub async fn export_recipient_package(
    pool: &SqlitePool,
    attachment_root: impl AsRef<Path>,
    local_key: &RootEncryptionKey,
    request: &RecipientRequest,
    destination: impl AsRef<Path>,
) -> Result<RecipientPackageOutcome, RecipientPackageError> {
    // Keep the operation, source pin, and work lock alive if an HTTP caller
    // disconnects while blocking filesystem/SQLite work is still running.
    let pool = pool.clone();
    let attachment_root = attachment_root.as_ref().to_path_buf();
    let destination = destination.as_ref().to_path_buf();
    let local_key = RootEncryptionKey::new(*local_key.as_bytes());
    let request = request.clone();
    tokio::spawn(async move {
        export_owned(&pool, &attachment_root, &local_key, &request, &destination).await
    })
    .await
    .map_err(|_| RecipientPackageError::Worker)?
}

async fn export_owned(
    pool: &SqlitePool,
    attachment_root: &Path,
    local_key: &RootEncryptionKey,
    request: &RecipientRequest,
    destination: &Path,
) -> Result<RecipientPackageOutcome, RecipientPackageError> {
    let destination = checked_destination(destination)?;
    let attachment_root = checked_directory(attachment_root, true)?;
    if destination.starts_with(&attachment_root) {
        return Err(RecipientPackageError::UnsafePath);
    }
    let media = MediaIdentity::capture(
        destination
            .parent()
            .ok_or(RecipientPackageError::UnsafePath)?,
    )?;
    let pin = StorePin::acquire(&attachment_root).map_err(io::Error::other)?;
    let private_parent = checked_directory(
        attachment_root
            .parent()
            .ok_or(RecipientPackageError::UnsafePath)?,
        true,
    )?;
    let mut work = Work::create(&private_parent)?;
    create_export_work_state(&work.path, &destination)?;
    let package_id = random_id()?;
    let (transfer_key, envelope) = create_transfer_key(request, &package_id)?;
    write_recovery_context(&work, &package_id, &transfer_key, local_key, &destination)?;
    // A cancelled async operation preserves locked private work for recovery.
    work.preserve = true;
    let snapshot = work.path.join("snapshot.sqlite3");
    let result = backup_database(pool, &snapshot).await?;
    if result.cleanup_warning().is_some() {
        return Err(RecipientPackageError::Integrity);
    }
    let snapshot_pool = connect_read_only_database(&snapshot).await?;
    let references = catalog_for_pool(&snapshot_pool).await;
    snapshot_pool.close().await;
    let references = references?;
    let package = work.path.join("package");
    let worker_package = package.clone();
    let worker_snapshot = snapshot.clone();
    let worker_local = RootEncryptionKey::new(*local_key.as_bytes());
    let transfer_key = Arc::new(transfer_key);
    let worker_key = transfer_key.clone();
    let worker_lock = work.lock.clone();
    let worker_pin = pin.clone();
    let worker_id = package_id.clone();
    tokio::task::spawn_blocking(move || {
        let _lock = worker_lock;
        build_package(
            &worker_snapshot,
            &attachment_root,
            &worker_package,
            &worker_id,
            &envelope,
            &references,
            &worker_pin,
            &worker_local,
            &worker_key,
        )
    })
    .await
    .map_err(|_| RecipientPackageError::Worker)??;
    std::fs::remove_file(snapshot)?;
    let verify_path = work.path.join("verify.sqlite3");
    let verified =
        inspect_recipient_package(&package, &verify_path, &transfer_key, &package_id).await?;
    std::fs::remove_file(&verify_path)?;
    work.validate()?;
    // Persist intent BEFORE rename: recovery treats both absent/present outcomes
    // conservatively and authenticates a present destination independently.
    transition_transfer_work_state(
        &work.path,
        TransferOperationState::Working,
        TransferOperationState::PublishedUnverified,
    )?;
    let parent = destination
        .parent()
        .ok_or(RecipientPackageError::UnsafePath)?;
    media.validate(parent)?;
    let stage = CiphertextStage::create(
        parent,
        &format!(".kAccounting-Extra-01-ciphertext-{package_id}.partial"),
        &media,
    )?;
    let names = verified
        .references
        .iter()
        .map(|a| format!("{}.kae", a.id.as_str()))
        .collect::<Vec<_>>();
    let copy_package = package.clone();
    let copy_lock = work.lock.clone();
    let stage = tokio::task::spawn_blocking(move || {
        let _lock = copy_lock;
        stage.copy_package(&copy_package, &names)?;
        Ok::<_, RecipientPackageError>(stage)
    })
    .await
    .map_err(|_| RecipientPackageError::Worker)??;
    let staged =
        inspect_recipient_package(stage.path(), &verify_path, &transfer_key, &package_id).await?;
    if staged.references != verified.references {
        return Err(RecipientPackageError::Integrity);
    }
    std::fs::remove_file(&verify_path)?;
    stage.validate()?;
    publish_private_directory_noreplace(stage.path(), &destination)?;
    let published_result = async {
        File::open(
            destination
                .parent()
                .ok_or(RecipientPackageError::UnsafePath)?,
        )?
        .sync_all()?;
        let names = [
            ENVELOPE_FILE.to_owned(),
            MANIFEST_FILE.to_owned(),
            DATABASE_FILE.to_owned(),
        ]
        .into_iter()
        .chain(
            verified
                .references
                .iter()
                .map(|a| format!("attachments/{}.kae", a.id.as_str())),
        );
        let mut entries = Vec::new();
        for name in names {
            let descriptor = describe_file(destination.join(&name))?;
            entries.push(
                crate::transfer_container::ContainerEntry::new(name, descriptor)
                    .map_err(|_| RecipientPackageError::Integrity)?,
            );
        }
        let actual =
            inspect_recipient_package(&destination, &verify_path, &transfer_key, &package_id)
                .await?;
        if actual.references != verified.references {
            return Err(RecipientPackageError::Integrity);
        }
        std::fs::remove_file(&verify_path)?;
        transition_transfer_work_state(
            &work.path,
            TransferOperationState::PublishedUnverified,
            TransferOperationState::Verified,
        )?;
        for entry in &entries {
            verify_file(destination.join(entry.name()), entry.descriptor())?;
        }
        Ok::<_, RecipientPackageError>(entries)
    }
    .await;
    let verified_download_entries = match published_result {
        Ok(entries) => entries,
        Err(source) => {
            return Err(RecipientPackageError::PublishedUnverified {
                path: destination,
                source: Box::new(source),
            });
        }
    };
    let mut outcome = RecipientPackageOutcome {
        path: destination,
        verified_download_entries,
        package_id,
        attachment_count: verified.references.len(),
        attachment_bytes: verified
            .references
            .iter()
            .map(|a| a.plaintext.byte_length())
            .sum(),
        cleanup_warning: None,
    };
    if let Err(error) = work.cleanup() {
        outcome.cleanup_warning = Some(error.to_string());
    }
    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
fn build_package(
    snapshot: &Path,
    source: &Path,
    package: &Path,
    id: &str,
    envelope: &RecipientEnvelope,
    references: &[Attachment],
    pin: &StorePin,
    local: &RootEncryptionKey,
    key: &RootEncryptionKey,
) -> Result<(), RecipientPackageError> {
    private_directory(package)?;
    private_directory(&package.join("attachments"))?;
    let envelope_bytes = envelope.to_json()?;
    write_file(&package.join(ENVELOPE_FILE), &envelope_bytes)?;
    let mut input = open_regular_file_without_symlink(snapshot)?;
    let mut output = private_file(&package.join(DATABASE_FILE))?;
    encrypt_object(
        &mut input,
        &mut output,
        key,
        EncryptedObjectPurpose::TransferDatabase,
    )?;
    output.sync_all()?;
    drop(output);
    let database = TransferredAttachment {
        plaintext: describe_file(snapshot)?,
        ciphertext: describe_file(package.join(DATABASE_FILE))?,
    };
    let mut objects = Vec::with_capacity(references.len());
    for reference in references {
        let name = format!("{}.kae", reference.id.as_str());
        let result = export_attachment_object(
            pin,
            &source.join(&name),
            &package.join("attachments").join(name),
            TransferredAttachment {
                plaintext: reference.plaintext,
                ciphertext: reference.ciphertext,
            },
            local,
            key,
        )?;
        if result.cleanup_warning.is_some() {
            return Err(RecipientPackageError::Integrity);
        }
        objects.push((reference.clone(), result.attachment));
    }
    let manifest = EncryptedTransferManifest::new(
        id,
        describe_reader(&mut Cursor::new(envelope_bytes))?,
        database,
        objects,
    )?;
    let mut output = private_file(&package.join(MANIFEST_FILE))?;
    manifest.encrypt(&mut output, key)?;
    output.sync_all()?;
    File::open(package.join("attachments"))?.sync_all()?;
    File::open(package)?.sync_all()?;
    Ok(())
}

/// Reads only a bounded public HPKE envelope; the manifest subsequently binds
/// the exact bytes. This is not proof of package authenticity by itself.
pub fn read_recipient_envelope(package: &Path) -> Result<RecipientEnvelope, RecipientPackageError> {
    checked_directory(package, false)?;
    let bytes = read_bounded(&package.join(ENVELOPE_FILE), 4096)?;
    Ok(RecipientEnvelope::from_json(&bytes)?)
}

/// Independently authenticates every package object, validates SQLite, and
/// reconciles the complete snapshot catalog. The caller must supply a NEW path
/// in a private directory outside the source tree and own its cleanup. It may
/// contain partial plaintext on failure; it must never become an active book.
pub async fn inspect_recipient_package(
    package: &Path,
    database_destination: &Path,
    key: &RootEncryptionKey,
    expected_package_id: &str,
) -> Result<VerifiedRecipientPackage, RecipientPackageError> {
    let package = checked_directory(package, false)?;
    let identity = std::fs::symlink_metadata(&package)?;
    let database_destination = checked_destination(database_destination)?;
    checked_directory(
        database_destination
            .parent()
            .ok_or(RecipientPackageError::UnsafePath)?,
        true,
    )?;
    if database_destination.starts_with(&package) {
        return Err(RecipientPackageError::UnsafePath);
    }
    let envelope_bytes = read_bounded(&package.join(ENVELOPE_FILE), 4096)?;
    let envelope = RecipientEnvelope::from_json(&envelope_bytes)?;
    if envelope.package_id() != expected_package_id {
        return Err(RecipientPackageError::Integrity);
    }
    let mut input = open_regular_file_without_symlink(&package.join(MANIFEST_FILE))?;
    let manifest_descriptor = describe_file(package.join(MANIFEST_FILE))?;
    let manifest = EncryptedTransferManifest::decrypt(
        &mut input,
        key,
        expected_package_id,
        describe_reader(&mut Cursor::new(&envelope_bytes))?,
    )?;
    ensure_path_still_names_open_file(&package.join(MANIFEST_FILE), &input)?;
    check_layout(&package, &manifest)?;
    let database = manifest.database()?;
    let worker_package = package.clone();
    let worker_destination = database_destination.clone();
    let worker_key = RootEncryptionKey::new(*key.as_bytes());
    tokio::task::spawn_blocking(move || {
        verify_file(worker_package.join(DATABASE_FILE), database.ciphertext)?;
        let mut input = open_regular_file_without_symlink(&worker_package.join(DATABASE_FILE))?;
        let mut output = private_file(&worker_destination)?;
        decrypt_object(
            &mut input,
            &mut output,
            &worker_key,
            EncryptedObjectPurpose::TransferDatabase,
        )?;
        output.sync_all()?;
        ensure_path_still_names_open_file(&worker_destination, &output)?;
        verify_file(&worker_destination, database.plaintext)?;
        verify_file(worker_package.join(DATABASE_FILE), database.ciphertext)?;
        Ok::<_, RecipientPackageError>(())
    })
    .await
    .map_err(|_| RecipientPackageError::Worker)??;
    verify_database_snapshot(&database_destination).await?;
    let pool = connect_read_only_database(&database_destination).await?;
    let references = catalog_for_pool(&pool).await;
    pool.close().await;
    let references = references?;
    manifest.verify_references(&references)?;
    let worker_objects = manifest.attachment_objects()?;
    let worker_package = package.clone();
    let worker_key = RootEncryptionKey::new(*key.as_bytes());
    tokio::task::spawn_blocking(move || {
        for (id, descriptor) in worker_objects {
            verify_transfer_object(
                &worker_package
                    .join("attachments")
                    .join(format!("{}.kae", id.as_str())),
                descriptor,
                &worker_key,
                EncryptedObjectPurpose::TransferAttachment,
            )?;
        }
        Ok::<_, RecipientPackageError>(())
    })
    .await
    .map_err(|_| RecipientPackageError::Worker)??;
    check_layout(&package, &manifest)?;
    verify_file(package.join(MANIFEST_FILE), manifest_descriptor)?;
    if read_bounded(&package.join(ENVELOPE_FILE), 4096)? != envelope_bytes {
        return Err(RecipientPackageError::Integrity);
    }
    verify_file(package.join(DATABASE_FILE), database.ciphertext)?;
    ensure_directory_still_matches(&package, &identity)?;
    Ok(VerifiedRecipientPackage {
        manifest,
        references,
    })
}

fn verify_transfer_object(
    path: &Path,
    expected: TransferredAttachment,
    key: &RootEncryptionKey,
    purpose: EncryptedObjectPurpose,
) -> Result<(), RecipientPackageError> {
    verify_file(path, expected.ciphertext)?;
    let mut input = open_regular_file_without_symlink(path)?;
    let mut sink = HashSink::default();
    decrypt_object(&mut input, &mut sink, key, purpose)?;
    ensure_path_still_names_open_file(path, &input)?;
    if sink.descriptor() != expected.plaintext {
        return Err(RecipientPackageError::Integrity);
    }
    verify_file(path, expected.ciphertext)?;
    Ok(())
}
#[derive(Default)]
struct HashSink {
    digest: Sha256,
    count: u64,
}
impl HashSink {
    fn descriptor(&self) -> StoredObjectDescriptor {
        let bytes: [u8; 32] = self.digest.clone().finalize().into();
        StoredObjectDescriptor::new(ContentDigest::from_bytes(bytes), self.count)
    }
}
impl Write for HashSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.count = self
            .count
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("length overflow"))?;
        self.digest.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn check_layout(
    root: &Path,
    manifest: &EncryptedTransferManifest,
) -> Result<(), RecipientPackageError> {
    checked_directory(root, false)?;
    let expected = [ENVELOPE_FILE, MANIFEST_FILE, DATABASE_FILE, "attachments"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if directory_names(root)? != expected {
        return Err(RecipientPackageError::Integrity);
    }
    for name in [ENVELOPE_FILE, MANIFEST_FILE, DATABASE_FILE] {
        let _ = open_regular_file_without_symlink(&root.join(name))?;
    }
    let objects = root.join("attachments");
    checked_directory(&objects, false)?;
    let expected: BTreeSet<String> = manifest
        .attachment_objects()?
        .iter()
        .map(|(id, _)| format!("{}.kae", id.as_str()))
        .collect();
    if directory_names(&objects)? != expected {
        return Err(RecipientPackageError::Integrity);
    }
    for name in expected {
        let _ = open_regular_file_without_symlink(&objects.join(name))?;
    }
    Ok(())
}
fn directory_names(root: &Path) -> Result<BTreeSet<String>, RecipientPackageError> {
    std::fs::read_dir(root)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| RecipientPackageError::Integrity)
        })
        .collect()
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, RecipientPackageError> {
    let mut file = open_regular_file_without_symlink(path)?;
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    ensure_path_still_names_open_file(path, &file)?;
    if bytes.len() as u64 > limit {
        return Err(RecipientPackageError::Integrity);
    }
    Ok(bytes)
}
fn checked_directory(path: &Path, private: bool) -> Result<PathBuf, RecipientPackageError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RecipientPackageError::UnsafePath);
    }
    #[cfg(unix)]
    if private && metadata.permissions().mode() & 0o077 != 0 {
        return Err(RecipientPackageError::UnsafePath);
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(path.canonicalize()?)
}
fn checked_destination(path: &Path) -> Result<PathBuf, RecipientPackageError> {
    let parent = checked_directory(
        path.parent().ok_or(RecipientPackageError::UnsafePath)?,
        false,
    )?;
    let candidate = parent.join(path.file_name().ok_or(RecipientPackageError::UnsafePath)?);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RecipientPackageError::UnsafePath)?;
    if candidate.starts_with(source) {
        return Err(RecipientPackageError::UnsafePath);
    }
    match std::fs::symlink_metadata(&candidate) {
        Ok(_) => Err(RecipientPackageError::DestinationExists),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(candidate),
        Err(e) => Err(e.into()),
    }
}
fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}
fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn random_id() -> io::Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct Work {
    path: PathBuf,
    identity: std::fs::Metadata,
    lock: Arc<File>,
    preserve: bool,
}
impl Work {
    fn create(parent: &Path) -> Result<Self, RecipientPackageError> {
        let id = random_id()?;
        let path = parent.join(format!("{PREFIX}{id}.work"));
        private_directory(&path)?;
        let identity = std::fs::symlink_metadata(&path)?;
        let mut lock = private_file(&path.join(".lock"))?;
        lock.try_lock()
            .map_err(|_| io::Error::other("could not lock new recipient work"))?;
        lock.write_all(format!("kAccounting-Extra-01/recipient-export-work/v1/{id}\n").as_bytes())?;
        lock.sync_all()?;
        File::open(&path)?.sync_all()?;
        File::open(parent)?.sync_all()?;
        Ok(Self {
            path,
            identity,
            lock: Arc::new(lock),
            preserve: false,
        })
    }
    fn validate(&self) -> io::Result<()> {
        ensure_directory_still_matches(&self.path, &self.identity)?;
        ensure_path_still_names_open_file(&self.path.join(".lock"), &self.lock)
    }
    fn cleanup(&mut self) -> Result<(), RecipientPackageError> {
        self.validate()?;
        if Arc::strong_count(&self.lock) != 1 {
            return Err(RecipientPackageError::Integrity);
        }
        std::fs::remove_dir_all(&self.path)?;
        File::open(
            self.path
                .parent()
                .ok_or(RecipientPackageError::UnsafePath)?,
        )?
        .sync_all()?;
        self.preserve = true;
        Ok(())
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        if !self.preserve && Arc::strong_count(&self.lock) == 1 && self.validate().is_ok() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
fn write_recovery_context(
    work: &Work,
    id: &str,
    transfer: &RootEncryptionKey,
    local: &RootEncryptionKey,
    destination: &Path,
) -> Result<(), RecipientPackageError> {
    let mut context = RecoveryContext {
        operation_id: work
            .path
            .file_name()
            .ok_or(RecipientPackageError::UnsafePath)?
            .to_str()
            .ok_or(RecipientPackageError::UnsafePath)?
            .to_owned(),
        package_id: id.to_owned(),
        destination: destination
            .to_str()
            .ok_or(RecipientPackageError::UnsafePath)?
            .to_owned(),
        media: Some(MediaIdentity::capture(
            destination
                .parent()
                .ok_or(RecipientPackageError::UnsafePath)?,
        )?),
        staging_name: Some(format!(".kAccounting-Extra-01-ciphertext-{id}.partial")),
        transfer_key: transfer
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let bytes =
        Zeroizing::new(serde_json::to_vec(&context).map_err(|_| RecipientPackageError::Integrity)?);
    use zeroize::Zeroize;
    context.transfer_key.zeroize();
    let mut output = private_file(&work.path.join("recovery.kae"))?;
    encrypt_object(
        &mut Cursor::new(&*bytes),
        &mut output,
        local,
        EncryptedObjectPurpose::TransferRecovery,
    )?;
    output.sync_all()?;
    File::open(&work.path)?.sync_all()?;
    Ok(())
}

/// Recovery never switches books or deletes an uncertain published destination.
/// Scan the private source attachment-store parent for newly created exports.
/// Missing/replaced media preserves local work and all external ciphertext stages.
#[derive(Debug, Default)]
pub struct RecipientExportRecovery {
    pub removed_count: usize,
    pub verified_destinations: Vec<PathBuf>,
    pub active_work: Vec<PathBuf>,
    pub unrecognized_work: Vec<PathBuf>,
}

pub async fn recover_interrupted_recipient_exports(
    parent: impl AsRef<Path>,
    local_key: &RootEncryptionKey,
) -> Result<RecipientExportRecovery, RecipientPackageError> {
    // A verification pass writes private SQLite scratch on blocking workers.
    // Keep its work lock alive until every worker completes even when a caller
    // disconnects; another recovery must never clean that live scratch tree.
    let parent = parent.as_ref().to_path_buf();
    let local_key = RootEncryptionKey::new(*local_key.as_bytes());
    tokio::spawn(async move { recover_exports_owned(&parent, &local_key).await })
        .await
        .map_err(|_| RecipientPackageError::Worker)?
}

async fn recover_exports_owned(
    parent: &Path,
    local_key: &RootEncryptionKey,
) -> Result<RecipientExportRecovery, RecipientPackageError> {
    let parent = checked_directory(parent, false)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RecipientPackageError::UnsafePath)?;
    if parent.starts_with(source) {
        return Err(RecipientPackageError::UnsafePath);
    }
    let parent_identity = std::fs::symlink_metadata(&parent)?;
    let mut result = RecipientExportRecovery::default();
    for entry in std::fs::read_dir(&parent)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(id) = name
            .strip_prefix(PREFIX)
            .and_then(|s| s.strip_suffix(".work"))
        else {
            continue;
        };
        let path = entry.path();
        if crate::attachments::AttachmentId::parse(id).is_err()
            || checked_directory(&path, true).is_err()
        {
            result.unrecognized_work.push(path);
            continue;
        }
        let identity = std::fs::symlink_metadata(&path)?;
        let Ok(lock) = open_regular_file_without_symlink(&path.join(".lock")) else {
            result.unrecognized_work.push(path);
            continue;
        };
        #[cfg(unix)]
        if lock.metadata()?.permissions().mode() & 0o077 != 0 {
            result.unrecognized_work.push(path);
            continue;
        }
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                result.active_work.push(path);
                continue;
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        let mut marker = String::new();
        (&lock).take(160).read_to_string(&mut marker)?;
        if marker != format!("kAccounting-Extra-01/recipient-export-work/v1/{id}\n") {
            result.unrecognized_work.push(path);
            continue;
        }
        let state = match load_transfer_work_state(&path) {
            Ok(state) => state,
            Err(_) => {
                result.unrecognized_work.push(path);
                continue;
            }
        };
        let recovery =
            recover_one(&path, &name, state.destination(), state.state(), local_key).await;
        match recovery {
            Ok(published) => {
                ensure_directory_still_matches(&parent, &parent_identity)?;
                ensure_directory_still_matches(&path, &identity)?;
                ensure_path_still_names_open_file(&path.join(".lock"), &lock)?;
                std::fs::remove_dir_all(&path)?;
                File::open(&parent)?.sync_all()?;
                result.removed_count += 1;
                if published {
                    result
                        .verified_destinations
                        .push(state.destination().to_path_buf());
                }
            }
            Err(_) => result.unrecognized_work.push(path),
        }
    }
    result.active_work.sort();
    result.unrecognized_work.sort();
    result.verified_destinations.sort();
    Ok(result)
}
async fn recover_one(
    work: &Path,
    operation_id: &str,
    destination: &Path,
    state: TransferOperationState,
    local: &RootEncryptionKey,
) -> Result<bool, RecipientPackageError> {
    let mut input = open_regular_file_without_symlink(&work.join("recovery.kae"))?;
    let mut bytes = BoundedRecovery(Zeroizing::new(Vec::new()));
    decrypt_object(
        &mut input,
        &mut bytes,
        local,
        EncryptedObjectPurpose::TransferRecovery,
    )?;
    let mut context: RecoveryContext =
        serde_json::from_slice(&bytes.0).map_err(|_| RecipientPackageError::Integrity)?;
    if Path::new(&context.destination) != destination
        || context.operation_id != operation_id
        || crate::attachments::AttachmentId::parse(&context.package_id).is_err()
        || context.transfer_key.len() != 64
        || !context
            .transfer_key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(RecipientPackageError::Integrity);
    }
    // Older operations were staged beside their destination. New operations bind
    // the external directory identity and retain source work if media is missing.
    if let Some(media) = &context.media {
        if context.staging_name.as_deref()
            != Some(
                format!(
                    ".kAccounting-Extra-01-ciphertext-{}.partial",
                    context.package_id
                )
                .as_str(),
            )
        {
            return Err(RecipientPackageError::Integrity);
        }
        media.validate(
            destination
                .parent()
                .ok_or(RecipientPackageError::UnsafePath)?,
        )?;
    } else if destination.parent() != work.parent() {
        return Err(RecipientPackageError::Integrity);
    }
    let mut key_bytes = Zeroizing::new([0; 32]);
    for (index, byte) in key_bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&context.transfer_key[index * 2..index * 2 + 2], 16)
            .map_err(|_| RecipientPackageError::Integrity)?;
    }
    use zeroize::Zeroize;
    context.transfer_key.zeroize();
    let key = RootEncryptionKey::new(*key_bytes);
    match std::fs::symlink_metadata(destination) {
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                && matches!(
                    state,
                    TransferOperationState::Working
                        | TransferOperationState::PublishedUnverified
                        | TransferOperationState::Failed
                ) =>
        {
            if state != TransferOperationState::Failed {
                transition_transfer_work_state(work, state, TransferOperationState::Failed)?;
            }
            Ok(false)
        }
        Ok(_)
            if matches!(
                state,
                TransferOperationState::PublishedUnverified | TransferOperationState::Verified
            ) =>
        {
            let database = work.join(format!("recovery-verify-{}.sqlite3", random_id()?));
            inspect_recipient_package(destination, &database, &key, &context.package_id).await?;
            std::fs::remove_file(database)?;
            if state == TransferOperationState::PublishedUnverified {
                transition_transfer_work_state(work, state, TransferOperationState::Verified)?;
            }
            Ok(true)
        }
        _ => Err(RecipientPackageError::Integrity),
    }
}
struct BoundedRecovery(Zeroizing<Vec<u8>>);
impl Write for BoundedRecovery {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() > 4096 {
            return Err(io::Error::other("recovery context limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-recipient-recovery-{}",
            random_id().unwrap()
        ));
        private_directory(&path).unwrap();
        path.canonicalize().unwrap()
    }
    #[tokio::test]
    async fn changed_or_unplugged_media_preserves_private_source_recovery() {
        let parent = fixture();
        let media = parent.join("media");
        private_directory(&media).unwrap();
        let local = RootEncryptionKey::new([34; 32]);
        let mut work = Work::create(&parent).unwrap();
        let destination = media.join("out.kagextra");
        create_export_work_state(&work.path, &destination).unwrap();
        write_recovery_context(
            &work,
            &"a".repeat(32),
            &RootEncryptionKey::new([91; 32]),
            &local,
            &destination,
        )
        .unwrap();
        work.preserve = true;
        let path = work.path.clone();
        drop(work);
        std::fs::rename(&media, parent.join("unplugged-media")).unwrap();
        let missing = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(missing.unrecognized_work, vec![path.clone()]);
        assert!(path.join("recovery.kae").exists());
        private_directory(&media).unwrap();
        let replaced = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(replaced.unrecognized_work, vec![path.clone()]);
        assert!(path.exists());
        std::fs::remove_dir(&media).unwrap();
        std::fs::rename(parent.join("unplugged-media"), &media).unwrap();
        let restored = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(restored.removed_count, 1);
        assert!(!path.exists());
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[tokio::test]
    async fn active_work_is_preserved_then_abandoned_work_requires_correct_local_key() {
        let parent = fixture();
        let mut work = Work::create(&parent).unwrap();
        let destination = parent.join("out.kagextra");
        create_export_work_state(&work.path, &destination).unwrap();
        let local = RootEncryptionKey::new([34; 32]);
        write_recovery_context(
            &work,
            &"a".repeat(32),
            &RootEncryptionKey::new([91; 32]),
            &local,
            &destination,
        )
        .unwrap();
        work.preserve = true;
        std::fs::write(
            work.path.join("snapshot.sqlite3"),
            b"private interrupted snapshot",
        )
        .unwrap();
        let result = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(result.active_work, vec![work.path.clone()]);
        assert_eq!(result.removed_count, 0);
        let path = work.path.clone();
        drop(work);
        let result =
            recover_interrupted_recipient_exports(&parent, &RootEncryptionKey::new([35; 32]))
                .await
                .unwrap();
        assert_eq!(result.unrecognized_work, vec![path.clone()]);
        assert!(path.join("snapshot.sqlite3").exists());
        let result = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(result.removed_count, 1);
        assert!(!path.exists());
        std::fs::remove_dir(parent).unwrap();
    }
    #[tokio::test]
    async fn cancelled_recovery_request_keeps_its_owned_operation_alive() {
        let parent = fixture();
        let mut work = Work::create(&parent).unwrap();
        let destination = parent.join("out.kagextra");
        create_export_work_state(&work.path, &destination).unwrap();
        let local = RootEncryptionKey::new([34; 32]);
        write_recovery_context(
            &work,
            &"a".repeat(32),
            &RootEncryptionKey::new([91; 32]),
            &local,
            &destination,
        )
        .unwrap();
        std::fs::write(
            work.path.join("snapshot.sqlite3"),
            b"private interrupted snapshot",
        )
        .unwrap();
        work.preserve = true;
        let path = work.path.clone();
        drop(work);
        let mut recovery = Box::pin(recover_interrupted_recipient_exports(&parent, &local));
        // Dispatch once without letting the single-threaded test runtime run
        // the owned operation, then simulate loss of the requesting client.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(recovery.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(recovery);
        for _ in 0..1024 {
            if !path.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !path.exists(),
            "cancelled requester must not cancel the owned recovery"
        );
        assert!(
            !destination.exists(),
            "cleanup must not create a publication"
        );
        std::fs::remove_dir(parent).unwrap();
    }
    #[tokio::test]
    async fn recovery_context_binds_destination_and_never_removes_unknown_published_output() {
        let parent = fixture();
        let mut work = Work::create(&parent).unwrap();
        let destination = parent.join("out.kagextra");
        create_export_work_state(&work.path, &destination).unwrap();
        let local = RootEncryptionKey::new([34; 32]);
        write_recovery_context(
            &work,
            &"a".repeat(32),
            &RootEncryptionKey::new([91; 32]),
            &local,
            &destination,
        )
        .unwrap();
        transition_transfer_work_state(
            &work.path,
            TransferOperationState::Working,
            TransferOperationState::PublishedUnverified,
        )
        .unwrap();
        private_directory(&destination).unwrap();
        std::fs::write(destination.join("sentinel"), b"keep").unwrap();
        work.preserve = true;
        let path = work.path.clone();
        drop(work);
        let result = recover_interrupted_recipient_exports(&parent, &local)
            .await
            .unwrap();
        assert_eq!(result.unrecognized_work, vec![path]);
        assert_eq!(result.removed_count, 0);
        assert_eq!(
            std::fs::read(destination.join("sentinel")).unwrap(),
            b"keep"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}
