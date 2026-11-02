use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::*;
use crate::config::Config;
use crate::wa::chats::{list_chats, list_contacts};
use crate::wa::WhaClient;

/// WhatsApp Web provider, backed by the native `wa-rs` client.
///
/// The legacy Go `wacli` sidecar (formerly `services/whatsapp_daemon`)
/// has been removed — `wa-rs` is the sole implementation. If
/// `WHATSAPP_STORE_DIR` is unset, the provider is inert (returns
/// errors on publish/auth).
pub struct WhatsAppProvider {
    store_dir: PathBuf,
    wa_client: Option<Arc<Mutex<WhaClient>>>,
    /// E6: Cloud API path, active when WHATSAPP_CLOUD_* creds are set AND a
    /// post's settings say auth_method=cloud. None keeps the Web-session path.
    cloud: Option<crate::social::whatsapp_cloud::WhatsAppCloudProvider>,
}

impl WhatsAppProvider {
    pub fn new(config: &Config, wa_client: Option<Arc<Mutex<WhaClient>>>) -> Self {
        // ONE default, one place: `config.whatsapp_store_dir` is always Some
        // now (`config::DEFAULT_WHATSAPP_STORE_DIR`), so the fallback here is
        // unreachable and only exists to keep this constructor total.
        let store_dir = PathBuf::from(
            config
                .whatsapp_store_dir
                .clone()
                .unwrap_or_else(|| crate::config::DEFAULT_WHATSAPP_STORE_DIR.into()),
        );
        Self {
            store_dir,
            wa_client,
            cloud: crate::social::whatsapp_cloud::WhatsAppCloudProvider::from_config(config),
        }
    }

    /// Returns the configured on-disk store directory for diagnostics.
    pub fn store_dir(&self) -> &std::path::Path {
        &self.store_dir
    }
}

#[async_trait]
impl SocialProvider for WhatsAppProvider {
    fn identifier(&self) -> &'static str {
        "whatsapp"
    }

    fn name(&self) -> &'static str {
        "WhatsApp"
    }

    fn scopes(&self) -> Vec<String> {
        vec![]
    }

    fn max_content_length(&self) -> usize {
        4096
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
        Ok(AuthUrlResponse { url: String::new() })
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<AuthToken, ProviderError> {
        let Some(ref wa_client) = self.wa_client else {
            return Err(ProviderError::Auth(
                "WhatsApp client not initialized. Set WHATSAPP_STORE_DIR and pair via wa_pair_code.".into(),
            ));
        };
        let locked = wa_client.lock().await;
        if locked.is_authenticated() {
            let jid = locked
                .inner()
                .get_pn()
                .await
                .map(|j| j.to_string())
                .unwrap_or_else(|| "unknown".into());
            return Ok(AuthToken {
                access_token: jid.clone(),
                refresh_token: None,
                expires_in: Some(999_999_999),
                provider_user_id: jid.clone(),
                name: format!("WhatsApp ({jid})"),
                username: jid,
                picture: None,
            });
        }
        Err(ProviderError::Auth(
            "WhatsApp not authenticated. Use wa_auth_status / wa_pair_code tools to link your device.".into(),
        ))
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthToken, ProviderError> {
        Err(ProviderError::Auth("WhatsApp tokens do not expire".into()))
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        // E6: Cloud path — settings.auth_method=cloud routes a scheduled post
        // through a template-message send (the compliant outbound path).
        if post.settings.get("auth_method").and_then(|v| v.as_str()) == Some("cloud") {
            let cloud = self.cloud.as_ref().ok_or_else(|| {
                ProviderError::Api(
                    "auth_method=cloud but WHATSAPP_CLOUD_* credentials are not set \
                     (WHATSAPP_CLOUD_TOKEN, WHATSAPP_CLOUD_PHONE_NUMBER_ID, WHATSAPP_CLOUD_WABA_ID)"
                        .into(),
                )
            })?;
            let template = post
                .settings
                .get("template")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ProviderError::Api(
                        "Cloud publish needs settings.template (the approved template name)\
                         plus optional template_language and template_components"
                            .into(),
                    )
                })?;
            let language = post
                .settings
                .get("template_language")
                .and_then(|v| v.as_str())
                .unwrap_or("en");
            let components = post
                .settings
                .get("template_components")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            let resp = cloud
                .send_template(access_token, template, language, components)
                .await?;
            let msg_id = resp["messages"][0]["id"]
                .as_str()
                .unwrap_or("sent")
                .to_string();
            return Ok(PublishResult {
                platform_post_id: msg_id,
                platform_post_url: None,
                status: "published".into(),
            });
        }

        let to = access_token;

        if !post.media.is_empty() {
            return Err(ProviderError::Api(
                "WhatsApp media posting not yet supported via server mode".into(),
            ));
        }

        let Some(ref wa_client) = self.wa_client else {
            return Err(ProviderError::Api(
                "WhatsApp client not initialized. Set WHATSAPP_STORE_DIR and pair via wa_pair_code.".into(),
            ));
        };

        let jid = wa_rs::Jid::pn(to);
        let msg_id = crate::wa::messages::send_text(wa_client, &jid, &post.content)
            .await
            .map_err(|e| ProviderError::Api(format!("WhatsApp send failed: {e}")))?;

        Ok(PublishResult {
            platform_post_id: msg_id,
            platform_post_url: None,
            status: "published".into(),
        })
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn targets(&self, _access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        let Some(ref wa_client) = self.wa_client else {
            return Ok(vec![]);
        };

        {
            let locked = wa_client.lock().await;
            if !locked.is_authenticated() {
                return Ok(vec![]);
            }
        }

        let mut targets: Vec<TargetInfo> = Vec::new();

        if let Ok(contacts) = list_contacts(wa_client, Some(500)).await {
            for contact in contacts {
                let display_name = if !contact.name.is_empty() {
                    contact.name
                } else if !contact.push_name.is_empty() {
                    contact.push_name.clone()
                } else {
                    contact
                        .jid
                        .split('@')
                        .next()
                        .unwrap_or(&contact.jid)
                        .to_string()
                };

                targets.push(TargetInfo {
                    id: contact.jid,
                    name: display_name,
                    target_type: "contact".into(),
                    picture: None,
                    metadata: None,
                });
            }
        }

        if let Ok(chats) = list_chats(wa_client, Some(500)).await {
            for chat in chats {
                if chat.jid.ends_with("@g.us") {
                    let name = if chat.name.is_empty() {
                        chat.jid.split('@').next().unwrap_or(&chat.jid).to_string()
                    } else {
                        chat.name
                    };
                    targets.push(TargetInfo {
                        id: chat.jid,
                        name,
                        target_type: "group".into(),
                        picture: None,
                        metadata: None,
                    });
                }
            }
        }

        Ok(targets)
    }
}
