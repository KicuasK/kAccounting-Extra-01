//! Managed local backup checkpoints and conservative newest-ten retention.
//!
//! A book ID must be generated once and persisted with the book's device-key
//! binding. A device key alone is not a book identity. Unmanaged, incomplete,
//! corrupt, foreign-book and symbolic-link entries are always preserved.
use crate::{
    book_backup::{BookBackupError, LocalBackupOutcome, create_local_backup, verify_local_backup},
    crypto::{EncryptedObjectPurpose, decrypt_object, encrypt_object},
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, load_device_key},
    storage::{
        describe_file, ensure_directory_still_matches, ensure_path_still_names_open_file,
        open_regular_file_without_symlink,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{self, Cursor},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub const RETAINED_BACKUPS: usize = 10;
const RECEIPT: &str = "receipt.kae";
const PACKAGE: &str = "backup.kabackup";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookBackupId(String);
impl BookBackupId {
    pub fn generate() -> Result<Self, KeyStoreError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| KeyStoreError::RandomUnavailable)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }
    pub fn parse(value: &str) -> Result<Self, KeyStoreError> {
        DeviceKeyId::parse(value)?;
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackupReason {
    Manual,
    BeforeMigration,
    BeforeImport,
}

#[derive(Debug, Error)]
pub enum BackupServiceError {
    #[error("managed backup identity or directory is invalid")]
    InvalidIdentity,
    #[error("another backup maintenance operation is active")]
    Busy,
    #[error("managed backup I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Backup(#[from] BookBackupError),
    #[error(transparent)]
    Key(#[from] KeyStoreError),
    #[error("managed backup encryption failed: {0}")]
    Crypto(#[from] crate::crypto::EncryptedObjectError),
}

#[derive(Debug, Default)]
pub struct RetentionOutcome {
    pub retained: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub preserved_unrecognized: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    format: String,
    book_id: String,
    key_id: String,
    name: String,
    created_nanos: String,
    reason: BackupReason,
    catalog_sha256: String,
    catalog_bytes: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BookIdentity {
    format: String,
    book_id: String,
    key_id: String,
}

/// Explicit create-only initialization for a new book. Existing books must use
/// `load_backup_identity`; a missing identity must never be silently replaced.
pub fn initialize_backup_identity(
    book_root: impl AsRef<Path>,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<BookBackupId, BackupServiceError> {
    let root = checked_root(book_root.as_ref())?;
    let key = load_device_key(keys, key_id)?;
    let id = BookBackupId::generate()?;
    let identity = BookIdentity {
        format: "kAccounting-Extra-01.backup-identity.v1".into(),
        book_id: id.as_str().to_owned(),
        key_id: key_id.as_str().to_owned(),
    };
    let bytes = serde_json::to_vec(&identity).map_err(|_| BackupServiceError::InvalidIdentity)?;
    let mut output = private_file(&root.join("backup-identity.kae"))?;
    encrypt_object(
        &mut Cursor::new(bytes),
        &mut output,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    output.sync_all()?;
    File::open(&root)?.sync_all()?;
    Ok(id)
}

pub fn load_backup_identity(
    book_root: impl AsRef<Path>,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<BookBackupId, BackupServiceError> {
    let root = checked_root(book_root.as_ref())?;
    let key = load_device_key(keys, key_id)?;
    let mut input = open_regular_file_without_symlink(&root.join("backup-identity.kae"))?;
    if input.metadata()?.len() > 8192 {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let mut bytes = Vec::new();
    decrypt_object(
        &mut input,
        &mut bytes,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    let identity: BookIdentity =
        serde_json::from_slice(&bytes).map_err(|_| BackupServiceError::InvalidIdentity)?;
    if identity.format != "kAccounting-Extra-01.backup-identity.v1"
        || identity.key_id != key_id.as_str()
    {
        return Err(BackupServiceError::InvalidIdentity);
    }
    Ok(BookBackupId::parse(&identity.book_id)?)
}

/// Creates a fully verified checkpoint before a caller is allowed to mutate a
/// book. Failure returns no checkpoint. Retention is deliberately separate: a
/// retention failure must not disguise a successfully published safety backup.
pub async fn create_managed_backup(
    pool: &SqlitePool,
    attachment_root: impl AsRef<Path>,
    backup_root: impl AsRef<Path>,
    book_id: &BookBackupId,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
    reason: BackupReason,
) -> Result<LocalBackupOutcome, BackupServiceError> {
    let root = checked_root(backup_root.as_ref())?;
    let _lock = lock_root(&root)?;
    let key = load_device_key(keys, key_id)?;
    let mut nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BackupServiceError::InvalidIdentity)?
        .as_nanos();
    // Keep creation order monotonic under the same maintenance lock. A clock
    // rollback must not cause retention to discard a newly created checkpoint.
    // Only authenticated receipts can advance this ordering timestamp. This
    // ordering step does not reread every old attachment; pruning still audits
    // each complete backup before it can become a deletion candidate.
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        if let Ok(previous) = authenticated_receipt_time(&path, book_id, key_id, keys) {
            nanos = nanos.max(
                previous
                    .checked_add(1)
                    .ok_or(BackupServiceError::InvalidIdentity)?,
            );
        }
    }
    let random = BookBackupId::generate()?;
    let name = format!(
        "{}-{nanos:039}-{}.managed",
        book_id.as_str(),
        random.as_str()
    );
    let directory = root.join(&name);
    private_directory(&directory)?;
    // An interrupted directory without an authenticated receipt is preserved.
    let outcome =
        create_local_backup(pool, attachment_root, directory.join(PACKAGE), key_id, keys).await?;
    if outcome.cleanup_warning.is_some() {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let descriptor = describe_file(outcome.path.join("catalog.kae"))?;
    let receipt = Receipt {
        format: "kAccounting-Extra-01.managed-backup.v1".to_owned(),
        book_id: book_id.as_str().to_owned(),
        key_id: key_id.as_str().to_owned(),
        name,
        created_nanos: nanos.to_string(),
        reason,
        catalog_sha256: descriptor.digest().to_hex(),
        catalog_bytes: descriptor.byte_length().to_string(),
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|_| BackupServiceError::InvalidIdentity)?;
    let mut output = private_file(&directory.join(RECEIPT))?;
    encrypt_object(
        &mut Cursor::new(bytes),
        &mut output,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    output.sync_all()?;
    File::open(&directory)?.sync_all()?;
    File::open(&root)?.sync_all()?;
    verify_managed(&directory, book_id, key_id, keys).await?;
    Ok(outcome)
}

/// Runs the mutation only after an authenticated, audited backup exists. A
/// failed mutation leaves the checkpoint intact. This supports audited current
/// schema imports; upgrading older schemas needs a compatible snapshot reader.
pub async fn with_verified_checkpoint<T, E, F, Fut>(
    checkpoint: Result<LocalBackupOutcome, BackupServiceError>,
    operation: F,
) -> Result<Result<T, E>, BackupServiceError>
where
    F: FnOnce(LocalBackupOutcome) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    Ok(operation(checkpoint?).await)
}

/// Retains exactly the newest ten *verified managed backups of this book* when
/// more exist. Verification of all candidates completes before any deletion.
/// Ordering comes from authenticated creation time, never filesystem mtime.
pub async fn retain_latest_verified_backups(
    backup_root: impl AsRef<Path>,
    book_id: &BookBackupId,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<RetentionOutcome, BackupServiceError> {
    let root = checked_root(backup_root.as_ref())?;
    let root_identity = std::fs::symlink_metadata(&root)?;
    let _lock = lock_root(&root)?;
    // Missing/unavailable keys must fail the entire operation, never masquerade
    // as zero candidates or provoke replacement-key creation.
    let _key = load_device_key(keys, key_id)?;
    let mut result = RetentionOutcome::default();
    let mut verified = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if entry.file_name() == ".backup-service.lock" {
            continue;
        }
        let path = entry.path();
        match verify_managed(&path, book_id, key_id, keys).await {
            Ok(time) => verified.push((time, path, std::fs::symlink_metadata(entry.path())?)),
            Err(_) => result.preserved_unrecognized.push(path),
        }
    }
    verified.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, path, _) in verified.iter().take(RETAINED_BACKUPS) {
        result.retained.push(path.clone());
    }
    // Newest survivor is reverified immediately before destructive maintenance.
    if verified.len() > RETAINED_BACKUPS {
        verify_managed(&verified[0].1, book_id, key_id, keys).await?;
    }
    for (_, path, identity) in verified.into_iter().skip(RETAINED_BACKUPS) {
        verify_managed(&path, book_id, key_id, keys).await?;
        ensure_directory_still_matches(&root, &root_identity)?;
        ensure_directory_still_matches(&path, &identity)?;
        std::fs::remove_dir_all(&path)?;
        result.removed.push(path);
    }
    File::open(&root)?.sync_all()?;
    result.preserved_unrecognized.sort();
    Ok(result)
}

async fn verify_managed(
    directory: &Path,
    book: &BookBackupId,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<u128, BackupServiceError> {
    let identity = std::fs::symlink_metadata(directory)?;
    let nanos = authenticated_receipt_time(directory, book, key_id, keys)?;
    let outcome = verify_local_backup(directory.join(PACKAGE), keys).await?;
    if outcome.key_id != *key_id || outcome.cleanup_warning.is_some() {
        return Err(BackupServiceError::InvalidIdentity);
    }
    ensure_directory_still_matches(directory, &identity)?;
    Ok(nanos)
}

fn authenticated_receipt_time(
    directory: &Path,
    book: &BookBackupId,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<u128, BackupServiceError> {
    private_existing_directory(directory)?;
    let identity = std::fs::symlink_metadata(directory)?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(BackupServiceError::InvalidIdentity)?;
    let remainder = name
        .strip_prefix(&format!("{}-", book.as_str()))
        .and_then(|name| name.strip_suffix(".managed"))
        .ok_or(BackupServiceError::InvalidIdentity)?;
    let (timestamp, random) = remainder
        .split_once('-')
        .ok_or(BackupServiceError::InvalidIdentity)?;
    if timestamp.len() != 39
        || !timestamp.bytes().all(|b| b.is_ascii_digit())
        || BookBackupId::parse(random).is_err()
    {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let mut children = std::fs::read_dir(directory)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    if children
        != [
            std::ffi::OsString::from(PACKAGE),
            std::ffi::OsString::from(RECEIPT),
        ]
    {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let key = load_device_key(keys, key_id)?;
    let mut input = open_regular_file_without_symlink(&directory.join(RECEIPT))?;
    if input.metadata()?.len() > 8192 {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let mut bytes = Vec::new();
    decrypt_object(
        &mut input,
        &mut bytes,
        &key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    let receipt: Receipt =
        serde_json::from_slice(&bytes).map_err(|_| BackupServiceError::InvalidIdentity)?;
    let nanos: u128 = receipt
        .created_nanos
        .parse()
        .map_err(|_| BackupServiceError::InvalidIdentity)?;
    if receipt.format != "kAccounting-Extra-01.managed-backup.v1"
        || receipt.book_id != book.as_str()
        || receipt.key_id != key_id.as_str()
        || receipt.name != name
        || receipt.created_nanos != nanos.to_string()
        || timestamp != format!("{nanos:039}")
    {
        return Err(BackupServiceError::InvalidIdentity);
    }
    let package = directory.join(PACKAGE);
    let catalog = describe_file(package.join("catalog.kae"))?;
    if catalog.digest().to_hex() != receipt.catalog_sha256
        || catalog.byte_length().to_string() != receipt.catalog_bytes
    {
        return Err(BackupServiceError::InvalidIdentity);
    }
    ensure_directory_still_matches(directory, &identity)?;
    Ok(nanos)
}

fn checked_root(path: &Path) -> Result<PathBuf, BackupServiceError> {
    private_existing_directory(path)?;
    let root = std::fs::canonicalize(path)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(BackupServiceError::InvalidIdentity)?;
    if root.starts_with(source) {
        return Err(BackupServiceError::InvalidIdentity);
    }
    Ok(root)
}
fn private_existing_directory(path: &Path) -> Result<(), BackupServiceError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BackupServiceError::InvalidIdentity);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(BackupServiceError::InvalidIdentity);
    }
    Ok(())
}
fn private_directory(path: &Path) -> io::Result<()> {
    let mut options = std::fs::DirBuilder::new();
    #[cfg(unix)]
    options.mode(0o700);
    options.create(path)
}
fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}
fn lock_root(root: &Path) -> Result<File, BackupServiceError> {
    let path = root.join(".backup-service.lock");
    let file = match private_file(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open_regular_file_without_symlink(&path)?
        }
        Err(error) => return Err(error.into()),
    };
    #[cfg(unix)]
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(BackupServiceError::InvalidIdentity);
    }
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(BackupServiceError::Busy),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    ensure_path_still_names_open_file(&path, &file)?;
    Ok(file)
}
