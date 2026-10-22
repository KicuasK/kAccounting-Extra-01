use sqlx::SqlitePool;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const APPLICATION_ID: i64 = 0x6B41_3031; // ASCII "kA01"
const CLAIM_APPLICATION_ID_SQL: &str = "PRAGMA application_id = 1799434289";

const REQUIRED_SCHEMA_OBJECTS: &[(&str, &str)] = &[
    ("table", "_sqlx_migrations"),
    ("table", "accounts"),
    ("index", "accounts_parent_id_idx"),
    ("index", "accounts_sibling_name_idx"),
    ("index", "accounts_root_name_idx"),
    ("trigger", "accounts_protect_active_descendants"),
    ("trigger", "accounts_validate_active_parent_insert"),
    ("trigger", "accounts_validate_active_parent_update"),
    ("index", "accounts_element_kind_active_idx"),
    ("trigger", "accounts_require_initial_revision"),
    ("trigger", "accounts_require_next_revision"),
    ("trigger", "accounts_preserve_id"),
    ("trigger", "accounts_preserve_created_at"),
    ("trigger", "accounts_validate_parent_insert"),
    ("trigger", "accounts_validate_parent_update"),
    ("trigger", "accounts_prevent_cycle_update"),
    ("trigger", "accounts_preserve_child_elements"),
    ("trigger", "accounts_posting_kind_requires_leaf"),
    ("table", "journal_entries"),
    ("table", "journal_lines"),
    ("table", "attachments"),
    ("index", "attachments_entry_id_idx"),
    ("trigger", "attachments_enforce_book_quota"),
    ("trigger", "attachments_immutable_update"),
    ("trigger", "attachments_validate_owner_insert"),
    ("trigger", "attachments_posted_immutable_delete"),
    ("index", "journal_entries_status_date_idx"),
    ("index", "journal_entries_one_reversal_idx"),
    ("index", "journal_entries_correction_target_idx"),
    ("index", "journal_lines_entry_id_idx"),
    ("index", "journal_lines_account_id_idx"),
    ("trigger", "accounts_preserve_journal_classification"),
    ("trigger", "journal_entries_insert_as_draft"),
    ("trigger", "journal_entries_preserve_identity"),
    ("trigger", "journal_entries_posted_immutable_update"),
    ("trigger", "journal_entries_posted_immutable_delete"),
    ("trigger", "journal_entries_validate_posting"),
    ("trigger", "journal_entries_require_initial_revision"),
    ("trigger", "journal_entries_require_next_draft_revision"),
    ("trigger", "journal_entries_validate_posted_revision"),
    ("trigger", "journal_entries_preserve_relationship"),
    ("trigger", "journal_entries_validate_relationship_insert"),
    ("trigger", "journal_entries_validate_relationship_posting"),
    ("trigger", "journal_lines_insert_only_into_draft"),
    ("trigger", "journal_lines_validate_account_insert"),
    ("trigger", "journal_lines_update_only_in_draft"),
    ("trigger", "journal_lines_preserve_identity"),
    ("trigger", "journal_lines_validate_account_update"),
    ("trigger", "journal_lines_delete_only_from_draft"),
];

const REQUIRED_ACCOUNT_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("id", "INTEGER", 0, 1),
    ("parent_id", "INTEGER", 0, 0),
    ("name", "TEXT", 1, 0),
    ("element", "TEXT", 1, 0),
    ("kind", "TEXT", 1, 0),
    ("is_active", "INTEGER", 1, 0),
    ("revision", "INTEGER", 1, 0),
    ("created_at", "TEXT", 1, 0),
    ("updated_at", "TEXT", 1, 0),
];

const REQUIRED_MIGRATION_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("version", "BIGINT", 0, 1),
    ("description", "TEXT", 1, 0),
    ("installed_on", "TIMESTAMP", 1, 0),
    ("success", "BOOLEAN", 1, 0),
    ("checksum", "BLOB", 1, 0),
    ("execution_time", "BIGINT", 1, 0),
];

const REQUIRED_JOURNAL_ENTRY_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("id", "INTEGER", 0, 1),
    ("entry_date", "TEXT", 1, 0),
    ("status", "TEXT", 1, 0),
    ("created_at", "TEXT", 1, 0),
    ("posted_at", "TEXT", 0, 0),
    ("revision", "INTEGER", 1, 0),
    ("reverses_entry_id", "INTEGER", 0, 0),
    ("corrects_entry_id", "INTEGER", 0, 0),
    ("explanation", "TEXT", 0, 0),
    ("reference", "TEXT", 0, 0),
];

const REQUIRED_JOURNAL_LINE_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("id", "INTEGER", 0, 1),
    ("entry_id", "INTEGER", 1, 0),
    ("line_no", "INTEGER", 1, 0),
    ("account_id", "INTEGER", 1, 0),
    ("side", "TEXT", 1, 0),
    ("amount_minor", "INTEGER", 1, 0),
    ("created_at", "TEXT", 1, 0),
    ("memo", "TEXT", 0, 0),
];

const REQUIRED_SCHEMA_SQL_FRAGMENTS: &[(&str, &[&str])] = &[
    (
        "attachments_validate_owner_insert",
        &[
            "BEFORE INSERT",
            "owner.id = NEW.entry_id",
            "owner.revision >= 1",
            "owner.revision >= 2",
            "owner.posted_at IS NULL",
            "owner.posted_at IS NOT NULL",
            "attachment owner must be a valid journal entry",
        ],
    ),
    (
        "attachments",
        &[
            "length(id) = 32",
            "id NOT GLOB '*[^0-9a-f]*'",
            "length(display_name) BETWEEN 1 AND 255",
            "display_name NOT IN ('.', '..')",
            "instr(display_name, '/') = 0",
            "instr(display_name, char(92)) = 0",
            "instr(display_name, char(0)) = 0",
            "char(127) || '-' || char(159)",
            "length(plaintext_sha256) = 64",
            "plaintext_sha256 NOT GLOB '*[^0-9a-f]*'",
            "byte_length BETWEEN 0 AND 262144000",
            "length(ciphertext_sha256) = 64",
            "ciphertext_sha256 NOT GLOB '*[^0-9a-f]*'",
            "ciphertext_byte_length = byte_length + 72 + 20 * max(1, (byte_length + 65535) / 65536)",
        ],
    ),
    (
        "attachments_entry_id_idx",
        &["attachments(entry_id, created_at, id)"],
    ),
    (
        "attachments_enforce_book_quota",
        &[
            "BEFORE INSERT",
            "21474836480",
            "SUM(byte_length)",
            "attachment book quota exceeded",
        ],
    ),
    (
        "attachments_immutable_update",
        &["BEFORE UPDATE", "attachment metadata is immutable"],
    ),
    (
        "attachments_posted_immutable_delete",
        &[
            "BEFORE DELETE",
            "status = 'posted'",
            "posted attachment cannot be deleted",
        ],
    ),
    (
        "accounts_sibling_name_idx",
        &[
            "CREATE UNIQUE INDEX",
            "accounts(parent_id, name)",
            "WHERE parent_id IS NOT NULL",
        ],
    ),
    (
        "accounts_root_name_idx",
        &[
            "CREATE UNIQUE INDEX",
            "accounts(name)",
            "WHERE parent_id IS NULL",
        ],
    ),
    (
        "accounts_protect_active_descendants",
        &[
            "UPDATE OF is_active",
            "NEW.is_active = 0",
            "WITH RECURSIVE descendants",
            "category has active descendants",
        ],
    ),
    (
        "accounts_validate_active_parent_insert",
        &[
            "BEFORE INSERT",
            "NEW.is_active = 1",
            "WITH RECURSIVE ancestors",
            "active account requires active ancestors",
        ],
    ),
    (
        "accounts_validate_active_parent_update",
        &[
            "UPDATE OF parent_id, is_active",
            "WITH RECURSIVE subtree",
            "ancestors",
            "active account requires active ancestors",
        ],
    ),
    (
        "accounts",
        &[
            "id INTEGER PRIMARY KEY AUTOINCREMENT",
            "CHECK (id > 0)",
            "name = trim(name)",
            "length(name) BETWEEN 1 AND 120",
            "instr(name, char(0)) = 0",
            "char(127) || '-' || char(159)",
            "8287, 12288",
            "element IN ('asset', 'liability', 'equity', 'income', 'expense')",
            "kind IN ('category', 'posting')",
            "is_active IN (0, 1)",
            "revision >= 1",
            "created_at TEXT NOT NULL DEFAULT",
            "updated_at TEXT NOT NULL DEFAULT",
            "parent_id IS NULL OR parent_id <> id",
        ],
    ),
    (
        "accounts_require_initial_revision",
        &["NEW.revision <> 1", "account revision must start at one"],
    ),
    (
        "accounts_require_next_revision",
        &[
            "NEW.revision <> OLD.revision + 1",
            "account revision must increase by one",
        ],
    ),
    (
        "accounts_preserve_id",
        &["NEW.id IS NOT OLD.id", "account id is immutable"],
    ),
    (
        "accounts_preserve_created_at",
        &[
            "NEW.created_at IS NOT OLD.created_at",
            "account creation timestamp is immutable",
        ],
    ),
    (
        "accounts_validate_parent_insert",
        &[
            "account parent not found",
            "account parent must be a category",
            "account parent must use the same element",
        ],
    ),
    (
        "accounts_validate_parent_update",
        &[
            "account parent not found",
            "account parent must be a category",
            "account parent must use the same element",
        ],
    ),
    (
        "accounts_prevent_cycle_update",
        &["WITH RECURSIVE subtree(id)", "account hierarchy cycle"],
    ),
    (
        "accounts_preserve_child_elements",
        &[
            "child.parent_id = OLD.id",
            "account element must match its children",
        ],
    ),
    (
        "accounts_posting_kind_requires_leaf",
        &[
            "NEW.kind = 'posting'",
            "posting account cannot have children",
        ],
    ),
    (
        "journal_entries",
        &[
            "id INTEGER PRIMARY KEY AUTOINCREMENT",
            "CHECK (id > 0)",
            "entry_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'",
            "BETWEEN 1 AND 9999",
            "BETWEEN 1 AND 12",
            "THEN 29",
            "status IN ('draft', 'posted')",
            "status = 'draft' AND posted_at IS NULL",
            "status = 'posted' AND posted_at IS NOT NULL",
            "revision >= 1",
            "reverses_entry_id INTEGER",
            "corrects_entry_id INTEGER",
            "explanation TEXT",
            "created_at TEXT NOT NULL DEFAULT",
            "reference TEXT CHECK (reference IS NULL OR instr(reference, char(0)) = 0)",
        ],
    ),
    (
        "journal_entries_one_reversal_idx",
        &[
            "CREATE UNIQUE INDEX",
            "ON journal_entries(reverses_entry_id)",
            "WHERE reverses_entry_id IS NOT NULL",
        ],
    ),
    (
        "journal_entries_correction_target_idx",
        &[
            "CREATE INDEX",
            "ON journal_entries(corrects_entry_id, id)",
            "WHERE corrects_entry_id IS NOT NULL",
        ],
    ),
    (
        "journal_lines",
        &[
            "id INTEGER PRIMARY KEY AUTOINCREMENT",
            "CHECK (id > 0)",
            "line_no INTEGER NOT NULL CHECK (line_no > 0)",
            "side IN ('debit', 'credit')",
            "amount_minor > 0",
            "created_at TEXT NOT NULL DEFAULT",
            "UNIQUE (entry_id, line_no)",
            "memo TEXT CHECK (memo IS NULL OR instr(memo, char(0)) = 0)",
        ],
    ),
    (
        "accounts_preserve_journal_classification",
        &[
            "EXISTS (SELECT 1 FROM journal_lines WHERE account_id = OLD.id)",
            "account journal classification is immutable",
        ],
    ),
    (
        "journal_entries_insert_as_draft",
        &[
            "NEW.status <> 'draft'",
            "journal entry must be inserted as a draft",
        ],
    ),
    (
        "journal_entries_preserve_identity",
        &[
            "NEW.id IS NOT OLD.id",
            "journal entry identity is immutable",
        ],
    ),
    (
        "journal_entries_posted_immutable_update",
        &["OLD.status = 'posted'", "posted journal entry is immutable"],
    ),
    (
        "journal_entries_posted_immutable_delete",
        &[
            "OLD.status = 'posted'",
            "posted journal entry cannot be deleted",
        ],
    ),
    (
        "journal_entries_validate_posting",
        &[
            "COUNT(*) FROM journal_lines WHERE entry_id = OLD.id",
            "journal line account must be active and posting",
            "SUM(CASE WHEN side = 'debit' THEN amount_minor ELSE 0 END)",
            "SUM(CASE WHEN side = 'credit' THEN amount_minor ELSE 0 END)",
            "journal entry is not balanced",
        ],
    ),
    (
        "journal_entries_require_initial_revision",
        &[
            "NEW.revision <> 1",
            "journal entry revision must start at one",
        ],
    ),
    (
        "journal_entries_require_next_draft_revision",
        &[
            "UPDATE OF entry_date, reference, explanation, revision",
            "NEW.revision <> OLD.revision + 1",
            "draft journal entry revision must increase by one",
        ],
    ),
    (
        "journal_entries_validate_posted_revision",
        &[
            "NEW.revision <> OLD.revision + 1",
            "posted journal entry revision is invalid",
        ],
    ),
    (
        "journal_entries_preserve_relationship",
        &[
            "NEW.reverses_entry_id IS NOT OLD.reverses_entry_id",
            "NEW.corrects_entry_id IS NOT OLD.corrects_entry_id",
            "OLD.corrects_entry_id IS NOT NULL",
            "journal entry relationship is immutable",
            "journal entry explanation contains invalid text",
        ],
    ),
    (
        "journal_entries_validate_relationship_insert",
        &[
            "cannot be both a reversal and a correction",
            "correcting journal entry requires an explanation",
            "explanation contains invalid text",
        ],
    ),
    (
        "journal_entries_validate_relationship_posting",
        &[
            "related journal entry must exist and be posted",
            "a reversal cannot reverse another reversal",
            "reversal date cannot precede original entry date",
            "reversal lines must exactly exchange debit and credit sides",
            "correcting journal entry requires an explanation",
        ],
    ),
    (
        "journal_lines_insert_only_into_draft",
        &[
            "entry.status = 'draft'",
            "journal line entry must be a draft",
        ],
    ),
    (
        "journal_lines_validate_account_insert",
        &[
            "account.kind = 'posting'",
            "account.is_active = 1",
            "journal line account must be active and posting",
        ],
    ),
    (
        "journal_lines_update_only_in_draft",
        &[
            "entry.status = 'posted'",
            "posted journal line is immutable",
        ],
    ),
    (
        "journal_lines_preserve_identity",
        &[
            "NEW.entry_id IS NOT OLD.entry_id",
            "journal line identity is immutable",
        ],
    ),
    (
        "journal_lines_validate_account_update",
        &[
            "account.kind = 'posting'",
            "account.is_active = 1",
            "journal line account must be active and posting",
        ],
    ),
    (
        "journal_lines_delete_only_from_draft",
        &[
            "entry.status = 'posted'",
            "posted journal line cannot be deleted",
        ],
    ),
];

#[derive(Debug, Error)]
/// Failure while identifying, opening, migrating, or validating SQLite.
pub enum DatabaseError {
    #[error("failed to connect to SQLite: {0}")]
    Connect(#[source] sqlx::Error),
    #[error("failed to migrate SQLite: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("failed to query SQLite: {0}")]
    Query(#[source] sqlx::Error),
    #[error(
        "refusing to migrate an unrecognized SQLite database (application_id {application_id}, schema objects {schema_object_count})"
    )]
    UnrecognizedDatabase {
        application_id: i64,
        schema_object_count: i64,
    },
    #[error("failed to apply owner-only database permissions: {0}")]
    Permissions(#[source] std::io::Error),
    #[error("refusing unsafe SQLite sidecar path: {0}")]
    UnsafeSidecar(PathBuf),
    #[error("SQLite integrity verification failed: {0}")]
    Integrity(String),
}

#[derive(Debug, Error)]
/// Failure while creating and publishing a verified SQLite snapshot backup.
pub enum BackupError {
    #[error("backup destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("backup destination has no parent directory: {0}")]
    MissingParent(PathBuf),
    #[error("backup path is not valid UTF-8: {0}")]
    InvalidPath(PathBuf),
    #[error("backup I/O failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("SQLite failed to create the backup snapshot: {0}")]
    Snapshot(#[source] sqlx::Error),
    #[error("backup snapshot verification failed: {0}")]
    Verification(#[source] DatabaseError),
}

#[derive(Debug, Error)]
#[error("backup cleanup warning while {operation} {path}: {source}")]
/// Non-fatal cleanup failure after a verified snapshot has been published.
pub struct BackupCleanupWarning {
    pub operation: &'static str,
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

#[derive(Debug)]
/// Result details for a successfully published and verified snapshot.
pub struct BackupOutcome {
    cleanup_warning: Option<BackupCleanupWarning>,
}

impl BackupOutcome {
    /// Returns a warning when private temporary work could not be fully removed.
    ///
    /// The destination is already a verified published snapshot in this case;
    /// callers must report success and may surface the warning separately.
    pub fn cleanup_warning(&self) -> Option<&BackupCleanupWarning> {
        self.cleanup_warning.as_ref()
    }

    pub fn into_cleanup_warning(self) -> Option<BackupCleanupWarning> {
        self.cleanup_warning
    }
}

/// Opens an isolated single-connection database for tests and transient work.
pub async fn open_memory_database() -> Result<SqlitePool, DatabaseError> {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(DatabaseError::Connect)?;

    migrate_and_verify(pool).await
}

/// Safely identifies, migrates, and opens a file-backed application database.
pub async fn open_database(path: impl AsRef<Path>) -> Result<SqlitePool, DatabaseError> {
    let path = path.as_ref();
    create_private_database_file_if_missing(path)?;
    preflight_file_database(path).await?;
    harden_database_permissions(path)?;

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .map_err(DatabaseError::Connect)?;

    let pool = migrate_and_verify(pool).await?;
    if let Err(error) = harden_database_permissions(path) {
        pool.close().await;
        return Err(error);
    }

    Ok(pool)
}

fn create_private_database_file_if_missing(path: &Path) -> Result<(), DatabaseError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);

    match options.open(path) {
        Ok(file) => {
            #[cfg(unix)]
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(DatabaseError::Permissions)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(DatabaseError::Permissions(error)),
    }
}

/// Verifies an existing standalone database without migrating or modifying it.
///
/// This is suitable for inspecting a backup or future restore candidate. It
/// opens read-only, does not enable WAL, and always closes its private pool.
pub async fn verify_database_snapshot(path: impl AsRef<Path>) -> Result<(), DatabaseError> {
    let pool = connect_read_only_database(path.as_ref()).await?;

    let result = async {
        verify_migration_target(&pool).await?;
        verify_database_integrity(&pool).await
    }
    .await;
    pool.close().await;
    result
}

/// Validate an exact recognized migration prefix without mutating the source.
/// Reconstructing its schema catches removed triggers and unexpected objects
/// before a later migration could mask the change.
pub async fn pending_schema_migrations(path: impl AsRef<Path>) -> Result<bool, DatabaseError> {
    let pool = connect_read_only_database(path.as_ref()).await?;
    let result = async {
        verify_migration_target(&pool).await?;
        let applied: Vec<(i64, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
        ).fetch_all(&pool).await.map_err(DatabaseError::Query)?;
        if applied.is_empty() || applied.len() > MIGRATOR.migrations.len() {
            return Err(DatabaseError::Integrity("unrecognized migration prefix".into()));
        }
        for ((version, success, checksum), expected) in applied.iter().zip(MIGRATOR.migrations.iter()) {
            if *version != expected.version || *success != 1 || checksum.as_slice() != expected.checksum.as_ref() {
                return Err(DatabaseError::Integrity("migration prefix checksum or success mismatch".into()));
            }
        }
        if applied.len() == MIGRATOR.migrations.len() { return Ok(false); }
        let expected = SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.map_err(DatabaseError::Connect)?;
        let prefix = Migrator {
            migrations: std::borrow::Cow::Owned(MIGRATOR.migrations[..applied.len()].to_vec()),
            ..Migrator::DEFAULT
        };
        let check = async {
            prefix.run(&expected).await.map_err(DatabaseError::Migrate)?;
            let sql = "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name";
            let actual: Vec<(String, String, String, Option<String>)> = sqlx::query_as(sql).fetch_all(&pool).await.map_err(DatabaseError::Query)?;
            let reference: Vec<(String, String, String, Option<String>)> = sqlx::query_as(sql).fetch_all(&expected).await.map_err(DatabaseError::Query)?;
            if actual != reference { return Err(DatabaseError::Integrity("legacy schema differs from its migration prefix".into())); }
            let integrity: String = sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&pool).await.map_err(DatabaseError::Query)?;
            let foreign: Vec<sqlx::sqlite::SqliteRow> = sqlx::query("PRAGMA foreign_key_check").fetch_all(&pool).await.map_err(DatabaseError::Query)?;
            if integrity != "ok" || !foreign.is_empty() { return Err(DatabaseError::Integrity("legacy snapshot integrity failed".into())); }
            Ok(true)
        }.await;
        expected.close().await;
        check
    }.await;
    pool.close().await;
    result
}

/// Creates a verified, owner-only snapshot without replacing an existing file.
///
/// SQLite writes into a private temporary directory beside the destination.
/// After a read-only integrity audit succeeds, a hard link publishes the file
/// atomically and fails if the destination appeared concurrently. The caller
/// must choose an existing parent directory; this primitive never creates one.
pub async fn backup_database(
    pool: &SqlitePool,
    destination: impl AsRef<Path>,
) -> Result<BackupOutcome, BackupError> {
    let destination = destination.as_ref();
    match std::fs::symlink_metadata(destination) {
        Ok(_) => return Err(BackupError::DestinationExists(destination.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(BackupError::Io {
                operation: "checking",
                path: destination.to_path_buf(),
                source,
            });
        }
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(BackupError::MissingParent(parent.to_path_buf()));
    }

    let temporary_directory = create_private_backup_directory(parent)?;
    let temporary_file = temporary_directory.join("snapshot.sqlite3");
    let result = create_and_publish_backup(pool, &temporary_file, destination).await;
    let cleanup_result = cleanup_backup_temporary_path(&temporary_file, &temporary_directory);

    resolve_backup_completion(result, cleanup_result)
}

fn resolve_backup_completion(
    publish_result: Result<(), BackupError>,
    cleanup_result: Result<(), BackupCleanupWarning>,
) -> Result<BackupOutcome, BackupError> {
    match (publish_result, cleanup_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Ok(())) => Ok(BackupOutcome {
            cleanup_warning: None,
        }),
        (Ok(()), Err(warning)) => Ok(BackupOutcome {
            cleanup_warning: Some(warning),
        }),
    }
}

#[cfg(unix)]
fn harden_database_permissions(path: &Path) -> Result<(), DatabaseError> {
    let mut sidecars = Vec::new();
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sqlite_sidecar_path(path, suffix);
        match std::fs::symlink_metadata(&sidecar) {
            Ok(metadata) if metadata.file_type().is_file() => sidecars.push(sidecar),
            Ok(_) => return Err(DatabaseError::UnsafeSidecar(sidecar)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(DatabaseError::Permissions(error)),
        }
    }

    // Validate every existing sidecar before changing any mode, so a refused
    // open cannot partially harden the database while leaving an unsafe path.
    let permissions = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, permissions.clone()).map_err(DatabaseError::Permissions)?;
    for sidecar in sidecars {
        std::fs::set_permissions(sidecar, permissions.clone())
            .map_err(DatabaseError::Permissions)?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn harden_database_permissions(_path: &Path) -> Result<(), DatabaseError> {
    Ok(())
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_owned();
    path.push(suffix);
    PathBuf::from(path)
}

fn create_private_backup_directory(parent: &Path) -> Result<PathBuf, BackupError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| BackupError::Io {
            operation: "reading the system clock for",
            path: parent.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .as_nanos();

    for attempt in 0..100_u32 {
        let path = parent.join(format!(
            ".kAccounting-Extra-01-backup-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(BackupError::Io {
                    operation: "creating private temporary directory",
                    path,
                    source,
                });
            }
        }
    }

    Err(BackupError::Io {
        operation: "creating a unique private temporary directory in",
        path: parent.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "100 candidate names already existed",
        ),
    })
}

async fn create_and_publish_backup(
    pool: &SqlitePool,
    temporary_file: &Path,
    destination: &Path,
) -> Result<(), BackupError> {
    let temporary_file_text = temporary_file
        .to_str()
        .ok_or_else(|| BackupError::InvalidPath(temporary_file.to_path_buf()))?;
    sqlx::query("VACUUM INTO ?")
        .bind(temporary_file_text)
        .execute(pool)
        .await
        .map_err(BackupError::Snapshot)?;
    if !temporary_file.is_file() {
        return Err(BackupError::Io {
            operation: "locating SQLite snapshot after VACUUM INTO",
            path: temporary_file.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "SQLite reported success without creating the requested file",
            ),
        });
    }
    harden_database_permissions(temporary_file).map_err(BackupError::Verification)?;
    verify_backup_file(temporary_file).await?;
    std::fs::hard_link(temporary_file, destination).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            BackupError::DestinationExists(destination.to_path_buf())
        } else {
            BackupError::Io {
                operation: "publishing",
                path: destination.to_path_buf(),
                source,
            }
        }
    })?;
    Ok(())
}

async fn verify_backup_file(path: &Path) -> Result<(), BackupError> {
    verify_database_snapshot(path)
        .await
        .map_err(BackupError::Verification)
}

fn cleanup_backup_temporary_path(
    file: &Path,
    directory: &Path,
) -> Result<(), BackupCleanupWarning> {
    if file.exists() {
        std::fs::remove_file(file).map_err(|source| BackupCleanupWarning {
            operation: "removing temporary backup file",
            path: file.to_path_buf(),
            source,
        })?;
    }
    std::fs::remove_dir(directory).map_err(|source| BackupCleanupWarning {
        operation: "removing temporary backup directory",
        path: directory.to_path_buf(),
        source,
    })
}

async fn preflight_file_database(path: &Path) -> Result<(), DatabaseError> {
    let pool = connect_read_only_database(path).await?;

    let result = verify_migration_target(&pool).await;
    pool.close().await;
    result
}

/// Opens an existing database without granting SQLite a writable handle.
///
/// Besides preventing explicit writes, read-only mode prevents identity
/// preflight from performing hot-journal recovery on a file that has not yet
/// been identified as belonging to this application.
pub(crate) async fn connect_read_only_database(path: &Path) -> Result<SqlitePool, DatabaseError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(DatabaseError::Connect)
}

async fn migrate_and_verify(pool: SqlitePool) -> Result<SqlitePool, DatabaseError> {
    if let Err(error) = verify_and_claim_migration_target(&pool).await {
        pool.close().await;
        return Err(error);
    }

    if let Err(error) = MIGRATOR.run(&pool).await {
        pool.close().await;
        return Err(DatabaseError::Migrate(error));
    }

    if let Err(error) = verify_database_integrity(&pool).await {
        pool.close().await;
        return Err(error);
    }

    Ok(pool)
}

async fn verify_migration_target(pool: &SqlitePool) -> Result<(), DatabaseError> {
    let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(pool)
        .await
        .map_err(DatabaseError::Query)?;
    // SQLite reserves the literal `sqlite_` prefix. GLOB keeps `_` literal;
    // a LIKE pattern would accidentally ignore names such as `sqlitex_data`.
    let schema_object_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(pool)
            .await
            .map_err(DatabaseError::Query)?;

    validate_migration_target(application_id, schema_object_count)
}

fn validate_migration_target(
    application_id: i64,
    schema_object_count: i64,
) -> Result<(), DatabaseError> {
    if application_id == APPLICATION_ID || (application_id == 0 && schema_object_count == 0) {
        Ok(())
    } else {
        Err(DatabaseError::UnrecognizedDatabase {
            application_id,
            schema_object_count,
        })
    }
}

/// Repeats identity validation on the writable pool and claims an empty file.
///
/// Keeping the validation and claim in one transaction prevents another
/// connection from turning an accepted empty file into an unrecognized schema
/// between those two steps. The earlier read-only preflight remains necessary
/// so an unrelated file never receives a writable SQLite handle.
async fn verify_and_claim_migration_target(pool: &SqlitePool) -> Result<(), DatabaseError> {
    // Reserve the writer slot before reading identity. A deferred transaction
    // can observe an empty snapshot, lose the migration race, and then fail its
    // write upgrade with SQLITE_BUSY_SNAPSHOT, which busy_timeout cannot retry.
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DatabaseError::Query)?;
    let result = verify_and_claim_migration_target_transaction(&mut transaction).await;
    match result {
        Ok(()) => transaction.commit().await.map_err(DatabaseError::Query),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn verify_and_claim_migration_target_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), DatabaseError> {
    let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&mut **transaction)
        .await
        .map_err(DatabaseError::Query)?;
    let schema_object_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(&mut **transaction)
            .await
            .map_err(DatabaseError::Query)?;

    validate_migration_target(application_id, schema_object_count)?;

    if application_id == 0 {
        sqlx::query(CLAIM_APPLICATION_ID_SQL)
            .execute(&mut **transaction)
            .await
            .map_err(DatabaseError::Query)?;

        let claimed_id: i64 = sqlx::query_scalar("PRAGMA application_id")
            .fetch_one(&mut **transaction)
            .await
            .map_err(DatabaseError::Query)?;
        if claimed_id != APPLICATION_ID {
            return Err(DatabaseError::Integrity(format!(
                "failed to claim empty SQLite database: application_id is {claimed_id}"
            )));
        }
    }

    Ok(())
}

/// Runs physical, relational, schema, and account-tree integrity checks.
pub async fn verify_database_integrity(pool: &SqlitePool) -> Result<(), DatabaseError> {
    let mut transaction = pool.begin().await.map_err(DatabaseError::Query)?;
    let result = verify_database_integrity_transaction(&mut transaction).await;
    match result {
        Ok(()) => transaction.commit().await.map_err(DatabaseError::Query),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn verify_database_integrity_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), DatabaseError> {
    let integrity_check: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&mut **transaction)
        .await
        .map_err(DatabaseError::Query)?;

    if integrity_check.as_slice() != ["ok"] {
        let details = if integrity_check.is_empty() {
            "integrity_check returned no result".to_owned()
        } else {
            integrity_check.join("; ")
        };
        return Err(DatabaseError::Integrity(details));
    }

    let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&mut **transaction)
        .await
        .map_err(DatabaseError::Query)?;
    if application_id != APPLICATION_ID {
        return Err(DatabaseError::Integrity(format!(
            "unexpected SQLite application_id {application_id}; expected {APPLICATION_ID}"
        )));
    }

    let foreign_key_violations: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&mut **transaction)
            .await
            .map_err(DatabaseError::Query)?;

    if let Some((table, row_id, parent, foreign_key_index)) = foreign_key_violations.first() {
        return Err(DatabaseError::Integrity(format!(
            "foreign key violation in table {table}, row {}, referencing {parent}, constraint {foreign_key_index}",
            row_id.map_or_else(|| "without rowid".to_owned(), |value| value.to_string())
        )));
    }

    let stored_schema_objects: Vec<(String, String, String)> =
        sqlx::query_as("SELECT type, name, tbl_name FROM sqlite_schema")
            .fetch_all(&mut **transaction)
            .await
            .map_err(DatabaseError::Query)?;
    let schema_object_keys: HashSet<(&str, &str)> = stored_schema_objects
        .iter()
        .map(|(object_type, name, _)| (object_type.as_str(), name.as_str()))
        .collect();

    for &(object_type, name) in REQUIRED_SCHEMA_OBJECTS {
        if !schema_object_keys.contains(&(object_type, name)) {
            return Err(DatabaseError::Integrity(format!(
                "required schema object is missing: {object_type} {name}"
            )));
        }
    }

    let migration_columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('_sqlx_migrations') ORDER BY cid",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if !columns_match(&migration_columns, REQUIRED_MIGRATION_COLUMNS) {
        return Err(DatabaseError::Integrity(
            "_sqlx_migrations table columns do not match SQLx 0.9".to_owned(),
        ));
    }

    verify_migration_history(transaction).await?;

    let accounts_is_strict: Option<i64> = sqlx::query_scalar(
        "SELECT strict FROM pragma_table_list('accounts') WHERE name = 'accounts'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if accounts_is_strict != Some(1) {
        return Err(DatabaseError::Integrity(
            "accounts table must exist in SQLite STRICT mode".to_owned(),
        ));
    }

    let account_columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('accounts') ORDER BY cid",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let columns_match = account_columns.len() == REQUIRED_ACCOUNT_COLUMNS.len()
        && account_columns.iter().zip(REQUIRED_ACCOUNT_COLUMNS).all(
            |((actual_name, actual_type, actual_not_null, actual_primary_key), expected)| {
                (
                    actual_name.as_str(),
                    actual_type.as_str(),
                    *actual_not_null,
                    *actual_primary_key,
                ) == *expected
            },
        );
    if !columns_match {
        return Err(DatabaseError::Integrity(
            "accounts table columns do not match migration 1".to_owned(),
        ));
    }

    let parent_index_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_index_info('accounts_parent_id_idx') ORDER BY seqno",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if parent_index_columns.as_slice() != ["parent_id"] {
        return Err(DatabaseError::Integrity(
            "accounts_parent_id_idx columns do not match migration 1".to_owned(),
        ));
    }

    let eligibility_index_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_index_info('accounts_element_kind_active_idx') ORDER BY seqno",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if eligibility_index_columns.as_slice() != ["element", "kind", "is_active"] {
        return Err(DatabaseError::Integrity(
            "accounts_element_kind_active_idx columns do not match migration 1".to_owned(),
        ));
    }

    let account_foreign_keys: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT "table", "from", "to", on_update, on_delete
        FROM pragma_foreign_key_list('accounts')
        ORDER BY id, seq
        "#,
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let expected_parent_foreign_key = (
        "accounts".to_owned(),
        "parent_id".to_owned(),
        "id".to_owned(),
        "RESTRICT".to_owned(),
        "RESTRICT".to_owned(),
    );
    if account_foreign_keys.as_slice() != [expected_parent_foreign_key] {
        return Err(DatabaseError::Integrity(
            "accounts parent foreign key does not match migration 1".to_owned(),
        ));
    }

    verify_journal_schema(transaction).await?;
    verify_attachment_schema(transaction).await?;

    for &(object_type, name) in REQUIRED_SCHEMA_OBJECTS {
        if !matches!(object_type, "index" | "trigger") {
            continue;
        }
        let expected_owner = if name.starts_with("accounts_") {
            "accounts"
        } else if name.starts_with("journal_entries_") {
            "journal_entries"
        } else if name.starts_with("journal_lines_") {
            "journal_lines"
        } else if name.starts_with("attachments_") {
            "attachments"
        } else {
            return Err(DatabaseError::Integrity(format!(
                "required schema object {name} has no owner-table rule"
            )));
        };
        let actual_owner = stored_schema_objects
            .iter()
            .find(|(stored_type, stored_name, _)| stored_type == object_type && stored_name == name)
            .map(|(_, _, owner)| owner.as_str());
        if actual_owner != Some(expected_owner) {
            return Err(DatabaseError::Integrity(format!(
                "required {object_type} {name} belongs to table {}, expected {expected_owner}",
                actual_owner.unwrap_or("<missing>")
            )));
        }
    }

    for &(name, fragments) in REQUIRED_SCHEMA_SQL_FRAGMENTS {
        let definition: Option<String> =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name = ?")
                .bind(name)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(DatabaseError::Query)?;
        let definition = definition.ok_or_else(|| {
            DatabaseError::Integrity(format!("required schema SQL is missing for {name}"))
        })?;
        if let Some(fragment) = fragments
            .iter()
            .find(|fragment| !definition.contains(**fragment))
        {
            return Err(DatabaseError::Integrity(format!(
                "schema definition for {name} is missing required SQL fragment: {fragment}"
            )));
        }
    }

    let invalid_parent_relationship: Option<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT child.id, parent.id
        FROM accounts AS child
        JOIN accounts AS parent ON parent.id = child.parent_id
        WHERE parent.kind <> 'category'
           OR parent.element <> child.element
           OR (child.is_active = 1 AND parent.is_active = 0)
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some((child_id, parent_id)) = invalid_parent_relationship {
        return Err(DatabaseError::Integrity(format!(
            "invalid account hierarchy relationship: child {child_id}, parent {parent_id}"
        )));
    }

    let hierarchy_cycle: Option<i64> = sqlx::query_scalar(
        r#"
        WITH RECURSIVE ancestry(start_id, current_id, path, cycle) AS (
            SELECT id, parent_id, printf(',%d,', id), 0
            FROM accounts

            UNION ALL

            SELECT ancestry.start_id,
                   parent.parent_id,
                   ancestry.path || parent.id || ',',
                   instr(ancestry.path, printf(',%d,', parent.id)) > 0
            FROM ancestry
            JOIN accounts AS parent ON parent.id = ancestry.current_id
            WHERE ancestry.current_id IS NOT NULL
              AND ancestry.cycle = 0
        )
        SELECT start_id
        FROM ancestry
        WHERE cycle = 1
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some(account_id) = hierarchy_cycle {
        return Err(DatabaseError::Integrity(format!(
            "account hierarchy cycle detected at account {account_id}"
        )));
    }

    let invalid_relationship: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT entry.id
        FROM journal_entries AS entry
        LEFT JOIN journal_entries AS related
          ON related.id = COALESCE(entry.reverses_entry_id, entry.corrects_entry_id)
        WHERE (entry.reverses_entry_id IS NOT NULL AND entry.corrects_entry_id IS NOT NULL)
           OR (
               (entry.reverses_entry_id IS NOT NULL OR entry.corrects_entry_id IS NOT NULL)
               AND (
                   entry.status <> 'posted'
                   OR related.id IS NULL
                   OR related.status <> 'posted'
                   OR related.id = entry.id
               )
           )
           OR (
               entry.corrects_entry_id IS NOT NULL
               AND (entry.explanation IS NULL OR trim(entry.explanation) = '')
           )
           OR (entry.explanation IS NOT NULL AND instr(entry.explanation, char(0)) <> 0)
           OR (
               entry.reverses_entry_id IS NOT NULL
               AND (
                   related.reverses_entry_id IS NOT NULL
                   OR entry.entry_date < related.entry_date
               )
           )
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some(entry_id) = invalid_relationship {
        return Err(DatabaseError::Integrity(format!(
            "journal entry {entry_id} has an invalid reversal or correction relationship"
        )));
    }

    let malformed_reversal: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT reversal.id
        FROM journal_entries AS reversal
        WHERE reversal.reverses_entry_id IS NOT NULL
          AND (
              (SELECT COUNT(*) FROM journal_lines WHERE entry_id = reversal.id)
                  <> (SELECT COUNT(*) FROM journal_lines WHERE entry_id = reversal.reverses_entry_id)
              OR EXISTS (
                  SELECT 1
                  FROM journal_lines AS reversal_line
                  LEFT JOIN journal_lines AS original_line
                    ON original_line.entry_id = reversal.reverses_entry_id
                   AND original_line.line_no = reversal_line.line_no
                  WHERE reversal_line.entry_id = reversal.id
                    AND (
                        original_line.id IS NULL
                        OR original_line.account_id <> reversal_line.account_id
                        OR original_line.amount_minor <> reversal_line.amount_minor
                        OR original_line.side <> CASE reversal_line.side
                            WHEN 'debit' THEN 'credit'
                            ELSE 'debit'
                        END
                    )
              )
          )
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some(entry_id) = malformed_reversal {
        return Err(DatabaseError::Integrity(format!(
            "reversal journal entry {entry_id} does not exactly exchange original sides"
        )));
    }

    let invalid_posted_entry: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT entry.id
        FROM journal_entries AS entry
        LEFT JOIN journal_lines AS line ON line.entry_id = entry.id
        WHERE entry.status = 'posted'
        GROUP BY entry.id
        HAVING COUNT(line.id) < 2
            OR SUM(CASE WHEN line.side = 'debit' THEN line.amount_minor ELSE 0 END)
               <> SUM(CASE WHEN line.side = 'credit' THEN line.amount_minor ELSE 0 END)
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some(entry_id) = invalid_posted_entry {
        return Err(DatabaseError::Integrity(format!(
            "posted journal entry {entry_id} is unbalanced or has fewer than two lines"
        )));
    }

    let invalid_posted_account: Option<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT entry.id, account.id
        FROM journal_entries AS entry
        JOIN journal_lines AS line ON line.entry_id = entry.id
        JOIN accounts AS account ON account.id = line.account_id
        WHERE entry.status = 'posted'
          AND account.kind <> 'posting'
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    if let Some((entry_id, account_id)) = invalid_posted_account {
        return Err(DatabaseError::Integrity(format!(
            "posted journal entry {entry_id} refers to non-posting account {account_id}"
        )));
    }

    if let Some((object_type, name, owner)) =
        stored_schema_objects.iter().find(|(object_type, name, _)| {
            !name.starts_with("sqlite_")
                && !REQUIRED_SCHEMA_OBJECTS.contains(&(object_type.as_str(), name.as_str()))
        })
    {
        return Err(DatabaseError::Integrity(format!(
            "unexpected schema object: {object_type} {name} on table {owner}"
        )));
    }

    Ok(())
}

async fn verify_migration_history(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), DatabaseError> {
    let applied: Vec<(i64, i64, Vec<u8>)> = sqlx::query_as(
        r#"
        SELECT version, success, checksum
        FROM _sqlx_migrations
        ORDER BY version
        "#,
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;

    if applied.len() != MIGRATOR.migrations.len() {
        return Err(DatabaseError::Integrity(format!(
            "migration history has {} rows; this build requires {}",
            applied.len(),
            MIGRATOR.migrations.len()
        )));
    }

    for ((version, success, checksum), expected) in applied.iter().zip(MIGRATOR.migrations.iter()) {
        if *version != expected.version {
            return Err(DatabaseError::Integrity(format!(
                "migration history contains version {version}; expected {}",
                expected.version
            )));
        }
        if *success != 1 {
            return Err(DatabaseError::Integrity(format!(
                "migration {version} has invalid success marker {success}; expected 1"
            )));
        }
        if checksum.as_slice() != expected.checksum.as_ref() {
            return Err(DatabaseError::Integrity(format!(
                "migration {version} checksum does not match this build"
            )));
        }
    }

    Ok(())
}

async fn verify_attachment_schema(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), DatabaseError> {
    let strict: Option<i64> = sqlx::query_scalar(
        "SELECT strict FROM pragma_table_list('attachments') WHERE name = 'attachments'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('attachments') ORDER BY cid",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let expected = [
        ("id", "TEXT", 1, 1),
        ("entry_id", "INTEGER", 1, 0),
        ("display_name", "TEXT", 1, 0),
        ("plaintext_sha256", "TEXT", 1, 0),
        ("byte_length", "INTEGER", 1, 0),
        ("ciphertext_sha256", "TEXT", 1, 0),
        ("ciphertext_byte_length", "INTEGER", 1, 0),
        ("created_at", "TEXT", 1, 0),
    ];
    if strict != Some(1) || !columns_match(&columns, &expected) {
        return Err(DatabaseError::Integrity(
            "attachments schema does not match migration 7".to_owned(),
        ));
    }
    let keys: Vec<(String, String, String, String, String)> = sqlx::query_as("SELECT \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list('attachments')")
        .fetch_all(&mut **transaction).await.map_err(DatabaseError::Query)?;
    if keys
        != vec![(
            "journal_entries".into(),
            "entry_id".into(),
            "id".into(),
            "RESTRICT".into(),
            "CASCADE".into(),
        )]
    {
        return Err(DatabaseError::Integrity(
            "attachment owner foreign key does not match migration 7".to_owned(),
        ));
    }
    let total: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(byte_length), 0) FROM attachments")
        .fetch_one(&mut **transaction)
        .await
        .map_err(DatabaseError::Query)?;
    if total > crate::attachments::MAX_BOOK_ATTACHMENT_BYTES as i64 {
        return Err(DatabaseError::Integrity(
            "attachment book quota exceeded".to_owned(),
        ));
    }
    Ok(())
}

async fn verify_journal_schema(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), DatabaseError> {
    for (table, expected_columns) in [
        ("journal_entries", REQUIRED_JOURNAL_ENTRY_COLUMNS),
        ("journal_lines", REQUIRED_JOURNAL_LINE_COLUMNS),
    ] {
        let is_strict: Option<i64> =
            sqlx::query_scalar("SELECT strict FROM pragma_table_list(?) WHERE name = ?")
                .bind(table)
                .bind(table)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(DatabaseError::Query)?;
        if is_strict != Some(1) {
            return Err(DatabaseError::Integrity(format!(
                "{table} table must exist in SQLite STRICT mode"
            )));
        }

        let columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT name, type, \"notnull\", pk FROM pragma_table_info(?) ORDER BY cid",
        )
        .bind(table)
        .fetch_all(&mut **transaction)
        .await
        .map_err(DatabaseError::Query)?;
        if !columns_match(&columns, expected_columns) {
            return Err(DatabaseError::Integrity(format!(
                "{table} table columns do not match required migrations"
            )));
        }
    }

    for (index, expected_columns) in [
        (
            "journal_entries_status_date_idx",
            &["status", "entry_date", "id"][..],
        ),
        (
            "journal_entries_one_reversal_idx",
            &["reverses_entry_id"][..],
        ),
        (
            "journal_entries_correction_target_idx",
            &["corrects_entry_id", "id"][..],
        ),
        ("journal_lines_entry_id_idx", &["entry_id", "line_no"][..]),
        (
            "journal_lines_account_id_idx",
            &["account_id", "entry_id"][..],
        ),
    ] {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                .bind(index)
                .fetch_all(&mut **transaction)
                .await
                .map_err(DatabaseError::Query)?;
        if columns.iter().map(String::as_str).collect::<Vec<_>>() != expected_columns {
            return Err(DatabaseError::Integrity(format!(
                "{index} columns do not match required migrations"
            )));
        }
    }

    let entry_foreign_keys: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT "table", "from", "to", on_update, on_delete
        FROM pragma_foreign_key_list('journal_entries')
        ORDER BY "from"
        "#,
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let expected_entry_foreign_keys = [
        (
            "journal_entries".to_owned(),
            "corrects_entry_id".to_owned(),
            "id".to_owned(),
            "RESTRICT".to_owned(),
            "RESTRICT".to_owned(),
        ),
        (
            "journal_entries".to_owned(),
            "reverses_entry_id".to_owned(),
            "id".to_owned(),
            "RESTRICT".to_owned(),
            "RESTRICT".to_owned(),
        ),
    ];
    if entry_foreign_keys.as_slice() != expected_entry_foreign_keys {
        return Err(DatabaseError::Integrity(
            "journal entry relationship foreign keys do not match migration 4".to_owned(),
        ));
    }

    let line_foreign_keys: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT "table", "from", "to", on_update, on_delete
        FROM pragma_foreign_key_list('journal_lines')
        ORDER BY "from"
        "#,
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(DatabaseError::Query)?;
    let expected_foreign_keys = [
        (
            "accounts".to_owned(),
            "account_id".to_owned(),
            "id".to_owned(),
            "RESTRICT".to_owned(),
            "RESTRICT".to_owned(),
        ),
        (
            "journal_entries".to_owned(),
            "entry_id".to_owned(),
            "id".to_owned(),
            "RESTRICT".to_owned(),
            "CASCADE".to_owned(),
        ),
    ];
    if line_foreign_keys.as_slice() != expected_foreign_keys {
        return Err(DatabaseError::Integrity(
            "journal line foreign keys do not match migration 2".to_owned(),
        ));
    }

    Ok(())
}

fn columns_match(
    actual: &[(String, String, i64, i64)],
    expected: &[(&str, &str, i64, i64)],
) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(
            |((actual_name, actual_type, actual_not_null, actual_primary_key), expected)| {
                (
                    actual_name.as_str(),
                    actual_type.as_str(),
                    *actual_not_null,
                    *actual_primary_key,
                ) == *expected
            },
        )
}

#[cfg(test)]
mod tests {
    use super::{
        APPLICATION_ID, BackupCleanupWarning, CLAIM_APPLICATION_ID_SQL, DatabaseError, MIGRATOR,
        connect_read_only_database, create_private_database_file_if_missing, migrate_and_verify,
        open_memory_database, resolve_backup_completion, verify_database_integrity,
    };

    #[cfg(unix)]
    #[test]
    fn newly_reserved_database_file_is_owner_only_without_touching_existing_files() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-private-create-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after Unix epoch")
                .as_nanos()
        ));

        create_private_database_file_if_missing(&path)
            .expect("missing database path should be reserved privately");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let marker = b"existing file must remain untouched";
        std::fs::write(&path, marker).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        create_private_database_file_if_missing(&path)
            .expect("an existing path should be left for identity preflight");
        assert_eq!(std::fs::read(&path).unwrap(), marker);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );

        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn permission_hardening_refuses_sidecar_symlinks_without_touching_their_targets() {
        use super::{harden_database_permissions, sqlite_sidecar_path};
        use std::os::unix::fs::{PermissionsExt, symlink};

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos();
        let database_path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-sidecar-source-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let target_path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-sidecar-target-{}-{nonce}",
            std::process::id()
        ));
        let sidecar_path = sqlite_sidecar_path(&database_path, "-wal");
        std::fs::write(&database_path, b"database marker").unwrap();
        std::fs::write(&target_path, b"unrelated target").unwrap();
        std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target_path, &sidecar_path).unwrap();

        let error = harden_database_permissions(&database_path)
            .expect_err("permission hardening must not follow a sidecar symlink");
        assert!(matches!(
            error,
            DatabaseError::UnsafeSidecar(path) if path == sidecar_path
        ));
        assert_eq!(
            std::fs::metadata(&database_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "all path types must be validated before any permissions change"
        );
        assert_eq!(
            std::fs::metadata(&target_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "sidecar symlink targets must never be chmodded"
        );

        std::fs::remove_file(sidecar_path).unwrap();
        std::fs::remove_file(database_path).unwrap();
        std::fs::remove_file(target_path).unwrap();
    }

    #[test]
    fn static_claim_statement_matches_the_application_id_constant() {
        let claimed_id: i64 = CLAIM_APPLICATION_ID_SQL
            .rsplit_once('=')
            .expect("claim statement should contain an equals sign")
            .1
            .trim()
            .parse()
            .expect("claim statement should end in a numeric application id");

        assert_eq!(claimed_id, APPLICATION_ID);
    }

    #[tokio::test]
    async fn read_only_connections_cannot_create_or_replace_database_content() {
        let path = std::env::temp_dir().join(format!(
            "kAccounting-Extra-01-read-only-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after Unix epoch")
                .as_nanos()
        ));
        let writable = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .expect("disposable database should be created");
        sqlx::query("CREATE TABLE preserved (id INTEGER PRIMARY KEY)")
            .execute(&writable)
            .await
            .expect("test schema should be created");
        writable.close().await;

        let read_only = connect_read_only_database(&path)
            .await
            .expect("existing database should open read-only");
        sqlx::query("CREATE TABLE forbidden (id INTEGER PRIMARY KEY)")
            .execute(&read_only)
            .await
            .expect_err("read-only verification handles must reject schema writes");
        let preserved: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'preserved'",
        )
        .fetch_one(&read_only)
        .await
        .expect("existing schema should remain readable");
        let forbidden: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'forbidden'",
        )
        .fetch_one(&read_only)
        .await
        .expect("schema should remain inspectable after rejected write");
        assert_eq!((preserved, forbidden), (1, 0));
        read_only.close().await;

        std::fs::remove_file(path).expect("disposable database should be removed");
    }

    #[tokio::test]
    async fn migration_audit_requires_the_exact_success_marker() {
        let pool = open_memory_database()
            .await
            .expect("in-memory database should open");
        sqlx::query("UPDATE _sqlx_migrations SET success = 2 WHERE version = 1")
            .execute(&pool)
            .await
            .expect("test should inject a non-canonical success marker");

        let error = verify_database_integrity(&pool)
            .await
            .expect_err("non-canonical migration success markers must fail closed");
        assert!(matches!(
            error,
            DatabaseError::Integrity(details)
                if details.contains("migration 1 has invalid success marker 2; expected 1")
        ));
    }

    #[test]
    fn published_backup_stays_successful_when_private_cleanup_fails() {
        let work_path = std::path::PathBuf::from("private-backup-work");
        let warning = BackupCleanupWarning {
            operation: "removing temporary backup directory",
            path: work_path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected cleanup failure",
            ),
        };

        let outcome = resolve_backup_completion(Ok(()), Err(warning))
            .expect("a published verified backup must remain successful");
        let warning = outcome
            .cleanup_warning()
            .expect("cleanup failure should be returned as a warning");

        assert_eq!(warning.path, work_path);
        assert_eq!(warning.source.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn initial_migration_matches_the_application_id_constant() {
        let migration = include_str!("../migrations/0001_create_accounts.sql");
        let declaration = migration
            .lines()
            .find(|line| line.starts_with("PRAGMA application_id = "))
            .expect("initial migration should declare an application id");
        let migrated_id: i64 = declaration
            .split_once('=')
            .expect("application id declaration should contain an equals sign")
            .1
            .split_once(';')
            .expect("application id declaration should end with a semicolon")
            .0
            .trim()
            .parse()
            .expect("migration application id should be numeric");

        assert_eq!(migrated_id, APPLICATION_ID);
    }

    #[tokio::test]
    async fn later_migrations_preserve_existing_version_two_drafts() {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("legacy in-memory database should open");
        sqlx::query(
            r#"
            CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                success BOOLEAN NOT NULL,
                checksum BLOB NOT NULL,
                execution_time BIGINT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("legacy migration table should be created");

        for migration in MIGRATOR.migrations.iter().take(2) {
            sqlx::raw_sql(migration.sql.as_str())
                .execute(&pool)
                .await
                .expect("legacy migration should apply");
            sqlx::query(
                r#"
                INSERT INTO _sqlx_migrations (
                    version, description, success, checksum, execution_time
                )
                VALUES (?, ?, 1, ?, 0)
                "#,
            )
            .bind(migration.version)
            .bind(migration.description.as_ref())
            .bind(migration.checksum.as_ref())
            .execute(&pool)
            .await
            .expect("legacy migration history should be recorded");
        }

        let account_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO accounts (name, element, kind)
            VALUES ('Existing cash', 'asset', 'posting')
            RETURNING id
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("legacy account should be stored");
        let equity_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO accounts (name, element, kind)
            VALUES ('Existing equity', 'equity', 'posting')
            RETURNING id
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("second legacy account should be stored");
        let entry_id: i64 = sqlx::query_scalar(
            "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-22') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy draft should be stored");
        sqlx::query(
            r#"
            INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
            VALUES (?, 1, ?, 'debit', 100)
            "#,
        )
        .bind(entry_id)
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("legacy draft line should be stored");
        let posted_id: i64 = sqlx::query_scalar(
            "INSERT INTO journal_entries (entry_date) VALUES ('2026-10-21') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy posted header should start as a draft");
        for (line_no, stored_account_id, side) in
            [(1, account_id, "debit"), (2, equity_id, "credit")]
        {
            sqlx::query(
                r#"
                INSERT INTO journal_lines (entry_id, line_no, account_id, side, amount_minor)
                VALUES (?, ?, ?, ?, 500)
                "#,
            )
            .bind(posted_id)
            .bind(line_no)
            .bind(stored_account_id)
            .bind(side)
            .execute(&pool)
            .await
            .expect("legacy posted line should be stored");
        }
        sqlx::query(
            r#"
            UPDATE journal_entries
            SET status = 'posted', posted_at = '2026-10-21T00:00:00.000Z'
            WHERE id = ?
            "#,
        )
        .bind(posted_id)
        .execute(&pool)
        .await
        .expect("version two entry should post without a revision column");

        let migrated = migrate_and_verify(pool)
            .await
            .expect("version two database should upgrade safely");
        let stored: (String, i64, i64) = sqlx::query_as(
            r#"
            SELECT entry.entry_date, entry.revision, COUNT(line.id)
            FROM journal_entries AS entry
            LEFT JOIN journal_lines AS line ON line.entry_id = entry.id
            WHERE entry.id = ?
            GROUP BY entry.id
            "#,
        )
        .bind(entry_id)
        .fetch_one(&migrated)
        .await
        .expect("migrated draft should remain readable");
        assert_eq!(stored, ("2026-10-22".to_owned(), 1, 1));

        let posted_state: (String, i64, Option<i64>, Option<i64>, Option<String>) =
            sqlx::query_as(
                "SELECT status, revision, reverses_entry_id, corrects_entry_id, explanation FROM journal_entries WHERE id = ?",
            )
                .bind(posted_id)
                .fetch_one(&migrated)
                .await
                .expect("legacy posted entry should remain available");
        assert_eq!(posted_state, ("posted".to_owned(), 1, None, None, None));
        let immutable =
            sqlx::query("UPDATE journal_entries SET revision = revision + 1 WHERE id = ?")
                .bind(posted_id)
                .execute(&migrated)
                .await
                .expect_err("migrated posted entry must remain immutable");
        assert!(
            immutable
                .to_string()
                .contains("posted journal entry is immutable")
        );

        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&migrated)
                .await
                .unwrap();
        assert_eq!(
            versions,
            MIGRATOR
                .migrations
                .iter()
                .map(|migration| migration.version)
                .collect::<Vec<_>>()
        );
    }
}
