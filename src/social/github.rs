// ─── GitHub Provider ──────────────────────────────────────────
// Uses GitHub REST API v3 + GraphQL API with Personal Access Tokens (PAT),
// not OAuth. Suitable for MCP tools that need GitHub data access.
//
// REST API: https://api.github.com
// GraphQL:  https://api.github.com/graphql
//
// Authentication: Authorization: Bearer <token>
// User-Agent header is required by GitHub API (set via Client builder).

use async_trait::async_trait;
use reqwest::StatusCode;

use super::*;
use crate::config::Config;

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "switchpost/1.0";

/// Shared GitHub HTTP client (F1): ONE UA-pinned pool. GitHub REQUIRES a
/// User-Agent header; per-provider construction rebuilt it per tool call.
fn github_http() -> reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("Failed to build reqwest client")
    });
    CLIENT.clone()
}

pub struct GithubProvider {
    pub token: String,
    pub http: reqwest::Client,
}

impl GithubProvider {
    pub fn new(config: &Config) -> Self {
        let token = config
            .provider_credentials("github")
            .map(|(_, t)| t)
            .unwrap_or_default();

        let http = github_http();

        Self { token, http }
    }

    /// Perform a GET request to the GitHub REST API.
    async fn github_get(
        &self,
        access_token: &str,
        path: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{GITHUB_API_BASE}{path}");
        let mut req = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/vnd.github.v3+json");

        if let Some(params) = query {
            req = req.query(params);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status.is_success() {
            Ok(json)
        } else if status == StatusCode::UNAUTHORIZED {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Authentication failed")
                .to_string();
            Err(ProviderError::Auth(msg))
        } else if status == StatusCode::FORBIDDEN {
            let msg = json["message"].as_str().unwrap_or("Forbidden").to_string();
            if msg.to_lowercase().contains("rate limit") {
                Err(ProviderError::RateLimited(msg))
            } else {
                Err(self.provider_error(&msg, status.as_u16()))
            }
        } else if status == StatusCode::NOT_FOUND {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Resource not found")
                    .to_string(),
            ))
        } else if status == StatusCode::UNPROCESSABLE_ENTITY {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Validation error")
                .to_string();
            Err(ProviderError::InvalidRequest(msg))
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("GitHub API error")
                    .to_string(),
            ))
        }
    }

    /// Perform a PATCH to the GitHub REST API.
    async fn github_patch(
        &self,
        access_token: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{GITHUB_API_BASE}{path}");
        let resp = self
            .http
            .patch(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/vnd.github.v3+json")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::OK {
            Ok(json)
        } else if status == StatusCode::UNAUTHORIZED {
            Err(ProviderError::Auth(
                json["message"]
                    .as_str()
                    .unwrap_or("Authentication failed")
                    .to_string(),
            ))
        } else if status == StatusCode::FORBIDDEN {
            let msg = json["message"].as_str().unwrap_or("Forbidden").to_string();
            if msg.to_lowercase().contains("rate limit") {
                Err(ProviderError::RateLimited(msg))
            } else {
                Err(self.provider_error(&msg, status.as_u16()))
            }
        } else if status == StatusCode::NOT_FOUND {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Resource not found")
                    .to_string(),
            ))
        } else if status == StatusCode::UNPROCESSABLE_ENTITY {
            Err(ProviderError::InvalidRequest(
                json["message"]
                    .as_str()
                    .unwrap_or("Validation error")
                    .to_string(),
            ))
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("GitHub API error")
                    .to_string(),
            ))
        }
    }

    /// Perform a POST to the GitHub REST API.
    async fn github_post(
        &self,
        access_token: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{GITHUB_API_BASE}{path}");
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/vnd.github.v3+json")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == StatusCode::CREATED || status == StatusCode::OK {
            Ok(json)
        } else if status == StatusCode::UNAUTHORIZED {
            Err(ProviderError::Auth(
                json["message"]
                    .as_str()
                    .unwrap_or("Authentication failed")
                    .to_string(),
            ))
        } else if status == StatusCode::FORBIDDEN {
            let msg = json["message"].as_str().unwrap_or("Forbidden").to_string();
            if msg.to_lowercase().contains("rate limit") {
                Err(ProviderError::RateLimited(msg))
            } else {
                Err(self.provider_error(&msg, status.as_u16()))
            }
        } else if status == StatusCode::UNPROCESSABLE_ENTITY {
            Err(ProviderError::InvalidRequest(
                json["message"]
                    .as_str()
                    .unwrap_or("Validation error")
                    .to_string(),
            ))
        } else {
            Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("GitHub API error")
                    .to_string(),
            ))
        }
    }

    // ── Public API Methods ────────────────────────────────────

    /// Get a user by login name.
    pub async fn get_user(
        &self,
        access_token: &str,
        login: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        self.github_get(access_token, &format!("/users/{login}"), None)
            .await
    }

    /// Get the currently authenticated user.
    pub async fn get_authenticated_user(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        self.github_get(access_token, "/user", None).await
    }

    /// List repositories for a user.
    pub async fn list_repos(
        &self,
        access_token: &str,
        username: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/users/{username}/repos"),
            Some(&[
                ("per_page", &per_page),
                ("sort", "updated"),
                ("direction", "desc"),
            ]),
        )
        .await
    }

    /// Get a specific repository.
    pub async fn get_repo(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        self.github_get(access_token, &format!("/repos/{owner}/{repo}"), None)
            .await
    }

    /// List issues for a repository.
    pub async fn list_issues(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        state: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/issues"),
            Some(&[
                ("state", state),
                ("per_page", &per_page),
                ("sort", "updated"),
                ("direction", "desc"),
            ]),
        )
        .await
    }

    /// Get a specific issue.
    pub async fn get_issue(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        issue_number: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/issues/{issue_number}"),
            None,
        )
        .await
    }

    /// Create an issue in a repository.
    pub async fn create_issue(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let payload = serde_json::json!({
            "title": title,
            "body": body,
        });
        self.github_post(
            access_token,
            &format!("/repos/{owner}/{repo}/issues"),
            &payload,
        )
        .await
    }

    /// List pull requests for a repository.
    pub async fn list_pull_requests(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        state: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/pulls"),
            Some(&[
                ("state", state),
                ("per_page", &per_page),
                ("sort", "updated"),
                ("direction", "desc"),
            ]),
        )
        .await
    }

    /// Get a specific pull request.
    pub async fn get_pull_request(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        pr_number: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}"),
            None,
        )
        .await
    }

    /// List commits for a repository.
    pub async fn list_commits(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/commits"),
            Some(&[("per_page", &per_page)]),
        )
        .await
    }

    /// List branches for a repository.
    pub async fn list_branches(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/branches"),
            Some(&[("per_page", &per_page)]),
        )
        .await
    }

    /// List releases for a repository.
    pub async fn list_releases(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/releases"),
            Some(&[("per_page", &per_page)]),
        )
        .await
    }

    /// Search repositories via the GitHub search API.
    pub async fn search_repos(
        &self,
        access_token: &str,
        query: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            "/search/repositories",
            Some(&[("q", query), ("per_page", &per_page)]),
        )
        .await
    }

    /// Search code via the GitHub search API.
    /// Note: code search requires the query to include `repo:` or other scope qualifiers.
    pub async fn search_code(
        &self,
        access_token: &str,
        query: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            "/search/code",
            Some(&[("q", query), ("per_page", &per_page)]),
        )
        .await
    }

    /// List contributors for a repository.
    pub async fn list_contributors(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/contributors"),
            Some(&[("per_page", &per_page)]),
        )
        .await
    }

    /// Get repository content at a given path.
    /// Returns file metadata (with `content` Base64-encoded) or a directory listing.
    pub async fn get_repo_content(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        path: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let path = path.trim_start_matches('/');
        self.github_get(
            access_token,
            &format!("/repos/{owner}/{repo}/contents/{path}"),
            None,
        )
        .await
    }

    /// Close an issue by issue number.
    /// Sets state to "closed".
    pub async fn close_issue(
        &self,
        access_token: &str,
        owner: &str,
        repo: &str,
        issue_number: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let payload = serde_json::json!({ "state": "closed" });
        self.github_patch(
            access_token,
            &format!("/repos/{owner}/{repo}/issues/{issue_number}"),
            &payload,
        )
        .await
    }

    /// List repositories for the authenticated user.
    pub async fn list_my_repos(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let per_page = limit.clamp(1, 100).to_string();
        self.github_get(
            access_token,
            "/user/repos",
            Some(&[
                ("per_page", &per_page),
                ("sort", "updated"),
                ("direction", "desc"),
            ]),
        )
        .await
    }
}

#[async_trait]
impl SocialProvider for GithubProvider {
    fn identifier(&self) -> &'static str {
        "github"
    }

    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn scopes(&self) -> Vec<String> {
        vec![] // GitHub uses PAT scopes (not OAuth-style scopes)
    }

    fn max_content_length(&self) -> usize {
        0 // GitHub does not support publishing content like social posts
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
            "GitHub uses Personal Access Tokens (PAT) instead of OAuth. \
             Set GITHUB_TOKEN in your .env file."
                .into(),
        ))
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        // Use the PAT from self.token (set via Config), not the code param
        let token = self.token.clone();

        // Fetch user info to populate AuthToken fields
        let resp = self
            .http
            .get(format!("{GITHUB_API_BASE}/user"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Failed to authenticate with GitHub PAT")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let provider_user_id = json["id"]
            .as_u64()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let name = json["name"]
            .as_str()
            .unwrap_or(json["login"].as_str().unwrap_or(""))
            .to_string();
        let username = json["login"].as_str().unwrap_or("").to_string();
        let picture = json["avatar_url"].as_str().map(String::from);

        Ok(AuthToken {
            access_token: token,
            refresh_token: None,
            expires_in: None,
            provider_user_id,
            name,
            username,
            picture,
        })
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "GitHub PATs do not expire. Set GITHUB_TOKEN in your .env file.".into(),
        ))
    }

    async fn publish(
        &self,
        _access_token: &str,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "GitHub provider does not support publishing content. \
             Use GitHub Issues, Pull Requests, or other repository APIs instead."
                .into(),
        ))
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        // Return the authenticated user's repositories as "pages"
        let resp = self
            .http
            .get(format!("{GITHUB_API_BASE}/user/repos"))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/vnd.github.v3+json")
            .query(&[
                ("per_page", "100"),
                ("sort", "updated"),
                ("direction", "desc"),
            ])
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            return Err(ProviderError::Api(
                json["message"]
                    .as_str()
                    .unwrap_or("Failed to fetch repositories")
                    .to_string(),
            ));
        }

        let repos: Vec<PageInfo> = json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| PageInfo {
                        id: r["id"]
                            .as_u64()
                            .map(|id| id.to_string())
                            .unwrap_or_default(),
                        name: r["full_name"].as_str().unwrap_or("").to_string(),
                        access_token: Some(access_token.to_string()),
                        picture: r["owner"]["avatar_url"].as_str().map(String::from),
                        username: r["owner"]["login"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(repos)
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        // `page_id` can be a numeric repo ID or a "owner/repo" full name
        let json = if let Ok(_id) = page_id.parse::<u64>() {
            self.github_get(access_token, &format!("/repositories/{page_id}"), None)
                .await?
        } else {
            self.github_get(access_token, &format!("/repos/{page_id}"), None)
                .await?
        };

        Ok(PageInfo {
            id: json["id"]
                .as_u64()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            name: json["full_name"].as_str().unwrap_or("").to_string(),
            access_token: Some(access_token.to_string()),
            picture: json["owner"]["avatar_url"].as_str().map(String::from),
            username: json["owner"]["login"].as_str().map(String::from),
        })
    }

    async fn get_recent_posts(
        &self,
        access_token: &str,
        _internal_id: &str,
        _limit: u32,
    ) -> Result<Vec<ExternalPostData>, ProviderError> {
        let repos = self
            .github_get(
                access_token,
                "/user/repos",
                Some(&[
                    ("per_page", "5"),
                    ("sort", "updated"),
                    ("direction", "desc"),
                ]),
            )
            .await?;

        let gists = self
            .github_get(
                access_token,
                "/gists",
                Some(&[("per_page", "5"), ("direction", "desc")]),
            )
            .await?;

        let mut posts: Vec<ExternalPostData> = Vec::new();

        if let Some(repos_arr) = repos.as_array() {
            for repo in repos_arr {
                // Skip undated items rather than stamp "now" on them.
                let Some(created_at) = repo["created_at"]
                    .as_str()
                    .and_then(crate::social::common::parse_timestamp_opt)
                else {
                    continue;
                };
                let repo_avatar = repo["owner"]["avatar_url"].as_str().map(String::from);
                let mut media = Vec::new();
                if let Some(avatar) = &repo_avatar {
                    media.push(MediaAttachment {
                        url: avatar.clone(),
                        mime_type: "image/jpeg".to_string(),
                        alt: repo["full_name"].as_str().map(String::from),
                        poster_url: None,
                    });
                }
                posts.push(ExternalPostData {
                    platform_post_id: repo["id"]
                        .as_u64()
                        .map(|id| id.to_string())
                        .unwrap_or_default(),
                    text: repo["description"].as_str().unwrap_or("").to_string(),
                    author_name: None,
                    author_handle: Some(repo["owner"]["login"].as_str().unwrap_or("").to_string()),
                    author_avatar: repo_avatar,
                    created_at,
                    url: repo["html_url"].as_str().map(String::from),
                    media,
                    metadata: Some(serde_json::json!({
                        "type": "repo",
                        "name": repo["name"],
                        "full_name": repo["full_name"],
                        "language": repo["language"],
                        "stars": repo["stargazers_count"],
                    })),
                });
            }
        }

        if let Some(gists_arr) = gists.as_array() {
            for gist in gists_arr {
                // Skip undated items rather than stamp "now" on them.
                let Some(created_at) = gist["created_at"]
                    .as_str()
                    .and_then(crate::social::common::parse_timestamp_opt)
                else {
                    continue;
                };
                let files: Vec<String> = gist["files"]
                    .as_object()
                    .map(|obj| obj.keys().cloned().collect())
                    .unwrap_or_default();
                let gist_avatar = gist["owner"]["avatar_url"].as_str().map(String::from);
                let mut media = Vec::new();
                if let Some(avatar) = &gist_avatar {
                    media.push(MediaAttachment {
                        url: avatar.clone(),
                        mime_type: "image/jpeg".to_string(),
                        alt: gist["description"].as_str().map(String::from),
                        poster_url: None,
                    });
                }
                posts.push(ExternalPostData {
                    platform_post_id: gist["id"].as_str().unwrap_or("").to_string(),
                    text: gist["description"].as_str().unwrap_or("").to_string(),
                    author_name: None,
                    author_handle: None,
                    author_avatar: gist_avatar,
                    created_at,
                    url: gist["html_url"].as_str().map(String::from),
                    media,
                    metadata: Some(serde_json::json!({
                        "type": "gist",
                        "files": files,
                        "public": gist["public"],
                    })),
                });
            }
        }

        posts.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        posts.truncate(_limit as usize);
        Ok(posts)
    }

    fn map_error(&self, body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Invalid GitHub token. Check GITHUB_TOKEN in your .env file.".into())
        } else if status == 403 {
            if body.contains("rate limit") {
                Some("GitHub API rate limit exceeded. Try again later.".into())
            } else {
                Some("GitHub token lacks permission for this operation.".into())
            }
        } else if status == 404 {
            Some("GitHub resource not found. Check owner/repo names.".into())
        } else if status == 422 {
            Some("GitHub validation error. Check request parameters.".into())
        } else {
            None
        }
    }
}
