# Single-file recipient import

`container_import::import_recipient_container` accepts a `Read + Send + 'static`
stream, canonical private work parent, new destination, parsed recipient request,
and owned recipient/device key-store adapters. It returns the existing verified
recipient-import outcome. It never selects a book or overwrites a destination.

The transport parser reads in at most 64 KiB chunks and enforces caller-supplied
entry, individual-object, and aggregate capacity limits. HTTP callers must bound
their producer queue too. It stages the public envelope and encrypted package
objects only. Transport hashes are not authenticity: the complete recipient
importer must then authenticate the recipient envelope, encrypted manifest,
database snapshot, attachment references and ciphertexts, and re-encrypt the
new book under fresh device keys before publication.

Dropping the caller's future detaches an owned operation. A blocking input read
keeps its reader and stage alive until it returns. The caller must close a
bounded HTTP bridge on disconnect to unblock that read. A disconnect is not a
promise to undo a published import. Inspect destination and recipient-import
recovery state before retrying with another destination.

After a fully verified successful import, the owned worker removes only its
internal encrypted container copy. The original Downloads or external-media
package is always preserved. Failed or interrupted imports retain staging under
`.kAccounting-Extra-01-container-import-<random>/package` for inspection.

Cleanup authority exists only in the live worker's recorded parent, work,
package, attachment-directory and individual-file identities. It first rejects
unknown directory entries and replaced files, then removes only known files and
empty known directories; it never recursively follows paths. A cleanup error
returns the successfully published book with `cleanup_warning` instead of
misreporting the import as failed. A path or directory name alone never permits
later adoption of cleanup authority. Existing recipient-import recovery remains
separate with its existing authentication requirements.

Safari downloads one `.kagextra` file to Downloads; the user may manually move
that file to APFS/exFAT media. This helper consumes the bytes without changing
that source file. Legacy directory packages remain supported by the lower-level
recipient importer. No UI is implemented here.
