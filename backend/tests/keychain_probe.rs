//! Contract tests use only memory: no default/login or disposable OS keychain.
use kaccounting_backend::{
    keychain_probe::{KeychainProbeBlocker, preflight},
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, load_device_key},
    recipient_transfer::RecipientKeyStore,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, hash_map::Entry},
};
use zeroize::Zeroizing;
type MemoryItems = HashMap<(bool, String), Zeroizing<Vec<u8>>>;

#[derive(Default)]
struct MemoryKeychain {
    values: RefCell<MemoryItems>,
    locked: Cell<bool>,
}
impl MemoryKeychain {
    fn read_namespace(
        &self,
        recipient: bool,
        id: &str,
    ) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        if self.locked.get() {
            return Err(KeyStoreError::Unavailable(-25308));
        }
        self.values
            .borrow()
            .get(&(recipient, id.to_owned()))
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_namespace(
        &self,
        recipient: bool,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), KeyStoreError> {
        if self.locked.get() {
            return Err(KeyStoreError::Unavailable(-25308));
        }
        if bytes.len() != 32 {
            return Err(KeyStoreError::MalformedSecret);
        }
        match self.values.borrow_mut().entry((recipient, id.to_owned())) {
            Entry::Occupied(_) => Err(KeyStoreError::AlreadyExists),
            Entry::Vacant(entry) => {
                entry.insert(Zeroizing::new(bytes.to_vec()));
                Ok(())
            }
        }
    }
}
impl DeviceKeyStore for MemoryKeychain {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.read_namespace(false, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.insert_namespace(false, id.as_str(), bytes)
    }
}
impl RecipientKeyStore for MemoryKeychain {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.read_namespace(true, id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        self.insert_namespace(true, id, bytes)
    }
}
const ID: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn preflight_never_authorizes_os_calls() {
    assert_eq!(
        preflight(false, true),
        Err(KeychainProbeBlocker::NotOptedIn)
    );
    assert_eq!(
        preflight(true, false),
        Err(KeychainProbeBlocker::UnsupportedPlatform)
    );
    assert_eq!(
        preflight(true, true),
        Err(KeychainProbeBlocker::IsolationUnverified)
    );
}

#[test]
fn namespaces_do_not_fall_back_to_each_other() {
    let store = MemoryKeychain::default();
    let id = DeviceKeyId::parse(ID).unwrap();
    DeviceKeyStore::insert_new(&store, &id, &[1; 32]).unwrap();
    assert_eq!(
        RecipientKeyStore::read(&store, ID),
        Err(KeyStoreError::Missing)
    );
    RecipientKeyStore::insert_new(&store, ID, &[2; 32]).unwrap();
    assert_eq!(&*DeviceKeyStore::read(&store, &id).unwrap(), &[1; 32]);
    assert_eq!(&*RecipientKeyStore::read(&store, ID).unwrap(), &[2; 32]);
}

#[test]
fn duplicate_insert_never_replaces_either_secret() {
    let store = MemoryKeychain::default();
    let id = DeviceKeyId::parse(ID).unwrap();
    DeviceKeyStore::insert_new(&store, &id, &[1; 32]).unwrap();
    RecipientKeyStore::insert_new(&store, ID, &[2; 32]).unwrap();
    assert_eq!(
        DeviceKeyStore::insert_new(&store, &id, &[3; 32]),
        Err(KeyStoreError::AlreadyExists)
    );
    assert_eq!(
        RecipientKeyStore::insert_new(&store, ID, &[4; 32]),
        Err(KeyStoreError::AlreadyExists)
    );
    assert_eq!(&*DeviceKeyStore::read(&store, &id).unwrap(), &[1; 32]);
    assert_eq!(&*RecipientKeyStore::read(&store, ID).unwrap(), &[2; 32]);
}

#[test]
fn missing_and_malformed_device_loads_never_repair_storage() {
    let store = MemoryKeychain::default();
    let id = DeviceKeyId::parse(ID).unwrap();
    assert!(matches!(
        load_device_key(&store, &id),
        Err(KeyStoreError::Missing)
    ));
    assert!(store.values.borrow().is_empty());
    store
        .values
        .borrow_mut()
        .insert((false, ID.to_owned()), Zeroizing::new(vec![7; 31]));
    assert!(matches!(
        load_device_key(&store, &id),
        Err(KeyStoreError::MalformedSecret)
    ));
    assert_eq!(&*DeviceKeyStore::read(&store, &id).unwrap(), &[7; 31]);
}

#[test]
fn malformed_insert_never_changes_either_namespace() {
    let store = MemoryKeychain::default();
    let id = DeviceKeyId::parse(ID).unwrap();
    assert_eq!(
        DeviceKeyStore::insert_new(&store, &id, &[1; 31]),
        Err(KeyStoreError::MalformedSecret)
    );
    assert_eq!(
        RecipientKeyStore::insert_new(&store, ID, &[2; 33]),
        Err(KeyStoreError::MalformedSecret)
    );
    assert!(store.values.borrow().is_empty());
}

#[test]
fn locked_errors_preserve_both_secrets_and_unlock_recovers_originals() {
    let store = MemoryKeychain::default();
    let id = DeviceKeyId::parse(ID).unwrap();
    DeviceKeyStore::insert_new(&store, &id, &[1; 32]).unwrap();
    RecipientKeyStore::insert_new(&store, ID, &[2; 32]).unwrap();
    store.locked.set(true);
    assert_eq!(
        DeviceKeyStore::read(&store, &id),
        Err(KeyStoreError::Unavailable(-25308))
    );
    assert_eq!(
        RecipientKeyStore::read(&store, ID),
        Err(KeyStoreError::Unavailable(-25308))
    );
    assert_eq!(
        DeviceKeyStore::insert_new(&store, &id, &[3; 32]),
        Err(KeyStoreError::Unavailable(-25308))
    );
    assert_eq!(
        RecipientKeyStore::insert_new(&store, ID, &[4; 32]),
        Err(KeyStoreError::Unavailable(-25308))
    );
    store.locked.set(false);
    assert_eq!(&*DeviceKeyStore::read(&store, &id).unwrap(), &[1; 32]);
    assert_eq!(&*RecipientKeyStore::read(&store, ID).unwrap(), &[2; 32]);
}

#[test]
#[ignore = "explicit opt-in preflight; real Keychain isolation remains unverified"]
fn real_macos_disposable_keychain_acceptance_preflight() {
    let opted_in =
        std::env::var(kaccounting_backend::keychain_probe::OPT_IN_VARIABLE).as_deref() == Ok("1");
    // A blocker is a failing acceptance result, never a success/skip claim.
    preflight(opted_in, cfg!(target_os = "macos"))
        .expect("OS Keychain acceptance blocked before any OS call");
}

#[test]
fn recipient_unwrap_failures_never_generate_or_repair_a_secret() {
    use kaccounting_backend::recipient_transfer::{
        RecipientTransferError, create_recipient_request, create_transfer_key, unwrap_transfer_key,
    };
    let store = MemoryKeychain::default();
    let request = create_recipient_request(&store).unwrap();
    let (_, envelope) = create_transfer_key(&request, ID).unwrap();
    let original = RecipientKeyStore::read(&store, request.request_id()).unwrap();
    store.locked.set(true);
    assert!(matches!(
        unwrap_transfer_key(&store, &request, ID, &envelope),
        Err(RecipientTransferError::KeyStore(
            KeyStoreError::Unavailable(-25308)
        ))
    ));
    assert!(matches!(
        create_recipient_request(&store),
        Err(RecipientTransferError::KeyStore(
            KeyStoreError::Unavailable(-25308)
        ))
    ));
    store.locked.set(false);
    assert_eq!(
        RecipientKeyStore::read(&store, request.request_id()).unwrap(),
        original
    );
    assert!(unwrap_transfer_key(&store, &request, ID, &envelope).is_ok());
    store
        .values
        .borrow_mut()
        .remove(&(true, request.request_id().to_owned()));
    assert!(matches!(
        unwrap_transfer_key(&store, &request, ID, &envelope),
        Err(RecipientTransferError::KeyStore(KeyStoreError::Missing))
    ));
    assert!(store.values.borrow().is_empty());
    store.values.borrow_mut().insert(
        (true, request.request_id().to_owned()),
        Zeroizing::new(vec![7; 31]),
    );
    assert!(matches!(
        unwrap_transfer_key(&store, &request, ID, &envelope),
        Err(RecipientTransferError::KeyStore(
            KeyStoreError::MalformedSecret
        ))
    ));
    assert_eq!(
        &*RecipientKeyStore::read(&store, request.request_id()).unwrap(),
        &[7; 31]
    );
}
