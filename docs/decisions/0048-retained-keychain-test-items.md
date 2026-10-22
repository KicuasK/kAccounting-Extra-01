# ADR 0048: Explicitly authorized retained Keychain test items

Status: accepted user operation decision and technical guard design; 2026-10-22.

The user authorizes a small number of new, dedicated macOS Keychain test items
and requests retaining them after acceptance for inspection. Existing financial
keys remain outside the authorized scope. The earlier disposable-keychain probe
could not establish live storage/read behavior without changing global keychain
state; this separate opt-in acceptance instead exercises the production default
file-keychain adapters and their separate device/recipient namespaces.

Require exactly `--allow-new-test-items` before invoking the OS. Generate IDs
through existing production creation APIs and register ownership only after
successful insertion. Guard every read with this run's in-memory owned-ID set;
reject unknown IDs before any OS operation. Validate insert lengths before
calling the store. Never enumerate, delete, update, lock/unlock, change defaults
or search lists, or query randomly chosen missing items.

Emit only public item metadata immediately after successful creation. Keep raw
secret material in process memory using production zeroizing boundaries; never
write it to disk, command arguments or logs. Retain items even if subsequent
checks fail. Fake-store tests validate guards during normal automated testing;
live execution remains a separate explicitly authorized operation. Record
permission/entitlement failures as blocked acceptance, preserving all known
created identifiers. This does not expand authorization to existing keys,
real books, login configuration, or application signing.
