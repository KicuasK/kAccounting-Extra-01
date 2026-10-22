# ADR 0033: Book commands and private integration preview

Status: accepted engineering implementation of approved Version 01 decisions.

## Decision

- Serve accounting writes, attachment metadata/list/verification/draft detach,
  and manual backup at the fixed private loopback boundary. Every command
  router keeps its own exact Host/Origin check, body limit, and private response
  headers when merged with the static application router.
- HTTP never accepts a server filesystem source path for attachment import.
  Upload/download remain unavailable until bounded-memory, fully verified
  streaming is integrated. Identifiers, revisions, sizes, and money retain
  exact string contracts. Domain repositories enforce ownership and immutability.
- `POST /api/backups` accepts an empty JSON object and writes only beneath the
  book's private `backups` directory. It uses a read-only view of the loaded
  device key. Requests neither read Keychain nor create keys.
- Report verified backup completion separately from later retention failures.
  Authenticated book identities prevent pruning another book's backups.
- Explicit CLI commands provide backup, new-destination restore, create-only
  restored-book backup identity setup, offline attachment collection, and public
  recipient requests. None changes active-book selection or runs during builds.
- The launcher generator prepares a new private plist without installation,
  service activation, or Tailscale changes. Login and real Keychain behavior
  still require Mac acceptance.
- The disposable HTTP example generates invented data and memory-only keys in
  a random temporary directory. It cannot select an existing book, does not
  access Keychain, and cleans its fixture after normal shutdown.

## Verification and remaining boundary

Tests cover exact integers, source rejection, immutable posted attachments,
stale revisions, authenticated backup/retention, zero Keychain calls inside HTTP
handlers, backup/restore/new namespace/backup, offline collection, preserved
destinations, and public-only recipient files. Actual loopback HTTP smoke checks
cover the frontend, health, a balanced posted report, and origin rejection.

The frontend remains the read-only account tree. Complete encrypted `.kagextra`
packages, attachment upload/download, legacy migration checkpoints, Tailscale
identity integration, and the full Safari workflow remain release work.
