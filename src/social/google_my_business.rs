// ─── Google My Business Provider ────────────────────────────
// Uses Google OAuth 2.0 (same credentials as google/youtube) +
// My Business API v4 for local posts.
// Supports: OAuth flow, account listing, local post publishing.

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct GoogleMyBusinessProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl GoogleMyBusinessProvider {
    pub fn new(config: &Config) -> Self {
        // GMB reuses the same Google OAuth credentials as the "google" provider
        let (client_id, client_secret) = config.provider_credentials("google").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }
}

#[async_trait]
impl SocialProvider for GoogleMyBusinessProvider {
    fn identifier(&self) -> &'static str {
        "google_my_business"
    }

    fn name(&self) -> &'static str {
        "Google My Business"
    }

    fn scopes(&self) -> Vec<String> {
        vec!["https://www.googleapis.com/auth/business.manage".into()]
    }

    fn max_content_length(&self) -> usize {
        3000
    }

    fn editor_type(&self) -> EditorType {
        EditorType::Normal
    }

    fn uses_oauth(&self) -> bool {
        true
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let scope = self.scopes().join(" ");
        let params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", scope.as_str()),
            ("state", state),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ];

        let url =
            url::Url::parse_with_params("https://accounts.google.com/o/oauth2/v2/auth", &params)
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
        let params: Vec<(&str, &str)> = vec![
            ("code", code),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];

        let resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error_description"]
                .as_str()
                .or_else(|| json["error"].as_str())
                .unwrap_or("Token exchange failed")
                .to_string();
            return Err(ProviderError::Auth(msg));
        }

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
            .to_string();
        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);

        // Get user info from Google
        let user: serde_json::Value = self
            .http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
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
            name: user["name"].as_str().unwrap_or("").to_string(),
            username: user["email"].as_str().unwrap_or("").to_string(),
            picture: user["picture"].as_str().map(String::from),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        super::google_oauth::refresh(
            &self.http,
            &self.client_id,
            &self.client_secret,
            refresh_token,
        )
        .await
    }

    /// Publish a local post to a Google Business Profile location.
    /// Simplified implementation — GMB API access requires special whitelisting.
    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // Step 1: List accounts
        let accounts_resp = self
            .http
            .get("https://mybusinessaccountmanagement.googleapis.com/v1/accounts")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        if accounts_resp.status() == 401 {
            return Err(ProviderError::TokenExpired);
        }

        let accounts_json: serde_json::Value = accounts_resp.json().await?;
        let accounts = accounts_json["accounts"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if accounts.is_empty() {
            return Err(ProviderError::Api(
                "No Google Business Profile accounts found".into(),
            ));
        }

        let account_name = accounts[0]["name"].as_str().unwrap_or("").to_string();

        if account_name.is_empty() {
            return Err(ProviderError::Api(
                "Could not determine GMB account name".into(),
            ));
        }

        // Step 2: Get locations for the account
        let locations_resp = self
            .http
            .get(format!(
                "https://mybusiness.googleapis.com/v4/{account_name}/locations"
            ))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        if locations_resp.status() == 401 {
            return Err(ProviderError::TokenExpired);
        }

        let locations_json: serde_json::Value = locations_resp.json().await?;
        let locations = locations_json["locations"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if locations.is_empty() {
            return Err(ProviderError::Api(
                "No Google Business Profile locations found".into(),
            ));
        }

        let location_name = locations[0]["name"].as_str().unwrap_or("").to_string();

        // Step 3: Create a local post
        let post_body = serde_json::json!({
            "summary": post.content,
            "languageCode": "en-US",
        });

        let post_resp = self
            .http
            .post(format!(
                "https://mybusiness.googleapis.com/v4/{location_name}/localPosts"
            ))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&post_body)
            .send()
            .await?;

        let post_status = post_resp.status();
        if post_status == 401 {
            return Err(ProviderError::TokenExpired);
        }

        if !post_status.is_success() {
            let err_json: serde_json::Value = post_resp.json().await.unwrap_or_default();
            let msg = err_json["error"]["message"]
                .as_str()
                .unwrap_or("Failed to create GMB local post")
                .to_string();
            return Err(self.provider_error(&msg, post_status.as_u16()));
        }

        let result: serde_json::Value = post_resp.json().await?;
        let post_id = result["name"].as_str().unwrap_or("").to_string();

        Ok(PublishResult {
            platform_post_id: post_id,
            platform_post_url: None,
            status: "published".into(),
        })
    }

    /// List GMB accounts as pages.
    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let resp = self
            .http
            .get("https://mybusinessaccountmanagement.googleapis.com/v1/accounts")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Failed to fetch GMB accounts")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }

        let accounts = json["accounts"].as_array().cloned().unwrap_or_default();

        Ok(accounts
            .iter()
            .map(|acc| {
                let acc_name = acc["name"].as_str().unwrap_or("").to_string();
                let account_id = acc_name.split('/').next_back().unwrap_or("").to_string();
                PageInfo {
                    id: account_id,
                    name: acc["accountName"]
                        .as_str()
                        .unwrap_or("GMB Account")
                        .to_string(),
                    access_token: Some(access_token.to_string()),
                    picture: None,
                    username: None,
                }
            })
            .collect())
    }

    /// Fetch page info by account ID.
    async fn fetch_page_info(
        &self,
        access_token: &str,
        page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://mybusinessaccountmanagement.googleapis.com/v1/accounts/{page_id}"
            ))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Failed to fetch GMB account")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }

        Ok(PageInfo {
            id: json["name"]
                .as_str()
                .and_then(|n| n.split('/').next_back())
                .unwrap_or("")
                .to_string(),
            name: json["accountName"]
                .as_str()
                .unwrap_or("GMB Account")
                .to_string(),
            access_token: Some(access_token.to_string()),
            // The account-management endpoint answers `accountName` and no
            // photo - a GMB account is not something with a face. The Google
            // ACCOUNT behind it has one, published at the same OpenID
            // userinfo endpoint this provider's own `exchange_code` reads, so
            // the avatar is available and the failure to return it was not a
            // limitation of the API.
            //
            // It matters more here than it looks: refresh is the only repair
            // path for a connected row, and it writes
            // `profile_picture = COALESCE(?, profile_picture)`, so a row that
            // came back with no photo can never gain one.
            picture: super::google_oauth::profile_picture(&self.http, access_token).await,
            username: None,
        })
    }

    async fn comment(
        &self,
        _access_token: &str,
        _post_id: &str,
        _last_comment_id: Option<&str>,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Google My Business does not support programmatic commenting".into(),
        ))
    }

    fn map_error(&self, _body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Google token expired. Re-authenticate via Google OAuth.".into())
        } else if status == 403 {
            Some("Google My Business API access not configured. Check API whitelisting.".into())
        } else {
            None
        }
    }
}
