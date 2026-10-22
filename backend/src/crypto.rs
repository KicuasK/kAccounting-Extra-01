//! Versioned, bounded-memory authenticated encryption for stored objects.
//!
//! This module deliberately accepts an already available root key. Keychain
//! access and passwordless recipient-key wrapping belong to higher-level
//! integrations.

use aead_stream::aead::{Key, Payload};
use aead_stream::{DecryptorBE32, EncryptorBE32, Nonce, StreamBE32};
use chacha20poly1305::XChaCha20Poly1305;
use hkdf::Hkdf;
use sha2::Sha256;
use std::io::{self, Read, Write};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8; 8] = b"KAEOBJ01";
const FORMAT_VERSION: u16 = 1;
const CIPHER_XCHACHA20_POLY1305: u8 = 1;
const STREAM_BE32: u8 = 1;
const KDF_HKDF_SHA256: u8 = 1;
const RESERVED_LENGTH: usize = 3;
const SALT_LENGTH: usize = 32;
const STREAM_NONCE_LENGTH: usize = 19;
const TAG_LENGTH: usize = 16;
const FRAME_LENGTH_LENGTH: usize = 4;
pub const ENCRYPTED_OBJECT_HEADER_LENGTH: usize = 72;
pub const ENCRYPTED_OBJECT_CHUNK_LENGTH: usize = 64 * 1024;
const MAX_CIPHERTEXT_CHUNK_LENGTH: usize = ENCRYPTED_OBJECT_CHUNK_LENGTH + TAG_LENGTH;

type Cipher = XChaCha20Poly1305;
type CipherStream = StreamBE32<Cipher>;
type StreamNonce = Nonce<Cipher, CipherStream>;

/// A domain separator for encrypted object keys and authenticated headers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EncryptedObjectPurpose {
    Attachment = 1,
    Backup = 2,
    TransferDatabase = 3,
    TransferManifest = 4,
    TransferAttachment = 5,
    BackupManifest = 6,
    TransferRecovery = 7,
}

impl EncryptedObjectPurpose {
    fn kdf_info(self) -> &'static [u8] {
        match self {
            Self::Attachment => b"kAccounting-Extra-01/encrypted-object/v1/attachment",
            Self::Backup => b"kAccounting-Extra-01/encrypted-object/v1/backup",
            Self::BackupManifest => b"kAccounting-Extra-01/encrypted-object/v1/backup-manifest",
            Self::TransferRecovery => b"kAccounting-Extra-01/encrypted-object/v1/transfer-recovery",
            Self::TransferDatabase => b"kAccounting-Extra-01/encrypted-object/v1/transfer-database",
            Self::TransferManifest => b"kAccounting-Extra-01/encrypted-object/v1/transfer-manifest",
            Self::TransferAttachment => {
                b"kAccounting-Extra-01/encrypted-object/v1/transfer-attachment"
            }
        }
    }
}

impl TryFrom<u8> for EncryptedObjectPurpose {
    type Error = EncryptedObjectError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Attachment),
            2 => Ok(Self::Backup),
            3 => Ok(Self::TransferDatabase),
            4 => Ok(Self::TransferManifest),
            5 => Ok(Self::TransferAttachment),
            6 => Ok(Self::BackupManifest),
            7 => Ok(Self::TransferRecovery),
            _ => Err(EncryptedObjectError::UnsupportedPurpose(value)),
        }
    }
}

/// A 256-bit root key whose memory is cleared when dropped.
pub struct RootEncryptionKey(Zeroizing<[u8; 32]>);

impl RootEncryptionKey {
    pub fn new(mut bytes: [u8; 32]) -> Self {
        let key = Self(Zeroizing::new(bytes));
        bytes.zeroize();
        key
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Exact byte counts processed by one successful operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncryptedObjectOutcome {
    pub plaintext_byte_length: u64,
    pub encrypted_byte_length: u64,
}

#[derive(Debug, Error)]
pub enum EncryptedObjectError {
    #[error("encrypted object I/O failed while {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("operating-system random generation failed: {0}")]
    Random(#[source] getrandom::Error),
    #[error("encrypted object header is invalid: {0}")]
    InvalidHeader(&'static str),
    #[error("encrypted object purpose {0} is unsupported")]
    UnsupportedPurpose(u8),
    #[error("encrypted object purpose mismatch: expected {expected:?}, found {actual:?}")]
    PurposeMismatch {
        expected: EncryptedObjectPurpose,
        actual: EncryptedObjectPurpose,
    },
    #[error("encrypted object frame is invalid: {0}")]
    InvalidFrame(&'static str),
    #[error("encrypted object authentication failed")]
    AuthenticationFailed,
    #[error("encrypted object key derivation failed")]
    KeyDerivationFailed,
    #[error("encrypted object byte count overflow")]
    LengthOverflow,
}

#[derive(Clone)]
struct Header {
    bytes: [u8; ENCRYPTED_OBJECT_HEADER_LENGTH],
    purpose: EncryptedObjectPurpose,
    salt: [u8; SALT_LENGTH],
    stream_nonce: [u8; STREAM_NONCE_LENGTH],
}

impl Header {
    fn generate(purpose: EncryptedObjectPurpose) -> Result<Self, EncryptedObjectError> {
        let mut salt = [0_u8; SALT_LENGTH];
        let mut stream_nonce = [0_u8; STREAM_NONCE_LENGTH];
        getrandom::fill(&mut salt).map_err(EncryptedObjectError::Random)?;
        getrandom::fill(&mut stream_nonce).map_err(EncryptedObjectError::Random)?;
        Ok(Self::from_material(purpose, salt, stream_nonce))
    }

    fn from_material(
        purpose: EncryptedObjectPurpose,
        salt: [u8; SALT_LENGTH],
        stream_nonce: [u8; STREAM_NONCE_LENGTH],
    ) -> Self {
        let mut bytes = [0_u8; ENCRYPTED_OBJECT_HEADER_LENGTH];
        bytes[0..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_be_bytes());
        bytes[10] = CIPHER_XCHACHA20_POLY1305;
        bytes[11] = STREAM_BE32;
        bytes[12] = KDF_HKDF_SHA256;
        bytes[13] = purpose as u8;
        bytes[14..18].copy_from_slice(&(ENCRYPTED_OBJECT_CHUNK_LENGTH as u32).to_be_bytes());
        bytes[18..50].copy_from_slice(&salt);
        bytes[50..69].copy_from_slice(&stream_nonce);
        debug_assert_eq!(&bytes[69..72], &[0_u8; RESERVED_LENGTH]);
        Self {
            bytes,
            purpose,
            salt,
            stream_nonce,
        }
    }

    fn parse(bytes: [u8; ENCRYPTED_OBJECT_HEADER_LENGTH]) -> Result<Self, EncryptedObjectError> {
        if &bytes[0..8] != MAGIC {
            return Err(EncryptedObjectError::InvalidHeader("wrong magic"));
        }
        if u16::from_be_bytes([bytes[8], bytes[9]]) != FORMAT_VERSION {
            return Err(EncryptedObjectError::InvalidHeader(
                "unsupported format version",
            ));
        }
        if bytes[10] != CIPHER_XCHACHA20_POLY1305 {
            return Err(EncryptedObjectError::InvalidHeader(
                "unsupported cipher identifier",
            ));
        }
        if bytes[11] != STREAM_BE32 {
            return Err(EncryptedObjectError::InvalidHeader(
                "unsupported stream identifier",
            ));
        }
        if bytes[12] != KDF_HKDF_SHA256 {
            return Err(EncryptedObjectError::InvalidHeader(
                "unsupported KDF identifier",
            ));
        }
        let purpose = EncryptedObjectPurpose::try_from(bytes[13])?;
        let chunk_length = u32::from_be_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        if chunk_length != ENCRYPTED_OBJECT_CHUNK_LENGTH as u32 {
            return Err(EncryptedObjectError::InvalidHeader(
                "unsupported chunk length",
            ));
        }
        if bytes[69..72] != [0_u8; RESERVED_LENGTH] {
            return Err(EncryptedObjectError::InvalidHeader(
                "reserved header bytes are not zero",
            ));
        }

        let mut salt = [0_u8; SALT_LENGTH];
        salt.copy_from_slice(&bytes[18..50]);
        let mut stream_nonce = [0_u8; STREAM_NONCE_LENGTH];
        stream_nonce.copy_from_slice(&bytes[50..69]);
        Ok(Self {
            bytes,
            purpose,
            salt,
            stream_nonce,
        })
    }
}

/// Encrypts one object with a fresh salt and nonce using bounded-memory I/O.
pub fn encrypt_object(
    reader: &mut impl Read,
    writer: &mut impl Write,
    root_key: &RootEncryptionKey,
    purpose: EncryptedObjectPurpose,
) -> Result<EncryptedObjectOutcome, EncryptedObjectError> {
    let header = Header::generate(purpose)?;
    encrypt_with_header(reader, writer, root_key, header)
}

/// Decrypts one complete object using bounded-memory I/O.
///
/// The writer can contain authenticated plaintext from earlier chunks if a
/// later chunk fails. Callers must therefore use private temporary storage and
/// publish it only after this function returns success.
pub fn decrypt_object(
    reader: &mut impl Read,
    writer: &mut impl Write,
    root_key: &RootEncryptionKey,
    expected_purpose: EncryptedObjectPurpose,
) -> Result<EncryptedObjectOutcome, EncryptedObjectError> {
    let mut header_bytes = [0_u8; ENCRYPTED_OBJECT_HEADER_LENGTH];
    read_exact(reader, &mut header_bytes, "reading encrypted object header")?;
    let header = Header::parse(header_bytes)?;
    if header.purpose != expected_purpose {
        return Err(EncryptedObjectError::PurposeMismatch {
            expected: expected_purpose,
            actual: header.purpose,
        });
    }

    let derived_key = derive_object_key(root_key, &header)?;
    let mut key = Key::<Cipher>::from(*derived_key);
    let nonce = StreamNonce::from(header.stream_nonce);
    let mut decryptor = DecryptorBE32::<Cipher>::new(&key, &nonce);
    let key_bytes: &mut [u8] = key.as_mut();
    key_bytes.zeroize();
    let mut encrypted_byte_length = ENCRYPTED_OBJECT_HEADER_LENGTH as u64;
    let mut plaintext_byte_length = 0_u64;

    let mut current = read_frame(reader)?.ok_or(EncryptedObjectError::InvalidFrame(
        "missing authenticated final frame",
    ))?;
    encrypted_byte_length = checked_add(encrypted_byte_length, framed_length(&current)?)?;

    while let Some(next) = read_frame(reader)? {
        encrypted_byte_length = checked_add(encrypted_byte_length, framed_length(&next)?)?;
        let plaintext = decryptor
            .decrypt_next(Payload {
                msg: &current,
                aad: &header.bytes,
            })
            .map_err(|_| EncryptedObjectError::AuthenticationFailed)?;
        if plaintext.len() != ENCRYPTED_OBJECT_CHUNK_LENGTH {
            return Err(EncryptedObjectError::InvalidFrame(
                "non-final plaintext chunk has the wrong length",
            ));
        }
        write_all(writer, &plaintext, "writing authenticated plaintext")?;
        plaintext_byte_length = checked_add(plaintext_byte_length, plaintext.len() as u64)?;
        current = next;
    }

    let plaintext = decryptor
        .decrypt_last(Payload {
            msg: &current,
            aad: &header.bytes,
        })
        .map_err(|_| EncryptedObjectError::AuthenticationFailed)?;
    write_all(writer, &plaintext, "writing authenticated plaintext")?;
    plaintext_byte_length = checked_add(plaintext_byte_length, plaintext.len() as u64)?;
    writer.flush().map_err(|source| EncryptedObjectError::Io {
        operation: "flushing authenticated plaintext",
        source,
    })?;

    Ok(EncryptedObjectOutcome {
        plaintext_byte_length,
        encrypted_byte_length,
    })
}

fn encrypt_with_header(
    reader: &mut impl Read,
    writer: &mut impl Write,
    root_key: &RootEncryptionKey,
    header: Header,
) -> Result<EncryptedObjectOutcome, EncryptedObjectError> {
    write_all(writer, &header.bytes, "writing encrypted object header")?;
    let derived_key = derive_object_key(root_key, &header)?;
    let mut key = Key::<Cipher>::from(*derived_key);
    let nonce = StreamNonce::from(header.stream_nonce);
    let mut encryptor = EncryptorBE32::<Cipher>::new(&key, &nonce);
    let key_bytes: &mut [u8] = key.as_mut();
    key_bytes.zeroize();
    let mut encrypted_byte_length = ENCRYPTED_OBJECT_HEADER_LENGTH as u64;
    let mut plaintext_byte_length = 0_u64;

    let mut plaintext = Vec::with_capacity(ENCRYPTED_OBJECT_CHUNK_LENGTH);
    fill_chunk(reader, &mut plaintext)?;

    loop {
        let mut lookahead = [0_u8; 1];
        let has_more = read_retry(reader, &mut lookahead, "reading plaintext lookahead")? != 0;
        if !has_more {
            break;
        }
        let ciphertext = encryptor
            .encrypt_next(Payload {
                msg: &plaintext,
                aad: &header.bytes,
            })
            .map_err(|_| EncryptedObjectError::AuthenticationFailed)?;
        write_frame(writer, &ciphertext)?;
        plaintext_byte_length = checked_add(plaintext_byte_length, plaintext.len() as u64)?;
        encrypted_byte_length = checked_add(encrypted_byte_length, framed_length(&ciphertext)?)?;
        plaintext.clear();
        plaintext.push(lookahead[0]);
        fill_chunk(reader, &mut plaintext)?;
    }

    let ciphertext = encryptor
        .encrypt_last(Payload {
            msg: &plaintext,
            aad: &header.bytes,
        })
        .map_err(|_| EncryptedObjectError::AuthenticationFailed)?;
    write_frame(writer, &ciphertext)?;
    plaintext_byte_length = checked_add(plaintext_byte_length, plaintext.len() as u64)?;
    encrypted_byte_length = checked_add(encrypted_byte_length, framed_length(&ciphertext)?)?;
    writer.flush().map_err(|source| EncryptedObjectError::Io {
        operation: "flushing encrypted object",
        source,
    })?;

    Ok(EncryptedObjectOutcome {
        plaintext_byte_length,
        encrypted_byte_length,
    })
}

fn derive_object_key(
    root_key: &RootEncryptionKey,
    header: &Header,
) -> Result<Zeroizing<[u8; 32]>, EncryptedObjectError> {
    let hkdf = Hkdf::<Sha256>::new(Some(&header.salt), root_key.as_bytes());
    let mut key = Zeroizing::new([0_u8; 32]);
    hkdf.expand(header.purpose.kdf_info(), key.as_mut())
        .map_err(|_| EncryptedObjectError::KeyDerivationFailed)?;
    Ok(key)
}

fn fill_chunk(reader: &mut impl Read, buffer: &mut Vec<u8>) -> Result<(), EncryptedObjectError> {
    while buffer.len() < ENCRYPTED_OBJECT_CHUNK_LENGTH {
        let old_length = buffer.len();
        buffer.resize(ENCRYPTED_OBJECT_CHUNK_LENGTH, 0);
        let read = read_retry(reader, &mut buffer[old_length..], "reading plaintext chunk")?;
        buffer.truncate(old_length + read);
        if read == 0 {
            break;
        }
    }
    Ok(())
}

fn write_frame(writer: &mut impl Write, ciphertext: &[u8]) -> Result<(), EncryptedObjectError> {
    let length = u32::try_from(ciphertext.len())
        .map_err(|_| EncryptedObjectError::InvalidFrame("ciphertext frame length exceeds u32"))?;
    write_all(
        writer,
        &length.to_be_bytes(),
        "writing encrypted frame length",
    )?;
    write_all(writer, ciphertext, "writing encrypted frame")
}

fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<u8>>, EncryptedObjectError> {
    let mut length_bytes = [0_u8; FRAME_LENGTH_LENGTH];
    let first = read_retry(
        reader,
        &mut length_bytes[..1],
        "reading encrypted frame length",
    )?;
    if first == 0 {
        return Ok(None);
    }
    read_exact(
        reader,
        &mut length_bytes[1..],
        "reading encrypted frame length",
    )?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(TAG_LENGTH..=MAX_CIPHERTEXT_CHUNK_LENGTH).contains(&length) {
        return Err(EncryptedObjectError::InvalidFrame(
            "ciphertext frame length is outside the allowed range",
        ));
    }
    let mut ciphertext = vec![0_u8; length];
    read_exact(reader, &mut ciphertext, "reading encrypted frame")?;
    Ok(Some(ciphertext))
}

fn read_exact(
    reader: &mut impl Read,
    mut buffer: &mut [u8],
    operation: &'static str,
) -> Result<(), EncryptedObjectError> {
    while !buffer.is_empty() {
        let read = read_retry(reader, buffer, operation)?;
        if read == 0 {
            return Err(EncryptedObjectError::InvalidFrame("truncated input"));
        }
        buffer = &mut buffer[read..];
    }
    Ok(())
}

fn read_retry(
    reader: &mut impl Read,
    buffer: &mut [u8],
    operation: &'static str,
) -> Result<usize, EncryptedObjectError> {
    loop {
        match reader.read(buffer) {
            Ok(read) => return Ok(read),
            Err(source) if source.kind() == io::ErrorKind::Interrupted => continue,
            Err(source) => return Err(EncryptedObjectError::Io { operation, source }),
        }
    }
}

fn write_all(
    writer: &mut impl Write,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), EncryptedObjectError> {
    writer
        .write_all(bytes)
        .map_err(|source| EncryptedObjectError::Io { operation, source })
}

fn framed_length(ciphertext: &[u8]) -> Result<u64, EncryptedObjectError> {
    checked_add(FRAME_LENGTH_LENGTH as u64, ciphertext.len() as u64)
}

fn checked_add(left: u64, right: u64) -> Result<u64, EncryptedObjectError> {
    left.checked_add(right)
        .ok_or(EncryptedObjectError::LengthOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const TEST_KEY_BYTES: [u8; 32] = [0x42; 32];
    const TEST_SALT: [u8; SALT_LENGTH] = [0x11; SALT_LENGTH];
    const TEST_NONCE: [u8; STREAM_NONCE_LENGTH] = [0x22; STREAM_NONCE_LENGTH];

    fn encrypt_deterministically(plaintext: &[u8]) -> Vec<u8> {
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        let header =
            Header::from_material(EncryptedObjectPurpose::Attachment, TEST_SALT, TEST_NONCE);
        let mut encrypted = Vec::new();
        encrypt_with_header(&mut Cursor::new(plaintext), &mut encrypted, &key, header)
            .expect("deterministic encryption should succeed");
        encrypted
    }

    fn decrypt_attachment(encrypted: &[u8], key_bytes: [u8; 32]) -> Result<Vec<u8>, String> {
        let key = RootEncryptionKey::new(key_bytes);
        let mut plaintext = Vec::new();
        decrypt_object(
            &mut Cursor::new(encrypted),
            &mut plaintext,
            &key,
            EncryptedObjectPurpose::Attachment,
        )
        .map_err(|error| error.to_string())?;
        Ok(plaintext)
    }

    #[test]
    fn round_trips_boundary_lengths() {
        for length in [
            0,
            1,
            ENCRYPTED_OBJECT_CHUNK_LENGTH - 1,
            ENCRYPTED_OBJECT_CHUNK_LENGTH,
            ENCRYPTED_OBJECT_CHUNK_LENGTH + 1,
            ENCRYPTED_OBJECT_CHUNK_LENGTH * 3 + 17,
        ] {
            let plaintext = (0..length).map(|index| index as u8).collect::<Vec<_>>();
            let encrypted = encrypt_deterministically(&plaintext);
            assert_eq!(
                decrypt_attachment(&encrypted, TEST_KEY_BYTES).unwrap(),
                plaintext,
                "length {length}"
            );
        }
    }

    #[test]
    fn rejects_wrong_key_and_tampering() {
        let encrypted = encrypt_deterministically(b"private receipt");
        assert!(decrypt_attachment(&encrypted, [0x24; 32]).is_err());

        let mut header_tampered = encrypted.clone();
        header_tampered[18] ^= 1;
        assert!(decrypt_attachment(&header_tampered, TEST_KEY_BYTES).is_err());

        let mut ciphertext_tampered = encrypted;
        *ciphertext_tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt_attachment(&ciphertext_tampered, TEST_KEY_BYTES).is_err());
    }

    #[test]
    fn deterministic_format_vector() {
        let encrypted = encrypt_deterministically(b"Version 01 test vector");
        let encoded = encrypted
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            encoded,
            "4b41454f424a30310001010101010001000011111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222200000000000026aceb52bf101f894d6cb407e3903cd28ed20a22217907d46fa246bd38a74ad0b0159ae618e36d"
        );
    }

    #[test]
    fn rejects_every_truncation_and_trailing_data() {
        let encrypted = encrypt_deterministically(b"truncation evidence");
        for end in 0..encrypted.len() {
            assert!(
                decrypt_attachment(&encrypted[..end], TEST_KEY_BYTES).is_err(),
                "truncation at {end} unexpectedly succeeded"
            );
        }

        let mut trailing = encrypted;
        trailing.push(0);
        assert!(decrypt_attachment(&trailing, TEST_KEY_BYTES).is_err());
    }

    fn encoded_frames(encrypted: &[u8]) -> Vec<&[u8]> {
        let mut frames = Vec::new();
        let mut offset = ENCRYPTED_OBJECT_HEADER_LENGTH;
        while offset < encrypted.len() {
            let length = u32::from_be_bytes(
                encrypted[offset..offset + FRAME_LENGTH_LENGTH]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let end = offset + FRAME_LENGTH_LENGTH + length;
            frames.push(&encrypted[offset..end]);
            offset = end;
        }
        frames
    }

    #[test]
    fn rejects_reordered_duplicated_and_missing_chunks() {
        let plaintext = vec![0x7a; ENCRYPTED_OBJECT_CHUNK_LENGTH * 2 + 7];
        let encrypted = encrypt_deterministically(&plaintext);
        let frames = encoded_frames(&encrypted);
        assert_eq!(frames.len(), 3);

        let rebuild = |order: &[usize]| {
            let mut candidate = encrypted[..ENCRYPTED_OBJECT_HEADER_LENGTH].to_vec();
            for index in order {
                candidate.extend_from_slice(frames[*index]);
            }
            candidate
        };
        for order in [&[1, 0, 2][..], &[0, 0, 1, 2], &[0, 2]] {
            assert!(
                decrypt_attachment(&rebuild(order), TEST_KEY_BYTES).is_err(),
                "invalid frame order {order:?} unexpectedly succeeded"
            );
        }
    }

    #[test]
    fn fresh_encryption_randomizes_the_header_and_ciphertext() {
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        let mut first = Vec::new();
        let mut second = Vec::new();
        encrypt_object(
            &mut Cursor::new(b"same plaintext"),
            &mut first,
            &key,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        encrypt_object(
            &mut Cursor::new(b"same plaintext"),
            &mut second,
            &key,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn rejects_purpose_mismatch_before_plaintext_output() {
        let encrypted = encrypt_deterministically(b"receipt");
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        let mut plaintext = Vec::new();
        let error = decrypt_object(
            &mut Cursor::new(encrypted),
            &mut plaintext,
            &key,
            EncryptedObjectPurpose::Backup,
        )
        .expect_err("purpose mismatch must fail");
        assert!(matches!(
            error,
            EncryptedObjectError::PurposeMismatch { .. }
        ));
        assert!(plaintext.is_empty());
    }

    #[test]
    fn every_object_purpose_is_separated_including_private_transfer_recovery() {
        let purposes = [
            EncryptedObjectPurpose::Attachment,
            EncryptedObjectPurpose::Backup,
            EncryptedObjectPurpose::TransferDatabase,
            EncryptedObjectPurpose::TransferManifest,
            EncryptedObjectPurpose::TransferAttachment,
            EncryptedObjectPurpose::BackupManifest,
            EncryptedObjectPurpose::TransferRecovery,
        ];
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        for purpose in purposes {
            assert_eq!(
                EncryptedObjectPurpose::try_from(purpose as u8).unwrap(),
                purpose
            );
            let header = Header::from_material(purpose, TEST_SALT, TEST_NONCE);
            let mut ciphertext = Vec::new();
            encrypt_with_header(
                &mut Cursor::new(b"purpose-bound secret"),
                &mut ciphertext,
                &key,
                header,
            )
            .unwrap();
            for candidate in purposes {
                let mut output = Vec::new();
                let result =
                    decrypt_object(&mut Cursor::new(&ciphertext), &mut output, &key, candidate);
                if candidate == purpose {
                    result.unwrap();
                    assert_eq!(output, b"purpose-bound secret");
                } else {
                    assert!(matches!(
                        result,
                        Err(EncryptedObjectError::PurposeMismatch { .. })
                    ));
                    assert!(output.is_empty());
                }
            }
        }
    }

    struct BoundedReader {
        inner: Cursor<Vec<u8>>,
        largest_request: usize,
    }

    impl Read for BoundedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.largest_request = self.largest_request.max(buffer.len());
            self.inner.read(buffer)
        }
    }

    struct FailingWriter {
        remaining: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected capacity failure",
                ));
            }
            let written = self.remaining.min(bytes.len());
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn reports_partial_write_failure_as_io_error() {
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        let mut writer = FailingWriter {
            remaining: ENCRYPTED_OBJECT_HEADER_LENGTH + 10,
        };
        let error = encrypt_object(
            &mut Cursor::new(vec![0x33; ENCRYPTED_OBJECT_CHUNK_LENGTH]),
            &mut writer,
            &key,
            EncryptedObjectPurpose::Attachment,
        )
        .expect_err("injected write failure must fail the object");
        assert!(matches!(error, EncryptedObjectError::Io { .. }));
    }

    #[test]
    fn large_input_uses_bounded_reads() {
        let plaintext = vec![0x5a; ENCRYPTED_OBJECT_CHUNK_LENGTH * 20 + 3];
        let mut reader = BoundedReader {
            inner: Cursor::new(plaintext.clone()),
            largest_request: 0,
        };
        let key = RootEncryptionKey::new(TEST_KEY_BYTES);
        let header =
            Header::from_material(EncryptedObjectPurpose::Attachment, TEST_SALT, TEST_NONCE);
        let mut encrypted = Vec::new();
        encrypt_with_header(&mut reader, &mut encrypted, &key, header).unwrap();
        assert!(reader.largest_request <= ENCRYPTED_OBJECT_CHUNK_LENGTH);
        assert_eq!(
            decrypt_attachment(&encrypted, TEST_KEY_BYTES).unwrap(),
            plaintext
        );
    }
}
