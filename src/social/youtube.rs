// ─── YouTube Provider (Stub) ───────────────────────────────────
// Uses Google OAuth 2.0 + YouTube Data API v3.
// Full implementation requires: OAuth flow, channel selection, video upload.
// Current state: Basic auth flow + info retrieval.

use async_trait::async_trait;

use super::*;
use crate::config::Config;

/// Read one entry of a `commentThreads`/`replies.comments` list — both share
/// the same `{ snippet: { commentId, authorDisplayName, textDisplay, … } }`.
fn youtube_comment(item: &serde_json::Value) -> Option<CommentData> {
    let snippet = item.get("snippet")?;
    Some(CommentData {
        id: snippet.get("commentId")?.as_str()?.to_string(),
        author_name: snippet
            .get("authorDisplayName")
            .and_then(|v| v.as_str())
            .map(String::from),
        author_avatar: snippet
            .pointer("/authorProfileImageUrl")
            .and_then(|v| v.as_str())
            .map(String::from),
        text: snippet
            .get("textDisplay")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        created_at: snippet
            .get("publishedAt")
            .and_then(|v| v.as_str())
            .map(crate::social::common::parse_timestamp)
            .unwrap_or_else(chrono::Utc::now),
        like_count: snippet
            .get("likeCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        replies: vec![],
    })
}

pub struct YoutubeProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl YoutubeProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("youtube").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    /// Map a `commentThreads` response into comments. The Data API wraps every
    /// top-level comment in a thread under `snippet.topLevelComment`, and its
    /// replies under `replies.comments`; both carry their own `snippet`.
    pub(crate) fn parse_comment_threads(json: &serde_json::Value) -> Vec<CommentData> {
        json.get("items")
            .and_then(|i| i.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|thread| {
                        let mut comment =
                            youtube_comment(thread.pointer("/snippet/topLevelComment")?)?;
                        comment.replies = thread
                            .pointer("/snippet/replies/comments")
                            .and_then(|r| r.as_array())
                            .map(|replies| replies.iter().filter_map(youtube_comment).collect())
                            .unwrap_or_default();
                        Some(comment)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Map a `search?type=channel` response into @-autocomplete candidates.
    pub(crate) fn parse_channel_search(json: &serde_json::Value) -> Vec<MentionResult> {
        json.get("items")
            .and_then(|i| i.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(MentionResult {
                            id: item.pointer("/id/channelId")?.as_str()?.to_string(),
                            label: item
                                .pointer("/snippet/title")?
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            image: item
                                .pointer("/snippet/thumbnails/default/url")
                                .and_then(|u| u.as_str())
                                .map(String::from),
                            do_not_cache: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn search_videos(
        &self,
        access_token: &str,
        query: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 50).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/search")
            .query(&[
                ("part", "snippet"),
                ("q", query),
                ("maxResults", &mr),
                ("type", "video"),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// `GET /youtube/v3/commentThreads` for a video.
    pub async fn get_comment_threads(
        &self,
        access_token: &str,
        video_id: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let max = max_results.clamp(1, 100).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/commentThreads")
            .query(&[
                ("part", "snippet"),
                ("videoId", video_id),
                ("textFormat", "plainText"),
                ("maxResults", &max),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("YouTube commentThreads failed")
                    .to_string(),
            ))
        }
    }

    pub async fn get_video(
        &self,
        access_token: &str,
        video_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/videos")
            .query(&[
                ("part", "snippet,statistics,contentDetails"),
                ("id", video_id),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_playlists(
        &self,
        access_token: &str,
        channel_id: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 50).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/playlists")
            .query(&[
                ("part", "snippet,contentDetails"),
                ("channelId", channel_id),
                ("maxResults", &mr),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_playlist_items(
        &self,
        access_token: &str,
        playlist_id: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 50).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/playlistItems")
            .query(&[
                ("part", "snippet"),
                ("playlistId", playlist_id),
                ("maxResults", &mr),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_comments(
        &self,
        access_token: &str,
        video_id: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 100).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/commentThreads")
            .query(&[
                ("part", "snippet"),
                ("videoId", video_id),
                ("maxResults", &mr),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_channel_stats(
        &self,
        access_token: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/channels")
            .query(&[
                ("part", "snippet,statistics"),
                ("id", channel_id),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_analytics(
        &self,
        access_token: &str,
        channel_id: &str,
        metrics: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let ids = format!("channel=={}", channel_id);
        let resp = self
            .http
            .get("https://youtubeanalytics.googleapis.com/v2/reports")
            .query(&[
                ("ids", ids.as_str()),
                ("metrics", metrics),
                ("startDate", start_date),
                ("endDate", end_date),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_subscriptions(
        &self,
        access_token: &str,
        channel_id: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 50).to_string();
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/subscriptions")
            .query(&[
                ("part", "snippet"),
                ("channelId", channel_id),
                ("maxResults", &mr),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Search for creators on a topic. Searches videos by query, groups by channel,
    /// and enriches each channel with subscriber count, email from description.
    pub async fn find_creators(
        &self,
        access_token: &str,
        query: &str,
        min_subscribers: Option<u32>,
        max_results: Option<u32>,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = max_results.unwrap_or(10).min(50);
        let encoded_query: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let search_url = format!(
            "https://www.googleapis.com/youtube/v3/search?part=snippet&q={}&type=video&maxResults={}&access_token={}",
            encoded_query, limit, access_token
        );
        let search_resp = self
            .http
            .get(&search_url)
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let search_status = search_resp.status();
        let search_text = search_resp
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        if !search_status.is_success() {
            let v: serde_json::Value = serde_json::from_str(&search_text).unwrap_or_default();
            return Err(ProviderError::Api(
                v["error"]["message"]
                    .as_str()
                    .unwrap_or(&search_text)
                    .into(),
            ));
        }
        let search_data: serde_json::Value = serde_json::from_str(&search_text).unwrap_or_default();

        let mut channel_ids: Vec<String> = Vec::new();
        if let Some(items) = search_data["items"].as_array() {
            for item in items {
                if let Some(ch_id) = item["snippet"]["channelId"].as_str() {
                    let id = ch_id.to_string();
                    if !channel_ids.contains(&id) {
                        channel_ids.push(id);
                    }
                }
            }
        }

        if channel_ids.is_empty() {
            return Ok(serde_json::json!({"creators": [], "total_videos": 0}));
        }

        let ids_param = channel_ids.join(",");
        let stats_url = format!(
            "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&access_token={}",
            ids_param, access_token
        );
        let stats_resp = self
            .http
            .get(&stats_url)
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let stats_status = stats_resp.status();
        let stats_text = stats_resp
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        if !stats_status.is_success() {
            let v: serde_json::Value = serde_json::from_str(&stats_text).unwrap_or_default();
            return Err(ProviderError::Api(
                v["error"]["message"].as_str().unwrap_or(&stats_text).into(),
            ));
        }
        let stats_data: serde_json::Value = serde_json::from_str(&stats_text).unwrap_or_default();

        let min_subs = min_subscribers.unwrap_or(0) as i64;
        let mut creators = Vec::new();
        if let Some(channels) = stats_data["items"].as_array() {
            for ch in channels {
                let sub_count: i64 = ch["statistics"]["subscriberCount"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if sub_count >= min_subs {
                    let description = ch["snippet"]["description"].as_str().unwrap_or("");
                    let email = description
                        .split_whitespace()
                        .find(|w| w.contains('@') && w.contains('.'))
                        .map(|e| e.trim_end_matches('.').trim_end_matches(',').to_string());
                    creators.push(serde_json::json!({
                        "channel_id": ch["id"],
                        "title": ch["snippet"]["title"],
                        "description": ch["snippet"]["description"],
                        "subscriber_count": sub_count,
                        "video_count": ch["statistics"]["videoCount"],
                        "view_count": ch["statistics"]["viewCount"],
                        "thumbnail": ch["snippet"]["thumbnails"]["default"]["url"],
                        "email": email,
                        "country": ch["snippet"]["country"],
                        "published_at": ch["snippet"]["publishedAt"]
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "creators": creators,
            "total_videos": search_data["pageInfo"]["totalResults"],
            "query": query
        }))
    }
}

impl YoutubeProvider {
    /// Resolve the channel to report on: the integration's bound channel when the
    /// caller knows it, otherwise the first channel the token's user owns.
    async fn resolve_channel_id(
        &self,
        access_token: &str,
        internal_id: &str,
    ) -> Result<Option<String>, ProviderError> {
        if !internal_id.is_empty() {
            return Ok(Some(internal_id.to_string()));
        }
        Ok(self
            .pages(access_token)
            .await?
            .first()
            .map(|c| c.id.clone()))
    }

    /// `statistics` sub-object of the first item in a Data API list response.
    fn first_statistics(json: &serde_json::Value) -> Option<&serde_json::Value> {
        json["items"].as_array()?.first().map(|i| &i["statistics"])
    }
}

// ── C1: the one resumable-upload path for every YouTube publisher ────
// google.rs (Google Suite) and this module carried byte-identical flows
// until the 2026-10-17 consolidation: init (metadata JSON) → Location →
// single PUT of the video bytes → the created video's JSON.

/// Upload one video through YouTube's resumable-upload session endpoint.
/// `metadata` is the snippet/status object; returns the created video's
/// JSON (callers extract `id`).
pub(crate) async fn resumable_upload(
    http: &reqwest::Client,
    access_token: &str,
    metadata: &serde_json::Value,
    video_bytes: Vec<u8>,
    mime_type: &str,
) -> Result<serde_json::Value, ProviderError> {
    let init_resp = http
        .post(
            "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status",
        )
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("X-Upload-Content-Type", mime_type)
        .header(
            "X-Upload-Content-Length",
            video_bytes.len().to_string(),
        )
        .body(
            serde_json::to_string(metadata)
                .map_err(|e| ProviderError::Api(e.to_string()))?,
        )
        .send()
        .await?;

    let init_status = init_resp.status();
    if init_status == 401 {
        return Err(ProviderError::TokenExpired);
    }
    if !init_status.is_success() {
        let err_body: serde_json::Value = init_resp.json().await.unwrap_or_default();
        return Err(upload_step_error(
            init_status,
            &err_body,
            "Failed to initialize resumable upload",
        ));
    }

    let upload_url = init_resp
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| {
            ProviderError::Api("No Location header in resumable upload response".into())
        })?
        .to_str()
        .map_err(|e| ProviderError::Api(format!("Invalid Location header: {e}")))?
        .to_string();

    let upload_resp = http
        .put(&upload_url)
        .header("Content-Type", mime_type)
        .header("Content-Length", video_bytes.len().to_string())
        .body(video_bytes)
        .send()
        .await?;

    let upload_status = upload_resp.status();
    if upload_status == 401 {
        return Err(ProviderError::TokenExpired);
    }
    if !upload_status.is_success() {
        let err_body: serde_json::Value = upload_resp.json().await.unwrap_or_default();
        return Err(upload_step_error(
            upload_status,
            &err_body,
            "Failed to upload video to YouTube",
        ));
    }

    Ok(upload_resp.json().await?)
}

/// One error shape for both upload steps: reason + message from the error
/// body, with a status hint prepended for the two operator-actionable ones.
fn upload_step_error(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
    fallback: &str,
) -> ProviderError {
    let msg = body["error"]["message"].as_str().unwrap_or(fallback);
    let reason = body["error"]["errors"]
        .as_array()
        .and_then(|e| e.first())
        .and_then(|e| e["reason"].as_str())
        .unwrap_or_default();
    let raw = format!("YouTube upload error (HTTP {status}, {reason}): {msg}");
    let hint = match status.as_u16() {
        403 => "YouTube API access forbidden. Check token scopes.",
        429 => "YouTube API rate limit exceeded. Try again later.",
        _ => return ProviderError::Api(raw),
    };
    ProviderError::Api(friendly_error(Some(hint.into()), &raw))
}

#[async_trait]
impl SocialProvider for YoutubeProvider {
    fn identifier(&self) -> &'static str {
        "youtube"
    }

    fn name(&self) -> &'static str {
        "YouTube"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "https://www.googleapis.com/auth/youtube".into(),
            "https://www.googleapis.com/auth/youtube.upload".into(),
            "https://www.googleapis.com/auth/youtube.force-ssl".into(),
            "https://www.googleapis.com/auth/userinfo.profile".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        5000
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    fn is_between_steps(&self) -> bool {
        true
    }

    /// Google access tokens live only ~1 hour, and YouTube uploads are the
    /// only way to publish on this provider — an expired token silently kills
    /// every scheduled video, so refresh ahead of the 24h scheduler window.
    fn needs_cron_refresh(&self) -> bool {
        true
    }

    /// YouTube-specific user-facing error copy (v25 §2 row 9).
    ///
    /// The two Google quota arms come first: `classify_error` cannot tell
    /// "you used your daily upload quota" from "your access token died", and
    /// the fix for each is completely different. Google spells the reason
    /// token two ways across `error.reason` and `error.message`, so match
    /// case-insensitively on both the camelCase and spaced forms.
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        let lower = body.to_ascii_lowercase();
        if lower.contains("uploadlimitexceeded") || lower.contains("daily upload limit") {
            return Some(
                "YouTube daily upload quota reached. The limit resets 24 hours after your \
                 first upload — schedule the video for tomorrow or use a project that does \
                 not count against the daily cap."
                    .into(),
            );
        }
        if lower.contains("quotaexceeded")
            || lower.contains("quota exceeded")
            || lower.contains("dailylimitexceeded")
            || lower.contains("daily limit")
        {
            return Some(
                "YouTube API quota exceeded. The default daily allowance is 10 000 units and \
                 a video upload costs 1600 — wait for the daily reset or request more quota."
                    .into(),
            );
        }
        if lower.contains("forbidden") {
            return Some(
                "YouTube denied this request. The OAuth client needs the \
                 youtube.upload scope and the channel must be linked to a YouTube Brand Account \
                 that accepts uploads."
                    .into(),
            );
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "YouTube rate limit hit (HTTP 429). Back off and retry — Forge already retries \
                 with backoff, so this resolves itself on the next attempt."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "YouTube rejected the video. MP4/MOV up to 256 GB or 12 hours are accepted, \
                 but the file must be fetched over HTTP and pass YouTube's Content ID check."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Google access token expired or was revoked. YouTube tokens last ~1 hour and \
                 Forge refreshes them automatically — reconnect the channel if refresh fails."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "YouTube denied this request (HTTP 403). The token is missing the \
                 youtube.upload scope, or the channel is not set up to accept uploads."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "YouTube video or channel not found — it may be private, deleted, or the \
                 integration points at a channel you no longer own."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "YouTube rejected the request as invalid. Check the title (100 chars) and \
                 description (5000 chars) limits, and that a video file is attached."
                    .into(),
            ),
            _ => None,
        }
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        // Google OAuth 2.0
        let scope = self.scopes().join(" ");
        let params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", scope.as_str()),
            ("state", state),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ];

        let url =
            url::Url::parse_with_params("https://accounts.google.com/o/oauth2/v2/auth", &params)
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
        super::google_oauth::exchange_code(
            &self.http,
            "youtube",
            &self.client_id,
            &self.client_secret,
            code,
            redirect_uri,
        )
        .await
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        super::google_oauth::refresh(
            &self.http,
            &self.client_id,
            &self.client_secret,
            refresh_token,
        )
        .await
    }

    /// List channels for page selection
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/channels")
            .query(&[
                ("part", "snippet"),
                ("mine", "true"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let items = json["items"].as_array().cloned().unwrap_or_default();

        Ok(items
            .iter()
            .map(|item| PageInfo {
                id: item["id"].as_str().unwrap_or("").to_string(),
                name: item["snippet"]["title"].as_str().unwrap_or("").to_string(),
                access_token: Some(access_token.to_string()),
                picture: item["snippet"]["thumbnails"]["default"]["url"]
                    .as_str()
                    .map(String::from),
                username: item["snippet"]["customUrl"].as_str().map(String::from),
            })
            .collect())
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/channels")
            .query(&[
                ("part", "snippet"),
                ("id", page_id),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(item) = json["items"].as_array().and_then(|a| a.first()) {
            Ok(PageInfo {
                id: item["id"].as_str().unwrap_or("").to_string(),
                name: item["snippet"]["title"].as_str().unwrap_or("").to_string(),
                access_token: Some(access_token.to_string()),
                picture: item["snippet"]["thumbnails"]["default"]["url"]
                    .as_str()
                    .map(String::from),
                username: item["snippet"]["customUrl"].as_str().map(String::from),
            })
        } else {
            Err(ProviderError::Api("YouTube channel not found".into()))
        }
    }

    async fn reconnect(
        &self,
        access_token: &str,
        _internal_id: &str,
        page_id: &str,
    ) -> Result<ReconnectResult, ProviderError> {
        let info = self.fetch_page_info(access_token, page_id).await?;
        Ok(ReconnectResult {
            id: info.id,
            name: info.name,
            access_token: info.access_token.unwrap_or_default(),
            picture: info.picture,
            username: info.username,
        })
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        // Get the user's channel
        let channels = self.pages(access_token).await?;
        if channels.is_empty() {
            return Ok(vec![]);
        }
        let channel_id = &channels[0].id;
        let max_results = limit.min(50).to_string();

        // Search for the channel's recent uploads
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/search")
            .query(&[
                ("part", "snippet"),
                ("channelId", channel_id.as_str()),
                ("maxResults", &max_results),
                ("order", "date"),
                ("type", "video"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let mut posts = Vec::new();

        if let Some(items) = json["items"].as_array() {
            for item in items {
                let snippet = &item["snippet"];
                let video_id = item["id"]["videoId"].as_str().unwrap_or("");
                let title = snippet["title"].as_str().unwrap_or("");
                let description = snippet["description"].as_str().unwrap_or("");
                let published_at = snippet["publishedAt"].as_str().unwrap_or("");
                let thumb = snippet["thumbnails"]["default"]["url"]
                    .as_str()
                    .map(|s| s.to_string());

                let posted_at = crate::social::common::parse_timestamp(published_at);

                // Single media item: embed URL for iframe playback
                // Thumbnail URL is stored in metadata.poster_url so frontend can show it as placeholder
                let embed_url = format!("https://www.youtube.com/embed/{video_id}");
                let meta = serde_json::json!({
                    "title": title,
                    "poster_url": thumb,
                });

                posts.push(ExternalPostData {
                    platform_post_id: video_id.to_string(),
                    text: description.to_string(),
                    author_name: None,
                    author_handle: None,
                    author_avatar: None,
                    media: vec![MediaAttachment {
                        url: embed_url,
                        mime_type: "text/html".into(),
                        alt: Some(title.to_string()),
                        poster_url: None,
                    }],
                    created_at: posted_at,
                    url: Some(format!("https://www.youtube.com/watch?v={video_id}")),
                    metadata: Some(meta),
                });
            }
        }

        Ok(posts)
    }

    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        // get_video() returns statistics with viewCount, likeCount, commentCount
        let json = self.get_video(access_token, platform_post_id).await?;
        if let Some(items) = json["items"].as_array() {
            if let Some(video) = items.first() {
                if let Some(stats) = video["statistics"].as_object() {
                    let result = serde_json::json!({
                        "viewCount": stats.get("viewCount").and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0),
                        "likeCount": stats.get("likeCount").and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0),
                        "commentCount": stats.get("commentCount").and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0),
                    });
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }

    /// Channel-level analytics for the dashboard.
    ///
    /// The YouTube Data API exposes only lifetime counters under the scopes this
    /// provider holds (`youtube`, `youtube.upload`, `youtube.force-ssl`); a real
    /// per-day range needs `yt-analytics.readonly`, which is deliberately not
    /// requested. So every series is a single point dated today — the same shape
    /// X returns — and `days` carries no meaning.
    async fn analytics(
        &self,
        access_token: &str,
        internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let Some(channel_id) = self.resolve_channel_id(access_token, internal_id).await? else {
            return Ok(vec![]);
        };
        let json = self.get_channel_stats(access_token, &channel_id).await?;
        let Some(stats) = Self::first_statistics(&json) else {
            return Ok(vec![]);
        };
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Ok(metrics_series(
            stats,
            &today,
            &[
                ("Subscribers", "subscriberCount"),
                ("Total Views", "viewCount"),
                ("Videos", "videoCount"),
            ],
        ))
    }

    /// Per-video analytics. The Data API serves lifetime counts only, so views /
    /// likes / comments come back as a single point dated today.
    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let json = self.get_video(access_token, platform_post_id).await?;
        let Some(stats) = Self::first_statistics(&json) else {
            return Ok(vec![]);
        };
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Ok(metrics_series(
            stats,
            &today,
            &[
                ("Views", "viewCount"),
                ("Likes", "likeCount"),
                ("Comments", "commentCount"),
            ],
        ))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        if post.media.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "YouTube video upload requires at least one media attachment (video file).".into(),
            ));
        }

        let media = &post.media[0];

        let video_resp = self.http.get(&media.url).send().await?;
        if !video_resp.status().is_success() {
            return Err(ProviderError::Api(format!(
                "Failed to download video from media URL: HTTP {}",
                video_resp.status()
            )));
        }
        let video_bytes = video_resp.bytes().await?.to_vec();

        let mime_type = if media.mime_type.is_empty() {
            "video/*"
        } else {
            &media.mime_type
        };

        // The title.
        //
        // The composer's YouTube spec has `titleRequired: true`, so
        // the founder IS forced to type a title - and it was thrown
        // away: the video was titled with the first 100 characters of
        // the body. The UI demanded a value the server discarded.
        //
        // So the title is read from the settings, and the body is only
        // the fallback for a post that has no title at all.
        //
        // YouTube's title max is 100 characters.
        let title: String = post
            .settings
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| post.content.chars().take(100).collect());
        let description = &post.content;

        let privacy_status = post
            .settings
            .get("privacyStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("private");

        let metadata = serde_json::json!({
            "snippet": {
                "title": title,
                "description": description,
            },
            "status": {
                "privacyStatus": privacy_status,
            }
        });

        // C1: one shared resumable-upload path (see resumable_upload above).
        let json =
            resumable_upload(&self.http, access_token, &metadata, video_bytes, mime_type).await?;
        let video_id = json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("YouTube did not return a video ID".into()))?;

        Ok(PublishResult {
            platform_post_id: video_id.to_string(),
            platform_post_url: Some(format!("https://www.youtube.com/watch?v={video_id}")),
            status: "published".into(),
        })
    }

    /// Top-level comments for a video via `commentThreads`; the first reply of
    /// each thread is nested, the rest stay in the thread (YouTube returns a
    /// flat `replies.comments` list).
    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_comment_threads(access_token, platform_post_id, 100)
            .await?;
        Ok(Self::parse_comment_threads(&json))
    }

    /// @-autocomplete over channel search — the only member list the YouTube
    /// Data API exposes, and what `@handle` resolves to in descriptions.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let resp = self
            .http
            .get("https://youtube.googleapis.com/youtube/v3/search")
            .query(&[
                ("part", "snippet"),
                ("q", query),
                ("type", "channel"),
                ("maxResults", "25"),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(Self::parse_channel_search(&json))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("YouTube channel search failed")
                    .to_string(),
            ))
        }
    }

    /// Channels are the posting targets, same listing as `pages()`.
    async fn targets(&self, access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        Ok(super::pages_to_targets(
            &self.pages(access_token).await?,
            "channel",
        ))
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "snippet": {
                "parentId": comment_id,
                "textOriginal": post.content,
            }
        });
        let resp = self
            .http
            .post("https://youtube.googleapis.com/youtube/v3/comments")
            .query(&[("part", "snippet")])
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            let reply_id = json["id"].as_str().unwrap_or("").to_string();
            Ok(PublishResult {
                platform_post_id: reply_id,
                platform_post_url: None,
                status: "published".into(),
            })
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("YouTube reply failed")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::parse_engagement_data;

    #[test]
    fn parse_should_map_youtube_string_encoded_statistics() {
        // The Data API encodes statistics as strings, not numbers.
        let raw = serde_json::json!({
            "viewCount": "1200",
            "likeCount": "42",
            "commentCount": "12"
        });
        let e = parse_engagement_data("youtube", raw);
        assert_eq!(e.views, 1200);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
    }

    #[test]
    fn parse_should_unwrap_youtube_statistics_object() {
        let raw = serde_json::json!({
            "id": "dQw4w9WgXcQ",
            "statistics": { "viewCount": 7, "likeCount": 3, "commentCount": 1 }
        });
        let e = parse_engagement_data("youtube", raw);
        assert_eq!(e.views, 7);
        assert_eq!(e.likes, 3);
        assert_eq!(e.comments, 1);
    }

    #[test]
    fn parse_should_default_missing_youtube_metrics_to_zero() {
        let e = parse_engagement_data("youtube", serde_json::json!({ "viewCount": "9" }));
        assert_eq!(e.views, 9);
        assert_eq!(e.likes, 0);
        assert_eq!(e.comments, 0);
    }

    #[test]
    fn metrics_series_should_skip_absent_keys() {
        let stats = serde_json::json!({ "subscriberCount": "128", "viewCount": "9" });
        let series = metrics_series(
            &stats,
            "2026-09-30",
            &[
                ("Subscribers", "subscriberCount"),
                ("Total Views", "viewCount"),
                ("Videos", "videoCount"),
            ],
        );
        assert_eq!(
            series.len(),
            2,
            "missing videoCount must be skipped, not zero-filled"
        );
        assert_eq!(series[0].label, "Subscribers");
        assert_eq!(series[0].data[0].total, "128");
        assert_eq!(series[1].data[0].date, "2026-09-30");
    }

    #[test]
    fn first_statistics_should_return_none_for_empty_list() {
        let json = serde_json::json!({ "items": [] });
        assert!(YoutubeProvider::first_statistics(&json).is_none());
    }

    // ── B4: needs_cron_refresh + map_error fixtures ────────

    fn provider() -> YoutubeProvider {
        YoutubeProvider::new(&crate::social::test_config())
    }

    #[test]
    fn youtube_requests_cron_refresh() {
        assert!(provider().needs_cron_refresh());
    }

    #[test]
    fn map_error_explains_upload_limit() {
        let msg = provider()
            .map_error("The uploadLimitExceeded quota was exceeded", 403)
            .unwrap();
        assert!(msg.contains("daily upload quota"), "got: {msg}");
    }

    #[test]
    fn map_error_treats_spaced_quota_message_as_quota_not_scope() {
        // Google writes the same failure two ways: a camelCase `error.reason`
        // token and a spaced `error.message`. Both must reach the quota arm,
        // never the missing-scope arm — the fixes are completely different.
        let msg = provider()
            .map_error("The daily upload limit has been reached.", 403)
            .unwrap();
        assert!(msg.contains("daily upload quota"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_api_quota() {
        let msg = provider()
            .map_error("Quota exceeded for quota metric", 403)
            .unwrap();
        assert!(msg.contains("API quota exceeded"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_youtube_forbidden() {
        let msg = provider()
            .map_error(
                "The request cannot be completed because you have exceeded the quota for youtube",
                403,
            )
            .unwrap();
        assert!(msg.contains("youtube.upload"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let msg = provider().map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let msg = provider()
            .map_error("The video upload was rejected", 422)
            .unwrap();
        assert!(msg.contains("256 GB"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let msg = provider().map_error("", 401).unwrap();
        assert!(msg.contains("~1 hour"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_upload_scope_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("youtube.upload"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_video_on_404() {
        let msg = provider().map_error("", 404).unwrap();
        assert!(msg.contains("not found"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let msg = provider().map_error("", 400).unwrap();
        assert!(msg.contains("100 chars"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 503).is_none());
    }

    // ── B3: comment threads ───────────────────────────────────

    #[test]
    fn parse_should_unwrap_top_level_comments_and_their_replies() {
        let raw = serde_json::json!({ "items": [
            { "snippet": {
                "topLevelComment": { "snippet": {
                    "commentId": "c1", "textDisplay": "great video", "authorDisplayName": "Ada",
                    "authorProfileImageUrl": "https://img/a.jpg",
                    "publishedAt": "2026-09-28T10:00:00Z", "likeCount": 4 } },
                "replies": { "comments": [
                    { "snippet": { "commentId": "c2", "textDisplay": "thanks", "authorDisplayName": "Bob" } }
                ]}
            }}
        ]});
        let comments = YoutubeProvider::parse_comment_threads(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "c1");
        assert_eq!(comments[0].text, "great video");
        assert_eq!(comments[0].author_name.as_deref(), Some("Ada"));
        assert_eq!(
            comments[0].author_avatar.as_deref(),
            Some("https://img/a.jpg")
        );
        assert_eq!(comments[0].like_count, 4);
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].id, "c2");
    }

    #[test]
    fn parse_should_read_comment_thread_without_replies() {
        let raw = serde_json::json!({ "items": [
            { "snippet": { "topLevelComment": { "snippet": { "commentId": "c1", "textDisplay": "solo" } } } }
        ]});
        let comments = YoutubeProvider::parse_comment_threads(&raw);
        assert_eq!(comments.len(), 1);
        assert!(comments[0].replies.is_empty());
        assert_eq!(comments[0].like_count, 0);
    }

    #[test]
    fn parse_should_read_no_comments_from_empty_page() {
        assert!(YoutubeProvider::parse_comment_threads(&serde_json::json!({})).is_empty());
    }

    // ── B3: channel search ───────────────────────────────────

    #[test]
    fn parse_should_map_channel_search_into_mention_results() {
        let raw = serde_json::json!({ "items": [
            { "id": { "kind": "youtube#channel", "channelId": "UC123" },
              "snippet": { "title": "Rust Lang",
                           "thumbnails": { "default": { "url": "https://img/c.jpg" } } } }
        ]});
        let mentions = YoutubeProvider::parse_channel_search(&raw);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "UC123");
        assert_eq!(mentions[0].label, "Rust Lang");
        assert_eq!(mentions[0].image.as_deref(), Some("https://img/c.jpg"));
    }

    #[test]
    fn parse_should_skip_channel_search_items_without_channel_id() {
        let raw = serde_json::json!({ "items": [ { "snippet": { "title": "orphan" } } ] });
        assert!(YoutubeProvider::parse_channel_search(&raw).is_empty());
    }
}
