# ADR 0032: Recipient request and HPKE wrapping foundation

Status: implemented foundation; complete transfer workflow remains pending

## Scope

`recipient_transfer` implements the ADR 0024 recipient request and root-key
envelope boundary. It does not publish a `.kagextra` package, export a database,
select or switch a book, or replace the existing plaintext transfer foundation.
There is no claim that cross-Mac transfer is ready for use.

## Contracts

- Requests contain only format, version, random 128-bit request identifier,
  canonical X25519 public key, and SHA-256 public-key fingerprint.
- Private keys are persisted before returning a request, using create-only
  insertion. The macOS adapter uses a separate non-synchronizing local Keychain
  service, `kAccounting-Extra-01.recipient-private.v1`. Reads never generate,
  repair, replace, or delete a key. Tests use in-memory fixtures exclusively.
- Key generation uses operating-system entropy. Root-key wrapping uses the
  published `hpke` crate, RFC 9180 base mode, DHKEM(X25519, HKDF-SHA-256),
  HKDF-SHA-256, and ChaCha20-Poly1305. The sender RNG is ChaCha20 seeded with
  fresh operating-system entropy; random-generation failures are returned.
- Authenticated information and associated data bind the exact application
  domain, suite/version, request identifier, public-key fingerprint, and
  caller-supplied unique 128-bit package identifier. The encrypted package
  manifest must independently bind that same identifier when integrated.
- Base mode authenticates ciphertext and context, **not sender identity**.
  Source confirmation must display the request identifier and fingerprint.
- Public documents are bounded to 4096 bytes and reject unknown/duplicate
  fields, unsupported formats/versions/suites, noncanonical hexadecimal,
  noncanonical X25519 coordinates, and low-order public points.
- Envelopes contain only an encapsulated key and encrypted 32-byte root key.
  Private-key byte buffers and recovered root-key bytes are zeroized.
- A replay against another expected package identifier fails. Reopening the
  same package for deliberate recovery is allowed; single-use import state
  and preventing duplicate book publication belong to the integration layer.

## Validation and remaining release gates

Tests include the selected-suite RFC 9180 reference-vector decryption, a
request/wrap/unwrap/encrypted-payload round trip, malformed documents, missing
and wrong private keys, recipient/context/ciphertext tampering, and cross-package
replay. No real user Keychain entries or financial data are accessed.

The upstream `hpke` documentation states that its implementation has not been
audited. Dependency/security review and real Keychain acceptance remain release
gates, together with encrypted package layout, complete object verification,
durable export recovery, import lifecycle, UI confirmation, and cross-Mac
end-to-end tests. This ADR does not waive these gates.

## Sources

- [RFC 9180](https://www.rfc-editor.org/rfc/rfc9180.html)
- [hpke 0.13 API and implementation status](https://docs.rs/hpke/0.13.0/hpke/)
- [CFRG reference vectors](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/test-vectors.json)
