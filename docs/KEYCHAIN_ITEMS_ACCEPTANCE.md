# Dedicated live Keychain item acceptance

The user authorized new dedicated test items on 2026-10-22 and requested that
items remain available for inspection. This acceptance requires no real book.
It does not read existing financial keys or prove application signing, login
startup, or Safari behavior.

Build without touching Keychain:

```sh
cargo test --manifest-path backend/Cargo.toml --test keychain_items_acceptance
cargo build --manifest-path backend/Cargo.toml --example keychain_items_acceptance
```

Only after permission to create retained test items:

```sh
target/debug/examples/keychain_items_acceptance --allow-new-test-items
```

The example rejects absent/additional arguments before any Keychain operation.
It uses production `MacKeychain` and `MacRecipientKeychain`, generating a fresh
128-bit random item ID through `create_device_key` and
`create_recipient_request`. Successful insertion registers the ID in an
in-memory guard; reads of all other IDs return before reaching the OS. Failed
inserts never register ownership. Malformed insertions fail before OS access.
The default Keychain/search list is never changed; there are no enumeration,
lock/unlock, deletion, update, or random-missing-ID OS queries.

Exactly two new items are intended, in these production namespaces:

- `kAccounting-Extra-01.device-root.v1`
- `kAccounting-Extra-01.recipient-private.v1`

Public service names and item IDs print immediately after each successful
creation, before later checks. The recipient public-key fingerprint and boolean
verification outcomes also print. No secret is printed, stored in a file, or
passed in process arguments. Root-key read-back equality is verified by
production encryption/decryption. Recipient HPKE wrapping and unwrapping are
verified the same way. Duplicate create-only insertion must return
`AlreadyExists`; wrong package context must fail. Items are retained, including
when a later check fails. Rerunning creates fresh items; it never reuses earlier
printed IDs.

macOS may show a permission prompt, which the user handles. A nonzero exit with
system status -25308 (interaction unavailable) or -34018 (missing entitlement)
is an incomplete/blocked live acceptance, not a passed test. Other system
statuses must likewise be recorded honestly. Keep any already printed public
IDs for inspection. Never work around errors by changing the keychain search
list, deleting items, or reading existing IDs. Standard automated tests use a
fake store and perform no OS Keychain operations.

This run records only capabilities exercised in the actual host execution;
compilation and fake-guard tests alone are not evidence of a live Keychain pass.
