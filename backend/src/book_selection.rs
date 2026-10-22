//! Explicit in-memory selection. Discovery never opens SQLite or reads keys.
use crate::{
    keys::{DeviceKeyId, DeviceKeyStore},
    runtime::{RuntimeBook, RuntimeError},
    storage::open_regular_file_without_symlink,
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BookCandidate {
    /// Exact direct-child name; not a number, display name or financial metadata.
    pub id: String,
    pub status: &'static str,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicBinding {
    format: String,
    version: u32,
    key_id: String,
}

fn private_path(path: &Path, directory: bool) -> Result<PathBuf, RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::UnsafePath);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err(RuntimeError::UnsafePath);
        }
        current.push(component);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(RuntimeError::UnsafePath);
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(RuntimeError::UnsafePath);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RuntimeError::UnsafePath);
    }
    let canonical = path.canonicalize()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RuntimeError::UnsafePath)?;
    if canonical.starts_with(source) || source.starts_with(&canonical) {
        return Err(RuntimeError::UnsafePath);
    }
    Ok(canonical)
}

/// Inspects only direct children of the explicitly supplied private collection.
/// Invalid, inaccessible, staging and symlink entries are excluded. A candidate
/// is not a promise that keys, schema or lock will be available on selection.
pub fn discover_books(parent: impl AsRef<Path>) -> Result<Vec<BookCandidate>, RuntimeError> {
    let parent = private_path(parent.as_ref(), true)?;
    let mut books = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if id.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if inspect_candidate(&path).is_err() {
            continue;
        }
        books.push(BookCandidate {
            id,
            status: "selection_required",
        });
    }
    books.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(books)
}
fn inspect_candidate(path: &Path) -> Result<(), RuntimeError> {
    private_path(path, true)?;
    private_path(&path.join("attachments"), true)?;
    private_path(&path.join("book.sqlite3"), false)?;
    let binding = path.join("device-key.json");
    private_path(&binding, false)?;
    let mut bytes = Vec::new();
    open_regular_file_without_symlink(&binding)?
        .take(4097)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(RuntimeError::InvalidBinding);
    }
    let binding: PublicBinding =
        serde_json::from_slice(&bytes).map_err(|_| RuntimeError::InvalidBinding)?;
    if binding.format != "kAccounting-Extra-01.local-backup"
        || binding.version != 1
        || DeviceKeyId::parse(&binding.key_id).is_err()
    {
        return Err(RuntimeError::InvalidBinding);
    }
    Ok(())
}

/// A fresh session always requires an explicit choice, even if exactly one book
/// exists. Successful import/restore does not change this object.
#[derive(Default)]
pub struct BookSelectionSession {
    current: Option<RuntimeBook>,
}
impl BookSelectionSession {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn current(&self) -> Option<&RuntimeBook> {
        self.current.as_ref()
    }
    /// Keep the old book and its exclusive lock until the replacement opens and
    /// verifies successfully. All failures leave the old selection untouched.
    pub async fn select(
        &mut self,
        path: impl AsRef<Path>,
        keys: &impl DeviceKeyStore,
    ) -> Result<(), RuntimeError> {
        let path = private_path(path.as_ref(), true)?;
        if self
            .current
            .as_ref()
            .is_some_and(|book| book.root() == path)
        {
            return Ok(());
        }
        let replacement = RuntimeBook::open_existing(path, keys).await?;
        if let Some(previous) = self.current.replace(replacement) {
            previous.close().await;
        }
        Ok(())
    }
    pub async fn close(&mut self) {
        if let Some(previous) = self.current.take() {
            previous.close().await;
        }
    }
}
