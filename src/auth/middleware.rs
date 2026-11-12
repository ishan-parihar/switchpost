// ─── Auth Middleware (single-user password gate) ──────────────
//
// Switchpost is a local-deployment tool: one user, one password.
// There is no user-registration, no user table lookups, no per-user
// permissions. The WebUI is gated by a single password set via the
// `APP_PASSWORD` env var. After a successful POST /api/auth/login,
// the server issues an HttpOnly signed session cookie (`sf_session`)
// containing a JWT whose `sub` is always `DEFAULT_USER_ID`.
//
// Programmatic clients skip the password and present an issued API key
// as `Authorization: Bearer sf_…` instead. Both paths resolve to the
// same single user — a key is another credential, not another identity.
//
// The CLI and MCP stdio paths are local (shell access already
// implies trust) and bypass this gate — they call `resolve_first_user`
// which returns `DEFAULT_USER_ID` directly.

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, Method, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::jwt;
use sqlx::SqlitePool;

/// The single local user. All data in the DB is owned by this id.
/// Kept as a stable constant so existing rows survive restarts and
/// so FK constraints have a deterministic target.
pub const DEFAULT_USER_ID: Uuid = Uuid::from_u128(0x22222222_2222_2222_2222_222222222222);

/// Cookie name used for the session token.
pub const SESSION_COOKIE: &str = "sf_session";

/// Every issued API key starts with this marker. Programmatic clients
/// send `Authorization: Bearer sf_…`.
pub const API_KEY_PREFIX: &str = "sf_";

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
}

/// Shared state injected into `auth_middleware` via `from_fn_with_state`.
/// Carries the HMAC secret used to sign session cookies and the pool
/// used to look up API keys.
#[derive(Clone)]
pub struct AuthState {
    pub session_secret: String,
    pub db: SqlitePool,
}

/// The single SHA-256 scheme for API keys at rest. Shared by issuance
/// (`api::developer::generate_api_key`) and verification (below) so a
/// key can never be hashed one way and looked up another — do not add a
/// second hashing helper.
pub fn hash_api_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// What an API key is allowed to do.
///
/// One ordered vocabulary, enforced in ONE place ([`scope_allows`]) so the
/// three surfaces (REST, MCP over HTTP, the developer console) cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApiKeyScope {
    /// Safe methods only.
    Read,
    /// Read + state-changing requests on the content routes.
    Write,
    /// Everything except the API-key console itself.
    Admin,
}

impl ApiKeyScope {
    /// Parse a stored scope. An unknown value — a hand-edited row, a key
    /// written by a future version — is treated as `Admin`, the value
    /// migration 0009 gives every existing key, so an unreadable scope
    /// never quietly DISABLES a working key.
    pub fn parse(raw: &str) -> ApiKeyScope {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read" => ApiKeyScope::Read,
            "write" => ApiKeyScope::Write,
            _ => ApiKeyScope::Admin,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ApiKeyScope::Read => "read",
            ApiKeyScope::Write => "write",
            ApiKeyScope::Admin => "admin",
        }
    }

    /// The scopes the founder may choose from when minting a key.
    pub fn all() -> &'static [ApiKeyScope] {
        &[ApiKeyScope::Read, ApiKeyScope::Write, ApiKeyScope::Admin]
    }
}

impl std::fmt::Display for ApiKeyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// May a key with this scope make this request?
///
/// The rules, in order:
///
/// - a safe method is read-only, so every scope gets it;
/// - the API-key console (`/api/developer/api-keys`) needs the SESSION at
///   every level — a key must never be able to mint, list, revoke or
///   regenerate another key, because then revoking the leaked one does not
///   evict the attacker;
/// - the rest of `/api/developer` needs `admin`;
/// - a `read` key may not make a state-changing request anywhere.
pub fn scope_allows(scope: ApiKeyScope, method: &Method, path: &str) -> bool {
    // The key console is SESSION-ONLY at every level, including reads: a
    // key that can LIST the keys can learn their prefixes, and a key that
    // can REVOKE the others can lock the founder out of their own API.
    if path.starts_with("/api/developer/api-keys") {
        return false;
    }
    let safe = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if safe {
        return true;
    }
    if path.starts_with("/api/developer") {
        return scope == ApiKeyScope::Admin;
    }
    scope >= ApiKeyScope::Write
}

/// Verify a raw `sf_…` key against `api_keys`.
///
/// The database does the digest equality (`key_hash = ?`), so a Rust-side
/// constant-time compare would be redundant — and the compared value is
/// a 256-bit digest, never a usable credential, so a timing signal on
/// the digest does not leak the key.
///
/// `None` means unknown, revoked, or expired — all three are 401 to the
/// caller, so an attacker cannot tell them apart.
///
/// ## `last_used_at` is stamped at most once a minute
///
/// It used to be an `UPDATE … RETURNING user_id` on EVERY authenticated
/// request, so a read-only agent doing N tool calls performed N write
/// transactions on the single-writer SQLite file — and that serialises
/// against the scheduler's publish writes. The value has second-level
/// value at best; a minute is generous.
///
/// ponytail: no index on `key_hash` (migration 011 only indexes user_id).
/// Seq scan over a handful of keys is fine; add a unique index if an
/// operator ever holds thousands.
pub async fn verify_api_key(pool: &SqlitePool, raw_key: &str) -> Option<(Uuid, ApiKeyScope)> {
    let row = sqlx::query(
        r#"
        SELECT user_id, scopes, last_used_at
        FROM api_keys
        WHERE key_hash = ?
          AND is_active = true
          AND (expires_at IS NULL OR expires_at > unixepoch())
        "#,
    )
    .bind(hash_api_key(raw_key))
    .fetch_optional(pool)
    .await
    .ok()??;

    let user_id = row.try_get::<Uuid, _>("user_id").ok()?;
    let scope = row
        .try_get::<Option<String>, _>("scopes")
        .ok()
        .flatten()
        .map(|s| ApiKeyScope::parse(&s))
        .unwrap_or(ApiKeyScope::Admin);

    let now = chrono::Utc::now().timestamp();
    let stale: Option<i64> = row.try_get::<Option<i64>, _>("last_used_at").ok().flatten();
    if stale.is_none_or(|t| now - t >= 60) {
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE key_hash = ?")
            .bind(now)
            .bind(hash_api_key(raw_key))
            .execute(pool)
            .await;
    }

    Some((user_id, scope))
}

/// Pull the credential out of an `Authorization: Bearer <token>` header.
pub fn extract_bearer(header_value: &str) -> Option<&str> {
    let (scheme, value) = header_value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

/// 403 body: the key authenticated, but its scope does not cover this.
fn forbidden_scope(
    scope: ApiKeyScope,
    method: &Method,
    path: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": format!(
                "This API key has the `{scope}` scope, which does not permit {method} {path}. \
                 The API-key console also needs a session, never a key."
            ),
            "code": "insufficient_scope"
        })),
    )
}

/// 401 body shared by both auth paths.
fn unauthorized(code: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "Not authenticated", "code": code})),
    )
}

/// Authenticate a request by API key *or* session cookie, then inject
/// `AuthenticatedUser` for downstream handlers.
///
/// Two paths, checked in order:
///
/// 1. `Authorization: Bearer sf_…` — a key issued by
///    `POST /api/developer/api-keys`, verified against `api_keys`.
/// 2. `sf_session` cookie — the WebUI path, validated against the JWT
///    secret. Unchanged.
///
/// A present-but-rejected `Authorization` header short-circuits to 401
/// rather than falling through to the cookie path: silently downgrading a
/// bad key to a session would mask the credential error.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
        .map(str::to_owned);

    if let Some(bearer) = bearer {
        let verified = if bearer.starts_with(API_KEY_PREFIX) {
            verify_api_key(&auth.db, &bearer).await
        } else {
            None
        };
        let Some((user_id, scope)) = verified else {
            return Err(unauthorized("invalid_api_key"));
        };
        // A key is only as powerful as its scope. This is the one place the
        // rule lives: REST, MCP-over-HTTP and any future surface all come
        // through here, so they cannot drift apart.
        if !scope_allows(scope, req.method(), req.uri().path()) {
            return Err(forbidden_scope(scope, req.method(), req.uri().path()));
        }
        req.extensions_mut().insert(AuthenticatedUser { user_id });
        return Ok(next.run(req).await);
    }

    let cookie_header = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = extract_cookie(cookie_header, SESSION_COOKIE);
    let user_id = match token.and_then(|t| jwt::validate_token(t, &auth.session_secret).ok()) {
        Some(claims) => Uuid::parse_str(&claims.sub).unwrap_or(DEFAULT_USER_ID),
        None => return Err(unauthorized("no_session")),
    };

    req.extensions_mut().insert(AuthenticatedUser { user_id });
    Ok(next.run(req).await)
}

/// Parse `name=value` out of a `Cookie:` header value.
pub fn extract_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    header
        .split(';')
        .map(|p| p.trim())
        .find_map(|p| p.strip_prefix(&prefix))
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Not authenticated"})),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the at-rest scheme to SHA-256 hex. If someone swaps this for
    /// another digest, existing rows stop verifying and this test says why.
    #[test]
    fn hash_api_key_matches_known_sha256_vectors() {
        assert_eq!(
            hash_api_key("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_api_key(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// The property `verify_api_key` relies on: two keys collide only when
    /// they are byte-identical, so a one-character difference never
    /// verifies against a stored digest.
    #[test]
    fn hash_api_key_distinguishes_keys_that_differ_by_one_character() {
        let stored = hash_api_key("sf_0123456789abcdef0123456789abcdef");

        assert_ne!(
            stored,
            hash_api_key("sf_0123456789abcdef0123456789abcdee"),
            "a one-character key change must not produce the stored digest"
        );
        assert_eq!(
            stored,
            hash_api_key("sf_0123456789abcdef0123456789abcdef"),
            "the same key must always produce the same digest"
        );
    }

    #[test]
    fn extract_bearer_returns_the_token() {
        assert_eq!(
            extract_bearer("Bearer sf_abc123"),
            Some("sf_abc123"),
            "the sf_ credential should be returned verbatim"
        );
    }

    #[test]
    fn extract_bearer_rejects_other_schemes_and_empty_tokens() {
        assert_eq!(extract_bearer("Basic dXNlcjpwYXNz"), None);
        assert_eq!(extract_bearer("Bearer "), None);
        assert_eq!(extract_bearer(""), None);
    }
}
