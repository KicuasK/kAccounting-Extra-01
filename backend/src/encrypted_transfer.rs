//! Bounded-memory attachment re-encryption for recipient transfer staging.
//!
//! This is not a complete `.kagextra` package workflow. A package orchestrator
//! must additionally bind these descriptors to its authenticated manifest and
//! verify the database's complete attachment reference set before publication.

use crate::{
    attachment_maintenance::StorePin,
    attachments::MAX_ATTACHMENT_BYTES,
    crypto::{
        ENCRYPTED_OBJECT_CHUNK_LENGTH, EncryptedObjectError, EncryptedObjectPurpose,
        RootEncryptionKey, decrypt_object, encrypt_object,
    },
    storage::{
        ContentDigest, LocalObjectPublishError, StoredObjectDescriptor, describe_file,
        ensure_directory_still_matches, ensure_path_still_names_open_file,
        open_regular_file_without_symlink, publish_verified_local_copy,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Debug, Error)]
pub enum EncryptedTransferError {
    #[error("encrypted transfer I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Crypto(#[from] EncryptedObjectError),
    #[error(transparent)]
    Publish(#[from] LocalObjectPublishError),
    #[error("encrypted attachment descriptor or pinned source does not match")]
    Integrity,
    #[error("attachment exceeds the Version 01 size limit")]
    TooLarge,
    #[error("encrypted transfer worker failed")]
    Worker,
    #[error("published ciphertext could not be verified at {path}")]
    PublishedUnverified { path: PathBuf },
    #[error("operating-system randomness is unavailable")]
    RandomUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferredAttachment {
    pub plaintext: StoredObjectDescriptor,
    pub ciphertext: StoredObjectDescriptor,
}

#[derive(Debug)]
pub struct AttachmentTransferOutcome {
    pub attachment: TransferredAttachment,
    /// Successful publication is independent from best-effort work cleanup.
    pub cleanup_warning: Option<String>,
}

/// Re-encrypts a live local attachment into transfer staging. The shared pin
/// prevents garbage collection from removing snapshot-referenced objects.
/// Hold the same pin from BEFORE taking the database snapshot until all objects
/// have been exported. This function does not create or publish a package.
pub fn export_attachment_object(
    pin: &StorePin,
    source: &Path,
    destination: &Path,
    expected: TransferredAttachment,
    local_key: &RootEncryptionKey,
    transfer_key: &RootEncryptionKey,
) -> Result<AttachmentTransferOutcome, EncryptedTransferError> {
    pin.validate()
        .map_err(|_| EncryptedTransferError::Integrity)?;
    let parent = source.parent().ok_or(EncryptedTransferError::Integrity)?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(EncryptedTransferError::Integrity);
    }
    if parent.canonicalize()? != pin.root().canonicalize()? {
        return Err(EncryptedTransferError::Integrity);
    }
    let result = transcode(
        source,
        destination,
        expected,
        ObjectCipher {
            key: local_key,
            purpose: EncryptedObjectPurpose::Attachment,
        },
        ObjectCipher {
            key: transfer_key,
            purpose: EncryptedObjectPurpose::TransferAttachment,
        },
    )?;
    pin.validate()
        .map_err(|_| EncryptedTransferError::PublishedUnverified {
            path: destination.to_path_buf(),
        })?;
    Ok(result)
}

/// Converts one authenticated transfer object into a new local encrypted
/// object. The destination belongs to a NEW private import staging tree; the
/// caller must not switch the active book until its full package is verified.
pub fn import_attachment_object(
    source: &Path,
    destination: &Path,
    expected: TransferredAttachment,
    transfer_key: &RootEncryptionKey,
    local_key: &RootEncryptionKey,
) -> Result<AttachmentTransferOutcome, EncryptedTransferError> {
    transcode(
        source,
        destination,
        expected,
        ObjectCipher {
            key: transfer_key,
            purpose: EncryptedObjectPurpose::TransferAttachment,
        },
        ObjectCipher {
            key: local_key,
            purpose: EncryptedObjectPurpose::Attachment,
        },
    )
}

#[derive(Clone, Copy)]
struct ObjectCipher<'a> {
    key: &'a RootEncryptionKey,
    purpose: EncryptedObjectPurpose,
}

fn transcode(
    source: &Path,
    destination: &Path,
    expected: TransferredAttachment,
    source_cipher: ObjectCipher<'_>,
    destination_cipher: ObjectCipher<'_>,
) -> Result<AttachmentTransferOutcome, EncryptedTransferError> {
    if expected.plaintext.byte_length() > MAX_ATTACHMENT_BYTES {
        return Err(EncryptedTransferError::TooLarge);
    }
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(EncryptedTransferError::Integrity);
    }
    if metadata.len() != expected.ciphertext.byte_length() {
        return Err(EncryptedTransferError::Integrity);
    }
    let source_file = open_regular_file_without_symlink(source)?;
    if !same_file(&metadata, &source_file.metadata()?) {
        return Err(EncryptedTransferError::Integrity);
    }
    let parent = destination
        .parent()
        .ok_or(EncryptedTransferError::Integrity)?;
    let mut work = CipherWork::new(parent)?;
    let work_path = work.path.join("object.kae");
    work.validate()?;
    let mut output = private_file(&work_path)?;
    transcode_stream(
        source_file,
        &mut output,
        expected,
        source_cipher,
        destination_cipher,
    )?;
    if !same_file(&metadata, &std::fs::symlink_metadata(source)?) {
        return Err(EncryptedTransferError::Integrity);
    }
    output.sync_all()?;
    ensure_path_still_names_open_file(&work_path, &output)?;
    drop(output);
    work.validate()?;
    let ciphertext = describe_file(&work_path)?;
    verify_ciphertext(
        &work_path,
        expected.plaintext,
        ciphertext,
        destination_cipher.key,
        destination_cipher.purpose,
    )?;
    let published = publish_verified_local_copy(&work_path, destination, ciphertext)?;
    // Independently authenticate the published name, not only the stage.
    if work.validate().is_err()
        || verify_ciphertext(
            destination,
            expected.plaintext,
            ciphertext,
            destination_cipher.key,
            destination_cipher.purpose,
        )
        .is_err()
    {
        return Err(EncryptedTransferError::PublishedUnverified {
            path: destination.to_path_buf(),
        });
    }
    let mut warnings = Vec::new();
    if let Some(warning) = published.cleanup_warning() {
        warnings.push(warning.to_string());
    }
    if let Err(error) = work.cleanup() {
        warnings.push(error.to_string());
    }
    Ok(AttachmentTransferOutcome {
        attachment: TransferredAttachment {
            plaintext: expected.plaintext,
            ciphertext,
        },
        cleanup_warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    })
}

fn transcode_stream(
    source: impl Read + Send,
    output: &mut impl Write,
    expected: TransferredAttachment,
    source_cipher: ObjectCipher<'_>,
    destination_cipher: ObjectCipher<'_>,
) -> Result<(), EncryptedTransferError> {
    // One queued chunk plus each endpoint's current chunk. Backpressure keeps
    // memory independent of attachment size; plaintext is never written to disk.
    let (sender, receiver) = sync_channel(1);
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let mut input = HashedReader::new(source);
            let mut pipe = PipeWriter {
                sender,
                digest: Sha256::new(),
                count: 0,
                limit: expected.plaintext.byte_length(),
            };
            decrypt_object(
                &mut input,
                &mut pipe,
                source_cipher.key,
                source_cipher.purpose,
            )?;
            if input.descriptor() != expected.ciphertext || pipe.descriptor() != expected.plaintext
            {
                return Err(EncryptedTransferError::Integrity);
            }
            Ok(())
        });
        let mut pipe = PipeReader {
            receiver,
            current: Zeroizing::new(Vec::new()),
            position: 0,
        };
        let encrypted = encrypt_object(
            &mut pipe,
            output,
            destination_cipher.key,
            destination_cipher.purpose,
        );
        // On output failure, drop the receiver before joining so a blocked
        // producer observes BrokenPipe instead of deadlocking.
        drop(pipe);
        let decrypted = worker.join().map_err(|_| EncryptedTransferError::Worker)?;
        decrypted?;
        encrypted?;
        Ok(())
    })
}

fn verify_ciphertext(
    path: &Path,
    plaintext: StoredObjectDescriptor,
    ciphertext: StoredObjectDescriptor,
    key: &RootEncryptionKey,
    purpose: EncryptedObjectPurpose,
) -> Result<(), EncryptedTransferError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(EncryptedTransferError::Integrity);
    }
    let mut input = HashedReader::new(open_regular_file_without_symlink(path)?);
    let mut output = HashSink::default();
    decrypt_object(&mut input, &mut output, key, purpose)?;
    ensure_path_still_names_open_file(path, &input.inner)?;
    if input.descriptor() != ciphertext || output.descriptor() != plaintext {
        return Err(EncryptedTransferError::Integrity);
    }
    Ok(())
}

#[derive(Default)]
struct HashSink {
    digest: Sha256,
    count: u64,
}
impl HashSink {
    fn descriptor(&self) -> StoredObjectDescriptor {
        descriptor(self.digest.clone(), self.count)
    }
}
impl Write for HashSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.count = self.count.checked_add(bytes.len() as u64).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "attachment length overflow")
        })?;
        self.digest.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct HashedReader<R> {
    inner: R,
    hash: HashSink,
}
impl<R> HashedReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hash: HashSink::default(),
        }
    }
    fn descriptor(&self) -> StoredObjectDescriptor {
        self.hash.descriptor()
    }
}
impl<R: Read> Read for HashedReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(bytes)?;
        self.hash.write_all(&bytes[..count])?;
        Ok(count)
    }
}
struct PipeWriter {
    sender: SyncSender<Zeroizing<Vec<u8>>>,
    digest: Sha256,
    count: u64,
    limit: u64,
}
impl PipeWriter {
    fn descriptor(&self) -> StoredObjectDescriptor {
        descriptor(self.digest.clone(), self.count)
    }
}
impl Write for PipeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.count.checked_add(bytes.len() as u64).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "attachment length overflow")
        })?;
        if next > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "attachment is longer than its descriptor",
            ));
        }
        for chunk in bytes.chunks(ENCRYPTED_OBJECT_CHUNK_LENGTH) {
            self.sender
                .send(Zeroizing::new(chunk.to_vec()))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "encryption consumer stopped")
                })?;
        }
        self.digest.update(bytes);
        self.count = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct PipeReader {
    receiver: Receiver<Zeroizing<Vec<u8>>>,
    current: Zeroizing<Vec<u8>>,
    position: usize,
}
impl Read for PipeReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        while self.position == self.current.len() {
            match self.receiver.recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.position = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let count = bytes.len().min(self.current.len() - self.position);
        bytes[..count].copy_from_slice(&self.current[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
fn descriptor(digest: Sha256, count: u64) -> StoredObjectDescriptor {
    let mut bytes = [0; 32];
    bytes.copy_from_slice(&digest.finalize());
    StoredObjectDescriptor::new(ContentDigest::from_bytes(bytes), count)
}

struct CipherWork {
    path: PathBuf,
    parent: PathBuf,
    parent_identity: std::fs::Metadata,
    identity: std::fs::Metadata,
    cleaned: bool,
}
impl CipherWork {
    fn new(parent: &Path) -> Result<Self, EncryptedTransferError> {
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(EncryptedTransferError::Integrity);
        }
        check_private_directory(&metadata)?;
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| EncryptedTransferError::RandomUnavailable)?;
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = parent.join(format!(".kAccounting-Extra-01-transfer-object-{name}.work"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        let identity = std::fs::symlink_metadata(&path)?;
        let work = Self {
            path,
            parent: parent.to_path_buf(),
            parent_identity: metadata,
            identity,
            cleaned: false,
        };
        work.validate()?;
        Ok(work)
    }
    fn validate(&self) -> io::Result<()> {
        ensure_directory_still_matches(&self.parent, &self.parent_identity)?;
        ensure_directory_still_matches(&self.path, &self.identity)?;
        check_private_directory(&std::fs::symlink_metadata(&self.parent)?)?;
        check_private_directory(&std::fs::symlink_metadata(&self.path)?)
    }
    fn cleanup(&mut self) -> io::Result<()> {
        self.validate()?;
        match std::fs::remove_file(self.path.join("object.kae")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::remove_dir(&self.path)?;
        self.cleaned = true;
        Ok(())
    }
}
fn check_private_directory(metadata: &std::fs::Metadata) -> io::Result<()> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transfer staging requires a private directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "transfer staging requires a private directory",
            ));
        }
    }
    Ok(())
}
impl Drop for CipherWork {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}
fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.is_file()
            && b.is_file()
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
    }
    #[cfg(not(unix))]
    {
        a.is_file() && b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct FailedOutput {
        remaining: usize,
    }
    impl Write for FailedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected storage exhaustion"));
            }
            let count = self.remaining.min(bytes.len());
            self.remaining -= count;
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn consumer_failure_unblocks_producer_and_returns_failure() {
        let local = RootEncryptionKey::new([17; 32]);
        let transfer = RootEncryptionKey::new([91; 32]);
        let bytes = vec![0x5a; ENCRYPTED_OBJECT_CHUNK_LENGTH * 12];
        let mut source = Vec::new();
        encrypt_object(
            &mut Cursor::new(&bytes),
            &mut source,
            &local,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        let expected = TransferredAttachment {
            plaintext: crate::storage::describe_reader(&mut Cursor::new(&bytes)).unwrap(),
            ciphertext: crate::storage::describe_reader(&mut Cursor::new(&source)).unwrap(),
        };
        // Fail at header, first chunk, and later chunks while the producer can
        // be blocked on the single-message queue. A successful join proves the
        // consumer shutdown wakes the producer rather than deadlocking.
        for remaining in [0, 1, 72, 65536, 3 * 65536] {
            let mut output = FailedOutput { remaining };
            assert!(
                transcode_stream(
                    Cursor::new(&source),
                    &mut output,
                    expected,
                    ObjectCipher {
                        key: &local,
                        purpose: EncryptedObjectPurpose::Attachment
                    },
                    ObjectCipher {
                        key: &transfer,
                        purpose: EncryptedObjectPurpose::TransferAttachment
                    },
                )
                .is_err()
            );
        }
    }
}
