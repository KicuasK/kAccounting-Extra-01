ALTER TABLE journal_entries
ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1);

CREATE TRIGGER journal_entries_require_initial_revision
BEFORE INSERT ON journal_entries
WHEN NEW.revision <> 1
BEGIN
    SELECT RAISE(ABORT, 'journal entry revision must start at one');
END;

CREATE TRIGGER journal_entries_require_next_draft_revision
BEFORE UPDATE OF entry_date, revision ON journal_entries
WHEN OLD.status = 'draft'
  AND NEW.status = 'draft'
  AND NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'draft journal entry revision must increase by one');
END;

CREATE TRIGGER journal_entries_validate_posted_revision
BEFORE UPDATE OF status ON journal_entries
WHEN OLD.status = 'draft'
  AND NEW.status = 'posted'
  AND NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'posted journal entry revision is invalid');
END;
