-- Binary comparison preserves the approved case and Unicode semantics.
CREATE UNIQUE INDEX accounts_sibling_name_idx ON accounts(parent_id, name)
    WHERE parent_id IS NOT NULL;
CREATE UNIQUE INDEX accounts_root_name_idx ON accounts(name)
    WHERE parent_id IS NULL;

CREATE TRIGGER accounts_protect_active_descendants
BEFORE UPDATE OF is_active ON accounts
WHEN NEW.is_active = 0
BEGIN
    WITH RECURSIVE descendants(id, is_active) AS (
        SELECT id, is_active FROM accounts WHERE parent_id = OLD.id
        UNION
        SELECT child.id, child.is_active FROM accounts AS child
        JOIN descendants ON child.parent_id = descendants.id
    )
    SELECT RAISE(ABORT, 'category has active descendants')
    FROM descendants WHERE is_active = 1 LIMIT 1;
END;

-- Keep the same invariant when creating, activating, or moving a subtree.
CREATE TRIGGER accounts_validate_active_parent_insert
BEFORE INSERT ON accounts
WHEN NEW.is_active = 1 AND NEW.parent_id IS NOT NULL
BEGIN
    WITH RECURSIVE ancestors(id, parent_id, is_active) AS (
        SELECT id, parent_id, is_active FROM accounts WHERE id = NEW.parent_id
        UNION
        SELECT parent.id, parent.parent_id, parent.is_active FROM accounts AS parent
        JOIN ancestors ON parent.id = ancestors.parent_id
    )
    SELECT RAISE(ABORT, 'active account requires active ancestors')
    FROM ancestors WHERE is_active = 0 LIMIT 1;
END;

CREATE TRIGGER accounts_validate_active_parent_update
BEFORE UPDATE OF parent_id, is_active ON accounts
WHEN NEW.parent_id IS NOT NULL
BEGIN
    WITH RECURSIVE subtree(id, is_active) AS (
        SELECT OLD.id, NEW.is_active
        UNION
        SELECT child.id, child.is_active FROM accounts AS child
        JOIN subtree ON child.parent_id = subtree.id
    ), ancestors(id, parent_id, is_active) AS (
        SELECT id, parent_id, is_active FROM accounts WHERE id = NEW.parent_id
        UNION
        SELECT parent.id, parent.parent_id, parent.is_active FROM accounts AS parent
        JOIN ancestors ON parent.id = ancestors.parent_id
    )
    SELECT RAISE(ABORT, 'active account requires active ancestors')
    WHERE EXISTS (SELECT 1 FROM subtree WHERE is_active = 1)
      AND EXISTS (SELECT 1 FROM ancestors WHERE is_active = 0);
END;
