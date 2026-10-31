// ─── X/Twitter Provider ───────────────────────────────────────
// Dual-path: GraphQL API (cookie auth) + OAuth 2.0 PKCE (Twitter API v2).
// GraphQL path uses wreq for Chrome TLS fingerprint emulation.
// OAuth path is the fallback used by the web onboarding flow.

use std::sync::LazyLock;
const RATE_LIMIT_RETRIES: u32 = 2;
async fn rate_limit_sleep(attempt: u32) {
    tokio::time::sleep(std::time::Duration::from_secs(1u64 << attempt)).await;
}
fn is_rate_limited(body: &str) -> bool {
    body.contains("\"code\":88") || body.contains("Rate limit exceeded")
}

use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use wreq::header::{self, HeaderMap, HeaderValue};
use wreq_util::Emulation;

use super::*;
use crate::config::Config;

// ── Constants ───────────────────────────────────────────────

static TWITTER_BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

static USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// static constants removed — unused dead code

// sec_ch_ua() and sec_ch_ua_full_version_list() removed — unused dead code

static GRAPHQL_FEATURES: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "responsive_web_graphql_exclude_directive_enabled": true,
        "verified_phone_label_enabled": false,
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "c9s_tweet_anatomy_moderator_badge_enabled": true,
        "tweetypie_unmention_optimization_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": true,
        "tweet_awards_web_tipping_enabled": false,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "rweb_video_timestamps_enabled": true,
        "responsive_web_media_download_video_enabled": true,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "responsive_web_enhance_cards_enabled": false
    })
});

// ── XProvider ───────────────────────────────────────────────

pub struct XProvider {
    client_id: String,
    client_secret: String,
    http: wreq::Client,
    /// Additional cookies beyond auth_token+ct0 (e.g. guest_id, kdt, twid)
    cookie_string: Option<String>,
}

/// Shared X wreq client (F1): ONE Chrome131-emulation pool with the static
/// bearer + browser headers baked in. Previously rebuilt per XProvider::new
/// — every MCP call paid a fresh TLS-fingerprint stack build.
fn x_http() -> wreq::Client {
    static CLIENT: std::sync::LazyLock<wreq::Client> = std::sync::LazyLock::new(|| {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, HeaderValue::from_static(USER_AGENT));
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert("origin", HeaderValue::from_static("https://x.com"));
        headers.insert("referer", HeaderValue::from_static("https://x.com/"));
        headers.insert("x-twitter-active-user", HeaderValue::from_static("yes"));
        headers.insert(
            "x-twitter-auth-type",
            HeaderValue::from_static("OAuth2Session"),
        );
        headers.insert("x-twitter-client-language", HeaderValue::from_static("en"));
        // Authorization header MUST include "Bearer " prefix
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", TWITTER_BEARER_TOKEN))
                .expect("bearer token constant is a valid header value"),
        );
        wreq::Client::builder()
            .default_headers(headers)
            .emulation(Emulation::Chrome131)
            .gzip(true)
            .brotli(true)
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("Failed to build wreq client")
    });
    CLIENT.clone()
}

impl XProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("x").unwrap_or_default();

        let auth_token = config.x_auth_token.as_deref();
        let ct0 = config.x_ct0.as_deref();

        let http = x_http();

        let mut provider = Self {
            client_id,
            client_secret,
            http,
            cookie_string: None,
        };

        // B12: env-var credentials ONLY. Browser-cookie auto-extraction
        // was removed from `new()`: on Linux it read encrypted values
        // from the Chrome cookie DB and "succeeded" with ciphertext as the
        // auth_token — at EVERY XProvider::new (registry boot, every X MCP
        // call), poisoning connections that then failed with a misleading
        // 401. Cookies are imported explicitly: the /connect/x-cookies
        // endpoint, X_AUTH_TOKEN + X_CT0 env vars, or the OAuth flow.
        if let (Some(at), Some(ct)) = (auth_token, ct0) {
            if !at.is_empty() && !ct.is_empty() {
                provider.set_credentials(at, ct, None);
            }
        }

        provider
    }

    // ── Cookie Token Parsing ─────────────────────────────────

    fn parse_cookie_token(token: &str) -> Option<(String, String)> {
        let v: serde_json::Value = serde_json::from_str(token).ok()?;
        let auth_token = v.get("auth_token")?.as_str()?.to_string();
        let ct0 = v.get("ct0")?.as_str()?.to_string();
        Some((auth_token, ct0))
    }

    /// Extract optional full cookie string from token JSON blob
    fn extract_cookie_string(token: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(token).ok()?;
        v.get("cookie_string")?.as_str().map(|s| s.to_string())
    }

    /// Initialize credentials from a token JSON blob (used by MCP handlers).
    /// Parses auth_token, ct0, and optional cookie_string from the token.
    pub fn prepare_from_token(&mut self, token: &str) {
        let v: serde_json::Value = serde_json::from_str(token).unwrap_or_default();
        let at = v.get("auth_token").and_then(|s| s.as_str()).unwrap_or("");
        let ct = v.get("ct0").and_then(|s| s.as_str()).unwrap_or("");
        let extras = v.get("cookie_string").and_then(|s| s.as_str());
        if !at.is_empty() && !ct.is_empty() {
            self.set_credentials(at, ct, extras);
        }
    }

    /// Set cookie credentials to be used for requests.
    ///
    /// Avoids duplicates: if `extra_cookies` already contains `auth_token=` and `ct0=`
    /// (e.g. a full Cookie header string), it is used directly without prepending.
    fn set_credentials(&mut self, auth_token: &str, ct0: &str, extra_cookies: Option<&str>) {
        self.cookie_string = Some(match extra_cookies {
            Some(extras) if !extras.is_empty() => {
                // extras is a full cookie string that already contains auth_token+ct0
                if extras.contains("auth_token=") && extras.contains("ct0=") {
                    extras.to_string()
                } else {
                    format!("auth_token={auth_token}; ct0={ct0}; {extras}")
                }
            }
            _ => format!("auth_token={auth_token}; ct0={ct0};"),
        });
    }

    /// Check if a token is a cookie auth JSON blob (public for testing)
    pub fn is_cookie_auth(token: &str) -> bool {
        Self::parse_cookie_token(token).is_some()
    }

    /// Public alias for is_cookie_auth for cross-crate test access
    pub fn is_cookie_auth_static(token: &str) -> bool {
        Self::parse_cookie_token(token).is_some()
    }

    // ── OAuth helpers (kept for SocialProvider trait) ────────

    fn oauth_token_endpoint(&self) -> &'static str {
        "https://api.twitter.com/2/oauth2/token"
    }

    fn oauth_authorize_endpoint(&self) -> &'static str {
        "https://twitter.com/i/oauth2/authorize"
    }

    // ── Core GraphQL Request ─────────────────────────────────

    fn graphql_url(
        &self,
        query_id: &str,
        operation: &str,
        variables: &serde_json::Value,
    ) -> Result<String, ProviderError> {
        use url::form_urlencoded;
        let vars_json = variables.to_string();
        let features_json = GRAPHQL_FEATURES.to_string();
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("variables", &vars_json)
            .append_pair("features", &features_json)
            .finish();
        Ok(format!(
            "https://x.com/i/api/graphql/{query_id}/{operation}?{query}"
        ))
    }

    /// Ensure the ct0 value in a cookie string matches the given ct0.
    /// This prevents CSRF mismatches when the stored cookies have a stale ct0.
    fn ensure_ct0_matches(cookie_str: &str, ct0: &str) -> String {
        let mut parts: Vec<String> = cookie_str
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.starts_with("ct0=") && !s.is_empty())
            .collect();
        parts.push(format!("ct0={ct0}"));
        parts.join("; ")
    }

    /// Build the effective cookie string for a GraphQL request.
    /// Prefers the access_token's embedded cookie_string over self.cookie_string,
    /// and ensures ct0 matches the header value.
    fn effective_cookie_str(&self, access_token: &str) -> (String, String) {
        // Parse ct0 from the access token JSON blob
        let ct0 = Self::parse_cookie_token(access_token)
            .map(|(_, ct)| ct)
            .unwrap_or_default();

        // Prefer the full cookie_string from the access token if available
        let base_cs = Self::extract_cookie_string(access_token)
            .filter(|s| !s.is_empty() && s.contains("auth_token="))
            .or_else(|| self.cookie_string.clone())
            .unwrap_or_default();

        // Ensure ct0 in cookie matches the header
        let cs = Self::ensure_ct0_matches(&base_cs, &ct0);
        (ct0, cs)
    }

    async fn graphql_get(
        &self,
        query_id: &str,
        operation: &str,
        variables: &serde_json::Value,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = self.graphql_url(query_id, operation, variables)?;
        let (ct0, cs) = self.effective_cookie_str(access_token);
        let resp = self
            .http
            .get(&url)
            .header("x-csrf-token", &ct0)
            .header("Cookie", &cs)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X GraphQL GET error: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ProviderError::Api(format!("X body error: {e}")))?;
        if !status.is_success() {
            let raw = format!("X GraphQL {operation}: HTTP {status}: {body}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(&body, status.as_u16()),
                &raw,
            )));
        }
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| ProviderError::Api(format!("X JSON parse error: {e}: {body}")))?;
        self.check_graphql_response(status, &json)
    }

    async fn graphql_post(
        &self,
        query_id: &str,
        operation: &str,
        variables: &serde_json::Value,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://x.com/i/api/graphql/{query_id}/{operation}");
        let body = serde_json::json!({
            "variables": variables,
            "queryId": query_id,
            "features": *GRAPHQL_FEATURES,
        });
        let (ct0, cs) = self.effective_cookie_str(access_token);
        let resp = self
            .http
            .post(&url)
            .header("x-csrf-token", &ct0)
            .header("Cookie", &cs)
            .header("Priority", "u=1, i")
            .header("Referer", "https://x.com/compose/post")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X GraphQL POST error: {e}")))?;
        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(format!("X JSON parse error: {e}")))?;
        self.check_graphql_response(status, &json)
    }

    // ── V2 REST API (OAuth Bearer fallback) ──────────────────

    /// Read the response body as text, tolerating an empty body (returns
    /// `json!({})`). This prevents `EOF while parsing a value` when X answers
    /// HTTP 200 with no body (e.g. empty DM conversation list).
    async fn read_json(
        &self,
        resp: wreq::Response,
    ) -> Result<(wreq::StatusCode, serde_json::Value), ProviderError> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Api(format!("X read body error: {e}")))?;
        let json: serde_json::Value = if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text)
                .map_err(|e| ProviderError::Api(format!("X JSON parse error: {e}")))?
        };
        Ok((status, json))
    }

    async fn v1_get_with_cookies(
        &self,
        url: &str,
        cookie_str: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let ct0 = Self::parse_cookie_token(cookie_str)
            .map(|(_, ct)| ct)
            .unwrap_or_default();
        let resp = self
            .http
            .get(url)
            .header("x-csrf-token", &ct0)
            .header("Cookie", cookie_str)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X cookie GET error: {e}")))?;
        let (status, json) = self.read_json(resp).await?;
        self.check_v2_response(status, &json)
    }

    /// Fetch a v2 endpoint with whichever credential the row holds.
    ///
    /// A cookie-connected row (auth_token + ct0, the shape `connect x`
    /// writes) has NO OAuth bearer — sending one to api.twitter.com
    /// returns 401 "Token expired" and every analytics refresh for a
    /// cookie-connected account fails, while the GraphQL surfaces that
    /// use the same cookie work fine. A bearer row keeps the bearer path.
    async fn v2_get_with_credential(
        &self,
        url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((at, ct)) = Self::parse_cookie_token(access_token) {
            let cookie_str = Self::extract_cookie_string(access_token)
                .unwrap_or_else(|| format!("auth_token={at}; ct0={ct};"));
            self.v1_get_with_cookies(url, &cookie_str).await
        } else {
            self.v2_get(url, access_token).await
        }
    }

    async fn v2_get(
        &self,
        url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(url)
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X v2 GET error: {e}")))?;
        let (status, json) = self.read_json(resp).await?;
        self.check_v2_response(status, &json)
    }

    async fn v2_post(
        &self,
        url: &str,
        access_token: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .post(url)
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .json(body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X v2 POST error: {e}")))?;
        let (status, json) = self.read_json(resp).await?;
        self.check_v2_response(status, &json)
    }

    async fn v2_delete(
        &self,
        url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .delete(url)
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X v2 DELETE error: {e}")))?;
        let (status, json) = self.read_json(resp).await?;
        self.check_v2_response(status, &json)
    }

    // ── Response Checkers ────────────────────────────────────

    fn check_graphql_response(
        &self,
        status: wreq::StatusCode,
        json: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        if status.is_success() {
            // Check for GraphQL errors in response body
            if let Some(errors) = json.get("errors").and_then(|e| e.as_array()) {
                if let Some(first) = errors.first() {
                    let msg = first["message"].as_str().unwrap_or("GraphQL error");
                    if msg.contains("Not authorized") || msg.contains("authentication") {
                        // C2: GraphQL = cookie-session mode. A dead cookie is
                        // not refreshable — say so instead of TokenExpired,
                        // which the scheduler would "refresh" (impossible for
                        // cookies) and misreport as an expired token.
                        return Err(ProviderError::Auth(
                            "X cookie session expired or was revoked — re-import cookies".into(),
                        ));
                    }
                    return Err(ProviderError::Api(msg.to_string()));
                }
            }
            return Ok(json.clone());
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("X API rate limit".into()));
        }
        if status == 401 || status == 403 {
            // C2: same reasoning as above — cookie sessions are re-imported,
            // never refreshed.
            return Err(ProviderError::Auth(
                "X cookie session expired or was revoked (HTTP {status}) — re-import cookies"
                    .replace("{status}", &status.as_u16().to_string()),
            ));
        }
        let msg = json["errors"][0]["message"]
            .as_str()
            .unwrap_or("GraphQL request failed");
        let raw = format!("HTTP {status}: {msg}");
        Err(ProviderError::Api(friendly_error(
            self.map_error(msg, status.as_u16()),
            &raw,
        )))
    }

    fn check_v2_response(
        &self,
        status: wreq::StatusCode,
        json: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        if status.is_success() {
            return Ok(json.clone());
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("X API rate limit".into()));
        }
        if json.get("title").and_then(|t| t.as_str()) == Some("Unauthorized") || status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        let detail = json
            .get("detail")
            .and_then(|d| d.as_str())
            .unwrap_or("Unknown API error");
        Err(ProviderError::Api(friendly_error(
            self.map_error(detail, status.as_u16()),
            detail,
        )))
    }

    // ── Pagination cursor extraction from GraphQL timeline ───

    fn extract_next_cursor(json: &serde_json::Value) -> Option<String> {
        // Walk common timeline paths
        let instructions = json
            .pointer("/data/home/home_timeline_urt/instructions")
            .or_else(|| json.pointer("/data/user/result/timeline/timeline/instructions"))
            .or_else(|| json.pointer("/data/user/result/timeline_v2/timeline/instructions"))
            .or_else(|| {
                json.pointer("/data/search_by_raw_query/search_timeline/timeline/instructions")
            })
            .or_else(|| json.pointer("/data/bookmark_timeline_v2/timeline/instructions"))
            .or_else(|| json.pointer("/data/list/tweets_timeline/timeline/instructions"))
            .and_then(|i| i.as_array())?;

        for instruction in instructions {
            if let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) {
                for entry in entries {
                    if let Some(content) = entry.get("content") {
                        if content.get("cursorType").and_then(|c| c.as_str()) == Some("Bottom") {
                            if let Some(val) = content.get("value").and_then(|v| v.as_str()) {
                                return Some(val.to_string());
                            }
                        }
                    }
                }
            }
        }
        None
    }

    // ── Analytics parsers (pure: fixture-testable, no HTTP) ──

    /// Map a `/2/users/{id}?user.fields=public_metrics` response into dashboard rows.
    fn parse_account_metrics(json: &serde_json::Value) -> Vec<AnalyticsData> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let metrics = &json["data"]["public_metrics"];
        [
            ("Followers", "followers_count"),
            ("Following", "following_count"),
            ("Tweets", "tweet_count"),
            ("Listed", "listed_count"),
        ]
        .into_iter()
        .filter_map(|(label, key)| {
            let total = metrics.get(key).and_then(|v| v.as_i64())?.to_string();
            Some(AnalyticsData {
                label: label.into(),
                data: vec![AnalyticsDataPoint {
                    total,
                    date: today.clone(),
                }],
                percentage_change: 0.0,
            })
        })
        .collect()
    }

    /// Map a `/2/tweets/{id}?tweet.fields=public_metrics` response into per-post rows.
    fn parse_tweet_metrics(json: &serde_json::Value) -> Vec<AnalyticsData> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let metrics = &json["data"]["public_metrics"];
        [
            ("Likes", "like_count"),
            ("Retweets", "retweet_count"),
            ("Replies", "reply_count"),
            ("Quotes", "quote_count"),
            ("Impressions", "impression_count"),
        ]
        .into_iter()
        .filter_map(|(label, key)| {
            let total = metrics.get(key).and_then(|v| v.as_i64())?.to_string();
            Some(AnalyticsData {
                label: label.into(),
                data: vec![AnalyticsDataPoint {
                    total,
                    date: today.clone(),
                }],
                percentage_change: 0.0,
            })
        })
        .collect()
    }

    /// Normalize both tweet shapes into the v2 `public_metrics` object that
    /// `parse_engagement_data("x", …)` understands.
    ///
    /// The OAuth path returns `data.public_metrics` verbatim. The GraphQL
    /// (cookie) path nests the tweet under `data.result.legacy` and uses v1
    /// key names, so without this translation the cookie path silently
    /// reported zero engagement.
    fn extract_public_metrics(detail: &serde_json::Value) -> Option<serde_json::Value> {
        let data = detail.get("data")?;
        if let Some(pm) = data.get("public_metrics") {
            return Some(pm.clone());
        }
        let legacy = data.get("result").unwrap_or(data).get("legacy")?;
        // X sometimes serializes GraphQL counters as strings; normalise to
        // numbers so `as_i64()` in the parser arm does not read them as 0.
        let count = |v: &serde_json::Value| -> Option<serde_json::Value> {
            v.as_i64().map(serde_json::Value::from).or_else(|| {
                v.as_str()?
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .map(serde_json::Value::from)
            })
        };
        let mut out = serde_json::Map::new();
        for (gql_key, v2_key) in [
            ("favorite_count", "like_count"),
            ("retweet_count", "retweet_count"),
            ("reply_count", "reply_count"),
            ("quote_count", "quote_count"),
            ("bookmark_count", "bookmark_count"),
            ("view_count", "impression_count"),
        ] {
            if let Some(v) = legacy.get(gql_key).and_then(count) {
                out.insert(v2_key.to_string(), v);
            }
        }
        (!out.is_empty()).then_some(serde_json::Value::Object(out))
    }

    // ── Comment + mention parsers (pure: fixture-testable) ───

    /// Walk a TweetDetail GraphQL response into comments, skipping the focal
    /// tweet itself. GraphQL returns the conversation as one flat entry list,
    /// so replies stay empty and the UI threads them client-side.
    pub(crate) fn parse_graphql_comments(
        json: &serde_json::Value,
        focal_tweet_id: &str,
    ) -> Vec<CommentData> {
        json.pointer("/data/threaded_conversation_with_injections_v2/instructions")
            .and_then(|instructions| instructions.as_array())
            .into_iter()
            .flatten()
            .filter_map(|instruction| instruction.get("entries").and_then(|e| e.as_array()))
            .flatten()
            .flat_map(|entry| {
                // A reply sits directly on the entry, or one level down inside a conversation module.
                let direct = entry.pointer("/content/itemContent/tweet_results/result");
                let in_module = entry
                    .pointer("/content/items")
                    .and_then(|items| items.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.pointer("/item/itemContent/tweet_results/result"));
                direct.into_iter().chain(in_module)
            })
            .filter_map(|tweet| {
                let id = tweet
                    .pointer("/rest_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| tweet.pointer("/legacy/id_str").and_then(|v| v.as_str()))?;
                if id == focal_tweet_id {
                    return None;
                }
                let legacy = tweet.get("legacy");
                let core = tweet.pointer("/core/user_results/result/core");
                Some(CommentData {
                    id: id.to_string(),
                    // Current responses keep the name in `legacy`; `core` is the older location.
                    author_name: tweet
                        .pointer("/core/user_results/result/legacy/name")
                        .or_else(|| core.and_then(|c| c.get("name")))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    author_avatar: core
                        .and_then(|c| c.pointer("/avatar/image_url"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    text: legacy
                        .and_then(|l| l.get("full_text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    created_at: legacy
                        .and_then(|l| l.get("created_at"))
                        .and_then(|v| v.as_str())
                        .map(crate::social::common::parse_timestamp)
                        .unwrap_or_else(chrono::Utc::now),
                    like_count: legacy
                        .and_then(|l| l.get("favorite_count"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as i32,
                    replies: vec![],
                })
            })
            .collect()
    }

    /// Turn a `conversation_id` recent-search response into comments. The
    /// search returns the whole conversation including the root tweet, which is
    /// skipped; author details come from `includes.users`.
    pub(crate) fn parse_v2_conversation(
        json: &serde_json::Value,
        root_tweet_id: &str,
    ) -> Vec<CommentData> {
        let users: std::collections::HashMap<&str, &serde_json::Value> = json
            .pointer("/includes/users")
            .and_then(|u| u.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|u| Some((u["id"].as_str()?, u)))
                    .collect()
            })
            .unwrap_or_default();

        json.get("data")
            .and_then(|d| d.as_array())
            .map(|tweets| {
                tweets
                    .iter()
                    .filter_map(|tweet| {
                        let id = tweet["id"].as_str()?;
                        if id == root_tweet_id {
                            return None;
                        }
                        let author = users.get(tweet["author_id"].as_str()?);
                        Some(CommentData {
                            id: id.to_string(),
                            author_name: author.and_then(|a| a["name"].as_str()).map(String::from),
                            author_avatar: author
                                .and_then(|a| a["profile_image_url"].as_str())
                                .map(String::from),
                            text: tweet["text"].as_str().unwrap_or_default().to_string(),
                            created_at: tweet["created_at"]
                                .as_str()
                                .map(crate::social::common::parse_timestamp)
                                .unwrap_or_else(chrono::Utc::now),
                            like_count: tweet
                                .pointer("/public_metrics/like_count")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0) as i32,
                            replies: vec![],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Map a `GET /2/users/search` response into @-autocomplete candidates.
    pub(crate) fn parse_user_search(json: &serde_json::Value) -> Vec<MentionResult> {
        json.get("data")
            .and_then(|d| d.as_array())
            .map(|users| {
                users
                    .iter()
                    .filter_map(|user| {
                        Some(MentionResult {
                            id: user["id"].as_str()?.to_string(),
                            label: user["username"]
                                .as_str()
                                .or(user["name"].as_str())
                                .unwrap_or_default()
                                .to_string(),
                            image: user["profile_image_url"].as_str().map(String::from),
                            do_not_cache: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // ── Media upload (v1.1 API, used by both paths) ──────────    // ── Media upload (v1.1 API, used by both paths) ──────────

    async fn fetch_media_bytes(&self, url: &str) -> Result<Vec<u8>, ProviderError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            // Use a separate reqwest client for media download (may not need emulation)
            let resp = reqwest::get(url)
                .await
                .map_err(|e| ProviderError::Api(format!("Failed to fetch media: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(ProviderError::Api(format!(
                    "Failed to fetch media: HTTP {status}"
                )));
            }
            resp.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| ProviderError::Api(format!("Failed to read media body: {e}")))
        } else {
            tokio::fs::read(url)
                .await
                .map_err(|e| ProviderError::Api(format!("Failed to read local media {url}: {e}")))
        }
    }

    /// Upload one attachment. B5: mode-split —
    /// - Cookie sessions use the web-shaped 1.1 chunked flow, now WITH
    ///   `Cookie` + `x-csrf-token` (the public web bearer alone is rejected)
    ///   and 4 MiB base64 segments (the old single-segment APPEND broke any
    ///   media > 5 MiB and its response was never checked).
    /// - OAuth tokens use `POST https://api.x.com/2/media/upload` (the
    ///   v1.1 endpoint is deprecated for API keys), multipart RAW chunks.
    ///
    /// Returns the `media_id` and whether X has finished processing the
    /// bytes. FINALIZE answers long before a video is transcoded — it
    /// reports `processing_info.state: pending|in_progress` while the
    /// video is still being encoded, and only `succeeded` once playable.
    async fn upload_single_media(
        &self,
        access_token: &str,
        media_url: &str,
        mime_type: &str,
    ) -> Result<(String, bool), ProviderError> {
        let bytes = self.fetch_media_bytes(media_url).await?;

        let media_category = if mime_type.starts_with("video/") {
            "tweet_video"
        } else if mime_type == "image/gif" {
            "tweet_gif"
        } else {
            "tweet_image"
        };

        if Self::parse_cookie_token(access_token).is_some() {
            self.upload_media_1_1_cookie(access_token, &bytes, mime_type, media_category)
                .await
        } else {
            self.upload_media_v2(access_token, &bytes, mime_type, media_category)
                .await
        }
    }

    /// B5 (cookie mode): 1.1 chunked upload WITH cookie auth.
    async fn upload_media_1_1_cookie(
        &self,
        access_token: &str,
        bytes: &[u8],
        mime_type: &str,
        media_category: &str,
    ) -> Result<(String, bool), ProviderError> {
        let (ct0, cookie_str) = self.effective_cookie_str(access_token);
        let api_base = "https://upload.twitter.com/1.1/media/upload.json";
        let auth_headers = |req: wreq::RequestBuilder| {
            req.header(header::COOKIE, cookie_str.clone())
                .header("x-csrf-token", ct0.clone())
        };

        // INIT
        let init_body = format!(
            "command=INIT&total_bytes={}&media_type={}&media_category={media_category}",
            bytes.len(),
            urlencoding::encode(mime_type),
        );
        let init_resp = auth_headers(
            self.http
                .post(api_base)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(init_body),
        )
        .send()
        .await
        .map_err(|e| ProviderError::Api(format!("Media INIT error: {e}")))?;
        let init_json = Self::checked_upload_json(init_resp, "INIT").await?;
        let media_id = Self::media_id_from(&init_json)?;

        // APPEND — 4 MiB base64 segments, each response checked (B5).
        for (segment_index, chunk) in bytes.chunks(4 * 1024 * 1024).enumerate() {
            let b64_data =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, chunk);
            let append_body = format!(
                "command=APPEND&media_id={media_id}&segment_index={segment_index}&media_data={}",
                urlencoding::encode(&b64_data),
            );
            let append_resp = auth_headers(
                self.http
                    .post(api_base)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(append_body),
            )
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("Media APPEND {segment_index} error: {e}")))?;
            Self::checked_upload_json(append_resp, &format!("APPEND {segment_index}")).await?;
        }

        // FINALIZE
        let final_body = format!("command=FINALIZE&media_id={media_id}");
        let finalize_resp = auth_headers(
            self.http
                .post(api_base)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(final_body),
        )
        .send()
        .await
        .map_err(|e| ProviderError::Api(format!("Media FINALIZE error: {e}")))?;
        let finalize_json = Self::checked_upload_json(finalize_resp, "FINALIZE").await?;

        let ready = finalize_json
            .pointer("/processing_info/state")
            .and_then(|s| s.as_str())
            .map(|state| !matches!(state, "pending" | "in_progress"))
            .unwrap_or(true);
        if !ready {
            tracing::debug!("Media {media_id} still processing; the post goes out as pending");
        }

        Ok((media_id, ready))
    }

    /// B5 (OAuth mode): v2 upload at api.x.com — v1.1 is deprecated for
    /// API-key auth. INIT/APPEND(multipart raw)/FINALIZE, 4 MiB segments.
    async fn upload_media_v2(
        &self,
        access_token: &str,
        bytes: &[u8],
        mime_type: &str,
        media_category: &str,
    ) -> Result<(String, bool), ProviderError> {
        let api_base = "https://api.x.com/2/media/upload";
        let bearer = format!("Bearer {access_token}");

        // INIT
        let init_json: serde_json::Value = self
            .http
            .post(api_base)
            .header(header::AUTHORIZATION, &bearer)
            .json(&serde_json::json!({
                "command": "INIT",
                "media_type": mime_type,
                "total_bytes": bytes.len(),
                "media_category": media_category,
            }))
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("Media INIT error: {e}")))?
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let media_id = Self::media_id_from(&init_json)?;

        // APPEND — multipart RAW chunks (≤5 MiB; 4 MiB is safe).
        // wreq's RequestBuilder has no .multipart(), so the body is
        // assembled by hand (boring, dependency-free).
        for (segment_index, chunk) in bytes.chunks(4 * 1024 * 1024).enumerate() {
            let boundary = format!("sf-upload-{}", uuid::Uuid::new_v4().simple());
            let mut body: Vec<u8> = Vec::with_capacity(chunk.len() + 512);
            for (name, value) in [
                ("command", "APPEND"),
                ("media_id", media_id.as_str()),
                ("segment_index", segment_index.to_string().as_str()),
            ] {
                body.extend_from_slice(
                    format!(
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                    )
                    .as_bytes(),
                );
            }
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"media\"; filename=\"media.bin\"\r\nContent-Type: {mime_type}\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(chunk);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

            let append_resp = self
                .http
                .post(api_base)
                .header(header::AUTHORIZATION, &bearer)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(body)
                .send()
                .await
                .map_err(|e| {
                    ProviderError::Api(format!("Media APPEND {segment_index} error: {e}"))
                })?;
            Self::checked_upload_json(append_resp, &format!("APPEND {segment_index}")).await?;
        }

        // FINALIZE
        let finalize_json: serde_json::Value = self
            .http
            .post(api_base)
            .header(header::AUTHORIZATION, &bearer)
            .json(&serde_json::json!({
                "command": "FINALIZE",
                "media_id": media_id,
            }))
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("Media FINALIZE error: {e}")))?
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        let ready = finalize_json
            .pointer("/data/processing_info/state")
            .or_else(|| finalize_json.pointer("/processing_info/state"))
            .and_then(|s| s.as_str())
            .map(|state| !matches!(state, "pending" | "in_progress"))
            .unwrap_or(true);
        if !ready {
            tracing::debug!("Media {media_id} still processing; the post goes out as pending");
        }

        Ok((media_id, ready))
    }

    /// Read an upload-endpoint response as JSON, failing on any non-2xx
    /// (B5: APPEND/INIT failures were previously swallowed).
    async fn checked_upload_json(
        resp: wreq::Response,
        what: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Api(format!("Media {what} body: {e}")))?;
        if !status.is_success() {
            return Err(ProviderError::Api(format!(
                "X media {what} failed (HTTP {status}): {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| ProviderError::Api(format!("Media {what} JSON: {e}")))
    }

    /// Media id from either response shape: 1.1 `media_id_string`, v2
    /// `data.id`.
    fn media_id_from(json: &serde_json::Value) -> Result<String, ProviderError> {
        json.get("media_id_string")
            .and_then(|v| v.as_str())
            .or_else(|| json.pointer("/data/id").and_then(|v| v.as_str()))
            .map(str::to_string)
            .ok_or_else(|| ProviderError::Api(format!("X media upload returned no id: {json:?}")))
    }

    /// Upload every attachment. Returns the media ids plus whether *all* of
    /// them finished processing.
    async fn upload_media(
        &self,
        access_token: &str,
        media: &[MediaAttachment],
    ) -> Result<(Vec<String>, bool), ProviderError> {
        if media.is_empty() {
            return Ok((vec![], true));
        }
        let mut media_ids = Vec::with_capacity(media.len());
        let mut all_ready = true;
        for attachment in media {
            let (id, ready) = self
                .upload_single_media(access_token, &attachment.url, &attachment.mime_type)
                .await?;
            all_ready &= ready;
            media_ids.push(id);
        }
        Ok((media_ids, all_ready))
    }

    /// Create the tweet, and report whether every attached upload finished
    /// processing. X's shared publish body: the trait's `publish` returns the
    /// first element, `post_pending` inspects the second.
    pub(crate) async fn publish_with_media_state(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<(PublishResult, bool), ProviderError> {
        let mut media_ready = true;
        // Try GraphQL path if cookie auth
        if let Some((_auth_token, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut variables = serde_json::json!({
                "tweet_text": post.content,
                "media": {
                    "media_entities": [],
                    "possibly_sensitive": false
                },
                "semantic_annotation_ids": [],
                "dark_request": false,
            });

            // Thread linking: if in_reply_to is set, add the reply field
            // so X creates this tweet as a reply to the predecessor.
            if let Some(ref reply_to_id) = post.in_reply_to {
                variables["reply"] = serde_json::json!({
                    "in_reply_to_tweet_id": reply_to_id,
                    "reply_options": [],
                });
            }

            // Upload and attach media
            if !post.media.is_empty() {
                let (media_ids, ready) = self.upload_media(access_token, &post.media).await?;
                media_ready = ready;
                let entities: Vec<serde_json::Value> = media_ids
                    .iter()
                    .map(|id| serde_json::json!({"media_id": id, "tagged_users": []}))
                    .collect();
                variables["media"] = serde_json::json!({
                    "media_entities": entities,
                    "possibly_sensitive": false,
                });
            }

            Self::write_delay();

            let query_id = crate::social::x_queryids::lookup("CreateTweet")
                .ok_or_else(|| ProviderError::Api("Missing CreateTweet queryId".into()))?;
            let json = self
                .graphql_post(&query_id, "CreateTweet", &variables, access_token)
                .await?;

            let tweet_id = json["data"]["create_tweet"]["tweet_results"]["result"]["rest_id"]
                .as_str()
                .unwrap_or("");

            return Ok((
                PublishResult {
                    platform_post_url: if tweet_id.is_empty() {
                        None
                    } else {
                        Some(format!("https://x.com/i/status/{tweet_id}"))
                    },
                    platform_post_id: tweet_id.to_string(),
                    status: "published".into(),
                },
                media_ready,
            ));
        }

        // Fallback: v2 API with Bearer token
        let mut body = serde_json::json!({ "text": post.content });
        if !post.media.is_empty() {
            let (media_ids, ready) = self.upload_media(access_token, &post.media).await?;
            media_ready = ready;
            if !media_ids.is_empty() {
                body["media"] = serde_json::json!({ "media_ids": media_ids });
            }
        }
        // v22 Phase 2 (D.1): Send Idempotency-Key header on X v2 POST
        // /2/tweets. X deduplicates requests with the same key for 24h,
        // which prevents double-publish when a retry happens after a
        // crash that left the post in `publishing` state. The key is
        // generated per-post (posts.idempotency_key column) and
        // regenerated on `reset_post_for_republish`.
        let mut req = self
            .http
            .post("https://api.twitter.com/2/tweets")
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .json(&body);
        if let Some(ref key) = post.idempotency_key {
            req = req.header("Idempotency-Key", key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("X v2 POST error: {e}")))?;
        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        self.check_v2_response(status, &json)?;
        let post_id = json["data"]["id"].as_str().unwrap_or("").to_string();
        Ok((
            PublishResult {
                platform_post_url: Some(format!("https://twitter.com/user/status/{post_id}")),
                platform_post_id: post_id,
                status: "published".into(),
            },
            media_ready,
        ))
    }

    /// First attached media id, for both the v2 and the cookie/GraphQL tweet
    /// shape. Only reached for tweets that reported pending, which X only
    /// does when a video is still transcoding — so the first attachment is
    /// the one to poll.
    pub(crate) fn first_media_id(detail: &serde_json::Value) -> Option<String> {
        detail["data"]["attachments"]["media_ids"]
            .as_array()
            .and_then(|ids| ids.first())
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                detail["data"]["legacy"]["extended_entities"]["media"]
                    .as_array()?
                    .first()
                    .and_then(|m| m["id"].as_str())
                    .map(String::from)
            })
    }

    /// `command=STATUS` on the upload endpoint — the documented way to learn
    /// whether X finished encoding an upload. B5: authenticated per mode
    /// (cookie sessions send Cookie + csrf; OAuth uses the v2 endpoint with
    /// Bearer) — the unauthenticated GET used to read as "pending" forever
    /// or, worse, as a false "succeeded".
    pub(crate) async fn media_processing_state(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<String, ProviderError> {
        let resp = if Self::parse_cookie_token(access_token).is_some() {
            let (ct0, cookie_str) = self.effective_cookie_str(access_token);
            self.http
                .get("https://upload.twitter.com/1.1/media/upload.json")
                .query(&[
                    ("command", "STATUS"),
                    ("media_id", media_id),
                    ("media_category", "tweet_video"),
                ])
                .header(header::COOKIE, cookie_str)
                .header("x-csrf-token", ct0)
                .send()
                .await
        } else {
            self.http
                .get("https://api.x.com/2/media/upload")
                .query(&[("command", "STATUS"), ("media_id", media_id)])
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .send()
                .await
        }
        .map_err(|e| ProviderError::Api(format!("X media STATUS error: {e}")))?;

        // An unreadable STATUS reply means we cannot prove the media is ready.
        // Stay pending and let the bounded poll give up loudly rather than
        // report a video the user cannot play.
        if !resp.status().is_success() {
            return Ok("pending".into());
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(format!("X media STATUS body: {e}")))?;
        Ok(json
            .pointer("/processing_info/state")
            .or_else(|| json.pointer("/data/processing_info/state"))
            .and_then(|s| s.as_str())
            .unwrap_or("succeeded")
            .to_string())
    }

    // ── Page Parser ────────────────────────────────────────

    /// Parse one page of user tweets (GraphQL or v2) into ExternalPostData + cursor.
    fn parse_user_tweets_page(&self, response: &serde_json::Value) -> UserTweetsPage {
        let mut posts = Vec::new();
        let mut next_cursor = None;

        // ── OAuth v2 path: response["data"] is an array of tweet objects ──
        if let Some(data) = response["data"].as_array() {
            let media_map: std::collections::HashMap<String, MediaAttachment> = response
                ["includes"]["media"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let key = m["media_key"].as_str()?;
                            let media_type = m["type"].as_str().unwrap_or("photo");
                            if media_type == "video" {
                                let video_url = m["variants"]
                                    .as_array()
                                    .and_then(|variants| {
                                        let mut best: Option<(&str, u64)> = None;
                                        for v in variants {
                                            if v["content_type"].as_str() == Some("video/mp4") {
                                                if let (Some(url), Some(bitrate)) =
                                                    (v["url"].as_str(), v["bitrate"].as_u64())
                                                {
                                                    if best.is_none_or(|(_, b)| bitrate > b) {
                                                        best = Some((url, bitrate));
                                                    }
                                                }
                                            }
                                        }
                                        best.map(|(url, _)| url.to_string())
                                    })
                                    .or_else(|| m["url"].as_str().map(String::from));
                                video_url.map(|url| {
                                    (
                                        key.to_string(),
                                        MediaAttachment {
                                            url,
                                            mime_type: "video/mp4".to_string(),
                                            alt: m["alt_text"].as_str().map(String::from),
                                            poster_url: m["preview_image_url"]
                                                .as_str()
                                                .map(String::from),
                                        },
                                    )
                                })
                            } else {
                                let url = m["url"]
                                    .as_str()
                                    .or_else(|| m["preview_image_url"].as_str())?;
                                Some((
                                    key.to_string(),
                                    MediaAttachment {
                                        url: url.to_string(),
                                        mime_type: if media_type == "animated_gif" {
                                            "image/gif".to_string()
                                        } else {
                                            "image/jpeg".to_string()
                                        },
                                        alt: m["alt_text"].as_str().map(String::from),
                                        poster_url: None,
                                    },
                                ))
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            let author_map: std::collections::HashMap<String, serde_json::Value> = response
                ["includes"]["users"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|u| {
                            let uid = u["id"].as_str()?;
                            Some((uid.to_string(), u.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();

            for item in data {
                let id = item["id"].as_str().unwrap_or("").to_string();
                if id.is_empty() {
                    continue;
                }
                let text = item["text"].as_str().unwrap_or("").to_string();
                // No fabricated "now": an undated item is skipped, not
                // shown as just-posted.
                let Some(created_at) = item["created_at"]
                    .as_str()
                    .and_then(crate::social::common::parse_timestamp_opt)
                else {
                    continue;
                };

                let author_id = item["author_id"].as_str();
                let author_name = author_id
                    .and_then(|aid| author_map.get(aid))
                    .and_then(|u| u["name"].as_str().map(String::from));
                let author_handle = author_id
                    .and_then(|aid| author_map.get(aid))
                    .and_then(|u| u["username"].as_str().map(String::from));
                let author_avatar = author_id
                    .and_then(|aid| author_map.get(aid))
                    .and_then(|u| u["profile_image_url"].as_str().map(String::from));

                let mut media = Vec::new();
                if let Some(keys) = item["attachments"]["media_keys"].as_array() {
                    for key in keys.iter().filter_map(|k| k.as_str()) {
                        if let Some(att) = media_map.get(key) {
                            media.push(att.clone());
                        }
                    }
                }

                posts.push(ExternalPostData {
                    platform_post_id: id,
                    text,
                    author_name,
                    author_handle,
                    author_avatar,
                    created_at,
                    url: None,
                    media,
                    metadata: Some(item.clone()),
                });
            }
            next_cursor = response["meta"]["next_token"].as_str().map(String::from);
        } else if let Some(timeline) = response["data"].as_object() {
            // ── Cookie auth (GraphQL) path: navigate instructions → entries ──
            if let Some(instructions) = timeline.get("instructions").and_then(|i| i.as_array()) {
                for entry in instructions
                    .iter()
                    .filter_map(|inst| inst["entries"].as_array())
                    .flatten()
                {
                    let raw_result = match entry["content"]["itemContent"]["tweet_results"]
                        ["result"]
                        .as_object()
                    {
                        Some(r) => r,
                        None => continue,
                    };
                    let result = if raw_result.get("__typename").and_then(|t| t.as_str())
                        == Some("TweetWithState")
                    {
                        match raw_result.get("tweet").and_then(|t| t.as_object()) {
                            Some(inner) => inner,
                            None => raw_result,
                        }
                    } else {
                        raw_result
                    };
                    let legacy = match result.get("legacy") {
                        Some(l) => l,
                        None => continue,
                    };
                    // The timeline mixes the account's own tweets with RETWEETS
                    // and pinned-promoted entries. A retweet's author is the
                    // ORIGINAL poster, so importing it shows a stranger's post
                    // under the founder's feed. Skip both.
                    if legacy
                        .get("retweeted_status_result")
                        .is_some_and(|r| r.get("result").is_some())
                    {
                        continue;
                    }
                    if result
                        .get("itemContent")
                        .and_then(|c| c.get("tweetDisplayType"))
                        .and_then(|v| v.as_str())
                        == Some("SelfThread")
                    {
                        continue;
                    }
                    let id = result["rest_id"].as_str().unwrap_or("").to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let text = legacy["full_text"]
                        .as_str()
                        .or_else(|| legacy["text"].as_str())
                        .unwrap_or("")
                        .to_string();
                    // No fabricated "now": an undated item is skipped, not
                    // shown as just-posted.
                    let created_at_str = legacy["created_at"].as_str().unwrap_or("");
                    let created_at =
                        chrono::DateTime::parse_from_str(created_at_str, "%a %b %d %H:%M:%S %z %Y")
                            .ok()
                            .map(|dt| dt.with_timezone(&chrono::Utc));
                    let Some(created_at) = created_at else {
                        continue;
                    };
                    let user = &result["core"]["user_results"]["result"]["legacy"];
                    let author_name = user["name"].as_str().map(String::from);
                    let author_handle = user["screen_name"].as_str().map(String::from);

                    let mut media = Vec::new();
                    Self::extract_media_from_legacy(legacy, &mut media);
                    if media.is_empty() {
                        if let Some(media_details) =
                            result.get("mediaDetails").and_then(|m| m.as_array())
                        {
                            for m in media_details {
                                let media_type = m["type"].as_str().unwrap_or("photo");
                                if media_type == "video" || media_type == "animated_gif" {
                                    let video_url = m["video_info"]["variants"]
                                        .as_array()
                                        .and_then(|variants| {
                                            let mut best: Option<(&str, u64)> = None;
                                            for v in variants {
                                                if v["content_type"].as_str() == Some("video/mp4") {
                                                    if let (Some(url), Some(bitrate)) =
                                                        (v["url"].as_str(), v["bitrate"].as_u64())
                                                    {
                                                        if best.is_none_or(|(_, b)| bitrate > b) {
                                                            best = Some((url, bitrate));
                                                        }
                                                    }
                                                }
                                            }
                                            best.map(|(url, _)| url.to_string())
                                        });
                                    if let Some(url) = video_url {
                                        media.push(MediaAttachment {
                                            url,
                                            mime_type: "video/mp4".to_string(),
                                            alt: None,
                                            poster_url: m["media_url_https"]
                                                .as_str()
                                                .map(String::from),
                                        });
                                    }
                                } else {
                                    let url = m["media_url_https"]
                                        .as_str()
                                        .or_else(|| m["media_url"].as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if !url.is_empty() {
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
                    if media.is_empty() {
                        let rt_legacy = legacy
                            .pointer("/retweeted_status_result/result/result/legacy")
                            .or_else(|| legacy.pointer("/retweeted_status_result/result/legacy"));
                        if let Some(rt) = rt_legacy {
                            Self::extract_media_from_legacy(rt, &mut media);
                        }
                    }
                    if media.is_empty() {
                        // The quote's media lives at the TOP level of the
                        // tweet result (`result.quoted_status_result`), with
                        // a legacy nesting only on older payload shapes.
                        let qt_legacy = result
                            .get("quoted_status_result")
                            .and_then(|q| q.pointer("/result/legacy"))
                            .or_else(|| {
                                result
                                    .get("quoted_status_result")
                                    .and_then(|q| q.pointer("/result/result/legacy"))
                            })
                            .or_else(|| {
                                legacy.pointer("/quoted_status_result/result/legacy")
                            });
                        if let Some(qt) = qt_legacy {
                            Self::extract_media_from_legacy(qt, &mut media);
                        }
                    }

                    let author_avatar = user["profile_image_url_https"].as_str().map(String::from);

                    posts.push(ExternalPostData {
                        platform_post_id: id,
                        text,
                        author_name,
                        author_handle,
                        author_avatar,
                        created_at,
                        url: None,
                        media,
                        metadata: Some(serde_json::Value::Object(result.clone())),
                    });
                }
            }
            next_cursor = Self::extract_next_cursor(response);
        }

        UserTweetsPage { posts, next_cursor }
    }

    // ── Media Extraction Helper ──────────────────────────────

    /// Extract media attachments from a legacy tweet object's extended_entities or entities.
    /// Works for original tweets, retweets, and quote tweets.
    fn extract_media_from_legacy(legacy: &serde_json::Value, media: &mut Vec<MediaAttachment>) {
        let arr = legacy["extended_entities"]["media"]
            .as_array()
            .or_else(|| legacy["entities"]["media"].as_array());
        if let Some(arr) = arr {
            for m in arr {
                let media_type = m["type"].as_str().unwrap_or("photo");
                // Thumbnail/poster URL available for all media types
                let poster = m["media_url_https"]
                    .as_str()
                    .or_else(|| m["media_url"].as_str())
                    .map(|s| s.to_string());
                if media_type == "video" || media_type == "animated_gif" {
                    // Extract video URL from video_info.variants — prefer highest-bitrate mp4
                    let video_url = m["video_info"]["variants"].as_array().and_then(|variants| {
                        let mut best: Option<(&str, u64)> = None;
                        for v in variants {
                            if v["content_type"].as_str() == Some("video/mp4") {
                                if let (Some(url), Some(bitrate)) =
                                    (v["url"].as_str(), v["bitrate"].as_u64())
                                {
                                    if best.is_none_or(|(_, b)| bitrate > b) {
                                        best = Some((url, bitrate));
                                    }
                                }
                            }
                        }
                        best.map(|(url, _)| url.to_string())
                    });
                    if let Some(video_url) = video_url {
                        media.push(MediaAttachment {
                            url: video_url,
                            mime_type: "video/mp4".to_string(),
                            alt: m["alt_text"].as_str().map(String::from),
                            poster_url: poster,
                        });
                    } else if let Some(url) = poster {
                        // Fallback to thumbnail
                        media.push(MediaAttachment {
                            url,
                            mime_type: "image/jpeg".to_string(),
                            alt: m["alt_text"].as_str().map(String::from),
                            poster_url: None,
                        });
                    }
                } else {
                    // Photo — use media_url_https
                    if let Some(url) = poster {
                        media.push(MediaAttachment {
                            url,
                            mime_type: "image/jpeg".to_string(),
                            alt: m["alt_text"].as_str().map(String::from),
                            poster_url: None,
                        });
                    }
                }
            }
        }
    }

    // ── Rate limit + write delay helpers ─────────────────────

    fn write_delay() {
        let delay = rand::thread_rng().gen_range(1.5..4.0);
        std::thread::sleep(Duration::from_secs_f64(delay));
    }
}

// ════════════════════════════════════════════════════════════════
// SocialProvider Trait Implementation
// ════════════════════════════════════════════════════════════════

/// Result of parsing a single page of user tweets.
struct UserTweetsPage {
    posts: Vec<ExternalPostData>,
    next_cursor: Option<String>,
}

#[async_trait]
impl SocialProvider for XProvider {
    fn identifier(&self) -> &'static str {
        "x"
    }

    fn name(&self) -> &'static str {
        "X (Twitter)"
    }

    fn scopes(&self) -> Vec<String> {
        // Each v2 endpoint the tools call needs its scope: like/unlike →
        // like.write, bookmarks → bookmark.read/write, follow/unfollow →
        // follows.*, list timeline → list.read, DMs → dm.read/write. Request
        // them all so OAuth-mode tools stay functional; enable the matching
        // products in the X developer portal or authorize fails on the
        // missing scope. Cookie-mode calls are unaffected.
        vec![
            "tweet.read".into(),
            "tweet.write".into(),
            "users.read".into(),
            "offline.access".into(),
            "like.read".into(),
            "like.write".into(),
            "bookmark.read".into(),
            "bookmark.write".into(),
            "follows.read".into(),
            "follows.write".into(),
            "list.read".into(),
            "dm.read".into(),
            "dm.write".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        4000
    }

    /// X counts WEIGHTED characters (twitter-text v3): an emoji or a CJK
    /// glyph costs 2. The composer already implements that rule, so the
    /// server measuring bytes made the two disagree on any post with an
    /// emoji in it.
    fn counting_rule(&self) -> super::CountingRule {
        super::CountingRule::Weighted
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    /// X OAuth2 access tokens live ~2 hours. Cookie-authenticated integrations
    /// store no refresh token, so the scheduler's `refresh_token IS NOT NULL`
    /// filter leaves them alone and their expiry surfaces as `refresh_needed`.
    fn needs_cron_refresh(&self) -> bool {
        true
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let challenge = common::generate_code_challenge(code_verifier);
        let params = [
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", redirect_uri),
            ("scope", &self.scopes().join(" ")),
            ("state", state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ];
        let url = url::Url::parse_with_params(self.oauth_authorize_endpoint(), &params)
            .map_err(|e| ProviderError::Auth(format!("URL parse: {e}")))?;
        Ok(AuthUrlResponse {
            url: url.to_string(),
        })
    }

    async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );
        let form_body = format!(
            "grant_type=authorization_code&code={}&code_verifier={}&redirect_uri={}&client_id={}",
            urlencoding::encode(code),
            urlencoding::encode(code_verifier),
            urlencoding::encode(redirect_uri),
            urlencoding::encode(&self.client_id),
        );
        let json: serde_json::Value = self
            .http
            .post(self.oauth_token_endpoint())
            .header(header::AUTHORIZATION, format!("Basic {credentials}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("OAuth token exchange error: {e}")))?
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();
        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        let user_info: serde_json::Value = self
            .http
            .get("https://api.twitter.com/2/users/me?user.fields=name,username,profile_image_url")
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("User info error: {e}")))?
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        if user_info.get("data").is_none() {
            tracing::warn!("X /2/users/me returned no data: {user_info}");
        }

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: user_info["data"]["id"].as_str().unwrap_or("").to_string(),
            name: user_info["data"]["name"].as_str().unwrap_or("").to_string(),
            username: user_info["data"]["username"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            picture: user_info["data"]["profile_image_url"]
                .as_str()
                .map(String::from),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );
        let form_body = format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            urlencoding::encode(refresh_token),
            urlencoding::encode(&self.client_id),
        );
        let json: serde_json::Value = self
            .http
            .post(self.oauth_token_endpoint())
            .header(header::AUTHORIZATION, format!("Basic {credentials}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(format!("Token refresh error: {e}")))?
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        Ok(AuthToken {
            access_token: json["access_token"]
                .as_str()
                .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
                .to_string(),
            refresh_token: json["refresh_token"].as_str().map(String::from),
            expires_in: json["expires_in"].as_u64().map(|v| v as u32),
            provider_user_id: String::new(),
            name: String::new(),
            username: String::new(),
            picture: None,
        })
    }

    /// §8.2: delete a published tweet — both auth modes (GraphQL DeleteTweet
    /// for cookie sessions, v2 DELETE /2/tweets/:id for OAuth).
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on X".into(),
            ));
        }
        self.delete_tweet(access_token, platform_post_id)
            .await
            .map(|_| ())
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        self.publish_with_media_state(access_token, post)
            .await
            .map(|(result, _media_ready)| result)
    }

    /// Publish, and report pending while X is still transcoding an attached
    /// video.
    ///
    /// The chunked upload's FINALIZE returns as soon as the bytes land; a
    /// tweet attached to a still-encoding video is published but unplayable
    /// for a minute or two. Polling to `succeeded` is the difference between
    /// "posted" and "posted and watchable".
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let (result, media_ready) = self.publish_with_media_state(access_token, post).await?;
        if media_ready {
            Ok(result)
        } else {
            Ok(PublishResult {
                status: "pending".into(),
                ..result
            })
        }
    }

    /// Ask X whether the upload behind this tweet finished transcoding.
    async fn check_post_status(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        // Text and image tweets carry no `processing_info` — they are done
        // the moment the tweet exists, so the poll is a no-op for them.
        let detail = self.tweet_detail(access_token, platform_post_id).await?;
        let state = match Self::first_media_id(&detail) {
            None => "succeeded".to_string(),
            Some(media_id) => self.media_processing_state(access_token, &media_id).await?,
        };

        let status = match state.as_str() {
            "succeeded" => "published",
            "failed" => {
                return Err(ProviderError::Api(format!(
                    "X failed to process the video attached to tweet {platform_post_id}. Check it \
                     is a supported MP4 under 140s, then republish."
                )))
            }
            _ => "pending",
        };

        Ok(PublishResult {
            platform_post_id: platform_post_id.to_string(),
            platform_post_url: Some(format!("https://x.com/i/status/{platform_post_id}")),
            status: status.into(),
        })
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn reconnect(
        &self,
        access_token: &str,
        _internal_id: &str,
        _page_id: &str,
    ) -> Result<super::ReconnectResult, ProviderError> {
        // Try cookie-based auth first (access_token may be a JSON blob)
        let json = if let Some((at, ct)) = Self::parse_cookie_token(access_token) {
            let cookie_str = Self::extract_cookie_string(access_token)
                .unwrap_or_else(|| format!("auth_token={at}; ct0={ct};"));
            self.v1_get_with_cookies(
                "https://api.twitter.com/2/users/me?user.fields=name,username,profile_image_url",
                &cookie_str,
            )
            .await?
        } else {
            self.v2_get(
                "https://api.twitter.com/2/users/me?user.fields=name,username,profile_image_url",
                access_token,
            )
            .await?
        };
        let data = &json["data"];
        Ok(super::ReconnectResult {
            id: data["id"].as_str().unwrap_or("").to_string(),
            name: data["name"].as_str().unwrap_or("").to_string(),
            access_token: access_token.to_string(),
            picture: data["profile_image_url"].as_str().map(|s| s.to_string()),
            username: data["username"].as_str().map(|s| s.to_string()),
        })
    }

    /// X-specific user-facing error copy (v25 §2 row 9). The GraphQL and v2
    /// response checkers both route their `Api` messages through this.
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "X rate limit reached. Forge retries with backoff automatically — \
                 lower the publish rate or move to a higher X API tier to remove the wait."
                    .into(),
            ),
            Some(ErrorKind::Duplicate) => Some(
                "X rejected this post as a duplicate of a recent post. \
                 Change the text or wait for X's duplicate window to pass."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "X rejected the attached media. Check that the file is a supported \
                 image/GIF (under 5 MB) or MP4 (under 140 s) and that the media URL \
                 is publicly fetchable by X."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "X session expired or the cookies are stale. Re-paste your X cookies \
                 (auth_token + ct0) on the Channels page, or reconnect via OAuth."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "X denied this request (HTTP 403). The account is typically suspended, \
                 temporarily blocked from posting, or missing a required API scope."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => {
                Some("X post or user not found — it may be deleted or protected.".into())
            }
            Some(ErrorKind::InvalidRequest) => Some(
                "X rejected the request as invalid. Check the text length (280 chars, \
                 25 000 for premium) and the media count (4 images / 1 video)."
                    .into(),
            ),
            None => None,
        }
    }

    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        // A cookie-connected row has no OAuth bearer, and X's v2 API
        // rejects cookie auth with 403 — so the v2 endpoint 401/403s for
        // every account that logged in with cookies. The cookie path
        // reads the same two counts (followers, following) off the
        // GraphQL profile that `get_me` already fetches.
        let json = if Self::parse_cookie_token(access_token).is_some() {
            self.get_me(access_token).await?
        } else {
            self.v2_get(
                "https://api.twitter.com/2/users/me?user.fields=public_metrics",
                access_token,
            )
            .await?
        };
        Ok(Self::parse_account_metrics(&json))
    }

    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let url = format!(
            "https://api.twitter.com/2/tweets/{platform_post_id}?tweet.fields=public_metrics"
        );
        let json = self
            .v2_get_with_credential(&url, access_token)
            .await?;
        Ok(Self::parse_tweet_metrics(&json))
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        tracing::info!(
            "X get_recent_posts: internal_id='{}' limit={}",
            internal_id,
            limit,
        );
        let page_size = limit.min(100);
        let mut all_posts = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages_fetched = 0u32;
        const MAX_PAGES: u32 = 10;

        loop {
            let response = self
                .user_tweets(access_token, internal_id, page_size, cursor.as_deref())
                .await?;
            pages_fetched += 1;
            let data_count = response["data"].as_array().map(|a| a.len()).unwrap_or(0);
            tracing::info!(
                "X get_recent_posts page {}: data_count={}, total_so_far={}",
                pages_fetched,
                data_count,
                all_posts.len(),
            );

            let page = self.parse_user_tweets_page(&response);
            let page_post_count = page.posts.len() as u32;
            all_posts.extend(page.posts);

            cursor = page.next_cursor;

            if all_posts.len() as u32 >= limit {
                tracing::info!(
                    "X get_recent_posts: reached limit={} (got {}), stopping",
                    limit,
                    all_posts.len(),
                );
                break;
            }
            if cursor.is_none() {
                tracing::info!(
                    "X get_recent_posts: no more pages after page {}",
                    pages_fetched,
                );
                break;
            }
            if pages_fetched >= MAX_PAGES {
                tracing::info!(
                    "X get_recent_posts: hit MAX_PAGES={} after {} posts",
                    MAX_PAGES,
                    all_posts.len(),
                );
                break;
            }
            if page_post_count == 0 {
                tracing::info!(
                    "X get_recent_posts: empty page after {} posts",
                    all_posts.len(),
                );
                break;
            }

            // Small delay between pages to avoid rate limiting
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        tracing::info!(
            "X get_recent_posts: returning {} posts (pages={})",
            all_posts.len(),
            pages_fetched,
        );
        Ok(all_posts)
    }

    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        let detail = self.tweet_detail(access_token, platform_post_id).await?;
        Ok(Self::extract_public_metrics(&detail)
            .map(|m| serde_json::json!({ "public_metrics": m })))
    }

    /// Read the replies under a tweet.
    ///
    /// Cookie auth reads the TweetDetail GraphQL conversation; OAuth v2 has no
    /// replies endpoint, so the conversation is reconstructed from recent
    /// search on `conversation_id`. Both land in the same shape via the two
    /// parsers below.
    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        if Self::parse_cookie_token(access_token).is_some() {
            // X rejects TweetDetail with HTTP 422 when any of these is missing; same set as tweet_detail.
            let vars = serde_json::json!({
                "focalTweetId": platform_post_id,
                "count": 40,
                "referrer": "tweet",
                "with_rux_injections": false,
                "includePromotedContent": false,
                "rankingMode": "Relevance",
                "withCommunity": true,
                "withQuickPromoteEligibilityTweetFields": true,
                "withBirdwatchNotes": true,
                "withVoice": true,
            });
            let qid = crate::social::x_queryids::lookup("TweetDetail")
                .ok_or_else(|| ProviderError::Api("Missing TweetDetail queryId".into()))?;
            let json = self
                .graphql_get(&qid, "TweetDetail", &vars, access_token)
                .await?;
            return Ok(Self::parse_graphql_comments(&json, platform_post_id));
        }

        let url = format!(
            "https://api.twitter.com/2/tweets/search/recent?query={}&tweet.fields=created_at,public_metrics,referenced_tweets,in_reply_to_user_id&expansions=author_id&user.fields=name,username,profile_image_url",
            urlencoding::encode(&format!("conversation_id:{platform_post_id}"))
        );
        let json = self.v2_get(&url, access_token).await?;
        Ok(Self::parse_v2_conversation(&json, platform_post_id))
    }

    /// @-autocomplete over user search (`GET /2/users/search`).
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let url = format!(
            "https://api.twitter.com/2/users/search?query={}&max_results=25&user.fields=name,username,profile_image_url",
            urlencoding::encode(query)
        );
        let json = self.v2_get(&url, access_token).await?;
        Ok(Self::parse_user_search(&json))
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        if let Some((_auth_token, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut variables = serde_json::json!({
                "tweet_text": post.content,
                "media": {
                    "media_entities": [],
                    "possibly_sensitive": false
                },
                "semantic_annotation_ids": [],
                "dark_request": false,
                "reply": {
                    "in_reply_to_tweet_id": comment_id,
                    "exclude_reply_user_ids": []
                },
            });

            if !post.media.is_empty() {
                let (media_ids, _ready) = self.upload_media(access_token, &post.media).await?;
                let entities: Vec<serde_json::Value> = media_ids
                    .iter()
                    .map(|id| serde_json::json!({"media_id": id, "tagged_users": []}))
                    .collect();
                variables["media"] = serde_json::json!({
                    "media_entities": entities,
                    "possibly_sensitive": false,
                });
            }

            Self::write_delay();

            let query_id = crate::social::x_queryids::lookup("CreateTweet")
                .ok_or_else(|| ProviderError::Api("Missing CreateTweet queryId".into()))?;
            let json = self
                .graphql_post(&query_id, "CreateTweet", &variables, access_token)
                .await?;

            let tweet_id = json["data"]["create_tweet"]["tweet_results"]["result"]["rest_id"]
                .as_str()
                .unwrap_or("");

            return Ok(PublishResult {
                platform_post_url: if tweet_id.is_empty() {
                    None
                } else {
                    Some(format!("https://x.com/i/status/{tweet_id}"))
                },
                platform_post_id: tweet_id.to_string(),
                status: "published".into(),
            });
        }

        let mut body = serde_json::json!({
            "text": post.content,
            "reply": { "in_reply_to_tweet_id": comment_id }
        });
        if !post.media.is_empty() {
            let (media_ids, _ready) = self.upload_media(access_token, &post.media).await?;
            if !media_ids.is_empty() {
                body["media"] = serde_json::json!({ "media_ids": media_ids });
            }
        }
        let json = self
            .v2_post("https://api.twitter.com/2/tweets", access_token, &body)
            .await?;
        let post_id = json["data"]["id"].as_str().unwrap_or("").to_string();
        Ok(PublishResult {
            platform_post_url: Some(format!("https://twitter.com/user/status/{post_id}")),
            platform_post_id: post_id,
            status: "published".into(),
        })
    }

    async fn send_dm(
        &self,
        access_token: &str,
        recipient: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "event": {
                "type": "message_create",
                "message_create": {
                    "target": {
                        "recipient_id": recipient
                    },
                    "message_data": {
                        "text": post.content
                    }
                }
            }
        });
        let json = self
            .v2_post(
                "https://api.twitter.com/2/dm_conversations/with/{recipient_id}/messages",
                access_token,
                &body,
            )
            .await?;
        let message_id = json["data"]["dm_event_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(PublishResult {
            platform_post_id: message_id,
            platform_post_url: None,
            status: "sent".into(),
        })
    }

    // This provider has a DM surface, so `dm_capable()` reports true — which is
    // what lets the /dms page learn it from the API instead of guessing a
    // provider list. NOTE this says the PROVIDER can do DMs, not that any
    // given CHANNEL can: X's DM API is bearer-only, and the API level
    // refuses a COOKIE-auth channel for that reason.
    fn dm_capable(&self) -> bool {
        true
    }

    async fn get_dm_conversations(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<Vec<super::DmConversation>, ProviderError> {
        let url = format!(
            "https://api.twitter.com/2/dm_conversations?max_results={}&dm_event.fields=created_at,message_create,text",
            limit.min(50)
        );
        let json = self.v2_get(&url, access_token).await?;
        let mut conversations = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for conv in data {
                let id = conv["dm_conversation_id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let participant = conv["participants"]
                    .as_array()
                    .and_then(|p| p.first())
                    .and_then(|p| p["user_id"].as_str())
                    .unwrap_or("")
                    .to_string();
                let last_message = conv["dm_events"]
                    .as_array()
                    .and_then(|e| e.last())
                    .and_then(|e| e["text"].as_str())
                    .map(|s| s.to_string());
                let last_message_at = conv["dm_events"]
                    .as_array()
                    .and_then(|e| e.last())
                    .and_then(|e| e["created_at"].as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                conversations.push(super::DmConversation {
                    id,
                    participant,
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
            "https://api.twitter.com/2/dm_conversations/{}/messages?max_results={}&dm_event.fields=created_at,message_create,text,sender_id",
            conversation_id,
            limit.min(50)
        );
        let json = self.v2_get(&url, access_token).await?;
        let mut messages = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for msg in data {
                let id = msg["dm_event_id"].as_str().unwrap_or("").to_string();
                let sender = msg["sender_id"].as_str().unwrap_or("").to_string();
                let content = msg["text"].as_str().unwrap_or("").to_string();
                let created_at = msg["created_at"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
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

// ════════════════════════════════════════════════════════════════
// Public API Methods (called from MCP tools)
// ════════════════════════════════════════════════════════════════

impl XProvider {
    // ── User / Profile ───────────────────────────────────────

    pub async fn get_me(&self, access_token: &str) -> Result<serde_json::Value, ProviderError> {
        if let Some((_auth_token, _ct0)) = Self::parse_cookie_token(access_token) {
            let url = "https://x.com/i/api/1.1/account/multi/list.json";
            let (ct0, cs) = self.effective_cookie_str(access_token);
            let mut last_err = None;
            for attempt in 0..=RATE_LIMIT_RETRIES {
                if attempt > 0 {
                    rate_limit_sleep(attempt - 1).await;
                }
                let resp = self
                    .http
                    .get(url)
                    .header("x-csrf-token", &ct0)
                    .header("Cookie", &cs)
                    .send()
                    .await
                    .map_err(|e| ProviderError::Api(format!("X whoami error: {e}")))?;
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    // ... parse and return below
                    let json: serde_json::Value =
                        serde_json::from_str(&body_text).map_err(|e| {
                            ProviderError::Api(format!(
                                "JSON parse: {e}: {}",
                                body_text.chars().take(100).collect::<String>()
                            ))
                        })?;
                    let user = json["users"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .ok_or_else(|| ProviderError::Api("No user found".into()))?;
                    let screen_name = user["screen_name"].as_str().unwrap_or("");
                    let user_id = user["user_id"].as_str().unwrap_or("");
                    if screen_name.is_empty() && user_id.is_empty() {
                        return Ok(json);
                    }
                    let vars = serde_json::json!({"screen_name": screen_name, "withSafetyModeUserFields": true});
                    let qid =
                        crate::social::x_queryids::lookup("UserByScreenName").ok_or_else(|| {
                            ProviderError::Api("Missing UserByScreenName queryId".into())
                        })?;
                    let profile = self
                        .graphql_get(&qid, "UserByScreenName", &vars, access_token)
                        .await?;
                    let data = profile
                        .pointer("/data/user/result")
                        .and_then(|r| r.get("legacy"))
                        .or_else(|| profile.get("data"));
                    let name = data
                        .and_then(|d| d.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let username = data
                        .and_then(|d| d.get("screen_name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let avatar = profile
                        .pointer("/data/user/result/legacy/profile_image_url_https")
                        .or_else(|| profile.pointer("/data/user/result/avatar/image_url"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    let description = data
                        .and_then(|d| d.get("description"))
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    let followers = data
                        .and_then(|d| d.get("followers_count"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0);
                    let following = data
                        .and_then(|d| d.get("friends_count"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0);
                    return Ok(
                        serde_json::json!({"data": {"id": user_id, "name": name, "username": username, "profile_image_url": avatar, "description": description, "public_metrics": {"followers_count": followers, "following_count": following}}}),
                    );
                }
                if status.as_u16() == 429
                    && is_rate_limited(&body_text)
                    && attempt < RATE_LIMIT_RETRIES
                {
                    last_err = Some(format!("rate limited (attempt {})", attempt + 1));
                    continue;
                }
                let snippet = body_text.chars().take(200).collect::<String>();
                return Err(ProviderError::Api(format!(
                    "X whoami HTTP {status}: {snippet}"
                )));
            }
            return Err(ProviderError::Api(format!(
                "X whoami failed after {} retries: {}",
                RATE_LIMIT_RETRIES,
                last_err.unwrap_or_default()
            )));
        }
        // Fallback: v2 REST API (OAuth Bearer) — for non-cookie auth
        let v2_url = "https://api.twitter.com/2/users/me?user.fields=profile_image_url,description,public_metrics".to_string();
        self.v2_get(&v2_url, access_token).await
    }

    pub async fn user_lookup(
        &self,
        access_token: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            // Cookie auth: use x.com internal v2 API with cookie headers
            let url = format!(
                "https://x.com/i/api/2/users/{user_id}.json?user.fields=profile_image_url,description,public_metrics"
            );
            let (ct0, cs) = self.effective_cookie_str(access_token);
            let mut last_err = None;
            for attempt in 0..=RATE_LIMIT_RETRIES {
                if attempt > 0 {
                    rate_limit_sleep(attempt - 1).await;
                }
                let resp = self
                    .http
                    .get(&url)
                    .header("x-csrf-token", &ct0)
                    .header("Cookie", &cs)
                    .send()
                    .await
                    .map_err(|e| ProviderError::Api(format!("X user lookup error: {e}")))?;
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    let json: serde_json::Value = serde_json::from_str(&body_text)
                        .map_err(|e| ProviderError::Api(format!("X JSON parse error: {e}")))?;
                    return Ok(json);
                }
                if status.as_u16() == 429
                    && is_rate_limited(&body_text)
                    && attempt < RATE_LIMIT_RETRIES
                {
                    last_err = Some(format!("rate limited (attempt {})", attempt + 1));
                    continue;
                }
                let snippet = body_text.chars().take(200).collect::<String>();
                return Err(ProviderError::Api(format!(
                    "X user lookup failed: HTTP {status}: {snippet}"
                )));
            }
            return Err(ProviderError::Api(format!(
                "X user lookup failed after {} retries: {}",
                RATE_LIMIT_RETRIES,
                last_err.unwrap_or_default()
            )));
        }
        // Fallback: v2 REST API (works with Bearer token in default_headers)
        let url = format!(
            "https://api.twitter.com/2/users/{user_id}?user.fields=profile_image_url,description,public_metrics"
        );
        self.v2_get(&url, access_token).await
    }

    pub async fn user_lookup_by_username(
        &self,
        access_token: &str,
        username: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars =
                serde_json::json!({"screen_name": username, "withSafetyModeUserFields": true});
            let qid = crate::social::x_queryids::lookup("UserByScreenName")
                .ok_or_else(|| ProviderError::Api("Missing UserByScreenName queryId".into()))?;
            let json = self
                .graphql_get(&qid, "UserByScreenName", &vars, access_token)
                .await?;
            let result = json.pointer("/data/user/result").cloned().unwrap_or(json);
            Ok(serde_json::json!({"data": {
                "id": result["rest_id"].as_str().unwrap_or(""),
                "name": result["legacy"]["name"].as_str().unwrap_or(""),
                "username": result["legacy"]["screen_name"].as_str().unwrap_or(""),
                "description": result["legacy"]["description"].as_str().unwrap_or(""),
                "profile_image_url": result["legacy"]["profile_image_url_https"].as_str().unwrap_or(""),
                "public_metrics": result["legacy"]["public_metrics"],
            }}))
        } else {
            let url = format!(
                "https://api.twitter.com/2/users/by/username/{username}?user.fields=profile_image_url,description,public_metrics"
            );
            self.v2_get(&url, access_token).await
        }
    }

    // ── Timeline ──────────────────────────────────────────────

    pub async fn home_timeline(
        &self,
        access_token: &str,
        user_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "count": max_results.clamp(1, 100),
                "includePromotedContent": false,
                "latestControlAvailable": true,
                "requestContext": "launch",
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("HomeLatestTimeline")
                .ok_or_else(|| ProviderError::Api("Missing HomeLatestTimeline queryId".into()))?;
            let json = self
                .graphql_get(&qid, "HomeLatestTimeline", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let timeline = json
                .pointer("/data/home/home_timeline_urt")
                .cloned()
                .unwrap_or(json.clone());
            let mut result = serde_json::json!({ "data": timeline });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/users/{user_id}/timelines/reverse_chronological?max_results={}&tweet.fields=created_at,public_metrics,attachments&user.fields=profile_image_url&expansions=author_id,attachments.media_keys&media.fields=url,preview_image_url",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    pub async fn user_tweets(
        &self,
        access_token: &str,
        user_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "userId": user_id,
                "count": max_results.clamp(1, 100),
                "includePromotedContent": false,
                "withQuickPromoteEligibilityTweetFields": true,
                "withVoice": true,
                "withV2Timeline": true,
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("UserTweets")
                .ok_or_else(|| ProviderError::Api("Missing UserTweets queryId".into()))?;
            let json = self
                .graphql_get(&qid, "UserTweets", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let timeline = json
                .pointer("/data/user/result/timeline_v2/timeline")
                .or_else(|| json.pointer("/data/user/result/timeline/timeline"))
                .cloned()
                .unwrap_or(json);
            let mut result = serde_json::json!({ "data": timeline });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/users/{user_id}/tweets?max_results={}&tweet.fields=created_at,public_metrics&expansions=author_id,attachments.media_keys&media.fields=url,preview_image_url&user.fields=profile_image_url",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    pub async fn tweet_detail(
        &self,
        access_token: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({
                "focalTweetId": tweet_id,
                "count": 40,
                "referrer": "tweet",
                "with_rux_injections": false,
                "includePromotedContent": false,
                "rankingMode": "Relevance",
                "withCommunity": true,
                "withQuickPromoteEligibilityTweetFields": true,
                "withBirdwatchNotes": true,
                "withVoice": true,
            });
            let qid = crate::social::x_queryids::lookup("TweetDetail")
                .ok_or_else(|| ProviderError::Api("Missing TweetDetail queryId".into()))?;
            let json = self
                .graphql_get(&qid, "TweetDetail", &vars, access_token)
                .await?;
            // Extract the focal tweet from the response
            let tweet = json
                .pointer("/data/threaded_conversation_with_injections_v2/instructions")
                .and_then(|inst| inst.as_array())
                .and_then(|arr| arr.first())
                .and_then(|first| first.get("entries"))
                .and_then(|entries| entries.as_array())
                .and_then(|arr| arr.first())
                .and_then(|entry| entry.pointer("/content/itemContent/tweet_results/result"))
                .cloned()
                .unwrap_or(json);
            Ok(serde_json::json!({ "data": tweet }))
        } else {
            let url = format!(
                "https://api.twitter.com/2/tweets/{tweet_id}?tweet.fields=created_at,public_metrics,attachments,referenced_tweets&expansions=author_id,attachments.media_keys,referenced_tweets.id&user.fields=profile_image_url&media.fields=url,preview_image_url"
            );
            self.v2_get(&url, access_token).await
        }
    }

    pub async fn search_tweets(
        &self,
        access_token: &str,
        query: &str,
        max_results: u32,
        next_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "rawQuery": query,
                "count": max_results.clamp(1, 100),
                "querySource": "typed_query",
                "product": "Top",
            });
            if let Some(cursor) = next_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("SearchTimeline")
                .ok_or_else(|| ProviderError::Api("Missing SearchTimeline queryId".into()))?;
            let json = self
                .graphql_post(&qid, "SearchTimeline", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let mut result = serde_json::json!({ "data": json });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let encoded_query = urlencoding::encode(query);
        let mut url = format!(
            "https://api.twitter.com/2/tweets/search/recent?query={encoded_query}&max_results={}&tweet.fields=created_at,public_metrics,attachments&expansions=author_id,attachments.media_keys&user.fields=profile_image_url&media.fields=url,preview_image_url",
            max_results.min(100)
        );
        if let Some(token) = next_token {
            url.push_str(&format!("&next_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    // ── Tweet CRUD ────────────────────────────────────────────

    pub async fn delete_tweet(
        &self,
        access_token: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({
                "tweet_id": tweet_id,
                "dark_request": false,
            });
            let qid = crate::social::x_queryids::lookup("DeleteTweet")
                .ok_or_else(|| ProviderError::Api("Missing DeleteTweet queryId".into()))?;
            let json = self
                .graphql_post(&qid, "DeleteTweet", &vars, access_token)
                .await?;
            Self::write_delay();
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/tweets/{tweet_id}");
        self.v2_delete(&url, access_token).await
    }

    pub async fn like_tweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({ "tweet_id": tweet_id });
            let qid = crate::social::x_queryids::lookup("FavoriteTweet")
                .ok_or_else(|| ProviderError::Api("Missing FavoriteTweet queryId".into()))?;
            let json = self
                .graphql_post(&qid, "FavoriteTweet", &vars, access_token)
                .await?;
            Self::write_delay();
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/likes");
        let body = serde_json::json!({"tweet_id": tweet_id});
        self.v2_post(&url, access_token, &body).await
    }

    pub async fn unlike_tweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({
                "tweet_id": tweet_id,
                "dark_request": false,
            });
            let qid = crate::social::x_queryids::lookup("UnfavoriteTweet")
                .ok_or_else(|| ProviderError::Api("Missing UnfavoriteTweet queryId".into()))?;
            let json = self
                .graphql_post(&qid, "UnfavoriteTweet", &vars, access_token)
                .await?;
            Self::write_delay();
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/likes/{tweet_id}");
        self.v2_delete(&url, access_token).await
    }

    pub async fn retweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({
                "tweet_id": tweet_id,
                "dark_request": false,
            });
            let qid = crate::social::x_queryids::lookup("CreateRetweet")
                .ok_or_else(|| ProviderError::Api("Missing CreateRetweet queryId".into()))?;
            let json = self
                .graphql_post(&qid, "CreateRetweet", &vars, access_token)
                .await?;
            Self::write_delay();
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/retweets");
        let body = serde_json::json!({"tweet_id": tweet_id});
        self.v2_post(&url, access_token, &body).await
    }

    pub async fn unretweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({
                "source_tweet_id": tweet_id,
                "dark_request": false,
            });
            let qid = crate::social::x_queryids::lookup("DeleteRetweet")
                .ok_or_else(|| ProviderError::Api("Missing DeleteRetweet queryId".into()))?;
            let json = self
                .graphql_post(&qid, "DeleteRetweet", &vars, access_token)
                .await?;
            Self::write_delay();
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/retweets/{tweet_id}");
        self.v2_delete(&url, access_token).await
    }

    // ── Bookmarks ─────────────────────────────────────────────

    pub async fn bookmarks(
        &self,
        access_token: &str,
        user_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "count": max_results.clamp(1, 100),
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("Bookmarks")
                .ok_or_else(|| ProviderError::Api("Missing Bookmarks queryId".into()))?;
            let json = self
                .graphql_get(&qid, "Bookmarks", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let timeline = json
                .pointer("/data/bookmark_timeline_v2/timeline")
                .or_else(|| json.pointer("/data/bookmark_timeline/timeline"))
                .cloned()
                .unwrap_or(json);
            let mut result = serde_json::json!({ "data": timeline });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/users/{user_id}/bookmarks?max_results={}&tweet.fields=created_at,public_metrics,attachments&expansions=author_id,attachments.media_keys&media.fields=url,preview_image_url&user.fields=profile_image_url",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    pub async fn bookmark_tweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({ "tweet_id": tweet_id });
            let qid = crate::social::x_queryids::lookup("CreateBookmark")
                .ok_or_else(|| ProviderError::Api("Missing CreateBookmark queryId".into()))?;
            let json = self
                .graphql_post(&qid, "CreateBookmark", &vars, access_token)
                .await?;
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/bookmarks");
        let body = serde_json::json!({"tweet_id": tweet_id});
        self.v2_post(&url, access_token, &body).await
    }

    pub async fn unbookmark_tweet(
        &self,
        access_token: &str,
        user_id: &str,
        tweet_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let vars = serde_json::json!({ "tweet_id": tweet_id });
            let qid = crate::social::x_queryids::lookup("DeleteBookmark")
                .ok_or_else(|| ProviderError::Api("Missing DeleteBookmark queryId".into()))?;
            let json = self
                .graphql_post(&qid, "DeleteBookmark", &vars, access_token)
                .await?;
            return Ok(serde_json::json!({ "data": json.get("data") }));
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/bookmarks/{tweet_id}");
        self.v2_delete(&url, access_token).await
    }

    // ── Followers / Following ─────────────────────────────────

    pub async fn followers(
        &self,
        access_token: &str,
        user_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "userId": user_id,
                "count": max_results.clamp(1, 100),
                "includePromotedContent": false,
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("UserByScreenName")
                .ok_or_else(|| ProviderError::Api("Missing UserByScreenName queryId".into()))?;
            // Followers is a POST GraphQL query
            let json = self
                .graphql_post(&qid, "Followers", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let mut result = serde_json::json!({ "data": json });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/users/{user_id}/followers?max_results={}&user.fields=profile_image_url,description,public_metrics",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    pub async fn following(
        &self,
        access_token: &str,
        user_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "userId": user_id,
                "count": max_results.clamp(1, 100),
                "includePromotedContent": false,
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid = crate::social::x_queryids::lookup("UserByScreenName")
                .ok_or_else(|| ProviderError::Api("Missing UserByScreenName queryId".into()))?;
            let json = self
                .graphql_post(&qid, "Following", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let mut result = serde_json::json!({ "data": json });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/users/{user_id}/following?max_results={}&user.fields=profile_image_url,description,public_metrics",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }

    pub async fn follow_user(
        &self,
        access_token: &str,
        user_id: &str,
        target_user_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            // Use REST friendships/create.json (same as twitter-cli)
            let body = format!("user_id={target_user_id}");
            let url = "https://x.com/i/api/1.1/friendships/create.json";
            let (ct0, cs) = self.effective_cookie_str(access_token);
            let resp = self
                .http
                .post(url)
                .header("x-csrf-token", &ct0)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("Cookie", &cs)
                .body(body)
                .send()
                .await
                .map_err(|e| ProviderError::Api(format!("Follow error: {e}")))?;
            let status = resp.status();
            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Api(e.to_string()))?;
            Self::write_delay();
            return if status.is_success() {
                Ok(serde_json::json!({ "data": json }))
            } else {
                Err(ProviderError::Api(format!("Follow failed: HTTP {status}")))
            };
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/following");
        let body = serde_json::json!({"target_user_id": target_user_id});
        self.v2_post(&url, access_token, &body).await
    }

    pub async fn unfollow_user(
        &self,
        access_token: &str,
        user_id: &str,
        target_user_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let body = format!("user_id={target_user_id}");
            let url = "https://x.com/i/api/1.1/friendships/destroy.json";
            let (ct0, cs) = self.effective_cookie_str(access_token);
            let resp = self
                .http
                .post(url)
                .header("x-csrf-token", &ct0)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("Cookie", &cs)
                .body(body)
                .send()
                .await
                .map_err(|e| ProviderError::Api(format!("Unfollow error: {e}")))?;
            let status = resp.status();
            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Api(e.to_string()))?;
            Self::write_delay();
            return if status.is_success() {
                Ok(serde_json::json!({ "data": json }))
            } else {
                Err(ProviderError::Api(format!(
                    "Unfollow failed: HTTP {status}"
                )))
            };
        }
        let url = format!("https://api.twitter.com/2/users/{user_id}/following/{target_user_id}");
        self.v2_delete(&url, access_token).await
    }

    // ── Lists ─────────────────────────────────────────────────

    pub async fn list_timeline(
        &self,
        access_token: &str,
        list_id: &str,
        max_results: u32,
        pagination_token: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        if let Some((_at, _ct0)) = Self::parse_cookie_token(access_token) {
            let mut vars = serde_json::json!({
                "listId": list_id,
                "count": max_results.clamp(1, 100),
            });
            if let Some(cursor) = pagination_token {
                vars["cursor"] = serde_json::json!(cursor);
            }
            let qid =
                crate::social::x_queryids::lookup("ListLatestTweetsTimeline").ok_or_else(|| {
                    ProviderError::Api("Missing ListLatestTweetsTimeline queryId".into())
                })?;
            let json = self
                .graphql_get(&qid, "ListLatestTweetsTimeline", &vars, access_token)
                .await?;
            let cursor = Self::extract_next_cursor(&json);
            let timeline = json
                .pointer("/data/list/tweets_timeline/timeline")
                .cloned()
                .unwrap_or(json);
            let mut result = serde_json::json!({ "data": timeline });
            if let Some(c) = cursor {
                result["meta"] = serde_json::json!({ "next_token": c });
            }
            return Ok(result);
        }
        let mut url = format!(
            "https://api.twitter.com/2/lists/{list_id}/tweets?max_results={}&tweet.fields=created_at,public_metrics&expansions=author_id&user.fields=profile_image_url",
            max_results.min(100)
        );
        if let Some(token) = pagination_token {
            url.push_str(&format!("&pagination_token={token}"));
        }
        self.v2_get(&url, access_token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    fn provider() -> XProvider {
        XProvider::new(&test_config())
    }

    // ── Credential shape decides the auth header ─────────────
    //
    // A cookie-connected row (what `connect x` writes) has NO OAuth
    // bearer. The v2 analytics endpoints were called with the bearer
    // path only, so every analytics refresh for a cookie-connected
    // account returned 401 "Token expired" while the GraphQL surfaces
    // using the SAME cookie worked. The dispatch is by credential
    // shape, so it is testable without a network.
    #[test]
    fn cookie_credentials_are_recognised_as_a_cookie_blob() {
        let cookie_blob = serde_json::json!({
            "auth_token": "abc123",
            "ct0": "csrf-token",
            "cookie_string": "auth_token=abc123; ct0=csrf-token; guest_id=v1%3A1",
        })
        .to_string();
        let (at, ct) = XProvider::parse_cookie_token(&cookie_blob).expect("cookie blob");
        assert_eq!(at, "abc123");
        assert_eq!(ct, "csrf-token");
        assert!(XProvider::extract_cookie_string(&cookie_blob).is_some());
    }

    #[test]
    fn an_oauth_bearer_is_not_mistaken_for_a_cookie_blob() {
        // A bearer row must stay on the bearer path — the cookie path
        // would send an empty Cookie header and get a 401.
        assert!(XProvider::parse_cookie_token("ghp_token_or_bearer_value").is_none());
        assert!(XProvider::extract_cookie_string("ghp_token_or_bearer_value").is_none());
    }

    // ── B2c: account analytics fixtures ──────────────────────

    #[test]
    fn parse_account_metrics_reads_all_four_counts() {
        let fixture = serde_json::json!({
            "data": { "public_metrics": {
                "followers_count": 1200,
                "following_count": 340,
                "tweet_count": 5000,
                "listed_count": 12
            }}
        });
        let rows = XProvider::parse_account_metrics(&fixture);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Followers", "Following", "Tweets", "Listed"]);
        assert_eq!(rows[0].data[0].total, "1200");
        assert!(!rows[0].data[0].date.is_empty(), "rows must carry a date");
    }

    #[test]
    fn parse_account_metrics_skips_metrics_x_omits() {
        // `listed_count` only appears for accounts that are listed.
        let fixture = serde_json::json!({
            "data": { "public_metrics": { "followers_count": 7 } }
        });
        let rows = XProvider::parse_account_metrics(&fixture);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Followers");
    }

    #[test]
    fn parse_account_metrics_empty_for_error_body() {
        let fixture = serde_json::json!({ "title": "Unauthorized", "status": 401 });
        assert!(XProvider::parse_account_metrics(&fixture).is_empty());
    }

    // ── B2c: post analytics fixtures ─────────────────────────

    #[test]
    fn parse_tweet_metrics_reads_all_five_counters() {
        let fixture = serde_json::json!({
            "data": { "public_metrics": {
                "like_count": 42, "retweet_count": 8, "reply_count": 3,
                "quote_count": 1, "impression_count": 1200
            }}
        });
        let rows = XProvider::parse_tweet_metrics(&fixture);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Likes", "Retweets", "Replies", "Quotes", "Impressions"]
        );
        assert_eq!(rows[4].data[0].total, "1200");
    }

    #[test]
    fn parse_tweet_metrics_skips_absent_impressions() {
        // v2 omits impression_count unless the token has the right scope.
        let fixture = serde_json::json!({ "data": { "public_metrics": { "like_count": 2 } } });
        let rows = XProvider::parse_tweet_metrics(&fixture);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Likes");
    }

    // ── B2c: engagement normalisation (both auth paths) ──────

    #[test]
    fn extract_public_metrics_passes_v2_through_unchanged() {
        let fixture = serde_json::json!({
            "data": { "public_metrics": { "like_count": 42, "retweet_count": 8 } }
        });
        let metrics = XProvider::extract_public_metrics(&fixture).unwrap();
        assert_eq!(metrics["like_count"], 42);
    }

    #[test]
    fn extract_public_metrics_maps_graphql_legacy_keys() {
        // The cookie/GraphQL path nests the tweet under `result.legacy` and
        // uses v1 key names — without translation this read as all-zero.
        let fixture = serde_json::json!({
            "data": { "result": { "rest_id": "123", "legacy": {
                "favorite_count": 42, "retweet_count": 8, "reply_count": 3,
                "quote_count": 1, "bookmark_count": 5, "view_count": 1200
            }}}
        });
        let metrics = XProvider::extract_public_metrics(&fixture).unwrap();
        assert_eq!(metrics["like_count"], 42);
        assert_eq!(metrics["impression_count"], 1200);
        assert_eq!(metrics["bookmark_count"], 5);

        let e = crate::social::parse_engagement_data(
            "x",
            serde_json::json!({ "public_metrics": metrics }),
        );
        assert_eq!(
            (e.likes, e.reposts, e.replies, e.quotes, e.views, e.saves),
            (42, 8, 3, 1, 1200, 5)
        );
    }

    #[test]
    fn extract_public_metrics_handles_unwrapped_graphql_tweet() {
        let fixture = serde_json::json!({
            "data": { "rest_id": "123", "legacy": { "favorite_count": 9 } }
        });
        let metrics = XProvider::extract_public_metrics(&fixture).unwrap();
        assert_eq!(metrics["like_count"], 9);
    }

    #[test]
    fn extract_public_metrics_coerces_stringified_graphql_counts() {
        let fixture = serde_json::json!({
            "data": { "result": { "legacy": { "favorite_count": "42" } } }
        });
        let metrics = XProvider::extract_public_metrics(&fixture).unwrap();
        assert_eq!(
            metrics["like_count"], 42,
            "string counts must be coerced to numbers"
        );
    }

    #[test]
    fn extract_public_metrics_returns_none_for_tombstone() {
        let fixture = serde_json::json!({
            "data": { "result": { "__typename": "TweetTombstone", "tombstone": { "text": "gone" } } }
        });
        assert!(XProvider::extract_public_metrics(&fixture).is_none());
    }

    #[test]
    fn extract_public_metrics_returns_none_for_empty_legacy() {
        let fixture = serde_json::json!({ "data": { "result": { "legacy": {} } } });
        assert!(XProvider::extract_public_metrics(&fixture).is_none());
    }

    // ── B2c: map_error fixtures ─────────────────────────────

    #[test]
    fn map_error_explains_rate_limit_from_graphql_code_88() {
        let msg = provider()
            .map_error(r#"{"errors":[{"code":88}]}"#, 400)
            .unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_duplicate_tweet() {
        let msg = provider().map_error("Status is a duplicate.", 403).unwrap();
        assert!(msg.contains("duplicate"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let msg = provider()
            .map_error("MEDIA upload failed to upload the chunk", 400)
            .unwrap();
        assert!(msg.contains("media"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_cookies() {
        let msg = provider()
            .map_error("Could not authenticate you", 401)
            .unwrap();
        assert!(msg.contains("expired"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_forbidden_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("denied"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 500).is_none());
    }

    // ── B2c: response-checker wiring ────────────────────────

    #[test]
    fn check_v2_response_keeps_rate_limited_variant() {
        let err = provider()
            .check_v2_response(wreq::StatusCode::TOO_MANY_REQUESTS, &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ProviderError::RateLimited(_)), "got: {err:?}");
    }

    #[test]
    fn check_v2_response_keeps_token_expired_variant() {
        let err = provider()
            .check_v2_response(wreq::StatusCode::UNAUTHORIZED, &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ProviderError::TokenExpired), "got: {err:?}");
    }

    #[test]
    fn check_v2_response_annotates_api_error_with_friendly_copy() {
        let err = provider()
            .check_v2_response(
                wreq::StatusCode::UNPROCESSABLE_ENTITY,
                &serde_json::json!({ "detail": "duplicate content" }),
            )
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("duplicate"), "got: {msg}");
        assert!(msg.contains("raw:"), "raw detail must be preserved: {msg}");
    }

    #[test]
    fn check_graphql_response_maps_auth_errors_to_cookie_auth() {
        // C2: GraphQL = cookie session. A dead cookie is NOT refreshable —
        // Auth("re-import cookies"), never TokenExpired (the scheduler would
        // try to "refresh" a cookie session, which is impossible).
        let err = provider()
            .check_graphql_response(
                wreq::StatusCode::OK,
                &serde_json::json!({ "errors": [{ "message": "Not authorized" }] }),
            )
            .unwrap_err();
        match err {
            ProviderError::Auth(msg) => {
                assert!(msg.contains("re-import cookies"), "got: {msg}")
            }
            other => panic!("must be Auth, got {other:?}"),
        }
    }

    #[test]
    fn check_graphql_response_maps_401_to_cookie_auth() {
        let err = provider()
            .check_graphql_response(wreq::StatusCode::UNAUTHORIZED, &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ProviderError::Auth(_)), "got: {err:?}");
    }

    // ── B3: comments (cookie GraphQL + OAuth v2) ────────────

    #[test]
    fn parse_should_read_graphql_conversation_entries_as_comments() {
        let fixture = serde_json::json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                { "entries": [
                    { "entryId": "tweet-1", "content": { "itemContent": { "tweet_results": { "result": {
                        "rest_id": "1", "legacy": { "full_text": "root", "created_at": "Wed Oct 07 12:00:00 +0000 2026", "favorite_count": 5 },
                        "core": { "user_results": { "result": { "core": { "name": "Ada", "avatar": { "image_url": "https://img/a.jpg" } } } } }
                    } } } } },
                    { "entryId": "tweet-2", "content": { "itemContent": { "tweet_results": { "result": {
                        "rest_id": "2", "legacy": { "full_text": "a reply", "created_at": "Wed Oct 07 12:05:00 +0000 2026", "favorite_count": 2 },
                        "core": { "user_results": { "result": { "core": { "name": "Bob" } } } }
                    } } } } }
                ]}
            ]}}
        });
        let comments = XProvider::parse_graphql_comments(&fixture, "1");
        assert_eq!(comments.len(), 1, "focal tweet must be skipped");
        assert_eq!(comments[0].id, "2");
        assert_eq!(comments[0].text, "a reply");
        assert_eq!(comments[0].author_name.as_deref(), Some("Bob"));
        assert_eq!(comments[0].like_count, 2);
    }

    #[test]
    fn parse_should_read_replies_inside_conversation_modules() {
        let fixture = serde_json::json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                { "entries": [
                    { "entryId": "tweet-1", "content": { "itemContent": { "tweet_results": { "result": {
                        "rest_id": "1", "legacy": { "full_text": "root" }
                    } } } } },
                    { "entryId": "conversationthread-2", "content": { "entryType": "TimelineTimelineModule", "items": [
                        { "entryId": "conversationthread-2-tweet-2", "item": { "itemContent": { "tweet_results": { "result": {
                            "rest_id": "2", "legacy": { "full_text": "a nested reply", "favorite_count": 1 },
                            "core": { "user_results": { "result": { "core": { "name": "Bob" } } } }
                        } } } } }
                    ] } }
                ]}
            ]}}
        });
        let comments = XProvider::parse_graphql_comments(&fixture, "1");
        assert_eq!(
            comments.len(),
            1,
            "module reply must be read, focal tweet skipped"
        );
        assert_eq!(comments[0].id, "2");
        assert_eq!(comments[0].text, "a nested reply");
    }

    #[test]
    fn parse_should_read_author_name_from_legacy_user() {
        let fixture = serde_json::json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                { "entries": [
                    { "entryId": "tweet-2", "content": { "itemContent": { "tweet_results": { "result": {
                        "rest_id": "2", "legacy": { "full_text": "reply" },
                        "core": { "user_results": { "result": { "legacy": { "name": "Ishan Parihar" } } } }
                    } } } } }
                ]}
            ]}}
        });
        let comments = XProvider::parse_graphql_comments(&fixture, "1");
        assert_eq!(comments[0].author_name.as_deref(), Some("Ishan Parihar"));
    }

    #[test]
    fn parse_should_skip_graphql_tombstone_entries() {
        let fixture = serde_json::json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                { "entries": [
                    { "content": { "itemContent": { "tweet_results": { "result": {
                        "__typename": "TweetTombstone", "tombstone": { "text": "gone" }
                    } } } } }
                ]}
            ]}}
        });
        assert!(XProvider::parse_graphql_comments(&fixture, "1").is_empty());
    }

    #[test]
    fn parse_should_read_v2_conversation_comments_and_skip_root() {
        let fixture = serde_json::json!({
            "data": [
                { "id": "1", "author_id": "u1", "text": "root", "created_at": "2026-10-07T12:00:00.000Z" },
                { "id": "2", "author_id": "u2", "text": "nice", "created_at": "2026-10-07T12:05:00.000Z",
                  "public_metrics": { "like_count": 3 } }
            ],
            "includes": { "users": [
                { "id": "u2", "name": "Bob", "profile_image_url": "https://img/b.jpg" }
            ]}
        });
        let comments = XProvider::parse_v2_conversation(&fixture, "1");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "nice");
        assert_eq!(comments[0].author_name.as_deref(), Some("Bob"));
        assert_eq!(
            comments[0].author_avatar.as_deref(),
            Some("https://img/b.jpg")
        );
        assert_eq!(comments[0].like_count, 3);
    }

    #[test]
    fn parse_should_default_v2_conversation_comment_without_author_expansion() {
        let fixture = serde_json::json!({
            "data": [{ "id": "2", "author_id": "u9", "text": "hi" }]
        });
        let comments = XProvider::parse_v2_conversation(&fixture, "1");
        assert_eq!(comments[0].author_name, None);
        assert_eq!(comments[0].like_count, 0);
    }

    // ── B3: mention search ───────────────────────────────────

    #[test]
    fn parse_should_map_user_search_into_mention_results() {
        let fixture = serde_json::json!({ "data": [
            { "id": "7", "username": "ada", "name": "Ada", "profile_image_url": "https://img/a.jpg" }
        ]});
        let mentions = XProvider::parse_user_search(&fixture);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "7");
        assert_eq!(mentions[0].label, "ada");
        assert_eq!(mentions[0].image.as_deref(), Some("https://img/a.jpg"));
    }

    #[test]
    fn parse_should_read_no_mentions_from_empty_user_search() {
        let fixture = serde_json::json!({ "meta": { "result_count": 0 } });
        assert!(XProvider::parse_user_search(&fixture).is_empty());
    }
}
