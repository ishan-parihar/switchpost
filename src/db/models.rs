// ─── Database Models ──────────────────────────────────────────
// Rust structs matching the PostgreSQL schema.
// Used for serialization/deserialization with sqlx and serde.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── User ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password: String,
    pub name: String,
    pub timezone: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Public user info (no password, no internal fields)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserPublic {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub timezone: i32,
}

impl From<User> for UserPublic {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            email: u.email,
            name: u.name,
            timezone: u.timezone,
        }
    }
}

// ── Integration (Social Channel) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Integration {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider_identifier: String,
    pub provider_name: String,
    pub internal_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub profile_name: Option<String>,
    pub profile_picture: Option<String>,
    pub profile_url: Option<String>,
    pub disabled: bool,
    pub refresh_needed: bool,
    pub root_internal_id: Option<String>,
    pub posting_times: serde_json::Value,
    pub auth_method: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationPublic {
    pub id: Uuid,
    pub provider_identifier: String,
    pub provider_name: String,
    pub internal_id: String,
    pub profile_name: Option<String>,
    pub profile_picture: Option<String>,
    pub profile_url: Option<String>,
    pub disabled: bool,
    pub refresh_needed: bool,
    pub root_internal_id: Option<String>,
    pub posting_times: serde_json::Value,
    pub auth_method: String,
}

impl From<Integration> for IntegrationPublic {
    fn from(i: Integration) -> Self {
        Self {
            id: i.id,
            provider_identifier: i.provider_identifier,
            provider_name: i.provider_name,
            internal_id: i.internal_id,
            profile_name: i.profile_name,
            profile_picture: i.profile_picture,
            profile_url: i.profile_url,
            disabled: i.disabled,
            refresh_needed: i.refresh_needed,
            root_internal_id: i.root_internal_id,
            posting_times: decode_posting_times(i.posting_times),
            auth_method: i.auth_method,
        }
    }
}

/// `posting_times` is stored as TEXT JSON. sqlx decodes a `serde_json::Value`
/// column as that raw text, so the value arrives as a JSON string rather than
/// an array. Every reader goes through here to get the array back.
pub fn decode_posting_times(raw: serde_json::Value) -> serde_json::Value {
    match raw {
        serde_json::Value::String(text) => {
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!([]))
        }
        other => other,
    }
}

/// `posts.media` is stored as raw JSON, and the column has held more than
/// one shape over the app's life:
///
///   `[]`                       the normal case
///   `{}`                       written by an older writer
///   `["/path/a.png", ...]`     a legacy array of BARE PATHS
///   `[{url, mime_type, ...}]`  the current shape
///
/// The frontend assumes an array: the kanban's `coverImage` calls
/// `post.media.find(...)` directly, and a `{}` took the whole ROUTE down
/// with "i.find is not a function" - a pageerror, not just one bad card.
///
/// So the array-ness is guaranteed ON THE READ, in ONE place. The
/// response shape stays what the frontend expects: an array, possibly
/// empty, never an object and never a bare-path string.
pub fn media_as_array(media: &serde_json::Value) -> serde_json::Value {
    match media {
        serde_json::Value::Array(_) => media.clone(),
        serde_json::Value::Null => serde_json::Value::Array(Vec::new()),
        // An object: keep it as a SINGLE attachment if it looks like one,
        // otherwise there is no media to show.
        serde_json::Value::Object(map) => {
            if map.contains_key("url") {
                serde_json::Value::Array(vec![media.clone()])
            } else {
                serde_json::Value::Array(Vec::new())
            }
        }
        _ => serde_json::Value::Array(Vec::new()),
    }
}

// ── Post ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "post_state")]
#[sqlx(rename_all = "lowercase")]
pub enum PostState {
    /// `Idea` is a kanban-only state (migration 026). Posts in this state
    /// are not scheduled and not visible on the calendar — they live in
    /// the "Ideas" kanban column until promoted to `Draft`.
    Idea,
    Draft,
    Queued,
    /// `Publishing` is a transient state: the scheduler has claimed the
    /// post and is mid-API-call. It exists so that (a) a second scheduler
    /// instance can't double-claim the same post, and (b) on shutdown
    /// the operator can distinguish "queued" (safe to retry) from
    /// "publishing" (may have been half-published — needs manual review).
    Publishing,
    Published,
    Error,
}

impl std::fmt::Display for PostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PostState::Idea => write!(f, "idea"),
            PostState::Draft => write!(f, "draft"),
            PostState::Queued => write!(f, "queued"),
            PostState::Publishing => write!(f, "publishing"),
            PostState::Published => write!(f, "published"),
            PostState::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Post {
    pub id: Uuid,
    pub user_id: Uuid,
    pub integration_id: Uuid,
    pub state: PostState,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub settings: serde_json::Value,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub platform_post_id: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<DateTime<Utc>>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    /// Phase v22: stable UUID sent to the provider on every publish
    /// attempt. Providers that support idempotency deduplicate on this
    /// key — prevents double-publish after crash-recovery retries.
    pub idempotency_key: Uuid,
    // v25-3: campaign_id (migration 026) — was missing from the Post struct,
    // which meant the /api/posts list response never included it and the
    // kanban campaign filter (p.campaign_id === selectedCampaign) silently
    // matched nothing. Now surfaced so the filter actually works.
    pub campaign_id: Option<Uuid>,
    // v25-3: kanban fields (migration 034). `#[sqlx(default)]` on the
    // non-Option fields so queries that don't SELECT these columns
    // (e.g. scheduler's get_post_for_publish) still deserialize — they
    // just get 0 / "" which those code paths never read. The list/get
    // queries that the kanban UI depends on DO select these columns.
    #[sqlx(default)]
    pub kanban_sort_order: i32,
    pub kanban_substate: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostPublic {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub integration_name: String,
    pub state: String,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub scheduled_at: Option<String>,
    pub published_at: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<String>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    /// The campaign the post is filed under.
    ///
    /// The CREATE response used to omit it, so a client that set a
    /// campaign could not confirm it was applied - the field was in
    /// the request, it was stored, and the response did not say so.
    /// The list response has carried it since v22; the create
    /// response is the one that was missing it.
    pub campaign_id: Option<Uuid>,
}

impl From<Post> for PostPublic {
    fn from(p: Post) -> Self {
        Self {
            id: p.id,
            integration_id: p.integration_id,
            integration_name: String::new(),
            state: p.state.to_string(),
            content: p.content,
            title: p.title,
            media: p.media,
            scheduled_at: p.scheduled_at.map(|d| d.to_rfc3339()),
            published_at: p.published_at.map(|d| d.to_rfc3339()),
            platform_post_url: p.platform_post_url,
            error_message: p.error_message,
            created_at: p.created_at.to_rfc3339(),
            repeat_interval_days: p.repeat_interval_days,
            repeat_end_date: p.repeat_end_date.map(|d| d.to_rfc3339()),
            group_id: p.group_id,
            first_comment: p.first_comment,
            sequence: p.sequence,
            campaign_id: p.campaign_id,
        }
    }
}

// ── Post with Integration Details (for publishing) ──────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostWithIntegration {
    pub id: Uuid,
    pub user_id: Uuid,
    pub integration_id: Uuid,
    pub state: PostState,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub settings: serde_json::Value,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub platform_post_id: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<DateTime<Utc>>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    /// Phase v22: stable idempotency key for provider deduplication.
    pub idempotency_key: Uuid,
    // Joined from integrations
    pub provider_identifier: String,
    /// The CHANNEL inside the provider: the connected Facebook Page, the
    /// connected LinkedIn organization, the connected subreddit-free account.
    /// A token can administer several sub-accounts, so a provider that
    /// resolves one from the API needs to know which one this post is for.
    pub internal_id: String,
    /// Set when the channel is a SUB-ACCOUNT of the OAuth user (a Facebook
    /// Page, for instance): it stores a PAGE token and the parent's USER
    /// token, so a refresh must re-derive the page token rather than
    /// overwrite it with the user one.
    pub root_internal_id: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub integration_disabled: bool,
    pub integration_refresh_needed: bool,
}

// ── Calendar Post with Engagement Metrics ──────────────────

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct CalendarPostWithMetrics {
    pub id: Uuid,
    pub user_id: Uuid,
    pub integration_id: Uuid,
    pub integration_name: Option<String>,
    pub state: String,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub platform_post_id: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<DateTime<Utc>>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    // v23-4: campaign_id for calendar filtering + chip coloring.
    pub campaign_id: Option<Uuid>,
    // Engagement metrics from analytics_cache LEFT JOIN
    pub likes: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub impressions: Option<i64>,
}

// ── Media ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MediaEntry {
    pub id: Uuid,
    pub user_id: Uuid,
    pub original_name: String,
    pub storage_path: String,
    pub mime_type: String,
    pub file_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub alt_text: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaPublic {
    pub id: Uuid,
    pub original_name: String,
    pub url: String,
    pub mime_type: String,
    pub file_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// The alt text the founder wrote for this media.
    ///
    /// The column is `alt_text` and the upload paths WRITE it, but the
    /// response shape did not carry it - so the frontend's `MediaItem.alt`
    /// was always undefined, and the composer could not show which
    /// attachments have alt text. An agent that set the alt text could
    /// not read it back either.
    #[serde(default)]
    pub alt: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<MediaEntry> for MediaPublic {
    fn from(m: MediaEntry) -> Self {
        Self {
            id: m.id,
            original_name: m.original_name,
            url: format!("/api/media/{}", m.id),
            mime_type: m.mime_type,
            file_size: m.file_size,
            width: m.width,
            height: m.height,
            alt: m.alt_text,
            created_at: m.created_at,
        }
    }
}

// ── OAuth State ─────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OAuthState {
    pub id: Uuid,
    pub state: String,
    pub provider: String,
    pub code_verifier: String,
    pub redirect_uri: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

// ── Notification ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub body: String,
    pub notification_type: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub is_read: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationPublic {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub notification_type: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub is_read: bool,
    pub created_at: String,
}

impl From<Notification> for NotificationPublic {
    fn from(n: Notification) -> Self {
        Self {
            id: n.id,
            title: n.title,
            body: n.body,
            notification_type: n.notification_type,
            reference_type: n.reference_type,
            reference_id: n.reference_id,
            is_read: n.is_read,
            created_at: n.created_at.to_rfc3339(),
        }
    }
}

// ── Rss Feed ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RssFeed {
    pub id: Uuid,
    pub user_id: Uuid,
    pub feed_url: String,
    pub integration_id: Uuid,
    pub title: String,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub poll_interval_min: i32,
    pub enabled: bool,
    pub use_ai_summary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RssPost {
    pub id: Uuid,
    pub feed_id: Uuid,
    pub post_id: Option<Uuid>,
    pub guid: String,
    pub title: String,
    pub url: String,
    pub published_at: Option<DateTime<Utc>>,
    pub content_hash: String,
    pub is_imported: bool,
    pub created_at: DateTime<Utc>,
}

// ── Analytics Cache ────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct AnalyticsCache {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub platform_post_id: Option<String>,
    pub data: serde_json::Value,
    pub cached_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

// ── External Posts ─────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ExternalPost {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub platform_post_id: String,
    pub text: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub author_avatar: Option<String>,
    /// The platform post's own timestamp, in INTEGER epoch seconds.
    ///
    /// The column is declared INTEGER, and every reader compares it
    /// against an INTEGER epoch cutoff. The writer used to bind a
    /// `DateTime<Utc>`, which sqlx stores as RFC 3339 TEXT - and SQLite
    /// orders every INTEGER before every TEXT, so `created_at >= ?` was
    /// ALWAYS TRUE and the 90-day window was a no-op. The `EpochUtc`
    /// seam reads and writes the column as the epoch it is declared to
    /// be.
    pub created_at: crate::db::types::EpochUtc,
    pub url: Option<String>,
    pub media: serde_json::Value,
    pub metadata: serde_json::Value,
    pub imported_at: crate::db::types::EpochUtc,
}

// ── Post Engagement Metrics ──────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PostEngagement {
    pub id: Uuid,
    pub post_id: Uuid,
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
    pub fetched_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// External post with its engagement metrics joined (for API responses).
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ExternalPostWithEngagement {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub platform_post_id: String,
    pub text: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub author_avatar: Option<String>,
    pub created_at: DateTime<Utc>,
    pub url: Option<String>,
    pub media: serde_json::Value,
    pub metadata: serde_json::Value,
    pub imported_at: DateTime<Utc>,
    // Engagement metrics (LEFT JOINed)
    pub engagement_likes: Option<i32>,
    pub engagement_comments: Option<i32>,
    pub engagement_shares: Option<i32>,
    pub engagement_views: Option<i32>,
    pub engagement_saves: Option<i32>,
    pub engagement_quotes: Option<i32>,
    pub engagement_reposts: Option<i32>,
    pub engagement_replies: Option<i32>,
    pub engagement_reactions: Option<serde_json::Value>,
    pub engagement_upvotes: Option<i32>,
    pub engagement_downvotes: Option<i32>,
    pub engagement_upvote_ratio: Option<f32>,
    pub engagement_awards: Option<i32>,
    pub engagement_raw: Option<serde_json::Value>,
    pub engagement_fetched_at: Option<DateTime<Utc>>,
}

// ── Signature ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Signature {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub content: String,
    pub provider: Option<String>,
    /// Phase v21/v22: if TRUE, this signature is auto-appended to new
    /// posts for its provider (or globally if provider is NULL). At most
    /// one default per (user_id, provider) — enforced by the partial
    /// unique index idx_signatures_default_per_provider.
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

use schemars::JsonSchema;

#[cfg(test)]
mod tests {
    use super::decode_posting_times;
    use serde_json::json;

    // Regression: TEXT JSON came back as a string, so the channels page
    // called `.map` on it and the slot finder silently ignored the times.
    #[test]
    fn stored_posting_times_text_decodes_to_an_array() {
        let stored = json!("[{\"time\":120},{\"time\":400}]");
        assert_eq!(
            decode_posting_times(stored),
            json!([{"time": 120}, {"time": 400}])
        );
        assert_eq!(decode_posting_times(json!("not json")), json!([]));
        assert_eq!(
            decode_posting_times(json!([{"time": 60}])),
            json!([{"time": 60}])
        );
    }
}
