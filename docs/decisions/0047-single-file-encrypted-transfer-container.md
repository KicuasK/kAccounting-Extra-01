# ADR 0047: Single-file encrypted transfer transport

Status: accepted technical implementation; 2026-10-22.

The user chose Safari downloads to Downloads, followed by manually moving the
package to APFS or exFAT. Existing recipient packages are directories. A minimal,
versioned transport file allows one browser download without adding a password
or duplicating encryption. The envelope is public; every financial payload is
already recipient-encrypted. No plaintext SQLite snapshot enters the container.

Use deterministic framing documented in `docs/TRANSFER_CONTAINER.md`; no archive
library, compression, external executable, arbitrary paths, or directory walk.
Callers supply exact descriptors from a verified package context. Streaming uses
64 KiB chunks. Exact lengths are unsigned checked integers. Extraction creates a
new private stage and refuses existing destinations, duplicate/unknown names,
noncanonical ordering, truncation, bad digests, over-capacity input and trailers.
Parsing and transport hashes do not authenticate an attacker-controlled file.
Full recipient HPKE/object authentication, manifest-reference matching and SQLite
verification remain mandatory before import/publication. Failed stages remain
known and private for explicit recovery; original sources remain untouched.

The container is a download transport, not an atomic or authenticated filesystem
format. HTTP callers must only report successful exports after the writer has
completed, and must discard interrupted downloads. Media publication continues
to use separately reviewed APFS/exFAT durability and collision handling.
