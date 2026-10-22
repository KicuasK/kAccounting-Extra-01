//! Authenticated encrypted-transfer catalog contract. This does not publish or
//! import a package; the orchestrator must verify the database and every object.

use crate::{
    attachments::{Attachment, AttachmentId, MAX_ATTACHMENT_BYTES, MAX_BOOK_ATTACHMENT_BYTES},
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, decrypt_object, encrypt_object},
    encrypted_transfer::TransferredAttachment,
    storage::{ContentDigest, StoredObjectDescriptor},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Cursor, Read, Write},
};
use thiserror::Error;
use zeroize::Zeroizing;

const FORMAT: &str = "kAccounting-Extra-01.encrypted-transfer-manifest";
const VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum TransferManifestError {
    #[error("encrypted transfer manifest is malformed or unsupported")]
    Invalid,
    #[error("encrypted transfer manifest exceeds its metadata limit")]
    TooLarge,
    #[error(
        "encrypted transfer manifest does not match the package context or database references"
    )]
    Mismatch,
    #[error(transparent)]
    Crypto(#[from] crate::crypto::EncryptedObjectError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    sha256: String,
    byte_length: String,
}
impl From<StoredObjectDescriptor> for Descriptor {
    fn from(value: StoredObjectDescriptor) -> Self {
        Self {
            sha256: value.digest().to_hex(),
            byte_length: value.byte_length().to_string(),
        }
    }
}
impl Descriptor {
    fn decode(&self) -> Result<StoredObjectDescriptor, TransferManifestError> {
        let digest: ContentDigest = self
            .sha256
            .parse()
            .map_err(|_| TransferManifestError::Invalid)?;
        let length: u64 = self
            .byte_length
            .parse()
            .map_err(|_| TransferManifestError::Invalid)?;
        if digest.to_hex() != self.sha256 || length.to_string() != self.byte_length {
            return Err(TransferManifestError::Invalid);
        }
        Ok(StoredObjectDescriptor::new(digest, length))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    id: String,
    plaintext: Descriptor,
    source_ciphertext: Descriptor,
    transfer_ciphertext: Descriptor,
}

/// Metadata stays encrypted. Paths are derived only from canonical attachment
/// IDs, never filenames supplied by a package. Exact lengths use JSON strings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedTransferManifest {
    format: String,
    version: u32,
    package_id: String,
    envelope: Descriptor,
    database_plaintext: Descriptor,
    database_ciphertext: Descriptor,
    attachments: Vec<Object>,
}

impl EncryptedTransferManifest {
    /// `envelope` describes the exact serialized recipient envelope bytes.
    /// Each tuple pairs a snapshot reference with its re-encrypted object.
    pub fn new(
        package_id: &str,
        envelope: StoredObjectDescriptor,
        database: TransferredAttachment,
        objects: Vec<(Attachment, TransferredAttachment)>,
    ) -> Result<Self, TransferManifestError> {
        let mut attachments = Vec::with_capacity(objects.len());
        for (source, transferred) in objects {
            if source.plaintext != transferred.plaintext {
                return Err(TransferManifestError::Mismatch);
            }
            attachments.push(Object {
                id: source.id.as_str().to_owned(),
                plaintext: source.plaintext.into(),
                source_ciphertext: source.ciphertext.into(),
                transfer_ciphertext: transferred.ciphertext.into(),
            });
        }
        attachments.sort_by(|a, b| a.id.cmp(&b.id));
        let manifest = Self {
            format: FORMAT.into(),
            version: VERSION,
            package_id: package_id.into(),
            envelope: envelope.into(),
            database_plaintext: database.plaintext.into(),
            database_ciphertext: database.ciphertext.into(),
            attachments,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), TransferManifestError> {
        if self.format != FORMAT
            || self.version != VERSION
            || AttachmentId::parse(&self.package_id).is_err()
        {
            return Err(TransferManifestError::Invalid);
        }
        self.envelope.decode()?;
        self.database_plaintext.decode()?;
        self.database_ciphertext.decode()?;
        let mut previous: Option<&str> = None;
        let mut total = 0u64;
        for object in &self.attachments {
            AttachmentId::parse(&object.id).map_err(|_| TransferManifestError::Invalid)?;
            if previous.is_some_and(|id| id >= object.id.as_str()) {
                return Err(TransferManifestError::Invalid);
            }
            previous = Some(&object.id);
            let size = object.plaintext.decode()?.byte_length();
            if size > MAX_ATTACHMENT_BYTES {
                return Err(TransferManifestError::Invalid);
            }
            total = total
                .checked_add(size)
                .ok_or(TransferManifestError::Invalid)?;
            if total > MAX_BOOK_ATTACHMENT_BYTES {
                return Err(TransferManifestError::Invalid);
            }
            object.source_ciphertext.decode()?;
            object.transfer_ciphertext.decode()?;
        }
        Ok(())
    }

    /// Compare the complete catalog from the verified, decrypted SQLite snapshot.
    /// Missing, extra, duplicate, or changed references fail closed.
    pub fn verify_references(
        &self,
        references: &[Attachment],
    ) -> Result<(), TransferManifestError> {
        self.validate()?;
        let mut expected = BTreeMap::new();
        for attachment in references {
            if expected
                .insert(attachment.id.as_str(), attachment)
                .is_some()
            {
                return Err(TransferManifestError::Mismatch);
            }
        }
        if expected.len() != self.attachments.len() {
            return Err(TransferManifestError::Mismatch);
        }
        for object in &self.attachments {
            let reference = expected
                .get(object.id.as_str())
                .ok_or(TransferManifestError::Mismatch)?;
            if reference.plaintext != object.plaintext.decode()?
                || reference.ciphertext != object.source_ciphertext.decode()?
            {
                return Err(TransferManifestError::Mismatch);
            }
        }
        Ok(())
    }

    pub fn database(&self) -> Result<TransferredAttachment, TransferManifestError> {
        self.validate()?;
        Ok(TransferredAttachment {
            plaintext: self.database_plaintext.decode()?,
            ciphertext: self.database_ciphertext.decode()?,
        })
    }

    pub fn attachment_objects(
        &self,
    ) -> Result<Vec<(AttachmentId, TransferredAttachment)>, TransferManifestError> {
        self.validate()?;
        self.attachments
            .iter()
            .map(|object| {
                Ok((
                    AttachmentId::parse(&object.id).map_err(|_| TransferManifestError::Invalid)?,
                    TransferredAttachment {
                        plaintext: object.plaintext.decode()?,
                        ciphertext: object.transfer_ciphertext.decode()?,
                    },
                ))
            })
            .collect()
    }

    pub fn encrypt(
        &self,
        output: &mut impl Write,
        key: &RootEncryptionKey,
    ) -> Result<(), TransferManifestError> {
        self.validate()?;
        let mut bytes = BoundedMetadata(Zeroizing::new(Vec::new()));
        serde_json::to_writer(&mut bytes, self).map_err(|_| TransferManifestError::TooLarge)?;
        encrypt_object(
            &mut Cursor::new(&*bytes.0),
            output,
            key,
            EncryptedObjectPurpose::TransferManifest,
        )?;
        Ok(())
    }

    /// Does not return parsed metadata until the entire object authenticates.
    pub fn decrypt(
        input: &mut impl Read,
        key: &RootEncryptionKey,
        expected_package_id: &str,
        expected_envelope: StoredObjectDescriptor,
    ) -> Result<Self, TransferManifestError> {
        let mut bytes = BoundedMetadata(Zeroizing::new(Vec::new()));
        decrypt_object(
            input,
            &mut bytes,
            key,
            EncryptedObjectPurpose::TransferManifest,
        )?;
        let manifest: Self =
            serde_json::from_slice(&bytes.0).map_err(|_| TransferManifestError::Invalid)?;
        manifest.validate()?;
        if manifest.package_id != expected_package_id
            || manifest.envelope.decode()? != expected_envelope
        {
            return Err(TransferManifestError::Mismatch);
        }
        Ok(manifest)
    }
}

struct BoundedMetadata(Zeroizing<Vec<u8>>);
impl Write for BoundedMetadata {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_MANIFEST_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other(
                "encrypted transfer metadata limit exceeded",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
