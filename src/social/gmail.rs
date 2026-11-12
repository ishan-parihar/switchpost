// ─── Gmail Provider ──────────────────────────────────────────
// Uses Google OAuth 2.0 + Gmail API v1.
// Reuses YouTube client credentials from YOUTUBE_CLIENT_ID / YOUTUBE_CLIENT_SECRET.

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct GmailProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl GmailProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("youtube").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    /// List messages in the user's mailbox.
    pub async fn list_messages(
        &self,
        access_token: &str,
        max_results: u32,
        query: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let mr = max_results.clamp(1, 500).to_string();
        let mut params: Vec<(&str, &str)> = vec![("maxResults", &mr)];
        if let Some(q) = query {
            params.push(("q", q));
        }
        let resp = self
            .http
            .get("https://gmail.googleapis.com/gmail/v1/users/me/messages")
            .query(&params)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Get a single message by ID.
    pub async fn get_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}"
            ))
            .query(&[("format", "full")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Send an email via Gmail API (raw MIME base64-encoded).
    pub async fn send_message(
        &self,
        access_token: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let raw_mime = format!(
            "From: me\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{body}"
        );
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            raw_mime.as_bytes(),
        );
        let payload = serde_json::json!({ "raw": encoded });
        let resp = self
            .http
            .post("https://gmail.googleapis.com/gmail/v1/users/me/messages/send")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&payload)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// List Gmail labels.
    pub async fn list_labels(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://gmail.googleapis.com/gmail/v1/users/me/labels")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Get a full thread by ID.
    pub async fn get_thread(
        &self,
        access_token: &str,
        thread_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/{thread_id}"
            ))
            .query(&[("format", "full")])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// Search messages with a query string.
    pub async fn search_messages(
        &self,
        access_token: &str,
        query: &str,
        max_results: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        self.list_messages(access_token, max_results, Some(query))
            .await
    }

    /// Get the Gmail profile (email address).
    pub async fn get_profile(
        &self,
        access_token: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// The Google account's profile photo. Delegates to the ONE shared
    /// implementation for the whole Google family (see
    /// `super::google_oauth::profile_picture`) so that gmail, calendar,
    /// drive, GMB and youtube cannot drift on the same rule.
    pub async fn get_profile_picture(&self, access_token: &str) -> Option<String> {
        super::google_oauth::profile_picture(&self.http, access_token).await
    }
}

#[async_trait]
impl SocialProvider for GmailProvider {
    fn identifier(&self) -> &'static str {
        "gmail"
    }

    fn name(&self) -> &'static str {
        "Gmail"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "https://www.googleapis.com/auth/gmail.readonly".into(),
            "https://www.googleapis.com/auth/gmail.send".into(),
            "https://www.googleapis.com/auth/gmail.labels".into(),
            "https://www.googleapis.com/auth/gmail.modify".into(),
        ]
    }

    fn max_content_length(&self) -> usize {
        0
    }

    fn needs_cron_refresh(&self) -> bool {
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
        super::google_oauth::exchange_code(
            &self.http,
            "gmail",
            &self.client_id,
            &self.client_secret,
            code,
            redirect_uri,
        )
        .await
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

    async fn publish(
        &self,
        _access_token: &str,
        _post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        Err(ProviderError::Api(
            "Gmail provider does not support publishing. Use send_message instead.".into(),
        ))
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        let profile = self.get_profile(access_token).await?;
        // The Gmail profile endpoint answers `emailAddress` and `historyId`
        // and nothing else - it has NO picture field. Hardcoding `None`
        // here meant the only repair path for a connected Gmail channel
        // (refresh, whose default reconnect() delegates to this) could
        // never fill one in, so a channel connected before the avatar
        // existed stayed avatar-less forever and the dashboard showed a
        // generic icon where every other Google-family channel showed a
        // photo.
        //
        // The picture is on the SAME Google account, published at the
        // OpenID userinfo endpoint that `exchange_code` already reads for
        // the initial connect - so the two paths now agree, which is the
        // invariant that was broken (a provider reading its own profile
        // two different ways). A failure here is NOT an error: the channel
        // is still perfectly usable, it just has no photo.
        let email = profile["emailAddress"].as_str().unwrap_or_default();
        let picture = self.get_profile_picture(access_token).await;
        Ok(PageInfo {
            id: email.to_string(),
            name: if email.is_empty() {
                "Gmail".to_string()
            } else {
                email.to_string()
            },
            access_token: Some(access_token.to_string()),
            picture,
            username: if email.is_empty() {
                None
            } else {
                Some(email.to_string())
            },
        })
    }

    fn map_error(&self, _body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Gmail token expired. Re-authenticate via Google OAuth.".into())
        } else if status == 403 {
            Some("Gmail API access forbidden. Check token scopes.".into())
        } else if status == 429 {
            Some("Gmail API rate limit exceeded. Try again later.".into())
        } else {
            None
        }
    }
}
