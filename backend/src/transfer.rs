//! Verification of decoded transfer staging directories.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use sqlx::SqlitePool;

use crate::db::{
    BackupCleanupWarning, BackupError, DatabaseError, backup_database, verify_database_snapshot,
};
use crate::storage::{
    LocalDirectoryPublishError, LocalObjectCleanupWarning, LocalObjectPublishError,
    ManifestAttachment, StoredObjectDescriptor, StoredObjectVerificationError,
    TRANSFER_MANIFEST_FILE_NAME, TransferManifest, TransferManifestError,
    TransferManifestJsonError, describe_file, publish_private_directory_noreplace,
    publish_verified_local_copy, verify_file,
};
use crate::transfer_state::{
    TransferWorkStateCleanupWarning, TransferWorkStateError, create_export_work_state,
    load_transfer_work_state, transition_transfer_work_state,
};

pub use crate::storage::TRANSFER_DATABASE_PATH as STAGED_DATABASE_PATH;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Persistable lifecycle state for one export or import operation.
pub enum TransferOperationState {
    Working,
    PublishedUnverified,
    Verified,
    Failed,
}

impl TransferOperationState {
    /// Applies one allowed monotonic lifecycle transition.
    pub fn transition(self, next: Self) -> Result<Self, TransferStateTransitionError> {
        let allowed = matches!(
            (self, next),
            (Self::Working, Self::PublishedUnverified | Self::Failed)
                | (Self::PublishedUnverified, Self::Verified | Self::Failed)
        );
        if allowed {
            Ok(next)
        } else {
            Err(TransferStateTransitionError {
                current: self,
                next,
            })
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Verified | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid transfer state transition from {current:?} to {next:?}")]
pub struct TransferStateTransitionError {
    pub current: TransferOperationState,
    pub next: TransferOperationState,
}

#[derive(Debug, Error)]
/// A decoded transfer candidate is incomplete, unsafe, or corrupt.
pub enum TransferVerificationError {
    #[error("invalid transfer staging root {path}: {source}")]
    InvalidRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("transfer object is not a regular non-symlink file: {path}")]
    UnsafeObjectType { path: PathBuf },
    #[error("transfer object verification failed for {logical_path}: {source}")]
    Object {
        logical_path: String,
        #[source]
        source: StoredObjectVerificationError,
    },
    #[error("transfer database integrity verification failed: {0}")]
    Database(#[source] DatabaseError),
}

#[derive(Debug, Error)]
/// A physical `.kagextra` directory package is malformed, incomplete, or
/// contains content that its manifest does not declare.
pub enum TransferPackageVerificationError {
    #[error("invalid transfer package root {path}: {source}")]
    InvalidRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("transfer package object is not the required real file or directory: {path}")]
    UnsafeObjectType { path: PathBuf },
    #[error("failed to read transfer manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid transfer manifest: {0}")]
    Manifest(#[from] TransferManifestJsonError),
    #[error("failed to inspect transfer package directory {path}: {source}")]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("transfer package contains undeclared object: {path}")]
    UndeclaredObject { path: PathBuf },
    #[error("transfer package content verification failed: {0}")]
    Content(#[from] TransferVerificationError),
}

#[derive(Debug, Error)]
/// A verified staging tree could not be assembled as one private `.kagextra`
/// directory package.
pub enum TransferPackageAssemblyError {
    #[error("source staging verification failed: {0}")]
    Source(#[from] TransferVerificationError),
    #[error("private package work path already exists: {0}")]
    WorkPathExists(PathBuf),
    #[error("private package work parent is missing or unsafe: {0}")]
    MissingParent(PathBuf),
    #[error("failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to copy declared object {logical_path}: {source}")]
    CopyObject {
        logical_path: String,
        #[source]
        source: LocalObjectPublishError,
    },
    #[error("copied object {logical_path} retained private cleanup residue: {source}")]
    ObjectCleanup {
        logical_path: String,
        #[source]
        source: LocalObjectCleanupWarning,
    },
    #[error("failed to encode transfer manifest: {0}")]
    Manifest(#[from] TransferManifestJsonError),
    #[error("assembled private package failed complete verification: {0}")]
    Verification(#[from] TransferPackageVerificationError),
}

#[derive(Debug, Error)]
/// A complete private package could not be durably published and reverified.
pub enum TransferPackagePublishError {
    #[error("private package verification failed: {0}")]
    PrivateVerification(#[source] Box<TransferPackageVerificationError>),
    #[error("failed to synchronize transfer package path {path}: {source}")]
    Synchronization {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("atomic transfer package publication failed: {0}")]
    Publication(#[from] LocalDirectoryPublishError),
    #[error("package was published at {destination}, but durable published state failed: {source}")]
    PublishedState {
        destination: PathBuf,
        #[source]
        source: Box<TransferWorkStateError>,
    },
    #[error(
        "package was published at {destination}, but synchronizing its parent {parent} failed: {source}"
    )]
    PublishedParentSynchronization {
        destination: PathBuf,
        parent: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("package was published at {destination}, but final verification failed: {source}")]
    PublishedVerification {
        destination: PathBuf,
        #[source]
        source: Box<TransferPackageVerificationError>,
    },
    #[error("package verified at {destination}, but durable verified state failed: {source}")]
    VerifiedState {
        destination: PathBuf,
        #[source]
        source: Box<TransferWorkStateError>,
    },
}

#[derive(Debug, Error)]
/// A verified staging tree could not be exported through the complete private
/// assembly and durable publication workflow.
pub enum TransferExportError {
    #[error("transfer export destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("transfer export destination parent is missing or unsafe: {0}")]
    MissingParent(PathBuf),
    #[error("failed to inspect transfer export destination parent {path}: {source}")]
    InspectParent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to inspect transfer export destination {path}: {source}")]
    InspectDestination {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read the system clock for transfer export beside {path}: {source}")]
    Clock {
        path: PathBuf,
        #[source]
        source: std::time::SystemTimeError,
    },
    #[error("could not allocate a private transfer export work path beside {0}")]
    WorkPathExhausted(PathBuf),
    #[error("transfer package assembly failed in private work path {work_root}: {source}")]
    Assembly {
        work_root: PathBuf,
        #[source]
        source: Box<TransferPackageAssemblyError>,
    },
    #[error(
        "transfer package publication failed from private work path {work_root} to {destination}: {source}"
    )]
    Publication {
        work_root: PathBuf,
        destination: PathBuf,
        #[source]
        source: Box<TransferPackagePublishError>,
    },
    #[error("invalid durable transfer work state at {state_root}: {source}")]
    WorkState {
        state_root: PathBuf,
        #[source]
        source: Box<TransferWorkStateError>,
    },
    #[error(
        "durable transfer work-state destination mismatch: recorded {recorded}, requested {requested}"
    )]
    WorkStateDestinationMismatch {
        recorded: PathBuf,
        requested: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One immutable live attachment selected for an export snapshot.
pub struct TransferAttachmentSource {
    source_path: PathBuf,
    manifest_attachment: ManifestAttachment,
}

impl TransferAttachmentSource {
    pub fn new(
        source_path: impl Into<PathBuf>,
        relative_path: impl Into<String>,
        expected: StoredObjectDescriptor,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            manifest_attachment: ManifestAttachment::new(relative_path, expected),
        }
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn relative_path(&self) -> &str {
        self.manifest_attachment.relative_path()
    }

    pub fn descriptor(&self) -> StoredObjectDescriptor {
        self.manifest_attachment.descriptor()
    }
}

#[derive(Debug, Error)]
/// A local database snapshot and its explicitly selected immutable attachment
/// set could not be prepared as a verified private staging tree.
pub enum TransferStagingError {
    #[error("private transfer staging path already exists: {0}")]
    StagingPathExists(PathBuf),
    #[error("private transfer staging parent is missing or unsafe: {0}")]
    MissingParent(PathBuf),
    #[error("failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create a verified SQLite export snapshot: {0}")]
    DatabaseSnapshot(#[from] BackupError),
    #[error("failed to describe the SQLite export snapshot {path}: {source}")]
    DescribeDatabase {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid transfer manifest for selected live objects: {0}")]
    Manifest(#[from] TransferManifestError),
    #[error("failed to copy selected attachment {logical_path}: {source}")]
    AttachmentCopy {
        logical_path: String,
        #[source]
        source: LocalObjectPublishError,
    },
    #[error("prepared transfer staging failed final verification: {0}")]
    Verification(#[from] TransferVerificationError),
}

#[derive(Debug, Error)]
/// Non-fatal private cleanup residue left after a selected object was safely
/// published into verified staging.
pub enum TransferStagingCleanupWarning {
    #[error("database snapshot cleanup warning: {0}")]
    Database(#[from] BackupCleanupWarning),
    #[error("attachment {logical_path} cleanup warning: {source}")]
    Attachment {
        logical_path: String,
        #[source]
        source: LocalObjectCleanupWarning,
    },
}

#[derive(Debug)]
/// A private staging tree whose database and every selected attachment match
/// its validated logical manifest.
pub struct PreparedTransferStaging {
    root: PathBuf,
    manifest: TransferManifest,
    cleanup_warnings: Vec<TransferStagingCleanupWarning>,
}

#[derive(Debug, Error)]
#[error("private transfer staging cleanup warning for {path}: {source}")]
/// Non-fatal failure to remove local private staging after a package was fully
/// published and verified.
pub struct TransferWorkCleanupWarning {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

#[derive(Debug, Error)]
/// A live-book object set could not complete the conservative end-to-end
/// snapshot and export workflow.
pub enum LiveBookExportError {
    #[error("transfer export destination preflight failed: {0}")]
    Destination(#[source] TransferExportError),
    #[error("local transfer work parent is missing or unsafe: {0}")]
    MissingWorkParent(PathBuf),
    #[error("failed to inspect local transfer work parent {path}: {source}")]
    InspectWorkParent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read the system clock for local transfer work in {path}: {source}")]
    Clock {
        path: PathBuf,
        #[source]
        source: std::time::SystemTimeError,
    },
    #[error("could not allocate private local transfer staging in {0}")]
    WorkPathExhausted(PathBuf),
    #[error("live-book staging preparation failed at {staging_root}: {source}")]
    Preparation {
        staging_root: PathBuf,
        #[source]
        source: Box<TransferStagingError>,
    },
    #[error("live-book export failed from staging {staging_root}: {source}")]
    Export {
        staging_root: PathBuf,
        #[source]
        source: Box<TransferExportError>,
    },
    #[error("failed to create durable export work state at {staging_root}: {source}")]
    WorkState {
        staging_root: PathBuf,
        #[source]
        source: Box<TransferWorkStateError>,
    },
}

#[derive(Debug)]
/// A fully verified public package plus any non-fatal cleanup diagnostics from
/// its private preparation work.
pub struct LiveBookExportOutcome {
    package: VerifiedTransferPackage,
    preparation_cleanup_warnings: Vec<TransferStagingCleanupWarning>,
    staging_cleanup_warning: Option<TransferWorkCleanupWarning>,
}

impl LiveBookExportOutcome {
    pub fn package(&self) -> &VerifiedTransferPackage {
        &self.package
    }

    pub fn preparation_cleanup_warnings(&self) -> &[TransferStagingCleanupWarning] {
        &self.preparation_cleanup_warnings
    }

    pub fn staging_cleanup_warning(&self) -> Option<&TransferWorkCleanupWarning> {
        self.staging_cleanup_warning.as_ref()
    }
}

impl PreparedTransferStaging {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &TransferManifest {
        &self.manifest
    }

    pub fn cleanup_warnings(&self) -> &[TransferStagingCleanupWarning] {
        &self.cleanup_warnings
    }
}

#[derive(Debug)]
/// A public `.kagextra` package that passed conservative synchronization and
/// an independent post-publication verification.
pub struct VerifiedTransferPackage {
    destination: PathBuf,
    manifest: TransferManifest,
    work_state_cleanup_warnings: Vec<TransferWorkStateCleanupWarning>,
}

impl VerifiedTransferPackage {
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn manifest(&self) -> &TransferManifest {
        &self.manifest
    }

    pub fn state(&self) -> TransferOperationState {
        TransferOperationState::Verified
    }

    pub fn work_state_cleanup_warnings(&self) -> &[TransferWorkStateCleanupWarning] {
        &self.work_state_cleanup_warnings
    }
}

/// Creates a verified private export staging tree from one SQLite point-in-time
/// snapshot and an explicit set of immutable attachment objects.
///
/// The caller must select the attachment set under the live-book consistency
/// boundary. Each source is copied only when its bytes match the supplied
/// descriptor, so a missing, replaced, or changing object fails closed. This
/// layer intentionally does not decide which accounting record owns an
/// attachment.
///
/// The staging path and its parent must already be selected on local private
/// storage; the path must not exist. On failure, any created private staging is
/// retained and is never mistaken for a published `.kagextra` package.
pub async fn prepare_transfer_staging(
    pool: &SqlitePool,
    staging_root: impl AsRef<Path>,
    attachments: &[TransferAttachmentSource],
) -> Result<PreparedTransferStaging, TransferStagingError> {
    let staging_root = staging_root.as_ref();
    create_private_staging_root(staging_root)?;
    let attachments_root = staging_root.join("attachments");
    create_staging_directory(&attachments_root, "creating staging attachment directory")?;

    let database_path = staging_root.join(STAGED_DATABASE_PATH);
    let backup = backup_database(pool, &database_path).await?;
    let database =
        describe_file(&database_path).map_err(|source| TransferStagingError::DescribeDatabase {
            path: database_path.clone(),
            source,
        })?;
    let manifest = TransferManifest::new(
        database,
        attachments
            .iter()
            .map(|source| source.manifest_attachment.clone())
            .collect(),
    )?;

    let mut cleanup_warnings = backup
        .into_cleanup_warning()
        .map(TransferStagingCleanupWarning::Database)
        .into_iter()
        .collect::<Vec<_>>();
    for attachment in attachments {
        let destination = staging_root.join(attachment.relative_path());
        create_private_staging_parent_directories(staging_root, &destination)?;
        let outcome = publish_verified_local_copy(
            attachment.source_path(),
            &destination,
            attachment.descriptor(),
        )
        .map_err(|source| TransferStagingError::AttachmentCopy {
            logical_path: attachment.relative_path().to_owned(),
            source,
        })?;
        if let Some(source) = outcome.into_cleanup_warning() {
            cleanup_warnings.push(TransferStagingCleanupWarning::Attachment {
                logical_path: attachment.relative_path().to_owned(),
                source,
            });
        }
    }

    verify_staged_transfer(staging_root, &manifest).await?;
    Ok(PreparedTransferStaging {
        root: staging_root.to_path_buf(),
        manifest,
        cleanup_warnings,
    })
}

/// Creates a verified local snapshot staging tree, exports it through the
/// conservative `.kagextra` publication workflow, and only then attempts to
/// remove the local staging tree.
///
/// A known destination conflict is rejected before taking the SQLite snapshot.
/// On preparation or export failure, the error returns the staging path and the
/// private tree is retained. Once public verification succeeds, later local
/// staging cleanup failure is only a warning and must never cause a retry of the
/// completed export.
pub async fn export_live_book_snapshot(
    pool: &SqlitePool,
    local_work_parent: impl AsRef<Path>,
    attachments: &[TransferAttachmentSource],
    destination: impl AsRef<Path>,
) -> Result<LiveBookExportOutcome, LiveBookExportError> {
    let local_work_parent = local_work_parent.as_ref();
    let destination = destination.as_ref();
    preflight_export_destination(destination).map_err(LiveBookExportError::Destination)?;

    let parent_metadata = std::fs::symlink_metadata(local_work_parent).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            LiveBookExportError::MissingWorkParent(local_work_parent.to_path_buf())
        } else {
            LiveBookExportError::InspectWorkParent {
                path: local_work_parent.to_path_buf(),
                source,
            }
        }
    })?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(LiveBookExportError::MissingWorkParent(
            local_work_parent.to_path_buf(),
        ));
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| LiveBookExportError::Clock {
            path: local_work_parent.to_path_buf(),
            source,
        })?
        .as_nanos();

    for attempt in 0..100_u32 {
        let staging_root = local_work_parent.join(format!(
            ".kAccounting-Extra-01-snapshot-{}-{nonce}-{attempt}.work",
            std::process::id()
        ));
        let prepared = match prepare_transfer_staging(pool, &staging_root, attachments).await {
            Ok(prepared) => prepared,
            Err(TransferStagingError::StagingPathExists(_)) => continue,
            Err(source) => {
                return Err(LiveBookExportError::Preparation {
                    staging_root,
                    source: Box::new(source),
                });
            }
        };
        create_export_work_state(&staging_root, destination).map_err(|source| {
            LiveBookExportError::WorkState {
                staging_root: staging_root.clone(),
                source: Box::new(source),
            }
        })?;

        let package = export_staged_transfer_package_with_work_state(
            prepared.root(),
            destination,
            prepared.manifest(),
            &staging_root,
        )
        .await
        .map_err(|source| LiveBookExportError::Export {
            staging_root: staging_root.clone(),
            source: Box::new(source),
        })?;
        let preparation_cleanup_warnings = prepared.cleanup_warnings;
        let staging_cleanup_warning =
            std::fs::remove_dir_all(&staging_root)
                .err()
                .map(|source| TransferWorkCleanupWarning {
                    path: staging_root,
                    source,
                });
        return Ok(LiveBookExportOutcome {
            package,
            preparation_cleanup_warnings,
            staging_cleanup_warning,
        });
    }

    Err(LiveBookExportError::WorkPathExhausted(
        local_work_parent.to_path_buf(),
    ))
}

fn create_private_staging_root(root: &Path) -> Result<(), TransferStagingError> {
    let parent = root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| TransferStagingError::MissingParent(parent.to_path_buf()))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(TransferStagingError::MissingParent(parent.to_path_buf()));
    }
    match std::fs::symlink_metadata(root) {
        Ok(_) => {
            return Err(TransferStagingError::StagingPathExists(root.to_path_buf()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(TransferStagingError::Io {
                operation: "checking private staging path",
                path: root.to_path_buf(),
                source,
            });
        }
    }
    create_staging_directory(root, "creating private staging directory")
}

fn create_private_staging_parent_directories(
    staging_root: &Path,
    destination: &Path,
) -> Result<(), TransferStagingError> {
    let parent = destination
        .parent()
        .ok_or_else(|| TransferStagingError::Io {
            operation: "resolving staging attachment parent for",
            path: destination.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment destination has no parent",
            ),
        })?;
    let relative_parent =
        parent
            .strip_prefix(staging_root)
            .map_err(|source| TransferStagingError::Io {
                operation: "confining staging attachment destination",
                path: destination.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, source),
            })?;
    let mut current = staging_root.to_path_buf();
    for component in relative_parent.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(TransferStagingError::Io {
                    operation: "checking private staging attachment directory",
                    path: current,
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "path is not a real directory",
                    ),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_staging_directory(
                    &current,
                    "creating private staging attachment directory",
                )?;
            }
            Err(source) => {
                return Err(TransferStagingError::Io {
                    operation: "checking private staging attachment directory",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn create_staging_directory(
    path: &Path,
    operation: &'static str,
) -> Result<(), TransferStagingError> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|source| TransferStagingError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

/// Exports an already consistent and verified logical staging tree as one
/// durable `.kagextra` directory package.
///
/// The private work directory is allocated beside the destination so final
/// publication cannot cross a file-system boundary. Existing destination
/// objects are rejected before assembly and are still protected by the atomic
/// no-replace publication primitive against races. The source staging tree is
/// never modified or deleted.
///
/// On an assembly or publication error, the returned error identifies the
/// selected private work path. Work that remains private is deliberately
/// retained for explicit diagnosis or cleanup. A post-rename publication
/// failure remains distinguishable through its nested error and never causes
/// automatic deletion of the public destination.
pub async fn export_staged_transfer_package(
    source_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    manifest: &TransferManifest,
) -> Result<VerifiedTransferPackage, TransferExportError> {
    export_staged_transfer_package_internal(source_root, destination, manifest, None).await
}

/// Exports verified staging while persisting the actual publication lifecycle
/// in a separately retained private work root. The work root must already hold
/// a matching initial `working` export record.
pub async fn export_staged_transfer_package_with_work_state(
    source_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    manifest: &TransferManifest,
    state_root: impl AsRef<Path>,
) -> Result<VerifiedTransferPackage, TransferExportError> {
    let destination = destination.as_ref();
    let state_root = state_root.as_ref();
    let persisted =
        load_transfer_work_state(state_root).map_err(|source| TransferExportError::WorkState {
            state_root: state_root.to_path_buf(),
            source: Box::new(source),
        })?;
    if persisted.state() != TransferOperationState::Working {
        return Err(TransferExportError::WorkState {
            state_root: state_root.to_path_buf(),
            source: Box::new(TransferWorkStateError::StaleState {
                expected: TransferOperationState::Working,
                actual: persisted.state(),
            }),
        });
    }
    if persisted.destination() != destination {
        return Err(TransferExportError::WorkStateDestinationMismatch {
            recorded: persisted.destination().to_path_buf(),
            requested: destination.to_path_buf(),
        });
    }
    export_staged_transfer_package_internal(source_root, destination, manifest, Some(state_root))
        .await
}

async fn export_staged_transfer_package_internal(
    source_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    manifest: &TransferManifest,
    state_root: Option<&Path>,
) -> Result<VerifiedTransferPackage, TransferExportError> {
    let source_root = source_root.as_ref();
    let destination = destination.as_ref();

    preflight_export_destination(destination)?;

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| TransferExportError::Clock {
            path: parent.to_path_buf(),
            source,
        })?
        .as_nanos();

    for attempt in 0..100_u32 {
        let work_root = parent.join(format!(
            ".kAccounting-Extra-01-export-{}-{nonce}-{attempt}.work",
            std::process::id()
        ));
        match assemble_private_transfer_package(source_root, &work_root, manifest).await {
            Ok(()) => {
                return publish_private_transfer_package_internal(
                    &work_root,
                    destination,
                    state_root,
                )
                .await
                .map_err(|source| TransferExportError::Publication {
                    work_root,
                    destination: destination.to_path_buf(),
                    source: Box::new(source),
                });
            }
            Err(TransferPackageAssemblyError::WorkPathExists(_)) => continue,
            Err(source) => {
                return Err(TransferExportError::Assembly {
                    work_root,
                    source: Box::new(source),
                });
            }
        }
    }

    Err(TransferExportError::WorkPathExhausted(parent.to_path_buf()))
}

fn preflight_export_destination(destination: &Path) -> Result<(), TransferExportError> {
    match std::fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(TransferExportError::DestinationExists(
                destination.to_path_buf(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(TransferExportError::InspectDestination {
                path: destination.to_path_buf(),
                source,
            });
        }
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            TransferExportError::MissingParent(parent.to_path_buf())
        } else {
            TransferExportError::InspectParent {
                path: parent.to_path_buf(),
                source,
            }
        }
    })?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(TransferExportError::MissingParent(parent.to_path_buf()));
    }
    Ok(())
}

/// Copies a verified staging tree into a newly created private package work
/// directory, writes `manifest.json`, and verifies the complete result.
///
/// This function never publishes the work directory. The caller must choose a
/// private, non-existing path beside the future destination and retain it on
/// error for explicit recovery or cleanup.
pub async fn assemble_private_transfer_package(
    source_root: impl AsRef<Path>,
    work_root: impl AsRef<Path>,
    manifest: &TransferManifest,
) -> Result<(), TransferPackageAssemblyError> {
    let source_root = source_root.as_ref();
    let work_root = work_root.as_ref();
    verify_staged_transfer(source_root, manifest).await?;
    create_private_package_root(work_root)?;

    let attachments_root = work_root.join("attachments");
    create_owner_only_directory(&attachments_root, "creating attachment directory")?;

    copy_package_object(
        &source_root.join(STAGED_DATABASE_PATH),
        &work_root.join(STAGED_DATABASE_PATH),
        STAGED_DATABASE_PATH,
        manifest.database(),
    )?;

    for attachment in manifest.attachments() {
        let destination = work_root.join(attachment.relative_path());
        create_private_parent_directories(work_root, &destination)?;
        copy_package_object(
            &source_root.join(attachment.relative_path()),
            &destination,
            attachment.relative_path(),
            attachment.descriptor(),
        )?;
    }

    write_private_manifest(work_root, manifest)?;
    let assembled = load_and_verify_transfer_package(work_root).await?;
    debug_assert_eq!(&assembled, manifest);
    Ok(())
}

/// Durably publishes one already assembled private package without replacing
/// a destination, then independently verifies the public package.
///
/// Every declared file and every package directory is synchronized before the
/// atomic rename. After publication, the destination parent is synchronized so
/// the new name is part of the success boundary. Unsupported directory sync or
/// any other synchronization failure is fatal. A failure after the rename is
/// reported explicitly and never causes automatic destination deletion.
pub async fn publish_private_transfer_package(
    work_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<VerifiedTransferPackage, TransferPackagePublishError> {
    publish_private_transfer_package_internal(work_root, destination, None).await
}

async fn publish_private_transfer_package_internal(
    work_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    state_root: Option<&Path>,
) -> Result<VerifiedTransferPackage, TransferPackagePublishError> {
    let work_root = work_root.as_ref();
    let destination = destination.as_ref();
    let mut work_state_cleanup_warnings = Vec::new();

    let manifest = load_and_verify_transfer_package(work_root)
        .await
        .map_err(|source| TransferPackagePublishError::PrivateVerification(Box::new(source)))?;
    synchronize_private_package(work_root, &manifest)?;
    publish_private_directory_noreplace(work_root, destination)?;
    if let Some(state_root) = state_root {
        let outcome = transition_transfer_work_state(
            state_root,
            TransferOperationState::Working,
            TransferOperationState::PublishedUnverified,
        )
        .map_err(|source| TransferPackagePublishError::PublishedState {
            destination: destination.to_path_buf(),
            source: Box::new(source),
        })?;
        if let Some(warning) = outcome.into_cleanup_warning() {
            work_state_cleanup_warnings.push(warning);
        }
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(
            |source| TransferPackagePublishError::PublishedParentSynchronization {
                destination: destination.to_path_buf(),
                parent: parent.to_path_buf(),
                source,
            },
        )?;

    let published_manifest = load_and_verify_transfer_package(destination)
        .await
        .map_err(
            |source| TransferPackagePublishError::PublishedVerification {
                destination: destination.to_path_buf(),
                source: Box::new(source),
            },
        )?;
    debug_assert_eq!(published_manifest, manifest);
    if let Some(state_root) = state_root {
        let outcome = transition_transfer_work_state(
            state_root,
            TransferOperationState::PublishedUnverified,
            TransferOperationState::Verified,
        )
        .map_err(|source| TransferPackagePublishError::VerifiedState {
            destination: destination.to_path_buf(),
            source: Box::new(source),
        })?;
        if let Some(warning) = outcome.into_cleanup_warning() {
            work_state_cleanup_warnings.push(warning);
        }
    }

    Ok(VerifiedTransferPackage {
        destination: destination.to_path_buf(),
        manifest: published_manifest,
        work_state_cleanup_warnings,
    })
}

fn synchronize_private_package(
    root: &Path,
    manifest: &TransferManifest,
) -> Result<(), TransferPackagePublishError> {
    let mut files = Vec::with_capacity(manifest.attachments().len() + 2);
    files.push(root.join(STAGED_DATABASE_PATH));
    files.extend(
        manifest
            .attachments()
            .iter()
            .map(|attachment| root.join(attachment.relative_path())),
    );
    files.push(root.join(TRANSFER_MANIFEST_FILE_NAME));
    for path in files {
        File::open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|source| TransferPackagePublishError::Synchronization {
                path: path.clone(),
                source,
            })?;
    }

    let mut directories = HashSet::from([root.to_path_buf(), root.join("attachments")]);
    for attachment in manifest.attachments() {
        let segments = attachment.relative_path().split('/').collect::<Vec<_>>();
        for end in 1..segments.len() {
            directories.insert(root.join(segments[..end].join("/")));
        }
    }
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });
    for path in directories {
        File::open(&path)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| TransferPackagePublishError::Synchronization {
                path: path.clone(),
                source,
            })?;
    }
    Ok(())
}

fn create_private_package_root(work_root: &Path) -> Result<(), TransferPackageAssemblyError> {
    let parent = work_root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| TransferPackageAssemblyError::MissingParent(parent.to_path_buf()))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(TransferPackageAssemblyError::MissingParent(
            parent.to_path_buf(),
        ));
    }

    match std::fs::symlink_metadata(work_root) {
        Ok(_) => {
            return Err(TransferPackageAssemblyError::WorkPathExists(
                work_root.to_path_buf(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(TransferPackageAssemblyError::Io {
                operation: "checking private package work path",
                path: work_root.to_path_buf(),
                source,
            });
        }
    }

    create_owner_only_directory(work_root, "creating private package work directory")
}

fn create_private_parent_directories(
    work_root: &Path,
    destination: &Path,
) -> Result<(), TransferPackageAssemblyError> {
    let parent = destination
        .parent()
        .ok_or_else(|| TransferPackageAssemblyError::Io {
            operation: "resolving attachment parent for",
            path: destination.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment destination has no parent",
            ),
        })?;
    let relative_parent =
        parent
            .strip_prefix(work_root)
            .map_err(|source| TransferPackageAssemblyError::Io {
                operation: "confining attachment destination",
                path: destination.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, source),
            })?;

    let mut current = work_root.to_path_buf();
    for component in relative_parent.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(TransferPackageAssemblyError::Io {
                    operation: "checking private attachment directory",
                    path: current,
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "path is not a real directory",
                    ),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_owner_only_directory(&current, "creating private attachment directory")?;
            }
            Err(source) => {
                return Err(TransferPackageAssemblyError::Io {
                    operation: "checking private attachment directory",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn create_owner_only_directory(
    path: &Path,
    operation: &'static str,
) -> Result<(), TransferPackageAssemblyError> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|source| TransferPackageAssemblyError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

fn copy_package_object(
    source: &Path,
    destination: &Path,
    logical_path: &str,
    descriptor: StoredObjectDescriptor,
) -> Result<(), TransferPackageAssemblyError> {
    let outcome =
        publish_verified_local_copy(source, destination, descriptor).map_err(|source| {
            TransferPackageAssemblyError::CopyObject {
                logical_path: logical_path.to_owned(),
                source,
            }
        })?;
    if let Some(source) = outcome.into_cleanup_warning() {
        return Err(TransferPackageAssemblyError::ObjectCleanup {
            logical_path: logical_path.to_owned(),
            source,
        });
    }
    Ok(())
}

fn write_private_manifest(
    work_root: &Path,
    manifest: &TransferManifest,
) -> Result<(), TransferPackageAssemblyError> {
    let path = work_root.join(TRANSFER_MANIFEST_FILE_NAME);
    let bytes = manifest.to_json_bytes()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|source| TransferPackageAssemblyError::Io {
            operation: "creating private transfer manifest",
            path: path.clone(),
            source,
        })?;
    file.write_all(&bytes)
        .map_err(|source| TransferPackageAssemblyError::Io {
            operation: "writing private transfer manifest",
            path: path.clone(),
            source,
        })?;
    file.sync_all()
        .map_err(|source| TransferPackageAssemblyError::Io {
            operation: "synchronizing private transfer manifest",
            path,
            source,
        })
}

/// Loads and verifies one complete physical `.kagextra` directory package.
///
/// In addition to checking every declared object's length, digest, and database
/// identity, this rejects unexpected top-level entries, undeclared attachment
/// files, extra directories, symlinks, and non-regular filesystem objects.
pub async fn load_and_verify_transfer_package(
    root: impl AsRef<Path>,
) -> Result<TransferManifest, TransferPackageVerificationError> {
    let root = root.as_ref();
    require_real_directory(root, true)?;

    let manifest_path = root.join(TRANSFER_MANIFEST_FILE_NAME);
    require_regular_file(&manifest_path)?;
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|source| {
        TransferPackageVerificationError::ReadManifest {
            path: manifest_path.clone(),
            source,
        }
    })?;
    let manifest = TransferManifest::from_json_bytes(&manifest_bytes)?;

    verify_package_layout(root, &manifest)?;
    verify_staged_transfer(root, &manifest).await?;
    Ok(manifest)
}

fn verify_package_layout(
    root: &Path,
    manifest: &TransferManifest,
) -> Result<(), TransferPackageVerificationError> {
    let allowed_top_level = [
        TRANSFER_MANIFEST_FILE_NAME,
        STAGED_DATABASE_PATH,
        "attachments",
    ];
    for entry in read_directory(root)? {
        let entry = entry.map_err(|source| TransferPackageVerificationError::ReadDirectory {
            path: root.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(TransferPackageVerificationError::UndeclaredObject { path: entry.path() });
        };
        if !allowed_top_level.contains(&name) {
            return Err(TransferPackageVerificationError::UndeclaredObject { path: entry.path() });
        }
    }

    require_regular_file(&root.join(STAGED_DATABASE_PATH))?;

    let expected_files = manifest
        .attachments()
        .iter()
        .map(|attachment| attachment.relative_path().to_owned())
        .collect::<HashSet<_>>();
    let mut expected_directories = HashSet::from(["attachments".to_owned()]);
    for path in &expected_files {
        let segments = path.split('/').collect::<Vec<_>>();
        for end in 1..segments.len() {
            expected_directories.insert(segments[..end].join("/"));
        }
    }

    let mut pending = vec!["attachments".to_owned()];
    while let Some(logical_directory) = pending.pop() {
        let directory_path = root.join(&logical_directory);
        require_real_directory(&directory_path, false)?;
        for entry in read_directory(&directory_path)? {
            let entry =
                entry.map_err(|source| TransferPackageVerificationError::ReadDirectory {
                    path: directory_path.clone(),
                    source,
                })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(TransferPackageVerificationError::UndeclaredObject {
                    path: entry.path(),
                });
            };
            let logical_path = format!("{logical_directory}/{name}");
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|source| {
                TransferPackageVerificationError::ReadDirectory {
                    path: entry.path(),
                    source,
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(TransferPackageVerificationError::UnsafeObjectType {
                    path: entry.path(),
                });
            }
            if metadata.is_dir() {
                if !expected_directories.contains(&logical_path) {
                    return Err(TransferPackageVerificationError::UndeclaredObject {
                        path: entry.path(),
                    });
                }
                pending.push(logical_path);
            } else if metadata.is_file() {
                if !expected_files.contains(&logical_path) {
                    return Err(TransferPackageVerificationError::UndeclaredObject {
                        path: entry.path(),
                    });
                }
            } else {
                return Err(TransferPackageVerificationError::UnsafeObjectType {
                    path: entry.path(),
                });
            }
        }
    }

    Ok(())
}

fn require_real_directory(
    path: &Path,
    is_root: bool,
) -> Result<(), TransferPackageVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| {
        if is_root {
            TransferPackageVerificationError::InvalidRoot {
                path: path.to_path_buf(),
                source,
            }
        } else {
            TransferPackageVerificationError::ReadDirectory {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(TransferPackageVerificationError::UnsafeObjectType {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<(), TransferPackageVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| {
        TransferPackageVerificationError::ReadDirectory {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(TransferPackageVerificationError::UnsafeObjectType {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn read_directory(path: &Path) -> Result<std::fs::ReadDir, TransferPackageVerificationError> {
    std::fs::read_dir(path).map_err(|source| TransferPackageVerificationError::ReadDirectory {
        path: path.to_path_buf(),
        source,
    })
}

/// Verifies every declared object and the application database in a decoded
/// private staging directory.
///
/// This function is read-only. Publication or replacement of a live book
/// requires a separate transactional import workflow.
pub async fn verify_staged_transfer(
    root: impl AsRef<Path>,
    manifest: &TransferManifest,
) -> Result<(), TransferVerificationError> {
    let root = root.as_ref();
    let root_metadata = std::fs::symlink_metadata(root).map_err(|source| {
        TransferVerificationError::InvalidRoot {
            path: root.to_path_buf(),
            source,
        }
    })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(TransferVerificationError::InvalidRoot {
            path: root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "staging root must be a real directory",
            ),
        });
    }

    let database_path = root.join(STAGED_DATABASE_PATH);
    verify_staged_object(&database_path, STAGED_DATABASE_PATH, manifest.database())?;
    verify_database_snapshot(&database_path)
        .await
        .map_err(TransferVerificationError::Database)?;

    for attachment in manifest.attachments() {
        let path = root.join(attachment.relative_path());
        verify_staged_object(&path, attachment.relative_path(), attachment.descriptor())?;
    }

    Ok(())
}

fn verify_staged_object(
    path: &Path,
    logical_path: &str,
    descriptor: StoredObjectDescriptor,
) -> Result<(), TransferVerificationError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(TransferVerificationError::UnsafeObjectType {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(TransferVerificationError::Object {
                logical_path: logical_path.to_owned(),
                source: StoredObjectVerificationError::Read {
                    path: path.to_path_buf(),
                    source,
                },
            });
        }
    }

    verify_file(path, descriptor).map_err(|source| TransferVerificationError::Object {
        logical_path: logical_path.to_owned(),
        source,
    })
}
