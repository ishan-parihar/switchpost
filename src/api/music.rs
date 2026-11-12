// ─── Music API Routes ────────────────────────────────────────
// Instagram Audio API integration — search trending music and
// original sounds, attach to Reels at publish time.
//
// Per Meta's docs (Instagram Platform, Content Publishing → Audio API):
//   GET /ig_audio?audio_type={music|original_sound}&user_id={ig-user-id}
//     [&search_query=...]   with a USER access token
// Response: { "audio": [ { "audio_id", "title", "display_artist",
//   "cover_artwork_thumbnail_uri", "duration_in_ms", ... } ] }
// The API is available with Facebook Login (and Unified Login) but NOT
// with Instagram Login (graph.instagram.com) — see search_music.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::AuthenticatedUser;
use crate::error::AppError;
use crate::social::GRAPH_VERSION;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct MusicSearchQuery {
    pub q: Option<String>,
    /// "music" or "original_sound". Default: "music"
    pub audio_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MusicSearchResponse {
    pub tracks: Vec<MusicTrack>,
}

#[derive(Debug, Serialize)]
pub struct MusicTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub audio_type: String,
}

/// GET /api/integrations/:id/music?q=...
/// Search Instagram's audio library for music or original sounds.
/// If q is omitted, returns trending tracks.
pub async fn search_music(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<MusicSearchQuery>,
    auth: AuthenticatedUser,
) -> Result<Json<MusicSearchResponse>, AppError> {
    let integration = crate::db::queries::get_integration(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    // Only Instagram supports the Audio API
    if integration.provider_identifier != "instagram"
        && integration.provider_identifier != "instagram-standalone"
    {
        return Err(AppError::BadRequest(
            "Music search is only available for Instagram integrations".into(),
        ));
    }

    // The Audio API is NOT available on the Instagram API with Instagram
    // Login (graph.instagram.com) — which is what instagram-standalone uses.
    // It requires an IG Business/Creator account via Facebook Login, so fail
    // here with the actionable message instead of calling an endpoint that
    // always 4xx-es.
    if integration.provider_identifier == "instagram-standalone" {
        return Err(AppError::BadRequest(
            "Music search requires an Instagram Business/Creator account connected via \
             Facebook Login. Reconnect this account using the \"Instagram\" channel \
             (Facebook Login) — the standalone (Instagram Login) channel cannot search \
             the audio library."
                .into(),
        ));
    }

    let audio_type = query.audio_type.as_deref().unwrap_or("music");
    if audio_type != "music" && audio_type != "original_sound" {
        return Err(AppError::BadRequest(format!(
            "audio_type must be \"music\" or \"original_sound\", got \"{audio_type}\""
        )));
    }

    // The Audio API requires a USER access token. A connected page
    // sub-account stores the PAGE token in access_token and the parent's
    // user-level token in refresh_token (connect_page copies it there) —
    // the same discovery available_pages uses.
    let raw_token = integration
        .refresh_token
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(&integration.access_token);
    let token = crate::crypto::maybe_decrypt_token(raw_token, state.token_key.as_ref());

    let ig_id = resolve_ig_user_id(&integration.internal_id, &token).await?;

    // GET /ig_audio?audio_type=..&user_id=..&search_query=.. — the token goes
    // in the Authorization header, not the URL (docs sample shows it as a
    // query param, but the header keeps it out of logs).
    let mut url = format!(
        "https://graph.facebook.com/{GRAPH_VERSION}/ig_audio?audio_type={audio_type}&user_id={ig_id}"
    );
    if let Some(ref q) = query.q {
        if !q.is_empty() {
            url.push_str(&format!("&search_query={}", urlencoding::encode(q)));
        }
    }

    let client = crate::social::shared_http();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("IG Audio API request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        // Surface Meta's own message: AppError::Internal would replace it
        // with a generic "Internal error" body and the picker would only show
        // its static note.
        let api_msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|j| j["error"]["message"].as_str().map(String::from))
            .unwrap_or_else(|| body.chars().take(300).collect());
        tracing::warn!("IG Audio API returned {status}: {body}");
        return Err(AppError::Provider(format!(
            "Instagram Audio API returned HTTP {status}: {api_msg}. Music search requires an \
             Instagram Business/Creator account connected to a Facebook Page via Facebook Login."
        )));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse IG Audio response: {e}")))?;

    // Documented shape is { "audio": [...] }; accept "data" as a fallback.
    let tracks = parse_audio_tracks(&json, audio_type);

    Ok(Json(MusicSearchResponse { tracks }))
}

/// Parse the `/ig_audio` response into tracks.
///
/// The documented shape (Meta, Content Publishing → Audio API) is
/// `{ "audio": [ { "audio_id", "title", "display_artist",
///   "cover_artwork_thumbnail_uri", "duration_in_ms" } ] }`. The old code
/// read `data`/`id`/`artist`/`cover_artwork_uri`, so a real response parsed
/// to ZERO tracks and the picker showed "No tracks found" even on success.
/// Both shapes are accepted here, documented fields first.
fn parse_audio_tracks(json: &serde_json::Value, audio_type: &str) -> Vec<MusicTrack> {
    json["audio"]
        .as_array()
        .or_else(|| json["data"].as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let id = item["audio_id"]
                        .as_str()
                        .or_else(|| item["id"].as_str())?
                        .to_string();
                    let title = item["title"].as_str().unwrap_or("Unknown").to_string();
                    let artist = item["display_artist"]
                        .as_str()
                        .or_else(|| item["artist"].as_str())
                        .or_else(|| item["owner"]["username"].as_str())
                        .or_else(|| item["ig_username"].as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let cover_url = item["cover_artwork_thumbnail_uri"]
                        .as_str()
                        .or_else(|| item["cover_artwork_uri"].as_str())
                        .or_else(|| item["display_image_uri"].as_str())
                        .map(String::from);
                    let duration_ms = item["duration_in_ms"].as_u64();
                    Some(MusicTrack {
                        id,
                        title,
                        artist,
                        cover_url,
                        duration_ms,
                        audio_type: audio_type.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the IG user id the Audio API wants in `user_id=`.
///
/// With a user token, `/me/accounts` lists the Pages and each page's inline
/// `instagram_business_account` gives the IG id — prefer the page whose IG
/// id matches the integration's `internal_id` (a connected sub-account's
/// internal_id IS the IG id). If the listing yields nothing (token without
/// page list, or the sub-account case), fall back to `internal_id` itself —
/// the `/ig_audio` call validates it and its error message is surfaced.
async fn resolve_ig_user_id(internal_id: &str, token: &str) -> Result<String, AppError> {
    let client = crate::social::shared_http();
    let url = format!(
        "https://graph.facebook.com/{GRAPH_VERSION}/me/accounts?fields=id,instagram_business_account&limit=100"
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to list Facebook pages: {e}")))?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse pages response: {e}")))?;

    if let Some(pages) = json["data"].as_array() {
        let mut first: Option<String> = None;
        for page in pages {
            if let Some(ig) = page["instagram_business_account"]["id"].as_str() {
                if !internal_id.is_empty() && ig == internal_id {
                    return Ok(ig.to_string());
                }
                if first.is_none() {
                    first = Some(ig.to_string());
                }
            }
        }
        if let Some(id) = first {
            return Ok(id);
        }
    }

    if !internal_id.is_empty() {
        // Connected sub-account: internal_id was minted as the IG account id
        // by InstagramProvider::pages().
        return Ok(internal_id.to_string());
    }

    Err(AppError::Provider(
        "Could not resolve an Instagram Business account for this channel. Reconnect the \
         Instagram channel (Facebook Login) and pick the page linked to your IG account."
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented /ig_audio response shape: { "audio": [...] } with
    /// audio_id / display_artist / cover_artwork_thumbnail_uri. The old
    /// parser read data/id/artist and got zero tracks from this payload.
    #[test]
    fn parses_documented_audio_array_shape() {
        let json = serde_json::json!({
            "audio": [
                {
                    "audio_id": "1234567890",
                    "title": "Birthday Wish",
                    "display_artist": "Shuba",
                    "cover_artwork_thumbnail_uri": "https://scontent.example/cover.jpg",
                    "duration_in_ms": 153760,
                    "audio_type": "music"
                },
                {
                    "audio_id": "987654321",
                    "title": "Original Sound",
                    "ig_username": "creator_handle",
                    "duration_in_ms": 30000,
                    "audio_type": "original_sound"
                }
            ]
        });
        let tracks = parse_audio_tracks(&json, "music");
        assert_eq!(tracks.len(), 2, "both audio entries must parse");
        assert_eq!(tracks[0].id, "1234567890");
        assert_eq!(tracks[0].artist, "Shuba");
        assert_eq!(
            tracks[0].cover_url.as_deref(),
            Some("https://scontent.example/cover.jpg")
        );
        assert_eq!(tracks[0].duration_ms, Some(153760));
        assert_eq!(tracks[1].artist, "creator_handle");
        assert_eq!(tracks[1].cover_url, None);
    }

    /// Entries without any id are dropped, not turned into "Unknown" rows.
    #[test]
    fn drops_audio_entries_without_id() {
        let json = serde_json::json!({ "audio": [ { "title": "no id" }, { "audio_id": "1" } ] });
        let tracks = parse_audio_tracks(&json, "original_sound");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].audio_type, "original_sound");
    }

    /// A response with neither "audio" nor "data" yields an empty list, not
    /// an error — the picker shows "No tracks found".
    #[test]
    fn empty_response_yields_empty_tracks() {
        assert!(parse_audio_tracks(&serde_json::json!({}), "music").is_empty());
        assert!(parse_audio_tracks(&serde_json::json!({ "audio": [] }), "music").is_empty());
    }
}
