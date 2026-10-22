//! Bounded single-file transport followed by the complete recipient importer.
//! Transport hashes alone never authorize a book publication.
use crate::{
    keys::DeviceKeyStore,
    recipient_import::{RecipientImportError, RecipientImportOutcome, import_recipient_package},
    recipient_transfer::{RecipientKeyStore, RecipientRequest},
    storage::ensure_directory_still_matches,
    transfer_container::{ContainerError, ContainerLimits, extract_container},
};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContainerImportError {
    #[error("container import requires a private canonical work parent and new destination")]
    UnsafeDestination,
    #[error("container worker failed; retain staging for inspection")]
    Worker,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Container(#[from] ContainerError),
    #[error(transparent)]
    Import(#[from] RecipientImportError),
}
fn private_parent(path: &Path) -> Result<fs::Metadata, ContainerImportError> {
    let m = fs::symlink_metadata(path)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(ContainerImportError::UnsafeDestination)?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.permissions().mode() & 0o077 != 0
        || path.canonicalize()? != path
        || path.starts_with(source)
        || source.starts_with(path)
    {
        return Err(ContainerImportError::UnsafeDestination);
    }
    Ok(m)
}
/// Reads at most 64 KiB per operation; callers must also bound their HTTP bridge.
/// All staging is encrypted apart from the public recipient envelope. It remains
/// in the private work parent on failure; verified success clears only the
/// recorded internal copy. The original input file is never modified.
/// Dropping this future detaches an OWNED worker: the reader and staging remain
/// alive until extraction and any already-started import finish. Disconnection
/// does not promise cancellation or rollback of a successfully published book.
/// No existing destination is overwritten and no active-book selection changes.
pub async fn import_recipient_container<R, K, D>(
    input: R,
    private_work_parent: PathBuf,
    new_destination: PathBuf,
    request: RecipientRequest,
    recipient_keys: Arc<K>,
    device_keys: Arc<D>,
    limits: ContainerLimits,
) -> Result<RecipientImportOutcome, ContainerImportError>
where
    R: Read + Send + 'static,
    K: RecipientKeyStore + Send + Sync + 'static,
    D: DeviceKeyStore + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let parent_identity = private_parent(&private_work_parent)?;
        private_parent(
            new_destination
                .parent()
                .ok_or(ContainerImportError::UnsafeDestination)?,
        )?;
        if fs::symlink_metadata(&new_destination).is_ok() {
            return Err(ContainerImportError::UnsafeDestination);
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| ContainerImportError::Worker)?;
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let work =
            private_work_parent.join(format!(".kAccounting-Extra-01-container-import-{name}"));
        ensure_directory_still_matches(&private_work_parent, &parent_identity)?;
        fs::DirBuilder::new().mode(0o700).create(&work)?;
        let work_identity = fs::symlink_metadata(&work)?;
        let stage = work.join("package");
        let extracted = stage.clone();
        let entries =
            tokio::task::spawn_blocking(move || extract_container(input, &extracted, limits))
                .await
                .map_err(|_| ContainerImportError::Worker)??;
        ensure_directory_still_matches(&private_work_parent, &parent_identity)?;
        ensure_directory_still_matches(&work, &work_identity)?;
        let cleanup = Cleanup::capture(&private_work_parent, &work, &stage, entries)?;
        let mut outcome = import_recipient_package(
            &stage,
            new_destination,
            &request,
            recipient_keys.as_ref(),
            device_keys.as_ref(),
        )
        .await?;
        if let Err(error) = cleanup.remove() {
            outcome.cleanup_warning = Some(format!(
                "internal encrypted container staging retained: {error}"
            ));
        }
        Ok(outcome)
    })
    .await
    .map_err(|_| ContainerImportError::Worker)?
}

// This authority is memory-only, built by the live owned worker. A path or
// similarly named on-disk folder is never sufficient to adopt cleanup work.
struct Cleanup {
    parent: (PathBuf, fs::Metadata),
    work: (PathBuf, fs::Metadata),
    stage: (PathBuf, fs::Metadata),
    attachments: (PathBuf, fs::Metadata),
    files: Vec<(PathBuf, fs::Metadata)>,
}
impl Cleanup {
    fn capture(
        parent: &Path,
        work: &Path,
        stage: &Path,
        entries: Vec<crate::transfer_container::ContainerEntry>,
    ) -> std::io::Result<Self> {
        let files = entries
            .into_iter()
            .map(|entry| {
                let path = stage.join(entry.name());
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(std::io::Error::other("unsafe container object"));
                }
                Ok((path, metadata))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        Ok(Self {
            parent: (parent.into(), fs::symlink_metadata(parent)?),
            work: (work.into(), fs::symlink_metadata(work)?),
            stage: (stage.into(), fs::symlink_metadata(stage)?),
            attachments: (
                stage.join("attachments"),
                fs::symlink_metadata(stage.join("attachments"))?,
            ),
            files,
        })
    }
    fn check_dirs(&self) -> std::io::Result<()> {
        for (path, metadata) in [&self.parent, &self.work, &self.stage, &self.attachments] {
            ensure_directory_still_matches(path, metadata)?;
        }
        Ok(())
    }
    fn check_file(path: &Path, original: &fs::Metadata) -> std::io::Result<()> {
        let current = fs::symlink_metadata(path)?;
        if !current.is_file()
            || current.file_type().is_symlink()
            || current.dev() != original.dev()
            || current.ino() != original.ino()
            || current.len() != original.len()
        {
            return Err(std::io::Error::other("container cleanup identity changed"));
        }
        Ok(())
    }
    fn remove(self) -> std::io::Result<()> {
        self.check_dirs()?;
        // Reject unexpected entries before deleting anything. No recursive
        // removal and no directory traversal through symlinks is used.
        let expected: std::collections::BTreeSet<_> =
            self.files.iter().map(|(path, _)| path).collect();
        for directory in [&self.work.0, &self.stage.0, &self.attachments.0] {
            for item in fs::read_dir(directory)? {
                let path = item?.path();
                if path != self.stage.0 && path != self.attachments.0 && !expected.contains(&path) {
                    return Err(std::io::Error::other("unknown container staging entry"));
                }
            }
        }
        for (path, original) in &self.files {
            Self::check_file(path, original)?;
        }
        for (path, original) in &self.files {
            self.check_dirs()?;
            Self::check_file(path, original)?;
            fs::remove_file(path)?;
        }
        self.check_dirs()?;
        fs::remove_dir(&self.attachments.0)?;
        ensure_directory_still_matches(&self.parent.0, &self.parent.1)?;
        ensure_directory_still_matches(&self.work.0, &self.work.1)?;
        ensure_directory_still_matches(&self.stage.0, &self.stage.1)?;
        fs::remove_dir(&self.stage.0)?;
        ensure_directory_still_matches(&self.parent.0, &self.parent.1)?;
        ensure_directory_still_matches(&self.work.0, &self.work.1)?;
        fs::remove_dir(&self.work.0)?;
        fs::File::open(&self.parent.0)?.sync_all()
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    fn setup() -> (PathBuf, Cleanup) {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "ka-container-cleanup-{:032x}",
            u128::from_be_bytes(nonce)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let work = root.join("work");
        let stage = work.join("package");
        fs::create_dir(&work).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::create_dir(stage.join("attachments")).unwrap();
        fs::write(stage.join("book.kae"), b"ciphertext").unwrap();
        let entry = crate::transfer_container::ContainerEntry::new(
            "book.kae",
            crate::storage::describe_file(stage.join("book.kae")).unwrap(),
        )
        .unwrap();
        let cleanup = Cleanup::capture(&root, &work, &stage, vec![entry]).unwrap();
        (root, cleanup)
    }
    #[test]
    fn known_internal_copy_is_removed() {
        let (root, cleanup) = setup();
        cleanup.remove().unwrap();
        assert!(!root.join("work").exists());
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn unknown_files_preserve_stage_before_any_removal() {
        let (root, cleanup) = setup();
        fs::write(root.join("work/package/unknown"), b"keep").unwrap();
        assert!(cleanup.remove().is_err());
        assert_eq!(
            fs::read(root.join("work/package/book.kae")).unwrap(),
            b"ciphertext"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unknown_work_and_attachment_entries_preserve_all_objects() {
        for relative in ["work/unknown", "work/package/attachments/unknown"] {
            let (root, cleanup) = setup();
            fs::write(root.join(relative), b"keep").unwrap();
            assert!(cleanup.remove().is_err());
            assert_eq!(
                fs::read(root.join("work/package/book.kae")).unwrap(),
                b"ciphertext"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn symlink_file_and_directory_replacements_preserve_targets() {
        use std::os::unix::fs::symlink;
        let (root, cleanup) = setup();
        let object = root.join("work/package/book.kae");
        fs::rename(&object, root.join("original")).unwrap();
        symlink(root.join("original"), &object).unwrap();
        assert!(cleanup.remove().is_err());
        assert_eq!(fs::read(root.join("original")).unwrap(), b"ciphertext");
        assert!(
            fs::symlink_metadata(&object)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
        let (root, cleanup) = setup();
        fs::rename(root.join("work"), root.join("original")).unwrap();
        symlink(root.join("original"), root.join("work")).unwrap();
        assert!(cleanup.remove().is_err());
        assert_eq!(
            fs::read(root.join("original/package/book.kae")).unwrap(),
            b"ciphertext"
        );
        assert!(
            fs::symlink_metadata(root.join("work"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replaced_object_and_replaced_work_are_not_removed() {
        let (root, cleanup) = setup();
        fs::rename(root.join("work/package/book.kae"), root.join("original")).unwrap();
        fs::write(root.join("work/package/book.kae"), b"keep").unwrap();
        assert!(cleanup.remove().is_err());
        assert_eq!(
            fs::read(root.join("work/package/book.kae")).unwrap(),
            b"keep"
        );
        fs::remove_dir_all(root).unwrap();
        let (root, cleanup) = setup();
        fs::rename(root.join("work"), root.join("original")).unwrap();
        fs::create_dir(root.join("work")).unwrap();
        fs::write(root.join("work/sentinel"), b"keep").unwrap();
        assert!(cleanup.remove().is_err());
        assert_eq!(fs::read(root.join("work/sentinel")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }
}
