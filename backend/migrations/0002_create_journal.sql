CREATE TABLE journal_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    entry_date TEXT NOT NULL CHECK (
        length(entry_date) = 10
        AND entry_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
        AND CAST(substr(entry_date, 1, 4) AS INTEGER) BETWEEN 1 AND 9999
        AND CAST(substr(entry_date, 6, 2) AS INTEGER) BETWEEN 1 AND 12
        AND CAST(substr(entry_date, 9, 2) AS INTEGER) BETWEEN 1 AND CASE
            WHEN CAST(substr(entry_date, 6, 2) AS INTEGER) IN (1, 3, 5, 7, 8, 10, 12) THEN 31
            WHEN CAST(substr(entry_date, 6, 2) AS INTEGER) IN (4, 6, 9, 11) THEN 30
            WHEN (
                CAST(substr(entry_date, 1, 4) AS INTEGER) % 400 = 0
                OR (
                    CAST(substr(entry_date, 1, 4) AS INTEGER) % 4 = 0
                    AND CAST(substr(entry_date, 1, 4) AS INTEGER) % 100 <> 0
                )
            ) THEN 29
            ELSE 28
        END
    ),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'posted')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    posted_at TEXT,
    CHECK (
        (status = 'draft' AND posted_at IS NULL)
        OR (status = 'posted' AND posted_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE journal_lines (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    entry_id INTEGER NOT NULL
        REFERENCES journal_entries(id) ON UPDATE RESTRICT ON DELETE CASCADE,
    line_no INTEGER NOT NULL CHECK (line_no > 0),
    account_id INTEGER NOT NULL
        REFERENCES accounts(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    side TEXT NOT NULL CHECK (side IN ('debit', 'credit')),
    amount_minor INTEGER NOT NULL CHECK (amount_minor > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (entry_id, line_no)
) STRICT;

CREATE INDEX journal_entries_status_date_idx
    ON journal_entries(status, entry_date, id);
CREATE INDEX journal_lines_entry_id_idx ON journal_lines(entry_id, line_no);
CREATE INDEX journal_lines_account_id_idx ON journal_lines(account_id, entry_id);

CREATE TRIGGER accounts_preserve_journal_classification
BEFORE UPDATE OF element, kind ON accounts
WHEN (NEW.element IS NOT OLD.element OR NEW.kind IS NOT OLD.kind)
  AND EXISTS (SELECT 1 FROM journal_lines WHERE account_id = OLD.id)
BEGIN
    SELECT RAISE(ABORT, 'account journal classification is immutable');
END;

CREATE TRIGGER journal_entries_insert_as_draft
BEFORE INSERT ON journal_entries
WHEN NEW.status <> 'draft' OR NEW.posted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'journal entry must be inserted as a draft');
END;

CREATE TRIGGER journal_entries_preserve_identity
BEFORE UPDATE ON journal_entries
WHEN NEW.id IS NOT OLD.id OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'journal entry identity is immutable');
END;

CREATE TRIGGER journal_entries_posted_immutable_update
BEFORE UPDATE ON journal_entries
WHEN OLD.status = 'posted'
BEGIN
    SELECT RAISE(ABORT, 'posted journal entry is immutable');
END;

CREATE TRIGGER journal_entries_posted_immutable_delete
BEFORE DELETE ON journal_entries
WHEN OLD.status = 'posted'
BEGIN
    SELECT RAISE(ABORT, 'posted journal entry cannot be deleted');
END;

CREATE TRIGGER journal_entries_validate_posting
BEFORE UPDATE OF status ON journal_entries
WHEN OLD.status = 'draft' AND NEW.status = 'posted'
BEGIN
    SELECT RAISE(ABORT, 'posted journal entry requires at least two lines')
    WHERE (SELECT COUNT(*) FROM journal_lines WHERE entry_id = OLD.id) < 2;

    SELECT RAISE(ABORT, 'journal line account must be active and posting')
    WHERE EXISTS (
        SELECT 1
        FROM journal_lines AS line
        LEFT JOIN accounts AS account ON account.id = line.account_id
        WHERE line.entry_id = OLD.id
          AND (
              account.id IS NULL
              OR account.kind <> 'posting'
              OR account.is_active <> 1
              OR EXISTS (
                  SELECT 1 FROM accounts AS child WHERE child.parent_id = account.id
              )
          )
    );

    SELECT RAISE(ABORT, 'journal entry is not balanced')
    WHERE (
        SELECT SUM(CASE WHEN side = 'debit' THEN amount_minor ELSE 0 END)
        FROM journal_lines
        WHERE entry_id = OLD.id
    ) <> (
        SELECT SUM(CASE WHEN side = 'credit' THEN amount_minor ELSE 0 END)
        FROM journal_lines
        WHERE entry_id = OLD.id
    );
END;

CREATE TRIGGER journal_lines_insert_only_into_draft
BEFORE INSERT ON journal_lines
BEGIN
    SELECT RAISE(ABORT, 'journal line entry must be a draft')
    WHERE NOT EXISTS (
        SELECT 1
        FROM journal_entries AS entry
        WHERE entry.id = NEW.entry_id AND entry.status = 'draft'
    );
END;

CREATE TRIGGER journal_lines_validate_account_insert
BEFORE INSERT ON journal_lines
BEGIN
    SELECT RAISE(ABORT, 'journal line account must be active and posting')
    WHERE NOT EXISTS (
        SELECT 1
        FROM accounts AS account
        WHERE account.id = NEW.account_id
          AND account.kind = 'posting'
          AND account.is_active = 1
          AND NOT EXISTS (
              SELECT 1 FROM accounts AS child WHERE child.parent_id = account.id
          )
    );
END;

CREATE TRIGGER journal_lines_update_only_in_draft
BEFORE UPDATE ON journal_lines
BEGIN
    SELECT RAISE(ABORT, 'posted journal line is immutable')
    WHERE EXISTS (
        SELECT 1
        FROM journal_entries AS entry
        WHERE entry.id = OLD.entry_id AND entry.status = 'posted'
    );
END;

CREATE TRIGGER journal_lines_preserve_identity
BEFORE UPDATE ON journal_lines
WHEN NEW.id IS NOT OLD.id
  OR NEW.entry_id IS NOT OLD.entry_id
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'journal line identity is immutable');
END;

CREATE TRIGGER journal_lines_validate_account_update
BEFORE UPDATE OF account_id ON journal_lines
BEGIN
    SELECT RAISE(ABORT, 'journal line account must be active and posting')
    WHERE NOT EXISTS (
        SELECT 1
        FROM accounts AS account
        WHERE account.id = NEW.account_id
          AND account.kind = 'posting'
          AND account.is_active = 1
          AND NOT EXISTS (
              SELECT 1 FROM accounts AS child WHERE child.parent_id = account.id
          )
    );
END;

CREATE TRIGGER journal_lines_delete_only_from_draft
BEFORE DELETE ON journal_lines
BEGIN
    SELECT RAISE(ABORT, 'posted journal line cannot be deleted')
    WHERE EXISTS (
        SELECT 1
        FROM journal_entries AS entry
        WHERE entry.id = OLD.entry_id AND entry.status = 'posted'
    );
END;
