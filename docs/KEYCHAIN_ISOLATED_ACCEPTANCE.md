# Isolated OS Keychain acceptance harness

This is a separate acceptance fixture, not a product password or a change to
book encryption policy. The user authorized a new isolated test Keychain,
lock/unlock, and an OS-random fixture secret held only in memory on 2026-10-22.
Existing device/recipient keys, the login keychain, default settings and the
search list must not be modified. No existing item is queried.

## Evidence supporting isolation

Apple's published Security source gates search-list/default modifications on
login/System keychain names; its private-keychain creation path does not request
those changes. The installed SDK permits an explicit keychain reference for
item creation, search, lock and unlock.
[Apple StorageManager implementation](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/StorageManager.cpp),
[Apple Keychain entry points](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecKeychain.cpp)

For locked-read testing, the helper snapshots and disables optional interaction
only in its own process, restores it before unlocking and in a deferred failure
path, and uses an explicit owned reference. Apple's implementation stores this
flag as the library Globals object's `mUI` boolean; it does not change a system
preference. The legacy API is used because the SDK warns that
`kSecUseAuthenticationUIFail` does not suppress legacy keychain prompts.
[Apple per-process Globals implementation](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/Globals.h)

This source evidence supports a measured fixture; it is not a promise about an
unmeasured installed OS. The harness compares read-only SHA-256 fingerprints of
the default keychain path and ordered search-list paths before and after all
operational exits. It prints no existing keychain paths. A mismatch fails closed,
records the discrepancy and does not attempt any automatic restoration/setter.

## Harness behavior

`native/IsolatedKeychainAcceptance.swift` creates a random, private, current-user
owned 0700 directory under `/private/tmp`. Only a fresh `acceptance.keychain`
(and any OS-created companion file) is eligible; an existing pathname, login or
System name is refused. Every item operation targets the newly created keychain
reference. The single test account is random and its namespace is isolated.

The sequence checks create/read equality, duplicate rejection, actual locked
status, locked read denial while this helper's interaction is disabled, explicit
unlock using the memory-only fixture secret, and read equality after unlocking.
Only interaction-denied/required or framework-cancelled (-128) statuses count
as the intended locked-read denial, after the same owned item matched on normal
read, locked status was verified, and the failed read returned no secret bytes.
Framework cancellation does not establish that a human clicked Cancel. Missing
items, generic authentication failure and other statuses remain incomplete. Real system login/logout is never
performed or counted as tested.

The fixture credential is 64 ASCII hexadecimal bytes derived directly from a
32-byte OS-random seed, satisfying the SDK UTF-8 requirement without constructing
a secret Swift String. A format guard runs before creation; both seed and
credential are wiped. The credential is never printed, serialized, saved or put into
command arguments. It is wiped with Darwin `memset_s` on function exit. This is
an ephemeral OS-test credential, not an application passphrase requirement.
The encrypted fixture and a secret-free `public-receipt.json` are retained for
inspection. Since the credential is not retained, do not assume this fixture
can be reopened/unlocked in another session. There is no automatic deletion of
items/keychains or preferences repair.

## Safe code-only validation

```sh
/usr/bin/python3 scripts/test_keychain_fixture_harness.py
```

This compiles into a disposable directory and tests only `--preflight` and
rejected argument combinations. It does not create/open/lock any OS keychain.

A compiled helper's `--preflight` prints the permission boundary without OS
keychain calls. No arguments, unknown flags or additional arguments exit before
operational work. Only the exact approved operational flag can enter the live
fixture. Run the live fixture through the operator's separately authorized
acceptance session, preserving its public receipt and recording actual status.

## Status

The helper compiles against this Mac's installed Security SDK; deprecated legacy
API warnings are expected. Pure preflight passed. Live OS results and the final
receipt must be recorded by the coordinating acceptance run; compilation alone
is not a live acceptance pass. Login/LaunchAgent and production signed-app ACL
behavior remain separate gates even if this isolated fixture succeeds.

Code-only validation on 2026-10-22: **3 argument-boundary tests passed**, including
an operational flag combined with an extra argument (refused before OS work).
The separately compiled `--preflight` also exited successfully without keychain
calls. Actual fixture results are intentionally not inferred from those checks.

The first live coordinating run reached create/read/duplicate/actual locked
status, then returned -128 from the locked read with helper interaction disabled;
that first run was recorded incomplete because its expected-status list was too
narrow. Default/search-list fingerprints matched. The revised harness records
this framework-cancelled denial precisely, rejects any returned secret bytes,
and continues to explicit unlock/read verification. Refer to the coordinating
run's public receipt for the actual final outcome; no user-click claim follows
from -128 alone.

## Actual approved OS result — 2026-10-22

The coordinating run of the revised ASCII-credential helper completed with
**exit status 0**. The secret-free receipt records:

- New owned test Keychain and item created; normal read-back matched.
- Duplicate insertion was rejected.
- Actual owned-Keychain locked status was verified.
- The locked explicit-reference read returned **-128 (`errSecUserCanceled`)**
  with helper-process optional interaction disabled and returned no secret.
  This is framework cancellation evidence, not a claim about a human action.
- Explicit memory-credential unlock and subsequent matching read succeeded.
- Default/search-list before and after SHA-256 fingerprints matched exactly:
  `2a59829978cb924bdf304dced4ce3d8942cba8c113bb977c697abe2787f02ac4`.
- The encrypted fixture and public receipt were retained; the credential was
  not persisted. No existing financial item was queried, no default/search-list
  setter or item/keychain deletion was used.

Receipt location (runtime artifact; never commit its parent or keychain):
`/private/tmp/ka-isolated-keychain-c0e81dad-fc66-409f-945d-f498a7e3d98d/public-receipt.json`.

This passes **isolated OS fixture create/read/duplicate/lock/denied-read/unlock/read**
acceptance. It does not exercise actual system login/logout, reboot launch,
locking the user's default/login Keychain, production signed-application ACLs,
or Safari. Those remain separate acceptance gates. A reboot may make the
retained fixture impossible to unlock because its random credential was
intentionally never retained; this has no effect on real books or device keys.
