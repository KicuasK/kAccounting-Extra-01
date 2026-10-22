//! Ciphertext-only removable-media staging. No private-mode semantics are assumed.
use crate::storage::{
    ensure_directory_still_matches, ensure_path_still_names_open_file,
    open_regular_file_without_symlink,
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs::{File, Metadata, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// Binds recovery to the directory/volume originally chosen by the operator.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaIdentity {
    device: u64,
    directory: u64,
}
impl MediaIdentity {
    pub fn capture(parent: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("unsafe media parent"));
        }
        #[cfg(unix)]
        {
            Ok(Self {
                device: metadata.dev(),
                directory: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::other(
                "media identity unsupported on this platform",
            ))
        }
    }
    pub fn validate(&self, parent: &Path) -> io::Result<()> {
        if *self != Self::capture(parent)? {
            return Err(io::Error::other("media parent identity changed"));
        }
        Ok(())
    }
}

/// This directory contains only encrypted objects and the public recipient envelope.
/// Failure deliberately preserves it: recovery never recursively deletes removable media.
pub struct CiphertextStage {
    path: PathBuf,
    identity: Metadata,
    parent: PathBuf,
    media: MediaIdentity,
}
impl CiphertextStage {
    pub fn create(parent: &Path, name: &str, media: &MediaIdentity) -> io::Result<Self> {
        if !name.starts_with(".kAccounting-Extra-01-ciphertext-")
            || !name.ends_with(".partial")
            || name.contains('/')
            || name.contains('\\')
        {
            return Err(io::Error::other("invalid ciphertext staging name"));
        }
        media.validate(parent)?;
        let path = parent.join(name);
        std::fs::create_dir(&path)?;
        let identity = std::fs::symlink_metadata(&path)?;
        let stage = Self {
            path,
            identity,
            parent: parent.to_path_buf(),
            media: media.clone(),
        };
        stage.validate()?;
        Ok(stage)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn validate(&self) -> io::Result<()> {
        self.media.validate(&self.parent)?;
        ensure_directory_still_matches(&self.path, &self.identity)
    }
    pub fn copy_package(&self, source: &Path, attachment_names: &[String]) -> io::Result<()> {
        self.validate()?;
        std::fs::create_dir(self.path.join("attachments"))?;
        for name in ["envelope.json", "manifest.kae", "book.kae"] {
            self.copy_file(&source.join(name), &self.path.join(name))?;
        }
        for name in attachment_names {
            if name.contains('/') || name.contains('\\') || !name.ends_with(".kae") {
                return Err(io::Error::other("invalid encrypted object name"));
            }
            self.copy_file(
                &source.join("attachments").join(name),
                &self.path.join("attachments").join(name),
            )?;
        }
        File::open(self.path.join("attachments"))?.sync_all()?;
        File::open(&self.path)?.sync_all()?;
        File::open(&self.parent)?.sync_all()?;
        self.validate()
    }
    fn copy_file(&self, source: &Path, destination: &Path) -> io::Result<()> {
        self.validate()?;
        let mut input = open_regular_file_without_symlink(source)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut output = options.open(destination)?;
        // std::io::copy uses bounded buffering; never materialize attachment plaintext.
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        ensure_path_still_names_open_file(source, &input)?;
        ensure_path_still_names_open_file(destination, &output)?;
        self.validate()
    }
}
