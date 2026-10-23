CREATE TABLE payment_memos (
 id INTEGER PRIMARY KEY CHECK(id > 0),
 revision INTEGER NOT NULL CHECK(revision >= 1),
 due_date TEXT NOT NULL CHECK(length(due_date)=10 AND due_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
 title TEXT NOT NULL CHECK(length(trim(title)) BETWEEN 1 AND 200),
 payee TEXT NOT NULL CHECK(length(payee)<=200),
 note TEXT NOT NULL CHECK(length(note)<=2000),
 amount_minor INTEGER NOT NULL CHECK(amount_minor>0),
 necessity TEXT NOT NULL CHECK(necessity IN ('required','optional')),
 status TEXT NOT NULL CHECK(status IN ('planned','cancelled')),
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX payment_memos_due_idx ON payment_memos(due_date,id);
CREATE TABLE payment_memo_history (
 memo_id INTEGER NOT NULL REFERENCES payment_memos(id) ON DELETE RESTRICT,
 revision INTEGER NOT NULL,
 due_date TEXT NOT NULL,
 title TEXT NOT NULL,
 payee TEXT NOT NULL,
 note TEXT NOT NULL,
 amount_minor INTEGER NOT NULL,
 necessity TEXT NOT NULL,
 status TEXT NOT NULL,
 recorded_at TEXT NOT NULL,
 PRIMARY KEY(memo_id,revision)
) STRICT;
CREATE TRIGGER payment_memos_initial BEFORE INSERT ON payment_memos
WHEN NEW.revision<>1 OR NEW.status<>'planned'
BEGIN SELECT RAISE(ABORT,'invalid initial payment memo'); END;
CREATE TRIGGER payment_memos_revision BEFORE UPDATE ON payment_memos
WHEN NEW.id<>OLD.id OR NEW.created_at<>OLD.created_at OR NEW.revision<>OLD.revision+1
BEGIN SELECT RAISE(ABORT,'invalid payment memo revision'); END;
CREATE TRIGGER payment_memos_no_delete BEFORE DELETE ON payment_memos
BEGIN SELECT RAISE(ABORT,'payment memo history must be retained'); END;
CREATE TRIGGER payment_memos_audit_insert AFTER INSERT ON payment_memos
BEGIN INSERT INTO payment_memo_history VALUES(NEW.id,NEW.revision,NEW.due_date,NEW.title,NEW.payee,NEW.note,NEW.amount_minor,NEW.necessity,NEW.status,NEW.updated_at); END;
CREATE TRIGGER payment_memos_audit_update AFTER UPDATE ON payment_memos
BEGIN INSERT INTO payment_memo_history VALUES(NEW.id,NEW.revision,NEW.due_date,NEW.title,NEW.payee,NEW.note,NEW.amount_minor,NEW.necessity,NEW.status,NEW.updated_at); END;
CREATE TRIGGER payment_memo_history_no_update BEFORE UPDATE ON payment_memo_history
BEGIN SELECT RAISE(ABORT,'payment memo audit is immutable'); END;
CREATE TRIGGER payment_memo_history_no_delete BEFORE DELETE ON payment_memo_history
BEGIN SELECT RAISE(ABORT,'payment memo audit is immutable'); END;
