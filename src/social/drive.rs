// ─── Google Drive Provider ───────────────────────────────────
// Uses Google OAuth 2.0 + Google Drive API v3.
// Reuses YouTube client credentials from YOUTUBE_CLIENT_ID / YOUTUBE_CLIENT_SECRET.

use async_trait::async_trait;

use super::*;
use crate::config::Config;

pub struct DriveProvider {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl DriveProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("youtube").unwrap_or_default();
        Self {
            client_id,
            client_secret,
            http: shared_http(),
        }
    }

    /// List files/folders in Drive.
    pub async fn list_files(
        &self,
        access_token: &str,
        page_size: u32,
        query: Option<&str>,
    ) -> Result<serde_json::Value, ProviderError> {
        let ps = page_size.clamp(1, 1000).to_string();
        let mut params: Vec<(&str, &str)> = vec![
            ("pageSize", &ps),
            ("fields", "files(id,name,mimeType,size,createdTime,modifiedTime,webViewLink,owners/displayName),nextPageToken"),
        ];
        let q = if let Some(q) = query {
            q.to_string()
        } else {
            "trashed = false".to_string()
        };
        params.push(("q", &q));
        let resp = self
            .http
            .get("https://www.googleapis.com/drive/v3/files")
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

    /// Get file metadata by ID.
    pub async fn get_file(
        &self,
        access_token: &str,
        file_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{file_id}"
            ))
            .query(&[(
                "fields",
                "id,name,mimeType,size,createdTime,modifiedTime,webViewLink,owners,description,starred,trashed",
            )])
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

    /// Search files with a full query string (Drive API v3 format).
    pub async fn search_files(
        &self,
        access_token: &str,
        query: &str,
        page_size: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        self.list_files(access_token, page_size, Some(query)).await
    }

    /// List folders in Drive.
    pub async fn list_folders(
        &self,
        access_token: &str,
        page_size: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        self.list_files(
            access_token,
            page_size,
            Some("mimeType='application/vnd.google-apps.folder' and trashed = false"),
        )
        .await
    }

    /// Get file metadata with additional fields.
    pub async fn get_file_metadata(
        &self,
        access_token: &str,
        file_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        self.get_file(access_token, file_id).await
    }

    /// Export a Google Docs/Sheets file to a different MIME type.
    /// Returns the exported content as base64-encoded data.
    pub async fn export_file(
        &self,
        access_token: &str,
        file_id: &str,
        mime_type: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let url = format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}/export?mimeType={}",
            urlencoding::encode(mime_type)
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;
        let status = resp.status();
        if status == 401 {
            return Err(ProviderError::TokenExpired);
        }
        if !status.is_success() {
            let json: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Export failed")
                .to_string();
            return Err(self.provider_error(&msg, status.as_u16()));
        }
        let bytes = resp.bytes().await?.to_vec();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        Ok(serde_json::json!({
            "file_id": file_id,
            "mime_type": mime_type,
            "content_base64": encoded,
            "size_bytes": bytes.len(),
        }))
    }
}

#[async_trait]
impl SocialProvider for DriveProvider {
    fn identifier(&self) -> &'static str {
        "drive"
    }

    fn name(&self) -> &'static str {
        "Google Drive"
    }

    fn scopes(&self) -> Vec<String> {
        vec![
            "https://www.googleapis.com/auth/drive.readonly".into(),
            "https://www.googleapis.com/auth/drive.file".into(),
            "https://www.googleapis.com/auth/drive.metadata.readonly".into(),
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
            "drive",
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
            "Drive provider does not support publishing content.".into(),
        ))
    }

    async fn fetch_page_info(
        &self,
        access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        // Fetch user info as the identity
        let user: serde_json::Value = self
            .http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json()
            .await?;
        Ok(PageInfo {
            id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["name"].as_str().unwrap_or("Google Drive").to_string(),
            access_token: Some(access_token.to_string()),
            picture: user["picture"].as_str().map(String::from),
            username: user["email"].as_str().map(String::from),
        })
    }

    fn map_error(&self, _body: &str, status: u16) -> Option<String> {
        if status == 401 {
            Some("Drive token expired. Re-authenticate via Google OAuth.".into())
        } else if status == 403 {
            Some("Drive API access forbidden. Check token scopes.".into())
        } else if status == 429 {
            Some("Drive API rate limit exceeded. Try again later.".into())
        } else {
            None
        }
    }
}
