// ─── Discord Provider ─────────────────────────────────────────
// Uses Discord Bot API with OAuth for guild access.

use async_trait::async_trait;

use super::*;
use super::common::extract_access_token;
use crate::config::Config;
use reqwest::StatusCode;
use serde_json::{json, Value};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

pub struct DiscordProvider {
    client_id: String,
    client_secret: String,
    bot_token: String,
    http: reqwest::Client,
}

impl DiscordProvider {
    pub fn new(config: &Config) -> Self {
        let (client_id, client_secret) = config.provider_credentials("discord").unwrap_or_default();
        let bot_token = config.discord_bot_token.clone().unwrap_or_default();
        Self {
            client_id,
            client_secret,
            bot_token,
            http: shared_http(),
        }
    }

    pub async fn get_channel(
        &self,
        _access_token: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("https://discord.com/api/channels/{channel_id}"))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;
        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 401 || status == 403 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Discord API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_channel_messages(
        &self,
        _access_token: &str,
        channel_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://discord.com/api/channels/{channel_id}/messages"
            ))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .query(&[("limit", &limit.clamp(1, 100).to_string())])
            .send()
            .await?;
        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 401 || status == 403 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Discord API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    /// E7-read: deep channel history with cursor pagination, ported from
    /// ../discord-cli fetch_messages. Discord caps one request at 100 and
    /// returns newest-first; this walks the cursor (`before` older-ward for
    /// history, `after` newer-ward for incremental sync) until `max_messages`
    /// or the channel start. Output is chronological (oldest first) so
    /// callers can process a conversation top-down.
    pub async fn fetch_channel_history(
        &self,
        channel_id: &str,
        max_messages: u32,
        before: Option<&str>,
        after: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, ProviderError> {
        let mut all: Vec<serde_json::Value> = Vec::new();
        let mut remaining = max_messages;
        let mut before = before.map(str::to_string);
        let mut after = after.map(str::to_string);

        while remaining > 0 {
            let batch = remaining.min(100);
            let mut url =
                format!("{DISCORD_API_BASE}/channels/{channel_id}/messages?limit={batch}");
            if let Some(b) = &before {
                url.push_str(&format!("&before={b}"));
            }
            if let Some(a) = &after {
                url.push_str(&format!("&after={a}"));
            }

            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bot {}", self.bot_token))
                .send()
                .await?;
            let status = resp.status();
            if status == 401 || status == 403 {
                return Err(ProviderError::TokenExpired);
            }
            if !status.is_success() {
                let json: serde_json::Value = resp.json().await.unwrap_or_default();
                let msg = json["message"]
                    .as_str()
                    .unwrap_or("Discord API error")
                    .to_string();
                return Err(self.provider_error(&msg, status.as_u16()));
            }
            let batch_json: Vec<serde_json::Value> = resp.json().await?;
            let n = batch_json.len();
            if n == 0 {
                break;
            }
            // Cursor moves to the newest (after-walk) or oldest (before-walk)
            // message of this batch — Discord returns newest-first.
            let cursor = if after.is_some() {
                batch_json[0]["id"].as_str().unwrap_or_default().to_string()
            } else {
                batch_json[n - 1]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };
            all.extend(batch_json);
            remaining = remaining.saturating_sub(n as u32);
            if n < batch as usize {
                break;
            }
            if after.is_some() {
                after = Some(cursor);
            } else {
                before = Some(cursor);
            }
        }

        // Chronological order (snowflake ids sort numerically).
        all.sort_by_key(|m| {
            m["id"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        });
        Ok(all)
    }

    /// E7-read: guild-wide message search via Discord's built-in search
    /// (ported from ../discord-cli search_guild_messages). Requires the
    /// Read Message History permission in the matched channels.
    pub async fn search_guild_messages(
        &self,
        guild_id: &str,
        query: &str,
        channel_id: Option<&str>,
        limit: u32,
    ) -> Result<serde_json::Value, ProviderError> {
        // .query() percent-encodes the content term — a raw format! put '&' or
        // '#' into the URL and silently truncated/corrupted the search
        // (E7-read audit bug).
        let mut params: Vec<(&str, &str)> = vec![("content", query)];
        if let Some(cid) = channel_id {
            params.push(("channel_id", cid));
        }
        let resp = self
            .http
            .get(format!(
                "{DISCORD_API_BASE}/guilds/{guild_id}/messages/search"
            ))
            .query(&params)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if status.is_success() {
            // The response nests hit messages inside per-result groups.
            let hits: Vec<serde_json::Value> = json["messages"]
                .as_array()
                .map(|groups| {
                    groups
                        .iter()
                        .flat_map(|g| g.as_array().cloned().unwrap_or_default())
                        .filter(|m| m["hit"].as_bool().unwrap_or(false))
                        .collect()
                })
                .unwrap_or_default();
            Ok(serde_json::json!({
                "total_results": json["total_results"].as_u64().unwrap_or(0),
                "messages": hits.into_iter().take(limit as usize).collect::<Vec<_>>(),
            }))
        } else if status == 401 || status == 403 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Discord API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_guild(
        &self,
        _access_token: &str,
        guild_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!("https://discord.com/api/guilds/{guild_id}"))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;
        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 401 || status == 403 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Discord API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn get_thread_members(
        &self,
        _access_token: &str,
        thread_id: &str,
    ) -> Result<serde_json::Value, ProviderError> {
        let resp = self
            .http
            .get(format!(
                "https://discord.com/api/channels/{thread_id}/thread-members"
            ))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?;
        let status = resp.status();
        let json = resp.json().await?;
        if status.is_success() {
            Ok(json)
        } else if status == 401 || status == 403 {
            Err(ProviderError::TokenExpired)
        } else {
            let msg = json["message"]
                .as_str()
                .unwrap_or("Discord API error")
                .to_string();
            Err(self.provider_error(&msg, status.as_u16()))
        }
    }

    pub async fn send_message(
        &self,
        channel_id: &str,
        content: &str,
    ) -> Result<Value, ProviderError> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);
        let body = json!({"content": content});
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        if status.is_success() {
            Ok(v)
        } else if status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth(
                v["message"].as_str().unwrap_or("forbidden").into(),
            ))
        } else {
            Err(ProviderError::Api(
                v["message"].as_str().unwrap_or(&text).into(),
            ))
        }
    }

    pub async fn delete_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<Value, ProviderError> {
        let url = format!(
            "{}/channels/{}/messages/{}",
            DISCORD_API_BASE, channel_id, message_id
        );
        let response = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            Ok(json!({"deleted": true}))
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(ProviderError::Api(text))
        }
    }

    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<Value, ProviderError> {
        let encoded: String = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect();
        let url = format!(
            "{}/channels/{}/messages/{}/reactions/{}/@me",
            DISCORD_API_BASE, channel_id, message_id, encoded
        );
        let response = self
            .http
            .put(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        if status.is_success() || status == StatusCode::NO_CONTENT {
            Ok(json!({"success": true}))
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(ProviderError::Api(text))
        }
    }

    pub async fn get_guild_channels(&self, guild_id: &str) -> Result<Value, ProviderError> {
        let url = format!("{}/guilds/{}/channels", DISCORD_API_BASE, guild_id);
        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        if status.is_success() {
            Ok(v)
        } else if status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth("Missing permissions".into()))
        } else {
            Err(ProviderError::Api(
                v["message"].as_str().unwrap_or(&text).into(),
            ))
        }
    }

    pub async fn get_server_info(&self, guild_id: &str) -> Result<Value, ProviderError> {
        let url = format!("{}/guilds/{}?with_counts=true", DISCORD_API_BASE, guild_id);
        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        if status.is_success() {
            Ok(v)
        } else if status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth("Missing permissions".into()))
        } else {
            Err(ProviderError::Api(
                v["message"].as_str().unwrap_or(&text).into(),
            ))
        }
    }

    pub async fn create_forum_post(
        &self,
        channel_id: &str,
        name: &str,
        content: &str,
        applied_tags: &[String],
    ) -> Result<Value, ProviderError> {
        let url = format!("{}/channels/{}/threads", DISCORD_API_BASE, channel_id);
        let mut body = json!({
            "name": name,
            "message": {"content": content}
        });
        if !applied_tags.is_empty() {
            body["applied_tags"] = json!(applied_tags);
        }
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        if status.is_success() {
            Ok(v)
        } else if status == StatusCode::FORBIDDEN {
            Err(ProviderError::Auth("Missing permissions".into()))
        } else {
            Err(ProviderError::Api(
                v["message"].as_str().unwrap_or(&text).into(),
            ))
        }
    }
}

#[async_trait]
impl SocialProvider for DiscordProvider {
    fn identifier(&self) -> &'static str {
        "discord"
    }

    fn name(&self) -> &'static str {
        "Discord"
    }

    fn scopes(&self) -> Vec<String> {
        vec!["identify".into(), "guilds".into()]
    }

    fn max_content_length(&self) -> usize {
        1980
    }

    /// The settings this provider needs to publish.
    ///
    /// Discord publishes to a channel. `publish` reads `post.settings["channel"]` and errors with "Missing channel in settings" when it is absent.
    ///
    /// The create path checks this list and refuses the post with a
    /// message naming the missing field, instead of the post going to
    /// `state='error'` in the scheduler with no reason shown.
    fn required_settings(&self) -> &'static [&'static str] {
        &["channel"]
    }

    fn editor_type(&self) -> EditorType {
        EditorType::Markdown
    }

    async fn generate_auth_url(
        &self,
        state: &str,
        _code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<AuthUrlResponse, ProviderError> {
        let params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", "identify guilds bot"),
            ("state", state),
            ("permissions", "377957124096"),
            ("integration_type", "0"),
        ];

        let url = url::Url::parse_with_params("https://discord.com/oauth2/authorize", &params)
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
        let auth_bytes = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );
        let auth_header = format!("Basic {auth_bytes}");

        let params: Vec<(&str, &str)> = vec![
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ];

        let resp = self
            .http
            .post("https://discord.com/api/oauth2/token")
            .header("Authorization", &auth_header)
            .form(&params)
            .send()
            .await?;

        let (access_token, json) = extract_access_token(resp, "discord")
            .await
            .map_err(ProviderError::Auth)?;

        let refresh_token = json["refresh_token"].as_str().map(String::from);
        let expires_in = json["expires_in"].as_u64().map(|v| v as u32);
        let guild_id = json["guild"]["id"].as_str().unwrap_or("").to_string();

        let me: serde_json::Value = self
            .http
            .get("https://discord.com/api/oauth2/@me")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json()
            .await?;

        let app = &me["application"];

        Ok(AuthToken {
            access_token,
            refresh_token,
            expires_in,
            provider_user_id: guild_id,
            name: app["name"].as_str().unwrap_or("Discord Server").to_string(),
            username: app["bot"]["username"].as_str().unwrap_or("").to_string(),
            picture: Some(format!(
                "https://cdn.discordapp.com/avatars/{}/{}.png",
                app["bot"]["id"].as_str().unwrap_or(""),
                app["bot"]["avatar"].as_str().unwrap_or("")
            )),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, ProviderError> {
        let auth_bytes = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", self.client_id, self.client_secret),
        );
        let auth_header = format!("Basic {auth_bytes}");

        let params: Vec<(&str, &str)> = vec![
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let resp = self
            .http
            .post("https://discord.com/api/oauth2/token")
            .header("Authorization", &auth_header)
            .form(&params)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // ─── Why the empty token is no longer accepted ───────────
        // This was the one site in the tree that read
        // `json["access_token"].as_str().unwrap_or("")`, so a refused
        // refresh SUCCEEDED with an empty token. Discord answers a revoked
        // refresh token with `{"error":"invalid_grant"}` and no
        // `access_token`, and the whole body was discarded: the integration
        // stayed "connected", the dashboard showed it healthy, and the
        // failure surfaced hours later as an opaque publish error with no
        // link back to the refresh that actually broke. AGENTS.md §6.3 is
        // explicit that a refresh path must never write a bad token, and an
        // empty string is the worst shape of a bad token because it is not
        // an error at all.
        if !status.is_success() {
            return Err(ProviderError::Api(format!(
                "Discord token refresh failed (HTTP {status}): {body}"
            )));
        }
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            ProviderError::Api(format!(
                "Discord token refresh response was not JSON ({e}). Body: {body}"
            ))
        })?;

        // An empty access_token is a FAILURE, not a value. The old
        // `unwrap_or("")` here is what made a refused refresh look like a
        // successful one.
        let access_token = json["access_token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                ProviderError::Auth(format!(
                    "Discord token refresh returned no access_token. Body: {body}"
                ))
            })?
            .to_string();

        Ok(AuthToken {
            access_token,
            refresh_token: json["refresh_token"].as_str().map(String::from),
            expires_in: json["expires_in"].as_u64().map(|v| v as u32),
            provider_user_id: String::new(),
            name: String::new(),
            username: String::new(),
            picture: None,
        })
    }

    async fn pages(&self, access_token: &str) -> Result<Vec<PageInfo>, ProviderError> {
        let channels: serde_json::Value = self
            .http
            .get(format!(
                "https://discord.com/api/guilds/{access_token}/channels"
            ))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await?
            .json()
            .await?;

        Ok(channels
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|c| {
                        c["type"].as_i64() == Some(0)
                            || c["type"].as_i64() == Some(5)
                            || c["type"].as_i64() == Some(15)
                    })
                    .map(|c| PageInfo {
                        id: c["id"].as_str().unwrap_or("").to_string(),
                        name: c["name"].as_str().unwrap_or("").to_string(),
                        access_token: None,
                        picture: None,
                        username: None,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }

    async fn publish(
        &self,
        access_token: &str,
        post: &PostContent,
    ) -> Result<PublishResult, ProviderError> {
        let channel_id = post.settings["channel"]
            .as_str()
            .ok_or_else(|| ProviderError::InvalidRequest("Missing channel in settings".into()))?;

        let mut payload = serde_json::json!({
            "content": post.content
        });

        if !post.media.is_empty() {
            payload["embeds"] = serde_json::json!([{
                "image": { "url": post.media[0].url }
            }]);
        }

        payload["allowed_mentions"] = serde_json::json!({"parse": ["users", "roles"]});

        let resp = self
            .http
            .post(format!(
                "https://discord.com/api/channels/{channel_id}/messages"
            ))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&payload)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        Ok(PublishResult {
            platform_post_id: json["id"].as_str().unwrap_or("").to_string(),
            platform_post_url: Some(format!(
                "https://discord.com/channels/{access_token}/{channel_id}/{}",
                json["id"].as_str().unwrap_or("")
            )),
            status: "published".into(),
        })
    }

    fn map_error(&self, body: &str, _status: u16) -> Option<String> {
        if body.contains("50001") {
            Some("Bot doesn't have access to this channel".into())
        } else if body.contains("50013") {
            Some("Bot lacks permission to send messages".into())
        } else if body.contains("10003") {
            Some("Channel no longer exists".into())
        } else {
            None
        }
    }

    async fn targets(&self, _access_token: &str) -> Result<Vec<TargetInfo>, ProviderError> {
        if self.bot_token.is_empty() {
            return Ok(vec![]);
        }

        // Fetch guilds the bot is in
        let guilds_resp = self
            .http
            .get(format!("{DISCORD_API_BASE}/users/@me/guilds"))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await;

        let guilds: Value = match guilds_resp {
            Ok(resp) if resp.status().is_success() => match resp.json().await {
                Ok(v) => v,
                Err(_) => return Ok(vec![]),
            },
            _ => return Ok(vec![]),
        };

        let Some(guild_array) = guilds.as_array() else {
            return Ok(vec![]);
        };

        let mut targets = Vec::new();

        for guild in guild_array {
            let guild_id = guild["id"].as_str().unwrap_or("").to_string();
            let guild_name = guild["name"].as_str().unwrap_or("Unknown").to_string();

            if guild_id.is_empty() {
                continue;
            }

            // Fetch channels for this guild
            let channels_resp = self
                .http
                .get(format!("{DISCORD_API_BASE}/guilds/{guild_id}/channels"))
                .header("Authorization", format!("Bot {}", self.bot_token))
                .send()
                .await;

            let channels: Value = match channels_resp {
                Ok(resp) if resp.status().is_success() => match resp.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                _ => continue,
            };

            let Some(channel_array) = channels.as_array() else {
                continue;
            };

            for channel in channel_array {
                // Filter to text channels only (type 0 = text, type 5 = announcement)
                let channel_type = channel["type"].as_i64();
                if channel_type != Some(0) && channel_type != Some(5) {
                    continue;
                }

                let channel_id = channel["id"].as_str().unwrap_or("").to_string();
                let channel_name = channel["name"].as_str().unwrap_or("unknown").to_string();

                if channel_id.is_empty() {
                    continue;
                }

                targets.push(TargetInfo {
                    id: channel_id,
                    name: format!("#{channel_name}"),
                    target_type: "channel".into(),
                    picture: None,
                    metadata: Some(json!({
                        "guild_id": guild_id,
                        "guild_name": guild_name,
                        "channel_type": channel_type,
                    })),
                });

                if targets.len() >= 50 {
                    return Ok(targets);
                }
            }
        }

        Ok(targets)
    }
}
