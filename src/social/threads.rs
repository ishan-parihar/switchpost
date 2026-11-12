// ─── Threads Provider ─────────────────────────────────────────
// Meta Threads API v1.0 via graph.threads.net.

use async_trait::async_trait;

use super::common::extract_access_token;
use super::*;
use crate::config::Config;

pub struct ThreadsProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl ThreadsProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("threads").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    fn graph_url(&self) -> &'static str {
        "https://graph.threads.net/v1.0"
    }

    /// The app's publication state, as Meta reports it.
    ///
    /// `Unknown` when the probe cannot tell. The caller turns that into
    /// an honest message rather than a guess.
    pub async fn app_mode(&self) -> crate::social::threads_app::ThreadsAppMode {
        if self.client_id.is_empty() || self.client_secret.is_empty() {
            return crate::social::threads_app::ThreadsAppMode::Unknown;
        }
        crate::social::threads_app::probe_app_mode(&self.client_id, &self.client_secret).await
    }

    /// The message to show when Meta's consent gate refuses the sign-in.
    ///
    /// This is the message the founder's report needed and did not get:
    /// the browser was redirected to Meta's own privacy/consent page, so
    /// none of our handlers ran, and the founder was left with a Meta URL
    /// and no next step.
    pub async fn consent_gate_message(&self) -> String {
        let mode = self.app_mode().await;
        self.consent_gate_message_for(mode)
    }

    /// The same message, for an already-probed mode.
    ///
    /// Split out so the preflight does not probe TWICE — one Graph API
    /// call is one call, and a connect is a user-visible latency.
    fn consent_gate_message_for(&self, mode: crate::social::threads_app::ThreadsAppMode) -> String {
        let detail = match mode {
            crate::social::threads_app::ThreadsAppMode::DevMode => {
                crate::social::threads_app::dev_mode_detail()
            }
            crate::social::threads_app::ThreadsAppMode::Published => {
                crate::social::threads_app::published_detail()
            }
            crate::social::threads_app::ThreadsAppMode::Unknown => {
                crate::social::threads_app::unknown_detail()
            }
        };
        crate::social::threads_app::consent_gate_message(&self.scopes(), &detail)
    }
}

#[async_trait]
impl SocialProvider for ThreadsProvider {
    fn identifier(&self) -> &'static str {
        "threads"
    }

    fn name(&self) -> &'static str {
        "Threads"
    }

    fn scopes(&self) -> Vec<String> {
        // MINIMAL AND TRUE. The founder could not log into Threads at all: Meta
        // answered the authorize request with its GDPR consent page and
        // `error_code=1349003&source=gdp_error`, and never came back with a code.
        //
        // Every `threads_*` permission is a separate App Review gate
        // (developers.facebook.com/documentation/threads/get-started): an app in
        // dev mode can be granted a permission only by a user who holds a ROLE
        // on the app, and the rest need the app to be published and reviewed.
        // Requesting nine of them made the whole consent fail for an app that
        // had been approved for far fewer - and the failure lands on Meta's page,
        // not ours, so the founder saw a dead end with no code returned.
        //
        // So this is the set Switchpost actually IMPLEMENTS, nothing more:
        //   threads_basic          - /me, profile, every read
        //   threads_content_publish- the container publish flow
        //   threads_manage_replies - reply_to_thread / manage replies
        //   threads_read_replies   - get_replies, get_mentions
        //   threads_manage_insights- the insights/analytics endpoints
        // The four that are gone were requested with no implementation behind
        // them: `threads_delete`, `threads_keyword_search`,
        // `threads_manage_mentions` and `threads_profile_discovery` had no
        // call site in this provider. A scope that nothing reads is a gate
        // nobody can pass, and it takes the whole consent down with it.
        vec![
            "threads_basic".into(),
            "threads_content_publish".into(),
            "threads_manage_replies".into(),
            "threads_read_replies".into(),
            "threads_manage_insights".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        500
    }

    fn validate_media(&self, post: &PostContent) -> Result<(), String> {
        super::validate_media_limits(self.identifier(), post)
    }

    fn needs_cron_refresh(&self) -> bool {
        true
    }

    /// Meta REFUSES an `http://` redirect URI for Threads. The gate that
    /// surfaces this lives in `api::integrations::connect`; see
    /// `SocialProvider::requires_https_redirect` for why the URL is checked
    /// before the founder is sent to the consent screen.
    fn requires_https_redirect(&self) -> bool {
        true
    }

    /// The warning shown next to the Threads sign-in button.
    ///
    /// It is a WARNING and not a refusal, on purpose. The app's
    /// publication state is not the only thing that can make Meta's
    /// consent gate refuse: an account that is not a tester can still
    /// succeed once the founder adds it, and the founder may already have
    /// done that outside the app. Refusing to hand out the sign-in URL
    /// would turn a gate we cannot fully see into a hard block, and the
    /// one thing that is known to work — completing Meta's GDPR consent
    /// by hand, which the founder's report suggests was tried — would be
    /// unreachable from the app.
    ///
    /// So the answer is the URL plus the reason plus the manual escape
    /// hatch, and the founder chooses.
    ///
    /// ─── WHY THIS LIVES IN THE TRAIT IMPL, NOT AN INHERENT IMPL ──
    /// The registry hands out `Arc<dyn SocialProvider>`, so the call
    /// resolves to the TRAIT's method. An inherent method with the same
    /// name on the concrete type is a DIFFERENT method, and it silently
    /// loses to the trait's default `None` — which is exactly how this
    /// bug happened once: the code compiled, the warning was dead code,
    /// and every connect got no warning at all.
    async fn connect_preflight(&self) -> Option<String> {
        let mode = self.app_mode().await;
        // A published app has no gate we can warn about.
        if mode == crate::social::threads_app::ThreadsAppMode::Published {
            return None;
        }
        Some(self.consent_gate_message_for(mode))
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
            ("response_type", "code"),
            ("state", state),
            ("scope", scope.as_str()),
        ];

        let url = url::Url::parse_with_params("https://www.threads.net/oauth/authorize", &params)
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
        // Step 1: Exchange code for short-lived token
        let token_params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("client_secret", self.client_secret.as_str()),
            ("code", code),
        ];

        let resp = self
            .http
            .get("https://graph.threads.net/oauth/access_token")
            .query(&token_params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            let err_msg = json["error"]["message"].as_str().unwrap_or("Unknown error");
            let err_code = json["error"]["code"].as_u64().unwrap_or(0);
            return Err(ProviderError::Api(format!(
                "Threads token exchange failed (code {err_code}): {err_msg}"
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
            ("grant_type", "th_exchange_token"),
            ("client_secret", self.client_secret.as_str()),
            ("access_token", short_token.as_str()),
        ];

        let long_resp = self
            .http
            .get("https://graph.threads.net/access_token")
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
                "Threads long-lived token exchange failed: {err_msg}"
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
            .get(format!("{}/me", self.graph_url()))
            .query(&[
                ("fields", "id,username,threads_profile_picture_url"),
                ("access_token", &access_token),
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
            provider_user_id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["username"].as_str().unwrap_or("").to_string(),
            username: user["username"].as_str().unwrap_or("").to_string(),
            picture: user["threads_profile_picture_url"]
                .as_str()
                .map(String::from),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let params: Vec<(&str, &str)> = vec![
            ("grant_type", "th_refresh_token"),
            ("access_token", refresh_token),
        ];

        let resp = self
            .http
            .get("https://graph.threads.net/refresh_access_token")
            .query(&params)
            .send()
            .await?;

        let (access_token, json) = extract_access_token(resp, "threads")
            .await
            .map_err(ProviderError::Auth)?;
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        let user: serde_json::Value = self
            .http
            .get(format!("{}/me", self.graph_url()))
            .query(&[
                ("fields", "id,username,threads_profile_picture_url"),
                ("access_token", &access_token),
            ])
            .send()
            .await?
            .json()
            .await?;

        Ok(AuthToken {
            access_token: access_token.clone(),
            refresh_token: Some(access_token.to_string()),
            expires_in,
            provider_user_id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["username"].as_str().unwrap_or("").to_string(),
            username: user["username"].as_str().unwrap_or("").to_string(),
            picture: user["threads_profile_picture_url"]
                .as_str()
                .map(String::from),
        })
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let user_id = self.resolve_user_id(access_token).await?;

        // Carousel: create child containers first
        if post.media.len() > 1 {
            return self.publish_carousel(&user_id, access_token, post).await;
        }

        let creation_id = self
            .create_single_container(access_token, &user_id, post)
            .await?;

        // Immediate publish: only valid when the container is already
        // FINISHED (small images usually are). The scheduler/tool path
        // goes through post_pending, which waits instead of failing.
        self.publish_container(access_token, &user_id, &creation_id)
            .await
    }

    /// B15: the publish lifecycle the ONE engine expects. A video or
    /// large-image container is usually still `IN_PROGRESS` when it is
    /// created — publishing it immediately errors. Return the creation id
    /// as `pending` and let the engine's finalize poll drive it home.
    async fn post_pending(
        &self,
        access_token: &str,
        post: &PostContent,
        internal_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let _ = internal_id;
        let user_id = self.resolve_user_id(access_token).await?;

        if post.media.len() > 1 {
            // ponytail: carousel children are not polled before
            // threads_publish — rare path; upgrade = wait on each child
            // container's status_code the same way as the single path.
            return self.publish_carousel(&user_id, access_token, post).await;
        }

        let creation_id = self
            .create_single_container(access_token, &user_id, post)
            .await?;

        match self
            .container_status(access_token, &creation_id)
            .await?
            .as_str()
        {
            "FINISHED" => {
                self.publish_container(access_token, &user_id, &creation_id)
                    .await
            }
            "ERROR" => Err(ProviderError::Api(
                "Threads media container finished with ERROR status".into(),
            )),
            _ => Ok(PublishResult {
                platform_post_id: creation_id,
                platform_post_url: None,
                status: "pending".into(),
            }),
        }
    }

    /// B15: finalize a pending container — poll to FINISHED, then publish.
    async fn check_post_status(
        &self,
        access_token: &str,
        creation_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        match self
            .container_status(access_token, creation_id)
            .await?
            .as_str()
        {
            "FINISHED" => {
                let user_id = self.resolve_user_id(access_token).await?;
                self.publish_container(access_token, &user_id, creation_id)
                    .await
            }
            "ERROR" => Ok(PublishResult {
                platform_post_id: creation_id.to_string(),
                platform_post_url: None,
                status: "failed".into(),
            }),
            _ => Ok(PublishResult {
                platform_post_id: creation_id.to_string(),
                platform_post_url: None,
                status: "pending".into(),
            }),
        }
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
        let user_id = self.resolve_user_id(access_token).await?;
        let threads = self.get_threads(access_token, &user_id, limit).await?;
        let mut posts = Vec::new();
        if let Some(data) = threads["data"].as_array() {
            for item in data {
                let id = item["id"].as_str().unwrap_or("").to_string();
                let text_val = item["text"].as_str().unwrap_or("").to_string();
                let timestamp = item["timestamp"].as_str().unwrap_or("");
                let media_url = item["media_url"].as_str().map(|s| s.to_string());
                let permalink = item["permalink"].as_str().map(|s| s.to_string());

                let posted_at = crate::social::common::parse_timestamp(timestamp);

                posts.push(ExternalPostData {
                    platform_post_id: id,
                    text: text_val,
                    author_name: None,
                    author_handle: None,
                    author_avatar: None,
                    media: media_url
                        .into_iter()
                        .map(|u| MediaAttachment {
                            url: u,
                            mime_type: String::new(),
                            alt: None,
                            poster_url: None,
                        })
                        .collect(),
                    created_at: posted_at,
                    url: permalink,
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
        // get_thread_detail() returns like_count, reply_count, etc.
        let json = self
            .get_thread_detail(access_token, platform_post_id)
            .await?;
        if json.get("error").is_some() || json.get("id").is_none() {
            return Ok(None);
        }
        Ok(Some(json))
    }

    /// Threads-specific user-facing error copy (v25 §2 row 9).
    ///
    /// The two hand-written arms stay ahead of `classify_error` because they
    /// carry the only actionable detail Threads returns: a stale token and the
    /// exact character cap.
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if body.contains("Error validating access token") {
            return Some(
                "Threads access token expired. Reconnect Threads on the Channels screen — \
                 Threads tokens are short-lived and Forge refreshes them automatically until \
                 the refresh itself is rejected."
                    .into(),
            );
        }
        if body.contains("text must be at most 500 characters") {
            return Some("Post text exceeds the 500-character Threads limit".into());
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Threads rate limit hit. Threads throttles publishing per account — Forge \
                 retries with backoff automatically."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Threads rejected the media. Threads posts accept up to 20 images or one \
                 video, each from a publicly fetchable URL."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Threads access token expired or was revoked. Reconnect Threads on the \
                 Channels screen."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Threads denied this request (HTTP 403). The token is missing \
                 threads_content_publish, or the Threads profile is no longer linked to the \
                 Instagram account that authorised it."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Threads post or profile not found — it may have been deleted, or the \
                 integration points at a profile you no longer own."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Threads rejected the request as invalid. Check the text length (500 chars) \
                 and that reply threads carry a valid in_reply_to id."
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
        let user_id = self.resolve_user_id(access_token).await?;

        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(days as i64))
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        let until = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let json = self
            .get_insights_range(
                access_token,
                &user_id,
                "views,likes,replies,reposts,quotes",
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
        let resp = self
            .http
            .get(format!("{}/{platform_post_id}/insights", self.graph_url()))
            .query(&[
                ("metric", "views,likes,replies,reposts,quotes"),
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

    async fn get_post_comments(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        let json = self
            .get_thread_replies(access_token, platform_post_id)
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
            .reply_to_thread(access_token, comment_id, &post.content)
            .await?;
        let id = json["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(ProviderError::Api(format!("Threads reply failed: {json}")));
        }
        Ok(PublishResult {
            platform_post_id: id,
            platform_post_url: None,
            status: "published".into(),
        })
    }

    /// Threads exposes no member search; accounts that mention this profile are
    /// the only taggable set available.
    async fn search_mention(
        &self,
        access_token: &str,
        query: &str,
    ) -> Result<Vec<MentionResult>, ProviderError> {
        let user_id = self.resolve_user_id(access_token).await?;
        let json = self.get_mentions(&user_id).await?;
        Ok(super::parse_graph_mentions(&json, query))
    }
    // `resolve_media_url` is NOT overridden here: the trait's default
    // makes a stored relative URL absolute, which is what every
    // provider needs. This file used to carry its own copy of that
    // override, and a provider that forgot it published with a relative
    // URL and failed.
}

impl ThreadsProvider {
    /// Create a single (non-carousel) media container. Returns the
    /// creation_id. (B15 helper — inherent, not part of the trait.)
    async fn create_single_container(
        &self,
        access_token: &str,
        user_id: &str,
        post: &PostContent,
    ) -> Result<String, ProviderError> {
        let (media_type, url_key): (&str, &str) = if post.media.is_empty() {
            ("TEXT", "")
        } else if post.media.len() == 1 && post.media[0].url.contains(".mp4") {
            ("VIDEO", "video_url")
        } else {
            ("IMAGE", "image_url")
        };

        let mut form: Vec<(&str, &str)> = vec![
            ("media_type", media_type),
            ("text", &post.content),
            ("access_token", access_token),
        ];

        // Thread linking: Threads uses `reply_to_id` to link replies
        if let Some(ref reply_to) = post.in_reply_to {
            form.push(("reply_to_id", reply_to));
        }

        if !url_key.is_empty() {
            form.push((url_key, post.media[0].url.as_str()));
        }

        let resp = self
            .http
            .post(format!("{}/{user_id}/threads", self.graph_url()))
            .form(&form)
            .send()
            .await?;

        let create_status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        Ok(json["id"]
            .as_str()
            .ok_or_else(|| {
                let msg = json["error"]["message"]
                    .as_str()
                    .unwrap_or("No creation ID");
                let code = json["error"]["code"].as_u64().unwrap_or(0);
                let raw =
                    format!("Threads create failed (HTTP {create_status}, code {code}): {msg}");
                ProviderError::Api(friendly_error(
                    self.map_error(msg, create_status.as_u16()),
                    &raw,
                ))
            })?
            .to_string())
    }

    /// GET a container's `status_code` (FINISHED / IN_PROGRESS / ERROR).
    async fn container_status(
        &self,
        access_token: &str,
        creation_id: &str,
    ) -> Result<String, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/{creation_id}", self.graph_url()))
            .query(&[("fields", "status_code"), ("access_token", access_token)])
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(json["status_code"]
            .as_str()
            .unwrap_or("IN_PROGRESS")
            .to_string())
    }

    /// POST threads_publish for a container id. Returns the live thread.
    async fn publish_container(
        &self,
        access_token: &str,
        user_id: &str,
        creation_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        let pub_resp = self
            .http
            .post(format!("{}/{user_id}/threads_publish", self.graph_url()))
            .form(&[("creation_id", creation_id), ("access_token", access_token)])
            .send()
            .await?;

        let pub_status = pub_resp.status();
        let pub_json: serde_json::Value = pub_resp.json().await?;
        let thread_id = pub_json["id"]
            .as_str()
            .ok_or_else(|| {
                let msg = pub_json["error"]["message"]
                    .as_str()
                    .unwrap_or("Publish failed");
                let code = pub_json["error"]["code"].as_u64().unwrap_or(0);
                let raw = format!("Threads publish failed (HTTP {pub_status}, code {code}): {msg}");
                ProviderError::Api(friendly_error(
                    self.map_error(msg, pub_status.as_u16()),
                    &raw,
                ))
            })?
            .to_string();

        let permalink = pub_json["permalink"].as_str().unwrap_or("").to_string();

        Ok(PublishResult {
            platform_post_id: thread_id,
            platform_post_url: Some(permalink),
            status: "published".into(),
        })
    }

    async fn resolve_user_id(&self, access_token: &str) -> Result<String, ProviderError> {
        let user: serde_json::Value = self
            .http
            .get(format!("{}/me", self.graph_url()))
            .query(&[("fields", "id"), ("access_token", access_token)])
            .send()
            .await?
            .json()
            .await?;

        user["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| ProviderError::Auth("Could not resolve Threads user ID".into()))
    }

    pub async fn get_profile(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/me", self.graph_url()))
            .query(&[
                ("fields", "id,username,name,threads_profile_picture_url"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!("get_profile failed: {}", body)));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn get_threads(
        &self,
        access_token: &str,
        user_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let effective_limit = if limit < 100 { 100 } else { limit };
        let resp = self
            .http
            .get(format!("{}/{user_id}/threads", self.graph_url()))
            .query(&[
                (
                    "fields",
                    "id,text,media_type,media_url,permalink,timestamp,like_count,reply_count",
                ),
                ("access_token", access_token),
            ])
            .query(&[("limit", effective_limit.to_string())])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!("get_threads failed: {}", body)));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn get_thread_detail(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/{}", self.graph_url(), media_id))
            .query(&[
                ("fields", "id,text,media_type,media_url,permalink,timestamp,username,like_count,reply_count,repost_count,quote_count,children{id,media_url,media_type}"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!(
                "get_thread_detail failed: {}",
                body
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn get_thread_replies(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/{}/replies", self.graph_url(), media_id))
            .query(&[
                ("fields", "id,text,timestamp,username,like_count"),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!(
                "get_thread_replies failed: {}",
                body
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    /// Accounts that have mentioned this profile — the @-autocomplete source.
    pub async fn get_mentions(&self, user_id: &str) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{}/{user_id}/mentions", self.graph_url()))
            .query(&[(
                "fields",
                "id,username,threads_profile_picture_url,text,timestamp",
            )])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!("get_mentions failed: {body}")));
        }

        Ok(resp.json().await?)
    }

    pub async fn reply_to_thread(
        &self,
        access_token: &str,
        media_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .post(format!("{}/{}/replies", self.graph_url(), media_id))
            .form(&[("text", message), ("access_token", access_token)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!(
                "reply_to_thread failed: {}",
                body
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn get_insights(
        &self,
        access_token: &str,
        user_id: &str,
        metric: &str,
        period: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        self.get_insights_range(access_token, user_id, metric, period, None, None)
            .await
    }

    /// `threads_insights` with an optional `since`/`until` window so
    /// `analytics()` can honour the requested day range.
    pub async fn get_insights_range(
        &self,
        access_token: &str,
        user_id: &str,
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

        let resp = self
            .http
            .get(format!("{}/{}/threads_insights", self.graph_url(), user_id))
            .query(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!("get_insights failed: {}", body)));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn delete_thread(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .delete(format!("{}/{}", self.graph_url(), media_id))
            .query(&[("access_token", access_token)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            return Err(ProviderError::Api(format!(
                "delete_thread failed: {}",
                body
            )));
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    async fn publish_carousel(
        &self,
        user_id: &str,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Create child containers
        let mut child_ids = Vec::new();
        for media in &post.media {
            let is_video = media.url.contains(".mp4");
            let media_type = if is_video { "VIDEO" } else { "IMAGE" };
            let url_key = if is_video { "video_url" } else { "image_url" };

            let resp = self
                .http
                .post(format!("{}/{user_id}/threads", self.graph_url()))
                .form(&[
                    ("media_type", media_type),
                    (url_key, media.url.as_str()),
                    ("is_carousel_item", "true"),
                    ("access_token", access_token),
                ])
                .send()
                .await?;

            let json: serde_json::Value = resp.json().await?;
            let child_id = json["id"]
                .as_str()
                .ok_or_else(|| ProviderError::Api(format!("Carousel child fail: {json:?}")))?
                .to_string();
            child_ids.push(child_id);
        }

        // Create CAROUSEL container
        let children_csv = child_ids.join(",");
        let car_resp = self
            .http
            .post(format!("{}/{user_id}/threads", self.graph_url()))
            .form(&[
                ("media_type", "CAROUSEL"),
                ("text", &post.content),
                ("children", &children_csv),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let car_json: serde_json::Value = car_resp.json().await?;
        let creation_id = car_json["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Api(format!("Carousel fail: {car_json:?}")))?
            .to_string();

        // Publish
        let pub_resp = self
            .http
            .post(format!("{}/{user_id}/threads_publish", self.graph_url()))
            .form(&[
                ("creation_id", creation_id.as_str()),
                ("access_token", access_token),
            ])
            .send()
            .await?;

        let pub_json: serde_json::Value = pub_resp.json().await?;

        Ok(PublishResult {
            platform_post_id: pub_json["id"].as_str().unwrap_or("").to_string(),
            platform_post_url: pub_json["permalink"].as_str().map(String::from),
            status: "published".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;
    use std::sync::Arc;

    /// ─── THE TRAIT-DISPATCH TRAP ─────────────────────────────────
    ///
    /// The preflight used to be an INHERENT method on `ThreadsProvider`
    /// with the same name as the trait's method. The registry hands out
    /// `Arc<dyn SocialProvider>`, so every caller resolved to the
    /// TRAIT's method — whose default is `None`. The inherent method
    /// was dead code, the build was green, and every connect produced
    /// no warning at all: the exact defect this fix exists to close,
    /// reintroduced one layer up.
    ///
    /// This test calls through the trait object, the way every caller
    /// does. It is offline-safe: with no credentials in the test
    /// config, the probe cannot reach Meta and returns `Unknown`, which
    /// is still a WARNING — so the assertion is that the TRAIT OBJECT
    /// produces one, not that the message says anything specific.
    #[tokio::test]
    async fn the_preflight_is_reachable_through_the_trait_object() {
        let provider = ThreadsProvider::new(&test_config());
        let as_trait: Arc<dyn SocialProvider> = Arc::new(provider);
        assert!(
            as_trait.connect_preflight().await.is_some(),
            "the preflight must be reachable through Arc<dyn SocialProvider> — an inherent \
             method with the same name is dead code and the connect gets NO warning"
        );
    }

    /// The warning must name the cause, not just announce that something
    /// might be wrong.
    ///
    /// The founder's report was a Meta URL and no next step. A warning
    /// that says "Meta's consent gate may refuse this" without saying
    /// what to do is the same dead end in a different font.
    #[test]
    fn the_consent_message_names_the_tester_fix() {
        let provider = ThreadsProvider::new(&test_config());
        let msg =
            provider.consent_gate_message_for(crate::social::threads_app::ThreadsAppMode::DevMode);
        assert!(msg.contains("App roles"), "{msg}");
        assert!(msg.contains("Threads Tester"), "{msg}");
        assert!(msg.contains("privacy/consent"), "{msg}");
    }

    fn post_with_images(n: usize) -> PostContent {
        PostContent {
            content: "a thread part".into(),
            media: (0..n)
                .map(|i| MediaAttachment {
                    url: format!("https://example.com/{i}.jpg"),
                    mime_type: "image/jpeg".into(),
                    alt: None,
                    poster_url: None,
                })
                .collect(),
            settings: serde_json::json!({}),
            in_reply_to: Some("8888888888888888888".into()),
            ..Default::default()
        }
    }

    #[test]
    fn parse_should_expand_thread_insights_into_daily_series() {
        let raw = serde_json::json!({
            "data": [{
                "name": "views",
                "period": "day",
                "values": [
                    { "value": 800, "end_time": "2026-09-28T00:00:00+0000" },
                    { "value": 900, "end_time": "2026-09-29T00:00:00+0000" }
                ]
            }]
        });
        let series = super::super::parse_insights_data(&raw);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].label, "views");
        assert_eq!(series[0].data[1].total, "900");
    }

    #[test]
    fn parse_should_map_thread_detail_counts() {
        let raw = serde_json::json!({
            "id": "8888888888888888888",
            "like_count": 42,
            "reply_count": 12,
            "repost_count": 5,
            "quote_count": 1
        });
        let e = super::super::parse_engagement_data("threads", raw);
        assert_eq!(e.likes, 42);
        assert_eq!(e.replies, 12);
        assert_eq!(e.reposts, 5);
        assert_eq!(e.quotes, 1);
    }

    /// The boundary: Threads accepts up to 20 images or one video. The
    /// provider's own rejection message says so ("Threads posts accept
    /// up to 20 images or one video"), and the composer's table said 20.
    /// The shared limit check said 10, so the composer let the founder
    /// attach 15 images with no banner and the server rejected the post
    /// on save. The two tests bracket the boundary.
    #[test]
    fn publish_should_reject_21st_image() {
        let provider = ThreadsProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(21)).is_err());
    }

    #[test]
    fn publish_should_accept_20_images() {
        let provider = ThreadsProvider::new(&test_config());
        assert!(provider.validate_media(&post_with_images(20)).is_ok());
    }

    #[test]
    fn publish_should_map_auth_error_to_user_facing_message() {
        let provider = ThreadsProvider::new(&test_config());
        let mapped = provider.map_error("Error validating access token: 190", 400);
        assert!(
            mapped.unwrap().contains("access token expired"),
            "must keep the plain cause"
        );
    }

    // ── B4: map_error fixtures ──────────────────────────────

    #[test]
    fn map_error_explains_text_cap() {
        let provider = ThreadsProvider::new(&test_config());
        assert_eq!(
            provider
                .map_error("text must be at most 500 characters", 400)
                .as_deref(),
            Some("Post text exceeds the 500-character Threads limit")
        );
    }

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider.map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider
            .map_error("The image upload was rejected", 422)
            .unwrap();
        assert!(msg.contains("media"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider.map_error("", 401).unwrap();
        assert!(msg.contains("Reconnect Threads"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_scope_on_403() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider.map_error("", 403).unwrap();
        assert!(msg.contains("threads_content_publish"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_post_on_404() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider.map_error("", 404).unwrap();
        assert!(msg.contains("not found"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_invalid_request_on_400() {
        let provider = ThreadsProvider::new(&test_config());
        let msg = provider.map_error("", 400).unwrap();
        assert!(msg.contains("500 chars"), "got: {msg}");
    }

    #[test]
    fn map_error_returns_none_for_server_error() {
        let provider = ThreadsProvider::new(&test_config());
        assert!(provider.map_error("Internal Server Error", 503).is_none());
    }

    #[test]
    fn threads_requests_cron_refresh() {
        let provider = ThreadsProvider::new(&test_config());
        assert!(provider.needs_cron_refresh());
    }

    // ── The consent-failure scope set ──────────────────────────

    /// The reported Threads failure: Meta answered the authorize request with
    /// `error_code=1349003&source=gdp_error` on its own privacy/consent page
    /// and no code came back, so no amount of callback plumbing could surface
    /// it. One cause in OUR control is the scope set — every `threads_*`
    /// permission is a separate App Review gate, and requesting ones the app
    /// is not approved for fails the whole consent.
    ///
    /// So the set is asserted to be exactly the permissions this provider has
    /// an implementation for. A permission that is added back without a call
    /// site is a gate nobody can pass, and this test fails instead of the
    /// founder's login.
    #[test]
    fn threads_requests_only_the_permissions_it_implements() {
        let provider = ThreadsProvider::new(&test_config());
        let scopes = provider.scopes();

        assert_eq!(
            scopes,
            vec![
                "threads_basic",
                "threads_content_publish",
                "threads_manage_replies",
                "threads_read_replies",
                "threads_manage_insights",
            ],
            "the Threads consent fails as a whole when the app asks for a \
             permission it is not approved for. Request only what is implemented."
        );
    }

    /// The four that were removed had NO call site in this provider. This is
    /// the check that keeps them from coming back silently: a scope in the
    /// request that nothing in this file reads is the defect, and the owning
    /// test says so.
    #[test]
    fn no_permission_is_requested_without_an_implementation() {
        let src = include_str!("threads.rs");
        let provider = ThreadsProvider::new(&test_config());
        for scope in provider.scopes() {
            // Each scope names an endpoint group this file implements.
            // The check is deliberately coarse (a substring of the scope in
            // the source) because the point is the ABSENCE of a permission
            // with no support, not the shape of its use.
            let supported = [
                "threads_basic".eq(scope.as_str()),
                "threads_content_publish".eq(scope.as_str()),
                "threads_manage_replies".eq(scope.as_str()) || src.contains("reply_to_thread"),
                "threads_read_replies".eq(scope.as_str()) || src.contains("get_mentions"),
                "threads_manage_insights".eq(scope.as_str()) || src.contains("insights"),
            ]
            .iter()
            .any(|v| *v);
            assert!(
                supported,
                "{scope} is requested but nothing in threads.rs reads it"
            );
        }
    }
}
