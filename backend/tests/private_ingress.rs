//! Socket filesystem boundary tests. No Tailscale service is started.
use kaccounting_backend::private_ingress::SocketPath;
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    },
    path::PathBuf,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let id: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::path::PathBuf::from("/private/tmp").join(format!("ka-ingress-{id}"));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn fresh_private_socket_requires_explicit_private_mode() {
    let f = Fixture::new();
    let path = f.0.join("proxy.sock");
    let choice = SocketPath::prepare(&path).unwrap();
    let _listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(choice.validate_bound().is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    choice.validate_bound().unwrap();
    assert!(
        SocketPath::prepare(&path).is_err(),
        "do not adopt even an existing socket"
    );
}
#[test]
fn unknown_existing_paths_and_symlink_parents_are_preserved() {
    let f = Fixture::new();
    let path = f.0.join("proxy.sock");
    fs::write(&path, b"keep").unwrap();
    assert!(SocketPath::prepare(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"keep");
    let alias = f.0.join("alias");
    symlink(&f.0, &alias).unwrap();
    assert!(SocketPath::prepare(&alias.join("other.sock")).is_err());
}
#[test]
fn changed_parent_identity_and_permissions_are_refused() {
    let f = Fixture::new();
    let parent = f.0.join("private");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let choice = SocketPath::prepare(&parent.join("proxy.sock")).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(choice.validate_parent().is_err());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(&parent, f.0.join("original")).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(choice.validate_parent().is_err());
}
#[test]
fn unsafe_ambient_locations_and_relative_paths_are_refused() {
    let f = Fixture::new();
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SocketPath::prepare(&f.0.join("proxy.sock")).is_err());
    assert!(SocketPath::prepare(std::path::Path::new("proxy.sock")).is_err());
    assert!(SocketPath::prepare(&f.0.join("../proxy.sock")).is_err());
}

#[test]
fn long_socket_path_fails_before_any_binding_or_deletion() {
    let f = Fixture::new();
    let path = f.0.join(format!("{}.sock", "x".repeat(104)));
    assert!(SocketPath::prepare(&path).is_err());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
}
