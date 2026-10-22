# Version 01 Decision Gate

Status: fully approved by the user on 2026-10-22.

This is the approved decision record for the remaining Version 01 work. Any
future change requires an explicit user decision and must be recorded here
before affected implementation changes.

## 1. TWD exact-unit scale

Recommended: store and display whole New Taiwan dollars, so `1` stored minor
unit displays as `NT$1` and the Version 01 UI has no fractional amount field.

Impact: choosing cents instead changes validation, formatting, fixtures, import
rules, and every amount example before real books exist.

Decision: approved as recommended on 2026-10-22.

## 2. General Journal fields and saving

Recommended:

- Entry header: accounting date, optional user-editable reference, and optional
  whole-entry explanation.
- Line fields: account, debit amount, credit amount, and optional line memo.
- Keep separate debit and credit columns on Mac Safari.
- Save a draft only through an explicit Save action in Version 01; do not add
  timed autosave or implicit crash recovery.
- Account lookup supports keyboard search by account name and ancestor path.

Impact: this is the required interaction approval before implementing the
General Journal editor.

Decision: approved as recommended on 2026-10-22.

## 3. Reversal and correction

Recommended:

- A reversal is a new balanced posted entry containing the original lines with
  debit and credit sides exchanged; amounts remain positive.
- Require the reversal date to be on or after the original entry date.
- Permit at most one direct reversal of an original posted entry in Version 01.
- Require any inactive account used by the original entry to be explicitly
  reactivated before reversal, preserving the active-posting-account invariant.
- An explicit correction is an ordinary new balanced entry linked permanently
  to the original entry and does not mutate either posted entry.
- Do not allow reversing a reversal in Version 01; use an explicit correction
  with an explanation instead.

Impact: these choices determine the forward-only migration and atomic domain
operations that must be implemented before the editor.

Decision: approved as recommended on 2026-10-22.

## 4. Attachment ownership and lifecycle

Recommended:

- Version 01 attachments belong to a journal entry, not to an individual line.
- Allow zero or more attachments on drafts and posted entries.
- Store immutable attachment bytes outside SQLite and keep stable metadata,
  SHA-256, exact byte length, original display name, and permanent journal-entry
  reference in SQLite.
- Replacing or deleting a draft may remove only attachment objects owned solely
  by that draft. Posted-entry attachments cannot be silently replaced or
  deleted.
- Do not deduplicate attachment content in Version 01; each attachment owns one
  object, avoiding shared-reference deletion ambiguity.
- Reject executables and macOS package/directory uploads; otherwise accept
  regular files up to 250 MiB each and 20 GiB per book.
- Require application-managed encryption at rest for live attachment objects,
  application-managed backups, and `.kagextra` exports. Keep SHA-256 and exact
  byte-length integrity verification mandatory; hashing is not encryption.

Impact: entry ownership is the smallest model that supports many receipts and
documents while keeping export selection and retention deterministic.

Decision: approved as recommended on 2026-10-22. The user separately answered
`Yes` to application-managed at-rest encryption on 2026-10-22.

## 5. Account-list policy

Recommended:

- Use creation order for siblings in Version 01.
- Do not add user-visible account codes in Version 01.
- Require sibling names to be unique after the existing edge-whitespace
  normalization, while preserving letter case and Unicode characters.
- Do not allow a category to be deactivated while it has active descendants.
- Do not allow an account with posted history to acquire children; create a new
  category and move only accounts whose history permits the move.

Impact: these choices close the remaining account schema and UI behavior gaps
without inventing a reclassification workflow for historical postings.

Decision: approved as recommended on 2026-10-22.

## 6. Balance sheet and periods

Recommended:

- Version 01 provides an as-of balance sheet derived from posted lines.
- Current/non-current presentation follows the accepted account hierarchy; the
  application does not infer classification from names or maturity dates.
- Do not add accounting-period closing or locking in Version 01.
- Do not generate automatic closing entries in Version 01.

Impact: the existing five-element summary can remain policy-neutral while the
balance-sheet presentation uses explicit account placement.

Decision: approved as recommended on 2026-10-22.

## 7. Backup policy

Recommended:

- Provide an explicit Back Up command and create a verified backup before a
  migration or import changes the live book.
- Keep the newest 10 application-managed local backups by default; never remove
  a user-selected `.kagextra` export automatically.
- Restore only into a new local destination, verify it completely, and switch
  the active book only after success.
- Encrypt application-managed backups at rest using the separately reviewed
  Version 01 cryptographic design.

Impact: scheduling can be added later without weakening Version 01 restore
safety or silently overwriting the live book.

Decision: approved as recommended on 2026-10-22, including retention of the
newest 10 application-managed local backups and application-managed at-rest
encryption.

## 8. Runtime and private access

Recommended:

- Runtime root:
  `~/Library/Application Support/kAccounting-Extra-01/`.
- Bind only to `127.0.0.1:3000`; never enable Funnel.
- Start after login through a user LaunchAgent.
- Use Tailscale Serve for private HTTPS and restrict access to the user's own
  Tailscale identity through the tailnet policy.
- Do not add an application password, backup password, transfer password,
  passphrase, PIN, or recovery phrase. Device keys and recipient public keys
  provide the application-managed cryptographic boundary.

Impact: this closes the listener, authentication-boundary, and deployment
decisions required before mutation routes are exposed.

Decision: approved as recommended on 2026-10-22.

## Approval record

### Additional operational decisions, 2026-10-22

Latest scope amendment: the user deferred physical APFS/exFAT external-drive
acceptance to Version 02 because no external drive is available. Existing code,
tests and acceptance tools must be preserved. Physical-media verification no
longer blocks Version 01 or its UI phase; no physical acceptance pass is claimed.
The APFS/exFAT compatibility target below remains planned for Version 02.

During the six-agent non-UI build session the user explicitly selected:

- Require explicit book selection each time the application opens. Do not
  automatically open the previously used book.
- After successful import or restore, retain the current book. Switching to
  the verified new destination requires a separate explicit user selection.
- Cross-Mac removable storage must cover APFS and exFAT. The user answered that
  either format may be used. This is a required compatibility/acceptance target,
  not a claim that current private staging works on exFAT. Do not silently
  require the user to reformat their transfer disk.

These refine startup/selection behavior without changing backup validation,
new-destination restore or accounting policy.

During the second timed build session the user further selected:

- After a fully verified successful import, automatically remove only internal
  encrypted container staging copies. Preserve the user's original Downloads or
  external-media package. Failed/interrupted staging remains for inspection and
  recovery; uncertain or substituted paths must never be silently removed.
- Browser exports download to Safari's Downloads destination first; the user
  manually moves the encrypted transfer file to APFS/exFAT media afterward.
  Do not require a native server-side external-drive picker for this workflow.
- Real macOS Keychain acceptance may create dedicated new test items, isolated
  by randomly generated identifiers, and retain them for user inspection. Only
  those new items may be read; existing financial keys remain untouched. Any OS
  authorization prompt is completed by the user. No item deletion was authorized.

The user responded `全部採用建議` on 2026-10-22 and separately answered `Yes`
to the at-rest encryption question on 2026-10-22. Every product decision above
is approved. ADR 0023 records the delegated cryptographic format, algorithm,
key derivation, local key-storage boundary, and failure behavior. The
user separately required that no password be designed. ADR 0024 records the
resulting passwordless recipient-key transfer workflow.

### Additional acceptance permissions, 2026-10-22 evening

- The user authorized installation of the official Tailscale distribution and
  explicitly accepted its service terms. Account login and macOS network/VPN
  authorization are performed by the user. No public service or Funnel is
  authorized. Installation or a permission switch alone is not connection proof.
- The user authorized one temporary, uniquely identified background acceptance
  job using an empty disposable collection. Remove that exact job afterward;
  this does not authorize a permanent accounting login item.
- The user authorized a new isolated test Keychain, including lock/unlock using
  a random memory-only operating-system fixture credential. Do not read existing
  keys or change the default Keychain/search list. Retain dedicated test items
  for inspection. This does not introduce a product password.
- When the Tailscale system extension remained in a pending transition, the user
  allowed a Mac restart during this round after saving work. Record actual
  restart and post-restart verification separately; permission is not evidence
  that a restart or network login has occurred.

### Private HTTPS operational acceptance, 2026-10-22 evening

The user enabled HTTPS certificates while explicitly deselecting optional Funnel.
The user ran the reviewed new-empty-directory preparation with administrator
authorization; this directory remained unsupported by the installed GUI sandbox.
The user then explicitly started, logged into and authorized Unix Serve for a
separate temporary, memory-state, userspace-networking test node, with DNS/route
acceptance disabled. This was not permission for a permanent root service or a
replacement GUI installation. After successful disposable HTTPS acceptance, the
user stopped both foreground test windows; independent cleanup checks passed.
No real financial books or existing financial keys were authorized for testing.
