# ADR 0029: Device keys and verified encrypted local backups

Status: accepted engineering foundations; application/runtime integration remains.

## Device key boundary

`keys::DeviceKeyStore` exposes only read and create-only insertion. Explicit
initialization generates a random 256-bit key; opening an existing identifier
never generates a replacement, repairs malformed bytes, or falls back to a
password. Errors contain system status codes, never secret bytes. Key material
and temporary Rust/Core Foundation backing allocations are zeroizing.

The compiled macOS adapter uses local file-Keychain items, create-only
`SecItemAdd`, and non-synchronizing lookups. It does not spawn a shell, put keys
in command-line arguments, export plaintext key files, or introduce an
application password. Generic password is the Keychain item class, not a
user-defined application password. Default-feature TLS support is disabled in
the dependency.

The adapter is compiled in automated checks. Automated tests use an in-memory
store and never touch the user's actual Keychain. Real signed/unsigned binary
access, login/locked-Keychain behavior, launch-at-login interaction, and key-loss
acceptance still need testing before real-book use.

Sources: [create-only item insertion](https://docs.rs/security-framework/3.7.0/security_framework/item/struct.ItemAddOptions.html),
[non-synchronizing lookups](https://docs.rs/security-framework/3.7.0/security_framework/passwords/struct.PasswordOptions.html).

## Local backup format

A local `.kabackup` directory is separate from the destination-recipient
`.kagextra` transfer format. It contains only:

- `envelope.json`: strict format/version and public opaque device-key identifier;
- `catalog.kae`: authenticated encrypted catalog, including the same key ID;
- `book.kae`: authenticated encrypted consistent SQLite snapshot;
- `attachments/<random-id>.kae`: the independently encrypted immutable objects.

The catalog binds exact database plaintext/ciphertext SHA-256 descriptors and
the complete set of encrypted attachment descriptors. Encrypted-object purpose
6 is reserved for the local backup catalog; purpose 2 remains the database
backup payload. Sensitive filenames and accounting data stay behind encryption.
Catalog decoding is capped at 64 MiB and rejects unsupported formats, unknown
or duplicate fields, noncanonical integers/hashes, duplicate IDs, malformed IDs,
undeclared files, symlinks, and missing objects.

## Snapshot and restore protocol

1. Load the existing device key; never create it implicitly.
2. Create a private, owner-only, locked work directory.
3. Obtain a verified SQLite snapshot and select attachments from that immutable
   snapshot, including both drafts and posted entries. Callers cannot supply an
   incomplete manual attachment list.
4. Encrypt the database and copy/authenticate every selected encrypted object.
   Remove the private plaintext snapshot before publication.
5. Independently decrypt/audit the database in private work, compare its full
   attachment reference set to the catalog, and authenticate every object.
6. Remove verification plaintext, synchronize, exclusively publish, and repeat
   complete verification on the published directory. Remove that private
   verification database before success. Existing destinations are never replaced.

Restore decrypts into a new private local book, validates SQLite and all
references/objects, writes only the public key-ID binding, and publishes the
new directory after verification. It never changes the active-book selection,
old book, or source backup. Live SQLite remains outside the at-rest encryption
requirement; live attachments remain encrypted. A required parent sync failure
is reported as published-but-unverified, and private cleanup warnings do not
turn a verified published result into a hidden retry/overwrite operation.

## Interrupted work

Ordinary errors and cancellation drop private work. A crash can leave a private
plaintext SQLite candidate; startup must call the recovery function before
normal operation. Recovery accepts only matching owner-only marked work,
obtains an OS file lock, skips active workers, rejects symlinks, and preserves
unrecognized work for diagnosis. Completed backups/books never match cleanup
names. This startup call remains part of runtime integration.

Rust's file locks provide the process-lifetime distinction without guessing
whether a recorded PID has been reused. [Rust file locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock).

## Remaining integration

- Persistent active-book/key binding and real Keychain acceptance.
- Backup-before-migration and before-import orchestration.
- Keep-newest-10 application-managed backup retention.
- Safe physical attachment reclamation with snapshot pins.
- Passwordless recipient-request/HPKE `.kagextra` integration and verified import.
- Runtime, UI commands, activation, cancellation status, and Mac Safari acceptance.

The local backup primitives do not claim those workflows are complete.
