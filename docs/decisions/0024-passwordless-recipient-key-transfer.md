# ADR 0024: Passwordless recipient-key transfer

Status: accepted for Version 01

## Context

The user explicitly required that the application design no password. A
portable `.kagextra` package still needs an encryption key that the destination
Mac can recover without embedding a plaintext key beside the ciphertext or
depending on cloud synchronization.

Local application-managed backups and cross-Mac transfer serve different
purposes. Local backups provide rollback on the current Mac. A `.kagextra`
package is created only for moving the book to another computer.

## Decision

- Do not create or request an application password, backup password, transfer
  password, passphrase, PIN, recovery phrase, password hint, or password KDF.
- Generate device and transfer secrets with the operating-system CSPRNG. Store
  device private keys as non-synchronizing macOS Keychain items; never store
  them in SQLite, the source tree, a manifest, or plaintext application files.
- Encrypt local backups to the current Mac's device recipient key. They are
  local rollback artifacts, not the cross-device recovery format.
- Before a transfer, the destination Mac generates an X25519 recipient key pair
  and stores the private key in Keychain. It writes a strict, versioned public
  recipient-request file to user-selected removable storage. The request
  contains no secret key material.
- The source Mac validates that request, generates a random 256-bit transfer
  root key, encrypts every `.kagextra` payload through ADR 0023, and wraps only
  the transfer root key to the destination recipient.
- Wrap the root key with RFC 9180 HPKE base mode using
  DHKEM(X25519, HKDF-SHA-256), HKDF-SHA-256, and ChaCha20-Poly1305. Bind the
  application name, recipient-request identifier, recipient public-key digest,
  and package format version as authenticated context.
- Give every recipient request a random identifier and public-key fingerprint.
  The source confirmation shows the selected request identifier and fingerprint
  so an accidental stale or wrong request is visible without asking for a
  password.
- Keep a source-device-wrapped copy of the transfer root key only in private
  durable export work state so interrupted export verification can resume.
  The public package contains only the destination envelope.
- The destination unwraps the transfer key through its Keychain private key,
  authenticates and verifies every encrypted object, imports into a new local
  destination, and switches books only after complete success.
- Retain the destination private key after successful import while that book or
  its transfer package may still need recovery. Key deletion is an explicit
  lifecycle operation and never an automatic side effect of import.

## Failure behavior

- A missing destination private key makes that package unrecoverable; there is
  no backdoor or password fallback.
- A malformed, unsupported, substituted, or wrong recipient request fails
  before package publication.
- Wrong recipient keys, modified envelopes, modified context, and modified
  ciphertext fail closed without publishing plaintext.
- Cancellation, capacity failure, restart, or verification failure preserves
  the source book and every pre-existing valid destination.

## Verification requirements

- RFC 9180 compatible test vectors for the selected suite.
- Request parsing tests for unknown fields, unsupported versions, malformed
  keys, duplicate identifiers, and noncanonical encodings.
- Wrong-recipient, modified-envelope, modified-context, and replay tests.
- End-to-end tests covering request creation on a destination fixture, export
  on a source fixture, interrupted restart recovery, and verified import.
- Tests proving no password, passphrase, PIN, recovery phrase, or plaintext
  secret is serialized in a request, package, error, or operation state.

## Sources

- [RFC 9180: Hybrid Public Key Encryption](https://www.rfc-editor.org/rfc/rfc9180.html)
- [Apple Keychain Services](https://developer.apple.com/documentation/security/keychain-services)
- [Apple: Storing keys in the Keychain](https://developer.apple.com/documentation/security/storing-keys-in-the-keychain)

## Consequences

Moving a book requires creating a recipient request on the new Mac before the
old Mac exports the final package. This extra application-guided round trip is
the usability cost of passwordless, offline, destination-bound transfer. It
does not require a public service, cloud key escrow, Tailscale Funnel, or a
shared application secret.
