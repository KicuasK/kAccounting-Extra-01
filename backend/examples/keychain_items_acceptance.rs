//! Opt-in live acceptance; leaves precisely two newly created items for inspection.
#[path = "../src/keychain_items_acceptance.rs"]
mod guards;

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use guards::{OwnedDeviceStore, OwnedRecipientStore};
    use kaccounting_backend::{
        crypto::{EncryptedObjectPurpose, RootEncryptionKey, decrypt_object, encrypt_object},
        keys::{DeviceKeyStore, KeyStoreError, MacKeychain, create_device_key, load_device_key},
        recipient_transfer::{
            MacRecipientKeychain, RecipientKeyStore, RecipientTransferError,
            create_recipient_request, create_transfer_key, unwrap_transfer_key,
        },
    };
    use std::io::Cursor;
    fn equal_keys(
        a: &RootEncryptionKey,
        b: &RootEncryptionKey,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut encrypted = Vec::new();
        encrypt_object(
            &mut Cursor::new(b"dedicated acceptance probe"),
            &mut encrypted,
            a,
            EncryptedObjectPurpose::Attachment,
        )?;
        let mut plaintext = Vec::new();
        decrypt_object(
            &mut Cursor::new(encrypted),
            &mut plaintext,
            b,
            EncryptedObjectPurpose::Attachment,
        )?;
        Ok(plaintext == b"dedicated acceptance probe")
    }
    let device = OwnedDeviceStore::new(MacKeychain);
    let (id, created) = create_device_key(&device)?;
    // Emit each public identifier immediately, preserving inspection metadata if a later operation fails.
    println!(
        "device_service=kAccounting-Extra-01.device-root.v1 device_item_id={id} retained=true"
    );
    let loaded = load_device_key(&device, &id)?;
    if !equal_keys(&created, &loaded)? {
        return Err("device key round trip failed".into());
    }
    if device.insert_new(&id, &[0; 32]) != Err(KeyStoreError::AlreadyExists) {
        return Err("device create-only check failed".into());
    }
    if device.insert_new(&id, &[0; 31]) != Err(KeyStoreError::MalformedSecret) {
        return Err("device malformed guard failed".into());
    }
    println!("device_round_trip=true device_create_only=true device_malformed_guard=true");
    let recipient = OwnedRecipientStore::new(MacRecipientKeychain);
    let request = create_recipient_request(&recipient)?;
    println!(
        "recipient_service=kAccounting-Extra-01.recipient-private.v1 recipient_item_id={} fingerprint={} retained=true",
        request.request_id(),
        request.fingerprint()
    );
    let package = "1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a";
    let (root, envelope) = create_transfer_key(&request, package)?;
    let unwrapped = unwrap_transfer_key(&recipient, &request, package, &envelope)?;
    if !equal_keys(&root, &unwrapped)? {
        return Err("recipient round trip failed".into());
    }
    if recipient.insert_new(request.request_id(), &[0; 32]) != Err(KeyStoreError::AlreadyExists) {
        return Err("recipient create-only check failed".into());
    }
    if recipient.insert_new(request.request_id(), &[0; 31]) != Err(KeyStoreError::MalformedSecret) {
        return Err("recipient malformed guard failed".into());
    }
    if !matches!(
        unwrap_transfer_key(
            &recipient,
            &request,
            "2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b",
            &envelope
        ),
        Err(RecipientTransferError::ContextMismatch)
    ) {
        return Err("wrong context accepted".into());
    }
    println!(
        "recipient_round_trip=true recipient_create_only=true recipient_malformed_guard=true wrong_context_rejected=true"
    );
    Ok(())
}
fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--allow-new-test-items"] {
        eprintln!(
            "Requires exactly --allow-new-test-items; creates two dedicated random Keychain items and retains them."
        );
        std::process::exit(2);
    }
    #[cfg(target_os = "macos")]
    if let Err(error) = run() {
        eprintln!("Live acceptance incomplete; already listed items remain retained. {error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Live acceptance requires macOS");
        std::process::exit(2);
    }
}
