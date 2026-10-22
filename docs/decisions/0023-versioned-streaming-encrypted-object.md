# ADR 0023: Versioned streaming encrypted-object format

Status: accepted for the Version 01 encryption primitive

## Context

Attachments may be as large as 250 MiB and a book may contain 20 GiB of
attachments. Encryption therefore cannot require a complete object in memory.
The format also has to reject reordered, removed, duplicated, truncated, or
modified chunks before any candidate is published.

The user delegated security and efficiency engineering choices on 2026-10-22.
User input is reserved for choices that change the visible workflow.

## Decision

- Use XChaCha20-Poly1305 from the audited RustCrypto implementation with the
  RustCrypto STREAM BE32 construction. STREAM authenticates segment order and
  the final segment, preventing naive chunked-encryption reorder and truncation
  failures.
- Process plaintext in fixed 64 KiB chunks. At most one plaintext chunk, its
  authentication tag, and bounded framing are held by the primitive at a time.
- Derive a separate 256-bit object key from the supplied root key with
  HKDF-SHA-256, a random 256-bit salt, and a format-and-purpose-specific info
  label. Do not use the root key directly as an AEAD key.
- Generate a fresh random STREAM nonce for every object from the operating
  system CSPRNG. Nonce or salt generation failure aborts before publication.
- Prefix every object with a fixed 72-byte Version 01 header containing magic,
  format version, cipher/STREAM/KDF identifiers, object purpose, chunk size,
  salt, STREAM nonce, and zeroed reserved bytes. Authenticate the exact header
  as associated data for every chunk.
- Frame each encrypted chunk with a big-endian 32-bit ciphertext length.
  Enforce the declared chunk limit, require exactly one authenticated final
  chunk, reject trailing bytes, and reject all malformed or unsupported header
  values before emitting a valid completed object.
- Treat authentication failure, wrong key, unsupported version, malformed
  framing, truncation, and trailing data as hard failures. Decryption callers
  must write only to private temporary storage and publish only after the whole
  stream succeeds.
- Zeroize root, derived, temporary AEAD, and cipher key material on drop or
  immediately after initialization. Never log or encode keys in manifests,
  errors, paths, or telemetry.

## Key boundaries

- Local object root keys will be stored as non-synchronizing macOS Keychain
  items. Apple documents Keychain Services as the storage location for small
  secrets such as cryptographic keys.
- Local backups use the local device recipient key in Keychain and are intended
  for rollback on that Mac.
- Portable `.kagextra` root keys are random application-generated keys wrapped
  to a destination Mac's public recipient key as defined by ADR 0024. No
  password, passphrase, PIN, recovery phrase, or password KDF is used.

## Verification requirements

- A fixed deterministic test vector for header, derivation, and ciphertext.
- Round trips for empty, sub-chunk, exact-chunk, multi-chunk, and large inputs.
- Rejection tests for wrong key, header tampering, ciphertext tampering,
  truncation at every framing boundary, reordered/duplicated chunks, oversized
  frame lengths, trailing data, unsupported identifiers, and RNG failure.
- Injected read and write failures proving partial output is never treated as a
  completed encrypted object.
- A bounded-I/O test proving the primitive never requests an unbounded read or
  write for large input.

## Sources

- [Apple Keychain Services](https://developer.apple.com/documentation/security/keychain-services)
- [Apple: Storing keys in the Keychain](https://developer.apple.com/documentation/security/storing-keys-in-the-keychain)
- [RustCrypto ChaCha20Poly1305](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/)
- [RustCrypto AEAD-STREAM](https://docs.rs/aead-stream/0.6.0/aead_stream/)

## Consequences

This format is an internal encrypted-object layer, not the final attachment
schema or `.kagextra` manifest. Integrations retain the existing no-overwrite,
private-staging, synchronization, descriptor, and verification rules. The
physical transfer manifest must move sensitive names and metadata behind the
encrypted boundary when its next format version is designed.
