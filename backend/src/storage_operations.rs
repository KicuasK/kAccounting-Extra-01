//! Process-local capabilities and owned jobs for explicitly selected storage.
//! This module has no HTTP path-registration API and never selects a book.
use crate::{
    application_commands as commands,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::{RecipientKeyStore, RecipientRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

const LIMIT: usize = 256;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionKind {
    Book,
    Request,
    Package,
    Backup,
    RequestDestination,
    ExportDestination,
    BookDestination,
    RecoveryParent,
}
#[derive(Clone)]
struct Selection {
    kind: SelectionKind,
    path: PathBuf,
    request: Option<RecipientRequest>,
    identities: Vec<(PathBuf, std::fs::Metadata)>,
}
#[derive(Clone)]
struct Device(Arc<dyn DeviceKeyStore + Send + Sync>);
impl DeviceKeyStore for Device {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.read(id)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.0.insert_new(id, bytes)
    }
}
#[derive(Clone)]
struct Recipient(Arc<dyn RecipientKeyStore + Send + Sync>);
impl RecipientKeyStore for Recipient {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.read(id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.0.insert_new(id, bytes)
    }
}
#[derive(Clone)]
pub struct StorageOperations {
    inner: Arc<Inner>,
}
struct Inner {
    device: Device,
    recipient: Recipient,
    active_book: Option<Arc<crate::runtime::RuntimeBook>>,
    selections: Mutex<HashMap<String, Selection>>,
    jobs: Mutex<HashMap<String, OperationStatus>>,
    downloads: Mutex<HashMap<String, DownloadArtifact>>,
    permits: Arc<tokio::sync::Semaphore>,
    download_permits: Arc<tokio::sync::Semaphore>,
    container_limits: crate::transfer_container::ContainerLimits,
}
#[derive(Clone)]
pub(crate) struct DownloadArtifact {
    pub root: PathBuf,
    pub entries: Vec<crate::transfer_container::ContainerEntry>,
    identities: Vec<(PathBuf, std::fs::Metadata)>,
}
impl DownloadArtifact {
    pub fn validate(&self) -> Result<(), OperationError> {
        validate_identity(&Selection {
            kind: SelectionKind::Package,
            path: self.root.clone(),
            request: None,
            identities: self.identities.clone(),
        })?;
        for entry in &self.entries {
            if crate::storage::describe_file(self.root.join(entry.name()))
                .map_err(|_| OperationError::NotFound)?
                != entry.descriptor()
            {
                return Err(OperationError::NotFound);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct OperationStatus {
    pub operation_id: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<OperationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct OperationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_bytes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unrecognized_count: Option<String>,
    pub cleanup_warning: bool,
    /// Import/restore never replace the active book. A separate user selection is required.
    pub selection_required: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct RequestPreview {
    pub request_id: String,
    pub fingerprint: String,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OperationError {
    InvalidSelection,
    ConfirmationRequired,
    Busy,
    Capacity,
    NotFound,
    Internal,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryInput {
    Imports { parent: String },
    Exports { book: String, parent: String },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestInput {
    pub destination: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportInput {
    pub book: String,
    pub request: String,
    pub destination: String,
    pub confirmed_request_id: String,
    pub confirmed_fingerprint: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportInput {
    pub package: String,
    pub request: String,
    pub destination: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreInput {
    pub backup: String,
    pub destination: String,
}
enum Work {
    Request(PathBuf),
    Export(PathBuf, RecipientRequest, PathBuf),
    Import(PathBuf, RecipientRequest, PathBuf),
    Restore(PathBuf, PathBuf),
    RecoverImports(PathBuf),
    RecoverExports(PathBuf, PathBuf),
}
impl StorageOperations {
    /// Session teardown may proceed only after its owned storage job finishes.
    pub fn is_idle(&self) -> bool {
        self.inner.permits.available_permits() == 1
    }
    /// Wait without polling until the owned mutation worker has finished.
    pub async fn wait_idle(&self) {
        let _permit = self.inner.permits.acquire().await;
    }
    /// Construct once per app process, sharing this same object with all routers.
    /// Real adapters are provided by the local/native layer; tests use fake keys.
    pub fn new<
        D: DeviceKeyStore + Send + Sync + 'static,
        R: RecipientKeyStore + Send + Sync + 'static,
    >(
        device: Arc<D>,
        recipient: Arc<R>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                device: Device(device),
                recipient: Recipient(recipient),
                active_book: None,
                selections: Mutex::new(HashMap::new()),
                jobs: Mutex::new(HashMap::new()),
                downloads: Mutex::new(HashMap::new()),
                permits: Arc::new(tokio::sync::Semaphore::new(1)),
                download_permits: Arc::new(tokio::sync::Semaphore::new(2)),
                container_limits: crate::transfer_container::ContainerLimits {
                    max_entries: 100_003,
                    max_object_bytes: 32 * 1024 * 1024 * 1024,
                    max_total_bytes: 64 * 1024 * 1024 * 1024,
                },
            }),
        }
    }
    /// Deployment transport resource limits, separate from accounting and
    /// attachment policy. Configure before sharing this service with routers.
    pub fn with_container_limits(
        mut self,
        limits: crate::transfer_container::ContainerLimits,
    ) -> Result<Self, OperationError> {
        if limits.max_entries < 3
            || limits.max_entries > 100_003
            || limits.max_object_bytes == 0
            || limits.max_object_bytes > limits.max_total_bytes
            || limits
                .max_total_bytes
                .checked_add(u64::from(limits.max_entries) * 128 + 64)
                .is_none()
        {
            return Err(OperationError::InvalidSelection);
        }
        Arc::get_mut(&mut self.inner)
            .ok_or(OperationError::Busy)?
            .container_limits = limits;
        Ok(self)
    }
    pub(crate) fn container_upload_maximum(&self) -> u64 {
        let limits = self.inner.container_limits;
        // Canonical names and descriptors consume less than 128 bytes per entry.
        limits.max_total_bytes + u64::from(limits.max_entries) * 128 + 64
    }
    /// Keep the active book and its exclusive runtime lock alive for HTTP export.
    /// This may only be configured before cloning the service into routers.
    pub fn with_active_book(
        mut self,
        book: Arc<crate::runtime::RuntimeBook>,
    ) -> Result<Self, OperationError> {
        Arc::get_mut(&mut self.inner)
            .ok_or(OperationError::Busy)?
            .active_book = Some(book);
        Ok(self)
    }
    /// Local/native registration only, after a genuine OS file-selection action.
    /// Never call this using an untrusted request path. No path is returned to HTTP.
    /// Public requests are parsed once here so later file replacement cannot change
    /// the recipient after the user confirms the displayed fingerprint.
    pub fn register_selection(
        &self,
        kind: SelectionKind,
        path: impl AsRef<Path>,
    ) -> Result<String, OperationError> {
        let path = path.as_ref();
        validate_selection(kind, path)?;
        let identities = capture_identities(kind, path)?;
        let request = if kind == SelectionKind::Request {
            Some(
                commands::read_recipient_request(path)
                    .map_err(|_| OperationError::InvalidSelection)?,
            )
        } else {
            None
        };
        let mut selections = self
            .inner
            .selections
            .lock()
            .map_err(|_| OperationError::Internal)?;
        if selections.len() >= LIMIT {
            return Err(OperationError::Capacity);
        }
        let id = random_id()?;
        selections.insert(
            id.clone(),
            Selection {
                kind,
                path: path.to_owned(),
                request,
                identities,
            },
        );
        Ok(id)
    }
    pub fn revoke_selection(&self, id: &str) -> Result<(), OperationError> {
        self.inner
            .selections
            .lock()
            .map_err(|_| OperationError::Internal)?
            .remove(id);
        Ok(())
    }
    pub fn request_preview(&self, id: &str) -> Result<RequestPreview, OperationError> {
        let value = self.selection(id, SelectionKind::Request)?;
        let request = value.request.ok_or(OperationError::Internal)?;
        Ok(RequestPreview {
            request_id: request.request_id().to_owned(),
            fingerprint: request.fingerprint().to_owned(),
        })
    }
    fn selection(&self, id: &str, kind: SelectionKind) -> Result<Selection, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::InvalidSelection);
        }
        let selection = self
            .inner
            .selections
            .lock()
            .map_err(|_| OperationError::Internal)?
            .get(id)
            .filter(|value| value.kind == kind)
            .cloned()
            .ok_or(OperationError::InvalidSelection)?;
        validate_identity(&selection)?;
        Ok(selection)
    }
    pub fn request(&self, input: RequestInput) -> Result<OperationStatus, OperationError> {
        let destination = self.selection(&input.destination, SelectionKind::RequestDestination)?;
        self.submit(
            Work::Request(destination.path.clone()),
            Some(&input.destination),
            vec![destination],
        )
    }
    pub fn export(&self, input: ExportInput) -> Result<OperationStatus, OperationError> {
        let book = self.selection(&input.book, SelectionKind::Book)?;
        let request_selection = self.selection(&input.request, SelectionKind::Request)?;
        let request = request_selection
            .request
            .clone()
            .ok_or(OperationError::Internal)?;
        if input.confirmed_request_id != request.request_id()
            || input.confirmed_fingerprint != request.fingerprint()
        {
            return Err(OperationError::ConfirmationRequired);
        }
        let destination = self.selection(&input.destination, SelectionKind::ExportDestination)?;
        self.submit(
            Work::Export(book.path.clone(), request, destination.path.clone()),
            Some(&input.destination),
            vec![book, request_selection, destination],
        )
    }
    pub fn import(&self, input: ImportInput) -> Result<OperationStatus, OperationError> {
        let package = self.selection(&input.package, SelectionKind::Package)?;
        let request_selection = self.selection(&input.request, SelectionKind::Request)?;
        let request = request_selection
            .request
            .clone()
            .ok_or(OperationError::Internal)?;
        let destination = self.selection(&input.destination, SelectionKind::BookDestination)?;
        self.submit(
            Work::Import(package.path.clone(), request, destination.path.clone()),
            Some(&input.destination),
            vec![package, request_selection, destination],
        )
    }
    pub fn restore(&self, input: RestoreInput) -> Result<OperationStatus, OperationError> {
        let backup = self.selection(&input.backup, SelectionKind::Backup)?;
        let destination = self.selection(&input.destination, SelectionKind::BookDestination)?;
        self.submit(
            Work::Restore(backup.path.clone(), destination.path.clone()),
            Some(&input.destination),
            vec![backup, destination],
        )
    }
    pub fn recover(&self, input: RecoveryInput) -> Result<OperationStatus, OperationError> {
        let (work, selected) = match input {
            RecoveryInput::Imports { parent } => {
                let parent = self.selection(&parent, SelectionKind::RecoveryParent)?;
                (Work::RecoverImports(parent.path.clone()), vec![parent])
            }
            RecoveryInput::Exports { book, parent } => {
                let book = self.selection(&book, SelectionKind::Book)?;
                let parent = self.selection(&parent, SelectionKind::RecoveryParent)?;
                (
                    Work::RecoverExports(book.path.clone(), parent.path.clone()),
                    vec![book, parent],
                )
            }
        };
        self.submit(work, None, selected)
    }

    pub fn status(&self, id: &str) -> Result<OperationStatus, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::NotFound);
        }
        self.inner
            .jobs
            .lock()
            .map_err(|_| OperationError::Internal)?
            .get(id)
            .cloned()
            .ok_or(OperationError::NotFound)
    }
    /// Own the session permit across upload extraction and final publication, even
    /// if the initiating HTTP future is dropped after sending its final bytes.
    pub(crate) fn begin_container_import<R: std::io::Read + Send + 'static>(
        &self,
        request: &str,
        destination: &str,
        reader: R,
    ) -> Result<tokio::task::JoinHandle<Result<OperationResult, OperationError>>, OperationError>
    {
        let request_selection = self.selection(request, SelectionKind::Request)?;
        let target = self.selection(destination, SelectionKind::BookDestination)?;
        let request = request_selection
            .request
            .clone()
            .ok_or(OperationError::InvalidSelection)?;
        let permit = self
            .inner
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| OperationError::Busy)?;
        let mut selections = self
            .inner
            .selections
            .lock()
            .map_err(|_| OperationError::Internal)?;
        if selections.remove(destination).is_none() {
            return Err(OperationError::InvalidSelection);
        }
        drop(selections);
        let inner = self.inner.clone();
        Ok(tokio::spawn(async move {
            let _permit = permit;
            validate_identity(&request_selection)?;
            validate_identity(&target)?;
            let parent = target
                .path
                .parent()
                .ok_or(OperationError::InvalidSelection)?
                .to_owned();
            let value = crate::container_import::import_recipient_container(
                reader,
                parent,
                target.path,
                request,
                Arc::new(inner.recipient.clone()),
                Arc::new(inner.device.clone()),
                inner.container_limits,
            )
            .await
            .map_err(|_| OperationError::InvalidSelection)?;
            Ok(OperationResult {
                package_id: Some(value.package_id),
                attachment_count: Some(value.attachment_count.to_string()),
                cleanup_warning: value.cleanup_warning.is_some(),
                selection_required: true,
                ..Default::default()
            })
        }))
    }
    pub(crate) fn acquire_download(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, OperationError> {
        self.inner
            .download_permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| OperationError::Busy)
    }
    pub(crate) fn download_artifact(&self, id: &str) -> Result<DownloadArtifact, OperationError> {
        if self.status(id)?.state != "succeeded" {
            return Err(OperationError::NotFound);
        }
        self.inner
            .downloads
            .lock()
            .map_err(|_| OperationError::Internal)?
            .get(id)
            .cloned()
            .ok_or(OperationError::NotFound)
    }
    /// Native owner may release completed statuses after acknowledged UI delivery.
    /// Running jobs cannot be forgotten or cancelled by dropping their HTTP future.
    pub fn forget_completed(&self, id: &str) -> Result<(), OperationError> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| OperationError::Internal)?;
        if jobs.get(id).is_some_and(|job| job.state == "running") {
            return Err(OperationError::Busy);
        }
        jobs.remove(id);
        self.inner
            .downloads
            .lock()
            .map_err(|_| OperationError::Internal)?
            .remove(id);
        Ok(())
    }
    fn submit(
        &self,
        work: Work,
        destination: Option<&str>,
        selected: Vec<Selection>,
    ) -> Result<OperationStatus, OperationError> {
        for selection in &selected {
            validate_identity(selection)?;
        }
        let permit = self
            .inner
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| OperationError::Busy)?;
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| OperationError::Internal)?;
        if jobs.len() >= LIMIT {
            return Err(OperationError::Capacity);
        }
        let id = random_id()?;
        let status = OperationStatus {
            operation_id: id.clone(),
            state: "running",
            result: None,
            error_code: None,
        };
        // Consume destination atomically with admission; retries require a fresh
        // native selection and can never accidentally overwrite an existing book.
        if let Some(token) = destination
            && self
                .inner
                .selections
                .lock()
                .map_err(|_| OperationError::Internal)?
                .remove(token)
                .is_none()
        {
            return Err(OperationError::InvalidSelection);
        }
        jobs.insert(id.clone(), status.clone());
        drop(jobs);
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let device = inner.device.clone();
            let recipient = inner.recipient.clone();
            let active_book = inner.active_book.clone();
            // The supervisor owns the permit until the worker exits even on panic.
            let result = tokio::spawn(async move {
                for selection in &selected {
                    validate_identity(selection)
                        .map_err(|_| commands::CommandError::UnsafeDestination)?;
                }
                execute(work, &device, &recipient, active_book).await
            })
            .await;
            let status = match result {
                Ok(Ok((result, artifact))) => {
                    if let Some(artifact) = artifact
                        && let Ok(mut downloads) = inner.downloads.lock()
                    {
                        downloads.insert(id.clone(), artifact);
                    }
                    OperationStatus {
                        operation_id: id.clone(),
                        state: "succeeded",
                        result: Some(result),
                        error_code: None,
                    }
                }
                _ => OperationStatus {
                    operation_id: id.clone(),
                    state: "failed",
                    result: None,
                    error_code: Some("storage_operation_failed"),
                },
            };
            if let Ok(mut jobs) = inner.jobs.lock() {
                jobs.insert(id, status);
            }
            drop(permit);
        });
        Ok(status)
    }
}
async fn execute(
    work: Work,
    device: &Device,
    recipient: &Recipient,
    active_book: Option<Arc<crate::runtime::RuntimeBook>>,
) -> Result<(OperationResult, Option<DownloadArtifact>), commands::CommandError> {
    let mut result = OperationResult::default();
    let mut artifact = None;
    match work {
        Work::Request(destination) => {
            let value = commands::write_recipient_request(destination, recipient)?;
            result.request_id = Some(value.request_id);
            result.fingerprint = Some(value.fingerprint);
        }
        Work::Export(book, request, destination) => {
            let value =
                if let Some(active) = active_book.as_ref().filter(|active| active.root() == book) {
                    crate::recipient_package::export_recipient_package(
                        active.pool(),
                        active.attachments(),
                        &active.key(),
                        &request,
                        destination,
                    )
                    .await?
                } else {
                    commands::export_recipient_book(book, &request, destination, device).await?
                };
            artifact = Some(DownloadArtifact {
                identities: capture_identities(SelectionKind::Package, &value.path)
                    .map_err(|_| commands::CommandError::UnsafeDestination)?,
                root: value.path.clone(),
                entries: value.verified_download_entries,
            });
            result.package_id = Some(value.package_id);
            result.attachment_count = Some(value.attachment_count.to_string());
            result.attachment_bytes = Some(value.attachment_bytes.to_string());
            result.cleanup_warning = value.cleanup_warning.is_some();
        }
        Work::Import(package, request, destination) => {
            let value =
                commands::import_recipient_book(package, &request, destination, recipient, device)
                    .await?;
            result.package_id = Some(value.package_id);
            result.attachment_count = Some(value.attachment_count.to_string());
            result.selection_required = true;
            result.cleanup_warning = value.cleanup_warning.is_some();
        }
        Work::Restore(backup, destination) => {
            let value = commands::restore_book(backup, destination, device).await?;
            result.attachment_count = Some(value.attachment_count.to_string());
            result.attachment_bytes = Some(value.attachment_bytes.to_string());
            result.cleanup_warning = value.cleanup_warning.is_some();
            result.selection_required = true;
        }
        Work::RecoverImports(parent) => {
            let value =
                crate::recipient_import::recover_interrupted_recipient_imports(parent, device)
                    .await?;
            result.removed_count = Some(value.removed_count.to_string());
            result.verified_count = Some(value.verified_destinations.len().to_string());
            result.active_count = Some(value.active_work.len().to_string());
            result.unrecognized_count = Some(value.unrecognized_work.len().to_string());
        }
        Work::RecoverExports(book, parent) => {
            let value =
                if let Some(active) = active_book.as_ref().filter(|active| active.root() == book) {
                    crate::recipient_package::recover_interrupted_recipient_exports(
                        parent,
                        &active.key(),
                    )
                    .await?
                } else {
                    commands::recover_recipient_exports(book, parent, device).await?
                };
            result.removed_count = Some(value.removed_count.to_string());
            result.verified_count = Some(value.verified_destinations.len().to_string());
            result.active_count = Some(value.active_work.len().to_string());
            result.unrecognized_count = Some(value.unrecognized_work.len().to_string());
        }
    }
    Ok((result, artifact))
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn random_id() -> Result<String, OperationError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| OperationError::Internal)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn validate_selection(kind: SelectionKind, path: &Path) -> Result<(), OperationError> {
    // Reject relative/parent paths and every existing symlink component. Native
    // selections still undergo the audited command's validation at execution.
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(OperationError::InvalidSelection);
    }
    let destination = matches!(
        kind,
        SelectionKind::RequestDestination
            | SelectionKind::ExportDestination
            | SelectionKind::BookDestination
    );
    let check = if destination {
        path.parent().ok_or(OperationError::InvalidSelection)?
    } else {
        path
    };
    let mut prefix = PathBuf::new();
    for part in check.components() {
        prefix.push(part);
        let metadata =
            std::fs::symlink_metadata(&prefix).map_err(|_| OperationError::InvalidSelection)?;
        if metadata.file_type().is_symlink() {
            return Err(OperationError::InvalidSelection);
        }
    }
    if destination && std::fs::symlink_metadata(path).is_ok() {
        return Err(OperationError::InvalidSelection);
    }
    Ok(())
}

// A capability binds the entire existing path chain, including mount points.
// Requests retain immutable parsed content, so their leaf may be replaced safely.
fn capture_identities(
    kind: SelectionKind,
    path: &Path,
) -> Result<Vec<(PathBuf, std::fs::Metadata)>, OperationError> {
    let omit_leaf = matches!(
        kind,
        SelectionKind::Request
            | SelectionKind::RequestDestination
            | SelectionKind::ExportDestination
            | SelectionKind::BookDestination
    );
    let check = if omit_leaf {
        path.parent().ok_or(OperationError::InvalidSelection)?
    } else {
        path
    };
    let mut prefix = PathBuf::new();
    let mut identities = Vec::new();
    for component in check.components() {
        prefix.push(component);
        let metadata =
            std::fs::symlink_metadata(&prefix).map_err(|_| OperationError::InvalidSelection)?;
        if metadata.file_type().is_symlink() {
            return Err(OperationError::InvalidSelection);
        }
        identities.push((prefix.clone(), metadata));
    }
    Ok(identities)
}
fn validate_identity(selection: &Selection) -> Result<(), OperationError> {
    for (path, expected) in &selection.identities {
        let actual =
            std::fs::symlink_metadata(path).map_err(|_| OperationError::InvalidSelection)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if actual.dev() != expected.dev()
                || actual.ino() != expected.ino()
                || actual.file_type() != expected.file_type()
            {
                return Err(OperationError::InvalidSelection);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (actual, expected);
            return Err(OperationError::InvalidSelection);
        }
    }
    // The immutable request document deliberately does not need a live leaf.
    if matches!(
        selection.kind,
        SelectionKind::Book
            | SelectionKind::Package
            | SelectionKind::Backup
            | SelectionKind::RecoveryParent
            | SelectionKind::RequestDestination
            | SelectionKind::ExportDestination
            | SelectionKind::BookDestination
    ) {
        validate_selection(selection.kind, &selection.path)?;
    }
    Ok(())
}
