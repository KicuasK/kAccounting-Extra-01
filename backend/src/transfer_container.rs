//! Single-file transport framing for recipient-encrypted packages.
//! Parsing verifies framing and transport hashes only. Recipient authentication,
//! database validation and attachment-reference verification MUST precede import.
use crate::{
    attachments::AttachmentId,
    storage::{
        ContentDigest, StoredObjectDescriptor, ensure_directory_still_matches,
        ensure_path_still_names_open_file, open_regular_file_without_symlink,
    },
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};
use thiserror::Error;

pub const CONTAINER_MAGIC: &[u8; 12] = b"KAXFER01\r\n\x1a\n";
pub const COPY_BUFFER_BYTES: usize = 64 * 1024;
#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("transfer container I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid, unsupported, or incomplete transfer container")]
    Invalid,
    #[error("transfer container exceeds configured capacity")]
    Capacity,
    #[error("transfer container object differs from its expected descriptor")]
    Integrity,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerEntry {
    name: String,
    descriptor: StoredObjectDescriptor,
}
impl ContainerEntry {
    pub fn new(
        name: impl Into<String>,
        descriptor: StoredObjectDescriptor,
    ) -> Result<Self, ContainerError> {
        let name = name.into();
        validate_name(&name)?;
        Ok(Self { name, descriptor })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn descriptor(&self) -> StoredObjectDescriptor {
        self.descriptor
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ContainerLimits {
    pub max_entries: u32,
    pub max_object_bytes: u64,
    pub max_total_bytes: u64,
}
impl Default for ContainerLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_003,
            max_object_bytes: 1024 * 1024 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024 * 1024,
        }
    }
}
fn validate_name(name: &str) -> Result<(), ContainerError> {
    if ["envelope.json", "manifest.kae", "book.kae"].contains(&name) {
        return Ok(());
    }
    let id = name
        .strip_prefix("attachments/")
        .and_then(|s| s.strip_suffix(".kae"))
        .ok_or(ContainerError::Invalid)?;
    AttachmentId::parse(id).map_err(|_| ContainerError::Invalid)?;
    Ok(())
}
fn complete(names: &BTreeSet<String>) -> Result<(), ContainerError> {
    if ["envelope.json", "manifest.kae", "book.kae"]
        .iter()
        .all(|s| names.contains(*s))
    {
        Ok(())
    } else {
        Err(ContainerError::Invalid)
    }
}
fn directory(path: &Path) -> Result<fs::Metadata, ContainerError> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(ContainerError::Invalid);
    }
    Ok(m)
}
/// Caller supplies descriptors obtained from its verified package context; no
/// directory walk or plaintext-file selection is performed. Output is tentative
/// until this function succeeds; discard incomplete downloads on any error.
pub fn write_container(
    root: &Path,
    entries: &[ContainerEntry],
    mut output: impl Write,
) -> Result<u64, ContainerError> {
    let root_meta = directory(root)?;
    let count = u32::try_from(entries.len()).map_err(|_| ContainerError::Capacity)?;
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut names = BTreeSet::new();
    let mut total = 16u64;
    for e in &sorted {
        validate_name(&e.name)?;
        if !names.insert(e.name.clone()) {
            return Err(ContainerError::Invalid);
        }
        total = total
            .checked_add(2 + e.name.len() as u64 + 8 + 32)
            .and_then(|n| n.checked_add(e.descriptor.byte_length()))
            .ok_or(ContainerError::Capacity)?;
    }
    complete(&names)?;
    output.write_all(CONTAINER_MAGIC)?;
    output.write_all(&count.to_be_bytes())?;
    for e in sorted {
        ensure_directory_still_matches(root, &root_meta)?;
        let parent = if e.name.starts_with("attachments/") {
            root.join("attachments")
        } else {
            root.to_path_buf()
        };
        let parent_meta = directory(&parent)?;
        let path = root.join(&e.name);
        let mut input = open_regular_file_without_symlink(&path)?;
        if input.metadata()?.len() != e.descriptor.byte_length() {
            return Err(ContainerError::Integrity);
        }
        output.write_all(&(e.name.len() as u16).to_be_bytes())?;
        output.write_all(e.name.as_bytes())?;
        output.write_all(&e.descriptor.byte_length().to_be_bytes())?;
        let digest_hex = e.descriptor.digest().to_hex();
        let digest = hex_digest(&digest_hex)?;
        output.write_all(&digest)?;
        let actual = copy_exact(&mut input, &mut output, e.descriptor.byte_length())?;
        let mut tail = [0];
        if input.read(&mut tail)? != 0 || actual != e.descriptor {
            return Err(ContainerError::Integrity);
        }
        ensure_path_still_names_open_file(&path, &input)?;
        ensure_directory_still_matches(&parent, &parent_meta)?;
    }
    ensure_directory_still_matches(root, &root_meta)?;
    Ok(total)
}
fn hex_digest(hex: &str) -> Result<[u8; 32], ContainerError> {
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| ContainerError::Invalid)?;
    }
    Ok(bytes)
}
fn copy_exact(
    input: &mut impl Read,
    output: &mut impl Write,
    mut left: u64,
) -> Result<StoredObjectDescriptor, ContainerError> {
    let length = left;
    let mut digest = Sha256::new();
    let mut buffer = [0; COPY_BUFFER_BYTES];
    while left != 0 {
        let n = usize::try_from(left.min(COPY_BUFFER_BYTES as u64))
            .map_err(|_| ContainerError::Invalid)?;
        input.read_exact(&mut buffer[..n])?;
        output.write_all(&buffer[..n])?;
        digest.update(&buffer[..n]);
        left -= n as u64;
    }
    Ok(StoredObjectDescriptor::new(
        ContentDigest::from_bytes(digest.finalize().into()),
        length,
    ))
}
/// Extracts ONLY to a newly created private stage. On failure the known stage
/// remains for inspection; never use it as a live book. The caller must perform
/// full recipient cryptographic and SQLite verification before import/publish.
pub fn extract_container(
    mut input: impl Read,
    destination: &Path,
    limits: ContainerLimits,
) -> Result<Vec<ContainerEntry>, ContainerError> {
    let mut magic = [0; 12];
    input.read_exact(&mut magic)?;
    if &magic != CONTAINER_MAGIC {
        return Err(ContainerError::Invalid);
    }
    let mut c = [0; 4];
    input.read_exact(&mut c)?;
    let count = u32::from_be_bytes(c);
    if count < 3 {
        return Err(ContainerError::Invalid);
    }
    if count > limits.max_entries {
        return Err(ContainerError::Capacity);
    }
    let parent = destination.parent().ok_or(ContainerError::Invalid)?;
    let parent_meta = directory(parent)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(destination)?;
    let stage_meta = directory(destination)?;
    // Empty books still require the package attachment directory.
    builder.create(destination.join("attachments"))?;
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    let mut total = 16u64;
    let mut previous = String::new();
    for _ in 0..count {
        ensure_directory_still_matches(parent, &parent_meta)?;
        ensure_directory_still_matches(destination, &stage_meta)?;
        let mut n = [0; 2];
        input.read_exact(&mut n)?;
        let n = u16::from_be_bytes(n) as usize;
        if n == 0 || n > 128 {
            return Err(ContainerError::Invalid);
        }
        let mut name = vec![0; n];
        input.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| ContainerError::Invalid)?;
        validate_name(&name)?;
        if name <= previous || !names.insert(name.clone()) {
            return Err(ContainerError::Invalid);
        }
        previous = name.clone();
        let mut length = [0; 8];
        input.read_exact(&mut length)?;
        let length = u64::from_be_bytes(length);
        total = total
            .checked_add(42 + n as u64)
            .and_then(|v| v.checked_add(length))
            .ok_or(ContainerError::Capacity)?;
        if length > limits.max_object_bytes || total > limits.max_total_bytes {
            return Err(ContainerError::Capacity);
        }
        let mut digest = [0; 32];
        input.read_exact(&mut digest)?;
        let expected = StoredObjectDescriptor::new(ContentDigest::from_bytes(digest), length);
        let object_parent = if name.starts_with("attachments/") {
            destination.join("attachments")
        } else {
            destination.to_path_buf()
        };
        let object_parent_meta = directory(&object_parent)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(destination.join(&name))?;
        let actual = copy_exact(&mut input, &mut file, length)?;
        if actual != expected {
            return Err(ContainerError::Integrity);
        }
        file.sync_all()?;
        ensure_directory_still_matches(&object_parent, &object_parent_meta)?;
        entries.push(ContainerEntry::new(name, expected)?);
    }
    complete(&names)?;
    let mut tail = [0];
    if input.read(&mut tail)? != 0 {
        return Err(ContainerError::Invalid);
    }
    ensure_directory_still_matches(parent, &parent_meta)?;
    ensure_directory_still_matches(destination, &stage_meta)?;
    fs::File::open(destination)?.sync_all()?;
    Ok(entries)
}
