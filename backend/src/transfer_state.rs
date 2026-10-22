//! Durable private work-state records for interrupted transfer recovery.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::transfer::{TransferOperationState, TransferStateTransitionError};

pub const TRANSFER_WORK_STATE_FILE_NAME: &str = "operation-state.json";
const TRANSFER_WORK_STATE_LOCK_FILE_NAME: &str = ".operation-state.lock";
const CURRENT_TRANSFER_WORK_STATE_VERSION: u32 = 1;
const MAX_TRANSFER_WORK_STATE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Strictly decoded durable state for one private export operation.
pub struct PersistedTransferWorkState {
    destination: PathBuf,
    state: TransferOperationState,
}

impl PersistedTransferWorkState {
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn state(&self) -> TransferOperationState {
        self.state
    }
}

#[derive(Debug, Error)]
/// A private transfer work-state record could not be created, read, or
/// advanced safely.
pub enum TransferWorkStateError {
    #[error("transfer work root is missing or unsafe: {0}")]
    InvalidRoot(PathBuf),
    #[error("transfer work-state destination path is not valid UTF-8: {0}")]
    InvalidDestination(PathBuf),
    #[error("transfer work-state file {path} is too large: {actual} bytes, maximum is {maximum}")]
    StateTooLarge {
        path: PathBuf,
        actual: u64,
        maximum: u64,
    },
    #[error("transfer work-state file already exists: {0}")]
    StateExists(PathBuf),
    #[error("transfer work-state file is not a regular non-symlink file: {0}")]
    UnsafeStateFile(PathBuf),
    #[error("transfer work-state update is already locked: {0}")]
    UpdateLocked(PathBuf),
    #[error("failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid transfer work-state JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "unsupported transfer work-state version {actual}; supported version is {CURRENT_TRANSFER_WORK_STATE_VERSION}"
    )]
    UnsupportedVersion { actual: u32 },
    #[error("unsupported transfer work-state operation {0}")]
    UnsupportedOperation(String),
    #[error("transfer work-state expected {expected:?}, found {actual:?}")]
    StaleState {
        expected: TransferOperationState,
        actual: TransferOperationState,
    },
    #[error("invalid transfer work-state transition: {0}")]
    Transition(#[from] TransferStateTransitionError),
}

#[derive(Debug, Error)]
#[error("transfer work-state cleanup warning while {operation} {path}: {source}")]
/// Non-fatal lock cleanup failure after a new state was atomically persisted.
pub struct TransferWorkStateCleanupWarning {
    pub operation: &'static str,
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

#[derive(Debug)]
/// Result of an atomically persisted monotonic state transition.
pub struct TransferWorkStateTransitionOutcome {
    state: PersistedTransferWorkState,
    cleanup_warning: Option<TransferWorkStateCleanupWarning>,
}

impl TransferWorkStateTransitionOutcome {
    pub fn state(&self) -> &PersistedTransferWorkState {
        &self.state
    }

    pub fn cleanup_warning(&self) -> Option<&TransferWorkStateCleanupWarning> {
        self.cleanup_warning.as_ref()
    }

    pub fn into_cleanup_warning(self) -> Option<TransferWorkStateCleanupWarning> {
        self.cleanup_warning
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkStateDocument {
    format_version: u32,
    operation: String,
    state: WorkStateValue,
    destination: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkStateValue {
    Working,
    PublishedUnverified,
    Verified,
    Failed,
}

impl From<TransferOperationState> for WorkStateValue {
    fn from(value: TransferOperationState) -> Self {
        match value {
            TransferOperationState::Working => Self::Working,
            TransferOperationState::PublishedUnverified => Self::PublishedUnverified,
            TransferOperationState::Verified => Self::Verified,
            TransferOperationState::Failed => Self::Failed,
        }
    }
}

impl From<WorkStateValue> for TransferOperationState {
    fn from(value: WorkStateValue) -> Self {
        match value {
            WorkStateValue::Working => Self::Working,
            WorkStateValue::PublishedUnverified => Self::PublishedUnverified,
            WorkStateValue::Verified => Self::Verified,
            WorkStateValue::Failed => Self::Failed,
        }
    }
}

/// Creates and durably synchronizes the initial `working` state without
/// replacing any existing record.
pub fn create_export_work_state(
    work_root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<PersistedTransferWorkState, TransferWorkStateError> {
    let work_root = work_root.as_ref();
    require_real_work_root(work_root)?;
    let destination = destination.as_ref();
    if !destination.is_absolute() {
        return Err(TransferWorkStateError::InvalidDestination(
            destination.to_path_buf(),
        ));
    }
    let destination_text = destination
        .to_str()
        .ok_or_else(|| TransferWorkStateError::InvalidDestination(destination.to_path_buf()))?;
    let state = PersistedTransferWorkState {
        destination: destination.to_path_buf(),
        state: TransferOperationState::Working,
    };
    let document = WorkStateDocument {
        format_version: CURRENT_TRANSFER_WORK_STATE_VERSION,
        operation: "export".to_owned(),
        state: state.state.into(),
        destination: destination_text.to_owned(),
    };
    let path = work_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let bytes = encode_document(&document, &path)?;
    write_new_private_file(&path, &bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            TransferWorkStateError::StateExists(path.clone())
        } else {
            TransferWorkStateError::Io {
                operation: "creating transfer work-state file",
                path: path.clone(),
                source: error,
            }
        }
    })?;
    synchronize_directory(work_root)?;
    Ok(state)
}

/// Loads a strict work-state record without modifying it.
pub fn load_transfer_work_state(
    work_root: impl AsRef<Path>,
) -> Result<PersistedTransferWorkState, TransferWorkStateError> {
    let work_root = work_root.as_ref();
    require_real_work_root(work_root)?;
    let path = work_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|source| TransferWorkStateError::Io {
            operation: "inspecting transfer work-state file",
            path: path.clone(),
            source,
        })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(TransferWorkStateError::UnsafeStateFile(path));
    }
    if metadata.len() > MAX_TRANSFER_WORK_STATE_BYTES {
        return Err(TransferWorkStateError::StateTooLarge {
            path,
            actual: metadata.len(),
            maximum: MAX_TRANSFER_WORK_STATE_BYTES,
        });
    }
    let bytes = std::fs::read(&path).map_err(|source| TransferWorkStateError::Io {
        operation: "reading transfer work-state file",
        path: path.clone(),
        source,
    })?;
    let document: WorkStateDocument =
        serde_json::from_slice(&bytes).map_err(|source| TransferWorkStateError::Json {
            path: path.clone(),
            source,
        })?;
    if document.format_version != CURRENT_TRANSFER_WORK_STATE_VERSION {
        return Err(TransferWorkStateError::UnsupportedVersion {
            actual: document.format_version,
        });
    }
    if document.operation != "export" {
        return Err(TransferWorkStateError::UnsupportedOperation(
            document.operation,
        ));
    }
    let destination = PathBuf::from(document.destination);
    if !destination.is_absolute() {
        return Err(TransferWorkStateError::InvalidDestination(destination));
    }
    Ok(PersistedTransferWorkState {
        destination,
        state: document.state.into(),
    })
}

/// Atomically and durably advances a work-state record while holding an
/// exclusive create-new lock. The expected current state prevents a stale
/// caller from overwriting a newer terminal result.
pub fn transition_transfer_work_state(
    work_root: impl AsRef<Path>,
    expected: TransferOperationState,
    next: TransferOperationState,
) -> Result<TransferWorkStateTransitionOutcome, TransferWorkStateError> {
    let work_root = work_root.as_ref();
    require_real_work_root(work_root)?;
    let lock_path = work_root.join(TRANSFER_WORK_STATE_LOCK_FILE_NAME);
    let lock = create_lock_file(&lock_path)?;

    let result = transition_while_locked(work_root, expected, next);
    drop(lock);
    match result {
        Err(error) => {
            let _ = std::fs::remove_file(&lock_path);
            Err(error)
        }
        Ok(state) => {
            let cleanup_warning = std::fs::remove_file(&lock_path)
                .and_then(|()| File::open(work_root)?.sync_all())
                .err()
                .map(|source| TransferWorkStateCleanupWarning {
                    operation: "removing and synchronizing transfer work-state lock",
                    path: lock_path,
                    source,
                });
            Ok(TransferWorkStateTransitionOutcome {
                state,
                cleanup_warning,
            })
        }
    }
}

fn transition_while_locked(
    work_root: &Path,
    expected: TransferOperationState,
    next: TransferOperationState,
) -> Result<PersistedTransferWorkState, TransferWorkStateError> {
    let current = load_transfer_work_state(work_root)?;
    if current.state != expected {
        return Err(TransferWorkStateError::StaleState {
            expected,
            actual: current.state,
        });
    }
    let next = current.state.transition(next)?;
    let destination = current
        .destination
        .to_str()
        .ok_or_else(|| TransferWorkStateError::InvalidDestination(current.destination.clone()))?;
    let document = WorkStateDocument {
        format_version: CURRENT_TRANSFER_WORK_STATE_VERSION,
        operation: "export".to_owned(),
        state: next.into(),
        destination: destination.to_owned(),
    };
    let state_path = work_root.join(TRANSFER_WORK_STATE_FILE_NAME);
    let bytes = encode_document(&document, &state_path)?;
    let temporary_path = create_transition_file(work_root, &bytes)?;
    std::fs::rename(&temporary_path, &state_path).map_err(|source| TransferWorkStateError::Io {
        operation: "atomically replacing transfer work-state file",
        path: state_path.clone(),
        source,
    })?;
    synchronize_directory(work_root)?;
    Ok(PersistedTransferWorkState {
        destination: current.destination,
        state: next,
    })
}

fn require_real_work_root(path: &Path) -> Result<(), TransferWorkStateError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| TransferWorkStateError::InvalidRoot(path.to_path_buf()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(TransferWorkStateError::InvalidRoot(path.to_path_buf()));
    }
    Ok(())
}

fn encode_document(
    document: &WorkStateDocument,
    path: &Path,
) -> Result<Vec<u8>, TransferWorkStateError> {
    let mut bytes =
        serde_json::to_vec_pretty(document).map_err(|source| TransferWorkStateError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_new_private_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn create_lock_file(path: &Path) -> Result<File, TransferWorkStateError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(file) => {
            file.sync_all()
                .map_err(|source| TransferWorkStateError::Io {
                    operation: "synchronizing transfer work-state lock",
                    path: path.to_path_buf(),
                    source,
                })?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(TransferWorkStateError::UpdateLocked(path.to_path_buf()))
        }
        Err(source) => Err(TransferWorkStateError::Io {
            operation: "creating transfer work-state lock",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn create_transition_file(
    work_root: &Path,
    bytes: &[u8],
) -> Result<PathBuf, TransferWorkStateError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| TransferWorkStateError::Io {
            operation: "reading system clock for transfer work-state update in",
            path: work_root.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .as_nanos();
    for attempt in 0..100_u32 {
        let path = work_root.join(format!(
            ".operation-state-{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        match write_new_private_file(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(TransferWorkStateError::Io {
                    operation: "writing private transfer work-state update",
                    path,
                    source,
                });
            }
        }
    }
    Err(TransferWorkStateError::Io {
        operation: "allocating private transfer work-state update in",
        path: work_root.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "100 transfer work-state update names already existed",
        ),
    })
}

fn synchronize_directory(path: &Path) -> Result<(), TransferWorkStateError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| TransferWorkStateError::Io {
            operation: "synchronizing transfer work directory",
            path: path.to_path_buf(),
            source,
        })
}
