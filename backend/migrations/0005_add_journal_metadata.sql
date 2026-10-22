ALTER TABLE journal_entries
ADD COLUMN reference TEXT CHECK (reference IS NULL OR instr(reference, char(0)) = 0);

ALTER TABLE journal_lines
ADD COLUMN memo TEXT CHECK (memo IS NULL OR instr(memo, char(0)) = 0);

DROP TRIGGER journal_entries_require_next_draft_revision;
CREATE TRIGGER journal_entries_require_next_draft_revision
BEFORE UPDATE OF entry_date, reference, explanation, revision ON journal_entries
WHEN OLD.status = 'draft'
  AND NEW.status = 'draft'
  AND NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'draft journal entry revision must increase by one');
END;
