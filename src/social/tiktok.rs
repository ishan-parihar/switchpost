// ─── TikTok Provider ────────────────────────────────────────
// Uses TikTok OAuth 2.0 + TikTok Content Posting API.
// Supports: OAuth flow, user info, video upload, video publish.

use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct TikTokProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl TikTokProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("tiktok").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    /// Fetch the authenticated user's profile info.
    pub async fn get_user_info(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://open.tiktokapis.com/v2/user/info/")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown TikTok API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Map a `research/video/comment` response into comments. The API returns
    /// a flat list with `parent_comment_id`, so replies are nested here rather
    /// than by a second request.
    pub(crate) fn parse_research_comments(json: &serde_json::Value) -> Vec<CommentData> {
        let flat: Vec<CommentData> = json
            .pointer("/data/comments")
            .and_then(|c| c.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| {
                        Some(CommentData {
                            id: c["id"].as_str()?.to_string(),
                            author_name: c
                                .pointer("/user/screen_name")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            author_avatar: None,
                            text: c["text"].as_str().unwrap_or_default().to_string(),
                            created_at: c["create_time"]
                                .as_i64()
                                .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                                .unwrap_or_else(chrono::Utc::now),
                            like_count: c["like_count"].as_i64().unwrap_or(0) as i32,
                            replies: vec![],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let parents: std::collections::HashMap<&str, &str> = json
            .pointer("/data/comments")
            .and_then(|c| c.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| Some((c["id"].as_str()?, c["parent_comment_id"].as_str()?)))
                    .collect()
            })
            .unwrap_or_default();

        let mut roots: Vec<CommentData> = Vec::new();
        let mut root_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for comment in flat {
            match parents
                .get(comment.id.as_str())
                .copied()
                .filter(|parent| *parent != comment.id)
                .and_then(|parent| root_index.get(parent))
            {
                Some(&i) => roots[i].replies.push(comment),
                // Parent not in this page — surface the reply at top level
                // rather than dropping it.
                None => {
                    root_index.insert(comment.id.clone(), roots.len());
                    roots.push(comment);
                }
            }
        }
        roots
    }

    /// Find one video's metrics in a `/v2/video/list/` response by id.    /// Find one video's metrics in a `/v2/video/list/` response by id.
    /// The Display API returns videos newest-first, so the first page is where a
    /// just-published post lives.
    fn find_video<'a>(
        list: &'a serde_json::Value,
        video_id: &str,
    ) -> Option<&'a serde_json::Value> {
        list["data"]["videos"]
            .as_array()?
            .iter()
            .find(|v| v["id"].as_str() == Some(video_id))
    }

    /// List the authenticated user's videos.
    pub async fn list_videos(
        &self,
        access_token: &str,
        max_count: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let count = max_count.clamp(1, 100);
        let body = serde_json::json!({
            "max_count": count,
            "cursor": 0,
        });

        let resp = self
            .http
            .post("https://open.tiktokapis.com/v2/video/list/")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown TikTok API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// B11: publish-id status → engine status string.
    fn publish_status_to_state(status: &str) -> &'static str {
        match status {
            "PUBLISH_COMPLETE" => "published",
            "PUBLISH_FAILED" => "failed",
            _ => "pending",
        }
    }

    /// Poll `POST /v2/post/publish/status/fetch/` until TikTok reports a
    /// terminal publish status for `post_id` (see fetch_publish_status).
    /// `video/publish/` is asynchronous: it queues the post and hands back a
    /// `post_id` for something still in flight, so returning `status:
    /// "published"` on submit claims success for uploads TikTok may still
    /// reject (copyright strike, community guidelines, region block, an
    /// unapproved audio track). Bounded to 6 attempts 10s apart (~60s): a
    /// still-processing post surfaces as an API error naming the last observed
    /// status, so the operator knows to check the app rather than repost blind.
    async fn await_publish_status(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<(), ProviderError> {
        const ATTEMPTS: usize = 6;
        const INTERVAL: Duration = Duration::from_secs(10);

        if post_id.is_empty() {
            return Err(ProviderError::Api(
                "TikTok accepted the publish but returned no post_id, so the video's \
                 status cannot be verified. Check the TikTok app before reposting."
                    .into(),
            ));
        }

        let mut last_seen = String::new();

        for attempt in 1..=ATTEMPTS {
            let seen = self.fetch_publish_status(access_token, post_id).await?;
            match seen.as_str() {
                "PUBLISH_COMPLETE" => {
                    tracing::debug!("TikTok video {post_id} live (status fetch #{attempt})");
                    return Ok(());
                }
                "PUBLISH_FAILED" => {
                    let raw = format!("TikTok did not publish video {post_id}: PUBLISH_FAILED");
                    return Err(self.provider_error(&raw, 0));
                }
                other => {
                    last_seen = other.to_string();
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(INTERVAL).await;
                    }
                }
            }
        }

        let raw = format!(
            "TikTok video {post_id} was still {last_seen} after {}s. The submit was \
             accepted, so check the TikTok app before reposting — a second upload \
             of the same video can be rejected as duplicate content.",
            ATTEMPTS as u64 * INTERVAL.as_secs()
        );
        Err(self.provider_error(&raw, 0))
    }

    /// B11: one fetch-status call. Returns the raw publish status string
    /// (e.g. PUBLISH_COMPLETE, PUBLISH_FAILED, or a processing state).
    async fn fetch_publish_status(
        &self,
        access_token: &str,
        publish_id: &str,
    ) -> Result<String, ProviderError> {
        let resp = self
            .http
            .post("https://open.tiktokapis.com/v2/post/publish/status/fetch/")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&serde_json::json!({ "publish_id": publish_id }))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() || json["error"]["code"].as_str() != Some("ok") {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("status fetch failed");
            let code = json["error"]["code"].as_str().unwrap_or("error");
            let raw = format!("TikTok status fetch failed (HTTP {status}, code {code}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(msg, status.as_u16()),
                &raw,
            )));
        }

        Ok(json["data"]["status"]
            .as_str()
            .unwrap_or("PUBLISH_PENDING")
            .to_string())
    }
}

#[async_trait]
impl SocialProvider for TikTokProvider {
    fn identifier(&self) -> &'static str {
        "tiktok"
    }

    fn name(&self) -> &'static str {
        "TikTok"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "user.info.basic".into(),
            "video.publish".into(),
            "video.upload".into(),
            // Display API read scope: backs list_videos(), which get_recent_posts()
            // and the analytics/engagement paths all depend on.
            "video.list".into(),
            // `video.delete` gates `POST /v2/video/delete/`. The provider
            // implements the delete (the app can remove a video it
            // published), so the consent has to ask for it. TikTok gates
            // this one behind App Review for accounts outside the app's
            // own test list, so the delete works for the developer's
            // channels and is refused for anyone else until the review
            // passes.
            "video.delete".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        // Caption limit. The 210 MB video cap is a media limit, not a text one;
        // using it here disabled caption validation entirely.
        2200
    }

    fn uses_oauth(&self) -> bool {
        true
    }

    /// TikTok access tokens live 24 hours (or only 7 days once the app is
    /// public), so a missed refresh takes publishing offline for a day.
    fn needs_cron_refresh(&self) -> bool {
        true
    }

    /// TikTok-specific user-facing error copy (v25 §2 row 9).
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if body.contains("DUPLICATE_CONTENT") || body.contains("duplicate content") {
            return Some(
                "TikTok rejected this video as duplicate content. TikTok hashes the audio \
                 track, so re-uploading the same clip fails even with new captions — change \
                 the video or use TikTok's repost flow instead."
                    .into(),
            );
        }
        if body.contains("unauthorized.creator_ttl") || body.contains("creator_ttl") {
            return Some(
                "TikTok rejected the video because the source URL has been posted to before. \
                 TikTok blocks re-uploads from the same URL — serve the file from a fresh URL."
                    .into(),
            );
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "TikTok rate limit hit. The Content Posting API allows roughly 1 000 video \
                 publishes per day per user — wait for the reset or lower the publish rate."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "TikTok rejected the video. Uploads must be MP4 under 210 MB, 3-10 minutes \
                 long, H.264 with AAC audio, fetched over HTTP from a public URL."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "TikTok access token expired or was revoked. TikTok tokens last 24 hours and \
                 Forge refreshes them automatically — reconnect if the refresh is rejected."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "TikTok denied this request (HTTP 403). The app needs an approved \
                 video.publish scope, an audit note, and the account must not be under 18."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "TikTok video or user not found — it may be private, deleted, or the app is \
                 not approved for the Content Posting API."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "TikTok rejected the request as invalid. Check that exactly one MP4 is \
                 attached, the title is within limits, and privacy_level is a value the app \
                 is authorised to use."
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
        let scope = self.scopes().join(",");
        let params: Vec<(&str, &str)> = vec![
            ("client_key", self.client_id.as_str()),
            ("scope", scope.as_str()),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("response_type", "code"),
        ];

        let url = url::Url::parse_with_params("https://www.tiktok.com/v2/auth/authorize/", &params)
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
        let params = serde_json::json!({
            "client_key": self.client_id,
            "client_secret": self.client_secret,
            "code": code,
            "grant_type": "authorization_code",
            "redirect_uri": redirect_uri,
        });

        let resp = self
            .http
            .post("https://open.tiktokapis.com/v2/oauth/token/")
            .header("Content-Type", "application/json")
            .json(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Token exchange failed")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();
        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);
        let open_id = json["open_id"].as_str().unwrap_or("").to_string();

        // Fetch user display info
        let user_info = self.get_user_info(&access_token).await.unwrap_or_default();

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: open_id,
            name: user_info["data"]["user"]["display_name"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            username: user_info["data"]["user"]["username"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            picture: user_info["data"]["user"]["avatar_url"]
                .as_str()
                .map(String::from),
        })
    }

    async fn refresh_token(&self, token: &str) -> Result<AuthToken, ProviderError> {
        let params = serde_json::json!({
            "client_key": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "refresh_token",
            "refresh_token": token,
        });

        let resp = self
            .http
            .post("https://open.tiktokapis.com/v2/oauth/token/")
            .header("Content-Type", "application/json")
            .json(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Token refresh failed")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();
        let new_refresh = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        Ok(AuthToken {
            access_token,
            refresh_token: new_refresh.or_else(|| Some(token.to_string())),
            expires_in,
            provider_user_id: String::new(),
            name: String::new(),
            username: String::new(),
            picture: None,
        })
    }

    /// Publish a video to TikTok.
    ///
    /// Flow:
    ///   1. Download the video from the first media attachment URL.
    ///   2. Upload the video bytes to TikTok via multipart POST.
    ///   3. Create the video post with metadata.
    /// Delete a published TikTok video.
    ///
    /// `POST /v2/video/delete/` with the video id, gated on the
    /// `video.delete` scope.
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on TikTok".into(),
            ));
        }
        let resp = self
            .http
            .post("https://open.tiktokapis.com/v2/video/delete/")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json; charset=UTF-8")
            .body(serde_json::json!({ "video_id": platform_post_id }).to_string())
            .send()
            .await?;
        let http_status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if http_status.is_success() {
            let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            if json["data"]["delete_status"] == "0" || json["error_code"].is_null() {
                return Ok(());
            }
            return Err(self.provider_error(&body, http_status.as_u16()));
        }
        Err(self.provider_error(&body, http_status.as_u16()))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // TikTok requires at least one video attachment
        let media = post.media.first().ok_or_else(|| {
            ProviderError::InvalidRequest("TikTok posts require a video attachment".into())
        })?;

        // Cap title at 150 characters (TikTok limit)
        let title = if post.content.len() > 150 {
            post.content[..150].to_string()
        } else {
            post.content.clone()
        };

        // Read privacy level from settings. B11: default SELF_ONLY —
        // unaudited API clients can ONLY post privately (a PUBLIC default
        // gets the whole init rejected with
        // unaudited_client_can_only_post_to_private). Explicit
        // settings.privacy_level wins for audited apps.
        let privacy_level = post
            .settings
            .get("privacy_level")
            .and_then(|v| v.as_str())
            .unwrap_or("SELF_ONLY");

        // B11: the Content Posting API — POST /v2/post/publish/video/init/.
        // The old code called /v2/video/upload/ + /v2/video/publish/, which
        // the Content Posting API does not have (every publish failed), and
        // treated the publish_id as a video id.
        // PULL_FROM_URL: TikTok fetches the video from the public URL — the
        // post is IN FLIGHT after init, so the publish_id is a tracking id,
        // not a video id.
        let init_body = serde_json::json!({
            "post_info": {
                "title": title,
                "privacy_level": privacy_level,
                "disable_duet": post.settings.get("disable_duet").and_then(|v| v.as_bool()).unwrap_or(false),
                "disable_comment": post.settings.get("disable_comment").and_then(|v| v.as_bool()).unwrap_or(false),
                "disable_stitch": post.settings.get("disable_stitch").and_then(|v| v.as_bool()).unwrap_or(false),
            },
            "source_info": {
                "source": "PULL_FROM_URL",
                "video_url": media.url,
            },
        });

        let init_resp = self
            .http
            .post("https://open.tiktokapis.com/v2/post/publish/video/init/")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&init_body)
            .send()
            .await?;

        let init_status = init_resp.status();
        let init_json: serde_json::Value = init_resp.json().await?;

        if !init_status.is_success() || init_json["error"]["code"].as_str() != Some("ok") {
            let msg = init_json["error"]["message"]
                .as_str()
                .unwrap_or("publish init failed");
            let code = init_json["error"]["code"].as_str().unwrap_or("error");
            let raw =
                format!("TikTok publish init failed (HTTP {init_status}, code {code}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(msg, init_status.as_u16()),
                &raw,
            )));
        }

        let publish_id = init_json["data"]["publish_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("TikTok init returned no publish_id".into()))?
            .to_string();

        // `publish` is the immediate path: poll the fetch endpoint until the
        // video is actually live (bounded) — see await_publish_status.
        self.await_publish_status(access_token, &publish_id).await?;

        Ok(PublishResult {
            platform_post_url: None, // resolved after transcoding; video list owns URLs
            platform_post_id: publish_id,
            status: "published".into(),
        })
    }

    /// B11: the engine-facing lifecycle. Init hands the video to TikTok
    /// (PULL_FROM_URL); transcoding takes minutes, so report `pending` with
    /// the publish_id and let the scheduler's finalize poll drive it home.
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let _ = internal_id;
        let media = post.media.first().ok_or_else(|| {
            ProviderError::InvalidRequest("TikTok posts require a video attachment".into())
        })?;

        let title = if post.content.len() > 150 {
            post.content[..150].to_string()
        } else {
            post.content.clone()
        };

        let privacy_level = post
            .settings
            .get("privacy_level")
            .and_then(|v| v.as_str())
            .unwrap_or("SELF_ONLY");

        let init_body = serde_json::json!({
            "post_info": {
                "title": title,
                "privacy_level": privacy_level,
                "disable_duet": post.settings.get("disable_duet").and_then(|v| v.as_bool()).unwrap_or(false),
                "disable_comment": post.settings.get("disable_comment").and_then(|v| v.as_bool()).unwrap_or(false),
                "disable_stitch": post.settings.get("disable_stitch").and_then(|v| v.as_bool()).unwrap_or(false),
            },
            "source_info": {
                "source": "PULL_FROM_URL",
                "video_url": media.url,
            },
        });

        let init_resp = self
            .http
            .post("https://open.tiktokapis.com/v2/post/publish/video/init/")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&init_body)
            .send()
            .await?;

        let init_status = init_resp.status();
        let init_json: serde_json::Value = init_resp.json().await?;

        if !init_status.is_success() || init_json["error"]["code"].as_str() != Some("ok") {
            let msg = init_json["error"]["message"]
                .as_str()
                .unwrap_or("publish init failed");
            let code = init_json["error"]["code"].as_str().unwrap_or("error");
            let raw =
                format!("TikTok publish init failed (HTTP {init_status}, code {code}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(msg, init_status.as_u16()),
                &raw,
            )));
        }

        let publish_id = init_json["data"]["publish_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("TikTok init returned no publish_id".into()))?
            .to_string();

        Ok(PublishResult {
            platform_post_url: None,
            platform_post_id: publish_id,
            status: "pending".into(),
        })
    }

    /// B11: poll one publish_id via the fetch-status endpoint.
    async fn check_post_status(
        &self,
        access_token: &str,
        publish_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let status = self.fetch_publish_status(access_token, publish_id).await?;
        let mapped = Self::publish_status_to_state(&status);
        Ok(PublishResult {
            platform_post_id: publish_id.to_string(),
            platform_post_url: None,
            status: mapped.into(),
        })
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let max_count = limit.clamp(1, 100);
        let videos = self.list_videos(access_token, max_count).await?;
        let mut posts = Vec::new();

        if let Some(data) = videos["data"]["videos"].as_array() {
            for item in data {
                let video_id = item["id"].as_str().unwrap_or("").to_string();
                let title = item["title"].as_str().unwrap_or("").to_string();
                let cover_url = item["cover_image_url"].as_str().map(|s| s.to_string());
                let share_url = item["share_url"].as_str().map(|s| s.to_string());
                let create_time_ts = item["create_time"].as_i64().unwrap_or(0);

                let posted_at =
                    chrono::DateTime::from_timestamp(create_time_ts, 0).unwrap_or_default();

                // Single media item: embed URL for iframe playback
                // Cover image URL is stored in metadata.poster_url so frontend can show it as placeholder
                let embed_url = format!("https://www.tiktok.com/embed/v2/{video_id}");
                let meta = serde_json::json!({
                    "title": title.clone(),
                    "poster_url": cover_url,
                });

                posts.push(ExternalPostData {
                    platform_post_id: video_id,
                    text: title.clone(),
                    author_name: None,
                    author_handle: None,
                    author_avatar: None,
                    media: vec![MediaAttachment {
                        url: embed_url,
                        mime_type: "text/html".into(),
                        alt: Some(title.clone()),
                        poster_url: None,
                    }],
                    created_at: posted_at,
                    url: share_url,
                    metadata: Some(meta),
                });
            }
        }

        Ok(posts)
    }

    /// Per-post counts (likes, comments, shares, views) for one video.
    ///
    /// The open API has no per-video analytics endpoint, so the counts come from
    /// the Display API `video.list` feed.
    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        // ponytail: first page only (100 newest). Covers any post the scheduler
        // published; a post that has fallen out of the window reports None rather
        // than a wrong zero. Page with data.cursor if that ever matters.
        let list = self.list_videos(access_token, 100).await?;
        Ok(Self::find_video(&list, platform_post_id).cloned())
    }

    /// Account-level analytics for the dashboard.
    ///
    /// `user.info.basic` returns lifetime counters only, so each series is a
    /// single point dated today and `days` carries no meaning.
    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let info = self.get_user_info(access_token).await?;
        let user = &info["data"]["user"];
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Ok(metrics_series(
            user,
            &today,
            &[
                ("Followers", "follower_count"),
                ("Following", "following_count"),
                ("Likes", "likes_count"),
                ("Videos", "video_count"),
            ],
        ))
    }

    /// Per-post analytics, rendered from the same Display API counts as
    /// `get_post_engagement` — there is no separate analytics endpoint.
    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let Some(video) = self
            .get_post_engagement(access_token, platform_post_id)
            .await?
        else {
            return Ok(vec![]);
        };
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Ok(metrics_series(
            &video,
            &today,
            &[
                ("Views", "view_count"),
                ("Likes", "like_count"),
                ("Comments", "comment_count"),
                ("Shares", "share_count"),
            ],
        ))
    }

    /// Return the authenticated user as a single "page" (TikTok has no multi-page concept).
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let info = self.get_user_info(access_token).await?;
        let user = &info["data"]["user"];

        Ok(vec![PageInfo {
            id: user["open_id"].as_str().unwrap_or("").to_string(),
            name: user["display_name"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture: user["avatar_url"].as_str().map(String::from),
            username: user["username"].as_str().map(String::from),
        }])
    }

    /// Fetch page info by open_id. Since TikTok is single-user OAuth, returns the
    /// authenticated user's info regardless of page_id.
    async fn fetch_page_info(
        &self,
        access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let info = self.get_user_info(access_token).await?;
        let user = &info["data"]["user"];

        Ok(PageInfo {
            id: user["open_id"].as_str().unwrap_or("").to_string(),
            name: user["display_name"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture: user["avatar_url"].as_str().map(String::from),
            username: user["username"].as_str().map(String::from),
        })
    }

    /// Read a video's comments from the Research API
    /// (`GET /v2/research/video/comment/`).
    ///
    /// The Content Posting API has no comment-read endpoint — this is the only
    /// one TikTok exposes, and it needs approved Research API access. An app
    /// without it gets HTTP 403, so the read degrades to an empty list rather
    /// than failing the whole Comments tab.
    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let resp = self
            .http
            .get("https://open.tiktokapis.com/v2/research/video/comment/")
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[
                ("video_id", platform_post_id),
                ("max_count", "50"),
                (
                    "fields",
                    "id,text,create_time,like_count,reply_id,parent_comment_id",
                ),
            ])
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("TikTok API rate limit".into()));
        }
        if !status.is_success() {
            // 403 = no Research API access; the Comments tab shows empty.
            tracing::debug!("TikTok comment read unavailable (HTTP {status})");
            return Ok(vec![]);
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(Self::parse_research_comments(&json))
    }

    async fn reconnect(
        &self,
        access_token: &str,
        _internal_id: &str,
        _page_id: &str,
    ) -> Result<ReconnectResult, ProviderError> {
        let info = self.fetch_page_info(access_token, "").await?;
        Ok(ReconnectResult {
            id: info.id,
            name: info.name,
            access_token: info.access_token.unwrap_or_default(),
            picture: info.picture,
            username: info.username,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::parse_engagement_data;

    #[test]
    fn publish_status_should_treat_publish_complete_as_success() {
        assert_eq!(
            TikTokProvider::publish_status_to_state("PUBLISH_COMPLETE"),
            "published"
        );
    }

    /// `max_content_length` once held the 210 MB video cap, so every caption
    /// passed validation. A 2201-char caption must be refused now.
    #[test]
    fn caption_over_2200_chars_should_be_rejected() {
        let provider = TikTokProvider::new(&crate::social::test_config());
        let post = PostContent {
            content: "a".repeat(2201),
            ..Default::default()
        };
        assert!(provider.validate_post(&post).is_err());
        let ok = PostContent {
            content: "a".repeat(2200),
            ..Default::default()
        };
        assert!(provider.validate_post(&ok).is_ok());
    }

    #[test]
    fn publish_status_should_surface_publish_failed_as_failed() {
        assert_eq!(
            TikTokProvider::publish_status_to_state("PUBLISH_FAILED"),
            "failed"
        );
    }

    #[test]
    fn publish_status_should_keep_polling_while_processing() {
        for status in ["PROCESSING", "PROCESSING_UPLOAD", "PUBLISHING"] {
            assert_eq!(
                TikTokProvider::publish_status_to_state(status),
                "pending",
                "{status} must stay pending"
            );
        }
    }

    #[test]
    fn publish_status_should_keep_polling_when_status_is_absent() {
        // fetch_publish_status defaults an absent data.status to
        // PUBLISH_PENDING, which must map to pending — reporting success
        // there is exactly the bug the poll exists to prevent.
        assert_eq!(
            TikTokProvider::publish_status_to_state("PUBLISH_PENDING"),
            "pending"
        );
    }

    #[test]
    fn parse_should_map_tiktok_display_api_counts() {
        let raw = serde_json::json!({
            "id": "7300000000000000000",
            "view_count": 1200,
            "like_count": 42,
            "comment_count": 12,
            "share_count": 5
        });
        let e = parse_engagement_data("tiktok", raw);
        assert_eq!(e.views, 1200);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
        assert_eq!(e.shares, 5);
    }

    #[test]
    fn parse_should_read_string_encoded_tiktok_counts() {
        // `/v2/video/query/?fields=…` can return counts as strings.
        let raw = serde_json::json!({ "like_count": "8", "view_count": "90" });
        let e = parse_engagement_data("tiktok", raw);
        assert_eq!(e.views, 90);
        assert_eq!(e.likes, 8);
    }

    #[test]
    fn parse_should_default_missing_tiktok_metrics_to_zero() {
        let e = parse_engagement_data("tiktok", serde_json::json!({ "like_count": 3 }));
        assert_eq!(e.likes, 3);
        assert_eq!(e.views, 0);
        assert_eq!(e.shares, 0);
    }

    #[test]
    fn find_video_should_match_by_id() {
        let list = serde_json::json!({
            "data": { "videos": [ { "id": "111", "like_count": 1 }, { "id": "222", "like_count": 2 } ] }
        });
        let found = TikTokProvider::find_video(&list, "222").expect("video 222 must be found");
        assert_eq!(found["like_count"], 2);
    }

    #[test]
    fn find_video_should_return_none_for_unknown_id() {
        let list = serde_json::json!({ "data": { "videos": [ { "id": "111" } ] } });
        assert!(TikTokProvider::find_video(&list, "999").is_none());
    }

    #[test]
    fn metrics_series_should_skip_tiktok_counters_absent_from_response() {
        // Display API omits counters for private/under-review videos.
        let video = serde_json::json!({ "id": "1", "like_count": 4 });
        let series = metrics_series(
            &video,
            "2026-09-30",
            &[
                ("Views", "view_count"),
                ("Likes", "like_count"),
                ("Shares", "share_count"),
            ],
        );
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].label, "Likes");
        assert_eq!(series[0].data[0].total, "4");
    }

    // ── B4: needs_cron_refresh + map_error fixtures ────────

    fn provider() -> TikTokProvider {
        TikTokProvider::new(&crate::social::test_config())
    }

    #[test]
    fn tiktok_requests_cron_refresh() {
        assert!(provider().needs_cron_refresh());
    }

    #[test]
    fn map_error_explains_duplicate_content() {
        let msg = provider().map_error("DUPLICATE_CONTENT", 400).unwrap();
        assert!(msg.contains("duplicate content"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_creator_ttl_rejection() {
        let msg = provider()
            .map_error(
                "The video has been posted before (unauthorized.creator_ttl)",
                400,
            )
            .unwrap();
        assert!(msg.contains("fresh URL"), "got: {msg}");
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
        assert!(msg.contains("210 MB"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let msg = provider().map_error("", 401).unwrap();
        assert!(msg.contains("24 hours"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_unapproved_app_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("video.publish"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_video_on_404() {
        let msg = provider().map_error("", 404).unwrap();
        assert!(msg.contains("not found"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let msg = provider().map_error("", 400).unwrap();
        assert!(msg.contains("privacy_level"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 503).is_none());
    }

    // ── B3: research comments ────────────────────────────────

    #[test]
    fn parse_should_nest_tiktok_replies_under_their_parent() {
        let raw = serde_json::json!({ "data": { "comments": [
            { "id": "c1", "text": "nice", "create_time": 1_757_000_000_i64,
              "like_count": 5, "parent_comment_id": "0", "user": { "screen_name": "ada" } },
            { "id": "c2", "text": "agreed", "create_time": 1_757_000_100_i64,
              "parent_comment_id": "c1", "user": { "screen_name": "bob" } }
        ]}});
        let comments = TikTokProvider::parse_research_comments(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "c1");
        assert_eq!(comments[0].author_name.as_deref(), Some("ada"));
        assert_eq!(comments[0].like_count, 5);
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].id, "c2");
    }

    #[test]
    fn parse_should_surface_tiktok_reply_whose_parent_is_absent() {
        let raw = serde_json::json!({ "data": { "comments": [
            { "id": "c2", "text": "orphan reply", "parent_comment_id": "missing" }
        ]}});
        let comments = TikTokProvider::parse_research_comments(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "c2");
    }

    #[test]
    fn parse_should_read_no_tiktok_comments_from_empty_response() {
        assert!(
            TikTokProvider::parse_research_comments(&serde_json::json!({ "data": {} })).is_empty()
        );
    }
}
