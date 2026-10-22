//! Deliberate operator-run OS picker acceptance. No socket and no OS key store.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kaccounting_backend::{
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError},
    native_selection::NativePicker,
    recipient_transfer::RecipientKeyStore,
    runtime::RuntimeBook,
    session_runtime::SessionRuntime,
    storage_operations::SelectionKind,
};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;
use zeroize::Zeroizing;
#[derive(Default)]
struct Keys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl RecipientKeyStore for Keys {
    fn read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(KeyStoreError::Missing)
    }
    fn insert_new(&self, id: &str, value: &[u8]) -> Result<(), KeyStoreError> {
        let mut keys = self.0.lock().unwrap();
        if keys.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists);
        }
        keys.insert(id.to_owned(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
}
impl DeviceKeyStore for Keys {
    fn read(&self, id: &DeviceKeyId) -> Result<Zeroizing<Vec<u8>>, KeyStoreError> {
        RecipientKeyStore::read(self, id.as_str())
    }
    fn insert_new(&self, id: &DeviceKeyId, value: &[u8]) -> Result<(), KeyStoreError> {
        RecipientKeyStore::insert_new(self, id.as_str(), value)
    }
}
struct Fixture {
    root: PathBuf,
    identity: fs::Metadata,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only this explicitly created random fixture root is cleanup authority.
        if fs::symlink_metadata(&self.root).is_ok_and(|m| {
            m.is_dir()
                && !m.file_type().is_symlink()
                && m.dev() == self.identity.dev()
                && m.ino() == self.identity.ino()
        }) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
struct Options {
    helper: PathBuf,
    cancel_expected: bool,
    selection_kind: SelectionKind,
}
fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options, &'static str> {
    let helper = PathBuf::from(args.next().ok_or(
        "usage: native_picker_acceptance /canonical/helper [--mode recovery-parent|request-destination|book-destination] [--cancel-expected]",
    )?);
    if !helper.is_absolute()
        || helper
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("helper must be an absolute path without parent traversal");
    }
    let mut cancel_expected = false;
    let mut selection_kind = SelectionKind::RecoveryParent;
    let mut mode_seen = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--cancel-expected" if !cancel_expected => cancel_expected = true,
            "--mode" if !mode_seen => {
                mode_seen = true;
                selection_kind = match args.next().as_deref() {
                    Some("recovery-parent") => SelectionKind::RecoveryParent,
                    Some("request-destination") => SelectionKind::RequestDestination,
                    Some("book-destination") => SelectionKind::BookDestination,
                    _ => {
                        return Err(
                            "--mode requires recovery-parent, request-destination, or book-destination",
                        );
                    }
                };
            }
            _ => return Err("unknown or duplicate option"),
        }
    }
    Ok(Options {
        helper,
        cancel_expected,
        selection_kind,
    })
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse the complete operation before opening a helper, creating a fixture,
    // or presenting a panel. Repeated flags never silently change acceptance.
    let Options {
        helper,
        cancel_expected,
        selection_kind,
    } = parse_options(std::env::args().skip(1))?;
    let helper_metadata = fs::symlink_metadata(&helper)?;
    let picker = NativePicker::pin(&helper, helper_metadata.uid())?;
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce)?;
    let root = std::env::temp_dir().canonicalize()?.join(format!(
        "ka-native-panel-acceptance-{:032x}",
        u128::from_be_bytes(nonce)
    ));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let fixture = Fixture {
        identity: fs::symlink_metadata(&root)?,
        root,
    };
    let keys = Arc::new(Keys::default());
    RuntimeBook::init_new(fixture.root.join("test-book"), keys.as_ref()).await?;
    let assets = fixture.root.join("assets");
    fs::create_dir(&assets)?;
    fs::write(
        assets.join("index.html"),
        "<!doctype html>disposable native acceptance",
    )?;
    let target = fixture.root.join(match selection_kind {
        SelectionKind::RequestDestination => "recipient-request.json",
        SelectionKind::BookDestination => "New Book",
        _ => "SELECT-ONLY-THIS-TEST-FOLDER",
    });
    let new_destination = selection_kind != SelectionKind::RecoveryParent;
    if !new_destination {
        fs::DirBuilder::new().mode(0o700).create(&target)?;
    }
    let session = SessionRuntime::new(&fixture.root, &assets, keys.clone(), keys.clone())?;
    let selected = session
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/session/selection")
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"book_id":"test-book"}"#))?,
        )
        .await?;
    if selected.status() != StatusCode::OK {
        return Err("disposable test-book selection failed".into());
    }
    println!("Disposable native picker acceptance; in-memory fake keys only; no network listener.");
    println!("Select ONLY this disposable target, or Cancel:");
    println!("{}", target.display());
    if new_destination {
        println!(
            "Save-panel navigation folder (Command-Shift-G): {}",
            fixture.root.display()
        );
        println!(
            "Keep destination name: {}",
            target
                .file_name()
                .ok_or("missing target name")?
                .to_string_lossy()
        );
        println!("The picker must NOT create this file/folder; this run only registers selection.");
    } else {
        println!(
            "Open-panel navigation folder (Command-Shift-G): {}",
            target.display()
        );
    }
    if cancel_expected {
        println!("This run expects Cancel.");
    }
    let result = tokio::select! {
        result = picker.select_and_register_exact(&session, selection_kind, &target) => Some(result),
        signal = tokio::signal::ctrl_c() => { signal?; None },
    };
    picker.wait_idle().await?;
    session.shutdown().await;
    let Some(result) = result else {
        println!("CANCELLED: Ctrl-C terminated and reaped the picker; disposable fixture cleaned.");
        return Ok(());
    };
    if new_destination && fs::symlink_metadata(&target).is_ok() {
        return Err("picker unexpectedly created a destination; acceptance failed".into());
    }
    match result? {
        None if cancel_expected=>println!("PASS: operator cancelled; no capability registered."),
        Some(_) if !cancel_expected=>println!("PASS: exact disposable target selected and capability registered; no destination created."),
        None=>return Err("operator cancelled: selection acceptance not completed (repeat with --cancel-expected to verify cancellation)".into()),
        Some(_)=>return Err("operator selected a folder, but this run expected cancellation".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Result<Options, &'static str> {
        parse_options(args.iter().map(|value| (*value).to_owned()))
    }
    #[test]
    fn defaults_and_explicit_modes_are_parsed_without_touching_a_helper() {
        let default = options(&["/invented/helper"]).unwrap();
        assert_eq!(default.selection_kind, SelectionKind::RecoveryParent);
        assert!(!default.cancel_expected);
        for (mode, expected) in [
            ("recovery-parent", SelectionKind::RecoveryParent),
            ("request-destination", SelectionKind::RequestDestination),
            ("book-destination", SelectionKind::BookDestination),
        ] {
            let parsed =
                options(&["/invented/helper", "--mode", mode, "--cancel-expected"]).unwrap();
            assert_eq!(parsed.selection_kind, expected);
            assert!(parsed.cancel_expected);
        }
    }
    #[test]
    fn duplicate_conflicting_unknown_and_incomplete_options_are_rejected_before_panel() {
        for args in [
            vec![
                "/invented/helper",
                "--mode",
                "recovery-parent",
                "--mode",
                "book-destination",
            ],
            vec!["/invented/helper", "--cancel-expected", "--cancel-expected"],
            vec!["/invented/helper", "--mode"],
            vec!["/invented/helper", "--mode", "unknown"],
            vec!["/invented/helper", "--unknown"],
        ] {
            assert!(options(&args).is_err());
        }
    }
    #[test]
    fn missing_relative_or_parent_traversing_helper_is_refused_without_io() {
        for args in [
            vec![],
            vec!["relative-helper"],
            vec!["/invented/../helper"],
            vec!["--cancel-expected"],
        ] {
            assert!(options(&args).is_err());
        }
    }
}
