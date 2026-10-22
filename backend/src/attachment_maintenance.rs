//! Offline ciphertext reclamation coordinated with live repositories and snapshot readers.
//!
//! Every reader of a live attachment store must acquire a `StorePin` BEFORE
//! taking its SQLite snapshot and retain it until all object reads finish.
//! Collection requires exclusive access and never runs against an open book.
use crate::{
    attachments::AttachmentId,
    storage::{
        ensure_directory_still_matches, ensure_path_still_names_open_file,
        open_regular_file_without_symlink,
    },
};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

const LOCK_NAME: &str = ".maintenance.lock";

#[derive(Debug, Error)]
pub enum MaintenanceError {
    #[error("attachment store is in use; close all books and snapshot readers before maintenance")]
    Busy,
    #[error("attachment maintenance requires a private, unchanged store outside the source tree")]
    UnsafeStore,
    #[error(
        "maintenance database must be the live sibling book.sqlite3 for this attachments store"
    )]
    MismatchedDatabase,
    #[error("attachment references contain an invalid identifier")]
    InvalidReference,
    #[error("attachment maintenance I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("attachment maintenance database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

/// Shared cross-process lease. Clones keep the same lease alive, including in
/// blocking workers whose awaiting task may be cancelled. Never delete its file.
#[derive(Clone)]
pub struct StorePin {
    lease: Arc<StoreLease>,
}
struct StoreLease {
    root: PathBuf,
    identity: std::fs::Metadata,
    lock: File,
}
impl StorePin {
    pub fn acquire(root: impl AsRef<Path>) -> Result<Self, MaintenanceError> {
        Ok(Self {
            lease: Arc::new(StoreLease::acquire(root.as_ref(), false)?),
        })
    }
    pub fn root(&self) -> &Path {
        &self.lease.root
    }
    pub fn validate(&self) -> Result<(), MaintenanceError> {
        self.lease.validate()
    }
}
impl StoreLease {
    fn acquire(root: &Path, exclusive: bool) -> Result<Self, MaintenanceError> {
        let identity = std::fs::symlink_metadata(root)?;
        check_private_directory(&identity)?;
        let root = std::fs::canonicalize(root)?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or(MaintenanceError::UnsafeStore)?;
        if root.starts_with(source) {
            return Err(MaintenanceError::UnsafeStore);
        }
        let path = root.join(LOCK_NAME);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let lock = match options.open(&path) {
            Ok(file) => {
                File::open(&root)?.sync_all()?;
                file
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_regular_file_without_symlink(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        check_private_file(&lock.metadata()?)?;
        if lock.metadata()?.len() != 0 {
            return Err(MaintenanceError::UnsafeStore);
        }
        let result = if exclusive {
            lock.try_lock()
        } else {
            lock.try_lock_shared()
        };
        match result {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(MaintenanceError::Busy),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let lease = Self {
            root,
            identity,
            lock,
        };
        lease.validate()?;
        Ok(lease)
    }
    fn validate(&self) -> Result<(), MaintenanceError> {
        ensure_directory_still_matches(&self.root, &self.identity)
            .map_err(|_| MaintenanceError::UnsafeStore)?;
        check_private_directory(&std::fs::symlink_metadata(&self.root)?)?;
        ensure_path_still_names_open_file(&self.root.join(LOCK_NAME), &self.lock)
            .map_err(|_| MaintenanceError::UnsafeStore)?;
        check_private_file(&self.lock.metadata()?)?;
        if self.lock.metadata()?.len() != 0 {
            return Err(MaintenanceError::UnsafeStore);
        }
        Ok(())
    }
}
fn check_private_directory(metadata: &std::fs::Metadata) -> Result<(), MaintenanceError> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(MaintenanceError::UnsafeStore);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(MaintenanceError::UnsafeStore);
    }
    Ok(())
}
fn check_private_file(metadata: &std::fs::Metadata) -> Result<(), MaintenanceError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(MaintenanceError::UnsafeStore);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o177 != 0 || metadata.nlink() != 1 {
        return Err(MaintenanceError::UnsafeStore);
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectionOutcome {
    pub removed_objects: u64,
    pub removed_bytes: u64,
    pub preserved_unknown_entries: u64,
}

/// Reclaims only private regular `32-lowercase-hex.kae` files absent from the
/// configured book's attachment table. Requires the approved runtime layout:
/// `book.sqlite3` and its sibling `attachments` directory; rejects an in-memory,
/// foreign or nonmatching database. Close all repositories first.
///
/// Holds an exclusive cross-process pin and a SQLite IMMEDIATE writer
/// transaction throughout. A failed/aborted run can remove a safe subset; all
/// committed attachment references and all unknown files remain untouched.
pub async fn collect_unreferenced_ciphertext(
    pool: &SqlitePool,
    root: impl AsRef<Path>,
) -> Result<CollectionOutcome, MaintenanceError> {
    let lease = StoreLease::acquire(root.as_ref(), true)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // Bind deletion to the application's runtime book layout. An unrelated or
    // in-memory pool must never classify another book's files as unreferenced.
    let databases: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(&mut *tx)
        .await?;
    let main_path = databases
        .iter()
        .find(|(_, name, _)| name == "main")
        .map(|(_, _, path)| PathBuf::from(path))
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(MaintenanceError::MismatchedDatabase)?;
    if lease.root.file_name() != Some(std::ffi::OsStr::new("attachments")) {
        return Err(MaintenanceError::MismatchedDatabase);
    }
    let expected_database = lease
        .root
        .parent()
        .ok_or(MaintenanceError::MismatchedDatabase)?
        .join("book.sqlite3");
    let database_file = open_regular_file_without_symlink(&main_path)
        .map_err(|_| MaintenanceError::MismatchedDatabase)?;
    check_private_file(&database_file.metadata()?)
        .map_err(|_| MaintenanceError::MismatchedDatabase)?;
    if std::fs::canonicalize(&main_path)? != expected_database {
        return Err(MaintenanceError::MismatchedDatabase);
    }
    ensure_path_still_names_open_file(&expected_database, &database_file)
        .map_err(|_| MaintenanceError::MismatchedDatabase)?;
    let references: Vec<String> = sqlx::query_scalar("SELECT id FROM main.attachments")
        .fetch_all(&mut *tx)
        .await?;
    for id in &references {
        AttachmentId::parse(id).map_err(|_| MaintenanceError::InvalidReference)?;
    }
    let mut outcome = CollectionOutcome::default();
    for entry in std::fs::read_dir(&lease.root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            outcome.preserved_unknown_entries += 1;
            continue;
        };
        if name == LOCK_NAME {
            continue;
        }
        let Some(id) = name
            .strip_suffix(".kae")
            .and_then(|id| AttachmentId::parse(id).ok())
        else {
            outcome.preserved_unknown_entries += 1;
            continue;
        };
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if check_private_file(&metadata).is_err() {
            outcome.preserved_unknown_entries += 1;
            continue;
        }
        let referenced: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM main.attachments WHERE id = ?)")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if referenced {
            continue;
        }
        let file = open_regular_file_without_symlink(&path)?;
        check_private_file(&file.metadata()?)?;
        lease.validate()?;
        ensure_path_still_names_open_file(&expected_database, &database_file)
            .map_err(|_| MaintenanceError::MismatchedDatabase)?;
        ensure_path_still_names_open_file(&path, &file)?;
        std::fs::remove_file(&path)?;
        outcome.removed_objects += 1;
        outcome.removed_bytes += file.metadata()?.len();
    }
    lease.validate()?;
    File::open(&lease.root)?.sync_all()?;
    tx.commit().await?;
    Ok(outcome)
}
