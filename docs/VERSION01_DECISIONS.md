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

The user responded `全部採用建議` on 2026-10-22 and separately answered `Yes`
to the at-rest encryption question on 2026-10-22. Every product decision above
is approved. ADR 0023 records the delegated cryptographic format, algorithm,
key derivation, local key-storage boundary, and failure behavior. The
user separately required that no password be designed. ADR 0024 records the
resulting passwordless recipient-key transfer workflow.
