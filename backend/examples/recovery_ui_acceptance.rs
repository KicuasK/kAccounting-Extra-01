//! Disposable authenticated recovery fixture. Never opens Keychain or real books.
//! Default: native Safari server; --self-check consumes the work without a listener.
use kaccounting_backend::{
    accounting::journal::{JournalDate, JournalRepository, SaveDraftEntry},
    attachments::AttachmentRepository,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    native_host::NativeHost,
    recipient_import::{import_recipient_package, recover_interrupted_recipient_imports},
    recipient_package::{export_recipient_package, recover_interrupted_recipient_exports},
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
fn unknown(parent: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    let suffix = if prefix.contains("recipient-export") {
        ".work"
    } else {
        ""
    };
    let path = parent.join(format!("{prefix}{}{suffix}", "f".repeat(32)));
    private_dir(&path)?;
    std::fs::write(path.join("sentinel"), b"preserve unknown fixture\n")?;
    Ok(path)
}
fn work_paths(parent: &Path, prefix: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut result = vec![];
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(|name| {
            name.starts_with(prefix)
                && (!prefix.contains("recipient-export") || name.ends_with(".work"))
        }) {
            result.push(entry.path())
        }
    }
    result.sort();
    Ok(result)
}
fn corrupt(path: &Path) -> std::io::Result<Vec<u8>> {
    let original = std::fs::read(path)?;
    let mut bad = original.clone();
    let Some(last) = bad.last_mut() else {
        return Err(std::io::Error::other("empty fixture ciphertext"));
    };
    *last ^= 1;
    std::fs::write(path, bad)?;
    Ok(original)
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
fn receipt(
    token: &str,
    imports: &[PathBuf],
    exports: &[PathBuf],
    tracked: &[(String, PathBuf, serde_json::Value)],
) -> std::io::Result<serde_json::Value> {
    let mut unchanged = true;
    let mut checks = vec![];
    for (label, path, before) in tracked {
        let after = snapshot(path)
            .unwrap_or_else(|_| serde_json::json!({"error":"tracked_file_unavailable"}));
        let same = before == &after;
        unchanged &= same;
        checks.push(
            serde_json::json!({"label":label,"before":before,"after":after,"unchanged":same}),
        );
    }
    let removed_imports = imports.iter().filter(|path| !path.exists()).count();
    let removed_exports = exports.iter().filter(|path| !path.exists()).count();
    Ok(
        serde_json::json!({"fixture_token":token,"fake_keys_only":true,"fixture_method":"authenticated injected failure and advisory marker loss; not SIGKILL","imports_owned_missing":removed_imports.to_string(),"exports_owned_missing":removed_exports.to_string(),"tracked_files":checks,"expected_owned_work_removed_and_content_preserved":removed_imports==2&&removed_exports==1&&unchanged,"scope":"filesystem after UI recovery; authenticated verified_count comes from observed UI job result"}),
    )
}

#[tokio::main]
async fn main() -> FixtureResult<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let self_check = args.len() == 1 && args[0] == "--self-check";
    if !self_check && args.len() != 2 {
        return Err(
            "usage: recovery_ui_acceptance <frontend-dist> <native-helper> | --self-check".into(),
        );
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random)?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("ka-recovery-ui-{token}"));
    private_dir(&root)?;
    let scratch = Scratch(root.clone());
    let collection = root.join("books");
    private_dir(&collection)?;
    let imports = root.join("import-recovery-parent");
    private_dir(&imports)?;
    let media = root.join("fixture-packages");
    private_dir(&media)?;
    let keys = Arc::new(FakeKeys::default());
    let source = collection.join("Recovery-Source-Book");
    RuntimeBook::init_new(&source, keys.as_ref()).await?;
    RuntimeBook::init_new(collection.join("Recovery-Control-Book"), keys.as_ref()).await?;
    let book = RuntimeBook::open_existing(&source, keys.as_ref()).await?;
    let draft = JournalRepository::new(book.pool().clone())
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23")?,
            vec![],
        ))
        .await?;
    let input = root.join("invented.txt");
    std::fs::write(
        &input,
        b"Invented authenticated recovery fixture. No financial data.\n",
    )?;
    let repository =
        AttachmentRepository::open(book.pool().clone(), book.attachments(), book.key())?;
    let attachment = repository
        .import_file(draft.id, &input, "invented.txt")
        .await?;
    std::fs::remove_file(input)?;
    drop(repository);
    let request = create_recipient_request(keys.as_ref())?;
    let package = export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        media.join("source-package.kagextra"),
    )
    .await?;

    // Genuine import owns encrypted recovery context before package authentication.
    // A deliberate ciphertext failure leaves a recoverable unpublished work tree.
    let package_database = package.path.join("book.kae");
    let original = corrupt(&package_database)?;
    let failed_import = import_recipient_package(
        &package.path,
        imports.join("Never-Published-Book"),
        &request,
        keys.as_ref(),
        keys.as_ref(),
    )
    .await;
    std::fs::write(&package_database, original)?;
    if failed_import.is_ok() {
        return Err("fixture corruption unexpectedly imported".into());
    }
    if imports.join("Never-Published-Book").exists() {
        return Err("failed fixture published a destination".into());
    }

    // Simulate process loss between durable publication and advisory marker.
    let published = imports.join("Verified-Published-Book");
    import_recipient_package(
        &package.path,
        &published,
        &request,
        keys.as_ref(),
        keys.as_ref(),
    )
    .await?;
    for work in work_paths(&imports, ".kAccounting-Extra-01-recipient-import-")? {
        let marker = work.join("published.json");
        if marker.exists() {
            std::fs::remove_file(marker)?;
        }
    }
    let import_unknown = unknown(&imports, ".kAccounting-Extra-01-recipient-import-")?;
    if work_paths(&imports, ".kAccounting-Extra-01-recipient-import-")?.len() != 3 {
        return Err("expected two owned import works and one unknown".into());
    }

    // Genuine export context and private snapshot precede attachment decrypt.
    // Restore the source bytes immediately after the deliberate export refusal.
    let object = source
        .join("attachments")
        .join(format!("{}.kae", attachment.id.as_str()));
    let original = corrupt(&object)?;
    let failed_export = export_recipient_package(
        book.pool(),
        book.attachments(),
        &book.key(),
        &request,
        media.join("Never-Published-Export.kagextra"),
    )
    .await;
    std::fs::write(&object, original)?;
    if failed_export.is_ok() {
        return Err("fixture corruption unexpectedly exported".into());
    }
    if media.join("Never-Published-Export.kagextra").exists() {
        return Err("failed fixture export published".into());
    }
    let export_unknown = unknown(&source, ".kAccounting-Extra-01-recipient-export-")?;
    if work_paths(&source, ".kAccounting-Extra-01-recipient-export-")?.len() != 2 {
        return Err("expected one owned export work and one unknown".into());
    }
    let import_owned: Vec<_> = work_paths(&imports, ".kAccounting-Extra-01-recipient-import-")?
        .into_iter()
        .filter(|path| path != &import_unknown)
        .collect();
    let export_owned: Vec<_> = work_paths(&source, ".kAccounting-Extra-01-recipient-export-")?
        .into_iter()
        .filter(|path| path != &export_unknown)
        .collect();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(book.pool())
        .await?;
    let source_key = book.key();
    book.close().await;
    drop(book);
    let mut tracked = vec![];
    for (label, path) in [
        ("import_unknown_sentinel", import_unknown.join("sentinel")),
        ("export_unknown_sentinel", export_unknown.join("sentinel")),
        ("published_book_database", published.join("book.sqlite3")),
        ("source_book_database", source.join("book.sqlite3")),
        ("source_attachment_ciphertext", object.clone()),
    ] {
        let before = snapshot(&path)?;
        tracked.push((label.to_owned(), path, before));
    }

    if self_check {
        let imports_result = recover_interrupted_recipient_imports(&imports, keys.as_ref()).await?;
        let exports_result = recover_interrupted_recipient_exports(&source, &source_key).await?;
        if imports_result.removed_count != 2
            || imports_result.verified_destinations != vec![published.clone()]
            || imports_result.unrecognized_work != vec![import_unknown.clone()]
            || !imports_result.active_work.is_empty()
        {
            return Err(format!("unexpected import recovery: {imports_result:?}").into());
        }
        if exports_result.removed_count != 1
            || exports_result.unrecognized_work != vec![export_unknown.clone()]
            || !exports_result.active_work.is_empty()
        {
            return Err(format!("unexpected export recovery: {exports_result:?}").into());
        }
        if std::fs::read(import_unknown.join("sentinel"))? != b"preserve unknown fixture\n"
            || std::fs::read(export_unknown.join("sentinel"))? != b"preserve unknown fixture\n"
        {
            return Err("unknown sentinel changed".into());
        }
        let verified = RuntimeBook::open_existing(&published, keys.as_ref()).await?;
        verified.close().await;
        let result = receipt(&token, &import_owned, &export_owned, &tracked)?;
        if result["expected_owned_work_removed_and_content_preserved"] != true {
            return Err("self-check receipt failed unchanged-content or removal checks".into());
        }
        println!("{result}");
        return Ok(());
    }
    let assets = root.join("frontend");
    copy_assets(&PathBuf::from(&args[0]).canonicalize()?, &assets)?;
    let session = SessionRuntime::new(&collection, &assets, keys.clone(), keys)?;
    use std::os::unix::fs::MetadataExt;
    let host = NativeHost::new(
        session,
        &PathBuf::from(&args[1]).canonicalize()?,
        std::fs::metadata(&root)?.uid(),
    )?;
    eprintln!("RECOVERY-FIXTURE-TOKEN: {token}");
    eprintln!("CANONICAL-FIXTURE-ROOT: {}", root.display());
    eprintln!("SELECT-BOOK: Recovery-Control-Book (explicit; never automatic)");
    eprintln!("IMPORT-RECOVERY-PARENT: {}", imports.display());
    eprintln!("EXPORT-SOURCE-BOOK: {}", source.display());
    eprintln!("EXPORT-RECOVERY-PARENT: {}", source.display());
    eprintln!(
        "EXPECTED: imports removed=2 verified=1 unknown=1; exports removed=1 verified=0 unknown=1; active=0"
    );
    eprintln!(
        "FIXTURE: authenticated injected failure/advisory-marker-loss states, not SIGKILL; fake memory keys only; Ctrl-C removes this unique fixture."
    );
    let outcome = serve_native_host(host).await;
    println!(
        "{}",
        receipt(&token, &import_owned, &export_owned, &tracked)?
    );
    outcome?;
    drop(scratch);
    Ok(())
}
