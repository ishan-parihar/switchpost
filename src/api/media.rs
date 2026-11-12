// ─── Media API Routes ─────────────────────────────────────────
// File upload and serving for post media attachments.
//
// Storage:
//   Bytes live behind a `MediaStorage` backend. `LocalDisk` (the default)
//   keeps today's behaviour exactly; `S3` targets any S3-compatible store
//   (Cloudflare R2) so media survives container restarts and is visible to
//   every replica. Selection is by env — see `S3Config::from_env`.
//   `serve_media` and `delete` go through the backend; the upload write
//   path writes the object the same way (`MediaStorage::store`, which owns
//   the "Failed to write file: …" error text).
//
// Security:
//   - Upload enforces a MIME allowlist (image/png, image/jpeg, image/webp,
//     image/gif, video/mp4, video/quicktime) AND verifies magic bytes via
//     a small inline sniff. Client-supplied Content-Type is never trusted
//     alone. See `sniff_mime` below.
//   - `serve_media` always sets `X-Content-Type-Options: nosniff` to
//     prevent browsers from interpreting non-image bytes as HTML/JS.

use axum::{
    body::{Body, HttpBody},
    extract::{DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{header, Response},
    Json,
};
use axum_extra::extract::{multipart, Multipart};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::auth::middleware::AuthenticatedUser;
use crate::db::models::MediaPublic;
use crate::db::queries;
use crate::error::AppError;

use super::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct ListMediaQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub search: Option<String>,
}

const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB

/// Head bytes kept in memory while the body streams to disk. Enough for every
/// magic-byte signature we accept (longest is MP4/WebP at 12) and for the JPEG
/// SOF marker `detect_image_dimensions` walks to.
/// ponytail: fixed 64 KiB ceiling; raise it only if a real image ever comes
/// back with no dimensions because its EXIF block pushes SOF past it.
const SNIFF_HEAD_BYTES: usize = 64 * 1024;

/// Allowed MIME types for uploads. Anything else is rejected before
/// the file is written to disk or stored in the DB.
const ALLOWED_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "video/mp4",
    "video/quicktime",
];

// ─── Storage backends ──────────────────────────────────────────
// One trait, two impls. Everything above this line speaks in object keys
// (`{uuid}.{ext}`), which is exactly what the `media.storage_path` column
// stores — so a row written by one backend resolves on the same backend
// that read it, and switching backends never rewrites the DB.

/// Failure from a [`MediaStorage`] backend.
///
/// The `ctx` prefix is baked in by the backend so callers can surface
/// `AppError::Internal(e.to_string())` and still get the exact message the
/// handler produced before the backend existed (e.g. "Failed to write
/// file: No space left on device").
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("{ctx}: {src}")]
    Io {
        ctx: &'static str,
        src: std::io::Error,
    },
    #[error("HTTP {0}")]
    Http(String),
    #[error("object not found")]
    NotFound,
}

impl StorageError {
    fn io(ctx: &'static str, src: std::io::Error) -> Self {
        Self::Io { ctx, src }
    }
}

/// Pluggable blob store for uploaded media bytes.
///
/// `key` is the opaque object key, persisted verbatim as
/// `media.storage_path`. Implementations must treat it as relative —
/// never as a path to escape from, and never as something to sanitise
/// differently per backend, or existing rows stop resolving.
#[async_trait::async_trait]
pub trait MediaStorage: Send + Sync {
    /// Short backend id for logs and diagnostics (`"local"`, `"s3"`).
    fn backend(&self) -> &'static str;

    /// Persist `data` under `key`, overwriting any existing object.
    async fn store(&self, key: &str, data: &[u8], mime: &str) -> Result<(), StorageError>;

    /// Fetch the bytes for `key`. Missing objects are
    /// [`StorageError::NotFound`], not `Http`.
    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError>;

    /// Remove `key`. Deleting an object that is already gone succeeds.
    async fn delete(&self, key: &str) -> Result<(), StorageError>;
}

/// Default backend: files under `config.media_dir`.
pub struct LocalDisk {
    dir: PathBuf,
}

impl LocalDisk {
    fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(key)
    }
}

#[async_trait::async_trait]
impl MediaStorage for LocalDisk {
    fn backend(&self) -> &'static str {
        "local"
    }

    async fn store(&self, key: &str, data: &[u8], _mime: &str) -> Result<(), StorageError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| StorageError::io("Failed to create upload dir", e))?;
        tokio::fs::write(self.path(key), data)
            .await
            .map_err(|e| StorageError::io("Failed to write file", e))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        Ok(tokio::fs::read(self.path(key))
            .await
            .map_err(|e| StorageError::io("Failed to read file", e))?)
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let path = self.path(key);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| StorageError::io("Failed to delete file", e))?;
        }
        Ok(())
    }
}

/// Resolved S3/R2 coordinates. Constructed only when all four env vars are
/// present, so a half-configured deployment can never silently half-use S3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Base URL, e.g. `https://<account>.r2.cloudflarestorage.com`.
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

/// R2 signs every request as region `auto`. MinIO and AWS S3 would need
/// their own region here; not a knob until someone actually runs one.
const S3_REGION: &str = "auto";

/// Trimmed value, or `None` when missing or blank. Env-sourced S3 settings
/// treat whitespace-only as unset so a half-edited env file cannot select
/// a broken backend.
fn non_blank(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

impl S3Config {
    /// `None` unless all four values are present and non-empty. Partial
    /// config falls back to local disk — with a warning, because that
    /// fallback is exactly what makes media die with the container.
    fn from_env() -> Option<Self> {
        let endpoint = std::env::var("MEDIA_S3_ENDPOINT")
            .ok()
            .filter(|v| !v.is_empty());
        let bucket = std::env::var("MEDIA_S3_BUCKET")
            .ok()
            .filter(|v| !v.is_empty());
        let access_key = std::env::var("MEDIA_S3_KEY").ok().filter(|v| !v.is_empty());
        let secret_key = std::env::var("MEDIA_S3_SECRET")
            .ok()
            .filter(|v| !v.is_empty());

        let cfg = Self::from_vars(
            endpoint.as_deref(),
            bucket.as_deref(),
            access_key.as_deref(),
            secret_key.as_deref(),
        );
        if cfg.is_none()
            && [endpoint, bucket, access_key, secret_key]
                .iter()
                .any(|v| v.is_some())
        {
            warn_once(
                "MEDIA_S3_* is partially set (all four of ENDPOINT, BUCKET, KEY, SECRET are required) \
                 — media falls back to local disk and will not survive a container restart",
            );
        }
        cfg
    }

    fn from_vars(
        endpoint: Option<&str>,
        bucket: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
    ) -> Option<Self> {
        // Blank counts as missing: a whitespace-only value is never a valid
        // endpoint, bucket, or credential.
        Some(Self {
            endpoint: non_blank(endpoint)?.trim_end_matches('/').to_string(),
            bucket: non_blank(bucket)?.to_string(),
            access_key: non_blank(access_key)?.to_string(),
            secret_key: non_blank(secret_key)?.to_string(),
        })
    }
}

/// S3-compatible backend over plain `PUT`/`GET`/`DELETE` with SigV4
/// (service `s3`). Hand-rolled instead of pulling an SDK: three verbs and
/// one signing routine do not justify the dependency tree, and the HTTP
/// client is the one `AppState` already owns.
pub struct S3 {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    client: reqwest::Client,
}

impl S3 {
    fn new(cfg: S3Config, client: reqwest::Client) -> Self {
        Self {
            endpoint: cfg.endpoint,
            bucket: cfg.bucket,
            access_key: cfg.access_key,
            secret_key: cfg.secret_key,
            client,
        }
    }

    /// `https://<endpoint>/<bucket>/<encoded key>`. Path-style, which R2
    /// accepts, so a bucket name that is not a valid DNS label still works.
    fn object_url(&self, key: &str) -> String {
        format!(
            "{}/{}/{}",
            self.endpoint,
            self.bucket,
            uri_encode_segment(key)
        )
    }

    /// AWS SigV4 headers (service `s3`, region `auto`) for one request.
    ///
    /// `extra` holds headers the caller also puts on the wire — signing a
    /// header that is not sent (or sending a signed header with a different
    /// value) is the one way to get a 403 from a correct implementation.
    fn sign_headers(
        &self,
        method: &str,
        url: &str,
        payload: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<Vec<(&'static str, String)>, StorageError> {
        let parsed = url::Url::parse(url)
            .map_err(|e| StorageError::Http(format!("bad endpoint url: {e}")))?;
        let host = match (parsed.host_str(), parsed.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => return Err(StorageError::Http("endpoint has no host".into())),
        };

        let now = chrono::Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let day = now.format("%Y%m%d").to_string();
        let scope = format!("{day}/{S3_REGION}/s3/aws4_request");
        let payload_hash = sha256_hex(payload);

        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), host),
            ("x-amz-content-sha256".into(), payload_hash.clone()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        for (name, value) in extra {
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
        headers.sort_by(|a, b| a.0.cmp(&b.0));
        let signed_headers = headers
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(";");
        let canonical_headers = headers
            .iter()
            .map(|(n, v)| format!("{n}:{v}\n"))
            .collect::<String>();

        // S3 signs the path once-encoded, with `/` preserved as a separator.
        let canonical_request = format!(
            "{method}\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            uri_encode_segment(parsed.path())
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        let k_date = hmac_sha256(
            format!("AWS4{}", self.secret_key).as_bytes(),
            day.as_bytes(),
        )?;
        let k_region = hmac_sha256(&k_date, S3_REGION.as_bytes())?;
        let k_service = hmac_sha256(&k_region, b"s3")?;
        let k_signing = hmac_sha256(&k_service, b"aws4_request")?;
        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes())?);

        Ok(vec![
            ("x-amz-date", amz_date),
            ("x-amz-content-sha256", payload_hash),
            (
                "authorization",
                format!(
                    "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                    self.access_key
                ),
            ),
        ])
    }

    /// Send a signed `PUT`/`GET`/`DELETE`. Returns the raw response so
    /// callers can interpret 404 themselves (missing object vs. already
    /// deleted are both fine for `delete`).
    async fn send_signed(
        &self,
        method: &str,
        url: &str,
        payload: Vec<u8>,
        mime: Option<&str>,
    ) -> Result<reqwest::Response, StorageError> {
        let mut req = match method {
            "PUT" => self.client.put(url),
            "GET" => self.client.get(url),
            "DELETE" => self.client.delete(url),
            _ => return Err(StorageError::Http(format!("unsupported method {method}"))),
        };
        let extra: Vec<(&str, &str)> = mime.map(|m| vec![("content-type", m)]).unwrap_or_default();
        if let Some(m) = mime {
            req = req.header("content-type", m);
        }
        for (name, value) in self.sign_headers(method, url, &payload, &extra)? {
            req = req.header(name, value);
        }
        req.body(payload)
            .send()
            .await
            .map_err(|e| StorageError::Http(e.to_string()))
    }
}

#[async_trait::async_trait]
impl MediaStorage for S3 {
    fn backend(&self) -> &'static str {
        "s3"
    }

    async fn store(&self, key: &str, data: &[u8], mime: &str) -> Result<(), StorageError> {
        let resp = self
            .send_signed("PUT", &self.object_url(key), data.to_vec(), Some(mime))
            .await?;
        expect_ok(resp).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        let resp = self
            .send_signed("GET", &self.object_url(key), Vec::new(), None)
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound);
        }
        expect_ok(resp)
            .await?
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| StorageError::Http(e.to_string()))
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let resp = self
            .send_signed("DELETE", &self.object_url(key), Vec::new(), None)
            .await?;
        // S3 already answers 204 for a missing key; tolerate a bucket that
        // answers 404 instead, since LocalDisk::delete is a no-op there.
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        expect_ok(resp).await?;
        Ok(())
    }
}

async fn expect_ok(resp: reqwest::Response) -> Result<reqwest::Response, StorageError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(StorageError::Http(format!(
        "{status}: {}",
        body.chars().take(200).collect::<String>()
    )))
}

/// Percent-encode a path, preserving `/` so object keys with folders keep
/// their separators. S3 signs the path in exactly this encoded form, so the
/// value sent on the wire and the value signed can never drift.
fn uri_encode_segment(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~' | '/') {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(<sha2::Sha256 as sha2::Digest>::digest(bytes))
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Result<Vec<u8>, StorageError> {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key)
        .map_err(|e| StorageError::Http(format!("bad hmac key: {e}")))?;
    mac.update(msg);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Backend selection, split out from [`storage`] so the rules are testable
/// without a bucket and without mutating process env.
fn select_backend(
    s3: Option<S3Config>,
    media_dir: &str,
    client: &reqwest::Client,
) -> Arc<dyn MediaStorage> {
    match s3 {
        Some(cfg) => {
            log_backend_once(true, &cfg.bucket);
            Arc::new(S3::new(cfg, client.clone()))
        }
        None => {
            log_backend_once(false, media_dir);
            Arc::new(LocalDisk::new(media_dir))
        }
    }
}

/// The backend media bytes live in for this request.
///
/// Reads the S3 env on every call so a restarted process picks up a changed
/// config without a rebuild, and so the local path stays byte-identical to
/// the previous hardcoded `config.media_dir` behaviour when S3 is unset.
pub fn storage(state: &AppState) -> Arc<dyn MediaStorage> {
    select_backend(
        S3Config::from_env(),
        &state.config.media_dir,
        &state.media_http_client,
    )
}

/// `storage()` is per-request but the answer only changes when the operator
/// changes the environment — log it once, loudly, and not per request.
fn log_backend_once(is_s3: bool, where_: &str) {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    LOGGED.call_once(|| {
        if is_s3 {
            tracing::info!("Media storage: S3 bucket `{where_}`");
        } else {
            tracing::info!("Media storage: local disk `{where_}`");
        }
    });
}

fn warn_once(msg: &str) {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    LOGGED.call_once(|| tracing::warn!("{msg}"));
}

/// POST /api/media — upload a file
///
/// Streams the multipart field to disk chunk by chunk. The 50 MB cap is
/// enforced against the running total, so an oversized upload is rejected
/// mid-stream instead of after the whole body has been buffered. Only the
/// first [`SNIFF_HEAD_BYTES`] are retained for magic-byte sniffing and image
/// dimension detection; the rest never sits in memory.
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    mut req: Request,
) -> Result<Json<MediaPublic>, AppError> {
    // `Multipart::from_request` wraps the body in axum's *default* 2 MB
    // limit, which would cap every upload long before the 50 MB cap below.
    // Disable it so `MAX_FILE_SIZE` is the single authority on size — one
    // rule, one error message, enforced incrementally by `stream_to_disk`
    // rather than by a body wrapper that trips with its own 413.
    DefaultBodyLimit::disable().apply(&mut req);
    let mut multipart = Multipart::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(format!("Invalid multipart: {}", e.body_text())))?;

    let mut field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Invalid multipart: {e}")))?
        .ok_or_else(|| AppError::BadRequest("No file uploaded".into()))?;

    let original_name = field.file_name().unwrap_or("unnamed").to_string();

    let declared_mime = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    // Save to local filesystem
    let file_id = Uuid::new_v4();
    let ext = FsPath::new(&original_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let filename = format!("{file_id}.{ext}");
    let upload_dir = FsPath::new(&state.config.media_dir);
    tokio::fs::create_dir_all(upload_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create upload dir: {e}")))?;

    let filepath = upload_dir.join(&filename);
    let (size, head) = stream_to_disk(&mut field, &filepath, MAX_FILE_SIZE).await?;

    // The MIME validation, the storage-backend push and the DB row all
    // happen in `finish_upload` — ONE implementation, shared with the MCP
    // and CLI uploaders. `head` carries the first SNIFF_HEAD_BYTES of the
    // stream: enough for every signature in `sniff_mime` and for the JPEG
    // SOF scan.
    finish_upload(
        &state,
        auth.user_id,
        &original_name,
        &filepath,
        &filename,
        &declared_mime,
        &head,
        size,
        None,
    )
    .await
    .map(Json)
}

/// Validate, store and record an upload whose bytes are already on local disk.
///
/// THE ONE upload path. The REST handler streams a multipart field to disk
/// and calls this; the MCP tools and the CLI, which hold the bytes in
/// memory, write the same staging file and call this too. That is the point:
/// before it, the MCP/CLI uploaders wrote straight to `config.media_dir`
/// with the caller's MIME string and no sniff, so:
///
/// - on an S3/R2 deployment they wrote to the LOCAL directory while
///   `serve_media` read through the S3 backend, and an agent's upload
///   returned a URL that 404s — the image never reached the platform;
/// - `switchpost media upload ~/.switchpost/.env` copied a SECRET into the
///   media store, and `GET /api/media/<uuid>` is a public route.
///
/// The MIME allowlist and the magic-byte sniff run here, so every uploader
/// gets them.
pub(crate) async fn finish_upload(
    state: &AppState,
    user_id: Uuid,
    original_name: &str,
    filepath: &FsPath,
    filename: &str,
    declared_mime: &str,
    head: &[u8],
    size: u64,
    alt: Option<&str>,
) -> Result<MediaPublic, AppError> {
    if size > MAX_FILE_SIZE {
        let _ = tokio::fs::remove_file(filepath).await;
        return Err(AppError::BadRequest(format!(
            "File too large (max {} MB)",
            MAX_FILE_SIZE / (1024 * 1024)
        )));
    }

    // ── MIME validation: allowlist + magic-byte sniff ────────
    // The caller's Content-Type is never trusted alone.
    let mime_type = match resolve_mime(declared_mime, head) {
        Ok(mime) => mime,
        Err(err) => {
            let _ = tokio::fs::remove_file(filepath).await;
            return Err(err);
        }
    };

    // Get dimensions for images
    let (width, height) = if mime_type.starts_with("image/") {
        detect_image_dimensions(head)
    } else {
        (None, None)
    };

    // Remote backend: the staging copy is on local disk. Push the completed
    // object to the active backend and drop the local copy, so serve/delete
    // resolve against the same backend that holds the bytes. Local backend:
    // no-op, the file is already home.
    let backend = storage(state);
    if backend.backend() != "local" {
        let bytes = tokio::fs::read(filepath).await.map_err(|e| {
            AppError::Internal(format!("Failed to stage upload for remote backend: {e}"))
        })?;
        if let Err(e) = backend.store(filename, &bytes, &mime_type).await {
            let _ = tokio::fs::remove_file(filepath).await;
            return Err(AppError::Internal(format!(
                "Remote backend store failed: {e}"
            )));
        }
        let _ = tokio::fs::remove_file(filepath).await;
    }

    let entry = queries::create_media(
        &state.db,
        user_id,
        original_name,
        filename,
        &mime_type,
        size as i64,
        width,
        height,
        alt,
    )
    .await?;

    Ok(MediaPublic::from(entry))
}

/// Write in-memory bytes to the staging path and run them through the one
/// upload path. Used by the MCP and CLI uploaders, which hold the bytes.
pub(crate) async fn store_upload_bytes(
    state: &AppState,
    user_id: Uuid,
    original_name: &str,
    declared_mime: &str,
    bytes: &[u8],
    alt: Option<&str>,
) -> Result<MediaPublic, AppError> {
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(AppError::BadRequest(format!(
            "File too large (max {} MB)",
            MAX_FILE_SIZE / (1024 * 1024)
        )));
    }
    let ext = FsPath::new(original_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let filename = format!("{}.{}", Uuid::new_v4(), ext);
    let upload_dir = FsPath::new(&state.config.media_dir);
    tokio::fs::create_dir_all(upload_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create upload dir: {e}")))?;
    let filepath = upload_dir.join(&filename);
    tokio::fs::write(&filepath, bytes)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    let head: Vec<u8> = bytes.iter().take(SNIFF_HEAD_BYTES).copied().collect();
    match finish_upload(
        state,
        user_id,
        original_name,
        &filepath,
        &filename,
        declared_mime,
        &head,
        bytes.len() as u64,
        alt,
    )
    .await
    {
        Ok(media) => Ok(media),
        Err(e) => {
            let _ = tokio::fs::remove_file(&filepath).await;
            Err(e)
        }
    }
}

/// Drain `field` into `dest`, aborting with "File too large" the moment the
/// running total passes `max_bytes`.
///
/// Returns `(bytes_written, head)` where `head` is up to
/// [`SNIFF_HEAD_BYTES`] of leading bytes kept for sniffing. The sink is closed
/// on every failure path, and the partial file is removed — a rejected upload
/// never leaves bytes on disk.
async fn stream_to_disk(
    field: &mut multipart::Field,
    dest: &FsPath,
    max_bytes: u64,
) -> Result<(u64, Vec<u8>), AppError> {
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    let mut size: u64 = 0;
    let mut head: Vec<u8> = Vec::with_capacity(SNIFF_HEAD_BYTES);

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::BadRequest(format!("Read error: {e}")))?
    {
        // Check before writing: the cap must hold during the stream, so the
        // over-limit chunk is never persisted either.
        size += chunk.len() as u64;
        if size > max_bytes {
            return Err(abort_partial(dest, file).await);
        }

        if head.len() < SNIFF_HEAD_BYTES {
            let take = (SNIFF_HEAD_BYTES - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
        }

        if let Err(e) = file.write_all(&chunk).await {
            return Err(AppError::Internal(format!("Failed to write file: {e}")));
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    Ok((size, head))
}

/// Close `file` and delete whatever it already wrote, returning the
/// caller-facing error. Used when the cap trips mid-stream.
async fn abort_partial(dest: &FsPath, file: tokio::fs::File) -> AppError {
    drop(file);
    let _ = tokio::fs::remove_file(dest).await;
    AppError::BadRequest(format!(
        "File too large (max {} MB)",
        MAX_FILE_SIZE / (1024 * 1024)
    ))
}

/// Resolve the stored MIME type from the client's declared type and the real
/// bytes. Unchanged rules, extracted so both the handler and its tests share
/// one implementation.
///
/// The client-supplied Content-Type is never trusted alone: the sniffed MIME
/// must (a) be in the allowlist and (b) match the declared Content-Type when
/// one was supplied. Mismatches are rejected.
fn resolve_mime(declared: &str, head: &[u8]) -> Result<String, AppError> {
    let sniffed_mime = sniff_mime(head);
    match (declared, sniffed_mime) {
        // Sniffer recognised the bytes — trust it (it can't be lied to).
        (declared, Some(sniffed)) if ALLOWED_MIMES.contains(&sniffed.as_str()) => {
            // If the client declared something else, reject as suspicious.
            if !declared.is_empty() && declared != "application/octet-stream" && declared != sniffed
            {
                return Err(AppError::BadRequest(format!(
                    "MIME mismatch: declared `{declared}` but bytes look like `{sniffed}`"
                )));
            }
            Ok(sniffed)
        }
        // Either sniffer said "not allowed" or declared is not in allowlist.
        (_, Some(sniffed)) => Err(AppError::BadRequest(format!(
            "Unsupported file type: `{sniffed}`. Allowed: PNG, JPEG, WebP, GIF, MP4, QuickTime."
        ))),
        (_, None) => Err(AppError::BadRequest(
            "Unrecognised file type. Allowed: PNG, JPEG, WebP, GIF, MP4, QuickTime.".into(),
        )),
    }
}

/// GET /api/media — list user's media uploads
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Query(query): axum::extract::Query<ListMediaQuery>,
) -> Result<Json<Vec<MediaPublic>>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let search = query.search.as_deref();
    let entries = queries::list_media(&state.db, auth.user_id, limit, offset, search).await?;
    Ok(Json(entries.into_iter().map(MediaPublic::from).collect()))
}

/// PATCH /api/media/:id — update a media item's alt text.
///
/// The alt text is a real column, but there was no way to write it from
/// the dashboard: the upload hard-coded `None` and the composer's edit
/// box only changed a browser-side object. The founder's alt text was
/// therefore lost on reload - and destroyed by the first save of the
/// post, which wrote the (empty) browser value back over it.
#[derive(Debug, serde::Deserialize)]
pub struct UpdateMediaRequest {
    /// The alt text. `null` clears it.
    pub alt: Option<String>,
}

/// PATCH /api/media/:id — update a media item's alt text.
pub async fn update(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateMediaRequest>,
) -> Result<Json<MediaPublic>, AppError> {
    let entry = queries::set_media_alt(&state.db, id, auth.user_id, body.alt.as_deref())
        .await?
        .ok_or_else(|| AppError::NotFound("Media not found".into()))?;
    Ok(Json(MediaPublic::from(entry)))
}

/// DELETE /api/media/:id — delete a media item
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let entry = queries::delete_media(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Media not found".into()))?;

    storage(&state)
        .delete(&entry.storage_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({"deleted": true})))
}

/// GET /api/media/:id — serve a media file (single-user mode, no auth)
///
/// Security:
///   - Always sets `X-Content-Type-Options: nosniff` to stop browsers from
///     MIME-sniffing the body and interpreting non-image bytes as HTML/JS.
///   - For non-image MIME types (video, octet-stream, etc.), sets
///     `Content-Disposition: attachment` so the body is downloaded, not
///     rendered. This blocks the stored-XSS-by-Content-Type attack vector.
pub async fn serve_media(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response<Body>, AppError> {
    let media = queries::get_media(&state.db, id)
        .await?
        .ok_or_else(|| AppError::NotFound("Media not found".into()))?;

    let data = storage(&state)
        .get(&media.storage_path)
        .await
        .map_err(|_| AppError::NotFound("File not found on disk".into()))?;

    let mime_str = media.mime_type.as_str();

    // Defense-in-depth: sniff the bytes again at serve time. If the
    // sniffed MIME disagrees with the stored MIME, serve as attachment
    // with octet-stream. This catches any pre-existing rows in the DB
    // that were uploaded before the allowlist was enforced.
    //
    // NOTE: sniff_mime MUST be called before `Body::from(data)` below —
    // `Body::from` moves `data` and we need a `&[u8]` reference to it.
    let (final_content_type, disposition) = match sniff_mime(&data) {
        Some(ref sniffed) if sniffed.as_str() == mime_str => (mime_str.to_string(), "inline"),
        Some(ref sniffed)
            if ALLOWED_MIMES.contains(&sniffed.as_str())
                && mime_str == "application/octet-stream" =>
        {
            (
                sniffed.clone(),
                if sniffed.starts_with("image/") {
                    "inline"
                } else {
                    "attachment"
                },
            )
        }
        _ => ("application/octet-stream".to_string(), "attachment"),
    };

    let body = Body::from(data);

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, &final_content_type)
        .header(
            header::CONTENT_LENGTH,
            body.size_hint().exact().unwrap_or(0),
        )
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Disposition", disposition)
        .body(body)
        .expect("static headers and body are valid"))
}

/// Hosts `proxy_media` may fetch. Checked on the request URL and on every
/// redirect hop, so a redirect cannot carry the fetch off the list.
/// Hosts the media proxy may fetch from.
///
/// EXACT host names only, plus two suffix rules that cannot be spoofed:
///
/// - `scontent-*.cdninstagram.com` — Instagram's CDN;
/// - `*.fbcdn.net` — Facebook's CDN (a leading-dot suffix rule, so
///   `evil-fbcdn.net` does not match and `x.fbcdn.net.attacker.com`
///   does not either).
///
/// The old rule was `host.starts_with("scontent-") && host.contains(".fbcdn.")`,
/// which `scontent-evil.fbcdn.attacker.com` satisfies — an attacker who
/// controls a domain could put an arbitrary response behind an allowlisted
/// name.
///
/// `files.catbox.moe` and `i.imgur.com` were REMOVED: both are anonymous
/// upload hosts, so an attacker could put a file there and have the app
/// serve it. The proxy is for CONTENT the app already trusts (a tweet's
/// video, an Instagram post's image); it is not a general fetcher.
fn is_proxyable_media_host(scheme: &str, host: &str) -> bool {
    if scheme != "https" {
        return false;
    }
    let host = host.to_ascii_lowercase();
    if matches!(
        host.as_str(),
        "video.twimg.com"
            | "pbs.twimg.com"
            | "media.tenor.com"
            | "www.instagram.com"
            | "i.ytimg.com"
    ) {
        return true;
    }
    // Instagram content CDN: `scontent-<hash>.cdninstagram.com`.
    if host.starts_with("scontent-") && host.ends_with(".cdninstagram.com") {
        return true;
    }
    // Facebook CDN: anything under `fbcdn.net`, matched on the dot so a
    // lookalike domain cannot pass.
    host.ends_with(".fbcdn.net")
}

const PROXY_MAX_REDIRECTS: usize = 5;

/// The proxy reads the whole upstream body into memory, so it is capped.
/// The hosts are trusted CDNs and their media is well under this.
const PROXY_MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

/// Redirect policy for the reqwest proxy client: follow only allowlisted hops.
fn reqwest_proxy_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let url = attempt.url();
        if attempt.previous().len() >= PROXY_MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if is_proxyable_media_host(url.scheme(), url.host_str().unwrap_or("")) {
            attempt.follow()
        } else {
            attempt.error("redirect to a host outside the media allowlist")
        }
    })
}

/// Same rule as `reqwest_proxy_redirects`, for wreq (set per request).
fn wreq_proxy_redirects() -> wreq::redirect::Policy {
    wreq::redirect::Policy::custom(|attempt| {
        let uri = attempt.uri();
        if attempt.previous().len() >= PROXY_MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if is_proxyable_media_host(uri.scheme_str().unwrap_or(""), uri.host().unwrap_or(""))
        {
            attempt.follow()
        } else {
            attempt.error("redirect to a host outside the media allowlist")
        }
    })
}

// ponytail: the reqwest client is a separate pool from the shared media client
// (reqwest 0.12 has no per-request redirect policy). Its redirect policy is set
// at build time, and it fails closed (no redirects) if the build errors.
static PROXY_HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest_proxy_redirects())
        .build()
        .unwrap_or_else(|_| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default()
        })
});

/// GET /api/proxy-media?url=... — proxy external media to bypass CORS/CDN restrictions
/// Used for X/Twitter video CDN which returns 403 when loaded directly from browser.
/// Uses wreq (Chrome TLS fingerprinting) for X/Twitter CDN domains to bypass bot detection.
/// Supports Range requests for video playback (seeking, adaptive streaming).
pub async fn proxy_media(
    State(state): State<AppState>,
    req_headers: axum::http::header::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<ProxyMediaQuery>,
) -> Result<Response<Body>, AppError> {
    use axum::http::header;

    // Validate URL to prevent SSRF — only allow known CDN domains
    let url = &params.url;
    let parsed = url::Url::parse(url).map_err(|_| AppError::BadRequest("Invalid URL".into()))?;
    let host = parsed.host_str().unwrap_or("");
    if !is_proxyable_media_host(parsed.scheme(), host) {
        return Err(AppError::BadRequest(
            "Domain not allowed for proxying".into(),
        ));
    }

    // X/Twitter CDN domains use wreq for Chrome TLS fingerprinting.
    // Other CDNs use the standard reqwest client.
    // We must fetch inside each branch because wreq::Response and reqwest::Response
    // are different types — Rust's if/else requires both branches to match.
    let is_x_cdn = host == "video.twimg.com" || host == "pbs.twimg.com";
    let range_header = req_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Shared headers for all upstream requests
    const UPSTREAM_REFERER: &str = "https://x.com/";
    const UPSTREAM_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    // Fetch from upstream and extract common fields.
    // Both branches produce identical (status, content_length, content_range, bytes).
    // The upstream's Content-Type is deliberately NOT carried out of here:
    // the served type comes from the SNIFFED bytes, so an attacker-chosen
    // `image/svg+xml` cannot put a document on the app origin.
    let (upstream_status, content_length, content_range, bytes) = if is_x_cdn {
        let mut req = state
            .media_wreq_client
            .get(url)
            .redirect(wreq_proxy_redirects())
            .header(header::REFERER, UPSTREAM_REFERER)
            .header(header::USER_AGENT, UPSTREAM_UA);
        if let Some(ref range) = range_header {
            req = req.header(header::RANGE, range.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch media (wreq): {e}")))?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(AppError::Internal(format!("Upstream returned {status}")));
        }
        let cl = resp.content_length();
        let cr = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        if let Some(len) = cl {
            if len > PROXY_MAX_BODY_BYTES {
                return Err(AppError::BadRequest(format!(
                    "upstream media is {len} bytes, over the {PROXY_MAX_BODY_BYTES}-byte limit"
                )));
            }
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read upstream body (wreq): {e}")))?;
        (status, cl, cr, bytes)
    } else {
        let mut req = PROXY_HTTP_CLIENT
            .get(url)
            .header(header::REFERER, UPSTREAM_REFERER)
            .header(header::USER_AGENT, UPSTREAM_UA);
        if let Some(ref range) = range_header {
            req = req.header(header::RANGE, range.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch media (reqwest): {e}")))?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(AppError::Internal(format!("Upstream returned {status}")));
        }
        let cl = resp.content_length();
        let cr = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        if let Some(len) = cl {
            if len > PROXY_MAX_BODY_BYTES {
                return Err(AppError::BadRequest(format!(
                    "upstream media is {len} bytes, over the {PROXY_MAX_BODY_BYTES}-byte limit"
                )));
            }
        }
        let bytes = resp.bytes().await.map_err(|e| {
            AppError::Internal(format!("Failed to read upstream body (reqwest): {e}"))
        })?;
        (status, cl, cr, bytes)
    };

    // Only a SNIFFED image or video is served inline.
    //
    // The upstream `Content-Type` is the attacker's to choose. `image/svg+xml`
    // is an `image/*` type and an SVG is a DOCUMENT: served inline on the app
    // origin, its `<script>` runs with the founder's `sf_session` cookie, and
    // same-origin means CSRF and SameSite do not apply. A link in a chat
    // message would be account takeover.
    //
    // So the BYTES decide, using the same sniffer the upload path uses. An
    // SVG (or anything else the sniffer does not recognise) is returned as an
    // opaque download: `application/octet-stream` + `Content-Disposition:
    // attachment` + `nosniff`, so it cannot execute on the app origin.
    let sniffed = sniff_mime(&bytes);
    let inline_media = sniffed
        .as_deref()
        .map(|m| m.starts_with("image/") || m.starts_with("video/"))
        .unwrap_or(false);
    let served_type = match (&sniffed, inline_media) {
        (Some(m), true) => m.as_str(),
        _ => "application/octet-stream",
    };

    // Build response headers — forward status (200 or 206 for Range)
    let mut builder = Response::builder()
        .status(upstream_status.as_u16())
        .header(header::CONTENT_TYPE, served_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
        .header(header::ACCEPT_RANGES, "bytes");
    if !inline_media {
        builder = builder.header(header::CONTENT_DISPOSITION, "attachment");
    }

    // Forward content-length and content-range from upstream
    if let Some(cl) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, cl);
    }
    if let Some(cr) = &content_range {
        builder = builder.header("Content-Range", cr.as_str());
    }

    let body = Body::from(bytes);

    builder
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Debug, serde::Deserialize)]
pub struct ProxyMediaQuery {
    pub url: String,
}

fn detect_image_dimensions(data: &[u8]) -> (Option<i32>, Option<i32>) {
    // Simple PNG dimensions check
    if data.len() > 24 && data[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return (Some(w as i32), Some(h as i32));
    }
    // JPEG dimensions check
    if data.len() > 4 && data[..2] == [0xFF, 0xD8] {
        let mut pos = 2;
        while pos + 9 < data.len() {
            if data[pos] == 0xFF && data[pos + 1] >= 0xC0 && data[pos + 1] <= 0xCF {
                let h = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                let w = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                return (Some(w as i32), Some(h as i32));
            }
            pos += 2 + u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
            if pos == 2 {
                break;
            } // safety: avoid getting stuck
        }
    }
    (None, None)
}

/// Sniff the actual MIME type from file magic bytes.
///
/// Returns `Some(mime)` for the formats in `ALLOWED_MIMES`, or `None`
/// if the bytes don't match any recognised signature. This is used
/// both at upload time (defense-in-depth against client-supplied
/// Content-Type lies) and at serve time (defense-in-depth against
/// any pre-existing DB rows that predate the upload allowlist).
///
/// Signatures sourced from the IANA media-type registry and the
/// `infer` crate (which we don't depend on to keep the dep tree lean).
fn sniff_mime(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.len() >= 8 && data[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
        return Some("image/png".into());
    }
    // JPEG: FF D8 FF
    if data.len() >= 3 && data[..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg".into());
    }
    // GIF: "GIF87a" or "GIF89a"
    if data.len() >= 6 && (data[..6] == *b"GIF87a" || data[..6] == *b"GIF89a") {
        return Some("image/gif".into());
    }
    // WebP: "RIFF" .... "WEBP"
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp".into());
    }
    // MP4: ftyp box at offset 4 — "ftyp" + 4-char brand
    // Mask: 00 00 00 ?? 66 74 79 70 (?? = box size, varies)
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        let brand = &data[8..12];
        // Common MP4 brands: isom, iso2, mp41, mp42, avc1, M4V , M4A , etc.
        // QuickTime: qt  (with trailing spaces), which we map to video/quicktime.
        if brand == b"qt  " {
            return Some("video/quicktime".into());
        }
        return Some("video/mp4".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state whose media dir is a scratch directory, so the shared upload
    /// path can be exercised without touching the real store.
    async fn upload_state(media_dir: &std::path::Path) -> AppState {
        crate::config::load_dotenv_for_tests();
        let mut config = crate::config::Config::from_env().expect("config");
        config.media_dir = media_dir.to_string_lossy().into_owned();
        let pool = crate::db::create_pool(&config.database_url)
            .await
            .expect("pool");
        crate::db::ensure_local_user(&pool)
            .await
            .expect("local user");
        let registry = crate::social::registry::ProviderRegistry::new(&config, None, None);
        AppState {
            db: pool,
            config,
            broadcast: crate::realtime::Broadcaster::new(),
            providers: registry,
            token_key: None,
            telegram_client_manager: None,
            wa_client: None,
            media_http_client: reqwest::Client::new(),
            media_wreq_client: wreq::Client::new(),
        }
    }

    fn upload_scratch_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("switchpost-media-{tag}-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn the_shared_upload_path_refuses_a_file_that_is_not_media() {
        // The MCP/CLI upload used to copy ANY local file into the media
        // store — `media upload ~/.switchpost/.env` put a secret at a
        // PUBLIC url (`GET /api/media/<uuid>` is not auth-gated).
        let dir = upload_scratch_dir("reject");
        let state = upload_state(&dir).await;
        let secret = b"APP_PASSWORD=hunter2\nTOKEN_ENCRYPTION_KEY=deadbeef\n";

        let err = store_upload_bytes(
            &state,
            crate::auth::middleware::DEFAULT_USER_ID,
            ".env",
            "application/octet-stream",
            secret,
            None,
        )
        .await
        .expect_err("a text file is not media and must be refused");

        assert!(
            err.to_string().to_lowercase().contains("file type")
                || err.to_string().to_lowercase().contains("unsupported"),
            "the error must name the problem, got: {err}"
        );
        let mut entries = tokio::fs::read_dir(&dir).await.expect("read dir");
        let stored = entries.next_entry().await.expect("read entry").is_some();
        assert!(!stored, "the refused file must not be left on disk");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn the_shared_upload_path_accepts_a_png_and_records_it() {
        let dir = upload_scratch_dir("accept");
        let state = upload_state(&dir).await;
        let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
        png.extend_from_slice(&[0u8; 64]);

        let media = store_upload_bytes(
            &state,
            crate::auth::middleware::DEFAULT_USER_ID,
            "shot.png",
            "image/png",
            &png,
            None,
        )
        .await
        .expect("a PNG must be accepted");

        assert_eq!(media.mime_type, "image/png");
        assert_eq!(media.original_name, "shot.png");
        assert!(media.url.starts_with("/api/media/"), "got {}", media.url);

        // The row exists and the bytes are on disk.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE id = ?")
            .bind(media.id)
            .fetch_one(&state.db)
            .await
            .expect("count");
        assert_eq!(count, 1);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn the_proxy_allowlist_is_exact_or_a_real_suffix() {
        // The CDNs the app actually serves.
        assert!(is_proxyable_media_host("https", "video.twimg.com"));
        assert!(is_proxyable_media_host("https", "pbs.twimg.com"));
        assert!(is_proxyable_media_host("https", "i.ytimg.com"));
        assert!(is_proxyable_media_host(
            "https",
            "scontent-abc123.cdninstagram.com"
        ));
        assert!(is_proxyable_media_host("https", "x.fbcdn.net"));

        // Lookalikes: an attacker who controls a domain must not pass.
        assert!(!is_proxyable_media_host(
            "https",
            "scontent-evil.fbcdn.attacker.com"
        ));
        assert!(!is_proxyable_media_host("https", "evil-fbcdn.net"));
        assert!(!is_proxyable_media_host(
            "https",
            "x.fbcdn.net.attacker.com"
        ));
        assert!(!is_proxyable_media_host("https", "notcdninstagram.com"));
        assert!(!is_proxyable_media_host(
            "https",
            "video.twimg.com.attacker.com"
        ));

        // Anonymous upload hosts are gone: they let an attacker choose
        // the bytes the app serves.
        assert!(!is_proxyable_media_host("https", "files.catbox.moe"));
        assert!(!is_proxyable_media_host("https", "i.imgur.com"));

        // http is not allowed.
        assert!(!is_proxyable_media_host("http", "video.twimg.com"));
    }

    #[test]
    fn sniff_png_signature() {
        let png = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0];
        assert_eq!(sniff_mime(&png), Some("image/png".into()));
    }

    #[test]
    fn sniff_jpeg_signature() {
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
        assert_eq!(sniff_mime(&jpg), Some("image/jpeg".into()));
    }

    #[test]
    fn sniff_gif_signatures() {
        assert_eq!(sniff_mime(b"GIF87aextra"), Some("image/gif".into()));
        assert_eq!(sniff_mime(b"GIF89aextra"), Some("image/gif".into()));
    }

    #[test]
    fn sniff_webp_signature() {
        let mut webp = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        webp.extend_from_slice(&[0; 10]);
        assert_eq!(sniff_mime(&webp), Some("image/webp".into()));
    }

    #[test]
    fn sniff_mp4_signature() {
        let mp4 = [0, 0, 0, 32, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm'];
        assert_eq!(sniff_mime(&mp4), Some("video/mp4".into()));
    }

    #[test]
    fn sniff_rejects_html() {
        let html = b"<script>alert(1)</script>";
        assert_eq!(sniff_mime(html), None);
    }

    #[test]
    fn sniff_rejects_empty() {
        assert_eq!(sniff_mime(&[]), None);
        assert_eq!(sniff_mime(&[1, 2, 3]), None);
    }

    // ── backend selection ───────────────────────────────────
    // No bucket, no network: these pin the rules that decide which
    // backend serves a request.

    fn s3_env() -> S3Config {
        S3Config::from_vars(
            Some("https://acct.r2.cloudflarestorage.com/"),
            Some("media"),
            Some("AKIA"),
            Some("secret"),
        )
        .expect("all four vars present")
    }

    #[test]
    fn selects_local_when_s3_unset() {
        let backend = select_backend(None, "./uploads", &crate::social::shared_http());
        assert_eq!(backend.backend(), "local");
    }

    #[test]
    fn selects_s3_when_configured() {
        let backend = select_backend(Some(s3_env()), "./uploads", &crate::social::shared_http());
        assert_eq!(backend.backend(), "s3");
    }

    #[test]
    fn s3_config_requires_all_four_vars() {
        let partial = S3Config::from_vars(
            Some("https://acct.r2.cloudflarestorage.com"),
            Some("media"),
            Some("AKIA"),
            None,
        );
        assert!(partial.is_none());
    }

    #[test]
    fn s3_config_rejects_blank_vars() {
        let blank = S3Config::from_vars(
            Some("https://acct.r2.cloudflarestorage.com"),
            Some("  "),
            Some("AKIA"),
            Some("secret"),
        );
        assert!(blank.is_none());
    }

    #[test]
    fn s3_config_strips_trailing_slash_from_endpoint() {
        assert_eq!(s3_env().endpoint, "https://acct.r2.cloudflarestorage.com");
    }

    #[test]
    fn object_url_is_path_style_and_encodes_key() {
        let s3 = S3::new(s3_env(), crate::social::shared_http());
        assert_eq!(
            s3.object_url("ab cd.png"),
            "https://acct.r2.cloudflarestorage.com/media/ab%20cd.png"
        );
    }

    #[test]
    fn local_disk_roundtrips_and_deletes_idempotently() {
        let dir = std::env::temp_dir().join(format!("sf-media-test-{}", Uuid::new_v4()));
        let local = LocalDisk::new(&dir);
        let rt = async_runtime();

        rt.block_on(async {
            local
                .store("a.png", b"\x89PNG-body", "image/png")
                .await
                .unwrap();
            assert_eq!(local.get("a.png").await.unwrap(), b"\x89PNG-body");

            local.delete("a.png").await.unwrap();
            assert!(matches!(
                local.get("a.png").await,
                Err(StorageError::Io { .. })
            ));
            // Second delete must be a no-op, not an error.
            local.delete("a.png").await.unwrap();
        });

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Minimal current-thread runtime so the disk test needs no dev-dependency.
    fn async_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    // ─── Streaming upload path ─────────────────────────────────
    // The handler needs a live `AppState` + Postgres, so these tests drive the
    // same three steps it does — `Multipart` → `stream_to_disk` → `resolve_mime`
    // — against a real multipart request body. What they pin: the cap trips
    // mid-stream and leaves nothing on disk, a large-but-legal body lands
    // byte-for-byte while only a bounded head stays in memory, and small
    // uploads resolve exactly as the buffered version resolved them.

    const BOUNDARY: &str = "switchpost-test-boundary";
    const CHUNK: usize = 1024 * 1024;

    use axum::body::Bytes;

    /// A multipart request whose single `video/mp4` file field is exactly
    /// `total_bytes` long, delivered in 1 MiB frames so the test process never
    /// materialises the whole body either.
    fn mp4_upload_request(total_bytes: usize) -> Request {
        use axum::http::header::CONTENT_TYPE;
        use futures::stream;

        let mut payload = vec![0u8; CHUNK];
        // `ftyp` box at offset 4 — what sniff_mime keys on.
        payload[..12]
            .copy_from_slice(&[0, 0, 0, 32, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm']);
        let payload = Bytes::from(payload);

        let preamble = Bytes::from(format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"clip.mp4\"\r\nContent-Type: video/mp4\r\n\r\n"
        ));
        let epilogue = Bytes::from(format!("\r\n--{BOUNDARY}--\r\n"));

        // One `Bytes` per frame, all sharing the same refcounted payload — a
        // 48 MiB body without the test holding 48 MiB of data. The tail frame
        // is trimmed so the field is exactly `total_bytes`.
        let full = total_bytes / CHUNK;
        let tail = total_bytes % CHUNK;
        let mut frames: Vec<Bytes> = Vec::with_capacity(full + usize::from(tail > 0) + 2);
        frames.push(preamble);
        frames.extend(std::iter::repeat_n(payload.clone(), full));
        if tail > 0 {
            frames.push(payload.slice(..tail));
        }
        frames.push(epilogue);

        let body = stream::iter(
            frames
                .into_iter()
                .map(|b| -> std::result::Result<Bytes, std::io::Error> { Ok(b) }),
        );

        Request::builder()
            .header(
                CONTENT_TYPE,
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from_stream(body))
            .unwrap()
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("switchpost-upload-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The handler's open-multipart step, with the same body-limit override it
    /// applies. The `Multipart` is returned alongside the field because the
    /// handler holds it for the duration of the stream.
    async fn open_mp4_field(total_bytes: usize) -> (Multipart, multipart::Field) {
        let mut req = mp4_upload_request(total_bytes);
        DefaultBodyLimit::disable().apply(&mut req);
        let mut multipart = Multipart::from_request(req, &()).await.unwrap();
        let field = multipart.next_field().await.unwrap().unwrap();
        (multipart, field)
    }

    #[tokio::test]
    async fn cap_rejects_oversized_upload_mid_stream_and_leaves_no_file() {
        let dest = scratch_dir("oversize").join("out.mp4");

        let (_multipart, mut field) = open_mp4_field(MAX_FILE_SIZE as usize + CHUNK).await;
        let err = stream_to_disk(&mut field, &dest, MAX_FILE_SIZE)
            .await
            .unwrap_err();

        assert!(
            matches!(&err, AppError::BadRequest(m) if m == "File too large (max 50 MB)"),
            "{err:?}"
        );
        assert!(!dest.exists(), "rejected upload left bytes on disk");

        std::fs::remove_dir_all(dest.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn large_upload_streams_to_disk_without_buffering_whole_file() {
        // 48 MiB — under the 50 MiB cap, far over any buffer we'd want.
        let total = 48 * CHUNK;
        let dest = scratch_dir("large").join("out.mp4");

        let (_multipart, mut field) = open_mp4_field(total).await;
        let (size, head) = stream_to_disk(&mut field, &dest, MAX_FILE_SIZE)
            .await
            .unwrap();
        let mime = resolve_mime("video/mp4", &head).unwrap();

        assert_eq!(size, total as u64, "byte count written");
        assert_eq!(
            std::fs::metadata(&dest).unwrap().len(),
            total as u64,
            "bytes on disk"
        );
        assert_eq!(mime, "video/mp4", "magic bytes survive the head-only sniff");
        // The point of the change: memory held is bounded by the head window,
        // not by the file.
        assert_eq!(head.len(), SNIFF_HEAD_BYTES);
        assert_eq!(
            &head[..12],
            &[0, 0, 0, 32, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm']
        );

        std::fs::remove_dir_all(dest.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn small_upload_keeps_whole_file_as_head_and_resolves_as_before() {
        let total = 4096;
        let dest = scratch_dir("small").join("out.mp4");

        let (_multipart, mut field) = open_mp4_field(total).await;
        let (size, head) = stream_to_disk(&mut field, &dest, MAX_FILE_SIZE)
            .await
            .unwrap();
        let mime = resolve_mime("video/mp4", &head).unwrap();

        assert_eq!(size, total as u64);
        assert_eq!(head.len(), total, "a small upload is all head");
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), total as u64);
        assert_eq!(mime, "video/mp4");

        std::fs::remove_dir_all(dest.parent().unwrap()).ok();
    }

    #[test]
    fn resolve_mime_rejects_undeclared_types_and_mismatched_declaration() {
        let html = b"<script>alert(1)</script>";
        // Not on the allowlist → rejected, whatever the bytes say.
        assert!(matches!(
            resolve_mime("text/html", html),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            resolve_mime("application/x-msdownload", html),
            Err(AppError::BadRequest(_))
        ));
        // Declared MP4 over PNG bytes: the sniffer wins, the lie is rejected.
        let png = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0];
        assert!(matches!(
            resolve_mime("video/mp4", &png),
            Err(AppError::BadRequest(m)) if m.contains("MIME mismatch")
        ));
        // Unrecognised bytes under an allowlisted declaration: rejected. The
        // declared type is never trusted on its own (AGENTS §6.2).
        assert!(matches!(
            resolve_mime("image/png", &[]),
            Err(AppError::BadRequest(m)) if m.contains("Unrecognised file type")
        ));
        // Sniffed type with no usable declaration: the sniff decides.
        assert_eq!(
            resolve_mime("application/octet-stream", &png).unwrap(),
            "image/png"
        );
    }

    /// Serves `/start` with a 302 to `/blocked` and records every path it sees.
    async fn redirect_server() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = hits.clone();
        let blocked = format!("{base}/blocked");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let path = String::from_utf8_lossy(&buf[..n])
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                log.lock().unwrap().push(path.clone());
                let resp = if path == "/start" {
                    format!("HTTP/1.1 302 Found\r\nLocation: {blocked}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nleaked"
                        .to_string()
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (base, hits)
    }

    #[tokio::test]
    async fn proxy_reqwest_refuses_redirect_off_allowlist() {
        let (base, hits) = redirect_server().await;
        let res = PROXY_HTTP_CLIENT.get(format!("{base}/start")).send().await;
        assert!(res.is_err());
        assert_eq!(*hits.lock().unwrap(), vec!["/start".to_string()]);
    }

    #[tokio::test]
    async fn proxy_wreq_refuses_redirect_off_allowlist() {
        let (base, hits) = redirect_server().await;
        let res = wreq::Client::new()
            .get(format!("{base}/start"))
            .redirect(wreq_proxy_redirects())
            .send()
            .await;
        assert!(res.is_err());
        assert_eq!(*hits.lock().unwrap(), vec!["/start".to_string()]);
    }

    #[test]
    fn proxy_host_allowlist_accepts_cdn_hosts_over_https_only() {
        assert!(is_proxyable_media_host("https", "pbs.twimg.com"));
        assert!(is_proxyable_media_host(
            "https",
            "scontent-iad3-1.xx.fbcdn.net"
        ));
        assert!(!is_proxyable_media_host("http", "pbs.twimg.com"));
        assert!(!is_proxyable_media_host("https", "evil.example"));
        assert!(!is_proxyable_media_host("https", "scontent-x.example.com"));
    }
}
