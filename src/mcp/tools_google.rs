// ─── MCP Google Tools ──────────────────────────────────────────
// Consolidated Google tools: YouTube, Gmail, Calendar, Drive.
// Each family resolves its own integration (see find_token_for).
// Follows the same pattern as tools_instagram_standalone.rs.

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::crypto;
use crate::social::calendar::CalendarProvider;
use crate::social::drive::DriveProvider;
use crate::social::gmail::GmailProvider;
use crate::social::youtube::YoutubeProvider;

// ══════════════════════════════════════════════════════════════
// SHARED HELPERS
// ══════════════════════════════════════════════════════════════

/// Resolve a token from the connected integration for `provider_id`.
///
/// WHY THIS IS PARAMETERISED, AND WHY DRIVE GETS ITS OWN CALL SITE:
/// Google refuses `auth/youtube*` and `auth/drive*` in ONE consent
/// ("This request contains scopes that cannot be requested together"). So
/// the unified `google` integration carries a YouTube/Gmail/Calendar grant
/// and never a Drive one. The Drive tools therefore resolve the `drive`
/// integration, which is connectable separately (see `social::registry`):
/// sending a Drive call with the `google` token draws a 403 from Google and
/// an error that names nothing the founder can act on.
async fn find_token_for(
    state: &AppState,
    user_id: Uuid,
    provider_id: &str,
) -> Result<String, String> {
    let integrations = crate::db::queries::list_integrations(&state.db, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let integration = integrations
        .iter()
        .find(|i| i.provider_identifier == provider_id)
        .ok_or_else(|| {
            format!("{provider_id} is not connected. Connect it from the Channels screen first.")
        })?;

    Ok(crypto::maybe_decrypt_token(
        &integration.access_token,
        state.token_key.as_ref(),
    ))
}

/// Each Google family resolves its OWN integration.
///
/// The unified `google` provider is gone: it published YouTube videos and
/// nothing else, so it was a second YouTube button (and, in the channels
/// list, a second set of the founder's YouTube channels). Each tool now
/// asks for the family it calls — a YouTube token on the Gmail API is a
/// 403, and a Gmail token on the YouTube API is the same.

/// The Drive token, with the error that says what to DO.
///
/// A 403 from Google does not distinguish "Drive is not connected" from
/// "the connected token has no Drive grant". This message does, and it
/// names the Channel screen the founder has to visit.
async fn find_drive_token(state: &AppState, user_id: Uuid) -> Result<String, String> {
    find_token_for(state, user_id, "drive").await.map_err(|_| {
        "Google Drive is not connected. Connect 'Google Drive' from the Channels screen: \
         Google will not grant Drive access in the same consent as the YouTube and \
         Calendar scopes, so it is a separate connection."
            .to_string()
    })
}

// F1: one construction site per provider type. The per-tool-call cost is a
// String clone + an Arc'd shared client clone — the providers themselves are
// stateless config holders, so a registry round-trip buys nothing here.
fn yt(state: &AppState) -> YoutubeProvider {
    YoutubeProvider::new(&state.config)
}

fn gmail(state: &AppState) -> GmailProvider {
    GmailProvider::new(&state.config)
}

fn cal(state: &AppState) -> CalendarProvider {
    CalendarProvider::new(&state.config)
}

fn drv(state: &AppState) -> DriveProvider {
    DriveProvider::new(&state.config)
}

// ══════════════════════════════════════════════════════════════
// YOUTUBE — Input Types
// ══════════════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtSearchVideosInput {
    pub channel_id: String,
    pub query: String,
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetVideoInput {
    pub channel_id: String,
    pub video_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtListPlaylistsInput {
    pub channel_id: String,
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetPlaylistItemsInput {
    pub channel_id: String,
    pub playlist_id: String,
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetCommentsInput {
    pub channel_id: String,
    pub video_id: String,
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetChannelStatsInput {
    pub channel_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetAnalyticsInput {
    pub channel_id: String,
    pub metrics: String,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtGetSubscriptionsInput {
    pub channel_id: String,
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct YtFindCreatorsInput {
    /// Topic/query to search for creators
    pub query: String,
    /// Minimum subscriber count filter (optional)
    pub min_subscribers: Option<u32>,
    /// Maximum number of results (default 10, max 50)
    pub max_results: Option<u32>,
}

// ══════════════════════════════════════════════════════════════
// YOUTUBE — Tool Handlers
// ══════════════════════════════════════════════════════════════

pub async fn handle_goog_search_videos(
    state: &AppState,
    input: &YtSearchVideosInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let max_results = input.max_results.unwrap_or(10);
    let result = provider
        .search_videos(&token, &input.query, max_results)
        .await
        .map_err(|e| format!("YouTube search videos failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_video(
    state: &AppState,
    input: &YtGetVideoInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let result = provider
        .get_video(&token, &input.video_id)
        .await
        .map_err(|e| format!("YouTube get video failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_playlists(
    state: &AppState,
    input: &YtListPlaylistsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let max_results = input.max_results.unwrap_or(10);
    let result = provider
        .get_playlists(&token, &input.channel_id, max_results)
        .await
        .map_err(|e| format!("YouTube list playlists failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_playlist_items(
    state: &AppState,
    input: &YtGetPlaylistItemsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let max_results = input.max_results.unwrap_or(10);
    let result = provider
        .get_playlist_items(&token, &input.playlist_id, max_results)
        .await
        .map_err(|e| format!("YouTube get playlist items failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_comments(
    state: &AppState,
    input: &YtGetCommentsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let max_results = input.max_results.unwrap_or(10);
    let result = provider
        .get_comments(&token, &input.video_id, max_results)
        .await
        .map_err(|e| format!("YouTube get comments failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_channel_stats(
    state: &AppState,
    input: &YtGetChannelStatsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let result = provider
        .get_channel_stats(&token, &input.channel_id)
        .await
        .map_err(|e| format!("YouTube get channel stats failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_analytics(
    state: &AppState,
    input: &YtGetAnalyticsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let result = provider
        .get_analytics(
            &token,
            &input.channel_id,
            &input.metrics,
            &input.start_date,
            &input.end_date,
        )
        .await
        .map_err(|e| format!("YouTube get analytics failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_subscriptions(
    state: &AppState,
    input: &YtGetSubscriptionsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let max_results = input.max_results.unwrap_or(10);
    let result = provider
        .get_subscriptions(&token, &input.channel_id, max_results)
        .await
        .map_err(|e| format!("YouTube get subscriptions failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_find_creators(
    state: &AppState,
    input: &YtFindCreatorsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = yt(state);
    let result = provider
        .find_creators(
            &token,
            &input.query,
            input.min_subscribers,
            input.max_results,
        )
        .await
        .map_err(|e| format!("YouTube find creators failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

// ══════════════════════════════════════════════════════════════
// GMAIL — Input Types
// ══════════════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmListMessagesInput {
    /// Max results (default 20, max 500)
    pub max_results: Option<u32>,
    /// Gmail search query (optional, e.g. "from:someone@example.com")
    pub query: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmGetMessageInput {
    /// Gmail message ID
    pub message_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmSendMessageInput {
    /// Recipient email address
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmGetThreadInput {
    /// Gmail thread ID
    pub thread_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmSearchMessagesInput {
    /// Gmail search query
    pub query: String,
    /// Max results (default 20, max 500)
    pub max_results: Option<u32>,
}

// ══════════════════════════════════════════════════════════════
// GMAIL — Tool Handlers
// ══════════════════════════════════════════════════════════════

pub async fn handle_goog_get_profile(
    state: &AppState,
    _input: &(),
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "youtube").await?;
    let provider = gmail(state);
    let result = provider
        .get_profile(&token)
        .await
        .map_err(|e| format!("Gmail get profile failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_list_messages(
    state: &AppState,
    input: &GmListMessagesInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let max_results = input.max_results.unwrap_or(20);
    let result = provider
        .list_messages(&token, max_results, input.query.as_deref())
        .await
        .map_err(|e| format!("Gmail list messages failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_message(
    state: &AppState,
    input: &GmGetMessageInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let result = provider
        .get_message(&token, &input.message_id)
        .await
        .map_err(|e| format!("Gmail get message failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_send_message(
    state: &AppState,
    input: &GmSendMessageInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let result = provider
        .send_message(&token, &input.to, &input.subject, &input.body)
        .await
        .map_err(|e| format!("Gmail send message failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_list_labels(
    state: &AppState,
    _input: &(),
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let result = provider
        .list_labels(&token)
        .await
        .map_err(|e| format!("Gmail list labels failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_thread(
    state: &AppState,
    input: &GmGetThreadInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let result = provider
        .get_thread(&token, &input.thread_id)
        .await
        .map_err(|e| format!("Gmail get thread failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_search_messages(
    state: &AppState,
    input: &GmSearchMessagesInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "gmail").await?;
    let provider = gmail(state);
    let max_results = input.max_results.unwrap_or(20);
    let result = provider
        .search_messages(&token, &input.query, max_results)
        .await
        .map_err(|e| format!("Gmail search messages failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

// ══════════════════════════════════════════════════════════════
// GOOGLE CALENDAR — Input Types
// ══════════════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalListCalendarsInput {
    /// Max results (default 50)
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalListEventsInput {
    /// Calendar ID (default: 'primary')
    pub calendar_id: Option<String>,
    /// Max results (default 20)
    pub max_results: Option<u32>,
    /// Start of time range in ISO 8601 (default: now)
    pub time_min: Option<String>,
    /// End of time range in ISO 8601 (default: +1 week)
    pub time_max: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalCreateEventInput {
    /// Calendar ID (default: 'primary')
    pub calendar_id: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    /// Start time in ISO 8601 (e.g. "2026-05-15T10:00:00Z")
    pub start_time: String,
    /// End time in ISO 8601
    pub end_time: String,
    /// Timezone (default: "UTC")
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalGetEventInput {
    /// Calendar ID (default: 'primary')
    pub calendar_id: Option<String>,
    pub event_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalUpdateEventInput {
    /// Calendar ID (default: 'primary')
    pub calendar_id: Option<String>,
    pub event_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GcalDeleteEventInput {
    /// Calendar ID (default: 'primary')
    pub calendar_id: Option<String>,
    pub event_id: String,
}

// ══════════════════════════════════════════════════════════════
// GOOGLE CALENDAR — Tool Handlers
// ══════════════════════════════════════════════════════════════

pub async fn handle_goog_list_calendars(
    state: &AppState,
    _input: &GcalListCalendarsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let result = provider
        .list_calendars(&token)
        .await
        .map_err(|e| format!("Calendar list failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_list_events(
    state: &AppState,
    input: &GcalListEventsInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let cal_id = input.calendar_id.as_deref().unwrap_or("primary");
    let max_results = input.max_results.unwrap_or(20);
    let result = provider
        .list_events(
            &token,
            cal_id,
            max_results,
            input.time_min.as_deref(),
            input.time_max.as_deref(),
        )
        .await
        .map_err(|e| format!("Calendar list events failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_event(
    state: &AppState,
    input: &GcalGetEventInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let cal_id = input.calendar_id.as_deref().unwrap_or("primary");
    let result = provider
        .get_event(&token, cal_id, &input.event_id)
        .await
        .map_err(|e| format!("Calendar get event failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_create_event(
    state: &AppState,
    input: &GcalCreateEventInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let cal_id = input.calendar_id.as_deref().unwrap_or("primary");
    let result = provider
        .create_event(
            &token,
            cal_id,
            &input.summary,
            &input.start_time,
            &input.end_time,
            input.description.as_deref(),
        )
        .await
        .map_err(|e| format!("Calendar create event failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_update_event(
    state: &AppState,
    input: &GcalUpdateEventInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let cal_id = input.calendar_id.as_deref().unwrap_or("primary");
    let result = provider
        .update_event(
            &token,
            cal_id,
            &input.event_id,
            input.summary.as_deref(),
            input.description.as_deref(),
        )
        .await
        .map_err(|e| format!("Calendar update event failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_delete_event(
    state: &AppState,
    input: &GcalDeleteEventInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_token_for(state, user_id, "calendar").await?;
    let provider = cal(state);
    let cal_id = input.calendar_id.as_deref().unwrap_or("primary");
    let result = provider
        .delete_event(&token, cal_id, &input.event_id)
        .await
        .map_err(|e| format!("Calendar delete event failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

// ══════════════════════════════════════════════════════════════
// GOOGLE DRIVE — Input Types
// ══════════════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrListFilesInput {
    /// Max results (default 20, max 100)
    pub max_results: Option<u32>,
    /// Optional MIME type filter (e.g. "application/pdf", "text/plain")
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrGetFileInput {
    pub file_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrSearchFilesInput {
    /// Search query (e.g. "name contains 'report'")
    pub query: String,
    /// Max results (default 20, max 100)
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrListFoldersInput {
    /// Max results (default 50)
    pub max_results: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrGetFileMetadataInput {
    pub file_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DrExportFileInput {
    pub file_id: String,
    /// Target MIME type (e.g. "application/pdf", "text/plain")
    pub mime_type: String,
}

// ══════════════════════════════════════════════════════════════
// GOOGLE DRIVE — Tool Handlers
// ══════════════════════════════════════════════════════════════

pub async fn handle_goog_list_files(
    state: &AppState,
    input: &DrListFilesInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let max_results = input.max_results.unwrap_or(20);
    let result = provider
        .list_files(&token, max_results, input.mime_type.as_deref())
        .await
        .map_err(|e| format!("Drive list files failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_file(
    state: &AppState,
    input: &DrGetFileInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let result = provider
        .get_file(&token, &input.file_id)
        .await
        .map_err(|e| format!("Drive get file failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_search_files(
    state: &AppState,
    input: &DrSearchFilesInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let max_results = input.max_results.unwrap_or(20);
    let result = provider
        .search_files(&token, &input.query, max_results)
        .await
        .map_err(|e| format!("Drive search files failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_list_folders(
    state: &AppState,
    input: &DrListFoldersInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let max_results = input.max_results.unwrap_or(50);
    let result = provider
        .list_folders(&token, max_results)
        .await
        .map_err(|e| format!("Drive list folders failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_get_file_metadata(
    state: &AppState,
    input: &DrGetFileMetadataInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let result = provider
        .get_file_metadata(&token, &input.file_id)
        .await
        .map_err(|e| format!("Drive get file metadata failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}

pub async fn handle_goog_export_file(
    state: &AppState,
    input: &DrExportFileInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let token = find_drive_token(state, user_id).await?;
    let provider = drv(state);
    let result = provider
        .export_file(&token, &input.file_id, &input.mime_type)
        .await
        .map_err(|e| format!("Drive export file failed: {e}"))?;
    Ok(Json(serde_json::json!({ "data": result })))
}
