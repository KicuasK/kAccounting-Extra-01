# ADR 0030: Explicit private local runtime

Status: implemented foundation; no live user book, Keychain, listener, or LaunchAgent was used during development.

## Startup and book identity

`kaccounting-backend init-new <new-absolute-book-directory>` is the only book/key creation command. Its existing parent must be private (`0700`), outside the source tree, and free of symlink components. Initialization stages a new private book, creates a local Keychain key through the create-only key contract, persists the public key ID, initializes an authenticated backup identity, creates and audits SQLite, closes the pool, then publishes the directory without replacement. Failed work and its Keychain key are preserved for explicit diagnosis; initialization never deletes potentially recoverable key material.

`kaccounting-backend serve <existing-absolute-book-directory> <frontend-dist-directory>` requires an existing private book. Missing bindings, missing/malformed keys, unknown backup work, unsafe files, and database integrity errors fail closed. Startup takes a process-lifetime exclusive `.runtime.lock` and performs interrupted local-backup recovery in the book directory. Existing databases receive a complete read-only schema/integrity audit before writable opening. Consequently an older schema cannot silently migrate: a separate backup-first upgrade workflow is still required.

The binding format intentionally matches the existing local-restore `device-key.json` envelope. It contains a format/version and public random key ID, never a secret. Newly initialized books also contain the encrypted persistent `backup-identity.kae`. Existing books load that identity if present and never generate a replacement silently; restored older books lacking it remain openable, but need an explicit managed-backup namespace setup before managed retention.

## Listener and browser boundary

The only listener address is the constant `127.0.0.1:3000`; there is no address/port override. A new outer guard protects accounting routes, static files, and unknown routes alike. Exactly one matching Host is required; writes additionally require exactly one matching `http://127.0.0.1:3000` Origin. A supplied read Origin must match. Cross-site or duplicate Fetch Metadata headers are rejected. The inner API origin guard remains intact.

Only the built `index.html` and recognized assets under `assets/` are served. No runtime/book directory is reachable through static routes. Dot segments, symlink components, paths outside the chosen dist directory, unsupported types, and files larger than 8 MiB are rejected. Responses use no-store, nosniff, no-referrer, and a same-origin Content Security Policy with frame embedding disabled. No CORS, proxy identity, remote origin, Tailscale configuration, or public exposure is enabled.

## Remaining integration

- Real macOS Keychain acceptance, including locked Keychain and deployed binary identity.
- Default Application Support installation, active-book selection UI, user LaunchAgent, and Tailscale Serve identity/origin review.
- Backup-first migration, backup scheduling/commands, recovery of work in separately chosen backup/restore parents, and retained-backup workflow wiring.
- Explicit recovery/diagnostics for incomplete new-book initialization staging directories; these are preserved rather than guessed safe to delete.
- Safari end-to-end acceptance against an explicitly selected disposable book.

## Validation

`backend/tests/runtime.rs` uses temporary directories and an in-memory key store only. It verifies explicit creation and reopening, no implicit creation or key replacement, exclusive book locking, missing key/binding failure, corruption refusal before writable open, symlink/public-directory rejection, the fixed loopback address, static/API/unknown-route origin protection, duplicate Host rejection, write Origin requirements, CSP/cache headers, and encoded path traversal rejection.
