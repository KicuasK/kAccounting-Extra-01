use std::time::{SystemTime, UNIX_EPOCH};

use kaccounting_backend::accounting::accounts::{
    AccountError, AccountKind, AccountRepository, AccountingElement, NewAccount,
};
use kaccounting_backend::accounting::journal::{
    JournalDate, JournalLineInput, JournalRepository, JournalSide, PostEntry, SaveDraftEntry,
};
use kaccounting_backend::accounting::money::MinorUnits;
use kaccounting_backend::db::{
    BackupError, DatabaseError, backup_database, open_database, open_memory_database,
    verify_database_integrity, verify_database_snapshot,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

#[tokio::test]
async fn file_database_uses_safe_pragmas_and_runs_migrations() {
    let database_path = unique_database_path("reopen");

    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open and migrate");

    let mut connections = Vec::new();
    for _ in 0..4 {
        connections.push(
            pool.acquire()
                .await
                .expect("all configured pool connections should be available"),
        );
    }
    for connection in &mut connections {
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .expect("foreign key pragma should be readable");
        let busy_timeout_ms: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut **connection)
            .await
            .expect("busy timeout pragma should be readable");
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut **connection)
            .await
            .expect("synchronous pragma should be readable");

        assert_eq!(foreign_keys, 1);
        assert_eq!(busy_timeout_ms, 5_000);
        assert_eq!(synchronous, 2, "SQLite FULL synchronous mode is 2");
    }
    drop(connections);

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .expect("journal mode should be readable");
    let accounts_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'accounts'",
    )
    .fetch_one(&pool)
    .await
    .expect("schema should be readable");
    let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&pool)
        .await
        .expect("application id should be readable");
    let seeded_account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .expect("new account table should be readable");
    let journal_table_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM sqlite_schema
        WHERE type = 'table' AND name IN ('journal_entries', 'journal_lines')
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("journal schema should be readable");
    let seeded_journal_entry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
            .fetch_one(&pool)
            .await
            .expect("new journal table should be readable");

    assert_eq!(journal_mode, "wal");
    assert_eq!(application_id, 1_799_434_289);
    assert_eq!(accounts_table_count, 1);
    assert_eq!(journal_table_count, 2);
    assert_eq!(
        seeded_account_count, 0,
        "unapproved account-policy labels must not be seeded"
    );
    assert_eq!(
        seeded_journal_entry_count, 0,
        "new databases must not contain financial entries"
    );

    #[cfg(unix)]
    for path in [
        database_path.clone(),
        database_path.with_extension("sqlite3-wal"),
        database_path.with_extension("sqlite3-shm"),
    ] {
        let mode = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("{} should exist: {error}", path.display()))
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{} must be owner-only", path.display());
    }

    let repository = AccountRepository::new(pool.clone());
    let account = repository
        .create(NewAccount::new(
            None,
            "測試資產",
            AccountingElement::Asset,
            AccountKind::Category,
        ))
        .await
        .expect("temporary account should be created");

    pool.close().await;

    #[cfg(unix)]
    std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o644))
        .expect("test should loosen the recognized file before reopening");

    let reopened_pool = open_database(&database_path)
        .await
        .expect("existing file database should reopen and migrate idempotently");
    let reopened_repository = AccountRepository::new(reopened_pool.clone());
    let reopened_account = reopened_repository
        .get(account.id)
        .await
        .expect("temporary account should persist after reopening");
    let successful_migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&reopened_pool)
            .await
            .expect("migration history should be readable");

    assert_eq!(reopened_account.name, "測試資產");
    assert_eq!(
        successful_migration_count,
        sqlx::migrate!("./migrations").iter().count() as i64
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&database_path)
            .expect("reopened database metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "recognized databases should be tightened on every open"
    );

    reopened_pool.close().await;

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_resumes_after_identity_claim_before_first_migration() {
    let database_path = unique_database_path("claimed-before-migration");
    let options = SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(true);
    let interrupted_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("test should create an empty SQLite file");
    sqlx::query("PRAGMA application_id = 1799434289")
        .execute(&interrupted_pool)
        .await
        .expect("test should simulate the completed identity claim");
    interrupted_pool.close().await;

    let resumed_pool = open_database(&database_path)
        .await
        .expect("opening should resume migrations for the claimed empty database");
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&resumed_pool)
            .await
            .expect("resumed migration history should be readable");
    assert_eq!(
        migration_count,
        sqlx::migrate!("./migrations").iter().count() as i64
    );
    verify_database_integrity(&resumed_pool)
        .await
        .expect("resumed database should pass the complete startup audit");

    resumed_pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_first_openers_leave_one_reopenable_migrated_database() {
    let database_path = unique_database_path("concurrent-first-open");

    let (first, second) =
        tokio::join!(open_database(&database_path), open_database(&database_path),);
    let mut successful_initializers = Vec::new();
    let mut initialization_errors = Vec::new();
    // A losing first opener may currently report a lock or migration race. The
    // safety contract covered here is that one complete database wins and the
    // loser cannot leave partial schema that prevents a clean audited reopen.
    for result in [first, second] {
        match result {
            Ok(pool) => successful_initializers.push(pool),
            Err(error) => initialization_errors.push(error.to_string()),
        }
    }
    assert!(
        !successful_initializers.is_empty(),
        "at least one initializer must publish a complete database; errors: {initialization_errors:?}"
    );
    assert_eq!(
        successful_initializers.len() + initialization_errors.len(),
        2
    );
    for pool in successful_initializers {
        pool.close().await;
    }

    let reopened_pool = open_database(&database_path)
        .await
        .expect("the database should reopen after competing initializers finish");
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&reopened_pool)
            .await
            .expect("the recovered database should have complete migration history");
    assert_eq!(
        migration_count,
        sqlx::migrate!("./migrations").iter().count() as i64
    );
    reopened_pool.close().await;

    let (first_reopen, second_reopen) =
        tokio::join!(open_database(&database_path), open_database(&database_path),);
    let first_pool = first_reopen.expect("first client should open the initialized database");
    let second_pool = second_reopen.expect("second client should open the initialized database");
    let account = AccountRepository::new(first_pool.clone())
        .create(NewAccount::new(
            None,
            "首次並行開啟",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("one opener should be able to write after migration");
    let reloaded = AccountRepository::new(second_pool.clone())
        .get(account.id)
        .await
        .expect("the other opener should observe the same database");
    assert_eq!(reloaded, account);

    verify_database_integrity(&first_pool)
        .await
        .expect("the recovered database should pass its audit");
    first_pool.close().await;
    second_pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_supports_spaces_and_unicode_in_its_path() {
    let database_path = unique_database_path("會計 資料");

    let pool = open_database(&database_path)
        .await
        .expect("database path with spaces and Unicode should open");
    let repository = AccountRepository::new(pool.clone());
    let account = repository
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("account should persist at the Unicode path");
    pool.close().await;

    let reopened_pool = open_database(&database_path)
        .await
        .expect("Unicode-path database should reopen");
    let reopened = AccountRepository::new(reopened_pool.clone())
        .get(account.id)
        .await
        .expect("stored account should remain readable");
    assert_eq!(reopened, account);

    reopened_pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_clients_cannot_overwrite_the_same_revision() {
    let database_path = unique_database_path("concurrency");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    let repository = AccountRepository::new(pool.clone());
    let cash = repository
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("account should be created");

    let first_repository = repository.clone();
    let second_repository = repository.clone();
    let first_update = first_repository.rename(cash.id, cash.revision, "手邊現金");
    let second_update = second_repository.rename(cash.id, cash.revision, "零用金");
    let (first_result, second_result) = tokio::join!(first_update, second_update);
    let results = [first_result, second_result];

    let success_count = results.iter().filter(|result| result.is_ok()).count();
    let conflict_count = results
        .iter()
        .filter(|result| {
            matches!(
                result,
                Err(AccountError::RevisionConflict {
                    expected: 1,
                    actual: 2
                })
            )
        })
        .count();
    let stored = repository
        .get(cash.id)
        .await
        .expect("winning update should remain readable");

    assert_eq!(success_count, 1);
    assert_eq!(conflict_count, 1);
    assert_eq!(stored.revision, 2);
    assert!(stored.name == "手邊現金" || stored.name == "零用金");

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn concurrent_delete_and_rename_cannot_both_succeed() {
    let database_path = unique_database_path("delete-rename-race");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    let repository = AccountRepository::new(pool.clone());
    let cash = repository
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("account should be created");

    let rename_repository = repository.clone();
    let delete_repository = repository.clone();
    let rename = rename_repository.rename(cash.id, cash.revision, "手邊現金");
    let delete = delete_repository.delete(cash.id, cash.revision);
    let (rename_result, delete_result) = tokio::join!(rename, delete);

    match (rename_result, delete_result) {
        (Ok(renamed), Err(AccountError::RevisionConflict { expected, actual })) => {
            assert_eq!((expected, actual), (1, 2));
            assert_eq!(renamed.name, "手邊現金");
            assert_eq!(
                repository
                    .get(cash.id)
                    .await
                    .expect("rename winner should remain stored"),
                renamed
            );
        }
        (Err(AccountError::NotFound(id)), Ok(())) => {
            assert_eq!(id, cash.id.get());
            assert!(matches!(
                repository.get(cash.id).await,
                Err(AccountError::NotFound(missing_id)) if missing_id == cash.id.get()
            ));
        }
        (rename_result, delete_result) => panic!(
            "exactly one same-revision mutation should succeed; rename={rename_result:?}, delete={delete_result:?}"
        ),
    }

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn integrity_verification_accepts_a_healthy_database() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    verify_database_integrity(&pool)
        .await
        .expect("a freshly migrated database should be healthy");
}

#[tokio::test]
async fn integrity_verification_rejects_a_changed_application_identity() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");
    sqlx::query("PRAGMA application_id = 123")
        .execute(&pool)
        .await
        .expect("test should change the disposable database identity");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a database with a changed identity must fail closed");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("unexpected SQLite application_id 123")
    ));

    sqlx::query("PRAGMA application_id = 1799434289")
        .execute(&pool)
        .await
        .expect("failed audit must release its read transaction");
    verify_database_integrity(&pool)
        .await
        .expect("restored disposable database identity should verify");
}

#[tokio::test]
async fn integrity_verification_reports_orphaned_foreign_keys() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("test connection should temporarily disable foreign keys");
    sqlx::query("DROP TRIGGER accounts_validate_parent_insert")
        .execute(&pool)
        .await
        .expect("disposable test database should allow simulated corruption");
    sqlx::query(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (9999, '孤兒科目', 'asset', 'posting')",
    )
    .execute(&pool)
    .await
    .expect("test should inject an orphan while enforcement is disabled");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("test connection should restore foreign key enforcement");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("orphaned rows must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("foreign key violation in table accounts")
                && details.contains("referencing accounts")
    ));
}

#[tokio::test]
async fn integrity_verification_reports_bypassed_check_constraints() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&pool)
        .await
        .expect("test connection should temporarily ignore check constraints");
    sqlx::query(
        "INSERT INTO accounts (name, element, kind) VALUES ('損壞科目', 'asset', 'invalid')",
    )
    .execute(&pool)
    .await
    .expect("test should inject invalid data while checks are disabled");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&pool)
        .await
        .expect("test connection should restore check enforcement");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("rows violating CHECK constraints must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("CHECK constraint failed in accounts")
    ));
}

#[tokio::test]
async fn integrity_verification_reports_missing_required_schema_objects() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("DROP TRIGGER accounts_prevent_cycle_update")
        .execute(&pool)
        .await
        .expect("disposable test database should allow simulated schema damage");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("missing required triggers must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("required schema object is missing")
                && details.contains("trigger accounts_prevent_cycle_update")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_unexpected_schema_objects() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");
    sqlx::query(
        r#"
        CREATE TRIGGER unexpected_account_side_effect
        AFTER INSERT ON accounts
        BEGIN
            SELECT 1;
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should add an undeclared no-op trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("an undeclared trigger must fail closed");
    assert!(matches!(error, DatabaseError::Integrity(details)
            if details.contains("unexpected schema object")
                && details.contains("unexpected_account_side_effect")));

    sqlx::query("DROP TRIGGER unexpected_account_side_effect")
        .execute(&pool)
        .await
        .expect("failed audit must release its read transaction");
    verify_database_integrity(&pool)
        .await
        .expect("the repaired disposable schema should verify on a new snapshot");
}

#[tokio::test]
async fn integrity_verification_rejects_a_non_strict_accounts_replacement() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("ALTER TABLE accounts RENAME TO accounts_original")
        .execute(&pool)
        .await
        .expect("disposable test database should allow simulated schema damage");
    sqlx::query(
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL, element TEXT NOT NULL, kind TEXT NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("test should replace the protected table with a non-strict table");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a non-strict replacement table must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("accounts table must exist in SQLite STRICT mode")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_an_accounts_table_with_wrong_columns() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("ALTER TABLE accounts RENAME TO accounts_original")
        .execute(&pool)
        .await
        .expect("disposable test database should allow simulated schema damage");
    sqlx::query(
        r#"
        CREATE TABLE accounts (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER REFERENCES accounts(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
            name TEXT NOT NULL
        ) STRICT
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should replace accounts with an incomplete strict table");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("wrong account columns must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("accounts table columns do not match migration 1")
    ));
}

#[tokio::test]
async fn integrity_verification_requires_non_reusable_account_identifiers() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("PRAGMA writable_schema = ON")
        .execute(&pool)
        .await
        .expect("test should enable disposable schema corruption");
    sqlx::query(
        r#"
        UPDATE sqlite_schema
        SET sql = replace(
            sql,
            'id INTEGER PRIMARY KEY AUTOINCREMENT',
            'id INTEGER PRIMARY KEY'
        )
        WHERE type = 'table' AND name = 'accounts'
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should remove the non-reuse declaration");
    sqlx::query("PRAGMA writable_schema = OFF")
        .execute(&pool)
        .await
        .expect("test should disable direct schema writes");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a reusable account identifier schema must fail closed");
    assert!(matches!(error, DatabaseError::Integrity(details)
            if details.contains("schema definition for accounts")
                && details.contains("AUTOINCREMENT")));
}

#[tokio::test]
async fn integrity_verification_rejects_a_weakened_calendar_constraint() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("PRAGMA writable_schema = ON")
        .execute(&pool)
        .await
        .expect("test should enable disposable schema corruption");
    sqlx::query(
        r#"
        UPDATE sqlite_schema
        SET sql = replace(sql, 'BETWEEN 1 AND 12', 'BETWEEN 0 AND 99')
        WHERE type = 'table' AND name = 'journal_entries'
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should weaken the stored month constraint");
    sqlx::query("PRAGMA writable_schema = OFF")
        .execute(&pool)
        .await
        .expect("test should disable direct schema writes");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a weakened calendar constraint must fail closed");
    assert!(matches!(error, DatabaseError::Integrity(details)
            if details.contains("schema definition for journal_entries")
                && details.contains("BETWEEN 1 AND 12")));
}

#[tokio::test]
async fn integrity_verification_rejects_a_same_name_index_on_wrong_columns() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("DROP INDEX accounts_parent_id_idx")
        .execute(&pool)
        .await
        .expect("test should remove the correct disposable index");
    sqlx::query("CREATE INDEX accounts_parent_id_idx ON accounts(name)")
        .execute(&pool)
        .await
        .expect("test should create a same-name index on the wrong column");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("same-name wrong-column indexes must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("accounts_parent_id_idx columns do not match migration 1")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_same_name_index_on_the_wrong_table() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("DROP INDEX accounts_parent_id_idx")
        .execute(&pool)
        .await
        .expect("test should remove the correct disposable index");
    sqlx::query("CREATE TABLE decoy_accounts (parent_id INTEGER)")
        .execute(&pool)
        .await
        .expect("test should create a decoy table");
    sqlx::query("CREATE INDEX accounts_parent_id_idx ON decoy_accounts(parent_id)")
        .execute(&pool)
        .await
        .expect("test should create a same-name, same-column decoy index");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("an index attached to the wrong table must fail closed");
    assert!(matches!(error, DatabaseError::Integrity(details)
            if details.contains("accounts_parent_id_idx")
                && details.contains("decoy_accounts")
                && details.contains("expected accounts")));
}

#[tokio::test]
async fn integrity_verification_rejects_a_required_trigger_on_the_wrong_table() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("DROP TRIGGER accounts_require_initial_revision")
        .execute(&pool)
        .await
        .expect("test should remove the correctly owned trigger");
    sqlx::query(
        r#"
        CREATE TRIGGER accounts_require_initial_revision
        BEFORE INSERT ON journal_entries
        BEGIN
            SELECT 1;
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should create a same-name trigger on a decoy owner");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a required trigger attached to the wrong table must fail closed");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("accounts_require_initial_revision")
                && details.contains("journal_entries")
                && details.contains("expected accounts")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_missing_account_parent_foreign_key() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    sqlx::query("ALTER TABLE accounts RENAME TO accounts_original")
        .execute(&pool)
        .await
        .expect("disposable test database should allow simulated schema damage");
    sqlx::query(
        r#"
        CREATE TABLE accounts (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER,
            name TEXT NOT NULL,
            element TEXT NOT NULL,
            kind TEXT NOT NULL,
            is_active INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should replace accounts with a table lacking its parent foreign key");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("missing account parent foreign key must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("accounts parent foreign key does not match migration 1")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_preexisting_account_cycle() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    let first_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (name, element, kind) VALUES ('第一層', 'asset', 'category') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("first test category should insert");
    let second_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (?, '第二層', 'asset', 'category') RETURNING id",
    )
    .bind(first_id)
    .fetch_one(&pool)
    .await
    .expect("second test category should insert");

    sqlx::query("DROP TRIGGER accounts_prevent_cycle_update")
        .execute(&pool)
        .await
        .expect("test should temporarily remove cycle protection");
    sqlx::query("UPDATE accounts SET parent_id = ?, revision = revision + 1 WHERE id = ?")
        .bind(second_id)
        .bind(first_id)
        .execute(&pool)
        .await
        .expect("test should inject a cycle while its trigger is absent");
    sqlx::query(
        r#"
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
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should restore the required trigger after simulating old corruption");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a preexisting hierarchy cycle must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("account hierarchy cycle detected")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_preexisting_posting_parent() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    let parent_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (name, element, kind) VALUES ('父分類', 'asset', 'category') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test category should insert");
    sqlx::query(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (?, '子科目', 'asset', 'posting')",
    )
    .bind(parent_id)
    .execute(&pool)
    .await
    .expect("test child should insert");

    sqlx::query("DROP TRIGGER accounts_posting_kind_requires_leaf")
        .execute(&pool)
        .await
        .expect("test should temporarily remove posting-leaf protection");
    sqlx::query("UPDATE accounts SET kind = 'posting', revision = revision + 1 WHERE id = ?")
        .bind(parent_id)
        .execute(&pool)
        .await
        .expect("test should inject a posting parent while its trigger is absent");
    sqlx::query(
        r#"
        CREATE TRIGGER accounts_posting_kind_requires_leaf
        BEFORE UPDATE OF kind ON accounts
        WHEN NEW.kind = 'posting' AND OLD.kind <> 'posting'
        BEGIN
            SELECT RAISE(ABORT, 'posting account cannot have children')
            WHERE EXISTS (
                SELECT 1 FROM accounts AS child WHERE child.parent_id = OLD.id
            );
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should restore the required trigger after simulating old corruption");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a preexisting posting parent must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("invalid account hierarchy relationship")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_preexisting_cross_element_relationship() {
    let pool = open_memory_database()
        .await
        .expect("in-memory database should open");

    let parent_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (name, element, kind) VALUES ('資產', 'asset', 'category') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("test category should insert");
    sqlx::query(
        "INSERT INTO accounts (parent_id, name, element, kind) VALUES (?, '現金', 'asset', 'posting')",
    )
    .bind(parent_id)
    .execute(&pool)
    .await
    .expect("test child should insert");

    sqlx::query("DROP TRIGGER accounts_preserve_child_elements")
        .execute(&pool)
        .await
        .expect("test should temporarily remove element protection");
    sqlx::query("UPDATE accounts SET element = 'liability', revision = revision + 1 WHERE id = ?")
        .bind(parent_id)
        .execute(&pool)
        .await
        .expect("test should inject an element mismatch while its trigger is absent");
    sqlx::query(
        r#"
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
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should restore the required trigger after simulating old corruption");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a cross-element relationship must fail integrity verification");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("invalid account hierarchy relationship")
    ));
}

#[tokio::test]
async fn file_database_refuses_schema_damage_when_reopened() {
    let database_path = unique_database_path("schema-damage");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    pool.close().await;

    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&database_path))
        .await
        .expect("test should open the file without application verification");
    sqlx::query("DROP TRIGGER accounts_posting_kind_requires_leaf")
        .execute(&raw_pool)
        .await
        .expect("disposable test database should allow simulated schema damage");
    raw_pool.close().await;

    let error = match open_database(&database_path).await {
        Ok(pool) => {
            pool.close().await;
            panic!("database opening must reject missing required triggers");
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("trigger accounts_posting_kind_requires_leaf")
    ));

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_refuses_a_migration_checksum_mismatch() {
    let database_path = unique_database_path("migration-checksum");
    let pool = open_database(&database_path)
        .await
        .expect("temporary file database should open");
    pool.close().await;

    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&database_path))
        .await
        .expect("test should reopen the disposable database directly");
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
        .execute(&raw_pool)
        .await
        .expect("test should simulate migration-history tampering");
    raw_pool.close().await;

    let error = match open_database(&database_path).await {
        Ok(pool) => {
            pool.close().await;
            panic!("database opening must reject a migration checksum mismatch");
        }
        Err(error) => error,
    };
    assert!(matches!(error, DatabaseError::Migrate(_)));

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_refuses_to_migrate_an_unrelated_sqlite_file() {
    let database_path = unique_database_path("unrelated");
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&database_path)
                .create_if_missing(true),
        )
        .await
        .expect("test should create an unrelated SQLite file");
    sqlx::query("CREATE TABLE sqlitex_data (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        .execute(&raw_pool)
        .await
        .expect("unrelated test schema should be created");
    raw_pool.close().await;
    #[cfg(unix)]
    std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o640))
        .expect("test should assign a recognizable unrelated-file mode");

    let error = match open_database(&database_path).await {
        Ok(pool) => {
            pool.close().await;
            panic!("application must not migrate an unrelated SQLite file");
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DatabaseError::UnrecognizedDatabase {
            application_id: 0,
            schema_object_count: 1
        }
    ));

    let inspection_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&database_path))
        .await
        .expect("unrelated file should remain readable after refusal");
    let application_object_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('accounts', '_sqlx_migrations')",
    )
    .fetch_one(&inspection_pool)
    .await
    .expect("unrelated schema should remain inspectable");
    let unrelated_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'sqlitex_data'",
    )
    .fetch_one(&inspection_pool)
    .await
    .expect("unrelated table should remain inspectable");
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&inspection_pool)
        .await
        .expect("unrelated journal mode should remain inspectable");
    assert_eq!(application_object_count, 0);
    assert_eq!(unrelated_table_count, 1);
    assert_eq!(journal_mode, "delete");
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&database_path)
            .expect("unrelated file metadata should remain readable")
            .permissions()
            .mode()
            & 0o777,
        0o640,
        "identity rejection must occur before chmod"
    );
    inspection_pool.close().await;

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_refuses_another_application_id_before_migration() {
    let database_path = unique_database_path("foreign-application-id");
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&database_path)
                .create_if_missing(true),
        )
        .await
        .expect("test should create an empty foreign application database");
    sqlx::query("PRAGMA application_id = 123")
        .execute(&raw_pool)
        .await
        .expect("test should mark the file for another application");
    raw_pool.close().await;

    let error = match open_database(&database_path).await {
        Ok(pool) => {
            pool.close().await;
            panic!("application must not claim another application's database");
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DatabaseError::UnrecognizedDatabase {
            application_id: 123,
            schema_object_count: 0
        }
    ));

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn file_database_does_not_modify_a_non_sqlite_file() {
    let database_path = unique_database_path("not-sqlite");
    let original_contents = b"This is not a SQLite database.\n";
    std::fs::write(&database_path, original_contents)
        .expect("test should create a disposable non-SQLite file");

    let error = match open_database(&database_path).await {
        Ok(pool) => {
            pool.close().await;
            panic!("application must not claim a non-SQLite file");
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DatabaseError::Connect(_) | DatabaseError::Query(_)
    ));
    assert_eq!(
        std::fs::read(&database_path).expect("original file should remain readable"),
        original_contents
    );

    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn integrity_verification_rejects_a_wrong_journal_index_signature() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("DROP INDEX journal_lines_account_id_idx")
        .execute(&pool)
        .await
        .expect("test should remove the real index");
    sqlx::query("CREATE INDEX journal_lines_account_id_idx ON journal_lines(entry_id, account_id)")
        .execute(&pool)
        .await
        .expect("test should create a same-name wrong index");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("wrong journal index order must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("journal_lines_account_id_idx columns"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_preexisting_unbalanced_posted_entry() {
    let pool = open_memory_database().await.expect("database should open");
    let repository = AccountRepository::new(pool.clone());
    let cash = repository
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("cash should be created");
    let revenue = repository
        .create(NewAccount::new(
            None,
            "收入",
            AccountingElement::Income,
            AccountKind::Posting,
        ))
        .await
        .expect("revenue should be created");
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("draft should be inserted");
    for (line_no, account_id, side, amount) in [
        (1, cash.id.get(), "debit", 101),
        (2, revenue.id.get(), "credit", 100),
    ] {
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry_id)
        .bind(line_no)
        .bind(account_id)
        .bind(side)
        .bind(amount)
        .execute(&pool)
        .await
        .expect("test draft line should be inserted");
    }

    sqlx::query("DROP TRIGGER journal_entries_validate_posting")
        .execute(&pool)
        .await
        .expect("test should temporarily remove the posting guard");
    sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .expect("test should inject an invalid posted entry");
    sqlx::query(
        r#"
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
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should restore the required trigger name and behavior");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("stored unbalanced posting must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("posted journal entry") && message.contains("unbalanced"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_posted_entry_with_only_one_line() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("cash should be created");
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![JournalLineInput::new(
                cash.id,
                JournalSide::Debit,
                MinorUnits::new(100),
            )],
        ))
        .await
        .expect("one-line draft should be allowed");
    let posting_trigger_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = 'journal_entries_validate_posting'",
    )
    .fetch_one(&pool)
    .await
    .expect("posting trigger definition should be readable");

    sqlx::query("DROP TRIGGER journal_entries_validate_posting")
        .execute(&pool)
        .await
        .expect("test should temporarily remove the posting guard");
    sqlx::query(
        r#"
        UPDATE journal_entries
        SET status = 'posted',
            posted_at = '2026-10-22T00:00:00.000Z',
            revision = revision + 1
        WHERE id = ?
        "#,
    )
    .bind(draft.id.get())
    .execute(&pool)
    .await
    .expect("test should inject a one-line posted entry");
    sqlx::query(sqlx::AssertSqlSafe(posting_trigger_sql))
        .execute(&pool)
        .await
        .expect("test should restore the exact posting trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a posted entry with fewer than two lines must fail closed");
    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains(&draft.id.get().to_string())
                && details.contains("fewer than two lines")
    ));
}

#[tokio::test]
async fn integrity_verification_rejects_a_same_name_noop_journal_trigger() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("DROP TRIGGER journal_entries_validate_posting")
        .execute(&pool)
        .await
        .expect("test should remove the real trigger");
    sqlx::query(
        r#"
        CREATE TRIGGER journal_entries_validate_posting
        BEFORE UPDATE OF status ON journal_entries
        BEGIN
            SELECT 1;
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should create a same-name no-op trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("same-name trigger replacement must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("journal_entries_validate_posting") && message.contains("required SQL fragment"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_same_name_noop_draft_revision_trigger() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("DROP TRIGGER journal_entries_require_next_draft_revision")
        .execute(&pool)
        .await
        .expect("test should remove the real revision trigger");
    sqlx::query(
        r#"
        CREATE TRIGGER journal_entries_require_next_draft_revision
        BEFORE UPDATE OF entry_date, revision ON journal_entries
        BEGIN
            SELECT 1;
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should create a same-name no-op revision trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("same-name revision trigger replacement must fail closed");
    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("journal_entries_require_next_draft_revision") && message.contains("required SQL fragment"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_same_name_noop_account_trigger() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("DROP TRIGGER accounts_prevent_cycle_update")
        .execute(&pool)
        .await
        .expect("test should remove the real trigger");
    sqlx::query(
        r#"
        CREATE TRIGGER accounts_prevent_cycle_update
        BEFORE UPDATE OF parent_id ON accounts
        BEGIN
            SELECT 1;
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should create a same-name no-op trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("same-name account trigger replacement must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("accounts_prevent_cycle_update") && message.contains("required SQL fragment"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_posted_line_reclassified_as_category() {
    let pool = open_memory_database().await.expect("database should open");
    let accounts = AccountRepository::new(pool.clone());
    let journal = JournalRepository::new(pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("cash should be created");
    let capital = accounts
        .create(NewAccount::new(
            None,
            "資本",
            AccountingElement::Equity,
            AccountKind::Posting,
        ))
        .await
        .expect("capital should be created");
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(100)),
                JournalLineInput::new(capital.id, JournalSide::Credit, MinorUnits::new(100)),
            ],
        ))
        .await
        .expect("entry should post");

    sqlx::query("DROP TRIGGER accounts_preserve_journal_classification")
        .execute(&pool)
        .await
        .expect("test should temporarily remove history classification guard");
    sqlx::query(
        r#"
        UPDATE accounts
        SET kind = 'category',
            revision = revision + 1,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(cash.id.get())
    .execute(&pool)
    .await
    .expect("test should inject invalid historical classification");
    sqlx::query(
        r#"
        CREATE TRIGGER accounts_preserve_journal_classification
        BEFORE UPDATE OF element, kind ON accounts
        WHEN (NEW.element IS NOT OLD.element OR NEW.kind IS NOT OLD.kind)
          AND EXISTS (SELECT 1 FROM journal_lines WHERE account_id = OLD.id)
        BEGIN
            SELECT RAISE(ABORT, 'account journal classification is immutable');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should restore the required classification trigger");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("posted line referencing a category must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains(&posted.id.get().to_string()) && message.contains("non-posting account"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_migration_history_checksum_mismatch() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 2")
        .execute(&pool)
        .await
        .expect("test should corrupt migration history");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("migration checksum mismatch must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("migration 2 checksum"))
    );
}

#[tokio::test]
async fn integrity_verification_rejects_a_retyped_migration_history_table() {
    let pool = open_memory_database().await.expect("database should open");
    sqlx::query("ALTER TABLE _sqlx_migrations RENAME TO original_migration_history")
        .execute(&pool)
        .await
        .expect("test should preserve the real migration table temporarily");
    sqlx::query(
        r#"
        CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMP NOT NULL,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL,
            execution_time TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("test should create a same-name table with one wrong column type");
    sqlx::query(
        r#"
        INSERT INTO _sqlx_migrations (
            version, description, installed_on, success, checksum, execution_time
        )
        SELECT version, description, installed_on, success, checksum,
               CAST(execution_time AS TEXT)
        FROM original_migration_history
        "#,
    )
    .execute(&pool)
    .await
    .expect("the replacement should retain convincing migration rows");

    let error = verify_database_integrity(&pool)
        .await
        .expect_err("a retyped migration table must fail closed");

    assert!(
        matches!(error, DatabaseError::Integrity(message) if message.contains("_sqlx_migrations table columns"))
    );
}

#[tokio::test]
async fn integrity_verification_observes_consistent_snapshots_during_valid_writes() {
    let database_path = unique_database_path("audit-concurrent-writes");
    let pool = open_database(&database_path)
        .await
        .expect("file database should open");
    let repository = AccountRepository::new(pool.clone());

    let writer = async {
        for sequence in 0..12 {
            repository
                .create(NewAccount::new(
                    None,
                    format!("並行科目 {sequence}"),
                    AccountingElement::Asset,
                    AccountKind::Posting,
                ))
                .await
                .expect("every concurrent account write should remain valid");
            tokio::task::yield_now().await;
        }
    };
    let auditor = async {
        for _ in 0..8 {
            verify_database_integrity(&pool)
                .await
                .expect("each audit should observe one internally consistent snapshot");
            tokio::task::yield_now().await;
        }
    };

    tokio::join!(writer, auditor);

    let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .expect("final account count should be readable");
    assert_eq!(account_count, 12);
    verify_database_integrity(&pool)
        .await
        .expect("the final committed state should remain healthy");

    pool.close().await;
    remove_sqlite_files(&database_path);
}

#[tokio::test]
async fn backup_creates_a_verified_owner_only_point_in_time_snapshot() {
    let source_path = unique_database_path("backup-source");
    let backup_path = unique_database_path("verified-backup");
    let source_pool = open_database(&source_path)
        .await
        .expect("source database should open");
    let accounts = AccountRepository::new(source_pool.clone());
    let journal = JournalRepository::new(source_pool.clone());
    let cash = accounts
        .create(NewAccount::new(
            None,
            "現金",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("cash should be created");
    let capital = accounts
        .create(NewAccount::new(
            None,
            "資本",
            AccountingElement::Equity,
            AccountKind::Posting,
        ))
        .await
        .expect("capital should be created");
    let posted = journal
        .post(PostEntry::new(
            JournalDate::parse("2026-10-22").unwrap(),
            vec![
                JournalLineInput::new(cash.id, JournalSide::Debit, MinorUnits::new(1_000)),
                JournalLineInput::new(capital.id, JournalSide::Credit, MinorUnits::new(1_000)),
            ],
        ))
        .await
        .expect("entry should post before backup");
    let draft = journal
        .create_draft(SaveDraftEntry::new(
            JournalDate::parse("2026-10-23").unwrap(),
            vec![JournalLineInput::new(
                cash.id,
                JournalSide::Debit,
                MinorUnits::new(250),
            )],
        ))
        .await
        .expect("incomplete draft should exist before backup");

    let outcome = backup_database(&source_pool, &backup_path)
        .await
        .expect("backup should be created and verified");
    assert!(
        outcome.cleanup_warning().is_none(),
        "normal backup should remove all private work"
    );
    verify_database_snapshot(&backup_path)
        .await
        .expect("published backup should pass a separate read-only verification");

    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&backup_path)
            .expect("backup metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "published backup must remain owner-only"
    );

    accounts
        .create(NewAccount::new(
            None,
            "備份後新增",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("source should continue changing after its snapshot");
    journal
        .replace_draft(
            draft.id,
            draft.revision,
            SaveDraftEntry::new(JournalDate::parse("2026-10-24").unwrap(), vec![]),
        )
        .await
        .expect("source draft should continue changing after its snapshot");

    let backup_pool = open_database(&backup_path)
        .await
        .expect("verified backup should reopen as an application database");
    let backup_account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&backup_pool)
        .await
        .expect("backup account count should be readable");
    let backup_journal = JournalRepository::new(backup_pool.clone());
    let reloaded = backup_journal
        .get_posted(posted.id)
        .await
        .expect("posted entry should be present in the backup");
    let reloaded_draft = backup_journal
        .get_draft(draft.id)
        .await
        .expect("draft and its revision should be present in the backup");

    assert_eq!(
        backup_account_count, 2,
        "backup must be a point-in-time copy"
    );
    assert_eq!(reloaded, posted);
    assert_eq!(reloaded_draft, draft);

    backup_pool.close().await;
    source_pool.close().await;
    remove_sqlite_files(&source_path);
    remove_sqlite_files(&backup_path);
}

#[tokio::test]
async fn backup_refuses_to_overwrite_an_existing_destination() {
    let backup_path = unique_database_path("existing-backup");
    let original = b"existing backup marker";
    std::fs::write(&backup_path, original).expect("test destination should be created");
    let source_pool = open_memory_database()
        .await
        .expect("source database should open");

    let error = backup_database(&source_pool, &backup_path)
        .await
        .expect_err("existing backup must not be overwritten");

    assert!(matches!(error, BackupError::DestinationExists(path) if path == backup_path));
    assert_eq!(
        std::fs::read(&backup_path).expect("existing destination should remain readable"),
        original
    );

    source_pool.close().await;
    std::fs::remove_file(&backup_path).expect("temporary marker should be removed");
}

#[tokio::test]
async fn concurrent_backups_to_one_destination_publish_exactly_one_source() {
    let first_source_path = unique_database_path("backup-race-source-first");
    let second_source_path = unique_database_path("backup-race-source-second");
    let backup_path = unique_database_path("backup-race-destination");
    let first_pool = open_database(&first_source_path)
        .await
        .expect("first source database should open");
    let second_pool = open_database(&second_source_path)
        .await
        .expect("second source database should open");

    AccountRepository::new(first_pool.clone())
        .create(NewAccount::new(
            None,
            "來源甲",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("first source marker should be created");
    AccountRepository::new(second_pool.clone())
        .create(NewAccount::new(
            None,
            "來源乙",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("second source marker should be created");

    let (first_result, second_result) = tokio::join!(
        backup_database(&first_pool, &backup_path),
        backup_database(&second_pool, &backup_path),
    );
    let results = [first_result, second_result];
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "exactly one verified snapshot should be published"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(BackupError::DestinationExists(path)) if path == &backup_path))
            .count(),
        1,
        "the losing publisher must report no-overwrite protection"
    );

    verify_database_snapshot(&backup_path)
        .await
        .expect("the winning snapshot should remain valid");
    let options = SqliteConnectOptions::new()
        .filename(&backup_path)
        .read_only(true);
    let backup_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("published snapshot should open read-only");
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM accounts ORDER BY id")
        .fetch_all(&backup_pool)
        .await
        .expect("published account marker should be readable");
    assert!(
        names == ["來源甲"] || names == ["來源乙"],
        "the destination must contain one complete source snapshot, never a mixture"
    );

    backup_pool.close().await;
    first_pool.close().await;
    second_pool.close().await;
    remove_sqlite_files(&first_source_path);
    remove_sqlite_files(&second_source_path);
    remove_sqlite_files(&backup_path);
}

#[tokio::test]
async fn backup_refuses_a_missing_destination_directory_without_creating_it() {
    let missing_parent = unique_database_path("missing-backup-parent");
    let backup_path = missing_parent.join("backup.sqlite3");
    let pool = open_memory_database()
        .await
        .expect("disposable source database should open");

    let error = backup_database(&pool, &backup_path)
        .await
        .expect_err("backup policy must not invent an unapproved destination directory");
    assert!(matches!(error, BackupError::MissingParent(path) if path == missing_parent));
    assert!(!missing_parent.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn backup_refuses_an_existing_symlink_without_following_it() {
    let backup_path = unique_database_path("symlink-backup");
    let target_path = unique_database_path("symlink-target");
    let original = b"do not replace through a symlink";
    std::fs::write(&target_path, original).expect("test target should be created");
    symlink(&target_path, &backup_path).expect("test backup path should be a symlink");
    let source_pool = open_memory_database()
        .await
        .expect("source database should open");

    let error = backup_database(&source_pool, &backup_path)
        .await
        .expect_err("an existing symlink destination must be rejected");

    assert!(matches!(error, BackupError::DestinationExists(path) if path == backup_path));
    assert!(
        std::fs::symlink_metadata(&backup_path)
            .expect("symlink metadata should remain readable")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(&target_path).expect("symlink target should remain readable"),
        original
    );

    source_pool.close().await;
    std::fs::remove_file(&backup_path).expect("temporary symlink should be removed");
    std::fs::remove_file(&target_path).expect("temporary target should be removed");
}

#[tokio::test]
async fn backup_does_not_publish_a_snapshot_with_invalid_migration_history() {
    let source_path = unique_database_path("backup-invalid-source");
    let backup_path = unique_database_path("backup-invalid-destination");
    let source_pool = open_database(&source_path)
        .await
        .expect("source database should open");
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 2")
        .execute(&source_pool)
        .await
        .expect("test should corrupt source migration history");

    let error = backup_database(&source_pool, &backup_path)
        .await
        .expect_err("invalid snapshot must not be published");

    assert!(matches!(
        error,
        BackupError::Verification(DatabaseError::Integrity(message))
            if message.contains("migration 2 checksum")
    ));
    assert!(!backup_path.exists());

    source_pool.close().await;
    remove_sqlite_files(&source_path);
}

#[tokio::test]
async fn snapshot_verification_refuses_an_unrelated_database_without_modifying_it() {
    let candidate_path = unique_database_path("unrelated-restore-candidate");
    let options = SqliteConnectOptions::new()
        .filename(&candidate_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("unrelated test database should open");
    sqlx::query("CREATE TABLE unrelated_data (id INTEGER PRIMARY KEY, value TEXT)")
        .execute(&pool)
        .await
        .expect("unrelated schema should be created");
    sqlx::query("INSERT INTO unrelated_data (value) VALUES ('keep me')")
        .execute(&pool)
        .await
        .expect("unrelated content should be created");
    pool.close().await;
    let original = std::fs::read(&candidate_path).expect("candidate should be readable");

    let error = verify_database_snapshot(&candidate_path)
        .await
        .expect_err("unrelated candidate must be rejected");

    assert!(matches!(error, DatabaseError::UnrecognizedDatabase { .. }));
    assert_eq!(
        std::fs::read(&candidate_path).expect("candidate should remain readable"),
        original
    );
    assert!(!candidate_path.with_extension("sqlite3-wal").exists());
    assert!(!candidate_path.with_extension("sqlite3-shm").exists());

    std::fs::remove_file(&candidate_path).expect("temporary candidate should be removed");
}

#[tokio::test]
async fn snapshot_verification_rejects_schema_tampering_without_modifying_the_candidate() {
    let source_path = unique_database_path("snapshot-tamper-source");
    let candidate_path = unique_database_path("snapshot-tamper-candidate");
    let source_pool = open_database(&source_path)
        .await
        .expect("source database should open");
    backup_database(&source_pool, &candidate_path)
        .await
        .expect("candidate snapshot should be created");

    let options = SqliteConnectOptions::new().filename(&candidate_path);
    let tamper_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("candidate should open for disposable tampering");
    sqlx::query("DROP TRIGGER journal_lines_delete_only_from_draft")
        .execute(&tamper_pool)
        .await
        .expect("test should remove a required trigger");
    tamper_pool.close().await;

    let original = std::fs::read(&candidate_path).expect("candidate bytes should be readable");
    #[cfg(unix)]
    let original_mode = std::fs::metadata(&candidate_path)
        .expect("candidate metadata should be readable")
        .permissions()
        .mode()
        & 0o777;

    let error = verify_database_snapshot(&candidate_path)
        .await
        .expect_err("a tampered snapshot must fail closed");

    assert!(matches!(
        error,
        DatabaseError::Integrity(details)
            if details.contains("required schema object is missing")
                && details.contains("journal_lines_delete_only_from_draft")
    ));
    assert_eq!(
        std::fs::read(&candidate_path).expect("candidate should remain readable"),
        original,
        "read-only verification must not rewrite a rejected candidate"
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&candidate_path)
            .expect("candidate metadata should remain readable")
            .permissions()
            .mode()
            & 0o777,
        original_mode,
        "snapshot verification must not change candidate permissions"
    );
    assert!(!candidate_path.with_extension("sqlite3-wal").exists());
    assert!(!candidate_path.with_extension("sqlite3-shm").exists());

    source_pool.close().await;
    remove_sqlite_files(&source_path);
    remove_sqlite_files(&candidate_path);
}

#[cfg(unix)]
#[tokio::test]
async fn concurrent_snapshot_verification_is_read_only_for_a_healthy_candidate() {
    let source_path = unique_database_path("snapshot-read-only-source");
    let candidate_path = unique_database_path("snapshot-read-only-candidate");
    let source_pool = open_database(&source_path)
        .await
        .expect("source database should open");
    AccountRepository::new(source_pool.clone())
        .create(NewAccount::new(
            None,
            "唯讀驗證科目",
            AccountingElement::Asset,
            AccountKind::Posting,
        ))
        .await
        .expect("source marker should be created");
    backup_database(&source_pool, &candidate_path)
        .await
        .expect("healthy candidate should be created");
    std::fs::set_permissions(&candidate_path, std::fs::Permissions::from_mode(0o640))
        .expect("test should set a recognizable candidate mode");
    let original = std::fs::read(&candidate_path).expect("candidate bytes should be readable");

    let (first, second, third) = tokio::join!(
        verify_database_snapshot(&candidate_path),
        verify_database_snapshot(&candidate_path),
        verify_database_snapshot(&candidate_path),
    );
    first.expect("first concurrent verification should succeed");
    second.expect("second concurrent verification should succeed");
    third.expect("third concurrent verification should succeed");

    assert_eq!(
        std::fs::read(&candidate_path).expect("candidate should remain readable"),
        original,
        "successful snapshot verification must not rewrite the candidate"
    );
    assert_eq!(
        std::fs::metadata(&candidate_path)
            .expect("candidate metadata should remain readable")
            .permissions()
            .mode()
            & 0o777,
        0o640,
        "snapshot verification must not harden or otherwise change permissions"
    );
    assert!(!candidate_path.with_extension("sqlite3-wal").exists());
    assert!(!candidate_path.with_extension("sqlite3-shm").exists());

    source_pool.close().await;
    remove_sqlite_files(&source_path);
    remove_sqlite_files(&candidate_path);
}

#[tokio::test]
async fn snapshot_verification_refuses_a_missing_path_without_creating_it() {
    let candidate_path = unique_database_path("missing-snapshot-candidate");

    let error = verify_database_snapshot(&candidate_path)
        .await
        .expect_err("a missing snapshot path must not be created as an empty database");

    assert!(matches!(error, DatabaseError::Connect(_)));
    assert!(!candidate_path.exists());
    assert!(!candidate_path.with_extension("sqlite3-wal").exists());
    assert!(!candidate_path.with_extension("sqlite3-shm").exists());
}

fn unique_database_path(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "kaccounting-extra-01-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

fn remove_sqlite_files(database_path: &std::path::Path) {
    for path in [
        database_path.to_path_buf(),
        database_path.with_extension("sqlite3-wal"),
        database_path.with_extension("sqlite3-shm"),
        database_path.with_extension("sqlite3-journal"),
    ] {
        if path.exists() {
            std::fs::remove_file(path).expect("temporary SQLite file should be removable");
        }
    }
}
