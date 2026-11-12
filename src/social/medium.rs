// ─── Medium Provider ─────────────────────────────────────────
// Uses Medium API v1 with personal access tokens (no OAuth).

use async_trait::async_trait;
use reqwest::StatusCode;

use super::*;
use crate::config::Config;

const MEDIUM_API_BASE: &str = "https://api.medium.com/v1";

pub struct MediumProvider {
    client: reqwest::Client,
}

impl MediumProvider {
    pub fn new(_config: &Config) -> Self {
        Self {
            client: shared_http(),
        }
    }

    /// Internal helper: get the user's Medium author ID.
    async fn get_author_id(&self, access_token: &str) -> Result<String, ProviderError> {
        let resp = self
            .client
            .get(format!("{MEDIUM_API_BASE}/me"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            json["data"]["id"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| ProviderError::Api("Missing user ID in Medium response".into()))
        } else if status.is_client_error() {
            Err(ProviderError::Auth(
                json["errors"][0]["message"]
                    .as_str()
                    .unwrap_or("Medium auth failed")
                    .to_string(),
            ))
        } else {
            Err(ProviderError::Api(
                json["errors"][0]["message"]
                    .as_str()
                    .unwrap_or("Medium API error")
                    .to_string(),
            ))
        }
    }
}

#[async_trait]
impl SocialProvider for MediumProvider {
    fn identifier(&self) -> &'static str {
        "medium"
    }

    fn name(&self) -> &'static str {
        "Medium"
    }

    fn scopes(&self) -> Vec<String> {
        vec![] // Medium uses personal access tokens, not OAuth scopes
    }

    fn max_content_length(&self) -> usize {
        15000
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
            "Medium uses a personal access token instead of OAuth. \
             Set MEDIUM_ACCESS_TOKEN in your .env file."
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
            "Medium uses a personal access token instead of OAuth. \
             Set MEDIUM_ACCESS_TOKEN in your .env file."
                .into(),
        ))
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "Medium tokens do not expire. Set MEDIUM_ACCESS_TOKEN in your .env file.".into(),
        ))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let author_id = self.get_author_id(access_token).await?;

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

        // Extract tags from settings
        let tags: Vec<String> = post
            .settings
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Extract publish status from settings
        let publish_status = post
            .settings
            .get("publish_status")
            .and_then(|v| v.as_str())
            .unwrap_or("draft");

        let body = serde_json::json!({
            "title": title,
            "contentFormat": "markdown",
            "content": post.content,
            "tags": tags,
            "publishStatus": publish_status,
        });

        let resp = self
            .client
            .post(format!("{MEDIUM_API_BASE}/users/{author_id}/posts"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::CREATED || status == StatusCode::OK {
            let post_data = &json["data"];
            let post_id = post_data["id"].as_str().unwrap_or("").to_string();
            let post_url = post_data["url"].as_str().map(String::from);

            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: post_url,
                status: publish_status.to_string(),
            })
        } else if status.is_client_error() {
            let msg = json["errors"][0]["message"]
                .as_str()
                .unwrap_or("Medium publish failed")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        } else {
            Err(ProviderError::Api(
                json["errors"][0]["message"]
                    .as_str()
                    .unwrap_or("Medium API error")
                    .to_string(),
            ))
        }
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .client
            .get(format!("{MEDIUM_API_BASE}/me"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            let user = &json["data"];
            Ok(vec![PageInfo {
                id: user["id"].as_str().unwrap_or("").to_string(),
                name: user["name"].as_str().unwrap_or("Medium User").to_string(),
                access_token: Some(access_token.to_string()),
                picture: user["imageUrl"].as_str().map(String::from),
                username: user["username"].as_str().map(String::from),
            }])
        } else {
            Err(ProviderError::Api(
                "Failed to fetch Medium user info".into(),
            ))
        }
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let pages = self.pages(access_token).await?;
        pages
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Api("No Medium user info available".into()))
    }

    async fn comment(
        &self,
        _access_token: &str,
        _post_id: &str,
        _last_comment_id: Option<&str>,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Medium does not support commenting via API".into(),
        ))
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let author_id = self.get_author_id(access_token).await?;
        let limit = _limit.min(100);
        let resp = self
            .client
            .get(format!("{MEDIUM_API_BASE}/users/{author_id}/posts"))
            .header("Authorization", format!("Bearer {access_token}"))
            .query(&[("limit", limit.to_string())])
            .send()
            .await?;

        let status_code = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status_code.is_success() {
            let msg = json["errors"][0]["message"]
                .as_str()
                .unwrap_or("Failed to fetch Medium posts")
                .to_string();
            return Err(self.provider_error(&msg, status_code.as_u16()));
        }

        let posts: Vec<ExternalPostData> = json["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        // Skip undated items rather than stamp "now" on them.
                        let created_at = p["createdAt"]
                            .as_u64()
                            .and_then(|ts| chrono::DateTime::from_timestamp(ts as i64 / 1000, 0))?;
                        let mut media = Vec::new();
                        // Medium v1 API doesn't provide post images directly,
                        // but some response formats include imageUrl or virtuals.previewImage
                        if let Some(img_url) = p["imageUrl"].as_str() {
                            if !img_url.is_empty() {
                                media.push(MediaAttachment {
                                    url: img_url.to_string(),
                                    mime_type: "image/jpeg".to_string(),
                                    alt: p["title"].as_str().map(String::from),
                                    poster_url: None,
                                });
                            }
                        } else if let Some(img_id) =
                            p["virtuals"]["previewImage"]["imageId"].as_str()
                        {
                            let img_url = format!("https://miro.medium.com/1/{img_id}");
                            media.push(MediaAttachment {
                                url: img_url,
                                mime_type: "image/jpeg".to_string(),
                                alt: p["title"].as_str().map(String::from),
                                poster_url: None,
                            });
                        }
Some(                        ExternalPostData {
                            platform_post_id: p["id"].as_str().unwrap_or("").to_string(),
                            text: p["content"]
                                .as_str()
                                .or_else(|| p["body"].as_str())
                                .unwrap_or("")
                                .to_string(),
                            author_name: p["author"]["name"].as_str().map(String::from),
                            author_handle: p["author"]["username"].as_str().map(String::from),
                            author_avatar: p["author"]["image"].as_str().map(String::from),
                            created_at,
                            url: p["url"].as_str().map(String::from),
                            media,
                            metadata: Some(serde_json::json!({
                                "title": p["title"],
                                "subtitle": p["subtitle"],
                                "tags": p["tags"],
                                "reading_time": p["readingTime"],
                                "claps": p["claps"],
                                "responses": p["responses"],
                                "publication_id": p["publicationId"],
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
            Some("Invalid Medium access token. Check MEDIUM_ACCESS_TOKEN in .env.".into())
        } else if status == 429 {
            Some("Medium API rate limit exceeded. Try again later.".into())
        } else if body.contains("publishStatus") {
            Some("Invalid publish status. Use 'draft', 'public', or 'unlisted'.".into())
        } else {
            None
        }
    }
}
