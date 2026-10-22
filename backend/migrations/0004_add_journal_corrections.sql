ALTER TABLE journal_entries
ADD COLUMN reverses_entry_id INTEGER
    REFERENCES journal_entries(id) ON UPDATE RESTRICT ON DELETE RESTRICT;

ALTER TABLE journal_entries
ADD COLUMN corrects_entry_id INTEGER
    REFERENCES journal_entries(id) ON UPDATE RESTRICT ON DELETE RESTRICT;

ALTER TABLE journal_entries
ADD COLUMN explanation TEXT;

DROP TRIGGER journal_entries_require_next_draft_revision;

CREATE TRIGGER journal_entries_require_next_draft_revision
BEFORE UPDATE OF entry_date, explanation, revision ON journal_entries
WHEN OLD.status = 'draft'
  AND NEW.status = 'draft'
  AND NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'draft journal entry revision must increase by one');
END;

CREATE UNIQUE INDEX journal_entries_one_reversal_idx
    ON journal_entries(reverses_entry_id)
    WHERE reverses_entry_id IS NOT NULL;

CREATE INDEX journal_entries_correction_target_idx
    ON journal_entries(corrects_entry_id, id)
    WHERE corrects_entry_id IS NOT NULL;

CREATE TRIGGER journal_entries_preserve_relationship
BEFORE UPDATE OF reverses_entry_id, corrects_entry_id, explanation ON journal_entries
BEGIN
    SELECT RAISE(ABORT, 'journal entry relationship is immutable')
    WHERE NEW.reverses_entry_id IS NOT OLD.reverses_entry_id
       OR NEW.corrects_entry_id IS NOT OLD.corrects_entry_id
       OR (
           OLD.corrects_entry_id IS NOT NULL
           AND NEW.explanation IS NOT OLD.explanation
       );

    SELECT RAISE(ABORT, 'journal entry explanation contains invalid text')
    WHERE NEW.explanation IS NOT NULL
      AND instr(NEW.explanation, char(0)) <> 0;
END;

CREATE TRIGGER journal_entries_validate_relationship_insert
BEFORE INSERT ON journal_entries
BEGIN
    SELECT RAISE(ABORT, 'journal entry cannot be both a reversal and a correction')
    WHERE NEW.reverses_entry_id IS NOT NULL
      AND NEW.corrects_entry_id IS NOT NULL;

    SELECT RAISE(ABORT, 'journal entry explanation contains invalid text')
    WHERE NEW.explanation IS NOT NULL
      AND instr(NEW.explanation, char(0)) <> 0;

    SELECT RAISE(ABORT, 'correcting journal entry requires an explanation')
    WHERE NEW.corrects_entry_id IS NOT NULL
      AND (NEW.explanation IS NULL OR trim(NEW.explanation) = '');
END;

CREATE TRIGGER journal_entries_validate_relationship_posting
BEFORE UPDATE OF status ON journal_entries
WHEN OLD.status = 'draft' AND NEW.status = 'posted'
BEGIN
    SELECT RAISE(ABORT, 'journal entry cannot be both a reversal and a correction')
    WHERE OLD.reverses_entry_id IS NOT NULL
      AND OLD.corrects_entry_id IS NOT NULL;

    SELECT RAISE(ABORT, 'related journal entry must exist and be posted')
    WHERE (OLD.reverses_entry_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM journal_entries AS original
        WHERE original.id = OLD.reverses_entry_id
          AND original.status = 'posted'
    )) OR (OLD.corrects_entry_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM journal_entries AS original
        WHERE original.id = OLD.corrects_entry_id
          AND original.status = 'posted'
    ));

    SELECT RAISE(ABORT, 'a reversal cannot reverse another reversal')
    WHERE OLD.reverses_entry_id IS NOT NULL
      AND EXISTS (
          SELECT 1 FROM journal_entries AS original
          WHERE original.id = OLD.reverses_entry_id
            AND original.reverses_entry_id IS NOT NULL
      );

    SELECT RAISE(ABORT, 'reversal date cannot precede original entry date')
    WHERE OLD.reverses_entry_id IS NOT NULL
      AND EXISTS (
          SELECT 1 FROM journal_entries AS original
          WHERE original.id = OLD.reverses_entry_id
            AND OLD.entry_date < original.entry_date
      );

    SELECT RAISE(ABORT, 'reversal lines must exactly exchange debit and credit sides')
    WHERE OLD.reverses_entry_id IS NOT NULL
      AND (
          (SELECT COUNT(*) FROM journal_lines WHERE entry_id = OLD.id)
              <> (SELECT COUNT(*) FROM journal_lines WHERE entry_id = OLD.reverses_entry_id)
          OR EXISTS (
              SELECT 1
              FROM journal_lines AS reversal_line
              LEFT JOIN journal_lines AS original_line
                ON original_line.entry_id = OLD.reverses_entry_id
               AND original_line.line_no = reversal_line.line_no
              WHERE reversal_line.entry_id = OLD.id
                AND (
                    original_line.id IS NULL
                    OR original_line.account_id <> reversal_line.account_id
                    OR original_line.amount_minor <> reversal_line.amount_minor
                    OR original_line.side <> CASE reversal_line.side
                        WHEN 'debit' THEN 'credit'
                        ELSE 'debit'
                    END
                )
          )
      );

    SELECT RAISE(ABORT, 'correcting journal entry requires an explanation')
    WHERE OLD.corrects_entry_id IS NOT NULL
      AND (OLD.explanation IS NULL OR trim(OLD.explanation) = '');
END;
