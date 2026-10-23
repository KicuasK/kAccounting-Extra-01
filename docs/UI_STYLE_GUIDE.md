# UI Style Guide

Status: implementation guide with later user-approved visual revisions; see
[latest UI design handoff](UI_DESIGN_HANDOFF_2026-10-22.md).

## Visual direction

- Latest user direction: macOS-inspired 02 clear-modern composition, selected in the separate UI conversation; Catalina is no longer the exclusive visual reference.
- Favor native-looking toolbars, sidebars, compact tables, subtle borders, and restrained decoration.
- Use black/white/neutral-gray chrome, white content, fine separators and neutral selection. Only accounting-element identification uses color, with text labels. Later requests supersede the older blue-selection direction.
- Avoid oversized cards, excessive whitespace, and dashboard decoration that reduces accounting density.

## Typography

- Default UI font: `system-ui`.
- Only Chinese characters inside account names use `DFKai-SB`, with a safe serif-style fallback if unavailable.
- English letters, numbers, punctuation, dates, and monetary amounts remain in `system-ui`, including within mixed-language account names.
- Amount columns use tabular numerals and right alignment.
- Font fallback and mixed-script rendering must be verified in Mac Safari for
  Version 01 and in iPad Safari when the Version 02 interface is implemented.

## General Journal entry direction

The user supplied a traditional General Journal reference and prefers its entry structure. The design direction is:

- Group entries by date.
- Reference reaffirmed 2026-10-23: omit the PR column; credit accounts are indented and the narrative follows account lines. The approved optional entry reference remains separate.
- Place account titles and the narrative in the main column.
- Keep debit and credit in fixed, separate, right-aligned columns.
- Show debit accounts flush left and indent credit accounts.
- Put the explanation below the account lines.
- Allow multiple debit and credit lines in one entry.
- Separate entries with fine horizontal rules.
- Display debit and credit totals and prevent posting while unequal.

Illustrative structure:

```text
日期       科目與說明                         借方          貸方
10/21     銀行－活期存款                    20,000
             薪資收入                                     20,000
          （十月份薪資入帳）
```

The Version 01 editing interaction was approved on 2026-10-22. Its header
contains accounting date, optional editable reference, and optional
whole-entry explanation. Each line contains account, separate debit and credit
amount columns, and an optional line memo. Drafts use an explicit Save action;
Version 01 does not add timed autosave or implicit crash recovery. Account
lookup supports keyboard search by account name and ancestor path. Validation,
line insertion, posting, reversal, and correction must preserve the structure
and accounting rules above.

## Responsive behavior

- Mac: persistent sidebar where space permits, keyboard-efficient entry, and full table layout.
- iPad Version 02: touch-sized controls, collapsible sidebar, and horizontally
  scrollable tables only where necessary.
- iPhone: intentionally unsupported in the first version.

## Accessibility baseline

- Visible keyboard focus.
- Semantic table headings and form labels.
- Selection must not rely on color alone.
- Sufficient contrast for text, rules, and disabled states.
- Validation messages must identify the affected field or journal line.

