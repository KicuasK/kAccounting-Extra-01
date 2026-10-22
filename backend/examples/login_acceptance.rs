//! Disposable launchd server with reject-only fake keys and an empty collection.
//! Never links operating-system keychain adapters into the acceptance path.
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::RecipientKeyStore,
    runtime::serve_session,
    session_runtime::SessionRuntime,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zeroize::Zeroizing;

#[derive(Default)]
struct RejectKeys(AtomicUsize);
impl RejectKeys {
    fn reject<T>(&self) -> Result<T, KeyStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(KeyStoreError::Unavailable(-1))
    }
}
impl DeviceKeyStore for RejectKeys {
    fn read(&self, _: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reject()
    }
    fn insert_new(&self, _: &DeviceKeyId, _: &[u8]) -> Result<(), KeyStoreError> {
        self.reject()
    }
}
impl RecipientKeyStore for RejectKeys {
    fn read(&self, _: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.reject()
    }
    fn insert_new(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
        self.reject()
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [assets, books, token] = args.as_slice() else {
        return Err(
            "usage: login_acceptance <owned-assets> <empty-private-books> <fixture-token>".into(),
        );
    };
    let token = token.to_str().ok_or("invalid token")?;
    if token.len() != 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid fixture token".into());
    }
    let assets = PathBuf::from(assets);
    let books = PathBuf::from(books);
    let marker = format!("<!-- kAccounting-login-acceptance:{token} -->\n");
    let index = std::fs::read(assets.join("index.html"))?;
    if !index.starts_with(marker.as_bytes()) || std::fs::read_dir(&books)?.next().is_some() {
        return Err("acceptance requires owned marker and empty collection".into());
    }
    let keys = Arc::new(RejectKeys::default());
    let session = SessionRuntime::new(&books, &assets, keys.clone(), keys.clone())?;
    println!(
        "{}",
        serde_json::json!({"fixture_token": token, "key_calls": 0, "phase": "startup"})
    );
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = serve_session(session.clone()) => result?,
            _ = terminate.recv() => session.shutdown().await,
        }
    }
    #[cfg(not(unix))]
    serve_session(session).await?;
    let calls = keys.0.load(Ordering::SeqCst);
    println!(
        "{}",
        serde_json::json!({"fixture_token": token, "key_calls": calls, "phase": "shutdown"})
    );
    if calls != 0 {
        return Err("acceptance made an unexpected fake-key call".into());
    }
    Ok(())
}
