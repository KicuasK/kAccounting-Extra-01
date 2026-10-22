//! Disposable negative Unix-ingress acceptance. No real keys, books, or Tailscale.
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    private_access::PrivateAccessPolicy,
    recipient_transfer::RecipientKeyStore,
    runtime::serve_private_session,
    session_runtime::SessionRuntime,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zeroize::Zeroizing;
#[derive(Default)]
struct NeverKeys(AtomicUsize);
impl DeviceKeyStore for NeverKeys {
    fn read(&self, _: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(KeyStoreError::Missing)
    }
    fn insert_new(&self, _: &DeviceKeyId, _: &[u8]) -> Result<(), KeyStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(KeyStoreError::Missing)
    }
}
impl RecipientKeyStore for NeverKeys {
    fn read(&self, _: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(KeyStoreError::Missing)
    }
    fn insert_new(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(KeyStoreError::Missing)
    }
}
struct Disposable {
    root: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for Disposable {
    fn drop(&mut self) {
        if let Ok(actual) = fs::symlink_metadata(&self.root)
            && actual.is_dir()
            && !actual.file_type().is_symlink()
            && actual.dev() == self.device
            && actual.ino() == self.inode
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("no arguments are accepted by this disposable fixture".into());
    }
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce)?;
    let id: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let root = PathBuf::from("/private/tmp").join(format!("ka-r3-proxy-{id}"));
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::symlink_metadata(&root)?;
    let fixture = Disposable {
        root,
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let books = fixture.root.join("books");
    let assets = fixture.root.join("assets");
    for path in [&books, &assets] {
        fs::create_dir(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    fs::write(
        assets.join("index.html"),
        "<!doctype html>Disposable private-ingress fixture",
    )?;
    let keys = Arc::new(NeverKeys::default());
    let session = SessionRuntime::new(&books, &assets, keys.clone(), keys.clone())?;
    let policy = PrivateAccessPolicy::new("book.example.ts.net", "owner@example.com")?;
    let socket = fixture.root.join("proxy.sock");
    eprintln!(
        "DISPOSABLE NEGATIVE UNIX FIXTURE — empty books, fake stores, no real keys or Tailscale"
    );
    eprintln!("UNIX-SOCKET: {}", socket.display());
    eprintln!("Only non-root denial is intended; root/TLS positive acceptance is not established.");
    serve_private_session(session, policy, socket).await?;
    if keys.0.load(Ordering::SeqCst) != 0 {
        return Err("negative ingress unexpectedly accessed a fake key store".into());
    }
    eprintln!(
        "KEY-STORE-OPERATIONS: 0; graceful shutdown complete; removing owned disposable fixture"
    );
    drop(fixture);
    Ok(())
}
