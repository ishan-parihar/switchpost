// ─── LinkedIn Page Provider ────────────────────────────────────
// Extends LinkedIn with org-level posting (companies/pages).
// Same OAuth credentials as LinkedIn but uses organization URNs.

use async_trait::async_trait;

use super::*;
use super::common::extract_access_token;
use crate::config::Config;
use crate::social::linkedin::LINKEDIN_VERSION;

pub struct LinkedInPageProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl LinkedInPageProvider {
    async fn publish_to_org(
        &self,
        access_token: &str,
        org_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "author": format!("urn:li:organization:{org_id}"),
            "lifecycleState": "PUBLISHED",
            "specificContent": {
                "com.linkedin.ugc.ShareContent": {
                    "shareCommentary": {
                        "text": post.content,
                    },
                    "shareMediaCategory": if post.media.is_empty() { "NONE" } else { "IMAGE" },
                }
            },
            "visibility": {
                "com.linkedin.ugc.MemberNetworkVisibility": "PUBLIC"
            }
        });

        let resp = self
            .http
            .post("https://api.linkedin.com/v2/ugcPosts")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();

        if status == 201 {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let post_id = location.rsplit('/').next().unwrap_or("").to_string();
            return Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: None,
                status: "published".into(),
            });
        }

        let json: serde_json::Value = resp.json().await?;

        if status == 401 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn Page publish failed")
                .to_string();
            let raw = format!("LinkedIn Page publish failed (HTTP {status}): {msg}");
            Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &raw,
            )))
        }
    }
    /// Publish to a SPECIFIC organization.
    ///
    /// `publish` resolves one (the first the token can reach) and calls
    /// this; `post_pending` resolves the one the CHANNEL is connected to
    /// and calls the same body.
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) =
            config.provider_credentials("linkedin").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    fn token_url(&self) -> &'static str {
        "https://www.linkedin.com/oauth/v2/accessToken"
    }

    // ── Analytics parser (pure: fixture-testable, no HTTP) ───

    /// Walk a LinkedIn REST share-statistics response (`elements[].totalShareStatistics`)
    /// into dashboard rows. Both the org-wide endpoint and the per-share
    /// endpoint return this shape, so one parser serves `analytics()` and
    /// `post_analytics()`.
    pub(crate) fn share_statistics_analytics(json: &serde_json::Value) -> Vec<AnalyticsData> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut out = Vec::new();
        let Some(elements) = json["elements"].as_array() else {
            return out;
        };
        for element in elements {
            let Some(stats) = element["totalShareStatistics"].as_object() else {
                continue;
            };
            for (key, val) in stats {
                let Some(n) = val.as_u64() else { continue };
                out.push(AnalyticsData {
                    label: key.clone(),
                    data: vec![AnalyticsDataPoint {
                        total: n.to_string(),
                        date: today.clone(),
                    }],
                    percentage_change: 0.0,
                });
            }
        }
        out
    }

    /// Fetch organization logo via REST API (fallback when decoration doesn't work)
    async fn fetch_org_logo(
        &self,
        access_token: &str,
        org_id: &str,
    ) -> Result<String, ProviderError> {
        // Use the REST API with version header to get logoV2 URN
        let resp = self
            .http
            .get(format!(
                "https://api.linkedin.com/rest/organizations/{org_id}?fields=logoV2"
            ))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        // logoV2.original is a URN like "urn:li:image:C560BAQ..."
        if let Some(urn) = json["logoV2"]["original"].as_str() {
            // Resolve the image URN to a download URL
            let encoded = urlencoding::encode(urn);
            let img_resp = self
                .http
                .get(format!("https://api.linkedin.com/rest/images/{encoded}"))
                .header("Authorization", format!("Bearer {access_token}"))
                .header("LinkedIn-Version", LINKEDIN_VERSION)
                .send()
                .await?;
            let img_json: serde_json::Value = img_resp.json().await.unwrap_or_default();
            if let Some(url) = img_json["downloadUrl"].as_str() {
                return Ok(url.to_string());
            }
        }
        Err(ProviderError::Api("No logo found".into()))
    }

    fn authorize_url(&self) -> &'static str {
        "https://www.linkedin.com/oauth/v2/authorization"
    }
}

#[async_trait]
impl SocialProvider for LinkedInPageProvider {
    fn identifier(&self) -> &'static str {
        "linkedin-page"
    }

    fn name(&self) -> &'static str {
        "LinkedIn Page"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "openid".into(),
            "profile".into(),
            "email".into(),
            "w_member_social".into(),
            "rw_organization_admin".into(),
            "w_organization_social".into(),
            "r_organization_social".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        3000
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    fn is_between_steps(&self) -> bool {
        true
    }

    /// Organization tokens use the same rotating refresh token as the personal
    /// LinkedIn provider (~60 day lifetime), so they need the same cron pass.
    fn needs_cron_refresh(&self) -> bool {
        true
    }

    /// Page tokens rotate on refresh and the new value is not immediately
    /// usable — same propagation delay as the personal LinkedIn provider.
    fn refresh_wait(&self) -> bool {
        true
    }

    fn tooltip(&self) -> Option<&'static str> {
        Some("Post to a LinkedIn Company Page you administer")
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let params = [
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", redirect_uri),
            ("scope", &self.scopes().join(" ")),
            ("state", state),
        ];

        let url = url::Url::parse_with_params(self.authorize_url(), &params)
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
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
        ];

        let resp = self
            .http
            .post(self.token_url())
            .form(&params)
            .send()
            .await?;
        let (access_token, json) = extract_access_token(resp, "linkedin-page")
            .await
            .map_err(ProviderError::Auth)?;

        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        let profile = self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let sub = profile["sub"].as_str().unwrap_or("").to_string();
        let name = profile["name"].as_str().unwrap_or("").to_string();
        let username = profile["preferred_username"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let picture = profile["picture"].as_str().map(String::from);

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: sub,
            name,
            username,
            picture,
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
        ];

        let resp = self
            .http
            .post(self.token_url())
            .form(&params)
            .send()
            .await?;
        let (access_token, json) = extract_access_token(resp, "linkedin-page")
            .await
            .map_err(ProviderError::Auth)?;

        // Fetch profile info so it's not lost on refresh.
        // Use async match — page-scoped tokens may not have openid/profile scope.
        let profile = match self
            .http
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await
        {
            Ok(r) => r.json::<serde_json::Value>().await.ok(),
            Err(_) => None,
        };

        Ok(AuthToken {
            access_token,
            refresh_token: json["refresh_token"].as_str().map(String::from),
            expires_in: json["expires_in"].as_u64().map(|v| v as u32),
            provider_user_id: profile
                .as_ref()
                .and_then(|p| p["sub"].as_str())
                .unwrap_or("")
                .to_string(),
            name: profile
                .as_ref()
                .and_then(|p| p["name"].as_str())
                .unwrap_or("")
                .to_string(),
            username: profile
                .as_ref()
                .and_then(|p| p["preferred_username"].as_str())
                .unwrap_or("")
                .to_string(),
            picture: profile
                .as_ref()
                .and_then(|p| p["picture"].as_str())
                .map(String::from),
        })
    }

    /// List organizations the user can administer
    /// Publish to the organization this channel is CONNECTED to.
    ///
    /// A LinkedIn token can administer several organizations, and
    /// `resolve_org_id` takes the first one it lists — so a channel
    /// connected to organization B had every scheduled post land in
    /// organization A. The scheduler knows the channel; this override is
    /// what carries it into the publish.
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let org_id = self
            .resolve_org_id_for(access_token, Some(internal_id))
            .await?;
        self.publish_to_org(access_token, &org_id, post).await
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .http
            .get("https://api.linkedin.com/v2/organizationalEntityAcls?q=roleAssignee&role=ADMINISTRATOR&projection=(elements*(organizationalTarget~(localizedName,vanityName,logoV2(original~:playableStreams))))")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let elements = json["elements"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();

        let mut pages = Vec::new();
        for e in &elements {
            let target = &e["organizationalTarget~"];
            let id = e["organizationalTarget"]
                .as_str()
                .unwrap_or("")
                .split(':')
                .next_back()
                .unwrap_or("")
                .to_string();

            // Try decoration first, fall back to separate logo fetch
            let mut picture = target["logoV2"]["original~"]["elements"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|el| el["identifiers"].as_array())
                .and_then(|ids| ids.first())
                .and_then(|id_val| id_val["identifier"].as_str())
                .map(String::from);

            // Fallback: fetch logo via /rest/organizations endpoint
            if picture.is_none() && !id.is_empty() {
                if let Ok(logo_url) = self.fetch_org_logo(access_token, &id).await {
                    picture = Some(logo_url);
                }
            }

            pages.push(PageInfo {
                id,
                name: target["localizedName"].as_str().unwrap_or("").to_string(),
                access_token: Some(access_token.to_string()),
                picture,
                username: target["vanityName"].as_str().map(String::from),
            });
        }

        Ok(pages)
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let resp = self
            .http
            .get(format!("https://api.linkedin.com/v2/organizations/{page_id}?projection=(id,localizedName,vanityName,logoV2(original~:playableStreams))"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        let id_str = json["id"]
            .as_u64()
            .map(|n| n.to_string())
            .or_else(|| json["id"].as_str().map(String::from))
            .unwrap_or_else(|| page_id.to_string());

        let mut picture = json["logoV2"]["original~"]["elements"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|el| el["identifiers"].as_array())
            .and_then(|ids| ids.first())
            .and_then(|id| id["identifier"].as_str())
            .map(String::from);

        if picture.is_none() {
            if let Ok(logo_url) = self.fetch_org_logo(access_token, page_id).await {
                picture = Some(logo_url);
            }
        }

        Ok(PageInfo {
            id: id_str,
            name: json["localizedName"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture,
            username: json["vanityName"].as_str().map(String::from),
        })
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

    /// §8.2: delete a published LinkedIn Page post (same /v2/rest/posts
    /// endpoint as personal; the page token authorizes it).
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on the page".into(),
            ));
        }
        let url = format!(
            "https://api.linkedin.com/v2/rest/posts/{}",
            urlencoding::encode(platform_post_id)
        );
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header(
                "LinkedIn-Version",
                crate::social::linkedin::LINKEDIN_VERSION,
            )
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() || status.as_u16() == 204 {
            Ok(())
        } else {
            Err(ProviderError::Api(format!(
                "LinkedIn Page delete failed (HTTP {status})"
            )))
        }
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Resolve org ID — internal_id tells us which org to post as
        let org_id = self.resolve_org_id(access_token).await?;
        self.publish_to_org(access_token, &org_id, post).await
    }

    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        // Page-specific copy first: these are the two failures only an
        // organization token hits, and they read better than the generic
        // bucket text.
        if body.contains("Unable to obtain activity") {
            return Some("Unable to obtain activity. Please try again.".into());
        }
        if body.contains("resource is forbidden") {
            return Some("Resource is forbidden. Check your organization permissions.".into());
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "LinkedIn rate limit hit. LinkedIn allows roughly 100 API calls per day \
                 for most apps — wait for the daily quota to reset."
                    .into(),
            ),
            Some(ErrorKind::Duplicate) => Some(
                "LinkedIn flagged this Page post as a duplicate of recent content. \
                 Vary the text or publish again later."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "LinkedIn rejected the media. Page posts accept JPEG/PNG/GIF images or \
                 MP4 video, up to 20 items, each from a publicly reachable URL."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "LinkedIn access token expired or was revoked. Reconnect the Page to \
                 refresh it (the token lives ~60 days)."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "LinkedIn denied this request. The token needs r_organization_social \
                 and the Page admin role, or the resource belongs to another organization."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "LinkedIn Page, post, or organization not found. Re-pick the Page on the \
                 Channels screen if it was transferred or renamed."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "LinkedIn rejected the request as invalid. Check the organization URN and \
                 that the post visibility is one LinkedIn allows for organizations."
                    .into(),
            ),
            None => None,
        }
    }

    async fn analytics(
        &self,
        access_token: &str,
        internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let org_id = if internal_id.is_empty() {
            self.resolve_org_id(access_token).await?
        } else {
            internal_id.to_string()
        };
        let org_urn = format!("urn:li:organization:{org_id}");

        let mut results = Vec::new();

        // Share statistics
        let share_url = format!(
            "https://api.linkedin.com/rest/organizationalEntityShareStatistics?q=organizationalEntity&organizationalEntity={org_urn}"
        );
        let resp = self
            .http
            .get(&share_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("LinkedIn API rate limit".into()));
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn share statistics error")
                .to_string();
            let raw = format!("LinkedIn share statistics error (HTTP {status}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &raw,
            )));
        }

        results.extend(Self::share_statistics_analytics(&json));

        // Follower count
        let follower_url = format!(
            "https://api.linkedin.com/rest/networkSizes/{org_urn}?edgeType=CompanyFollowedByMember"
        );
        let resp = self
            .http
            .get(&follower_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("LinkedIn API rate limit".into()));
        }

        if status.is_success() {
            let json: serde_json::Value = resp.json().await.unwrap_or_default();
            if let Some(elements) = json["elements"].as_array() {
                if let Some(elem) = elements.first() {
                    if let Some(count) = elem["firstDegreeSize"].as_u64() {
                        results.push(AnalyticsData {
                            label: "followerCount".into(),
                            data: vec![AnalyticsDataPoint {
                                total: count.to_string(),
                                date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                            }],
                            percentage_change: 0.0,
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let url =
            format!("https://api.linkedin.com/rest/shares/{platform_post_id}/shareStatistics");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .header("X-Restli-Protocol-Version", "2.0.0")
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if status == 429 {
            return Err(ProviderError::RateLimited("LinkedIn API rate limit".into()));
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn post statistics error")
                .to_string();
            let raw = format!("LinkedIn post statistics error (HTTP {status}): {msg}");
            return Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &raw,
            )));
        }

        Ok(Self::share_statistics_analytics(&json))
    }

    // ── Import recent posts from LinkedIn Page ────────────────
    async fn get_recent_posts(
        &self,
        access_token: &str,
        internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let page_id = if internal_id.is_empty() {
            self.resolve_org_id(access_token).await?
        } else {
            internal_id.to_string()
        };
        // Try fetching with the given page_id; if it fails, resolve the correct org ID
        let json = match self.get_page_posts(access_token, &page_id, limit).await {
            Ok(json) => json,
            Err(e)
                if e.to_string().contains("No virtual resource")
                    || e.to_string().contains("not found") =>
            {
                let msg = e.to_string();
                tracing::warn!(
                    "LinkedIn page posts failed with '{}' for page_id={}, resolving correct org ID...",
                    msg, page_id,
                );
                let resolved_id = self.resolve_org_id(access_token).await?;
                self.get_page_posts(access_token, &resolved_id, limit)
                    .await?
            }
            Err(e) => return Err(e),
        };

        let mut posts = Vec::new();
        if let Some(elements) = json["elements"].as_array() {
            for element in elements {
                let post_urn = element["id"].as_str().unwrap_or("").to_string();
                let commentary = element["commentary"]
                    .as_object()
                    .and_then(|c| c["text"].as_str())
                    .unwrap_or("")
                    .to_string();
                // Skip undated items rather than stamp "now" on them.
                let Some(created_at) = element["createdAt"]
                    .as_i64()
                    .and_then(chrono::DateTime::from_timestamp_millis)
                else {
                    continue;
                };

                // Extract media from the post content
                let mut media = Vec::new();
                const MAX_MEDIA_PER_POST: usize = 4;
                if let Some(content) = element.get("content") {
                    if !content.is_null() {
                        // Single media: content.media { id: "urn:li:image:xxx" }
                        if let Some(m) = content.get("media") {
                            if let Some(media_urn) = m.get("id").and_then(|v| v.as_str()) {
                                if let Some(url) =
                                    self.resolve_media_url(access_token, media_urn).await
                                {
                                    let is_video = media_urn.contains(":video:");
                                    media.push(MediaAttachment {
                                        url,
                                        mime_type: if is_video {
                                            "video/mp4".to_string()
                                        } else {
                                            "image/jpeg".to_string()
                                        },
                                        alt: m
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .map(String::from),
                                        poster_url: None,
                                    });
                                }
                            }
                        }
                        // Multi-image / carousel: content.multiImage array
                        if media.is_empty() {
                            if let Some(multi) =
                                content.get("multiImage").and_then(|v| v.as_array())
                            {
                                for img in multi.iter().take(MAX_MEDIA_PER_POST) {
                                    if let Some(media_urn) = img.get("id").and_then(|v| v.as_str())
                                    {
                                        if let Some(url) =
                                            self.resolve_media_url(access_token, media_urn).await
                                        {
                                            media.push(MediaAttachment {
                                                url,
                                                mime_type: "image/jpeg".to_string(),
                                                alt: None,
                                                poster_url: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        // Article with thumbnail
                        if media.is_empty() {
                            if let Some(article) = content.get("article") {
                                if let Some(thumb) =
                                    article.get("thumbnail").and_then(|v| v.as_str())
                                {
                                    if !thumb.is_empty() {
                                        media.push(MediaAttachment {
                                            url: thumb.to_string(),
                                            mime_type: "image/jpeg".to_string(),
                                            alt: article
                                                .get("title")
                                                .and_then(|v| v.as_str())
                                                .map(String::from),
                                            poster_url: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                let post_url = Some(format!("https://www.linkedin.com/feed/update/{post_urn}"));
                posts.push(ExternalPostData {
                    platform_post_id: post_urn,
                    text: commentary,
                    author_name: None,
                    author_handle: None,
                    author_avatar: None,
                    created_at,
                    url: post_url,
                    media,
                    metadata: Some(element.clone()),
                });
            }
        }
        Ok(posts)
    }

    // ── Fetch engagement for a Page post ─────────────────────
    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        // Use the social actions API to get likes/comments/shares counts
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{platform_post_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            return Ok(None);
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let counts = super::linkedin::LinkedInProvider::social_action_counts(&json);
        if counts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(serde_json::Value::Object(counts)))
        }
    }

    /// Page posts use the same `socialActions/{urn}/comments` endpoint as
    /// member posts, so the shared reader serves both providers.
    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://api.linkedin.com/v2/rest/socialActions/{platform_post_id}/comments"
            ))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            return Ok(vec![]);
        }
        Ok(super::parse_linkedin_comments(&json))
    }

    /// Replies are authored by the organization, not the member behind the
    /// token, so the actor URN comes from `resolve_org_id`.
    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let org_id = self.resolve_org_id(access_token).await?;
        let json = self
            .create_comment(
                access_token,
                comment_id,
                &format!("urn:li:organization:{org_id}"),
                &post.content,
            )
            .await?;
        let id = json["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(ProviderError::Api(format!(
                "LinkedIn Page reply failed: {json}"
            )));
        }
        Ok(PublishResult {
            platform_post_id: id,
            platform_post_url: None,
            status: "published".into(),
        })
    }

    /// Organizations this token administers are the posting targets.
    async fn targets(&self, access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        Ok(super::pages_to_targets(
            &self.pages(access_token).await?,
            "page",
        ))
    }
}

impl LinkedInPageProvider {}

impl LinkedInPageProvider {
    /// Resolve a LinkedIn media URN (e.g. "urn:li:image:12345") to a direct download/playback URL.
    pub async fn resolve_media_url(&self, access_token: &str, media_urn: &str) -> Option<String> {
        let parts: Vec<&str> = media_urn.split(':').collect();
        if parts.len() < 4 {
            return None;
        }
        let asset_type = parts[2];
        let asset_id = parts[3];
        let base_url = match asset_type {
            "image" => "https://api.linkedin.com/rest/images",
            "video" => "https://api.linkedin.com/rest/videos",
            _ => return None,
        };
        let url = format!("{base_url}/{asset_id}");
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.http
                .get(&url)
                .header("Authorization", format!("Bearer {access_token}"))
                .header("X-Restli-Protocol-Version", "2.0.0")
                .header("LinkedIn-Version", LINKEDIN_VERSION)
                .send(),
        )
        .await;
        match resp {
            Ok(Ok(r)) if r.status().is_success() => {
                if let Ok(json) = r.json::<serde_json::Value>().await {
                    if let Some(dl) = json.pointer("/value/downloadUrl").and_then(|v| v.as_str()) {
                        return Some(dl.to_string());
                    }
                    if let Some(streams) = json
                        .pointer("/value/playbackStreams")
                        .and_then(|v| v.as_array())
                    {
                        if let Some(stream) = streams.first() {
                            if let Some(s) =
                                stream.get("streamLocations").and_then(|v| v.as_array())
                            {
                                if let Some(loc) = s.first() {
                                    if let Some(u) = loc.get("url").and_then(|v| v.as_str()) {
                                        return Some(u.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }

    async fn resolve_org_id(&self, access_token: &str) -> Result<String, ProviderError> {
        self.resolve_org_id_for(access_token, None).await
    }

    /// Resolve the organization to post as, preferring the CHANNEL's org.
    ///
    /// `resolve_org_id` takes `pages.first()` — the first organization the
    /// token can post to — which is arbitrary for a token that administers
    /// several. The founder picks one in the connect flow and the channel's
    /// `internal_id` records it, so a post aimed at channel B went to
    /// organization A. This override is what closes that.
    ///
    /// `internal_id = None` (the REST/CLI/MCP direct path, which has no
    /// channel identity) keeps the old behaviour.
    async fn resolve_org_id_for(
        &self,
        access_token: &str,
        internal_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        if let Some(want) = internal_id.filter(|w| !w.is_empty()) {
            // The channel names its org. Trust it, but only once it is
            // still reachable: an org the token lost (an admin removed
            // them, a role changed) must fail loudly rather than be
            // silently resolved to a different one.
            let pages = self.pages(access_token).await?;
            if pages.iter().any(|p| p.id == want) {
                return Ok(want.to_string());
            }
            return Err(ProviderError::Auth(format!(
                "This channel is connected to organization {want}, which the token can no longer \
                 post to. Reconnect the channel."
            )));
        }
        let pages = self.pages(access_token).await?;
        pages
            .first()
            .map(|p| p.id.clone())
            .ok_or_else(|| ProviderError::Auth("No LinkedIn organizations found".into()))
    }

    pub async fn get_page_posts(
        &self,
        access_token: &str,
        page_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = limit.clamp(1, 100);
        let author_urn = format!("urn:li:organization:{page_id}");
        let url =
            format!("https://api.linkedin.com/v2/rest/posts?author={author_urn}&count={limit}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("LinkedIn Page API error")
                .to_string();
            tracing::warn!("LinkedIn get_page_posts HTTP {status} for page_id={page_id}: {msg}");
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn create_comment(
        &self,
        access_token: &str,
        post_urn: &str,
        page_urn: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.linkedin.com/v2/rest/socialActions/{post_urn}/comments");
        let body = serde_json::json!({
            "actor": page_urn,
            "message": { "text": message },
            "object": post_urn
        });
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header("LinkedIn-Version", LINKEDIN_VERSION)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();

        if status == 201 {
            let json: serde_json::Value = resp.json().await?;
            Ok(json)
        } else {
            let json: serde_json::Value = resp.json().await?;
            if status == 401 {
                Err(ProviderError::TokenExpired)
            } else {
                let msg = json["message"]
                    .as_str()
                    .unwrap_or("LinkedIn Page API error")
                    .to_string();
                Err(self.provider_error(&msg, status.as_u16()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    fn provider() -> LinkedInPageProvider {
        LinkedInPageProvider::new(&test_config())
    }

    #[test]
    fn identifier_is_linkedin_page() {
        assert_eq!(provider().identifier(), "linkedin-page");
    }

    // ── B2c: share-statistics parser fixtures ────────────────

    #[test]
    fn share_statistics_analytics_flattens_total_share_statistics() {
        let fixture = serde_json::json!({
            "elements": [{
                "totalShareStatistics": {
                    "impressionCount": 1200,
                    "engagement": 47,
                    "clickCount": 33
                }
            }]
        });
        let rows = LinkedInPageProvider::share_statistics_analytics(&fixture);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["impressionCount", "engagement", "clickCount"]);
        assert_eq!(rows[0].data[0].total, "1200");
        assert!(!rows[0].data[0].date.is_empty(), "rows must carry a date");
    }

    #[test]
    fn share_statistics_analytics_sums_multiple_elements() {
        let fixture = serde_json::json!({
            "elements": [
                { "totalShareStatistics": { "impressionCount": 10 } },
                { "totalShareStatistics": { "impressionCount": 5 } }
            ]
        });
        let rows = LinkedInPageProvider::share_statistics_analytics(&fixture);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn share_statistics_analytics_empty_for_null_elements() {
        let fixture = serde_json::json!({ "elements": null });
        assert!(LinkedInPageProvider::share_statistics_analytics(&fixture).is_empty());
    }

    // ── B2c: map_error fixtures ─────────────────────────────

    #[test]
    fn map_error_keeps_activity_specific_copy() {
        assert_eq!(
            provider()
                .map_error("Unable to obtain activity", 500)
                .unwrap(),
            "Unable to obtain activity. Please try again."
        );
    }

    #[test]
    fn map_error_keeps_forbidden_resource_copy() {
        assert_eq!(
            provider().map_error("resource is forbidden", 403).unwrap(),
            "Resource is forbidden. Check your organization permissions."
        );
    }

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let msg = provider().map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_duplicate_page_post() {
        let msg = provider()
            .map_error("This content is a duplicate", 400)
            .unwrap();
        assert!(msg.contains("duplicate"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let msg = provider()
            .map_error("The image upload was rejected", 422)
            .unwrap();
        assert!(msg.contains("media"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let msg = provider().map_error("", 401).unwrap();
        assert!(msg.contains("expired"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_scope_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("r_organization_social"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 503).is_none());
    }
}
