# Open Questions

These items are intentionally unresolved. Do not turn them into implementation policy without a user decision or direct source review.

The user delegated security and efficiency engineering choices on 2026-10-22.
Questions in those areas are engineering review work, not requests for the user,
unless a choice changes the supported workflow or other user-visible behavior.

The actionable Version 01 recommendations in `VERSION01_DECISIONS.md` were
approved by the user on 2026-10-22. Items resolved there are no longer open and
must not be reintroduced as implementation questions.

The generic account schema is designed to remain valid while these questions are open. They block default-data seeding and some future UI actions, not the tested hierarchy storage itself.

## Accounting and accounts

- What is the exact hierarchy in the handwritten chart-of-accounts image?
- Which top-level and intermediate categories, if any, are locked system nodes?
- What do the equity labels 「工作」 and 「資助」 mean in accounting terms?
- What usage policy distinguishes income labels 「正」 and 「偏」?
- When should a time deposit be current versus non-current in this personal reporting model?
- What recognition rules apply to the two contract-related liability categories?

## Four-digit account codes (new UI request)

The later user request supersedes the earlier no-code display policy: four digits
and account name in one column. The core has AccountId, not this code contract.
Allocation/uniqueness, reuse, category/posting mapping, existing-account migration
and capacity limits remain to be designed before implementation. Mockup codes and
classifications are examples, not an approved default chart of accounts.

## Credit-card core/API review (pending user policy)

See [credit-card assessment](CREDIT_CARD_CORE_REVIEW_2026-10-22.md). The current
core has generic liability accounting but no card subtype/limit/derived-capacity
contract. The UI concern that logic may change is not approval to copy legacy
rules. Decide signed account-list balance presentation, separate available credit,
prepayment/over-limit behavior, reversals/backdating and effective-dated limit
validation before implementation. Available credit must never replace a report
account balance.

## Encryption engineering

Application-managed at-rest encryption is required for live attachment
objects, application-managed backups, and `.kagextra` exports. ADR 0023 records
the engineering decisions for the encrypted-object container, authenticated
streaming, key derivation, and local Keychain boundary. ADR 0024 records the
passwordless recipient-key transfer workflow. No application password,
passphrase, PIN, or recovery phrase may be introduced.

- Is encryption of the live SQLite database required in a later version? The
  current Version 01 decision covers attachment objects, backups, and exports,
  not the live database.

## Attachments and transfer storage

- Updated user scope on 2026-10-22: physical APFS/exFAT external-drive acceptance
  is deferred to Version 02; no external drive is available. Preserve all existing
  code/tests/tooling; this is not a Version 01 or UI-start blocker.
- Resolved on 2026-10-22: computer transfer must cover APFS and exFAT. Physical
  removable-drive acceptance and exFAT-safe staging/publication remain engineering
  work; reformatting must not become an implicit workflow requirement.
- How should the later iPad Safari workflow receive or send a directory package
  when Finder package semantics are unavailable?

## Future TPASS tracking

- Is the first supported product fixed to 「基北北桃 1200」, or should the initial data model support multiple pass products?
- Should amortization be created as soon as its posting date is reached, or only after local time 23:59:59 in Asia/Taipei?
- Which operators, stations, and train types belong in the first release, and what authoritative source governs updates?
- Should archived ride evidence remain in break-even statistics, or should archive exclude it while preserving the record?
- What restrictions should apply when correcting a ride's date, pass plan, or pass-period association?
- Which receipt and ride-image attachment fields are specific to the later
  TPASS workflow beyond the general Version 01 attachment store?

