PRAGMA application_id = 1799434289; -- ASCII "kA01"

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    parent_id INTEGER REFERENCES accounts(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (
        name = trim(name)
        AND length(name) BETWEEN 1 AND 120
        AND instr(name, char(0)) = 0
        AND instr(name, char(9)) = 0
        AND instr(name, char(10)) = 0
        AND instr(name, char(13)) = 0
        -- Remaining C0 and C1 control characters.
        AND name NOT GLOB (
            '*[' || char(1) || '-' || char(31)
            || char(127) || '-' || char(159) || ']*'
        )
        -- Non-control Unicode White_Space code points rejected at either edge.
        AND instr(
            char(
                160, 5760, 8192, 8193, 8194, 8195, 8196, 8197,
                8198, 8199, 8200, 8201, 8202, 8232, 8233, 8239,
                8287, 12288
            ),
            substr(name, 1, 1)
        ) = 0
        AND instr(
            char(
                160, 5760, 8192, 8193, 8194, 8195, 8196, 8197,
                8198, 8199, 8200, 8201, 8202, 8232, 8233, 8239,
                8287, 12288
            ),
            substr(name, -1, 1)
        ) = 0
    ),
    element TEXT NOT NULL CHECK (
        element IN ('asset', 'liability', 'equity', 'income', 'expense')
    ),
    kind TEXT NOT NULL CHECK (
        kind IN ('category', 'posting')
    ),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (
        is_active IN (0, 1)
    ),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

CREATE INDEX accounts_parent_id_idx ON accounts(parent_id);
CREATE INDEX accounts_element_kind_active_idx
    ON accounts(element, kind, is_active);

CREATE TRIGGER accounts_require_initial_revision
BEFORE INSERT ON accounts
WHEN NEW.revision <> 1
BEGIN
    SELECT RAISE(ABORT, 'account revision must start at one');
END;

CREATE TRIGGER accounts_require_next_revision
BEFORE UPDATE ON accounts
WHEN NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'account revision must increase by one');
END;

CREATE TRIGGER accounts_preserve_id
BEFORE UPDATE OF id ON accounts
WHEN NEW.id IS NOT OLD.id
BEGIN
    SELECT RAISE(ABORT, 'account id is immutable');
END;

CREATE TRIGGER accounts_preserve_created_at
BEFORE UPDATE OF created_at ON accounts
WHEN NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'account creation timestamp is immutable');
END;

CREATE TRIGGER accounts_validate_parent_insert
BEFORE INSERT ON accounts
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'account parent not found')
    WHERE NOT EXISTS (
        SELECT 1 FROM accounts AS parent WHERE parent.id = NEW.parent_id
    );

    SELECT RAISE(ABORT, 'account parent must be a category')
    WHERE EXISTS (
        SELECT 1
        FROM accounts AS parent
        WHERE parent.id = NEW.parent_id
          AND parent.kind <> 'category'
    );

    SELECT RAISE(ABORT, 'account parent must use the same element')
    WHERE EXISTS (
        SELECT 1
        FROM accounts AS parent
        WHERE parent.id = NEW.parent_id
          AND parent.element <> NEW.element
    );
END;

CREATE TRIGGER accounts_validate_parent_update
BEFORE UPDATE OF parent_id, element ON accounts
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'account parent not found')
    WHERE NOT EXISTS (
        SELECT 1 FROM accounts AS parent WHERE parent.id = NEW.parent_id
    );

    SELECT RAISE(ABORT, 'account parent must be a category')
    WHERE EXISTS (
        SELECT 1
        FROM accounts AS parent
        WHERE parent.id = NEW.parent_id
          AND parent.kind <> 'category'
    );

    SELECT RAISE(ABORT, 'account parent must use the same element')
    WHERE EXISTS (
        SELECT 1
        FROM accounts AS parent
        WHERE parent.id = NEW.parent_id
          AND parent.element <> NEW.element
    );
END;

CREATE TRIGGER accounts_prevent_cycle_update
BEFORE UPDATE OF parent_id ON accounts
WHEN NEW.parent_id IS NOT NULL
BEGIN
    WITH RECURSIVE subtree(id) AS (
        SELECT OLD.id
        UNION ALL
        SELECT child.id
        FROM accounts AS child
        JOIN subtree ON child.parent_id = subtree.id
    )
    SELECT RAISE(ABORT, 'account hierarchy cycle')
    FROM subtree
    WHERE id = NEW.parent_id
    LIMIT 1;
END;

CREATE TRIGGER accounts_preserve_child_elements
BEFORE UPDATE OF element ON accounts
WHEN NEW.element <> OLD.element
BEGIN
    SELECT RAISE(ABORT, 'account element must match its children')
    WHERE EXISTS (
        SELECT 1
        FROM accounts AS child
        WHERE child.parent_id = OLD.id
          AND child.element <> NEW.element
    );
END;

CREATE TRIGGER accounts_posting_kind_requires_leaf
BEFORE UPDATE OF kind ON accounts
WHEN NEW.kind = 'posting' AND OLD.kind <> 'posting'
BEGIN
    SELECT RAISE(ABORT, 'posting account cannot have children')
    WHERE EXISTS (
        SELECT 1 FROM accounts AS child WHERE child.parent_id = OLD.id
    );
END;

