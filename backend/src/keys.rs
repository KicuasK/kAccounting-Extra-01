//! Passwordless device-key boundary. Reading an existing key never generates,
//! replaces, repairs, exports, or deletes one. Tests use only an in-memory store.
use crate::crypto::RootEncryptionKey;
use std::fmt;
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeviceKeyId(String);
impl DeviceKeyId {
    pub fn parse(value: &str) -> Result<Self, KeyStoreError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(KeyStoreError::InvalidIdentifier);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn generate() -> Result<Self, KeyStoreError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| KeyStoreError::RandomUnavailable)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }
}
impl fmt::Display for DeviceKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum KeyStoreError {
    #[error("device key identifier is invalid")]
    InvalidIdentifier,
    #[error("the required device key is missing; preserve the book and recover its original key")]
    Missing,
    #[error("a device key with this identifier already exists")]
    AlreadyExists,
    #[error("device key storage is unavailable (system status {0})")]
    Unavailable(i32),
    #[error("stored device key has an invalid length; it was not changed")]
    MalformedSecret,
    #[error("operating-system random generation is unavailable")]
    RandomUnavailable,
}

/// Minimal persistence contract: read and create-only insertion, with no update
/// or deletion operation that could silently destroy encrypted-book access.
pub trait DeviceKeyStore {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError>;
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError>;
}

/// Explicit first-book initialization. The key is returned only after secure
/// storage confirms create-only insertion. No secret is written to a file.
pub fn create_device_key(
    store: &impl DeviceKeyStore,
) -> Result<(DeviceKeyId, RootEncryptionKey), KeyStoreError> {
    let id = DeviceKeyId::generate()?;
    let mut bytes = Zeroizing::new([0; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| KeyStoreError::RandomUnavailable)?;
    store.insert_new(&id, bytes.as_ref())?;
    Ok((id, RootEncryptionKey::new(*bytes)))
}

/// Loads precisely the requested existing key. Failure never falls back to a
/// generated key or a password, and malformed secret bytes are zeroized on drop.
pub fn load_device_key(
    store: &impl DeviceKeyStore,
    id: &DeviceKeyId,
) -> Result<RootEncryptionKey, KeyStoreError> {
    let bytes = store.read(id)?;
    if bytes.len() != 32 {
        return Err(KeyStoreError::MalformedSecret);
    }
    let mut array = Zeroizing::new([0; 32]);
    array.copy_from_slice(&bytes);
    Ok(RootEncryptionKey::new(*array))
}

#[cfg(target_os = "macos")]
/// Non-synchronizing local macOS Keychain adapter. No shell commands, secret
/// arguments, user-defined passwords, or application key files are involved.
pub struct MacKeychain;
#[cfg(target_os = "macos")]
impl DeviceKeyStore for MacKeychain {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        use security_framework::passwords::{PasswordOptions, generic_password};
        let mut options = PasswordOptions::new_generic_password(
            "kAccounting-Extra-01.device-root.v1",
            id.as_str(),
        );
        options.set_access_synchronized(Some(false));
        generic_password(options)
            .map(Zeroizing::new)
            .map_err(|error| map_system_status(error.code()))
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
        if bytes.len() != 32 {
            return Err(KeyStoreError::MalformedSecret);
        }
        // Keep the CFData backing allocation zeroizing as well. SecItemAdd
        // receives an opaque byte buffer, never a textual/command-line secret.
        struct SecretBuffer(Zeroizing<Vec<u8>>);
        impl AsRef<[u8]> for SecretBuffer {
            fn as_ref(&self) -> &[u8] {
                self.0.as_slice()
            }
        }
        let data = CFData::from_arc(std::sync::Arc::new(SecretBuffer(Zeroizing::new(
            bytes.to_vec(),
        ))));
        ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data,
        })
        .set_service("kAccounting-Extra-01.device-root.v1")
        .set_account_name(id.as_str())
        .set_label("kAccounting-Extra-01 device encryption key")
        // File-keychain items created without synchronizable attributes
        // are local; reads additionally require synchronizable=false.
        .set_location(Location::DefaultFileKeychain)
        .add()
        .map_err(|error| map_system_status(error.code()))
    }
}
#[cfg(target_os = "macos")]
fn map_system_status(status: i32) -> KeyStoreError {
    match status {
        -25300 => KeyStoreError::Missing,
        -25299 => KeyStoreError::AlreadyExists,
        other => KeyStoreError::Unavailable(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{EncryptedObjectPurpose, decrypt_object, encrypt_object};
    use std::{cell::RefCell, collections::HashMap, io::Cursor};
    #[derive(Default)]
    struct MemoryStore {
        values: RefCell<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>,
        inserts: RefCell<usize>,
    }
    impl DeviceKeyStore for MemoryStore {
        fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
            self.values
                .borrow()
                .get(id)
                .cloned()
                .ok_or(KeyStoreError::Missing)
        }
        fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
            let mut values = self.values.borrow_mut();
            if values.contains_key(id) {
                return Err(KeyStoreError::AlreadyExists);
            }
            values.insert(id.clone(), Zeroizing::new(bytes.to_vec()));
            *self.inserts.borrow_mut() += 1;
            Ok(())
        }
    }
    #[test]
    fn explicit_creation_and_load_recover_the_same_key_without_replacing_it() {
        let store = MemoryStore::default();
        let (id, created) = create_device_key(&store).unwrap();
        let loaded = load_device_key(&store, &id).unwrap();
        let mut encrypted = Vec::new();
        encrypt_object(
            &mut Cursor::new(b"private evidence"),
            &mut encrypted,
            &created,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        let mut plaintext = Vec::new();
        decrypt_object(
            &mut Cursor::new(encrypted),
            &mut plaintext,
            &loaded,
            EncryptedObjectPurpose::Attachment,
        )
        .unwrap();
        assert_eq!(plaintext, b"private evidence");
        assert_eq!(*store.inserts.borrow(), 1);
        assert_eq!(
            store.insert_new(&id, &[0; 32]),
            Err(KeyStoreError::AlreadyExists)
        );
        assert_eq!(*store.inserts.borrow(), 1);
    }
    #[test]
    fn lost_or_malformed_keys_never_trigger_generation_repair_or_replacement() {
        let store = MemoryStore::default();
        let id = DeviceKeyId::parse(&"a".repeat(32)).unwrap();
        assert!(matches!(
            load_device_key(&store, &id),
            Err(KeyStoreError::Missing)
        ));
        store
            .values
            .borrow_mut()
            .insert(id.clone(), Zeroizing::new(vec![1; 31]));
        assert!(matches!(
            load_device_key(&store, &id),
            Err(KeyStoreError::MalformedSecret)
        ));
        assert_eq!(store.values.borrow()[&id].len(), 31);
        assert_eq!(*store.inserts.borrow(), 0);
    }
    #[test]
    fn unavailable_store_fails_closed_and_error_text_has_no_key_bytes() {
        struct Denied;
        impl DeviceKeyStore for Denied {
            fn read(&self, _: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
                Err(KeyStoreError::Unavailable(-25308))
            }
            fn insert_new(&self, _: &DeviceKeyId, _: &[u8]) -> Result<(), KeyStoreError> {
                Err(KeyStoreError::Unavailable(-25308))
            }
        }
        let id = DeviceKeyId::parse(&"0".repeat(32)).unwrap();
        assert!(matches!(
            load_device_key(&Denied, &id),
            Err(KeyStoreError::Unavailable(-25308))
        ));
        assert!(matches!(
            create_device_key(&Denied),
            Err(KeyStoreError::Unavailable(-25308))
        ));
        assert_eq!(
            KeyStoreError::Unavailable(-25308).to_string(),
            "device key storage is unavailable (system status -25308)"
        );
    }
    #[test]
    fn key_identifiers_are_canonical_public_random_identifiers() {
        for bad in [
            "",
            "../key",
            "A0000000000000000000000000000000",
            "0000000000000000000000000000000g",
        ] {
            assert!(DeviceKeyId::parse(bad).is_err());
        }
        let store = MemoryStore::default();
        let (a, _) = create_device_key(&store).unwrap();
        let (b, _) = create_device_key(&store).unwrap();
        assert_ne!(a, b);
        assert_eq!(DeviceKeyId::parse(a.as_str()).unwrap(), a);
    }
}
