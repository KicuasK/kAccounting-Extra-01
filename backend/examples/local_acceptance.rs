//! Disposable HTTP acceptance server. All keys and accounting data are invented.
//! No user Keychain item or existing financial book is ever opened.
use kaccounting_backend::{
    accounting::{
        accounts::{AccountKind, AccountRepository, AccountingElement, NewAccount},
        journal::{JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry},
        money::MinorUnits,
    },
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    recipient_transfer::{RecipientKeyStore, create_recipient_request},
    runtime::{RuntimeBook, serve, serve_private_session, serve_session},
    session_runtime::SessionRuntime,
    storage_operations::SelectionKind,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct TestKeys(Mutex<HashMap<DeviceKeyId, Zeroizing<Vec<u8>>>>);
#[derive(Default)]
struct TestRecipients(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for TestRecipients {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .map_err(|_| KeyStoreError::Unavailable(-1))?
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, bytes: &[u8]) -> Result<(), KeyStoreError> {
        let mut values = self.0.lock().map_err(|_| KeyStoreError::Unavailable(-1))?;
        if values.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        values.insert(id.to_owned(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
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

struct Options {
    assets: PathBuf,
    chooser: bool,
    transfer: bool,
    private_config: Option<PathBuf>,
    ui: bool,
    native_helper: Option<PathBuf>,
    legacy_restore: bool,
    planning: bool,
}
fn parse_options(args: Vec<std::ffi::OsString>) -> Result<Options, &'static str> {
    let Some(assets) = args.first() else {
        return Err(
            "usage: local_acceptance <frontend-dist> [--chooser | --chooser-ui [--native-helper <helper>] | --chooser-legacy-ui [--native-helper <helper>] | --chooser-planning-ui [--native-helper <helper>] | --chooser-transfer [--private-config <external-config>]]",
        );
    };
    let (chooser, transfer, private_config) = match &args[1..] {
        [] => (false, false, None),
        [mode]
            if mode == "--chooser"
                || mode == "--chooser-ui"
                || mode == "--chooser-legacy-ui"
                || mode == "--chooser-planning-ui" =>
        {
            (true, false, None)
        }
        [mode, flag, _path]
            if (mode == "--chooser-ui"
                || mode == "--chooser-legacy-ui"
                || mode == "--chooser-planning-ui")
                && flag == "--native-helper" =>
        {
            (true, false, None)
        }
        [mode] if mode == "--chooser-transfer" => (true, true, None),
        [mode, flag, path] if mode == "--chooser-transfer" && flag == "--private-config" => {
            (true, true, Some(PathBuf::from(path)))
        }
        _ => return Err("unknown, duplicate or incomplete acceptance options"),
    };
    Ok(Options {
        assets: PathBuf::from(assets),
        chooser,
        transfer,
        private_config,
        ui: args.get(1).is_some_and(|mode| {
            mode == "--chooser-ui"
                || mode == "--chooser-legacy-ui"
                || mode == "--chooser-planning-ui"
        }),
        planning: args
            .get(1)
            .is_some_and(|mode| mode == "--chooser-planning-ui"),
        legacy_restore: args
            .get(1)
            .is_some_and(|mode| mode == "--chooser-legacy-ui"),
        native_helper: if args.get(2).is_some_and(|flag| flag == "--native-helper") {
            args.get(3).map(PathBuf::from)
        } else {
            None
        },
    })
}
fn copy_assets(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if entry.file_type()?.is_symlink() {
            return Err(std::io::Error::other("symlink acceptance asset refused"));
        }
        if metadata.is_dir() {
            copy_assets(&entry.path(), &destination.join(entry.file_name()))?;
        } else if metadata.is_file() && metadata.len() <= 8 * 1024 * 1024 {
            std::fs::copy(entry.path(), destination.join(entry.file_name()))?;
        } else {
            return Err(std::io::Error::other(
                "invalid or oversized acceptance asset",
            ));
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        assets,
        chooser,
        transfer,
        private_config,
        ui,
        native_helper,
        legacy_restore,
        planning,
    } = parse_options(std::env::args_os().skip(1).collect())?;
    // Validate external private configuration before creating any fixture/key.
    let private = private_config
        .map(|path| {
            kaccounting_backend::private_runtime_config::PrivateRuntimeConfig::load(path)
                .map(|config| config.into_parts())
        })
        .transpose()?;
    let assets = assets.canonicalize()?;
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
    let served_assets = fixture.0.join("served-assets");
    copy_assets(&assets, &served_assets)?;
    let index_path = served_assets.join("index.html");
    let mut index = format!("<!-- kAccounting-acceptance:{token} -->\n").into_bytes();
    index.extend(std::fs::read(&index_path)?);
    std::fs::write(index_path, index)?;
    let assets = served_assets;
    let keys = Arc::new(TestKeys::default());
    let path = RuntimeBook::init_new(fixture.0.join("book"), keys.as_ref()).await?;
    let book = RuntimeBook::open_existing(path, keys.as_ref()).await?;
    let accounts = AccountRepository::new(book.pool().clone());
    let parent = if ui {
        Some(
            accounts
                .create(NewAccount::new(
                    None,
                    "驗收資產 Assets",
                    AccountingElement::Asset,
                    AccountKind::Category,
                ))
                .await?
                .id,
        )
    } else {
        None
    };
    let cash = accounts
        .create(NewAccount::new(
            parent,
            format!("驗收現金 Cash {token}"),
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await?;
    if ui {
        let inactive = accounts
            .create(NewAccount::new(
                parent,
                "驗收停用科目",
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await?;
        accounts
            .set_active(inactive.id, inactive.revision, false)
            .await?;
        for (name, element) in [
            ("驗收負債", AccountingElement::Liability),
            ("驗收收益", AccountingElement::Income),
            ("驗收費損", AccountingElement::Expense),
        ] {
            accounts
                .create(NewAccount::new(None, name, element, AccountKind::Posting))
                .await?;
        }
        RuntimeBook::init_new(fixture.0.join("empty"), keys.as_ref()).await?;
    }
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
                    JournalLineInput::new(
                        cash.id,
                        JournalSide::Debit,
                        MinorUnits::new(if ui { 9_007_199_254_740_993 } else { 1000 }),
                    ),
                    JournalLineInput::new(
                        equity.id,
                        JournalSide::Credit,
                        MinorUnits::new(if ui { 9_007_199_254_740_993 } else { 1000 }),
                    ),
                ],
            )
            .with_header(
                Some("ACCEPTANCE-ONLY".into()),
                Some("Disposable invented test data".into()),
            ),
        )
        .await?;
    if planning {
        // Separate fictitious AR/AP accounts avoid classifying the cash opening
        // line as a receivable merely to exercise the user interface.
        let ar = accounts
            .create(NewAccount::new(
                None,
                "驗收應收",
                AccountingElement::Asset,
                AccountKind::Posting,
            ))
            .await?;
        let ap = accounts
            .create(NewAccount::new(
                None,
                "驗收應付",
                AccountingElement::Liability,
                AccountKind::Posting,
            ))
            .await?;
        let expense = accounts
            .create(NewAccount::new(
                None,
                "驗收服務費用",
                AccountingElement::Expense,
                AccountKind::Posting,
            ))
            .await?;
        let income = accounts
            .create(NewAccount::new(
                None,
                "驗收服務收益",
                AccountingElement::Income,
                AccountKind::Posting,
            ))
            .await?;
        for (label, debit, credit, amount) in [
            ("AR", ar.id, income.id, 500),
            ("AP", expense.id, ap.id, 300),
            ("AR-RECEIPT", cash.id, ar.id, 100),
            ("AP-PAYMENT", ap.id, cash.id, 75),
        ] {
            let entry = JournalRepository::new(book.pool().clone())
                .post(
                    PostEntry::new(
                        JournalDate::parse("2026-10-23")?,
                        vec![
                            JournalLineInput::new(
                                debit,
                                JournalSide::Debit,
                                MinorUnits::new(amount),
                            ),
                            JournalLineInput::new(
                                credit,
                                JournalSide::Credit,
                                MinorUnits::new(amount),
                            ),
                        ],
                    )
                    .with_header(
                        Some(format!("TEST-{label}")),
                        Some("Disposable planning acceptance only".into()),
                    ),
                )
                .await?;
            eprintln!("PLANNING-{label}: entry={} amount={amount}", entry.id.get());
        }
    }
    if legacy_restore {
        // Exercise the real old backup/restore format, which intentionally does
        // not confer the source book's managed-retention namespace.
        let backup = fixture.0.join("legacy-source.kabackup");
        kaccounting_backend::book_backup::create_local_backup(
            book.pool(),
            book.attachments(),
            &backup,
            book.key_id(),
            keys.as_ref(),
        )
        .await?;
        let restored = fixture.0.join("legacy-restored");
        kaccounting_backend::book_backup::restore_local_backup(&backup, &restored, keys.as_ref())
            .await?;
        if restored.join("backup-identity.kae").try_exists()? {
            return Err("legacy acceptance unexpectedly inherited a backup identity".into());
        }
        eprintln!(
            "LEGACY-RESTORED-ID: legacy-restored — actual verified backup/restore; no managed backup identity"
        );
    }
    eprintln!(
        "DISPOSABLE TEST SERVER {} — no real financial data or Keychain; Control-C cleans the fixture.",
        if private.is_some() {
            "private Unix ingress"
        } else {
            "http://127.0.0.1:3000"
        }
    );
    eprintln!("FIXTURE-TOKEN: {token}");
    if chooser {
        drop(book);
        let recipients = Arc::new(TestRecipients::default());
        let session = SessionRuntime::new(&fixture.0, &assets, keys, recipients.clone())?;
        if transfer {
            let request = create_recipient_request(recipients.as_ref())?;
            let request_path = fixture.0.join("recipient-request.json");
            std::fs::write(&request_path, request.to_json()?)?;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::set_permissions(&request_path, std::fs::Permissions::from_mode(0o600))?;
            let capability_path = fixture.0.join("transfer-capabilities.json");
            eprintln!("TRANSFER-CAPABILITIES: {}", capability_path.display());
            let registration_session = session.clone();
            let root = fixture.0.clone();
            let token = token.clone();
            tokio::spawn(async move {
                let book = loop {
                    if let Ok(id) = registration_session
                        .register_storage_selection(SelectionKind::Book, root.join("book"))
                        .await
                    {
                        break id;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                };
                let mut capabilities = serde_json::json!({"fixture_token":token,"book":book,"confirmed_request_id":request.request_id(),"confirmed_fingerprint":request.fingerprint()});
                for (field, kind, path) in [
                    ("request", SelectionKind::Request, request_path),
                    (
                        "destination",
                        SelectionKind::ExportDestination,
                        root.join("export"),
                    ),
                    (
                        "import_destination",
                        SelectionKind::BookDestination,
                        root.join("imported"),
                    ),
                ] {
                    let Ok(id) = registration_session
                        .register_storage_selection(kind, path)
                        .await
                    else {
                        return;
                    };
                    capabilities[field] = serde_json::Value::String(id);
                }
                // Public fixture capabilities only; never key bytes or real paths.
                let Ok(mut file) = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(capability_path)
                else {
                    return;
                };
                use std::io::Write;
                let _ = file.write_all(&serde_json::to_vec(&capabilities).unwrap_or_default());
            });
        }
        eprintln!("CHOOSER: explicit selection required; disposable book_id=book");
        if let Some((policy, socket)) = private {
            serve_private_session(session, policy, socket).await?;
        } else if let Some(helper) = native_helper {
            use std::os::unix::fs::MetadataExt;
            let host = kaccounting_backend::native_host::NativeHost::new(
                session,
                &helper,
                std::fs::metadata(&fixture.0)?.uid(),
            )?;
            kaccounting_backend::runtime::serve_native_host(host).await?;
        } else {
            serve_session(session).await?;
        }
    } else {
        serve(&book, assets).await?;
        drop(book);
    }
    drop(fixture);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Result<Options, &'static str> {
        parse_options(args.iter().map(std::ffi::OsString::from).collect())
    }
    #[test]
    fn modes_are_explicit_and_private_requires_transfer() {
        assert!(!options(&["/invented/assets"]).unwrap().chooser);
        assert!(options(&["/invented/assets", "--chooser"]).unwrap().chooser);
        assert!(options(&["/invented/assets", "--chooser-ui"]).unwrap().ui);
        let planning = options(&["/invented/assets", "--chooser-planning-ui"]).unwrap();
        assert!(planning.ui && planning.chooser && planning.planning && !planning.legacy_restore);
        let legacy = options(&[
            "/invented/assets",
            "--chooser-legacy-ui",
            "--native-helper",
            "/private/helper",
        ])
        .unwrap();
        assert!(legacy.ui && legacy.chooser && legacy.legacy_restore);
        assert_eq!(legacy.native_helper, Some(PathBuf::from("/private/helper")));
        assert!(
            options(&["/invented/assets", "--chooser-transfer"])
                .unwrap()
                .transfer
        );
        assert_eq!(
            options(&[
                "/invented/assets",
                "--chooser-transfer",
                "--private-config",
                "/external/config.json"
            ])
            .unwrap()
            .private_config,
            Some(PathBuf::from("/external/config.json"))
        );
        for args in [
            vec![],
            vec!["/assets", "--unknown"],
            vec!["/assets", "--chooser", "extra"],
            vec!["/assets", "--chooser", "--private-config", "/config"],
            vec!["/assets", "--chooser-transfer", "--private-config"],
            vec![
                "/assets",
                "--chooser-transfer",
                "--private-config",
                "/config",
                "extra",
            ],
        ] {
            assert!(options(&args).is_err());
        }
    }
}
