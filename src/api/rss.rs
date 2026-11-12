use axum::extract::{Path, Query, State};
use axum::Json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

use crate::api::AppState;
use crate::auth::middleware::AuthenticatedUser;
use crate::db::{models::RssFeed, queries};
use crate::error::AppError;

#[derive(serde::Deserialize)]
pub struct CreateFeedRequest {
    pub feed_url: String,
    pub integration_id: String,
    pub title: Option<String>,
    pub use_ai_summary: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(serde::Serialize)]
pub struct RssFeedItemResponse {
    pub id: String,
    pub feed_id: String,
    pub guid: String,
    pub title: String,
    pub url: String,
    pub published_at: Option<String>,
    pub content_hash: String,
    pub is_imported: bool,
    pub post_id: Option<String>,
    pub created_at: String,
}

impl From<crate::db::models::RssPost> for RssFeedItemResponse {
    fn from(p: crate::db::models::RssPost) -> Self {
        Self {
            id: p.id.to_string(),
            feed_id: p.feed_id.to_string(),
            guid: p.guid,
            title: p.title,
            url: p.url,
            published_at: p.published_at.map(|d| d.to_rfc3339()),
            content_hash: p.content_hash,
            is_imported: p.is_imported,
            post_id: p.post_id.map(|id| id.to_string()),
            created_at: p.created_at.to_rfc3339(),
        }
    }
}

// GET /api/rss/feeds
pub async fn list_feeds(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<Vec<RssFeed>>, AppError> {
    let feeds = queries::list_rss_feeds(&state.db, auth.user_id).await?;
    Ok(Json(feeds))
}

/// A feed body larger than this is truncated: a hostile feed must not be able
/// to stream megabytes into the single binary's memory.
const RSS_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

// POST /api/rss/feeds
pub async fn create_feed(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<CreateFeedRequest>,
) -> Result<Json<RssFeed>, AppError> {
    let integration_id: Uuid = body
        .integration_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid integration_id format".into()))?;

    let integration = queries::get_integration(&state.db, integration_id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let title = body.title.unwrap_or_default();
    let use_ai_summary = body.use_ai_summary.unwrap_or(false);
    let enabled = body.enabled.unwrap_or(true);

    // The poller FETCHES this URL, so it is an SSRF target for the same
    // reason a webhook URL is: the parsed title/link/content come back to
    // the founder. Refuse a private/loopback host at the door.
    crate::net_guard::validate_outbound_url(&body.feed_url)
        .await
        .map_err(|e| AppError::BadRequest(format!("Feed URL rejected: {e}")))?;

    let feed = queries::create_rss_feed(
        &state.db,
        auth.user_id,
        &body.feed_url,
        integration.id,
        &title,
        use_ai_summary,
        enabled,
    )
    .await?;

    Ok(Json(feed))
}

// DELETE /api/rss/feeds/{id}
pub async fn delete_feed(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let affected = queries::delete_rss_feed(&state.db, id, auth.user_id).await?;
    if affected == 0 {
        return Err(AppError::NotFound("RSS feed not found".into()));
    }
    Ok(Json(serde_json::json!({ "success": true })))
}

// PUT /api/rss/feeds/{id}/toggle
pub async fn toggle_feed(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<RssFeed>, AppError> {
    let feed = queries::toggle_rss_feed(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("RSS feed not found".into()))?;
    Ok(Json(feed))
}

// POST /api/rss/feeds/{id}/poll — manual trigger
pub async fn poll_feed(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let feed = queries::get_rss_feed(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("RSS feed not found".into()))?;

    // Guarded fetch: the URL and every redirect hop are validated, and the
    // body is capped so a hostile feed cannot stream megabytes into memory.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to create HTTP client: {e}")))?;
    let response = crate::net_guard::send_guarded(
        &client,
        reqwest::Method::GET,
        &feed.feed_url,
        &[],
        None,
        RSS_MAX_BODY_BYTES,
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("Failed to fetch RSS feed: {e}")))?;
    let xml = response.body;

    let parsed = feed_rs::parser::parse(xml.as_bytes())
        .map_err(|e| AppError::Internal(format!("Failed to parse RSS XML: {e}")))?;

    let mut new_count = 0u32;

    for entry in parsed.entries {
        let title = entry.title.map(|t| t.content).unwrap_or_default();
        let url = entry
            .links
            .first()
            .map(|l| l.href.clone())
            .unwrap_or_default();
        let guid = entry.id;
        let published = entry
            .published
            .and_then(|d| chrono::DateTime::from_timestamp(d.timestamp(), 0));
        let content = entry
            .content
            .and_then(|c| c.body)
            .or_else(|| entry.summary.map(|s| s.content))
            .unwrap_or_default();

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        // Only entries newly claimed by guid count as new items.
        let claimed = queries::insert_rss_post(
            &state.db,
            feed.id,
            &guid,
            &title,
            &url,
            published,
            &content_hash,
        )
        .await?;
        if claimed.is_some() {
            new_count += 1;
        }
    }

    queries::update_feed_last_polled(&state.db, feed.id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "new_items": new_count,
    })))
}

// GET /api/rss/feeds/{id}/items
pub async fn list_feed_items(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<RssFeedItemResponse>>, AppError> {
    let _feed = queries::get_rss_feed(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("RSS feed not found".into()))?;

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let items = queries::list_rss_feed_items(&state.db, id, auth.user_id, limit, offset).await?;

    let response: Vec<RssFeedItemResponse> = items.into_iter().map(Into::into).collect();
    Ok(Json(response))
}

// POST /api/rss/feeds/{id}/items/{guid}/import
pub async fn import_item(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((feed_id, guid)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let feed = queries::get_rss_feed(&state.db, feed_id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("RSS feed not found".into()))?;

    let items = queries::list_rss_feed_items(&state.db, feed_id, auth.user_id, 1000, 0).await?;
    let rss_post = items
        .into_iter()
        .find(|p| p.guid == guid)
        .ok_or_else(|| AppError::NotFound("RSS item not found".into()))?;

    if rss_post.post_id.is_some() {
        return Err(AppError::BadRequest("RSS item already imported".into()));
    }

    let content = format!("{}\n\n{}", rss_post.title, rss_post.url);
    let post = queries::create_post(
        &state.db,
        auth.user_id,
        feed.integration_id,
        &content,
        Some(&rss_post.title),
        // `media` is an ARRAY - the column's default is '[]' and every
        // reader treats it as one. It was `{}`, and the kanban's
        // `post.media.find(...)` crashed the whole ROUTE with
        // "i.find is not a function" on every imported draft.
        &serde_json::json!([]),
        &serde_json::json!({}),
        None,
        Some(crate::db::models::PostState::Draft),
        None,
        0,
        None,
    )
    .await?;

    queries::update_rss_post_post_id(&state.db, rss_post.id, post.id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "post_id": post.id.to_string(),
    })))
}
