//! Disposable native directory-import fixture; fake memory keys only.
//! Default: native Safari server; --self-check consumes the work without a listener.
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry},
        money::MinorUnits,
    },
    attachments::AttachmentRepository,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    native_host::NativeHost,
    recipient_import::import_recipient_package,
    recipient_package::export_recipient_package,
    recipient_transfer::{RecipientKeyStore, create_recipient_request},
    runtime::{RuntimeBook, serve_native_host},
    session_runtime::SessionRuntime,
    storage::describe_file,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct FakeKeys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for FakeKeys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .map_err(|_| KeyStoreError::Unavailable(-1))?
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().map_err(|_| KeyStoreError::Unavailable(-1))?;
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for FakeKeys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, secret: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), secret)
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
type FixtureResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}
fn private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
        return Err(std::io::Error::other(
            "fixture file must have private 0600 permissions",
        ));
    }
    Ok(())
}
fn copy_assets(source: &Path, target: &Path) -> std::io::Result<()> {
    private_dir(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_symlink() {
            return Err(std::io::Error::other("symlink asset refused"));
        }
        if kind.is_dir() {
            copy_assets(&entry.path(), &destination)?;
        } else if kind.is_file() && entry.metadata()?.len() <= 8 * 1024 * 1024 {
            std::fs::copy(entry.path(), destination)?;
        } else {
            return Err(std::io::Error::other("invalid asset"));
        }
    }
    Ok(())
}

fn snapshot(path: &Path) -> std::io::Result<serde_json::Value> {
    use std::os::unix::fs::MetadataExt;
    let descriptor = describe_file(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(
        serde_json::json!({"device":metadata.dev().to_string(),"inode":metadata.ino().to_string(),"bytes":descriptor.byte_length().to_string(),"sha256":descriptor.digest().to_hex()}),
    )
}

fn tracked_files(
    root: &Path,
    label: &str,
    result: &mut Vec<(String, PathBuf, serde_json::Value)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            tracked_files(&entry.path(), label, result)?;
        } else if entry.file_type()?.is_file() {
            result.push((
                format!("{label}/{}", entry.file_name().to_string_lossy()),
                entry.path(),
                snapshot(&entry.path())?,
            ));
        } else {
            return Err(std::io::Error::other("unexpected fixture symlink"));
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> FixtureResult<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let self_check = args.len() == 1 && args[0] == "--self-check";
    if !self_check && args.len() != 2 {
        return Err(
            "usage: directory_ui_acceptance <frontend-dist> <native-helper> | --self-check".into(),
        );
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("ka-directory-ui-{token}"));
    private_dir(&root)?;
    let scratch = Scratch(root.clone());
    let collection = root.join("books");
    private_dir(&collection)?;
    let media = root.join("transfer-media");
    private_dir(&media)?;
    let destination = collection.join("Imported-Directory-Book");
    let keys = Arc::new(FakeKeys::default());
    RuntimeBook::init_new(collection.join("Directory-Control-Book"), keys.as_ref()).await?;
    let source = collection.join("Directory-Source-Book");
    RuntimeBook::init_new(&source, keys.as_ref()).await?;
    let book = RuntimeBook::open_existing(&source, keys.as_ref()).await?;
    let accounts = AccountRepository::new(book.pool().clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "目錄移轉驗收資產",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await?;
    let equity = accounts
        .create(NewAccount::new(
            None,
            "目錄移轉驗收權益",
            AccountingElement::Equity,
            AccountKind::Posting,
        ))
        .await?;
    let entry = JournalRepository::new(book.pool().clone())
        .post(
            PostEntry::new(
                JournalDate::parse("2026-10-23")?,
                vec![
                    JournalLineInput::new(
                        cash.id,
                        JournalSide::Debit,
                        MinorUnits::new(9_007_199_254_740_993),
                    ),
                    JournalLineInput::new(
                        equity.id,
                        JournalSide::Credit,
                        MinorUnits::new(9_007_199_254_740_993),
                    ),
                ],
            )
            .with_header(
                Some("DIRECTORY-ACCEPTANCE-ONLY".into()),
                Some("Invented data; not a real financial book".into()),
            ),
        )
        .await?;
    let plaintext = b"Invented native directory transfer attachment. No real financial data.\n";
    let input = root.join("invented-directory-receipt.txt");
    private_file(&input, plaintext)?;
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key())?;
    let attachment = repository
        .import_file(entry.id, &input, "invented-directory-receipt.txt")
        .await?;
    std::fs::remove_file(&input)?;
    drop(repository);
    let request = create_recipient_request(keys.as_ref())?;
    let request_path = media.join("directory-recipient-request.json");
    private_file(&request_path, &request.to_json()?)?;
    let package = export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        media.join("Directory-Transfer.kagextra"),
    )
    .await?;
    let source_key_id = book.key_id().clone();
    let source_key = DeviceKeyStore::read(keys.as_ref(), &source_key_id)?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(book.pool())
        .await?;
    book.close().await;
    drop(book);
    let mut tracked = vec![];
    for (label, path) in [
        ("source_database", source.join("book.sqlite3")),
        (
            "source_attachment",
            source
                .join("attachments")
                .join(format!("{}.kae", attachment.id.as_str())),
        ),
    ] {
        tracked.push((label.to_owned(), path.clone(), snapshot(&path)?));
    }
    tracked_files(&media, "transfer_media", &mut tracked)?;
    if self_check {
        import_recipient_package(
            &package.path,
            &destination,
            &request,
            keys.as_ref(),
            keys.as_ref(),
        )
        .await?;
    } else {
        let assets = root.join("frontend");
        copy_assets(&PathBuf::from(&args[0]).canonicalize()?, &assets)?;
        let session = SessionRuntime::new(&collection, &assets, keys.clone(), keys.clone())?;
        use std::os::unix::fs::MetadataExt;
        let host = NativeHost::new(
            session,
            &PathBuf::from(&args[1]).canonicalize()?,
            std::fs::metadata(&root)?.uid(),
        )?;
        eprintln!(
            "DIRECTORY-FIXTURE-TOKEN: {token}\nCANONICAL-FIXTURE-ROOT: {}\nSELECT-BOOK: Directory-Control-Book\nREQUEST-FILE: {}\nPACKAGE-DIRECTORY: {}\nNEW-BOOK-DESTINATION: {}\nEXPECTED: attachment_count=1; posted amount=9007199254740993; source and package preserved; fake memory keys only; explicit native selections required.",
            root.display(),
            request_path.display(),
            package.path.display(),
            destination.display()
        );
        serve_native_host(host).await?;
    }
    let mut checks = vec![];
    let mut unchanged = true;
    for (label, path, before) in tracked {
        let after = snapshot(&path)?;
        let same = before == after;
        unchanged &= same;
        checks.push(
            serde_json::json!({"label":label,"before":before,"after":after,"unchanged":same}),
        );
    }
    let imported = RuntimeBook::open_existing(&destination, keys.as_ref()).await?;
    let imported_entry = JournalRepository::new(imported.pool().clone())
        .get_posted(entry.id)
        .await?;
    let exact = imported_entry == entry;
    let fresh_key = imported.key_id() != &source_key_id
        && DeviceKeyStore::read(keys.as_ref(), imported.key_id())? != source_key;
    let repository = AttachmentRepository::open(
        imported.pool().clone(),
        imported.attachments(),
        imported.key(),
    )?;
    let (metadata, mut chunks) = repository.download_stream(&attachment.id).await?;
    let mut received = vec![];
    while let Some(chunk) = chunks.recv().await {
        received.extend_from_slice(&chunk?);
    }
    let content = received == plaintext
        && metadata.plaintext == attachment.plaintext
        && metadata.ciphertext != attachment.ciphertext;
    imported.close().await;
    let success = unchanged && exact && fresh_key && content;
    println!(
        "{}",
        serde_json::json!({"fixture_token":token,"fake_keys_only":true,"mode":if self_check{"self_check_not_Safari"}else{"native_Safari_fixture"},"tracked_files":checks,"posted_entry_exact":exact,"posted_amount":"9007199254740993","fresh_device_key":fresh_key,"attachment_plaintext_verified":content,"all_checks_passed":success})
    );
    if !success {
        return Err("directory fixture verification failed".into());
    }
    drop(scratch);
    Ok(())
}
