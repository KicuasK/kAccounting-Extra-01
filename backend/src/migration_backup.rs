//! Backup-first legacy upgrades. Never migrate a live book until its original
//! schema and every referenced encrypted object have a durable verified copy.
use crate::{
    attachment_maintenance::StorePin,
    attachments::{self, Attachment},
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, decrypt_object, encrypt_object},
    db,
    keys::DeviceKeyId,
    storage::{self, ContentDigest, StoredObjectDescriptor},
};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationBackupError {
    #[error("migration backup I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Database(#[from] db::DatabaseError),
    #[error(transparent)]
    Crypto(#[from] crate::crypto::EncryptedObjectError),
    #[error(transparent)]
    Attachment(#[from] attachments::AttachmentError),
    #[error(transparent)]
    Maintenance(#[from] crate::attachment_maintenance::MaintenanceError),
    #[error(transparent)]
    Publish(#[from] storage::LocalDirectoryPublishError),
    #[error(transparent)]
    Verify(#[from] storage::StoredObjectVerificationError),
    #[error("interrupted migration backup work requires inspection before upgrade")]
    RecoveryRequired,
    #[error("migration backup catalog, identity or layout is invalid")]
    Invalid,
    #[error("migration backup database operation failed: {0}")]
    Query(#[from] sqlx::Error),
    #[error("restored migration checkpoint requires verification at {path}: {source}")]
    PublishedUnverified {
        path: PathBuf,
        source: Box<MigrationBackupError>,
    },
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    digest: String,
    bytes: u64,
}
impl Descriptor {
    fn from(value: StoredObjectDescriptor) -> Self {
        Self {
            digest: value.digest().to_hex(),
            bytes: value.byte_length(),
        }
    }
    fn value(&self) -> Result<StoredObjectDescriptor, MigrationBackupError> {
        Ok(StoredObjectDescriptor::new(
            self.digest
                .parse::<ContentDigest>()
                .map_err(|_| MigrationBackupError::Invalid)?,
            self.bytes,
        ))
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Object {
    id: String,
    plaintext: Descriptor,
    ciphertext: Descriptor,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    key_id: String,
    database: Descriptor,
    objects: Vec<Object>,
}
const FORMAT: &str = "kAccounting-Extra-01.pre-migration-backup";
const MAX_MANIFEST: u64 = 64 * 1024 * 1024;

/// Called while the runtime owns the book lock. Current schemas cause no work.
/// The returned package is retained indefinitely; automatic retention never
/// deletes the only pre-upgrade recovery point.
pub async fn prepare_upgrade(
    root: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
) -> Result<Option<PathBuf>, MigrationBackupError> {
    private_directory(root)?;
    outside_source(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".migration-work-")
        {
            return Err(MigrationBackupError::RecoveryRequired);
        }
    }
    if !db::pending_schema_migrations(root.join("book.sqlite3")).await? {
        return Ok(None);
    }
    let pin = StorePin::acquire(root.join("attachments"))?;
    let work = Work::new(root)?;
    let original = work.0.join("original.sqlite3");
    let source = db::connect_read_only_database(&root.join("book.sqlite3")).await?;
    let snapshot = sqlx::query("VACUUM INTO ?")
        .bind(original.to_str().ok_or(MigrationBackupError::Invalid)?)
        .execute(&source)
        .await;
    source.close().await;
    snapshot?;
    fs::set_permissions(&original, fs::Permissions::from_mode(0o600))?;
    db::pending_schema_migrations(&original).await?;
    let upgraded = work.0.join("audit.sqlite3");
    copy_private(&original, &upgraded)?;
    let pool = db::open_database(&upgraded).await?;
    let selected = attachments::catalog_for_pool(&pool).await;
    pool.close().await;
    let selected = selected?;
    let package = work.0.join("package");
    mkdir(&package)?;
    mkdir(&package.join("attachments"))?;
    let manifest = Manifest {
        format: FORMAT.into(),
        version: 1,
        key_id: key_id.as_str().into(),
        database: Descriptor::from(storage::describe_file(&original)?),
        objects: objects(&selected),
    };
    let mut input = storage::open_regular_file_without_symlink(&original)?;
    let mut output = new_file(&package.join("database.kae"))?;
    encrypt_object(&mut input, &mut output, key, EncryptedObjectPurpose::Backup)?;
    output.sync_all()?;
    for item in &selected {
        pin.validate()?;
        let filename = format!("{}.kae", item.id.as_str());
        let source = pin.root().join(&filename);
        attachments::verify_encrypted_file(&source, item.plaintext, item.ciphertext, key)?;
        let destination = package.join("attachments").join(filename);
        copy_private(&source, &destination)?;
        attachments::verify_encrypted_file(&destination, item.plaintext, item.ciphertext, key)?;
    }
    let mut bytes = Limited(zeroize::Zeroizing::new(Vec::new()));
    serde_json::to_writer(&mut bytes, &manifest).map_err(|_| MigrationBackupError::Invalid)?;
    let mut output = new_file(&package.join("manifest.kae"))?;
    encrypt_object(
        &mut Cursor::new(&*bytes.0),
        &mut output,
        key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    output.sync_all()?;
    File::open(package.join("attachments"))?.sync_all()?;
    File::open(&package)?.sync_all()?;
    verify_in_work(&package, key_id, key, &work.0.join("verify-before")).await?;
    pin.validate()?;
    let name = work
        .0
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(MigrationBackupError::Invalid)?;
    let destination = root.join(format!(
        "pre-migration-{}.kabackup",
        name.trim_start_matches(".migration-work-")
    ));
    storage::publish_private_directory_noreplace(&package, &destination)?;
    File::open(root)?.sync_all()?;
    verify_in_work(&destination, key_id, key, &work.0.join("verify-after")).await?;
    pin.validate()?;
    work.cleanup()?;
    Ok(Some(destination))
}

fn objects(selected: &[Attachment]) -> Vec<Object> {
    let mut result: Vec<_> = selected
        .iter()
        .map(|a| Object {
            id: a.id.as_str().into(),
            plaintext: Descriptor::from(a.plaintext),
            ciphertext: Descriptor::from(a.ciphertext),
        })
        .collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

/// Independently decrypts and validates the original schema, upgrades only a
/// disposable validation copy, then verifies its exact complete reference set.
pub async fn verify_pre_migration_backup(
    package: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
) -> Result<(), MigrationBackupError> {
    private_directory(package)?;
    outside_source(package)?;
    let parent = package.parent().ok_or(MigrationBackupError::Invalid)?;
    let work = Work::new(parent)?;
    verify_in_work(package, key_id, key, &work.0.join("verify")).await?;
    work.cleanup()?;
    Ok(())
}
/// Restores into a NEW local destination using the selected book's existing
/// key binding. Only its private validation copy receives schema upgrades.
/// Existing books, keys and active selection are never replaced.
pub async fn restore_pre_migration_backup(
    package: &Path,
    destination: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
) -> Result<crate::book_backup::LocalBackupOutcome, MigrationBackupError> {
    outside_source(package)?;
    outside_source(destination)?;
    private_directory(package)?;
    let parent = destination.parent().ok_or(MigrationBackupError::Invalid)?;
    private_directory(parent)?;
    if destination.starts_with(package)
        || package.starts_with(destination)
        || destination.file_name().is_none()
        || destination.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(MigrationBackupError::Invalid);
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(
                io::Error::new(io::ErrorKind::AlreadyExists, "restore destination exists").into(),
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let work = Work::new(parent)?;
    let validation = work.0.join("validation");
    verify_in_work(package, key_id, key, &validation).await?;
    let book = work.0.join("book");
    mkdir(&book)?;
    mkdir(&book.join("attachments"))?;
    let expected_database = storage::describe_file(validation.join("audit.sqlite3"))?;
    copy_private(
        &validation.join("audit.sqlite3"),
        &book.join("book.sqlite3"),
    )?;
    let pool = db::connect_read_only_database(&book.join("book.sqlite3")).await?;
    let references = attachments::catalog_for_pool(&pool).await;
    pool.close().await;
    let references = references?;
    for attachment in &references {
        let name = format!("{}.kae", attachment.id.as_str());
        let target = book.join("attachments").join(&name);
        copy_private(&package.join("attachments").join(&name), &target)?;
        attachments::verify_encrypted_file(
            &target,
            attachment.plaintext,
            attachment.ciphertext,
            key,
        )?;
    }
    let binding = serde_json::json!({
        "format": "kAccounting-Extra-01.local-backup", "version": 1, "key_id": key_id.as_str(),
    });
    let mut output = new_file(&book.join("device-key.json"))?;
    let binding_bytes = serde_json::to_vec(&binding).map_err(|_| MigrationBackupError::Invalid)?;
    output.write_all(&binding_bytes)?;
    output.sync_all()?;
    File::open(book.join("attachments"))?.sync_all()?;
    File::open(&book)?.sync_all()?;
    db::verify_database_snapshot(book.join("book.sqlite3")).await?;
    storage::verify_file(book.join("book.sqlite3"), expected_database)?;
    storage::ensure_directory_still_matches(parent, &work.2)?;
    storage::ensure_directory_still_matches(&work.0, &work.1)?;
    storage::publish_private_directory_noreplace(&book, destination)?;
    let published = async {
        File::open(parent)?.sync_all()?;
        db::verify_database_snapshot(destination.join("book.sqlite3")).await?;
        storage::verify_file(destination.join("book.sqlite3"), expected_database)?;
        let mut actual_binding = Vec::new();
        storage::open_regular_file_without_symlink(&destination.join("device-key.json"))?
            .take(4097)
            .read_to_end(&mut actual_binding)?;
        if actual_binding != binding_bytes {
            return Err(MigrationBackupError::Invalid);
        }
        let pool = db::connect_read_only_database(&destination.join("book.sqlite3")).await?;
        let actual = attachments::catalog_for_pool(&pool).await;
        pool.close().await;
        if actual? != references {
            return Err(MigrationBackupError::Invalid);
        }
        for attachment in &references {
            attachments::verify_encrypted_file(
                &destination
                    .join("attachments")
                    .join(format!("{}.kae", attachment.id.as_str())),
                attachment.plaintext,
                attachment.ciphertext,
                key,
            )?;
        }
        Ok::<_, MigrationBackupError>(())
    }
    .await;
    if let Err(source) = published {
        return Err(MigrationBackupError::PublishedUnverified {
            path: destination.to_owned(),
            source: Box::new(source),
        });
    }
    let cleanup_warning = work.cleanup().err().map(|error| error.to_string());
    Ok(crate::book_backup::LocalBackupOutcome {
        path: destination.to_owned(),
        key_id: key_id.clone(),
        attachment_count: references.len(),
        attachment_bytes: references.iter().map(|a| a.plaintext.byte_length()).sum(),
        cleanup_warning,
    })
}

async fn verify_in_work(
    package: &Path,
    key_id: &DeviceKeyId,
    key: &RootEncryptionKey,
    work: &Path,
) -> Result<(), MigrationBackupError> {
    private_directory(package)?;
    private_directory(&package.join("attachments"))?;
    let mut names: Vec<_> = fs::read_dir(package)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    names.sort();
    if names != ["attachments", "database.kae", "manifest.kae"] {
        return Err(MigrationBackupError::Invalid);
    }

    let mut input = storage::open_regular_file_without_symlink(&package.join("manifest.kae"))?;
    let mut bytes = Limited(zeroize::Zeroizing::new(Vec::new()));
    decrypt_object(
        &mut input,
        &mut bytes,
        key,
        EncryptedObjectPurpose::BackupManifest,
    )?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes.0).map_err(|_| MigrationBackupError::Invalid)?;
    if manifest.format != FORMAT || manifest.version != 1 || manifest.key_id != key_id.as_str() {
        return Err(MigrationBackupError::Invalid);
    }
    mkdir(work)?;
    let original = work.join("original.sqlite3");
    let mut input = storage::open_regular_file_without_symlink(&package.join("database.kae"))?;
    let mut output = new_file(&original)?;
    decrypt_object(&mut input, &mut output, key, EncryptedObjectPurpose::Backup)?;
    output.sync_all()?;
    drop(output);
    if storage::describe_file(&original)? != manifest.database.value()?
        || !db::pending_schema_migrations(&original).await?
    {
        return Err(MigrationBackupError::Invalid);
    }
    let audit = work.join("audit.sqlite3");
    copy_private(&original, &audit)?;
    let pool = db::open_database(&audit).await?;
    let selected = attachments::catalog_for_pool(&pool).await;
    pool.close().await;
    let selected = selected?;
    if objects(&selected) != manifest.objects {
        return Err(MigrationBackupError::Invalid);
    }
    let mut filenames: Vec<String> = fs::read_dir(package.join("attachments"))?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    filenames.sort();
    let mut expected: Vec<_> = selected
        .iter()
        .map(|a| format!("{}.kae", a.id.as_str()))
        .collect();
    expected.sort();
    if filenames != expected {
        return Err(MigrationBackupError::Invalid);
    }
    for item in selected {
        attachments::verify_encrypted_file(
            &package
                .join("attachments")
                .join(format!("{}.kae", item.id.as_str())),
            item.plaintext,
            item.ciphertext,
            key,
        )?;
    }
    Ok(())
}
struct Limited(zeroize::Zeroizing<Vec<u8>>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (self.0.len() as u64).saturating_add(bytes.len() as u64) > MAX_MANIFEST {
            return Err(io::Error::other("manifest limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn mkdir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)
}
fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
}
fn private_directory(path: &Path) -> Result<(), MigrationBackupError> {
    let mut current = PathBuf::new();
    for part in path.components() {
        if matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(MigrationBackupError::Invalid);
        }
        current.push(part);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(MigrationBackupError::Invalid);
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(MigrationBackupError::Invalid);
    }
    Ok(())
}
fn copy_private(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = storage::open_regular_file_without_symlink(source)?;
    let mut output = new_file(destination)?;
    storage::ensure_path_still_names_open_file(source, &input)?;
    io::copy(&mut input, &mut output)?;
    storage::ensure_path_still_names_open_file(source, &input)?;
    output.sync_all()
}
fn outside_source(path: &Path) -> Result<(), MigrationBackupError> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(MigrationBackupError::Invalid)?;
    if path.starts_with(source) || source.starts_with(path) {
        return Err(MigrationBackupError::Invalid);
    }
    Ok(())
}
struct Work(PathBuf, fs::Metadata, fs::Metadata);
impl Work {
    fn new(parent: &Path) -> Result<Self, MigrationBackupError> {
        private_directory(parent)?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| MigrationBackupError::Invalid)?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = parent.join(format!(".migration-work-{token}"));
        mkdir(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let parent_metadata = fs::symlink_metadata(parent)?;
        Ok(Self(path, metadata, parent_metadata))
    }
}
impl Work {
    fn cleanup(&self) -> Result<(), MigrationBackupError> {
        storage::ensure_directory_still_matches(
            self.0.parent().ok_or(MigrationBackupError::Invalid)?,
            &self.2,
        )?;
        storage::ensure_directory_still_matches(&self.0, &self.1)?;
        fs::remove_dir_all(&self.0)?;
        File::open(self.0.parent().ok_or(MigrationBackupError::Invalid)?)?.sync_all()?;
        Ok(())
    }
}
