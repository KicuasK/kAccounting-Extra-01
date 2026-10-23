CREATE TABLE open_item_batches (
    id INTEGER PRIMARY KEY CHECK(id > 0),
    source_line_id INTEGER NOT NULL UNIQUE REFERENCES journal_lines(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK(kind IN ('receivable', 'payable')),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE TABLE open_items (
    id INTEGER PRIMARY KEY CHECK(id > 0),
    batch_id INTEGER NOT NULL REFERENCES open_item_batches(id) ON DELETE RESTRICT,
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    description TEXT NOT NULL CHECK(length(trim(description)) BETWEEN 1 AND 1000 AND instr(description,char(0))=0),
    counterparty TEXT CHECK(counterparty IS NULL OR (length(trim(counterparty)) BETWEEN 1 AND 200 AND instr(counterparty,char(0))=0)),
    due_date TEXT,
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE TABLE open_item_allocations (
    id INTEGER PRIMARY KEY CHECK(id > 0),
    settlement_line_id INTEGER NOT NULL REFERENCES journal_lines(id) ON DELETE RESTRICT,
    item_id INTEGER NOT NULL REFERENCES open_items(id) ON DELETE RESTRICT,
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    kind TEXT NOT NULL CHECK(kind IN ('payment','adjustment')),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE TABLE open_item_allocation_voids (
    allocation_id INTEGER PRIMARY KEY REFERENCES open_item_allocations(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK(length(trim(reason)) BETWEEN 1 AND 1000 AND instr(reason,char(0))=0),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE TABLE open_item_events (
    id INTEGER PRIMARY KEY CHECK(id > 0),
    kind TEXT NOT NULL CHECK(kind IN ('registered','allocated','allocation_voided')),
    entity_id INTEGER NOT NULL CHECK(entity_id > 0),
    details TEXT NOT NULL CHECK(json_valid(details)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX open_items_batch_idx ON open_items(batch_id,id);
CREATE INDEX open_item_allocations_item_idx ON open_item_allocations(item_id,id);
CREATE INDEX open_item_allocations_line_idx ON open_item_allocations(settlement_line_id,id);
CREATE TRIGGER open_item_batches_no_update BEFORE UPDATE ON open_item_batches BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_batches_no_delete BEFORE DELETE ON open_item_batches BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_items_no_update BEFORE UPDATE ON open_items BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_items_no_delete BEFORE DELETE ON open_items BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_allocations_no_update BEFORE UPDATE ON open_item_allocations BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_allocations_no_delete BEFORE DELETE ON open_item_allocations BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_allocation_voids_no_update BEFORE UPDATE ON open_item_allocation_voids BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_allocation_voids_no_delete BEFORE DELETE ON open_item_allocation_voids BEGIN SELECT RAISE(ABORT,'open item history is immutable'); END;
CREATE TRIGGER open_item_events_no_update BEFORE UPDATE ON open_item_events BEGIN SELECT RAISE(ABORT,'open item audit is immutable'); END;
CREATE TRIGGER open_item_events_no_delete BEFORE DELETE ON open_item_events BEGIN SELECT RAISE(ABORT,'open item audit is immutable'); END;
CREATE TRIGGER open_item_reversal_requires_unallocation
BEFORE UPDATE OF status ON journal_entries
WHEN OLD.status='draft' AND NEW.status='posted' AND OLD.reverses_entry_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT,'open item allocations must be explicitly voided before reversal')
    WHERE EXISTS (
        SELECT 1 FROM open_item_allocations x
        JOIN journal_lines payment ON payment.id=x.settlement_line_id
        JOIN open_items item ON item.id=x.item_id
        JOIN open_item_batches batch ON batch.id=item.batch_id
        JOIN journal_lines source ON source.id=batch.source_line_id
        WHERE NOT EXISTS(SELECT 1 FROM open_item_allocation_voids v WHERE v.allocation_id=x.id)
          AND (source.entry_id=OLD.reverses_entry_id OR payment.entry_id=OLD.reverses_entry_id)
    );
END;
