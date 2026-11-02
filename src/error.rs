// ─── Unified Error Types ───────────────────────────────────────
// All errors in the system flow through AppError.
// Each variant maps to an HTTP status code for the API layer.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// A token store failed its at-rest encryption (or the DB write). Never
    /// silently downgrades to plaintext — audit B8.
    #[error("{0}")]
    TokenUpdate(#[from] crate::db::queries::TokenUpdateError),

    #[error("Auth error: {0}")]
    Auth(#[from] jsonwebtoken::errors::Error),

    #[error("Hashing error: {0}")]
    Hash(String),

    #[error("Provider error: {0}")]
    Provider(String),

    /// A channel connect was refused because of something the FOUNDER can
    /// change: a phone number in the wrong format, a bot token Telegram does
    /// not recognise, a paste with no session cookie, env vars that were
    /// never set.
    ///
    /// ─── WHY THIS IS NOT `Provider` ─────────────────────────────
    /// `AppError::Provider` maps to HTTP 502 BAD GATEWAY, which means "this
    /// server could not reach the upstream". It is the right status for a
    /// provider outage and the WRONG one for a typo.
    ///
    /// The status is not cosmetic. A reverse proxy in front of the app
    /// (Caddy/nginx, per docs/deployment.md) replaces the BODY of a 502 with
    /// its own HTML error page — so the actionable message this handler wrote
    /// never reaches the browser. The founder then sees
    ///
    ///   "Expected JSON from /api/integrations/connect/telegram-user/
    ///    request-code but got 502 (received HTML, not JSON — the route
    ///    likely does not exist or the server is behind a proxy error page)"
    ///
    /// which names a MISSING ROUTE for a route that exists and a phone
    /// number that is simply invalid. The WhatsApp pair route and the
    /// Telegram request-code route both produced exactly this report.
    ///
    /// 400 (Bad request) survives every proxy unrewritten and reaches the
    /// dialog the founder is looking at.
    #[error("{0}")]
    ConnectFailed(String),

    /// The stored OAuth grant no longer carries the scopes this integration
    /// needs (a revoked scope, a downgraded app review, an admin that pulled
    /// the permission). Retrying the same token cannot succeed, so the
    /// operator has to re-connect the account — hence 409, not 502.
    #[error("{provider} lost the OAuth scopes this integration needs (integration {integration_id}): {detail} Reconnect the account on the Channels screen to re-authorize.")]
    ScopeLost {
        provider: String,
        integration_id: String,
        detail: String,
    },

    #[error("Token expired")]
    TokenExpired,

    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<argon2::password_hash::Error> for AppError {
    fn from(e: argon2::password_hash::Error) -> Self {
        AppError::Hash(e.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<crate::social::ProviderError> for AppError {
    fn from(e: crate::social::ProviderError) -> Self {
        match e {
            crate::social::ProviderError::Auth(m) => AppError::Unauthorized(m),
            crate::social::ProviderError::Api(m) => AppError::Provider(m),
            crate::social::ProviderError::TokenExpired => AppError::TokenExpired,
            crate::social::ProviderError::RateLimited(m) => AppError::RateLimited(m),
            crate::social::ProviderError::InvalidRequest(m) => AppError::BadRequest(m),
            // Not a failure. A channel on this provider IS the account —
            // there is no page/sub-account to pick. `pages()` answer.
            crate::social::ProviderError::NoPageManagement => AppError::BadRequest(
                "This provider has no page or sub-account concept. The channel is the account."
                    .to_string(),
            ),
            crate::social::ProviderError::Network(e) => {
                AppError::Internal(format!("Provider network error: {e}"))
            }
        }
    }
}

// ── OAuth scope-loss detection ─────────────────────────────────
//
// Providers answer a lost grant in their own words ("missing a required API
// scope", "Check token scopes.", `insufficient_scope`). This is the one place
// that knows those words, so the API boundary and the scheduler cannot drift.

/// Markers that identify a lost/revoked OAuth grant in a provider message.
///
/// Deliberately narrow. A bare HTTP 403 must NOT match: several providers
/// answer 403 for things a re-connect cannot fix (app still in review, wrong
/// account type, hit the posting limit), and calling those "reconnect me"
/// would be a lie. The word "scope" is safe because providers only use it when
/// they name what the grant is missing.
const SCOPE_LOSS_MARKERS: &[&str] = &[
    // RFC 6749 §4.1.2.1 codes plus the prose copy that names the scope.
    "scope",
    "insufficient_permission",
    "insufficient permission",
    "missing permission",
    "lacks permission",
];

/// True when a provider failure reads as a lost/revoked OAuth grant rather
/// than a transient API fault.
pub fn looks_like_scope_loss(msg: &str) -> bool {
    let msg = msg.to_ascii_lowercase();
    SCOPE_LOSS_MARKERS.iter().any(|m| msg.contains(m))
}

/// Map a provider failure to [`AppError::ScopeLost`] (409) when the message
/// says the grant is missing scopes; otherwise keep the plain provider error so
/// every non-scope path is byte-identical to before.
pub fn map_scope_loss(provider: &str, integration_id: &str, msg: &str) -> AppError {
    if looks_like_scope_loss(msg) {
        AppError::ScopeLost {
            provider: provider.to_string(),
            integration_id: integration_id.to_string(),
            detail: msg.to_string(),
        }
    } else {
        AppError::Provider(msg.to_string())
    }
}

/// Add the re-connect hint to a provider message for the scheduler's log /
/// `post.error_message` / `post.failed` webhook, which have no HTTP status to
/// carry. Non-scope messages are returned unchanged.
pub fn annotate_scope_loss(provider: &str, msg: &str) -> String {
    if looks_like_scope_loss(msg) {
        format!("{msg} Reconnect '{provider}' to re-grant the missing scopes.")
    } else {
        msg.to_string()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message, code) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone(), "not_found"),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone(), "unauthorized"),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone(), "bad_request"),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone(), "conflict"),
            AppError::Database(e) => {
                tracing::error!("Database error: {:?}", e);
                // Include a stable error code so the frontend can branch on it
                // without parsing the human-readable message. We do NOT leak
                // SQL details — just the sqlx::Error discriminant.
                //
                // Note: sqlx::Error variants vary slightly across versions.
                // We list the ones that exist in sqlx 0.8 and use a fallback
                // for everything else. Pool-related errors are represented
                // by PoolTimedOut / PoolClosed (there is no generic `Pool`
                // variant in sqlx::Error itself — pool errors come through
                // these specific variants or as Database/Configuration).
                let code = match e {
                    sqlx::Error::RowNotFound => "db_row_not_found",
                    sqlx::Error::TypeNotFound { .. } => "db_type_not_found",
                    sqlx::Error::ColumnNotFound(_) => "db_column_not_found",
                    sqlx::Error::ColumnDecode { .. } => "db_column_decode",
                    sqlx::Error::Decode(_) => "db_decode",
                    sqlx::Error::PoolTimedOut => "db_pool_timed_out",
                    sqlx::Error::PoolClosed => "db_pool_closed",
                    sqlx::Error::WorkerCrashed => "db_worker_crashed",
                    sqlx::Error::Database(_) => "db_database",
                    sqlx::Error::Io(_) => "db_io",
                    sqlx::Error::Tls(_) => "db_tls",
                    sqlx::Error::Protocol(_) => "db_protocol",
                    sqlx::Error::Configuration(_) => "db_configuration",
                    _ => "db_unknown",
                };
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".into(),
                    code,
                )
            }
            AppError::Auth(e) => {
                tracing::error!("Auth error: {:?}", e);
                (StatusCode::UNAUTHORIZED, "Invalid token".into(), "auth")
            }
            AppError::Hash(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Hashing error".into(),
                "hash",
            ),
            AppError::Provider(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), "provider"),
            // A connect the founder can fix themselves is a 400, not a 502 —
            // see the variant's doc comment for the proxy-body reason.
            AppError::ConnectFailed(msg) => {
                (StatusCode::BAD_REQUEST, msg.clone(), "connect_failed")
            }
            // `self.to_string()` keeps the Display impl the single source of
            // truth for the human message (provider + integration id + hint).
            AppError::ScopeLost { .. } => (StatusCode::CONFLICT, self.to_string(), "scope_lost"),
            AppError::TokenExpired => (
                StatusCode::UNAUTHORIZED,
                "Token expired".into(),
                "token_expired",
            ),
            // B8: a failed token store (encrypt or DB write) is an internal
            // error — the client must retry after the operator fixes the
            // key/DB; the message is logged server-side.
            AppError::TokenUpdate(e) => {
                tracing::error!("Token store failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Token store failed".into(),
                    "token_update",
                )
            }
            AppError::RateLimited(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, msg.clone(), "rate_limited")
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal error".into(),
                    "internal",
                )
            }
        };

        let mut body = json!({ "error": message, "code": code });
        if let AppError::ScopeLost {
            provider,
            integration_id,
            ..
        } = &self
        {
            body["provider"] = json!(provider);
            body["integration_id"] = json!(integration_id);
            body["reconnect"] = json!(format!("/api/integrations/connect/{provider}"));
        }

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real provider copy (src/social/youtube.rs) — no live OAuth needed.
    const YOUTUBE_SCOPE_COPY: &str =
        "Publish failed: API error: YouTube denied this request (HTTP 403). \
         The token is missing the youtube.upload scope, or the channel is not \
         set up to accept uploads.";

    #[test]
    fn map_scope_loss_returns_409_for_a_lost_grant() {
        let resp = map_scope_loss("youtube", "0d5f-1", YOUTUBE_SCOPE_COPY).into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn map_scope_loss_message_names_provider_integration_and_hint() {
        let msg = map_scope_loss("youtube", "0d5f-1", YOUTUBE_SCOPE_COPY).to_string();
        assert!(msg.contains("youtube"), "{msg}");
        assert!(msg.contains("0d5f-1"), "{msg}");
        assert!(msg.contains("Reconnect"), "{msg}");
    }

    #[test]
    fn map_scope_loss_keeps_non_scope_failures_on_the_provider_path() {
        // 403 that re-connecting cannot fix must stay a 502, not become a 409.
        let err = map_scope_loss(
            "facebook",
            "0d5f-2",
            "Facebook denied this request (HTTP 403). The app is pending App Review.",
        );
        assert!(matches!(err, AppError::Provider(_)));
    }

    #[test]
    fn annotate_scope_loss_appends_the_hint_only_for_scope_failures() {
        assert_eq!(
            annotate_scope_loss("x", "API error: rate limited"),
            "API error: rate limited"
        );
    }
}
