//! Disposable HTTP acceptance server. All keys and accounting data are invented.
//! No user Keychain item or existing financial book is ever opened.
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry},
        money::MinorUnits,
    },
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    runtime::{RuntimeBook, serve},
};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
use zeroize::Zeroizing;

#[derive(Default)]
struct TestKeys(Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>);
impl DeviceKeyStore for TestKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .map_err(|_| KeyStoreError::Unavailable(-1))?
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &DeviceKeyId, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().map_err(|_| KeyStoreError::Unavailable(-1))?;
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.clone(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
struct Disposable(PathBuf);
impl Drop for Disposable {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run --example local_acceptance -- <frontend-dist-directory>")?;
    let mut random = [0; 16];
    getrandom::fill(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("ka-http-acceptance-{token}"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&root)?;
    let fixture = Disposable(root);
    let keys = TestKeys::default();
    let path = RuntimeBook::init_new(fixture.0.join("book"), &keys).await?;
    let book = RuntimeBook::open_existing(path, &keys).await?;
    let accounts = AccountRepository::new(book.pool().clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            format!("驗收現金 Cash {token}"),
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await?;
    let equity = accounts
        .create(NewAccount::new(
            None,
            "驗收權益 Equity",
            AccountingElement::Equity,
            AccountKind::Posting,
        ))
        .await?;
    JournalRepository::new(book.pool().clone())
        .post(
            PostEntry::new(
                JournalDate::parse("2026-10-22")?,
                vec![
                    JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(1000)),
                    JournalLineInput::new(equity.id, JournalSide::Credit, MinorUnits::new(1000)),
                ],
            )
            .with_header(
                Some("ACCEPTANCE-ONLY".into()),
                Some("Disposable invented test data".into()),
            ),
        )
        .await?;
    eprintln!(
        "DISPOSABLE TEST SERVER http://127.0.0.1:3000 — no real financial data or Keychain; Control-C cleans the fixture."
    );
    eprintln!("FIXTURE-TOKEN: {token}");
    serve(&book, assets).await?;
    drop(book);
    drop(fixture);
    Ok(())
}
