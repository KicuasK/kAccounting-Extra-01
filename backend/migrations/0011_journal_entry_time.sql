-- Optional local civil time; accounting filters continue to use entry_date.
ALTER TABLE journal_entries ADD COLUMN entry_time TEXT CHECK (
    entry_time IS NULL OR (
        length(entry_time) = 8
        AND length(CAST(entry_time AS BLOB)) = 8
        AND entry_time GLOB '[0-2][0-9]:[0-5][0-9]:[0-5][0-9]'
        AND substr(entry_time, 1, 2) <= '23'
    )
);
DROP TRIGGER journal_entries_require_next_draft_revision;
CREATE TRIGGER journal_entries_require_next_draft_revision
BEFORE UPDATE OF entry_date, entry_time, reference, explanation, revision ON journal_entries
WHEN OLD.status = 'draft'
  AND NEW.status = 'draft'
  AND NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'draft journal entry revision must increase by one');
END;
