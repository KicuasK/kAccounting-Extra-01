# ADR 0001: Adjacency-list account hierarchy with guarded posting leaves

- Status: accepted for the initial schema
- Date: 2026-10-21
- Scope: generic chart-of-accounts structure only

## Context

The user must be able to add multiple levels below broad default categories, such as 「銀行 → 某銀行活期存款」 or 「食 → 早餐」. Category nodes organize the tree, while only eligible detailed accounts may receive journal lines.

The exact handwritten hierarchy and several labels remain under review. The storage model therefore must support arbitrary valid trees without hard-coding those unresolved labels as accounting policy.

## Decision

Use an adjacency list in `accounts`:

- `parent_id` references another account or is `NULL` for a root.
- `id` is a stable surrogate key and cannot be changed after insertion.
- `kind` distinguishes `category` from `posting`.
- `element` is stored on every node and must match its parent.
- Posting nodes cannot have children.
- Recursive SQLite validation prevents moving a node below itself or one of its descendants.
- Restricted foreign keys prevent deleting a parent before its children.
- Every mutable row carries a monotonic `revision` used for optimistic concurrency.
- A trigger requires every update, including direct SQL, to advance `revision` by exactly one.

The Rust access layer repeats essential validation and reconstructs a defensive tree before hierarchical data reaches the UI.

## Alternatives considered

### Materialized path

Rejected for the first version. It simplifies subtree reads but makes moves rewrite every descendant path and introduces path-encoding rules unrelated to accounting.

### Nested sets

Rejected. Reads are efficient, but insertion and reordering require broad left/right boundary updates. That complexity is unnecessary for a personal chart of accounts.

### Closure table

Deferred. It supports fast ancestry queries and strong cycle checks but adds a second relationship table and more transactional maintenance. The expected personal account tree is small enough for recursive queries and in-memory reconstruction.

### Derive `element` only from the root

Rejected. Explicit element values make filtering and reporting clearer. Database triggers prevent the repeated values from diverging inside a tree.

## Consequences

- Creating a child and listing the tree are straightforward.
- Moving a subtree changes only its root account's `parent_id`.
- Descendants inherit accounting-element validity through enforced equality, not implicit UI behavior.
- A posting account is structurally guaranteed to be a leaf.
- Recursive queries or Rust tree reconstruction are required for ancestry and nested presentation.
- Sibling ordering remains creation order until the user decides whether explicit ordering is needed.

## Explicit non-decisions

This ADR does not decide:

- The final default category labels or exact handwritten hierarchy.
- Which nodes are locked or system-managed.
- Whether sibling names must be unique.
- Account codes, multi-currency, contra-account behavior, or normal-balance overrides.
- Whether an inactive parent may contain active descendants.
- How an account with posted history could ever become a category.

