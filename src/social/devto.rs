// ─── Dev.to Provider ─────────────────────────────────────────
// Uses Dev.to API v0 with API keys (no OAuth).

use async_trait::async_trait;
use reqwest::StatusCode;

use super::*;
use crate::config::Config;

const DEVTO_API_BASE: &str = "https://dev.to/api";

pub struct DevtoProvider {
    client: reqwest::Client,
}

impl DevtoProvider {
    pub fn new(_config: &Config) -> Self {
        Self {
            client: shared_http(),
        }
    }
}

#[async_trait]
impl SocialProvider for DevtoProvider {
    fn identifier(&self) -> &'static str {
        "devto"
    }

    fn name(&self) -> &'static str {
        "Dev.to"
    }

    fn scopes(&self) -> Vec<String> {
        vec![] // Dev.to uses API keys, not OAuth scopes
    }

    fn max_content_length(&self) -> usize {
        800000 // ~800KB limit for body_markdown
    }

    fn editor_type(&self) -> EditorType {
        EditorType::Markdown
    }

    fn uses_oauth(&self) -> bool {
        false
    }

    async fn generate_auth_url(
        &self,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        Err(ProviderError::Auth(
            "Dev.to uses an API key instead of OAuth. \
             Set DEVTO_API_KEY in your .env file."
                .into(),
        ))
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "Dev.to uses an API key instead of OAuth. \
             Set DEVTO_API_KEY in your .env file."
                .into(),
        ))
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "Dev.to API keys do not expire. Set DEVTO_API_KEY in your .env file.".into(),
        ))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Extract title from post content (first line) or settings
        let title = post
            .settings
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                post.content
                    .lines()
                    .next()
                    .unwrap_or("Untitled")
                    .trim_start_matches("# ")
                    .trim_start_matches("#")
                    .trim()
                    .to_string()
            });

        // Extract tags from settings (Dev.to max 4 tags)
        let tags: Vec<String> = post
            .settings
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .take(4)
                    .collect()
            })
            .unwrap_or_default();

        // Extract published flag from settings
        let published = post
            .settings
            .get("published")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let body = serde_json::json!({
            "article": {
                "title": title,
                "body_markdown": post.content,
                "tags": tags,
                "published": published,
            }
        });

        let resp = self
            .client
            .post(format!("{DEVTO_API_BASE}/articles"))
            .header("api-key", access_token)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::CREATED || status == StatusCode::OK {
            let article_id = json["id"]
                .as_u64()
                .map(|id| id.to_string())
                .unwrap_or_default();
            let article_url = json["url"].as_str().map(String::from);

            Ok(PublishResult {
                platform_post_id: article_id,
                platform_post_url: article_url,
                status: if published {
                    "published".into()
                } else {
                    "draft".into()
                },
            })
        } else if status.is_client_error() {
            let msg = json["error"]
                .as_str()
                .unwrap_or(json["message"].as_str().unwrap_or("Dev.to publish failed"))
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        } else {
            Err(ProviderError::Api(
                json["error"]
                    .as_str()
                    .unwrap_or("Dev.to API error")
                    .to_string(),
            ))
        }
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .client
            .get(format!("{DEVTO_API_BASE}/articles/me"))
            .header("api-key", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            let articles: Vec<PageInfo> = json
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|article| PageInfo {
                            id: article["id"]
                                .as_u64()
                                .map(|id| id.to_string())
                                .unwrap_or_default(),
                            name: article["title"].as_str().unwrap_or("Untitled").to_string(),
                            access_token: Some(access_token.to_string()),
                            picture: article["social_image"].as_str().map(String::from).or_else(
                                || article["user"]["profile_image"].as_str().map(String::from),
                            ),
                            username: article["user"]["username"].as_str().map(String::from),
                        })
                        .collect()
                })
                .unwrap_or_default();

            // If no articles exist, return user info
            if articles.is_empty() {
                let user_info = self.get_user_info(access_token).await?;
                Ok(vec![user_info])
            } else {
                Ok(articles)
            }
        } else {
            Err(ProviderError::Api("Failed to fetch Dev.to articles".into()))
        }
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        // Try fetching a specific article first
        let resp = self
            .client
            .get(format!("{DEVTO_API_BASE}/articles/{page_id}"))
            .header("api-key", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            Ok(PageInfo {
                id: json["id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                name: json["title"].as_str().unwrap_or("Untitled").to_string(),
                access_token: Some(access_token.to_string()),
                picture: json["social_image"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| json["user"]["profile_image"].as_str().map(String::from)),
                username: json["user"]["username"].as_str().map(String::from),
            })
        } else {
            // Fall back to user info
            self.get_user_info(access_token).await
        }
    }

    async fn comment(
        &self,
        _access_token: &str,
        _post_id: &str,
        _last_comment_id: Option<&str>,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Dev.to does not support commenting via API".into(),
        ))
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let resp = self
            .client
            .get(format!("{DEVTO_API_BASE}/articles/me"))
            .header("api-key", access_token)
            .query(&[("per_page", _limit.to_string())])
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            return Err(ProviderError::Api("Failed to fetch Dev.to articles".into()));
        }

        let posts: Vec<ExternalPostData> = json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|article| {
                        // Skip undated items rather than stamp "now" on them.
                        let created_at = article["published_at"]
                            .as_str()
                            .and_then(crate::social::common::parse_timestamp_opt)?;
                        let tags: Vec<String> = article["tag_list"]
                            .as_array()
                            .map(|t| {
                                t.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut media = Vec::new();
                        if let Some(cover_url) = article["cover_image"].as_str() {
                            if !cover_url.is_empty() {
                                media.push(MediaAttachment {
                                    url: cover_url.to_string(),
                                    mime_type: "image/jpeg".to_string(),
                                    alt: article["title"].as_str().map(String::from),
                                    poster_url: None,
                                });
                            }
                        }
Some(                        ExternalPostData {
                            platform_post_id: article["id"]
                                .as_u64()
                                .map(|id| id.to_string())
                                .unwrap_or_default(),
                            text: article["body_markdown"].as_str().unwrap_or("").to_string(),
                            author_name: article["user"]["name"].as_str().map(String::from),
                            author_handle: article["user"]["username"].as_str().map(String::from),
                            author_avatar: article["user"]["profile_image_90"]
                                .as_str()
                                .map(String::from),
                            created_at,
                            url: article["url"].as_str().map(String::from),
                            media,
                            metadata: Some(serde_json::json!({
                                "title": article["title"],
                                "tags": tags,
                                "reading_time": article["reading_time_minutes"],
                                "comments_count": article["comments_count"],
                                "positive_reactions": article["positive_reactions_count"],
                            })),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(posts)
    }

    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Invalid Dev.to API key. Check DEVTO_API_KEY in .env.".into())
        } else if status == 429 {
            Some("Dev.to API rate limit exceeded. Try again later.".into())
        } else if body.contains("has already been taken") {
            Some("A Dev.to article with this title already exists.".into())
        } else {
            None
        }
    }
}

impl DevtoProvider {
    /// Internal helper: get the authenticated user's info.
    async fn get_user_info(&self, access_token: &str) -> Result<PageInfo, ProviderError> {
        let resp = self
            .client
            .get(format!("{DEVTO_API_BASE}/users/me"))
            .header("api-key", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            Ok(PageInfo {
                id: json["id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                name: json["name"].as_str().unwrap_or("Dev.to User").to_string(),
                access_token: Some(access_token.to_string()),
                picture: json["profile_image"].as_str().map(String::from),
                username: json["username"].as_str().map(String::from),
            })
        } else {
            Err(ProviderError::Api(
                "Failed to fetch Dev.to user info".into(),
            ))
        }
    }
}
