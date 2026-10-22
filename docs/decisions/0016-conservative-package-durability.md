# ADR 0016: Conservative package durability boundary

Status: accepted by user decision and implemented

## Decision

A `.kagextra` publication reports success only after all of these steps finish:

1. the complete private package passes strict layout, digest, and SQLite audits;
2. `book.sqlite3`, every declared attachment, and `manifest.json` each accept a
   full synchronization request;
3. every nested attachment directory and the package root accept full
   synchronization, from deepest directory to shallowest;
4. one atomic no-replace rename gives the package its public destination name;
5. the destination's parent directory accepts full synchronization so the new
   name is included in the success boundary;
6. the public destination independently passes the complete package audit.

If a file system does not support any required synchronization or exclusive
rename operation, publication fails closed. There is no relaxed fallback.

A failure before the atomic rename leaves the private work directory and any
existing destination unchanged. A failure after the rename reports explicitly
that a destination was published but did not reach verified success. The
application does not automatically delete that destination or the original
staging source.

## Rationale

Successful write or rename calls alone do not prove that package contents and
directory entries have crossed the selected durability boundary. Synchronizing
all files, then directories, then the destination parent gives the most
conservative behavior available through the supported filesystem interface.
Post-publication verification also catches damage or unexpected changes at the
actual public path.

## Consequences

Some removable-drive file systems may be rejected even when ordinary Finder
copying appears to work. This is intentional: compatibility cannot take
priority over preserving the only valid accounting book. Supported removable
file systems still require explicit compatibility testing and documentation.
