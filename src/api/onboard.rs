// ─── Public Onboarding Page ─────────────────────────────────────
// Browser-accessible OAuth flow: no JWT header required.
// Shows all providers with config status and clickable connect buttons.
// Auto-creates a dev user on first access.
//
// SECURITY: every value interpolated into HTML must go through
// `html_escape()`. Every value interpolated into a JS string literal
// must go through `js_escape()`. These helpers are defined at the
// bottom of this file — keep them in sync if you add new templates.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Redirect, Response},
    Form, Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::jwt;
use crate::auth::middleware::{extract_cookie, SESSION_COOKIE};
use crate::db::queries;
use crate::error::AppError;
use crate::social::tier::{all_known_identifiers, provider_meta};

use super::AppState;

/// Query params for public connect
#[derive(Debug, Deserialize)]
pub struct PublicConnectQuery {
    pub token: Option<String>,
    pub redirect_uri: Option<String>,
}

/// Query params for the onboarding page (page-picker)
#[derive(Debug, Default, Deserialize)]
pub struct OnboardQuery {
    pub pending: Option<String>,
    pub integration_id: Option<String>,
    /// A one-time setup code minted by `/setup` (preferred over `token`).
    pub code: Option<String>,
    /// A session JWT from an in-flight OAuth chain (kept for compatibility).
    pub token: Option<String>,
    pub connected: Option<String>,
    pub error: Option<String>,
    pub name: Option<String>,
}

/// Resolve the authenticated user from a request.
///
/// Accepts EITHER:
///   (a) the `sf_session` cookie (preferred — set by /api/auth/login), OR
///   (b) a `?token=<jwt>` query parameter (used by OAuth redirect chains
///       where the cookie may not be present, e.g. cross-origin callback).
///
/// Returns `Ok(user_id)` on success, or `Err` with a redirect to /login
/// if neither credential is present/valid. This closes the "free JWT
/// issuance" hole where /setup previously minted a fresh JWT for any
/// anonymous visitor — that path bypassed APP_PASSWORD entirely.
async fn resolve_authed_user(
    headers: &HeaderMap,
    credential: Option<&str>,
    jwt_secret: &str,
    pool: &sqlx::SqlitePool,
) -> Result<Uuid, AppError> {
    // (a) Try the session cookie first.
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(cookie_val) = extract_cookie(cookie_header, SESSION_COOKIE) {
        if let Ok(claims) = jwt::validate_token(cookie_val, jwt_secret) {
            return Uuid::parse_str(&claims.sub)
                .map_err(|_| AppError::BadRequest("Invalid user ID in session".into()));
        }
    }
    // (b) The credential in the URL. ONE parameter carrying either an
    // `ac_…` one-time code or — for OAuth chains already in flight — a
    // session JWT. A code works for 30 minutes; a JWT works for 30 days.
    // A URL is browser history, a proxy log line and the Referer header the
    // provider sends back, so it should carry the first.
    if let Some(cred) = credential.filter(|c| !c.trim().is_empty()) {
        if let Some(code) = cred.strip_prefix("ac_") {
            return match queries::exchange_auth_code(pool, code).await {
                Ok(user_id) => Ok(user_id),
                Err(_) => Err(AppError::BadRequest(
                    "This setup link has expired. Open /setup again for a fresh one.".into(),
                )),
            };
        }
        let claims = jwt::validate_token(cred, jwt_secret).map_err(|_| {
            AppError::BadRequest("Invalid or expired token. Log in via the WebUI first.".into())
        })?;
        return Uuid::parse_str(&claims.sub)
            .map_err(|_| AppError::BadRequest("Invalid user ID in token".into()));
    }
    // Neither cookie nor token — require login.
    Err(AppError::Unauthorized(
        "Not authenticated. Visit /login to sign in with APP_PASSWORD.".into(),
    ))
}

/// GET /setup — onboarding HTML dashboard
///
/// AUTH: requires the `sf_session` cookie OR a `?token=<credential>` query
/// param — a 30-minute `ac_…` code minted by this page, or, for OAuth chains
/// already in flight, a session JWT. Anonymous visitors
/// are rejected with 401 and the frontend redirects them to /login.
///
/// Supports query params:
///   ?pending={provider}&integration_id={id}&token={jwt} — shows page-picker for multi-step providers
///   ?connected={provider}&name={name} — shows success banner
pub async fn onboard_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OnboardQuery>,
) -> Result<Html<String>, AppError> {
    // Auth gate: rejects anonymous visitors with 401 if neither
    // session cookie nor ?token= JWT is valid. The resolved user_id
    // is intentionally not used downstream — `get_or_create_dev_user`
    // returns the canonical DEFAULT_USER_ID which is what we bind
    // integrations to in single-user mode. The call here is purely
    // an auth check.
    let authed_user_id = resolve_authed_user(
        &headers,
        query.token.as_deref(),
        &state.config.jwt_secret,
        &state.db,
    )
    .await?;
    let user = get_or_create_dev_user(&state).await?;
    // Mint a 30-MINUTE code for this render. The provider-card links and the
    // cookie forms below carry it instead of a 30-DAY session JWT: a URL is
    // browser history, a proxy log line and the Referer header the provider
    // sends back, and none of those should hold a credential that works for
    // a month. It is NOT single-use — the form the founder lands on posts
    // the same credential back inside the same 30-minute window.
    let token = queries::create_auth_code(&state.db, authed_user_id)
        .await
        .unwrap_or_default();

    // ── Fetch existing integrations ───────────────────────────
    let integrations = queries::list_integrations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let mut connected_cards = String::new();
    for integration in &integrations {
        let pid = &integration.provider_identifier;
        let (_, icon, _) = provider_meta(pid);
        let name = integration
            .profile_name
            .as_deref()
            .unwrap_or(&integration.provider_name);
        // Profile pic URL comes from upstream provider response — escape
        // it for both HTML-attribute context (quote-breakout) and JS
        // context (the `dc('{iid}')` onclick). A malicious upstream
        // could otherwise break out of `src="..."` with a `"` and inject
        // an `onerror=` handler.
        let pic_html = if let Some(ref pic) = integration.profile_picture {
            let pic_escaped = html_escape(pic);
            format!(
                r#"<img class="profile-pic" src="{pic_escaped}" alt="" onerror="this.style.display='none'" />"#
            )
        } else {
            format!(
                r#"<div class="profile-pic profile-pic-placeholder">{}</div>"#,
                icon
            )
        };
        let connected_at = integration.created_at.format("%b %e, %Y").to_string();
        let integration_id = integration.id.to_string();

        // Provider display name for the identifier
        let provider_display = provider_meta(pid).0;

        connected_cards.push_str(&format!(
            r#"<div class="connected-card" id="ic-{iid}">
                <div class="connected-avatar">{pic_html}</div>
                <div class="connected-body">
                    <div class="connected-name">{name}</div>
                    <div class="connected-provider">{provider_display}</div>
                    <div class="connected-since">Connected {connected_at}</div>
                </div>
                <div class="connected-status-badge">
                    <span class="badge badge-connected">✅ Connected</span>
                </div>
                <div class="connected-actions">
                    <button class="btn-disconnect" onclick="dc('{iid}')" title="Disconnect this account">✕</button>
                </div>
            </div>"#,
            iid = html_escape(&integration_id),
            name = html_escape(name),
            provider_display = html_escape(provider_display),
            connected_at = html_escape(&connected_at),
        ));
    }

    let has_connected = !integrations.is_empty();

    // ── All providers grid ────────────────────────────────────
    let all_providers = state.providers.all();
    let mut sorted: Vec<_> = all_providers.into_iter().collect();
    sorted.sort_by(|a, b| a.name().cmp(b.name()));

    let mut cards = String::new();

    // Count integrations per provider for multi-account display
    let mut connected_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for integration in &integrations {
        *connected_counts
            .entry(integration.provider_identifier.clone())
            .or_insert(0) += 1;
    }

    for provider in &sorted {
        let id = provider.identifier();
        let name = provider.name();
        // The LIVE configured state, not the Config snapshot: the
        // Telegram USER channel's credentials can be supplied from the
        // app, and this page has to reflect that without a restart.
        let has_creds = state.provider_configured(id);
        let uses_oauth = provider.uses_oauth();
        let is_onetime = provider.one_time_token();
        let redirect_uri = format!("{}/api/auth/callback", state.config.app_url);
        let connected_count = connected_counts.get(id).copied().unwrap_or(0);

        // ── Status badge + action ─────────────────────────────
        let badge_class: &str;
        let badge_text: String;
        let hint_text: String;
        let action_html: String;

        if connected_count > 0 {
            badge_class = "badge-connected";
            badge_text = format!("Connected ({})", connected_count);
            if id == "x" {
                hint_text = format!(
                    "{} account(s) connected — Add Another (OAuth) or Enter Cookies",
                    connected_count
                );
                action_html = format!(
                    r#"<div style="display:flex;gap:6px;flex-direction:column;">
                        <a href="/api/public/connect/{}?token={}" class="btn btn-primary" style="font-size:12px;">+ Add Another (OAuth)</a>
                        <a href="/api/public/connect/x-cookies?token={}" class="btn" style="font-size:12px;background:#f0f2f5;color:#333;border:1px solid #ccc;">🍪 Enter Cookies</a>
                    </div>"#,
                    id, token, token
                );
            } else if id == "reddit" {
                hint_text = format!(
                    "{} account(s) connected — Add Another (OAuth) or Enter Cookies",
                    connected_count
                );
                action_html = format!(
                    r#"<div style="display:flex;gap:6px;flex-direction:column;">
                        <a href="/api/public/connect/{}?token={}" class="btn btn-primary" style="font-size:12px;">+ Add Another (OAuth)</a>
                        <a href="/api/public/connect/reddit-cookies?token={}" class="btn" style="font-size:12px;background:#f0f2f5;color:#333;border:1px solid #ccc;">🍪 Enter Cookies</a>
                    </div>"#,
                    id, token, token
                );
            } else {
                hint_text = format!(
                    "{} account(s) connected — click Add Another to connect more",
                    connected_count
                );
                action_html = format!(
                    r#"<a href="/api/public/connect/{}?token={}" class="btn btn-primary">+ Add Another</a>"#,
                    id, token
                );
            }
        } else if !has_creds {
            badge_class = "badge-error";
            badge_text = "Not Configured".into();
            hint_text = provider_meta(id).2.into();
            // Reddit and X can use cookie auth without any env vars
            // Telegram Bot can accept a token directly
            if id == "reddit" {
                action_html = format!(
                    r#"<a href="/api/public/connect/reddit-cookies?token={}" class="btn btn-primary">🍪 Connect via Cookies</a>"#,
                    token
                );
            } else if id == "x" {
                action_html = format!(
                    r#"<a href="/api/public/connect/x-cookies?token={}" class="btn btn-primary">🍪 Connect via Cookies</a>"#,
                    token
                );
            } else if id == "telegram-bot" {
                action_html = format!(
                    r#"<a href="/api/public/connect/telegram-bot-token?token={}" class="btn btn-primary">🤖 Enter Bot Token</a>"#,
                    token
                );
            } else {
                action_html = format!(
                    r#"<a href="/api/public/connect/{}" class="btn btn-disabled">Not Available</a>"#,
                    id
                );
            }
        } else if is_onetime {
            badge_class = "badge-info";
            badge_text = "One-Time Token".into();
            hint_text = match id {
                "telegram-bot" => "Send /connect {code} to the Telegram bot".into(),
                "telegram-user" => "Login via telegram-cli when prompted".into(),
                "skool" => {
                    "Install Chrome extension, login to Skool, extract auth_token cookie".into()
                }
                _ => "One-time token provider — follow provider instructions".into(),
            };
            action_html = format!(
                r#"<a href="/api/public/connect/{}?token={}" class="btn btn-primary">Connect ➜</a>"#,
                id, token
            );
        } else if uses_oauth {
            badge_class = "badge-success";
            badge_text = "OAuth 2.0".into();
            hint_text = "Opens provider OAuth page in browser".into();
            // X/Twitter shows both OAuth and cookie options
            if id == "x" {
                action_html = format!(
                    r#"<div style="display:flex;gap:6px;flex-direction:column;">
                        <a href="/api/public/connect/{}?token={}" class="btn btn-primary" style="font-size:12px;">OAuth ➜</a>
                        <a href="/api/public/connect/x-cookies?token={}" class="btn" style="font-size:12px;background:#f0f2f5;color:#333;border:1px solid #ccc;">🍪 Enter Cookies</a>
                    </div>"#,
                    id, token, token
                );
            } else if id == "reddit" {
                action_html = format!(
                    r#"<div style="display:flex;gap:6px;flex-direction:column;">
                        <a href="/api/public/connect/{}?token={}" class="btn btn-primary" style="font-size:12px;">OAuth ➜</a>
                        <a href="/api/public/connect/reddit-cookies?token={}" class="btn" style="font-size:12px;background:#f0f2f5;color:#333;border:1px solid #ccc;">🍪 Enter Cookies</a>
                    </div>"#,
                    id, token, token
                );
            } else {
                action_html = format!(
                    r#"<a href="/api/public/connect/{}?token={}" class="btn btn-primary">Connect ➜</a>"#,
                    id, token
                );
            }
        } else {
            badge_class = "badge-warning";
            badge_text = "Direct Connect".into();
            hint_text = "Connects using credentials from .env".into();
            action_html = format!(
                r#"<a href="/api/public/connect/{}?token={}" class="btn btn-primary">Connect ➜</a>"#,
                id, token
            );
        };

        // ── Provider icon ──────────────────────────────────────
        let (_, icon, _) = provider_meta(id);

        cards.push_str(&format!(
            r#"<div class="card{css_connected}">
                <div class="card-icon">{icon}</div>
                <div class="card-body">
                    <div class="card-title">{name}</div>
                    <div class="card-id"><code>{id}</code></div>
                    <div class="card-status"><span class="badge {badge_class}">{badge_text}</span></div>
                    <div class="card-hint">{hint_text}</div>
                    <div class="card-redirect"><small>Redirect: <code>{redirect_uri}</code></small></div>
                </div>
                <div class="card-action">{action_html}</div>
            </div>"#,
            icon = icon,
            name = name,
            id = id,
            badge_class = badge_class,
            badge_text = badge_text,
            hint_text = hint_text,
            redirect_uri = redirect_uri,
            action_html = action_html,
            css_connected = if connected_count > 0 { " card-connected" } else { "" },
        ));
    }

    let app_url = &state.config.app_url;
    let frontend_url = &state.config.frontend_url;

    // ── Page-picker for multi-step providers ───────────────
    let page_picker_html = build_page_picker_html(
        query.pending.as_deref(),
        query.integration_id.as_deref(),
        query.token.as_deref(),
    );

    // ── Success/Error banner ───────────────────────────────
    // All query-string values are HTML-escaped before interpolation.
    // See SECURITY note at the top of this file.
    let banner_html = if let Some(connected) = &query.connected {
        let name = query.name.as_deref().unwrap_or(connected);
        format!(
            r#"<div class="banner banner-success" id="banner">
                ✅ Connected to <strong>{connected}</strong> as <strong>{name}</strong>!
                <button onclick="this.parentElement.style.display='none'" style="margin-left:12px;background:none;border:none;cursor:pointer;font-size:16px;">&times;</button>
            </div>
            <script>setTimeout(function(){{var b=document.getElementById('banner');if(b)b.style.display='none';}},8000);</script>"#,
            connected = html_escape(connected),
            name = html_escape(name),
        )
    } else if let Some(error) = &query.error {
        format!(
            r#"<div class="banner banner-error" id="banner">
                ❌ Connection failed: <strong>{error}</strong>
                <button onclick="this.parentElement.style.display='none'" style="margin-left:12px;background:none;border:none;cursor:pointer;font-size:16px;">&times;</button>
            </div>"#,
            error = html_escape(error),
        )
    } else {
        String::new()
    };

    let connected_section = if has_connected {
        format!(
            r#"<div id="connected-section">
            <h2 class="section-title">✅ Connected Channels <span class="count-badge">{count}</span></h2>
            <p class="section-subtitle">These social accounts are already linked to your Switchpost account.</p>
            <div class="connected-grid">{cards}</div>
            <h2 class="section-title" style="margin-top:28px;">🔌 Available Providers</h2>
            <p class="section-subtitle">Click <strong>Connect</strong> to add more channels.</p>
            </div>"#,
            count = integrations.len(),
            cards = connected_cards,
        )
    } else {
        String::from("<h2 class=\"section-title\">🔌 Available Providers</h2><p class=\"section-subtitle\">Click <strong>Connect</strong> to authorize a social media account. You'll be redirected to the provider's OAuth page.</p>")
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Switchpost — Channel Onboarding</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #f0f2f5; color: #1a1a2e; padding: 40px 20px;
        }}
        .container {{ max-width: 960px; margin: 0 auto; }}
        h1 {{ font-size: 28px; margin-bottom: 4px; }}
        .subtitle {{ color: #666; margin-bottom: 20px; font-size: 15px; }}
        .section-title {{ font-size: 20px; margin: 0 0 4px 0; display: flex; align-items: center; gap: 8px; }}
        .section-subtitle {{ color: #666; margin-bottom: 14px; font-size: 14px; }}
        .count-badge {{
            display: inline-flex; align-items: center; justify-content: center;
            background: #4361ee; color: white; font-size: 12px; font-weight: 600;
            border-radius: 10px; padding: 1px 9px; min-width: 22px; height: 20px;
        }}
        .user-info {{
            font-size: 13px; color: #555; margin-bottom: 20px;
            padding: 12px 16px; background: #e8f4f8; border-radius: 8px;
            border: 1px solid #b8daff; display: flex; flex-wrap: wrap; gap: 16px;
            align-items: center;
        }}
        .user-info .label {{ color: #888; }}
        .user-info code {{ font-size: 12px; background: #fff; padding: 2px 6px; border-radius: 3px; }}
        /* ── Connected accounts ────────────────────── */
        .connected-grid {{ display: flex; flex-direction: column; gap: 8px; margin-bottom: 24px; }}
        .connected-card {{
            background: white; border-radius: 10px; padding: 14px 18px;
            display: flex; align-items: center; gap: 14px;
            box-shadow: 0 1px 3px rgba(0,0,0,0.08); border: 1px solid #d4edda;
            transition: box-shadow 0.15s;
        }}
        .connected-card:hover {{ box-shadow: 0 2px 8px rgba(0,0,0,0.12); }}
        .connected-avatar {{ flex-shrink: 0; }}
        .profile-pic {{ width: 44px; height: 44px; border-radius: 50%; object-fit: cover; border: 2px solid #d4edda; }}
        .profile-pic-placeholder {{
            width: 44px; height: 44px; border-radius: 50%;
            display: flex; align-items: center; justify-content: center;
            font-size: 22px; background: #f0f2f5; border: 2px solid #e8e8e8;
        }}
        .connected-body {{ flex: 1; min-width: 0; }}
        .connected-name {{ font-weight: 600; font-size: 15px; }}
        .connected-provider {{ font-size: 13px; color: #666; }}
        .connected-since {{ font-size: 11px; color: #999; margin-top: 1px; }}
        .connected-status-badge {{ flex-shrink: 0; }}
        .connected-meta {{ flex-shrink: 0; margin-left: 8px; }}
        .connected-meta code {{ font-size: 11px; color: #aaa; }}
        .connected-actions {{ flex-shrink: 0; margin-left: 4px; }}
        .btn-disconnect {{
            background: none; border: none; cursor: pointer; font-size: 16px;
            width: 32px; height: 32px; border-radius: 50%;
            display: flex; align-items: center; justify-content: center;
            color: #dc3545; opacity: 0.4; transition: all 0.15s;
        }}
        .btn-disconnect:hover {{ opacity: 1; background: #f8d7da; }}
        /* ── Provider grid ─────────────────────────── */
        .grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(400px, 1fr)); gap: 14px; }}
        .card {{
            background: white; border-radius: 12px; padding: 18px 20px;
            display: flex; align-items: flex-start; gap: 16px;
            box-shadow: 0 1px 3px rgba(0,0,0,0.08); border: 1px solid #e8e8e8;
            transition: box-shadow 0.15s;
        }}
        .card:hover {{ box-shadow: 0 2px 8px rgba(0,0,0,0.12); }}
        .card-connected {{ border-color: #b7e4c7; background: #f6fef9; }}
        .card-icon {{ font-size: 34px; line-height: 1; flex-shrink: 0; width: 48px; text-align: center; padding-top: 2px; }}
        .card-body {{ flex: 1; min-width: 0; }}
        .card-title {{ font-weight: 600; font-size: 16px; margin-bottom: 1px; }}
        .card-id {{ margin-bottom: 5px; }}
        .card-id code {{ font-size: 12px; color: #888; }}
        .card-status {{ margin-bottom: 3px; }}
        .card-hint {{ font-size: 11px; color: #999; margin-bottom: 4px; }}
        .card-redirect code {{ font-size: 11px; color: #aaa; word-break: break-all; }}
        .card-action {{ flex-shrink: 0; padding-top: 4px; }}
        .badge {{ display: inline-block; padding: 3px 9px; border-radius: 4px; font-size: 11px; font-weight: 600; }}
        .badge-success {{ background: #d4edda; color: #155724; }}
        .badge-error {{ background: #f8d7da; color: #721c24; }}
        .badge-info {{ background: #d1ecf1; color: #0c5460; }}
        .badge-warning {{ background: #fff3cd; color: #856404; }}
        .badge-connected {{ background: #d4edda; color: #155724; }}
        .btn {{ display: inline-block; padding: 8px 18px; border-radius: 6px; text-decoration: none; font-size: 13px; font-weight: 500; transition: all 0.15s; text-align: center; white-space: nowrap; }}
        .btn-primary {{ background: #4361ee; color: white; }}
        .btn-primary:hover {{ background: #3a56d4; }}
        .btn-connected {{ background: #d4edda; color: #155724; cursor: default; border: 1px solid #b7e4c7; }}
        .btn-disabled {{ background: #e9ecef; color: #adb5bd; cursor: default; pointer-events: none; }}
        /* ── Page-picker ─────────────────────────── */
        .page-picker {{ display: none; margin-bottom: 24px; }}
        .page-picker.active {{ display: block; }}
        .page-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px; margin-top: 12px; }}
        .page-card {{ background: white; border-radius: 10px; padding: 14px; display: flex; align-items: center; gap: 12px; box-shadow: 0 1px 3px rgba(0,0,0,0.08); border: 1px solid #e8e8e8; }}
        .page-card:hover {{ box-shadow: 0 2px 8px rgba(0,0,0,0.12); }}
        .page-card img {{ width: 44px; height: 44px; border-radius: 50%; object-fit: cover; }}
        .page-card .info {{ flex: 1; min-width: 0; }}
        .page-card .name {{ font-weight: 600; font-size: 14px; }}
        .page-card .btn-connect {{ padding: 7px 16px; border-radius: 6px; border: none; background: #4361ee; color: white; font-size: 12px; font-weight: 500; cursor: pointer; flex-shrink: 0; }}
        .page-card .btn-connect:disabled {{ background: #d4edda; color: #155724; cursor: default; }}
        .page-card .btn-connect.done {{ background: #d4edda; color: #155724; cursor: default; }}
        /* ── Banner ──────────────────────────────── */
        .banner {{ padding: 14px 18px; border-radius: 10px; margin-bottom: 16px; font-size: 14px; display: flex; align-items: center; }}
        .banner-success {{ background: #d4edda; border: 1px solid #b7e4c7; color: #155724; }}
        .banner-error {{ background: #f8d7da; border: 1px solid #f5c6cb; color: #721c24; }}
        .note {{
            margin-top: 20px; font-size: 13px; color: #666;
            background: #fffbe6; padding: 10px 14px; border-radius: 8px;
            border: 1px solid #ffe58f; line-height: 1.5;
        }}
        .note code {{ font-size: 12px; background: #fff3cd; padding: 1px 5px; border-radius: 3px; }}
        .footer {{ margin-top: 28px; text-align: center; font-size: 12px; color: #aaa; border-top: 1px solid #e8e8e8; padding-top: 14px; }}
        @media (max-width: 640px) {{ .grid {{ grid-template-columns: 1fr; }} }}
    </style>
</head>
<body>
    <div class="container">
        <h1>🔗 Switchpost — Channel Onboarding</h1>

        <div class="user-info">
            <span>👤 <span class="label">User:</span> <strong>{dev_email}</strong></span>
            <span>🔗 <span class="label">App URL:</span> <code>{app_url}</code></span>
            <span>🎯 <span class="label">Frontend:</span> <code>{frontend_url}</code></span>
        </div>

        {banner_html}

        {page_picker_html}

        {connected_section}

        <div class="grid">
            {cards}
        </div>

        <div class="note">
            <strong>ℹ️ After authorizing</strong>, you'll be redirected back here — the connected channel
            appears in the <strong>Connected Channels</strong> section above. Verify with:
            <code>curl -H "Authorization: Bearer {token}" {app_url}/api/integrations</code>
        </div>

        <div class="footer">
            Switchpost v{version} · server: {app_url} · MCP stdio available with `switchpost mcp`
        </div>
    </div>
<script>
function dc(iid){{
    if(!confirm('Disconnect this account? This cannot be undone.'))return;
    var tok='{token}';
    var x=new XMLHttpRequest();
    x.open('DELETE','/api/integrations/'+iid);
    x.setRequestHeader('Authorization','Bearer '+tok);
    x.onload=function(){{
        if(x.status>=200&&x.status<300){{
            var el=document.getElementById('ic-'+iid);
            if(el)el.style.opacity='0.3';
            setTimeout(function(){{
                if(el)el.remove();
                // check if any connected cards remain
                var remaining=document.querySelectorAll('.connected-card');
                if(!remaining.length){{
                    var sec=document.getElementById('connected-section');
                    if(sec)sec.style.display='none';
                }}
                // refresh the page to update provider card states
                location.reload();
            }},300);
        }}else{{
            alert('Failed to disconnect. Check server logs.');
        }}
    }};
    x.onerror=function(){{alert('Network error. Is the server running?');}};
    x.send();
}}
</script>
</body>
</html>"#,
        dev_email = "local@switchpost",
        app_url = app_url,
        frontend_url = frontend_url,
        banner_html = banner_html,
        page_picker_html = page_picker_html,
        connected_section = connected_section,
        cards = cards,
        token = token,
        version = env!("CARGO_PKG_VERSION"),
    );

    Ok(Html(html))
}

/// GET /api/public/connect/{provider} — initiate OAuth from browser
///
/// AUTH: requires `sf_session` cookie OR `?token=<jwt>` query param.
/// Anonymous visitors are rejected with 401. The "auto-mint a fresh
/// JWT and redirect" path that existed before was a security hole —
/// it let anyone who could reach the server drive the OAuth flow
/// without knowing APP_PASSWORD.
///
/// Redirects browser directly to the OAuth provider's authorization page.
///
/// For non-OAuth providers (Telegram), returns instructions as JSON.
pub async fn public_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<PublicConnectQuery>,
) -> Result<Response, AppError> {
    let user_id = resolve_authed_user(
        &headers,
        query.token.as_deref(),
        &state.config.jwt_secret,
        &state.db,
    )
    .await?;

    if !state.provider_configured(&provider) {
        // A known-but-unset provider and an outright typo need different
        // advice, and only `provider_meta` knows the former's env hint.
        let known = all_known_identifiers();
        let needed = if known.iter().any(|k| *k == provider) {
            provider_meta(&provider).2
        } else {
            "Unknown provider. Check server logs."
        };
        return Err(AppError::BadRequest(format!(
            "❌ Provider '{provider}' is not configured.\n\nSet these in your .env:\n  {needed}\n\nThen restart the server."
        )));
    }

    let provider_obj = state
        .providers
        .get(&provider)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {provider}")))?;

    // One rule, four call sites: a caller-supplied `redirect_uri` is validated
    // against this deployment's own callback, not honoured verbatim. See
    // `social::common::resolve_redirect_uri`.
    let redirect_uri = crate::social::common::resolve_redirect_uri(
        query.redirect_uri.as_deref(),
        &state.config.app_url,
    )
    .map_err(AppError::BadRequest)?;

    if !provider_obj.uses_oauth() {
        if provider_obj.one_time_token() {
            let code = Uuid::new_v4().to_string();
            return Ok(Json(serde_json::json!({
                "type": "one_time_token",
                "provider": provider,
                "instructions": format!(
                    "Open Telegram, start a chat with the bot, and send: /connect {}",
                    code
                ),
                "code": code,
            }))
            .into_response());
        }

        let token = provider_obj.exchange_code("", "", &redirect_uri).await?;

        queries::create_integration(
            &state.db,
            user_id,
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
            state.token_key,
        )
        .await?;

        state.broadcast.send(
            "integration_connected",
            &serde_json::json!({
                "provider": provider,
                "name": token.name,
            }),
        );

        return Ok(Json(serde_json::json!({
            "type": "connected",
            "provider": provider,
            "name": token.name,
            "message": format!("Connected to {} as {}", provider, token.name),
        }))
        .into_response());
    }

    let code_verifier = crate::social::common::generate_code_verifier();
    let oauth_state = crate::social::common::generate_state();

    let auth_url = provider_obj
        .generate_auth_url(&oauth_state, &code_verifier, &redirect_uri)
        .await?;

    crate::db::queries::save_oauth_state(
        &state.db,
        &oauth_state,
        &provider,
        &code_verifier,
        Some(&format!("{}:{}", user_id, redirect_uri)),
    )
    .await?;

    tracing::info!("Public connect: {provider} for user {user_id} — redirecting to OAuth");

    Ok(Redirect::to(&auth_url.url).into_response())
}

// ── Page-picker HTML builder ─────────────────────────────────

/// Build the page-picker section HTML (CSS + body + JS) for multi-step providers.
/// Shows available pages/accounts for the user to select and connect.
///
/// SECURITY: `prov` is HTML-escaped before interpolation. `iid` and `tok`
/// are JS-escaped (they live inside JS string literals) AND HTML-escaped
/// (the whole script block is inside HTML). The client-side JS uses
/// `textContent`/`setAttribute` instead of string concatenation for any
/// server-supplied page data — preventing DOM-based XSS if a page name
/// contains quotes or angle brackets.
fn build_page_picker_html(
    pending: Option<&str>,
    integration_id: Option<&str>,
    pending_token: Option<&str>,
) -> String {
    let (prov, iid, tok) = match (pending, integration_id, pending_token) {
        (Some(p), Some(i), Some(t)) => (p, i, t),
        _ => return String::new(),
    };

    // Double-escape: js_escape first (escapes \, ', ", <, >, &), then
    // html_escape to convert the resulting \u sequences to entities. This
    // is safe because the JS-string context and the HTML context both get
    // their respective metacharacters neutralised.
    let iid_safe = html_escape(&js_escape(iid));
    let tok_safe = html_escape(&js_escape(tok));

    format!(
        r#"<div id="page-picker" style="margin-bottom:24px;">
    <h2 class="section-title"> Select which pages to connect</h2>
    <p class="section-subtitle">These pages are linked to your {prov} account. Click <strong>Connect</strong> for each one to add.</p>
    <div id="page-grid" class="page-grid"><p style="color:#888;font-size:14px;">Loading available pages...</p></div>
    <p style="margin-top:12px;"><a href="/" style="font-size:13px;color:#4361ee;"> Done back to main page</a></p>
</div>
<script>
(function(){{
var iid='{iid_safe}';
var tok='{tok_safe}';
var grid=document.getElementById('page-grid');
fetch('/api/integrations/'+iid+'/available-pages',{{headers:{{'Authorization':'Bearer '+tok}}}})
.then(function(r){{return r.ok?r.json():Promise.reject('HTTP '+r.status)}})
.then(function(d){{
if(!d.pages||!d.pages.length){{grid.innerHTML='<div class=\"note\">No pages found. <a href=\"/\">Go back</a></div>';return;}}
var h='';
d.pages.forEach(function(p){{
// Use textContent + setAttribute instead of string concatenation
// to prevent DOM-based XSS from page names containing quotes/HTML.
var card=document.createElement('div');
card.className='page-card';
var iconWrap=document.createElement('div');
iconWrap.className='card-icon';
if(p.picture){{
  var img=document.createElement('img');
  img.src=p.picture;
  img.style.width='44px';img.style.height='44px';img.style.borderRadius='50%';img.style.objectFit='cover';
  img.onerror=function(){{this.style.display='none'}};
  iconWrap.appendChild(img);
}}else{{
  iconWrap.textContent='F';
}}
var body=document.createElement('div');
body.className='card-body';
var title=document.createElement('div');
title.className='card-title';
title.textContent=p.name;  // textContent — no HTML interpretation
body.appendChild(title);
var btn=document.createElement('button');
btn.className='btn btn-primary cp';
btn.setAttribute('data-id',p.id);  // setAttribute — no quote breakout
btn.textContent='Connect';
card.appendChild(iconWrap);card.appendChild(body);card.appendChild(btn);
grid.appendChild(card);
}});
var btns=grid.querySelectorAll('.cp');
for(var i=0;i<btns.length;i++){{(function(btn){{btn.addEventListener('click',function(){{
btn.disabled=true;btn.textContent='Connecting...';
fetch('/api/integrations/'+iid+'/connect-page/'+btn.getAttribute('data-id'),{{method:'POST',headers:{{'Authorization':'Bearer '+tok}}}})
.then(function(r){{if(!r.ok)return r.text().then(function(t){{throw new Error(t)}});return r.json()}})
.then(function(){{btn.textContent='Connected';btn.className='btn btn-connected';}})
.catch(function(e){{btn.disabled=false;btn.textContent='Connect';alert('Failed: '+e.message);}});
}})}})(btns[i]);}}
}})
.catch(function(e){{grid.innerHTML='<div class=\"note\" style=\"background:#f8d7da;color:#721c24;\">Error: '+String(e).replace(/</g,'&lt;')+'</div>';}});
}})();
</script>"#,
        prov = html_escape(prov),
        iid_safe = iid_safe,
        tok_safe = tok_safe,
    )
}

#[derive(Debug, Deserialize)]
pub struct XCookieForm {
    pub token: String,
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    pub cookie_string: Option<String>,
}

/// GET /api/public/connect/x-cookies — show cookie input form with instructions
///
/// AUTH: requires `sf_session` cookie OR `?token=<jwt>` query param.
/// The previous "auto-mint a JWT and meta-refresh" path was removed
/// because it bypassed APP_PASSWORD.
pub async fn x_cookies_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
) -> Result<Html<String>, AppError> {
    let _user_id = resolve_authed_user(
        &headers,
        query.token.as_deref(),
        &state.config.jwt_secret,
        &state.db,
    )
    .await?;

    let token = query.token.clone().unwrap_or_default();
    let error = query.redirect_uri.as_deref().unwrap_or("");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Connect X/Twitter — Cookie Auth</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f0f2f5; color: #1a1a2e; padding: 40px 20px; }}
  .container {{ max-width: 640px; margin: 0 auto; }}
  h1 {{ font-size: 26px; margin-bottom: 6px; }}
  .subtitle {{ color: #666; margin-bottom: 20px; font-size: 14px; }}
  .instructions {{ background: #fffbe6; border: 1px solid #ffe58f; border-radius: 10px; padding: 18px 20px; margin-bottom: 24px; }}
  .instructions h3 {{ font-size: 15px; margin-bottom: 8px; }}
  .instructions ol {{ padding-left: 20px; font-size: 14px; line-height: 1.7; color: #333; }}
  .instructions code {{ background: #fff3cd; padding: 1px 6px; border-radius: 3px; font-size: 13px; }}
  .form-group {{ margin-bottom: 16px; }}
  label {{ display: block; font-weight: 600; font-size: 13px; margin-bottom: 4px; color: #333; }}
  input[type=text], input[type=password] {{ width: 100%; padding: 10px 14px; border: 1px solid #ccc; border-radius: 8px; font-size: 14px; font-family: monospace; }}
  input[type=text]:focus, input[type=password]:focus {{ outline: none; border-color: #4361ee; box-shadow: 0 0 0 3px rgba(67,97,238,0.15); }}
  .btn {{ display: inline-block; padding: 10px 24px; border-radius: 8px; border: none; font-size: 14px; font-weight: 600; cursor: pointer; }}
  .btn-primary {{ background: #4361ee; color: white; }}
  .btn-primary:hover {{ background: #3a56d4; }}
  .btn:disabled {{ opacity: 0.6; cursor: default; }}
  .error {{ background: #f8d7da; border: 1px solid #f5c6cb; border-radius: 8px; padding: 12px 16px; margin-bottom: 16px; color: #721c24; font-size: 14px; }}
  .note {{ margin-top: 20px; font-size: 12px; color: #999; line-height: 1.5; }}
  .back {{ display: block; margin-top: 16px; font-size: 13px; color: #4361ee; text-decoration: none; }}
</style>
</head>
<body>
<div class="container">
  <h1>𝕏 Connect X/Twitter</h1>
  <p class="subtitle">Use your browser cookies to authenticate with X's GraphQL API.</p>

  {error_html}

  <div class="instructions" style="border-left:4px solid #52c41a;background:#f6ffed;">
    <h3>✅ Recommended: Paste the full Cookie header</h3>
    <ol>
      <li>Open <strong><a href="https://x.com" target="_blank">x.com</a></strong> and log into your account</li>
      <li>Press <code>F12</code> (DevTools) → <strong>Network</strong> tab → refresh page</li>
      <li>Click any <code>x.com</code> request → <strong>Request Headers</strong> → find <code>Cookie:</code></li>
      <li><strong>Right-click → Copy value</strong> (the entire long string)</li>
      <li>Paste it into the textarea below and click <strong>Connect</strong></li>
    </ol>
    <p style="margin-top:10px;font-size:12px;color:#888;">⚠️ This includes ALL session cookies (auth_token, ct0, guest_id, kdt, twid…) for the most reliable authentication.</p>
  </div>

    <form action="/api/public/connect/x-cookies/import" method="POST" style="margin-bottom:16px;">
      <input type="hidden" name="token" value="{token}" />
      <button type="submit" name="submit" value="1" class="btn" style="background:#52c41a;color:white;width:100%;font-size:15px;padding:12px 24px;">
        🔍 Import from Browser (Zen / Chrome / Brave / Firefox)
      </button>
      <p style="font-size:12px;color:#888;margin-top:6px;text-align:center;">
        Reads X/Twitter cookies directly from your local browser profile. No copy-paste needed.
      </p>
    </form>

    <hr style="border:none;border-top:1px solid #eee;margin:20px 0;" />

    <form action="/api/public/connect/x-cookies" method="POST">
    <input type="hidden" name="token" value="{token}" />

    <div class="form-group">
      <label for="cookie_string">Full Cookie Header String</label>
      <textarea id="cookie_string" name="cookie_string" rows="4" style="width:100%;padding:10px 14px;border:1px solid #ccc;border-radius:8px;font-family:monospace;font-size:12px;" placeholder="auth_token=...; ct0=...; guest_id=...; kdt=...; twid=...; lang=en; ..."></textarea>
    </div>

    <details style="margin-bottom:16px;">
      <summary style="cursor:pointer;font-size:13px;color:#888;user-select:none;">⌨️ Manually enter auth_token + ct0 instead</summary>
      <div style="margin-top:12px;padding:12px;background:#fafafa;border-radius:8px;border:1px solid #eee;">
        <p style="font-size:12px;color:#888;margin-bottom:10px;">
          Alternative: DevTools → <strong>Application</strong> tab → <strong>Cookies</strong> → <code>x.com</code>
        </p>
        <div class="form-group">
          <label for="auth_token">auth_token</label>
          <input type="password" id="auth_token" name="auth_token" placeholder="Paste your auth_token cookie value" autocomplete="off" />
        </div>
        <div class="form-group">
          <label for="ct0">ct0 (CSRF token)</label>
          <input type="text" id="ct0" name="ct0" placeholder="Paste your ct0 cookie value" autocomplete="off" />
        </div>
      </div>
    </details>

    <button type="submit" name="submit" value="1" class="btn btn-primary">🔗 Connect X/Twitter</button>
  </form>

  <a href="/" class="back">← Back to onboarding</a>
</div>
</body>
</html>"#,
        error_html = if error.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="error">❌ {error}</div>"#,
                error = html_escape(error)
            )
        },
        token = html_escape(&token),
    );

    Ok(Html(html))
}

/// POST /api/public/connect/x-cookies — store X cookies as encrypted integration
///
/// AUTH: requires `sf_session` cookie OR `token` field in the form body.
pub async fn x_cookies_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
    Form(form): Form<XCookieForm>,
) -> Result<Response, AppError> {
    let token_opt: Option<&str> = Some(form.token.as_str()).or(query.token.as_deref());
    let user_id =
        resolve_authed_user(&headers, token_opt, &state.config.jwt_secret, &state.db).await?;

    let auth_token = form.auth_token.as_deref().unwrap_or("");
    let ct0 = form.ct0.as_deref().unwrap_or("");
    let cookie_string = form.cookie_string.as_deref().unwrap_or("");

    // Parse cookie_string if provided, extracting auth_token and ct0
    let (final_at, final_ct0, final_cookie_string) = if !cookie_string.is_empty() {
        if let Some(parsed) = crate::social::x_cookies::parse_cookie_string(cookie_string) {
            (parsed.0, parsed.1, parsed.2)
        } else {
            return Ok(Redirect::to(&format!(
                "/api/public/connect/x-cookies?token={}&redirect_uri=Could+not+parse+cookie+string.+Use+individual+fields+instead.",
                form.token
            )).into_response());
        }
    } else if !auth_token.is_empty() && !ct0.is_empty() {
        (auth_token.to_string(), ct0.to_string(), String::new())
    } else {
        return Ok(Redirect::to(&format!(
            "/api/public/connect/x-cookies?token={}&redirect_uri=Please+provide+auth_token+and+ct0,+or+a+full+Cookie+string.",
            form.token
        )).into_response());
    };

    let token_str = crate::social::x_cookies::build_cookie_token(
        &final_at,
        &final_ct0,
        Some(&final_cookie_string),
    );

    let (internal_id, profile_name, profile_picture) = {
        let mut provider = crate::social::x::XProvider::new(&state.config);
        provider.prepare_from_token(&token_str);
        match provider.get_me(&token_str).await {
            Ok(json) => {
                let data = json.get("data");
                let name = data
                    .and_then(|d| d.get("name"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("X User")
                    .to_string();
                let username = data
                    .and_then(|d| d.get("username"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let avatar = data
                    .and_then(|d| d.get("profile_image_url"))
                    .and_then(|s| s.as_str())
                    .map(String::from);
                let id = data
                    .and_then(|d| d.get("id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                tracing::info!("X cookie auth identified user: @{username} ({name}) id={id}");
                (id, name, avatar)
            }
            Err(e) => {
                tracing::warn!(
                    "X cookie auth succeeded but get_me failed: {e}. Using fallback profile."
                );
                (
                    format!("cookie-{}", &final_at[..8.min(final_at.len())]),
                    "X (Cookie Auth)".to_string(),
                    None,
                )
            }
        }
    };

    queries::create_integration(
        &state.db,
        user_id,
        "x",
        "X (Twitter)",
        &internal_id,
        &token_str,
        None,
        None,
        Some(&profile_name),
        None,
        profile_picture.as_deref(),
        None,
        Some("cookie"), // §6: explicit auth_mode — dispatch reads this, not the blob shape
        state.token_key,
    )
    .await?;

    let display = urlencoding::encode(&profile_name);
    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({ "provider": "x", "method": "cookie", "profile": profile_name }),
    );

    tracing::info!("X cookie auth connected for user {user_id} as {profile_name}");

    Ok(Redirect::to(&format!("/?connected=x&name={display}")).into_response())
}

/// POST /api/public/connect/x-cookies/import — extract cookies from local browser
pub async fn x_cookies_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
    Form(form): Form<XCookieForm>,
) -> Result<Response, AppError> {
    let token_opt: Option<&str> = Some(form.token.as_str()).or(query.token.as_deref());
    let user_id =
        resolve_authed_user(&headers, token_opt, &state.config.jwt_secret, &state.db).await?;

    let cookies = crate::social::x_cookies::extract_x_cookies()
        .ok_or_else(|| AppError::BadRequest(
            "Could not find X/Twitter cookies in any browser. Make sure you're logged into x.com in Zen, Chrome, Brave, or Firefox.".into()
        ))?;

    let token_str = crate::social::x_cookies::build_cookie_token(
        &cookies.auth_token,
        &cookies.ct0,
        Some(&cookies.cookie_string),
    );

    let (internal_id, profile_name, profile_picture) = {
        let mut provider = crate::social::x::XProvider::new(&state.config);
        provider.prepare_from_token(&token_str);
        match provider.get_me(&token_str).await {
            Ok(json) => {
                let data = json.get("data");
                let name = data
                    .and_then(|d| d.get("name"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("X User")
                    .to_string();
                let username = data
                    .and_then(|d| d.get("username"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let avatar = data
                    .and_then(|d| d.get("profile_image_url"))
                    .and_then(|s| s.as_str())
                    .map(String::from);
                let id = data
                    .and_then(|d| d.get("id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                tracing::info!(
                    "X browser import identified user: @{username} ({name}) id={id} from {}",
                    cookies.source
                );
                (id, name, avatar)
            }
            Err(e) => {
                tracing::warn!("X browser import get_me failed: {e}. Using fallback profile.");
                (
                    format!(
                        "cookie-{}",
                        &cookies.auth_token[..8.min(cookies.auth_token.len())]
                    ),
                    format!("X (Cookie Auth) — {}", cookies.source),
                    None,
                )
            }
        }
    };

    queries::create_integration(
        &state.db,
        user_id,
        "x",
        "X (Twitter)",
        &internal_id,
        &token_str,
        None,
        None,
        Some(&profile_name),
        None,
        profile_picture.as_deref(),
        None,
        Some("cookie"), // §6: explicit auth_mode — dispatch reads this, not the blob shape
        state.token_key,
    )
    .await?;

    let display = urlencoding::encode(&profile_name);
    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({ "provider": "x", "method": "browser-import", "source": cookies.source, "profile": profile_name }),
    );

    tracing::info!(
        "X cookie auth connected for user {user_id} via browser import ({}) as {profile_name}",
        cookies.source
    );

    Ok(Redirect::to(&format!("/?connected=x&name={display}")).into_response())
}

// ── Reddit Cookie Auth ────────────────────────────────────────

#[derive(Deserialize)]
pub struct RedditCookieForm {
    pub token: String,
    pub reddit_session: Option<String>,
    pub token_v2: Option<String>,
    pub cookie_string: Option<String>,
}

/// GET /api/public/connect/reddit-cookies — show cookie input form
///
/// AUTH: requires `sf_session` cookie OR `?token=<jwt>` query param.
pub async fn reddit_cookies_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
) -> Result<Html<String>, AppError> {
    let _user_id = resolve_authed_user(
        &headers,
        query.token.as_deref(),
        &state.config.jwt_secret,
        &state.db,
    )
    .await?;

    let token = query.token.clone().unwrap_or_default();
    let error = query.redirect_uri.as_deref().unwrap_or("");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Connect Reddit — Cookie Auth</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f0f2f5; color: #1a1a2e; padding: 40px 20px; }}
  .container {{ max-width: 640px; margin: 0 auto; }}
  h1 {{ font-size: 26px; margin-bottom: 6px; }}
  .subtitle {{ color: #666; margin-bottom: 20px; font-size: 14px; }}
  .instructions {{ background: #fff3e0; border: 1px solid #ffcc80; border-radius: 10px; padding: 18px 20px; margin-bottom: 24px; }}
  .instructions h3 {{ font-size: 15px; margin-bottom: 8px; }}
  .instructions ol {{ padding-left: 20px; font-size: 14px; line-height: 1.7; color: #333; }}
  .instructions code {{ background: #fff3cd; padding: 1px 6px; border-radius: 3px; font-size: 13px; }}
  .form-group {{ margin-bottom: 16px; }}
  label {{ display: block; font-weight: 600; font-size: 13px; margin-bottom: 4px; color: #333; }}
  input, textarea {{ width: 100%; padding: 10px 14px; border: 1px solid #ccc; border-radius: 8px; font-size: 14px; font-family: monospace; }}
  .btn {{ display: inline-block; padding: 10px 24px; border-radius: 8px; border: none; font-size: 14px; font-weight: 600; cursor: pointer; }}
  .btn-primary {{ background: #ff4500; color: white; }}
  .btn-primary:hover {{ background: #e03d00; }}
  .error {{ background: #f8d7da; border: 1px solid #f5c6cb; border-radius: 8px; padding: 12px 16px; margin-bottom: 16px; color: #721c24; font-size: 14px; }}
  .back {{ display: block; margin-top: 16px; font-size: 13px; color: #ff4500; text-decoration: none; }}
</style></head><body>
<div class="container">
  <h1>🤖 Connect Reddit</h1>
  <p class="subtitle">Use your browser cookies to authenticate with Reddit's API (enables voting, saving, moderation).</p>

  {error_html}

  <form action="/api/public/connect/reddit-cookies/import" method="POST" style="margin-bottom:16px;">
    <input type="hidden" name="token" value="{token}" />
    <button type="submit" name="submit" value="1" class="btn" style="background:#ff4500;color:white;width:100%;font-size:15px;padding:12px 24px;">
      🔍 Import from Browser (Zen / Chrome / Brave / Firefox)
    </button>
    <p style="font-size:12px;color:#888;margin-top:6px;text-align:center;">
      Reads Reddit cookies directly from your local browser profile. No copy-paste needed.
    </p>
  </form>

  <hr style="border:none;border-top:1px solid #eee;margin:20px 0;" />

  <div class="instructions">
    <h3>📋 Manual: Paste the full Cookie header</h3>
    <ol>
      <li>Open <strong><a href="https://www.reddit.com" target="_blank">reddit.com</a></strong> and log in</li>
      <li>Press <code>F12</code> → <strong>Network</strong> tab → refresh page</li>
      <li>Click any <code>reddit.com</code> request → <strong>Request Headers</strong> → find <code>Cookie:</code></li>
      <li><strong>Copy the entire value</strong> and paste below</li>
    </ol>
  </div>

  <form action="/api/public/connect/reddit-cookies" method="POST">
    <input type="hidden" name="token" value="{token}" />
    <div class="form-group">
      <label for="cookie_string">Full Cookie Header String</label>
      <textarea id="cookie_string" name="cookie_string" rows="4" placeholder="reddit_session=...; token_v2=...; csrf_token=...; loid=..."></textarea>
    </div>
    <details style="margin-bottom:16px;">
      <summary style="cursor:pointer;font-size:13px;color:#888;">⌨️ Manually enter reddit_session instead</summary>
      <div style="margin-top:12px;padding:12px;background:#fafafa;border-radius:8px;border:1px solid #eee;">
        <div class="form-group">
          <label for="reddit_session">reddit_session</label>
          <input type="password" id="reddit_session" name="reddit_session" placeholder="Your reddit_session cookie value" autocomplete="off" />
        </div>
        <div class="form-group">
          <label for="token_v2">token_v2 (optional)</label>
          <input type="text" id="token_v2" name="token_v2" placeholder="Your token_v2 cookie value (optional)" autocomplete="off" />
        </div>
      </div>
    </details>
    <button type="submit" name="submit" value="1" class="btn btn-primary">🔗 Connect Reddit</button>
  </form>
  <a href="/" class="back">← Back to onboarding</a>
</div></body></html>"#,
        error_html = if error.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="error">❌ {error}</div>"#,
                error = html_escape(error)
            )
        },
        token = html_escape(&token),
    );
    Ok(Html(html))
}

/// POST /api/public/connect/reddit-cookies — submit cookies manually
///
/// AUTH: requires `sf_session` cookie OR `token` field in the form body.
pub async fn reddit_cookies_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
    Form(form): Form<RedditCookieForm>,
) -> Result<Response, AppError> {
    let token_opt: Option<&str> = Some(form.token.as_str()).or(query.token.as_deref());
    let user_id =
        resolve_authed_user(&headers, token_opt, &state.config.jwt_secret, &state.db).await?;

    let cookie_string = form.cookie_string.as_deref().unwrap_or("");
    let reddit_session = form.reddit_session.as_deref().unwrap_or("");
    let token_v2 = form.token_v2.as_deref().unwrap_or("");

    let (final_session, final_token_v2, final_cookie_string) = if !cookie_string.is_empty() {
        if let Some(parsed) = crate::social::reddit_cookies::parse_cookie_string(cookie_string) {
            (
                parsed.reddit_session,
                parsed.token_v2,
                Some(parsed.cookie_string),
            )
        } else {
            return Ok(Redirect::to(&format!(
                "/api/public/connect/reddit-cookies?token={}&redirect_uri=Could+not+parse+cookie+string.+Ensure+it+contains+reddit_session.",
                form.token
            )).into_response());
        }
    } else if !reddit_session.is_empty() {
        (
            reddit_session.to_string(),
            if token_v2.is_empty() {
                None
            } else {
                Some(token_v2.to_string())
            },
            None,
        )
    } else {
        return Ok(Redirect::to(&format!(
            "/api/public/connect/reddit-cookies?token={}&redirect_uri=Please+provide+reddit_session+or+a+full+Cookie+string.",
            form.token
        )).into_response());
    };

    let token_str = crate::social::reddit_cookies::build_cookie_token(
        &final_session,
        final_token_v2.as_deref(),
        final_cookie_string.as_deref(),
    );

    // Validate by fetching /api/me.json
    let (internal_id, profile_name, profile_picture) = {
        let mut provider = crate::social::reddit::RedditProvider::new(&state.config);
        provider.prepare_from_token(&token_str);
        match provider.get_www("/api/me.json", &[]).await {
            Ok(json) => {
                let name = json["data"]["name"]
                    .as_str()
                    .unwrap_or("Reddit User")
                    .to_string();
                let icon = json["data"]["icon_img"]
                    .as_str()
                    .and_then(|s| s.split('?').next())
                    .map(String::from);
                let id = json["data"]["id"].as_str().unwrap_or("").to_string();
                tracing::info!("Reddit cookie auth identified user: u/{name} id={id}");
                (id, name, icon)
            }
            Err(e) => {
                tracing::warn!("Reddit cookie auth validation failed: {e}");
                return Ok(Redirect::to(&format!(
                    "/api/public/connect/reddit-cookies?token={}&redirect_uri=Cookie+validation+failed:+{}",
                    form.token, urlencoding::encode(&e.to_string())
                )).into_response());
            }
        }
    };

    queries::create_integration(
        &state.db,
        user_id,
        "reddit",
        "Reddit",
        &internal_id,
        &token_str,
        None,
        None,
        Some(&profile_name),
        None,
        profile_picture.as_deref(),
        None,
        Some("cookie"), // §6: explicit auth_mode
        state.token_key,
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({ "provider": "reddit", "method": "cookie", "profile": profile_name }),
    );

    Ok(Redirect::to(&format!(
        "/?connected=reddit&name={}",
        urlencoding::encode(&profile_name)
    ))
    .into_response())
}

/// POST /api/public/connect/reddit-cookies/import — extract from browser
///
/// AUTH: requires `sf_session` cookie OR `token` field in the form body.
pub async fn reddit_cookies_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PublicConnectQuery>,
    Form(form): Form<RedditCookieForm>,
) -> Result<Response, AppError> {
    let token_opt: Option<&str> = Some(form.token.as_str()).or(query.token.as_deref());
    let user_id =
        resolve_authed_user(&headers, token_opt, &state.config.jwt_secret, &state.db).await?;

    let cookies = crate::social::reddit_cookies::extract_reddit_cookies()
        .ok_or_else(|| AppError::BadRequest(
            "Could not find Reddit cookies in any browser. Make sure you're logged into reddit.com.".into()
        ))?;

    let token_str = crate::social::reddit_cookies::build_cookie_token(
        &cookies.reddit_session,
        cookies.token_v2.as_deref(),
        Some(&cookies.cookie_string),
    );

    let (internal_id, profile_name, profile_picture) = {
        let mut provider = crate::social::reddit::RedditProvider::new(&state.config);
        provider.prepare_from_token(&token_str);
        match provider.get_www("/api/me.json", &[]).await {
            Ok(json) => {
                let name = json["data"]["name"]
                    .as_str()
                    .unwrap_or("Reddit User")
                    .to_string();
                let icon = json["data"]["icon_img"]
                    .as_str()
                    .and_then(|s| s.split('?').next())
                    .map(String::from);
                let id = json["data"]["id"].as_str().unwrap_or("").to_string();
                tracing::info!(
                    "Reddit browser import identified user: u/{name} from {}",
                    cookies.source
                );
                (id, name, icon)
            }
            Err(e) => {
                tracing::warn!("Reddit browser import validation failed: {e}");
                (
                    format!(
                        "cookie-{}",
                        &cookies.reddit_session[..8.min(cookies.reddit_session.len())]
                    ),
                    format!("Reddit (Cookie) — {}", cookies.source),
                    None,
                )
            }
        }
    };

    queries::create_integration(
        &state.db,
        user_id,
        "reddit",
        "Reddit",
        &internal_id,
        &token_str,
        None,
        None,
        Some(&profile_name),
        None,
        profile_picture.as_deref(),
        None,
        Some("cookie"), // §6: explicit auth_mode
        state.token_key,
    )
    .await?;

    state.broadcast.send(
        "integration_connected",
        &serde_json::json!({ "provider": "reddit", "method": "browser-import", "source": cookies.source, "profile": profile_name }),
    );

    Ok(Redirect::to(&format!(
        "/?connected=reddit&name={}",
        urlencoding::encode(&profile_name)
    ))
    .into_response())
}

// ── Local user helper ─────────────────────────────────────────
//
// Single-user mode: the onboarding flow (and all public OAuth/cookie
// connect endpoints) operate on `DEFAULT_USER_ID`. There is no separate
// "dev user" anymore — `ensure_local_user` in `db/mod.rs` creates the
// row at startup, and every connect call binds the integration to that
// single user id.

/// Return the single local user row. The row is created at startup by
/// `db::ensure_local_user`; if for some reason it's missing (e.g. the
/// DB was wiped between startup and this request), we fall back to a
/// `get_user_by_id` lookup and surface a clear error if still absent.
async fn get_or_create_dev_user(state: &AppState) -> Result<crate::db::models::User, AppError> {
    let user_id = crate::auth::middleware::DEFAULT_USER_ID;
    if let Some(user) = queries::get_user_by_id(&state.db, user_id).await? {
        return Ok(user);
    }
    // Row missing — recreate it (mirrors db::ensure_local_user).
    crate::db::ensure_local_user(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to ensure local user: {e}")))?;
    queries::get_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::Internal("Local user row missing after ensure".into()))
}

// ── Telegram Bot Token Form ──────────────────────────────────

/// GET /api/public/connect/telegram-bot-token — form to enter bot token
///
/// AUTH: requires `sf_session` cookie OR `?token=<jwt>` query param.
pub async fn telegram_bot_token_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<PublicConnectQuery>,
) -> Result<axum::response::Html<String>, AppError> {
    let _user_id = resolve_authed_user(
        &headers,
        query.token.as_deref(),
        &state.config.jwt_secret,
        &state.db,
    )
    .await?;
    // Embed the JWT as a hidden form field so the POST handler can
    // re-validate auth even if the session cookie isn't sent (e.g.
    // the user reached this form via an OAuth-redirect link with
    // ?token= and hasn't logged into the WebUI to set the cookie).
    let token_hidden = query.token.as_deref().map(html_escape).unwrap_or_default();
    Ok(axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Switchpost — Telegram Bot Token</title>
<style>body{{font-family:system-ui;background:#0b0e14;color:#d1d5db;display:flex;justify-content:center;align-items:center;min-height:100vh;margin:0}}
.card{{background:#131720;border:1px solid #1e2435;border-radius:12px;padding:2rem;max-width:480px;width:100%}}
h2{{color:#fff;margin-top:0}}input{{width:100%;padding:.75rem;border:1px solid #1e2435;border-radius:8px;background:#0d1117;color:#d1d5db;margin:.5rem 0;box-sizing:border-box}}
.btn{{display:inline-block;padding:.75rem 1.5rem;background:#6366f1;color:#fff;border:none;border-radius:8px;cursor:pointer;font-size:.9rem;width:100%;margin-top:.5rem}}
.btn:hover{{background:#4f46e5}}.hint{{font-size:.8rem;color:#6b7280;margin-top:.5rem}}</style></head>
<body><div class="card"><h2>🤖 Telegram Bot Token</h2>
<p style="font-size:.9rem;color:#9ca3af">Enter your bot token from <a href="https://t.me/BotFather" style="color:#6366f1">@BotFather</a></p>
<form method="POST" action="/api/public/connect/telegram-bot-token">
<input type="hidden" name="token" value="{token_hidden}" />
<input name="bot_token" placeholder="123456789:ABCdefGHIjklMNOpqrsTUVwxyz" required>
<button type="submit" class="btn">Connect Bot</button>
</form>
<p class="hint">The token looks like: 123456789:ABCdefGHIjklMNOpqrsTUVwxyz</p>
</div></body></html>"#
    )))
}

/// POST /api/public/connect/telegram-bot-token — submit bot token
///
/// AUTH: requires `sf_session` cookie OR `token` field in the form body
/// (the form embeds it as a hidden input via the same JWT used for the
/// GET form). Anonymous submissions are rejected with 401.
pub async fn telegram_bot_token_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String, String>>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;
    // Auth gate — accept either the session cookie or a `token` form field.
    let form_token = form.get("token").map(String::as_str);
    let _user_id =
        resolve_authed_user(&headers, form_token, &state.config.jwt_secret, &state.db).await?;

    let bot_token = match form.get("bot_token") {
        Some(t) if !t.is_empty() => t.clone(),
        _ => {
            return Ok(
                axum::response::Html("<p>Error: bot_token is required</p>".to_string())
                    .into_response(),
            )
        }
    };

    // Validate token via getMe
    let url = format!("https://api.telegram.org/bot{}/getMe", bot_token);
    let resp = match reqwest::get(&url).await {
        Ok(r) => r,
        Err(e) => {
            return Ok(axum::response::Html(format!(
                "<p>Error: Failed to reach Telegram API: {e}</p>",
                // without_url(): the request URL carries the bot token.
                e = html_escape(&e.without_url().to_string())
            ))
            .into_response());
        }
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            return Ok(axum::response::Html(format!(
                "<p>Error: Invalid response: {e}</p>",
                e = html_escape(&e.to_string())
            ))
            .into_response())
        }
    };
    if !json["ok"].as_bool().unwrap_or(false) {
        let desc = json["description"].as_str().unwrap_or("unknown error");
        return Ok(axum::response::Html(format!(
            "<p>Error: Invalid bot token. Telegram says: {desc}</p>",
            desc = html_escape(desc)
        ))
        .into_response());
    }

    let bot = &json["result"];
    let bot_id = bot["id"].as_i64().unwrap_or(0).to_string();
    let bot_name = bot["first_name"].as_str().unwrap_or("Bot");
    let bot_username = bot["username"].as_str().unwrap_or("");

    let user = match get_or_create_dev_user(&state).await {
        Ok(u) => u,
        Err(e) => {
            return Ok(axum::response::Html(format!(
                "<p>Error creating user: {e}</p>",
                e = html_escape(&e.to_string())
            ))
            .into_response())
        }
    };
    let token_json = serde_json::json!({"bot_token": bot_token}).to_string();
    if let Err(e) = crate::db::queries::create_integration(
        &state.db,
        user.id,
        "telegram-bot",
        "Telegram Bot",
        &bot_id,
        &token_json,
        None,
        None,
        Some(bot_name),
        None,
        Some(&format!("https://t.me/{bot_username}")),
        None,
        Some("api_key"),
        state.token_key,
    )
    .await
    {
        tracing::error!("Failed to create Telegram Bot integration: {e}");
        let msg = html_escape(&format!(
            "Failed to save Telegram Bot connection: {e}. Please try again."
        ));
        return Ok(
            axum::response::Html(format!("<p style=\"color:red\">{msg}</p>")).into_response(),
        );
    }

    Ok(axum::response::Redirect::to("/setup?connected=telegram-bot").into_response())
}

// ── HTML/JS escaping helpers ──────────────────────────────────
//
// These are the single source of truth for any value interpolated
// into an HTML template or JS string literal in this file. Use them
// every time a value of unknown provenance (query string, DB row,
// upstream API response, form input) is rendered.
//
// Why hand-roll instead of depending on `askama`/`maud`/`v_htmlescape`?
// The onboarding templates are static strings already in this file —
// the cost of adding a template-engine dep tree for ~10 interpolation
// sites is not justified. These helpers are ~20 LOC, well-tested by
// the unit tests below, and easy to audit.

/// Escape a string for safe interpolation into HTML text content
/// or a double-quoted attribute value.
///
/// Escapes: `&`, `<`, `>`, `"`, `'`. Numeric entity form (`&#34;`)
/// is used for both quote types so the same string is safe in both
/// text content and either single- or double-quoted attributes.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a string for safe interpolation into a JavaScript single-
/// or double-quoted string literal. Prevents quote-breakout attacks
/// when the resulting JS string is later placed inside an HTML
/// `<script>` block (still requires `html_escape` afterwards to be
/// safe in the HTML context — see `build_page_picker_html` above).
///
/// Escapes: `\`, `'`, `"`, `<`, `>`, `&`, and the line terminators
/// `\n`/`\r`/`\u{2028}`/`\u{2029}` (the latter two break out of JS
/// string literals in some older engines).
pub fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_basic() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("a<b>c"), "a&lt;b&gt;c");
    }

    #[test]
    fn html_escape_quotes() {
        assert_eq!(html_escape(r#""foo""#), "&#34;foo&#34;");
        assert_eq!(html_escape("it's"), "it&#39;s");
    }

    #[test]
    fn html_escape_xss_payloads() {
        // Classic XSS payloads must produce inert output.
        assert_eq!(
            html_escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
        assert_eq!(
            html_escape(r#"<img src=x onerror="alert(1)">"#),
            "&lt;img src=x onerror=&#34;alert(1)&#34;&gt;"
        );
    }

    #[test]
    fn html_escape_empty() {
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn js_escape_basic() {
        assert_eq!(js_escape("hello"), "hello");
        assert_eq!(js_escape(r"back\slash"), r"back\\slash");
        assert_eq!(js_escape("quote'quote"), r"quote\'quote");
        assert_eq!(js_escape(r#""quoted""#), r#"\"quoted\""#);
    }

    #[test]
    fn js_escape_html_chars() {
        // <, >, & must be unicode-escaped so they can't be misread
        // by an HTML parser even if the JS string ends up in a script block.
        assert_eq!(js_escape("<script>"), "\\u003cscript\\u003e");
        assert_eq!(js_escape("a&b"), "a\\u0026b");
    }

    #[test]
    fn js_escape_breakout_payload() {
        // Classic JS-string breakout: '; alert(1); //
        // The `'` becomes `\'`, neutralising the breakout. The rest of
        // the payload is left alone because none of its chars are in
        // the escape set.
        assert_eq!(js_escape("';alert(1);//"), "\\';alert(1);//");
    }

    #[test]
    fn js_escape_line_terminators() {
        assert_eq!(js_escape("a\nb"), "a\\nb");
        assert_eq!(js_escape("a\rb"), "a\\rb");
    }
}
