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
        [command, path] if command == "init-new" => {
            let path = RuntimeBook::init_new(std::path::Path::new(path), &MacKeychain).await?;
            println!("Created private empty book: {}", path.display());
            Ok(())
        }
        [command, path, assets] if command == "serve" => {
            let book = RuntimeBook::open_existing(std::path::Path::new(path), &MacKeychain).await?;
            eprintln!("kAccounting-Extra-01: http://127.0.0.1:3000");
            serve(&book, std::path::Path::new(assets)).await?;
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
        _ => Err("usage: kaccounting-backend init-new <new-absolute-book-directory> | serve <existing-book> <frontend-dist> | setup-backup-identity <restored-book> | backup <existing-book> | restore <backup.kabackup> <new-book-directory> | collect-attachments <existing-book> | recipient-request <new-public-request-file>; paths must be absolute and private; startup never creates a book implicitly".into()),
    }
}

#[cfg(not(target_os = "macos"))]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("Version 01 runtime requires the macOS local Keychain".into())
}
