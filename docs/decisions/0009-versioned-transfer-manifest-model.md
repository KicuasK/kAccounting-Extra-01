# ADR 0009: Versioned logical transfer-manifest model

Status: accepted

## Decision

The first transfer-manifest model has format version `1` and declares:

- one database integrity descriptor;
- zero or more attachment entries;
- for each attachment, a safe relative logical path under `attachments/` and
  its SHA-256 plus exact byte-length descriptor.

Decoded input is validated before referenced files are opened. Version 01 fails
closed on unknown manifest versions, absolute paths, parent/current-directory
components, platform separator ambiguity, paths outside `attachments/`, and
duplicate logical paths. Valid attachment entries are ordered by logical path
so later encoders can produce deterministic output.

The logical model is independent from its encoding. ADR 0011 selects a macOS
directory package as the physical container, and ADR 0012 selects the strict
`manifest.json` encoding. A manifest path is not an attachment's
accounting-domain identifier.

## Rationale

Compatibility and path safety can be proven before choosing the physical
container. Keeping the database descriptor separate ensures that a manifest
cannot disguise an attachment as the book database. Deterministic ordering
makes exports and tests easier to compare without requiring deduplication.

## Consequences

The `.kagextra` package manifest must decode into this validated model or adopt
a new explicit manifest version. Import must still verify every declared
object, reject undeclared or missing required content according to the package
policy, and validate the SQLite snapshot independently.
