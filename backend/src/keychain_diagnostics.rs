//! Pure, bounded diagnostics for local key access failures. No OS calls, key
//! identifiers, status integers, secret bytes, or regeneration instructions are
//! included in the resulting problem. Classifying does not retry or mutate.
use crate::keys::KeyStoreError;
use serde::Serialize;

// Definitions verified against the local Apple SDK Security/SecBase.h, not
// inferred from a failing live key operation. In particular -25308 says only
// that interaction is not allowed; it does not establish that a keychain is locked.
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_SEC_INTERACTION_REQUIRED: i32 = -25315;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Problem {
    pub code: &'static str,
    pub operator_guidance: &'static str,
}

const MISSING: Problem = Problem {
    code: "missing_original_key",
    operator_guidance: "Preserve this book and its attachments. Recover its original key or obtain a verified encrypted transfer from the original Mac. Do not create a replacement key for this existing book.",
};
const MALFORMED: Problem = Problem {
    code: "malformed_key",
    operator_guidance: "Preserve this book and the current key item unchanged. Recover the verified original key or obtain a verified encrypted transfer from the original Mac. Do not repair or replace the stored key automatically.",
};
const CANCELLED: Problem = Problem {
    code: "keychain_user_cancelled",
    operator_guidance: "The key access operation was cancelled. Keep the current book unchanged. Retry from your local logged-in session when ready, and approve a macOS access prompt only if you intend to open this book.",
};
const INTERACTION: Problem = Problem {
    code: "keychain_interaction_required",
    operator_guidance: "macOS cannot complete the required key access interaction in this context. Retry from your local logged-in session and review any macOS prompt. The keychain may need unlocking, but this result alone does not prove it is locked. Preserve the book and do not create a replacement key.",
};
const UNAVAILABLE: Problem = Problem {
    code: "keychain_unavailable",
    operator_guidance: "Key access is unavailable. Preserve the book and existing key items, check the local macOS session and Keychain availability or access permissions, then retry. Do not create a replacement key or change the default keychain or search list automatically.",
};
const INVALID_BINDING: Problem = Problem {
    code: "invalid_key_binding",
    operator_guidance: "The book key reference is invalid. Preserve the book and key items unchanged, and recover a verified original book or transfer. Do not guess a key reference or initialize a replacement key.",
};
const EXISTS: Problem = Problem {
    code: "key_already_exists",
    operator_guidance: "A key item already exists for this operation. Preserve the existing item and book. Do not overwrite or delete it; inspect the interrupted operation before trying a new explicit initialization.",
};
const RANDOM_UNAVAILABLE: Problem = Problem {
    code: "secure_random_unavailable",
    operator_guidance: "macOS secure random generation is unavailable. Preserve existing books and keys, then retry after the local system issue is resolved. Do not substitute a predictable key or regenerate an existing book key.",
};

/// Return only static operator-facing data. Unknown statuses fail closed to
/// unavailable; authentication/decode failures never assert a missing or locked
/// key, nor do they authorize replacing an existing key.
pub fn classify(error: &KeyStoreError) -> Problem {
    match error {
        KeyStoreError::Missing | KeyStoreError::Unavailable(ERR_SEC_ITEM_NOT_FOUND) => MISSING,
        KeyStoreError::MalformedSecret => MALFORMED,
        KeyStoreError::Unavailable(ERR_SEC_USER_CANCELED) => CANCELLED,
        KeyStoreError::Unavailable(
            ERR_SEC_INTERACTION_NOT_ALLOWED | ERR_SEC_INTERACTION_REQUIRED,
        ) => INTERACTION,
        KeyStoreError::Unavailable(ERR_SEC_DUPLICATE_ITEM) => EXISTS,
        KeyStoreError::Unavailable(_) => UNAVAILABLE,
        KeyStoreError::InvalidIdentifier => INVALID_BINDING,
        KeyStoreError::AlreadyExists => EXISTS,
        KeyStoreError::RandomUnavailable => RANDOM_UNAVAILABLE,
    }
}
