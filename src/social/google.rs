// ─── Google Suite Provider (unified OAuth entry) ──────────────
// F2: GoogleProvider is now ONLY the registry's "google" identity — the
// OAuth connect flow (combined scopes), YouTube channel page selection,
// and YouTube video publishing. The per-API methods (Gmail/Calendar/Drive
// and the YouTube read helpers) previously duplicated here live canonically
// in gmail.rs / calendar.rs / drive.rs / youtube.rs, which tools_google
// dispatches to. Token refresh is shared: google_oauth::refresh().

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct GoogleProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl GoogleProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("youtube").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }
}

#[async_trait]
impl SocialProvider for GoogleProvider {
    fn identifier(&self) -> &'static str {
        "google"
    }

    fn name(&self) -> &'static str {
        "Google Suite"
    }

    fn scopes(&self) -> Vec<String> {
        // ─── Why the Drive scopes are NOT here ──────────────────
        // Google's authorize endpoint refuses some scope PAIRS outright,
        // before any consent screen exists. On a live deployment
        // (2026-10-30) this provider requested YouTube + Gmail + Calendar +
        // Drive together and Google answered:
        //
        //   Error 400: invalid_request
        //   This request contains scopes that cannot be requested together :
        //   [https://www.googleapis.com/auth/youtube,
        //    https://www.googleapis.com/auth/youtube.upload,
        //    https://www.googleapis.com/auth/youtube.force-ssl,
        //    https://www.googleapis.com/auth/drive.file]
        //
        // The entire connect flow was therefore unreachable — no consent
        // screen, no token, and an error that names no fix.
        //
        // So the set is narrowed to what Google grants together. Each
        // dropped scope was chosen for a reason, not merely to make the
        // URL pass:
        //
        //   * `drive.*` (both) — the conflicting pair. Drive is now its own
        //     connectable provider (`social::drive`, identifier `drive`),
        //     so the MCP Drive tools still have a token; see that module.
        //   * `gmail.modify` — a Drive-grade grant, refused next to the
        //     YouTube scopes. Labels stay readable and a compose draft is
        //     the highest write this product performs.
        //   * `calendar.events` — implied by the `calendar` scope. Requesting
        //     both is how this whole class of failure starts.
        //
        // `scope_guard::google_scope_conflict` enforces the combination on
        // every authorize request, so a future edit that re-adds one of
        // these fails loudly at connect time instead of on Google's page.
        vec![
            // YouTube
            "https://www.googleapis.com/auth/youtube".into(),
            "https://www.googleapis.com/auth/youtube.upload".into(),
            "https://www.googleapis.com/auth/youtube.force-ssl".into(),
            "https://www.googleapis.com/auth/userinfo.profile".into(),
            // Gmail: `send` is the app's outbound mail path; `readonly`
            // is the inbox.
            "https://www.googleapis.com/auth/gmail.readonly".into(),
            "https://www.googleapis.com/auth/gmail.send".into(),
            // Calendar: read the founder's own schedule. `calendar.events`
            // is implied by `calendar` and requesting both is how the
            // "cannot be requested together" class of failure starts.
            "https://www.googleapis.com/auth/calendar".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        5000
    }

    fn is_between_steps(&self) -> bool {
        true
    }

    fn needs_cron_refresh(&self) -> bool {
        true
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let scope = self.scopes().join(" ");
        // Refuse an incompatible combination HERE, where the error can name
        // the scopes and the fix, instead of letting Google's authorize
        // endpoint answer `invalid_request` on its own page.
        if let Err((a, b, _)) =
            super::scope_guard::google_scope_conflict(&self.scopes(), self.identifier())
        {
            return Err(ProviderError::Auth(
                super::scope_guard::google_scope_conflict_message(&a, &b, self.identifier()),
            ));
        }
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
            "google",
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

    /// List YouTube channels for page selection
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

    /// Delete a published YouTube video (the `google` provider uploads
    /// videos through the YouTube Data API).
    ///
    /// Without this, the trait's default refused the delete — "This
    /// provider does not support deleting a published post" — so a video
    /// uploaded through the Google Suite channel could never be taken down
    /// from the app. Measured live on 2026-11-13: two test videos were
    /// live on the founder's channel with no way to remove them.
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on YouTube".into(),
            ));
        }
        let resp = self
            .http
            .delete("https://youtube.googleapis.com/youtube/v3/videos")
            .query(&[("id", platform_post_id), ("access_token", access_token)])
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

        // YouTube title max is 100 chars
        let title: String = post.content.chars().take(100).collect();
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

        // C1: the resumable-upload flow lives once in youtube.rs; this
        // provider (and youtube.rs's own publish) both call it.
        let json = super::youtube::resumable_upload(
            &self.http,
            access_token,
            &metadata,
            video_bytes,
            mime_type,
        )
        .await?;
        let video_id = json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api("YouTube did not return a video ID".into()))?;

        Ok(PublishResult {
            platform_post_id: video_id.to_string(),
            platform_post_url: Some(format!("https://www.youtube.com/watch?v={video_id}")),
            status: "published".into(),
        })
    }

    fn map_error(&self, _body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Google token expired. Re-authenticate via Google OAuth.".into())
        } else if status == 403 {
            Some("Google API access forbidden. Check token scopes.".into())
        } else if status == 429 {
            Some("Google API rate limit exceeded. Try again later.".into())
        } else {
            None
        }
    }
}
