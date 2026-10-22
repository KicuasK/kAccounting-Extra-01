//! Passwordless recipient requests and RFC 9180 destination-bound root-key wrapping.
//!
//! This is a cryptographic foundation, not a complete `.kagextra` export/import
//! workflow. Base mode authenticates context and ciphertext, not sender identity.
use crate::{crypto::RootEncryptionKey, keys::KeyStoreError};
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::ChaCha20Poly1305, kdf::HkdfSha256,
    kem::X25519HkdfSha256, rand_core::SeedableRng,
};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

type SuiteKem = X25519HkdfSha256;
type PrivateKey = <SuiteKem as Kem>::PrivateKey;
type PublicKey = <SuiteKem as Kem>::PublicKey;
type EncappedKey = <SuiteKem as Kem>::EncappedKey;
const REQUEST_FORMAT: &str = "kAccounting-Extra-01.recipient-request";
const ENVELOPE_FORMAT: &str = "kAccounting-Extra-01.recipient-envelope";
const SUITE: &str = "DHKEM-X25519-HKDF-SHA256/HKDF-SHA256/ChaCha20Poly1305";
const VERSION: u32 = 1;
const MAX_PUBLIC_DOCUMENT_BYTES: usize = 4096;

#[derive(Debug, Error)]
pub enum RecipientTransferError {
    #[error("recipient request or envelope is malformed or unsupported")]
    InvalidDocument,
    #[error("recipient request or package context does not match")]
    ContextMismatch,
    #[error("recipient key wrapping authentication failed")]
    AuthenticationFailed,
    #[error("operating-system random generation is unavailable")]
    RandomUnavailable,
    #[error(transparent)]
    KeyStore(#[from] KeyStoreError),
}

/// Separate key-store namespace from local backup root keys. Implementations
/// must store create-only, non-synchronizing secrets; there is no delete method.
pub trait RecipientKeyStore {
    fn read(&self, request_id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError>;
    fn insert_new(&self, request_id: &str, secret: &[u8]) -> Result<(), KeyStoreError>;
}

#[derive(Clone, Debug, Serialize)]
pub struct RecipientRequest(RequestDocument);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestDocument {
    format: String,
    version: u32,
    request_id: String,
    public_key: String,
    public_key_sha256: String,
}

impl RecipientRequest {
    pub fn from_json(bytes: &[u8]) -> Result<Self, RecipientTransferError> {
        let request = Self(parse_document(bytes)?);
        request.validate()?;
        Ok(request)
    }
    pub fn to_json(&self) -> Result<Vec<u8>, RecipientTransferError> {
        serde_json::to_vec(&self.0).map_err(|_| RecipientTransferError::InvalidDocument)
    }
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }
    pub fn fingerprint(&self) -> &str {
        &self.0.public_key_sha256
    }
    fn validate(&self) -> Result<PublicKey, RecipientTransferError> {
        if self.0.format != REQUEST_FORMAT || self.0.version != VERSION {
            return Err(RecipientTransferError::InvalidDocument);
        }
        decode_hex::<16>(&self.0.request_id)?;
        let bytes = decode_hex::<32>(&self.0.public_key)?;
        if digest_hex(&bytes) != self.0.public_key_sha256 {
            return Err(RecipientTransferError::InvalidDocument);
        }
        // X25519 accepts aliases; public documents require one canonical field
        // element encoding below 2^255-19, and reject low-order points through
        // the standard KEM's all-zero DH check using a public validation probe.
        let mut prime = [0xff; 32];
        prime[0] = 0xed;
        prime[31] = 0x7f;
        if bytes.iter().rev().cmp(prime.iter().rev()) != std::cmp::Ordering::Less {
            return Err(RecipientTransferError::InvalidDocument);
        }
        let public =
            PublicKey::from_bytes(&bytes).map_err(|_| RecipientTransferError::InvalidDocument)?;
        let (probe, _) =
            SuiteKem::derive_keypair(b"kAccounting-Extra-01/public-point-validation/v1");
        let enc =
            EncappedKey::from_bytes(&bytes).map_err(|_| RecipientTransferError::InvalidDocument)?;
        SuiteKem::decap(&probe, None, &enc).map_err(|_| RecipientTransferError::InvalidDocument)?;
        Ok(public)
    }
}

/// Generates an OS-random destination key, stores the private key first, and
/// returns only public material. Store failures never publish a request.
pub fn create_recipient_request(
    store: &impl RecipientKeyStore,
) -> Result<RecipientRequest, RecipientTransferError> {
    let mut ikm = Zeroizing::new([0; 32]);
    getrandom::fill(ikm.as_mut()).map_err(|_| RecipientTransferError::RandomUnavailable)?;
    let (secret, public) = SuiteKem::derive_keypair(ikm.as_ref());
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| RecipientTransferError::RandomUnavailable)?;
    let request_id = hex(&id);
    let mut stored = Zeroizing::new([0; 32]);
    secret.write_exact(stored.as_mut());
    store.insert_new(&request_id, stored.as_ref())?;
    Ok(RecipientRequest(RequestDocument {
        format: REQUEST_FORMAT.to_owned(),
        version: VERSION,
        request_id,
        public_key: hex(&public.to_bytes()),
        public_key_sha256: digest_hex(&public.to_bytes()),
    }))
}

#[derive(Clone, Debug, Serialize)]
pub struct RecipientEnvelope(EnvelopeDocument);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeDocument {
    format: String,
    version: u32,
    suite: String,
    package_id: String,
    request_id: String,
    public_key_sha256: String,
    encapsulated_key: String,
    wrapped_root_key: String,
}

impl RecipientEnvelope {
    pub fn from_json(bytes: &[u8]) -> Result<Self, RecipientTransferError> {
        let envelope = Self(parse_document(bytes)?);
        envelope.validate()?;
        Ok(envelope)
    }
    pub fn to_json(&self) -> Result<Vec<u8>, RecipientTransferError> {
        serde_json::to_vec(&self.0).map_err(|_| RecipientTransferError::InvalidDocument)
    }
    fn validate(&self) -> Result<(), RecipientTransferError> {
        if self.0.format != ENVELOPE_FORMAT || self.0.version != VERSION || self.0.suite != SUITE {
            return Err(RecipientTransferError::InvalidDocument);
        }
        decode_hex::<16>(&self.0.package_id)?;
        decode_hex::<16>(&self.0.request_id)?;
        decode_hex::<32>(&self.0.public_key_sha256)?;
        decode_hex::<32>(&self.0.encapsulated_key)?;
        decode_hex::<48>(&self.0.wrapped_root_key)?;
        Ok(())
    }
}

/// The caller supplies a unique canonical 128-bit package ID and must bind this
/// ID in the encrypted package manifest as well. No plaintext key is serialized.
pub fn create_transfer_key(
    request: &RecipientRequest,
    package_id: &str,
) -> Result<(RootEncryptionKey, RecipientEnvelope), RecipientTransferError> {
    let public = request.validate()?;
    decode_hex::<16>(package_id)?;
    let mut material = Zeroizing::new([0; 32]);
    getrandom::fill(material.as_mut()).map_err(|_| RecipientTransferError::RandomUnavailable)?;
    let root = RootEncryptionKey::new(*material);
    let mut seed = Zeroizing::new([0; 32]);
    getrandom::fill(seed.as_mut()).map_err(|_| RecipientTransferError::RandomUnavailable)?;
    let mut rng = ChaCha20Rng::from_seed(*seed);
    let context = context(request, package_id);
    let (enc, mut sender) = hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, SuiteKem, _>(
        &OpModeS::Base,
        &public,
        &context,
        &mut rng,
    )
    .map_err(|_| RecipientTransferError::AuthenticationFailed)?;
    let wrapped = sender
        .seal(material.as_ref(), &context)
        .map_err(|_| RecipientTransferError::AuthenticationFailed)?;
    Ok((
        root,
        RecipientEnvelope(EnvelopeDocument {
            format: ENVELOPE_FORMAT.to_owned(),
            version: VERSION,
            suite: SUITE.to_owned(),
            package_id: package_id.to_owned(),
            request_id: request.request_id().to_owned(),
            public_key_sha256: request.fingerprint().to_owned(),
            encapsulated_key: hex(&enc.to_bytes()),
            wrapped_root_key: hex(&wrapped),
        }),
    ))
}

pub fn unwrap_transfer_key(
    store: &impl RecipientKeyStore,
    request: &RecipientRequest,
    expected_package_id: &str,
    envelope: &RecipientEnvelope,
) -> Result<RootEncryptionKey, RecipientTransferError> {
    let public = request.validate()?;
    decode_hex::<16>(expected_package_id)?;
    envelope.validate()?;
    if envelope.0.package_id != expected_package_id
        || envelope.0.request_id != request.request_id()
        || envelope.0.public_key_sha256 != request.fingerprint()
    {
        return Err(RecipientTransferError::ContextMismatch);
    }
    let bytes = store.read(request.request_id())?;
    if bytes.len() != 32 {
        return Err(KeyStoreError::MalformedSecret.into());
    }
    let secret =
        PrivateKey::from_bytes(&bytes).map_err(|_| RecipientTransferError::AuthenticationFailed)?;
    if SuiteKem::sk_to_pk(&secret) != public {
        return Err(RecipientTransferError::AuthenticationFailed);
    }
    let enc = EncappedKey::from_bytes(&decode_hex::<32>(&envelope.0.encapsulated_key)?)
        .map_err(|_| RecipientTransferError::InvalidDocument)?;
    let context = context(request, expected_package_id);
    let mut receiver = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, SuiteKem>(
        &OpModeR::Base,
        &secret,
        &enc,
        &context,
    )
    .map_err(|_| RecipientTransferError::AuthenticationFailed)?;
    let plaintext = Zeroizing::new(
        receiver
            .open(&decode_hex::<48>(&envelope.0.wrapped_root_key)?, &context)
            .map_err(|_| RecipientTransferError::AuthenticationFailed)?,
    );
    if plaintext.len() != 32 {
        return Err(RecipientTransferError::AuthenticationFailed);
    }
    let mut material = Zeroizing::new([0; 32]);
    material.copy_from_slice(&plaintext);
    Ok(RootEncryptionKey::new(*material))
}

fn context(request: &RecipientRequest, package_id: &str) -> Vec<u8> {
    // All variable fields have validated, fixed-length lowercase hex encodings.
    format!(
        "kAccounting-Extra-01/recipient-root-wrap/v1\0{SUITE}\0{}\0{}\0{package_id}",
        request.request_id(),
        request.fingerprint()
    )
    .into_bytes()
}

fn parse_document<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, RecipientTransferError> {
    if bytes.len() > MAX_PUBLIC_DOCUMENT_BYTES {
        return Err(RecipientTransferError::InvalidDocument);
    }
    serde_json::from_slice(bytes).map_err(|_| RecipientTransferError::InvalidDocument)
}
fn digest_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], RecipientTransferError> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(RecipientTransferError::InvalidDocument);
    }
    let mut out = [0; N];
    for (i, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        out[i] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
/// Non-synchronizing Keychain recipient namespace, distinct from device roots.
pub struct MacRecipientKeychain;
#[cfg(target_os = "macos")]
impl RecipientKeyStore for MacRecipientKeychain {
    fn read(&self, request_id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        use security_framework::passwords::{PasswordOptions, generic_password};
        crate::keys::DeviceKeyId::parse(request_id)?;
        let mut options = PasswordOptions::new_generic_password(
            "kAccounting-Extra-01.recipient-private.v1",
            request_id,
        );
        options.set_access_synchronized(Some(false));
        generic_password(options)
            .map(Zeroizing::new)
            .map_err(|e| keychain_error(e.code()))
    }
    fn insert_new(&self, request_id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
        crate::keys::DeviceKeyId::parse(request_id)?;
        if secret.len() != 32 {
            return Err(KeyStoreError::MalformedSecret);
        }
        struct SecretBuffer(Zeroizing<Vec<u8>>);
        impl AsRef<[u8]> for SecretBuffer {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }
        let data = CFData::from_arc(std::sync::Arc::new(SecretBuffer(Zeroizing::new(
            secret.to_vec(),
        ))));
        ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data,
        })
        .set_service("kAccounting-Extra-01.recipient-private.v1")
        .set_account_name(request_id)
        .set_label("kAccounting-Extra-01 transfer recipient private key")
        .set_location(Location::DefaultFileKeychain)
        .add()
        .map_err(|e| keychain_error(e.code()))
    }
}
#[cfg(target_os = "macos")]
fn keychain_error(status: i32) -> KeyStoreError {
    match status {
        -25300 => KeyStoreError::Missing,
        -25299 => KeyStoreError::AlreadyExists,
        other => KeyStoreError::Unavailable(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 9180 reference vectors: mode=0, kem_id=32, kdf_id=1, aead_id=3.
    // https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/test-vectors.json
    #[test]
    fn selected_suite_opens_rfc_9180_reference_vector() {
        let secret = PrivateKey::from_bytes(
            &decode_hex::<32>("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb")
                .unwrap(),
        )
        .unwrap();
        let enc = EncappedKey::from_bytes(
            &decode_hex::<32>("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            hex(&SuiteKem::sk_to_pk(&secret).to_bytes()),
            "4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a"
        );
        let mut receiver = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, SuiteKem>(
            &OpModeR::Base,
            &secret,
            &enc,
            b"Ode on a Grecian Urn",
        )
        .unwrap();
        let ciphertext = decode_hex::<45>(
            "1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db21993c62ce81883d2dd1b51a28",
        ).unwrap();
        let plaintext = receiver.open(&ciphertext, b"Count-0").unwrap();
        assert_eq!(plaintext, b"Beauty is truth, truth beauty");
    }
}
