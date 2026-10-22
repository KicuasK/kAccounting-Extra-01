# ADR 0014: Strict directory-package verification

Status: accepted engineering decision and implemented

## Decision

Loading a physical `.kagextra` package is read-only and fail-closed. The loader
requires the package root to be a real directory and its top level to contain
exactly:

- the regular non-symlink file `manifest.json`;
- the regular non-symlink file `book.sqlite3`;
- the real non-symlink directory `attachments/`.

After strictly decoding `manifest.json`, every file below `attachments/` must
be declared by the manifest. A nested directory is allowed only when it is a
required prefix of at least one declared attachment path. Symlinks, special
filesystem objects, non-UTF-8 names, undeclared files, unexpected directories,
and extra top-level entries are rejected.

The manifest name must identify the same regular non-symlink file before,
during, and after it is opened and read. A concurrent replacement is rejected
rather than following a newly introduced symlink or validating bytes that no
longer belong to the package entry. The loader reads it again after the complete
layout, object, and database audit and rejects any byte change, so one result
cannot combine an earlier manifest with later package contents.

Only after the physical layout passes does the loader invoke the existing
staged-transfer verifier. That verifier recomputes every declared length and
SHA-256 digest and independently audits the SQLite snapshot's application
identity, migrations, schema, constraints, and accounting semantics.

## Rationale

A valid manifest does not make undeclared package content safe. Strict layout
checking prevents hidden payloads, symlink escapes, and ambiguous partial
packages from being accepted or recopied. Reusing the staged verifier keeps
container policy separate from database and content-integrity checks.

## Consequences

Manually adding Finder metadata or other files inside a `.kagextra` package
makes it invalid until those files are removed. Package size limits and the
supported maximum package size remain unresolved product decisions; this
loader does not silently choose them. ADR 0016 separately selects and
implements the conservative synchronization checkpoint for publication.
