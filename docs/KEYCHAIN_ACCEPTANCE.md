# macOS Keychain acceptance status

## Scope and current result

The production device-root and transfer-recipient adapters use separate,
non-synchronizing, create-only Keychain namespaces. Normal tests must never
access real Keychain items. This round adds an explicit acceptance preflight and
memory-only contract tests. **Real macOS Keychain acceptance is blocked and has
not passed.** No default/login keychain, search list, OS keychain record or real
book was accessed or modified by this probe.

## Why the disposable probe is blocked

The cached `security-framework` 3.7.0 implementation in
`src/os/macos/keychain.rs` implements `CreateOptions::create` by directly calling
`SecKeychainCreate`. The installed Apple SDK `SecKeychain.h` declaration documents
creation and its parameters, but provides no guarantee that creation preserves
the user's keychain search list and exposes no isolation option. A randomized
file path does not establish search-list isolation. Creating then removing or
restoring a search-list entry would still modify user state and is outside this
probe's boundary.

The probe therefore calls no OS APIs. It does not change default/search-list
settings, disable user interaction globally, create a temporary OS keychain,
read real keys, or introduce an application password. There is no need to clean
up an OS keychain because none is created.

## Repeatable checks

Run memory contracts with:

```sh
cargo test --manifest-path backend/Cargo.toml --test keychain_probe
```

The explicitly ignored preflight can be run with:

```sh
KACCOUNTING_DISPOSABLE_KEYCHAIN_PROBE=1 cargo test --manifest-path backend/Cargo.toml --test keychain_probe real_macos_disposable_keychain_acceptance_preflight -- --ignored --exact
```

That command deliberately **fails** with `IsolationUnverified` on macOS. It is
not a successful OS acceptance result. Without opt-in it fails with `NotOptedIn`;
on another platform the opted-in result is `UnsupportedPlatform`. The
preflight is pure and safe to execute; opt-in cannot bypass the isolation guard.

Memory tests cover duplicate create-only insertion, wrong namespace/missing
lookup, malformed bytes, locked read/write failures, recovery after unlocking,
and preservation of original bytes for both device and recipient secrets.
Recipient tests also exercise production request creation and unwrap failure
paths. These are contract checks, not evidence of OS ACL/lock behavior.

## Remaining acceptance work

Identify a documented creation mechanism that guarantees no user/default/search
list mutation, or use a dedicated disposable macOS user/VM under separately
approved operational scope. Then add explicitly scoped adapters and test actual
create/read/duplicate/missing/malformed/wrong-namespace/lock/unlock behavior.
Only a newly owned test keychain may be cleaned up. Any temporary keychain
credential must be OS-random, held in zeroizing memory, and never logged,
serialized, placed in command arguments, or used as a product password.

## Evidence from 2026-10-22 build session

Targeted integration run: **7 memory tests passed, 1 OS preflight ignored**.
An explicit opt-in execution of the just-built preflight binary returned
`IsolationUnverified` and exit status 101 as designed, before any OS calls.
