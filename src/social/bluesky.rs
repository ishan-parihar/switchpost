// ─── Bluesky Provider ─────────────────────────────────────────
// Uses AT Protocol (ATP) with username + app password (no OAuth).
// Simplest provider to implement — ideal for MVP validation.

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct BlueskyProvider {
    handle: String,
    app_password: String,
    http: reqwest::Client,
}

impl BlueskyProvider {
    pub fn new(config: &Config) -> Self {
        Self {
            handle: config.bluesky_handle.clone().unwrap_or_default(),
            app_password: config.bluesky_app_password.clone().unwrap_or_default(),
            http: shared_http(),
        }
    }

    /// Authenticate with Bluesky and get a session token (JWT)
    async fn create_session(&self) -> Result<String, ProviderError> {
        let resp = self
            .http
            .post("https://bsky.social/xrpc/com.atproto.server.createSession")
            .json(&serde_json::json!({
                "identifier": self.handle,
                "password": self.app_password,
            }))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 200 {
            json["accessJwt"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| ProviderError::Auth("Missing accessJwt".into()))
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Bluesky auth failed")
                .to_string();
            Err(ProviderError::Auth(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &msg,
            )))
        }
    }

    /// Get the user's DID (Decentralized Identifier)
    async fn resolve_handle(&self) -> Result<String, ProviderError> {
        let resp = self
            .http
            .get("https://bsky.social/xrpc/com.atproto.identity.resolveHandle")
            .query(&[("handle", &self.handle)])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        json["did"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| ProviderError::Auth("Could not resolve handle".into()))
    }

    /// Fetch the CID for a Bluesky record URI (needed for the `reply` field
    /// when threading). The URI looks like `at://did:plc:xxx/app.bsky.feed.post/yyy`.
    async fn fetch_record_cid(
        &self,
        uri: &str,
        access_token: &str,
    ) -> Result<String, ProviderError> {
        // Parse the URI to extract the repo (DID), collection, and rkey
        // Format: at://did:plc:xxx/app.bsky.feed.post/yyy
        let parts: Vec<&str> = uri.split('/').collect();
        if parts.len() < 5 {
            return Err(ProviderError::Api(format!("Invalid Bluesky URI: {uri}")));
        }
        let repo = parts[2]; // did:plc:xxx
        let collection = parts[3]; // app.bsky.feed.post
        let rkey = parts[4]; // post rkey

        let resp = self
            .http
            .get("https://bsky.social/xrpc/com.atproto.repo.getRecord")
            .query(&[("repo", repo), ("collection", collection), ("rkey", rkey)])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        json["cid"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| ProviderError::Api(format!("Could not fetch CID for {uri}")))
    }

    // ── Analytics helpers ────────────────────────────────────

    /// Fetch the author profile record (follower/follows/post counts).
    pub async fn get_profile(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let jwt = self.session_jwt(access_token).await?;
        let resp = self
            .http
            .get("https://bsky.social/xrpc/app.bsky.actor.getProfile")
            .header("Authorization", format!("Bearer {jwt}"))
            .query(&[("actor", &self.handle)])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        self.check_status(status, &body, "getProfile")?;
        Self::parse_body(&body, "getProfile")
    }

    /// Look up a post's counters by CID. The AT Protocol has no "get one post
    /// by CID" endpoint that carries engagement counts, so we page the author
    /// feed and match — the same lookup `get_post_engagement` already used.
    async fn post_metrics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        let jwt = self.session_jwt(access_token).await?;
        let did = self.resolve_handle().await?;

        let limit = "30".to_string();
        let resp = self
            .http
            .get("https://bsky.social/xrpc/app.bsky.feed.getAuthorFeed")
            .header("Authorization", format!("Bearer {jwt}"))
            .query(&[("actor", &did), ("limit", &limit)])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        self.check_status(status, &body, "getAuthorFeed")?;
        let json = Self::parse_body(&body, "getAuthorFeed")?;
        Ok(Self::post_metrics_from_feed(&json, platform_post_id))
    }

    /// Use the stored session JWT, or mint one when the caller has none.
    async fn session_jwt(&self, access_token: &str) -> Result<String, ProviderError> {
        if access_token.is_empty() {
            self.create_session().await
        } else {
            Ok(access_token.to_string())
        }
    }

    /// Check an XRPC response, routing failures through the shared error
    /// classifier so every Bluesky call reports the same user-facing copy.
    fn check_status(&self, status: u16, body: &str, operation: &str) -> Result<(), ProviderError> {
        if (200..300).contains(&status) {
            return Ok(());
        }
        let raw = format!("Bluesky {operation} failed (HTTP {status}): {body}");
        Err(self.provider_error(&raw, status))
    }

    fn parse_body(body: &str, operation: &str) -> Result<serde_json::Value, ProviderError> {
        serde_json::from_str(body)
            .map_err(|e| ProviderError::Api(format!("Bluesky {operation} JSON parse error: {e}")))
    }

    // ── Analytics parsers (pure: fixture-testable, no HTTP) ──

    /// Map a `getPostThread` response into the reply chain. `thread.reply` is a
    /// flat array where each view carries its `parent`, so replies nest one
    /// level; a reply whose parent is not in the page stays top-level.
    pub(crate) fn parse_thread_replies(json: &serde_json::Value) -> Vec<CommentData> {
        let flat: Vec<(String, Option<String>, CommentData)> = json
            .pointer("/thread/reply")
            .and_then(|r| r.as_array())
            .map(|views| {
                views
                    .iter()
                    .filter_map(|view| {
                        let post = view.get("post")?;
                        let uri = post["uri"].as_str()?.to_string();
                        let parent = post["record"]
                            .get("reply")
                            .and_then(|r| r.get("parent"))
                            .and_then(|p| p["uri"].as_str())
                            .map(String::from);
                        Some((
                            uri.clone(),
                            parent,
                            CommentData {
                                id: uri,
                                author_name: post["author"]["displayName"]
                                    .as_str()
                                    .or(post["author"]["handle"].as_str())
                                    .map(String::from),
                                author_avatar: post["author"]["avatar"].as_str().map(String::from),
                                text: post["record"]["text"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                                created_at: post["record"]["createdAt"]
                                    .as_str()
                                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                    .map(|dt| dt.with_timezone(&chrono::Utc))
                                    .unwrap_or_else(chrono::Utc::now),
                                like_count: post["likeCount"].as_i64().unwrap_or(0) as i32,
                                replies: vec![],
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut roots: Vec<CommentData> = Vec::new();
        let mut root_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (uri, parent, comment) in flat {
            match parent
                .as_deref()
                .filter(|p| *p != uri)
                .and_then(|p| root_index.get(p))
            {
                Some(&i) => roots[i].replies.push(comment),
                // No parent on the page (or it is this reply itself): keep it
                // top-level rather than dropping it.
                None => {
                    root_index.insert(uri, roots.len());
                    roots.push(comment);
                }
            }
        }
        roots
    }

    /// Map an `app.bsky.actor.searchActors` response into @-autocomplete
    /// candidates. The DID is the stable id; the handle is what `@` resolves to.
    pub(crate) fn parse_actor_search(json: &serde_json::Value) -> Vec<MentionResult> {
        json.get("actors")
            .and_then(|a| a.as_array())
            .map(|actors| {
                actors
                    .iter()
                    .filter_map(|actor| {
                        Some(MentionResult {
                            id: actor["did"].as_str()?.to_string(),
                            label: actor["handle"].as_str()?.to_string(),
                            image: actor["avatar"].as_str().map(String::from),
                            do_not_cache: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Scan a `getAuthorFeed` response for the post with this CID and return
    /// its counters. Returns `None` when the post is not on the first page.
    pub(crate) fn post_metrics_from_feed(
        json: &serde_json::Value,
        platform_post_id: &str,
    ) -> Option<serde_json::Value> {
        json["feed"].as_array()?.iter().find_map(|item| {
            let post = &item["post"];
            if post["cid"].as_str()? != platform_post_id {
                return None;
            }
            Some(serde_json::json!({
                "likeCount": post["likeCount"].as_i64().unwrap_or(0),
                "repostCount": post["repostCount"].as_i64().unwrap_or(0),
                "replyCount": post["replyCount"].as_i64().unwrap_or(0),
                "quoteCount": post["quoteCount"].as_i64().unwrap_or(0),
            }))
        })
    }

    /// Map an `app.bsky.actor.getProfile` response into dashboard rows.
    pub(crate) fn profile_analytics(json: &serde_json::Value) -> Vec<AnalyticsData> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        [
            ("Followers", "followersCount"),
            ("Following", "followsCount"),
            ("Posts", "postsCount"),
        ]
        .into_iter()
        .filter_map(|(label, key)| {
            let total = json.get(key).and_then(|v| v.as_i64())?.to_string();
            Some(AnalyticsData {
                label: label.into(),
                data: vec![AnalyticsDataPoint {
                    total,
                    date: today.clone(),
                }],
                percentage_change: 0.0,
            })
        })
        .collect()
    }

    /// Map a post's counters into per-post dashboard rows.
    pub(crate) fn post_analytics_rows(metrics: &serde_json::Value) -> Vec<AnalyticsData> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        [
            ("Likes", "likeCount"),
            ("Reposts", "repostCount"),
            ("Replies", "replyCount"),
            ("Quotes", "quoteCount"),
        ]
        .into_iter()
        .filter_map(|(label, key)| {
            let total = metrics.get(key).and_then(|v| v.as_i64())?.to_string();
            Some(AnalyticsData {
                label: label.into(),
                data: vec![AnalyticsDataPoint {
                    total,
                    date: today.clone(),
                }],
                percentage_change: 0.0,
            })
        })
        .collect()
    }
}

#[async_trait]
impl SocialProvider for BlueskyProvider {
    fn identifier(&self) -> &'static str {
        "bluesky"
    }

    fn name(&self) -> &'static str {
        "Bluesky"
    }

    fn scopes(&self) -> Vec<String> {
        vec![] // Bluesky uses app password, not OAuth scopes
    }

    fn max_content_length(&self) -> usize {
        300
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    /// Bluesky doesn't use OAuth; this returns an error instructing the user
    fn uses_oauth(&self) -> bool {
        false // Bluesky uses app passwords instead of OAuth
    }

    /// Bluesky-specific user-facing error copy (v25 §2 row 9).
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Bluesky rate limit exceeded. The PDS is throttling this session — \
                 retry in a minute or reduce the publish cadence."
                    .into(),
            ),
            Some(ErrorKind::Duplicate) => Some(
                "Bluesky rejected this post as a duplicate of a recent post. \
                 Change the text before republishing."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Bluesky rejected the blob. Images must be under 1 MB and reachable at a \
                 public URL that the PDS can fetch."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Bluesky session expired — app-password sessions last about 2 hours. \
                 Reconnect by re-entering BLUESKY_HANDLE and BLUESKY_APP_PASSWORD."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Bluesky denied the request. Re-generate the app password with the \
                 required permissions, or check the handle is correct."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Bluesky post or handle not found. Check the handle spelling and that \
                 the post is still on the account."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Bluesky rejected the record. Posts are limited to 300 characters and \
                 4 images, and the handle must be a valid atproto identifier."
                    .into(),
            ),
            None => None,
        }
    }

    /// Account-level dashboard metrics: follower / following / post counts.
    async fn analytics(
        &self,
        access_token: &str,
        _internal_id: &str,
        _days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let profile = self.get_profile(access_token).await?;
        Ok(Self::profile_analytics(&profile))
    }

    /// Per-post dashboard metrics. Bluesky exposes no single-post stats
    /// endpoint, so this reuses the author-feed lookup.
    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        match self.post_metrics(access_token, platform_post_id).await? {
            Some(metrics) => Ok(Self::post_analytics_rows(&metrics)),
            None => Ok(vec![]),
        }
    }

    async fn generate_auth_url(
        &self,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        Err(ProviderError::Auth(
            "Bluesky uses app passwords instead of OAuth. \
             The agent will guide you through setting BLUESKY_HANDLE and \
             BLUESKY_APP_PASSWORD in your .env file."
                .into(),
        ))
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        // Auto-connect: create session and return token
        let session_jwt = self.create_session().await?;
        let did = self.resolve_handle().await?;

        Ok(AuthToken {
            access_token: session_jwt,
            refresh_token: None,
            expires_in: Some(7200), // 2 hours
            provider_user_id: did,
            name: self.handle.clone(),
            username: self.handle.clone(),
            picture: None,
        })
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        // Bluesky sessions are short-lived; just create a new one
        self.exchange_code("", "", "").await
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let did = self.resolve_handle().await?;

        // Build the Bluesky post record
        let mut record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "text": post.content,
        });

        // Thread linking: Bluesky uses a "reply" field with the parent
        // post's URI + CID. We have the platform_post_id (which is the
        // URI) but not the CID — so we fetch it via getRecord.
        if let Some(ref reply_uri) = post.in_reply_to {
            // Fetch the parent post's CID from the Bluesky API
            if let Ok(parent_cid) = self.fetch_record_cid(reply_uri, access_token).await {
                record["reply"] = serde_json::json!({
                    "root": { "uri": reply_uri, "cid": parent_cid },
                    "parent": { "uri": reply_uri, "cid": parent_cid },
                });
            }
        }

        // Add embed if media present
        if !post.media.is_empty() {
            if let Ok(embed) = self.upload_and_embed(access_token, &post.media).await {
                record["embed"] = embed;
            }
        }

        let body = serde_json::json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "record": record,
        });

        let resp = self
            .http
            .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 200 {
            let uri = json["uri"].as_str().unwrap_or("").to_string();
            let post_id = json["cid"].as_str().unwrap_or("").to_string();
            let at_uri = format!(
                "https://bsky.app/profile/{}/post/{}",
                self.handle,
                uri.rsplit('/').next().unwrap_or("")
            );
            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: Some(at_uri),
                status: "published".into(),
            })
        } else {
            Err(ProviderError::Api(friendly_error(
                self.map_error(&json.to_string(), status.as_u16()),
                "Bluesky publish failed",
            )))
        }
    }

    /// Read the reply chain for a post via `app.bsky.feed.getPostThread`.
    /// `platform_post_id` is the AT URI (or a bare rkey, which is resolved to
    /// this handle's URI first).
    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let jwt = self.session_jwt(access_token).await?;
        let uri = if platform_post_id.starts_with("at://") {
            platform_post_id.to_string()
        } else {
            let did = self.resolve_handle().await?;
            format!("at://{did}/app.bsky.feed.post/{platform_post_id}")
        };

        let resp = self
            .http
            .get("https://bsky.social/xrpc/app.bsky.feed.getPostThread")
            .header("Authorization", format!("Bearer {jwt}"))
            .query(&[("uri", uri.as_str()), ("depth", "10")])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        self.check_status(status, &body, "getPostThread")?;
        let json = Self::parse_body(&body, "getPostThread")?;
        Ok(Self::parse_thread_replies(&json))
    }

    /// @-autocomplete over `app.bsky.actor.searchActors`.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let jwt = self.session_jwt(access_token).await?;
        let resp = self
            .http
            .get("https://bsky.social/xrpc/app.bsky.actor.searchActors")
            .header("Authorization", format!("Bearer {jwt}"))
            .query(&[("q", query), ("limit", "25")])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        self.check_status(status, &body, "searchActors")?;
        Ok(Self::parse_actor_search(&Self::parse_body(
            &body,
            "searchActors",
        )?))
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        // Use the passed JWT directly, or create a fresh session if expired
        let jwt = if access_token.is_empty() {
            self.create_session().await?
        } else {
            access_token.to_string()
        };
        let did = self.resolve_handle().await?;

        let resp = self
            .http
            .get("https://bsky.social/xrpc/app.bsky.feed.getAuthorFeed")
            .header("Authorization", format!("Bearer {jwt}"))
            .query(&[("actor", &did), ("limit", &limit.to_string())])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let feed = json["feed"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();

        let mut posts = Vec::new();
        for item in &feed {
            // The author feed mixes three item kinds: the author's own posts,
            // reposts, and reply-context posts. Only the author's OWN posts
            // belong in the feed: a repost's `post.author` is the ORIGINAL
            // author, so importing every item showed strangers' posts under
            // the founder's account - the exact "hallucinated feed" report.
            if item
                .get("reason")
                .is_some_and(|r| r["$type"].as_str() == Some("app.bsky.feed.defs#reasonRepost"))
            {
                continue;
            }
            let post = &item["post"];
            let record = &post["record"];

            // A reply to someone else is also not the author's own content.
            if record["reply"].is_object() {
                continue;
            }

            let cid = post["cid"].as_str().unwrap_or("").to_string();
            if cid.is_empty() {
                continue;
            }

            let text = record["text"].as_str().unwrap_or("").to_string();
            // A missing/unparseable date must not fabricate a "posted just
            // now" timestamp - that reads as a fake post. Skip the item
            // instead; the feed only ever shows posts with a real date.
            let created_at = match record["createdAt"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            {
                Some(dt) => dt.with_timezone(&chrono::Utc),
                None => continue,
            };

            let author = &post["author"];
            let author_name = author["displayName"].as_str().map(String::from);
            let author_handle = author["handle"].as_str().map(String::from);
            let uri = post["uri"].as_str().map(String::from);

            let mut media = Vec::new();
            if let Some(embed) = post.get("embed") {
                let embed_type = embed["$type"].as_str().unwrap_or("");
                if embed_type == "app.bsky.embed.video" || embed_type == "app.bsky.embed.video#view"
                {
                    // Bluesky video — resolve blob reference to a playable URL
                    if let Some(video) = embed.get("video") {
                        let mime = video["mimeType"].as_str().unwrap_or("video/mp4");
                        // Try direct playlist URL first (video.playlist)
                        if let Some(playlist) = video["playlist"].as_str() {
                            media.push(MediaAttachment {
                                url: playlist.to_string(),
                                mime_type: mime.to_string(),
                                alt: None,
                                poster_url: None,
                            });
                        } else if let Some(cid) = video["ref"]["$link"]
                            .as_str()
                            .or_else(|| video["ref"]["cid"].as_str())
                        {
                            // Construct blob URL from DID + CID
                            let did = post["author"]["did"].as_str().unwrap_or("");
                            let blob_url = format!(
                                "https://bsky.social/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}"
                            );
                            media.push(MediaAttachment {
                                url: blob_url,
                                mime_type: mime.to_string(),
                                alt: None,
                                poster_url: None,
                            });
                        }
                    }
                } else if let Some(images) = embed["images"].as_array() {
                    for img in images {
                        if let Some(url) =
                            img["fullsize"].as_str().or_else(|| img["thumb"].as_str())
                        {
                            media.push(MediaAttachment {
                                url: url.to_string(),
                                mime_type: "image/jpeg".to_string(),
                                alt: img["alt"].as_str().map(String::from),
                                poster_url: None,
                            });
                        }
                    }
                } else if let Some(external) = embed.get("external") {
                    // External link embed — use thumbnail if available
                    if let Some(thumb) = external["thumb"].as_str() {
                        media.push(MediaAttachment {
                            url: thumb.to_string(),
                            mime_type: "image/jpeg".to_string(),
                            alt: None,
                            poster_url: None,
                        });
                    }
                }
            }

            let author_avatar = author["avatar"].as_str().map(String::from);

            posts.push(ExternalPostData {
                platform_post_id: cid,
                text,
                author_name,
                author_handle,
                author_avatar,
                created_at,
                url: uri,
                media,
                metadata: Some(post.clone()),
            });
        }

        Ok(posts)
    }

    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        self.post_metrics(access_token, platform_post_id).await
    }

    async fn reply_to_comment(
        &self,
        access_token: &str,
        comment_id: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let body = serde_json::json!({
            "collection": "app.bsky.feed.post",
            "repo": self.resolve_handle().await?,
            "record": {
                "$type": "app.bsky.feed.post",
                "text": post.content,
                "createdAt": chrono::Utc::now().to_rfc3339(),
                "reply": {
                    "root": {
                        "uri": comment_id,
                        "cid": ""
                    },
                    "parent": {
                        "uri": comment_id,
                        "cid": ""
                    }
                }
            }
        });
        let resp = self
            .http
            .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status == 200 {
            let uri = json["uri"].as_str().unwrap_or("").to_string();
            let post_id = uri.rsplit('/').next().unwrap_or("").to_string();
            Ok(PublishResult {
                platform_post_id: post_id.clone(),
                platform_post_url: Some(format!(
                    "https://bsky.app/profile/{}/post/{}",
                    self.handle, post_id
                )),
                status: "published".into(),
            })
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Bluesky reply failed")
                .to_string();
            Err(ProviderError::Api(friendly_error(
                self.map_error(&msg, status.as_u16()),
                &msg,
            )))
        }
    }
}

impl BlueskyProvider {
    async fn upload_and_embed(
        &self,
        access_token: &str,
        media: &[MediaAttachment],
    ) -> Result<serde_json::Value, ProviderError> {
        // EVERY image, not just the first.
        //
        // The `return` was INSIDE the loop, so only the first image was
        // ever embedded - with no error anywhere. The server ACCEPTS
        // four images ("Bluesky allows max 4 images") and the composer
        // offers four, so the founder attached three photos, every
        // check passed, and the post went out with ONE photo.
        //
        // Bluesky's `app.bsky.embed.images` takes an `images` array of
        // up to four, each with its own alt text.
        let mut images: Vec<serde_json::Value> = Vec::new();
        for item in media {
            if !item.mime_type.starts_with("image/") {
                // Not an image: Bluesky's image embed cannot carry it.
                continue;
            }
            // Download image and upload to Bluesky blob store
            let img_resp = self.http.get(&item.url).send().await?;
            let img_bytes = img_resp.bytes().await?;
            let mime = &item.mime_type;

            let blob_resp = self
                .http
                .post("https://bsky.social/xrpc/com.atproto.repo.uploadBlob")
                .header("Authorization", format!("Bearer {access_token}"))
                .header("Content-Type", mime)
                .body(img_bytes)
                .send()
                .await?;

            let blob_json: serde_json::Value = blob_resp.json().await?;
            let blob_ref = &blob_json["blob"];

            images.push(serde_json::json!({
                "alt": item.alt.as_deref().unwrap_or(""),
                "image": blob_ref,
            }));
        }

        if images.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        Ok(serde_json::json!({
            "$type": "app.bsky.embed.images",
            "images": images,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    fn provider() -> BlueskyProvider {
        BlueskyProvider::new(&test_config())
    }

    #[test]
    fn identifier_is_bluesky() {
        assert_eq!(provider().identifier(), "bluesky");
    }

    // ── B2c: profile analytics fixtures ──────────────────────

    #[test]
    fn profile_analytics_reads_all_three_counts() {
        let fixture = serde_json::json!({
            "handle": "test.bsky.social",
            "followersCount": 1200,
            "followsCount": 340,
            "postsCount": 87
        });
        let rows = BlueskyProvider::profile_analytics(&fixture);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Followers", "Following", "Posts"]);
        assert_eq!(rows[0].data[0].total, "1200");
        assert!(!rows[0].data[0].date.is_empty());
    }

    #[test]
    fn profile_analytics_empty_for_error_body() {
        let fixture = serde_json::json!({ "error": "Profile not found" });
        assert!(BlueskyProvider::profile_analytics(&fixture).is_empty());
    }

    // ── B2c: post metrics fixtures ───────────────────────────

    fn feed_fixture() -> serde_json::Value {
        serde_json::json!({
            "feed": [
                { "post": { "cid": "bafy1", "likeCount": 1, "repostCount": 0, "replyCount": 0, "quoteCount": 0 } },
                { "post": { "cid": "bafy2", "likeCount": 42, "repostCount": 8, "replyCount": 3, "quoteCount": 1 } }
            ]
        })
    }

    #[test]
    fn post_metrics_from_feed_matches_by_cid() {
        let metrics = BlueskyProvider::post_metrics_from_feed(&feed_fixture(), "bafy2").unwrap();
        assert_eq!(metrics["likeCount"], 42);
        assert_eq!(metrics["repostCount"], 8);
        assert_eq!(metrics["replyCount"], 3);
        assert_eq!(metrics["quoteCount"], 1);
    }

    #[test]
    fn post_metrics_from_feed_returns_none_when_post_not_on_page() {
        assert!(BlueskyProvider::post_metrics_from_feed(&feed_fixture(), "bafy-missing").is_none());
    }

    #[test]
    fn post_metrics_from_feed_returns_none_for_empty_feed() {
        let fixture = serde_json::json!({ "feed": [] });
        assert!(BlueskyProvider::post_metrics_from_feed(&fixture, "bafy1").is_none());
    }

    #[test]
    fn post_metrics_feed_into_engagement_data_matches_parser_arm() {
        let metrics = BlueskyProvider::post_metrics_from_feed(&feed_fixture(), "bafy2").unwrap();
        let e = crate::social::parse_engagement_data("bluesky", metrics);
        assert_eq!((e.likes, e.reposts, e.replies, e.quotes), (42, 8, 3, 1));
    }

    #[test]
    fn post_analytics_rows_covers_all_four_counters() {
        let metrics = serde_json::json!({
            "likeCount": 5, "repostCount": 2, "replyCount": 1, "quoteCount": 0
        });
        let rows = BlueskyProvider::post_analytics_rows(&metrics);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Likes", "Reposts", "Replies", "Quotes"]);
        assert_eq!(rows[0].data[0].total, "5");
    }

    // ── B2c: map_error fixtures ─────────────────────────────

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let msg = provider().map_error("Too Many Requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_duplicate_post() {
        let msg = provider().map_error("duplicate record", 400).unwrap();
        assert!(msg.contains("duplicate"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_blob() {
        let msg = provider()
            .map_error("BlobTooLarge: blob is too large", 400)
            .unwrap();
        assert!(msg.contains("blob"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_session_on_401() {
        let msg = provider().map_error("Expired token", 401).unwrap();
        assert!(msg.contains("expired"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_forbidden_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("denied"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        assert!(provider().map_error("Internal Server Error", 503).is_none());
    }

    #[test]
    fn check_status_passes_through_2xx() {
        assert!(provider().check_status(200, "{}", "getProfile").is_ok());
    }

    #[test]
    fn check_status_classifies_401_as_a_token_expiry() {
        // A 401 is the ONE error the scheduler refreshes and retries. It
        // used to arrive as a plain API error on the providers that
        // hand-rolled their mapping — so the scheduler gave up on the
        // first try on some platforms and retried on others, for the
        // same failure.
        let err = provider()
            .check_status(401, "Expired token", "getProfile")
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::TokenExpired),
            "a 401 must be a token expiry, got {err:?}"
        );
    }

    #[test]
    fn check_status_keeps_the_raw_text_on_an_ordinary_failure() {
        let err = provider()
            .check_status(500, "Internal Server Error", "getProfile")
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Internal Server Error"), "got: {msg}");
    }

    #[test]
    fn check_status_keeps_the_raw_text_beside_the_friendly_copy() {
        let err = provider()
            .check_status(403, "not permitted", "getProfile")
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("denied"), "got: {msg}");
        assert!(msg.contains("raw:"), "raw text must be preserved: {msg}");
    }

    // ── B3: thread replies + actor search ────────────────────

    #[test]
    fn parse_should_nest_bluesky_replies_under_their_parent() {
        let raw = serde_json::json!({ "thread": { "reply": [
            { "post": { "uri": "at://d/app.bsky.feed.post/1",
                       "author": { "did": "did:plc:1", "handle": "ada.bsky.social",
                                   "displayName": "Ada", "avatar": "https://img/a.jpg" },
                       "record": { "text": "nice post", "createdAt": "2026-09-28T10:00:00Z" },
                       "likeCount": 7 } },
            { "post": { "uri": "at://d/app.bsky.feed.post/2",
                       "author": { "did": "did:plc:2", "handle": "bob.bsky.social" },
                       "record": { "text": "agreed", "createdAt": "2026-09-28T11:00:00Z",
                                   "reply": { "parent": { "uri": "at://d/app.bsky.feed.post/1" } } },
                       "likeCount": 1 } }
        ]}});
        let comments = BlueskyProvider::parse_thread_replies(&raw);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "at://d/app.bsky.feed.post/1");
        assert_eq!(comments[0].author_name.as_deref(), Some("Ada"));
        assert_eq!(
            comments[0].author_avatar.as_deref(),
            Some("https://img/a.jpg")
        );
        assert_eq!(comments[0].like_count, 7);
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].id, "at://d/app.bsky.feed.post/2");
    }

    #[test]
    fn parse_should_fall_back_to_handle_when_bluesky_display_name_absent() {
        let raw = serde_json::json!({ "thread": { "reply": [
            { "post": { "uri": "at://d/1", "author": { "handle": "bob.bsky.social" }, "record": { "text": "hi" } } }
        ]}});
        let comments = BlueskyProvider::parse_thread_replies(&raw);
        assert_eq!(comments[0].author_name.as_deref(), Some("bob.bsky.social"));
    }

    #[test]
    fn parse_should_read_no_bluesky_replies_from_empty_thread() {
        assert!(
            BlueskyProvider::parse_thread_replies(&serde_json::json!({ "thread": {} })).is_empty()
        );
    }

    #[test]
    fn parse_should_map_bluesky_actor_search_into_mention_results() {
        let raw = serde_json::json!({ "actors": [
            { "did": "did:plc:1", "handle": "ada.bsky.social", "displayName": "Ada", "avatar": "https://img/a.jpg" }
        ]});
        let mentions = BlueskyProvider::parse_actor_search(&raw);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "did:plc:1");
        assert_eq!(mentions[0].label, "ada.bsky.social");
        assert_eq!(mentions[0].image.as_deref(), Some("https://img/a.jpg"));
    }

    #[test]
    fn parse_should_read_no_bluesky_mentions_from_empty_search() {
        assert!(
            BlueskyProvider::parse_actor_search(&serde_json::json!({ "actors": [] })).is_empty()
        );
    }
}
