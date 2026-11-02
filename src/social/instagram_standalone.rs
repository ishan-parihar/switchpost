// ─── Instagram Standalone Provider ────────────────────────────
// Uses Instagram Basic Display API (graph.instagram.com).
// Separate OAuth flow from Facebook Graph API.

use async_trait::async_trait;

use super::*;
use super::common::extract_access_token;
use crate::config::Config;

pub struct InstagramStandaloneProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl InstagramStandaloneProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config
            .provider_credentials("instagram-standalone")
            .unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    /// Instagram Basic Display / Graph insights host (same pinned v21 as the
    /// Facebook-side providers — do not drift the version independently).
    fn graph_url(&self) -> String {
        format!("https://graph.instagram.com/{GRAPH_VERSION}")
    }
}

#[async_trait]
impl SocialProvider for InstagramStandaloneProvider {
    fn identifier(&self) -> &'static str {
        "instagram-standalone"
    }

    fn name(&self) -> &'static str {
        "Instagram (Standalone)"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "instagram_business_basic".into(),
            "instagram_business_content_publish".into(),
            "instagram_business_manage_comments".into(),
            "instagram_business_manage_insights".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        2200
    }

    /// Meta refuses an `http://` redirect URI for Instagram too — the same
    /// gate as Threads, in `api::integrations::connect`.
    fn requires_https_redirect(&self) -> bool {
        true
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    fn needs_cron_refresh(&self) -> bool {
        true
    }

    fn tooltip(&self) -> Option<&'static str> {
        Some("Connect a personal Instagram account (no Facebook Page required)")
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let scope = self.scopes().join(",");
        let params: Vec<(&str, &str)> = vec![
            ("enable_fb_login", "0"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("state", state),
            ("scope", scope.as_str()),
        ];

        let url = url::Url::parse_with_params("https://www.instagram.com/oauth/authorize", &params)
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
        let params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
            ("code", code),
        ];

        let resp = self
            .http
            .post("https://api.instagram.com/oauth/access_token")
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            let err_msg = json["error_message"]
                .as_str()
                .or_else(|| json["error"]["message"].as_str())
                .unwrap_or("Unknown error");
            let err_code = json["error"]["code"].as_u64().unwrap_or(0);
            return Err(ProviderError::Api(format!(
                "Instagram token exchange failed (code {err_code}): {err_msg}"
            )));
        }
        let short_token = json["access_token"]
            .as_str()
            .ok_or_else(|| {
                let err = serde_json::to_string(&json).unwrap_or_default();
                ProviderError::Auth(format!("Missing access_token in response: {err}"))
            })?
            .to_string();

        // Step 2: Exchange for long-lived token (60 days)
        let long_params: Vec<(&str, &str)> = vec![
            ("grant_type", "ig_exchange_token"),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("access_token", short_token.as_str()),
        ];

        let long_resp = self
            .http
            .get("https://graph.instagram.com/access_token")
            .query(&long_params)
            .send()
            .await?;

        let long_status = long_resp.status();
        let long_json: serde_json::Value = long_resp.json().await?;
        if !long_status.is_success() {
            let err_msg = long_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error");
            return Err(ProviderError::Api(format!(
                "Instagram long-lived token exchange failed: {err_msg}"
            )));
        }
        let access_token = long_json["access_token"]
            .as_str()
            .unwrap_or(&short_token)
            .to_string();
        let expires_in = long_json["expires_in"].as_u64().map(|v| v as u32);

        // Get user info
        let user: serde_json::Value = self
            .http
            .get(format!("https://graph.instagram.com/{GRAPH_VERSION}/me"))
            .query(&[
                ("fields", "user_id,username,name,profile_picture_url"),
                ("access_token", access_token.as_str()),
            ])
            .send()
            .await?
            .json()
            .await?;

        let at = access_token.clone();
        Ok(AuthToken {
            access_token,
            refresh_token: Some(at),
            expires_in,
            provider_user_id: user["user_id"].as_str().unwrap_or("").to_string(),
            name: user["name"].as_str().unwrap_or("").to_string(),
            username: user["username"].as_str().unwrap_or("").to_string(),
            picture: user["profile_picture_url"].as_str().map(String::from),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let params: Vec<(&str, &str)> = vec![
            ("grant_type", "ig_refresh_token"),
            ("access_token", refresh_token),
        ];

        let resp = self
            .http
            .get("https://graph.instagram.com/refresh_access_token")
            .query(&params)
            .send()
            .await?;

        let (access_token, json) = extract_access_token(resp, "instagram-standalone")
            .await
            .map_err(ProviderError::Auth)?;
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        let user: serde_json::Value = self
            .http
            .get(format!("https://graph.instagram.com/{GRAPH_VERSION}/me"))
            .query(&[
                ("fields", "user_id,username,name,profile_picture_url"),
                ("access_token", access_token.as_str()),
            ])
            .send()
            .await?
            .json()
            .await?;

        let at = access_token.clone();
        Ok(AuthToken {
            access_token,
            refresh_token: Some(at),
            expires_in,
            provider_user_id: user["user_id"].as_str().unwrap_or("").to_string(),
            name: user["name"].as_str().unwrap_or("").to_string(),
            username: user["username"].as_str().unwrap_or("").to_string(),
            picture: user["profile_picture_url"].as_str().map(String::from),
        })
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        if post.media.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "Instagram requires at least one media attachment".into(),
            ));
        }

        let ig_id = self.resolve_user_id(access_token).await?;

        // Create media container
        let is_video = post.media[0].url.contains(".mp4");
        let media_type = if is_video { "VIDEO" } else { "IMAGE" };
        let url_key = if is_video { "video_url" } else { "image_url" };

        // The music track, if the composer picked one.
        //
        // The composer's MusicPicker is shown for Instagram and the
        // pick is written to `settings.audio_id` - and nothing read it,
        // so the reel went out silent while the composer showed a
        // "Change/Browse" control that implied it had been applied.
        let audio_id = post
            .settings
            .get("audio_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        // The form holds OWNED strings: the audio configuration is
        // built at runtime, so a `Vec<(&str, &str)>` cannot carry it.
        let mut form: Vec<(&str, String)> = vec![
            (url_key, post.media[0].url.clone()),
            ("media_type", media_type.to_string()),
            ("access_token", access_token.to_string()),
        ];
        if post.media.len() == 1 {
            form.push(("caption", post.content.clone()));
        }
        if let Some(audio_id) = audio_id {
            if !is_video {
                return Err(ProviderError::InvalidRequest(
                    "Instagram music can only be attached to a video (Reel), not to an image."
                        .into(),
                ));
            }
            let audio_configuration = serde_json::json!({ "audio_id": audio_id }).to_string();
            // The form owns the strings, so the configuration has to be
            // owned too - a borrowed local would not outlive the
            // request.
            form.push(("audio_configuration", audio_configuration));
        }

        let resp = self
            .http
            .post(format!(
                "https://graph.instagram.com/{GRAPH_VERSION}/{ig_id}/media"
            ))
            .form(&form)
            .send()
            .await?;

        let container_status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if let Some(err) = json["error"].as_object() {
            let msg = err["message"]
                .as_str()
                .unwrap_or("Container creation failed");
            let code = err["code"].as_u64().unwrap_or(0);
            let raw = format!("Instagram container creation failed (code {code}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(msg, container_status.as_u16()),
                &raw,
            )));
        }

        let container_id = json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api(format!("No container ID: {json:?}")))?
            .to_string();

        // Publish container
        let pub_form: Vec<(&str, &str)> = vec![
            ("creation_id", container_id.as_str()),
            ("access_token", access_token),
        ];

        let pub_resp = self
            .http
            .post(format!(
                "https://graph.instagram.com/{GRAPH_VERSION}/{ig_id}/media_publish"
            ))
            .form(&pub_form)
            .send()
            .await?;

        let pub_status = pub_resp.status();
        let pub_json: serde_json::Value = pub_resp.json().await?;

        if let Some(err) = pub_json["error"].as_object() {
            let msg = err["message"].as_str().unwrap_or("Publish failed");
            let code = err["code"].as_u64().unwrap_or(0);
            let raw = format!("Instagram publish failed (code {code}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(msg, pub_status.as_u16()),
                &raw,
            )));
        }

        let media_id = pub_json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api(format!("Publish failed: {pub_json:?}")))?
            .to_string();

        let post_url = format!("https://instagram.com/p/{media_id}");
        Ok(PublishResult {
            platform_post_id: media_id,
            platform_post_url: Some(post_url),
            status: "published".into(),
        })
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    /// Account-level insights over a day range. Feeds the dashboard via the
    /// shared analytics cache refresher.
    /// Instagram-standalone-specific user-facing error copy (v25 §2 row 9).
    ///
    /// Unlike the Facebook-login Instagram provider, this one *does* have a
    /// refresh endpoint (`ig_refresh_token`), so the auth-expired copy points
    /// at the automatic refresh rather than at a reconnect.
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Instagram rate limit hit. The Instagram Graph API throttles container \
                 creation — Forge retries with backoff automatically."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Instagram rejected the media. Images must be JPEG under 8 MB with a public \
                 URL; videos must be MP4 under 1 GB and 10 minutes long."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Instagram token expired or was revoked. Reconnect the account — Instagram \
                 standalone short-lived tokens last 24 hours and refresh only once per token."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Instagram denied this request (HTTP 403). The token needs the instagram_basic \
                 scope and the account must be a Business or Creator account."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Instagram account or media not found. Reconnect the account — it may have \
                 been switched to a personal profile."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Instagram rejected the request as invalid. Check the caption length (2200 \
                 chars) and that exactly one media item is attached."
                    .into(),
            ),
            _ => None,
        }
    }

    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let ig_id = self.resolve_user_id(access_token).await?;

        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(days as i64))
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        let until = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let json = self
            .get_insights_range(
                access_token,
                &ig_id,
                "reach,views,profile_views,follower_count",
                "day",
                Some(&since),
                Some(&until),
            )
            .await?;

        Ok(super::parse_insights_data(&json))
    }

    /// Per-media insights (lifetime). Graph returns single-value metrics here,
    /// which `parse_insights_data` turns into one data point per metric.
    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let json = self
            .get_insights_range(
                access_token,
                platform_post_id,
                "reach,views,saved,likes,comments",
                "lifetime",
                None,
                None,
            )
            .await?;

        Ok(super::parse_insights_data(&json))
    }

    /// Merge the media object's own counters with its insights envelope so
    /// `parse_engagement_data("instagram-standalone", …)` gets one flat shape.
    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        let mut detail = self
            .get_media_detail(access_token, platform_post_id)
            .await?;
        if detail.get("error").is_some() || detail.get("id").is_none() {
            return Ok(None);
        }

        // Insights are best-effort: a metrics scope or metric-level failure
        // must not drop the counts the media object already gave us.
        match self
            .get_insights_range(
                access_token,
                platform_post_id,
                "reach,views,saved",
                "lifetime",
                None,
                None,
            )
            .await
        {
            Ok(insights) => {
                if let Some(obj) = detail.as_object_mut() {
                    for metric in ["reach", "views", "saved"] {
                        if let Some(v) = super::insight_value(&insights, metric) {
                            obj.insert(metric.to_string(), serde_json::json!(v));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!("IG standalone insights unavailable for {platform_post_id}: {e}")
            }
        }

        Ok(Some(detail))
    }

    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_media_comments(access_token, platform_post_id)
            .await?;
        Ok(super::parse_graph_comments(&json))
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let json = self
            .reply_to_comment(access_token, comment_id, &post.content)
            .await?;
        let id = json["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(ProviderError::Api(format!(
                "Instagram reply failed: {json}"
            )));
        }
        Ok(PublishResult {
            platform_post_id: id,
            platform_post_url: None,
            status: "published".into(),
        })
    }

    /// The IG API has no member search; accounts that mention this profile are
    /// the only taggable set available.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let ig_id = self.resolve_user_id(access_token).await?;
        let json = self.get_mentions(access_token, &ig_id).await?;
        Ok(super::parse_graph_mentions(&json, query))
    }
}

impl InstagramStandaloneProvider {
    async fn resolve_user_id(&self, access_token: &str) -> Result<String, ProviderError> {
        let user: serde_json::Value = self
            .http
            .get(format!("https://graph.instagram.com/{GRAPH_VERSION}/me"))
            .query(&[("fields", "user_id"), ("access_token", access_token)])
            .send()
            .await?
            .json()
            .await?;

        user["user_id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| ProviderError::Auth("Could not resolve Instagram user ID".into()))
    }

    pub async fn get_media(
        &self,
        access_token: &str,
        ig_user_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = limit.min(100);
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{ig_user_id}/media",);
        let resp = self
            .http
            .get(&url)
            .query(&[
                (
                    "fields",
                    "id,caption,media_type,media_url,permalink,timestamp,like_count,comments_count",
                ),
                ("limit", &limit.to_string()),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_media_detail(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{media_id}",);
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("fields", "id,caption,media_type,media_url,permalink,timestamp,username,like_count,comments_count"),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    /// `/{id}/insights` with an optional `since`/`until` window, so
    /// `analytics()` can honour the requested day range.
    pub async fn get_insights_range(
        &self,
        access_token: &str,
        ig_id: &str,
        metric: &str,
        period: &str,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut params: Vec<(&str, &str)> = vec![
            ("metric", metric),
            ("period", period),
            ("access_token", access_token),
        ];
        if let Some(s) = since {
            params.push(("since", s));
        }
        if let Some(u) = until {
            params.push(("until", u));
        }

        let url = format!("{}/{}/insights", self.graph_url(), ig_id);
        let resp = self.http.get(&url).query(&params).send().await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_media_comments(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{media_id}/comments",);
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("fields", "id,text,timestamp,username,like_count,replies"),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{comment_id}/replies",);
        let resp = self
            .http
            .post(&url)
            .form(&[("message", message), ("access_token", access_token)])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    /// Accounts that have mentioned this profile — the @-autocomplete source.
    pub async fn get_mentions(
        &self,
        access_token: &str,
        ig_user_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_user_id}/mentions", self.graph_url());
        let resp = self
            .http
            .get(url)
            .query(&[
                ("fields", "id,username,profile_picture_url,text,timestamp"),
                ("access_token", access_token),
            ])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Instagram API error")
                    .to_string(),
            ))
        }
    }

    pub async fn create_container(
        &self,
        access_token: &str,
        ig_user_id: &str,
        media_url: &str,
        caption: &str,
        media_type: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{ig_user_id}/media",);
        let mut params: Vec<(&str, &str)> = vec![
            ("media_type", media_type),
            ("caption", caption),
            ("access_token", access_token),
        ];
        if media_type == "IMAGE" {
            params.push(("image_url", media_url));
        } else {
            params.push(("video_url", media_url));
        }
        let resp = self.http.post(&url).form(&params).send().await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn publish_container(
        &self,
        access_token: &str,
        ig_user_id: &str,
        creation_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url =
            format!("https://graph.instagram.com/{GRAPH_VERSION}/{ig_user_id}/media_publish",);
        let resp = self
            .http
            .post(&url)
            .form(&[("creation_id", creation_id), ("access_token", access_token)])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited(
                "Instagram API rate limit".into(),
            ))
        } else if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn poll_container_status(
        &self,
        access_token: &str,
        creation_id: &str,
    ) -> Result<String, ProviderError> {
        let url = format!("https://graph.instagram.com/{GRAPH_VERSION}/{creation_id}",);
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "status_code"), ("access_token", access_token)])
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await?;
        if let Some(err) = json["error"].as_object() {
            let msg = err["message"]
                .as_str()
                .unwrap_or("Container status check failed");
            return Err(ProviderError::Api(msg.to_string()));
        }
        let status_code = json["status_code"].as_str().unwrap_or("IN_PROGRESS");
        Ok(status_code.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    fn post_with_images(n: usize) -> PostContent {
        PostContent {
            content: "caption".into(),
            media: (0..n)
                .map(|i| MediaAttachment {
                    url: format!("https://example.com/{i}.jpg"),
                    mime_type: "image/jpeg".into(),
                    alt: None,
                    poster_url: None,
                })
                .collect(),
            settings: serde_json::json!({}),
            ..Default::default()
        }
    }

    /// Shape the provider hands to `fetch_engagement` after merging the media
    /// object with its insights envelope.
    fn merged_media_and_insights() -> serde_json::Value {
        let mut detail = serde_json::json!({
            "id": "17900000000000000",
            "media_type": "IMAGE",
            "like_count": 42,
            "comments_count": 12
        });
        let insights = serde_json::json!({
            "data": [
                { "name": "reach", "period": "lifetime", "values": [{ "value": 300, "end_time": "2026-09-29T00:00:00+0000" }] },
                { "name": "views", "period": "lifetime", "values": [{ "value": 420, "end_time": "2026-09-29T00:00:00+0000" }] },
                { "name": "saved", "period": "lifetime", "values": [{ "value": 7, "end_time": "2026-09-29T00:00:00+0000" }] }
            ]
        });
        let obj = detail.as_object_mut().expect("json object");
        for metric in ["reach", "views", "saved"] {
            if let Some(v) = super::super::insight_value(&insights, metric) {
                obj.insert(metric.to_string(), serde_json::json!(v));
            }
        }
        detail
    }

    #[test]
    fn test_identifier_uses_hyphenated_slug() {
        let provider = InstagramStandaloneProvider::new(&test_config());
        assert_eq!(provider.identifier(), "instagram-standalone");
    }

    #[test]
    fn test_graph_url_is_pinned_to_v21() {
        let provider = InstagramStandaloneProvider::new(&test_config());
        assert_eq!(
            provider.graph_url(),
            format!("https://graph.instagram.com/{GRAPH_VERSION}")
        );
    }

    #[test]
    fn parse_should_expand_account_insights_into_daily_series() {
        let raw = serde_json::json!({
            "data": [{
                "name": "reach",
                "period": "day",
                "values": [
                    { "value": 300, "end_time": "2026-09-28T00:00:00+0000" },
                    { "value": 420, "end_time": "2026-09-29T00:00:00+0000" }
                ]
            }]
        });
        let series = super::super::parse_insights_data(&raw);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].label, "reach");
        assert_eq!(series[0].data.len(), 2);
        assert_eq!(series[0].data[1].total, "420");
    }

    #[test]
    fn parse_should_expand_lifetime_media_insights_into_one_point_each() {
        let raw = serde_json::json!({
            "data": [
                { "name": "reach", "period": "lifetime", "values": [{ "value": 300, "end_time": "2026-09-29T00:00:00+0000" }] },
                { "name": "saved", "period": "lifetime", "values": [{ "value": 7, "end_time": "2026-09-29T00:00:00+0000" }] }
            ]
        });
        let series = super::super::parse_insights_data(&raw);
        assert_eq!(series.len(), 2);
        assert!(series.iter().all(|s| s.data.len() == 1));
        assert_eq!(series[1].data[0].total, "7");
    }

    #[test]
    fn parse_should_map_merged_media_counts_and_insights() {
        let e = super::super::parse_engagement_data(
            "instagram-standalone",
            merged_media_and_insights(),
        );
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
        assert_eq!(e.saves, 7);
        assert_eq!(e.views, 420);
    }

    #[test]
    fn publish_should_reject_11th_image() {
        let provider = InstagramStandaloneProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(11)).is_err());
    }

    #[test]
    fn publish_should_accept_10_images() {
        let provider = InstagramStandaloneProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(10)).is_ok());
    }

    // ── B4: map_error fixtures ──────────────────────────────

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("The video upload was rejected", 422).unwrap();
        assert!(msg.contains("8 MB"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("", 401).unwrap();
        assert!(msg.contains("24 hours"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_scope_on_403() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("", 403).unwrap();
        assert!(msg.contains("instagram_basic"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_account_on_404() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("", 404).unwrap();
        assert!(msg.contains("not found"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let p = InstagramStandaloneProvider::new(&test_config());
        let msg = p.map_error("", 400).unwrap();
        assert!(msg.contains("2200"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        let p = InstagramStandaloneProvider::new(&test_config());
        assert!(p.map_error("Internal Server Error", 503).is_none());
    }

    #[test]
    fn instagram_standalone_requests_cron_refresh() {
        let p = InstagramStandaloneProvider::new(&test_config());
        assert!(p.needs_cron_refresh());
    }
}
