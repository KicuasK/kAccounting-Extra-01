//! Explicit local maintenance commands. These never change active-book selection.
use crate::{
    attachment_maintenance::{self, CollectionOutcome, MaintenanceError},
    backup_service::{self, BackupReason, BackupServiceError, RetentionOutcome},
    book_backup::{self, BookBackupError, LocalBackupOutcome},
    keys::DeviceKeyStore,
    recipient_transfer::{self, RecipientKeyStore, RecipientTransferError},
    runtime::{RuntimeBook, RuntimeError},
};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error(transparent)]
    Package(#[from] crate::recipient_package::RecipientPackageError),
    #[error(transparent)]
    Import(#[from] crate::recipient_import::RecipientImportError),
    #[error(transparent)]
    Migration(#[from] crate::migration_backup::MigrationBackupError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Backup(#[from] BackupServiceError),
    #[error(transparent)]
    Restore(#[from] BookBackupError),
    #[error(transparent)]
    Maintenance(#[from] MaintenanceError),
    #[error(transparent)]
    Recipient(#[from] RecipientTransferError),
    #[error(
        "book has no managed backup identity; preserve its existing key and initialize a new namespace explicitly before managed backup"
    )]
    MissingBackupIdentity,
    #[error("book already has a backup identity; it was not replaced")]
    ExistingBackupIdentity,
    #[error("command destination must be outside source, absolute, private, and free of symlinks")]
    UnsafeDestination,
    #[error("command I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub struct BackupCommandOutcome {
    pub backup: LocalBackupOutcome,
    pub retention: Option<RetentionOutcome>,
    /// A completed verified backup remains successful if later retention fails.
    pub retention_warning: Option<String>,
}

pub async fn backup_book(
    path: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<BackupCommandOutcome, CommandError> {
    let book = RuntimeBook::open_existing(path, keys).await?;
    let result = backup_open_book(&book, keys).await;
    book.close().await;
    result
}
async fn backup_open_book(
    book: &RuntimeBook,
    keys: &impl DeviceKeyStore,
) -> Result<BackupCommandOutcome, CommandError> {
    let id = book
        .backup_id()
        .ok_or(CommandError::MissingBackupIdentity)?;
    let backup_root = book.root().join("backups");
    ensure_private_directory(&backup_root)?;
    let recovered = book_backup::recover_interrupted_local_backup_work(&backup_root)?;
    if !recovered.unrecognized_work.is_empty() {
        return Err(RuntimeError::RecoveryRequired.into());
    }
    let backup = backup_service::create_managed_backup(
        book.pool(),
        book.attachments(),
        &backup_root,
        id,
        book.key_id(),
        keys,
        BackupReason::Manual,
    )
    .await?;
    let (retention, retention_warning) =
        match backup_service::retain_latest_verified_backups(&backup_root, id, book.key_id(), keys)
            .await
        {
            Ok(retention) => (Some(retention), None),
            Err(error) => (None, Some(error.to_string())),
        };
    Ok(BackupCommandOutcome {
        backup,
        retention,
        retention_warning,
    })
}

/// Explicitly assigns a new retention namespace to an existing restored book.
/// Uses the bound existing root key and cannot replace an existing identity.
pub async fn setup_backup_identity(
    path: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<backup_service::BookBackupId, CommandError> {
    let book = RuntimeBook::open_existing(path, keys).await?;
    let result = if book.backup_id().is_some() {
        Err(CommandError::ExistingBackupIdentity)
    } else {
        backup_service::initialize_backup_identity(book.root(), book.key_id(), keys)
            .map_err(CommandError::from)
    };
    book.close().await;
    result
}

/// Existing destinations are refused by the audited restore primitive. The
/// original book, source package and active-book selection are not modified.
pub async fn restore_book(
    source: impl AsRef<Path>,
    new_destination: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<LocalBackupOutcome, CommandError> {
    Ok(book_backup::restore_local_backup(source, new_destination, keys).await?)
}

/// RuntimeBook supplies the exclusive app-process lock, but intentionally no
/// attachment repository/shared pin is opened before the exclusive collector.
pub async fn collect_attachments(
    path: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<CollectionOutcome, CommandError> {
    let book = RuntimeBook::open_existing(path, keys).await?;
    let result =
        attachment_maintenance::collect_unreferenced_ciphertext(book.pool(), book.attachments())
            .await
            .map_err(CommandError::from);
    book.close().await;
    result
}

#[derive(Debug)]
pub struct RecipientRequestOutcome {
    pub path: PathBuf,
    pub request_id: String,
    pub fingerprint: String,
}
/// Stores the recipient private key only in the supplied secure key store.
/// The exclusive destination file contains public request JSON only.
pub fn write_recipient_request(
    destination: impl AsRef<Path>,
    keys: &impl RecipientKeyStore,
) -> Result<RecipientRequestOutcome, CommandError> {
    let destination = destination.as_ref();
    let parent = destination
        .parent()
        .ok_or(CommandError::UnsafeDestination)?;
    check_private_directory(parent)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(CommandError::UnsafeDestination)?;
    if parent.starts_with(source) || source.starts_with(parent) {
        return Err(CommandError::UnsafeDestination);
    }
    let name = destination
        .file_name()
        .ok_or(CommandError::UnsafeDestination)?;
    if name == "." || name == ".." {
        return Err(CommandError::UnsafeDestination);
    }
    match std::fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "recipient request destination already exists",
            )
            .into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let request = recipient_transfer::create_recipient_request(keys)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(destination)?;
    file.write_all(&request.to_json()?)?;
    file.sync_all()?;
    crate::storage::ensure_path_still_names_open_file(destination, &file)?;
    File::open(parent)?.sync_all()?;
    Ok(RecipientRequestOutcome {
        path: destination.to_owned(),
        request_id: request.request_id().to_owned(),
        fingerprint: request.fingerprint().to_owned(),
    })
}
fn ensure_private_directory(path: &Path) -> Result<(), CommandError> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => {
            File::open(path.parent().ok_or(CommandError::UnsafeDestination)?)?.sync_all()?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    check_private_directory(path)
}
fn check_private_directory(path: &Path) -> Result<(), CommandError> {
    if !path.is_absolute() {
        return Err(CommandError::UnsafeDestination);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        ) {
            return Err(CommandError::UnsafeDestination);
        }
        current.push(component);
        if std::fs::symlink_metadata(&current)?
            .file_type()
            .is_symlink()
        {
            return Err(CommandError::UnsafeDestination);
        }
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(CommandError::UnsafeDestination);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(CommandError::UnsafeDestination);
    }
    Ok(())
}

/// Parse a bounded public request without following links or accessing keys.
pub fn read_recipient_request(
    path: impl AsRef<Path>,
) -> Result<recipient_transfer::RecipientRequest, CommandError> {
    let path = path.as_ref();
    let parent = path.parent().ok_or(CommandError::UnsafeDestination)?;
    check_private_directory(parent)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(CommandError::UnsafeDestination)?;
    if parent.starts_with(source) || source.starts_with(parent) {
        return Err(CommandError::UnsafeDestination);
    }
    let mut file = crate::storage::open_regular_file_without_symlink(path)?;
    #[cfg(unix)]
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(CommandError::UnsafeDestination);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(recipient_transfer::RecipientTransferError::InvalidDocument.into());
    }
    Ok(recipient_transfer::RecipientRequest::from_json(&bytes)?)
}

/// Caller must display this immutable request's ID and fingerprint and obtain
/// explicit source confirmation before invoking export. No request is re-read.
pub async fn export_recipient_book(
    source: impl AsRef<Path>,
    request: &recipient_transfer::RecipientRequest,
    destination: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<crate::recipient_package::RecipientPackageOutcome, CommandError> {
    let book = RuntimeBook::open_existing(source, keys).await?;
    let result = crate::recipient_package::export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        request,
        destination,
    )
    .await
    .map_err(CommandError::from);
    book.close().await;
    result
}

/// New destination only. Does not select, close or replace an active book.
pub async fn import_recipient_book(
    package: impl AsRef<Path>,
    request: &recipient_transfer::RecipientRequest,
    destination: impl AsRef<Path>,
    recipient_keys: &impl RecipientKeyStore,
    device_keys: &impl DeviceKeyStore,
) -> Result<crate::recipient_import::RecipientImportOutcome, CommandError> {
    Ok(crate::recipient_import::import_recipient_package(
        package,
        destination,
        request,
        recipient_keys,
        device_keys,
    )
    .await?)
}

pub async fn recover_recipient_exports(
    source: impl AsRef<Path>,
    export_parent: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<crate::recipient_package::RecipientExportRecovery, CommandError> {
    // Export recovery must not depend on a healthy live database. Load the
    // existing bound key under the book lock without opening/migrating SQLite.
    let book = crate::runtime::load_recovery_book_key(source, keys)?;
    crate::recipient_package::recover_interrupted_recipient_exports(export_parent, &book.key())
        .await
        .map_err(CommandError::from)
}

pub async fn restore_pre_migration(
    source: impl AsRef<Path>,
    checkpoint: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<LocalBackupOutcome, CommandError> {
    // Recovery must work even when live SQLite or migration scratch is broken.
    // This holder validates only the existing binding/key and owns the same
    // exclusive runtime lock; it never opens or modifies the source database.
    let book = crate::runtime::load_recovery_book_key(source, keys)?;
    crate::migration_backup::restore_pre_migration_backup(
        checkpoint.as_ref(),
        destination.as_ref(),
        book.key_id(),
        &book.key(),
    )
    .await
    .map_err(CommandError::from)
}
