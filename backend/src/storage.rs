//! Storage-integrity primitives shared by attachments and transfer manifests.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

/// Stable algorithm label to record beside content digests in future manifests.
pub const CONTENT_DIGEST_ALGORITHM: &str = "sha256";
pub const TRANSFER_MANIFEST_FILE_NAME: &str = "manifest.json";
pub const TRANSFER_DATABASE_PATH: &str = "book.sqlite3";
const SHA256_BYTE_LENGTH: usize = 32;
const SHA256_HEX_LENGTH: usize = SHA256_BYTE_LENGTH * 2;
const READ_BUFFER_LENGTH: usize = 64 * 1024;
pub const CURRENT_TRANSFER_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Exact SHA-256 digest of stored bytes.
pub struct ContentDigest([u8; SHA256_BYTE_LENGTH]);

impl ContentDigest {
    /// Constructs a digest already computed by a SHA-256 streaming implementation.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the lowercase hexadecimal representation used by manifests and
    /// content-addressed paths.
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(SHA256_HEX_LENGTH);
        for byte in self.0 {
            output.push(HEX[usize::from(byte >> 4)] as char);
            output.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        output
    }
}

impl Display for ContentDigest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for ContentDigest {
    type Err = ContentDigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != SHA256_HEX_LENGTH {
            return Err(ContentDigestParseError::WrongLength {
                actual: value.len(),
            });
        }

        let mut bytes = [0_u8; SHA256_BYTE_LENGTH];
        for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = decode_hex_digit(pair[0])
                .ok_or(ContentDigestParseError::InvalidHex { index: index * 2 })?;
            let low = decode_hex_digit(pair[1]).ok_or(ContentDigestParseError::InvalidHex {
                index: index * 2 + 1,
            })?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
/// Invalid encoded content digest.
pub enum ContentDigestParseError {
    #[error("SHA-256 digest must contain 64 hexadecimal characters, got {actual}")]
    WrongLength { actual: usize },
    #[error("SHA-256 digest contains a non-hexadecimal character at byte {index}")]
    InvalidHex { index: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Integrity metadata for one attachment or other stored object.
pub struct StoredObjectDescriptor {
    digest: ContentDigest,
    byte_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One attachment object declared by a transfer manifest.
pub struct ManifestAttachment {
    relative_path: String,
    descriptor: StoredObjectDescriptor,
}

impl ManifestAttachment {
    pub fn new(relative_path: impl Into<String>, descriptor: StoredObjectDescriptor) -> Self {
        Self {
            relative_path: relative_path.into(),
            descriptor,
        }
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn descriptor(&self) -> StoredObjectDescriptor {
        self.descriptor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Encoding-neutral logical contents of a `.kagextra` transfer manifest.
pub struct TransferManifest {
    format_version: u32,
    database: StoredObjectDescriptor,
    attachments: Vec<ManifestAttachment>,
}

impl TransferManifest {
    pub fn new(
        database: StoredObjectDescriptor,
        attachments: Vec<ManifestAttachment>,
    ) -> Result<Self, TransferManifestError> {
        Self::from_version(CURRENT_TRANSFER_MANIFEST_VERSION, database, attachments)
    }

    /// Validates decoded manifest fields before any referenced file is opened.
    pub fn from_version(
        format_version: u32,
        database: StoredObjectDescriptor,
        mut attachments: Vec<ManifestAttachment>,
    ) -> Result<Self, TransferManifestError> {
        if format_version != CURRENT_TRANSFER_MANIFEST_VERSION {
            return Err(TransferManifestError::UnsupportedVersion {
                actual: format_version,
            });
        }

        let mut paths = HashSet::with_capacity(attachments.len());
        for attachment in &attachments {
            validate_attachment_manifest_path(&attachment.relative_path)?;
            if !paths.insert(attachment.relative_path.clone()) {
                return Err(TransferManifestError::DuplicatePath {
                    path: attachment.relative_path.clone(),
                });
            }
        }
        attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        Ok(Self {
            format_version,
            database,
            attachments,
        })
    }

    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn database(&self) -> StoredObjectDescriptor {
        self.database
    }

    pub fn attachments(&self) -> &[ManifestAttachment] {
        &self.attachments
    }

    /// Encodes the validated manifest as deterministic, human-readable UTF-8
    /// JSON. Exact byte lengths remain decimal strings so JavaScript readers
    /// cannot lose precision above 2^53.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, TransferManifestJsonError> {
        let document = TransferManifestJson {
            format_version: self.format_version,
            database: ManifestObjectJson::from_descriptor(TRANSFER_DATABASE_PATH, self.database),
            attachments: self
                .attachments
                .iter()
                .map(|attachment| {
                    ManifestObjectJson::from_descriptor(
                        attachment.relative_path(),
                        attachment.descriptor(),
                    )
                })
                .collect(),
        };
        let mut bytes = serde_json::to_vec_pretty(&document)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Decodes strict UTF-8 JSON and re-applies all logical-manifest checks.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, TransferManifestJsonError> {
        let document: TransferManifestJson = serde_json::from_slice(bytes)?;
        if document.database.path != TRANSFER_DATABASE_PATH {
            return Err(TransferManifestJsonError::UnexpectedDatabasePath {
                actual: document.database.path,
            });
        }

        let database = document.database.into_descriptor()?;
        let attachments = document
            .attachments
            .into_iter()
            .map(|object| {
                let path = object.path.clone();
                object
                    .into_descriptor()
                    .map(|descriptor| ManifestAttachment::new(path, descriptor))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::from_version(document.format_version, database, attachments).map_err(Into::into)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TransferManifestJson {
    format_version: u32,
    database: ManifestObjectJson,
    attachments: Vec<ManifestObjectJson>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestObjectJson {
    path: String,
    sha256: String,
    byte_length: String,
}

impl ManifestObjectJson {
    fn from_descriptor(path: impl Into<String>, descriptor: StoredObjectDescriptor) -> Self {
        Self {
            path: path.into(),
            sha256: descriptor.digest().to_string(),
            byte_length: descriptor.byte_length().to_string(),
        }
    }

    fn into_descriptor(self) -> Result<StoredObjectDescriptor, TransferManifestJsonError> {
        let digest = ContentDigest::from_str(&self.sha256).map_err(|_| {
            TransferManifestJsonError::NonCanonicalDigest {
                path: self.path.clone(),
                value: self.sha256.clone(),
            }
        })?;
        if digest.to_string() != self.sha256 {
            return Err(TransferManifestJsonError::NonCanonicalDigest {
                path: self.path,
                value: self.sha256,
            });
        }

        let canonical_length = !self.byte_length.is_empty()
            && self.byte_length.bytes().all(|byte| byte.is_ascii_digit())
            && (self.byte_length == "0" || !self.byte_length.starts_with('0'));
        let byte_length = self.byte_length.parse::<u64>().map_err(|_| {
            TransferManifestJsonError::InvalidByteLength {
                path: self.path.clone(),
                value: self.byte_length.clone(),
            }
        })?;
        if !canonical_length {
            return Err(TransferManifestJsonError::InvalidByteLength {
                path: self.path,
                value: self.byte_length,
            });
        }

        Ok(StoredObjectDescriptor::new(digest, byte_length))
    }
}

#[derive(Debug, Error)]
/// A `manifest.json` document could not be encoded or decoded safely.
pub enum TransferManifestJsonError {
    #[error("invalid transfer manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("transfer manifest database path must be {TRANSFER_DATABASE_PATH}, got {actual}")]
    UnexpectedDatabasePath { actual: String },
    #[error("non-canonical SHA-256 digest for {path}: {value}")]
    NonCanonicalDigest { path: String, value: String },
    #[error("invalid decimal byte length for {path}: {value}")]
    InvalidByteLength { path: String, value: String },
    #[error("invalid logical transfer manifest: {0}")]
    Manifest(#[from] TransferManifestError),
}

#[derive(Debug, Error, Eq, PartialEq)]
/// Invalid logical transfer-manifest contents.
pub enum TransferManifestError {
    #[error(
        "unsupported transfer manifest version {actual}; supported version is {CURRENT_TRANSFER_MANIFEST_VERSION}"
    )]
    UnsupportedVersion { actual: u32 },
    #[error("unsafe attachment manifest path: {path}")]
    UnsafePath { path: String },
    #[error("duplicate attachment manifest path: {path}")]
    DuplicatePath { path: String },
}

fn validate_attachment_manifest_path(path: &str) -> Result<(), TransferManifestError> {
    let invalid_text = path.is_empty() || path.contains('\0') || path.contains('\\');
    let segments = path.split('/').collect::<Vec<_>>();
    let valid_segments = segments.len() >= 2
        && segments[0] == "attachments"
        && segments
            .iter()
            .all(|segment| !segment.is_empty() && *segment != "." && *segment != "..");

    if invalid_text || !valid_segments {
        return Err(TransferManifestError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

impl StoredObjectDescriptor {
    /// Reconstructs a descriptor read from a validated manifest.
    pub fn new(digest: ContentDigest, byte_length: u64) -> Self {
        Self {
            digest,
            byte_length,
        }
    }

    pub fn digest(self) -> ContentDigest {
        self.digest
    }

    pub fn byte_length(self) -> u64 {
        self.byte_length
    }
}

/// Hashes an existing file without modifying it.
pub fn describe_file(path: impl AsRef<Path>) -> io::Result<StoredObjectDescriptor> {
    let path = path.as_ref();
    let mut file = open_regular_file_without_symlink(path)?;
    let descriptor = describe_reader(&mut file)?;
    ensure_path_still_names_open_file(path, &file)?;
    Ok(descriptor)
}

/// Opens a path only when it is a regular file before and after opening.
/// On Unix (including the supported macOS target), device and inode checks
/// also reject a name that was substituted during the check/open sequence.
pub(crate) fn open_regular_file_without_symlink(path: &Path) -> io::Result<std::fs::File> {
    let before = std::fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stored object must be a regular non-symlink file",
        ));
    }

    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    let after = std::fs::symlink_metadata(path)?;
    if after.file_type().is_symlink() || !after.is_file() || !opened.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stored object path changed while it was being opened",
        ));
    }

    #[cfg(unix)]
    {
        if !same_file_identity(&before, &opened) || !same_file_identity(&after, &opened) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stored object path changed while it was being opened",
            ));
        }
    }

    Ok(file)
}

/// Re-checks the pathname after the complete read. Holding the file open keeps
/// the bytes readable if the name is replaced, but callers verifying a stored
/// object need the stronger guarantee that the verified name still denotes
/// that opened file when verification returns.
pub(crate) fn ensure_path_still_names_open_file(
    path: &Path,
    file: &std::fs::File,
) -> io::Result<()> {
    let opened = file.metadata()?;
    let current = std::fs::symlink_metadata(path)?;
    if current.file_type().is_symlink() || !current.is_file() || !opened.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stored object path changed while it was being read",
        ));
    }

    #[cfg(unix)]
    if !same_file_identity(&current, &opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stored object path changed while it was being read",
        ));
    }

    Ok(())
}

#[cfg(unix)]
fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

pub(crate) fn ensure_directory_still_matches(
    path: &Path,
    expected: &std::fs::Metadata,
) -> io::Result<()> {
    let current = std::fs::symlink_metadata(path)?;
    if current.file_type().is_symlink() || !current.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage destination parent changed during the operation",
        ));
    }

    #[cfg(unix)]
    if !same_file_identity(&current, expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage destination parent changed during the operation",
        ));
    }

    Ok(())
}

#[derive(Debug, Error)]
/// A stored object could not be read or did not match its expected descriptor.
pub enum StoredObjectVerificationError {
    #[error("failed to read stored object {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("stored object {path} has byte length {actual}, expected {expected}")]
    ByteLengthMismatch {
        path: std::path::PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("stored object {path} has digest {actual}, expected {expected}")]
    DigestMismatch {
        path: std::path::PathBuf,
        expected: ContentDigest,
        actual: ContentDigest,
    },
}

#[derive(Debug, Error)]
/// Failure before a verified local object can be safely published.
pub enum LocalObjectPublishError {
    #[error("local object destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("local object destination parent is missing or unsafe: {0}")]
    MissingParent(PathBuf),
    #[error("local object source is not a regular non-symlink file: {0}")]
    UnsafeSource(PathBuf),
    #[error("local object I/O failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("copied local object failed verification: {0}")]
    Verification(#[from] StoredObjectVerificationError),
}

#[derive(Debug, Error)]
#[error("local object cleanup warning while {operation} {path}: {source}")]
pub struct LocalObjectCleanupWarning {
    pub operation: &'static str,
    pub path: PathBuf,
    #[source]
    pub source: io::Error,
}

#[derive(Debug)]
/// Successful publication details for a verified local object.
pub struct LocalObjectPublishOutcome {
    cleanup_warning: Option<LocalObjectCleanupWarning>,
}

impl LocalObjectPublishOutcome {
    pub fn cleanup_warning(&self) -> Option<&LocalObjectCleanupWarning> {
        self.cleanup_warning.as_ref()
    }

    pub fn into_cleanup_warning(self) -> Option<LocalObjectCleanupWarning> {
        self.cleanup_warning
    }
}

#[derive(Debug, Error)]
/// A complete private directory could not be published without replacement.
pub enum LocalDirectoryPublishError {
    #[error("local directory destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("local directory destination parent is missing or unsafe: {0}")]
    MissingParent(PathBuf),
    #[error("local directory source is not a real directory: {0}")]
    UnsafeSource(PathBuf),
    #[error("local directory publication failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Atomically gives a complete private directory its public destination name
/// without replacing any file, directory, or symlink already there.
///
/// The caller must create the private directory beside the destination, fully
/// verify its contents, and finish required synchronization before calling.
/// Success moves the source directory; failure leaves it available for
/// inspection or cleanup.
pub fn publish_private_directory_noreplace(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), LocalDirectoryPublishError> {
    let source = source.as_ref();
    let destination = destination.as_ref();

    let source_metadata = std::fs::symlink_metadata(source).map_err(|source_error| {
        LocalDirectoryPublishError::Io {
            operation: "checking source",
            path: source.to_path_buf(),
            source: source_error,
        }
    })?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(LocalDirectoryPublishError::UnsafeSource(
            source.to_path_buf(),
        ));
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| LocalDirectoryPublishError::MissingParent(parent.to_path_buf()))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(LocalDirectoryPublishError::MissingParent(
            parent.to_path_buf(),
        ));
    }

    match std::fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(LocalDirectoryPublishError::DestinationExists(
                destination.to_path_buf(),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(LocalDirectoryPublishError::Io {
                operation: "checking destination",
                path: destination.to_path_buf(),
                source,
            });
        }
    }

    let current_source = std::fs::symlink_metadata(source).map_err(|source_error| {
        LocalDirectoryPublishError::Io {
            operation: "rechecking source",
            path: source.to_path_buf(),
            source: source_error,
        }
    })?;
    if !current_source.is_dir() || current_source.file_type().is_symlink() || {
        #[cfg(unix)]
        {
            !same_file_identity(&current_source, &source_metadata)
        }
        #[cfg(not(unix))]
        {
            false
        }
    } {
        return Err(LocalDirectoryPublishError::UnsafeSource(
            source.to_path_buf(),
        ));
    }

    ensure_directory_still_matches(parent, &parent_metadata)
        .map_err(|_| LocalDirectoryPublishError::MissingParent(parent.to_path_buf()))?;

    renamore::rename_exclusive(source, destination).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            LocalDirectoryPublishError::DestinationExists(destination.to_path_buf())
        } else {
            LocalDirectoryPublishError::Io {
                operation: "atomically publishing",
                path: destination.to_path_buf(),
                source,
            }
        }
    })
}

/// Recomputes and verifies an existing object's exact length and content hash.
pub fn verify_file(
    path: impl AsRef<Path>,
    expected: StoredObjectDescriptor,
) -> Result<(), StoredObjectVerificationError> {
    let path = path.as_ref();
    let actual = describe_file(path).map_err(|source| StoredObjectVerificationError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    if actual.byte_length != expected.byte_length {
        return Err(StoredObjectVerificationError::ByteLengthMismatch {
            path: path.to_path_buf(),
            expected: expected.byte_length,
            actual: actual.byte_length,
        });
    }
    if actual.digest != expected.digest {
        return Err(StoredObjectVerificationError::DigestMismatch {
            path: path.to_path_buf(),
            expected: expected.digest,
            actual: actual.digest,
        });
    }
    Ok(())
}

/// Copies and publishes one verified object to local storage without replacing
/// any existing file or symlink.
///
/// The temporary work file lives beside the destination, so the final exclusive
/// rename is atomic without leaving a second mutable hard-link name behind.
/// This is not the removable-drive `.kagextra` publication mechanism.
pub fn publish_verified_local_copy(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    expected: StoredObjectDescriptor,
) -> Result<LocalObjectPublishOutcome, LocalObjectPublishError> {
    let source = source.as_ref();
    let destination = destination.as_ref();

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| LocalObjectPublishError::MissingParent(parent.to_path_buf()))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(LocalObjectPublishError::MissingParent(parent.to_path_buf()));
    }
    ensure_destination_absent(destination)?;

    let source_metadata =
        std::fs::symlink_metadata(source).map_err(|source_error| LocalObjectPublishError::Io {
            operation: "checking source",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err(LocalObjectPublishError::UnsafeSource(source.to_path_buf()));
    }

    let work_directory = create_private_object_directory(parent)?;
    let work_file = work_directory.join("object");
    let result = ensure_directory_still_matches(parent, &parent_metadata)
        .map_err(|source| LocalObjectPublishError::Io {
            operation: "rechecking destination parent",
            path: parent.to_path_buf(),
            source,
        })
        .and_then(|()| copy_verify_and_publish(source, &work_file, destination, expected));
    let cleanup = cleanup_object_work(&work_file, &work_directory);

    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Ok(())) => Ok(LocalObjectPublishOutcome {
            cleanup_warning: None,
        }),
        (Ok(()), Err(warning)) => Ok(LocalObjectPublishOutcome {
            cleanup_warning: Some(warning),
        }),
    }
}

fn ensure_destination_absent(destination: &Path) -> Result<(), LocalObjectPublishError> {
    match std::fs::symlink_metadata(destination) {
        Ok(_) => Err(LocalObjectPublishError::DestinationExists(
            destination.to_path_buf(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LocalObjectPublishError::Io {
            operation: "checking destination",
            path: destination.to_path_buf(),
            source,
        }),
    }
}

fn create_private_object_directory(parent: &Path) -> Result<PathBuf, LocalObjectPublishError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| LocalObjectPublishError::Io {
            operation: "reading system clock for",
            path: parent.to_path_buf(),
            source: io::Error::other(source),
        })?
        .as_nanos();

    for attempt in 0..100_u32 {
        let path = parent.join(format!(
            ".kAccounting-Extra-01-object-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(LocalObjectPublishError::Io {
                    operation: "creating private object directory",
                    path,
                    source,
                });
            }
        }
    }

    Err(LocalObjectPublishError::Io {
        operation: "creating unique private object directory in",
        path: parent.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "100 candidate names already existed",
        ),
    })
}

fn copy_verify_and_publish(
    source: &Path,
    work_file: &Path,
    destination: &Path,
    expected: StoredObjectDescriptor,
) -> Result<(), LocalObjectPublishError> {
    let mut source_file = open_regular_file_without_symlink(source).map_err(|source_error| {
        if source_error.kind() == io::ErrorKind::InvalidInput {
            LocalObjectPublishError::UnsafeSource(source.to_path_buf())
        } else {
            LocalObjectPublishError::Io {
                operation: "opening source",
                path: source.to_path_buf(),
                source: source_error,
            }
        }
    })?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options
        .open(work_file)
        .map_err(|source| LocalObjectPublishError::Io {
            operation: "creating private object",
            path: work_file.to_path_buf(),
            source,
        })?;
    copy_and_describe(&mut source_file, &mut output).map_err(|source| {
        LocalObjectPublishError::Io {
            operation: "copying private object",
            path: work_file.to_path_buf(),
            source,
        }
    })?;
    output
        .sync_all()
        .map_err(|source| LocalObjectPublishError::Io {
            operation: "synchronizing private object",
            path: work_file.to_path_buf(),
            source,
        })?;
    drop(output);

    verify_file(work_file, expected)?;
    renamore::rename_exclusive(work_file, destination).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            LocalObjectPublishError::DestinationExists(destination.to_path_buf())
        } else {
            LocalObjectPublishError::Io {
                operation: "publishing verified local object",
                path: destination.to_path_buf(),
                source,
            }
        }
    })
}

fn cleanup_object_work(file: &Path, directory: &Path) -> Result<(), LocalObjectCleanupWarning> {
    match std::fs::symlink_metadata(file) {
        Ok(_) => {
            std::fs::remove_file(file).map_err(|source| LocalObjectCleanupWarning {
                operation: "removing private object file",
                path: file.to_path_buf(),
                source,
            })?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(LocalObjectCleanupWarning {
                operation: "checking private object file during cleanup",
                path: file.to_path_buf(),
                source,
            });
        }
    }
    std::fs::remove_dir(directory).map_err(|source| LocalObjectCleanupWarning {
        operation: "removing private object directory",
        path: directory.to_path_buf(),
        source,
    })
}

/// Streams bytes into SHA-256 while counting their exact length.
///
/// The fixed buffer prevents large attachments from being loaded wholly into
/// memory. A length overflow fails closed instead of wrapping.
pub fn describe_reader(reader: &mut impl Read) -> io::Result<StoredObjectDescriptor> {
    let mut sink = io::sink();
    copy_and_describe(reader, &mut sink)
}

/// Copies bytes through bounded buffers while computing the descriptor of the
/// exact bytes successfully written.
///
/// On any read or write error the caller must keep the partial destination in
/// private work state and must not publish it as a valid object.
pub fn copy_and_describe(
    reader: &mut impl Read,
    writer: &mut impl Write,
) -> io::Result<StoredObjectDescriptor> {
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = [0_u8; READ_BUFFER_LENGTH];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        byte_length = byte_length.checked_add(read as u64).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "stored object length overflow")
        })?;
        hasher.update(&buffer[..read]);
    }
    writer.flush()?;

    let mut digest = [0_u8; SHA256_BYTE_LENGTH];
    digest.copy_from_slice(&hasher.finalize());
    Ok(StoredObjectDescriptor {
        digest: ContentDigest(digest),
        byte_length,
    })
}

fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
