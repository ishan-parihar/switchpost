// ─── MCP Admin/Parity Tools ────────────────────────────────────
// MCP wrappers for REST endpoints that were missing from the MCP
// surface — identified in the v9 AI-agent-efficacy audit.
//
// Each tool here is a thin wrapper over an existing db::queries
// function or service call. The goal is MCP/CLI/API parity so an
// AI agent using only MCP can do everything the WebUI can do.
//
// NOTE: all SQL here uses runtime `sqlx::query()` (not the `query!`
// macro) to avoid requiring a live DB or .sqlx offline cache refresh
// at build time. Trade-off: no compile-time column type checking,
// but the queries are simple and the structs derive FromRow.

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::db::queries;

// ── Recurring Posts ───────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PostsRepeatInput {
    /// Post ID (UUID)
    pub post_id: String,
    /// Repeat interval in days (e.g. 7 for weekly, 30 for monthly)
    pub interval_days: i32,
    /// End date for the repeat series (RFC3339, after the post's scheduled_at).
    pub end_date: String,
}

/// B13: set up a recurring/evergreen post. Creates a queued copy of the
/// post at every `interval_days` until `end_date` — the same
/// set_post_recurring_with_copies path REST `/api/posts/{id}/repeat` uses.
/// Previously this tool only wrote `repeat_interval_days`, which nothing
/// reads: it told the agent "Post will repeat every N days" while no
/// repeat ever fired.
pub async fn handle_posts_repeat(
    state: &AppState,
    input: &PostsRepeatInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let post_id =
        Uuid::parse_str(&input.post_id).map_err(|_| "Invalid post_id UUID".to_string())?;

    if input.interval_days < 1 {
        return Err("interval_days must be >= 1".into());
    }

    let post = queries::get_post(&state.db, post_id, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| "Post not found".to_string())?;
    let original_scheduled = post
        .scheduled_at
        .ok_or_else(|| "Post has no scheduled_at — schedule it before repeating".to_string())?;

    let end_date = chrono::DateTime::parse_from_rfc3339(&input.end_date)
        .map_err(|e| format!("Invalid end_date (use RFC3339): {e}"))?
        .with_timezone(&chrono::Utc);
    if end_date <= original_scheduled {
        return Err("end_date must be after the post's scheduled_at".into());
    }

    // Guard against runaway copy loops (mirrors the REST handler).
    const MAX_COPIES: i64 = 100;
    let max_possible = (end_date - original_scheduled).num_days() / input.interval_days as i64 + 1;
    if max_possible > MAX_COPIES {
        return Err(format!(
            "end_date implies {max_possible} copies; the cap is {MAX_COPIES}"
        ));
    }

    let group_id = Uuid::new_v4();
    let (post_ids, scheduled_dates) = queries::set_post_recurring_with_copies(
        &state.db,
        post_id,
        user_id,
        input.interval_days,
        &end_date,
        group_id,
        &original_scheduled,
    )
    .await
    .map_err(|e| format!("Failed to set repeat: {e}"))?;

    Ok(Json(serde_json::json!({
        "group_id": group_id.to_string(),
        "count": post_ids.len(),
        "post_ids": post_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        "scheduled_dates": scheduled_dates,
        "message": format!(
            "Post will repeat every {} days until {} — {} copy/copies created",
            input.interval_days,
            end_date.to_rfc3339(),
            post_ids.len() - 1
        ),
    })))
}

// ── Post Tags ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PostsSetTagsInput {
    /// Post ID (UUID)
    pub post_id: String,
    /// List of tag IDs (UUIDs) to attach to this post. Replaces any existing tags.
    pub tag_ids: Vec<String>,
}

/// Attach tags to a post. Replaces any existing tags.
pub async fn handle_posts_set_tags(
    state: &AppState,
    input: &PostsSetTagsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let post_id =
        Uuid::parse_str(&input.post_id).map_err(|_| "Invalid post_id UUID".to_string())?;

    let tag_uuids: Vec<Uuid> = input
        .tag_ids
        .iter()
        .map(|s| Uuid::parse_str(s).map_err(|e| format!("Invalid tag UUID '{s}': {e}")))
        .collect::<Result<_, _>>()?;

    // The REST handler is the one implementation. It checks that every
    // tag belongs to the user, and it writes the DELETE and the
    // INSERTs in ONE TRANSACTION.
    //
    // The MCP tool re-implemented the SQL inline and left both halves
    // out: a tag belonging to another account could be attached, and a
    // failed INSERT left the post with NO tags at all - the REST path
    // rolls back to the original set.
    super::rest_bridge::rest_result(
        crate::api::posts::set_post_tags(
            axum::extract::State(state.clone()),
            crate::auth::middleware::AuthenticatedUser { user_id },
            axum::extract::Path(post_id),
            axum::Json(crate::api::posts::SetPostTagsRequest { tag_ids: tag_uuids }),
        )
        .await,
    )
}

// ── Media Delete ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaDeleteInput {
    /// Media ID (UUID)
    pub media_id: String,
}

/// Delete a media file. Removes the DB row AND the file from whichever
/// storage backend is active.
pub async fn handle_media_delete(
    state: &AppState,
    input: &MediaDeleteInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let media_id =
        Uuid::parse_str(&input.media_id).map_err(|_| "Invalid media_id UUID".to_string())?;

    let entry = queries::delete_media(&state.db, media_id, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| "Media not found".to_string())?;

    // Delete the file through the ACTIVE BACKEND. This used to join
    // `state.config.media_dir` directly, so on a deployment with
    // S3/R2 configured the row was removed and the OBJECT LEFT IN THE
    // BUCKET - still publicly reachable at its CDN URL, and the bucket
    // growing without bound.
    let storage = crate::api::media::storage(state);
    if let Err(e) = storage.delete(&entry.storage_path).await {
        tracing::warn!("Failed to delete media file {:?}: {e}", entry.storage_path);
    }

    Ok(Json(serde_json::json!({
        "deleted": true,
        "media_id": media_id.to_string(),
    })))
}

// ── Integration Refresh ───────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationsRefreshInput {
    /// Integration ID (UUID)
    pub integration_id: String,
}

/// Manually refresh the OAuth access token for an integration.
/// Useful when the proactive refresh cycle hasn't run yet but the
/// token is known to be expiring soon.
pub async fn handle_integrations_refresh(
    state: &AppState,
    input: &IntegrationsRefreshInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let integration_id = Uuid::parse_str(&input.integration_id)
        .map_err(|_| "Invalid integration_id UUID".to_string())?;

    let integration = queries::get_integration(&state.db, integration_id, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| "Integration not found".to_string())?;

    let provider = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| {
            format!(
                "Provider '{}' not registered",
                integration.provider_identifier
            )
        })?;

    // THE ONE REFRESH PATH — the same one the REST refresh, the scheduler's
    // pre-publish refresh and the mid-publish retry use. It preserves the
    // sub-account contract: a Facebook Page channel keeps its PAGE token
    // and re-derives it from the refreshed user token, instead of storing
    // the user token (which sends later posts to the wrong page).
    let outcome = crate::services::integrations::refresh_channel_credentials(
        &state.db,
        provider.as_ref(),
        &integration,
        state.token_key,
    )
    .await?;

    Ok(Json(serde_json::json!({
        "integration_id": integration_id.to_string(),
        "refreshed": true,
        "expires_at": outcome.expires_at.map(|e| e.to_rfc3339()),
    })))
}

// ── Signatures ────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignatureCreateInput {
    /// Signature name (for identification)
    pub name: String,
    /// Signature content (appended to posts)
    pub content: String,
    /// Provider identifier (e.g. "x", "linkedin"). Optional — if omitted, applies to all.
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignatureListInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignatureUpdateInput {
    pub signature_id: String,
    pub name: Option<String>,
    pub content: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignatureDeleteInput {
    pub signature_id: String,
}

#[derive(Default, sqlx::FromRow)]
struct SignatureRow {
    id: Uuid,
    name: String,
    content: String,
    provider: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

pub async fn handle_signatures_list(
    state: &AppState,
    _input: &SignatureListInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let sigs: Vec<SignatureRow> = sqlx::query_as(
        "SELECT id, name, content, provider, created_at, updated_at
         FROM signatures WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let data: Vec<_> = sigs
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id.to_string(),
                "name": s.name,
                "content": s.content,
                "provider": s.provider,
                "created_at": s.created_at.to_rfc3339(),
                "updated_at": s.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "signatures": data })))
}

pub async fn handle_signatures_create(
    state: &AppState,
    input: &SignatureCreateInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let sig: SignatureRow = sqlx::query_as(
        "INSERT INTO signatures (user_id, name, content, provider)
         VALUES ($1, $2, $3, $4)
         RETURNING id, name, content, provider, created_at, updated_at",
    )
    .bind(user_id)
    .bind(&input.name)
    .bind(&input.content)
    .bind(&input.provider)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(serde_json::json!({
        "id": sig.id.to_string(),
        "name": sig.name,
        "content": sig.content,
        "provider": sig.provider,
        "created_at": sig.created_at.to_rfc3339(),
    })))
}

pub async fn handle_signatures_update(
    state: &AppState,
    input: &SignatureUpdateInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let sig_id = Uuid::parse_str(&input.signature_id)
        .map_err(|_| "Invalid signature_id UUID".to_string())?;

    let sig: SignatureRow = sqlx::query_as(
        "UPDATE signatures
         SET name = COALESCE($2, name),
             content = COALESCE($3, content),
             provider = COALESCE($4, provider),
             updated_at = unixepoch()
         WHERE id = $1 AND user_id = $5
         RETURNING id, name, content, provider, created_at, updated_at",
    )
    .bind(sig_id)
    .bind(&input.name)
    .bind(&input.content)
    .bind(&input.provider)
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(serde_json::json!({
        "id": sig.id.to_string(),
        "name": sig.name,
        "content": sig.content,
        "provider": sig.provider,
    })))
}

pub async fn handle_signatures_delete(
    state: &AppState,
    input: &SignatureDeleteInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let sig_id = Uuid::parse_str(&input.signature_id)
        .map_err(|_| "Invalid signature_id UUID".to_string())?;

    sqlx::query("DELETE FROM signatures WHERE id = $1 AND user_id = $2")
        .bind(sig_id)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(
        serde_json::json!({ "deleted": true, "id": input.signature_id }),
    ))
}

// ── Analytics Summary ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AnalyticsSummaryInput {
    /// Number of days to include in the summary (default 30)
    pub days: Option<i32>,
}

#[derive(Default, sqlx::FromRow)]
struct SummaryCounts {
    total: i64,
    published: i64,
    failed: i64,
    draft: i64,
    queued: i64,
}

/// Get aggregate analytics summary across all platforms: total posts,
/// published/failed/draft/queued counts, posts by provider, posts by day.
pub async fn handle_analytics_summary(
    state: &AppState,
    input: &AnalyticsSummaryInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let days = input.days.unwrap_or(30).clamp(1, 365) as i64;

    let counts: SummaryCounts = sqlx::query_as(
        "SELECT
            COUNT(*) as total,
            COUNT(*) FILTER (WHERE state = 'published') as published,
            COUNT(*) FILTER (WHERE state = 'error') as failed,
            COUNT(*) FILTER (WHERE state = 'draft') as draft,
            COUNT(*) FILTER (WHERE state = 'queued') as queued
           FROM posts
           WHERE user_id = $1
             AND created_at > unixepoch() - $2 * 86400
             AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(days as f64)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(serde_json::json!({
        "total_posts": counts.total,
        "published": counts.published,
        "failed": counts.failed,
        "draft": counts.draft,
        "queued": counts.queued,
        "days": days,
    })))
}

// ── Feed Analytics ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FeedAnalyticsInput {
    /// Number of days to include (default 30)
    pub days: Option<i32>,
}

/// Get feed-level analytics: engagement totals across all imported
/// feed posts, top posts by engagement, engagement trends over time.
pub async fn handle_feed_analytics(
    state: &AppState,
    input: &FeedAnalyticsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let days = input.days.unwrap_or(30).clamp(1, 365) as i64;

    // The SAME query the REST endpoint runs, with the SAME user
    // scoping.
    //
    // The MCP version used to aggregate `post_engagement` across every
    // row in the table - it read the user id and threw it away
    // (`let _user_id = ...`). The REST version filters by
    // `ep.user_id = ?`. So the agent's tool and the founder's page
    // reported DIFFERENT numbers for the same question, and the
    // agent's number was not scoped to the founder at all.
    //
    // The cutoff is an epoch, and the query binds it through the
    // `EpochUtc` seam - `external_posts.created_at` is an INTEGER
    // epoch column, and a TEXT cutoff compared against it matches NO
    // ROWS.
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    let summary =
        crate::db::queries::get_engagement_summary(&state.db, user_id, None, Some(cutoff))
            .await
            .map_err(|e| format!("DB error: {e}"))?;

    Ok(Json(serde_json::json!({
        "total_likes": summary.total_likes.unwrap_or(0),
        "total_comments": summary.total_comments.unwrap_or(0),
        "total_shares": summary.total_shares.unwrap_or(0),
        "total_views": summary.total_views.unwrap_or(0),
        "total_reposts": summary.total_reposts.unwrap_or(0),
        "total_replies": summary.total_replies.unwrap_or(0),
        "total_upvotes": summary.total_upvotes.unwrap_or(0),
        "total_awards": summary.total_awards.unwrap_or(0),
        "posts_with_engagement": summary.posts_with_engagement.unwrap_or(0),
        "days": days,
    })))
}
