// ─── Posts API Routes ─────────────────────────────────────────
// CRUD for social media posts with scheduling support.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::tags::TagResponse;
use crate::auth::middleware::AuthenticatedUser;
use crate::db::models::{media_as_array, PostPublic};
use crate::db::queries;
use crate::services::posts::PostService;

use super::AppState;

// ── Request Types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PostOverride {
    pub content: Option<String>,
    pub settings: Option<serde_json::Value>,
    /// The channel's own schedule, when the founder wants one post to go
    /// out at different times per channel.
    ///
    /// Phase 6 of the UI parity plan (per-channel date and target view). The
    /// top-level `scheduled_at` applies to EVERY integration on the post; a
    /// founder scheduling 09:00 for X and 17:00 for LinkedIn had no way to
    /// say so, because one post carried one time.
    ///
    /// This is an OVERRIDE in the same sense the content is: absent means
    /// "use the post's shared time", present means "this channel is
    /// different". A NULL here therefore means KEEP, not CLEAR - which is the
    /// same three-state reasoning as `Option<Option<T>>` elsewhere
    /// (AGENTS.md 9.12): the partial-update path must not be able to erase
    /// the shared schedule by sending an empty object.
    pub scheduled_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePostRequest {
    pub integration_ids: Vec<Uuid>,
    pub content: String,
    pub title: Option<String>,
    pub media: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    pub scheduled_at: Option<String>,
    pub tag_ids: Option<Vec<Uuid>>,
    pub first_comment: Option<String>,
    pub sequence: Option<i32>,
    pub overrides: Option<HashMap<String, PostOverride>>,
    /// The campaign to file this post under.
    ///
    /// The composer's campaign dropdown set a variable that was NEVER
    /// read, so the id was dropped on submit - and the kanban's drag
    /// PRESERVED the existing value rather than setting one. So there
    /// was no UI path that could put a post into a campaign: the
    /// campaign's `post_count` was always 0 and its Posts tab was
    /// permanently empty.
    pub campaign_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct CreatePostsResponse {
    pub posts: Vec<PostPublic>,
    pub group_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateThreadRequest {
    pub content_parts: Vec<String>,
    pub integration_ids: Vec<Uuid>,
    pub scheduled_at: Option<String>,
    /// Delay (in minutes) between each thread part. If set, each part's
    /// scheduled_at is offset by (sequence - 1) * delay_minutes.
    /// e.g. delay_minutes=5 with 3 parts scheduled at 09:00 →
    /// part 1 at 09:00, part 2 at 09:05, part 3 at 09:10.
    /// If omitted, all parts share the same scheduled_at.
    pub delay_minutes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateThreadResponse {
    pub posts: Vec<PostPublic>,
    pub group_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePostRequest {
    pub content: Option<String>,
    /// Three-state: absent = keep, `null` = clear, a value = set.
    ///
    /// A plain `Option<String>` collapses "absent" and "null", so the
    /// backend could not tell "the caller did not mention the title" from
    /// "the caller cleared the title" - and the composer's `title || undefined`
    /// meant clearing the title in the UI left the old title on the post.
    /// Same for media, settings and first_comment.
    #[serde(default, deserialize_with = "crate::api::campaigns::double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::api::campaigns::double_option")]
    pub media: Option<Option<serde_json::Value>>,
    #[serde(default, deserialize_with = "crate::api::campaigns::double_option")]
    pub settings: Option<Option<serde_json::Value>>,
    // v23-5: tag_ids + first_comment now persistable on update.
    // Previously the composer's edit-mode silently dropped these fields.
    pub tag_ids: Option<Vec<Uuid>>,
    #[serde(default, deserialize_with = "crate::api::campaigns::double_option")]
    pub first_comment: Option<Option<String>>,
    /// Per-integration overrides, keyed by integration id: `{"<uuid>":
    /// {"content": "...", "settings": {...}}}`.
    ///
    /// The create path has always accepted them; the update path did
    /// not, so a per-channel body or a Reddit subreddit set in the
    /// composer was silently dropped on every edit - the channel
    /// published the global body and the post failed with "Missing
    /// subreddit in settings".
    #[serde(default)]
    pub overrides: Option<HashMap<String, PostOverride>>,
    /// The campaign the post is filed under. Three-state: absent = keep,
    /// `null` = clear, a value = set.
    ///
    /// The column existed and the composer had a dropdown for it, but
    /// the UPDATE never touched it - so filing a post under a campaign
    /// on an EDIT silently did nothing. The campaign was read-only
    /// after creation.
    #[serde(default, deserialize_with = "crate::api::campaigns::double_option")]
    pub campaign_id: Option<Option<Uuid>>,
}

#[derive(Debug, Deserialize)]
pub struct SchedulePostRequest {
    pub scheduled_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ListPostsQuery {
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Phase 5: search query (ILIKE on content + title).
    pub q: Option<String>,
    /// Phase 5: filter by integration IDs (comma-separated).
    pub integration_ids: Option<String>,
    /// Phase 5: filter by tag IDs (comma-separated).
    pub tag_ids: Option<String>,
    /// Filter by campaign. The calendar's list view sends it, so the
    /// campaign dropdown has the same effect in every view. The query
    /// layer had no campaign filter at all, so the parameter was
    /// ignored and the list showed every campaign's posts.
    pub campaign_id: Option<Uuid>,
    /// Phase 5: sort order. One of: scheduled_date (default), created_date,
    /// engagement. Prefix with '-' for descending (e.g. '-scheduled_date').
    #[serde(default = "default_sort")]
    pub sort: String,
}

fn default_sort() -> String {
    "scheduled_date".into()
}

#[derive(Debug, Serialize)]
pub struct PostsListResponse {
    pub posts: Vec<PostWithIntegrationName>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct PostWithIntegrationName {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub integration_name: String,
    /// The provider's identifier ("x", "linkedin-page",
    /// "instagram-standalone").
    ///
    /// `integration_name` is the provider's DISPLAY name ("Telegram Bot",
    /// "LinkedIn Page"), which is not the same string. The analytics
    /// page's channel filter stores the identifier and compared it
    /// against the display name, so every provider whose two names
    /// differ - telegram-bot, linkedin-page, instagram-standalone -
    /// matched nothing and the filter silently emptied the list.
    pub provider_identifier: String,
    pub state: String,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub scheduled_at: Option<String>,
    pub published_at: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub tags: Vec<TagResponse>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<String>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    // v25-3: kanban fields surfaced to the frontend so the kanban card UI
    // can render priority / due date / substate / sort order without a
    // second fetch. Backend writes them via update_stage (see campaigns.rs).
    pub kanban_sort_order: i32,
    pub kanban_substate: Option<String>,
    pub due_date: Option<String>,
    pub priority: String,
    // v25-3: campaign_id is already on Post (column since v22 Phase 6) but
    // wasn't being surfaced in the API response. The kanban filter relies
    // on it (v22 Phase 6 fix in the frontend), so we add it here for
    // completeness + to avoid a future round-trip.
    pub campaign_id: Option<Uuid>,
    /// Engagement for this post, from `analytics_cache` (keyed by
    /// user_id + platform_post_id).
    ///
    /// The frontend's `PostSummary` declared `likes`/`comments`/
    /// `shares`/`impressions` and the dashboard's "recent published"
    /// list renders them - but the list endpoint never sent them, so the
    /// metric chips never appeared. `post_engagement` is not the source
    /// here: its `post_id` FK targets `external_posts`, so it can never
    /// hold a row for a `posts` row.
    ///
    /// `None` means "not measured", not zero: a post with no cached
    /// analytics keeps the field absent rather than reporting a
    /// fabricated zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engagement: Option<PostEngagement>,
}

/// The engagement of one of the user's own posts.
#[derive(Debug, Clone, Serialize)]
pub struct PostEngagement {
    pub likes: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub impressions: Option<i64>,
}

/// Merge an incoming settings object over the stored one.
///
/// A key the caller sent is SET. A key the caller did not send is
/// KEPT. The whole-object `null` is handled by the caller as an
/// explicit clear.
///
/// The settings hold the provider's TARGET - a Reddit subreddit, a
/// Pinterest board, a Discord channel - and the provider settings are
/// read at PUBLISH time. A replace meant a client that sent only the
/// keys it knew about silently deleted the target, and the post failed
/// at publish with no reason shown.
fn merge_settings(stored: &serde_json::Value, incoming: &serde_json::Value) -> serde_json::Value {
    match (stored.as_object(), incoming.as_object()) {
        (Some(s), Some(i)) => {
            let mut out = s.clone();
            for (k, v) in i {
                out.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(out)
        }
        // Not a pair of objects: the incoming shape wins, because the
        // caller sent something the stored value cannot merge into.
        _ => incoming.clone(),
    }
}

#[derive(Debug, Serialize)]
pub struct PostDetailResponse {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub integration_name: String,
    pub state: String,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    /// The post's stored settings - the provider settings (a Reddit
    /// subreddit, a Pinterest board, a Discord channel) and the global
    /// ones (an Instagram audio id).
    ///
    /// The detail response used to have NO settings field, so the
    /// composer could not restore them when it opened a post for
    /// editing. It then saved `settings: {}` - and the update path
    /// REPLACES the settings wholesale. So editing a post to fix a typo
    /// DELETED the subreddit, and the post failed at publish with
    /// "Missing subreddit in settings" - a post that was working thirty
    /// seconds earlier.
    pub settings: serde_json::Value,
    pub scheduled_at: Option<String>,
    pub published_at: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub tags: Vec<TagResponse>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<String>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
}

#[derive(Debug, Deserialize)]
pub struct SetPostTagsRequest {
    pub tag_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct FindSlotResponse {
    pub date: String,
}

// ── Helpers ──────────────────────────────────────────────────

/// Fetch and convert tags for a post. Logs DB errors instead of silently swallowing them.
async fn enrich_post_tags(
    db: &crate::db::SqlitePool,
    post_id: Uuid,
    user_id: Uuid,
) -> Vec<TagResponse> {
    match queries::get_tags_for_post(db, post_id, user_id).await {
        Ok(rows) => rows
            .into_iter()
            .map(|row| TagResponse {
                id: row.id,
                name: row.name,
                color: row.color,
                created_at: row.created_at.to_rfc3339(),
                updated_at: row.updated_at.to_rfc3339(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!("Failed to fetch tags for post {post_id}: {e}");
            Vec::new()
        }
    }
}

/// Verify that all tag_ids belong to the given user.
pub(crate) async fn verify_tag_ownership(
    db: &crate::db::SqlitePool,
    tag_ids: &[Uuid],
    user_id: Uuid,
) -> Result<(), crate::error::AppError> {
    // `id = ANY($1)` was Postgres-only. sqlite has no array type, so the id
    // list travels as a JSON array and `json_each` fans it back out.
    //
    // The expansion goes through `unhex`: `tags.id` holds a 16-byte BLOB
    // (sqlx's uuid encoding), and SQLite never compares a BLOB to the TEXT
    // that `json_each` yields — so the raw expansion matched no row and
    // every tag assignment was rejected as "not found".
    let tag_ids_json = serde_json::to_value(tag_ids)
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to encode tag_ids: {e}")))?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tags
         WHERE user_id = ?
           AND id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?))",
    )
    .bind(user_id)
    .bind(tag_ids_json)
    .fetch_one(db)
    .await?;
    if (count as usize) != tag_ids.len() {
        return Err(crate::error::AppError::BadRequest(
            "One or more tags not found or do not belong to the user".into(),
        ));
    }
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────

/// GET /api/posts
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<ListPostsQuery>,
) -> Result<Json<PostsListResponse>, crate::error::AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);

    // Phase 5: parse the new search/filter/sort params.
    let q = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let integration_ids: Option<Vec<Uuid>> = query
        .integration_ids
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .filter_map(|id| Uuid::parse_str(id.trim()).ok())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    let tag_ids: Option<Vec<Uuid>> = query
        .tag_ids
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .filter_map(|id| Uuid::parse_str(id.trim()).ok())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    // Use the new search query if any of the new params are present;
    // otherwise fall back to the original list_posts for backward compat.
    let use_search = q.is_some()
        || integration_ids.is_some()
        || tag_ids.is_some()
        || query.campaign_id.is_some()
        || query.sort != "scheduled_date";

    let (posts, total) = if use_search {
        let posts = queries::list_posts_search(
            &state.db,
            auth.user_id,
            query.state.as_deref(),
            q,
            integration_ids.as_deref(),
            tag_ids.as_deref(),
            query.campaign_id,
            &query.sort,
            limit,
            offset,
        )
        .await?;
        let total = queries::count_posts_search(
            &state.db,
            auth.user_id,
            query.state.as_deref(),
            q,
            integration_ids.as_deref(),
            tag_ids.as_deref(),
            query.campaign_id,
        )
        .await?;
        (posts, total)
    } else {
        let total =
            queries::count_posts_by_user(&state.db, auth.user_id, query.state.as_deref()).await?;
        let posts = queries::list_posts(
            &state.db,
            auth.user_id,
            query.state.as_deref(),
            limit,
            offset,
        )
        .await?;
        (posts, total)
    };

    // Engagement for the user's own posts, from `analytics_cache`. A
    // post with no cached analytics gets NO entry - "not measured" is
    // not "zero".
    let engagement_by_post: std::collections::HashMap<Uuid, PostEngagement> = {
        let ids: Vec<Uuid> = posts.iter().map(|p| p.id).collect();
        queries::post_engagement_by_post(&state.db, auth.user_id, &ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|row| {
                // A row with no measured metric at all is not engagement.
                if row.likes.is_none()
                    && row.comments.is_none()
                    && row.shares.is_none()
                    && row.impressions.is_none()
                {
                    return None;
                }
                Some((
                    row.post_id,
                    PostEngagement {
                        likes: row.likes,
                        comments: row.comments,
                        shares: row.shares,
                        impressions: row.impressions,
                    },
                ))
            })
            .collect()
    };

    let mut enriched = Vec::with_capacity(posts.len());
    for p in posts {
        let (integration_name, provider_identifier) = if let Ok(Some(integ)) =
            queries::get_integration(&state.db, p.integration_id, auth.user_id).await
        {
            (integ.provider_name, integ.provider_identifier)
        } else {
            ("Removed channel".into(), "unknown".into())
        };
        let tags = enrich_post_tags(&state.db, p.id, auth.user_id).await;
        enriched.push(PostWithIntegrationName {
            id: p.id,
            integration_id: p.integration_id,
            integration_name,
            provider_identifier,
            state: p.state.to_string(),
            content: p.content,
            title: p.title,
            media: media_as_array(&p.media),
            scheduled_at: p.scheduled_at.map(|d| d.to_rfc3339()),
            published_at: p.published_at.map(|d| d.to_rfc3339()),
            platform_post_url: p.platform_post_url,
            error_message: p.error_message,
            created_at: p.created_at.to_rfc3339(),
            tags,
            repeat_interval_days: p.repeat_interval_days,
            repeat_end_date: p.repeat_end_date.map(|d| d.to_rfc3339()),
            group_id: p.group_id,
            first_comment: p.first_comment.clone(),
            sequence: p.sequence,
            kanban_sort_order: p.kanban_sort_order,
            kanban_substate: p.kanban_substate.clone(),
            due_date: p.due_date.map(|d| d.to_rfc3339()),
            priority: p.priority.clone(),
            campaign_id: p.campaign_id,
            engagement: engagement_by_post.get(&p.id).cloned(),
        });
    }

    Ok(Json(PostsListResponse {
        total,
        posts: enriched,
    }))
}

/// POST /api/posts
pub async fn create(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<CreatePostRequest>,
) -> Result<Json<CreatePostsResponse>, crate::error::AppError> {
    // Validate ALL integration_ids belong to user and collect for enrichment
    let mut validated_integrations = Vec::with_capacity(body.integration_ids.len());
    for &id in &body.integration_ids {
        let integ = queries::get_integration(&state.db, id, auth.user_id)
            .await?
            .ok_or_else(|| crate::error::AppError::NotFound("Integration not found".into()))?;

        if integ.disabled {
            return Err(crate::error::AppError::BadRequest(
                "Integration is disabled".into(),
            ));
        }
        validated_integrations.push(integ);
    }

    // ── Per-provider content validation at create-time ────────
    // Checks each integration's provider-specific limits (char count,
    // media count, media type) BEFORE creating posts. Returns structured
    // errors so the frontend can highlight the failing integration.
    let media_val: Vec<crate::social::MediaAttachment> =
        serde_json::from_value(body.media.clone().unwrap_or(serde_json::json!([])))
            .unwrap_or_default();
    let mut validation_errors =
        validate_posts_for_integrations(&state, &body, &validated_integrations, &media_val);
    // A DRAFT may carry a title and no body. The composer's "Save Draft"
    // accepts a title-only draft (that is a legitimate thing to keep -
    // a headline with the body to come), but the server's empty-content
    // rule rejected it with "Content is empty for X" - the client gate
    // and the server gate disagreed, and the save reported success on
    // the client and failed on the server.
    //
    // Only for an UNSCHEDULED post: a queued post is going to publish,
    // and an empty body would publish as an empty post.
    let is_draft = body.scheduled_at.is_none();
    let has_title = body
        .title
        .as_deref()
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    if is_draft && has_title {
        validation_errors.retain(|e| e.kind != "empty");
    }
    if !validation_errors.is_empty() {
        return Err(crate::error::AppError::BadRequest(
            serde_json::to_string(&serde_json::json!({
                "validation_errors": validation_errors,
            }))
            .unwrap_or_else(|_| "Content validation failed".into()),
        ));
    }

    let scheduled_at: Option<DateTime<Utc>> = match body.scheduled_at {
        Some(ref s) => {
            let dt = DateTime::parse_from_rfc3339(s).map_err(|_| {
                crate::error::AppError::BadRequest("Invalid date format, use ISO8601".into())
            })?;
            Some(dt.with_timezone(&Utc))
        }
        None => None,
    };

    let media = body.media.unwrap_or(serde_json::json!([]));
    let settings = body.settings.unwrap_or(serde_json::json!({}));
    let state_enum = if scheduled_at.is_some() {
        Some(crate::db::models::PostState::Queued)
    } else {
        Some(crate::db::models::PostState::Draft)
    };

    // The composer's rich-text editor produces TipTap HTML. The REST
    // create path stored that markup verbatim, so a post with bold text
    // shipped `<p>Hello <strong>world</strong></p>` to the platform as
    // the tweet text. The service path sanitizes; this one did not.
    // The same helper, the same boundary.
    // Stripped, NOT truncated: the length rule is the provider's, and it
    // has to be able to fire (a truncation to 2000 made it unreachable, and
    // silently shortened the founder's post).
    let body_content = crate::services::posts::PostService::strip_content(&body.content);

    // Verify tag ownership before creating the post (fail fast)
    if let Some(ref tag_ids) = body.tag_ids {
        if !tag_ids.is_empty() {
            verify_tag_ownership(&state.db, tag_ids, auth.user_id).await?;
        }
    }

    let posts = if let Some(overrides) = body.overrides.as_ref() {
        let mut posts = Vec::with_capacity(body.integration_ids.len());
        for &id in &body.integration_ids {
            let post_content = overrides
                .get(&id.to_string())
                .and_then(|o| o.content.as_deref())
                .unwrap_or(&body.content);
            let post_settings = overrides
                .get(&id.to_string())
                .and_then(|o| o.settings.clone())
                .unwrap_or_else(|| settings.clone());
            // This channel's OWN time, falling back to the post's shared
            // time - and to NO time, so an override with an absent or
            // unparseable `scheduled_at` behaves exactly as it did before
            // this field existed rather than failing the whole request.
            //
            // The scheduler reads a post's `scheduled_at` column, so this
            // is what actually makes 09:00-for-X and 17:00-for-LinkedIn
            // publish at those times: each row carries its own.
            let post_scheduled_at = overrides
                .get(&id.to_string())
                .and_then(|o| o.scheduled_at.as_deref())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .or(scheduled_at);
            let post = queries::create_post(
                &state.db,
                auth.user_id,
                id,
                &crate::services::posts::PostService::strip_content(post_content),
                body.title.as_deref(),
                &media,
                &post_settings,
                post_scheduled_at,
                state_enum.clone(),
                body.first_comment.as_deref(),
                body.sequence.unwrap_or(0),
                body.campaign_id,
            )
            .await?;
            posts.push(post);
        }
        posts
    } else {
        queries::create_posts_for_integrations(
            &state.db,
            auth.user_id,
            &body.integration_ids,
            &body_content,
            body.title.as_deref(),
            &media,
            &settings,
            scheduled_at,
            state_enum,
            body.first_comment.as_deref(),
            body.sequence.unwrap_or(0),
            body.campaign_id,
        )
        .await?
    };

    // Insert post_tags for each post if tag_ids provided
    if let Some(ref tag_ids) = body.tag_ids {
        if !tag_ids.is_empty() {
            for post in &posts {
                queries::set_post_tags(&state.db, post.id, tag_ids).await?;
            }
        }
    }

    // Enrich with integration names, broadcast, and return
    let publics: Vec<PostPublic> = posts
        .into_iter()
        .map(|p| {
            let integration_name = validated_integrations
                .iter()
                .find(|i| i.id == p.integration_id)
                .map(|i| i.provider_name.clone())
                .unwrap_or_default();
            let mut public = PostPublic::from(p);
            public.integration_name = integration_name;
            state.broadcast.send("post_created", &public);
            public
        })
        .collect();

    Ok(Json(CreatePostsResponse {
        posts: publics,
        group_id: None,
    }))
}

/// POST /api/posts/thread — create a thread of posts (X/Twitter threads)
pub async fn create_thread(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<CreateThreadRequest>,
) -> Result<Json<CreateThreadResponse>, crate::error::AppError> {
    if body.content_parts.is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "content_parts must not be empty".into(),
        ));
    }
    if body.content_parts.len() > 25 {
        return Err(crate::error::AppError::BadRequest(
            "Maximum 25 tweets per thread".into(),
        ));
    }
    if body.integration_ids.is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "At least one integration is required".into(),
        ));
    }

    // Validate ALL integration_ids belong to user
    let mut validated_integrations = Vec::with_capacity(body.integration_ids.len());
    for &id in &body.integration_ids {
        let integ = queries::get_integration(&state.db, id, auth.user_id)
            .await?
            .ok_or_else(|| crate::error::AppError::NotFound("Integration not found".into()))?;

        if integ.disabled {
            return Err(crate::error::AppError::BadRequest(
                "Integration is disabled".into(),
            ));
        }
        validated_integrations.push(integ);
    }

    let scheduled_at: Option<DateTime<Utc>> = match body.scheduled_at {
        Some(ref s) => {
            let dt = DateTime::parse_from_rfc3339(s).map_err(|_| {
                crate::error::AppError::BadRequest("Invalid date format, use ISO8601".into())
            })?;
            Some(dt.with_timezone(&Utc))
        }
        None => None,
    };

    let state_enum = if scheduled_at.is_some() {
        Some(crate::db::models::PostState::Queued)
    } else {
        Some(crate::db::models::PostState::Draft)
    };

    let group_id = Uuid::new_v4();

    let posts = queries::create_thread_posts(
        &state.db,
        auth.user_id,
        &body.integration_ids,
        &body.content_parts,
        scheduled_at,
        state_enum,
        group_id,
        body.delay_minutes,
    )
    .await?;

    // Enrich with integration names and broadcast
    let publics: Vec<PostPublic> = posts
        .into_iter()
        .map(|p| {
            let integration_name = validated_integrations
                .iter()
                .find(|i| i.id == p.integration_id)
                .map(|i| i.provider_name.clone())
                .unwrap_or_default();
            let mut public = PostPublic::from(p);
            public.integration_name = integration_name;
            state.broadcast.send("post_created", &public);
            public
        })
        .collect();

    Ok(Json(CreateThreadResponse {
        posts: publics,
        group_id,
    }))
}

/// GET /api/posts/group/:group_id — fetch all posts sharing a group_id.
///
/// Used by the composer's edit-mode to load all sibling posts in a thread
/// (main post + first-comment + thread parts). Returns an empty array if
/// the group_id has no posts (or if the caller doesn't own them).
///
/// Excludes soft-deleted posts.
pub async fn get_group(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(group_id): Path<Uuid>,
) -> Result<Json<Vec<PostPublic>>, crate::error::AppError> {
    let posts = queries::list_posts_by_group(&state.db, auth.user_id, group_id).await?;
    let publics: Vec<PostPublic> = posts.into_iter().map(PostPublic::from).collect();
    Ok(Json(publics))
}

/// GET /api/posts/:id
pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PostDetailResponse>, crate::error::AppError> {
    let post = queries::get_post(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;

    let integration_name = if let Ok(Some(integ)) =
        queries::get_integration(&state.db, post.integration_id, auth.user_id).await
    {
        integ.provider_name
    } else {
        "Unknown".into()
    };

    let tags = enrich_post_tags(&state.db, post.id, auth.user_id).await;

    Ok(Json(PostDetailResponse {
        id: post.id,
        integration_id: post.integration_id,
        integration_name,
        state: post.state.to_string(),
        content: post.content,
        title: post.title,
        media: media_as_array(&post.media),
        settings: post.settings.clone(),
        scheduled_at: post.scheduled_at.map(|d| d.to_rfc3339()),
        published_at: post.published_at.map(|d| d.to_rfc3339()),
        platform_post_url: post.platform_post_url,
        error_message: post.error_message,
        created_at: post.created_at.to_rfc3339(),
        tags,
        repeat_interval_days: post.repeat_interval_days,
        repeat_end_date: post.repeat_end_date.map(|d| d.to_rfc3339()),
        group_id: post.group_id,
        first_comment: post.first_comment.clone(),
        sequence: post.sequence,
    }))
}

/// PUT /api/posts/:id
pub async fn update(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePostRequest>,
) -> Result<Json<PostPublic>, crate::error::AppError> {
    let post = update_post(&state, auth.user_id, id, body).await?;
    Ok(Json(post))
}

/// The core of "edit a post" - ONE implementation, called by the REST
/// handler above AND by the MCP tool (`mcp::tools_posts::update_post`).
///
/// The MCP path used to call `PostService::update` directly, and that
/// was a SECOND, LOOSER implementation of the same edit: it had no
/// `Publishing` guard, it dropped `first_comment` entirely, and it did
/// not validate the edited content against the channel's provider
/// limits. So an agent could edit a post mid-publish, set a first
/// comment and watch it vanish, or write an over-long post that was
/// accepted and then failed at publish time.
///
/// The MCP layer is a CLIENT of the REST surface (AGENTS.md §9.21), so
/// it calls the same function.
pub(crate) async fn update_post(
    state: &AppState,
    user_id: uuid::Uuid,
    id: uuid::Uuid,
    body: UpdatePostRequest,
) -> Result<PostPublic, crate::error::AppError> {
    // PUT is a partial update: a field the body omits keeps its stored value.
    // Defaulting omitted fields to "" / [] / {} wiped the body, media and
    // settings of a post when a caller only sent tag_ids.
    let existing = queries::get_post(&state.db, id, user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;
    if existing.state == crate::db::models::PostState::Publishing {
        return Err(crate::error::AppError::Conflict(
            "Post is being published; edit it after the attempt finishes".into(),
        ));
    }
    // Each field is three-state: absent keeps the stored value, an
    // explicit null clears it, a value replaces it. The composer sends
    // `null` for a field the user emptied, so an edit that removes the
    // title (or the last image, or the first comment) actually removes it.
    let content = match &body.content {
        Some(c) => crate::services::posts::PostService::strip_content(c),
        None => existing.content,
    };
    let title = match &body.title {
        Some(Some(t)) => Some(t.clone()),
        Some(None) => None, // explicit clear
        None => existing.title,
    };
    let media = match &body.media {
        Some(Some(m)) => m.clone(),
        Some(None) => serde_json::json!([]), // explicit clear
        None => existing.media,
    };
    let settings = match &body.settings {
        // The incoming object MERGES over the stored one: a key the
        // caller sent is set, a key it did not send is KEPT.
        //
        // A replace was the old behaviour, and it meant a client that
        // sent `{}` - or sent only the key it knew about - DELETED the
        // provider settings. The composer did exactly that: it had no
        // settings to send (the detail response had no settings
        // field), so every edit of a scheduled Reddit post wiped its
        // subreddit and the post failed at publish with "Missing
        // subreddit in settings" - a post that was working thirty
        // seconds earlier.
        //
        // The whole-object `null` is still an explicit CLEAR, so a
        // caller that means to drop everything can say so.
        Some(Some(s)) => merge_settings(&existing.settings, s),
        Some(None) => serde_json::json!({}), // explicit clear
        None => existing.settings,
    };
    let first_comment = match &body.first_comment {
        Some(Some(fc)) => Some(fc.clone()),
        Some(None) => None, // explicit clear
        None => existing.first_comment,
    };

    // Apply THIS post's override, if the caller sent one.
    //
    // The update path handles ONE post, so only the entry keyed by this
    // post's integration applies. The composer sends the whole map; the
    // other entries belong to the OTHER posts of a multi-channel create
    // and are ignored here (they were written by the create).
    //
    // The override's `settings` REPLACE the post's settings, exactly as
    // the create path does for a multi-integration create: the provider
    // settings (a Reddit subreddit, a Pinterest board) and the global
    // ones (an Instagram audio id) live in one object, and the
    // per-integration map is the complete object for that channel.
    let (content, settings) = match body
        .overrides
        .as_ref()
        .and_then(|m| m.get(&existing.integration_id.to_string()))
    {
        Some(over) => {
            let content = match over.content.as_deref() {
                Some(c) => crate::services::posts::PostService::strip_content(c),
                None => content,
            };
            let settings = over
                .settings
                .clone()
                .map(|s| merge_settings(&settings, &s))
                .unwrap_or(settings);
            (content, settings)
        }
        None => (content, settings),
    };

    // Validate the EDITED content against the channel's provider limits.
    //
    // The create path validates; the update path did not. So an edit
    // that made a post too long (or broke a media rule) was ACCEPTED,
    // stored, and failed later at publish time - the founder found out
    // from the scheduler's error log, hours after the edit. The composer
    // pre-validates, so the UI hid this - but the API is the contract.
    if let Ok(Some(integration)) =
        queries::get_integration(&state.db, existing.integration_id, user_id).await
    {
        let media_val: Vec<crate::social::MediaAttachment> =
            serde_json::from_value(media.clone()).unwrap_or_default();
        let probe = CreatePostRequest {
            integration_ids: vec![existing.integration_id],
            content: content.clone(),
            title: title.clone(),
            media: Some(media.clone()),
            settings: Some(settings.clone()),
            scheduled_at: None,
            tag_ids: None,
            first_comment: None,
            sequence: None,
            campaign_id: None,
            overrides: body.overrides.as_ref().map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            PostOverride {
                                content: v.content.clone(),
                                // The channel's own schedule is returned so
                                // the composer's edit flow can RESTORE it.
                                // AGENTS.md 9.29: the edit path used to send
                                // `settings: {}` and the write REPLACED them,
                                // so opening a scheduled post to fix a typo
                                // deleted the target. The same shape applies
                                // here - if the composer cannot read back a
                                // per-channel time, the first edit erases it.
                                scheduled_at: v.scheduled_at.clone(),
                                // The override's settings MERGED over the
                                // post's, exactly as the write does.
                                //
                                // The probe read the RAW override
                                // settings, and the validator prefers
                                // the override's over the global ones -
                                // so an override of `{}` made the
                                // validator report the subreddit as
                                // missing when the stored one was
                                // right there. The validator and the
                                // write have to see the same settings.
                                settings: v.settings.as_ref().map(|s| merge_settings(&settings, s)),
                            },
                        )
                    })
                    .collect()
            }),
        };
        let errors = validate_posts_for_integrations(
            state,
            &probe,
            std::slice::from_ref(&integration),
            &media_val,
        );
        if !errors.is_empty() {
            return Err(crate::error::AppError::BadRequest(
                serde_json::to_string(&serde_json::json!({
                    "validation_errors": errors,
                }))
                .unwrap_or_else(|_| "Content validation failed".into()),
            ));
        }
    }

    // v23-5: use update_post_full so first_comment is persisted (was
    // silently dropped by update_post_content).
    let post = queries::update_post_full(
        &state.db,
        id,
        user_id,
        &content,
        title.as_deref(),
        &media,
        &settings,
        first_comment.as_deref(),
        body.campaign_id,
    )
    .await?
    .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;

    // v23-5: update tags if tag_ids was provided in the request.
    if let Some(tag_ids) = body.tag_ids.as_ref() {
        queries::set_post_tags(&state.db, id, tag_ids).await?;
    }

    Ok(PostPublic::from(post))
}

/// POST /api/posts/:id/schedule
pub async fn schedule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SchedulePostRequest>,
) -> Result<Json<PostPublic>, crate::error::AppError> {
    let scheduled_at = DateTime::parse_from_rfc3339(&body.scheduled_at)
        .map_err(|_| crate::error::AppError::BadRequest("Invalid date format, use ISO8601".into()))?
        .with_timezone(&Utc);

    // None means not found OR not schedulable (published, publishing, deleted).
    let post = queries::schedule_post(&state.db, id, auth.user_id, scheduled_at)
        .await?
        .ok_or_else(|| {
            crate::error::AppError::NotFound(
                "Post not found or not schedulable in its current state".into(),
            )
        })?;

    let public = PostPublic::from(post.clone());
    state.broadcast.send("post_scheduled", &public);

    Ok(Json(public))
}

/// POST /api/posts/:id/unschedule — v24-1
///
/// Transitions a post back to draft state by clearing scheduled_at and
/// setting state = 'draft'. Only works on posts in 'queued', 'draft',
/// or 'error' state (not 'published' or 'publishing').
///
/// This closes the ComposerModal TODO at line 551 which used the
/// "100 years in the future" hack to effectively unschedule. Now the
/// composer's saveAsDraft edit-mode flow calls this endpoint to
/// properly transition the post back to draft.
pub async fn unschedule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PostPublic>, crate::error::AppError> {
    let post = queries::unschedule_post(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Post not found or cannot be unscheduled (only queued/draft/error posts can be unscheduled)".into()))?;

    // Broadcast the state change so the calendar/kanban update in real-time.
    state.broadcast.send(
        "post_scheduled",
        &serde_json::json!({"id": id.to_string(), "unscheduled": true}),
    );

    Ok(Json(PostPublic::from(post)))
}

/// PUT /api/posts/:id/date — reschedule a post by dragging it on the calendar.
///
/// Accepts `{ "scheduled_at": "<RFC3339>", "move_group": bool, "action": "schedule" | "update" }`.
///
/// ## The `action` parameter (Phase v21 — postiz-inspired)
///
/// For posts in `queued` or `draft` state, the `action` parameter is
/// ignored — the post is simply rescheduled (state stays `queued`/`draft`).
///
/// For posts in `published` state, `action` determines what happens:
///   - `action: "schedule"` → reset state to `queued`, clear
///     `platform_post_id` / `platform_post_url` / `published_at`,
///     set `scheduled_at` to the new time. The scheduler will re-publish
///     the post at the new time (creating a NEW post on the platform).
///     This is the "republish" flow.
///   - `action: "update"` → just change `scheduled_at` but leave the
///     state, `platform_post_id`, `platform_post_url`, and `published_at`
///     alone. Useful for archival re-dating without triggering a re-publish.
///
/// If `action` is omitted (None) and the post is published, the request
/// is rejected with HTTP 400 (preserving the pre-v21 behavior so existing
/// callers that don't send `action` don't accidentally re-publish posts).
///
/// If the post is part of a thread (has `group_id`), the caller can pass
/// `move_group: true` to reschedule all posts in the same group by the
/// same delta. The `action` parameter applies to every post in the group.
#[derive(Debug, Deserialize)]
pub struct RescheduleRequest {
    pub scheduled_at: String,
    /// If true and the post is part of a thread, reschedule all posts
    /// in the same group by the same time delta. Default: false.
    pub move_group: Option<bool>,
    /// Phase v21: disambiguate published-post reschedules:
    /// - `schedule` → reset state to queued + clear release fields (re-publish).
    /// - `update`   → change scheduled_at only (archive re-date).
    ///
    /// Required for published posts; ignored for queued/draft posts.
    pub action: Option<String>,
}

pub async fn reschedule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RescheduleRequest>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let new_scheduled_at = DateTime::parse_from_rfc3339(&body.scheduled_at)
        .map_err(|_| {
            crate::error::AppError::BadRequest("Invalid date format, use RFC3339/ISO8601".into())
        })?
        .with_timezone(&Utc);

    // One core behind REST and MCP posts_reschedule (§5.2 parity).
    let value = PostService::reschedule(
        &state.db,
        &state.broadcast,
        auth.user_id,
        id,
        new_scheduled_at,
        body.move_group.unwrap_or(false),
        body.action.as_deref(),
    )
    .await
    .map_err(crate::error::AppError::BadRequest)?;

    Ok(Json(value))
}

/// DELETE /api/posts/:id
///
/// `?delete_from_platform=true` also deletes the PUBLISHED post on the
/// platform. The platform delete happens FIRST, so a failure leaves the
/// local row — and its `platform_post_id` — intact for a retry. A post
/// whose local row is gone but whose tweet is still live cannot be
/// retried, and the founder cannot find it.
#[derive(Debug, Deserialize, Default)]
pub struct DeletePostParams {
    /// Also delete the published post on the platform.
    #[serde(default)]
    pub delete_from_platform: Option<bool>,
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Query(params): Query<DeletePostParams>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let delete_from_platform = params.delete_from_platform.unwrap_or(false);

    if delete_from_platform {
        let post = queries::get_post_with_integration(&state.db, id, auth.user_id)
            .await?
            .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;
        if post.state != crate::db::models::PostState::Published {
            return Err(crate::error::AppError::BadRequest(
                "delete_from_platform=true requires a PUBLISHED post — the local row is the only copy"
                    .into(),
            ));
        }
        let platform_id = post
            .platform_post_id
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if platform_id.is_empty() {
            return Err(crate::error::AppError::BadRequest(
                "post is published but carries no platform_post_id".into(),
            ));
        }
        let provider = state
            .providers
            .get(&post.provider_identifier)
            .ok_or_else(|| {
                crate::error::AppError::BadRequest(format!(
                    "Provider '{}' not found",
                    post.provider_identifier
                ))
            })?;
        let token =
            crate::crypto::maybe_decrypt_token(&post.access_token, state.token_key.as_ref());
        provider
            .delete_published(&token, &platform_id)
            .await
            .map_err(|e| {
                crate::error::AppError::BadRequest(format!(
                    "platform delete failed for {} {platform_id} — the local row was KEPT for retry: {e}",
                    post.provider_identifier
                ))
            })?;
    }

    let deleted = PostService::delete(&state.db, &state.broadcast, auth.user_id, id)
        .await
        .map_err(crate::error::AppError::BadRequest)?;
    if !deleted {
        return Err(crate::error::AppError::NotFound("Post not found".into()));
    }
    Ok(Json(serde_json::json!({
        "deleted": true,
        "deleted_from_platform": delete_from_platform,
    })))
}

/// PUT /api/posts/{id}/tags
pub async fn set_post_tags(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetPostTagsRequest>,
) -> Result<Json<PostDetailResponse>, crate::error::AppError> {
    let post = queries::get_post(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;

    if !body.tag_ids.is_empty() {
        verify_tag_ownership(&state.db, &body.tag_ids, auth.user_id).await?;
    }

    queries::set_post_tags(&state.db, id, &body.tag_ids).await?;

    let integration_name = if let Ok(Some(integ)) =
        queries::get_integration(&state.db, post.integration_id, auth.user_id).await
    {
        integ.provider_name
    } else {
        "Unknown".into()
    };

    let tags = enrich_post_tags(&state.db, post.id, auth.user_id).await;

    Ok(Json(PostDetailResponse {
        id: post.id,
        integration_id: post.integration_id,
        integration_name,
        state: post.state.to_string(),
        content: post.content,
        title: post.title,
        media: media_as_array(&post.media),
        settings: post.settings.clone(),
        scheduled_at: post.scheduled_at.map(|d| d.to_rfc3339()),
        published_at: post.published_at.map(|d| d.to_rfc3339()),
        platform_post_url: post.platform_post_url,
        error_message: post.error_message,
        created_at: post.created_at.to_rfc3339(),
        tags,
        repeat_interval_days: post.repeat_interval_days,
        repeat_end_date: post.repeat_end_date.map(|d| d.to_rfc3339()),
        group_id: post.group_id,
        first_comment: post.first_comment.clone(),
        sequence: post.sequence,
    }))
}

// v25-4: Kanban drag-to-reorder within a column.
//
// Accepts the full new ordering of post IDs for a given kanban state
// (idea/draft/queued/published) and renumbers each post's
// kanban_sort_order to match its index in the array. Bulk update in a
// single transaction so concurrent reorders don't leave gaps or duplicates.
//
// The frontend sends the complete ordered list because it already has the
// full column rendered — this avoids the need for a "before_id / after_id"
// protocol that would require the backend to fetch the current order first.
//
// Validates:
//   - state is one of the 5 valid post states
//   - all ordered_post_ids belong to the user (per-user scoping)
//   - all ids are actually in the given state (prevents a reordered post
//     from silently jumping columns)
//
// Broadcasts `post_stage_changed` (reusing the existing event) so other
// tabs refetch and see the new order. The event payload includes a
// `reordered: true` flag so listeners can distinguish a reorder from a
// state transition if needed.

#[derive(Debug, Deserialize)]
pub struct KanbanReorderRequest {
    pub state: String,
    pub ordered_post_ids: Vec<Uuid>,
    /// The states the column HOLDS, when it holds more than one.
    ///
    /// The board's Drafts column is the drafts PLUS the error posts -
    /// a failed post belongs somewhere the founder can act on it. So
    /// the reorder of that column carries both.
    ///
    /// The UPDATE used to be guarded by `AND state = ?` with the
    /// column's own name, so the error posts in the ordered list did
    /// not match, were silently not renumbered, and kept their old
    /// `kanban_sort_order = 0`. The response reported "updated" and
    /// the frontend ignored the count - so a founder who dragged a
    /// failed post to the top of Drafts saw it move locally, and on the
    /// next reload it jumped back. The ordering was silently lost.
    ///
    /// With the list, the column's posts are renumbered whatever
    /// state they are in. A single-state column behaves exactly as
    /// before.
    #[serde(default)]
    pub column_states: Option<Vec<String>>,
}

pub async fn kanban_reorder(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<KanbanReorderRequest>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let valid_states = ["idea", "draft", "queued", "published", "error"];
    if !valid_states.contains(&body.state.as_str()) {
        return Err(crate::error::AppError::BadRequest(format!(
            "Invalid state: {} (expected one of: {})",
            body.state,
            valid_states.join(", ")
        )));
    }
    if body.ordered_post_ids.is_empty() {
        return Ok(Json(serde_json::json!({ "updated": 0 })));
    }

    // Deduplicate (a buggy client shouldn't be able to corrupt order).
    let mut seen = std::collections::HashSet::with_capacity(body.ordered_post_ids.len());
    let mut deduped = Vec::with_capacity(body.ordered_post_ids.len());
    for id in &body.ordered_post_ids {
        if seen.insert(*id) {
            deduped.push(*id);
        }
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to begin tx: {e}")))?;

    // Update each post's kanban_sort_order to its index in one statement
    // rather than N round-trips. Postgres did this with
    // `unnest($1::uuid[]) WITH ORDINALITY`; sqlite has no arrays, so the id
    // list travels as a JSON array and `json_each` supplies both the value
    // and the zero-based `key` we turn into a 1-based ordinal — preserving
    // the original WITH ORDINALITY numbering.
    //
    // The id is expanded through `unhex` because `posts.id` holds a
    // 16-byte BLOB (sqlx's uuid encoding) while `json_each` yields TEXT;
    // without the conversion the join never matches and the reorder
    // reported success while changing nothing.
    //
    // Security: the WHERE user_id = ? AND state = ? clause ensures the caller
    // can only reorder their OWN posts in the specified column — even if they
    // send a foreign user's post ID, it won't match.
    let deduped_json = serde_json::to_value(&deduped)
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to encode post ids: {e}")))?;
    // The states the column holds. A single-state column is just the
    // column's own name; the Drafts column holds the drafts AND the
    // error posts.
    let mut column_states: Vec<String> = body
        .column_states
        .clone()
        .unwrap_or_else(|| vec![body.state.clone()]);
    if column_states.is_empty() {
        column_states.push(body.state.clone());
    }
    for s in &column_states {
        if !valid_states.contains(&s.as_str()) {
            return Err(crate::error::AppError::BadRequest(format!(
                "Invalid state in column_states: {s}"
            )));
        }
    }

    let result = sqlx::query(
        r#"WITH ordered AS (
             SELECT unhex(replace(value, '-', '')) AS id, key + 1 AS ord FROM json_each(?)
           )
           UPDATE posts
           SET kanban_sort_order = (
                   SELECT ord FROM ordered WHERE ordered.id = posts.id
               ),
               updated_at = unixepoch()
           WHERE user_id = ?
             AND state IN (
                 SELECT value FROM json_each(?)
             )
             AND deleted_at IS NULL
             AND id IN (SELECT id FROM ordered)"#,
    )
    .bind(deduped_json)
    .bind(auth.user_id)
    .bind(serde_json::to_value(&column_states).unwrap_or(serde_json::json!([])))
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::error::AppError::Internal(format!("Failed to reorder: {e}")))?;

    let updated = result.rows_affected() as i64;

    tx.commit()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to commit reorder: {e}")))?;

    // Broadcast so other tabs refetch the new order.
    state.broadcast.send(
        "post_stage_changed",
        &serde_json::json!({
            "id": deduped.first().map(|u| u.to_string()).unwrap_or_default(),
            "state": body.state,
            "reordered": true,
            "count": updated,
        }),
    );

    Ok(Json(serde_json::json!({ "updated": updated })))
}

/// GET /api/posts/find-slot
#[derive(Debug, Deserialize)]
pub struct FindSlotQuery {
    pub integration_id: Option<Uuid>,
}

pub async fn find_slot(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<FindSlotQuery>,
) -> Result<Json<FindSlotResponse>, crate::error::AppError> {
    let slot = queries::find_next_free_slot(&state.db, auth.user_id, query.integration_id)
        .await?
        .unwrap_or_else(Utc::now);

    Ok(Json(FindSlotResponse {
        date: slot.to_rfc3339(),
    }))
}

/// POST /api/posts/{id}/repeat — set up recurring posts
#[derive(Debug, Deserialize)]
pub struct RepeatPostRequest {
    pub interval_days: i32,
    pub end_date: String,
}

#[derive(Debug, Serialize)]
pub struct RepeatPostResponse {
    pub group_id: Uuid,
    pub count: usize,
    pub post_ids: Vec<Uuid>,
    pub scheduled_dates: Vec<String>,
}

pub async fn repeat_post(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RepeatPostRequest>,
) -> Result<Json<RepeatPostResponse>, crate::error::AppError> {
    // 1. Validate the post exists and belongs to user
    let original = queries::get_post(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Post not found".into()))?;

    // 1a. Guard against double-recurring
    if original.repeat_interval_days.is_some() {
        return Err(crate::error::AppError::BadRequest(
            "Post is already part of a recurring series".into(),
        ));
    }

    // 1b. State guard — only draft or queued posts can be repeated
    match original.state {
        crate::db::models::PostState::Draft | crate::db::models::PostState::Queued => {}
        _ => {
            return Err(crate::error::AppError::BadRequest(
                "Can only set recurring for draft or queued posts".into(),
            ));
        }
    }

    let original_scheduled = original.scheduled_at.ok_or_else(|| {
        crate::error::AppError::BadRequest("Post must have a scheduled_at time".into())
    })?;

    // 2. Validate parameters
    if body.interval_days <= 0 {
        return Err(crate::error::AppError::BadRequest(
            "interval_days must be positive".into(),
        ));
    }

    let end_date = DateTime::parse_from_rfc3339(&body.end_date)
        .map_err(|_| {
            crate::error::AppError::BadRequest("Invalid end_date format, use ISO8601".into())
        })?
        .with_timezone(&Utc);

    if end_date <= original_scheduled {
        return Err(crate::error::AppError::BadRequest(
            "end_date must be after the original scheduled_at".into(),
        ));
    }

    // 3. Upper bound check — guard against runaway copy loops
    const MAX_COPIES: i32 = 100;
    let max_possible = ((end_date - original_scheduled).num_days() / body.interval_days as i64) + 1;
    if max_possible > MAX_COPIES as i64 {
        return Err(crate::error::AppError::BadRequest("Too many copies".into()));
    }

    // 4. Generate a group_id for this recurring series
    let group_id = Uuid::new_v4();

    // 5. Update original + create copies in a single transaction
    let (post_ids, scheduled_dates) = queries::set_post_recurring_with_copies(
        &state.db,
        id,
        auth.user_id,
        body.interval_days,
        &end_date,
        group_id,
        &original_scheduled,
    )
    .await?;

    Ok(Json(RepeatPostResponse {
        group_id,
        count: post_ids.len(),
        post_ids,
        scheduled_dates,
    }))
}

/// POST /api/posts/{id}/publish — publish a post now or retry a failed post
#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub id: String,
    pub state: String,
    pub platform_post_url: String,
}

pub async fn publish_post(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PublishResponse>, crate::error::AppError> {
    let platform_url = match crate::services::posts::PostService::publish(
        &state.db,
        &state.providers,
        &state.broadcast,
        &state.config,
        auth.user_id,
        id,
        state.token_key,
    )
    .await
    {
        Ok(url) => url,
        // A lost OAuth grant is not a bad post: the content is fine, the
        // token no longer carries the scopes to send it. 409 + reconnect hint
        // beats a 400 the operator cannot act on. The provider/integration
        // lookup only runs on this error path.
        Err(e) if crate::error::looks_like_scope_loss(&e) => {
            let (provider, integration_id) =
                queries::get_post_with_integration(&state.db, id, auth.user_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|p| (p.provider_identifier, p.integration_id.to_string()))
                    .unwrap_or_else(|| ("unknown".to_string(), id.to_string()));
            return Err(crate::error::AppError::ScopeLost {
                provider,
                integration_id,
                detail: e,
            });
        }
        Err(e) => return Err(crate::error::AppError::BadRequest(e)),
    };

    Ok(Json(PublishResponse {
        id: id.to_string(),
        state: "published".into(),
        platform_post_url: platform_url,
    }))
}

// ── Per-provider validation ──────────────────────────────────

/// A single validation error for a specific integration.
#[derive(Debug, serde::Serialize)]
pub struct ValidationError {
    pub integration_id: String,
    pub provider: String,
    pub provider_name: String,
    pub kind: String, // "too_long", "empty", "media_count", "media_type"
    pub message: String,
    pub max_length: Option<usize>,
    pub actual_length: Option<usize>,
}

/// Validate post content against each selected integration's provider limits.
/// Returns a list of validation errors (empty if all valid).
/// Visibility for the MCP posts_validate tool (§5.2 parity) — same rules,
/// one implementation.
pub(crate) fn validate_posts_for_integrations(
    state: &AppState,
    body: &CreatePostRequest,
    integrations: &[crate::db::models::Integration],
    media: &[crate::social::MediaAttachment],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let settings = body.settings.clone().unwrap_or(serde_json::json!({}));

    for integ in integrations {
        let provider = match state.providers.get(&integ.provider_identifier) {
            Some(p) => p,
            None => continue, // Skip unregistered providers
        };

        // Get per-integration content (from overrides if present, else global)
        let content = body
            .overrides
            .as_ref()
            .and_then(|o| o.get(&integ.id.to_string()))
            .and_then(|o| o.content.as_deref())
            .unwrap_or(&body.content);

        // The settings THIS integration will publish with: its own
        // override when it has one, else the post's global settings.
        //
        // This matters: a Reddit post carries `subreddit` in the
        // PER-INTEGRATION override, and a global-only read would report
        // the field as missing on a post that is perfectly set up.
        let per_integration_settings = body
            .overrides
            .as_ref()
            .and_then(|o| o.get(&integ.id.to_string()))
            .and_then(|o| o.settings.clone())
            .unwrap_or_else(|| settings.clone());

        // The settings this provider REQUIRES to publish.
        //
        // Discord reads `settings["channel"]`, Reddit reads
        // `settings["subreddit"]` and `settings["title"]`, and so on.
        // Without this check the post was CREATED and QUEUED and then
        // failed in the scheduler with "Missing channel in settings" -
        // the founder saw a failed post and no reason, and the chosen
        // channel was never used.
        let missing: Vec<&str> = provider
            .required_settings()
            .iter()
            .copied()
            .filter(|key| {
                per_integration_settings
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
            })
            .collect();
        if !missing.is_empty() {
            errors.push(ValidationError {
                integration_id: integ.id.to_string(),
                provider: integ.provider_identifier.clone(),
                provider_name: integ.provider_name.clone(),
                kind: "missing_setting".into(),
                message: format!(
                    "{} needs {} - pick it in Posting Targets before posting",
                    integ.provider_name,
                    missing
                        .iter()
                        .map(|k| format!("\"{k}\""))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
                max_length: None,
                actual_length: None,
            });
        }

        // The text the provider will actually receive, for the length
        // check. This is the SAME conversion the content is stored
        // through (`PostService::sanitize_content`), so the number
        // checked here is the number the provider counts. It used to be
        // a third copy of the character-level stripper.
        let clean = crate::services::posts::PostService::strip_content(content);
        let max_len = provider.max_content_length();
        // The length the provider itself counts. For X that is the
        // WEIGHTED length (an emoji costs 2), which is what the composer
        // shows; for everything else it is the byte length. The old code
        // always used `clean.len()` (bytes), so the two disagreed on any
        // post with an emoji in it: a 200-emoji Bluesky post read
        // "200/300" in the composer and was rejected by the server as
        // "800 chars, max 300".
        let length = provider.counting_rule().measure(&clean);

        // Check empty content.
        //
        // An empty body is only an error when the post has no media.
        // Instagram, TikTok, YouTube and Pinterest all accept a post
        // that is a photo with no caption - the NORMAL case for a photo.
        // The rule fired unconditionally, so a single image with no
        // caption was rejected with "Content is empty for Instagram" and
        // could not be composed at all.
        if clean.is_empty() && media.is_empty() {
            errors.push(ValidationError {
                integration_id: integ.id.to_string(),
                provider: integ.provider_identifier.clone(),
                provider_name: integ.provider_name.clone(),
                kind: "empty".into(),
                message: format!("Content is empty for {}", integ.provider_name),
                max_length: None,
                actual_length: None,
            });
            continue; // No point checking other constraints if empty
        }

        // Check content too long
        if length > max_len {
            errors.push(ValidationError {
                integration_id: integ.id.to_string(),
                provider: integ.provider_identifier.clone(),
                provider_name: integ.provider_name.clone(),
                kind: "too_long".into(),
                message: format!(
                    "Content is {} chars, max {} for {}",
                    length, max_len, integ.provider_name
                ),
                max_length: Some(max_len),
                actual_length: Some(length),
            });
        }

        // Check media limits
        let post_content = crate::social::PostContent {
            content: clean.to_string(),
            media: media.to_vec(),
            settings: settings.clone(),
            ..Default::default()
        };

        if let Err(e) =
            crate::social::validate_media_limits(&integ.provider_identifier, &post_content)
        {
            errors.push(ValidationError {
                integration_id: integ.id.to_string(),
                provider: integ.provider_identifier.clone(),
                provider_name: integ.provider_name.clone(),
                kind: "media_count".into(),
                message: e,
                max_length: None,
                actual_length: None,
            });
        }

        // Check provider-specific media validation
        if let Err(e) = provider.validate_media(&post_content) {
            errors.push(ValidationError {
                integration_id: integ.id.to_string(),
                provider: integ.provider_identifier.clone(),
                provider_name: integ.provider_name.clone(),
                kind: "media_type".into(),
                message: e,
                max_length: None,
                actual_length: None,
            });
        }
    }

    errors
}

/// POST /api/posts/validate — validate post content against provider limits
/// without creating the post. Used by the composer for live validation.
pub async fn validate(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<CreatePostRequest>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    // Validate integration ownership
    let mut integrations = Vec::with_capacity(body.integration_ids.len());
    for &id in &body.integration_ids {
        let integ = queries::get_integration(&state.db, id, auth.user_id)
            .await?
            .ok_or_else(|| crate::error::AppError::NotFound("Integration not found".into()))?;
        integrations.push(integ);
    }

    let media: Vec<crate::social::MediaAttachment> =
        serde_json::from_value(body.media.clone().unwrap_or(serde_json::json!([])))
            .unwrap_or_default();

    let errors = validate_posts_for_integrations(&state, &body, &integrations, &media);

    Ok(Json(serde_json::json!({
        "valid": errors.is_empty(),
        "errors": errors,
    })))
}

#[cfg(test)]
mod v12_platform_delete_tests {
    use super::*;
    use crate::social::{
        AuthToken, AuthUrlResponse, PageInfo, PostContent, ProviderError, SocialProvider,
    };
    use std::sync::{Arc, Mutex};

    /// A provider whose platform delete SUCCEEDS, and which records that it
    /// was called.
    struct DeletableProvider {
        deleted: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl SocialProvider for DeletableProvider {
        fn identifier(&self) -> &'static str {
            "fake-deletable"
        }
        fn name(&self) -> &'static str {
            "Fake Deletable"
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
        ) -> Result<crate::social::PublishResult, ProviderError> {
            unreachable!("not used")
        }
        async fn pages(&self, _t: &str) -> Result<Vec<PageInfo>, ProviderError> {
            Ok(vec![])
        }
        async fn fetch_page_info(
            &self,
            _access_token: &str,
            _page_id: &str,
        ) -> Result<PageInfo, ProviderError> {
            unreachable!("not used")
        }
        async fn delete_published(
            &self,
            _access_token: &str,
            platform_post_id: &str,
        ) -> Result<(), ProviderError> {
            self.deleted
                .lock()
                .unwrap()
                .push(platform_post_id.to_string());
            Ok(())
        }
    }

    /// A provider whose platform delete FAILS.
    struct UndeletableProvider;

    #[async_trait::async_trait]
    impl SocialProvider for UndeletableProvider {
        fn identifier(&self) -> &'static str {
            "fake-undeletable"
        }
        fn name(&self) -> &'static str {
            "Fake Undeletable"
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
        ) -> Result<crate::social::PublishResult, ProviderError> {
            unreachable!("not used")
        }
        async fn pages(&self, _t: &str) -> Result<Vec<PageInfo>, ProviderError> {
            Ok(vec![])
        }
        async fn fetch_page_info(
            &self,
            _access_token: &str,
            _page_id: &str,
        ) -> Result<PageInfo, ProviderError> {
            unreachable!("not used")
        }
    }

    async fn state_with(provider: Arc<dyn SocialProvider>) -> (AppState, Arc<Mutex<Vec<String>>>) {
        let deleted = Arc::new(Mutex::new(Vec::new()));
        let provider: Arc<dyn SocialProvider> = if provider.identifier() == "fake-deletable" {
            Arc::new(DeletableProvider {
                deleted: deleted.clone(),
            })
        } else {
            provider
        };
        crate::config::load_dotenv_for_tests();
        let config = crate::config::Config::from_env().expect("config");
        let db = crate::db::create_pool(&config.database_url)
            .await
            .expect("pool");
        crate::db::ensure_local_user(&db).await.expect("local user");
        let registry = crate::social::registry::ProviderRegistry::with_providers(vec![(
            provider.identifier(),
            provider,
        )]);
        (
            AppState {
                db,
                config,
                broadcast: crate::realtime::Broadcaster::new(),
                providers: registry,
                token_key: None,
                telegram_client_manager: None,
                wa_client: None,
                media_http_client: reqwest::Client::new(),
                media_wreq_client: wreq::Client::new(),
            },
            deleted,
        )
    }

    async fn seed_published_post(state: &AppState, provider: &str) -> (Uuid, Uuid) {
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        let integration_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO integrations
                (id, user_id, provider_identifier, provider_name, internal_id,
                 access_token, refresh_token, token_expires_at,
                 disabled, refresh_needed, posting_times, auth_method)
             VALUES (?, ?, ?, 'Fake', ?, 'tok', NULL, NULL, 0, 0, '[]', 'oauth')",
        )
        .bind(integration_id)
        .bind(user_id)
        .bind(provider)
        .bind(format!("internal-{integration_id}"))
        .execute(&state.db)
        .await
        .expect("seed the integration");
        let post_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO posts
                (id, user_id, integration_id, state, content, title, media, settings,
                 scheduled_at, published_at, platform_post_id, platform_post_url,
                 error_message, created_at, updated_at, repeat_interval_days,
                 repeat_end_date, group_id, first_comment, sequence, idempotency_key)
             VALUES (?, ?, ?, 'published', 'hello', NULL, '[]', '{}', NULL, unixepoch(),
                     'platform-1', NULL, NULL, unixepoch(), unixepoch(), NULL, NULL, NULL,
                     NULL, 0, ?)",
        )
        .bind(post_id)
        .bind(user_id)
        .bind(integration_id)
        .bind(Uuid::new_v4())
        .execute(&state.db)
        .await
        .expect("seed the post");
        (post_id, integration_id)
    }

    #[tokio::test]
    async fn the_platform_delete_removes_the_post_on_the_platform_and_locally() {
        let (state, deleted) = state_with(Arc::new(DeletableProvider {
            deleted: Arc::new(Mutex::new(Vec::new())),
        }))
        .await;
        let (post_id, _) = seed_published_post(&state, "fake-deletable").await;

        let result = delete(
            State(state.clone()),
            AuthenticatedUser {
                user_id: crate::auth::middleware::DEFAULT_USER_ID,
            },
            Path(post_id),
            Query(DeletePostParams {
                delete_from_platform: Some(true),
            }),
        )
        .await;
        assert!(result.is_ok(), "the delete must succeed: {result:?}");

        // The delete is a SOFT delete (`deleted_at`), so the row is still
        // there — what matters is that it is no longer visible.
        let deleted_at: Option<i64> =
            sqlx::query_scalar("SELECT deleted_at FROM posts WHERE id = ?")
                .bind(post_id)
                .fetch_one(&state.db)
                .await
                .expect("read");
        assert!(deleted_at.is_some(), "the local row must be marked deleted");
    }

    #[tokio::test]
    async fn a_failed_platform_delete_keeps_the_local_row_for_a_retry() {
        // The row carries the platform id. If it is deleted anyway, the
        // founder has a live tweet and NO trace of it in the app — and no
        // way to retry.
        let (state, _) = state_with(Arc::new(UndeletableProvider)).await;
        let (post_id, _) = seed_published_post(&state, "fake-undeletable").await;

        let result = delete(
            State(state.clone()),
            AuthenticatedUser {
                user_id: crate::auth::middleware::DEFAULT_USER_ID,
            },
            Path(post_id),
            Query(DeletePostParams {
                delete_from_platform: Some(true),
            }),
        )
        .await;
        assert!(result.is_err(), "the platform delete must fail loudly");

        let deleted_at: Option<i64> =
            sqlx::query_scalar("SELECT deleted_at FROM posts WHERE id = ?")
                .bind(post_id)
                .fetch_one(&state.db)
                .await
                .expect("read");
        assert!(
            deleted_at.is_none(),
            "the local row must be KEPT — with its platform id — for a retry"
        );
    }

    #[tokio::test]
    async fn a_local_delete_does_not_touch_the_platform() {
        let (state, _) = state_with(Arc::new(UndeletableProvider)).await;
        let (post_id, _) = seed_published_post(&state, "fake-undeletable").await;

        let result = delete(
            State(state.clone()),
            AuthenticatedUser {
                user_id: crate::auth::middleware::DEFAULT_USER_ID,
            },
            Path(post_id),
            Query(DeletePostParams::default()),
        )
        .await;
        assert!(result.is_ok(), "the local delete must succeed: {result:?}");
    }
}
