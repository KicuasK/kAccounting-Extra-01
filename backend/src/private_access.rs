//! Fail-closed policy for a future dedicated private HTTPS ingress.
//!
//! Identity headers do not prove proxy provenance. This module does not create
//! the authenticated ingress, and must not be enabled on the regular listener.
use axum::http::{HeaderMap, Method};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct PrivateAccessPolicy {
    hostname: String,
    origin: String,
    login: String,
}

/// A transport assertion made by trusted server wiring, never by request data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedIngress {
    RegularLoopback,
    /// Reserved for a separate ingress with verified proxy provenance.
    /// No production caller may select this until that boundary is implemented.
    AuthenticatedPrivateProxy,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PrivateAccessError {
    #[error("private HTTPS hostname must be canonical lowercase DNS under .ts.net")]
    InvalidHostname,
    #[error("own-user login must be an explicit canonical ASCII Tailscale login")]
    InvalidLogin,
    #[error("private requests require authenticated proxy provenance")]
    UntrustedIngress,
    #[error("missing, duplicate, non-text or unexpected request header")]
    InvalidHeaders,
}

impl PrivateAccessPolicy {
    pub fn new(hostname: &str, login: &str) -> Result<Self, PrivateAccessError> {
        if !canonical_hostname(hostname) {
            return Err(PrivateAccessError::InvalidHostname);
        }
        let mut parts = login.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        if parts.next().is_some()
            || local.is_empty()
            || domain.is_empty()
            || login.len() > 254
            || !login.is_ascii()
            || !local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
            || !domain.split('.').all(dns_label)
            || (!domain.contains('.') && domain != "github")
        {
            return Err(PrivateAccessError::InvalidLogin);
        }
        Ok(Self {
            hostname: hostname.into(),
            origin: format!("https://{hostname}"),
            login: login.into(),
        })
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Validate the entire request before any private book information is read.
    /// Safe navigation may omit Origin; mutations require its exact value.
    pub fn authorize(
        &self,
        headers: &HeaderMap,
        method: &Method,
        ingress: TrustedIngress,
    ) -> Result<(), PrivateAccessError> {
        if ingress != TrustedIngress::AuthenticatedPrivateProxy {
            return Err(PrivateAccessError::UntrustedIngress);
        }
        if singleton(headers, "host")? != Some(self.hostname.as_str())
            || singleton(headers, "tailscale-user-login")? != Some(self.login.as_str())
        {
            return Err(PrivateAccessError::InvalidHeaders);
        }
        // These headers must never override the explicit canonical origin.
        for name in [
            "forwarded",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-original-host",
            "x-original-url",
        ] {
            if headers.contains_key(name) {
                return Err(PrivateAccessError::InvalidHeaders);
            }
        }
        let origin = singleton(headers, "origin")?;
        let safe = matches!(*method, Method::GET | Method::HEAD);
        if origin.is_some_and(|value| value != self.origin) || (!safe && origin.is_none()) {
            return Err(PrivateAccessError::InvalidHeaders);
        }
        if let Some(site) = singleton(headers, "sec-fetch-site")?
            && site != "same-origin"
            && !(safe && site == "none")
        {
            return Err(PrivateAccessError::InvalidHeaders);
        }
        Ok(())
    }
}

fn singleton<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a str>, PrivateAccessError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(PrivateAccessError::InvalidHeaders);
    }
    value
        .map(|v| v.to_str().map_err(|_| PrivateAccessError::InvalidHeaders))
        .transpose()
}
fn dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.as_bytes()[0].is_ascii_alphanumeric()
        && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn canonical_hostname(host: &str) -> bool {
    host.len() <= 253
        && host.ends_with(".ts.net")
        && host.split('.').count() >= 4
        && host.split('.').all(dns_label)
}
