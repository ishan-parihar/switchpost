// ─── MCP Media Tools ───────────────────────────────────────────
// Tool handlers for media upload via MCP protocol.
// Allows AI agents to upload images/videos for post attachments.

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadInput {
    pub filename: String,
    pub mime_type: String,
    pub data: String, // base64-encoded file data
    pub alt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadOutput {
    pub id: String,
    pub url: String,
    pub mime_type: String,
    pub filename: String,
    pub size: i64,
    /// The alt text, echoed back so the agent can confirm what it set.
    /// The upload persists it, but the response did not carry it.
    #[serde(default)]
    pub alt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaListInput {
    pub limit: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaItem {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    /// The alt text. The frontend's `MediaItem` declares it as a real
    /// field, but neither the REST nor the MCP response shape carried
    /// it, so it was always undefined.
    #[serde(default)]
    pub alt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaListOutput {
    pub media: Vec<MediaItem>,
    pub total: i64,
}

pub async fn upload_media(
    state: &AppState,
    input: &MediaUploadInput,
) -> Result<Json<MediaUploadOutput>, String> {
    let user_id = resolve_first_user(state).await?;

    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &input.data)
        .map_err(|e| format!("Invalid base64 data: {e}"))?;

    // The ONE upload path: MIME allowlist, magic-byte sniff, size cap and
    // the ACTIVE storage backend. This tool used to write the bytes straight
    // to `config.media_dir` with the caller's MIME string — on an S3/R2
    // deployment that produced a URL which 404s, and any file (a `.env`)
    // could be copied into the media store.
    let media = crate::api::media::store_upload_bytes(
        state,
        user_id,
        &input.filename,
        &input.mime_type,
        &data,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(Json(MediaUploadOutput {
        id: media.id.to_string(),
        url: media.url.clone(),
        mime_type: media.mime_type.clone(),
        filename: media.original_name.clone(),
        size: media.file_size,
        alt: media.alt.clone(),
    }))
}

/// Upload media directly from a file path (avoids base64 round-trip for CLI callers)
pub async fn upload_from_path(
    state: &AppState,
    file_path: &str,
) -> Result<Json<MediaUploadOutput>, String> {
    upload_from_path_with_alt(state, file_path, None).await
}

/// Upload media from a file path with optional alt text.
pub async fn upload_from_path_with_alt(
    state: &AppState,
    file_path: &str,
    alt: Option<&str>,
) -> Result<Json<MediaUploadOutput>, String> {
    let user_id = resolve_first_user(state).await?;

    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("Failed to read file metadata: {e}"))?;
    let file_size = metadata.len();

    if file_size > 50 * 1024 * 1024 {
        return Err("File too large (max 50 MB)".into());
    }

    let original_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("upload.bin")
        .to_string();

    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    };

    // Read the file and run it through the ONE upload path. A local file
    // that is not a media file is REFUSED by the sniff, so
    // `media upload ~/.switchpost/.env` no longer copies a secret into the
    // media store (and `GET /api/media/<uuid>` is a public route).
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let media =
        crate::api::media::store_upload_bytes(state, user_id, &original_name, mime, &bytes, alt)
            .await
            .map_err(|e| e.to_string())?;

    Ok(Json(MediaUploadOutput {
        id: media.id.to_string(),
        url: media.url.clone(),
        mime_type: media.mime_type.clone(),
        filename: media.original_name.clone(),
        size: media.file_size,
        alt: media.alt.clone(),
    }))
}

pub async fn list_media(
    state: &AppState,
    input: &MediaListInput,
) -> Result<Json<MediaListOutput>, String> {
    let user_id = resolve_first_user(state).await?;
    let limit = input.limit.unwrap_or(50).min(200);

    let entries =
        crate::db::queries::list_media(&state.db, user_id, limit, 0, input.search.as_deref())
            .await
            .map_err(|e| format!("DB error: {e}"))?;

    let total = entries.len() as i64;
    let media = entries
        .into_iter()
        .map(|e| MediaItem {
            id: e.id.to_string(),
            url: format!("/api/media/{}", e.id),
            filename: e.original_name,
            mime_type: e.mime_type,
            size: e.file_size,
            alt: e.alt_text,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(MediaListOutput { media, total }))
}

/// Upload media from a local file path via MCP (more efficient than base64 for large files).
/// Accepts an absolute file path and copies the file to the uploads directory.
pub async fn upload_from_path_mcp(
    state: &AppState,
    input: &MediaUploadFromPathInput,
) -> Result<Json<MediaUploadOutput>, String> {
    upload_from_path_with_alt(state, &input.path, input.alt.as_deref()).await
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadFromPathInput {
    /// Absolute path to the local file to upload
    pub path: String,
    /// Optional alt text for accessibility
    pub alt: Option<String>,
}

// ── Batch Upload ──────────────────────────────────────────────

/// Upload multiple media files from local paths in a single call.
/// Optimized for carousel workflows (5-10 images).
pub async fn upload_batch(
    state: &AppState,
    input: &MediaUploadBatchInput,
) -> Result<Json<MediaUploadBatchOutput>, String> {
    if input.paths.is_empty() {
        return Err("At least one file path is required".into());
    }
    if input.paths.len() > 20 {
        return Err("Maximum 20 files per batch upload".into());
    }

    let mut media_items = Vec::with_capacity(input.paths.len());
    let mut upload_errors = Vec::new();

    for path in &input.paths {
        match upload_from_path_with_alt(state, path, input.alt.as_deref()).await {
            Ok(Json(item)) => media_items.push(item),
            Err(e) => upload_errors.push(BatchUploadError {
                path: path.clone(),
                error: e,
            }),
        }
    }

    let total = input.paths.len() as i32;
    let succeeded = media_items.len() as i32;
    let failed = upload_errors.len() as i32;

    Ok(Json(MediaUploadBatchOutput {
        media: media_items,
        errors: upload_errors,
        total,
        succeeded,
        failed,
    }))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadBatchInput {
    /// List of absolute file paths to upload
    pub paths: Vec<String>,
    /// Optional alt text applied to all files
    pub alt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadBatchOutput {
    pub media: Vec<MediaUploadOutput>,
    pub errors: Vec<BatchUploadError>,
    pub total: i32,
    pub succeeded: i32,
    pub failed: i32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BatchUploadError {
    pub path: String,
    pub error: String,
}

// ── URL Upload ────────────────────────────────────────────────

/// Download media from an external URL and store it locally.
/// Useful when an AI agent has image URLs but needs them in the local media library.
pub async fn upload_from_url(
    state: &AppState,
    input: &MediaUploadFromUrlInput,
) -> Result<Json<MediaUploadOutput>, String> {
    let user_id = resolve_first_user(state).await?;

    // SSRF protection: validate URL scheme and block non-public IPs
    let parsed = url::Url::parse(&input.url).map_err(|e| format!("Invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "URL scheme '{other}' not allowed (use http or https)"
            ))
        }
    }
    if let Some(host) = parsed.host_str() {
        // Reject localhost aliases
        if host == "localhost" || host == "0.0.0.0" || host == "::1" {
            return Err("URLs pointing to localhost are not allowed".into());
        }
        // Parse as IP and check if private/reserved — stable Rust equivalent of is_global()
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            match ip {
                std::net::IpAddr::V4(v4) => {
                    if v4.is_private()
                        || v4.is_loopback()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_unspecified()
                    {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                    // 100.64.0.0/10 (Carrier-grade NAT)
                    let octets = v4.octets();
                    if octets[0] == 100 && (octets[1] & 0xC0) == 64 {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                    // 192.0.0.0/24 (IETF protocol assignments)
                    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                    // 198.18.0.0/15 (benchmarking)
                    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                }
                std::net::IpAddr::V6(v6) => {
                    if v6.is_loopback() || v6.is_unspecified() {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                    // fc00::/7 (unique local addresses)
                    if v6.octets()[0] == 0xfc || v6.octets()[0] == 0xfd {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                    // fe80::/10 (link-local)
                    if v6.octets()[0] == 0xfe && (v6.octets()[1] & 0xC0) == 0x80 {
                        return Err(format!(
                            "URLs pointing to private/reserved IPs ({ip}) are not allowed"
                        ));
                    }
                }
            }
        }
        // Block known cloud metadata hostnames
        if host == "metadata.google.internal" || host == "169.254.169.254" {
            return Err("URLs pointing to cloud metadata services are not allowed".into());
        }
    }

    // Download from URL using the shared HTTP client from AppState
    // Note: We build a one-off client to disable redirects, preventing SSRF bypass
    // where a 302 from a public URL redirects to a private IP.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(&input.url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;
    // If the server returned a redirect, follow it manually with SSRF re-validation
    let resp = if resp.status().is_redirection() {
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| "Redirect missing Location header".to_string())?;
        // Re-validate the redirect target against SSRF rules
        let redirect_url =
            url::Url::parse(location).map_err(|e| format!("Invalid redirect URL: {e}"))?;
        match redirect_url.scheme() {
            "http" | "https" => {}
            other => return Err(format!("Redirect to scheme '{other}' not allowed")),
        }
        if let Some(host) = redirect_url.host_str() {
            if host == "localhost" || host == "0.0.0.0" || host == "::1" {
                return Err("Redirect to localhost not allowed".into());
            }
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                match ip {
                    std::net::IpAddr::V4(v4) => {
                        if v4.is_private()
                            || v4.is_loopback()
                            || v4.is_link_local()
                            || v4.is_broadcast()
                            || v4.is_unspecified()
                        {
                            return Err(format!("Redirect to private IP ({ip}) not allowed"));
                        }
                    }
                    std::net::IpAddr::V6(v6) => {
                        if v6.is_loopback() || v6.is_unspecified() {
                            return Err(format!("Redirect to private IP ({ip}) not allowed"));
                        }
                    }
                }
            }
        }
        client
            .get(location)
            .send()
            .await
            .map_err(|e| format!("Failed to follow redirect: {e}"))?
    } else {
        resp
    };

    if !resp.status().is_success() {
        return Err(format!("URL returned HTTP {}", resp.status()));
    }

    // Reject oversized responses before downloading the body
    if let Some(content_length) = resp.content_length() {
        if content_length > 50 * 1024 * 1024 {
            return Err(format!(
                "File too large (max 50 MB, server reported {content_length} bytes)"
            ));
        }
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if bytes.len() > 50 * 1024 * 1024 {
        return Err("File too large (max 50 MB)".into());
    }

    // Determine filename from URL path (reuse already-parsed URL)
    let filename = input.filename.clone().unwrap_or_else(|| {
        parsed
            .path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                let ext = match content_type.as_str() {
                    "image/jpeg" => "jpg",
                    "image/png" => "png",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    "video/mp4" => "mp4",
                    _ => "bin",
                };
                format!("download.{ext}")
            })
    });

    let file_id = Uuid::new_v4();
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let stored_name = format!("{file_id}.{ext}");

    let upload_dir = std::path::Path::new(&state.config.media_dir);
    tokio::fs::create_dir_all(upload_dir)
        .await
        .map_err(|e| format!("Failed to create upload dir: {e}"))?;

    let filepath = upload_dir.join(&stored_name);
    tokio::fs::write(&filepath, &bytes)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;

    let entry = crate::db::queries::create_media(
        &state.db,
        user_id,
        &filename,
        &stored_name,
        &content_type,
        bytes.len() as i64,
        None,
        None,
        None,
    )
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(MediaUploadOutput {
        id: entry.id.to_string(),
        url: format!("/api/media/{}", entry.id),
        mime_type: content_type,
        filename,
        size: bytes.len() as i64,
        alt: entry.alt_text.clone(),
    }))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaUploadFromUrlInput {
    /// External URL to download media from
    pub url: String,
    /// Override filename (optional, inferred from URL if not set)
    pub filename: Option<String>,
    /// Optional alt text for accessibility
    pub alt: Option<String>,
}

use super::auth::resolve_first_user;
