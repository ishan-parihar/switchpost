// ─── Hashnode Provider ────────────────────────────────────────
// Uses Hashnode's REST API with an API key (no OAuth).

use async_trait::async_trait;
use reqwest::StatusCode;

use super::*;
use crate::config::Config;

const HASHNODE_API_BASE: &str = "https://api.hashnode.com";

pub struct HashnodeProvider {
    http: reqwest::Client,
}

impl HashnodeProvider {
    pub fn new(_config: &Config) -> Self {
        Self {
            http: shared_http(),
        }
    }

    pub async fn list_posts(
        &self,
        access_token: &str,
        publication_id: &str,
        page: i32,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "{HASHNODE_API_BASE}/api/me/stories/publication/{publication_id}"
            ))
            .query(&[("page", page)])
            .header("Authorization", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            Ok(json)
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Failed to list Hashnode posts")
                    .to_string(),
            ))
        }
    }

    pub async fn get_post(
        &self,
        access_token: &str,
        post_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("{HASHNODE_API_BASE}/api/post/{post_id}"))
            .header("Authorization", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            Ok(json)
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Failed to get Hashnode post")
                    .to_string(),
            ))
        }
    }
}

#[async_trait]
impl SocialProvider for HashnodeProvider {
    fn identifier(&self) -> &'static str {
        "hashnode"
    }

    fn name(&self) -> &'static str {
        "Hashnode"
    }

    fn scopes(&self) -> Vec<String> {
        vec!["write:posts".into()]
    }

    fn max_content_length(&self) -> usize {
        100000
    }

    /// The settings this provider needs to publish.
    ///
    /// Hashnode publishes to a publication. `publish` reads `post.settings["publication_id"]` and errors with "Missing publication_id in settings".
    ///
    /// The create path checks this list and refuses the post with a
    /// message naming the missing field, instead of the post going to
    /// `state='error'` in the scheduler with no reason shown.
    fn required_settings(&self) -> &'static [&'static str] {
        &["publication_id", "title"]
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
            "Hashnode uses an API key instead of OAuth. \
             Set HASHNODE_API_KEY in your .env file."
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
            "Hashnode uses an API key instead of OAuth. \
             Set HASHNODE_API_KEY in your .env file."
                .into(),
        ))
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "Hashnode API keys do not expire. Set HASHNODE_API_KEY in your .env file.".into(),
        ))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let publication_id = post
            .settings
            .get("publication_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProviderError::Api("Missing publication_id in settings".into()))?;

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

        let tags: Vec<serde_json::Value> = post
            .settings
            .get("tags")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let body = serde_json::json!({
            "title": title,
            "contentMarkdown": post.content,
            "tags": tags,
            "isRepublished": false,
        });

        let resp = self
            .http
            .post(format!(
                "{HASHNODE_API_BASE}/api/publication/{publication_id}/posts"
            ))
            .header("Authorization", access_token)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK || status == StatusCode::CREATED {
            let post_data = &json["post"];
            let post_id = post_data["_id"]
                .as_str()
                .or_else(|| post_data["id"].as_str())
                .unwrap_or("")
                .to_string();
            let post_url = post_data["url"]
                .as_str()
                .or_else(|| post_data["slug"].as_str())
                .map(String::from);

            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: post_url,
                status: "published".into(),
            })
        } else if status.is_client_error() {
            let msg = json["message"]
                .as_str()
                .or_else(|| json["error"].as_str())
                .unwrap_or("Hashnode publish failed")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        } else {
            Err(ProviderError::Api("Hashnode API error".into()))
        }
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .http
            .get(format!("{HASHNODE_API_BASE}/api/me"))
            .header("Authorization", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status != StatusCode::OK {
            return Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Failed to fetch Hashnode user info")
                    .to_string(),
            ));
        }

        let resp = self
            .http
            .get(format!("{HASHNODE_API_BASE}/api/me/publications"))
            .header("Authorization", access_token)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            let publications: Vec<PageInfo> = json["publications"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|pub_info| {
                            let p = &pub_info["publication"];
                            PageInfo {
                                id: p["_id"].as_str().unwrap_or("").to_string(),
                                name: p["title"]
                                    .as_str()
                                    .unwrap_or("Hashnode Publication")
                                    .to_string(),
                                access_token: None,
                                picture: p["logo"]
                                    .as_str()
                                    .map(String::from)
                                    .or_else(|| p["logoUrl"].as_str().map(String::from)),
                                username: None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(publications)
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Failed to fetch Hashnode publications")
                    .to_string(),
            ))
        }
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let pages = self.pages(access_token).await?;
        let mut posts = Vec::new();
        let pages_to_check = (limit / 5).clamp(1, 5);

        for page in pages.iter().take(pages_to_check as usize) {
            let data = self.list_posts(access_token, &page.id, 1).await?;
            if let Some(stories) = data["stories"].as_array() {
                for item in stories {
                    let post_id = item["_id"]
                        .as_str()
                        .or_else(|| item["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    let title = item["title"].as_str().unwrap_or("").to_string();
                    let brief = item["brief"].as_str().unwrap_or("");
                    let slug = item["slug"].as_str().unwrap_or("");
                    let cover = item["coverImage"].as_str().map(|s| s.to_string());
                    let date_added = item["dateAdded"].as_str().unwrap_or("");

                    let posted_at = crate::social::common::parse_timestamp(date_added);

                    posts.push(ExternalPostData {
                        platform_post_id: post_id,
                        text: brief.to_string(),
                        author_name: None,
                        author_handle: None,
                        author_avatar: None,
                        media: cover
                            .into_iter()
                            .map(|u| MediaAttachment {
                                url: u,
                                mime_type: String::new(),
                                alt: None,
                                poster_url: None,
                            })
                            .collect(),
                        created_at: posted_at,
                        url: Some(format!("https://hashnode.com/post/{slug}")),
                        metadata: Some(serde_json::json!({"title": title})),
                    });
                }
            }
        }

        // Sort by posted_at descending, take top 20
        posts.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        posts.truncate(20);

        Ok(posts)
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn comment(
        &self,
        _access_token: &str,
        _post_id: &str,
        _last_comment_id: Option<&str>,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Hashnode does not support commenting via API".into(),
        ))
    }

    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Invalid Hashnode API key. Check HASHNODE_API_KEY in .env.".into())
        } else if status == 429 {
            Some("Hashnode API rate limit exceeded. Try again later.".into())
        } else if body.contains("publication_id") {
            Some("Invalid or missing publication_id in settings.".into())
        } else {
            None
        }
    }
}
