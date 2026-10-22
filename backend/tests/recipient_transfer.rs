use kaccounting_backend::{
    crypto::{EncryptedObjectPurpose, decrypt_object, encrypt_object},
    keys::KeyStoreError,
    recipient_transfer::{
        RecipientEnvelope, RecipientKeyStore, RecipientRequest, RecipientTransferError,
        create_recipient_request, create_transfer_key, unwrap_transfer_key,
    },
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, io::Cursor};
use zeroize::Zeroizing;

#[derive(Default)]
struct MemoryKeys(RefCell<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for MemoryKeys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .borrow()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.borrow_mut();
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.to_owned(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
const PACKAGE: &str = "00000000000000000000000000000001";

#[test]
fn destination_recovers_transfer_root_and_decrypts_payload_without_exporting_secret() {
    let store = MemoryKeys::default();
    let request = create_recipient_request(&store).unwrap();
    let request_bytes = request.to_json().unwrap();
    let parsed = RecipientRequest::from_json(&request_bytes).unwrap();
    assert_eq!(request.request_id(), parsed.request_id());
    let (source_key, envelope) = create_transfer_key(&parsed, PACKAGE).unwrap();
    let public_envelope = envelope.to_json().unwrap();
    let envelope = RecipientEnvelope::from_json(&public_envelope).unwrap();
    let recipient_key = unwrap_transfer_key(&store, &request, PACKAGE, &envelope).unwrap();
    let mut ciphertext = Vec::new();
    encrypt_object(
        &mut Cursor::new(b"sensitive accounting snapshot"),
        &mut ciphertext,
        &source_key,
        EncryptedObjectPurpose::TransferDatabase,
    )
    .unwrap();
    let mut plaintext = Vec::new();
    decrypt_object(
        &mut Cursor::new(ciphertext),
        &mut plaintext,
        &recipient_key,
        EncryptedObjectPurpose::TransferDatabase,
    )
    .unwrap();
    assert_eq!(plaintext, b"sensitive accounting snapshot");
    let serialized = format!(
        "{}{}",
        String::from_utf8(request_bytes).unwrap(),
        String::from_utf8(public_envelope).unwrap()
    );
    let private = store.0.borrow()[request.request_id()].clone();
    let private_hex: String = private.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!serialized.contains(&private_hex));
    for forbidden in [
        "password",
        "passphrase",
        "pin",
        "private_key",
        "secret_key",
        "recovery_phrase",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert_eq!(store.0.borrow().len(), 1);
}

#[test]
fn strict_request_parser_rejects_unknown_duplicate_unsupported_and_noncanonical_fields() {
    let request = create_recipient_request(&MemoryKeys::default()).unwrap();
    let original: Value = serde_json::from_slice(&request.to_json().unwrap()).unwrap();
    for (field, replacement) in [
        ("version", json!(2)),
        ("format", json!("other")),
        ("request_id", json!("../escape")),
        ("public_key", json!("AB".repeat(32))),
        ("public_key_sha256", json!("0".repeat(64))),
        ("secret", json!("forbidden")),
    ] {
        let mut value = original.clone();
        value[field] = replacement;
        assert!(RecipientRequest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let text = String::from_utf8(request.to_json().unwrap()).unwrap();
    let duplicate = text.replacen('{', "{\"version\":1,", 1);
    assert!(RecipientRequest::from_json(duplicate.as_bytes()).is_err());
    assert!(RecipientRequest::from_json(&vec![b' '; 4097]).is_err());
    assert!(RecipientRequest::from_json(format!("{text}{{}}").as_bytes()).is_err());
}

#[test]
fn low_order_and_noncanonical_x25519_points_are_rejected_with_correct_fingerprints() {
    use sha2::{Digest, Sha256};
    let request = create_recipient_request(&MemoryKeys::default()).unwrap();
    for point in [[0; 32], [0xff; 32], {
        let mut p = [0; 32];
        p[0] = 1;
        p
    }] {
        let mut value: Value = serde_json::from_slice(&request.to_json().unwrap()).unwrap();
        value["public_key"] = json!(point.iter().map(|b| format!("{b:02x}")).collect::<String>());
        value["public_key_sha256"] = json!(
            Sha256::digest(point)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(RecipientRequest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn missing_wrong_and_malformed_private_keys_fail_without_replacement() {
    let store = MemoryKeys::default();
    let request = create_recipient_request(&store).unwrap();
    let (_, envelope) = create_transfer_key(&request, PACKAGE).unwrap();
    let missing = MemoryKeys::default();
    assert!(matches!(
        unwrap_transfer_key(&missing, &request, PACKAGE, &envelope),
        Err(RecipientTransferError::KeyStore(KeyStoreError::Missing))
    ));
    assert!(missing.0.borrow().is_empty());
    for bytes in [vec![8; 32], vec![8; 31]] {
        store.0.borrow_mut().insert(
            request.request_id().to_owned(),
            Zeroizing::new(bytes.clone()),
        );
        assert!(unwrap_transfer_key(&store, &request, PACKAGE, &envelope).is_err());
        assert_eq!(store.0.borrow()[request.request_id()].as_slice(), bytes);
    }
}

#[test]
fn envelope_tampering_wrong_recipient_and_cross_package_replay_fail_closed() {
    let store = MemoryKeys::default();
    let request = create_recipient_request(&store).unwrap();
    let other = create_recipient_request(&store).unwrap();
    let (_, envelope) = create_transfer_key(&request, PACKAGE).unwrap();
    assert!(unwrap_transfer_key(&store, &other, PACKAGE, &envelope).is_err());
    assert!(
        unwrap_transfer_key(
            &store,
            &request,
            "00000000000000000000000000000002",
            &envelope
        )
        .is_err()
    );
    let original: Value = serde_json::from_slice(&envelope.to_json().unwrap()).unwrap();
    for field in [
        "encapsulated_key",
        "wrapped_root_key",
        "request_id",
        "public_key_sha256",
        "package_id",
    ] {
        let mut value = original.clone();
        let text = value[field].as_str().unwrap();
        let replacement = format!(
            "{}{}",
            if text.starts_with('0') { "1" } else { "0" },
            &text[1..]
        );
        value[field] = json!(replacement);
        let modified = RecipientEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(unwrap_transfer_key(&store, &request, PACKAGE, &modified).is_err());
    }
    let (_, another) = create_transfer_key(&request, PACKAGE).unwrap();
    assert_ne!(envelope.to_json().unwrap(), another.to_json().unwrap());
}

#[test]
fn unavailable_key_store_does_not_return_a_public_request_or_leak_secret() {
    struct Denied;
    impl RecipientKeyStore for Denied {
        fn read(&self, _: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
            Err(KeyStoreError::Unavailable(-25308))
        }
        fn insert_new(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::Unavailable(-25308))
        }
    }
    let error = create_recipient_request(&Denied).unwrap_err();
    assert_eq!(
        error.to_string(),
        "device key storage is unavailable (system status -25308)"
    );
}
