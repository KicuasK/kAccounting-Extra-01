CREATE TABLE attachments (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'
    ),
    entry_id INTEGER NOT NULL REFERENCES journal_entries(id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    display_name TEXT NOT NULL CHECK (
        length(display_name) BETWEEN 1 AND 255
        AND display_name NOT IN ('.', '..')
        AND instr(display_name, '/') = 0
        AND instr(display_name, char(92)) = 0
        AND instr(display_name, char(0)) = 0
        AND display_name NOT GLOB ('*[' || char(1) || '-' || char(31)
            || char(127) || '-' || char(159) || ']*')
    ),
    plaintext_sha256 TEXT NOT NULL CHECK (
        length(plaintext_sha256) = 64 AND plaintext_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    byte_length INTEGER NOT NULL CHECK (byte_length BETWEEN 0 AND 262144000),
    ciphertext_sha256 TEXT NOT NULL CHECK (
        length(ciphertext_sha256) = 64 AND ciphertext_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    ciphertext_byte_length INTEGER NOT NULL CHECK (
        ciphertext_byte_length = byte_length + 72 + 20 * max(1, (byte_length + 65535) / 65536)
    ),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX attachments_entry_id_idx ON attachments(entry_id, created_at, id);

CREATE TRIGGER attachments_enforce_book_quota
BEFORE INSERT ON attachments
BEGIN
    SELECT RAISE(ABORT, 'attachment book quota exceeded')
    WHERE NEW.byte_length > 21474836480 - COALESCE((SELECT SUM(byte_length) FROM attachments), 0);
END;

CREATE TRIGGER attachments_immutable_update
BEFORE UPDATE ON attachments
BEGIN
    SELECT RAISE(ABORT, 'attachment metadata is immutable');
END;

CREATE TRIGGER attachments_posted_immutable_delete
BEFORE DELETE ON attachments
WHEN EXISTS (SELECT 1 FROM journal_entries WHERE id = OLD.entry_id AND status = 'posted')
BEGIN
    SELECT RAISE(ABORT, 'posted attachment cannot be deleted');
END;

CREATE TRIGGER attachments_validate_owner_insert
BEFORE INSERT ON attachments
BEGIN
    SELECT RAISE(ABORT, 'attachment owner must be a valid journal entry')
    WHERE NOT EXISTS (
        SELECT 1 FROM journal_entries AS owner
        WHERE owner.id = NEW.entry_id
          AND (
              (owner.status = 'draft' AND owner.revision >= 1 AND owner.posted_at IS NULL)
              OR (owner.status = 'posted' AND owner.revision >= 2 AND owner.posted_at IS NOT NULL)
          )
    );
END;
