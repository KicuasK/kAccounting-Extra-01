# Open Questions

These items are intentionally unresolved. Do not turn them into implementation policy without a user decision or direct source review.

The generic account schema is designed to remain valid while these questions are open. They block default-data seeding and some future UI actions, not the tested hierarchy storage itself.

## Accounting and accounts

- What is the exact hierarchy in the handwritten chart-of-accounts image?
- Which top-level and intermediate categories, if any, are locked system nodes?
- What do the equity labels 「工作」 and 「資助」 mean in accounting terms?
- What usage policy distinguishes income labels 「正」 and 「偏」?
- When should a time deposit be current versus non-current in this personal reporting model?
- May an account with posted history later receive child accounts? If so, what is the safe conversion rule?
- What recognition rules apply to the two contract-related liability categories?
- Must sibling account names be unique, and if so, how should Latin-letter case differences be treated?
- May a category be deactivated while it still contains active descendants?
- Does the user need explicit sibling ordering, or is creation order sufficient for the MVP?
- Are user-visible account codes required in the MVP?

## MVP scope

- What exact stored/display scale should the TWD integer minor unit use?
- Are accounting-period closing and locking included?
- Is application authentication required in addition to Tailscale access?
- What backup frequency, retention, encryption, and restore-validation policy is required?

## Attachments and transfer storage

- Which entry or line records may own attachments, and may an unattached import
  exist temporarily?
- Which file types and per-file or per-book size limits are required?
- Should identical attachment content be deduplicated, and what deletion or
  audit-retention rules apply?
- Must attachments or `.kagextra` exports be encrypted at rest?
- Which removable-drive file systems must be supported for computer transfer?
- How should the later iPad Safari workflow receive or send a directory package
  when Finder package semantics are unavailable?

## Runtime

- Confirm the final runtime data directory name under macOS Application Support.
- Confirm when Tailscale login and Serve configuration should be activated.

## Journal interaction

- What fields belong to the entry header?
- Is reference numbering automatic, editable, or both?
- How should account search and keyboard navigation work?
- Should debit/credit entry use two columns, a direction selector, or both on smaller screens?
- Drafts may be changed or deleted without a count limit. What autosave timing
  and crash-recovery behavior is expected once the journal UI is designed?
- Which explanation fields apply to the whole entry versus individual lines?

## Reversal and correction policy

- May a reversal reuse an account that became inactive after the original posting, or must the account be explicitly reactivated first?
- May one posted entry have more than one reversal, and may a reversal itself be reversed?
- Must a reversal date be on or after the original entry date, and how will future period locks affect that rule?
- Should an explicit correcting entry store a permanent link to the corrected entry, or remain an ordinary balanced entry with an explanation/reference?

## Future TPASS tracking

- Is the first supported product fixed to 「基北北桃 1200」, or should the initial data model support multiple pass products?
- Should amortization be created as soon as its posting date is reached, or only after local time 23:59:59 in Asia/Taipei?
- Which operators, stations, and train types belong in the first release, and what authoritative source governs updates?
- Should archived ride evidence remain in break-even statistics, or should archive exclude it while preserving the record?
- What restrictions should apply when correcting a ride's date, pass plan, or pass-period association?
- Which receipt and ride-image attachment fields are specific to the later
  TPASS workflow beyond the general Version 01 attachment store?

