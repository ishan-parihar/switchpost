// ─── MCP Integration Tools ────────────────────────────────────
// Tool handlers for social media channel management.
// Thin wrappers over the shared IntegrationService.

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::services::integrations::IntegrationService;
use crate::social::TargetInfo;

/// The `(auth_url, instructions)` pair a one-time-token connect returns.
///
/// Extracted from `connect_integration` so the RULE is testable on its own:
/// the text must name the flow THAT provider actually uses. Every
/// one-time-token channel once answered "Open Telegram, start a chat with
/// this bot", which is only true of telegram-bot - for telegram-user (MTProto,
/// no bot) and whatsapp (phone pairing) it told the founder to do something
/// impossible, and a wrong instruction is worse than an error because it
/// reads like guidance.
pub fn one_time_token_instructions(provider: &str, code: &str) -> (String, String) {
    match provider {
        "telegram-bot" => (
            format!("Open Telegram, start a chat with your bot, and send: /connect {code}"),
            format!(
                "Send /connect {code} to your Telegram bot to link this channel. The bot is the \
                 one whose token is in TELEGRAM_BOT_TOKENS - the REST connect route resolves its \
                 @username via getMe, so the web UI names the exact chat to open."
            ),
        ),
        "telegram-user" => (
            format!("Telegram user login (MTProto): {code}"),
            format!(
                "Telegram user accounts have no bot to message. Request a login code for the phone \
                 number with the telegram_user_request_code tool, then redeem it with \
                 telegram_user_sign_in. Code: {code}"
            ),
        ),
        "whatsapp" => (
            format!("WhatsApp pairing: {code}"),
            format!(
                "WhatsApp pairs by phone number, not by a bot chat. Call the whatsapp pairing \
                 endpoint with the number in international format, then enter the pair code it \
                 returns on the phone under Linked Devices. Code: {code}"
            ),
        ),
        _ => (
            format!("/connect {code}"),
            format!("Send this code to the provider to link this channel: /connect {code}"),
        ),
    }
}

// ── Input/Output Types ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListIntegrationsInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListProvidersInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProviderInfo {
    pub identifier: String,
    pub name: String,
    pub configured: bool,
    pub oauth: bool,
    pub has_credentials: bool,
    pub editor_type: String,
    pub redirect_uri: String,
    /// Support tier: `tier-1` (full depth: analytics, engagement, comments,
    /// mentions, targets), `tier-2` (publish-maintained only),
    /// `tier-3` (archive; listed only when ENABLE_ARCHIVE_PROVIDERS is set).
    #[schemars(
        description = "Support tier: tier-1 (full depth), tier-2 (publish-maintained), tier-3 (archive)"
    )]
    pub tier: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListProvidersOutput {
    pub providers: Vec<ProviderInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationInfo {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub profile_name: Option<String>,
    pub disabled: bool,
    pub refresh_needed: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListIntegrationsOutput {
    pub integrations: Vec<IntegrationInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConnectInput {
    pub provider: String,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConnectOutput {
    pub auth_url: String,
    pub state: String,
    pub instructions: String,
    /// A warning about this provider's sign-in, when the provider can
    /// name one. `None` for the providers that have no gate.
    ///
    /// Threads is the one that needs it — Meta's consent gate redirects
    /// the BROWSER to its own privacy/consent page and never comes back
    /// to our callback, so the failure is invisible to the agent too, not
    /// just to the WebUI. An agent that gets this can tell the USER what
    /// to fix, which is the only thing that unblocks the connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DisconnectInput {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConnectCompleteInput {
    pub code: String,
    pub state: String,
}

// ── Tool Implementations ────────────────────────────────────

pub async fn list_providers(
    state: &AppState,
    _input: &ListProvidersInput,
) -> Result<Json<ListProvidersOutput>, String> {
    // Verify user exists (auth gate)
    let _user_id = super::tools_posts::resolve_first_user(state).await?;

    let all = state.providers.all();
    let mut providers: Vec<ProviderInfo> = all
        .into_iter()
        .map(|p| {
            let id = p.identifier();
            // The LIVE configured state, so a channel whose credentials the
            // founder just supplied in the app is reported as configured
            // on the very next read — with no restart in between.
            let has_creds = state.provider_configured(id);
            ProviderInfo {
                identifier: id.to_string(),
                name: p.name().to_string(),
                configured: has_creds,
                oauth: p.uses_oauth(),
                has_credentials: has_creds,
                editor_type: format!("{:?}", p.editor_type()),
                redirect_uri: if p.uses_oauth() {
                    // APP_URL, NOT frontend_url. `frontend_url` controls the
                    // CORS allow-list and the CSRF origin check; the OAuth
                    // callback the providers redirect to is built from
                    // APP_URL (see `api::integrations`' connect handler and
                    // `.env.example`, which documents APP_URL as "Used for
                    // OAuth redirect URIs"). Reporting the frontend value
                    // here makes Meta answer 'URL Blocked' (error 1349168)
                    // for a redirect_uri the connect flow never actually
                    // sent, so the operator chases a URL that looks right.
                    format!("{}/api/auth/callback", state.config.app_url)
                } else {
                    "N/A (non-OAuth)".into()
                },
                tier: state.providers.tier(id).as_str().to_string(),
            }
        })
        .collect();
    providers.sort_by(|a, b| a.identifier.cmp(&b.identifier));

    Ok(Json(ListProvidersOutput { providers }))
}

pub async fn list_integrations(
    state: &AppState,
    _input: &ListIntegrationsInput,
) -> Result<Json<ListIntegrationsOutput>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;

    let integrations = IntegrationService::list(&state.db, user_id).await?;

    let list: Vec<IntegrationInfo> = integrations
        .into_iter()
        .map(|i| IntegrationInfo {
            id: i.id.to_string(),
            provider: i.provider_identifier,
            name: i.provider_name,
            profile_name: i.profile_name,
            disabled: i.disabled,
            refresh_needed: i.refresh_needed,
        })
        .collect();

    Ok(Json(ListIntegrationsOutput { integrations: list }))
}

pub async fn connect_integration(
    state: &AppState,
    input: &ConnectInput,
) -> Result<Json<ConnectOutput>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;

    let provider_obj = state
        .providers
        .get(&input.provider)
        .ok_or_else(|| format!("Unknown provider: {}", input.provider))?;

    // Handle non-OAuth providers (e.g., Bluesky with app passwords)
    if !provider_obj.uses_oauth() {
        // One-time-token providers. The instructions name the flow THAT
        // provider uses — one shared "message this bot" string was wrong for
        // telegram-user and whatsapp (see one_time_token_instructions).
        if provider_obj.one_time_token() {
            let code = uuid::Uuid::new_v4().to_string();
            let (auth_url, instructions) = one_time_token_instructions(&input.provider, &code);
            return Ok(Json(ConnectOutput {
                auth_url,
                state: "one-time-token".into(),
                instructions,
                warning: None,
            }));
        }

        // The redirect_uri a tool asks for is VALIDATED, not trusted: a tool
        // caller controls this string as surely as a query-string caller does,
        // and an unvalidated redirect_uri sends the provider's authorization
        // code wherever the string says. One rule with the REST routes -
        // `social::common::resolve_redirect_uri`.
        //
        // APP_URL, NOT frontend_url: this value is handed to `exchange_code`,
        // which sends it back to the provider as the redirect_uri it
        // authorized. A mismatch is a hard failure (Meta: 'URL Blocked';
        // Google: redirect_uri_mismatch), and `frontend_url` is a different
        // setting for a different purpose (CORS + CSRF). See `.env.example`.
        let redirect_uri = crate::social::common::resolve_redirect_uri(
            input.redirect_uri.as_deref(),
            &state.config.app_url,
        )
        .map_err(|e| e.to_string())?;

        let token = provider_obj
            .exchange_code("", "", &redirect_uri)
            .await
            .map_err(|e| format!("Failed to connect {}: {e}", input.provider))?;

        crate::db::queries::create_integration(
            &state.db,
            user_id,
            &input.provider,
            &token.name,
            &token.provider_user_id,
            &token.access_token,
            token.refresh_token.as_deref(),
            token
                .expires_in
                .map(|exp| chrono::Utc::now() + chrono::Duration::seconds(exp as i64)),
            Some(&token.name),
            token.picture.as_deref(),
            None,
            None,
            None, // auth_method
            state.token_key,
        )
        .await
        .map_err(|e| e.to_string())?;

        return Ok(Json(ConnectOutput {
            auth_url: format!(
                "Connected to {} as {}. No browser OAuth needed.",
                input.provider, token.name
            ),
            state: "auto".into(),
            instructions: format!(
                "Connected to {} as {}. Store is complete.",
                input.provider, token.name
            ),
            warning: None,
        }));
    }

    let result = IntegrationService::initiate_connect(
        &state.db,
        &state.providers,
        user_id,
        &input.provider,
        &state.config.app_url,
    )
    .await?;

    // Asking the provider for a sign-in warning is the SAME question the
    // REST connect route asks, and it is asked here for the same reason:
    // Meta's Threads consent gate redirects the BROWSER to its own page
    // and never returns to our callback, so a failure there is invisible
    // to every handler of ours. The agent needs the reason too, because
    // an agent that reports "the sign-in failed" without naming the
    // cause has to hand the problem back to the user unsolved.
    let warning = provider_obj.connect_preflight().await;

    Ok(Json(ConnectOutput {
        auth_url: result.auth_url,
        state: result.state,
        instructions: format!(
            "Open the auth_url in a browser, authorize the app, then \
             the callback URL will complete the connection. \
             For MCP flow, after authorization the callback at {} \
             will process the result.",
            state.config.app_url,
        ),
        warning,
    }))
}

pub async fn complete_connect_integration(
    state: &AppState,
    input: &ConnectCompleteInput,
) -> Result<Json<crate::mcp::McpJsonValue>, String> {
    let _integration = IntegrationService::complete_connect(
        &state.db,
        &state.providers,
        &state.broadcast,
        &input.state,
        &input.code,
        state.token_key.as_ref(),
    )
    .await?;

    Ok(Json(crate::mcp::McpJsonValue(
        serde_json::json!({"success": true, "message": "Integration connected successfully",}),
    )))
}

pub async fn disconnect_integration(
    state: &AppState,
    input: &DisconnectInput,
) -> Result<Json<crate::mcp::McpJsonValue>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let integration_id =
        uuid::Uuid::parse_str(&input.id).map_err(|_| "Invalid integration ID".to_string())?;

    let deleted =
        IntegrationService::disconnect(&state.db, &state.broadcast, user_id, integration_id)
            .await?;

    if !deleted {
        return Err("Integration not found".into());
    }

    Ok(Json(crate::mcp::McpJsonValue(
        serde_json::json!({"success": true, "message": "Integration disconnected",}),
    )))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListTargetsInput {
    pub integration_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListTargetsOutput {
    pub targets: Vec<TargetInfo>,
    pub integration_id: String,
    pub provider: String,
}

pub async fn list_targets(
    state: &AppState,
    input: &ListTargetsInput,
) -> Result<Json<ListTargetsOutput>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let integration_id = uuid::Uuid::parse_str(&input.integration_id)
        .map_err(|_| "Invalid integration ID".to_string())?;

    let integration = crate::db::queries::get_integration(&state.db, integration_id, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| "Integration not found".to_string())?;

    let provider = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| {
            format!(
                "Provider '{}' not in registry",
                integration.provider_identifier
            )
        })?;

    let token =
        crate::crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

    let targets = provider
        .targets(&token)
        .await
        .map_err(|e| format!("Failed to fetch targets: {}", e))?;

    Ok(Json(ListTargetsOutput {
        targets,
        integration_id: input.integration_id.clone(),
        provider: integration.provider_identifier,
    }))
}

#[cfg(test)]
mod tests {
    use super::one_time_token_instructions;

    /// The bug: every one-time-token channel answered "Open Telegram, start a
    /// chat with this bot, and send: /connect <code>". That is true of
    /// telegram-bot and FALSE of telegram-user (MTProto, no bot exists) and
    /// whatsapp (a phone pairing, not a chat command). A wrong instruction is
    /// worse than an error, because an error says "this did not work" while a
    /// wrong instruction says "do this impossible thing" and reads as guidance.
    #[test]
    fn one_time_token_instructions_name_the_providers_own_flow() {
        for (provider, must_contain, must_not_contain) in [
            (
                "telegram-bot",
                vec!["Open Telegram", "your bot"],
                vec![] as Vec<&str>,
            ),
            (
                "telegram-user",
                vec!["MTProto", "no bot to message"],
                vec!["your Telegram bot to link", "start a chat with your bot"],
            ),
            (
                "whatsapp",
                vec!["pairs by phone number", "Linked Devices"],
                vec!["start a chat with your bot", "Telegram"],
            ),
        ] {
            let (auth_url, instructions) = one_time_token_instructions(provider, "CODE");
            for needle in must_contain {
                assert!(
                    auth_url.contains(needle) || instructions.contains(needle),
                    "{provider}: expected {needle:?} in the answer, got {auth_url:?} / {instructions:?}"
                );
            }
            for needle in must_not_contain {
                assert!(
                    !auth_url.contains(needle) && !instructions.contains(needle),
                    "{provider}: must NOT say {needle:?} - that is another channel's flow"
                );
            }
            // Both halves must carry the code, or the founder has nothing to redeem.
            assert!(
                auth_url.contains("CODE"),
                "{provider}: code missing from auth_url"
            );
            assert!(
                instructions.contains("CODE"),
                "{provider}: code missing from instructions"
            );
        }
    }

    /// An unknown provider still gets a usable generic answer rather than
    /// Telegram's bot flow, which is the shape of the original defect.
    #[test]
    fn an_unknown_provider_does_not_inherit_the_telegram_flow() {
        let (auth_url, instructions) = one_time_token_instructions("skool", "CODE");
        assert!(!auth_url.contains("Telegram"), "got {auth_url:?}");
        assert!(!instructions.contains("Telegram"), "got {instructions:?}");
        assert!(auth_url.contains("CODE") && instructions.contains("CODE"));
    }
}
