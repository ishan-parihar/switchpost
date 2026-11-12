// ─── Pinterest Provider ───────────────────────────────────────
// Pinterest API v5 OAuth2. Supports boards, pins, and video pins.

use async_trait::async_trait;

use super::*;
use super::common::extract_access_token;
use crate::config::Config;
use reqwest::StatusCode;
use serde_json::{json, Value};

pub struct PinterestProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl PinterestProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) =
            config.provider_credentials("pinterest").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    pub async fn get_user_account(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://api.pinterest.com/v5/user_account")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_board(
        &self,
        access_token: &str,
        board_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.pinterest.com/v5/boards/{board_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_board_pins(
        &self,
        access_token: &str,
        board_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.pinterest.com/v5/boards/{board_id}/pins");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[("page_size", &limit.clamp(1, 100).to_string())])
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_pin(
        &self,
        access_token: &str,
        pin_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.pinterest.com/v5/pins/{pin_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_board_analytics(
        &self,
        access_token: &str,
        board_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.pinterest.com/v5/boards/{board_id}/analytics");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[("start_date", start_date), ("end_date", end_date)])
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_pin_analytics(
        &self,
        access_token: &str,
        pin_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("https://api.pinterest.com/v5/pins/{pin_id}/analytics");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[("start_date", start_date), ("end_date", end_date)])
            .send()
            .await?;

        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Pinterest API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Board to report on: the integration's bound board when the caller knows
    /// it, otherwise the user's first board.
    async fn resolve_board_id(
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
            .map(|b| b.id.clone()))
    }

    /// Search pins by keyword using Pinterest API v5
    pub async fn search_pins(
        &self,
        access_token: &str,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Value, ProviderError> {
        let max = limit.unwrap_or(25).min(100);
        let encoded_query: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = format!(
            "https://api.pinterest.com/v5/pins/search?query={}&page_size={}",
            encoded_query, max
        );
        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        if status.is_success() {
            Ok(v)
        } else if status == StatusCode::UNAUTHORIZED {
            Err(ProviderError::TokenExpired)
        } else if status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth(
                v["message"].as_str().unwrap_or("forbidden").into(),
            ))
        } else {
            Err(ProviderError::Api(
                v["message"].as_str().unwrap_or(&text).into(),
            ))
        }
    }
}

/// Build the `(start_date, end_date)` window Pinterest analytics accepts.
/// v5 rejects a range wider than 90 days, so a longer dashboard request is
/// clamped here instead of erroring at the API.
fn analytics_window(days: u32) -> (String, String) {
    let days = i64::from(days.clamp(1, 90));
    let end = chrono::Utc::now().date_naive();
    let start = end - chrono::Duration::days(days - 1);
    (
        start.format("%Y-%m-%d").to_string(),
        end.format("%Y-%m-%d").to_string(),
    )
}

/// Pinterest analytics metric labels paired with the keys the v5 API uses.
const PIN_METRICS: [(&str, &str); 4] = [
    ("Impressions", "IMPRESSION"),
    ("Pin Clicks", "PIN_CLICK"),
    ("Outbound Clicks", "OUTBOUND_CLICK"),
    ("Saves", "SAVE"),
];

/// Convert a Pinterest analytics payload into one date series per metric.
///
/// Prefers `all.daily_metrics` (one point per day, which is what the dashboard
/// plots) and falls back to a single `today` point when the API returns
/// `summary_metrics` only. `today` is passed in so the mapping stays a pure
/// function of the payload and can be fixture-tested.
fn pin_analytics_series(raw: &serde_json::Value, today: &str) -> Vec<AnalyticsData> {
    let all = raw.get("all").unwrap_or(raw);
    let daily = all.get("daily_metrics").and_then(|d| d.as_object());
    let summary = all.get("summary_metrics").unwrap_or(all);

    PIN_METRICS
        .iter()
        .map(|(label, key)| {
            let points: Vec<AnalyticsDataPoint> = daily
                .map(|days| {
                    days.iter()
                        .filter_map(|(date, metrics)| {
                            count(metrics, key).map(|v| AnalyticsDataPoint {
                                total: v.to_string(),
                                date: date.clone(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let data = if points.is_empty() {
                count(summary, key)
                    .map(|v| {
                        vec![AnalyticsDataPoint {
                            total: v.to_string(),
                            date: today.to_string(),
                        }]
                    })
                    .unwrap_or_default()
            } else {
                points
            };

            AnalyticsData {
                label: (*label).to_string(),
                data,
                percentage_change: 0.0,
            }
        })
        .collect()
}

#[async_trait]
impl SocialProvider for PinterestProvider {
    fn identifier(&self) -> &'static str {
        "pinterest"
    }

    fn name(&self) -> &'static str {
        "Pinterest"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "boards:read".into(),
            "boards:write".into(),
            "pins:read".into(),
            "pins:write".into(),
            "user_accounts:read".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        500
    }

    /// The settings this provider needs to publish.
    ///
    /// Pinterest publishes to a board and needs a title. `publish` reads `post.settings["board"]` and `post.settings["title"]`.
    ///
    /// The create path checks this list and refuses the post with a
    /// message naming the missing field, instead of the post going to
    /// `state='error'` in the scheduler with no reason shown.
    fn required_settings(&self) -> &'static [&'static str] {
        &["board", "title"]
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let scope = "boards:read,boards:write,pins:read,pins:write,user_accounts:read";
        let params: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope),
            ("state", state),
        ];

        let url = url::Url::parse_with_params("https://www.pinterest.com/oauth/", &params)
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
        let auth_bytes = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );

        let params: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ];

        let resp = self
            .http
            .post("https://api.pinterest.com/v5/oauth/token")
            .header("Authorization", format!("Basic {auth_bytes}"))
            .form(&params)
            .send()
            .await?;

        let (access_token, json) = extract_access_token(resp, "pinterest")
            .await
            .map_err(ProviderError::Auth)?;
        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        // Get user info
        let user: serde_json::Value = self
            .http
            .get("https://api.pinterest.com/v5/user_account")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json()
            .await?;

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["username"].as_str().unwrap_or("").to_string(),
            username: user["username"].as_str().unwrap_or("").to_string(),
            picture: user["profile_image"].as_str().map(String::from),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let auth_bytes = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );

        let scope = self.scopes().join(",");
        let params: Vec<(&str, &str)> = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", scope.as_str()),
        ];

        let resp = self
            .http
            .post("https://api.pinterest.com/v5/oauth/token")
            .header("Authorization", format!("Basic {auth_bytes}"))
            .form(&params)
            .send()
            .await?;

        let (access_token, json) = extract_access_token(resp, "pinterest")
            .await
            .map_err(ProviderError::Auth)?;
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        Ok(AuthToken {
            access_token,
            refresh_token: Some(refresh_token.to_string()),
            expires_in,
            provider_user_id: String::new(),
            name: String::new(),
            username: String::new(),
            picture: None,
        })
    }

    /// List boards for the authenticated user
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .http
            .get("https://api.pinterest.com/v5/boards")
            .query(&[("page_size", "250")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let items = json["items"].as_array().cloned().unwrap_or_default();

        Ok(items
            .iter()
            .map(|item| PageInfo {
                id: item["id"].as_str().unwrap_or("").to_string(),
                name: item["name"].as_str().unwrap_or("").to_string(),
                access_token: None,
                picture: None,
                username: None,
            })
            .collect())
    }

    /// Pinterest-specific user-facing error copy (v25 §2 row 9).
    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if body.contains("The board does not exist") || body.contains("board_id") {
            return Some(
                "Pinterest rejected the board. Re-pick the board on the Channels screen — \
                 boards you lost access to are dropped from the list."
                    .into(),
            );
        }
        match classify_error(body, status) {
            Some(ErrorKind::RateLimited) => Some(
                "Pinterest rate limit hit. Pinterest allows roughly 1 000 API calls per user \
                 per hour — wait for the hourly reset or lower the publish rate."
                    .into(),
            ),
            Some(ErrorKind::MediaRejected) => Some(
                "Pinterest rejected the image or video. Pins accept JPEG/PNG under 20 MB \
                 fetched from a public URL; video pins must be MP4 under 500 MB sourced via \
                 the /media endpoint."
                    .into(),
            ),
            Some(ErrorKind::AuthExpired) => Some(
                "Pinterest access token expired or was revoked. Reconnect the account on \
                 the Channels screen."
                    .into(),
            ),
            Some(ErrorKind::Forbidden) => Some(
                "Pinterest denied this request (HTTP 403). The token is missing \
                 pins:write or boards:write, or the app lost its API approval."
                    .into(),
            ),
            Some(ErrorKind::NotFound) => Some(
                "Pinterest pin or board not found — it may be deleted, or the token belongs \
                 to a different Pinterest account."
                    .into(),
            ),
            Some(ErrorKind::InvalidRequest) => Some(
                "Pinterest rejected the request as invalid. Check the title (100 chars), \
                 description (800 chars) and link domain limits."
                    .into(),
            ),
            _ => None,
        }
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    /// Boards are the posting targets, same listing as `pages()`.
    async fn targets(&self, access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        Ok(super::pages_to_targets(
            &self.pages(access_token).await?,
            "board",
        ))
    }

    /// Pinterest's v5 API exposes no comment endpoints on pins — pin comments
    /// were removed with the v3 API — so the Comments tab stays empty for this
    /// provider rather than reporting an error the user cannot act on.
    async fn get_post_comments(
        &self,
        _access_token: &str,
        _platform_post_id: &str,
    ) -> Result<Vec<CommentData>, ProviderError> {
        Ok(vec![])
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let boards = self.pages(access_token).await?;
        let mut posts = Vec::new();
        let max_pins = (limit / 5).clamp(1, 10);

        for board in boards.iter().take(max_pins as usize) {
            let pins = self
                .get_board_pins(access_token, &board.id, limit.min(50))
                .await?;
            if let Some(items) = pins["items"].as_array() {
                for item in items {
                    let pin_id = item["id"].as_str().unwrap_or("").to_string();
                    let description = item["description"].as_str().unwrap_or("").to_string();
                    let title = item["title"].as_str().map(|s| s.to_string());
                    let media_url = item["media"]["images"]["originals"]["url"]
                        .as_str()
                        .map(|s| s.to_string());
                    let link = item["link"].as_str().map(|s| s.to_string());
                    let created_at = item["created_at"].as_str().unwrap_or("");

                    let posted_at = crate::social::common::parse_timestamp(created_at);

                    let post_url = link.or_else(|| {
                        Some(format!(
                            "https://www.pinterest.com/pin/{}",
                            item["id"].as_str().unwrap_or("")
                        ))
                    });

                    posts.push(ExternalPostData {
                        platform_post_id: pin_id,
                        text: description,
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
                        url: post_url,
                        metadata: title.map(|t| serde_json::json!({"title": t})),
                    });
                }
            }
        }

        // Sort by posted_at descending, take top 20
        posts.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        posts.truncate(20);

        Ok(posts)
    }

    /// Pin engagement counts (impressions, saves, pin clicks) for one pin.
    ///
    /// Pinterest exposes these only through the analytics endpoints, not on the
    /// pin resource itself, so this returns the raw analytics response and lets
    /// `parse_engagement_data` normalize it.
    async fn get_post_engagement(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Option<serde_json::Value>, ProviderError> {
        let (start, end) = analytics_window(30);
        let json = self
            .get_pin_analytics(access_token, platform_post_id, &start, &end)
            .await?;
        // Non-2xx already surfaces as Err from get_pin_analytics(); this guards
        // the 200-with-unexpected-body case so a junk payload never reaches the parser.
        if json.get("all").is_none() && json.get("summary_metrics").is_none() {
            return Ok(None);
        }
        Ok(Some(json))
    }

    /// Board-level analytics for the dashboard, one date series per metric.
    async fn analytics(
        &self,
        access_token: &str,
        internal_id: &str,
        days: u32,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let Some(board_id) = self.resolve_board_id(access_token, internal_id).await? else {
            return Ok(vec![]);
        };
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let (start, end) = analytics_window(days);
        let json = self
            .get_board_analytics(access_token, &board_id, &start, &end)
            .await?;
        Ok(pin_analytics_series(&json, &today))
    }

    /// Per-pin analytics, one date series per metric over the last 30 days.
    async fn post_analytics(
        &self,
        access_token: &str,
        platform_post_id: &str,
    ) -> Result<Vec<AnalyticsData>, ProviderError> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let (start, end) = analytics_window(30);
        let json = self
            .get_pin_analytics(access_token, platform_post_id, &start, &end)
            .await?;
        Ok(pin_analytics_series(&json, &today))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let board_id = post.settings["board"]
            .as_str()
            .ok_or_else(|| ProviderError::InvalidRequest("Missing board in settings".into()))?;

        let title = post.settings["title"].as_str().unwrap_or("");
        let link = post.settings["link"].as_str().unwrap_or("");

        // Construct media source
        let media_source = if !post.media.is_empty() && post.media[0].url.contains(".mp4") {
            // Video: upload first via /media endpoint, then create pin
            return self.publish_video(access_token, post, board_id).await;
        } else if post.media.len() == 1 {
            serde_json::json!({
                "source_type": "image_url",
                "url": post.media[0].url
            })
        } else {
            let items: Vec<serde_json::Value> = post
                .media
                .iter()
                .map(|m| serde_json::json!({"url": m.url}))
                .collect();
            serde_json::json!({
                "source_type": "multiple_image_urls",
                "items": items
            })
        };

        let mut body = serde_json::json!({
            "board_id": board_id,
            "description": post.content,
            "media_source": media_source,
        });

        if !title.is_empty() {
            body["title"] = serde_json::json!(title);
        }
        if !link.is_empty() {
            body["link"] = serde_json::json!(link);
        }

        let resp = self
            .http
            .post("https://api.pinterest.com/v5/pins")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await?;

        let publish_status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        let pin_id = json["id"].as_str().ok_or_else(|| {
            let err = json["message"]
                .as_str()
                .unwrap_or("Pinterest publish failed");
            let code = json["code"].as_u64().unwrap_or(0);
            let raw =
                format!("Pinterest publish failed (HTTP {publish_status}, code {code}): {err}");
            ProviderError::Api(friendly_error(
                self.map_error(err, publish_status.as_u16()),
                &raw,
            ))
        })?;

        Ok(PublishResult {
            platform_post_id: pin_id.to_string(),
            platform_post_url: Some(format!("https://www.pinterest.com/pin/{pin_id}")),
            status: "published".into(),
        })
    }
}

impl PinterestProvider {
    async fn publish_video(
        &self,
        access_token: &str,
        _post: &PostContent,
        _board_id: &str,
    ) -> Result<PublishResult, ProviderError> {
        // Step 1: Register media upload
        let reg_resp = self
            .http
            .post("https://api.pinterest.com/v5/media")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&serde_json::json!({"media_type": "video"}))
            .send()
            .await?;

        let _reg_json: serde_json::Value = reg_resp.json().await?;
        // Note: actual video upload requires presigned URL handling
        // Stub: return error with guidance
        Err(ProviderError::Api(
            "Video pin upload requires additional setup. Use image pins instead.".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::parse_engagement_data;

    #[test]
    fn parse_should_map_pinterest_summary_metrics() {
        let raw = serde_json::json!({
            "all": {
                "summary_metrics": {
                    "IMPRESSION": 1200,
                    "PIN_CLICK": 42,
                    "OUTBOUND_CLICK": 12,
                    "SAVE": 7
                }
            }
        });
        let e = parse_engagement_data("pinterest", raw);
        assert_eq!(e.views, 1200);
        assert_eq!(e.saves, 7);
        // Pinterest has no like metric; pin clicks stand in for it.
        assert_eq!(e.likes, 42);
    }

    #[test]
    fn parse_should_fall_back_to_flat_pinterest_metrics() {
        let raw = serde_json::json!({ "IMPRESSION": 5, "PIN_CLICK": 1, "SAVE": 0 });
        let e = parse_engagement_data("pinterest", raw);
        assert_eq!(e.views, 5);
        assert_eq!(e.likes, 1);
        assert_eq!(e.saves, 0);
    }

    #[test]
    fn parse_should_default_missing_pinterest_metrics_to_zero() {
        let e = parse_engagement_data(
            "pinterest",
            serde_json::json!({ "all": { "summary_metrics": { "IMPRESSION": 3 } } }),
        );
        assert_eq!(e.views, 3);
        assert_eq!(e.saves, 0);
        assert_eq!(e.likes, 0);
    }

    #[test]
    fn parse_should_read_pinterest_metrics_case_insensitively() {
        let raw = serde_json::json!({
            "all": { "summary_metrics": { "impression": 11, "save": 2 } }
        });
        let e = parse_engagement_data("pinterest", raw);
        assert_eq!(e.views, 11);
        assert_eq!(e.saves, 2);
    }

    #[test]
    fn pin_analytics_series_should_map_daily_metrics_to_date_series() {
        let raw = serde_json::json!({
            "all": {
                "daily_metrics": {
                    "2026-09-28": { "IMPRESSION": 10, "SAVE": 1 },
                    "2026-09-29": { "IMPRESSION": 20, "SAVE": 2 }
                },
                "summary_metrics": { "IMPRESSION": 30, "SAVE": 3 }
            }
        });
        let series = pin_analytics_series(&raw, "2026-09-30");
        let impressions = series
            .iter()
            .find(|s| s.label == "Impressions")
            .expect("Impressions series");
        assert_eq!(impressions.data.len(), 2);
        assert_eq!(impressions.data[0].date, "2026-09-28");
        assert_eq!(impressions.data[1].total, "20");
        let saves = series
            .iter()
            .find(|s| s.label == "Saves")
            .expect("Saves series");
        assert_eq!(saves.data[1].total, "2");
    }

    #[test]
    fn pin_analytics_series_should_fall_back_to_summary_point() {
        let raw = serde_json::json!({
            "all": { "summary_metrics": { "IMPRESSION": 30, "PIN_CLICK": 4 } }
        });
        let series = pin_analytics_series(&raw, "2026-09-30");
        let impressions = series
            .iter()
            .find(|s| s.label == "Impressions")
            .expect("Impressions series");
        assert_eq!(impressions.data.len(), 1);
        assert_eq!(impressions.data[0].total, "30");
        assert_eq!(impressions.data[0].date, "2026-09-30");
    }

    #[test]
    fn analytics_window_should_clamp_to_pinterest_ninety_day_max() {
        let (start, end) = analytics_window(3650);
        assert_eq!(
            end,
            chrono::Utc::now()
                .date_naive()
                .format("%Y-%m-%d")
                .to_string()
        );
        let span = end.parse::<chrono::NaiveDate>().expect("end date")
            - start.parse::<chrono::NaiveDate>().expect("start date");
        assert_eq!(
            span.num_days(),
            89,
            "v5 caps analytics at 90 days inclusive"
        );
    }

    // ── B4: map_error fixtures ──────────────────────────────

    fn provider() -> PinterestProvider {
        PinterestProvider::new(&crate::social::test_config())
    }

    #[test]
    fn map_error_keeps_board_specific_copy() {
        let msg = provider()
            .map_error("The board does not exist", 404)
            .unwrap();
        assert!(msg.contains("rejected the board"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rate_limit_on_429() {
        let msg = provider().map_error("Too many requests", 429).unwrap();
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_rejected_media() {
        let msg = provider()
            .map_error("The image upload was rejected", 422)
            .unwrap();
        assert!(msg.contains("20 MB"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_expired_token_on_401() {
        let msg = provider().map_error("", 401).unwrap();
        assert!(msg.contains("Reconnect"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_scope_on_403() {
        let msg = provider().map_error("", 403).unwrap();
        assert!(msg.contains("pins:write"), "got: {msg}");
    }

    #[test]
    fn map_error_explains_missing_pin_on_404() {
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

    #[test]
    fn pinterest_does_not_request_cron_refresh() {
        // Pinterest v5 access tokens do not expire — the refresh endpoint
        // exists but there is no expiry for the scheduler to act on.
        assert!(!provider().needs_cron_refresh());
    }

    // ── B3: board targets ────────────────────────────────────

    #[test]
    fn parse_should_map_pinterest_boards_into_targets() {
        let boards = vec![
            PageInfo {
                id: "b1".into(),
                name: "Recipes".into(),
                access_token: None,
                picture: None,
                username: None,
            },
            PageInfo {
                id: "b2".into(),
                name: "Travel".into(),
                access_token: None,
                picture: Some("https://img/b.jpg".into()),
                username: None,
            },
        ];
        let targets = super::super::pages_to_targets(&boards, "board");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "b1");
        assert_eq!(targets[0].name, "Recipes");
        assert_eq!(targets[0].target_type, "board");
        assert_eq!(targets[1].picture.as_deref(), Some("https://img/b.jpg"));
    }

    #[test]
    fn parse_should_read_no_pinterest_targets_from_empty_boards() {
        assert!(super::super::pages_to_targets(&[], "board").is_empty());
    }
}
