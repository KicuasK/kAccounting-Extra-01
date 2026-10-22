# kAccounting-Extra-01 Agent Rules

## Scope

- Preserve the exact project name `kAccounting-Extra-01`.
- Build a private, single-user double-entry accounting MVP for macOS, Mac Safari, and iPad Safari.
- Treat IFRS concepts as design guidance; do not claim full IFRS reporting compliance.
- Keep source code separate from runtime financial data.

## Accounting invariants

- Store money as exact integer minor units or an exact decimal type; never use binary floating point.
- A posted journal entry must contain at least two valid lines and have equal debit and credit totals.
- Posting must be atomic and enforced by the backend.
- Posted entries must not be silently edited or deleted; corrections use reversal or an explicit correcting entry.
- Reports derive from posted journal lines, never from directly editable account balances.
- Opening balances must be represented by a balanced opening entry.
- Only active posting accounts may receive journal lines. Category nodes are not posting accounts.
- Prevent account-tree cycles and invalid moves across accounting elements.

## Product and UI constraints

- Use a macOS Catalina 10.15 light Aqua visual direction: compact toolbars and tables, subtle separators, white content, and blue selection states.
- Avoid large modern SaaS-style cards as the primary information layout.
- Use `system-ui` for all UI text except Chinese characters inside account names, which use `DFKai-SB` with a safe fallback.
- Keep English, digits, dates, and amounts in account names in the system font.
- Right-align amounts and use tabular numerals.
- Support Mac Safari and touch-friendly iPad Safari. iPhone support is out of scope.
- Use the traditional General Journal reference recorded in `docs/UI_STYLE_GUIDE.md` for entry structure.

## Boundaries

- Do not expose a public service or enable Tailscale Funnel.
- Do not modify real financial data during development or tests.
- Do not finalize ambiguous accounting policies without recording a user decision.
- Do not implement the journal-entry editor until its detailed interaction design is approved. The supplied General Journal image establishes direction, not a complete interaction specification.

## Engineering practice

- Add tests for every accounting invariant before or with its implementation.
- Use SQLite transactions for multi-step writes.
- Prefer small, reviewable changes and report changed files, tests run, unresolved questions, and Git status.
- Never commit secrets, production databases, or backups.

