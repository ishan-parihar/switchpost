// ─── Social Provider Trait ─────────────────────────────────────
// Abstract interface for all social media platforms.
// Each platform implements this trait for OAuth + publishing.

pub mod archive;
pub mod bluesky;
pub mod browser_cookies;
pub mod calendar;
pub mod common;
pub mod cookie_paste;
pub mod devto;
pub mod discord;
pub mod drive;
pub mod facebook;
pub mod github;
pub mod gmail;
pub mod google_my_business;
pub mod google_oauth;
pub mod hashnode;
pub mod instagram;
pub mod instagram_standalone;
pub mod linkedin;
pub mod linkedin_page;
pub mod mastodon;
pub mod medium;
pub mod pinterest;
pub mod reddit;
pub mod reddit_cookies;
pub mod registry;
pub mod skool;
pub mod slack;
pub mod telegram_bot;
pub mod telegram_user;
pub mod threads;
pub mod threads_app;
pub mod tier;
pub mod tiktok;
pub mod whatsapp;
pub mod whatsapp_cloud;
pub mod wordpress;
pub mod x;
pub mod x_cookies;
pub mod x_queryids;
pub mod youtube;

use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Common Types ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthUrlResponse {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u32>,
    pub provider_user_id: String,
    pub name: String,
    pub username: String,
    pub picture: Option<String>,
}

/// Shared plain HTTP client for providers (F1): ONE connection pool,
/// Arc-cloned per provider instance. A `reqwest::Client::new()` builds a
/// TLS stack + fresh pool every call — providers re-constructed per tool
/// call must not pay that. Providers needing custom default headers
/// (reddit, github) own a per-type `LazyLock` static instead, so their
/// headers never leak into this one.
pub fn shared_http() -> reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);
    CLIENT.clone()
}

/// Shared impersonating HTTP client (F1): ONE Chrome131 pool for code paths
/// that need a real-browser TLS fingerprint. Mirrors `shared_http` so
/// per-call sites (CLI platform runners, media proxy) never rebuild it.
/// For X provider traffic prefer the `x_http()` static in `x.rs`, which
/// additionally pins the bearer + browser default headers.
pub fn shared_wreq() -> wreq::Client {
    static CLIENT: std::sync::LazyLock<wreq::Client> = std::sync::LazyLock::new(|| {
        wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome131)
            .gzip(true)
            .brotli(true)
            .build()
            .unwrap_or_else(|_| wreq::Client::new())
    });
    CLIENT.clone()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PostContent {
    pub content: String,
    pub media: Vec<MediaAttachment>,
    pub settings: serde_json::Value,
    /// Platform-specific post ID that this post is replying to (for threads).
    /// When the scheduler publishes a multi-part thread (posts sharing a
    /// `group_id`), it sets this to the previous part's `platform_post_id`
    /// so providers that support threading (X, Bluesky, Mastodon, Threads)
    /// can link them. Providers that don't support threading ignore this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    /// Phase v22: idempotency key for the publish attempt. Stable across
    /// retries (same post = same key). Providers that support idempotency
    /// (X, LinkedIn, etc.) deduplicate on this key — if the same key is
    /// sent twice, the second request is a no-op returning the original
    /// post's platform_post_id, preventing duplicate posts after
    /// crash-recovery retries.
    ///
    /// Providers that don't support idempotency simply ignore this field.
    /// The key is a UUID v4 string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// v24-5: delay (in minutes) between this row and its thread
    /// predecessor. B2: the delay lives ONLY in each part's
    /// `scheduled_at` (offset by (sequence-1)*delay at creation — see
    /// `create_thread_posts`); the scheduler never sleeps for it.
    /// A value of 0 or None means no delay.
    ///
    /// Postiz uses a similar mechanism (per-row `delay` field in the
    /// composer store, applied to the scheduled time).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_minutes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaAttachment {
    pub url: String,
    pub mime_type: String,
    pub alt: Option<String>,
    /// Optional poster/thumbnail URL for videos
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PublishResult {
    pub platform_post_id: String,
    pub platform_post_url: Option<String>,
    pub status: String,
}

impl PublishResult {
    /// Accepted by the platform but not published yet — the post is in
    /// flight (IG transcoding a reel, an async Reddit submit, X still
    /// processing a video chunk). Must be polled, never recorded as live.
    pub fn is_pending(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "pending"
                | "processing"
                | "in_progress"
                | "accepted"
                | "queued"
                | "uploaded"
                | "submitted"
        )
    }

    /// The platform has published the post. Terminal and successful.
    pub fn is_published(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "published" | "succeeded" | "success" | "complete" | "completed" | "ready" | "live" | "done"
            // `comment` results use "sent" for the same thing.
            | "sent"
        )
    }

    /// No longer in flight. Anything terminal that is not published is a
    /// failure — an unknown future status fails safe rather than claim a post
    /// went live that may not have.
    pub fn is_terminal(&self) -> bool {
        !self.is_pending()
    }
}

/// How many times `finalize_post` polls a pending publish before giving up.
pub const MAX_POLL_ATTEMPTS: u32 = 40;

/// Default gap between pending-publish polls (~3.5 min total budget).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Meta Graph API version (audit 2026-10-14, P1 "mixed versions").
/// ONE bump point for facebook.rs + instagram.rs (graph.facebook.com) and
/// instagram_standalone.rs (graph.instagram.com). Threads runs its own API
/// on graph.threads.net (v1.0 as of 2026-10 — do NOT bump it here).
/// v26.0: released 2026-07-29. Older versions keep answering for ~2 years
/// (v24.0 until 2028-02-18), so a missed bump degrades gracefully.
pub const GRAPH_VERSION: &str = "v26.0";

/// Drive `check` until it reports a terminal state (published or failed).
///
/// The one piece of logic every async platform shares: poll, and stop at the
/// first non-pending answer. `check` is a closure rather than the provider
/// itself so a platform can finalize whatever handle it handed back, and so
/// the loop is testable with no network.
pub async fn poll_until_terminal<F, Fut>(
    interval: Duration,
    mut check: F,
) -> Result<PublishResult, ProviderError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PublishResult, ProviderError>>,
{
    let mut last_seen = "unknown".to_string();

    for attempt in 1..=MAX_POLL_ATTEMPTS {
        match check().await {
            Ok(result) if result.is_terminal() => return Ok(result),
            Ok(result) => {
                last_seen = result.status.clone();
                tracing::debug!(
                    "Pending publish still {last_seen} after poll {attempt}/{MAX_POLL_ATTEMPTS}"
                );
            }
            Err(e) => return Err(e),
        }
        if attempt < MAX_POLL_ATTEMPTS {
            tokio::time::sleep(interval).await;
        }
    }

    Err(ProviderError::Api(format!(
        "Platform never finished processing the post: still {last_seen} after {MAX_POLL_ATTEMPTS} \
         checks. The post may still go live later — check the platform before republishing."
    )))
}

// ── Additional Common Types ─────────────────────────────────

/// Editor type for content creation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum EditorType {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "html")]
    Html,
}

/// Page/channel info for multi-step auth (isBetweenSteps providers)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PageInfo {
    pub id: String,
    pub name: String,
    pub access_token: Option<String>,
    pub picture: Option<String>,
    pub username: Option<String>,
}

/// Discoverable posting target (channel, group, subreddit, peer, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub target_type: String,
    pub picture: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Result of reconnecting after re-authentication
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconnectResult {
    pub id: String,
    pub name: String,
    pub access_token: String,
    pub picture: Option<String>,
    pub username: Option<String>,
}

/// Analytics data for dashboards
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnalyticsData {
    pub label: String,
    pub data: Vec<AnalyticsDataPoint>,
    pub percentage_change: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnalyticsDataPoint {
    pub total: String,
    pub date: String,
}

/// @mention autocomplete result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MentionResult {
    pub id: String,
    pub label: String,
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_cache: Option<bool>,
}

/// Standardized engagement data for any social media post.
/// All platforms normalize their metrics into this unified schema.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EngagementData {
    /// Core metrics (all platforms)
    pub likes: i32,
    pub comments: i32,
    pub shares: i32,
    pub views: i32,
    /// Platform-specific
    pub saves: i32,
    pub quotes: i32,
    pub reposts: i32,
    pub replies: i32,
    /// Reaction breakdown (e.g., Facebook: {"like": 42, "love": 7, "haha": 3})
    pub reactions: Option<serde_json::Value>,
    /// Reddit-specific
    pub upvotes: i32,
    pub downvotes: i32,
    pub upvote_ratio: Option<f32>,
    pub awards: i32,
    /// Raw platform response for extensibility
    pub raw: Option<serde_json::Value>,
}

/// A single comment from a social media post
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommentData {
    pub id: String,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    pub text: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub like_count: i32,
    pub replies: Vec<CommentData>,
}

/// A DM conversation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DmConversation {
    pub id: String,
    pub participant: String,
    pub participant_name: Option<String>,
    pub participant_avatar: Option<String>,
    pub last_message: Option<String>,
    pub last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    pub unread_count: u32,
}

/// A single DM message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DmMessage {
    pub id: String,
    pub conversation_id: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub content: String,
    pub media: Vec<MediaAttachment>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub read: bool,
}

/// External post data for import (CLI command)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExternalPostData {
    pub platform_post_id: String,
    pub text: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub author_avatar: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub url: Option<String>,
    pub media: Vec<MediaAttachment>,
    pub metadata: Option<serde_json::Value>,
}

/// Extra fields for provider OAuth config
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CustomField {
    pub key: String,
    pub label: String,
    pub default_value: Option<String>,
    pub validation: String,
    pub field_type: CustomFieldType,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum CustomFieldType {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "password")]
    Password,
}

// ── Trait ───────────────────────────────────────────────────

/// How a provider measures the length of a post.
///
/// The default is bytes, which is what `String::len()` gives and what
/// a platform's character limit means for ordinary text. X's limit is
/// 280 WEIGHTED characters (twitter-text v3), where an emoji or a CJK
/// glyph costs 2 and everything else costs 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountingRule {
    /// UTF-8 byte length.
    Bytes,
    /// X's weighted length: emoji and CJK presentation ranges cost 2.
    Weighted,
}

impl CountingRule {
    /// The length of `text` under this rule.
    pub fn measure(self, text: &str) -> usize {
        match self {
            CountingRule::Bytes => text.len(),
            CountingRule::Weighted => weighted_length(text),
        }
    }
}

/// X's weighted character count (twitter-text v3 rules, as the v2 API
/// applies them): characters in emoji and CJK presentation ranges cost
/// 2, everything else costs 1.
///
/// The composer's `weightedLength` in `frontend/src/lib/composer/platforms.ts`
/// is the same rule. The two MUST agree: the composer shows the number
/// and the server enforces it.
pub fn weighted_length(text: &str) -> usize {
    let mut count = 0usize;
    for ch in text.chars() {
        let cp = ch as u32;
        let heavy = (0x3040..=0x30ff).contains(&cp)   // Hiragana + Katakana
            || (0x3400..=0x4dbf).contains(&cp)   // CJK Extension A
            || (0x4e00..=0x9fff).contains(&cp)   // CJK Unified Ideographs
            || (0xf900..=0xfaff).contains(&cp)   // CJK Compatibility Ideographs
            || (0xac00..=0xd7af).contains(&cp)   // Hangul Syllables
            || (0x1f300..=0x1f9ff).contains(&cp) // Emoji blocks
            || (0x2600..=0x27bf).contains(&cp); // Misc Symbols + Dingbats
        count += if heavy { 2 } else { 1 };
    }
    count
}

#[async_trait]
pub trait SocialProvider: Send + Sync {
    /// Unique provider identifier (e.g., "x", "linkedin", "bluesky")
    fn identifier(&self) -> &'static str;

    /// Human-readable provider name
    fn name(&self) -> &'static str;

    /// OAuth2 scopes required for this provider
    fn scopes(&self) -> Vec<String>;

    /// Max content length (characters)
    fn max_content_length(&self) -> usize;

    /// How this provider measures a post's length.
    ///
    /// The default is BYTES, which is what `String::len()` gives and
    /// what every platform's own limit is expressed in for a plain
    /// character. X is the exception: its limit is 280 WEIGHTED
    /// characters (twitter-text v3), where an emoji or a CJK glyph
    /// costs 2.
    ///
    /// The composer already implements the weighted rule (it has to -
    /// that is the number X counts), so the server measuring bytes
    /// meant the two disagreed: a 200-emoji Bluesky post read "200/300"
    /// in the composer and was rejected by the server as "800 chars,
    /// max 300".
    fn counting_rule(&self) -> CountingRule {
        CountingRule::Bytes
    }

    /// Tooltip shown in UI
    fn tooltip(&self) -> Option<&'static str> {
        None
    }

    /// Editor type for content creation
    fn editor_type(&self) -> EditorType {
        EditorType::Normal
    }

    /// Does this provider have multi-step auth (page selection after OAuth)?
    fn is_between_steps(&self) -> bool {
        false
    }

    /// Does this provider use a Chrome extension for auth?
    fn is_chrome_extension(&self) -> bool {
        false
    }

    /// Does this provider need proactive cron-based token refresh?
    fn needs_cron_refresh(&self) -> bool {
        false
    }

    /// Should we wait for refresh to complete before publishing?
    fn refresh_wait(&self) -> bool {
        false
    }

    /// Is this a one-time token provider?
    fn one_time_token(&self) -> bool {
        false
    }

    /// Custom fields for OAuth config
    async fn custom_fields(&self) -> Vec<CustomField> {
        vec![]
    }

    /// Does this provider REFUSE an `http://` OAuth redirect_uri?
    ///
    /// ─── THE DEFECT THIS CLOSES ────────────────────────────────
    /// Meta's Threads (and Instagram) reject an `http://` redirect URI
    /// outright — Google's docs and ours both say so, and
    /// `.env.example:21-24` documents it — but nothing ENFORCED it. The
    /// server happily generated
    ///
    /// ```text
    /// https://www.threads.net/oauth/authorize?...
    ///     &redirect_uri=http%3A%2F%2F127.0.0.1%3A6543%2Fapi%2Fauth%2Fcallback
    /// ```
    /// when `APP_URL` was `http://…`, the founder completed the consent
    /// screen, and Meta answered the callback with
    /// `?error=invalid_request&error_description=…redirect_uri…`. That came
    /// back to the OAuth callback page as the single word the founder
    /// reported: a "login-error" with no indication the fix was to serve the
    /// app over HTTPS.
    ///
    /// The default is `false` because most providers accept http on a
    /// loopback dev instance. Only the providers that demonstrably refuse it
    /// override this to `true`.
    fn requires_https_redirect(&self) -> bool {
        false
    }

    /// A warning to show the founder BEFORE they are sent to the
    /// provider's consent screen.
    ///
    /// ─── WHY THIS EXISTS ───────────────────────────────────────
    /// Some providers can refuse a sign-in at a step our server never
    /// sees. Meta's Threads is the one we have measured: when its own
    /// privacy/consent gate refuses the login, the BROWSER is redirected
    /// to `www.threads.com/privacy/consent/?flow=gdp` with
    /// `error_code=1349003&source=gdp_error` in the tail, and it never
    /// comes back to our callback. So no handler of ours runs, no error
    /// is recorded, and the founder is left staring at a Meta page with
    /// nothing actionable — the exact dead end in the login-error report.
    ///
    /// A provider that can answer "will this sign-in work?" BEFORE the
    /// founder is redirected answers it here, and the caller shows the
    /// answer next to the sign-in button.
    ///
    /// The default is `None`: most providers have no such gate, and a
    /// warning that fires for every provider is a warning nobody reads.
    async fn connect_preflight(&self) -> Option<String> {
        None
    }

    /// Extension cookies needed (for Chrome extension providers like Skool)
    fn extension_cookies(&self) -> Vec<(&'static str, &'static str)> {
        vec![]
    }

    /// Generate the OAuth authorization URL
    /// `code_verifier` is the PKCE code verifier (S256 challenge will be derived from it).
    async fn generate_auth_url(
        &self,
        state: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError>;

    /// Check if this provider uses OAuth (vs direct API key / app password).
    fn uses_oauth(&self) -> bool {
        true
    }

    /// Exchange authorization code for access token
    async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError>;

    /// Refresh an expired access token
    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError>;

    /// Publish content to the social platform
    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError>;

    /// Publish, reporting a pending result when the platform only *accepted*
    /// the post (IG container publish, async Reddit submit, X still
    /// transcoding a video).
    ///
    /// Default: plain `publish`, so the 23 synchronous providers are
    /// untouched. Async providers override this and keep `publish` for the
    /// CLI/REST/MCP paths that want the immediate, unpolled answer.
    ///
    /// `internal_id` is the CHANNEL the post belongs to — the connected
    /// page, the connected organization. A token that can administer SEVERAL
    /// sub-accounts resolves one from the API, and the natural answer (the
    /// first in the list) is often not the one the founder picked. A
    /// provider with that shape (Facebook Page, Instagram, LinkedIn Page)
    /// overrides this so it publishes to the channel the post is for.
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let _ = internal_id;
        self.publish(access_token, post).await
    }

    /// Check a post that `post_pending` reported as pending.
    ///
    /// Default: assume it went live — exactly what the scheduler did before
    /// this lifecycle existed, so a provider that never returns pending is
    /// never polled.
    async fn check_post_status(
        &self,
        _access_token: &str,
        platform_post_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        Ok(PublishResult {
            platform_post_id: platform_post_id.to_string(),
            platform_post_url: None,
            status: "published".into(),
        })
    }

    /// Poll a pending post to a terminal state (published or failed).
    ///
    /// Default: one `check_post_status` call. A platform only overrides this
    /// if it needs a different cadence.
    async fn finalize_post(
        &self,
        access_token: &str,
        platform_post_id: &str,
        poll_interval: Duration,
    ) -> Result<PublishResult, ProviderError> {
        poll_until_terminal(poll_interval, || {
            self.check_post_status(access_token, platform_post_id)
        })
        .await
    }

    /// Post a comment/reply to an existing post
    ///
    /// B7: the default delegates to [`Self::reply_to_comment`] with the
    /// POST id — providers that implement comment-threading replies but
    /// never implemented `comment()` (IG/FB/LinkedIn/X/Threads/YouTube/
    /// Bluesky/Mastodon/TikTok) used to hit a hard "not supported" stub,
    /// so `--first-comment` silently did nothing. Providers with a
    /// native first-comment API still override this.
    async fn comment(
        &self,
        access_token: &str,
        post_id: &str,
        _last_comment_id: Option<&str>,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        self.reply_to_comment(access_token, post_id, post).await
    }

    /// §8.2: delete a PUBLISHED post on the platform. Default: unsupported
    /// (providers that have a delete API override — X v2, Reddit, LinkedIn,
    /// Facebook, WordPress…). Callers surface the failure rather than
    /// pretending the local row's deletion removed the platform post.
    async fn delete_published(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::Api(
            "This provider does not support deleting a published post".into(),
        ))
    }

    /// §8.2: edit a PUBLISHED post's content in place. Optional capability —
    /// most platforms either forbid edits (X, IG) or replace-by-reference
    /// (LinkedIn, FB). Unsupported by default.
    async fn edit_published(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
        _new_content: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "This provider does not support editing a published post".into(),
        ))
    }

    /// Get analytics for a time range (dashboard)
    async fn analytics(
        &self,
        _access_token: &str,
        _internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        Ok(vec![])
    }

    /// Get per-post analytics
    async fn post_analytics(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        Ok(vec![])
    }

    /// List manageable pages/channels (for isBetweenSteps providers)
    async fn pages(&self, _access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        Ok(vec![])
    }

    /// Discover available posting targets (channels, groups, subreddits, peers, etc.)
    /// Returns empty vec for providers where target = integration (e.g., X, LinkedIn personal)
    async fn targets(&self, _access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        Ok(vec![])
    }

    /// Fetch page information by ID (for reConnect)
    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError>;

    /// Reconnect/re-bind after re-authentication.
    ///
    /// The default routes through `fetch_page_info`, which is the
    /// PAGE-SELECTION function - and which is SUPPOSED to refuse for a
    /// provider with no page concept
    /// (`Err(ProviderError::NoPageManagement)`). Left alone, that
    /// refusal becomes the answer to the refresh endpoint's "Refresh",
    /// because a `?` forwards it: "Failed to refresh profile: This
    /// provider has no page or sub-account concept. The channel is the
    /// account.", from a button on a Reddit channel.
    ///
    /// So the default DEGRADES on a page-management refusal rather than
    /// forwarding it: it returns the channel's own identity with the
    /// existing token, which lets the refresh endpoint keep the stored
    /// `profile_name`/`profile_picture` and only clear
    /// `refresh_needed`. The caller writes those with `COALESCE`, so a
    /// provider that supplies nothing does not blank a channel — it
    /// just cannot improve it.
    ///
    /// A provider that CAN re-read its own profile (LinkedIn, X, the
    /// Google family) overrides this with a direct userinfo call.
    ///
    /// A refusal for any OTHER reason is still an error: it is a
    /// genuine failure, not a "no pages here".
    async fn reconnect(
        &self,
        access_token: &str,
        _internal_id: &str,
        page_id: &str,
    ) -> Result<ReconnectResult, ProviderError> {
        match self.fetch_page_info(access_token, page_id).await {
            Ok(info) => Ok(ReconnectResult {
                id: info.id,
                name: info.name,
                access_token: info.access_token.unwrap_or_default(),
                picture: info.picture,
                username: info.username,
            }),
            Err(ProviderError::NoPageManagement) => Ok(ReconnectResult {
                id: page_id.to_string(),
                name: String::new(),
                access_token: access_token.to_string(),
                picture: None,
                username: None,
            }),
            Err(e) => Err(e),
        }
    }

    /// Search for @mentions
    async fn search_mention(
        &self,
        _access_token: &str,
        _query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        Ok(vec![])
    }

    /// Format an @mention string for this provider
    fn format_mention(&self, id_or_handle: &str, _name: &str) -> String {
        format!("@{}", id_or_handle)
    }

    /// Map provider API error body/status to user-friendly message
    fn map_error(&self, _body: &str, _status: u16) -> Option<String> {
        None
    }

    /// The ONE translation from an HTTP failure to the error the app acts on.
    ///
    /// ## Why this exists
    ///
    /// Every provider decided the error CATEGORY in its own error branch:
    /// a 429 was `RateLimited` on some platforms (the scheduler backs off
    /// and retries) and a plain `Api` error on others (the scheduler gives
    /// up on the first try). The same failure therefore behaved
    /// differently depending on which platform the founder posted to, and
    /// the RETRY POLICY keys on the category.
    ///
    /// The category now comes from `classify_error` — the one classifier —
    /// and this method is its only translation into the error type the app
    /// reacts to. The provider's own `map_error` still supplies the
    /// WORDING ("Invalid Dev.to API key. Check DEVTO_API_KEY in .env."),
    /// so nothing about the message the founder reads changes.
    fn provider_error(&self, body: &str, status: u16) -> ProviderError {
        match classify_error(body, status) {
            // A rate limit is retryable: the scheduler backs off and tries
            // again. Reporting it as a plain API error meant the post
            // failed on the first try and the founder had to re-queue it.
            Some(ErrorKind::RateLimited) => {
                ProviderError::RateLimited(friendly_error(self.map_error(body, status), body))
            }
            // An expired or revoked token is the one error the scheduler
            // refreshes and retries — once.
            Some(ErrorKind::AuthExpired) => ProviderError::TokenExpired,
            // A forbidden request is a grant problem, not a transient
            // fault: retrying it changes nothing.
            Some(ErrorKind::Forbidden) => {
                ProviderError::Auth(friendly_error(self.map_error(body, status), body))
            }
            _ => ProviderError::Api(friendly_error(self.map_error(body, status), body)),
        }
    }

    /// Import recent posts from this platform (for the External Post Import CLI).
    async fn get_recent_posts(
        &self,
        _access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        Ok(vec![])
    }

    /// Fetch engagement data for a post (likes, comments, shares, etc.)
    /// Returns raw JSON to be parsed into EngagementData by the caller.
    async fn get_post_engagement(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        Ok(None)
    }

    /// Fetch comments for a post.
    /// Returns a flat or threaded list of comments from the platform.
    async fn get_post_comments(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        Ok(vec![])
    }

    /// Fetch normalized engagement data for a post.
    /// This is the canonical method used by the engagement sync engine.
    /// Returns None if the post has no engagement data or the API doesn't support it.
    /// Default implementation calls get_post_engagement() and parses it.
    async fn fetch_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<EngagementData>, ProviderError> {
        let raw = self
            .get_post_engagement(access_token, platform_post_id)
            .await?;
        match raw {
            Some(value) => Ok(Some(parse_engagement_data(self.identifier(), value))),
            None => Ok(None),
        }
    }

    /// Can this provider ever put a post on a feed?
    ///
    /// Derived from `max_content_length() == 0`, which is how a provider
    /// declares "I have no post surface" (GitHub: issues/PRs/repos only —
    /// its `publish()` is a documented error stub). Storing a draft against
    /// such a provider looks identical to a normal draft until the scheduler
    /// fails it, so callers that report readiness ask this first.
    fn publish_capable(&self) -> bool {
        self.max_content_length() > 0
    }

    /// The `settings` keys this provider REQUIRES to publish.
    ///
    /// A provider that needs a target (Discord's channel, Reddit's
    /// subreddit, Pinterest's board, Hashnode's publication) reads it out
    /// of the post's settings and errors when it is missing. Without
    /// this list the error only appeared at PUBLISH time, in the
    /// scheduler, as a post that silently went to `state='error'` - the
    /// founder saw a failed post and no reason.
    ///
    /// The create path checks the list and refuses the post with a
    /// message naming the missing field.
    ///
    /// The DEFAULT is an empty list: a provider that publishes from the
    /// content alone needs nothing. Only a provider that reads a key
    /// out of `post.settings` overrides it - and the key it lists must
    /// be the key it actually reads.
    fn required_settings(&self) -> &'static [&'static str] {
        &[]
    }

    /// Validate content against platform-specific limits before publishing.
    ///
    /// A zero content limit is reported as a capability gap rather than as
    /// "content too long": the length is irrelevant when the provider cannot
    /// publish at all, and "Maximum is 0 chars" reads like a bug in the
    /// staged draft rather than a property of the provider.
    fn validate_post(&self, post: &PostContent) -> Result<(), String> {
        validate_post_rules(self.name(), self.max_content_length(), &post.content)
    }

    /// Validate media attachments against platform-specific limits.
    /// Checks image/video counts, file sizes, and dimensions where applicable.
    /// Default implementation accepts all media (no restrictions).
    fn validate_media(&self, _post: &PostContent) -> Result<(), String> {
        Ok(())
    }

    /// Resolve a media attachment URL to a publicly accessible URL.
    /// Platforms like Instagram, Facebook, and Threads require publicly accessible URLs
    /// because they fetch media server-side. By default, returns the URL unchanged.
    /// Override in providers that need URL resolution (e.g., Meta Graph API providers).
    /// Turn a stored media URL into one the PLATFORM can fetch.
    ///
    /// The app stores media as a relative URL - `/api/media/<uuid>` -
    /// because that is what the dashboard renders. A platform cannot
    /// fetch a relative URL: it would try to read the literal path
    /// `/api/media/<uuid>` from its own filesystem, or fail to parse it
    /// as a URL at all.
    ///
    /// So the URL is made ABSOLUTE here, against the app's public base
    /// URL. This is the DEFAULT because it is correct for every
    /// provider: the alternative - a default that returns the
    /// attachment unchanged, with each provider opting in - meant that
    /// X, Bluesky, Mastodon, Pinterest, TikTok, YouTube and LinkedIn
    /// published with a relative URL and failed, with the error
    /// surfacing as an opaque network failure rather than a media
    /// error. Four providers had their own copy of this override and
    /// eight did not have it at all.
    fn resolve_media_url(&self, attachment: &MediaAttachment, app_url: &str) -> MediaAttachment {
        let url = attachment.url.trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            // Already absolute - leave it alone.
            return attachment.clone();
        }
        if !url.starts_with('/') {
            // Not a path we recognise (a data: URL, a blob: URL, a
            // provider CDN URL the user pasted). Leave it alone.
            return attachment.clone();
        }
        MediaAttachment {
            url: format!("{}{}", app_url.trim_end_matches('/'), url),
            ..attachment.clone()
        }
    }

    // ── DM Methods ───────────────────────────────────────────

    /// Whether this provider has a DIRECT-MESSAGE surface the app can drive.
    ///
    /// The default is the honest answer for the trait's other implementations:
    /// this method is OVERRIDDEN only by a provider that has a DM surface
    /// (`x`, `instagram`, `linkedin`), so "did they override it" is a code
    /// fact rather than a list someone has to keep in step.
    ///
    /// It exists so the API can PUBLISH the capability instead of leaving
    /// every client to guess. The `/dms` page carried its own hardcoded
    /// `['x','instagram','linkedin']` and picked the first match as the
    /// default channel; on an instance whose only X channel is COOKIE-auth
    /// that shipped a 400 on a page load, because X's DM API is bearer-only.
    /// Then the client-side "fix" guessed again (`auth_method === 'oauth'`)
    /// and the same page offered X in the same selector. Same failure shape
    /// as the duplicated `char_limit` table (9.20): a rule with more than
    /// one implementation is a rule that will drift.
    ///
    /// NOTE this says the PROVIDER has DMs, not that a given CHANNEL can
    /// reach them - an X channel authenticated with cookies cannot, however
    /// capable X is. That second distinction is `auth_method`, and the
    /// channel-level check belongs where the channel is known. The API's
    /// guard (`x_cookie_auth_dm_refusal`) remains the authority there; this
    /// is what lets a UI avoid offering the login in the first place.
    fn dm_capable(&self) -> bool {
        false
    }

    /// Send a direct message to a user
    async fn send_dm(
        &self,
        _access_token: &str,
        _recipient: &str,
        _content: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "DMs not supported by this provider".into(),
        ))
    }

    /// Get list of DM conversations
    async fn get_dm_conversations(
        &self,
        _access_token: &str,
        _limit: u32,
    ) -> Result<Vec<DmConversation>, ProviderError> {
        Ok(vec![])
    }

    /// Get messages in a DM conversation
    async fn get_dm_messages(
        &self,
        _access_token: &str,
        _conversation_id: &str,
        _limit: u32,
    ) -> Result<Vec<DmMessage>, ProviderError> {
        Ok(vec![])
    }

    // ── Comment Enhancement Methods ──────────────────────────

    /// Reply to a specific comment
    async fn reply_to_comment(
        &self,
        _access_token: &str,
        _comment_id: &str,
        _content: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Comment replies not supported by this provider".into(),
        ))
    }

    /// Get a specific comment by ID
    async fn get_comment(
        &self,
        _access_token: &str,
        _comment_id: &str,
    ) -> Result<Option<CommentData>, ProviderError> {
        Ok(None)
    }

    /// Delete a comment
    async fn delete_comment(
        &self,
        _access_token: &str,
        _comment_id: &str,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::Api(
            "Comment deletion not supported by this provider".into(),
        ))
    }
}

/// Platform-specific media validation limits.
pub fn validate_media_limits(identifier: &str, post: &PostContent) -> Result<(), String> {
    let images: Vec<_> = post
        .media
        .iter()
        .filter(|m| m.mime_type.starts_with("image/"))
        .collect();
    let videos: Vec<_> = post
        .media
        .iter()
        .filter(|m| m.mime_type.starts_with("video/"))
        .collect();

    match identifier {
        "instagram" | "instagram_standalone" | "instagram-standalone" => {
            if images.len() > 10 {
                return Err(format!(
                    "Instagram allows max 10 images, got {}",
                    images.len()
                ));
            }
            if post.media.len() > 10 {
                return Err(format!(
                    "Instagram allows max 10 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "x" => {
            if images.len() > 4 {
                return Err(format!(
                    "X/Twitter allows max 4 images, got {}",
                    images.len()
                ));
            }
            if post.media.len() > 4 {
                return Err(format!(
                    "X/Twitter allows max 4 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "linkedin" | "linkedin_page" | "linkedin-page" => {
            if images.len() > 20 {
                return Err(format!(
                    "LinkedIn allows max 20 images, got {}",
                    images.len()
                ));
            }
            if post.media.len() > 20 {
                return Err(format!(
                    "LinkedIn allows max 20 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "reddit" => {
            if images.len() > 20 {
                return Err(format!("Reddit allows max 20 images, got {}", images.len()));
            }
            if !videos.is_empty() {
                return Err("Reddit posts do not support video. Use a video link instead.".into());
            }
        }
        "facebook" => {
            if images.len() > 10 {
                return Err(format!(
                    "Facebook allows max 10 images, got {}",
                    images.len()
                ));
            }
            if post.media.len() > 10 {
                return Err(format!(
                    "Facebook allows max 10 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "youtube" => {
            if !images.is_empty() && !videos.is_empty() {
                return Err("YouTube posts require video only, not images.".into());
            }
            if videos.is_empty() && images.is_empty() {
                return Err("YouTube posts require a video attachment.".into());
            }
        }
        "threads" => {
            // Threads accepts up to 20 images or one video - the
            // provider's own rejection message says so (threads.rs:
            // "Threads posts accept up to 20 images or one video"), and
            // the composer's table said 20. The limit here said 10, so
            // the composer let the founder attach 15 images with no
            // banner and the server rejected the post on save.
            if images.len() > 20 {
                return Err(format!(
                    "Threads allows max 20 images, got {}",
                    images.len()
                ));
            }
            if post.media.len() > 20 {
                return Err(format!(
                    "Threads allows max 20 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "bluesky" => {
            if images.len() > 4 {
                return Err(format!("Bluesky allows max 4 images, got {}", images.len()));
            }
            if post.media.len() > 4 {
                return Err(format!(
                    "Bluesky allows max 4 media items, got {}",
                    post.media.len()
                ));
            }
        }
        "mastodon" if post.media.len() > 4 => {
            return Err(format!(
                "Mastodon allows max 4 media items, got {}",
                post.media.len()
            ));
        }
        _ => {}
    }
    Ok(())
}

/// Providers that fetch every attachment from its URL themselves.
///
/// They are handed a URL, not bytes, so the URL must resolve from the
/// public internet — a filesystem path or a loopback address fails at
/// publish time no matter how the post itself is delivered. See
/// `SocialProvider::resolve_media_url`, which exists for exactly this.
pub fn fetches_media_server_side(identifier: &str) -> bool {
    matches!(
        identifier,
        "instagram" | "instagram-standalone" | "instagram_standalone" | "facebook" | "threads"
    )
}

/// True when `host` only resolves from this machine or a private LAN
/// (`localhost`, loopback, RFC1918, `.local`), so a remote platform server
/// cannot fetch a URL built on it.
pub fn is_host_local(host: &str) -> bool {
    let h = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if h == "localhost"
        || h == "::1"
        || h == "0.0.0.0"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.starts_with("127.")
        || h.starts_with("10.")
        || h.starts_with("192.168.")
    {
        return true;
    }
    let mut octets = h.split('.');
    match (octets.next(), octets.next()) {
        (Some(a), Some(b)) => {
            a.parse::<u8>().is_ok_and(|o| o == 172)
                && b.parse::<u8>().is_ok_and(|o| (16..=31).contains(&o))
        }
        _ => false,
    }
}

/// Why `identifier` cannot fetch the media at `url`, or `None` when it can.
///
/// Three ways a provider obtains the bytes, three rules:
///   - server-side fetchers need a publicly resolvable http(s) URL;
///   - X downloads the URL or reads the file itself, so a local path is
///     fine but a path that does not exist is not;
///   - everything else passes the URL through, which is the caller's call.
///
/// Pure so the pre-check needs no network and no server — the point is to
/// catch an unreachable URL *before* a publish burns a retry budget.
pub fn media_url_problem(identifier: &str, url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Some("media attachment has an empty URL".into());
    }
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");
    if is_http {
        let Ok(parsed) = url::Url::parse(trimmed) else {
            return Some(format!("`{trimmed}` is not a parseable URL"));
        };
        if fetches_media_server_side(identifier) {
            let host = parsed.host_str().unwrap_or_default();
            if is_host_local(host) {
                return Some(format!(
                    "{identifier} fetches media server-side, so it cannot reach host `{host}`"
                ));
            }
        }
        return None;
    }
    if fetches_media_server_side(identifier) {
        return Some(format!(
            "{identifier} fetches media server-side and cannot read the local path `{trimmed}`"
        ));
    }
    if identifier == "x" && !std::path::Path::new(trimmed).exists() {
        return Some(format!(
            "X accepts an http(s) URL or an existing local file; `{trimmed}` is neither"
        ));
    }
    None
}

// ── Error Types ─────────────────────────────────────────────

/// The length + capability half of [`SocialProvider::validate_post`], as a
/// free function so an override can run the same rules: calling
/// `SocialProvider::validate_post(self, …)` from inside an impl resolves to
/// the override again and recurses forever.
pub fn validate_post_rules(
    provider_name: &str,
    max_len: usize,
    content: &str,
) -> Result<(), String> {
    if max_len == 0 {
        return Err(format!(
            "{provider_name} has no post-publish surface (content limit is 0) — a staged draft \
             for it is stored but can never go live. Use the provider's own API (issues, PRs, \
             repository files) instead."
        ));
    }
    if content.len() > max_len {
        return Err(format!(
            "Content too long ({} chars). Maximum is {} chars for {provider_name}.",
            content.len(),
            max_len
        ));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("API error: {0}")]
    Api(String),
    /// This provider has no "page" / sub-account concept. NOT a
    /// failure: it is the correct answer to `pages()` /
    /// `fetch_page_info()` for a channel that is itself the unit.
    #[error("Provider does not support page management")]
    NoPageManagement,
    #[error("Token expired")]
    TokenExpired,
    #[error("Rate limited: {0}")]
    RateLimited(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Auth error: {0}")]
    Auth(String),
}

// ── Error classification (v25 §2 row 9) ────────────────────────

/// Coarse bucket a provider error response falls into.
///
/// Per-provider `map_error` impls turn this into user-facing copy, so the
/// body-sniffing heuristics live in one place instead of being
/// re-implemented (and drifting) across every provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    RateLimited,
    Duplicate,
    MediaRejected,
    AuthExpired,
    Forbidden,
    NotFound,
    InvalidRequest,
}

/// Classify an error response body + HTTP status into an [`ErrorKind`].
///
/// `status` is authoritative when it says something specific (401/403/404/429/
/// 400/422); the body is only sniffed for the cases a status cannot express
/// (duplicate submissions, rejected media, expired grants). Returns `None`
/// when nothing useful can be said — callers then fall back to the raw body.
pub fn classify_error(body: &str, status: u16) -> Option<ErrorKind> {
    let lower = body.to_ascii_lowercase();

    // Body signals are checked before the status code because they are more
    // specific: a 403 carrying "rate limit exceeded" is a rate limit, not a
    // permission problem.
    if lower.contains("\"code\":88")
        || lower.contains("rate limit exceeded")
        || lower.contains("too many requests")
        || lower.contains("ratelimit")
    {
        return Some(ErrorKind::RateLimited);
    }
    if lower.contains("duplicate") || lower.contains("already exists") {
        return Some(ErrorKind::Duplicate);
    }
    if lower.contains("expired")
        || lower.contains("invalid_grant")
        || lower.contains("unauthorized")
        || lower.contains("not authorized")
        || lower.contains("invalid token")
        || lower.contains("bad authentication")
    {
        return Some(ErrorKind::AuthExpired);
    }
    let is_media = ["media", "image", "video", "blob", "upload", "thumbnail"]
        .iter()
        .any(|t| lower.contains(t));
    let is_media_failure = [
        "rejected",
        "invalid",
        "unsupported",
        "not allowed",
        "could not be uploaded",
        "failed to upload",
        "too large",
    ]
    .iter()
    .any(|t| lower.contains(t));
    if is_media && is_media_failure {
        return Some(ErrorKind::MediaRejected);
    }

    match status {
        401 => Some(ErrorKind::AuthExpired),
        403 => Some(ErrorKind::Forbidden),
        404 => Some(ErrorKind::NotFound),
        429 => Some(ErrorKind::RateLimited),
        400 | 422 => Some(ErrorKind::InvalidRequest),
        _ => None,
    }
}

/// Prefix a raw provider error with the user-friendly copy returned by
/// `map_error`, keeping the raw text so debugging stays possible. Falls back
/// to the raw text alone when the provider has no mapping for this error.
pub fn friendly_error(friendly: Option<String>, raw: &str) -> String {
    match friendly {
        Some(msg) => format!("{msg} (raw: {raw})"),
        None => raw.to_string(),
    }
}

// ── Engagement Data Parser ────────────────────────────────────

// ── Meta Graph insights helpers ───────────────────────────────
// Facebook, Instagram, Instagram-standalone and Threads all serve metrics
// through the same Graph insights envelope, so one parser serves all four.

/// Read a numeric JSON field, accepting both integer and float encodings.
fn json_i64(v: Option<&serde_json::Value>) -> Option<i64> {
    match v? {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Look up one metric in a Graph insights envelope or a flat media object.
///
/// Accepts both shapes the Graph API returns:
/// - multi-point: `{"data":[{"name":"views","values":[{"value":12,"end_time":"…"}]}]}`
/// - single-value: `{"data":[{"name":"reach","value":30}]}`
/// - flat: `{"like_count":42}`
///
/// Returns `None` when the metric is absent, so callers can fall back to the
/// next candidate metric without mistaking a real zero for a missing field.
pub fn insight_value(raw: &serde_json::Value, metric: &str) -> Option<i64> {
    if let Some(v) = json_i64(raw.get(metric)) {
        return Some(v);
    }
    let entry = raw
        .get("data")
        .and_then(|d| d.as_array())?
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some(metric))?;

    match entry.get("values").and_then(|v| v.as_array()) {
        // Multi-point envelope: the first bucket is the closest-to-now total
        // for lifetime metrics, and the only one for period-scoped metrics.
        Some(values) => json_i64(values.first().and_then(|v| v.get("value"))),
        None => json_i64(entry.get("value")),
    }
}

/// Parse a Graph insights response into dashboard-shaped `AnalyticsData`.
///
/// One `AnalyticsData` per metric, labelled with the metric name, one data point
/// per bucket. Metrics without a `values` array become a single data point.
/// `percentage_change` is left at 0.0 — no consumer reads it today.
pub fn parse_insights_data(raw: &serde_json::Value) -> Vec<AnalyticsData> {
    raw.get("data")
        .and_then(|d| d.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let label = entry.get("name").and_then(|n| n.as_str())?.to_string();
                    let data = match entry.get("values").and_then(|v| v.as_array()) {
                        Some(values) => values
                            .iter()
                            .map(|v| AnalyticsDataPoint {
                                total: json_i64(v.get("value")).unwrap_or(0).to_string(),
                                date: v
                                    .get("end_time")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                            })
                            .collect(),
                        // Single-value metric (e.g. IG `reach`, `follower_count`)
                        None => vec![AnalyticsDataPoint {
                            total: json_i64(entry.get("value")).unwrap_or(0).to_string(),
                            date: entry
                                .get("end_time")
                                .and_then(|d| d.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        }],
                    };
                    Some(AnalyticsData {
                        label,
                        data,
                        percentage_change: 0.0,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Provider metric readers (v25 §2 rows 3–4) ─────────────────

/// Read one count out of a JSON object, tolerating the shapes the analytics
/// endpoints use: YouTube Data API sends string-encoded numbers
/// (`"viewCount": "1200"`), Pinterest uses SCREAMING_SNAKE keys
/// (`"IMPRESSION"`), TikTok uses snake_case integers. Key lookup is
/// case-insensitive so both key styles match.
pub(crate) fn count(source: &serde_json::Value, key: &str) -> Option<i32> {
    let obj = source.as_object()?;
    let v = obj.get(key).or_else(|| {
        obj.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    })?;
    // Saturate instead of `as i32`: an out-of-range count is a bad reading,
    // not a negative one, and must never be written to the DB as such.
    let n = v
        .as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str()?.trim().parse().ok())?;
    Some(i32::try_from(n).unwrap_or(i32::MAX))
}

/// Build single-date `AnalyticsData` series from a flat metrics object.
/// `keys` are `(label, json_key)` pairs; absent or non-numeric keys are skipped
/// so a provider that omits a metric reports the rest rather than a bogus zero.
pub(crate) fn metrics_series(
    source: &serde_json::Value,
    date: &str,
    keys: &[(&str, &str)],
) -> Vec<AnalyticsData> {
    keys.iter()
        .filter_map(|(label, key)| {
            count(source, key).map(|v| AnalyticsData {
                label: (*label).to_string(),
                data: vec![AnalyticsDataPoint {
                    total: v.to_string(),
                    date: date.to_string(),
                }],
                percentage_change: 0.0,
            })
        })
        .collect()
}

/// Reshape a provider's `pages()` listing into TargetPicker targets (v25 §2
/// row 7).
///
/// Facebook Pages, LinkedIn organizations, YouTube channels and Pinterest
/// boards are all discovered by the same `pages()` call, so each provider only
/// supplies its own `target_type` instead of re-writing the field mapping.
/// `PageInfo::username` is the platform's own handle for the target (a FB
/// username, a LinkedIn vanity name, a YouTube custom URL), published under
/// the neutral `handle` key so consumers do not branch per platform.
pub(crate) fn pages_to_targets(pages: &[PageInfo], target_type: &str) -> Vec<TargetInfo> {
    pages
        .iter()
        .map(|p| TargetInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            target_type: target_type.to_string(),
            picture: p.picture.clone(),
            metadata: p
                .username
                .as_ref()
                .map(|u| serde_json::json!({ "handle": u })),
        })
        .collect()
}

/// Parse a Meta Graph API comments payload into `CommentData` (v25 §2 row 5).
///
/// Facebook, Instagram, Instagram-standalone and Threads all expose the same
/// envelope with different field names: Facebook uses `message` /
/// `created_time` / `from.name` and nests replies under `comments.data`,
/// Instagram and Threads use `text` / `timestamp` / `username` and nest them
/// under `replies.data`. One reader serves all four, so the providers stay
/// thin and the shape is fixture-tested once.
pub(crate) fn parse_graph_comments(raw: &serde_json::Value) -> Vec<CommentData> {
    raw.get("data")
        .and_then(|d| d.as_array())
        .map(|items| items.iter().filter_map(graph_comment).collect())
        .unwrap_or_default()
}

fn graph_comment(item: &serde_json::Value) -> Option<CommentData> {
    let id = item.get("id")?.as_str()?.to_string();
    let text = item
        .get("text")
        .or_else(|| item.get("message"))
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_string();
    let created_at = item
        .get("timestamp")
        .or_else(|| item.get("created_time"))
        .and_then(|t| t.as_str())
        .map(common::parse_timestamp)
        .unwrap_or_else(chrono::Utc::now);
    let author_name = item
        .get("username")
        .or_else(|| item.pointer("/from/name"))
        .and_then(|n| n.as_str())
        .map(String::from);
    let author_avatar = item
        .get("profile_picture_url")
        .or_else(|| item.pointer("/from/picture/data/url"))
        .and_then(|u| u.as_str())
        .map(String::from);
    let like_count = count(item, "like_count").unwrap_or(0);
    let replies = item
        .pointer("/replies/data")
        .or_else(|| item.pointer("/comments/data"))
        .and_then(|d| d.as_array())
        .map(|items| items.iter().filter_map(graph_comment).collect())
        .unwrap_or_default();

    Some(CommentData {
        id,
        author_name,
        author_avatar,
        text,
        created_at,
        like_count,
        replies,
    })
}

/// Parse a Meta mentions payload into @-autocomplete candidates (v25 §2 row 6).
///
/// Meta exposes no member-search endpoint, so the only autocomplete source the
/// API offers is the set of accounts that already mention you
/// (`/{ig-id}/mentions`, `/{user-id}/mentions`). An empty `query` returns the
/// whole page; otherwise only usernames containing it (case-insensitive) are
/// kept, so the caller can pass the API's `search` param or filter locally.
pub(crate) fn parse_graph_mentions(raw: &serde_json::Value, query: &str) -> Vec<MentionResult> {
    let needle = query.trim().to_lowercase();
    raw.get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let username = item.get("username").and_then(|u| u.as_str())?;
                    if !needle.is_empty() && !username.to_lowercase().contains(&needle) {
                        return None;
                    }
                    Some(MentionResult {
                        id: item
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or(username)
                            .to_string(),
                        label: username.to_string(),
                        image: item
                            .get("profile_picture_url")
                            .or_else(|| item.get("threads_profile_picture_url"))
                            .and_then(|u| u.as_str())
                            .map(String::from),
                        do_not_cache: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a LinkedIn v2 `socialActions/{urn}/comments` response into comments
/// (v25 §2 row 5). The member and page providers share this shape.
pub(crate) fn parse_linkedin_comments(raw: &serde_json::Value) -> Vec<CommentData> {
    raw.get("elements")
        .and_then(|e| e.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|element| {
                    let id = element.get("id")?.as_str()?.to_string();
                    Some(CommentData {
                        id,
                        // `actor` is a URN ("urn:li:person:abc"); only the id
                        // half is meaningful — the REST comments endpoint
                        // returns no profile or avatar.
                        author_name: element
                            .get("actor")
                            .and_then(|a| a.as_str())
                            .map(|a| a.rsplit(':').next().unwrap_or(a).to_string()),
                        author_avatar: None,
                        text: element
                            .pointer("/message/text")
                            .and_then(|t| t.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        created_at: element
                            .get("createdAt")
                            .and_then(|ms| ms.as_i64())
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .unwrap_or_else(chrono::Utc::now),
                        like_count: element
                            .get("likesSummary")
                            .and_then(|s| count(s, "totalLikes"))
                            .unwrap_or(0),
                        // LinkedIn's REST comments endpoint returns one flat
                        // level; the UI threads replies client-side.
                        replies: vec![],
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a LinkedIn `/v2/connections` response into @-autocomplete candidates
/// (v25 §2 row 6). LinkedIn has no people-search endpoint, so first-degree
/// connections are the only member list a token can read.
pub(crate) fn parse_linkedin_connections(
    raw: &serde_json::Value,
    query: &str,
) -> Vec<MentionResult> {
    let needle = query.trim().to_lowercase();
    raw.get("elements")
        .and_then(|e| e.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(|n| n.as_str())?;
                    if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                        return None;
                    }
                    Some(MentionResult {
                        id: name.to_string(),
                        label: name.to_string(),
                        image: None,
                        // Connections change slowly and the endpoint is not
                        // query-filterable server-side; don't cache per keystroke.
                        do_not_cache: Some(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a provider's raw engagement JSON into a normalized EngagementData struct.
/// Each provider returns a different JSON shape from get_post_engagement().
/// This function handles all known provider-specific formats.
pub fn parse_engagement_data(provider: &str, raw: serde_json::Value) -> EngagementData {
    let mut e = EngagementData {
        likes: 0,
        comments: 0,
        shares: 0,
        views: 0,
        saves: 0,
        quotes: 0,
        reposts: 0,
        replies: 0,
        reactions: None,
        upvotes: 0,
        downvotes: 0,
        upvote_ratio: None,
        awards: 0,
        raw: Some(raw.clone()),
    };

    match provider {
        // X/Twitter: { "public_metrics": { "like_count": 42, "retweet_count": 8, "reply_count": 3, "quote_count": 1, "impression_count": 1200, "bookmark_count": 5 } }
        "x" => {
            let pm = raw
                .get("public_metrics")
                .or_else(|| raw.as_object().map(|_| &raw));
            if let Some(m) = pm {
                e.likes = m.get("like_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                e.replies = m.get("reply_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                e.reposts = m.get("retweet_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                e.quotes = m.get("quote_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                e.views = m
                    .get("impression_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                e.saves = m
                    .get("bookmark_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
            }
        }

        // Reddit: { "score": 42, "num_comments": 12, "upvote_ratio": 0.95, "ups": 45, "downs": 3, "total_awards_received": 2 }
        "reddit" => {
            e.upvotes = raw.get("ups").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            e.downvotes = raw.get("downs").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            e.upvote_ratio = raw
                .get("upvote_ratio")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            e.comments = raw
                .get("num_comments")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.awards = raw
                .get("total_awards_received")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            // Score as likes (positive engagement indicator)
            e.likes = raw.get("score").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        }

        // Bluesky: { "likeCount": 42, "repostCount": 8, "replyCount": 3, "quoteCount": 1 }
        "bluesky" => {
            e.likes = raw.get("likeCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            e.reposts = raw.get("repostCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            e.replies = raw.get("replyCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            e.quotes = raw.get("quoteCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        }

        // Instagram (Facebook Graph + graph.instagram.com), flat or insights-envelope:
        // { "like_count": 42, "comments_count": 12, "saved": 5, "reach": 300, "views": 420 }
        "instagram" | "instagram_standalone" | "instagram-standalone" => {
            let g = |m: &str| insight_value(&raw, m).unwrap_or(0) as i32;
            e.likes = g("like_count");
            e.comments = g("comments_count");
            e.saves = match insight_value(&raw, "saved") {
                Some(v) => v as i32,
                None => g("saved_count"),
            };
            e.views = ["views", "plays", "reach", "impressions"]
                .iter()
                .find_map(|m| insight_value(&raw, m))
                .unwrap_or(0) as i32;
        }

        // LinkedIn: { "likeCount": 42, "commentCount": 12, "shareCount": 5 }
        "linkedin" | "linkedin_page" | "linkedin-page" => {
            e.likes = raw
                .get("likeCount")
                .or_else(|| raw.get("likes"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.comments = raw
                .get("commentCount")
                .or_else(|| raw.get("comments"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.shares = raw
                .get("shareCount")
                .or_else(|| raw.get("shares"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.views = raw
                .get("impressionCount")
                .or_else(|| raw.get("impressions"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
        }

        // Facebook: Graph API returns reactions.summary.total_count (not "likes")
        // { "reactions": { "summary": { "total_count": 42 } }, "comments": { "summary": { "total_count": 12 } }, "shares": { "count": 5 } }
        "facebook" => {
            e.likes = raw
                .get("reactions")
                .and_then(|l| l.get("summary"))
                .and_then(|s| s.get("total_count"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.comments = raw
                .get("comments")
                .and_then(|c| c.get("summary"))
                .and_then(|s| s.get("total_count"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.shares = raw
                .get("shares")
                .and_then(|s| s.get("count"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            // Reactions breakdown from data array (not the summary object)
            if let Some(data) = raw
                .get("reactions")
                .and_then(|r| r.get("data"))
                .and_then(|d| d.as_array())
            {
                let mut rmap = serde_json::Map::new();
                for reaction in data {
                    if let Some(rtype) = reaction["type"].as_str() {
                        let key = rtype.to_lowercase();
                        let count = rmap.get(&key).and_then(|v| v.as_i64()).unwrap_or(0) + 1;
                        rmap.insert(key, serde_json::json!(count));
                    }
                }
                if !rmap.is_empty() {
                    e.reactions = Some(serde_json::Value::Object(rmap));
                }
            }
        }

        // YouTube: { "viewCount": 1200, "likeCount": 42, "dislikeCount": 2, "commentCount": 12 }
        // The Data API encodes these as strings, and callers may hand us either
        // the normalized flat object or the raw `statistics` sub-object.
        "youtube" => {
            let m = raw.get("statistics").unwrap_or(&raw);
            e.views = count(m, "viewCount").unwrap_or(0);
            e.likes = count(m, "likeCount").unwrap_or(0);
            e.comments = count(m, "commentCount").unwrap_or(0);
        }

        // Mastodon: { "favourites_count": 42, "reblogs_count": 8, "replies_count": 3 }
        "mastodon" => {
            e.likes = raw
                .get("favourites_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.reposts = raw
                .get("reblogs_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            e.replies = raw
                .get("replies_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
        }

        // TikTok: { "like_count": 42, "comment_count": 12, "share_count": 5, "view_count": 1200 }
        // The provider flattens the Display API envelope to the single video
        // object before calling this, so the flat shape is the contract.
        "tiktok" => {
            e.likes = count(&raw, "like_count").unwrap_or(0);
            e.comments = count(&raw, "comment_count").unwrap_or(0);
            e.shares = count(&raw, "share_count").unwrap_or(0);
            e.views = count(&raw, "view_count").unwrap_or(0);
        }

        // Pinterest: { "all": { "summary_metrics": { "IMPRESSION": 1200, "PIN_CLICK": 42,
        //                          "OUTBOUND_CLICK": 12, "SAVE": 7 } } }
        // Also accepts a flat metrics object, and a body-level `summary_metrics`.
        // Pinterest has no like metric, so pin clicks stand in as the positive
        // engagement signal (the same treatment Reddit's score gets above);
        // saves and impressions map literally, and OUTBOUND_CLICK stays in `raw`.
        "pinterest" => {
            let m = raw
                .get("all")
                .and_then(|a| a.get("summary_metrics"))
                .or_else(|| raw.get("summary_metrics"))
                .unwrap_or(&raw);
            e.views = count(m, "IMPRESSION").unwrap_or(0);
            e.saves = count(m, "SAVE").unwrap_or(0);
            e.likes = count(m, "PIN_CLICK").unwrap_or(0);
        }

        // Threads: { "like_count": 42, "reply_count": 12, "repost_count": 5, "quote_count": 1 }
        "threads" => {
            let g = |m: &str| insight_value(&raw, m).unwrap_or(0) as i32;
            e.likes = g("like_count");
            e.replies = g("reply_count");
            e.reposts = g("repost_count");
            e.quotes = g("quote_count");
        }

        _ => {}
    }

    e
}

/// Convert EngagementData into the DB row format for upsert (all numeric fields plus JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementRow {
    pub likes: i32,
    pub comments: i32,
    pub shares: i32,
    pub views: i32,
    pub saves: i32,
    pub quotes: i32,
    pub reposts: i32,
    pub replies: i32,
    pub reactions: serde_json::Value,
    pub upvotes: i32,
    pub downvotes: i32,
    pub upvote_ratio: Option<f32>,
    pub awards: i32,
    pub raw: serde_json::Value,
}

#[cfg(test)]
/// Minimal offline `Config` for provider unit tests.
///
/// Lives here so every provider's `#[cfg(test)]` module shares one literal
/// instead of each carrying its own ~60-line copy. No credentials, no DB.
pub(crate) fn test_config() -> crate::config::Config {
    crate::config::Config {
        database_url: format!(
            "sqlite://{}?mode=rwc",
            std::env::temp_dir()
                .join(format!("switchpost-test-{}.db", uuid::Uuid::new_v4()))
                .display()
        ),
        jwt_secret: "test".into(),
        app_password: "test".into(),
        app_url: "http://localhost:3000".into(),
        frontend_url: "http://localhost:4200".into(),
        x_client_id: None,
        x_client_secret: None,
        x_auth_token: None,
        x_ct0: None,
        linkedin_client_id: Some("test_linkedin_id".into()),
        linkedin_client_secret: Some("test_linkedin_secret".into()),
        bluesky_handle: Some("test.bsky.social".into()),
        bluesky_app_password: Some("test-app-password".into()),
        facebook_client_id: None,
        facebook_client_secret: None,
        instagram_client_id: None,
        instagram_client_secret: None,
        threads_app_id: None,
        threads_app_secret: None,
        youtube_client_id: None,
        youtube_client_secret: None,
        reddit_client_id: Some("test_reddit_id".into()),
        reddit_client_secret: Some("test_reddit_secret".into()),
        reddit_username: Some("test_reddit_user".into()),
        reddit_password: Some("test_reddit_pass".into()),
        reddit_access_token: None,
        reddit_refresh_token: None,
        reddit_cookie_token: None,
        discord_client_id: None,
        discord_client_secret: None,
        discord_bot_token: None,
        telegram_bot_tokens: None,
        telegram_session_dir: None,
        telegram_api_id: None,
        telegram_api_hash: None,
        tiktok_client_id: None,
        tiktok_client_secret: None,
        medium_access_token: None,
        devto_api_key: None,
        pinterest_client_id: None,
        pinterest_client_secret: None,
        whatsapp_store_dir: None,
        whatsapp_cloud_token: None,
        whatsapp_cloud_phone_number_id: None,
        whatsapp_cloud_waba_id: None,
        whatsapp_cloud_verify_token: None,
        whatsapp_cloud_app_secret: None,
        slack_client_id: None,
        slack_client_secret: None,
        instagram_app_id: None,
        instagram_app_secret: None,
        mastodon_client_id: None,
        mastodon_client_secret: None,
        mastodon_instance_url: None,
        hashnode_api_key: None,
        github_token: None,
        neynar_api_key: None,
        token_encryption_key: None,
        media_dir: "./uploads".into(),
        llm_endpoint: None,
        llm_model: None,
        dub_co_api_key: None,
        dub_co_workspace: None,
        strip_links_from_x: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_error ───────────────────────────────────────

    #[test]
    fn classify_error_detects_rate_limit_from_code_88() {
        assert_eq!(
            classify_error(r#"{"errors":[{"code":88}]}"#, 400),
            Some(ErrorKind::RateLimited)
        );
    }

    #[test]
    fn classify_error_detects_rate_limit_from_429_status() {
        assert_eq!(
            classify_error("too many requests", 429),
            Some(ErrorKind::RateLimited)
        );
    }

    #[test]
    fn classify_error_prefers_body_rate_limit_over_403() {
        assert_eq!(
            classify_error("Rate limit exceeded for this method", 403),
            Some(ErrorKind::RateLimited)
        );
    }

    #[test]
    fn classify_error_detects_duplicate() {
        assert_eq!(
            classify_error("Status is a duplicate.", 403),
            Some(ErrorKind::Duplicate)
        );
    }

    #[test]
    fn classify_error_detects_media_rejection() {
        assert_eq!(
            classify_error("The media upload was rejected by the platform", 400),
            Some(ErrorKind::MediaRejected)
        );
    }

    #[test]
    fn classify_error_detects_expired_grant() {
        assert_eq!(
            classify_error(r#"{"error":"invalid_grant"}"#, 400),
            Some(ErrorKind::AuthExpired)
        );
    }

    #[test]
    fn classify_error_falls_back_to_status_codes() {
        assert_eq!(classify_error("", 401), Some(ErrorKind::AuthExpired));
        assert_eq!(classify_error("", 403), Some(ErrorKind::Forbidden));
        assert_eq!(classify_error("", 404), Some(ErrorKind::NotFound));
        assert_eq!(classify_error("", 400), Some(ErrorKind::InvalidRequest));
        assert_eq!(classify_error("", 422), Some(ErrorKind::InvalidRequest));
    }

    #[test]
    fn classify_error_returns_none_for_uninformative_server_error() {
        assert_eq!(classify_error("Internal Server Error", 500), None);
    }

    // ── parse_engagement_data: X ─────────────────────────────

    #[test]
    fn parse_engagement_data_maps_x_v2_public_metrics() {
        let raw = serde_json::json!({
            "public_metrics": {
                "like_count": 42, "retweet_count": 8, "reply_count": 3,
                "quote_count": 1, "impression_count": 1200, "bookmark_count": 5
            }
        });
        let e = parse_engagement_data("x", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.reposts, 8);
        assert_eq!(e.replies, 3);
        assert_eq!(e.quotes, 1);
        assert_eq!(e.views, 1200);
        assert_eq!(e.saves, 5);
    }

    // ── parse_engagement_data: Reddit ────────────────────────

    #[test]
    fn parse_engagement_data_maps_reddit_submission_metrics() {
        let raw = serde_json::json!({
            "score": 120, "num_comments": 12, "upvote_ratio": 0.95,
            "ups": 123, "downs": 3, "total_awards_received": 2
        });
        let e = parse_engagement_data("reddit", raw);
        assert_eq!(e.likes, 120);
        assert_eq!(e.comments, 12);
        assert_eq!(e.upvote_ratio, Some(0.95));
        assert_eq!(e.upvotes, 123);
        assert_eq!(e.downvotes, 3);
        assert_eq!(e.awards, 2);
    }

    #[test]
    fn parse_engagement_data_treats_null_reddit_ups_as_zero() {
        // Reddit omits ups/downs for non-authors — must not panic or read as NaN.
        let raw = serde_json::json!({ "score": 7, "num_comments": 1, "ups": null, "downs": null });
        let e = parse_engagement_data("reddit", raw);
        assert_eq!(e.upvotes, 0);
        assert_eq!(e.downvotes, 0);
        assert_eq!(e.likes, 7);
    }

    // ── parse_engagement_data: Bluesky ───────────────────────

    #[test]
    fn parse_engagement_data_maps_bluesky_post_metrics() {
        let raw = serde_json::json!({
            "likeCount": 42, "repostCount": 8, "replyCount": 3, "quoteCount": 1
        });
        let e = parse_engagement_data("bluesky", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.reposts, 8);
        assert_eq!(e.replies, 3);
        assert_eq!(e.quotes, 1);
    }

    // ── parse_engagement_data: LinkedIn (member + page) ──────

    #[test]
    fn parse_engagement_data_maps_linkedin_social_actions() {
        let raw = serde_json::json!({
            "likeCount": 42, "commentCount": 12, "shareCount": 5, "impressionCount": 900
        });
        for provider in ["linkedin", "linkedin_page", "linkedin-page"] {
            let e = parse_engagement_data(provider, raw.clone());
            assert_eq!(e.likes, 42, "{provider} likes");
            assert_eq!(e.comments, 12, "{provider} comments");
            assert_eq!(e.shares, 5, "{provider} shares");
            assert_eq!(e.views, 900, "{provider} views");
        }
    }

    #[test]
    fn parse_engagement_data_unknown_provider_yields_zeroes() {
        let e = parse_engagement_data("not-a-provider", serde_json::json!({ "likeCount": 5 }));
        assert_eq!(e.likes, 0);
    }
}

impl From<EngagementData> for EngagementRow {
    fn from(e: EngagementData) -> Self {
        EngagementRow {
            likes: e.likes,
            comments: e.comments,
            shares: e.shares,
            views: e.views,
            saves: e.saves,
            quotes: e.quotes,
            reposts: e.reposts,
            replies: e.replies,
            reactions: e
                .reactions
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
            upvotes: e.upvotes,
            downvotes: e.downvotes,
            upvote_ratio: e.upvote_ratio,
            awards: e.awards,
            raw: e
                .raw
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        }
    }
}

#[cfg(test)]
mod insights_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn parse_should_read_flat_metric() {
        let raw = serde_json::json!({ "like_count": 42 });
        assert_eq!(insight_value(&raw, "like_count"), Some(42));
    }

    #[test]
    fn parse_should_read_metric_from_values_envelope() {
        let raw = serde_json::json!({
            "data": [
                { "name": "reach", "period": "lifetime", "values": [{ "value": 300, "end_time": "2026-09-29T00:00:00+0000" }] }
            ]
        });
        assert_eq!(insight_value(&raw, "reach"), Some(300));
    }

    #[test]
    fn parse_should_read_metric_from_single_value_envelope() {
        let raw = serde_json::json!({ "data": [{ "name": "follower_count", "value": 1200 }] });
        assert_eq!(insight_value(&raw, "follower_count"), Some(1200));
    }

    #[test]
    fn parse_should_return_none_for_absent_metric() {
        let raw = serde_json::json!({ "data": [{ "name": "reach", "value": 1 }] });
        assert_eq!(insight_value(&raw, "views"), None);
    }

    #[test]
    fn parse_should_expand_daily_insights_into_dashboard_series() {
        let raw = serde_json::json!({
            "data": [
                {
                    "name": "views",
                    "period": "day",
                    "values": [
                        { "value": 10, "end_time": "2026-09-28T00:00:00+0000" },
                        { "value": 25, "end_time": "2026-09-29T00:00:00+0000" }
                    ]
                },
                { "name": "follower_count", "value": 1200 }
            ]
        });
        let series = parse_insights_data(&raw);
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].label, "views");
        assert_eq!(series[0].data.len(), 2);
        assert_eq!(series[0].data[1].total, "25");
        assert_eq!(series[0].data[1].date, "2026-09-29T00:00:00+0000");
        assert_eq!(series[1].label, "follower_count");
        assert_eq!(series[1].data[0].total, "1200");
    }

    #[test]
    fn parse_should_map_facebook_reaction_comment_share_shape() {
        let raw = serde_json::json!({
            "id": "123_456",
            "reactions": {
                "summary": { "total_count": 42 },
                "data": [
                    { "type": "LIKE", "total_count": 30 },
                    { "type": "LOVE", "total_count": 7 },
                    { "type": "LIKE", "total_count": 5 }
                ]
            },
            "comments": { "summary": { "total_count": 12 } },
            "shares": { "count": 5 }
        });
        let e = parse_engagement_data("facebook", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
        assert_eq!(e.shares, 5);
        // Repeated reaction types are aggregated into the breakdown map.
        assert_eq!(
            e.reactions,
            Some(serde_json::json!({ "like": 2, "love": 1 }))
        );
    }

    #[test]
    fn parse_should_map_instagram_graph_media_shape() {
        let raw = serde_json::json!({
            "id": "17900000000000000",
            "like_count": 42,
            "comments_count": 12,
            "saved": 7,
            "reach": 300,
            "views": 420
        });
        let e = parse_engagement_data("instagram", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
        assert_eq!(e.saves, 7);
        assert_eq!(e.views, 420);
    }

    #[test]
    fn parse_should_map_instagram_standalone_insights_envelope_shape() {
        let raw = serde_json::json!({
            "id": "17900000000000000",
            "like_count": 8,
            "comments_count": 2,
            "data": [
                { "name": "reach", "values": [{ "value": 90, "end_time": "2026-09-29T00:00:00+0000" }] },
                { "name": "saved", "values": [{ "value": 3, "end_time": "2026-09-29T00:00:00+0000" }] }
            ]
        });
        let e = parse_engagement_data("instagram-standalone", raw);
        assert_eq!(e.likes, 8);
        assert_eq!(e.comments, 2);
        assert_eq!(e.saves, 3);
        assert_eq!(e.views, 90);
    }

    #[test]
    fn parse_should_map_threads_reaction_shape() {
        let raw = serde_json::json!({
            "id": "8888888888888888888",
            "like_count": 42,
            "reply_count": 12,
            "repost_count": 5,
            "quote_count": 1
        });
        let e = parse_engagement_data("threads", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.replies, 12);
        assert_eq!(e.reposts, 5);
        assert_eq!(e.quotes, 1);
    }

    #[test]
    fn parse_should_keep_raw_payload_for_engagement_row() {
        let raw = serde_json::json!({ "like_count": 1 });
        let e = parse_engagement_data("instagram", raw.clone());
        assert_eq!(e.raw, Some(raw));
    }

    // ── parse_graph_comments ─────────────────────────────────

    #[test]
    fn parse_should_read_facebook_comment_field_names() {
        let raw = serde_json::json!({
            "data": [{
                "id": "c1",
                "message": "Nice post",
                "created_time": "2026-09-28T10:00:00+0000",
                "from": { "name": "Ada", "picture": { "data": { "url": "https://img/a.jpg" } } },
                "comments": { "data": [{
                    "id": "c2", "message": "thanks", "created_time": "2026-09-28T11:00:00+0000",
                    "from": { "name": "Bob" }
                }]}
            }]
        });
        let comments = parse_graph_comments(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "c1");
        assert_eq!(comments[0].text, "Nice post");
        assert_eq!(comments[0].author_name.as_deref(), Some("Ada"));
        assert_eq!(
            comments[0].author_avatar.as_deref(),
            Some("https://img/a.jpg")
        );
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].author_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn parse_should_read_instagram_comment_field_names() {
        let raw = serde_json::json!({
            "data": [{
                "id": "ig1",
                "text": "great shot",
                "timestamp": "2026-09-28T10:00:00+0000",
                "username": "ada",
                "like_count": 4,
                "replies": { "data": [{ "id": "ig2", "text": "ty", "username": "bob" }] }
            }]
        });
        let comments = parse_graph_comments(&raw);
        assert_eq!(comments[0].text, "great shot");
        assert_eq!(comments[0].author_name.as_deref(), Some("ada"));
        assert_eq!(comments[0].like_count, 4);
        assert_eq!(comments[0].replies.len(), 1);
    }

    #[test]
    fn parse_should_skip_comments_without_an_id() {
        let raw = serde_json::json!({ "data": [{ "text": "orphan" }] });
        assert!(parse_graph_comments(&raw).is_empty());
    }

    #[test]
    fn parse_should_read_no_comments_from_error_body() {
        let raw = serde_json::json!({ "error": { "message": "Unsupported get request." } });
        assert!(parse_graph_comments(&raw).is_empty());
    }

    // ── parse_graph_mentions ─────────────────────────────────

    #[test]
    fn parse_should_return_every_account_for_blank_mention_query() {
        let raw = serde_json::json!({ "data": [
            { "id": "1", "username": "ada", "profile_picture_url": "https://img/a.jpg" },
            { "id": "2", "username": "bob" }
        ]});
        let mentions = parse_graph_mentions(&raw, "");
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].label, "ada");
        assert_eq!(mentions[0].image.as_deref(), Some("https://img/a.jpg"));
    }

    #[test]
    fn parse_should_filter_mentions_by_username_case_insensitively() {
        let raw = serde_json::json!({ "data": [
            { "id": "1", "username": "AdaLovelace" },
            { "id": "2", "username": "bob" }
        ]});
        let mentions = parse_graph_mentions(&raw, "adalove");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "1");
    }

    #[test]
    fn parse_should_fall_back_to_username_when_mention_id_absent() {
        let raw = serde_json::json!({ "data": [{ "username": "ada" }] });
        assert_eq!(parse_graph_mentions(&raw, "")[0].id, "ada");
    }

    // ── parse_linkedin_comments ──────────────────────────────

    #[test]
    fn parse_should_read_linkedin_social_actions_comments() {
        let raw = serde_json::json!({ "elements": [{
            "id": "urn:li:comment:123",
            "message": { "text": "Congrats" },
            "createdAt": 1_757_000_000_000_i64,
            "actor": "urn:li:person:abc",
            "likesSummary": { "totalLikes": 3 }
        }]});
        let comments = parse_linkedin_comments(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "Congrats");
        assert_eq!(comments[0].author_name.as_deref(), Some("abc"));
        assert_eq!(comments[0].like_count, 3);
        assert!(comments[0].replies.is_empty());
    }

    #[test]
    fn parse_should_default_missing_linkedin_comment_fields() {
        let comments =
            parse_linkedin_comments(&serde_json::json!({ "elements": [{ "id": "c1" }] }));
        assert_eq!(comments[0].text, "");
        assert_eq!(comments[0].author_name, None);
        assert_eq!(comments[0].like_count, 0);
    }

    // ── parse_linkedin_connections ───────────────────────────

    #[test]
    fn parse_should_return_all_linkedin_connections_for_blank_query() {
        let raw = serde_json::json!({ "elements": [
            { "name": "Ada Lovelace" }, { "name": "Bob Smith" }
        ]});
        let mentions = parse_linkedin_connections(&raw, "");
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].label, "Ada Lovelace");
        assert_eq!(mentions[0].do_not_cache, Some(true));
    }

    #[test]
    fn parse_should_filter_linkedin_connections_by_name_case_insensitively() {
        let raw = serde_json::json!({ "elements": [
            { "name": "Ada Lovelace" }, { "name": "Bob Smith" }
        ]});
        let mentions = parse_linkedin_connections(&raw, "BOB");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "Bob Smith");
    }

    #[test]
    fn parse_should_read_no_linkedin_connections_from_error_body() {
        assert!(
            parse_linkedin_connections(&serde_json::json!({ "message": "Not Found" }), "")
                .is_empty()
        );
    }

    // ── pages_to_targets ─────────────────────────────────────

    fn page(id: &str, name: &str, username: Option<&str>) -> PageInfo {
        PageInfo {
            id: id.into(),
            name: name.into(),
            access_token: None,
            picture: None,
            username: username.map(String::from),
        }
    }

    #[test]
    fn parse_should_map_pages_onto_targets_with_the_given_type() {
        let targets = pages_to_targets(&[page("p1", "Acme", None)], "page");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "p1");
        assert_eq!(targets[0].name, "Acme");
        assert_eq!(targets[0].target_type, "page");
    }

    #[test]
    fn parse_should_pass_through_the_target_type_each_provider_asks_for() {
        for (kind, pages) in [
            ("page", vec![page("p1", "Acme", None)]),
            ("channel", vec![page("c1", "Rust Lang", None)]),
            ("board", vec![page("b1", "Recipes", None)]),
        ] {
            assert_eq!(pages_to_targets(&pages, kind)[0].target_type, kind);
        }
    }

    #[test]
    fn parse_should_publish_page_username_as_a_handle_in_target_metadata() {
        // One neutral key for the FB username / LI vanity name / YT custom URL
        // so TargetPicker does not branch per platform.
        let targets = pages_to_targets(&[page("p1", "Acme", Some("acme"))], "page");
        assert_eq!(targets[0].metadata.as_ref().unwrap()["handle"], "acme");
    }

    #[test]
    fn parse_should_omit_target_metadata_when_the_page_has_no_username() {
        let targets = pages_to_targets(&[page("p1", "Acme", None)], "page");
        assert!(targets[0].metadata.is_none());
    }

    #[test]
    fn parse_should_read_no_targets_from_an_empty_page_listing() {
        assert!(pages_to_targets(&[], "board").is_empty());
    }

    // ── pending-publish lifecycle ────────────────────────────

    fn result(status: &str) -> PublishResult {
        PublishResult {
            platform_post_id: "post-1".into(),
            platform_post_url: None,
            status: status.into(),
        }
    }

    #[test]
    fn pending_statuses_are_distinguished_from_terminal_ones() {
        for s in [
            "pending",
            "processing",
            "IN_PROGRESS",
            "accepted",
            "uploaded",
        ] {
            assert!(result(s).is_pending(), "{s} should be pending");
            assert!(!result(s).is_terminal(), "{s} should not be terminal");
            assert!(!result(s).is_published(), "{s} should not be published");
        }
        for s in ["published", "succeeded", "COMPLETED", "live", "sent"] {
            assert!(result(s).is_published(), "{s} should be published");
            assert!(result(s).is_terminal());
        }
        for s in ["error", "failed", "expired", "some_future_status"] {
            assert!(result(s).is_terminal(), "{s} should be terminal");
            assert!(!result(s).is_published(), "{s} must not claim a live post");
        }
    }

    /// Stands in for an async platform: `post_pending` accepts, then
    /// `check_post_status` reports pending `pending_times` times before the
    /// terminal status. With `pending_times == 0` it never reports pending at
    /// all, which is exactly one of the 23 synchronous providers.
    struct FakeAsyncProvider {
        pending_times: u32,
        terminal_status: &'static str,
        polls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SocialProvider for FakeAsyncProvider {
        fn identifier(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn scopes(&self) -> Vec<String> {
            vec![]
        }
        fn max_content_length(&self) -> usize {
            280
        }

        async fn generate_auth_url(
            &self,
            _s: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthUrlResponse, ProviderError> {
            unimplemented!()
        }
        async fn exchange_code(
            &self,
            _c: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn refresh_token(&self, _r: &str) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn fetch_page_info(&self, _t: &str, _p: &str) -> Result<PageInfo, ProviderError> {
            unimplemented!()
        }
        async fn publish(
            &self,
            _t: &str,
            _p: &PostContent,
        ) -> Result<PublishResult, ProviderError> {
            Ok(result("published"))
        }

        async fn post_pending(
            &self,
            t: &str,
            p: &PostContent,
            _internal_id: &str,
        ) -> Result<PublishResult, ProviderError> {
            // `pending_times == 0` is the synchronous case: delegate to
            // `publish` and report published, exactly as the trait default does.
            if self.pending_times == 0 {
                return self.publish(t, p).await;
            }
            Ok(result("pending"))
        }

        async fn check_post_status(
            &self,
            _t: &str,
            _id: &str,
        ) -> Result<PublishResult, ProviderError> {
            let seen = self.polls.fetch_add(1, Ordering::SeqCst);
            if (seen as u32) < self.pending_times {
                Ok(result("processing"))
            } else {
                Ok(result(self.terminal_status))
            }
        }
    }

    fn fake(pending_times: u32, terminal_status: &'static str) -> FakeAsyncProvider {
        FakeAsyncProvider {
            pending_times,
            terminal_status,
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[tokio::test]
    async fn poll_until_terminal_keeps_polling_until_the_post_goes_live() {
        let p = fake(3, "published");
        let polls = Arc::clone(&p.polls);
        let out = poll_until_terminal(Duration::ZERO, || p.check_post_status("", "post-1"))
            .await
            .expect("should reach a terminal state");

        assert!(out.is_published());
        assert_eq!(polls.load(Ordering::SeqCst), 4, "3 pending + 1 published");
    }

    #[tokio::test]
    async fn poll_until_terminal_surfaces_a_platform_failure_as_terminal() {
        let p = fake(2, "error");
        let out = poll_until_terminal(Duration::ZERO, || p.check_post_status("", "post-1"))
            .await
            .expect("failure is a terminal result, not a poll error");

        assert!(!out.is_published(), "a failed post must not look live");
    }

    #[tokio::test]
    async fn poll_until_terminal_gives_up_rather_than_reporting_a_pending_post_as_published() {
        let p = fake(u32::MAX, "published");
        let polls = Arc::clone(&p.polls);
        let err = poll_until_terminal(Duration::ZERO, || p.check_post_status("", "post-1"))
            .await
            .expect_err("an endlessly pending platform must not resolve");

        assert!(matches!(err, ProviderError::Api(_)));
        assert_eq!(polls.load(Ordering::SeqCst), MAX_POLL_ATTEMPTS as usize);
    }

    #[tokio::test]
    async fn finalize_post_polls_a_pending_publish_to_terminal() {
        let p = fake(2, "published");
        let out = p
            .finalize_post("", "post-1", Duration::ZERO)
            .await
            .expect("finalize should reach a terminal state");

        assert!(out.is_published());
    }

    #[tokio::test]
    async fn synchronous_providers_keep_defaulting_to_a_single_publish() {
        // A provider that never reports pending must not reach the poll path:
        // `post_pending` delegates to `publish` and reports published, so the
        // scheduler's `if result.is_pending()` is false. This is the
        // 23-provider no-change path.
        let p = fake(0, "published");
        let accepted = p
            .post_pending("", &PostContent::default(), "")
            .await
            .unwrap();
        assert!(
            accepted.is_published(),
            "default post_pending must delegate to publish"
        );
        assert!(!accepted.is_pending(), "so the scheduler never polls it");
    }

    /// A provider that overrides nothing but the required methods — the shape
    /// of all 23 synchronous providers.
    struct SyncProvider;

    #[async_trait]
    impl SocialProvider for SyncProvider {
        fn identifier(&self) -> &'static str {
            "sync"
        }
        fn name(&self) -> &'static str {
            "Sync"
        }
        fn scopes(&self) -> Vec<String> {
            vec![]
        }
        fn max_content_length(&self) -> usize {
            280
        }

        async fn generate_auth_url(
            &self,
            _s: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthUrlResponse, ProviderError> {
            unimplemented!()
        }
        async fn exchange_code(
            &self,
            _c: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn refresh_token(&self, _r: &str) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn fetch_page_info(&self, _t: &str, _p: &str) -> Result<PageInfo, ProviderError> {
            unimplemented!()
        }
        async fn publish(
            &self,
            _t: &str,
            _p: &PostContent,
        ) -> Result<PublishResult, ProviderError> {
            Ok(result("published"))
        }
    }

    #[tokio::test]
    async fn provider_with_no_lifecycle_overrides_finalizes_in_a_single_default_check() {
        // The whole point of the defaults: a synchronous provider is polled
        // once, reports published, and behaves exactly as it did before this
        // lifecycle existed.
        let p = SyncProvider;
        let out = p.finalize_post("", "post-1", Duration::ZERO).await.unwrap();
        assert!(out.is_published());
        assert_eq!(out.platform_post_id, "post-1");
    }
}

#[cfg(test)]
mod provider_error_tests {
    use super::*;

    /// A provider with a provider-specific message, to prove the wording
    /// survives the shared classification.
    struct TestProvider;

    #[async_trait]
    impl SocialProvider for TestProvider {
        fn identifier(&self) -> &'static str {
            "test"
        }
        fn name(&self) -> &'static str {
            "Test"
        }
        fn scopes(&self) -> Vec<String> {
            vec![]
        }
        fn max_content_length(&self) -> usize {
            3000
        }
        async fn generate_auth_url(
            &self,
            _state: &str,
            _code_verifier: &str,
            _redirect_uri: &str,
        ) -> Result<AuthUrlResponse, ProviderError> {
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
        async fn refresh_token(&self, _r: &str) -> Result<AuthToken, ProviderError> {
            unreachable!("not used")
        }
        async fn publish(
            &self,
            _t: &str,
            _p: &PostContent,
        ) -> Result<PublishResult, ProviderError> {
            unreachable!("not used")
        }
        async fn fetch_page_info(
            &self,
            _access_token: &str,
            _page_id: &str,
        ) -> Result<PageInfo, ProviderError> {
            unreachable!("not used")
        }
        fn map_error(&self, _body: &str, status: u16) -> Option<String> {
            if status == 401 {
                Some("Check the API key in .env".into())
            } else {
                None
            }
        }
    }

    #[test]
    fn a_rate_limit_is_retryable_whatever_the_provider() {
        // The scheduler backs off and retries a RateLimited error and gives
        // up on a plain Api error. The same 429 used to be one on some
        // platforms and the other on the rest.
        let err = TestProvider.provider_error("slow down", 429);
        assert!(
            matches!(err, ProviderError::RateLimited(_)),
            "a 429 must be a rate limit, got {err:?}"
        );
    }

    #[test]
    fn a_rate_limit_in_the_body_counts_even_on_a_200() {
        let err = TestProvider.provider_error("{\"error\":\"rate limit exceeded\"}", 400);
        assert!(
            matches!(err, ProviderError::RateLimited(_)),
            "the body signal wins, got {err:?}"
        );
    }

    #[test]
    fn an_expired_token_is_the_one_error_the_scheduler_refreshes() {
        let err = TestProvider.provider_error("token expired", 401);
        assert!(
            matches!(err, ProviderError::TokenExpired),
            "a 401 must be a token expiry, got {err:?}"
        );
    }

    #[test]
    fn a_forbidden_request_is_not_retried() {
        let err = TestProvider.provider_error("no permission", 403);
        assert!(
            matches!(err, ProviderError::Auth(_)),
            "a 403 is a grant problem, got {err:?}"
        );
    }

    #[test]
    fn the_provider_wording_survives_the_shared_classification() {
        // A token expiry carries no message (the scheduler writes its own),
        // so the wording shows up on the errors that do carry one.
        let err = TestProvider.provider_error("whatever", 500);
        match err {
            ProviderError::Api(msg) => assert!(
                msg.contains("whatever"),
                "the raw body must survive, got {msg}"
            ),
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[test]
    fn an_ordinary_failure_keeps_its_body() {
        let err = TestProvider.provider_error("weird failure", 500);
        match err {
            ProviderError::Api(msg) => assert!(msg.contains("weird failure"), "got {msg}"),
            other => panic!("expected an API error, got {other:?}"),
        }
    }
}
