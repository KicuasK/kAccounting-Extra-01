#![forbid(unsafe_code)]

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("kAccounting-Extra-01: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    use kaccounting_backend::{
        application_commands,
        keys::MacKeychain,
        recipient_transfer::MacRecipientKeychain,
        runtime::{RuntimeBook, serve},
    };
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [command, parent, assets, helper] if command == "serve-native-chooser" => {
            let session = kaccounting_backend::session_runtime::SessionRuntime::new(
                std::path::Path::new(parent), std::path::Path::new(assets),
                std::sync::Arc::new(MacKeychain), std::sync::Arc::new(MacRecipientKeychain),
            )?;
            let host = kaccounting_backend::native_host::NativeHost::new(
                session, std::path::Path::new(helper), rustix::process::geteuid().as_raw(),
            )?;
            eprintln!("kAccounting-Extra-01: http://127.0.0.1:3000 — explicit book selection required; trusted native helper pinned, no panel opened");
            kaccounting_backend::runtime::serve_native_host(host).await?;
            Ok(())
        }
        [command, parent, assets, configuration, helper] if command == "serve-private-native-chooser" => {
            let (policy, socket) = kaccounting_backend::private_runtime_config::PrivateRuntimeConfig::load(
                std::path::Path::new(configuration),
            )?.into_parts();
            let session = kaccounting_backend::session_runtime::SessionRuntime::new(
                std::path::Path::new(parent), std::path::Path::new(assets),
                std::sync::Arc::new(MacKeychain), std::sync::Arc::new(MacRecipientKeychain),
            )?;
            let host = kaccounting_backend::native_host::NativeHost::new(
                session, std::path::Path::new(helper), rustix::process::geteuid().as_raw(),
            )?;
            eprintln!("kAccounting-Extra-01: private Unix ingress only; explicit book selection required; trusted native helper pinned, no panel opened");
            kaccounting_backend::runtime::serve_private_native_host(host, policy, socket).await?;
            Ok(())
        }
        [command, parent, assets, configuration] if command == "serve-private-chooser" => {
            let (policy, socket) = kaccounting_backend::private_runtime_config::PrivateRuntimeConfig::load(
                std::path::Path::new(configuration),
            )?.into_parts();
            let session = kaccounting_backend::session_runtime::SessionRuntime::new(
                std::path::Path::new(parent), std::path::Path::new(assets),
                std::sync::Arc::new(MacKeychain), std::sync::Arc::new(MacRecipientKeychain),
            )?;
            eprintln!("kAccounting-Extra-01: private Unix session requested; explicit book selection required; Tailscale is not configured by this command");
            kaccounting_backend::runtime::serve_private_session(session, policy, socket).await?;
            Ok(())
        }
        [command, parent, assets] if command == "serve-chooser" => {
            let session = kaccounting_backend::session_runtime::SessionRuntime::new(
                std::path::Path::new(parent), std::path::Path::new(assets),
                std::sync::Arc::new(MacKeychain), std::sync::Arc::new(MacRecipientKeychain),
            )?;
            eprintln!("kAccounting-Extra-01: http://127.0.0.1:3000 — explicit book selection required");
            kaccounting_backend::runtime::serve_session(session).await?;
            Ok(())
        }
        [command, parent] if command == "books" => {
            let books = kaccounting_backend::book_selection::discover_books(std::path::Path::new(parent))?;
            println!("{}", serde_json::to_string(&books)?);
            Ok(())
        }
        [command, source, request, destination] if command == "export-recipient" => {
            let request = application_commands::read_recipient_request(std::path::Path::new(request))?;
            eprintln!("Recipient request ID: {}\nRecipient fingerprint: {}\nCompare both with the destination Mac. Type the request ID to confirm export:", request.request_id(), request.fingerprint());
            let mut confirmation = String::new();
            std::io::stdin().read_line(&mut confirmation)?;
            if confirmation.trim() != request.request_id() { return Err("recipient confirmation did not match; nothing exported".into()); }
            let result = application_commands::export_recipient_book(std::path::Path::new(source), &request, std::path::Path::new(destination), &MacKeychain).await?;
            println!("Verified encrypted package: {}\nPackage ID: {}\nAttachments: {}", result.path.display(), result.package_id, result.attachment_count);
            if let Some(warning) = result.cleanup_warning { eprintln!("Export completed; cleanup warning: {warning}"); }
            Ok(())
        }
        [command, package, request, destination] if command == "import-recipient" => {
            let request = application_commands::read_recipient_request(std::path::Path::new(request))?;
            let result = application_commands::import_recipient_book(std::path::Path::new(package), &request, std::path::Path::new(destination), &MacRecipientKeychain, &MacKeychain).await?;
            println!("Verified new book: {}\nPackage ID: {}\nChoose this book separately to open it.", result.path.display(), result.package_id);
            Ok(())
        }
        [command, source, parent] if command == "recover-recipient-exports" => {
            let result = application_commands::recover_recipient_exports(std::path::Path::new(source), std::path::Path::new(parent), &MacKeychain).await?;
            println!("Removed completed work: {}; verified destinations: {}; active work: {}; unrecognized preserved: {}", result.removed_count, result.verified_destinations.len(), result.active_work.len(), result.unrecognized_work.len());
            Ok(())
        }
        [command, parent] if command == "recover-recipient-imports" => {
            let result = kaccounting_backend::recipient_import::recover_interrupted_recipient_imports(std::path::Path::new(parent), &MacKeychain).await?;
            println!("Recovered work: {}; verified destinations: {}; active work: {}; unrecognized preserved: {}", result.removed_count, result.verified_destinations.len(), result.active_work.len(), result.unrecognized_work.len());
            Ok(())
        }
        [command, parent] if command == "import-status" => {
            let result = kaccounting_backend::recipient_import::inspect_interrupted_recipient_imports(std::path::Path::new(parent))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        [command, source, checkpoint, destination] if command == "restore-pre-migration" => {
            let result = application_commands::restore_pre_migration(std::path::Path::new(source), std::path::Path::new(checkpoint), std::path::Path::new(destination), &MacKeychain).await?;
            println!("Verified restored book: {}\nChoose this book separately to open it.", result.path.display());
            if let Some(warning) = result.cleanup_warning { eprintln!("Restore completed; cleanup warning: {warning}"); }
            Ok(())
        }
        [command, path] if command == "init-new" => {
            let path = RuntimeBook::init_new(std::path::Path::new(path), &MacKeychain).await?;
            println!("Created private empty book: {}", path.display());
            Ok(())
        }
        [command, path, assets] if command == "serve" => {
            let book = RuntimeBook::open_existing(std::path::Path::new(path), &MacKeychain).await?;
            eprintln!("kAccounting-Extra-01: http://127.0.0.1:3000");
            let result = serve(&book, std::path::Path::new(assets)).await;
            book.close().await;
            result?;
            Ok(())
        }
        [command, path] if command == "setup-backup-identity" => {
            let identity = application_commands::setup_backup_identity(std::path::Path::new(path), &MacKeychain).await?;
            println!("Initialized new managed-backup namespace: {}", identity.as_str());
            Ok(())
        }
        [command, path] if command == "backup" => {
            let outcome = application_commands::backup_book(std::path::Path::new(path), &MacKeychain).await?;
            println!("Verified backup: {}", outcome.backup.path.display());
            if let Some(warning) = outcome.retention_warning { eprintln!("Backup completed; retention warning: {warning}"); }
            if let Some(retention) = outcome.retention { println!("Retained {} verified backups; removed {}; preserved {} unrecognized items", retention.retained.len(), retention.removed.len(), retention.preserved_unrecognized.len()); }
            Ok(())
        }
        [command, source, destination] if command == "restore" => {
            let outcome = application_commands::restore_book(std::path::Path::new(source), std::path::Path::new(destination), &MacKeychain).await?;
            println!("Verified restored book: {}", outcome.path.display());
            if let Some(warning) = outcome.cleanup_warning { eprintln!("Restore completed; cleanup warning: {warning}"); }
            Ok(())
        }
        [command, path] if command == "collect-attachments" => {
            let outcome = application_commands::collect_attachments(std::path::Path::new(path), &MacKeychain).await?;
            println!("Removed {} unreferenced ciphertext objects ({} bytes); preserved {} unknown items", outcome.removed_objects, outcome.removed_bytes, outcome.preserved_unknown_entries);
            Ok(())
        }
        [command, destination] if command == "recipient-request" => {
            let outcome = application_commands::write_recipient_request(std::path::Path::new(destination), &MacRecipientKeychain)?;
            println!("Public recipient request: {}\nRequest ID: {}\nFingerprint: {}", outcome.path.display(), outcome.request_id, outcome.fingerprint);
            Ok(())
        }
        _ => Err("usage: kaccounting-backend serve-native-chooser <private-collection> <frontend-dist> <trusted-helper> | serve-private-native-chooser <private-collection> <frontend-dist> <private-config> <trusted-helper> | serve-private-chooser <private-collection> <frontend-dist> <private-config> | serve-chooser <private-collection> <frontend-dist> | books <private-collection> | export-recipient <existing-book> <request-file> <new.kagextra> | import-recipient <package> <request-file> <new-book> | recover-recipient-exports <existing-book> <source-work-parent> | import-status <private-parent> | recover-recipient-imports <private-parent> | restore-pre-migration <existing-book> <checkpoint> <new-book> | init-new <new-absolute-book-directory> | serve <existing-book> <frontend-dist> | setup-backup-identity <restored-book> | backup <existing-book> | restore <backup.kabackup> <new-book-directory> | collect-attachments <existing-book> | recipient-request <new-public-request-file>; paths must be absolute and private; startup never creates a book implicitly".into()),
    }
}

#[cfg(not(target_os = "macos"))]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("Version 01 runtime requires the macOS local Keychain".into())
}
