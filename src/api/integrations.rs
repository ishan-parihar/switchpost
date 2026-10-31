// ─── Integrations API Routes ──────────────────────────────────
// Social channel connections: OAuth flow initiation, callback, listing, deletion.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::AuthenticatedUser;
use crate::crypto;
use crate::db::models::IntegrationPublic;
use crate::db::queries;
use crate::error::AppError;
use crate::services::integrations::IntegrationService;
use crate::social::PageInfo;
use crate::social::TargetInfo;

use super::AppState;

// ── Request/Response Types ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectQuery {
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectResponse {
    pub url: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntegrationListResponse {
    pub integrations: Vec<IntegrationPublic>,
}

// ── Handlers ─────────────────────────────────────────────────

/// GET /api/integrations — list all connected channels
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<IntegrationListResponse>, AppError> {
    let integrations = queries::list_integrations(&state.db, auth.user_id).await?;
    let public: Vec<IntegrationPublic> = integrations.into_iter().map(Into::into).collect();
    Ok(Json(IntegrationListResponse {
        integrations: public,
    }))
}

/// GET /api/integrations/connect/:provider — initiate OAuth flow
pub async fn connect(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(provider): Path<String>,
    Query(query): Query<ConnectQuery>,
) -> Result<Json<ConnectResponse>, AppError> {
    // Check credentials first
    if state.config.provider_credentials(&provider).is_none() {
        return Err(AppError::BadRequest(
            format!("Provider '{provider}' is not configured. Set the corresponding environment variables first.")
        ));
    }

    // Find the provider in the registry
    let provider_obj = state
        .providers
        .get(&provider)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {provider}")))?;

    // Handle non-OAuth providers (e.g., Bluesky, Telegram)
    if !provider_obj.uses_oauth() {
        // One rule, four call sites: a caller-supplied `redirect_uri` is validated
        // against this deployment's own callback, not honoured verbatim. See
        // `social::common::resolve_redirect_uri`.
        let redirect_uri = crate::social::common::resolve_redirect_uri(
            query.redirect_uri.as_deref(),
            &state.config.app_url,
        )
        .map_err(AppError::BadRequest)?;

        // For one-time-token providers (Telegram), give instructions
        if provider_obj.one_time_token() {
            let code = uuid::Uuid::new_v4().to_string();
            let instructions = match provider.as_str() {
                "telegram-bot" => {
                    // Get bot username from configured token
                    let mut bot_username = String::from("your bot");
                    if let Some((_, tokens_str)) = state.config.provider_credentials("telegram-bot") {
                        let token = tokens_str.split(',').next().unwrap_or("").trim().to_string();
                        if !token.is_empty() {
                            if let Ok(resp) = crate::social::shared_http()
                                .get(format!("https://api.telegram.org/bot{token}/getMe"))
                                .send().await
                            {
                                if let Ok(json) = resp.json::<serde_json::Value>().await {
                                    if let Some(u) = json["result"]["username"].as_str() {
                                        bot_username = format!("@{u}");
                                    }
                                }
                            }
                        }
                    }
                    format!("{bot_username}\n/connect {code}")
                },
                "telegram-user" => "Use MCP tools (tu_request_code → tu_sign_in) to authenticate your Telegram account, then click Verify.".into(),
                "whatsapp" => "Use MCP tools (wa_pair_code) to link your WhatsApp device, then click Verify.".into(),
                _ => format!("Send this code to the provider: /connect {code}"),
            };
            return Ok(Json(ConnectResponse {
                url: instructions,
                state: "one-time-token".into(),
            }));
        }

        let token = provider_obj
            .exchange_code("", "", &redirect_uri)
            .await
            .map_err(|e| {
                tracing::error!("Direct connect failed for {}: {e}", provider);
                AppError::Provider(format!(
                    "Failed to connect {}. Make sure the provider env vars are set. Error: {e}",
                    provider
                ))
            })?;

        queries::create_integration(
            &state.db,
            auth.user_id,
            &provider,
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
        )
        .await?;

        state.broadcast.send(
            "integration_connected",
            &serde_json::json!({
                "provider": provider,
                "name": token.name,
            }),
        );

        return Ok(Json(ConnectResponse {
            url: format!(
                "Connected to {} as {}. No browser OAuth needed.",
                provider, token.name
            ),
            state: "auto".into(),
        }));
    }

    // Generate PKCE and OAuth URL
    let code_verifier = crate::social::common::generate_code_verifier();
    let oauth_state = crate::social::common::generate_state();
    let redirect_uri = crate::social::common::resolve_redirect_uri(
        query.redirect_uri.as_deref(),
        &state.config.app_url,
    )
    .map_err(AppError::BadRequest)?;

    tracing::info!("OAuth connect {provider}: redirect_uri={redirect_uri}");

    let auth_url = provider_obj
        .generate_auth_url(&oauth_state, &code_verifier, &redirect_uri)
        .await
        .map_err(|e| AppError::Provider(format!("Failed to generate auth URL: {e}")))?;

    // Store OAuth state with user context
    queries::save_oauth_state(
        &state.db,
        &oauth_state,
        &provider,
        &code_verifier,
        Some(&format!("{}:{}", auth.user_id, redirect_uri)),
    )
    .await?;

    Ok(Json(ConnectResponse {
        url: auth_url.url,
        state: oauth_state,
    }))
}

/// GET /api/auth/callback — complete OAuth flow
///
/// For single-step providers: redirects to onboarding page at {app_url}/?connected=...
/// For multi-step providers (isBetweenSteps): redirects to page-picker at {app_url}/?pending=...
pub async fn oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let app_url = state.config.app_url.clone();

    // Log the full callback URL params for debugging
    tracing::info!(
        "OAuth callback received: code={:?} state={:?} error={:?} error_description={:?}",
        query.code.as_deref().map(|c| &c[..c.len().min(20)]),
        query.state,
        query.error,
        query.error_description,
    );

    // Handle error redirects from provider (e.g. LinkedIn denied access)
    if let Some(ref error) = query.error {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("no description");
        tracing::warn!("OAuth provider returned error: {error} — {desc}");
        return Ok(Redirect::to(&format!(
            "{}/auth/callback?error={}",
            app_url,
            urlencoding::encode(&format!("{error}: {desc}")),
        )));
    }

    let code = query.code.as_deref().ok_or_else(|| {
        tracing::error!("OAuth callback missing both code and error params");
        AppError::BadRequest("OAuth callback missing authorization code".into())
    })?;
    let state_val = query.state.as_deref().unwrap_or("");

    match IntegrationService::complete_connect(
        &state.db,
        &state.providers,
        &state.broadcast,
        state_val,
        code,
        state.token_key.as_ref(),
    )
    .await
    {
        Ok(integration) => {
            tracing::info!(
                "Integration connected: {} ({}) for user {}",
                integration.provider_identifier,
                integration.provider_name,
                integration.user_id
            );

            // Multi-step providers (isBetweenSteps) redirect to page-picker
            if let Some(provider_obj) = state.providers.get(&integration.provider_identifier) {
                // The page picker authenticates with the session cookie, so the
                // redirect carries no credential. A token in the URL would land in
                // browser history and access logs.
                if provider_obj.is_between_steps() {
                    return Ok(Redirect::to(&format!(
                        "{}/auth/callback?pending={}&integration_id={}",
                        app_url, integration.provider_identifier, integration.id,
                    )));
                }
            }

            Ok(Redirect::to(&format!(
                "{}/auth/callback?connected={}&name={}",
                app_url,
                integration.provider_identifier,
                urlencoding::encode(&integration.provider_name),
            )))
        }
        Err(e) => {
            tracing::error!("OAuth callback failed: {e}");
            Ok(Redirect::to(&format!(
                "{}/auth/callback?error={}",
                app_url,
                urlencoding::encode(&e.to_string()),
            )))
        }
    }
}

/// DELETE /api/integrations/:id — remove a connected channel
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Delete posts for this integration first
    let deleted = queries::delete_integration(&state.db, id, auth.user_id).await?;
    if !deleted {
        return Err(AppError::NotFound("Integration not found".into()));
    }

    state
        .broadcast
        .send("integration_disconnected", &serde_json::json!({"id": id}));

    Ok(Json(serde_json::json!({"deleted": true})))
}

// ── Provider Status ────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ProviderStatus {
    pub identifier: String,
    pub name: String,
    pub configured: bool,
    pub oauth: bool,
    pub has_credentials: bool,
    pub editor_type: String,
    pub redirect_uri: String,
    /// Support tier: `tier-1` (depth), `tier-2` (publish-maintained),
    /// `tier-3` (archive — only present here when ENABLE_ARCHIVE_PROVIDERS
    /// is set). See `docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md` §1.
    pub tier: &'static str,
    /// The provider's own content limit, straight from the provider
    /// implementation.
    ///
    /// The frontend used to hardcode these in a TypeScript table, and the
    /// two drifted: X showed 280 in the composer (so a 300-character post
    /// was blocked and the Post button disabled) while the server enforced
    /// 4000. Every surface that shows a limit was wrong for whichever
    /// provider drifted. The provider implementation is the owner; this
    /// publishes it.
    pub char_limit: usize,
    /// Whether this provider has a DIRECT-MESSAGE surface the app can drive.
    ///
    /// The `/dms` page used to carry its own hardcoded list
    /// (`['x','instagram','linkedin']`) and pick the first match as the
    /// default channel, so on an instance whose only X channel is
    /// COOKIE-auth the founder landed on a 400: X's DM API is bearer-only,
    /// and a cookie channel has no bearer. MEASURED 2026-11-01 - /dms
    /// opened with the X channel selected and the server answering
    /// 400 "X direct messages need an OAuth-connected channel".
    ///
    /// A client-side guess is not the owner of that rule, and guessing it
    /// differently (auth_method) just moves the drift: the same client
    /// then offered X in the same selector and produced the 400 from the
    /// UI. `capabilities` in `src/social/mod.rs` is the one place that
    /// knows, so this publishes it for the same reason `char_limit` is
    /// published above.
    pub dm_supported: bool,
}

/// GET /api/providers — list all providers with config status
///
/// Tier-3 (archive) providers are absent unless the operator sets
/// `ENABLE_ARCHIVE_PROVIDERS`, so the default response is the 12 Tier-1 +
/// 14 Tier-2 surface.
pub async fn list_providers(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
) -> Result<Json<Vec<ProviderStatus>>, AppError> {
    let all = state.providers.all();
    let mut statuses = Vec::new();
    for provider in all {
        let id = provider.identifier();
        let has_creds = state.config.provider_credentials(id).is_some();
        statuses.push(ProviderStatus {
            identifier: id.to_string(),
            name: provider.name().to_string(),
            configured: has_creds,
            oauth: provider.uses_oauth(),
            has_credentials: has_creds,
            editor_type: format!("{:?}", provider.editor_type()),
            redirect_uri: if provider.uses_oauth() {
                // APP_URL, NOT frontend_url — see the note in
                // `mcp::tools_integrations`. `frontend_url` is the CORS/CSRF
                // origin; the redirect_uri the providers are actually sent is
                // derived from APP_URL. Reporting the other one means the
                // operator registers the wrong URL in the Meta/Google console
                // and then gets 'URL Blocked' for a flow that looks correct.
                format!("{}/api/auth/callback", state.config.app_url)
            } else {
                "N/A (non-OAuth)".into()
            },
            tier: state.providers.tier(id).as_str(),
            char_limit: provider.max_content_length(),
            dm_supported: provider.dm_capable(),
        });
    }
    statuses.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    Ok(Json(statuses))
}
// ── Timeslots & Disable ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateTimeslotsRequest {
    pub timeslots: Vec<TimeslotEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeslotEntry {
    pub time: i32, // minutes from midnight
}

/// PUT /api/integrations/{id}/timeslots — update posting time slots
pub async fn update_timeslots(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTimeslotsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify integration belongs to user
    let _integration = queries::get_integration(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    // Validate: max 3 slots
    if body.timeslots.len() > 3 {
        return Err(AppError::BadRequest("Maximum 3 time slots allowed".into()));
    }

    // Validate no duplicate times
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for slot in &body.timeslots {
        if !seen.insert(&slot.time) {
            return Err(AppError::BadRequest(format!(
                "Duplicate time slot: {} minutes is already set",
                slot.time
            )));
        }
    }

    // Validate each time is 0-1439
    for slot in &body.timeslots {
        if slot.time < 0 || slot.time >= 1440 {
            return Err(AppError::BadRequest(
                "Invalid time: must be 0-1439 minutes from midnight".into(),
            ));
        }
    }

    let timeslots_json = serde_json::to_value(&body.timeslots)
        .map_err(|_| AppError::Internal("Failed to serialize timeslots".into()))?;

    sqlx::query(
        "UPDATE integrations SET posting_times = $1, updated_at = unixepoch() WHERE id = $2 AND user_id = $3",
    )
    .bind(&timeslots_json)
    .bind(id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "posting_times": timeslots_json
    })))
}

#[derive(Debug, Deserialize)]
pub struct ToggleDisableRequest {
    pub disabled: bool,
}

/// PUT /api/integrations/{id}/disable — toggle integration disabled state
pub async fn toggle_disable(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ToggleDisableRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _integration = queries::get_integration(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    queries::set_integration_disabled(&state.db, auth.user_id, id, body.disabled).await?;

    Ok(Json(
        serde_json::json!({"success": true, "disabled": body.disabled}),
    ))
}

/// POST /api/integrations/{id}/refresh — refresh OAuth token and profile info
pub async fn refresh(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let integration = queries::get_integration(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let provider_obj = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| AppError::BadRequest("Provider not found in registry".into()))?;

    let mut new_access_token = None;
    if let Some(ref refresh_token_encrypted) = integration.refresh_token {
        let refresh_token =
            crypto::maybe_decrypt_token(refresh_token_encrypted, state.token_key.as_ref());
        match provider_obj.refresh_token(&refresh_token).await {
            Ok(new_token) => {
                // B8: update_integration_token is the single encrypting
                // writer — pass plaintext and let it encrypt. Keeping the
                // plaintext here also means reconnect() below receives a
                // usable token (it previously got the encrypted blob).
                let expires_at = new_token
                    .expires_in
                    .map(|e| chrono::Utc::now() + chrono::Duration::seconds(e as i64));

                queries::update_integration_token(
                    &state.db,
                    id,
                    &new_token.access_token,
                    new_token.refresh_token.as_deref(),
                    expires_at,
                    state.token_key,
                )
                .await?;

                new_access_token = Some(new_token.access_token.clone());

                if provider_obj.refresh_wait() {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }
            Err(e) => {
                tracing::warn!("Token refresh failed for integration {}: {e}", id);
                if let Err(e) = queries::mark_integration_refresh_needed(&state.db, id).await {
                    tracing::warn!("DB operation failed: {e}");
                }
                return Err(crate::error::map_scope_loss(
                    &integration.provider_identifier,
                    &id.to_string(),
                    &format!("Token refresh failed: {e}"),
                ));
            }
        }
    }

    let fallback_token =
        crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());
    let token_to_use = new_access_token.as_deref().unwrap_or(&fallback_token);

    let info = provider_obj
        .reconnect(
            token_to_use,
            &integration.internal_id,
            &integration.internal_id,
        )
        .await
        .map_err(|e| {
            crate::error::map_scope_loss(
                &integration.provider_identifier,
                &id.to_string(),
                &format!("Failed to refresh profile: {e}"),
            )
        })?;

    // The write is COALESCE on BOTH profile fields, not just the picture.
    //
    // `reconnect`'s default DEGRADES on a channel-shaped provider by returning
    // its own identity with an empty name and no picture: that is what lets a
    // Reddit channel be refreshable without a per-provider override. Written
    // bare, those two blanks overwrite what the founder already had - the
    // channel on the Channels page went from "u/cooluser r/NICE SUB" to an
    // empty label, with no error and no way back.
    //
    // The picture was already guarded; the name was not. Proven live: a row
    // seeded with `'u/cooluser r/NICE SUB'` and a real avatar came back from
    // POST /api/integrations/:id/refresh with the avatar intact and the name
    // `''`.
    //
    // An EMPTY name is what "the provider told us nothing" looks like here, so
    // it must not be distinguishable from "keep what you had" at the write. A
    // name that is genuinely cleared is a different operation and a different
    // endpoint.
    let info_name = if info.name.is_empty() {
        None
    } else {
        Some(info.name.clone())
    };

    sqlx::query(
        "UPDATE integrations SET profile_name = COALESCE($1, profile_name), profile_picture = COALESCE($2, profile_picture), refresh_needed = false, updated_at = unixepoch() WHERE id = $3 AND user_id = $4",
    )
    .bind(info_name)
    .bind(info.picture.clone())
    .bind(id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "profile_name": info.name,
        "profile_picture": info.picture,
        "token_refreshed": new_access_token.is_some(),
    })))
}

// ── X Cookie Connect ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectXCookieRequest {
    pub auth_token: String,
    pub ct0: String,
}

/// POST /api/integrations/connect/:provider/verify — verify one-time-token (Telegram Bot)
pub async fn verify_one_time_token(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(provider): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ConnectResponse>, AppError> {
    let code = body["code"].as_str().unwrap_or("").to_string();

    let provider_obj = state
        .providers
        .get(&provider)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {provider}")))?;

    let redirect_uri = format!("{}/api/auth/callback", state.config.app_url);
    let token = provider_obj
        .exchange_code(&code, "", &redirect_uri)
        .await
        .map_err(|e| AppError::Provider(format!("Verification failed: {e}")))?;

    let profile_url = if token.username.is_empty() {
        None
    } else {
        Some(format!("@{}", token.username))
    };
    queries::create_integration(
        &state.db,
        auth.user_id,
        &provider,
        provider_obj.name(),
        &token.provider_user_id,
        &token.access_token,
        token.refresh_token.as_deref(),
        token
            .expires_in
            .map(|exp| chrono::Utc::now() + chrono::Duration::seconds(exp as i64)),
        Some(&token.name),
        token.picture.as_deref(),
        profile_url.as_deref(),
        None,
        None,
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({ "provider": provider, "name": token.name }),
    );

    Ok(Json(ConnectResponse {
        url: format!("Connected to {} as {}", provider_obj.name(), token.name),
        state: "connected".into(),
    }))
}

/// POST /api/integrations/connect/telegram-bot/token — connect with a custom bot token
pub async fn connect_telegram_bot_token(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = body["token"].as_str().unwrap_or("").trim().to_string();
    if token.is_empty() {
        return Err(AppError::BadRequest("Bot token required".into()));
    }

    // Validate token via getMe
    let http = crate::social::shared_http();
    let resp = http
        .get(format!("https://api.telegram.org/bot{token}/getMe"))
        .send()
        .await
        // without_url(): the request URL carries the bot token.
        .map_err(|e| {
            AppError::Provider(format!("Failed to reach Telegram: {}", e.without_url()))
        })?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Invalid response: {e}")))?;

    if !json["ok"].as_bool().unwrap_or(false) {
        return Err(AppError::Provider("Invalid bot token".into()));
    }

    let bot = &json["result"];
    let bot_id = bot["id"].as_i64().unwrap_or(0).to_string();
    let bot_username = bot["username"].as_str().unwrap_or("");
    let display_name = if bot_username.is_empty() {
        "Telegram Bot".to_string()
    } else {
        format!("@{bot_username}")
    };
    let profile_url = if bot_username.is_empty() {
        None
    } else {
        Some(format!("@{bot_username}"))
    };

    // Store as JSON with bot_token (no chat_id yet — user connects a chat via /connect flow)
    let access_token = serde_json::json!({ "bot_token": token }).to_string();

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        "telegram-bot",
        "Telegram Bot",
        &bot_id,
        &access_token,
        None,
        None,
        Some(&display_name),
        None,
        profile_url.as_deref(),
        None,
        None,
    )
    .await?;

    let public: IntegrationPublic = integration.into();
    Ok(Json(serde_json::json!({ "integration": public })))
}

/// POST /api/integrations/connect/whatsapp/pair — request pair code
pub async fn whatsapp_pair(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let phone = body["phone_number"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if phone.is_empty() || !phone.starts_with('+') {
        return Err(AppError::BadRequest(
            "phone_number required in international format (e.g. +1234567890)".into(),
        ));
    }

    let wa = state
        .wa_client
        .as_ref()
        .ok_or_else(|| AppError::Provider("WhatsApp client not configured".into()))?;

    // Force fresh connection for reliable pair code generation
    {
        let mut locked = wa.lock().await;
        locked
            .reconnect()
            .await
            .map_err(|e| AppError::Provider(format!("WhatsApp connect failed: {e}")))?;
    }

    let code = crate::wa::auth::pair_with_code(
        wa,
        crate::wa::auth::PairOptions {
            phone_number: phone,
            show_push_notification: true,
        },
    )
    .await
    .map_err(|e| AppError::Provider(format!("Pair code request failed: {e}")))?;

    Ok(Json(
        serde_json::json!({ "pair_code": code, "expires_in": 180 }),
    ))
}

/// GET /api/integrations/connect/whatsapp/status — poll auth status
pub async fn whatsapp_status(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let wa = state
        .wa_client
        .as_ref()
        .ok_or_else(|| AppError::Provider("WhatsApp client not configured".into()))?;

    let locked = wa.lock().await;
    let authenticated = locked.is_authenticated();
    let jid = if authenticated {
        locked.inner().get_pn().await.map(|j| j.to_string())
    } else {
        None
    };
    Ok(Json(
        serde_json::json!({ "authenticated": authenticated, "jid": jid }),
    ))
}

/// POST /api/integrations/connect/telegram-user/request-code
pub async fn telegram_user_request_code(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let phone = body["phone"].as_str().unwrap_or("").trim().to_string();
    if phone.is_empty() {
        return Err(AppError::BadRequest("phone number required".into()));
    }

    let mgr = state.telegram_client_manager.as_ref().ok_or_else(|| {
        AppError::Provider(
            "Telegram user client not configured. Set TELEGRAM_API_ID and TELEGRAM_API_HASH."
                .into(),
        )
    })?;

    mgr.request_login_code(&phone)
        .await
        .map_err(|e| AppError::Provider(format!("Failed to send code: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "code_sent" })))
}

/// POST /api/integrations/connect/telegram-user/sign-in
pub async fn telegram_user_sign_in(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let code = body["code"].as_str().unwrap_or("").trim().to_string();
    if code.is_empty() {
        return Err(AppError::BadRequest("verification code required".into()));
    }

    let mgr = state
        .telegram_client_manager
        .as_ref()
        .ok_or_else(|| AppError::Provider("Telegram user client not configured".into()))?;

    mgr.sign_in("", &code)
        .await
        .map_err(|e| AppError::Provider(format!("Sign in failed: {e}")))?;

    // Fetch user info and create integration
    let info = mgr
        .user_info()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to get user info: {e}")))?;

    let user_id = info["id"].as_i64().unwrap_or(0).to_string();
    let name = info["name"].as_str().unwrap_or("Telegram User").to_string();
    let username = info["username"].as_str().unwrap_or("").to_string();
    let profile_url = if username.is_empty() {
        None
    } else {
        Some(format!("@{username}"))
    };

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        "telegram-user",
        "Telegram User",
        &user_id,
        &user_id,
        None,
        None,
        Some(&name),
        None,
        profile_url.as_deref(),
        None,
        None,
    )
    .await?;

    let public: IntegrationPublic = integration.into();
    Ok(Json(serde_json::json!({ "integration": public })))
}

/// POST /api/integrations/connect/x-cookie — connect X via browser cookies
pub async fn connect_x_cookie(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<ConnectXCookieRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.auth_token.trim().is_empty() || body.ct0.trim().is_empty() {
        return Err(AppError::BadRequest(
            "auth_token and ct0 are required".into(),
        ));
    }

    let cookie_str = format!(
        "auth_token={}; ct0={};",
        body.auth_token.trim(),
        body.ct0.trim()
    );
    let ct0 = body.ct0.trim();
    let bearer = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

    let http = crate::social::shared_http();

    // Step 1: Get current user from multi/list
    let resp = http
        .get("https://x.com/i/api/1.1/account/multi/list.json")
        .header("x-csrf-token", ct0)
        .header("Cookie", &cookie_str)
        .header("Authorization", bearer)
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("X cookie auth failed: {e}")))?;

    let list: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("X response parse error: {e}")))?;

    let users = list["users"]
        .as_array()
        .ok_or_else(|| AppError::Provider("X cookie auth: no users in response".into()))?;
    let user = users
        .first()
        .ok_or_else(|| AppError::Provider("X cookie auth: empty users list".into()))?;

    let screen_name = user["screen_name"].as_str().unwrap_or("");
    let user_id_str = user["user_id"].as_str().unwrap_or("");

    if screen_name.is_empty() || user_id_str.is_empty() {
        return Err(AppError::Provider(
            "X cookie auth: could not determine user".into(),
        ));
    }

    // Step 2: Get full profile via GraphQL UserByScreenName
    let vars = serde_json::json!({"screen_name": screen_name, "withSafetyModeUserFields": true});
    let features = serde_json::json!({"hidden_profile_subscriptions_enabled":true,"responsive_web_graphql_exclude_directive_enabled":true,"verified_phone_label_enabled":false,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"responsive_web_graphql_timeline_navigation_enabled":true});
    let qid = "1VOOyvKkiI3FMmkeDNxM9A";
    let gql_url = format!(
        "https://x.com/i/api/graphql/{qid}/UserByScreenName?variables={}&features={}",
        urlencoding::encode(&vars.to_string()),
        urlencoding::encode(&features.to_string()),
    );

    let resp = http
        .get(&gql_url)
        .header("x-csrf-token", ct0)
        .header("Cookie", &cookie_str)
        .header("Authorization", bearer)
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("X GraphQL error: {e}")))?;

    let profile: serde_json::Value = resp.json().await.unwrap_or_default();
    let legacy = profile.pointer("/data/user/result/legacy");
    let name = legacy
        .and_then(|l| l["name"].as_str())
        .unwrap_or(screen_name);
    let picture = legacy
        .and_then(|l| l["profile_image_url_https"].as_str())
        .map(|u| u.replace("_normal", "_400x400"));

    // Store as JSON blob
    let token_json = serde_json::json!({
        "auth_token": body.auth_token.trim(),
        "ct0": body.ct0.trim(),
        "cookie_string": cookie_str,
    });
    let access_token = token_json.to_string();

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        "x",
        "X (Twitter)",
        user_id_str,
        &access_token,
        None,
        None,
        Some(name),
        picture.as_deref(),
        Some(&format!("@{screen_name}")),
        None,
        Some("cookie"),
    )
    .await?;

    let public: IntegrationPublic = integration.into();
    Ok(Json(serde_json::json!({ "integration": public })))
}

// ── Reddit Cookie Connect ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectRedditCookieRequest {
    pub cookie_string: String,
}

/// POST /api/integrations/connect/reddit-cookie
pub async fn connect_reddit_cookie(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<ConnectRedditCookieRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.cookie_string.trim().is_empty() {
        return Err(AppError::BadRequest("cookie_string is required".into()));
    }

    let parsed = crate::social::reddit_cookies::parse_cookie_string(&body.cookie_string)
        .ok_or_else(|| {
            AppError::BadRequest(
                "Could not parse cookie string. Ensure it contains reddit_session.".into(),
            )
        })?;

    let token_str = crate::social::reddit_cookies::build_cookie_token(
        &parsed.reddit_session,
        parsed.token_v2.as_deref(),
        Some(&parsed.cookie_string),
    );

    // Validate by fetching user info
    let http = crate::social::shared_http();
    let me_resp = http
        .get("https://www.reddit.com/api/me.json")
        .header("Cookie", &parsed.cookie_string)
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) switchpost/0.1",
        )
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Reddit cookie validation failed: {e}")))?;
    let me: serde_json::Value = me_resp
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Reddit response parse error: {e}")))?;

    let username = me["data"]["name"].as_str().unwrap_or("unknown");
    if username == "unknown" || username.is_empty() {
        return Err(AppError::BadRequest(
            "Invalid cookies — could not authenticate with Reddit".into(),
        ));
    }
    let icon = me["data"]["icon_img"]
        .as_str()
        .and_then(|s| s.split('?').next())
        .unwrap_or("");

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        "reddit",
        "Reddit",
        username,
        &token_str,
        None,
        None,
        Some(username),
        if icon.is_empty() { None } else { Some(icon) },
        Some(&format!("https://reddit.com/user/{username}")),
        None,
        Some("cookie"),
    )
    .await
    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?;

    Ok(Json(
        serde_json::json!({ "integration": { "id": integration.id, "provider_name": "Reddit", "profile_name": username } }),
    ))
}

// ── GitHub PAT Connect ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectGithubPatRequest {
    pub pat: String,
    pub label: Option<String>,
}

/// POST /api/integrations/connect/github-pat — connect GitHub via Personal Access Token
pub async fn connect_github_pat(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<ConnectGithubPatRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.pat.trim().is_empty() {
        return Err(AppError::BadRequest("pat is required".into()));
    }

    let http = crate::social::shared_http();
    let resp = http
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", body.pat.trim()))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "switchpost:v0.1.0")
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("GitHub API error: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::BadRequest(
            "Invalid GitHub PAT or insufficient permissions".into(),
        ));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("GitHub response parse error: {e}")))?;

    let gh_id = json["id"]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let login = json["login"].as_str().unwrap_or("");
    let name = json["name"].as_str().or(Some(login)).unwrap_or("");
    let avatar = json["avatar_url"].as_str();

    if gh_id.is_empty() {
        return Err(AppError::Provider(
            "Failed to fetch GitHub user info".into(),
        ));
    }

    let label = body.label.as_deref().unwrap_or(name);

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        "github",
        "GitHub",
        &gh_id,
        body.pat.trim(),
        None,
        None,
        Some(label),
        avatar,
        Some(&format!("@{login}")),
        None,
        Some("pat"),
    )
    .await?;

    let public: IntegrationPublic = integration.into();
    Ok(Json(serde_json::json!({ "integration": public })))
}

// ── API Key Connect ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectApiKeyRequest {
    pub provider: String,
    pub api_key: String,
    pub instance_url: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectApiKeyResponse {
    pub integration: IntegrationPublic,
}

/// POST /api/integrations/connect/api-key — connect a provider using API key
///
/// For API-key providers (medium, devto, hashnode, and any self-hosted
/// instance reached via `instance_url`). Validates the API key by calling the
/// provider's pages() method, then stores the credentials as JSON in the
/// integration record.
pub async fn connect_api_key(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<ConnectApiKeyRequest>,
) -> Result<Json<ConnectApiKeyResponse>, AppError> {
    let provider_obj = state
        .providers
        .get(&body.provider)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", body.provider)))?;

    if provider_obj.uses_oauth() {
        return Err(AppError::BadRequest(format!(
            "Provider '{}' uses OAuth, not API key auth. Use the standard connect flow.",
            body.provider
        )));
    }

    if body.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("api_key must not be empty".into()));
    }

    // Build the credential JSON (WordPress-style per-user credential storage)
    let instance_url = body.instance_url.unwrap_or_default();
    let label = body
        .label
        .unwrap_or_else(|| provider_obj.name().to_string());

    let creds_json = serde_json::json!({
        "api_key": body.api_key,
        "instance_url": instance_url,
    });
    let access_token = creds_json.to_string();

    // Validate by calling provider's pages() method
    let pages = provider_obj
        .pages(&access_token)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to validate API key: {e}")))?;

    let page = pages.into_iter().next().unwrap_or(PageInfo {
        id: provider_obj.identifier().to_string(),
        name: label.clone(),
        access_token: Some(access_token.clone()),
        picture: None,
        username: None,
    });

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        &body.provider,
        provider_obj.name(),
        &page.id,
        &access_token,
        None,
        None,
        Some(&label),
        page.picture.as_deref(),
        None,
        None,
        None, // auth_method
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({
            "provider": body.provider,
            "name": label,
        }),
    );

    tracing::info!(
        "API key integration connected: {} ({}) for user {}",
        body.provider,
        label,
        auth.user_id,
    );

    Ok(Json(ConnectApiKeyResponse {
        integration: integration.into(),
    }))
}

// ── Web3 Connect ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConnectWeb3Request {
    pub provider: String,
    pub address: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectWeb3Response {
    pub integration: IntegrationPublic,
}

/// POST /api/integrations/connect/web3 — connect a Web3 provider
///
/// For providers that use wallet addresses. Farcaster is the only one left,
/// and it is Tier-3 (archive) — the lookup below fails unless the server
/// runs with `ENABLE_ARCHIVE_PROVIDERS` set. Accepts
/// { provider, address, label }, validates via exchange_code, then stores
/// the address as the access_token.
pub async fn connect_web3(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<ConnectWeb3Request>,
) -> Result<Json<ConnectWeb3Response>, AppError> {
    let provider_obj = state
        .providers
        .get(&body.provider)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", body.provider)))?;

    if provider_obj.uses_oauth() {
        return Err(AppError::BadRequest(format!(
            "Provider '{}' uses OAuth, not Web3 auth. Use the standard connect flow.",
            body.provider
        )));
    }

    // Verify provider is Web3-capable (farcaster only — nostr was deleted)
    if body.provider != "farcaster" {
        return Err(AppError::BadRequest(
            format!("Web3 auth is not supported for provider '{}'. Only 'farcaster' supports this auth method.", body.provider)
        ));
    }

    // Check credentials first
    if state.config.provider_credentials(&body.provider).is_none() {
        return Err(AppError::BadRequest(format!(
            "Provider '{}' is not configured. Set the corresponding environment variables first.",
            body.provider
        )));
    }

    if body.address.trim().is_empty() {
        return Err(AppError::BadRequest("address must not be empty".into()));
    }

    let label = body
        .label
        .unwrap_or_else(|| provider_obj.name().to_string());

    // Build credential JSON and pass through exchange_code for validation
    let creds_json = serde_json::json!({
        "address": body.address,
    });
    let code = creds_json.to_string();

    let auth_result = provider_obj
        .exchange_code(&code, "", "")
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to validate Web3 credentials: {e}")))?;

    // Use the normalized address from exchange_code as the access_token
    let final_token = auth_result.access_token;

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        &body.provider,
        provider_obj.name(),
        &auth_result.provider_user_id,
        &final_token,
        None,
        None,
        Some(&label),
        auth_result.picture.as_deref(),
        None,
        None,
        None, // auth_method
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({
            "provider": body.provider,
            "name": label,
        }),
    );

    tracing::info!(
        "Web3 integration connected: {} ({}) for user {}",
        body.provider,
        label,
        auth.user_id,
    );

    Ok(Json(ConnectWeb3Response {
        integration: integration.into(),
    }))
}

// ── Multi-Account Pages API ──────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AvailablePagesResponse {
    pub pages: Vec<PageInfo>,
    pub parent_integration_id: String,
    pub provider: String,
}

/// GET /api/integrations/{id}/available-pages — list sub-accounts for a provider
///
/// Uses provider.pages() to discover connectable pages/channels.
/// For multi-step providers (isBetweenSteps), access_token is the user-level token.
pub async fn available_pages(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<AvailablePagesResponse>, AppError> {
    let integration = queries::get_integration(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let provider_obj = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| AppError::BadRequest("Provider not found in registry".into()))?;

    // Facebook/Instagram store the user-level token in refresh_token for page discovery.
    // Other multi-step providers (LinkedIn Page) use access_token directly.
    let raw_token = if integration.provider_identifier == "facebook"
        || integration.provider_identifier == "instagram"
    {
        integration
            .refresh_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&integration.access_token)
    } else {
        &integration.access_token
    };
    // Decrypt the stored token if encryption is enabled
    let token = crypto::maybe_decrypt_token(raw_token, state.token_key.as_ref());

    let pages = provider_obj
        .pages(&token)
        .await
        .map_err(|e| AppError::Provider(format!("Failed to list pages: {e}")))?;

    Ok(Json(AvailablePagesResponse {
        pages,
        parent_integration_id: integration.id.to_string(),
        provider: integration.provider_identifier,
    }))
}

#[derive(Debug, Serialize)]
pub struct ConnectPageResponse {
    pub integration: IntegrationPublic,
    pub parent_id: String,
}

/// POST /api/integrations/{parent_id}/connect-page/{page_id} — connect a sub-account
///
/// Creates a new integration linked to the parent via root_internal_id.
/// The page token is obtained from provider.pages().
pub async fn connect_page(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((parent_id, page_id)): Path<(Uuid, String)>,
) -> Result<Json<ConnectPageResponse>, AppError> {
    let parent = queries::get_integration(&state.db, parent_id, auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let provider_obj = state
        .providers
        .get(&parent.provider_identifier)
        .ok_or_else(|| AppError::BadRequest("Provider not found in registry".into()))?;

    // Same token discovery logic as available_pages
    let raw_token =
        if parent.provider_identifier == "facebook" || parent.provider_identifier == "instagram" {
            parent
                .refresh_token
                .as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or(&parent.access_token)
        } else {
            &parent.access_token
        };

    // A repeat of a connect that already succeeded is not an error.
    //
    // The old guard answered "Page 'X' is already connected" and stopped, which
    // is the exact message a founder hits when they re-run the unified Google
    // flow: the picker lists every page under the parent, including the ones
    // already connected, and choosing one that is connected dead-ends the
    // multi-step flow with nothing to do next. The integration is ALREADY in
    // the state they asked for, so the honest answer is to say so and succeed.
    //
    // Note what this does NOT reopen: `create_integration` upserts on
    // (user_id, provider_identifier, internal_id), so a repeat refreshes the
    // token on the row that is already there rather than creating a duplicate.
    // The one-across-many-providers invariant the picker exists to preserve is
    // unaffected - this only stops a no-op request from failing.
    //
    // The lookup happens BEFORE the pages() call on purpose, and on the repeat
    // path the pages() call does not happen at all (see below). Everything it
    // needs is already in hand (the parent's provider and the requested page
    // id), so a repeat answers from the database instead of asking the provider
    // for it first - and that means a repeat while the provider's API is down or
    // rate-limiting still succeeds, instead of failing on the network call it
    // never needed.
    let already_connected = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM integrations WHERE user_id = $1 \
         AND provider_identifier = $2 AND internal_id = $3",
    )
    .bind(auth.user_id)
    .bind(&parent.provider_identifier)
    .bind(&page_id)
    .fetch_optional(&state.db)
    .await?;

    let token = crypto::maybe_decrypt_token(raw_token, state.token_key.as_ref());

    // Fetch all pages and find the matching one.
    //
    // Skipped on the repeat path, and that is the point rather than an
    // optimisation: `pages()` is the call that fails during a provider outage or
    // a rate limit, and a repeat is exactly what a founder retries after a
    // connect failed halfway for that reason. Making the provider fetch a list
    // we do not need would turn a recovery into another failure on the same
    // broken network path.
    //
    // The earlier note here said the lookup was ordered first so a repeat
    // "answers from the database instead of making the provider fetch it", but
    // the fetch was still unconditional - so the claim was true of the lookup
    // and false of the handler. Verified before the change: nothing in the
    // repeat branch needs `page`, except to refresh the token, and it handles
    // `None` precisely because a provider that no longer lists the page is a
    // case it already expected. First connect still requires the page.
    let pages = if already_connected.is_some() {
        Vec::new()
    } else {
        provider_obj
            .pages(&token)
            .await
            .map_err(|e| AppError::Provider(format!("Failed to list pages: {e}")))?
    };

    let page = pages.into_iter().find(|p| p.id == page_id);

    if let Some(id) = already_connected {
        // The repeat path: the row is already in the state the caller asked
        // for, so refresh its token and return it. `page` may be None when
        // the provider no longer lists this page (it was deleted over there,
        // or the parent lost access), but the token we already hold on the
        // parent row is still a working one for it - that is the token the
        // row was connected with. Not re-fetching is what makes the answer
        // stable when the provider is unavailable.
        let refreshed = match page {
            Some(p) => {
                // An empty token would be stored as a working-looking
                // integration whose every publish then fails. Refuse it
                // here, where the user can act on it.
                let page_token = p.access_token.filter(|t| !t.is_empty()).ok_or_else(|| {
                    AppError::BadRequest(
                        "This page did not return an access token; reconnect with a token that manages it"
                            .into(),
                    )
                })?;

                // Refresh the token on the row that already exists, so a
                // repeat fixes a stale or empty one rather than leaving it
                // broken. A failure here is reported rather than swallowed:
                // the whole point of re-running the connect was to fix the
                // token.
                queries::update_integration_token(
                    &state.db,
                    id,
                    &page_token,
                    parent.refresh_token.as_deref(),
                    None,
                    state.token_key,
                )
                .await
                .map_err(|e| match e {
                    queries::TokenUpdateError::Encryption { reason, .. } => {
                        AppError::Provider(format!("Failed to re-encrypt token: {reason}"))
                    }
                    queries::TokenUpdateError::Db(e) => AppError::Database(e),
                })?;
                true
            }
            None => false,
        };

        let updated = queries::get_integration(&state.db, id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;
        state.broadcast.send(
            "integration_connected",
            &serde_json::json!({
                "provider": parent.provider_identifier,
                "name": updated.provider_name,
                "refreshed": refreshed,
            }),
        );
        return Ok(Json(ConnectPageResponse {
            integration: updated.into(),
            parent_id: parent_id.to_string(),
        }));
    }

    // First connect for this page. The page must be reachable and must have a
    // token: an empty one would be stored as a working-looking integration
    // whose every publish then fails. Refuse it here, where the user can act on it.
    let page = page.ok_or_else(|| {
        AppError::BadRequest("Page not found or not accessible with this token".into())
    })?;
    let page_token = page.access_token.filter(|t| !t.is_empty()).ok_or_else(|| {
        AppError::BadRequest(
            "This page did not return an access token; reconnect with a token that manages it"
                .into(),
        )
    })?;

    let integration = queries::create_integration(
        &state.db,
        auth.user_id,
        &parent.provider_identifier,
        provider_obj.name(),
        &page.id,
        &page_token,
        parent.refresh_token.as_deref(),
        None,
        Some(&page.name),
        page.picture.as_deref(),
        None,
        Some(&parent.internal_id),
        None, // auth_method
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({
            "id": integration.id.to_string(),
            "provider": parent.provider_identifier,
            "parent_id": parent_id.to_string(),
        }),
    );

    tracing::info!(
        "Sub-account connected: {} page '{}' ({}) under parent {}",
        parent.provider_identifier,
        page.name,
        page.id,
        parent.internal_id,
    );

    Ok(Json(ConnectPageResponse {
        integration: integration.into(),
        parent_id: parent.id.to_string(),
    }))
}

// ── Target Discovery ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TargetsResponse {
    pub targets: Vec<TargetInfo>,
    pub integration_id: String,
    pub provider: String,
}

/// GET /api/integrations/{id}/targets — list discoverable posting targets
pub async fn list_targets(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    user: AuthenticatedUser,
) -> Result<Json<TargetsResponse>, AppError> {
    let integration = queries::get_integration(&state.db, id, user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let provider = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Provider '{}' not in registry",
                integration.provider_identifier
            ))
        })?;

    let token = crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

    let targets = provider
        .targets(&token)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch targets: {}", e)))?;

    Ok(Json(TargetsResponse {
        targets,
        integration_id: id.to_string(),
        provider: integration.provider_identifier.clone(),
    }))
}

// ── Mentions search ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MentionQuery {
    pub q: String,
}

#[derive(Debug, Serialize)]
pub struct MentionResponse {
    pub results: Vec<MentionItem>,
}

#[derive(Debug, Serialize)]
pub struct MentionItem {
    pub id: String,
    pub label: String,
    pub image: Option<String>,
    pub provider: String,
    pub formatted: String,
}

/// GET /api/integrations/:id/mentions?q=...
/// Search for users/accounts to @mention on the given integration's platform.
pub async fn search_mentions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<MentionQuery>,
    user: AuthenticatedUser,
) -> Result<Json<MentionResponse>, AppError> {
    let integration = queries::get_integration(&state.db, id, user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Integration not found".into()))?;

    let provider = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Provider '{}' not in registry",
                integration.provider_identifier
            ))
        })?;

    let token = crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

    let mentions = provider
        .search_mention(&token, &query.q)
        .await
        .map_err(|e| AppError::Internal(format!("Mention search failed: {}", e)))?;

    let results = mentions
        .into_iter()
        .map(|m| MentionItem {
            formatted: provider.format_mention(&m.id, &m.label),
            id: m.id,
            label: m.label,
            image: m.image,
            provider: integration.provider_identifier.clone(),
        })
        .collect();

    Ok(Json(MentionResponse { results }))
}
