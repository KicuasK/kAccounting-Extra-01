//! Explicit, fail-closed local application startup. No book or key is created
//! by opening an existing book; all mutation remains behind the origin guard.
use crate::{
    api, book_backup,
    crypto::RootEncryptionKey,
    db,
    keys::{DeviceKeyId, DeviceKeyStore, KeyStoreError, create_device_key, load_device_key},
    storage::{open_regular_file_without_symlink, publish_private_directory_noreplace},
};
use axum::{
    Router,
    body::Body,
    extract::{Path as RoutePath, State},
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

pub const LOCAL_ADDRESS: &str = "127.0.0.1:3000";
const BINDING_FORMAT: &str = "kAccounting-Extra-01.local-backup";

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(
        "runtime paths must be private, regular, outside the source tree, and free of symlinks"
    )]
    UnsafePath,
    #[error("book key binding is missing or invalid; no replacement key was created")]
    InvalidBinding,
    #[error("another process holds this book; it was not opened")]
    BookBusy,
    #[error("unrecognized interrupted backup work requires inspection before startup")]
    RecoveryRequired,
    #[error("runtime I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Key(#[from] KeyStoreError),
    #[error(transparent)]
    Database(#[from] db::DatabaseError),
    #[error(transparent)]
    Backup(#[from] book_backup::BookBackupError),
    #[error(transparent)]
    BackupIdentity(#[from] crate::backup_service::BackupServiceError),
    #[error(transparent)]
    Attachment(#[from] crate::attachments::AttachmentError),
    #[error("new book could not be published without replacing an existing destination")]
    Publish,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    format: String,
    version: u32,
    key_id: String,
}

/// Holds the process lock and encryption key for the entire server lifetime.
/// Call close() on normal shutdown before this object is dropped.
pub struct RuntimeBook {
    pool: SqlitePool,
    root: PathBuf,
    key_id: DeviceKeyId,
    key: Arc<RootEncryptionKey>,
    backup_id: Option<crate::backup_service::BookBackupId>,
    _lock: File,
}
impl RuntimeBook {
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn attachments(&self) -> PathBuf {
        self.root.join("attachments")
    }
    pub fn key_id(&self) -> &DeviceKeyId {
        &self.key_id
    }
    pub fn key(&self) -> Arc<RootEncryptionKey> {
        self.key.clone()
    }
    pub fn backup_id(&self) -> Option<&crate::backup_service::BookBackupId> {
        self.backup_id.as_ref()
    }
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Existing-schema audit happens before writable opening. Legacy schemas
    /// require the separate backup-first upgrade workflow rather than migration.
    pub async fn open_existing(
        path: impl AsRef<Path>,
        store: &impl DeviceKeyStore,
    ) -> Result<Self, RuntimeError> {
        let root = private_directory(path.as_ref())?;
        outside_source(&root)?;
        let lock = lock_book(&root)?;
        let recovered = book_backup::recover_interrupted_local_backup_work(&root)?;
        if !recovered.unrecognized_work.is_empty() {
            return Err(RuntimeError::RecoveryRequired);
        }
        let binding_path = root.join("device-key.json");
        private_regular(&binding_path)?;
        let mut bytes = Vec::new();
        open_regular_file_without_symlink(&binding_path)?
            .take(4097)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(RuntimeError::InvalidBinding);
        }
        let binding: Binding =
            serde_json::from_slice(&bytes).map_err(|_| RuntimeError::InvalidBinding)?;
        if binding.format != BINDING_FORMAT || binding.version != 1 {
            return Err(RuntimeError::InvalidBinding);
        }
        let key_id =
            DeviceKeyId::parse(&binding.key_id).map_err(|_| RuntimeError::InvalidBinding)?;
        let key = Arc::new(load_device_key(store, &key_id)?);
        let backup_id = if root.join("backup-identity.kae").try_exists()? {
            Some(crate::backup_service::load_backup_identity(
                &root, &key_id, store,
            )?)
        } else {
            None
        };
        private_directory(&root.join("attachments"))?;
        let database = root.join("book.sqlite3");
        private_regular(&database)?;
        db::verify_database_snapshot(&database).await?;
        let pool = db::open_database(&database).await?;
        Ok(Self {
            pool,
            root,
            key_id,
            key,
            backup_id,
            _lock: lock,
        })
    }

    /// Explicit initialization creates a new directory only. A failed
    /// publication never replaces a book. A created Keychain key is retained
    /// on failure so recovery never silently deletes key material.
    pub async fn init_new(
        path: impl AsRef<Path>,
        store: &impl DeviceKeyStore,
    ) -> Result<PathBuf, RuntimeError> {
        let requested = path.as_ref();
        let parent = private_directory(requested.parent().ok_or(RuntimeError::UnsafePath)?)?;
        outside_source(&parent)?;
        let name = requested.file_name().ok_or(RuntimeError::UnsafePath)?;
        if name == "." || name == ".." {
            return Err(RuntimeError::UnsafePath);
        }
        let destination = parent.join(name);
        if destination.try_exists()? {
            return Err(RuntimeError::UnsafePath);
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| KeyStoreError::RandomUnavailable)?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let staging = parent.join(format!(".kAccounting-Extra-01-init-{token}"));
        create_private_dir(&staging)?;
        // Failures preserve private work for explicit diagnosis. Nothing is
        // published and no potentially recoverable key or book is deleted.
        let (key_id, _key) = create_device_key(store)?;
        crate::backup_service::initialize_backup_identity(&staging, &key_id, store)?;
        create_private_dir(&staging.join("attachments"))?;
        let binding = Binding {
            format: BINDING_FORMAT.to_owned(),
            version: 1,
            key_id: key_id.to_string(),
        };
        let bytes = serde_json::to_vec(&binding).map_err(|_| RuntimeError::InvalidBinding)?;
        let mut file = private_new_file(&staging.join("device-key.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let pool = db::open_database(staging.join("book.sqlite3")).await?;
        pool.close().await;
        db::verify_database_snapshot(staging.join("book.sqlite3")).await?;
        File::open(&staging)?.sync_all()?;
        publish_private_directory_noreplace(&staging, &destination)
            .map_err(|_| RuntimeError::Publish)?;
        File::open(parent)?.sync_all()?;
        Ok(destination)
    }
}

fn outside_source(path: &Path) -> Result<(), RuntimeError> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RuntimeError::UnsafePath)?;
    if path.starts_with(source) || source.starts_with(path) {
        return Err(RuntimeError::UnsafePath);
    }
    Ok(())
}
fn reject_symlinks(path: &Path) -> Result<(), RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::UnsafePath);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(RuntimeError::UnsafePath);
        }
        current.push(component);
        if std::fs::symlink_metadata(&current)?
            .file_type()
            .is_symlink()
        {
            return Err(RuntimeError::UnsafePath);
        }
    }
    Ok(())
}
fn private_directory(path: &Path) -> Result<PathBuf, RuntimeError> {
    reject_symlinks(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(RuntimeError::UnsafePath);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RuntimeError::UnsafePath);
    }
    Ok(path.canonicalize()?)
}
fn private_regular(path: &Path) -> Result<(), RuntimeError> {
    let file = open_regular_file_without_symlink(path)?;
    #[cfg(unix)]
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(RuntimeError::UnsafePath);
    }
    Ok(())
}
fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}
fn private_new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}
fn lock_book(root: &Path) -> Result<File, RuntimeError> {
    let path = root.join(".runtime.lock");
    let file = match private_new_file(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            private_regular(&path)?;
            OpenOptions::new().read(true).write(true).open(&path)?
        }
        Err(error) => return Err(error.into()),
    };
    file.try_lock().map_err(|_| RuntimeError::BookBusy)?;
    crate::storage::ensure_path_still_names_open_file(&path, &file)?;
    Ok(file)
}

#[derive(Clone)]
struct Assets(Arc<PathBuf>);
/// Serve only the built index and assets; no runtime path can be addressed.
/// This extra outer guard protects static routes as strictly as accounting APIs.
pub fn application_router(
    pool: SqlitePool,
    frontend_dist: impl AsRef<Path>,
) -> Result<Router, RuntimeError> {
    let assets = frontend_dist.as_ref().canonicalize()?;
    if !assets.is_dir() {
        return Err(RuntimeError::UnsafePath);
    }
    let _ = open_regular_file_without_symlink(&assets.join("index.html"))?;
    let static_routes = Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(asset))
        .with_state(Assets(Arc::new(assets)));
    Ok(api::localhost_write_router(pool)
        .merge(static_routes)
        .layer(middleware::from_fn(local_boundary)))
}
async fn index(State(assets): State<Assets>) -> Response {
    asset_response(&assets.0, "index.html")
}
async fn asset(State(assets): State<Assets>, RoutePath(path): RoutePath<String>) -> Response {
    asset_response(&assets.0, &format!("assets/{path}"))
}
fn asset_response(root: &Path, relative: &str) -> Response {
    if relative
        .split('/')
        .any(|part| part.is_empty() || part.starts_with('.') || part.contains('\\'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = root.join(relative);
    let Ok(canonical) = path.canonicalize() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !canonical.starts_with(root) || reject_symlinks(&path).is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let content_type = match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") if relative == "index.html" => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(file) = open_regular_file_without_symlink(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut bytes = Vec::new();
    if file
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 8 * 1024 * 1024
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}
async fn local_boundary(request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    let hosts = headers.get_all(header::HOST);
    let origins = headers.get_all(header::ORIGIN);
    let sites = headers.get_all("sec-fetch-site");
    let read = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let valid_origin =
        origins.iter().count() == 1 && origins.iter().all(|value| value == "http://127.0.0.1:3000");
    if hosts.iter().count() != 1
        || !hosts.iter().all(|value| value == LOCAL_ADDRESS)
        || (origins.iter().count() != 0 && !valid_origin)
        || (!read && !valid_origin)
        || sites.iter().count() > 1
        || !sites
            .iter()
            .all(|value| value == "same-origin" || value == "none")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

/// The address is fixed in code; callers cannot request a public listener.
pub async fn serve(
    book: &RuntimeBook,
    frontend_dist: impl AsRef<Path>,
) -> Result<(), RuntimeError> {
    let router = application_router(book.pool.clone(), frontend_dist)?
        .merge(api::localhost_book_commands_router(book)?);
    let listener = tokio::net::TcpListener::bind(LOCAL_ADDRESS).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("shutdown signal unavailable: {error}");
            }
        })
        .await?;
    book.close().await;
    Ok(())
}
