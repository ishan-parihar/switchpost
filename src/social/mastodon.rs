// ─── Mastodon Provider ────────────────────────────────────────
// Mastodon OAuth 2.0 with per-instance URLs.
// Supports: OAuth flow (with app registration), posting, timeline, search.
//
// OAuth Flow:
//   1. generate_auth_url: POST /api/v1/apps to register app, store client_id/secret,
//      return GET /oauth/authorize URL
//   2. exchange_code: POST /oauth/token with code
//   3. refresh_token: POST /oauth/token with refresh_token

use async_trait::async_trait;
use std::sync::Mutex;

use super::*;
use crate::config::Config;

pub struct MastodonProvider {
    http: reqwest::Client,
    instance_url: String,
    /// Temporarily stores (client_id, client_secret) during the OAuth
    /// app registration step so exchange_code can use them.
    app_credentials: Mutex<Option<(String, String)>>,
}

impl MastodonProvider {
    pub fn new(config: &Config) -> Self {
        let instance_url = config
            .mastodon_instance_url
            .clone()
            .unwrap_or_else(|| "mastodon.social".into());
        Self {
            http: shared_http(),
            instance_url,
            app_credentials: Mutex::new(None),
        }
    }

    /// Instance URL (e.g. "mastodon.social" or "https://mastodon.social")
    pub fn api_url(&self, path: &str) -> String {
        let base = self.instance_url.trim_end_matches('/');
        // If user provided a full URL (https://...), use it directly
        if base.starts_with("http://") || base.starts_with("https://") {
            format!("{}{}", base, path)
        } else {
            format!("https://{}{}", base, path)
        }
    }

    /// Get the authenticated user's info.
    pub async fn get_user_info(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(self.api_url("/api/v1/accounts/verify_credentials"))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Unknown Mastodon API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Fetch account by ID.
    pub async fn get_account(
        &self,
        access_token: &str,
        account_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(self.api_url(&format!("/api/v1/accounts/{account_id}")))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Unknown Mastodon API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Get a timeline (home, local, trending, public).
    pub async fn get_timeline(
        &self,
        access_token: &str,
        timeline_type: &str,
        limit: i32,
    ) -> Result<serde_json::Value, ProviderError> {
        let tl = match timeline_type {
            "home" => "home",
            "local" => "public?local=true",
            "trending" => "trends",
            "public" => "public",
            _ => "home",
        };
        let url = if tl == "trends" {
            self.api_url(&format!("/api/v1/trends/statuses?limit={}", limit))
        } else {
            self.api_url(&format!("/api/v1/timelines/{}?limit={}", tl, limit))
        };
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Unknown Mastodon API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Get a single status/post by ID.
    pub async fn get_post(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(self.api_url(&format!("/api/v1/statuses/{post_id}")))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Unknown Mastodon API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Search across Mastodon for accounts, statuses, hashtags.
    pub async fn search(
        &self,
        access_token: &str,
        query: &str,
        limit: i32,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = self.api_url(&format!(
            "/api/v2/search?q={}&limit={}",
            urlencoding::encode(query),
            limit
        ));
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Unknown Mastodon API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Upload media and return the media attachment ID.
    async fn upload_media(
        &self,
        access_token: &str,
        media_url: &str,
        mime_type: &str,
        alt: Option<&str>,
    ) -> Result<String, ProviderError> {
        // Download the media bytes
        let bytes = self
            .http
            .get(media_url)
            .send()
            .await
            .map_err(ProviderError::Network)?
            .bytes()
            .await
            .map_err(ProviderError::Network)?;

        // Determine file extension from mime type
        let ext = match mime_type {
            t if t.starts_with("image/jpeg") || t == "image/jpg" => "jpg",
            t if t.starts_with("image/png") => "png",
            t if t.starts_with("image/gif") => "gif",
            t if t.starts_with("image/webp") => "webp",
            t if t.starts_with("video/") => "mp4",
            _ => "jpg",
        };

        let file_part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(format!("media.{}", ext))
            .mime_str(mime_type)
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        let mut form = reqwest::multipart::Form::new().part("file", file_part);
        if let Some(desc) = alt {
            form = form.text("description", desc.to_string());
        }

        let resp = self
            .http
            .post(self.api_url("/api/v1/media"))
            .header("Authorization", format!("Bearer {access_token}"))
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Media upload failed")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }

        let id = json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("Missing media ID from response".into()))?
            .to_string();
        Ok(id)
    }
}

#[async_trait]
impl SocialProvider for MastodonProvider {
    fn identifier(&self) -> &'static str {
        "mastodon"
    }

    fn name(&self) -> &'static str {
        "Mastodon"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "read".into(),
            "write".into(),
            "follow".into(),
            "push".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        500
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    fn uses_oauth(&self) -> bool {
        true
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        // Step 1: Register an app on the instance
        let scope = self.scopes().join(" ");
        let app_body = serde_json::json!({
            "client_name": "Switchpost",
            "redirect_uris": redirect_uri,
            "scopes": scope,
            "website": "https://switchpost.app",
        });

        let resp = self
            .http
            .post(self.api_url("/api/v1/apps"))
            .header("Content-Type", "application/json")
            .json(&app_body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]
                .as_str()
                .unwrap_or("App registration failed")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let client_id = json["client_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing client_id from app registration".into()))?
            .to_string();
        let client_secret = json["client_secret"]
            .as_str()
            .ok_or_else(|| {
                ProviderError::Auth("Missing client_secret from app registration".into())
            })?
            .to_string();

        // Store for exchange_code step
        *self
            .app_credentials
            .lock()
            .expect("app credentials lock not poisoned") =
            Some((client_id.clone(), client_secret.clone()));

        // Step 2: Build the authorize URL
        let scope = self.scopes().join(" ");
        let params: Vec<(&str, &str)> = vec![
            ("client_id", &client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", &scope),
            ("state", state),
        ];

        let auth_url = self.api_url("/oauth/authorize");
        let url = url::Url::parse_with_params(&auth_url, &params)
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
        // Read client_id and client_secret from the stored app credentials
        let (client_id, client_secret) = self
            .app_credentials
            .lock()
            .expect("app credentials lock not poisoned")
            .clone()
            .ok_or_else(|| {
                ProviderError::Auth(
                    "No app credentials. Call generate_auth_url first to register the app.".into(),
                )
            })?;

        let body = serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
            "scope": self.scopes().join(" "),
        });

        let resp = self
            .http
            .post(self.api_url("/oauth/token"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]
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

        // Fetch user info to get profile details
        let user_info = self.get_user_info(&access_token).await.unwrap_or_default();

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: user_info["id"].as_str().unwrap_or("").to_string(),
            name: user_info["display_name"].as_str().unwrap_or("").to_string(),
            username: user_info["username"].as_str().unwrap_or("").to_string(),
            picture: user_info["avatar"].as_str().map(String::from),
        })
    }

    async fn refresh_token(&self, token: &str) -> Result<AuthToken, ProviderError> {
        // Re-register the app to get fresh credentials for the refresh
        // (app registration returns client_id/secret needed for token refresh)
        let scope = self.scopes().join(" ");
        let app_body = serde_json::json!({
            "client_name": "Switchpost",
            "redirect_uris": "urn:ietf:wg:oauth:2.0:oob",
            "scopes": scope,
        });

        let resp = self
            .http
            .post(self.api_url("/api/v1/apps"))
            .header("Content-Type", "application/json")
            .json(&app_body)
            .send()
            .await?;

        let status = resp.status();
        let app_json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = app_json["error"]
                .as_str()
                .unwrap_or("App registration failed for refresh")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let client_id = app_json["client_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing client_id".into()))?
            .to_string();
        let client_secret = app_json["client_secret"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing client_secret".into()))?
            .to_string();

        let body = serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "grant_type": "refresh_token",
            "refresh_token": token,
            "scope": self.scopes().join(" "),
        });

        let resp = self
            .http
            .post(self.api_url("/oauth/token"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]
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

    /// Delete a published status.
    ///
    /// Mastodon gates `DELETE /api/v1/statuses/{id}` on the `write:statuses`
    /// scope, which the `write` scope the app already asks for covers.
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on Mastodon".into(),
            ));
        }
        let resp = self
            .http
            .delete(self.api_url(&format!("/api/v1/statuses/{platform_post_id}")))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let http_status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if http_status.is_success() {
            return Ok(());
        }
        Err(self.provider_error(&body, http_status.as_u16()))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Upload media if present
        let mut media_ids: Vec<String> = Vec::new();
        for media in &post.media {
            let mime = if media.mime_type.is_empty() {
                "image/jpeg"
            } else {
                &media.mime_type
            };
            let alt = media.alt.as_deref();
            let id = self
                .upload_media(access_token, &media.url, mime, alt)
                .await?;
            media_ids.push(id);
        }

        // Read settings
        let visibility = post
            .settings
            .get("visibility")
            .and_then(|v| v.as_str())
            .unwrap_or("public");
        // Thread linking: prefer the explicit `in_reply_to` field on
        // PostContent (set by the scheduler for group_id threads), fall
        // back to `settings.in_reply_to_id` for backward compat.
        let in_reply_to_id = post.in_reply_to.clone().or_else(|| {
            post.settings
                .get("in_reply_to_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

        // Build the status body
        let mut body = serde_json::json!({
            "status": post.content,
            "visibility": visibility,
        });

        if !media_ids.is_empty() {
            body["media_ids"] = serde_json::json!(media_ids);
        }
        if let Some(reply_to) = in_reply_to_id {
            body["in_reply_to_id"] = serde_json::json!(reply_to);
        }
        // Add spoiler_text if present
        if let Some(spoiler) = post.settings.get("spoiler_text").and_then(|v| v.as_str()) {
            body["spoiler_text"] = serde_json::json!(spoiler);
        }
        // Add language if present
        if let Some(lang) = post.settings.get("language").and_then(|v| v.as_str()) {
            body["language"] = serde_json::json!(lang);
        }
        // Add sensitivity
        if let Some(sensitive) = post.settings.get("sensitive").and_then(|v| v.as_bool()) {
            body["sensitive"] = serde_json::json!(sensitive);
        }

        let resp = self
            .http
            .post(self.api_url("/api/v1/statuses"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Publish failed")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }

        let post_id = json["id"].as_str().unwrap_or("").to_string();
        let post_url = json["url"].as_str().map(|u| u.to_string());

        Ok(PublishResult {
            platform_post_id: post_id,
            platform_post_url: post_url,
            status: "published".into(),
        })
    }

    /// Return Mastodon account info (Mastodon has single-user OAuth by default).
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let info = self.get_user_info(access_token).await?;
        Ok(vec![PageInfo {
            id: info["id"].as_str().unwrap_or("").to_string(),
            name: info["display_name"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture: info["avatar"].as_str().map(String::from),
            username: info["username"].as_str().map(String::from),
        }])
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let info = self.get_account(access_token, page_id).await?;
        Ok(PageInfo {
            id: info["id"].as_str().unwrap_or("").to_string(),
            name: info["display_name"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture: info["avatar"].as_str().map(String::from),
            username: info["username"].as_str().map(String::from),
        })
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let resp = self
            .http
            .get(self.api_url(&format!(
                "/api/v1/accounts/{}/statuses?limit={}",
                _internal_id, _limit
            )))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }

        if !status.is_success() {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Failed to fetch statuses")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }

        let posts: Vec<ExternalPostData> = json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        // Skip undated items rather than stamp "now" on them.
                        let created_at = s["created_at"]
                            .as_str()
                            .and_then(crate::social::common::parse_timestamp_opt)?;
                        let media_urls: Vec<String> = s["media_attachments"]
                            .as_array()
                            .map(|ma| {
                                ma.iter()
                                    .filter_map(|m| m["url"].as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let media: Vec<MediaAttachment> = media_urls
                            .into_iter()
                            .map(|url| MediaAttachment {
                                url,
                                mime_type: "image/jpeg".into(),
                                alt: None,
                                poster_url: None,
                            })
                            .collect();
Some(                        ExternalPostData {
                            platform_post_id: s["id"].as_str().unwrap_or("").to_string(),
                            text: s["content"].as_str().unwrap_or("").to_string(),
                            author_name: s["account"]["display_name"].as_str().map(String::from),
                            author_handle: s["account"]["acct"].as_str().map(String::from),
                            author_avatar: s["account"]["avatar"].as_str().map(String::from),
                            created_at,
                            url: s["url"].as_str().map(String::from),
                            media,
                            metadata: Some(serde_json::json!({
                                "visibility": s["visibility"],
                                "favourites_count": s["favourites_count"],
                                "reblogs_count": s["reblogs_count"],
                                "replies_count": s["replies_count"],
                                "sensitive": s["sensitive"],
                                "spoiler_text": s["spoiler_text"],
                            })),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(posts)
    }

    fn map_error(&self, body: &str, _status: u16) -> Option<String> {
        // Try to extract error from Mastodon's JSON error format
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(msg) = v["error"].as_str() {
                return Some(msg.to_string());
            }
        }
        None
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "status": post.content,
            "in_reply_to_id": comment_id,
        });
        let resp = self
            .http
            .post(self.api_url("/api/v1/statuses"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            let post_id = json["id"].as_str().unwrap_or("").to_string();
            let post_url = json["url"].as_str().map(|s| s.to_string());
            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: post_url,
                status: "published".into(),
            })
        } else {
            let msg = json["error"]
                .as_str()
                .unwrap_or("Mastodon reply failed")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }
}
