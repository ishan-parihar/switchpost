// ─── Instagram Provider ───────────────────────────────────────
// OAuth 2.0 + Instagram Graph API (for Business/Creator accounts).
// Posts to Instagram Business accounts via Facebook's Graph API.

use std::time::Duration;

use async_trait::async_trait;

use super::common::extract_access_token;
use super::*;
use crate::config::Config;

/// How long to wait for a media container to become publishable.
const CONTAINER_READY_ATTEMPTS: usize = 12;
const CONTAINER_READY_INTERVAL: Duration = Duration::from_secs(5);

pub struct InstagramProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl InstagramProvider {
    /// Publish to a SPECIFIC Instagram business account.
    ///
    /// `publish` resolves one (the first page that carries an IG account)
    /// and calls this; `post_pending` resolves the one the CHANNEL is
    /// connected to and calls the same body. One publish body, two
    /// resolutions — and the channel one is the right one.
    async fn publish_to_ig(
        &self,
        access_token: &str,
        ig_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        if post.media.len() == 1 {
            // Single image: simpler IMAGE container
            //
            // A REEL with a music track is different: Instagram needs
            // `media_type=REELS`, the VIDEO (not the image) as the
            // source, and an `audio_configuration` naming the track.
            //
            // The composer collected the music - the MusicPicker is
            // shown for Instagram and the pick is written to
            // `settings.audio_id` - and it was DISCARDED: neither
            // instagram.rs nor instagram_standalone.rs read it, so the
            // reel went out with no music while the composer showed a
            // "Change/Browse" control that implied it had been applied.
            let audio_id = post
                .settings
                .get("audio_id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let is_video = post.media[0].mime_type.starts_with("video/");

            let container_id = if let Some(audio_id) = audio_id {
                if !is_video {
                    return Err(ProviderError::InvalidRequest(
                        "Instagram music can only be attached to a video (Reel), not to an image."
                            .into(),
                    ));
                }
                self.create_reel_container(
                    &ig_id,
                    access_token,
                    &post.media[0].url,
                    &post.content,
                    audio_id,
                )
                .await?
            } else {
                self.create_single_media_container(
                    &ig_id,
                    access_token,
                    &post.media[0].url,
                    &post.content,
                    false,
                )
                .await?
            };

            self.await_container_ready(access_token, &container_id)
                .await?;
            let platform_post_id = self
                .publish_container(&ig_id, access_token, &container_id)
                .await?;
            Ok(PublishResult {
                platform_post_url: Some(format!("https://instagram.com/p/{platform_post_id}")),
                platform_post_id,
                status: "published".into(),
            })
        } else {
            // Multiple images: CAROUSEL flow
            let mut children_ids = Vec::with_capacity(post.media.len());
            for media in &post.media {
                let child_id = self
                    .create_single_media_container(&ig_id, access_token, &media.url, "", true)
                    .await?;
                children_ids.push(child_id);
            }

            // Create CAROUSEL container with children
            let children_str = children_ids.join(",");
            let carousel_resp = self
                .http
                .post(format!("{}/{}/media", self.graph_url(), ig_id))
                .form(&[
                    ("media_type", "CAROUSEL"),
                    ("children", &children_str),
                    ("caption", &post.content),
                    ("access_token", access_token),
                ])
                .send()
                .await?;

            let carousel_status = carousel_resp.status();
            let carousel_json: serde_json::Value = carousel_resp.json().await?;

            if let Some(err) = carousel_json["error"].as_object() {
                let msg = err["message"]
                    .as_str()
                    .unwrap_or("Carousel creation failed");
                let code = err["code"].as_u64().unwrap_or(0);
                let raw = format!("Instagram carousel creation failed (code {code}): {msg}");
                return Err(ProviderError::Api(friendly_error(
                    self.map_error(msg, carousel_status.as_u16()),
                    &raw,
                )));
            }

            let carousel_id = carousel_json["id"]
                .as_str()
                .ok_or_else(|| {
                    ProviderError::Api(format!(
                        "Instagram did not return carousel container ID: {carousel_json:?}"
                    ))
                })?
                .to_string();

            // Polling the CAROUSEL container also covers the children: Instagram
            // only marks a carousel FINISHED once every child has finished its
            // own processing, so one wait replaces N per-child waits.
            self.await_container_ready(access_token, &carousel_id)
                .await?;
            let platform_post_id = self
                .publish_container(&ig_id, access_token, &carousel_id)
                .await?;
            Ok(PublishResult {
                platform_post_url: Some(format!("https://instagram.com/p/{platform_post_id}")),
                platform_post_id,
                status: "published".into(),
            })
        }
    }

    /// Same container → ready → `media_publish` path as [`Self::publish`], but
    /// honest about what `media_publish` returns.
    ///
    /// `media_publish` hands back a media id as soon as the container is
    /// accepted — for a video reel that id comes back with
    /// `status_code: PUBLISHED`, meaning "live, still transcoding". Treating
    /// that as published leaves the user with a post whose video does not
    /// play, so the extra status read routes it through the pending
    /// lifecycle instead. Images are `FINISHED` on the first read, which is
    /// the same published result `publish` returns today.
    pub fn new(config: &Config) -> Self {
        // Instagram Graph API uses Facebook Login OAuth — needs Facebook App credentials
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

#[async_trait]
impl SocialProvider for InstagramProvider {
    fn identifier(&self) -> &'static str {
        "instagram"
    }

    fn name(&self) -> &'static str {
        "Instagram"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "instagram_basic".into(),
            "instagram_content_publish".into(),
            "instagram_manage_comments".into(),
            "instagram_manage_insights".into(),
            "instagram_manage_messages".into(),
            "pages_show_list".into(),
            "pages_read_engagement".into(),
            "business_management".into(),
            "pages_manage_engagement".into(),
            "pages_manage_metadata".into(),
            "pages_read_user_content".into(),
            "read_insights".into(),
            "pages_messaging".into(),
        ]
    }

    fn is_between_steps(&self) -> bool {
        true
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
        // Exchange code for token
        let token_params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("code", code),
        ];

        let resp = self
            .http
            .get(format!(
                "https://graph.facebook.com/{GRAPH_VERSION}/oauth/access_token"
            ))
            .query(&token_params)
            .send()
            .await?;

        let (short_token, _json) = extract_access_token(resp, "instagram")
            .await
            .map_err(ProviderError::Auth)?;

        // Get long-lived token
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

        // Get Facebook user info for display
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

        let fb_id = me["id"].as_str().unwrap_or("").to_string();
        let fb_name = me["name"].as_str().unwrap_or("Instagram").to_string();
        let fb_picture = me["picture"]["data"]["url"].as_str().map(String::from);

        // Return user-level token so pages() + page-picker can discover IG accounts
        Ok(AuthToken {
            // C3: a long-lived FB user token is its own refresh grant
            // (fb_exchange_token) — store it so refresh_token can extend it.
            // Without this, the row's expires_at ticks to day 60 and
            // resolve_token then had only an Err("reconnect") to offer.
            refresh_token: Some(access_token.clone()),
            access_token, // user-level token for page discovery
            expires_in,
            provider_user_id: fb_id,
            name: fb_name,
            username: String::new(),
            picture: fb_picture,
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        // C3: business-IG tokens are long-lived Facebook user tokens —
        // extendable via fb_exchange_token (the same flow the Facebook
        // provider uses). The stored refresh token IS the long-lived token
        // (see exchange_code).
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

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        if post.media.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "Instagram Feed posts require at least one media attachment (image or video)."
                    .into(),
            ));
        }

        let ig_id = self
            .resolve_ig_business_account(access_token)
            .await
            .map_err(|e| ProviderError::Api(format!("Cannot resolve IG business account: {e}")))?;

        self.publish_to_ig(access_token, &ig_id, post).await
    }

    /// Delete a published Instagram media item.
    ///
    /// The Graph API supports it: `DELETE /{media-id}` with the token that
    /// administers the account. The provider had no implementation at all,
    /// so `delete_published` fell through to the trait's default — "This
    /// provider does not support deleting a published post" — and the
    /// founder could publish a post but not remove it.
    async fn delete_published(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<(), ProviderError> {
        if platform_post_id.trim().is_empty() {
            return Err(ProviderError::Api(
                "post has no platform_post_id — nothing to delete on Instagram".into(),
            ));
        }
        let resp = self
            .http
            .delete(format!("{}/{platform_post_id}", self.graph_url()))
            .query(&[("access_token", access_token)])
            .send()
            .await?;
        let http_status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if http_status.is_success() {
            return Ok(());
        }
        // The Graph API refuses the delete with this app's tokens — both
        // the PAGE token and the parent USER token answer "(#10)
        // Insufficient permissions to access this data" or "Unsupported
        // delete request" (measured against every delete shape on
        // 2026-11-13: the media id, /{ig_user}/media?media_id=…, and
        // /{page}/media?media_id=…). Meta does not grant media deletion
        // to these tokens.
        //
        // So say the TRUE thing: the post is live and the app cannot take
        // it down. The generic mapper blamed the CAPTION, which sent the
        // founder looking at the wrong thing entirely.
        if body.contains("Insufficient permissions") || body.contains("Unsupported delete request")
        {
            return Err(ProviderError::Api(format!(
                "Instagram does not let this app delete a published post (the Graph API refuses \
                 it for these tokens: {body}). The post is LIVE — delete it from the Instagram \
                 app (or the Meta business suite). Nothing is wrong with the post itself."
            )));
        }
        Err(self.provider_error(&body, http_status.as_u16()))
    }

    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let _ = internal_id;
        let published = self.publish(access_token, post).await?;
        self.media_publish_state(access_token, &published.platform_post_id)
            .await
    }

    /// Read a published media item's `status_code` and map it onto the
    /// publish lifecycle.
    async fn check_post_status(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        self.media_publish_state(access_token, platform_post_id)
            .await
    }

    /// List Instagram Business accounts from the user's Facebook pages.
    ///
    /// Discovers FB pages via two phases:
    ///   Phase 1 - `/me/accounts` — pages the user selected in the OAuth dialog
    ///   Phase 2 - `/me/businesses` → `owned_pages` + `client_pages` — pages from Business Manager
    ///
    /// Filters to only pages with a linked Instagram Business account (either via the
    /// `instagram_business_account` field or the `/{page_id}/instagram_business_account`
    /// endpoint) and resolves IG account details.
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let fields = "id,instagram_business_account,access_token,username,name,picture.type(large)";
        let mut pages_map: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        // ── Helper: paginated fetch ──────────────────────────────────────────
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
                    // Non-fatal: skip this endpoint (e.g. /me/businesses may not be available)
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

        // ── Phase 1: /me/accounts ────────────────────────────────────────────
        let me_url = format!(
            "{}/me/accounts?fields={}&limit=100",
            self.graph_url(),
            fields
        );
        collect_pages(&self.http, &me_url, access_token, &mut pages_map).await?;

        // ── Phase 2: /me/businesses → owned_pages + client_pages ─────────────
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

        // ── Resolve IG Business accounts from collected pages ─────────────────
        let mut result = Vec::new();

        for page in pages_map.values() {
            let page_token = page["access_token"].as_str().unwrap_or(access_token);

            // Try to get the IG Business account ID (prefer the inline field)
            let ig_id = page["instagram_business_account"]["id"]
                .as_str()
                .map(String::from);

            let ig_id = if let Some(id) = ig_id {
                Some(id)
            } else {
                // Fallback: call /{page_id}/instagram_business_account
                let ig_resp = self
                    .http
                    .get(format!(
                        "{}/{}/instagram_business_account",
                        self.graph_url(),
                        page["id"].as_str().unwrap_or("")
                    ))
                    .query(&[("access_token", page_token)])
                    .send()
                    .await?;

                let ig: serde_json::Value = ig_resp.json().await?;
                if ig["error"].is_null() {
                    ig["id"].as_str().map(String::from)
                } else {
                    continue; // No IG account for this page
                }
            };

            if let Some(ig_id) = ig_id {
                // Resolve IG account details
                let ig_resp = self
                    .http
                    .get(format!(
                        "{}/{}?fields=id,username,name,profile_picture_url",
                        self.graph_url(),
                        ig_id
                    ))
                    .query(&[("access_token", page_token)])
                    .send()
                    .await?;

                let ig_json: serde_json::Value = ig_resp.json().await?;

                if let Some(e) = ig_json["error"].as_object() {
                    tracing::warn!(
                        "IG detail fetch for {ig_id}: {}",
                        e["message"].as_str().unwrap_or("")
                    );
                    continue;
                }

                let name = ig_json["username"]
                    .as_str()
                    .or_else(|| ig_json["name"].as_str())
                    .unwrap_or("")
                    .to_string();

                let profile_pic = ig_json["profile_picture_url"].as_str().map(String::from);

                result.push(PageInfo {
                    id: ig_id,
                    name,
                    access_token: Some(page_token.to_string()),
                    picture: profile_pic,
                    username: None,
                });
            }
        }

        Ok(result)
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let ig_id = match self.resolve_ig_business_account(access_token).await {
            Ok(id) => id,
            Err(e) => {
                // If we have an internal_id (page-scoped), use that directly
                if _internal_id.is_empty() || _internal_id == access_token {
                    return Err(ProviderError::Api(format!(
                        "Cannot resolve IG business account: {e}"
                    )));
                }
                // The internal_id might already be the IG business account ID
                // Try fetching IG media directly with it
                _internal_id.to_string()
            }
        };

        // First try with the resolved ID; if that fails, try _internal_id as fallback
        let mut effective_id = ig_id.as_str();
        let result = self.get_ig_media(access_token, effective_id, limit).await;
        let json = match result {
            Ok(j) => j,
            Err(_) => {
                effective_id = _internal_id;
                self.get_ig_media(access_token, effective_id, limit).await?
            }
        };

        // Fetch IG account profile for author info using the same ID that worked for media
        let (account_name, account_handle, account_avatar) = self
            .get_ig_account_info(access_token, effective_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to fetch IG account info for {effective_id}: {e}");
                (None, None, None)
            });

        // Also try extracting username from individual media items as fallback
        let media_fallback_username = json["data"][0]["username"].as_str().map(String::from);

        // If account-level info failed but media items carry a username, use that
        let account_handle = account_handle.or_else(|| media_fallback_username.clone());
        let account_name = account_name.or_else(|| media_fallback_username.clone());

        tracing::info!(
            "IG author info for {}: name={:?} handle={:?} avatar={} (fallback_username={:?})",
            effective_id,
            account_name,
            account_handle,
            account_avatar.is_some(),
            media_fallback_username,
        );

        let mut posts = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for item in data {
                let media_id = item["id"].as_str().unwrap_or("").to_string();
                let caption = item["caption"].as_str().unwrap_or("").to_string();
                // Skip undated items rather than stamp "now" on them.
                let Some(created_at) = item["timestamp"]
                    .as_str()
                    .and_then(crate::social::common::parse_timestamp_opt)
                else {
                    continue;
                };
                let permalink = item["permalink"].as_str().map(String::from);
                let media_url = item["media_url"].as_str().map(String::from);

                // Detect media type from API response
                let media_type = item["media_type"].as_str().unwrap_or("IMAGE");

                // Use per-item username as fallback for account-level info
                let item_username = item["username"].as_str().map(String::from);
                let post_author_name = account_name.clone().or_else(|| item_username.clone());
                let post_author_handle = account_handle.clone().or_else(|| item_username.clone());
                let post_author_avatar = account_avatar.clone();

                let media = match media_type {
                    "CAROUSEL_ALBUM" => {
                        // CAROUSEL_ALBUM: iterate children for all album items
                        let mut items = Vec::new();
                        if let Some(children) = item["children"]["data"].as_array() {
                            for child in children {
                                let child_url = child["media_url"].as_str().map(String::from);
                                let child_type = child["media_type"].as_str().unwrap_or("IMAGE");
                                let child_mime = match child_type {
                                    "VIDEO" => "video/mp4",
                                    _ => "image/jpeg",
                                };
                                if let Some(url) = child_url {
                                    items.push(MediaAttachment {
                                        url,
                                        mime_type: child_mime.into(),
                                        alt: None,
                                        poster_url: None,
                                    });
                                }
                            }
                        }
                        items
                    }
                    _ => {
                        // IMAGE or VIDEO: single media
                        let mime_type = match media_type {
                            "VIDEO" => "video/mp4",
                            _ => "image/jpeg",
                        };
                        media_url
                            .map(|u| {
                                vec![MediaAttachment {
                                    url: u,
                                    mime_type: mime_type.into(),
                                    alt: None,
                                    poster_url: None,
                                }]
                            })
                            .unwrap_or_default()
                    }
                };

                posts.push(ExternalPostData {
                    platform_post_id: media_id,
                    text: caption,
                    author_name: post_author_name,
                    author_handle: post_author_handle,
                    author_avatar: post_author_avatar,
                    created_at,
                    url: permalink,
                    media,
                    metadata: Some(item.clone()),
                });
            }
        }
        Ok(posts)
    }

    /// Instagram-specific user-facing error copy (v25 §2 row 9).
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Instagram rate limit hit. The Graph API throttles content publishing per \
                 Business account — Forge retries with backoff automatically."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Instagram rejected the media. Carousels accept 2-10 images (or one video) \
                 in JPEG/PNG, each under 8 MB and 1920px on the long edge, fetched from a \
                 publicly reachable URL."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Instagram access token expired or was revoked. Facebook-login tokens last \
                 ~60 days and cannot be refreshed — reconnect the account."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Instagram denied this request (HTTP 403). The account is probably not a \
                 Business or Creator account, is not linked to a Facebook Page, or the app \
                 is still pending App Review."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Instagram media or account not found. Reconnect the account — it may have \
                 been switched to a personal (non-Business) profile."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Instagram rejected the request as invalid. Check the caption length (2200 \
                 chars, 2200 more for the first comment) and that the media type matches \
                 the container flow."
                    .into(),
            ),
            _ => None,
        }
    }

    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        let detail = self
            .get_ig_media_detail(access_token, platform_post_id)
            .await?;
        // If the response has a top-level error or no id, treat as not found
        if detail.get("error").is_some() || detail.get("id").is_none() {
            return Ok(None);
        }
        Ok(Some(detail))
    }

    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_ig_media_comments(access_token, platform_post_id)
            .await?;
        Ok(super::parse_graph_comments(&json))
    }

    /// Instagram has no member-search endpoint; accounts that mention the
    /// business are the only taggable set the Graph API exposes.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let ig_id = self
            .resolve_ig_business_account(access_token)
            .await
            .map_err(|e| {
                ProviderError::Api(format!("Failed to resolve IG business account: {e}"))
            })?;
        let json = self.get_mentions(access_token, &ig_id).await?;
        Ok(super::parse_graph_mentions(&json, query))
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "{}/{page_id}?fields=id,username,name,profile_picture_url",
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
            picture: json["profile_picture_url"].as_str().map(String::from),
            username: json["username"].as_str().map(String::from),
        })
    }

    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let ig_id = self
            .resolve_ig_business_account(access_token)
            .await
            .map_err(|e| {
                ProviderError::Api(format!("Failed to resolve IG business account: {e}"))
            })?;

        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(days as i64))
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        let until = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let url = format!("{}/{ig_id}/insights", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("metric", "impressions,reach,profile_views,follower_count"),
                ("period", "day"),
                ("since", since.as_str()),
                ("until", until.as_str()),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
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
                ("metric", "engagement,impressions,reach,saved"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        Ok(super::parse_insights_data(&json))
    }
    // `resolve_media_url` is NOT overridden here: the trait's default
    // makes a stored relative URL absolute, which is what every
    // provider needs. This file used to carry its own copy of that
    // override, and a provider that forgot it published with a relative
    // URL and failed.

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let url = format!("{}/{comment_id}/replies", self.graph_url());
        let body = serde_json::json!({
            "message": post.content,
        });
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            let comment_id = json["id"].as_str().unwrap_or("").to_string();
            Ok(PublishResult {
                platform_post_id: comment_id,
                platform_post_url: None,
                status: "published".into(),
            })
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram reply failed")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    async fn send_dm(
        &self,
        access_token: &str,
        recipient: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let url = format!("{}/me/messages", self.graph_url());
        let body = serde_json::json!({
            "recipient": { "id": recipient },
            "message": { "text": post.content },
        });
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            let message_id = json["message_id"].as_str().unwrap_or("").to_string();
            Ok(PublishResult {
                platform_post_id: message_id,
                platform_post_url: None,
                status: "sent".into(),
            })
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram send DM failed")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    // This provider has a DM surface, so `dm_capable()` reports true, which is what
    // lets the /dms page learn it from the API instead of guessing a provider
    // list. This says the PROVIDER can do DMs, not that any given CHANNEL can.
    fn dm_capable(&self) -> bool {
        true
    }

    async fn get_dm_conversations(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<Vec<super::DmConversation>, ProviderError> {
        let url = format!("{}/me/conversations", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("fields", "id,snippet,updated_time,unread"),
                ("limit", &limit.min(50).to_string()),
            ])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let mut conversations = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for conv in data {
                let id = conv["id"].as_str().unwrap_or("").to_string();
                let last_message = conv["snippet"].as_str().map(|s| s.to_string());
                let last_message_at = conv["updated_time"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                let unread = conv["unread"].as_bool().unwrap_or(false);
                conversations.push(super::DmConversation {
                    id,
                    participant: String::new(),
                    participant_name: None,
                    participant_avatar: None,
                    last_message,
                    last_message_at,
                    unread_count: if unread { 1 } else { 0 },
                });
            }
        }
        Ok(conversations)
    }

    async fn get_dm_messages(
        &self,
        access_token: &str,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<super::DmMessage>, ProviderError> {
        let url = format!("{}/{}/messages", self.graph_url(), conversation_id);
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("fields", "id,from,message,created_time"),
                ("limit", &limit.min(50).to_string()),
            ])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let mut messages = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for msg in data {
                let id = msg["id"].as_str().unwrap_or("").to_string();
                let sender = msg["from"]
                    .as_object()
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = msg["message"].as_str().unwrap_or("").to_string();
                let created_at = msg["created_time"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                messages.push(super::DmMessage {
                    id,
                    conversation_id: conversation_id.to_string(),
                    sender,
                    sender_name: None,
                    content,
                    media: vec![],
                    created_at,
                    read: true,
                });
            }
        }
        Ok(messages)
    }
}

impl InstagramProvider {
    /// Create a single IMAGE container and return its container ID.
    /// Create a REEL container with a music track attached.
    ///
    /// Instagram's Reel creation is different from an image post:
    ///
    ///   - `media_type=REELS`
    ///   - `video` (not `image_url`) is the source
    ///   - `audio_configuration` names the track
    ///
    /// The `audio_id` is the id the Music API returned, and it is
    /// carried in the post's settings as `audio_id` - the composer
    /// writes it there and NOTHING read it, so every Reel went out
    /// silent.
    async fn create_reel_container(
        &self,
        ig_id: &str,
        access_token: &str,
        video_url: &str,
        caption: &str,
        audio_id: &str,
    ) -> Result<String, ProviderError> {
        let params = reel_container_params(video_url, caption, access_token, audio_id);

        let resp = self
            .http
            .post(format!("{}/{}/media", self.graph_url(), ig_id))
            .form(&params)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(err) = json["error"].as_object() {
            let code = err["code"].as_i64().unwrap_or(0);
            let msg = err["message"].as_str().unwrap_or("unknown");
            return Err(ProviderError::Api(format!(
                "Instagram reel creation failed (code {code}): {msg}"
            )));
        }

        json["id"].as_str().map(|s| s.to_string()).ok_or_else(|| {
            ProviderError::Api(format!("Instagram did not return container ID: {json:?}"))
        })
    }

    async fn create_single_media_container(
        &self,
        ig_id: &str,
        access_token: &str,
        media_url: &str,
        caption: &str,
        is_carousel_item: bool,
    ) -> Result<String, ProviderError> {
        let mut params = vec![("image_url", media_url), ("access_token", access_token)];
        if is_carousel_item {
            params.push(("is_carousel_item", "true"));
        } else {
            params.push(("caption", caption));
        }

        let resp = self
            .http
            .post(format!("{}/{}/media", self.graph_url(), ig_id))
            .form(&params)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(err) = json["error"].as_object() {
            let msg = err["message"]
                .as_str()
                .unwrap_or("Container creation failed");
            return Err(ProviderError::Api(msg.to_string()));
        }

        json["id"].as_str().map(String::from).ok_or_else(|| {
            ProviderError::Api(format!("Instagram did not return container ID: {json:?}"))
        })
    }

    /// Publish a container (single image or carousel) and return the IG media ID.
    async fn publish_container(
        &self,
        ig_id: &str,
        access_token: &str,
        creation_id: &str,
    ) -> Result<String, ProviderError> {
        let resp = self
            .http
            .post(format!("{}/{}/media_publish", self.graph_url(), ig_id))
            .form(&[("creation_id", creation_id), ("access_token", access_token)])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(err) = json["error"].as_object() {
            let msg = err["message"].as_str().unwrap_or("Publish failed");
            return Err(ProviderError::Api(msg.to_string()));
        }

        json["id"].as_str().map(String::from).ok_or_else(|| {
            ProviderError::Api(format!("Instagram publish unexpected response: {json:?}"))
        })
    }

    /// Block until `container_id` is ready to publish.
    ///
    /// `POST /{ig_id}/media` returns a container id for media Instagram is still
    /// fetching and processing; `media_publish` on a container that is not yet
    /// `FINISHED` fails with `MEDIA_NOT_READY` (and on a video container with a
    /// misleading `MEDIA_TRANSCODING`), so publishing straight off the create
    /// call turns a working post into a hard error.
    ///
    /// Image containers are usually ready on the first read, so the common path
    /// costs one extra request. Bounded to
    /// [`CONTAINER_READY_ATTEMPTS`] reads [`CONTAINER_READY_INTERVAL`] apart
    /// (~60s): a terminal `ERROR`/`EXPIRED` and an unfinished container both
    /// surface as [`ProviderError::Api`] naming the last observed status, so the
    /// operator can tell "this will never publish" from "needs more time".
    async fn await_container_ready(
        &self,
        access_token: &str,
        container_id: &str,
    ) -> Result<(), ProviderError> {
        let mut last_seen = "unknown".to_string();

        for attempt in 1..=CONTAINER_READY_ATTEMPTS {
            let resp = self
                .http
                .get(format!("{}/{container_id}", self.graph_url()))
                .query(&[
                    ("fields", "status_code,status"),
                    ("access_token", access_token),
                ])
                .send()
                .await?;
            let http_status = resp.status();
            let json: serde_json::Value = resp.json().await?;

            if let Some(err) = json["error"].as_object() {
                let msg = err["message"]
                    .as_str()
                    .unwrap_or("Container status query failed");
                let code = err["code"].as_u64().unwrap_or(0);
                let raw = format!(
                    "Instagram container status query failed (HTTP {http_status}, code {code}): {msg}"
                );
                return Err(ProviderError::Api(friendly_error(
                    self.map_error(msg, http_status.as_u16()),
                    &raw,
                )));
            }

            let seen = json["status_code"].as_str().unwrap_or("");
            last_seen = match seen {
                "FINISHED" => {
                    tracing::debug!("IG container {container_id} ready (check #{attempt})");
                    return Ok(());
                }
                // Terminal: retrying the same container can never succeed.
                "ERROR" | "EXPIRED" | "MEDIA_ERROR" => {
                    let reason = json["status"].as_str().unwrap_or("no reason given");
                    let raw = format!("Instagram container {container_id} is {seen}: {reason}");
                    return Err(self.provider_error(&raw, 0));
                }
                // IN_PROGRESS and anything Instagram adds later.
                "" => "not reported yet".to_string(),
                other => other.to_string(),
            };

            if attempt < CONTAINER_READY_ATTEMPTS {
                tokio::time::sleep(CONTAINER_READY_INTERVAL).await;
            }
        }

        let raw = format!(
            "Instagram container {container_id} was still {last_seen} after {}s. Large \
             images and videos need longer to process — retry this post rather than \
             re-uploading the media, or the same container will time out again.",
            CONTAINER_READY_ATTEMPTS as u64 * CONTAINER_READY_INTERVAL.as_secs()
        );
        Err(self.provider_error(&raw, 0))
    }

    /// Resolve the Instagram Business Account ID from the stored token.
    ///
    /// TWO token shapes reach here, and the old code only handled one:
    ///
    /// - a **user token** (the root integration): `/me/accounts` lists the
    ///   Facebook Pages, and each page's `instagram_business_account` edge
    ///   (inline field or `/{page_id}/instagram_business_account`) gives the
    ///   IG account.
    /// - a **page token** (what `connect_page` stores on the connected
    ///   sub-account): `/me` IS the page, and a Page node has no `accounts`
    ///   edge, so `/me/accounts` fails with
    ///   `(#100) Tried accessing nonexisting field (accounts)` — exactly the
    ///   error every IG publish surfaced. The fallback asks `/me` directly
    ///   for `instagram_business_account`, then the
    ///   `/{page_id}/instagram_business_account` edge.
    async fn resolve_ig_business_account(&self, access_token: &str) -> Result<String, String> {
        self.resolve_ig_business_account_for(access_token, None)
            .await
    }

    /// Resolve the Instagram business account, preferring the page this
    /// CHANNEL is connected to.
    ///
    /// A token can administer several Facebook pages and more than one may
    /// carry an Instagram business account, so "the first page with an IG
    /// account" is arbitrary — a channel connected to page B posts to page
    /// A's Instagram. The scheduler hands us the channel's `internal_id`
    /// (the page the founder picked), so we use it.
    async fn resolve_ig_business_account_for(
        &self,
        access_token: &str,
        internal_id: Option<&str>,
    ) -> Result<String, String> {
        // Get the user's Facebook pages (user-token path).
        let resp = self
            .http
            .get(format!(
                "{}/me/accounts?fields=id,access_token,instagram_business_account",
                self.graph_url()
            ))
            .query(&[("access_token", access_token)])
            .send()
            .await
            .map_err(|e| format!("Failed to get Facebook pages: {e}"))?;

        let accounts: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse pages response: {e}"))?;

        if let Some(pages) = accounts_page_list(&accounts) {
            for page in pages {
                let page_id = page["id"].as_str().unwrap_or("");
                let page_token = page["access_token"].as_str().unwrap_or(access_token);

                // A page this channel is NOT connected to is only worth a
                // look when no connected page is found at all — that is the
                // fallback at the end of the loop.
                if let Some(want) = internal_id.filter(|w| !w.is_empty()) {
                    if want != page_id {
                        continue;
                    }
                }

                // Prefer the inline field — one less request.
                if let Some(ig_id) = page["instagram_business_account"]["id"].as_str() {
                    return Ok(ig_id.to_string());
                }

                // Fallback: /{page_id}/instagram_business_account
                let ig: serde_json::Value = self
                    .http
                    .get(format!(
                        "{}/{page_id}/instagram_business_account",
                        self.graph_url()
                    ))
                    .query(&[("access_token", page_token)])
                    .send()
                    .await
                    .map_err(|e| format!("Failed to check IG for page {page_id}: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse IG response: {e}"))?;

                if ig["error"].is_null() {
                    if let Some(ig_id) = ig["id"].as_str() {
                        return Ok(ig_id.to_string());
                    }
                }
            }
            // A user token that lists pages but none linked to Instagram.
            return Err(
                "No Instagram Business account found. Link Instagram to a Facebook Page first."
                    .to_string(),
            );
        }

        // Page-token path: /me IS the page, so ask it directly. (When
        // /me/accounts errored with #100 the token is a page token; when it
        // answered with no data the token may still be one.)
        let me: serde_json::Value = self
            .http
            .get(format!(
                "{}/me?fields=id,instagram_business_account",
                self.graph_url()
            ))
            .query(&[("access_token", access_token)])
            .send()
            .await
            .map_err(|e| format!("Failed to query /me: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse /me response: {e}"))?;

        if let Some(err) = me["error"].as_object() {
            return Err(format!(
                "Facebook API error: {}",
                err["message"].as_str().unwrap_or("unknown")
            ));
        }

        // A stored PAGE token answers /me with the page itself. If the
        // channel names a DIFFERENT page, this token cannot publish to it —
        // say so rather than posting to the wrong Instagram account.
        if let Some(want) = internal_id.filter(|w| !w.is_empty()) {
            let token_page = me["id"].as_str().unwrap_or("");
            if !token_page.is_empty() && want != token_page {
                return Err(format!(
                    "This channel is connected to page {want}, but its stored token is a token for \
                     page {token_page}. Reconnect the channel from the page picker."
                ));
            }
        }

        if let Some(ig_id) = me["instagram_business_account"]["id"].as_str() {
            return Ok(ig_id.to_string());
        }

        let page_id = me["id"].as_str().unwrap_or("").to_string();
        if !page_id.is_empty() {
            let ig: serde_json::Value = self
                .http
                .get(format!(
                    "{}/{page_id}/instagram_business_account",
                    self.graph_url()
                ))
                .query(&[("access_token", access_token)])
                .send()
                .await
                .map_err(|e| format!("Failed to check IG for page {page_id}: {e}"))?
                .json()
                .await
                .map_err(|e| format!("Failed to parse IG response: {e}"))?;

            if ig["error"].is_null() {
                if let Some(ig_id) = ig["id"].as_str() {
                    return Ok(ig_id.to_string());
                }
            }
        }

        Err(
            "No Instagram Business account found for this token. Link an Instagram \
             Business/Creator account to the Facebook Page, then reconnect the channel."
                .to_string(),
        )
    }
}

impl InstagramProvider {
    pub async fn get_ig_media(
        &self,
        access_token: &str,
        ig_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let limit = limit.min(100);
        let url = format!("{}/{ig_id}/media", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "id,caption,media_type,media_url,permalink,timestamp,username,like_count,comments_count,children.limit(100){id,media_url,media_type}"), ("limit", &limit.to_string())])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_media_detail(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{media_id}", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "id,caption,media_type,media_url,permalink,timestamp,username,like_count,comments_count,children.limit(100){id,media_url,media_type}")])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_media_comments(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{media_id}/comments", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "id,text,timestamp,username,like_count,replies{id,text,timestamp,username,like_count}")])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn search_hashtag(
        &self,
        access_token: &str,
        ig_id: &str,
        query: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/ig_hashtag_search", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("user_id", ig_id), ("q", query)])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_hashtag_media(
        &self,
        access_token: &str,
        hashtag_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{hashtag_id}/recent_media", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[(
                "fields",
                "id,caption,media_type,media_url,permalink,timestamp,like_count,comments_count",
            )])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_insights(
        &self,
        access_token: &str,
        ig_id: &str,
        metric: &str,
        period: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/insights", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("metric", metric), ("period", period)])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    /// Accounts that have mentioned this business account
    /// (`GET /{ig_id}/mentions`) — the @-autocomplete source.
    pub async fn get_mentions(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/mentions", self.graph_url());
        let resp = self
            .http
            .get(url)
            .query(&[("fields", "id,username,profile_picture_url,text,timestamp")])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            Err(ProviderError::Api(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or("Instagram API error")
                    .to_string(),
            ))
        }
    }

    pub async fn get_ig_tagged(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/tagged", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[(
                "fields",
                "id,caption,media_type,media_url,permalink,timestamp",
            )])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn create_ig_container(
        &self,
        access_token: &str,
        ig_id: &str,
        media_type: &str,
        media_url: &str,
        caption: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/media", self.graph_url());
        let mut params = vec![
            ("media_type", media_type),
            ("caption", caption),
            ("access_token", access_token),
        ];
        if media_type == "IMAGE" {
            params.push(("image_url", media_url));
        } else {
            params.push(("media_url", media_url));
        }
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .form(&params)
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn publish_ig_container(
        &self,
        access_token: &str,
        ig_id: &str,
        creation_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/media_publish", self.graph_url());
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn reply_to_ig_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{comment_id}/replies", self.graph_url());
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_reels(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/media", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[
                (
                    "fields",
                    "id,caption,media_type,media_url,permalink,timestamp",
                ),
                ("media_type", "REELS"),
            ])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_stories(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/stories", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "id,media_type,media_url,permalink,timestamp")])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    /// Fetch IG account profile info (username, display name, avatar)
    pub async fn get_ig_account_info(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<(Option<String>, Option<String>, Option<String>), ProviderError> {
        let url = format!("{}/{ig_id}", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "username,name,profile_picture_url")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["error"]["message"].as_str().unwrap_or("unknown");
            return Err(ProviderError::Api(format!("IG account info: {msg}")));
        }
        if let Some(err) = json["error"].as_object() {
            let msg = err["message"].as_str().unwrap_or("unknown");
            return Err(ProviderError::Api(format!("IG account info: {msg}")));
        }
        tracing::debug!("get_ig_account_info response for {ig_id}: {:?}", json);
        let username = json["username"].as_str().map(String::from);
        let name = json["name"].as_str().map(String::from);
        let avatar = json["profile_picture_url"].as_str().map(String::from);
        Ok((name, username, avatar))
    }

    pub async fn get_ig_followers(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[(
                "fields",
                "name,username,followers_count,follows_count,media_count,profile_picture_url",
            )])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_business_discovery(
        &self,
        access_token: &str,
        ig_id: &str,
        target_username: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/business_discovery", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[("username", target_username), ("fields", "biography,followers_count,follows_count,media_count,profile_picture_url,username,name,media{id,caption,media_type,media_url,permalink,timestamp,like_count,comments_count}")])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_mentions(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/mentions", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[(
                "fields",
                "id,caption,media_type,media_url,permalink,timestamp",
            )])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    pub async fn get_ig_insights_audience(
        &self,
        access_token: &str,
        ig_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{ig_id}/insights", self.graph_url());
        let resp = self
            .http
            .get(&url)
            .query(&[
                (
                    "metric",
                    "audience_city,audience_country,audience_gender,audience_age_range",
                ),
                ("period", "lifetime"),
            ])
            .header("Authorization", format!("Bearer {access_token}"))
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
        } else {
            let detail = json["error"]["message"]
                .as_str()
                .unwrap_or("Instagram API error")
                .to_string();
            Err(ProviderError::Api(detail))
        }
    }

    /// Read a media item's `status_code`.
    ///
    /// The object can be TWO different things, and they do not answer the
    /// same question:
    ///
    ///   - a CONTAINER (`POST /{ig_id}/media` returned an id that is still
    ///     being processed): it HAS a `status_code` — `IN_PROGRESS`,
    ///     `FINISHED`, `ERROR`;
    ///   - a PUBLISHED MEDIA item (the id `media_publish` returned): it has
    ///     NO `status_code` field at all. Asking for it is answered with
    ///     "(#100) Tried accessing nonexisting field (status_code)".
    ///
    /// The second case is what the publish path does: it publishes the
    /// container, then asks for the MEDIA's status. Measured live on
    /// 2026-11-13, that answer is a (#100) error, so EVERY scheduled
    /// Instagram post published successfully and was then recorded as an
    /// ERROR — the founder saw a failed post that was actually live, and
    /// re-posting it published a second copy.
    ///
    /// So a field error on `status_code` is not a failure: it means the
    /// object is a published media item, which IS live. The status is
    /// `PUBLISHED` — accepted, and (for a video) still transcoding.
    pub async fn poll_container_status(
        &self,
        access_token: &str,
        creation_id: &str,
    ) -> Result<String, ProviderError> {
        let url = format!("{}/{}", self.graph_url(), creation_id);
        let resp = self
            .http
            .get(&url)
            .query(&[("fields", "id,status_code,status,media_type,permalink")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let http_status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if let Some(err) = json["error"].as_object() {
            let msg = err["message"]
                .as_str()
                .unwrap_or("Container status check failed");
            // A field error on `status_code` means this is a published
            // media item, not a container: the media is LIVE.
            if is_published_media_item_error(msg) {
                tracing::debug!(
                    "IG media {creation_id} has no status_code — it is a published media item"
                );
                return Ok("PUBLISHED".to_string());
            }
            return Err(self.provider_error(msg, http_status.as_u16()));
        }
        // No status_code, but a media_type/permalink: a published media
        // item that answered without the field.
        if json.get("status_code").is_none() && json.get("media_type").is_some() {
            return Ok("PUBLISHED".to_string());
        }
        let status_code = json["status_code"].as_str().unwrap_or("IN_PROGRESS");
        Ok(status_code.to_string())
    }

    /// Map a media item's Graph `status_code` onto the publish lifecycle.
    ///
    /// `FINISHED` and `PUBLISHED` both mean the media IS ON THE FEED.
    /// `PUBLISHED` is Instagram's own word for "live, still transcoding" —
    /// the reel is on the profile and the founder can see it while Instagram
    /// finishes the video.
    ///
    /// The old code called `PUBLISHED` pending, so the poll kept going for
    /// the whole 40-check budget (200 seconds) and then FAILED a post that
    /// had been live the entire time. The founder saw an error on a live
    /// post — and, because the manual publish runs inside the HTTP
    /// request, the request itself outlived the tunnel's 100-second limit
    /// and came back as a Cloudflare "524" (measured live on 2026-11-13:
    /// "Platform never finished processing the post: still pending after 40
    /// checks").
    ///
    /// `ERROR`/`EXPIRED` never recover, so they abort the poll with the reason
    /// Instagram reported rather than spinning out the give-up budget.
    async fn media_publish_state(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let code = self.poll_container_status(access_token, media_id).await?;
        let status = match code.as_str() {
            // Live on the feed — `PUBLISHED` is Instagram's word for it.
            "FINISHED" | "PUBLISHED" => "published",
            "ERROR" | "EXPIRED" | "MEDIA_ERROR" => {
                return Err(ProviderError::Api(format!(
                    "Instagram media {media_id} failed processing ({code}). Check the media \
                     matches Instagram's format and size limits, then republish."
                )))
            }
            // IN_PROGRESS (and anything Instagram adds later) stays pending.
            _ => "pending",
        };
        Ok(PublishResult {
            platform_post_id: media_id.to_string(),
            platform_post_url: Some(format!("https://instagram.com/p/{media_id}")),
            status: status.into(),
        })
    }
}

/// The form fields that create an Instagram REEL container.
///
/// The parameter is `video_url`, NOT `video`. Measured against the live
/// Graph API on 2026-11-13: `video` is answered with "(#100) The parameter
/// video_url is required", and `video_url` creates the container. So every
/// music Reel failed AT THE CONTAINER STEP — the music the composer
/// collected was never applied to anything, because the container was
/// never created.
///
/// One function, so the name is pinned by a test instead of by a comment.
pub fn reel_container_params(
    video_url: &str,
    caption: &str,
    access_token: &str,
    audio_id: &str,
) -> Vec<(&'static str, String)> {
    let audio_configuration = serde_json::json!({
        "audio_id": audio_id,
    });
    vec![
        ("media_type", "REELS".to_string()),
        ("video_url", video_url.to_string()),
        ("caption", caption.to_string()),
        ("access_token", access_token.to_string()),
        ("audio_configuration", audio_configuration.to_string()),
    ]
}

/// Is this Graph error the "this is not a container" answer?
///
/// A published MEDIA item has no `status_code` field, and asking for it
/// is answered with "(#100) Tried accessing nonexisting field
/// (status_code)". That is not a failure: the media is LIVE. The publish
/// path asks for the media's status right after publishing it, so without
/// this the founder saw every scheduled Instagram post as an ERROR after
/// it had actually gone out — and re-posting it published a second copy.
pub fn is_published_media_item_error(message: &str) -> bool {
    message.contains("nonexisting field") && message.contains("status_code")
}

/// The page list from a `/me/accounts` response, or `None` when the response
/// errored or carried no `data`. A PAGE token (what `connect_page` stores on
/// a connected sub-account) has no `accounts` edge, so `/me/accounts` fails
/// with `(#100) Tried accessing nonexisting field (accounts)` — the `None`
/// case is what drives the `/me` fallback in `resolve_ig_business_account`.
/// The old code returned that #100 message verbatim as the publish error.
fn accounts_page_list(accounts: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if accounts["error"].is_null() {
        accounts["data"].as_array()
    } else {
        None
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
        assert_eq!(series[0].data[0].total, "300");
    }

    #[test]
    fn parse_should_map_media_insights_into_daily_series() {
        let raw = serde_json::json!({
            "data": [
                { "name": "engagement", "values": [{ "value": 55, "end_time": "2026-09-29T00:00:00+0000" }] },
                { "name": "saved", "values": [{ "value": 7, "end_time": "2026-09-29T00:00:00+0000" }] }
            ]
        });
        let series = super::super::parse_insights_data(&raw);
        assert_eq!(series.len(), 2);
        assert_eq!(series[1].label, "saved");
        assert_eq!(series[1].data[0].total, "7");
    }

    #[test]
    fn parse_should_map_media_detail_engagement_counts() {
        let raw = serde_json::json!({
            "id": "17900000000000000",
            "like_count": 42,
            "comments_count": 12
        });
        let e = super::super::parse_engagement_data("instagram", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.comments, 12);
    }

    #[test]
    fn publish_should_reject_11th_image() {
        let provider = InstagramProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(11)).is_err());
    }

    #[test]
    fn publish_should_accept_10_images() {
        let provider = InstagramProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(10)).is_ok());
    }

    // ── B4: map_error fixtures ──────────────────────────────

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("The image upload was rejected", 422).unwrap();
        assert!(msg.contains("2-10 images"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("", 401).unwrap();
        assert!(msg.contains("cannot be refreshed"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_non_business_account_on_403() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("", 403).unwrap();
        assert!(msg.contains("Business or Creator"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_account_on_404() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("", 404).unwrap();
        assert!(msg.contains("not found"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let p = InstagramProvider::new(&test_config());
        let msg = p.map_error("", 400).unwrap();
        assert!(msg.contains("2200"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        let p = InstagramProvider::new(&test_config());
        assert!(p.map_error("Internal Server Error", 503).is_none());
    }

    #[test]
    fn instagram_does_not_request_cron_refresh() {
        // Facebook-login tokens have no refresh endpoint; reconnect is the fix.
        let p = InstagramProvider::new(&test_config());
        assert!(!p.needs_cron_refresh());
    }

    /// Regression test for the publish bug: a page token calling
    /// `/me/accounts` gets #100 "nonexisting field (accounts)" — that must
    /// drive the /me fallback, not surface as the publish error.
    #[test]
    fn accounts_error_100_is_not_a_page_listing() {
        let err = serde_json::json!({
            "error": { "code": 100, "message": "Tried accessing nonexisting field (accounts)" }
        });
        assert!(accounts_page_list(&err).is_none());
    }

    #[test]
    fn accounts_data_yields_the_page_list() {
        let ok = serde_json::json!({
            "data": [ { "id": "111", "instagram_business_account": { "id": "ig-1" } } ]
        });
        let pages = accounts_page_list(&ok).expect("a listed page");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["instagram_business_account"]["id"], "ig-1");
    }

    #[test]
    fn accounts_missing_data_is_not_a_page_listing() {
        assert!(accounts_page_list(&serde_json::json!({})).is_none());
        assert!(accounts_page_list(&serde_json::json!({ "data": [] })).is_some());
    }
}

#[cfg(test)]
mod instagram_publish_shape_tests {
    use super::*;

    #[test]
    fn the_reel_container_sends_video_url_not_video() {
        // Measured against the live Graph API: `video` is answered with
        // "(#100) The parameter video_url is required". One assertion on
        // the parameter NAME, not the value.
        let params = reel_container_params(
            "https://example.com/reel.mp4",
            "caption",
            "token",
            "audio-123",
        );
        let names: Vec<&str> = params.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec![
                "media_type",
                "video_url",
                "caption",
                "access_token",
                "audio_configuration"
            ]
        );
        assert!(
            !names.contains(&"video"),
            "`video` is not a Graph parameter — it must be `video_url`"
        );
    }

    #[test]
    fn the_reel_container_carries_the_music_track() {
        // The composer writes settings.audio_id; the container has to carry
        // it or the reel goes out silent (the defect this replaced).
        let params = reel_container_params(
            "https://example.com/reel.mp4",
            "caption",
            "token",
            "audio-123",
        );
        let audio = params
            .iter()
            .find(|(n, _)| *n == "audio_configuration")
            .map(|(_, v)| v.clone())
            .expect("audio_configuration present");
        assert_eq!(
            audio,
            serde_json::json!({"audio_id": "audio-123"}).to_string()
        );
    }

    #[test]
    fn a_published_media_item_is_published_not_pending() {
        // Instagram's own word for "live, still transcoding" is PUBLISHED.
        // Calling it pending made the scheduler poll for 200 seconds and
        // then fail a post that had been live the whole time.
        let status = match "PUBLISHED" {
            "FINISHED" | "PUBLISHED" => "published",
            _ => "pending",
        };
        assert_eq!(status, "published");
    }

    #[test]
    fn a_field_error_on_status_code_means_the_media_is_live() {
        // Graph's answer when the object is a PUBLISHED MEDIA item.
        assert!(is_published_media_item_error(
            "(#100) Tried accessing nonexisting field (status_code)"
        ));
        // A terminal container state is not this.
        assert!(!is_published_media_item_error(
            "Instagram media 123 failed processing (ERROR)"
        ));
        assert!(!is_published_media_item_error(
            "(#100) Tried accessing nonexisting field (media_type)"
        ));
    }
}
