# Keychain readiness diagnostics

`keychain_diagnostics::classify(&KeyStoreError)` converts a local key access
failure into a bounded `Problem { code, operator_guidance }`. Both fields are
static strings; classification performs no OS calls, retries, key generation,
item mutation, keychain locking, default-keychain changes, or search-list changes.
The result intentionally omits raw OS status, key references, secret bytes and
paths. HTTP integration can serialize these two safe fields without rendering an
unfiltered `RuntimeError` or OS error.

| Condition | Code | Operator action |
| --- | --- | --- |
| Missing original item | `missing_original_key` | Preserve book/attachments; recover original key or verified encrypted transfer from original Mac. |
| Invalid secret length | `malformed_key` | Preserve book and existing key item; recover original verified material. |
| Explicit macOS cancellation | `keychain_user_cancelled` | Retry voluntarily from local logged-in session and review intended prompt. |
| Interaction unavailable/required | `keychain_interaction_required` | Retry in local logged-in session; check prompts and Keychain availability. |
| Other/unknown OS failure | `keychain_unavailable` | Preserve everything, check local availability/access permissions, retry after resolution. |
| Invalid key reference | `invalid_key_binding` | Preserve the binding and book; recover verified original material, never guess references. |
| Create-only collision | `key_already_exists` | Keep existing item; inspect interrupted work before new explicit initialization. |
| Secure random unavailable | `secure_random_unavailable` | Resolve local system failure; never substitute predictable material. |

No problem authorizes regeneration, repair, replacement, or deletion of an
existing book's key. A source-code clone does not transfer Keychain items or
make encrypted books readable on another Mac. Local encrypted backups may still
require their original device key; copying a backup cannot bypass that dependency.

## Status definitions checked against local primary headers

Verified on this machine in Apple's installed SDK:

`/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/Security.framework/Headers/SecBase.h`

| Apple symbol | Value | Classification |
| --- | --- | --- |
| `errSecUserCanceled` | -128 | User cancellation |
| `errSecDuplicateItem` | -25299 | Existing item collision |
| `errSecItemNotFound` | -25300 | Missing original item |
| `errSecInteractionNotAllowed` | -25308 | Interaction required/unavailable in this context |
| `errSecInteractionRequired` | -25315 | Interaction required/unavailable in this context |

The header defines -25308 as interaction being disallowed. It does **not** prove
that the keychain is locked: a non-interactive process or other execution context
can also prevent interaction. Diagnostics therefore say it may need unlocking,
but never assert locked state. Authorization failure (-25293), unavailable or
invalid keychains, decode errors (-26275), missing entitlement (-34018), and
unknown statuses stay generic unavailable; none prove a missing/malformed key.
The adapter normally maps not-found/collision into typed errors; classification
also handles these statuses defensively if a caller supplies an unnormalized error.

## What this validates

Five pure fake-error tests cover the complete typed-error set, relevant Apple
status families, unknown numeric extremes, preservation guidance, bounded static
serialization, and absence of raw status or key identifiers. They require no
keychain adapter or operating-system key operations:

```sh
cargo test --locked --manifest-path backend/Cargo.toml --test keychain_diagnostics
```

The pure module proves only operator-diagnostic behavior. Separate live evidence
has since passed isolated create/read/duplicate/lock/denied-read/unlock/read on a
new fixture; see `KEYCHAIN_ISOLATED_ACCEPTANCE.md` and the
[ongoing round checkpoint](BUILD_NON_UI_45MIN_2026-10-22.md). The successful receipt
records -128 with helper interaction disabled and no returned secret, unchanged
preference fingerprints, and `login_logout_tested: false`. The earlier failed
attempt stopped before unlock and is not counted as passing.

Neither this classifier nor that isolated fixture establishes production
signed-app ACL, true logout/login, or existing financial-key behavior. Those
remain separate acceptance cases. The earlier retained dedicated test items in
`KEYCHAIN_ITEMS_ACCEPTANCE.md` are separate evidence and remain preserved.
