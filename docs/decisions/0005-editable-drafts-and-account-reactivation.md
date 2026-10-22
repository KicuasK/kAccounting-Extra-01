# ADR 0005: Editable drafts and account reactivation

Status: accepted on 2026-10-22.

## Decision

- A journal draft may be empty, incomplete, or unbalanced.
- A draft may be replaced or deleted without an artificial count limit.
- A saved draft line temporarily prevents deletion of its referenced account.
  Deleting or replacing the draft removes obsolete line references, so an
  otherwise unused account can then be deleted.
- Replacing a draft is atomic: either its new date and complete line set are all
  stored, or the previous draft remains unchanged.
- Draft reads return a monotonic revision. Replace, delete, and post operations
  require the caller's current revision so simultaneous Mac and iPad sessions
  cannot silently overwrite one another.
- Posting is the boundary that requires at least two lines, equal debit and
  credit totals, and currently active posting-leaf accounts.
- A posted entry remains immutable and cannot be deleted. Its future correction
  workflow will use reversal or an explicit correcting entry.
- An account may be deactivated and later reactivated. While inactive, it cannot
  receive new journal lines; after reactivation it is eligible again if it is
  still a posting leaf.

## Scope

This decision covers backend and SQLite behavior only. It does not approve or
specify the journal-entry UI, autosave timing, recovery prompts, explanations,
references, or reversal-date policy.
