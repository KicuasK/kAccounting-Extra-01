# ADR 0011: `.kagextra` is a macOS directory package

Status: accepted by user decision

## Decision

The physical `.kagextra` transfer container is a versioned macOS directory
package. Finder may display it as one document once the future application
registers the extension, while the application treats it as a directory tree.

Its logical contents are:

- `manifest.json`, using the strict encoding selected by ADR 0012;
- `book.sqlite3`, a standalone verified database snapshot;
- `attachments/`, containing the objects declared by the manifest.

The supported export, removable-drive copy, verification, and import workflow
remains application-managed. Users are not expected to open the package and
manually replace internal files. The package is a transfer artifact, not the
live book and not a claim that the operating system stores it as one file.

## Safety requirements

- A `.kagextra` path that already exists is never replaced implicitly.
- Work-in-progress directories remain hidden and distinguishable from a
  verified published package.
- Verification rejects symlinks, unsafe relative paths, missing declared
  objects, damaged content, unsupported manifest versions, and an invalid or
  unrelated SQLite database.
- Copying to removable storage must verify the destination package after the
  copy, not merely trust a successful filesystem call.
- The old computer's local live book is never deleted automatically.

## Consequences

Directory packages make large attachments streamable without first assembling
one large archive, but partial copies and manual internal edits are possible at
the filesystem level. The manifest and full post-copy verification are
therefore mandatory.

The hard-link publication primitive remains suitable only for individual local
files. ADR 0013 adds the distinct atomic no-replace rename primitive required
to make one fully prepared private package directory visible. ADR 0014 adds
strict loading and read-only verification of the complete physical package.
ADR 0015 adds private package assembly from an already verified staging tree.
ADR 0016 adds conservative synchronization, atomic publication, and
post-publication verification. ADRs 0017–0019 compose those primitives into a
verified export from one SQLite snapshot and an explicit attachment set; ADRs
0020–0021 add durable export state and conservative restart classification.
Live-book import replacement, automatic cleanup of ambiguous private work, and
removable-drive compatibility still require dedicated decisions,
implementation, and tests.

Package registration and Finder presentation belong to the future macOS
application packaging work. Browser and iPad transfer behavior must be designed
separately rather than assuming Finder package semantics in Safari.
