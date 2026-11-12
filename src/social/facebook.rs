// ─── Facebook Provider ────────────────────────────────────────
// OAuth 2.0 + Graph API. Posts to Facebook Page (not personal timeline).

use async_trait::async_trait;

use super::common::extract_access_token;
use super::*;
use crate::config::Config;
use crate::social::GRAPH_VERSION;

pub struct FacebookProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl FacebookProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) =
            config.provider_credentials("facebook").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    fn graph_url(&self) -> String {
        format!("https://graph.facebook.com/{GRAPH_VERSION}")
    }
}

/// C3: extend a long-lived FB-family user token (fb_exchange_token
/// grant). A FREE function so the business-Instagram provider shares it
/// (its tokens are FB user tokens minted by the IG app's own credentials)
/// without depending on a FacebookProvider instance.
pub(crate) async fn exchange_long_lived(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    long_token: &str,
) -> Result<AuthToken, ProviderError> {
    let params: Vec<(&str, &str)> = vec![
        ("grant_type", "fb_exchange_token"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("fb_exchange_token", long_token),
    ];
    let resp = http
        .get(format!(
            "https://graph.facebook.com/{GRAPH_VERSION}/oauth/access_token"
        ))
        .query(&params)
        .send()
        .await?;
    let status = resp.status();
    let json: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        let msg = json["error"]["message"]
            .as_str()
            .unwrap_or("token exchange failed");
        return Err(ProviderError::Auth(format!(
            "Facebook token refresh failed (HTTP {status}): {msg}"
        )));
    }
    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| ProviderError::Auth("refresh returned no access_token".into()))?
        .to_string();
    let expires_in = json["expires_in"].as_u64().map(|v| v as u32);
    Ok(AuthToken {
        // The new long-lived token doubles as the NEXT refresh grant.
        refresh_token: Some(access_token.clone()),
        access_token,
        expires_in,
        provider_user_id: String::new(),
        name: String::new(),
        username: String::new(),
        picture: None,
    })
}

#[async_trait]
impl SocialProvider for FacebookProvider {
    fn identifier(&self) -> &'static str {
        "facebook"
    }

    fn name(&self) -> &'static str {
        "Facebook"
    }

    fn is_between_steps(&self) -> bool {
        true
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "pages_show_list".into(),
            "pages_read_engagement".into(),
            "pages_manage_posts".into(),
            "business_management".into(),
            "pages_manage_engagement".into(),
            "pages_manage_metadata".into(),
            "pages_read_user_content".into(),
            "public_profile".into(),
            "read_insights".into(),
            "pages_messaging".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        63206
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let scope = self.scopes().join(",");
        let params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", scope.as_str()),
            ("state", state),
            ("response_type", "code"),
        ];

        let url = url::Url::parse_with_params(
            &format!("https://www.facebook.com/{GRAPH_VERSION}/dialog/oauth"),
            &params,
        )
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
        // Exchange code for access token
        let token_params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("code", code),
        ];

        let token_resp = self
            .http
            .get(format!(
                "https://graph.facebook.com/{GRAPH_VERSION}/oauth/access_token"
            ))
            .query(&token_params)
            .send()
            .await?;

        let (short_token, _token_json) = extract_access_token(token_resp, "facebook")
            .await
            .map_err(ProviderError::Auth)?;

        // Exchange short-lived token for long-lived token
        let long_params: Vec<(&str, &str)> = vec![
            ("grant_type", "fb_exchange_token"),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("fb_exchange_token", short_token.as_str()),
        ];

        let long_resp = self
            .http
            .get(format!(
                "https://graph.facebook.com/{GRAPH_VERSION}/oauth/access_token"
            ))
            .query(&long_params)
            .send()
            .await?;

        let long_json: serde_json::Value = long_resp.json().await?;
        // Keep the SHORT token only if the long-lived exchange gave
        // nothing back - but `extract_access_token` already made an
        // empty token an error, so this fallback is for the
        // long-exchange returning a differently-named field, not for
        // a failure.
        let access_token = long_json["access_token"]
            .as_str()
            .unwrap_or(&short_token)
            .to_string();
        let expires_in = long_json["expires_in"].as_u64().map(|v| v as u32);

        // Get Facebook user info (name + picture) for the root integration
        let me: serde_json::Value = self
            .http
            .get(format!(
                "{}/me?fields=id,name,picture.type(large)",
                self.graph_url()
            ))
            .query(&[("access_token", &access_token)])
            .send()
            .await?
            .json()
            .await?;

        let user_id = me["id"].as_str().unwrap_or("me").to_string();
        let user_name = me["name"].as_str().unwrap_or("Facebook User").to_string();
        let user_pic = me["picture"]["data"]["url"].as_str().map(String::from);

        tracing::info!(
            "Facebook OAuth: user '{}' ({}). Use available-pages + connect-page to link specific pages.",
            user_name, user_id
        );

        // Return the user-level token. The caller (is_between_steps = true) will
        // store this as a root integration. The user then calls:
        //   GET  /api/integrations/{id}/available-pages  — list all pages
        //   POST /api/integrations/{parent_id}/connect-page/{page_id} — connect a page
        return Ok(AuthToken {
            // C3: the long-lived token is its own refresh grant — store it
            // so refresh_token can extend the row before day 60.
            refresh_token: Some(access_token.clone()),
            access_token, // user-level token for listing pages via me/accounts
            expires_in,
            provider_user_id: user_id,
            name: user_name.clone(),
            username: user_name,
            picture: user_pic,
        });
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        // C3: extend the long-lived token instead of erroring at day 60.
        // The stored refresh token IS the long-lived token (exchange_code
        // stores it); fb_exchange_token mints a fresh ~60-day one.
        if refresh_token.trim().is_empty() {
            return Err(ProviderError::Auth(
                "No stored long-lived token — reconnect the channel once to enable \
                 automatic refresh"
                    .into(),
            ));
        }
        crate::social::facebook::exchange_long_lived(
            &self.http,
            &self.client_id,
            &self.client_secret,
            refresh_token,
        )
        .await
    }

    /// §8.2: delete a published Facebook post (Graph DELETE /{post_id}).
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on Facebook".into(),
            ));
        }
        self.delete_post(access_token, platform_post_id)
            .await
            .map(|_| ())
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Resolve the page to post to. A connected sub-account stores a PAGE
        // token, in which case /me IS the page; a user token lists pages via
        // /me/accounts and the page's own token is required to post to it.
        let (page_id, page_token) = self.resolve_page(access_token).await?;
        self.publish_to_page(access_token, page_id, page_token, post)
            .await
    }

    /// Publish to the page this channel is CONNECTED to.
    ///
    /// The scheduler knows the channel; `publish` alone does not, and a
    /// founder with three pages would have every scheduled post land on the
    /// first one their token happens to list. This override is what makes
    /// the channel identity reach the publish.
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let (page_id, page_token) = self
            .resolve_page_for(access_token, Some(internal_id))
            .await?;
        self.publish_to_page(access_token, page_id, page_token, post)
            .await
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let fields = "id,access_token,username,name,picture.type(large)";
        let mut pages_map: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        // Paginated fetch helper
        async fn collect_pages(
            http: &reqwest::Client,
            url: &str,
            access_token: &str,
            map: &mut std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(), ProviderError> {
            let mut current = url.to_string();
            loop {
                let resp = http
                    .get(&current)
                    .query(&[("access_token", access_token)])
                    .send()
                    .await?;

                let json: serde_json::Value = resp.json().await?;

                if let Some(err) = json["error"].as_object() {
                    tracing::warn!(
                        "collect_pages skipped ({:?}): {}",
                        url,
                        err["message"].as_str().unwrap_or("")
                    );
                    return Ok(());
                }

                if let Some(data) = json["data"].as_array() {
                    for page in data {
                        if let Some(pid) = page["id"].as_str() {
                            map.entry(pid.to_string()).or_insert_with(|| page.clone());
                        }
                    }
                }

                if let Some(next) = json["paging"]["next"].as_str() {
                    current = next.to_string();
                } else {
                    break;
                }
            }
            Ok(())
        }

        // Phase 1: /me/accounts — pages user selected in OAuth
        let me_url = format!(
            "{}/me/accounts?fields={}&limit=100",
            self.graph_url(),
            fields
        );
        collect_pages(&self.http, &me_url, access_token, &mut pages_map).await?;

        // Phase 2: /me/businesses → owned_pages + client_pages — Business Manager pages
        let biz_resp = self
            .http
            .get(format!("{}/me/businesses", self.graph_url()))
            .query(&[("access_token", access_token)])
            .send()
            .await?;

        let biz_json: serde_json::Value = biz_resp.json().await?;

        if biz_json["error"].is_null() {
            if let Some(businesses) = biz_json["data"].as_array() {
                for biz in businesses {
                    let biz_id = match biz["id"].as_str() {
                        Some(id) if !id.is_empty() => id,
                        _ => continue,
                    };

                    for endpoint in &["owned_pages", "client_pages"] {
                        let biz_url = format!(
                            "{}/{biz_id}/{endpoint}?fields={fields}&limit=100",
                            self.graph_url(),
                        );
                        if let Err(e) =
                            collect_pages(&self.http, &biz_url, access_token, &mut pages_map).await
                        {
                            tracing::warn!("Business Manager {endpoint} for {biz_id}: {e:?}");
                        }
                    }
                }
            }
        } else {
            tracing::debug!(
                "/me/businesses not available: {:?}",
                biz_json["error"]["message"]
            );
        }

        let mut result: Vec<PageInfo> = pages_map
            .into_values()
            .map(|page| PageInfo {
                id: page["id"].as_str().unwrap_or("").to_string(),
                name: page["name"].as_str().unwrap_or("").to_string(),
                access_token: page["access_token"].as_str().map(String::from),
                picture: page["picture"]["data"]["url"].as_str().map(String::from),
                username: page["username"].as_str().map(String::from),
            })
            .collect();

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    /// Facebook-specific user-facing error copy (v25 §2 row 9).
    ///
    /// Facebook does not flag duplicate submissions, so that arm never fires
    /// here — it stays in the match because `classify_error` shares one
    /// heuristic set across providers and future Graph edits may surface it.
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        // Page-scoped signals first: both only appear when the Page token lost
        // its Page scope, which is a different fix from a plain expired token.
        if body.contains("Invalid OAuth access token") {
            return Some(
                "Facebook Page token is no longer valid. Reconnect the Page on the \
                 Channels screen — Page tokens cannot be refreshed, they must be reissued."
                    .into(),
            );
        }
        if body.contains("Unsupported get request") || body.contains("Unsupported post request") {
            return Some(
                "Facebook rejected the request shape. Page posts need a Page-scoped token \
                 with publish_to_groups — personal profiles cannot post through this API."
                    .into(),
            );
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Facebook rate limit hit. Page publishing is throttled per Page — Forge retries \
                 with backoff, or lower the publish rate for this Page."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Facebook rejected the attached media. Photos and videos must come from a \
                 publicly fetchable URL and pass Facebook's copyright scan."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Facebook access token expired or was revoked. Page tokens last ~60 days and \
                 have no refresh endpoint — reconnect the Page to continue."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Facebook denied this request (HTTP 403). The actor usually lost Page access, \
                 the app is pending App Review, or the Page hit its posting limit."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Facebook Page or post not found. Re-pick the Page on the Channels screen if \
                 it was renamed, merged, or the integration points at a deleted Page."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Facebook rejected the request as invalid. Check the post text length and that \
                 the Page ID in the integration is still correct."
                    .into(),
            ),
            // classify_error returns Duplicate for a body mentioning "already
            // exists"; Facebook has no such rule, so keep the raw body.
            _ => None,
        }
    }

    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let (page_id, _) = self.resolve_page(access_token).await?;

        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(days as i64))
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        let until = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let json = self
            .get_page_insights(
                access_token,
                &page_id,
                "page_impressions,page_engaged_users,page_fans",
                "day",
                Some(&since),
                Some(&until),
            )
            .await?;

        Ok(super::parse_insights_data(&json))
    }

    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let url = format!("{}/{platform_post_id}/insights", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[
                (
                    "metric",
                    "post_impressions,post_engaged_users,post_reactions_by_type_total",
                ),
                ("period", "lifetime"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            if status == 429 {
                return Err(ProviderError::RateLimited("Facebook API rate limit".into()));
            } else if status == 401 {
                return Err(ProviderError::TokenExpired);
            } else {
                return Err(ProviderError::Api(
                    json["error"]["message"]
                        .as_str()
                        .unwrap_or("Facebook API error")
                        .to_string(),
                ));
            }
        }

        Ok(super::parse_insights_data(&json))
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        // Use _internal_id as the page_id if provided (from a connected page integration),
        // otherwise fall back to resolving from the token.
        let page_id = if !_internal_id.is_empty() {
            _internal_id.to_string()
        } else {
            self.resolve_page(access_token).await?.0
        };

        // Fetch page info for author details (name, handle, avatar)
        let page_info = self.fetch_page_info(access_token, &page_id).await.ok();
        let page_name = page_info.as_ref().map(|p| p.name.clone());
        let page_handle = page_info.as_ref().and_then(|p| p.username.clone());
        let page_avatar = page_info.as_ref().and_then(|p| p.picture.clone());

        let json = self
            .get_page_feed(access_token, &page_id, limit, None, None)
            .await?;

        let mut posts = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for item in data {
                let post_id = item["id"].as_str().unwrap_or("").to_string();
                let message = item["message"].as_str().unwrap_or("").to_string();
                let story = item["story"].as_str().map(|s| s.to_string());
                let content = if message.is_empty() {
                    story.unwrap_or_default()
                } else {
                    message
                };
                // Facebook always sends created_time; skip the item when absent
                // rather than stamping "now" on it.
                let Some(created_time) = item["created_time"]
                    .as_str()
                    .and_then(crate::social::common::parse_timestamp_opt)
                else {
                    continue;
                };

                // Build media vector — for video attachments, make a separate API call
                // to get the actual playable video URL instead of the thumbnail.
                let mut media = Vec::new();
                if let Some(arr) = item["attachments"]["data"].as_array() {
                    for a in arr {
                        let attach_type = a["type"].as_str().unwrap_or("");

                        // Handle album/carousel → extract individual items from subattachments
                        if attach_type == "album" || attach_type == "multiple" {
                            if let Some(children) = a["subattachments"]["data"].as_array() {
                                for child in children {
                                    if let Some(m) = Self::extract_single_media(child) {
                                        media.push(m);
                                    }
                                }
                            }
                            continue;
                        }

                        // Handle video: extract target.id and make a dedicated API call
                        // to get the actual playable video URL.
                        if attach_type.contains("video") || attach_type == "animated_video" {
                            if let Some(video_id) = a["target"]["id"].as_str() {
                                match self.get_video_source(access_token, video_id).await {
                                    Ok(url_str) => {
                                        media.push(MediaAttachment {
                                            url: url_str,
                                            mime_type: "video/mp4".into(),
                                            alt: None,
                                            poster_url: None,
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to fetch video source for {}: {e}",
                                            video_id
                                        );
                                        // Fallback: use extract_single_media which will try media.source
                                        if let Some(m) = Self::extract_single_media(a) {
                                            media.push(m);
                                        }
                                    }
                                }
                            } else {
                                // No target.id — fallback to extract_single_media
                                if let Some(m) = Self::extract_single_media(a) {
                                    media.push(m);
                                }
                            }
                        } else {
                            // Handle all other types (photo, share, etc.)
                            if let Some(m) = Self::extract_single_media(a) {
                                media.push(m);
                            }
                        }
                    }
                }

                let permalink = item["permalink_url"].as_str().map(String::from);

                posts.push(ExternalPostData {
                    platform_post_id: post_id,
                    text: content,
                    author_name: page_name.clone(),
                    author_handle: page_handle.clone(),
                    author_avatar: page_avatar.clone(),
                    created_at: created_time,
                    url: permalink,
                    media,
                    metadata: None,
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
        // get_page_post already requests comments.summary(true),reactions.summary(true)
        let json = self.get_page_post(access_token, platform_post_id).await?;
        Ok(Some(json))
    }

    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_post_comments_raw(access_token, platform_post_id)
            .await?;
        Ok(super::parse_graph_comments(&json))
    }

    /// Reply to one comment on a Page post (`POST /{comment-id}/comments`).
    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let json = self
            .post_page_comment_raw(access_token, comment_id, &post.content)
            .await?;
        let id = json["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(ProviderError::Api(format!("Facebook reply failed: {json}")));
        }
        Ok(PublishResult {
            platform_post_url: None,
            platform_post_id: id,
            status: "published".into(),
        })
    }

    /// TargetPicker reads the same `/me/accounts` listing as `pages()`.
    async fn targets(&self, access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        Ok(super::pages_to_targets(
            &self.pages(access_token).await?,
            "page",
        ))
    }

    /// Facebook exposes no member search to a page token, so @-autocomplete
    /// offers the Pages this token can tag.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let pages = self.pages(access_token).await?;
        let needle = query.trim().to_lowercase();
        Ok(pages
            .into_iter()
            .filter(|p| {
                needle.is_empty()
                    || p.name.to_lowercase().contains(&needle)
                    || p.username
                        .as_deref()
                        .is_some_and(|u| u.to_lowercase().contains(&needle))
            })
            .map(|p| MentionResult {
                id: p.id,
                label: p.username.unwrap_or(p.name),
                image: p.picture,
                do_not_cache: None,
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
            .get(format!(
                "{}/{page_id}?fields=id,name,username,picture.type(large)",
                self.graph_url()
            ))
            .query(&[("access_token", access_token)])
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(PageInfo {
            id: json["id"].as_str().unwrap_or("").to_string(),
            name: json["name"].as_str().unwrap_or("").to_string(),
            access_token: None,
            picture: json["picture"]["data"]["url"].as_str().map(String::from),
            username: json["username"].as_str().map(String::from),
        })
    }
    // `resolve_media_url` is NOT overridden here: the trait's default
    // makes a stored relative URL absolute, which is what every
    // provider needs. This file used to carry its own copy of that
    // override, and a provider that forgot it published with a relative
    // URL and failed.
}

impl FacebookProvider {
    /// Publish to a SPECIFIC page.
    ///
    /// `publish` resolves a page (the first the token can manage) and calls
    /// this; `post_pending` resolves the page the channel is connected to
    /// and calls the same body. One publish body, two resolutions.
    async fn publish_to_page(
        &self,
        access_token: &str,
        page_id: String,
        page_token: Option<String>,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let token = page_token.as_deref().unwrap_or(access_token);

        let params = vec![("message", post.content.as_str()), ("access_token", token)];

        let resp = self
            .http
            .post(format!("{}/{page_id}/feed", self.graph_url()))
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 200 {
            let post_id = json["id"].as_str().unwrap_or("").to_string();
            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: None,
                status: "published".into(),
            })
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Facebook publish failed");
            let raw = format!("Facebook publish failed (HTTP {status}): {msg}");
            Err(ProviderError::Api(friendly_error(
                self.map_error(msg, status.as_u16()),
                &raw,
            )))
        }
    }

    /// Resolve the page to act on from the stored token, plus the token the
    /// action should use when the listing returned a page-scoped one.
    ///
    /// TWO token shapes reach here, and the old code only handled one:
    ///
    /// - a **user token** (the root integration minted by `exchange_code`):
    ///   `/me/accounts` lists the Pages, each carrying its own `access_token`
    ///   — and a user token cannot post to a Page, so the page token is
    ///   returned alongside the id.
    /// - a **page token** (what `connect_page` stores on the connected
    ///   sub-account): `/me` IS the page, and `/me/accounts` has no `accounts`
    ///   edge on a Page node, so it either errors (#100) or answers with no
    ///   `data`. The old code reported the misleading "No pages found for
    ///   token. Ensure you have a Facebook Page." for every page token —
    ///   which is precisely the token shape a connected page channel holds,
    ///   so every Facebook publish failed. The fallback asks `/me` directly.
    ///
    /// `category` is requested next to `id` on that fallback because it exists
    /// on Page nodes but not on User nodes: a page token's `/me` answers with
    /// it, a user-token-with-no-pages errors with #100, and the caller still
    /// gets the actionable "connect a Page" message.
    async fn resolve_page(
        &self,
        access_token: &str,
    ) -> Result<(String, Option<String>), ProviderError> {
        self.resolve_page_for(access_token, None).await
    }

    /// Resolve the page to post to, preferring the one the channel is
    /// CONNECTED to.
    ///
    /// A token that administers several pages returns them all from
    /// `/me/accounts`, and the first in that list is arbitrary. When the
    /// scheduler hands us the channel's `internal_id` (the page the founder
    /// picked in the page picker) we publish to THAT one — otherwise a
    /// channel connected to page B quietly sends every post to page A.
    ///
    /// `internal_id = None` (the REST/CLI/MCP direct path, which has no
    /// channel identity) keeps the old behaviour: the first page listed.
    async fn resolve_page_for(
        &self,
        access_token: &str,
        internal_id: Option<&str>,
    ) -> Result<(String, Option<String>), ProviderError> {
        let me: serde_json::Value = self
            .http
            .get(format!(
                "{}/me/accounts?fields=id,access_token",
                self.graph_url()
            ))
            .query(&[("access_token", access_token)])
            .send()
            .await?
            .json()
            .await?;

        if let Some((id, page_token)) = first_page_from_accounts(&me) {
            // A connected channel names its page. Prefer it over the first
            // row of the list, which is arbitrary.
            if let Some(want) = internal_id.filter(|w| !w.is_empty()) {
                if let Some((id, page_token)) = find_page_by_id(&me, want) {
                    return Ok((id, page_token));
                }
                tracing::warn!(
                    "Facebook channel {want} is no longer in /me/accounts — falling back to the                      first page the token can manage"
                );
            }
            return Ok((id, page_token));
        } else {
            tracing::debug!(
                "Facebook /me/accounts failed or listed no page, trying /me: {:?}",
                me["error"]["message"]
            );
        }

        // Page-token path: /me returns the Page itself.
        let page: serde_json::Value = self
            .http
            .get(format!("{}/me?fields=id,category", self.graph_url()))
            .query(&[("access_token", access_token)])
            .send()
            .await?
            .json()
            .await?;

        if page["error"].is_null() {
            let page_id = page["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if let Some(id) = page_id {
                // The stored token is a PAGE token (that is what
                // `connect_page` writes), so `/me` IS the connected page.
                // If the caller named a DIFFERENT page, the stored token
                // cannot publish to it — say so rather than publishing to
                // the wrong audience.
                if let Some(want) = internal_id.filter(|w| !w.is_empty()) {
                    if want != id.as_str() {
                        return Err(ProviderError::Auth(format!(
                            "This channel is connected to page {want}, but its stored token is a                              token for page {id}. Reconnect the channel from the page picker."
                        )));
                    }
                }
                return Ok((id, None));
            }
        }

        Err(ProviderError::Auth(
            "No pages found for token. Ensure you have a Facebook Page and the token was granted \
             pages_show_list + pages_manage_posts when the channel was connected."
                .into(),
        ))
    }
}

/// The entry for `want` in a `/me/accounts` response, if it is there.
fn find_page_by_id(me: &serde_json::Value, want: &str) -> Option<(String, Option<String>)> {
    me["data"]
        .as_array()?
        .iter()
        .find(|p| p["id"].as_str() == Some(want))
        .map(|p| {
            let id = want.to_string();
            let token = p["access_token"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            (id, token)
        })
}

/// Interpret a `/me/accounts` response: `Some((page_id, page_token))` when it
/// lists at least one page, `None` when it errored (a Page token has no
/// `accounts` edge — #100) or listed none. The `None` case is what drives the
/// `/me` fallback in `resolve_page`; the old code turned the #100 error into
/// the misleading "No pages found for token" on every connected page channel.
fn first_page_from_accounts(me: &serde_json::Value) -> Option<(String, Option<String>)> {
    if !me["error"].is_null() {
        return None;
    }
    let page = me["data"]
        .as_array()?
        .iter()
        .find(|p| p["id"].as_str().is_some_and(|s| !s.is_empty()))?;
    Some((
        page["id"].as_str().unwrap_or("").to_string(),
        page["access_token"].as_str().map(String::from),
    ))
}

// ── Inherent Graph API Methods ──────────────────────────────────
impl FacebookProvider {
    /// Get the page's feed (posts).
    pub async fn get_page_feed(
        &self,
        access_token: &str,
        page_id: &str,
        limit: u32,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = limit.min(100);
        let mut url = format!(
            "{}/{page_id}/feed?fields=message,created_time,story,permalink_url,attachments{{media{{source,image{{src}}}},type,url,title,target{{id}},subattachments{{media{{source,image{{src}}}},type,url,title,target{{id}}}}}}&limit={limit}",
            self.graph_url()
        );
        if let Some(s) = since {
            url.push_str(&format!("&since={s}"));
        }
        if let Some(u) = until {
            url.push_str(&format!("&until={u}"));
        }
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get a single post by ID.
    pub async fn get_page_post(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "{}/{post_id}?fields=id,message,created_time,permalink_url,comments.summary(true),reactions.summary(true)",
            self.graph_url()
        );
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Post a reply under an existing comment (raw Graph API response).
    pub async fn post_page_comment_raw(
        &self,
        access_token: &str,
        comment_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{comment_id}/comments", self.graph_url());
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&[("message", message)])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get comments on a post (raw Graph API response).
    pub async fn get_post_comments_raw(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "{}/{post_id}/comments?fields=id,message,from,created_time",
            self.graph_url()
        );
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Create a text/link post on a page.
    pub async fn create_post(
        &self,
        access_token: &str,
        page_id: &str,
        message: &str,
        link: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut params: Vec<(&str, &str)> =
            vec![("message", message), ("access_token", access_token)];
        if let Some(l) = link {
            params.push(("link", l));
        }
        let resp = self
            .http
            .post(format!("{}/{page_id}/feed", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Create a photo post on a page.
    pub async fn create_photo_post(
        &self,
        access_token: &str,
        page_id: &str,
        url: &str,
        caption: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let params: Vec<(&str, &str)> = vec![
            ("url", url),
            ("caption", caption),
            ("access_token", access_token),
        ];
        let resp = self
            .http
            .post(format!("{}/{page_id}/photos", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Create a video post on a page.
    pub async fn create_video_post(
        &self,
        access_token: &str,
        page_id: &str,
        file_url: &str,
        title: &str,
        description: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let params: Vec<(&str, &str)> = vec![
            ("file_url", file_url),
            ("title", title),
            ("description", description),
            ("access_token", access_token),
        ];
        let resp = self
            .http
            .post(format!("{}/{page_id}/videos", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Delete a post by ID.
    pub async fn delete_post(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{post_id}", self.graph_url());
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[("access_token", access_token)])
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Comment on a post.
    pub async fn comment_on_post(
        &self,
        access_token: &str,
        post_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let params: Vec<(&str, &str)> = vec![("message", message), ("access_token", access_token)];
        let resp = self
            .http
            .post(format!("{}/{post_id}/comments", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }
    /// Get the actual playable video source URL for a video ID.
    /// Facebook's feed endpoint `media.source` often returns a thumbnail/preview.
    /// This makes a dedicated call to `/{video_id}?fields=source,embeddable,format`
    /// which returns the actual playable mp4 URL.
    pub async fn get_video_source(
        &self,
        access_token: &str,
        video_id: &str,
    ) -> Result<String, ProviderError> {
        let url = format!(
            "{}/{video_id}?fields=source,embeddable,format",
            self.graph_url()
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            // Try direct `source` field first
            if let Some(source) = json["source"].as_str() {
                if !source.is_empty() {
                    return Ok(source.to_string());
                }
            }
            // Fallback: pick the highest-quality format from the `format` array
            if let Some(formats) = json["format"].as_array() {
                if let Some(best) = formats
                    .iter()
                    .filter(|f| {
                        f["filetype"].as_str() == Some("mp4")
                            && f["embeddable"].as_bool() == Some(true)
                    })
                    .max_by_key(|f| f["quality"].as_str().unwrap_or(""))
                {
                    if let Some(url_str) = best["embed_url"]
                        .as_str()
                        .or_else(|| best["preview_url"].as_str())
                    {
                        return Ok(url_str.to_string());
                    }
                }
                // If no embeddable mp4, try any mp4
                if let Some(best) = formats
                    .iter()
                    .filter(|f| f["filetype"].as_str() == Some("mp4"))
                    .max_by_key(|f| f["quality"].as_str().unwrap_or(""))
                {
                    if let Some(url_str) = best["embed_url"]
                        .as_str()
                        .or_else(|| best["preview_url"].as_str())
                    {
                        return Ok(url_str.to_string());
                    }
                }
            }
            Err(ProviderError::Api(
                "No usable video source found in response".into(),
            ))
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// React to a post (LIKE, LOVE, WOW, HAHA, SAD, ANGRY).
    pub async fn react_to_post(
        &self,
        access_token: &str,
        post_id: &str,
        reaction_type: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let params: Vec<(&str, &str)> =
            vec![("type", reaction_type), ("access_token", access_token)];
        let resp = self
            .http
            .post(format!("{}/{post_id}/reactions", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get page insights for a given metric and period.
    pub async fn get_page_insights(
        &self,
        access_token: &str,
        page_id: &str,
        metric: &str,
        period: &str,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut url = format!(
            "{}/{page_id}/insights?metric={metric}&period={period}",
            self.graph_url()
        );
        if let Some(s) = since {
            url.push_str(&format!("&since={s}"));
        }
        if let Some(u) = until {
            url.push_str(&format!("&until={u}"));
        }
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get page conversations (inbox).
    pub async fn get_page_conversations(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "{}/{page_id}/conversations?fields=id,snippet,updated_time,participants,message_count",
            self.graph_url()
        );
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get messages in a conversation.
    pub async fn get_conversation_messages(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "{}/{conversation_id}/messages?fields=id,message,from,created_time",
            self.graph_url()
        );
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Send a message in a conversation.
    pub async fn send_message(
        &self,
        access_token: &str,
        conversation_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let params: Vec<(&str, &str)> = vec![("message", message), ("access_token", access_token)];
        let resp = self
            .http
            .post(format!("{}/{conversation_id}/messages", self.graph_url()))
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Extract a single media attachment from a raw attachment JSON object.
    /// Checks `media.source` first (which returns the actual video/photo URL),
    /// falls back to `media.image.src` for thumbnails, then `url`.
    /// Determines mime type from URL extension when possible.
    fn extract_single_media(a: &serde_json::Value) -> Option<MediaAttachment> {
        let attach_type = a["type"].as_str().unwrap_or("");
        let is_video = attach_type.contains("video") || attach_type == "animated_video";

        // Check `media.source` first: this is the actual source URL for both videos and images.
        // Facebook returns `media.source` for video attachments and some image types.
        if let Some(url) = a["media"]["source"].as_str() {
            let mime = if is_video
                || url.contains(".mp4")
                || url.contains(".webm")
                || url.contains(".mov")
            {
                "video/mp4"
            } else if url.contains(".gif") {
                "image/gif"
            } else {
                "image/jpeg"
            };
            return Some(MediaAttachment {
                url: url.to_string(),
                mime_type: mime.into(),
                alt: None,
                poster_url: None,
            });
        }

        // Fallback to `media.image.src` (thumbnail / photo URL)
        if let Some(url) = a["media"]["image"]["src"].as_str() {
            return Some(MediaAttachment {
                url: url.to_string(),
                mime_type: "image/jpeg".into(),
                alt: None,
                poster_url: None,
            });
        }

        // Fallback: use the generic attachment URL
        if let Some(url) = a["url"].as_str() {
            let mime = if url.contains(".mp4") || url.contains(".webm") || url.contains(".mov") {
                "video/mp4"
            } else {
                "image/jpeg"
            };
            return Some(MediaAttachment {
                url: url.to_string(),
                mime_type: mime.into(),
                alt: None,
                poster_url: None,
            });
        }

        None
    }

    /// Search for Facebook pages by query string.
    pub async fn search_pages(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/search", self.graph_url()))
            .query(&[("q", query), ("type", "page")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }

    /// Get albums for a page.
    pub async fn get_page_albums(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "{}/{page_id}/albums?fields=id,name,count,cover_photo,created_time",
            self.graph_url()
        );
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
        } else if status == 429 {
            Err(ProviderError::RateLimited("Facebook API rate limit".into()))
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Facebook API error")
                    .to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    fn provider() -> FacebookProvider {
        FacebookProvider::new(&test_config())
    }

    fn post_with_images(n: usize) -> PostContent {
        PostContent {
            content: "hello".into(),
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

    #[test]
    fn test_scopes_contain_required() {
        let provider = FacebookProvider::new(&test_config());
        let scopes = provider.scopes();
        assert!(scopes.contains(&"pages_show_list".to_string()));
        assert!(scopes.contains(&"pages_manage_posts".to_string()));
        assert!(scopes.contains(&"public_profile".to_string()));
    }

    /// A page token's `/me/accounts` answers #100 "nonexisting field
    /// (accounts)" — that must NOT be read as "user has no pages"; it drives
    /// the /me fallback instead. Regression test for the publish bug where
    /// every connected Facebook page failed with "No pages found for token".
    #[test]
    fn accounts_error_100_is_not_a_page_listing() {
        let err = serde_json::json!({
            "error": { "code": 100, "message": "Tried accessing nonexisting field (accounts)" }
        });
        assert!(first_page_from_accounts(&err).is_none());
    }

    #[test]
    fn accounts_data_yields_first_page_with_its_token() {
        let ok = serde_json::json!({
            "data": [
                { "id": "111", "access_token": "PAGE_TOKEN_A" },
                { "id": "222", "access_token": "PAGE_TOKEN_B" }
            ]
        });
        let (id, tok) = first_page_from_accounts(&ok).expect("a listed page");
        assert_eq!(id, "111");
        assert_eq!(tok.as_deref(), Some("PAGE_TOKEN_A"));
    }

    #[test]
    fn accounts_empty_data_is_not_a_page_listing() {
        let empty = serde_json::json!({ "data": [] });
        assert!(first_page_from_accounts(&empty).is_none());
    }

    #[test]
    fn test_identifier_and_name() {
        let provider = FacebookProvider::new(&test_config());
        assert_eq!(provider.identifier(), "facebook");
        assert_eq!(provider.name(), "Facebook");
    }

    #[test]
    fn test_max_content_length() {
        let provider = FacebookProvider::new(&test_config());
        assert_eq!(provider.max_content_length(), 63206);
    }

    #[tokio::test]
    async fn test_generate_auth_url_contains_params() {
        let provider = FacebookProvider::new(&test_config());
        let result = provider
            .generate_auth_url(
                "test_state",
                "test_verifier",
                "http://localhost:3000/callback",
            )
            .await;
        let url = result.unwrap().url;

        assert!(url.contains("client_id="), "should contain client_id");
        assert!(url.contains("redirect_uri="), "should contain redirect_uri");
        assert!(url.contains("state=test_state"), "should contain state");
        assert!(url.contains("scope="), "should contain scope");
        assert!(
            url.contains("response_type=code"),
            "should contain response_type"
        );
        assert!(url.starts_with(&format!(
            "https://www.facebook.com/{GRAPH_VERSION}/dialog/oauth"
        )));
    }

    #[test]
    fn publish_should_reject_11th_image() {
        let provider = FacebookProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(11)).is_err());
    }

    #[test]
    fn publish_should_accept_10_images() {
        let provider = FacebookProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(10)).is_ok());
    }

    #[test]
    fn parse_should_expand_page_insights_into_daily_series() {
        let raw = serde_json::json!({
            "data": [{
                "name": "page_impressions",
                "period": "day",
                "values": [
                    { "value": 100, "end_time": "2026-09-28T00:00:00+0000" },
                    { "value": 150, "end_time": "2026-09-29T00:00:00+0000" }
                ]
            }]
        });
        let series = super::super::parse_insights_data(&raw);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].label, "page_impressions");
        assert_eq!(series[0].data.len(), 2);
        assert_eq!(series[0].data[0].total, "100");
    }

    #[test]
    fn parse_should_map_post_reactions_and_shares() {
        let raw = serde_json::json!({
            "reactions": { "summary": { "total_count": 42 } },
            "comments": { "summary": { "total_count": 12 } },
            "shares": { "count": 5 }
        });
        let e = super::super::parse_engagement_data("facebook", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
        assert_eq!(e.shares, 5);
    }

    // ── B4: map_error fixtures ──────────────────────────────

    #[test]
    fn map_error_keeps_invalid_oauth_token_copy() {
        assert_eq!(
            provider()
                .map_error("Invalid OAuth access token.", 400)
                .unwrap(),
            "Facebook Page token is no longer valid. Reconnect the Page on the Channels \
             screen — Page tokens cannot be refreshed, they must be reissued."
        );
    }

    #[test]
    fn map_error_explains_unsupported_post_request() {
        let msg = provider()
            .map_error("Unsupported post request.", 400)
            .unwrap();
        assert!(msg.contains("Page-scoped token"), "got: {msg}");
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
        assert!(msg.contains("media"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let msg = provider().map_error("", 401).unwrap();
        assert!(msg.contains("no refresh endpoint"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_forbidden_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("App Review"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_page_on_404() {
        let msg = provider().map_error("", 404).unwrap();
        assert!(msg.contains("Channels screen"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let msg = provider().map_error("", 400).unwrap();
        assert!(msg.contains("invalid"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 503).is_none());
    }

    #[test]
    fn facebook_does_not_request_cron_refresh() {
        // Page tokens have no refresh endpoint — refresh_token() errors by
        // design, so the scheduler must never try to refresh them.
        assert!(!provider().needs_cron_refresh());
    }

    // ── C3: long-lived-token refresh wiring ────────────────────

    #[tokio::test]
    async fn refresh_with_no_stored_grant_is_an_honest_reconnect_error() {
        let err = provider().refresh_token("").await.unwrap_err();
        match err {
            ProviderError::Auth(msg) => {
                assert!(
                    msg.contains("reconnect the channel once"),
                    "must tell the operator what to do, got: {msg}"
                );
            }
            other => panic!("must be Auth, got {other:?}"),
        }
    }

    // ── B3: comment read path wiring ────────────────────────

    #[test]
    fn parse_should_emit_no_facebook_comments_for_error_body() {
        let raw = serde_json::json!({ "error": { "message": "Unsupported get request." } });
        assert!(super::parse_graph_comments(&raw).is_empty());
    }
}
