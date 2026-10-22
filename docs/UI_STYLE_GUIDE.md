# UI Style Guide

Status: draft

## Visual direction

- Reference macOS Catalina 10.15 light Aqua, not Mojave.
- Favor native-looking toolbars, sidebars, compact tables, subtle borders, and restrained decoration.
- Use a gray toolbar, white primary content, fine separators, and blue selection states.
- Avoid oversized cards, excessive whitespace, and dashboard decoration that reduces accounting density.

## Typography

- Default UI font: `system-ui`.
- Only Chinese characters inside account names use `DFKai-SB`, with a safe serif-style fallback if unavailable.
- English letters, numbers, punctuation, dates, and monetary amounts remain in `system-ui`, including within mixed-language account names.
- Amount columns use tabular numerals and right alignment.
- Font fallback and mixed-script rendering must be verified in Mac Safari and iPad Safari.

## General Journal entry direction

The user supplied a traditional General Journal reference and prefers its entry structure. The design direction is:

- Group entries by date.
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

This reference defines information hierarchy and visual character, not the complete editing interaction. Field navigation, account search, validation messages, drafts, line insertion, touch behavior, and correction flows still require a dedicated design review before implementation.

## Responsive behavior

- Mac: persistent sidebar where space permits, keyboard-efficient entry, and full table layout.
- iPad: touch-sized controls, collapsible sidebar, and horizontally scrollable tables only where necessary.
- iPhone: intentionally unsupported in the first version.

## Accessibility baseline

- Visible keyboard focus.
- Semantic table headings and form labels.
- Selection must not rely on color alone.
- Sufficient contrast for text, rules, and disabled states.
- Validation messages must identify the affected field or journal line.

