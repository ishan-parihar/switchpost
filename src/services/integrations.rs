// ─── Integration Service ──────────────────────────────────────
// Shared business logic for OAuth flow and integration management.
// Used by both `api/integrations.rs` (HTTP) and `mcp/tools_integrations.rs` (MCP).

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::models::Integration;
use crate::db::queries;
use crate::realtime::Broadcaster;
use crate::social::common;
use crate::social::registry::ProviderRegistry;

/// Result type for service operations
pub type ServiceResult<T> = Result<T, String>;

/// OAuth initiate result
pub struct OAuthInitiateResult {
    pub auth_url: String,
    pub state: String,
    pub code_verifier: String,
    /// The redirect URI the provider's developer app must have registered.
    /// Returned so a caller can report it when the provider answers
    /// `redirect_uri_mismatch` — the URI is otherwise invisible.
    pub redirect_uri: String,
}

/// Pack the user id and the redirect URI into the one string the
/// `oauth_states.redirect_uri` column carries.
///
/// The callback needs BOTH: the user to attach the integration to, and
/// the exact redirect URI the provider must see during the code exchange.
/// The URL may itself contain colons, so the user id is the FIRST segment
/// and everything after it is the URL.
pub fn pack_oauth_redirect(user_id: Uuid, redirect_uri: &str) -> String {
    format!("{user_id}:{redirect_uri}")
}

/// Unpack what `pack_oauth_redirect` wrote.
///
/// The CLI once stored the bare URL here, so the callback parsed the
/// whole string as a user id and every CLI-started OAuth flow died with
/// "Invalid user ID" before the code exchange. This is the one rule.
pub fn unpack_oauth_redirect(stored: &str) -> Result<(Uuid, String), String> {
    let parts: Vec<&str> = stored.split(':').collect();
    let user_id = Uuid::parse_str(parts.first().ok_or("Invalid state format")?)
        .map_err(|e| format!("Invalid user ID: {e}"))?;
    let redirect_uri = if parts.len() > 1 {
        parts[1..].join(":")
    } else {
        String::new()
    };
    Ok((user_id, redirect_uri))
}

/// Integration service
pub struct IntegrationService;

impl IntegrationService {
    /// Initiate OAuth flow for a provider
    pub async fn initiate_connect(
        db: &SqlitePool,
        registry: &ProviderRegistry,
        user_id: Uuid,
        provider_identifier: &str,
        app_url: &str,
    ) -> ServiceResult<OAuthInitiateResult> {
        let provider = registry
            .get(provider_identifier)
            .ok_or_else(|| format!("Unknown provider: {provider_identifier}"))?;

        let state = common::generate_state();
        let code_verifier = common::generate_code_verifier();

        let redirect_uri = if provider.uses_oauth() {
            format!("{app_url}/api/auth/callback")
        } else {
            format!("{app_url}/api/auth/connect")
        };

        let auth_response = provider
            .generate_auth_url(&state, &code_verifier, &redirect_uri)
            .await
            .map_err(|e| format!("Failed to generate auth URL: {e}"))?;

        // Save OAuth state
        queries::save_oauth_state(
            db,
            &state,
            provider_identifier,
            &code_verifier,
            Some(&pack_oauth_redirect(user_id, &redirect_uri)),
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

        Ok(OAuthInitiateResult {
            auth_url: auth_response.url,
            state,
            code_verifier,
            redirect_uri,
        })
    }

    /// Complete OAuth exchange from a callback state + code.
    /// Extracts provider_identifier and user_id from the stored OAuth state.
    /// Encrypts tokens at rest when token_key is configured.
    pub async fn complete_connect(
        db: &SqlitePool,
        registry: &ProviderRegistry,
        broadcaster: &Broadcaster,
        state: &str,
        code: &str,
        token_key: Option<&[u8; 32]>,
    ) -> ServiceResult<Integration> {
        // Verify state and get stored verifier
        let oauth_state = queries::get_oauth_state(db, state)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| "Invalid or expired OAuth state".to_string())?;

        let provider_identifier = &oauth_state.provider;

        let provider = registry
            .get(provider_identifier)
            .ok_or_else(|| format!("Unknown provider: {provider_identifier}"))?;

        // Parse user_id and redirect_uri from stored redirect_uri
        let stored_uri = oauth_state.redirect_uri.as_deref().unwrap_or("");
        let (user_id, redirect_uri) = unpack_oauth_redirect(stored_uri)?;

        let token = provider
            .exchange_code(code, &oauth_state.code_verifier, &redirect_uri)
            .await
            .map_err(|e| format!("Token exchange failed: {e}"))?;

        // Delete the used state
        if let Err(e) = queries::delete_oauth_state(db, state).await {
            tracing::warn!("DB operation failed: {e}");
        }

        // Encrypt tokens before storing if encryption key is configured
        let enc_access_token = match token_key {
            Some(key) => crate::crypto::encrypt_string(&token.access_token, key)
                .unwrap_or_else(|_| token.access_token.clone()),
            None => token.access_token.clone(),
        };
        let enc_refresh_token = token
            .refresh_token
            .as_ref()
            .and_then(|rt| token_key.and_then(|key| crate::crypto::encrypt_string(rt, key).ok()));

        // Create or update integration
        let integration = queries::create_integration(
            db,
            user_id,
            provider_identifier,
            provider.name(),
            &token.provider_user_id,
            &enc_access_token,
            enc_refresh_token
                .as_deref()
                .or(token.refresh_token.as_deref()),
            token
                .expires_in
                .map(|exp| Utc::now() + chrono::Duration::seconds(exp as i64)),
            Some(&token.name),
            token.picture.as_deref(),
            None,
            None, // root_internal_id — set by pages API for sub-accounts
            None, // auth_method
            token_key.copied(),
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

        broadcaster.send(
            "integration_connected",
            &serde_json::json!({
                "id": integration.id.to_string(),
                "provider": provider_identifier,
            }),
        );

        Ok(integration)
    }

    /// List all integrations for a user
    pub async fn list(db: &SqlitePool, user_id: Uuid) -> ServiceResult<Vec<Integration>> {
        queries::list_integrations(db, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))
    }

    /// Disconnect (delete) an integration
    pub async fn disconnect(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        user_id: Uuid,
        integration_id: Uuid,
    ) -> ServiceResult<bool> {
        let deleted = queries::delete_integration(db, integration_id, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        if deleted {
            broadcaster.send(
                "integration_disconnected",
                &serde_json::json!({"id": integration_id.to_string()}),
            );
        }

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The oauth_states.redirect_uri column carries "<user_id>:<url>".
    // The URL itself contains colons ("https://host:6544/..."), so the
    // user id is the FIRST segment and the rest is the URL. A caller
    // that stores the bare URL makes every callback fail with
    // "Invalid user ID" and the code exchange never runs.
    #[test]
    fn oauth_redirect_round_trips_through_the_state_column() {
        let user_id = Uuid::new_v4();
        let url = "https://localhost:6544/api/auth/callback";
        let packed = pack_oauth_redirect(user_id, url);
        let (parsed_id, parsed_url) = unpack_oauth_redirect(&packed).expect("must unpack");
        assert_eq!(parsed_id, user_id);
        assert_eq!(parsed_url, url);
    }

    #[test]
    fn a_bare_url_in_the_state_column_is_rejected() {
        // The exact shape the CLI used to store.
        let err = unpack_oauth_redirect("https://localhost:6544/api/auth/callback")
            .expect_err("a bare URL is not a state");
        assert!(err.starts_with("Invalid user ID"), "got: {err}");
    }
}

// ══════════════════════════════════════════════════════════════
// THE ONE REFRESH PATH (v12)
// ══════════════════════════════════════════════════════════════

/// How long a Meta page token lasts.
///
/// A page token derived from a long-lived user token inherits the user
/// token's life: 60 days. Storing the REAL expiry is what lets the
/// pre-publish refresh fire — `token_expires_at = None` meant the token
/// was never refreshed, so a page channel died silently the day its token
/// expired.
const META_PAGE_TOKEN_TTL_DAYS: i64 = 60;

/// The expiry of a page token derived from a Meta OAuth exchange.
///
/// Meta reports no `expires_in` for a page token, and `None` told the
/// scheduler "never refresh" — so a page channel died the day its token
/// expired, with no prompt and no way back except disconnecting. Storing
/// the REAL life of a page token (60 days, inherited from the long-lived
/// user token it comes from) is what lets the pre-publish refresh fire
/// while the parent's refresh token is still good.
///
/// `None` for every other provider: they either report a real expiry from
/// the API or have no page sub-accounts.
pub fn meta_page_token_expiry(provider_identifier: &str) -> Option<chrono::DateTime<Utc>> {
    match provider_identifier {
        "facebook" | "instagram" | "instagram_standalone" | "threads" => {
            Some(Utc::now() + chrono::Duration::days(META_PAGE_TOKEN_TTL_DAYS))
        }
        _ => None,
    }
}

/// What a refresh produced.
pub struct RefreshOutcome {
    /// The token to use for the next API call.
    ///
    /// For a sub-account channel this is the PAGE token, not the user
    /// token the refresh returned.
    pub access_token: String,
    /// The refresh token to keep.
    pub refresh_token: Option<String>,
    /// When the access token expires.
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

/// Refresh a channel's credentials, preserving the sub-account contract.
///
/// ## The bug this closes
///
/// A Facebook/Instagram channel connected to a PAGE stores a PAGE token and
/// the parent's USER token as its refresh token. Refreshing the user token
/// and writing the result over the PAGE token is what the old refresh path
/// did — and it silently changed WHERE posts went: with a user token, the
/// provider resolves "the first page the token can manage", which is not
/// necessarily the page the founder connected. Every scheduled post then
/// went to the wrong page (or failed outright).
///
/// ## The rule
///
/// - A channel that is NOT a sub-account (no `root_internal_id`) stores the
///   refreshed token as-is.
/// - A channel that IS a sub-account stores the PAGE token, re-derived from
///   the refreshed user token, and keeps the user token as the refresh
///   token. If the page cannot be re-derived, the EXISTING page token is
///   kept — a failed refresh must never replace a working credential with
///   a different one.
pub async fn refresh_channel_credentials(
    db: &SqlitePool,
    provider: &dyn crate::social::SocialProvider,
    integration: &Integration,
    token_key: Option<[u8; 32]>,
) -> Result<RefreshOutcome, String> {
    let encrypted_refresh = integration
        .refresh_token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "The {} channel has no refresh token — reconnect it.",
                integration.provider_identifier
            )
        })?;
    let refresh_token = crate::crypto::maybe_decrypt_token(encrypted_refresh, token_key.as_ref());

    let token = provider
        .refresh_token(&refresh_token)
        .await
        .map_err(|e| format!("Token refresh failed: {e}"))?;

    // A provider that rotates the refresh token hands back a new one; one
    // that does not leaves it alone.
    let stored_refresh = token
        .refresh_token
        .clone()
        .or_else(|| Some(refresh_token.clone()));

    // ── A sub-account channel: re-derive the PAGE token ──────────
    //
    // `root_internal_id` is `TEXT DEFAULT ''` — so every row written
    // before the column was populated carries an EMPTY STRING, not NULL.
    // `is_some()` would call every legacy channel a sub-account and send
    // its refresh down the page path. The value has to be non-empty.
    let is_sub_account = integration
        .root_internal_id
        .as_deref()
        .map(|r| !r.trim().is_empty())
        .unwrap_or(false);
    if is_sub_account {
        let pages = provider
            .pages(&token.access_token)
            .await
            .unwrap_or_default();
        let page = pages.iter().find(|p| p.id == integration.internal_id);
        match page.and_then(|p| p.access_token.clone()) {
            Some(page_token) => {
                let expires_at = Utc::now() + chrono::Duration::days(META_PAGE_TOKEN_TTL_DAYS);
                queries::update_integration_token(
                    db,
                    integration.id,
                    &page_token,
                    stored_refresh.as_deref(),
                    Some(expires_at),
                    token_key,
                )
                .await
                .map_err(|e| format!("Failed to store the refreshed token: {e}"))?;
                return Ok(RefreshOutcome {
                    access_token: page_token,
                    refresh_token: stored_refresh,
                    expires_at: Some(expires_at),
                });
            }
            None => {
                // The page is not reachable with the refreshed user token.
                // KEEP the page token that is already stored — it may still
                // work — and record the new user token as the refresh
                // token. Never replace a working page token with a user
                // token: that is the wrong-destination bug.
                tracing::warn!(
                    "Could not re-derive the page token for channel {} (page {}); keeping the \\
                     stored page token",
                    integration.id,
                    integration.internal_id
                );
                let expires_at = integration.token_expires_at;
                queries::update_integration_token(
                    db,
                    integration.id,
                    &crate::crypto::maybe_decrypt_token(
                        &integration.access_token,
                        token_key.as_ref(),
                    ),
                    stored_refresh.as_deref(),
                    expires_at,
                    token_key,
                )
                .await
                .map_err(|e| format!("Failed to store the refreshed token: {e}"))?;
                return Ok(RefreshOutcome {
                    access_token: crate::crypto::maybe_decrypt_token(
                        &integration.access_token,
                        token_key.as_ref(),
                    ),
                    refresh_token: stored_refresh,
                    expires_at,
                });
            }
        }
    }

    // ── A plain channel: the refreshed token IS the channel token ──
    let expires_at = token
        .expires_in
        .map(|e| Utc::now() + chrono::Duration::seconds(e as i64));
    queries::update_integration_token(
        db,
        integration.id,
        &token.access_token,
        token.refresh_token.as_deref(),
        expires_at,
        token_key,
    )
    .await
    .map_err(|e| format!("Failed to store the refreshed token: {e}"))?;

    Ok(RefreshOutcome {
        access_token: token.access_token,
        refresh_token: stored_refresh,
        expires_at,
    })
}

#[cfg(test)]
mod v12_refresh_tests {
    use super::*;
    use crate::social::{AuthToken, PageInfo, PostContent, ProviderError, SocialProvider};

    /// A provider shaped like Meta: the refresh returns a USER token, and
    /// the pages carry their own PAGE tokens.
    struct FakeMeta {
        page_token: String,
    }

    #[async_trait::async_trait]
    impl SocialProvider for FakeMeta {
        fn identifier(&self) -> &'static str {
            "facebook"
        }
        fn name(&self) -> &'static str {
            "Facebook"
        }
        fn scopes(&self) -> Vec<String> {
            vec!["pages_manage_posts".into()]
        }
        fn max_content_length(&self) -> usize {
            63206
        }
        async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
            Ok(AuthToken {
                access_token: "fresh-user-token".into(),
                refresh_token: Some("fresh-user-refresh".into()),
                expires_in: Some(3600),
                provider_user_id: "user-1".into(),
                name: "Founder".into(),
                username: "founder".into(),
                picture: None,
            })
        }
        async fn publish(
            &self,
            _access_token: &str,
            _post: &PostContent,
        ) -> Result<crate::social::PublishResult, ProviderError> {
            unreachable!("not used")
        }
        async fn generate_auth_url(
            &self,
            _state: &str,
            _code_verifier: &str,
            _redirect_uri: &str,
        ) -> Result<crate::social::AuthUrlResponse, ProviderError> {
            unreachable!("not used")
        }
        async fn exchange_code(
            &self,
            _code: &str,
            _code_verifier: &str,
            _redirect_uri: &str,
        ) -> Result<AuthToken, ProviderError> {
            unreachable!("not used")
        }
        async fn fetch_page_info(
            &self,
            _access_token: &str,
            _page_id: &str,
        ) -> Result<PageInfo, ProviderError> {
            unreachable!("not used")
        }
        async fn pages(&self, _access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
            Ok(vec![PageInfo {
                id: "page-1".into(),
                name: "Page One".into(),
                access_token: Some(self.page_token.clone()),
                picture: None,
                username: None,
            }])
        }
    }

    async fn fixture() -> (SqlitePool, Uuid) {
        let pool = crate::db::create_pool("sqlite::memory:").await.unwrap();
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        crate::db::ensure_local_user(&pool).await.unwrap();
        (pool, user_id)
    }

    async fn seed(pool: &SqlitePool, user_id: Uuid, root: Option<&str>) -> Integration {
        queries::create_integration(
            pool,
            user_id,
            "facebook",
            "Facebook",
            "page-1",
            "stored-page-token",
            Some("user-refresh-token"),
            None,
            None,
            None,
            None,
            root,
            None,
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_page_channel_keeps_its_page_token_after_a_refresh() {
        let (pool, user_id) = fixture().await;
        let integration = seed(&pool, user_id, Some("parent-1")).await;
        let provider = FakeMeta {
            page_token: "derived-page-token".into(),
        };

        let outcome = refresh_channel_credentials(&pool, &provider, &integration, None)
            .await
            .expect("refresh");

        assert_eq!(
            outcome.access_token, "derived-page-token",
            "a page channel must keep a PAGE token, not the user token the refresh returned"
        );
        assert_eq!(outcome.refresh_token.as_deref(), Some("fresh-user-refresh"));

        let stored = queries::get_integration(&pool, integration.id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.access_token, "derived-page-token");
        assert_eq!(stored.refresh_token.as_deref(), Some("fresh-user-refresh"));
    }

    #[tokio::test]
    async fn a_plain_channel_stores_the_refreshed_token() {
        let (pool, user_id) = fixture().await;
        let integration = seed(&pool, user_id, None).await;
        let provider = FakeMeta {
            page_token: "derived-page-token".into(),
        };

        let outcome = refresh_channel_credentials(&pool, &provider, &integration, None)
            .await
            .expect("refresh");

        assert_eq!(outcome.access_token, "fresh-user-token");
        let stored = queries::get_integration(&pool, integration.id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.access_token, "fresh-user-token");
    }

    #[tokio::test]
    async fn a_page_that_cannot_be_re_derived_keeps_the_stored_page_token() {
        let (pool, user_id) = fixture().await;
        let integration = seed(&pool, user_id, Some("parent-1")).await;
        // The provider lists a DIFFERENT page, so the connected page's token
        // cannot be re-derived.
        struct OtherPage;
        #[async_trait::async_trait]
        impl SocialProvider for OtherPage {
            fn identifier(&self) -> &'static str {
                "facebook"
            }
            fn name(&self) -> &'static str {
                "Facebook"
            }
            fn scopes(&self) -> Vec<String> {
                vec![]
            }
            fn max_content_length(&self) -> usize {
                63206
            }
            async fn refresh_token(&self, _r: &str) -> Result<AuthToken, ProviderError> {
                Ok(AuthToken {
                    access_token: "fresh-user-token".into(),
                    refresh_token: None,
                    expires_in: Some(3600),
                    provider_user_id: "user-1".into(),
                    name: "Founder".into(),
                    username: "founder".into(),
                    picture: None,
                })
            }
            async fn publish(
                &self,
                _t: &str,
                _p: &PostContent,
            ) -> Result<crate::social::PublishResult, ProviderError> {
                unreachable!("not used")
            }
            async fn generate_auth_url(
                &self,
                _state: &str,
                _code_verifier: &str,
                _redirect_uri: &str,
            ) -> Result<crate::social::AuthUrlResponse, ProviderError> {
                unreachable!("not used")
            }
            async fn exchange_code(
                &self,
                _code: &str,
                _code_verifier: &str,
                _redirect_uri: &str,
            ) -> Result<AuthToken, ProviderError> {
                unreachable!("not used")
            }
            async fn fetch_page_info(
                &self,
                _access_token: &str,
                _page_id: &str,
            ) -> Result<PageInfo, ProviderError> {
                unreachable!("not used")
            }
            async fn pages(&self, _t: &str) -> Result<Vec<PageInfo>, ProviderError> {
                Ok(vec![PageInfo {
                    id: "some-other-page".into(),
                    name: "Other".into(),
                    access_token: Some("other-page-token".into()),
                    picture: None,
                    username: None,
                }])
            }
        }

        let outcome = refresh_channel_credentials(&pool, &OtherPage, &integration, None)
            .await
            .expect("refresh");

        assert_eq!(
            outcome.access_token, "stored-page-token",
            "a failed re-derivation must keep the working page token"
        );
        let stored = queries::get_integration(&pool, integration.id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.access_token, "stored-page-token");
    }

    #[test]
    fn a_meta_page_token_gets_a_real_expiry() {
        let expiry = meta_page_token_expiry("facebook").expect("facebook pages expire");
        let days = (expiry - Utc::now()).num_days();
        assert!((59..=60).contains(&days), "got {days} days");
        assert!(meta_page_token_expiry("x").is_none());
    }
}
