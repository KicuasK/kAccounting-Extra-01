#[path = "../src/keychain_items_acceptance.rs"]
mod guards;
use guards::{OwnedDeviceStore, OwnedRecipientStore};
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
};
use std::cell::Cell;
use zeroize::Zeroizing;
#[derive(Default)]
struct Fake {
    calls: Cell<usize>,
    fail: Cell<bool>,
}
impl Fake {
    fn insert(&self) -> Result<(), KeyStoreError> {
        self.calls.set(self.calls.get() + 1);
        if self.fail.get() {
            Err(KeyStoreError::AlreadyExists)
        } else {
            Ok(())
        }
    }
    fn get(&self) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.calls.set(self.calls.get() + 1);
        Ok(Zeroizing::new(vec![7; 32]))
    }
}
impl DeviceKeyStore for Fake {
    fn read(&self, _: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.get()
    }
    fn insert_new(&self, _: &DeviceKeyId, _: &[u8]) -> Result<(), KeyStoreError> {
        self.insert()
    }
}
impl RecipientKeyStore for Fake {
    fn read(&self, _: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.get()
    }
    fn insert_new(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
        self.insert()
    }
}
#[test]
fn device_guard_rejects_unowned_reads_failed_creations_and_malformed_insert_before_store() {
    let store = OwnedDeviceStore::new(Fake::default());
    let id = DeviceKeyId::parse(&"a".repeat(32)).unwrap();
    assert_eq!(store.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(
        store.insert_new(&id, &[0; 31]),
        Err(KeyStoreError::MalformedSecret)
    );
    assert_eq!(store.inner.calls.get(), 0);
    store.inner.fail.set(true);
    assert_eq!(
        store.insert_new(&id, &[0; 32]),
        Err(KeyStoreError::AlreadyExists)
    );
    assert_eq!(store.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(store.inner.calls.get(), 1);
    store.inner.fail.set(false);
    store.insert_new(&id, &[0; 32]).unwrap();
    assert_eq!(store.read(&id).unwrap().len(), 32);
    let other_run = OwnedDeviceStore::new(Fake::default());
    assert_eq!(other_run.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(other_run.inner.calls.get(), 0);
}
#[test]
fn recipient_guard_validates_and_only_reads_successfully_created_ids_this_run() {
    let store = OwnedRecipientStore::new(Fake::default());
    let id = "b".repeat(32);
    assert_eq!(store.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(
        store.insert_new("invalid", &[0; 32]),
        Err(KeyStoreError::InvalidIdentifier)
    );
    assert_eq!(
        store.insert_new(&id, &[0; 31]),
        Err(KeyStoreError::MalformedSecret)
    );
    assert_eq!(store.inner.calls.get(), 0);
    store.inner.fail.set(true);
    assert_eq!(
        store.insert_new(&id, &[0; 32]),
        Err(KeyStoreError::AlreadyExists)
    );
    assert_eq!(store.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(store.inner.calls.get(), 1);
    store.inner.fail.set(false);
    store.insert_new(&id, &[0; 32]).unwrap();
    assert_eq!(store.read(&id).unwrap().len(), 32);
    let other_run = OwnedRecipientStore::new(Fake::default());
    assert_eq!(other_run.read(&id), Err(KeyStoreError::InvalidIdentifier));
    assert_eq!(other_run.inner.calls.get(), 0);
}
