// ─── LinkedIn Provider ────────────────────────────────────────
// OAuth 2.0 PKCE. Posts to LinkedIn profile or page.
// Uses LinkedIn API v2 (https://api.linkedin.com/v2).

use async_trait::async_trait;

use super::*;
use crate::config::Config;

/// LinkedIn REST API version pin (audit B10).
///
/// LinkedIn retires versions ~12 months after release; the codebase was
/// pinned to 202401 (long dead — personal/Page publishing, analytics and
/// comments were being rejected). Bump THIS one line; every LinkedIn call
/// header uses it.
/// 202609: current Marketing API release (2026-09). 202510 sunsets
/// 2026-10-15; next routine bump ~2027-09.
pub const LINKEDIN_VERSION: &str = "202609";

/// Warn once at startup when the pinned version is getting old, so a
/// sunset never comes as a surprise (audit B10 self-check).
/// Returns the (years, months) age of `version` at `now`.
pub fn linkedin_version_age_years_months(
    version: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<(i32, i32)> {
    let digits: String = version.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 6 {
        return None;
    }
    let year: i32 = digits[0..4].parse().ok()?;
    let month: i32 = digits[4..6].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let mut months = (now.format("%Y").to_string().parse::<i32>().ok()? - year) * 12
        + (now.format("%m").to_string().parse::<i32>().ok()? - month);
    if months < 0 {
        months = 0;
    }
    Some((months / 12, months % 12))
}

pub struct LinkedInProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl LinkedInProvider {
    // ─── The opt-in read scope helper ─────────────────────────
    // Returns the scope list to chain onto the always-present publish set,
    // so `scopes()` reads as "what we need" plus "what the operator opted
    // into".
    //
    // The env var is read per call rather than cached in `self` because the
    // registry builds the provider once at startup: the flag must be settled
    // before `cargo run`, which is why `.env.example` documents it beside the
    // connect flow. Reading it here also keeps the helper trivially testable.
    pub fn member_social_read_scope(&self) -> Vec<String> {
        if std::env::var("LINKEDIN_ENABLE_MEMBER_SOCIAL_READ").ok().as_deref() == Some("1") {
            vec!["r_member_social".into()]
        } else {
            vec![]
        }
    }

    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) =
            config.provider_credentials("linkedin").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    fn token_url(&self) -> &'static str {
        "https://www.linkedin.com/oauth/v2/accessToken"
    }

    fn authorize_url(&self) -> &'static str {
        "https://www.linkedin.com/oauth/v2/authorization"
    }

    // ─── API Methods ──────────────────────────────────────────

    pub async fn get_profile(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = "https://api.linkedin.com/v2/userinfo";
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_user_id(&self, access_token: &str) -> Result<String, ProviderError> {
        let profile = self.get_profile(access_token).await?;
        let user_id = profile["sub"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("Could not get LinkedIn user ID".into()))?
            .to_string();
        Ok(user_id)
    }

    // ── Analytics parsers (pure: fixture-testable, no HTTP) ──

    /// Pull the like/comment/share counts out of a `/rest/socialActions/{urn}`
    /// response. Returns an empty map when the response carries no summaries,
    /// which callers treat as "no engagement data for this post".
    pub(crate) fn social_action_counts(
        json: &serde_json::Value,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        for (key, path) in [
            ("likeCount", "/likesSummary/totalLikes"),
            ("commentCount", "/commentsSummary/totalFirstLevelComments"),
            ("shareCount", "/sharesSummary/totalShares"),
        ] {
            if let Some(n) = json.pointer(path).and_then(|v| v.as_i64()) {
                out.insert(key.into(), serde_json::json!(n));
            }
        }
        out
    }

    pub async fn get_posts(
        &self,
        access_token: &str,
        author_urn: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = limit.clamp(1, 100);
        // URL-encode the author URN (contains colons like "urn:li:person:abc123")
        let encoded_author = urlencoding::encode(author_urn);
        let url =
            format!("https://api.linkedin.com/v2/rest/posts?author={encoded_author}&count={limit}");
        tracing::debug!("LinkedIn get_posts: calling {}", url);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            tracing::debug!(
                "LinkedIn get_posts SUCCESS — response keys: {:?}, elements count: {}",
                json.as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                json["elements"].as_array().map(|a| a.len()).unwrap_or(0),
            );
            Ok(json)
        } else if status == 401 {
            // Don't log the full response body — LinkedIn error responses
            // can echo the requested Bearer token or URN. Log status only.
            tracing::warn!("LinkedIn get_posts 401 — token expired");
            Err(ProviderError::TokenExpired)
        } else if status == 403 {
            // ─── 403 on the READ path ───────────────────────────
            // The read path is the only consumer of `r_member_social`, which
            // is now opt-in. A 403 here almost always means the token was
            // granted WITHOUT that scope (the operator connected before
            // enabling the LinkedIn product, or never enabled it), so the
            // raw "Access denied" is replaced with the reconnection the
            // founder actually has to do. Publishing is unaffected and does
            // NOT emit this — its scope is always present.
            tracing::warn!("LinkedIn get_posts 403 — missing r_member_social grant");
            Err(ProviderError::Api(
                "LinkedIn could not read this member's posts: the connected account was \
                 not granted 'r_member_social'. To read your own LinkedIn posts and \
                 feed, enable the 'Read member's social' product on the LinkedIn app, \
                 set LINKEDIN_ENABLE_MEMBER_SOCIAL_READ=1 on the server, then \
                 reconnect LinkedIn from the Channels screen."
                    .into(),
            ))
        } else {
            let msg = json["message"]
                .as_str()
                .or_else(|| json["error_description"].as_str())
                .unwrap_or("LinkedIn API error")
                .to_string();
            tracing::warn!(
                "LinkedIn get_posts failed with status {}: {}. Full response: {:#}",
                status,
                msg,
                json
            );
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// GET a LinkedIn REST/v2 URL with the version headers the API requires,
    /// mapping auth failures onto the shared error variants.
    pub async fn rest_get(
        &self,
        access_token: &str,
        url: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(url)
            .query(params)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("LinkedIn API error")
                    .to_string(),
            ))
        }
    }

    pub async fn get_post_detail(
        &self,
        access_token: &str,
        post_urn: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/posts/{post_urn}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_post_comments_linkedin(
        &self,
        access_token: &str,
        post_urn: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{post_urn}/comments");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn delete_post(
        &self,
        access_token: &str,
        post_urn: &str,
    ) -> Result<(), ProviderError> {
        let url = format!(
            "https://api.linkedin.com/v2/rest/posts/{}",
            urlencoding::encode(post_urn)
        );
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() || status.as_u16() == 204 {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(ProviderError::Api(format!(
                "Delete failed ({}): {}",
                status, body
            )))
        }
    }

    pub async fn get_reactions(
        &self,
        access_token: &str,
        post_urn: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{post_urn}/likes");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_shares(
        &self,
        access_token: &str,
        post_urn: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{post_urn}/shares");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn create_comment(
        &self,
        access_token: &str,
        post_urn: &str,
        actor_urn: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{post_urn}/comments");
        let body = serde_json::json!({
            "actor": actor_urn,
            "message": {
                "text": message,
            },
            "object": post_urn,
        });
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status == 201 {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Resolve a LinkedIn media URN (e.g. "urn:li:image:12345") to a direct download/playback URL.
    /// Uses the LinkedIn Images or Videos REST API to get the actual CDN URL.
    /// Returns None if resolution fails (e.g. expired token, missing permission).
    pub async fn resolve_media_url(&self, access_token: &str, media_urn: &str) -> Option<String> {
        // Extract asset type and ID from URN: "urn:li:image:ABC" -> ("image", "ABC")
        let parts: Vec<&str> = media_urn.split(':').collect();
        if parts.len() < 4 {
            tracing::debug!("LinkedIn: invalid media URN format: {media_urn}");
            return None;
        }
        let asset_type = parts[2]; // "image" or "video"
        let asset_id = parts[3];

        let base_url = match asset_type {
            "image" => "https://api.linkedin.com/rest/images",
            "video" => "https://api.linkedin.com/rest/videos",
            _ => {
                tracing::debug!(
                    "LinkedIn: unsupported media type '{asset_type}' in URN {media_urn}"
                );
                return None;
            }
        };

        // Primary: GET the asset directly — this returns metadata including download/playback URLs
        let url = format!("{base_url}/{asset_id}");
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.http
                .get(&url)
                .header("Authorization", format!("Bearer {access_token}"))
                .header("X-Restli-Protocol-Version", "2.0.0")
                .header("LinkedIn-Version", LINKEDIN_VERSION)
                .send(),
        )
        .await;

        match resp {
            Ok(Ok(r)) if r.status().is_success() => {
                if let Ok(json) = r.json::<serde_json::Value>().await {
                    // Direct asset GET may return {"value": {"downloadUrl": "..."}}
                    if let Some(dl) = json.pointer("/value/downloadUrl").and_then(|v| v.as_str()) {
                        return Some(dl.to_string());
                    }
                    // Or {"value": {"playbackStreams": [...]}}
                    if let Some(streams) = json
                        .pointer("/value/playbackStreams")
                        .and_then(|v| v.as_array())
                    {
                        if let Some(stream) = streams.first() {
                            if let Some(s) =
                                stream.get("streamLocations").and_then(|v| v.as_array())
                            {
                                if let Some(loc) = s.first() {
                                    if let Some(u) = loc.get("url").and_then(|v| v.as_str()) {
                                        return Some(u.to_string());
                                    }
                                }
                            }
                        }
                    }
                    // Fallback: recursively search for any URL in the response
                    if let Some(url_val) = find_first_url(&json) {
                        return Some(url_val);
                    }
                }
            }
            Ok(Ok(r)) => {
                tracing::debug!(
                    "LinkedIn: resolve_media_url HTTP {} for {media_urn}",
                    r.status()
                );
            }
            Ok(Err(e)) => {
                tracing::debug!("LinkedIn: resolve_media_url network error for {media_urn}: {e}");
            }
            Err(_) => {
                tracing::debug!("LinkedIn: resolve_media_url timed out for {media_urn}");
            }
        }

        None
    }
}

#[async_trait]
impl SocialProvider for LinkedInProvider {
    fn identifier(&self) -> &'static str {
        "linkedin"
    }

    fn name(&self) -> &'static str {
        "LinkedIn"
    }

    fn scopes(&self) -> Vec<String> {
        // ─── Why `r_member_social` is opt-in ───────────────────────
        // `r_member_social` requires LinkedIn's "Sign In with LinkedIn using
        // Mobile or the LinkedIn API" product (and the separate
        // "Read member's social" grant) to be enabled on the app. When it is
        // NOT enabled, LinkedIn refuses the authorize request BEFORE any
        // consent screen, answering with its own error page:
        //
        //   "Bummer, something went wrong.
        //    In five seconds, you will be redirected to: <app_url>"
        //
        // That is a dead end with no diagnosable reason on our side, and it
        // happened on a live deployment (2026-10-30) — so the publishing
        // path (which needs only `openid` + `w_member_social`) was made
        // unusable by a scope that only the READ path needs.
        //
        // So: the publish path always works, and the read path is opt-in
        // through `LINKEDIN_ENABLE_MEMBER_SOCIAL_READ=1`. The scope is only
        // added once the operator has enabled the product, which turns the
        // same connect button from a dead end into a working consent.
        vec![
            "openid".into(),
            "profile".into(),
            "email".into(),
            // Publishing (and the profile the connected card shows).
            "w_member_social".into(),
        ]
        .into_iter()
        .chain(self.member_social_read_scope())
        .collect()
    }

    fn max_content_length(&self) -> usize {
        3000
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    /// LinkedIn uses rotating refresh tokens that expire after ~60 days.
    /// Proactive refresh ensures tokens don't silently expire between API calls.
    fn needs_cron_refresh(&self) -> bool {
        true
    }

    /// LinkedIn needs a 10-second propagation delay after token rotation.
    fn refresh_wait(&self) -> bool {
        true
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let params = [
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", redirect_uri),
            ("scope", &self.scopes().join(" ")),
            ("state", state),
        ];

        let url = url::Url::parse_with_params(self.authorize_url(), &params)
            .map_err(|e| ProviderError::Auth(format!("URL parse: {e}")))?;

        Ok(AuthUrlResponse {
            url: url.to_string(),
        })
    }

    async fn exchange_code(
        &self,
        code: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        // LinkedIn uses client_secret_post, not PKCE
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
        ];

        let resp = self
            .http
            .post(self.token_url())
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // ─── Why this path checks the status when the others could skip ──
        // This is the ONE call whose failure is reported to the founder at the
        // moment of a failed connect, and its errors are the ones that name a
        // fix. A redirect_uri mismatch answers 400 with an `error_description`
        // that says exactly which URI was wrong. Parsing that body straight
        // into a JSON value and then reporting "Missing access_token" threw
        // it away, so the founder saw the same opaque message for a typo'd
        // client secret, an expired code AND a redirect_uri mismatch - three
        // different fixes behind one sentence. That is the LinkedIn "login
        // backend error" report: not a wrong answer, a non-answer.
        // Reddit's exchange has always done this; compare
        // `social::reddit`'s equivalent.
        if !status.is_success() {
            return Err(ProviderError::Api(format!(
                "LinkedIn token exchange failed (HTTP {status}): {body}"
            )));
        }

        // A 200 with a body we cannot parse is a different failure from a
        // non-2xx, and it used to surface as a JSON decode panic-ish string
        // from `.json()` inside the `?` above. Name it instead.
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            ProviderError::Api(format!(
                "LinkedIn token response was not JSON ({e}). Body: {body}"
            ))
        })?;

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();

        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        // Get user profile
        let profile = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let sub = profile["sub"].as_str().unwrap_or("").to_string();
        let name = profile["name"].as_str().unwrap_or("").to_string();
        let username = profile["preferred_username"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let picture = profile["picture"].as_str().map(String::from);

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: sub,
            name,
            username,
            picture,
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
        ];

        let resp = self
            .http
            .post(self.token_url())
            .form(&params)
            .send()
            .await?;
        // Same shape as `exchange_code` above, and for the same reason: a
        // rotating-refresh failure used to arrive as "Missing access_token",
        // which is the message the scheduler surfaces when a token dies
        // mid-flight. LinkedIn answers a revoked refresh token with a body
        // that says so; name the status and quote it.
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ProviderError::Api(format!(
                "LinkedIn token refresh failed (HTTP {status}): {body}"
            )));
        }
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            ProviderError::Api(format!(
                "LinkedIn token refresh response was not JSON ({e}). Body: {body}"
            ))
        })?;

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();

        // Fetch profile info so it's not lost on refresh
        let profile = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        Ok(AuthToken {
            access_token,
            refresh_token: json["refresh_token"].as_str().map(String::from),
            expires_in: json["expires_in"].as_u64().map(|v| v as u32),
            provider_user_id: profile["sub"].as_str().unwrap_or("").to_string(),
            name: profile["name"].as_str().unwrap_or("").to_string(),
            username: profile["preferred_username"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            picture: profile["picture"].as_str().map(String::from),
        })
    }

    /// LinkedIn-specific user-facing error copy (v25 §2 row 9).
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "LinkedIn rate limit hit. LinkedIn allows roughly 100 API calls per day \
                 for most apps — wait for the daily quota to reset."
                    .into(),
            ),
            Some(ErrorKind::Duplicate) => Some(
                "LinkedIn flagged this post as a duplicate of recent content. \
                 Vary the text or publish to a different visibility."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "LinkedIn rejected the media. UGC posts accept JPEG/PNG/GIF images or \
                 MP4 video, up to 20 items, each from a publicly reachable URL."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "LinkedIn access token expired or was revoked. Reconnect the account \
                 to refresh it (the token lives ~60 days)."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "LinkedIn denied this request. The token is usually missing the scope \
                 the endpoint needs (r_member_social, r_organization_social) or the \
                 resource does not belong to this member."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => {
                Some("LinkedIn post or member not found. Check the URN (urn:li:activity:…).".into())
            }
            Some(ErrorKind::InvalidRequest) => Some(
                "LinkedIn rejected the request as invalid. Check the post text length \
                 (3 000 chars) and that the author URN is a person URN."
                    .into(),
            ),
            None => None,
        }
    }

    /// §8.2: delete a published LinkedIn post (shares URN).
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on LinkedIn".into(),
            ));
        }
        self.delete_post(access_token, platform_post_id).await
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Get user profile ID for posting.
        //
        // The HTTP STATUS IS READ. The old code parsed the JSON and looked
        // for `sub`, so a 401 (or a revoked token) arrived as "Could not get
        // user profile" — an ordinary API error the scheduler does NOT
        // retry, and the founder is told nothing about the token. Measured
        // live on 2026-11-13: this channel's token IS expired, the CLI
        // answers "Token expired" for the same call, and the publish
        // answered "Could not get user profile".
        let resp = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let http_status = resp.status();
        let body = resp.text().await?;
        let user_id = linkedin_member_id(http_status.as_u16(), &body).map_err(|e| match e {
            LinkedInProfileError::Auth => ProviderError::TokenExpired,
            LinkedInProfileError::Other(msg) => self.provider_error(&msg, http_status.as_u16()),
        })?;

        let body = serde_json::json!({
            "author": format!("urn:li:person:{user_id}"),
            "lifecycleState": "PUBLISHED",
            "specificContent": {
                "com.linkedin.ugc.ShareContent": {
                    "shareCommentary": {
                        "text": post.content,
                    },
                    "shareMediaCategory": if post.media.is_empty() { "NONE" } else { "IMAGE" },
                }
            },
            "visibility": {
                "com.linkedin.ugc.MemberNetworkVisibility": "PUBLIC"
            }
        });

        let resp = {
            // v23-7: send Idempotency-Key header so LinkedIn deduplicates
            // retries after a crash. LinkedIn supports this on POST /v2/ugcPosts.
            let mut req = self
                .http
                .post("https://api.linkedin.com/v2/ugcPosts")
                .header("Authorization", format!("Bearer {access_token}"))
                .header("X-Restli-Protocol-Version", "2.0.0")
                .json(&body);
            if let Some(ref key) = post.idempotency_key {
                req = req.header("Idempotency-Key", key);
            }
            req.send().await?
        };

        let status = resp.status();

        if status == 201 {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let post_id = location.rsplit('/').next().unwrap_or("").to_string();
            return Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: None, // LinkedIn doesn't always provide URL
                status: "published".into(),
            });
        }

        let json: serde_json::Value = resp.json().await?;

        if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .or_else(|| json["error_description"].as_str())
                .unwrap_or("LinkedIn publish failed")
                .to_string();
            let raw = format!("LinkedIn publish failed (HTTP {status}): {msg}");
            Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &raw,
            )))
        }
    }

    async fn reconnect(
        &self,
        access_token: &str,
        _internal_id: &str,
        _page_id: &str,
    ) -> Result<super::ReconnectResult, ProviderError> {
        let profile = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        Ok(super::ReconnectResult {
            id: profile["sub"].as_str().unwrap_or("").to_string(),
            name: profile["name"].as_str().unwrap_or("").to_string(),
            access_token: access_token.to_string(),
            picture: profile["picture"].as_str().map(String::from),
            username: profile["preferred_username"].as_str().map(String::from),
        })
    }

    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        // LinkedIn personal profiles: get network size (connections/followers)
        let user_id = self.get_user_id(access_token).await?;
        let person_urn = format!("urn:li:person:{user_id}");

        let mut results = Vec::new();

        // Get follower/connection count
        let url = format!(
            "https://api.linkedin.com/v2/networkSizes/{}?edgeType=CompanyFollowedByMember",
            person_urn
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        if resp.status().is_success() {
            let json: serde_json::Value = resp.json().await.unwrap_or_default();
            if let Some(count) = json["firstDegreeSize"].as_u64() {
                results.push(AnalyticsData {
                    label: "connections".into(),
                    data: vec![AnalyticsDataPoint {
                        total: count.to_string(),
                        date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                    }],
                    percentage_change: 0.0,
                });
            }
        }

        Ok(results)
    }

    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        // Get social actions (likes, comments, shares counts) for a post
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{platform_post_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn post analytics error")
                .to_string();
            let raw = format!("LinkedIn post analytics error (HTTP {status}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &raw,
            )));
        }

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let counts = Self::social_action_counts(&json);
        let mut results = Vec::new();

        for (label, key) in [
            ("likes", "likeCount"),
            ("comments", "commentCount"),
            ("shares", "shareCount"),
        ] {
            if let Some(n) = counts.get(key).and_then(|v| v.as_i64()) {
                results.push(AnalyticsData {
                    label: label.into(),
                    data: vec![AnalyticsDataPoint {
                        total: n.to_string(),
                        date: today.clone(),
                    }],
                    percentage_change: 0.0,
                });
            }
        }

        Ok(results)
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let user_id = self.get_user_id(access_token).await?;
        let author_urn = format!("urn:li:person:{user_id}");
        let json = self.get_posts(access_token, &author_urn, limit).await?;

        // Fetch profile info for author details (name, handle, avatar)
        let profile = self.get_profile(access_token).await.ok();
        let author_name = profile
            .as_ref()
            .and_then(|p| p["name"].as_str().map(String::from));
        let author_handle = profile
            .as_ref()
            .and_then(|p| p["preferred_username"].as_str().map(String::from));
        let author_avatar = profile
            .as_ref()
            .and_then(|p| p["picture"].as_str().map(String::from));

        tracing::info!(
            "LinkedIn get_recent_posts: {} elements in response, user_id={}",
            json["elements"].as_array().map(|a| a.len()).unwrap_or(0),
            user_id,
        );

        let mut posts = Vec::new();
        if let Some(elements) = json["elements"].as_array() {
            for element in elements {
                let post_urn = element["id"].as_str().unwrap_or("").to_string();
                if post_urn.is_empty() {
                    tracing::warn!(
                        "LinkedIn get_recent_posts: element has no id: {:#}",
                        element
                    );
                    continue;
                }

                // LinkedIn Posts API (202601) may return `commentary` as:
                // - A string: "My post content"
                // - An object: { "text": "My post content" }
                // Handle both formats gracefully.
                let commentary = element["commentary"]
                    .as_str()
                    .or_else(|| {
                        element["commentary"]
                            .as_object()
                            .and_then(|c| c["text"].as_str())
                    })
                    .unwrap_or("")
                    .to_string();

                // Skip undated items rather than stamp "now" on them.
                let Some(created_at) = element["createdAt"]
                    .as_i64()
                    .and_then(chrono::DateTime::from_timestamp_millis)
                else {
                    continue;
                };

                tracing::debug!(
                    "LinkedIn post parsed: id={} text_len={} created={:?}",
                    post_urn,
                    commentary.len(),
                    created_at,
                );

                // Extract media from the post content
                let mut media = Vec::new();
                // Limit API calls: at most 4 media items per post to avoid N+1 latency
                const MAX_MEDIA_PER_POST: usize = 4;

                if let Some(content) = element.get("content") {
                    if !content.is_null() {
                        // Single media: content.media { id: "urn:li:image:xxx" }
                        if let Some(m) = content.get("media") {
                            if let Some(media_urn) = m.get("id").and_then(|v| v.as_str()) {
                                if let Some(url) =
                                    self.resolve_media_url(access_token, media_urn).await
                                {
                                    let is_video = media_urn.contains(":video:");
                                    media.push(MediaAttachment {
                                        url,
                                        mime_type: if is_video {
                                            "video/mp4".to_string()
                                        } else {
                                            "image/jpeg".to_string()
                                        },
                                        alt: m
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .map(String::from),
                                        poster_url: None,
                                    });
                                }
                            }
                        }

                        // Multi-image / carousel: content.multiImage array
                        if media.is_empty() {
                            if let Some(multi) =
                                content.get("multiImage").and_then(|v| v.as_array())
                            {
                                for img in multi.iter().take(MAX_MEDIA_PER_POST) {
                                    if let Some(media_urn) = img.get("id").and_then(|v| v.as_str())
                                    {
                                        if let Some(url) =
                                            self.resolve_media_url(access_token, media_urn).await
                                        {
                                            media.push(MediaAttachment {
                                                url,
                                                mime_type: "image/jpeg".to_string(),
                                                alt: None,
                                                poster_url: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        // Article with thumbnail
                        if media.is_empty() {
                            if let Some(article) = content.get("article") {
                                if let Some(thumb) =
                                    article.get("thumbnail").and_then(|v| v.as_str())
                                {
                                    if !thumb.is_empty() {
                                        media.push(MediaAttachment {
                                            url: thumb.to_string(),
                                            mime_type: "image/jpeg".to_string(),
                                            alt: article
                                                .get("title")
                                                .and_then(|v| v.as_str())
                                                .map(String::from),
                                            poster_url: None,
                                        });
                                    }
                                }
                            }
                        }

                        // Single file: content.singlefile (document/PDF)
                        if media.is_empty() {
                            if let Some(file) = content.get("singlefile") {
                                if let Some(media_urn) = file.get("id").and_then(|v| v.as_str()) {
                                    if let Some(url) =
                                        self.resolve_media_url(access_token, media_urn).await
                                    {
                                        media.push(MediaAttachment {
                                            url,
                                            mime_type: "application/pdf".to_string(),
                                            alt: file
                                                .get("title")
                                                .and_then(|v| v.as_str())
                                                .map(String::from),
                                            poster_url: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // Build LinkedIn post URL
                let post_url = Some(format!("https://www.linkedin.com/feed/update/{post_urn}"));

                posts.push(ExternalPostData {
                    platform_post_id: post_urn,
                    text: commentary,
                    author_name: author_name.clone(),
                    author_handle: author_handle.clone(),
                    author_avatar: author_avatar.clone(),
                    created_at,
                    url: post_url,
                    media,
                    metadata: Some(element.clone()),
                });
            }
        } else if json["elements"].is_null() {
            tracing::warn!(
                "LinkedIn get_recent_posts: 'elements' is null/absent. Response keys: {:?}",
                json.as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>()),
            );
        }
        Ok(posts)
    }

    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        // Use the social actions API to get likes/comments/shares counts
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{platform_post_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            return Ok(None);
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let counts = Self::social_action_counts(&json);
        if counts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(serde_json::Value::Object(counts)))
        }
    }

    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_post_comments_linkedin(access_token, platform_post_id)
            .await?;
        Ok(super::parse_linkedin_comments(&json))
    }

    /// LinkedIn's REST API has no people-search endpoint, so @-autocomplete
    /// offers the connections returned by `/connections` when the `r_liteprofile`
    /// / community-management access has been granted, and an empty list
    /// otherwise rather than a hard error.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let json = match self
            .rest_get(
                access_token,
                "https://api.linkedin.com/v2/connections",
                &[("count", "50")],
            )
            .await
        {
            Ok(json) => json,
            // No partner access to connections: an empty autocomplete is the
            // documented LinkedIn behaviour, not an error the UI should show.
            Err(_) => return Ok(vec![]),
        };
        Ok(super::parse_linkedin_connections(&json, query))
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let profile = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let user_id = profile["sub"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("Could not get user profile".into()))?;

        let body = serde_json::json!({
            "author": format!("urn:li:person:{user_id}"),
            "lifecycleState": "PUBLISHED",
            "specificContent": {
                "com.linkedin.ugc.ShareContent": {
                    "shareCommentary": {
                        "text": post.content,
                    },
                    "shareMediaCategory": "NONE",
                    "parentComment": comment_id,
                }
            },
            "visibility": {
                "com.linkedin.ugc.MemberNetworkVisibility": "PUBLIC"
            }
        });

        let resp = {
            // v23-7: send Idempotency-Key header so LinkedIn deduplicates
            // retries after a crash. LinkedIn supports this on POST /v2/ugcPosts.
            let mut req = self
                .http
                .post("https://api.linkedin.com/v2/ugcPosts")
                .header("Authorization", format!("Bearer {access_token}"))
                .header("X-Restli-Protocol-Version", "2.0.0")
                .json(&body);
            if let Some(ref key) = post.idempotency_key {
                req = req.header("Idempotency-Key", key);
            }
            req.send().await?
        };

        let status = resp.status();

        if status == 201 {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let post_id = location.rsplit('/').next().unwrap_or("").to_string();
            return Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: None,
                status: "published".into(),
            });
        }

        let json: serde_json::Value = resp.json().await?;
        let msg = json["message"]
            .as_str()
            .or_else(|| json["error_description"].as_str())
            .unwrap_or("LinkedIn reply failed")
            .to_string();
        Err(self.provider_error(&msg, status.as_u16()))
    }

    async fn send_dm(
        &self,
        access_token: &str,
        recipient: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "recipients": {
                "elements": [{
                    "id": recipient,
                }]
            },
            "messageBody": {
                "text": post.content,
            },
        });

        let resp = self
            .http
            .post("https://api.linkedin.com/v2/messages")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();

        if status == 201 {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let message_id = location.rsplit('/').next().unwrap_or("").to_string();
            return Ok(PublishResult {
                platform_post_id: message_id,
                platform_post_url: None,
                status: "sent".into(),
            });
        }

        let json: serde_json::Value = resp.json().await?;
        let msg = json["message"]
            .as_str()
            .or_else(|| json["error_description"].as_str())
            .unwrap_or("LinkedIn send DM failed")
            .to_string();
        Err(self.provider_error(&msg, status.as_u16()))
    }

    // This provider has a DM surface, so `dm_capable()` reports true, which is what
    // lets the /dms page learn it from the API instead of guessing a provider
    // list. This says the PROVIDER can do DMs, not that any given CHANNEL can.
    fn dm_capable(&self) -> bool {
        true
    }

    async fn get_dm_conversations(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<Vec<super::DmConversation>, ProviderError> {
        let url = format!(
            "https://api.linkedin.com/v2/messages?action=search&count={}",
            limit.min(50)
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let mut conversations = Vec::new();

        if let Some(elements) = json["elements"].as_array() {
            for elem in elements {
                let id = elem["conversationId"].as_str().unwrap_or("").to_string();
                let participants = elem["participants"]
                    .as_array()
                    .and_then(|p| p.first())
                    .and_then(|p| p["id"].as_str())
                    .unwrap_or("")
                    .to_string();
                let last_message = elem["body"].as_str().map(|s| s.to_string());
                let last_message_at = elem["createdAt"]
                    .as_i64()
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts / 1000, 0));

                conversations.push(super::DmConversation {
                    id,
                    participant: participants,
                    participant_name: None,
                    participant_avatar: None,
                    last_message,
                    last_message_at,
                    unread_count: 0,
                });
            }
        }

        Ok(conversations)
    }

    async fn get_dm_messages(
        &self,
        access_token: &str,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<super::DmMessage>, ProviderError> {
        let url = format!(
            "https://api.linkedin.com/v2/messages?action=byConversation&conversationId={}&count={}",
            conversation_id,
            limit.min(50)
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let mut messages = Vec::new();

        if let Some(elements) = json["elements"].as_array() {
            for elem in elements {
                let id = elem["id"].as_str().unwrap_or("").to_string();
                let sender = elem["sender"]
                    .as_object()
                    .and_then(|s| s.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = elem["body"].as_str().unwrap_or("").to_string();
                let created_at = elem["createdAt"]
                    .as_i64()
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts / 1000, 0))
                    .unwrap_or_else(chrono::Utc::now);

                messages.push(super::DmMessage {
                    id,
                    conversation_id: conversation_id.to_string(),
                    sender,
                    sender_name: None,
                    content,
                    media: vec![],
                    created_at,
                    read: true,
                });
            }
        }

        Ok(messages)
    }
}

/// Recursively search a JSON value for the first string that looks like a URL.
fn find_first_url(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                Some(s.clone())
            } else {
                None
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(url) = find_first_url(item) {
                    return Some(url);
                }
            }
            None
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map {
                if let Some(url) = find_first_url(v) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

/// Why a `/v2/userinfo` response did not yield a member id.
///
/// `Auth` means the TOKEN is the problem (401/403) — the scheduler refreshes
/// it and retries, and the founder is told to reconnect. `Other` carries
/// the provider's own words for anything else.
///
/// The old code parsed the JSON and looked for `sub`, so an expired token
/// arrived as "Could not get user profile": an ordinary API error the
/// scheduler does NOT retry, with nothing telling the founder that the
/// channel needs a reconnect. Measured live on 2026-11-13 — the same
/// channel answered "Token expired" from the CLI and "Could not get user
/// profile" from the publish.
#[derive(Debug, PartialEq)]
pub enum LinkedInProfileError {
    Auth,
    Other(String),
}

/// The member id (`sub`) from a LinkedIn `/v2/userinfo` response.
pub fn linkedin_member_id(status: u16, body: &str) -> Result<String, LinkedInProfileError> {
    if status == 401 || status == 403 {
        return Err(LinkedInProfileError::Auth);
    }
    if !status_is_success(status) {
        return Err(LinkedInProfileError::Other(body.to_string()));
    }
    let profile: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    profile["sub"]
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LinkedInProfileError::Other(body.to_string()))
}

/// Is this a 2xx status?
fn status_is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::social::test_config;

    #[test]
    fn test_scopes_contain_required() {
        let provider = LinkedInProvider::new(&test_config());
        let scopes = provider.scopes();
        assert!(scopes.contains(&"openid".to_string()));
        assert!(scopes.contains(&"w_member_social".to_string()));
        assert!(scopes.contains(&"profile".to_string()));
    }

    #[test]
    fn test_identifier_and_name() {
        let provider = LinkedInProvider::new(&test_config());
        assert_eq!(provider.identifier(), "linkedin");
        assert_eq!(provider.name(), "LinkedIn");
    }

    #[test]
    fn test_max_content_length() {
        let provider = LinkedInProvider::new(&test_config());
        assert_eq!(provider.max_content_length(), 3000);
    }

    #[tokio::test]
    async fn test_generate_auth_url_contains_params() {
        let provider = LinkedInProvider::new(&test_config());
        let result = provider
            .generate_auth_url(
                "test_state",
                "test_verifier",
                "http://localhost:3000/callback",
            )
            .await;
        let url = result.unwrap().url;

        assert!(
            url.contains("response_type=code"),
            "should contain response_type=code"
        );
        assert!(
            url.contains("client_id=test_linkedin_id"),
            "should contain client_id"
        );
        assert!(url.contains("redirect_uri="), "should contain redirect_uri");
        assert!(url.contains("state=test_state"), "should contain state");
        assert!(url.contains("scope="), "should contain scope");
        assert!(url.starts_with("https://www.linkedin.com/oauth/v2/authorization"));
    }

    // ── B2c: social-actions parser fixtures ──────────────────

    #[test]
    fn social_action_counts_reads_all_three_summaries() {
        let fixture = serde_json::json!({
            "likesSummary": { "totalLikes": 42 },
            "commentsSummary": { "totalFirstLevelComments": 12 },
            "sharesSummary": { "totalShares": 5 }
        });
        let counts = LinkedInProvider::social_action_counts(&fixture);
        assert_eq!(counts["likeCount"], 42);
        assert_eq!(counts["commentCount"], 12);
        assert_eq!(counts["shareCount"], 5);
    }

    #[test]
    fn social_action_counts_skips_absent_summaries() {
        let fixture = serde_json::json!({ "likesSummary": { "totalLikes": 0 } });
        let counts = LinkedInProvider::social_action_counts(&fixture);
        assert_eq!(counts["likeCount"], 0);
        assert!(!counts.contains_key("commentCount"));
        assert!(!counts.contains_key("shareCount"));
    }

    #[test]
    fn social_action_counts_empty_for_error_body() {
        let fixture = serde_json::json!({ "message": "Not Found", "status": 404 });
        assert!(LinkedInProvider::social_action_counts(&fixture).is_empty());
    }

    #[test]
    fn social_action_counts_feed_normalises_into_engagement_data() {
        // The engagement arm in mod.rs reads exactly these keys.
        let fixture = serde_json::json!({
            "likesSummary": { "totalLikes": 42 },
            "commentsSummary": { "totalFirstLevelComments": 12 },
            "sharesSummary": { "totalShares": 5 }
        });
        let e = crate::social::parse_engagement_data(
            "linkedin",
            serde_json::Value::Object(LinkedInProvider::social_action_counts(&fixture)),
        );
        assert_eq!((e.likes, e.comments, e.shares), (42, 12, 5));
    }

    // ── B2c: map_error fixtures ─────────────────────────────

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let p = LinkedInProvider::new(&test_config());
        let msg = p.map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_duplicate_content() {
        let p = LinkedInProvider::new(&test_config());
        let msg = p.map_error("This content is a duplicate", 400).unwrap();
        assert!(msg.contains("duplicate"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let p = LinkedInProvider::new(&test_config());
        let msg = p.map_error("The image upload was rejected", 422).unwrap();
        assert!(msg.contains("media"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let p = LinkedInProvider::new(&test_config());
        let msg = p.map_error("", 401).unwrap();
        assert!(msg.contains("expired"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_forbidden_on_403() {
        let p = LinkedInProvider::new(&test_config());
        let msg = p.map_error("", 403).unwrap();
        assert!(msg.contains("denied"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        let p = LinkedInProvider::new(&test_config());
        assert!(p.map_error("Internal Server Error", 503).is_none());
    }
}

#[cfg(test)]
mod linkedin_profile_rule_tests {
    use super::*;

    #[test]
    fn an_expired_token_is_an_auth_error_not_a_missing_profile() {
        // A 401 on /v2/userinfo means the TOKEN expired. The publish path
        // used to report "Could not get user profile", so the scheduler
        // never refreshed it.
        assert_eq!(
            linkedin_member_id(401, r#"{"error":"invalid_token"}"#),
            Err(LinkedInProfileError::Auth)
        );
        assert_eq!(
            linkedin_member_id(403, r#"{"error":"forbidden"}"#),
            Err(LinkedInProfileError::Auth)
        );
    }

    #[test]
    fn a_good_response_yields_the_member_id() {
        assert_eq!(
            linkedin_member_id(200, r#"{"sub":"abc123","name":"Ishan"}"#),
            Ok("abc123".to_string())
        );
    }

    #[test]
    fn a_success_without_a_sub_is_the_providers_own_words() {
        let err = linkedin_member_id(200, r#"{"name":"Ishan"}"#).unwrap_err();
        assert!(matches!(err, LinkedInProfileError::Other(_)));
    }

    #[test]
    fn a_server_error_is_not_an_auth_error() {
        // A 500 is not a token problem — retrying the refresh would not
        // help, and the founder must not be told to reconnect.
        let err = linkedin_member_id(500, "oops").unwrap_err();
        assert!(matches!(err, LinkedInProfileError::Other(_)));
    }
}
