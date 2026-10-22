//! Fail-closed preflight for optional macOS Keychain acceptance.
//!
//! This module deliberately performs no Keychain API calls. The available
//! SecKeychainCreate wrapper does not establish that creation leaves the user's
//! search list unchanged, so an opt-in alone cannot authorize this probe.
use thiserror::Error;

pub const OPT_IN_VARIABLE: &str = "KACCOUNTING_DISPOSABLE_KEYCHAIN_PROBE";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum KeychainProbeBlocker {
    #[error("disposable Keychain acceptance requires explicit opt-in")]
    NotOptedIn,
    #[error("real Keychain acceptance requires macOS")]
    UnsupportedPlatform,
    #[error(
        "isolated keychain creation has not been verified to preserve the default keychain and search list; no operating-system calls were made"
    )]
    IsolationUnverified,
}

/// Pure preflight, with explicit inputs so normal tests never read or mutate
/// process/OS Keychain state. No combination currently enables OS operations.
pub fn preflight(opted_in: bool, macos: bool) -> Result<(), KeychainProbeBlocker> {
    if !opted_in {
        return Err(KeychainProbeBlocker::NotOptedIn);
    }
    if !macos {
        return Err(KeychainProbeBlocker::UnsupportedPlatform);
    }
    Err(KeychainProbeBlocker::IsolationUnverified)
}
