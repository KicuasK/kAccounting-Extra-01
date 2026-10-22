//! Configuration tests use fake identities and files; no key stores or listeners.
use kaccounting_backend::private_runtime_config::PrivateRuntimeConfig;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let id: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let root = PathBuf::from("/private/tmp").join(format!("ka-private-cfg-{id}"));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn document(&self) -> Value {
        json!({"hostname":"book.example.ts.net", "login":"owner@example.com", "socket": self.0.join("proxy.sock")})
    }
    fn write(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("config.json");
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
    fn config(&self, value: Value) -> PathBuf {
        self.write(&serde_json::to_vec(&value).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn valid_explicit_configuration_is_read_only_and_returns_checked_policy() {
    let f = Fixture::new();
    let path = f.config(f.document());
    let before = fs::read(&path).unwrap();
    let (policy, socket) = PrivateRuntimeConfig::load(&path).unwrap().into_parts();
    assert_eq!(policy.hostname(), "book.example.ts.net");
    assert_eq!(policy.origin(), "https://book.example.ts.net");
    assert_eq!(socket, f.0.join("proxy.sock"));
    assert!(!socket.exists());
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}
#[test]
fn unknown_duplicate_missing_or_non_string_fields_are_refused() {
    let f = Fixture::new();
    for value in [
        json!({}),
        json!({"hostname":"book.example.ts.net","login":"owner@example.com"}),
        json!({"hostname":false,"login":"owner@example.com","socket":f.0.join("proxy.sock")}),
        {
            let mut d = f.document();
            d["enable_funnel"] = json!(true);
            d
        },
    ] {
        assert!(PrivateRuntimeConfig::load(f.config(value)).is_err());
    }
    let text = format!(
        r#"{{"hostname":"book.example.ts.net","hostname":"other.example.ts.net","login":"owner@example.com","socket":"{}/proxy.sock"}}"#,
        f.0.display()
    );
    assert!(PrivateRuntimeConfig::load(f.write(text.as_bytes())).is_err());
}
#[test]
fn public_or_special_mode_files_and_nonprivate_parents_are_refused() {
    let f = Fixture::new();
    let path = f.config(f.document());
    for mode in [0o644, 0o640, 0o660, 0o400, 0o4600] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        // Some macOS temporary filesystems strip set-ID bits on chmod.
        if fs::metadata(&path).unwrap().permissions().mode() & 0o7777 != 0o600 {
            assert!(PrivateRuntimeConfig::load(&path).is_err(), "mode {mode:o}");
        }
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(PrivateRuntimeConfig::load(path).is_err());
}
#[test]
fn hardlinks_symlinks_and_noncanonical_paths_are_refused() {
    let f = Fixture::new();
    let path = f.config(f.document());
    let link = f.0.join("hardlink");
    fs::hard_link(&path, &link).unwrap();
    assert!(PrivateRuntimeConfig::load(&path).is_err());
    fs::remove_file(link).unwrap();
    let link = f.0.join("symlink.json");
    symlink(&path, &link).unwrap();
    assert!(PrivateRuntimeConfig::load(link).is_err());
    let directory_link = f.0.join("alias");
    symlink(&f.0, &directory_link).unwrap();
    assert!(PrivateRuntimeConfig::load(directory_link.join("config.json")).is_err());
    assert!(PrivateRuntimeConfig::load(Path::new("relative.json")).is_err());
    assert!(PrivateRuntimeConfig::load(f.0.join("unused/../config.json")).is_err());
}
#[test]
fn oversized_bad_policies_and_existing_or_long_socket_paths_are_refused() {
    let f = Fixture::new();
    assert!(PrivateRuntimeConfig::load(f.write(&vec![b' '; 16 * 1024 + 1])).is_err());
    for (name, value) in [
        ("hostname", json!("localhost")),
        ("login", json!("owner")),
        ("socket", json!("relative.sock")),
        ("socket", json!(f.0.join("x".repeat(104)))),
    ] {
        let mut d = f.document();
        d[name] = value;
        assert!(PrivateRuntimeConfig::load(f.config(d)).is_err());
    }
    fs::write(f.0.join("proxy.sock"), b"keep").unwrap();
    assert!(PrivateRuntimeConfig::load(f.config(f.document())).is_err());
    assert_eq!(fs::read(f.0.join("proxy.sock")).unwrap(), b"keep");
}
#[test]
fn source_tree_and_error_details_are_not_exposed() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let error = match PrivateRuntimeConfig::load(source) {
        Err(e) => e,
        Ok(_) => panic!("source configuration accepted"),
    };
    let text = error.to_string();
    assert!(!text.contains("Cargo.toml"));
    assert!(!text.contains("example.com"));
    assert!(!text.contains(env!("CARGO_MANIFEST_DIR")));
}

#[test]
fn github_provider_login_is_accepted_by_explicit_runtime_configuration() {
    let f = Fixture::new();
    let mut document = f.document();
    document["login"] = json!("FakeOwner@github");
    let (policy, _) = PrivateRuntimeConfig::load(f.config(document))
        .unwrap()
        .into_parts();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "host",
        axum::http::HeaderValue::from_static("book.example.ts.net"),
    );
    headers.insert(
        "tailscale-user-login",
        axum::http::HeaderValue::from_static("FakeOwner@github"),
    );
    assert!(
        policy
            .authorize(
                &headers,
                &axum::http::Method::GET,
                kaccounting_backend::private_access::TrustedIngress::AuthenticatedPrivateProxy
            )
            .is_ok()
    );
    headers.insert(
        "tailscale-user-login",
        axum::http::HeaderValue::from_static("fakeowner@github"),
    );
    assert!(
        policy
            .authorize(
                &headers,
                &axum::http::Method::GET,
                kaccounting_backend::private_access::TrustedIngress::AuthenticatedPrivateProxy
            )
            .is_err()
    );
}
