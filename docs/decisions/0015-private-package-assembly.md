# ADR 0015: Verified private package assembly

Status: accepted engineering decision and implemented

## Decision

Package assembly is separate from publication. Given a private staging tree and
its validated logical manifest, the assembler:

1. verifies the source database and every declared attachment before creating
   destination work;
2. creates a new, non-existing work root with owner-only directory permissions;
3. streams the database and declared attachments into that work root, checking
   every copied length and SHA-256 digest;
4. creates only the attachment directories required by declared paths;
5. writes deterministic `manifest.json` as an owner-only file and synchronizes
   that file;
6. loads and verifies the complete physical package again.

The source remains unchanged. An existing work path is never reused or
modified. If assembly fails after work begins, the incomplete private directory
remains distinguishable for explicit recovery or cleanup rather than being
published or mistaken for a valid package.

## Rationale

Separating assembly from publication prevents users from observing a package
while its database, attachments, or manifest are incomplete. Revalidating the
finished directory catches copy damage, undeclared residue, invalid database
content, and assembly mistakes before any public destination name is created.

## Consequences

The caller must choose a private work path beside the future destination so the
atomic no-replace rename from ADR 0013 can later publish it without crossing a
file-system boundary. Assembly alone does not publish the package or report
transfer success. ADR 0016 defines the conservative synchronization,
publication, and public verification steps that follow.
