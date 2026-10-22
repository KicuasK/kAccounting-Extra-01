# ADR 0025: Immutable Journal Reversals and Corrections

Status: accepted

## Context

Version 01 requires posted history to remain immutable while still allowing a
user to correct mistakes. The approved policy permits one exact direct
reversal of an original entry and any balanced explicit correcting entry that
retains a permanent link and explanation.

## Decision

- Migration `0004` adds nullable self-references for the entry being reversed
  or corrected, plus an optional whole-entry explanation.
- An entry can carry at most one relationship type. Relationship fields and a
  correction's required explanation are immutable from insertion onward, and
  posted rows retain the existing full immutability protection. The optional
  explanation of an ordinary draft remains available to the separately planned
  draft-metadata workflow.
- A unique partial index permits at most one direct reversal for each original
  entry.
- A reversal is created and posted in one `BEGIN IMMEDIATE` transaction. It
  copies line order, accounts, and positive amounts exactly while exchanging
  debit and credit sides. Its date cannot precede the original, and its source
  cannot itself be a reversal.
- Account posting eligibility is checked again inside the reversal transaction.
  An inactive original account therefore must be explicitly reactivated first.
- A correction is an ordinary balanced entry posted atomically with a permanent
  link and a non-blank explanation. A correction may target a reversal because
  direct reversal of a reversal is not allowed in Version 01.
- SQLite posting triggers independently enforce relationship, date,
  explanation, and exact-reversal-line rules. Startup integrity checks and
  repository reads fail closed on stored relationship corruption.

## Consequences

Reports continue to derive solely from posted lines: original, reversal, and
correcting entries all remain visible audit records. No balance is edited
directly, no posted entry is mutated, and no hidden netting rule is introduced.
Mutation HTTP routes and UI confirmation flows remain separate later work.
