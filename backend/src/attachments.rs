//! Encrypted, immutable attachment objects owned by journal entries.
//!
//! Files become durable before their SQLite reference commits. A crash can
//! therefore leave encrypted unreferenced bytes, but never a committed reference
//! to an unpublished candidate. Unreferenced bytes are retained conservatively
//! until offline maintenance obtains an exclusive cross-process store lease.

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

use crate::{
    accounting::journal::JournalEntryId,
    attachment_maintenance::StorePin,
    crypto::{
        EncryptedObjectError, EncryptedObjectPurpose, RootEncryptionKey, decrypt_object,
        encrypt_object,
    },
    storage::{
        ContentDigest, StoredObjectDescriptor, StoredObjectVerificationError, describe_file,
        ensure_directory_still_matches, ensure_path_still_names_open_file,
        open_regular_file_without_symlink, verify_file,
    },
};

pub const MAX_ATTACHMENT_BYTES: u64 = 250 * 1024 * 1024;
pub const MAX_BOOK_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Random identifier, independent of the content and original filename.
pub struct AttachmentId(String);
impl AttachmentId {
    pub fn parse(value: &str) -> Result<Self, AttachmentError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AttachmentError::InvalidId);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn generate() -> Result<Self, AttachmentError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| AttachmentError::Random)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub id: AttachmentId,
    pub entry_id: JournalEntryId,
    pub display_name: String,
    pub plaintext: StoredObjectDescriptor,
    pub ciphertext: StoredObjectDescriptor,
    pub created_at: String,
}

#[derive(Debug, Error)]
pub enum AttachmentError {
    #[error("attachment identifier must be 32 lowercase hexadecimal characters")]
    InvalidId,
    #[error("attachment display name must be a plain filename of 1 to 255 non-control characters")]
    InvalidName,
    #[error("attachment source must be a regular non-executable file, not a symlink or package")]
    UnsafeSource,
    #[error("attachment exceeds the 250 MiB limit")]
    FileTooLarge,
    #[error("book attachments exceed the 20 GiB limit")]
    BookQuotaExceeded,
    #[error("journal entry was not found")]
    EntryNotFound,
    #[error("attachment was not found")]
    NotFound,
    #[error("posted attachment cannot be removed or replaced")]
    PostedImmutable,
    #[error("journal draft changed; reload it before removing an attachment")]
    StaleDraft,
    #[error("journal draft revision is exhausted")]
    RevisionExhausted,
    #[error("attachment source changed while reading")]
    SourceChanged,
    #[error(
        "attachment store must be a private non-symlink directory outside the source repository"
    )]
    UnsafeStore,
    #[error("attachment integrity verification failed")]
    Integrity,
    #[error("operating-system random generation failed")]
    Random,
    #[error("attachment worker did not complete")]
    Worker,
    #[error("attachment I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Crypto(#[from] EncryptedObjectError),
    #[error(transparent)]
    Verification(#[from] StoredObjectVerificationError),
    #[error("attachment database operation failed: {0}")]
    Database(#[source] sqlx::Error),
}

/// One application-configured store. Clones retain the same key and root identity.
#[derive(Clone)]
pub struct AttachmentRepository {
    pool: SqlitePool,
    store: AttachmentStore,
}

#[derive(Clone)]
struct AttachmentStore {
    root: PathBuf,
    identity: std::fs::Metadata,
    key: Arc<RootEncryptionKey>,
    pin: StorePin,
}

impl AttachmentRepository {
    /// Opens or creates only the attachment directory; it never creates key files.
    /// Production callers must obtain the supplied key from the approved Keychain boundary.
    pub fn open(
        pool: SqlitePool,
        root: impl AsRef<Path>,
        key: Arc<RootEncryptionKey>,
    ) -> Result<Self, AttachmentError> {
        let root = root.as_ref();
        let parent = std::fs::canonicalize(root.parent().ok_or(AttachmentError::UnsafeStore)?)?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or(AttachmentError::UnsafeStore)?;
        let candidate = parent.join(root.file_name().ok_or(AttachmentError::UnsafeStore)?);
        if candidate.starts_with(source) {
            return Err(AttachmentError::UnsafeStore);
        }
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                builder.mode(0o700);
                builder.create(&candidate)?;
                File::open(&parent)?.sync_all()?;
            }
            Err(e) => return Err(e.into()),
        }
        let identity = std::fs::symlink_metadata(&candidate)?;
        if !identity.is_dir() || identity.file_type().is_symlink() {
            return Err(AttachmentError::UnsafeStore);
        }
        #[cfg(unix)]
        if identity.permissions().mode() & 0o077 != 0 {
            return Err(AttachmentError::UnsafeStore);
        }
        Ok(Self {
            pool,
            store: AttachmentStore {
                pin: StorePin::acquire(&candidate).map_err(|error| match error {
                    crate::attachment_maintenance::MaintenanceError::Io(error) => {
                        AttachmentError::Io(error)
                    }
                    _ => AttachmentError::UnsafeStore,
                })?,
                root: candidate,
                identity,
                key,
            },
        })
    }

    /// Imports a regular local file through bounded-memory encryption and full verification.
    /// No plaintext intermediate file is written, and existing objects are never replaced.
    pub async fn import_file(
        &self,
        entry_id: JournalEntryId,
        source: impl AsRef<Path>,
        display_name: &str,
    ) -> Result<Attachment, AttachmentError> {
        validate_display_name(display_name)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM journal_entries WHERE id = ?)")
                .bind(entry_id.get())
                .fetch_one(&self.pool)
                .await
                .map_err(AttachmentError::Database)?;
        if !exists {
            return Err(AttachmentError::EntryNotFound);
        }
        let source = source.as_ref().to_path_buf();
        let name = display_name.to_owned();
        let store = self.store.clone();
        let prepared = tokio::task::spawn_blocking(move || store.prepare(&source, name))
            .await
            .map_err(|_| AttachmentError::Worker)??;
        // The writer lock coordinates publication/reference creation with any
        // offline reference-based maintenance. Encryption runs before this lock.
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AttachmentError::Database)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM journal_entries WHERE id = ?)")
                .bind(entry_id.get())
                .fetch_one(&mut *tx)
                .await
                .map_err(AttachmentError::Database)?;
        if !exists {
            return Err(AttachmentError::EntryNotFound);
        }
        let created_at: String = sqlx::query_scalar("INSERT INTO attachments (id, entry_id, display_name, plaintext_sha256, byte_length, ciphertext_sha256, ciphertext_byte_length) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING created_at")
            .bind(prepared.id.as_str()).bind(entry_id.get()).bind(&prepared.name)
            .bind(prepared.plaintext.digest().to_hex()).bind(prepared.plaintext.byte_length() as i64)
            .bind(prepared.ciphertext.digest().to_hex()).bind(prepared.ciphertext.byte_length() as i64)
            .fetch_one(&mut *tx).await.map_err(map_database_error)?;
        self.store.validate()?;
        prepared.work.validate()?;
        let destination = self.store.object_path(&prepared.id);
        renamore::rename_exclusive(prepared.work.file(), &destination)?;
        File::open(&self.store.root)?.sync_all()?;
        // On a commit error the ciphertext remains. Deleting it here would be
        // unsafe because an I/O error can make the commit outcome uncertain.
        tx.commit().await.map_err(AttachmentError::Database)?;
        Ok(Attachment {
            id: prepared.id.clone(),
            entry_id,
            display_name: prepared.name.clone(),
            plaintext: prepared.plaintext,
            ciphertext: prepared.ciphertext,
            created_at,
        })
    }

    pub async fn get(&self, id: &AttachmentId) -> Result<Attachment, AttachmentError> {
        let row: Option<AttachmentRow> = sqlx::query_as("SELECT attachment.*, owner.status AS owner_status, owner.revision AS owner_revision, owner.posted_at AS owner_posted_at FROM attachments AS attachment LEFT JOIN journal_entries AS owner ON owner.id = attachment.entry_id WHERE attachment.id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(AttachmentError::Database)?;
        row.ok_or(AttachmentError::NotFound)?.try_into()
    }

    /// Lists immutable metadata in attachment creation order from one read snapshot.
    pub async fn list_for_entry(
        &self,
        entry_id: JournalEntryId,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        let mut tx = self.pool.begin().await.map_err(AttachmentError::Database)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM journal_entries WHERE id = ?)")
                .bind(entry_id.get())
                .fetch_one(&mut *tx)
                .await
                .map_err(AttachmentError::Database)?;
        if !exists {
            return Err(AttachmentError::EntryNotFound);
        }
        let rows: Vec<AttachmentRow> = sqlx::query_as("SELECT attachment.*, owner.status AS owner_status, owner.revision AS owner_revision, owner.posted_at AS owner_posted_at FROM attachments AS attachment LEFT JOIN journal_entries AS owner ON owner.id = attachment.entry_id WHERE attachment.entry_id = ? ORDER BY attachment.created_at, attachment.rowid")
        .bind(entry_id.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(AttachmentError::Database)?;
        let result = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(AttachmentError::Database)?;
        Ok(result)
    }

    /// Removes a draft's reference atomically. Encrypted bytes remain available
    /// for snapshots already in flight; safe physical reclamation is separate.
    pub async fn remove_from_draft(
        &self,
        id: &AttachmentId,
        expected_revision: i64,
    ) -> Result<(), AttachmentError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AttachmentError::Database)?;
        let owner: Option<(i64, String, i64)> = sqlx::query_as("SELECT entry.id, entry.status, entry.revision FROM attachments AS attachment JOIN journal_entries AS entry ON entry.id = attachment.entry_id WHERE attachment.id = ?")
            .bind(id.as_str()).fetch_optional(&mut *tx).await.map_err(AttachmentError::Database)?;
        let (entry_id, status, revision) = owner.ok_or(AttachmentError::NotFound)?;
        if status == "posted" {
            return Err(AttachmentError::PostedImmutable);
        }
        if status != "draft" {
            return Err(AttachmentError::Integrity);
        }
        if revision != expected_revision {
            return Err(AttachmentError::StaleDraft);
        }
        if revision == i64::MAX {
            return Err(AttachmentError::RevisionExhausted);
        }
        sqlx::query(
            "UPDATE journal_entries SET revision = revision + 1 WHERE id = ? AND revision = ?",
        )
        .bind(entry_id)
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(AttachmentError::Database)?;
        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        tx.commit().await.map_err(AttachmentError::Database)?;
        Ok(())
    }

    /// Fully authenticates and hashes both encrypted bytes and decrypted bytes.
    /// No plaintext is written to disk or returned on verification failure.
    pub async fn verify(&self, id: &AttachmentId) -> Result<(), AttachmentError> {
        let attachment = self.get(id).await?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.verify_attachment(&attachment))
            .await
            .map_err(|_| AttachmentError::Worker)?
    }

    /// Lists all object descriptors owned by the selected database snapshot. The supplied
    /// pool may be a read-only snapshot, so deletion from the live book cannot
    /// silently remove the export's selected attachment set.
    pub async fn list_all(&self) -> Result<Vec<Attachment>, AttachmentError> {
        catalog_for_pool(&self.pool).await
    }
}

/// Selects references from an already verified immutable database snapshot.
pub(crate) async fn catalog_for_pool(
    pool: &SqlitePool,
) -> Result<Vec<Attachment>, AttachmentError> {
    let rows: Vec<AttachmentRow> = sqlx::query_as("SELECT attachment.*, owner.status AS owner_status, owner.revision AS owner_revision, owner.posted_at AS owner_posted_at FROM attachments AS attachment LEFT JOIN journal_entries AS owner ON owner.id = attachment.entry_id ORDER BY attachment.rowid")
        .fetch_all(pool)
        .await
        .map_err(AttachmentError::Database)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

impl AttachmentStore {
    fn validate(&self) -> Result<(), AttachmentError> {
        self.pin
            .validate()
            .map_err(|_| AttachmentError::UnsafeStore)?;
        ensure_directory_still_matches(&self.root, &self.identity)
            .map_err(|_| AttachmentError::UnsafeStore)?;
        #[cfg(unix)]
        if std::fs::symlink_metadata(&self.root)?.permissions().mode() & 0o077 != 0 {
            return Err(AttachmentError::UnsafeStore);
        }
        Ok(())
    }
    fn object_path(&self, id: &AttachmentId) -> PathBuf {
        self.root.join(format!("{}.kae", id.as_str()))
    }
    fn prepare(&self, source: &Path, name: String) -> Result<PreparedAttachment, AttachmentError> {
        self.validate()?;
        let mut input = open_regular_file_without_symlink(source).map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidInput {
                AttachmentError::UnsafeSource
            } else {
                e.into()
            }
        })?;
        let before = input.metadata()?;
        if before.len() > MAX_ATTACHMENT_BYTES {
            return Err(AttachmentError::FileTooLarge);
        }
        #[cfg(unix)]
        if before.permissions().mode() & 0o111 != 0 {
            return Err(AttachmentError::UnsafeSource);
        }
        let mut prefix = [0_u8; 8];
        let n = input.read(&mut prefix)?;
        if is_executable(&name, &prefix[..n])
            || source
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| is_executable(name, &[]))
            || source
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(is_package_extension)
        {
            return Err(AttachmentError::UnsafeSource);
        }
        input.rewind()?;
        let id = AttachmentId::generate()?;
        let work = PrivateWork::create(&self.root, &id)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut output = options.open(work.file())?;
        let mut reader = HashingReader {
            inner: (&mut input).take(MAX_ATTACHMENT_BYTES + 1),
            hash: HashState::new(),
        };
        let outcome = encrypt_object(
            &mut reader,
            &mut output,
            &self.key,
            EncryptedObjectPurpose::Attachment,
        )?;
        let plaintext = reader.hash.finish();
        if outcome.plaintext_byte_length > MAX_ATTACHMENT_BYTES {
            return Err(AttachmentError::FileTooLarge);
        }
        if plaintext.byte_length() != before.len()
            || before.modified()? != input.metadata()?.modified()?
        {
            return Err(AttachmentError::SourceChanged);
        }
        ensure_path_still_names_open_file(source, &input)
            .map_err(|_| AttachmentError::SourceChanged)?;
        output.sync_all()?;
        drop(output);
        let ciphertext = describe_file(work.file())?;
        verify_encrypted_file(&work.file(), plaintext, ciphertext, &self.key)?;
        File::open(&work.directory)?.sync_all()?;
        self.validate()?;
        Ok(PreparedAttachment {
            id,
            name,
            plaintext,
            ciphertext,
            work,
        })
    }
    fn verify_attachment(&self, attachment: &Attachment) -> Result<(), AttachmentError> {
        self.validate()?;
        verify_encrypted_file(
            &self.object_path(&attachment.id),
            attachment.plaintext,
            attachment.ciphertext,
            &self.key,
        )?;
        self.validate()
    }
}

pub(crate) fn verify_encrypted_file(
    path: &Path,
    plaintext: StoredObjectDescriptor,
    ciphertext: StoredObjectDescriptor,
    key: &RootEncryptionKey,
) -> Result<(), AttachmentError> {
    verify_file(path, ciphertext)?;
    let mut input = open_regular_file_without_symlink(path)?;
    let mut sink = HashState::new();
    let outcome = decrypt_object(
        &mut input,
        &mut sink,
        key,
        EncryptedObjectPurpose::Attachment,
    )?;
    if sink.finish() != plaintext || outcome.encrypted_byte_length != ciphertext.byte_length() {
        return Err(AttachmentError::Integrity);
    }
    ensure_path_still_names_open_file(path, &input)?;
    Ok(())
}

struct PreparedAttachment {
    id: AttachmentId,
    name: String,
    plaintext: StoredObjectDescriptor,
    ciphertext: StoredObjectDescriptor,
    work: PrivateWork,
}
struct PrivateWork {
    directory: PathBuf,
    identity: std::fs::Metadata,
}
impl PrivateWork {
    fn create(root: &Path, id: &AttachmentId) -> Result<Self, AttachmentError> {
        let directory = root.join(format!(".kAccounting-Extra-01-object-{}", id.as_str()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&directory)?;
        let identity = std::fs::symlink_metadata(&directory)?;
        Ok(Self {
            directory,
            identity,
        })
    }
    fn file(&self) -> PathBuf {
        self.directory.join("object")
    }
    fn validate(&self) -> Result<(), AttachmentError> {
        ensure_directory_still_matches(&self.directory, &self.identity)
            .map_err(|_| AttachmentError::UnsafeStore)
    }
}
impl Drop for PrivateWork {
    fn drop(&mut self) {
        if self.validate().is_ok() {
            let _ = std::fs::remove_file(self.file());
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}

struct HashState {
    hasher: Sha256,
    length: u64,
}
impl HashState {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
            length: 0,
        }
    }
    fn finish(self) -> StoredObjectDescriptor {
        StoredObjectDescriptor::new(
            ContentDigest::from_bytes(self.hasher.finalize().into()),
            self.length,
        )
    }
}
impl Write for HashState {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.length = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("attachment length overflow"))?;
        self.hasher.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct HashingReader<R> {
    inner: R,
    hash: HashState,
}
impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(bytes)?;
        self.hash.write_all(&bytes[..n])?;
        Ok(n)
    }
}

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    id: String,
    entry_id: i64,
    display_name: String,
    plaintext_sha256: String,
    byte_length: i64,
    ciphertext_sha256: String,
    ciphertext_byte_length: i64,
    created_at: String,
    owner_status: Option<String>,
    owner_revision: Option<i64>,
    owner_posted_at: Option<String>,
}
impl TryFrom<AttachmentRow> for Attachment {
    type Error = AttachmentError;
    fn try_from(row: AttachmentRow) -> Result<Self, Self::Error> {
        let valid_owner = match (
            row.owner_status.as_deref(),
            row.owner_revision,
            row.owner_posted_at.as_deref(),
        ) {
            (Some("draft"), Some(revision), None) => revision >= 1,
            (Some("posted"), Some(revision), Some(_)) => revision >= 2,
            _ => false,
        };
        if !valid_owner {
            return Err(AttachmentError::Integrity);
        }
        validate_display_name(&row.display_name).map_err(|_| AttachmentError::Integrity)?;
        if row.entry_id <= 0
            || !(0..=MAX_ATTACHMENT_BYTES as i64).contains(&row.byte_length)
            || row.ciphertext_byte_length != encrypted_length(row.byte_length as u64) as i64
        {
            return Err(AttachmentError::Integrity);
        }
        let plaintext_digest: ContentDigest = row
            .plaintext_sha256
            .parse()
            .map_err(|_| AttachmentError::Integrity)?;
        let ciphertext_digest: ContentDigest = row
            .ciphertext_sha256
            .parse()
            .map_err(|_| AttachmentError::Integrity)?;
        if plaintext_digest.to_hex() != row.plaintext_sha256
            || ciphertext_digest.to_hex() != row.ciphertext_sha256
        {
            return Err(AttachmentError::Integrity);
        }
        Ok(Self {
            id: AttachmentId::parse(&row.id).map_err(|_| AttachmentError::Integrity)?,
            entry_id: JournalEntryId::new(row.entry_id),
            display_name: row.display_name,
            plaintext: StoredObjectDescriptor::new(plaintext_digest, row.byte_length as u64),
            ciphertext: StoredObjectDescriptor::new(
                ciphertext_digest,
                row.ciphertext_byte_length as u64,
            ),
            created_at: row.created_at,
        })
    }
}
fn encrypted_length(length: u64) -> u64 {
    length + 72 + 20 * length.div_ceil(65536).max(1)
}
fn validate_display_name(name: &str) -> Result<(), AttachmentError> {
    if name.is_empty()
        || name.chars().count() > 255
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(AttachmentError::InvalidName);
    }
    Ok(())
}
fn is_package_extension(extension: &str) -> bool {
    [
        "app",
        "pkg",
        "mpkg",
        "bundle",
        "framework",
        "kext",
        "plugin",
        "workflow",
        "action",
    ]
    .iter()
    .any(|ext| extension.eq_ignore_ascii_case(ext))
}
fn is_executable(name: &str, prefix: &[u8]) -> bool {
    let extension = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    is_package_extension(extension)
        || [
            "exe",
            "dll",
            "com",
            "bat",
            "cmd",
            "command",
            "sh",
            "bash",
            "zsh",
            "ps1",
            "scpt",
            "applescript",
        ]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
        || prefix.starts_with(b"#!")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(b"\x7fELF")
        || [
            b"\xfe\xed\xfa\xce",
            b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe",
            b"\xbe\xba\xfe\xca",
            b"\xca\xfe\xba\xbf",
            b"\xbf\xba\xfe\xca",
        ]
        .iter()
        .any(|magic| prefix.starts_with(*magic))
}
fn map_database_error(error: sqlx::Error) -> AttachmentError {
    if let sqlx::Error::Database(ref db) = error {
        if db.message().contains("attachment book quota exceeded") {
            return AttachmentError::BookQuotaExceeded;
        }
        if db.message().contains("posted attachment cannot be deleted") {
            return AttachmentError::PostedImmutable;
        }
    }
    AttachmentError::Database(error)
}
