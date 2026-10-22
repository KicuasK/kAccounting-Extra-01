//! Explicit, bounded local private-runtime configuration. Never invokes Tailscale.
use crate::{private_access::PrivateAccessPolicy, private_ingress::SocketPath};
use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;
use std::{
    fs::{self, File, Metadata},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;
const LIMIT: u64 = 16 * 1024;
#[derive(Debug, Error)]
#[error(
    "Private runtime configuration is invalid, unsafe, changed, or unavailable; preserve it and review the local configuration"
)]
pub struct ConfigError;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    hostname: String,
    login: String,
    socket: PathBuf,
}
/// Does not expose a Debug implementation containing operator identities.
pub struct PrivateRuntimeConfig {
    policy: PrivateAccessPolicy,
    socket: PathBuf,
}
impl PrivateRuntimeConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        load(path.as_ref()).map_err(|_| ConfigError)
    }
    pub fn into_parts(self) -> (PrivateAccessPolicy, PathBuf) {
        (self.policy, self.socket)
    }
}
fn identity(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn load(path: &Path) -> Result<PrivateRuntimeConfig, Box<dyn std::error::Error>> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        || path.canonicalize()? != path
    {
        return Err(ConfigError.into());
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(ConfigError)?;
    let parent = path.parent().ok_or(ConfigError)?;
    if parent.starts_with(source) || source.starts_with(parent) {
        return Err(ConfigError.into());
    }
    let mut directory = File::from(open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    let mut chain = Vec::new();
    let mut prefix = PathBuf::from("/");
    for component in parent.components().skip(1) {
        let Component::Normal(name) = component else {
            return Err(ConfigError.into());
        };
        directory = File::from(openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        prefix.push(name);
        chain.push((prefix.clone(), directory.metadata()?));
    }
    let metadata = directory.metadata()?;
    let owner = rustix::process::geteuid().as_raw();
    if metadata.uid() != owner || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(ConfigError.into());
    }
    let mut file = File::from(openat(
        &directory,
        path.file_name().ok_or(ConfigError)?,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    let before = file.metadata()?;
    if !before.is_file()
        || before.uid() != owner
        || before.permissions().mode() & 0o7777 != 0o600
        || before.nlink() != 1
        || before.len() > LIMIT
    {
        return Err(ConfigError.into());
    }
    let mut bytes = Vec::new();
    (&mut file).take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT
        || !identity(&before, &file.metadata()?)
        || !identity(&before, &fs::symlink_metadata(path)?)
    {
        return Err(ConfigError.into());
    }
    for (name, expected) in chain {
        let current = fs::symlink_metadata(name)?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || current.dev() != expected.dev()
            || current.ino() != expected.ino()
            || current.uid() != expected.uid()
            || current.mode() != expected.mode()
        {
            return Err(ConfigError.into());
        }
    }
    let document: Document = serde_json::from_slice(&bytes)?;
    let policy = PrivateAccessPolicy::new(&document.hostname, &document.login)?;
    let socket = SocketPath::prepare(&document.socket)?;
    socket.validate_parent()?;
    Ok(PrivateRuntimeConfig {
        policy,
        socket: document.socket,
    })
}
