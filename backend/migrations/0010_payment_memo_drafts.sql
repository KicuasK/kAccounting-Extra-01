CREATE TABLE payment_memo_drafts (
 entry_id INTEGER PRIMARY KEY CHECK(entry_id>0),
 memo_id INTEGER NOT NULL REFERENCES payment_memos(id) ON DELETE RESTRICT,
 memo_revision INTEGER NOT NULL CHECK(memo_revision>0),
 amount_minor INTEGER NOT NULL CHECK(amount_minor>0),
 debit_account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
 payment_asset_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 CHECK(debit_account_id<>payment_asset_id)
) STRICT;
CREATE INDEX payment_memo_drafts_memo_idx ON payment_memo_drafts(memo_id,entry_id);
CREATE TRIGGER payment_memo_drafts_no_update BEFORE UPDATE ON payment_memo_drafts
BEGIN SELECT RAISE(ABORT,'payment memo draft link is immutable'); END;
CREATE TRIGGER payment_memo_drafts_no_delete BEFORE DELETE ON payment_memo_drafts
BEGIN SELECT RAISE(ABORT,'payment memo draft link history is retained'); END;
CREATE TRIGGER payment_memo_drafts_validate_insert BEFORE INSERT ON payment_memo_drafts
BEGIN
 SELECT RAISE(ABORT,'invalid payment memo draft link') WHERE NOT EXISTS(
 SELECT 1 FROM payment_memos m JOIN journal_entries e ON e.id=NEW.entry_id
 WHERE m.id=NEW.memo_id AND m.revision=NEW.memo_revision AND m.status='planned' AND e.status='draft'
 AND NEW.amount_minor<=m.amount_minor);
 SELECT RAISE(ABORT,'payment memo already has an editable draft') WHERE EXISTS(
 SELECT 1 FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id
 WHERE l.memo_id=NEW.memo_id AND e.status='draft');
END;
CREATE TRIGGER payment_memos_protect_linked BEFORE UPDATE ON payment_memos
WHEN EXISTS(SELECT 1 FROM payment_memo_drafts l JOIN journal_entries e ON e.id=l.entry_id
WHERE l.memo_id=OLD.id AND (e.status='draft' OR (e.status='posted' AND NOT EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted'))))
BEGIN SELECT RAISE(ABORT,'payment memo has a draft or effective payment'); END;
CREATE TRIGGER journal_entries_payment_memo_posting BEFORE UPDATE OF status ON journal_entries
WHEN OLD.status='draft' AND NEW.status='posted' AND EXISTS(SELECT 1 FROM payment_memo_drafts WHERE entry_id=OLD.id)
BEGIN
 SELECT RAISE(ABORT,'payment memo draft no longer matches its payment plan') WHERE NOT EXISTS(
 SELECT 1 FROM payment_memo_drafts l JOIN payment_memos m ON m.id=l.memo_id
 WHERE l.entry_id=OLD.id AND m.status='planned' AND m.revision=l.memo_revision
 AND (SELECT COUNT(*) FROM journal_lines WHERE entry_id=OLD.id)=2
 AND EXISTS(SELECT 1 FROM journal_lines WHERE entry_id=OLD.id AND side='debit' AND account_id=l.debit_account_id AND amount_minor=l.amount_minor)
 AND EXISTS(SELECT 1 FROM journal_lines WHERE entry_id=OLD.id AND side='credit' AND account_id=l.payment_asset_id AND amount_minor=l.amount_minor)
 AND EXISTS(SELECT 1 FROM accounts WHERE id=l.payment_asset_id AND element='asset' AND kind='posting' AND is_active=1)
 AND l.amount_minor<=m.amount_minor-COALESCE((SELECT SUM(p.amount_minor) FROM payment_memo_drafts p JOIN journal_entries e ON e.id=p.entry_id WHERE p.memo_id=m.id AND e.status='posted' AND NOT EXISTS(SELECT 1 FROM journal_entries r WHERE r.reverses_entry_id=e.id AND r.status='posted')),0));
END;
