//! Undeployed Unix-socket proxy boundary. No listener is opened by this module.
//!
//! Only a kernel-reported root peer can carry the proxy assertion. The desktop
//! user, request headers, and a loopback source address cannot construct it.
//! A root adversary is outside this boundary: root can already read process
//! memory and the user's private filesystem. Non-root Tailscale distributions
//! are intentionally unsupported until a different proven boundary exists.
use crate::private_access::{PrivateAccessPolicy, TrustedIngress};
use axum::http::{HeaderMap, HeaderValue, Method};
use std::{
    fs, io,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
};
use thiserror::Error;
use tokio::net::UnixStream;

#[derive(Debug, Error)]
pub enum IngressError {
    #[error("private ingress requires a root Unix-socket peer")]
    UntrustedPeer,
    #[error("private ingress request headers are invalid")]
    InvalidRequest,
    #[error("private ingress socket path is unsafe or changed")]
    UnsafeSocket,
    #[error("private ingress I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// A connection-scoped proof created solely from OS peer credentials.
/// Do not reuse it for a different accepted connection or place it in a public
/// HTTP extension. The runtime owns it together with that connection's service.
#[derive(Debug)]
pub(crate) struct RootProxyConnection {
    _private: (),
}
impl RootProxyConnection {
    pub(crate) fn authenticate(stream: &UnixStream) -> Result<Self, IngressError> {
        if stream.peer_cred()?.uid() != 0 {
            return Err(IngressError::UntrustedPeer);
        }
        Ok(Self { _private: () })
    }
    /// Normalize the documented Unix-target Serve envelope only AFTER proving
    /// the connection. Forwarding values are never themselves authentication.
    /// Headers are changed only after the complete policy succeeds.
    pub(crate) fn authorize_local(
        &self,
        policy: &PrivateAccessPolicy,
        headers: &mut HeaderMap,
        method: &Method,
    ) -> Result<(), IngressError> {
        normalize(policy, headers, method)?;
        // Only after public HTTPS authorization, adapt to the unchanged local
        // routers. Preserve absent Origin for safe navigation.
        headers.insert("host", HeaderValue::from_static("127.0.0.1:3000"));
        if headers.contains_key("origin") {
            headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:3000"));
        }
        Ok(())
    }
}
fn one<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, IngressError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(IngressError::InvalidRequest);
    }
    first
        .map(|v| v.to_str().map_err(|_| IngressError::InvalidRequest))
        .transpose()
}
fn normalize(
    policy: &PrivateAccessPolicy,
    headers: &mut HeaderMap,
    method: &Method,
) -> Result<(), IngressError> {
    if one(headers, "host")? != Some("localhost")
        || one(headers, "x-forwarded-host")? != Some(policy.hostname())
        || one(headers, "x-forwarded-proto")? != Some("https")
        || headers.contains_key("tailscale-funnel-request")
        || headers.contains_key("forwarded")
        || headers.contains_key("x-original-host")
        || headers.contains_key("x-original-url")
    {
        return Err(IngressError::InvalidRequest);
    }
    // Serve populates this value. It is not used to establish identity, but a
    // duplicate/non-text envelope is rejected rather than ambiguously forwarded.
    let _ = one(headers, "x-forwarded-for")?;
    let mut normalized = headers.clone();
    for name in ["x-forwarded-host", "x-forwarded-proto", "x-forwarded-for"] {
        normalized.remove(name);
    }
    normalized.insert(
        "host",
        HeaderValue::from_str(policy.hostname()).map_err(|_| IngressError::InvalidRequest)?,
    );
    policy
        .authorize(
            &normalized,
            method,
            TrustedIngress::AuthenticatedPrivateProxy,
        )
        .map_err(|_| IngressError::InvalidRequest)?;
    *headers = normalized;
    Ok(())
}

/// File identity for a dedicated, absent socket pathname under a canonical
/// private parent. Runtime binding stays in runtime.rs. Existing sockets,
/// including seemingly stale ones, are never automatically adopted or removed.
pub struct SocketPath {
    path: PathBuf,
    parent: PathBuf,
    identity: fs::Metadata,
}
impl SocketPath {
    pub fn prepare(path: &Path) -> Result<Self, IngressError> {
        // macOS sockaddr_un reserves 104 bytes including the trailing NUL.
        // Keep this conservative limit even on other Unix development hosts.
        if path.as_os_str().as_bytes().len() > 103
            || !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        {
            return Err(IngressError::UnsafeSocket);
        }
        let parent = path.parent().ok_or(IngressError::UnsafeSocket)?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or(IngressError::UnsafeSocket)?;
        if parent.canonicalize()? != parent
            || parent.starts_with(source)
            || source.starts_with(parent)
        {
            return Err(IngressError::UnsafeSocket);
        }
        let identity = fs::symlink_metadata(parent)?;
        if !identity.is_dir()
            || identity.file_type().is_symlink()
            || identity.permissions().mode() & 0o777 != 0o700
        {
            return Err(IngressError::UnsafeSocket);
        }
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            _ => return Err(IngressError::UnsafeSocket),
        }
        Ok(Self {
            path: path.to_owned(),
            parent: parent.to_owned(),
            identity,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn validate_parent(&self) -> Result<(), IngressError> {
        let current = fs::symlink_metadata(&self.parent)?;
        if current.file_type().is_symlink()
            || !current.is_dir()
            || current.dev() != self.identity.dev()
            || current.ino() != self.identity.ino()
            || current.uid() != self.identity.uid()
            || current.permissions().mode() & 0o777 != 0o700
            || self.parent.canonicalize()? != self.parent
        {
            return Err(IngressError::UnsafeSocket);
        }
        Ok(())
    }
    /// Call immediately after binding and chmod 0600. This does not delete the
    /// socket on error; uncertain paths must be inspected by the local operator.
    pub fn validate_bound(&self) -> Result<(), IngressError> {
        self.validate_parent()?;
        let socket = fs::symlink_metadata(&self.path)?;
        if !socket.file_type().is_socket()
            || socket.uid() != self.identity.uid()
            || socket.permissions().mode() & 0o777 != 0o600
        {
            return Err(IngressError::UnsafeSocket);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> PrivateAccessPolicy {
        PrivateAccessPolicy::new("book.example.ts.net", "owner@example.com").unwrap()
    }
    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in [
            ("host", "localhost"),
            ("x-forwarded-host", "book.example.ts.net"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "100.64.0.1"),
            ("tailscale-user-login", "owner@example.com"),
            ("origin", "https://book.example.ts.net"),
            ("sec-fetch-site", "same-origin"),
        ] {
            h.insert(name, HeaderValue::from_static(value));
        }
        h
    }
    #[tokio::test]
    async fn kernel_peer_credentials_reject_unprivileged_local_forgery() {
        let (accepted, _client) = UnixStream::pair().unwrap();
        let uid = accepted.peer_cred().unwrap().uid();
        if uid == 0 {
            assert!(RootProxyConnection::authenticate(&accepted).is_ok());
        } else {
            assert!(matches!(
                RootProxyConnection::authenticate(&accepted),
                Err(IngressError::UntrustedPeer)
            ));
        }
    }
    #[test]
    fn documented_unix_proxy_envelope_normalizes_after_policy() {
        let mut h = headers();
        normalize(&policy(), &mut h, &Method::POST).unwrap();
        assert_eq!(h["host"], "book.example.ts.net");
        assert!(!h.contains_key("x-forwarded-host"));
        assert!(!h.contains_key("x-forwarded-proto"));
        assert!(!h.contains_key("x-forwarded-for"));
    }
    #[test]
    fn public_policy_is_checked_before_local_guard_adaptation() {
        let proof = RootProxyConnection { _private: () };
        let mut h = headers();
        proof
            .authorize_local(&policy(), &mut h, &Method::POST)
            .unwrap();
        assert_eq!(h["host"], "127.0.0.1:3000");
        assert_eq!(h["origin"], "http://127.0.0.1:3000");
        let mut forged = headers();
        forged.insert("origin", HeaderValue::from_static("http://127.0.0.1:3000"));
        assert!(
            proof
                .authorize_local(&policy(), &mut forged, &Method::POST)
                .is_err()
        );
        assert_eq!(forged["host"], "localhost");
        let mut navigation = headers();
        navigation.remove("origin");
        proof
            .authorize_local(&policy(), &mut navigation, &Method::GET)
            .unwrap();
        assert!(!navigation.contains_key("origin"));
    }
    #[test]
    fn bad_envelopes_funnel_origins_and_identity_fail_without_mutation() {
        for (name, bad) in [
            ("host", "127.0.0.1"),
            ("x-forwarded-host", "evil.example.ts.net"),
            ("x-forwarded-proto", "http"),
            ("tailscale-user-login", "other@example.com"),
            ("origin", "null"),
            ("sec-fetch-site", "cross-site"),
            ("tailscale-funnel-request", "?1"),
            ("forwarded", "host=evil"),
        ] {
            let mut h = headers();
            h.insert(name, HeaderValue::from_static(bad));
            let original = h.clone();
            assert!(
                normalize(&policy(), &mut h, &Method::POST).is_err(),
                "{name}"
            );
            assert_eq!(h, original);
        }
    }
    #[test]
    fn duplicate_proxy_or_identity_headers_fail_closed() {
        for name in [
            "host",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-for",
            "tailscale-user-login",
            "origin",
            "sec-fetch-site",
        ] {
            let mut h = headers();
            let value = h[name].clone();
            h.append(name, value);
            assert!(
                normalize(&policy(), &mut h, &Method::GET).is_err(),
                "{name}"
            );
        }
    }
    #[test]
    fn mutations_require_origin_navigation_may_omit_it() {
        let mut h = headers();
        h.remove("origin");
        h.remove("sec-fetch-site");
        assert!(normalize(&policy(), &mut h.clone(), &Method::GET).is_ok());
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(normalize(&policy(), &mut h.clone(), &method).is_err());
        }
    }
}
