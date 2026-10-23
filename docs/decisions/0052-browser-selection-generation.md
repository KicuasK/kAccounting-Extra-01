# Browser requests pin the selected-book generation

Status: accepted engineering decision, 2026-10-23. No accounting-policy change.

## Problem

Browser tabs share the process-local chooser. A tab can retain book A while
another switches to B. Client cancellation cannot bind a later request to A;
comparing book IDs also misses A → B → A transitions.

## Decision

Give each successful replacement a random 128-bit `selection_id`. Expose it in
session status without persisting it or using it as authentication. Re-selecting
the active book preserves it; returning after a switch gets a new generation.
Failure preserves the previous selection and generation.

New browser data requests and select/close transitions send
`X-kAccounting-Selection`. Check the single canonical header under the same
read/write lease as dispatch/transition; `unselected` pins an empty session.
Mismatch is 409 `selection_changed`; malformed/duplicate guards are 400
`invalid_selection_guard`. Failure precedes accounting dispatch/key reads.
Existing origin, identity, root-peer, job and leased-response checks remain.

Earlier integration callers may omit the header for compatibility. This is an
opt-in concurrency contract, not authentication. Every new browser financial
control must supply it. The frontend clears content on transitions, reconciles
after failed writes and checks generation again before rendering.

## Verification

Disposable fake-key tests cover matching reads, stale read/write/select/close
refusal without key access, no unintended account creation, switch-back rejection,
same-book preservation and malformed/duplicate headers. Existing leased-body and
cancelled-shutdown regressions remain. No real books or keys were used.
