// ─── Developer API Routes ──────────────────────────────────────
// API key management for programmatic access.
// Keys are SHA-256 hashed at rest; the raw key is returned only once at creation.

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::middleware::{hash_api_key, ApiKeyScope, AuthenticatedUser, API_KEY_PREFIX};
use crate::db::types::EpochUtc;
use crate::error::AppError;

use super::AppState;

// ── Database Row Type ─────────────────────────────────────────

#[derive(Debug, FromRow)]
struct ApiKeyRow {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub scopes: String,
}

// ── Request Types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub expires_at: Option<String>,
    /// `read` | `write` | `admin`. Absent means `admin`.
    pub scope: Option<String>,
}

// ── Response Types ────────────────────────────────────────────

/// Returned when listing keys — never includes the full key.
#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    /// The scope this key carries — `read`, `write` or `admin`.
    pub scope: String,
}

/// Returned at creation/regeneration — includes the full key once.
#[derive(Debug, Serialize)]
pub struct ApiKeyCreatedResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub full_key: String,
    pub created_at: String,
}

// ── Helpers ───────────────────────────────────────────────────

fn row_to_response(row: ApiKeyRow) -> ApiKeyResponse {
    ApiKeyResponse {
        id: row.id,
        name: row.name,
        key_prefix: row.key_prefix,
        last_used_at: row.last_used_at.map(|dt| dt.to_rfc3339()),
        expires_at: row.expires_at.map(|dt| dt.to_rfc3339()),
        is_active: row.is_active,
        created_at: row.created_at.to_rfc3339(),
        scope: row.scopes,
    }
}

/// Generate a new API key and return (full_key, prefix, sha256_hash).
///
/// The key is `sf_`-prefixed so clients can recognise it in a secret
/// store, and the digest comes from the shared `hash_api_key` — the same
/// helper `auth::middleware::verify_api_key` looks up with.
fn generate_api_key() -> (String, String, String) {
    let random = uuid::Uuid::new_v4().to_string().replace("-", "")
        + &uuid::Uuid::new_v4().to_string().replace("-", "");
    let full = format!("{API_KEY_PREFIX}{random}");
    // Display prefix comes from the random part — the `sf_` marker would
    // otherwise eat the first 3 of the 8 characters.
    let prefix = random[..8].to_string();
    let hash = hash_api_key(&full);
    (full, prefix, hash)
}

// ── Handlers ─────────────────────────────────────────────────

/// POST /api/developer/api-keys
/// Creates a new API key. Returns the full key ONCE in the response.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyCreatedResponse>, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("API key name is required".into()));
    }

    let expires_at = match body.expires_at {
        Some(ref s) if !s.trim().is_empty() => {
            let dt = DateTime::parse_from_rfc3339(s.trim())
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%dT%H:%M:%S")
                        .map(|n| n.and_utc())
                        .map(|u| u.into())
                })
                .map_err(|_| {
                    AppError::BadRequest("Invalid expires_at format — use RFC 3339".into())
                })?;
            Some(dt.with_timezone(&Utc))
        }
        _ => None,
    };

    // The scope is the founder's choice, and it is enforced in ONE place:
    // `auth::middleware::scope_allows`.
    let scope = match body.scope.as_deref().map(str::trim) {
        None | Some("") => ApiKeyScope::Admin,
        Some(raw) => {
            let parsed = ApiKeyScope::parse(raw);
            if !ApiKeyScope::all()
                .iter()
                .any(|s| s.as_str() == parsed.as_str())
                || parsed.as_str() != raw.to_ascii_lowercase()
            {
                return Err(AppError::BadRequest(format!(
                    "Unknown scope `{raw}`. Use read, write or admin."
                )));
            }
            parsed
        }
    };

    let (full_key, prefix, hash) = generate_api_key();

    let row: ApiKeyRow = sqlx::query_as(
        r#"
        INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, expires_at, scopes)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        RETURNING id, name, key_prefix, last_used_at, expires_at, is_active, created_at, scopes
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.user_id)
    .bind(&name)
    .bind(&prefix)
    .bind(&hash)
    // Epoch INTEGER column: a bare DateTime binds as TEXT, which SQLite orders
    // after every INTEGER, so `expires_at > unixepoch()` never expired the key.
    .bind(expires_at.map(EpochUtc::from))
    .bind(scope.as_str())
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ApiKeyCreatedResponse {
        id: row.id,
        name: row.name,
        key_prefix: row.key_prefix,
        full_key,
        created_at: row.created_at.to_rfc3339(),
    }))
}

/// GET /api/developer/api-keys
/// Lists the user's API keys. Never returns the full key.
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<Vec<ApiKeyResponse>>, AppError> {
    let rows: Vec<ApiKeyRow> = sqlx::query_as(
        r#"
        SELECT id, name, key_prefix, last_used_at, expires_at, is_active, created_at, scopes
        FROM api_keys
        WHERE user_id = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

/// DELETE /api/developer/api-keys/{id}
/// Revokes (soft-deletes) an API key by setting is_active = false.
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = sqlx::query("UPDATE api_keys SET is_active = false WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("API key not found".into()));
    }

    Ok(Json(serde_json::json!({"revoked": true})))
}

/// POST /api/developer/api-keys/{id}/regenerate
/// Generates a new key value, updates prefix + hash, returns new full key once.
pub async fn regenerate(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiKeyCreatedResponse>, AppError> {
    // Verify ownership first — only allow regeneration on active keys
    let existing: ApiKeyRow = sqlx::query_as(
        r#"
        SELECT id, name, key_prefix, last_used_at, expires_at, is_active, created_at, scopes
        FROM api_keys
        WHERE id = ? AND user_id = ? AND is_active = true
        "#,
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("API key not found".into()))?;

    let (full_key, prefix, hash) = generate_api_key();

    let row: ApiKeyRow = sqlx::query_as(
        r#"
        UPDATE api_keys
        SET key_prefix = ?, key_hash = ?
        WHERE id = ? AND user_id = ?
        RETURNING id, name, key_prefix, last_used_at, expires_at, is_active, created_at, scopes
        "#,
    )
    .bind(&prefix)
    .bind(&hash)
    .bind(id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ApiKeyCreatedResponse {
        id: row.id,
        name: existing.name,
        key_prefix: prefix,
        full_key,
        created_at: row.created_at.to_rfc3339(),
    }))
}
