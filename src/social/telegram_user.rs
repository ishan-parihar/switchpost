use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::config::Config;
use crate::services::telegram_client::TelegramClientManager;

/// Telegram User Account provider via Grammers (MTProto client).
///
/// Unlike TelegramBotProvider (which uses Bot API with token),
/// this provider uses Telegram MTProto directly for full user
/// account access:
/// - Send to any peer (username, phone, group)
/// - Access to dialogs, contacts, search
/// - Real user account, not a bot
///
/// Authentication is done via the `tu_request_code` and `tu_sign_in`
/// MCP tools. This provider's `exchange_code` simply checks whether
/// authentication is complete.
pub struct TelegramUserProvider {
    client_manager: Option<Arc<TelegramClientManager>>,
}

impl TelegramUserProvider {
    pub fn new(_config: &Config, client_manager: Option<Arc<TelegramClientManager>>) -> Self {
        Self { client_manager }
    }
}

#[async_trait]
impl SocialProvider for TelegramUserProvider {
    fn identifier(&self) -> &'static str {
        "telegram-user"
    }

    fn name(&self) -> &'static str {
        "Telegram User"
    }

    fn tooltip(&self) -> Option<&'static str> {
        Some("Send messages from your personal Telegram account via MTProto (Grammers)")
    }

    fn scopes(&self) -> Vec<String> {
        vec![]
    }

    fn max_content_length(&self) -> usize {
        4096
    }

    /// The settings this provider needs to publish.
    ///
    /// Telegram-user sends to a peer. `publish` reads `post.settings["target"]` and errors with "No target peer specified".
    ///
    /// The create path checks this list and refuses the post with a
    /// message naming the missing field, instead of the post going to
    /// `state='error'` in the scheduler with no reason shown.
    fn required_settings(&self) -> &'static [&'static str] {
        &["target"]
    }

    fn uses_oauth(&self) -> bool {
        false
    }

    fn one_time_token(&self) -> bool {
        true
    }

    fn editor_type(&self) -> EditorType {
        EditorType::Normal
    }

    async fn generate_auth_url(
        &self,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        // MTProto-based auth — no OAuth URL
        Ok(AuthUrlResponse { url: String::new() })
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        let mgr = self.client_manager.as_ref().ok_or_else(|| {
            ProviderError::Api(
                "Telegram user client not available in this process. Set TELEGRAM_API_ID and \
                 TELEGRAM_API_HASH."
                    .into(),
            )
        })?;

        if mgr.credentials().is_none() {
            return Err(ProviderError::Api(
                "Telegram user client has no API credentials. Set TELEGRAM_API_ID and \
                 TELEGRAM_API_HASH (from https://my.telegram.org) in ~/.switchpost/.env."
                    .into(),
            ));
        }

        let is_auth = mgr
            .is_authenticated()
            .await
            .map_err(|e| ProviderError::Api(format!("Telegram auth check failed: {e}")))?;

        if !is_auth {
            return Err(ProviderError::Auth(
                "Telegram user not authenticated. Use the tu_request_code and \
                 tu_sign_in MCP tools to sign in first."
                    .into(),
            ));
        }

        // Get user info for the token payload
        let info = mgr
            .user_info()
            .await
            .map_err(|e| ProviderError::Api(format!("Failed to get user info: {e}")))?;

        let user_id = info["id"].as_i64().unwrap_or(0).to_string();
        let name = info["name"].as_str().unwrap_or("Telegram User").to_string();
        let username = info["username"].as_str().unwrap_or("").to_string();

        Ok(AuthToken {
            access_token: user_id.clone(),
            refresh_token: None,
            expires_in: Some(999_999_999),
            provider_user_id: user_id,
            name,
            username,
            picture: None,
        })
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth(
            "Telegram user tokens do not expire".into(),
        ))
    }

    async fn publish(
        &self,
        _access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let mgr = self.client_manager.as_ref().ok_or_else(|| {
            ProviderError::Api(
                "Telegram user client not available in this process. Set TELEGRAM_API_ID and \
                 TELEGRAM_API_HASH."
                    .into(),
            )
        })?;

        if mgr.credentials().is_none() {
            return Err(ProviderError::Api(
                "Telegram user client has no API credentials. Set TELEGRAM_API_ID and \
                 TELEGRAM_API_HASH (from https://my.telegram.org) in ~/.switchpost/.env."
                    .into(),
            ));
        }

        let target = post
            .settings
            .get("target")
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                ProviderError::Api(
                    "No target peer specified. Add 'target' field in post settings \
                 (e.g., username, phone number, or chat name)."
                        .into(),
                )
            })?;

        if post.media.is_empty() {
            let result = mgr
                .send_message(target, &post.content)
                .await
                .map_err(|e| ProviderError::Api(format!("Telegram send failed: {e}")))?;

            let msg_id = result["id"].as_i64().unwrap_or(0);

            Ok(PublishResult {
                platform_post_id: msg_id.to_string(),
                platform_post_url: None,
                status: "published".into(),
            })
        } else {
            Err(ProviderError::Api(
                "Media posting not yet supported for Telegram user accounts via Grammers. \
                 Text messages are supported."
                    .into(),
            ))
        }
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn targets(&self, _access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        let mgr = match self.client_manager.as_ref() {
            Some(mgr) => mgr,
            None => return Ok(vec![]),
        };

        let result = match mgr.list_dialogs_detailed().await {
            Ok(result) => result,
            Err(_) => return Ok(vec![]),
        };

        let dialogs = result["dialogs"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|d| {
                        let id = d["id"].to_string();
                        let name = d["name"].as_str().unwrap_or("Unknown").to_string();
                        let target_type = d["type"].as_str().unwrap_or("peer").to_string();

                        TargetInfo {
                            id,
                            name,
                            target_type,
                            picture: None,
                            metadata: None,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(dialogs)
    }
}
