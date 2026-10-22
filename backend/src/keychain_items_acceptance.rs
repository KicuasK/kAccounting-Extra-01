//! Acceptance-only guards: reads are limited to successful insertions this run.
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
};
use std::{cell::RefCell, collections::HashSet};
use zeroize::Zeroizing;

pub struct OwnedDeviceStore<S> {
    pub inner: S,
    owned: RefCell<HashSet<String>>,
}
impl<S> OwnedDeviceStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            owned: RefCell::new(HashSet::new()),
        }
    }
}
impl<S: DeviceKeyStore> DeviceKeyStore for OwnedDeviceStore<S> {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        if !self.owned.borrow().contains(id.as_str()) {
            return Err(KeyStoreError::InvalidIdentifier);
        }
        self.inner.read(id)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        if bytes.len() != 32 {
            return Err(KeyStoreError::MalformedSecret);
        }
        self.inner.insert_new(id, bytes)?;
        self.owned.borrow_mut().insert(id.as_str().to_owned());
        Ok(())
    }
}

pub struct OwnedRecipientStore<S> {
    pub inner: S,
    owned: RefCell<HashSet<String>>,
}
impl<S> OwnedRecipientStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            owned: RefCell::new(HashSet::new()),
        }
    }
}
impl<S: RecipientKeyStore> RecipientKeyStore for OwnedRecipientStore<S> {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        if !self.owned.borrow().contains(id) {
            return Err(KeyStoreError::InvalidIdentifier);
        }
        self.inner.read(id)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        DeviceKeyId::parse(id)?;
        if bytes.len() != 32 {
            return Err(KeyStoreError::MalformedSecret);
        }
        self.inner.insert_new(id, bytes)?;
        self.owned.borrow_mut().insert(id.to_owned());
        Ok(())
    }
}
