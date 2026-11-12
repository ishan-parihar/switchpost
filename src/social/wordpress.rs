// ─── WordPress Provider ──────────────────────────────────────
// Uses WordPress REST API with Basic Auth (Application Passwords).
// Credentials stored as JSON: {"site_url":"...","username":"...","app_password":"..."}

use async_trait::async_trait;
use base64::Engine;
use reqwest::StatusCode;

use super::*;
use crate::config::Config;

pub struct WordPressProvider {
    http: reqwest::Client,
}

impl WordPressProvider {
    pub fn new(_config: &Config) -> Self {
        Self {
            http: shared_http(),
        }
    }

    /// Parse credentials from JSON token: {"site_url":"...","username":"...","app_password":"..."}
    fn parse_creds(&self, access_token: &str) -> (String, String, String) {
        match serde_json::from_str::<serde_json::Value>(access_token) {
            Ok(creds) => {
                let site_url = creds["site_url"].as_str().unwrap_or("").to_string();
                let username = creds["username"].as_str().unwrap_or("").to_string();
                let app_password = creds["app_password"].as_str().unwrap_or("").to_string();
                (site_url, username, app_password)
            }
            Err(_) => (String::new(), String::new(), String::new()),
        }
    }

    /// Build Basic auth header value
    fn basic_auth(&self, username: &str, app_password: &str) -> String {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", username, app_password));
        format!("Basic {}", encoded)
    }

    /// List posts with optional status filter and pagination
    pub async fn list_posts(
        &self,
        access_token: &str,
        status: Option<&str>,
        per_page: Option<i32>,
    ) -> Result<serde_json::Value, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let mut url = format!("{}/wp-json/wp/v2/posts", site_url.trim_end_matches('/'));
        let mut params: Vec<String> = Vec::new();
        if let Some(s) = status {
            params.push(format!("status={}", s));
        }
        if let Some(pp) = per_page {
            params.push(format!("per_page={}", pp));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status_code.is_success() {
            Ok(json)
        } else {
            let msg = json["message"].as_str().unwrap_or(&body).to_string();
            Err(ProviderError::Api(format!(
                "WordPress API error ({}): {}",
                status_code, msg
            )))
        }
    }

    /// Get a single post by ID
    pub async fn get_post(
        &self,
        access_token: &str,
        post_id: i32,
    ) -> Result<serde_json::Value, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let url = format!(
            "{}/wp-json/wp/v2/posts/{}",
            site_url.trim_end_matches('/'),
            post_id
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status_code.is_success() {
            Ok(json)
        } else if status_code == StatusCode::NOT_FOUND {
            Err(ProviderError::Api(format!("Post {} not found", post_id)))
        } else {
            let msg = json["message"].as_str().unwrap_or(&body).to_string();
            Err(ProviderError::Api(format!(
                "WordPress API error ({}): {}",
                status_code, msg
            )))
        }
    }

    /// List categories
    pub async fn list_categories(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let url = format!(
            "{}/wp-json/wp/v2/categories",
            site_url.trim_end_matches('/')
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status_code.is_success() {
            Ok(json)
        } else {
            let msg = json["message"].as_str().unwrap_or(&body).to_string();
            Err(ProviderError::Api(format!(
                "WordPress API error ({}): {}",
                status_code, msg
            )))
        }
    }

    /// List tags
    pub async fn list_tags(&self, access_token: &str) -> Result<serde_json::Value, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let url = format!("{}/wp-json/wp/v2/tags", site_url.trim_end_matches('/'));

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status_code.is_success() {
            Ok(json)
        } else {
            let msg = json["message"].as_str().unwrap_or(&body).to_string();
            Err(ProviderError::Api(format!(
                "WordPress API error ({}): {}",
                status_code, msg
            )))
        }
    }
}

#[async_trait]
impl SocialProvider for WordPressProvider {
    fn identifier(&self) -> &'static str {
        "wordpress"
    }

    fn name(&self) -> &'static str {
        "WordPress"
    }

    fn scopes(&self) -> Vec<String> {
        vec![] // WordPress uses Application Passwords, not OAuth scopes
    }

    fn max_content_length(&self) -> usize {
        usize::MAX // WordPress has very high content limits
    }

    fn uses_oauth(&self) -> bool {
        false
    }

    fn tooltip(&self) -> Option<&'static str> {
        Some("Uses Application Password (REST API). Provide site URL, username, and app password.")
    }

    async fn generate_auth_url(
        &self,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        Err(ProviderError::Auth(
            "WordPress uses Application Password authentication. \
             Provide your site URL, username, and Application Password directly."
                .into(),
        ))
    }

    async fn exchange_code(
        &self,
        code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        // Parse the JSON credentials
        let creds: serde_json::Value = serde_json::from_str(code).map_err(|_| {
            ProviderError::Auth(
                "Invalid credentials format. Expected JSON with site_url, username, app_password."
                    .into(),
            )
        })?;

        let site_url = creds["site_url"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing 'site_url' in credentials".into()))?;
        let username = creds["username"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing 'username' in credentials".into()))?;
        let app_password = creds["app_password"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing 'app_password' in credentials".into()))?;

        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth(
                "site_url, username, and app_password must not be empty.".into(),
            ));
        }

        // Validate by fetching current user info
        let user_url = format!(
            "{}/wp-json/wp/v2/users/me?context=edit",
            site_url.trim_end_matches('/')
        );
        let auth_header = self.basic_auth(username, app_password);

        let resp = self
            .http
            .get(&user_url)
            .header("Authorization", &auth_header)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status == StatusCode::OK {
            let user_id = json["id"]
                .as_u64()
                .map(|id| id.to_string())
                .unwrap_or_default();
            let user_name = json["name"]
                .as_str()
                .unwrap_or("WordPress User")
                .to_string();
            let user_username = json["slug"].as_str().unwrap_or("").to_string();
            let avatar_url = json["avatar_urls"]["96"].as_str().map(String::from);

            // Store the full credentials JSON as the access_token
            let token_json = serde_json::json!({
                "site_url": site_url,
                "username": username,
                "app_password": app_password,
            });

            Ok(AuthToken {
                access_token: token_json.to_string(),
                refresh_token: None,
                expires_in: None,
                provider_user_id: user_id,
                name: user_name,
                username: user_username,
                picture: avatar_url,
            })
        } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth(
                "Invalid WordPress credentials. Check your site URL, username, and Application Password."
                    .into(),
            ))
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("WordPress API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "WordPress Application Passwords do not expire. Re-connect if needed.".into(),
        ))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth(
                "Invalid WordPress credentials. Re-connect your WordPress integration.".into(),
            ));
        }

        // Extract title from settings
        let title = post
            .settings
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Extract publish status (default: draft)
        let publish_status = post
            .settings
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("draft");

        // Extract categories (array of integers) and tags (array of integers)
        let categories: Option<Vec<i64>> = post
            .settings
            .get("categories")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect());

        let tags: Option<Vec<i64>> = post
            .settings
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect());

        // Extract featured_media (integer ID)
        let featured_media = post.settings.get("featured_media").and_then(|v| v.as_i64());

        // Build the post body
        let mut body = serde_json::json!({
            "title": title,
            "content": post.content,
            "status": publish_status,
        });

        if let Some(cats) = categories {
            body["categories"] = serde_json::json!(cats);
        }
        if let Some(t) = tags {
            body["tags"] = serde_json::json!(t);
        }
        if let Some(fm) = featured_media {
            body["featured_media"] = serde_json::json!(fm);
        }

        let url = format!("{}/wp-json/wp/v2/posts", site_url.trim_end_matches('/'));

        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status_code = resp.status();
        let response_body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&response_body).unwrap_or(serde_json::Value::Null);

        if status_code == StatusCode::CREATED || status_code == StatusCode::OK {
            let post_id = json["id"]
                .as_u64()
                .map(|id| id.to_string())
                .unwrap_or_default();
            let post_url = json["link"].as_str().map(String::from);
            let post_status = json["status"]
                .as_str()
                .unwrap_or(publish_status)
                .to_string();

            Ok(PublishResult {
                platform_post_id: post_id,
                platform_post_url: post_url,
                status: post_status,
            })
        } else if status_code.is_client_error() {
            let msg = json["message"]
                .as_str()
                .unwrap_or("WordPress publish failed")
                .to_string();
            Err(self.provider_error(&msg, status_code.as_u16()))
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("WordPress API error")
                .to_string();
            Err(self.provider_error(&msg, status_code.as_u16()))
        }
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let url = format!(
            "{}/wp-json/wp/v2/users/me?context=edit",
            site_url.trim_end_matches('/')
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status == StatusCode::OK {
            Ok(vec![PageInfo {
                id: json["id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                name: json["name"]
                    .as_str()
                    .unwrap_or("WordPress User")
                    .to_string(),
                access_token: Some(access_token.to_string()),
                picture: json["avatar_urls"]["96"].as_str().map(String::from),
                username: json["slug"].as_str().map(String::from),
            }])
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Failed to fetch WordPress user info")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let url = format!(
            "{}/wp-json/wp/v2/users/{}",
            site_url.trim_end_matches('/'),
            page_id
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if status == StatusCode::OK {
            Ok(PageInfo {
                id: json["id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                name: json["name"]
                    .as_str()
                    .unwrap_or("WordPress User")
                    .to_string(),
                access_token: Some(access_token.to_string()),
                picture: json["avatar_urls"]["96"].as_str().map(String::from),
                username: json["slug"].as_str().map(String::from),
            })
        } else if status == StatusCode::NOT_FOUND {
            Err(ProviderError::Api(format!(
                "WordPress user {} not found",
                page_id
            )))
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Failed to fetch WordPress user info")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let (site_url, username, app_password) = self.parse_creds(access_token);
        if site_url.is_empty() || username.is_empty() || app_password.is_empty() {
            return Err(ProviderError::Auth("Invalid WordPress credentials".into()));
        }

        let limit = _limit.min(100);
        let url = format!(
            "{}/wp-json/wp/v2/posts?per_page={}&orderby=date&order=desc",
            site_url.trim_end_matches('/'),
            limit
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.basic_auth(&username, &app_password))
            .send()
            .await?;

        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

        if !status_code.is_success() {
            let msg = json["message"].as_str().unwrap_or(&body).to_string();
            return Err(ProviderError::Api(format!(
                "WordPress API error ({}): {}",
                status_code, msg
            )));
        }

        let posts: Vec<ExternalPostData> = json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        // Skip undated items rather than stamp "now" on them.
                        let created_at = p["date_gmt"]
                            .as_str()
                            .and_then(crate::social::common::parse_timestamp_opt)?;
                        let _content_text = p["content"]["rendered"]
                            .as_str()
                            .map(common::strip_html_tags)
                            .unwrap_or_default();
                        let excerpt_text = p["excerpt"]["rendered"]
                            .as_str()
                            .map(common::strip_html_tags)
                            .unwrap_or_default();
                        let featured_media_url = p["_embedded"]["wp:featuredmedia"][0]
                            ["source_url"]
                            .as_str()
                            .map(String::from);
                        let mut media = Vec::new();
                        if let Some(url) = featured_media_url {
                            media.push(MediaAttachment {
                                url,
                                mime_type: "image/jpeg".into(),
                                alt: p["_embedded"]["wp:featuredmedia"][0]["alt_text"]
                                    .as_str()
                                    .map(String::from),
                                poster_url: None,
                            });
                        }
Some(                        ExternalPostData {
                            platform_post_id: p["id"]
                                .as_u64()
                                .map(|id| id.to_string())
                                .unwrap_or_default(),
                            text: excerpt_text,
                            author_name: p["_embedded"]["author"][0]["name"]
                                .as_str()
                                .map(String::from),
                            author_handle: p["_embedded"]["author"][0]["slug"]
                                .as_str()
                                .map(String::from),
                            author_avatar: p["_embedded"]["author"][0]["avatar_urls"]["96"]
                                .as_str()
                                .map(String::from),
                            created_at,
                            url: p["link"].as_str().map(String::from),
                            media,
                            metadata: Some(serde_json::json!({
                                "title": p["title"]["rendered"],
                                "status": p["status"],
                                "post_type": p["type"],
                                "sticky": p["sticky"],
                            })),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(posts)
    }

    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if status == 401 || status == 403 {
            Some(
                "Invalid WordPress credentials. Check your site URL, username, and Application Password."
                    .into(),
            )
        } else if status == 404 {
            Some("WordPress resource not found. Check the URL or resource ID.".into())
        } else if status == 429 {
            Some("WordPress API rate limit exceeded. Try again later.".into())
        } else if body.contains("rest_cannot_create") {
            Some("You don't have permission to create posts on this WordPress site.".into())
        } else if body.contains("rest_invalid_param") {
            Some("Invalid parameter in WordPress API request. Check required fields.".into())
        } else {
            None
        }
    }
}
