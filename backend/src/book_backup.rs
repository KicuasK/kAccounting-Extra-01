//! Verified, encrypted local backup and restore foundations.
//!
//! This local device-key format is separate from destination-recipient
//! `.kagextra` transfer. It never switches the active book or prunes backups.

use crate::{
    attachment_maintenance::StorePin,
    attachments::{
        Attachment, AttachmentError, AttachmentId, catalog_for_pool, verify_encrypted_file,
    },
    crypto::{
        EncryptedObjectError, EncryptedObjectPurpose, RootEncryptionKey, decrypt_object,
        encrypt_object,
    },
    db::{
        BackupError, DatabaseError, backup_database, connect_read_only_database,
        verify_database_snapshot,
    },
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, load_device_key},
    storage::{
        ContentDigest, LocalDirectoryPublishError, LocalObjectPublishError, StoredObjectDescriptor,
        StoredObjectVerificationError, describe_file, ensure_directory_still_matches,
        ensure_path_still_names_open_file, open_regular_file_without_symlink,
        publish_private_directory_noreplace, publish_verified_local_copy, verify_file,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

const FORMAT: &str = "kAccounting-Extra-01.local-backup";
const VERSION: u32 = 1;
const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;
const ENVELOPE_FILE: &str = "envelope.json";
const CATALOG_FILE: &str = "catalog.kae";
const DATABASE_FILE: &str = "book.kae";

#[derive(Debug, Error)]
pub enum BookBackupError {
    #[error("backup or restore destination already exists")]
    DestinationExists,
    #[error("backup/restore paths must be outside the source tree and source attachment store")]
    UnsafePath,
    #[error("backup format or layout is invalid or unsupported")]
    InvalidFormat,
    #[error("backup catalog does not match its database's attachment references")]
    CatalogMismatch,
    #[error("backup catalog exceeds the 64 MiB safety limit")]
    CatalogTooLarge,
    #[error("backup work could not be cleaned up: {0}")]
    Cleanup(io::Error),
    #[error("published output requires verification at {path}: {source}")]
    PublishedUnverified {
        path: PathBuf,
        source: Box<BookBackupError>,
    },
    #[error("backup I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Key(#[from] KeyStoreError),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Snapshot(#[from] BackupError),
    #[error(transparent)]
    Attachment(#[from] AttachmentError),
    #[error(transparent)]
    Crypto(#[from] EncryptedObjectError),
    #[error(transparent)]
    Verify(#[from] StoredObjectVerificationError),
    #[error(transparent)]
    Copy(#[from] LocalObjectPublishError),
    #[error(transparent)]
    Publish(#[from] LocalDirectoryPublishError),
    #[error("backup worker did not complete")]
    Worker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalBackupOutcome {
    pub path: PathBuf,
    pub key_id: DeviceKeyId,
    pub attachment_count: usize,
    pub attachment_bytes: u64,
    pub cleanup_warning: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: String,
    version: u32,
    key_id: String,
}
impl Envelope {
    fn new(id: &DeviceKeyId) -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            key_id: id.as_str().to_owned(),
        }
    }
    fn validate(&self) -> Result<DeviceKeyId, BookBackupError> {
        if self.format != FORMAT || self.version != VERSION {
            return Err(BookBackupError::InvalidFormat);
        }
        DeviceKeyId::parse(&self.key_id).map_err(|_| BookBackupError::InvalidFormat)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    sha256: String,
    byte_length: String,
}
impl From<StoredObjectDescriptor> for Descriptor {
    fn from(value: StoredObjectDescriptor) -> Self {
        Self {
            sha256: value.digest().to_hex(),
            byte_length: value.byte_length().to_string(),
        }
    }
}
impl Descriptor {
    fn decode(&self) -> Result<StoredObjectDescriptor, BookBackupError> {
        let hash: ContentDigest = self
            .sha256
            .parse()
            .map_err(|_| BookBackupError::InvalidFormat)?;
        let length: u64 = self
            .byte_length
            .parse()
            .map_err(|_| BookBackupError::InvalidFormat)?;
        if hash.to_hex() != self.sha256 || length.to_string() != self.byte_length {
            return Err(BookBackupError::InvalidFormat);
        }
        Ok(StoredObjectDescriptor::new(hash, length))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogObject {
    id: String,
    ciphertext: Descriptor,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    format: String,
    version: u32,
    key_id: String,
    database_plaintext: Descriptor,
    database_ciphertext: Descriptor,
    attachments: Vec<CatalogObject>,
}
impl Catalog {
    fn validate(
        &self,
        key_id: &DeviceKeyId,
    ) -> Result<BTreeMap<String, StoredObjectDescriptor>, BookBackupError> {
        if self.format != FORMAT || self.version != VERSION || self.key_id != key_id.as_str() {
            return Err(BookBackupError::InvalidFormat);
        }
        self.database_plaintext.decode()?;
        self.database_ciphertext.decode()?;
        let mut objects = BTreeMap::new();
        for object in &self.attachments {
            AttachmentId::parse(&object.id).map_err(|_| BookBackupError::InvalidFormat)?;
            if objects
                .insert(object.id.clone(), object.ciphertext.decode()?)
                .is_some()
            {
                return Err(BookBackupError::InvalidFormat);
            }
        }
        Ok(objects)
    }
}

/// Takes a consistent SQLite snapshot first, then automatically selects every
/// attachment referenced by that snapshot, including drafts and posted entries.
/// No caller-supplied attachment list can accidentally omit evidence.
pub async fn create_local_backup(
    pool: &SqlitePool,
    attachment_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    key_id: &DeviceKeyId,
    keys: &impl DeviceKeyStore,
) -> Result<LocalBackupOutcome, BookBackupError> {
    let destination = checked_destination(destination.as_ref())?;
    let attachment_root = checked_directory(attachment_root.as_ref())?;
    if destination.starts_with(&attachment_root) {
        return Err(BookBackupError::UnsafePath);
    }
    let source_pin = StorePin::acquire(&attachment_root)
        .map_err(|error| BookBackupError::Io(io::Error::other(error)))?;
    let key = Arc::new(load_device_key(keys, key_id)?);
    let work = WorkDirectory::create(destination.parent().ok_or(BookBackupError::UnsafePath)?)?;
    let snapshot = work.path.join("snapshot.sqlite3");
    let snapshot_outcome = backup_database(pool, &snapshot).await?;
    if snapshot_outcome.cleanup_warning().is_some() {
        return Err(BookBackupError::Cleanup(io::Error::other(
            "private SQLite snapshot cleanup did not complete",
        )));
    }
    let snapshot_pool = connect_read_only_database(&snapshot).await?;
    let selected = catalog_for_pool(&snapshot_pool).await;
    snapshot_pool.close().await;
    let selected = selected?;
    let id = key_id.clone();
    let backup_path = work.path.join("package");
    let worker_path = backup_path.clone();
    let worker_key = key.clone();
    let worker_lock = work.lock.clone();
    let source_snapshot = snapshot.clone();
    let worker_source_pin = source_pin.clone();
    tokio::task::spawn_blocking(move || {
        let _worker_lock = worker_lock;
        let _worker_source_pin = worker_source_pin;
        build_package(
            &source_snapshot,
            &attachment_root,
            &worker_path,
            &id,
            &worker_key,
            &selected,
        )
    })
    .await
    .map_err(|_| BookBackupError::Worker)??;
    // Snapshot plaintext is removed before a package can become public.
    std::fs::remove_file(&snapshot)?;
    let verified = inspect_package(
        &backup_path,
        &work.path.join("verify.sqlite3"),
        key_id,
        &key,
        &work.lock,
    )
    .await?;
    std::fs::remove_file(work.path.join("verify.sqlite3"))?;
    work.validate()?;
    publish_private_directory_noreplace(&backup_path, &destination)?;
    if let Err(error) = File::open(destination.parent().ok_or(BookBackupError::UnsafePath)?)
        .and_then(|directory| directory.sync_all())
    {
        return Err(BookBackupError::PublishedUnverified {
            path: destination,
            source: Box::new(error.into()),
        });
    }
    let published_verify = work.path.join("published-verify.sqlite3");
    let public_result =
        inspect_package(&destination, &published_verify, key_id, &key, &work.lock).await;
    match public_result {
        Ok(actual) if actual == verified => {}
        Ok(_) => {
            return Err(BookBackupError::PublishedUnverified {
                path: destination,
                source: Box::new(BookBackupError::CatalogMismatch),
            });
        }
        Err(error) => {
            return Err(BookBackupError::PublishedUnverified {
                path: destination,
                source: Box::new(error),
            });
        }
    }
    std::fs::remove_file(published_verify).map_err(|error| {
        BookBackupError::PublishedUnverified {
            path: destination.clone(),
            source: Box::new(error.into()),
        }
    })?;
    let mut outcome = outcome(destination, key_id.clone(), &verified)?;
    if let Err(error) = work.cleanup() {
        outcome.cleanup_warning = Some(error.to_string());
    }
    Ok(outcome)
}

/// Authenticates an existing backup without publishing a restored book. The
/// temporary database is private and removed before success is returned.
pub async fn verify_local_backup(
    source: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<LocalBackupOutcome, BookBackupError> {
    let source = checked_directory(source.as_ref())?;
    let source_tree = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(BookBackupError::UnsafePath)?;
    if source.starts_with(source_tree) {
        return Err(BookBackupError::UnsafePath);
    }
    let key_id = read_envelope(&source)?.validate()?;
    let key = Arc::new(load_device_key(keys, &key_id)?);
    let work = WorkDirectory::create(source.parent().ok_or(BookBackupError::UnsafePath)?)?;
    let database = work.path.join("verify.sqlite3");
    let selected = inspect_package(&source, &database, &key_id, &key, &work.lock).await?;
    std::fs::remove_file(database)?;
    work.cleanup()?;
    outcome(source, key_id, &selected)
}

/// Restores into a new local directory and returns it only after complete
/// authentication, SQLite audit, attachment-set comparison, and byte verification.
/// The source backup and any existing destination are never modified.
pub async fn restore_local_backup(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    keys: &impl DeviceKeyStore,
) -> Result<LocalBackupOutcome, BookBackupError> {
    let source = checked_directory(source.as_ref())?;
    let destination = checked_destination(destination.as_ref())?;
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err(BookBackupError::UnsafePath);
    }
    let envelope = read_envelope(&source)?;
    let key_id = envelope.validate()?;
    let key = Arc::new(load_device_key(keys, &key_id)?);
    let work = WorkDirectory::create(destination.parent().ok_or(BookBackupError::UnsafePath)?)?;
    let book = work.path.join("book");
    create_private_directory(&book)?;
    let database = book.join("book.sqlite3");
    let selected = inspect_package(&source, &database, &key_id, &key, &work.lock).await?;
    let target_objects = book.join("attachments");
    create_private_directory(&target_objects)?;
    let source_objects = source.join("attachments");
    let copy_selected = selected.clone();
    let copy_key = key.clone();
    let copy_target = target_objects.clone();
    let copy_lock = work.lock.clone();
    tokio::task::spawn_blocking(move || -> Result<(), BookBackupError> {
        let _copy_lock = copy_lock;
        for attachment in &copy_selected {
            let filename = format!("{}.kae", attachment.id.as_str());
            let copied = publish_verified_local_copy(
                source_objects.join(&filename),
                copy_target.join(&filename),
                attachment.ciphertext,
            )?;
            if copied.cleanup_warning().is_some() {
                return Err(BookBackupError::InvalidFormat);
            }
            verify_encrypted_file(
                &copy_target.join(filename),
                attachment.plaintext,
                attachment.ciphertext,
                &copy_key,
            )?;
        }
        File::open(&copy_target)?.sync_all()?;
        Ok(())
    })
    .await
    .map_err(|_| BookBackupError::Worker)??;
    let binding =
        serde_json::to_vec(&Envelope::new(&key_id)).map_err(|_| BookBackupError::InvalidFormat)?;
    write_private_file(&book.join("device-key.json"), &binding)?;
    verify_database_snapshot(&database).await?;
    File::open(&book)?.sync_all()?;
    work.validate()?;
    publish_private_directory_noreplace(&book, &destination)?;
    if let Err(error) = File::open(destination.parent().ok_or(BookBackupError::UnsafePath)?)
        .and_then(|directory| directory.sync_all())
    {
        return Err(BookBackupError::PublishedUnverified {
            path: destination,
            source: Box::new(error.into()),
        });
    }
    if let Err(error) = verify_published_book(&destination, &key_id, &key, &selected).await {
        return Err(BookBackupError::PublishedUnverified {
            path: destination,
            source: Box::new(error),
        });
    }
    let mut result = outcome(destination, key_id, &selected)?;
    if let Err(error) = work.cleanup() {
        result.cleanup_warning = Some(error.to_string());
    }
    Ok(result)
}

async fn verify_published_book(
    path: &Path,
    key_id: &DeviceKeyId,
    key: &Arc<RootEncryptionKey>,
    expected: &[Attachment],
) -> Result<(), BookBackupError> {
    let identity = std::fs::symlink_metadata(path)?;
    checked_directory(path)?;
    if directory_names(path)?
        != ["book.sqlite3", "attachments", "device-key.json"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    {
        return Err(BookBackupError::InvalidFormat);
    }
    let mut binding = String::new();
    open_regular_file_without_symlink(&path.join("device-key.json"))?
        .take(4097)
        .read_to_string(&mut binding)?;
    if binding.len() > 4096 {
        return Err(BookBackupError::InvalidFormat);
    }
    let binding: Envelope =
        serde_json::from_str(&binding).map_err(|_| BookBackupError::InvalidFormat)?;
    if binding.validate()? != *key_id {
        return Err(BookBackupError::InvalidFormat);
    }
    let database = path.join("book.sqlite3");
    verify_database_snapshot(&database).await?;
    let pool = connect_read_only_database(&database).await?;
    let actual = catalog_for_pool(&pool).await;
    pool.close().await;
    if actual? != expected {
        return Err(BookBackupError::CatalogMismatch);
    }
    let objects = checked_directory(&path.join("attachments"))?;
    if directory_names(&objects)?
        != expected
            .iter()
            .map(|a| format!("{}.kae", a.id.as_str()))
            .collect()
    {
        return Err(BookBackupError::CatalogMismatch);
    }
    let selected = expected.to_vec();
    let worker_key = key.clone();
    tokio::task::spawn_blocking(move || -> Result<(), BookBackupError> {
        for attachment in selected {
            verify_encrypted_file(
                &objects.join(format!("{}.kae", attachment.id.as_str())),
                attachment.plaintext,
                attachment.ciphertext,
                &worker_key,
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|_| BookBackupError::Worker)??;
    ensure_directory_still_matches(path, &identity)?;
    Ok(())
}

fn outcome(
    path: PathBuf,
    key_id: DeviceKeyId,
    selected: &[Attachment],
) -> Result<LocalBackupOutcome, BookBackupError> {
    let attachment_bytes = selected.iter().try_fold(0_u64, |sum, a| {
        sum.checked_add(a.plaintext.byte_length())
            .ok_or(BookBackupError::InvalidFormat)
    })?;
    Ok(LocalBackupOutcome {
        path,
        key_id,
        attachment_count: selected.len(),
        attachment_bytes,
        cleanup_warning: None,
    })
}

fn build_package(
    snapshot: &Path,
    objects: &Path,
    package: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
    selected: &[Attachment],
) -> Result<(), BookBackupError> {
    create_private_directory(package)?;
    let target_objects = package.join("attachments");
    create_private_directory(&target_objects)?;
    let plaintext = describe_file(snapshot)?;
    let mut input = open_regular_file_without_symlink(snapshot)?;
    let mut output = private_file(&package.join(DATABASE_FILE))?;
    encrypt_object(&mut input, &mut output, key, EncryptedObjectPurpose::Backup)?;
    output.sync_all()?;
    drop(output);
    let ciphertext = describe_file(package.join(DATABASE_FILE))?;
    let mut catalog = Catalog {
        format: FORMAT.to_owned(),
        version: VERSION,
        key_id: key_id.as_str().to_owned(),
        database_plaintext: plaintext.into(),
        database_ciphertext: ciphertext.into(),
        attachments: Vec::with_capacity(selected.len()),
    };
    for attachment in selected {
        let filename = format!("{}.kae", attachment.id.as_str());
        let result = publish_verified_local_copy(
            objects.join(&filename),
            target_objects.join(&filename),
            attachment.ciphertext,
        )?;
        if result.cleanup_warning().is_some() {
            return Err(BookBackupError::InvalidFormat);
        }
        verify_encrypted_file(
            &target_objects.join(&filename),
            attachment.plaintext,
            attachment.ciphertext,
            key,
        )?;
        catalog.attachments.push(CatalogObject {
            id: attachment.id.as_str().to_owned(),
            ciphertext: attachment.ciphertext.into(),
        });
    }
    catalog.attachments.sort_by(|a, b| a.id.cmp(&b.id));
    let bytes = serde_json::to_vec(&catalog).map_err(|_| BookBackupError::InvalidFormat)?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(BookBackupError::CatalogTooLarge);
    }
    let mut output = private_file(&package.join(CATALOG_FILE))?;
    encrypt_object(
        &mut Cursor::new(bytes),
        &mut output,
        key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    output.sync_all()?;
    write_private_file(
        &package.join(ENVELOPE_FILE),
        &serde_json::to_vec(&Envelope::new(key_id)).map_err(|_| BookBackupError::InvalidFormat)?,
    )?;
    File::open(&target_objects)?.sync_all()?;
    File::open(package)?.sync_all()?;
    Ok(())
}

async fn inspect_package(
    package: &Path,
    database: &Path,
    key_id: &DeviceKeyId,
    key: &Arc<RootEncryptionKey>,
    work_lock: &Arc<File>,
) -> Result<Vec<Attachment>, BookBackupError> {
    let root_identity = std::fs::symlink_metadata(package)?;
    let envelope = read_envelope(package)?;
    if envelope.validate()? != *key_id {
        return Err(BookBackupError::InvalidFormat);
    }
    let catalog = decrypt_catalog(package, key_id, key)?;
    let expected = catalog.validate(key_id)?;
    check_layout(package, expected.keys())?;
    verify_file(
        package.join(DATABASE_FILE),
        catalog.database_ciphertext.decode()?,
    )?;
    let mut input = open_regular_file_without_symlink(&package.join(DATABASE_FILE))?;
    let mut output = private_file(database)?;
    decrypt_object(&mut input, &mut output, key, EncryptedObjectPurpose::Backup)?;
    output.sync_all()?;
    drop(output);
    verify_file(database, catalog.database_plaintext.decode()?)?;
    verify_database_snapshot(database).await?;
    let pool = connect_read_only_database(database).await?;
    let selected = catalog_for_pool(&pool).await;
    pool.close().await;
    let selected = selected?;
    let actual: BTreeMap<_, _> = selected
        .iter()
        .map(|a| (a.id.as_str().to_owned(), a.ciphertext))
        .collect();
    if actual != expected {
        return Err(BookBackupError::CatalogMismatch);
    }
    let worker_selected = selected.clone();
    let object_root = package.join("attachments");
    let worker_key = key.clone();
    let worker_lock = work_lock.clone();
    tokio::task::spawn_blocking(move || -> Result<(), BookBackupError> {
        let _worker_lock = worker_lock;
        for a in &worker_selected {
            verify_encrypted_file(
                &object_root.join(format!("{}.kae", a.id.as_str())),
                a.plaintext,
                a.ciphertext,
                &worker_key,
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|_| BookBackupError::Worker)??;
    check_layout(package, expected.keys())?;
    ensure_directory_still_matches(package, &root_identity)?;
    Ok(selected)
}

fn decrypt_catalog(
    package: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
) -> Result<Catalog, BookBackupError> {
    let mut input = open_regular_file_without_symlink(&package.join(CATALOG_FILE))?;
    let mut bounded = BoundedBytes(Vec::new());
    decrypt_object(
        &mut input,
        &mut bounded,
        key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    let catalog: Catalog =
        serde_json::from_slice(&bounded.0).map_err(|_| BookBackupError::InvalidFormat)?;
    catalog.validate(key_id)?;
    Ok(catalog)
}
struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_CATALOG_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("backup catalog size limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn read_envelope(package: &Path) -> Result<Envelope, BookBackupError> {
    let file = open_regular_file_without_symlink(&package.join(ENVELOPE_FILE))?;
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(BookBackupError::InvalidFormat);
    }
    let value: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| BookBackupError::InvalidFormat)?;
    value.validate()?;
    Ok(value)
}
fn check_layout<'a>(
    root: &Path,
    ids: impl Iterator<Item = &'a String>,
) -> Result<(), BookBackupError> {
    checked_directory(root)?;
    let expected: BTreeSet<String> = [ENVELOPE_FILE, CATALOG_FILE, DATABASE_FILE, "attachments"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if directory_names(root)? != expected {
        return Err(BookBackupError::InvalidFormat);
    }
    for file in [ENVELOPE_FILE, CATALOG_FILE, DATABASE_FILE] {
        let _ = open_regular_file_without_symlink(&root.join(file))?;
    }
    let objects = root.join("attachments");
    checked_directory(&objects)?;
    let expected: BTreeSet<String> = ids.map(|id| format!("{id}.kae")).collect();
    if directory_names(&objects)? != expected {
        return Err(BookBackupError::InvalidFormat);
    }
    for name in expected {
        let _ = open_regular_file_without_symlink(&objects.join(name))?;
    }
    Ok(())
}
fn directory_names(path: &Path) -> Result<BTreeSet<String>, BookBackupError> {
    std::fs::read_dir(path)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| BookBackupError::InvalidFormat)
        })
        .collect()
}
fn checked_directory(path: &Path) -> Result<PathBuf, BookBackupError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BookBackupError::UnsafePath);
    }
    Ok(std::fs::canonicalize(path)?)
}
fn checked_destination(path: &Path) -> Result<PathBuf, BookBackupError> {
    let parent = checked_directory(path.parent().ok_or(BookBackupError::UnsafePath)?)?;
    let candidate = parent.join(path.file_name().ok_or(BookBackupError::UnsafePath)?);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(BookBackupError::UnsafePath)?;
    if candidate.starts_with(source) {
        return Err(BookBackupError::UnsafePath);
    }
    match std::fs::symlink_metadata(&candidate) {
        Ok(_) => Err(BookBackupError::DestinationExists),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(candidate),
        Err(e) => Err(e.into()),
    }
}
fn create_private_directory(path: &Path) -> io::Result<()> {
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
fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Recovery must run before allowing normal backup/restore work at application
/// startup. Locks distinguish an abandoned process from another live worker.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct BackupWorkRecovery {
    pub removed_count: usize,
    pub active_work: Vec<PathBuf>,
    pub unrecognized_work: Vec<PathBuf>,
}

/// Removes only marked, owner-only, unlocked work directories created by this
/// module. A completed backup or restored book is never a cleanup candidate.
/// Unrecognized directories are preserved and must block automatic startup use
/// until their status is understood; this function does not guess ownership.
pub fn recover_interrupted_local_backup_work(
    parent: impl AsRef<Path>,
) -> Result<BackupWorkRecovery, BookBackupError> {
    let parent = checked_directory(parent.as_ref())?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(BookBackupError::UnsafePath)?;
    if parent.starts_with(source) {
        return Err(BookBackupError::UnsafePath);
    }
    let parent_identity = std::fs::symlink_metadata(&parent)?;
    let mut result = BackupWorkRecovery::default();
    for entry in std::fs::read_dir(&parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name
            .strip_prefix(".kAccounting-Extra-01-encrypted-backup-")
            .and_then(|name| name.strip_suffix(".work"))
        else {
            continue;
        };
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if DeviceKeyId::parse(id).is_err()
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
        {
            result.unrecognized_work.push(path);
            continue;
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o777 != 0o700 {
            result.unrecognized_work.push(path);
            continue;
        }
        let lock_path = path.join(".lock");
        let Ok(lock) = open_regular_file_without_symlink(&lock_path) else {
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
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let mut marker = String::new();
        (&lock).take(161).read_to_string(&mut marker)?;
        if marker != format!("kAccounting-Extra-01/encrypted-backup-work/v1/{id}\n") {
            result.unrecognized_work.push(path);
            continue;
        }
        ensure_directory_still_matches(&parent, &parent_identity)?;
        ensure_directory_still_matches(&path, &metadata)?;
        ensure_path_still_names_open_file(&lock_path, &lock)?;
        std::fs::remove_dir_all(&path)?;
        result.removed_count += 1;
    }
    File::open(&parent)?.sync_all()?;
    result.active_work.sort();
    result.unrecognized_work.sort();
    Ok(result)
}

struct WorkDirectory {
    path: PathBuf,
    identity: std::fs::Metadata,
    cleaned: bool,
    lock: Arc<File>,
}
impl WorkDirectory {
    fn create(parent: &Path) -> Result<Self, BookBackupError> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| KeyStoreError::RandomUnavailable)?;
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = parent.join(format!(".kAccounting-Extra-01-encrypted-backup-{id}.work"));
        create_private_directory(&path)?;
        let identity = std::fs::symlink_metadata(&path)?;
        let mut lock = private_file(&path.join(".lock"))?;
        lock.try_lock()
            .map_err(|_| io::Error::other("could not lock newly created backup work"))?;
        lock.write_all(format!("kAccounting-Extra-01/encrypted-backup-work/v1/{id}\n").as_bytes())?;
        lock.sync_all()?;
        File::open(&path)?.sync_all()?;
        File::open(parent)?.sync_all()?;
        Ok(Self {
            path,
            identity,
            cleaned: false,
            lock: Arc::new(lock),
        })
    }
    fn validate(&self) -> io::Result<()> {
        ensure_directory_still_matches(&self.path, &self.identity)?;
        ensure_path_still_names_open_file(&self.path.join(".lock"), &self.lock)
    }
    fn cleanup(mut self) -> Result<(), BookBackupError> {
        if Arc::strong_count(&self.lock) != 1 {
            return Err(BookBackupError::Cleanup(io::Error::other(
                "backup work is still held by an active worker",
            )));
        }
        self.validate()?;
        std::fs::remove_dir_all(&self.path).map_err(BookBackupError::Cleanup)?;
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for WorkDirectory {
    fn drop(&mut self) {
        // Dropping an awaiting async future does not stop spawn_blocking.
        // Preserve its private work and OS lock until the final worker exits;
        // startup recovery can then remove the abandoned directory safely.
        if !self.cleaned && Arc::strong_count(&self.lock) == 1 && self.validate().is_ok() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    fn fixture() -> PathBuf {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("kAccounting-Extra-01-backup-recovery-{id}"));
        std::fs::create_dir(&path).unwrap();
        std::fs::canonicalize(path).unwrap()
    }
    #[test]
    fn cancelled_awaiter_preserves_worker_work_until_recovery_after_worker_exit() {
        let parent = fixture();
        let work = WorkDirectory::create(&parent).unwrap();
        let path = work.path.clone();
        let worker_lock = work.lock.clone();
        std::fs::write(path.join("snapshot.sqlite3"), b"private snapshot").unwrap();
        drop(work);
        assert!(path.join("snapshot.sqlite3").exists());
        let recovery = recover_interrupted_local_backup_work(&parent).unwrap();
        assert_eq!(recovery.active_work, vec![path.clone()]);
        assert_eq!(recovery.removed_count, 0);
        drop(worker_lock);
        let recovery = recover_interrupted_local_backup_work(&parent).unwrap();
        assert_eq!(recovery.removed_count, 1);
        assert!(!path.exists());
        std::fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn explicit_cleanup_refuses_to_remove_work_held_by_an_active_worker() {
        let parent = fixture();
        let work = WorkDirectory::create(&parent).unwrap();
        let path = work.path.clone();
        let worker_lock = work.lock.clone();
        assert!(matches!(work.cleanup(), Err(BookBackupError::Cleanup(_))));
        assert!(path.exists());
        drop(worker_lock);
        assert_eq!(
            recover_interrupted_local_backup_work(&parent)
                .unwrap()
                .removed_count,
            1
        );
        std::fs::remove_dir(parent).unwrap();
    }
    #[test]
    fn recovery_skips_live_workers_and_removes_abandoned_plaintext_without_touching_completed_books()
     {
        let parent = fixture();
        let active = WorkDirectory::create(&parent).unwrap();
        let mut abandoned = WorkDirectory::create(&parent).unwrap();
        std::fs::write(
            abandoned.path.join("snapshot.sqlite3"),
            b"private interrupted database",
        )
        .unwrap();
        let abandoned_path = abandoned.path.clone();
        abandoned.cleaned = true;
        drop(abandoned);
        let complete = parent.join("complete.kabackup");
        std::fs::create_dir(&complete).unwrap();
        std::fs::write(complete.join("sentinel"), b"keep").unwrap();
        let result = recover_interrupted_local_backup_work(&parent).unwrap();
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.active_work, vec![active.path.clone()]);
        assert!(!abandoned_path.exists());
        assert_eq!(std::fs::read(complete.join("sentinel")).unwrap(), b"keep");
        assert!(result.unrecognized_work.is_empty());
        drop(active);
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn unmarked_or_malformed_work_is_preserved_instead_of_guessed() {
        let parent = fixture();
        let path = parent.join(format!(
            ".kAccounting-Extra-01-encrypted-backup-{}.work",
            "a".repeat(32)
        ));
        create_private_directory(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"keep").unwrap();
        let result = recover_interrupted_local_backup_work(&parent).unwrap();
        assert_eq!(result.removed_count, 0);
        assert_eq!(result.unrecognized_work, vec![path.clone()]);
        assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"keep");
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn recovery_never_follows_a_work_directory_symlink() {
        let parent = fixture();
        let target = parent.join("external");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("sentinel"), b"keep").unwrap();
        let link = parent.join(format!(
            ".kAccounting-Extra-01-encrypted-backup-{}.work",
            "b".repeat(32)
        ));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let result = recover_interrupted_local_backup_work(&parent).unwrap();
        assert_eq!(result.removed_count, 0);
        assert_eq!(result.unrecognized_work, vec![link]);
        assert_eq!(std::fs::read(target.join("sentinel")).unwrap(), b"keep");
        std::fs::remove_dir_all(parent).unwrap();
    }
}
