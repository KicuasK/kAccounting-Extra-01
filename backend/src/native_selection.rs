//! Trusted native file-picker child bridge. No HTTP routes or raw-path input.
use crate::{
    session_runtime::SessionRuntime,
    storage::{StoredObjectDescriptor, describe_file, ensure_directory_still_matches},
    storage_operations::{OperationError, SelectionKind},
};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use thiserror::Error;
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
const FORMAT: &str = "kAccounting-Extra-01.native-selection.v1";
const RESPONSE_LIMIT: u64 = 16 * 1024;
const HELPER_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum NativeSelectionError {
    #[error("native selection helper identity or permissions are unsafe")]
    UnsafeHelper,
    #[error("native selection helper returned invalid or unbound output")]
    InvalidResponse,
    #[error("native selection worker failed")]
    Worker,
    #[error("another native selection panel is already active")]
    Busy,
    #[error("native selection I/O failed")]
    Io(#[from] std::io::Error),
    #[error("native selection registration failed: {0:?}")]
    Registration(OperationError),
}
impl From<OperationError> for NativeSelectionError {
    fn from(error: OperationError) -> Self {
        Self::Registration(error)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PickerKind {
    Book,
    Request,
    Package,
    Backup,
    RequestDestination,
    ExportDestination,
    BookDestination,
    RecoveryParent,
}
impl From<SelectionKind> for PickerKind {
    fn from(kind: SelectionKind) -> Self {
        match kind {
            SelectionKind::Book => Self::Book,
            SelectionKind::Request => Self::Request,
            SelectionKind::Package => Self::Package,
            SelectionKind::Backup => Self::Backup,
            SelectionKind::RequestDestination => Self::RequestDestination,
            SelectionKind::ExportDestination => Self::ExportDestination,
            SelectionKind::BookDestination => Self::BookDestination,
            SelectionKind::RecoveryParent => Self::RecoveryParent,
        }
    }
}
#[derive(Serialize)]
struct Request {
    format: &'static str,
    nonce: String,
    kind: PickerKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    format: String,
    nonce: String,
    kind: PickerKind,
    result: PickerResult,
}
#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum PickerResult {
    Selected { path: PathBuf },
    Cancelled {},
}

/// Pin an independently installed, trusted native helper. `expected_owner_uid`
/// comes from trusted native startup configuration, never from a web request.
/// The host must obtain user authorization before invoking an actual OS panel.
#[derive(Clone)]
pub struct NativePicker {
    executable: PathBuf,
    metadata: fs::Metadata,
    descriptor: StoredObjectDescriptor,
    parent: PathBuf,
    parent_identity: fs::Metadata,
    admission: Arc<tokio::sync::Semaphore>,
}
impl NativePicker {
    pub fn pin(
        executable: impl AsRef<Path>,
        expected_owner_uid: u32,
    ) -> Result<Self, NativeSelectionError> {
        let executable = executable.as_ref();
        let metadata = fs::symlink_metadata(executable)?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or(NativeSelectionError::UnsafeHelper)?;
        let parent = executable
            .parent()
            .ok_or(NativeSelectionError::UnsafeHelper)?;
        let parent_identity = fs::symlink_metadata(parent)?;
        if executable.canonicalize()? != executable
            || executable.starts_with(source)
            || !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != expected_owner_uid
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o100 == 0
            || metadata.len() > HELPER_LIMIT
            || !parent_identity.is_dir()
            || parent_identity.file_type().is_symlink()
            || parent_identity.mode() & 0o022 != 0
            || (parent_identity.uid() != expected_owner_uid && parent_identity.uid() != 0)
        {
            return Err(NativeSelectionError::UnsafeHelper);
        }
        let descriptor = describe_file(executable)?;
        Ok(Self {
            executable: executable.into(),
            metadata,
            descriptor,
            parent: parent.into(),
            parent_identity,
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }
    fn validate(&self) -> Result<(), NativeSelectionError> {
        ensure_directory_still_matches(&self.parent, &self.parent_identity)?;
        let parent = fs::symlink_metadata(&self.parent)?;
        if parent.uid() != self.parent_identity.uid()
            || parent.mode() != self.parent_identity.mode()
            || parent.mode() & 0o022 != 0
            || self.executable.canonicalize()? != self.executable
        {
            return Err(NativeSelectionError::UnsafeHelper);
        }
        let current = fs::symlink_metadata(&self.executable)?;
        if !current.is_file()
            || current.file_type().is_symlink()
            || current.dev() != self.metadata.dev()
            || current.ino() != self.metadata.ino()
            || current.uid() != self.metadata.uid()
            || current.mode() != self.metadata.mode()
            || current.len() != self.metadata.len()
            || describe_file(&self.executable)? != self.descriptor
        {
            return Err(NativeSelectionError::UnsafeHelper);
        }
        Ok(())
    }
    #[cfg(test)]
    fn choose(&self, kind: SelectionKind) -> Result<Option<PathBuf>, NativeSelectionError> {
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| NativeSelectionError::Busy)?;
        self.choose_cancellable(kind, Arc::new(AtomicBool::new(false)), permit)
    }
    fn choose_cancellable(
        &self,
        kind: SelectionKind,
        cancelled: Arc<AtomicBool>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Option<PathBuf>, NativeSelectionError> {
        self.validate()?;
        if cancelled.load(Ordering::Acquire) {
            return Err(NativeSelectionError::Worker);
        }
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| NativeSelectionError::Worker)?;
        let request = Request {
            format: FORMAT,
            nonce: random.iter().map(|b| format!("{b:02x}")).collect(),
            kind: kind.into(),
        };
        let encoded = serde_json::to_vec(&request).map_err(|_| NativeSelectionError::Worker)?;
        let mut child = Command::new(&self.executable)
            .env_clear()
            .current_dir(&self.parent)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let result = (|| {
            let mut stdin = child.stdin.take().ok_or(NativeSelectionError::Worker)?;
            stdin.write_all(&encoded)?;
            drop(stdin);
            let stdout = child.stdout.take().ok_or(NativeSelectionError::Worker)?;
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("native-picker-output".into())
                .spawn(move || {
                    let mut output = Vec::new();
                    let result = stdout
                        .take(RESPONSE_LIMIT + 1)
                        .read_to_end(&mut output)
                        .map(|_| output);
                    let _ = sender.send(result);
                })?;
            let interval = std::time::Duration::from_millis(50);
            let output = loop {
                if cancelled.load(Ordering::Acquire) {
                    return Err(NativeSelectionError::Worker);
                }
                match receiver.recv_timeout(interval) {
                    Ok(output) => break output?,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => return Err(NativeSelectionError::Worker),
                }
            };
            if output.len() as u64 > RESPONSE_LIMIT {
                return Err(NativeSelectionError::InvalidResponse);
            }
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return Err(NativeSelectionError::Worker);
                }
                if let Some(status) = child.try_wait()? {
                    if !status.success() {
                        return Err(NativeSelectionError::InvalidResponse);
                    }
                    break;
                }
                std::thread::sleep(interval);
            }
            self.validate()?;
            parse_response(&request, &output)
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }
    /// Wait until the owned helper worker has terminated and reaped its child.
    /// Call after dropping a pending selection future during orderly shutdown.
    pub async fn wait_idle(&self) -> Result<(), NativeSelectionError> {
        let _permit = self
            .admission
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| NativeSelectionError::Worker)?;
        Ok(())
    }
    /// Invoke only from trusted native UI action, never a web-supplied path.
    /// Dropping the future requests child termination; the worker kills and
    /// reaps it. A cancelled future never registers a returned selection.
    pub async fn select_and_register(
        &self,
        session: &SessionRuntime,
        kind: SelectionKind,
    ) -> Result<Option<String>, NativeSelectionError> {
        self.select_and_register_inner(session, kind, None).await
    }
    /// Native acceptance/host restriction: only this exact expected path may be
    /// registered. The expected path is trusted local configuration, not HTTP.
    pub async fn select_and_register_exact(
        &self,
        session: &SessionRuntime,
        kind: SelectionKind,
        expected: &Path,
    ) -> Result<Option<String>, NativeSelectionError> {
        self.select_and_register_inner(session, kind, Some(expected))
            .await
    }
    async fn select_and_register_inner(
        &self,
        session: &SessionRuntime,
        kind: SelectionKind,
        expected: Option<&Path>,
    ) -> Result<Option<String>, NativeSelectionError> {
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| NativeSelectionError::Busy)?;
        let selection_session = session.native_selection_session().await?;
        let picker = self.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancelled.clone());
        let selected =
            tokio::task::spawn_blocking(move || picker.choose_cancellable(kind, cancelled, permit))
                .await
                .map_err(|_| NativeSelectionError::Worker)??;
        match selected {
            None => Ok(None),
            Some(path) => {
                if expected.is_some_and(|expected| expected != path) {
                    return Err(NativeSelectionError::InvalidResponse);
                }
                Ok(Some(
                    session
                        .register_storage_selection_for(&selection_session, kind, path)
                        .await?,
                ))
            }
        }
    }
}
fn parse_response(
    request: &Request,
    bytes: &[u8],
) -> Result<Option<PathBuf>, NativeSelectionError> {
    if bytes.len() as u64 > RESPONSE_LIMIT {
        return Err(NativeSelectionError::InvalidResponse);
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| NativeSelectionError::InvalidResponse)?;
    if response.format != FORMAT || response.nonce != request.nonce || response.kind != request.kind
    {
        return Err(NativeSelectionError::InvalidResponse);
    }
    match response.result {
        PickerResult::Cancelled {} => Ok(None),
        PickerResult::Selected { path } => {
            if !path.is_absolute()
                || path.as_os_str().len() > 4096
                || path.as_os_str().is_empty()
                || path.components().any(|part| {
                    matches!(
                        part,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
                || path.as_os_str().as_encoded_bytes().contains(&0)
            {
                return Err(NativeSelectionError::InvalidResponse);
            }
            // Existing selections must already be canonical. For new paths,
            // only their existing parent is canonicalized; no path is created.
            let checked = if matches!(
                request.kind,
                PickerKind::RequestDestination
                    | PickerKind::ExportDestination
                    | PickerKind::BookDestination
            ) {
                path.parent().ok_or(NativeSelectionError::InvalidResponse)?
            } else {
                path.as_path()
            };
            if checked.canonicalize()? != checked {
                return Err(NativeSelectionError::InvalidResponse);
            }
            Ok(Some(path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
    }
    impl Fixture {
        fn new(body: &str) -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let root = std::env::temp_dir().join(format!(
                "ka-native-picker-{:032x}",
                u128::from_be_bytes(random)
            ));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let root = root.canonicalize().unwrap();
            let executable = root.join("fixture-helper");
            fs::write(
                &executable,
                format!("#!/usr/bin/python3\nimport json,sys\nr=json.load(sys.stdin)\n{body}\n"),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            Self { root, executable }
        }
        fn pin(&self) -> NativePicker {
            NativePicker::pin(
                &self.executable,
                fs::metadata(&self.executable).unwrap().uid(),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    fn cancelled() -> &'static str {
        "print(json.dumps(dict(r,result={'outcome':'cancelled'})))"
    }
    #[test]
    fn private_child_cancel_is_bound_to_nonce_and_kind() {
        let f = Fixture::new(cancelled());
        assert!(f.pin().choose(SelectionKind::Request).unwrap().is_none());
    }
    #[test]
    fn private_child_returns_canonical_selected_path_only() {
        let f = Fixture::new(
            "import os\nprint(json.dumps(dict(r,result={'outcome':'selected','path':os.getcwd()})))",
        );
        assert_eq!(
            f.pin().choose(SelectionKind::RecoveryParent).unwrap(),
            Some(f.root.clone())
        );
    }
    #[test]
    fn malformed_oversized_nonce_and_kind_mismatches_are_rejected() {
        for body in [
            "r['nonce']='0'*64\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
            "r['kind']='book'\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
            "r['extra']='forbidden'\nprint(json.dumps(dict(r,result={'outcome':'cancelled'})))",
            "print('x'*20000)",
            "print('{}')",
            "print(json.dumps(dict(r,result={'outcome':'cancelled','path':'/'})))",
        ] {
            let f = Fixture::new(body);
            assert!(
                f.pin().choose(SelectionKind::Request).is_err(),
                "accepted unexpected body: {body}"
            );
        }
    }
    #[test]
    fn helper_changes_and_unsafe_owner_permissions_are_rejected() {
        let f = Fixture::new(cancelled());
        let picker = f.pin();
        fs::write(&f.executable, b"changed in place").unwrap();
        assert!(picker.choose(SelectionKind::Book).is_err());
        let f = Fixture::new(cancelled());
        let owner = fs::metadata(&f.executable).unwrap().uid();
        assert!(NativePicker::pin(&f.executable, owner.wrapping_add(1)).is_err());
        fs::set_permissions(&f.executable, fs::Permissions::from_mode(0o722)).unwrap();
        assert!(NativePicker::pin(&f.executable, owner).is_err());
    }
    #[tokio::test]
    async fn orderly_shutdown_waits_for_owned_worker_permit() {
        let f = Fixture::new(cancelled());
        let picker = f.pin();
        let permit = picker.admission.clone().try_acquire_owned().unwrap();
        let waiting = picker.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            entered_tx.send(()).unwrap();
            waiting.wait_idle().await
        });
        entered_rx.await.unwrap();
        assert!(!waiter.is_finished());
        drop(permit);
        waiter.await.unwrap().unwrap();
    }
    #[test]
    fn pinned_parent_permission_changes_are_rejected() {
        let f = Fixture::new(cancelled());
        let picker = f.pin();
        fs::set_permissions(&f.root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(picker.choose(SelectionKind::Request).is_err());
        fs::set_permissions(&f.root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn cloned_pickers_share_single_worker_admission() {
        let f = Fixture::new(cancelled());
        let picker = f.pin();
        let cloned = picker.clone();
        let permit = picker.admission.clone().try_acquire_owned().unwrap();
        assert!(matches!(
            cloned.choose(SelectionKind::Request),
            Err(NativeSelectionError::Busy)
        ));
        drop(permit);
        assert!(cloned.choose(SelectionKind::Request).unwrap().is_none());
    }
    #[test]
    fn replaced_helper_or_parent_symlink_cannot_be_adopted() {
        let f = Fixture::new(cancelled());
        let picker = f.pin();
        fs::rename(&f.executable, f.root.join("original")).unwrap();
        fs::copy(f.root.join("original"), &f.executable).unwrap();
        assert!(picker.choose(SelectionKind::Book).is_err());
        fs::remove_file(&f.executable).unwrap();
        std::os::unix::fs::symlink(f.root.join("original"), &f.executable).unwrap();
        assert!(picker.choose(SelectionKind::Book).is_err());
    }
    #[test]
    fn raw_relative_and_parent_paths_are_never_registered() {
        for path in ["relative", "../book", "/private/tmp/../book"] {
            let body = format!(
                "print(json.dumps(dict(r,result={{'outcome':'selected','path':{path:?}}})))"
            );
            let f = Fixture::new(&body);
            assert!(f.pin().choose(SelectionKind::Book).is_err());
        }
    }
    #[test]
    fn cancelled_native_action_terminates_and_reaps_its_child() {
        let f = Fixture::new(
            "import os,time\nopen('started','w').write(str(os.getpid()))\ntime.sleep(30)",
        );
        let picker = f.pin();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancellation.clone();
        let watcher = picker.clone();
        let permit = picker.admission.clone().try_acquire_owned().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = done_tx.send(picker.choose_cancellable(
                SelectionKind::Request,
                worker_cancel,
                permit,
            ));
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !f.root.join("started").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(f.root.join("started").exists());
        let pid = fs::read_to_string(f.root.join("started")).unwrap();
        assert!(matches!(
            watcher.choose(SelectionKind::Request),
            Err(NativeSelectionError::Busy)
        ));
        cancellation.store(true, Ordering::Release);
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
        worker.join().unwrap();
        assert_eq!(watcher.admission.available_permits(), 1);
        assert!(
            !Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn failed_child_cannot_authorize_a_selection() {
        let f =
            Fixture::new("print(json.dumps(dict(r,result={'outcome':'cancelled'})))\nsys.exit(2)");
        assert!(f.pin().choose(SelectionKind::Request).is_err());
    }
}
