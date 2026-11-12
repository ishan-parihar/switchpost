// ─── API Router ────────────────────────────────────────────────
// axum HTTP router combining all route modules.
// Protected routes use the auth middleware chain.

use axum::{
    body::Body,
    extract::{Query, State},
    http::{Request, Response, StatusCode},
    middleware::{self, Next},
    Router,
};
use rust_embed::RustEmbed;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// Embedded SvelteKit static build, compiled into the binary at build time.
/// If `frontend/build/` doesn't exist at compile time, the set is empty.
/// Disable serving at runtime by setting `SERVE_FRONTEND=false`.
#[derive(RustEmbed)]
#[folder = "frontend/build"]
#[include = "*"]
struct FrontendAssets;

use crate::auth::middleware::{auth_middleware, AuthState};
use crate::config::Config;
use crate::realtime::Broadcaster;
use crate::services::telegram_client::OptionalTelegramClient;
use crate::social::registry::ProviderRegistry;
use crate::wa::OptionalWhaClient;
use sqlx::SqlitePool;

pub mod activity; // pub: tests/activity_feed_test.rs calls the handler directly
pub(crate) mod ai;
pub mod analytics; // `calculate_streak` is THE one streak measure: /api/streak, the cadence card and the MCP tool all read it
mod auth;
mod automation;
pub mod calendar; // pub: tests/wall_clock_parity_test.rs drives zone_offset_by_name (the day-key seam)
pub(crate) mod campaigns;
pub(crate) mod comments;
mod developer;
mod dms;
mod feed;
mod integrations;
// pub(crate): the MCP `media_delete` tool deletes through the ACTIVE
// STORAGE BACKEND, so an S3/R2 deployment does not leave the object in
// the bucket.
pub(crate) mod media;
mod music;
mod notifications;
mod onboard;
// pub(crate): the MCP `posts_create` tool calls the REST handler
// in-process, so the provider validation lives in ONE place (see
// mcp::rest_bridge).
pub(crate) mod posts;
pub(crate) use posts::validate_posts_for_integrations;
pub(crate) use posts::verify_tag_ownership;
pub(crate) use posts::CreatePostRequest;
mod profile;
mod rate_limit;
pub mod research; // `week_slot`/`WEEKDAYS` shared with the MCP posts_best_times fold (9.18)
pub(crate) mod rss;
pub(crate) mod sets;
mod signatures;
mod sse;
pub(crate) mod tags; // the MCP tags tools call the REST handlers (9.21)
mod v1;
pub(crate) mod webhooks;
pub mod whatsapp_webhook;

/// Shared application state available to all handlers
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Config,
    pub broadcast: Broadcaster,
    pub providers: ProviderRegistry,
    /// Optional AES-256 key for token encryption at rest (32 bytes)
    pub token_key: Option<[u8; 32]>,
    /// Shared Telegram user client (Grammers-based, lazy init)
    pub telegram_client_manager: OptionalTelegramClient,
    /// Shared WhatsApp Web client (wa-rs-based, replaces Go wacli sidecar)
    pub wa_client: OptionalWhaClient,
    /// Shared HTTP client for media proxying (CDN bypass)
    pub media_http_client: reqwest::Client,
    /// Shared HTTP client with Chrome TLS fingerprinting for X/Twitter CDN
    pub media_wreq_client: wreq::Client,
}

impl AppState {
    /// Is this provider CONFIGURED right now?
    ///
    /// Usually this is `config.provider_credentials(id).is_some()`, which
    /// reads the environment the process started with.
    ///
    /// The exception is `telegram-user`, whose MTProto credentials the
    /// founder can now supply from the app (`POST
    /// /api/integrations/connect/telegram-user/credentials`). Those are
    /// written to the config file AND swapped into the live client
    /// manager, so the live manager is the authority — the `Config`
    /// snapshot still holds the OLD value (or none) until the process
    /// restarts.
    ///
    /// Reading `Config` here would make the channels screen offer a
    /// Connect button for a channel that is already connected, and hide
    /// the credential form for one that is not, in the same response.
    pub fn provider_configured(&self, id: &str) -> bool {
        if id == "telegram-user" {
            if let Some(mgr) = self.telegram_client_manager.as_ref() {
                return mgr.credentials().is_some();
            }
        }
        self.config.provider_credentials(id).is_some()
    }

    /// Find one integration by its `(provider_identifier, internal_id)` pair —
    /// the key a platform uses for a specific sub-account (LinkedIn Page, FB
    /// page, IG account, Discord channel, …), as opposed to the
    /// provider-level match used for the parent account.
    ///
    /// `Ok(None)` means "not connected"; callers keep their own not-connected
    /// wording, so this carries no provider-specific copy. DB failures surface
    /// as `sqlx::Error` so each caller maps them into its own error type the
    /// same way it already does for `list_integrations`.
    pub async fn integration_by_internal_id(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        internal_id: &str,
    ) -> Result<Option<crate::db::models::Integration>, sqlx::Error> {
        Ok(crate::db::queries::list_integrations(&self.db, user_id)
            .await?
            .into_iter()
            .find(|i| i.provider_identifier == provider && i.internal_id == internal_id))
    }
}

/// Build the axum router with all routes
pub fn build_router(state: AppState) -> Router {
    // Extract allowed origin for CORS
    let cors_origin = state.config.frontend_url.clone();
    let cors_layer = if cors_origin == "*" || cors_origin.is_empty() {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(tower_http::cors::AllowOrigin::exact(
                cors_origin
                    .parse::<axum::http::HeaderValue>()
                    .unwrap_or(axum::http::HeaderValue::from_static("*")),
            ))
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                // PATCH is used by the kanban routes (`/api/posts/{id}/stage`
                // and `/api/posts/kanban-reorder`). It was missing from this
                // list, so in any deployment where the frontend is served
                // from a different origin than the API, the browser
                // rejected the preflight and the whole kanban board was
                // dead - the card moved and then reverted.
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    };

    // Public routes — no auth required
    let public_routes = Router::new()
        .route("/health", axum::routing::get(health_check))
        .route(
            "/ready",
            axum::routing::get({
                let db_for_ready = state.db.clone();
                move || ready_check(db_for_ready)
            }),
        )
        .route("/api/auth/login", axum::routing::post(auth::login))
        .route(
            "/api/auth/callback",
            axum::routing::get(integrations::oauth_callback),
        )
        .route(
            "/api/auth/callback/{provider}",
            axum::routing::get(integrations::oauth_callback),
        )
        .route(
            "/integrations/social/{provider}",
            axum::routing::get(integrations::oauth_callback),
        )
        .route("/api/media/{id}", axum::routing::get(media::serve_media))
        // Public onboarding — browser-accessible OAuth flow (no JWT header needed)
        .route("/setup", axum::routing::get(onboard::onboard_page))
        .route(
            "/api/public/connect/x-cookies",
            axum::routing::get(onboard::x_cookies_form).post(onboard::x_cookies_submit),
        )
        .route(
            "/api/public/connect/x-cookies/import",
            axum::routing::post(onboard::x_cookies_import),
        )
        .route(
            "/api/public/connect/reddit-cookies",
            axum::routing::get(onboard::reddit_cookies_form).post(onboard::reddit_cookies_submit),
        )
        .route(
            "/api/public/connect/reddit-cookies/import",
            axum::routing::post(onboard::reddit_cookies_import),
        )
        .route(
            "/api/public/connect/telegram-bot-token",
            axum::routing::get(onboard::telegram_bot_token_form)
                .post(onboard::telegram_bot_token_submit),
        )
        .route(
            "/api/public/connect/{provider}",
            axum::routing::get(onboard::public_connect),
        )
        // WhatsApp Cloud webhook (E6) — no auth: GET echoes Meta's
        // hub.challenge against WHATSAPP_CLOUD_VERIFY_TOKEN, POST verifies
        // the X-Hub-Signature-256 HMAC when WHATSAPP_CLOUD_APP_SECRET is set.
        .route(
            "/api/webhooks/whatsapp",
            axum::routing::get(whatsapp_webhook::verify).post(whatsapp_webhook::receive),
        )
        // The POST routes above are browser forms (login, cookie and token
        // submits) reachable without a session, so they get the same Origin
        // check as protected routes. Without it, SameSite=None on HTTPS lets
        // a cross-site auto-submit overwrite a connected account. The two
        // server-to-server webhooks are exempted by path inside the check.
        .layer(middleware::from_fn_with_state(
            CsrfState {
                allowed_origin: state.config.frontend_url.clone(),
            },
            csrf_origin_check,
        ));

    // Protected routes — auth required
    let protected_routes = Router::new()
        .route("/api/auth/me", axum::routing::get(auth::me))
        // Fetches arbitrary (allowlisted) CDN URLs on the caller's behalf, so it
        // needs the session. The browser sends sf_session on same-origin media requests.
        .route("/api/proxy-media", axum::routing::get(media::proxy_media))
        // Post counts and streak are account data, so they need the session.
        .route(
            "/api/metrics",
            axum::routing::get({
                let db_for_metrics = state.db.clone();
                move || metrics_check(db_for_metrics)
            }),
        )
        .route("/api/streak", axum::routing::get(streak_check))
        .route("/api/auth/logout", axum::routing::post(auth::logout))
        // SSE realtime stream — auth-gated to prevent event leakage on
        // networked deployments (BUG #19). The auth_middleware validates
        // the sf_session cookie; the SSE handler then subscribes.
        .route("/api/events", axum::routing::get(sse::sse_handler))
        // Read-only merged agent-activity feed (events_log + automation_logs
        // + publish_attempts). GET only — the surface has no write shape, and
        // adding one here would need its own CSRF and audit story first.
        .route("/api/activity", axum::routing::get(activity::list))
        .route(
            "/api/posts",
            axum::routing::get(posts::list).post(posts::create),
        )
        .route(
            "/api/posts/thread",
            axum::routing::post(posts::create_thread),
        )
        .route("/api/posts/validate", axum::routing::post(posts::validate))
        .route(
            "/api/ai/generate-post",
            axum::routing::post(ai::generate_post),
        )
        .route(
            "/api/ai/generate-bulk",
            axum::routing::post(ai::generate_bulk),
        )
        .route(
            "/api/ai/improve-writing",
            axum::routing::post(ai::improve_writing),
        )
        .route(
            "/api/ai/suggest-hashtags",
            axum::routing::post(ai::suggest_hashtags),
        )
        .route("/api/ai/change-tone", axum::routing::post(ai::change_tone))
        .route("/api/ai/summarize", axum::routing::post(ai::summarize))
        .route(
            "/api/providers",
            axum::routing::get(integrations::list_providers),
        )
        .route(
            "/api/posts/{id}",
            axum::routing::get(posts::get)
                .put(posts::update)
                .delete(posts::delete),
        )
        .route(
            "/api/posts/{id}/schedule",
            axum::routing::post(posts::schedule),
        )
        .route(
            "/api/posts/{id}/unschedule",
            axum::routing::post(posts::unschedule),
        )
        .route(
            "/api/posts/{id}/date",
            axum::routing::put(posts::reschedule),
        )
        .route(
            "/api/posts/{id}/publish",
            axum::routing::post(posts::publish_post),
        )
        .route(
            "/api/posts/{id}/repeat",
            axum::routing::post(posts::repeat_post),
        )
        .route(
            "/api/posts/{id}/tags",
            axum::routing::put(posts::set_post_tags),
        )
        .route(
            "/api/posts/{id}/stage",
            axum::routing::patch(campaigns::update_stage),
        )
        .route(
            "/api/posts/kanban-reorder",
            axum::routing::patch(posts::kanban_reorder),
        )
        .route("/api/posts/find-slot", axum::routing::get(posts::find_slot))
        .route(
            "/api/posts/group/{group_id}",
            axum::routing::get(posts::get_group),
        )
        // Phase 7: Campaigns + Kanban
        .route(
            "/api/campaigns",
            axum::routing::get(campaigns::list).post(campaigns::create),
        )
        .route(
            "/api/campaigns/{id}",
            axum::routing::put(campaigns::update).delete(campaigns::delete),
        )
        .route("/api/integrations", axum::routing::get(integrations::list))
        .route(
            "/api/integrations/connect/{provider}",
            axum::routing::get(integrations::connect),
        )
        .route(
            "/api/integrations/connect/{provider}/verify",
            axum::routing::post(integrations::verify_one_time_token),
        )
        .route(
            "/api/integrations/connect/whatsapp/pair",
            axum::routing::post(integrations::whatsapp_pair),
        )
        .route(
            "/api/integrations/connect/whatsapp/status",
            axum::routing::get(integrations::whatsapp_status),
        )
        .route(
            "/api/integrations/connect/telegram-user/credentials",
            axum::routing::post(integrations::telegram_user_credentials),
        )
        .route(
            "/api/integrations/connect/telegram-user/status",
            axum::routing::get(integrations::telegram_user_status),
        )
        .route(
            "/api/integrations/connect/telegram-user/request-code",
            axum::routing::post(integrations::telegram_user_request_code),
        )
        .route(
            "/api/integrations/connect/telegram-user/sign-in",
            axum::routing::post(integrations::telegram_user_sign_in),
        )
        .route(
            "/api/integrations/connect/telegram-bot/token",
            axum::routing::post(integrations::connect_telegram_bot_token),
        )
        .route(
            "/api/integrations/connect/api-key",
            axum::routing::post(integrations::connect_api_key),
        )
        .route(
            "/api/integrations/connect/x-cookie",
            axum::routing::post(integrations::connect_x_cookie),
        )
        .route(
            "/api/integrations/connect/reddit-cookie",
            axum::routing::post(integrations::connect_reddit_cookie),
        )
        .route(
            "/api/integrations/connect/github-pat",
            axum::routing::post(integrations::connect_github_pat),
        )
        .route(
            "/api/integrations/connect/web3",
            axum::routing::post(integrations::connect_web3),
        )
        .route(
            "/api/integrations/{id}",
            axum::routing::delete(integrations::delete),
        )
        .route(
            "/api/integrations/{id}/available-pages",
            axum::routing::get(integrations::available_pages),
        )
        .route(
            "/api/integrations/{parent_id}/connect-page/{page_id}",
            axum::routing::post(integrations::connect_page),
        )
        .route(
            "/api/integrations/{id}/timeslots",
            axum::routing::put(integrations::update_timeslots),
        )
        .route(
            "/api/integrations/{id}/disable",
            axum::routing::put(integrations::toggle_disable),
        )
        .route(
            "/api/integrations/{id}/refresh",
            axum::routing::post(integrations::refresh),
        )
        .route(
            "/api/integrations/{id}/targets",
            axum::routing::get(integrations::list_targets),
        )
        .route(
            "/api/integrations/{id}/mentions",
            axum::routing::get(integrations::search_mentions),
        )
        .route(
            "/api/integrations/{id}/music",
            axum::routing::get(music::search_music),
        )
        .route("/api/calendar", axum::routing::get(calendar::get))
        .route("/api/feed", axum::routing::get(feed::get))
        .route("/api/feed/import", axum::routing::post(feed::import))
        .route("/api/feed/accounts", axum::routing::get(feed::accounts))
        .route("/api/feed/analytics", axum::routing::get(feed::analytics))
        // Read-only "what should I post next" aggregation, built only from
        // data the app already stores (own post performance, imported
        // competitor posts, connected feeds). No writes, no network calls.
        .route("/api/research", axum::routing::get(research::get))
        .route(
            "/api/feed/{post_id}/comments",
            axum::routing::get(feed::get_comments),
        )
        .route(
            "/api/feed/{post_id}/repurpose",
            axum::routing::post(feed::repurpose_post),
        )
        .route(
            "/api/feed/{post_id}/save",
            axum::routing::post(feed::save_post).delete(feed::unsave_post),
        )
        .route(
            "/api/feed/{post_id}",
            axum::routing::put(feed::update_post).delete(feed::delete_post),
        )
        .route("/api/comments", axum::routing::get(comments::list))
        .route(
            "/api/comments/{id}/resolve",
            axum::routing::post(comments::resolve),
        )
        .route(
            "/api/comments/{id}/reply",
            axum::routing::post(comments::reply),
        )
        .route("/api/media", axum::routing::get(media::list))
        .route("/api/media/{id}", axum::routing::delete(media::delete))
        .route(
            "/api/media/{id}",
            axum::routing::patch(media::update),
        )
        .route("/api/analytics", axum::routing::get(analytics::get))
        .route(
            "/api/analytics/summary",
            axum::routing::get(analytics::get_summary),
        )
        .route(
            "/api/analytics/engagement",
            axum::routing::get(analytics::get_engagement),
        )
        .route(
            "/api/analytics/adherence",
            axum::routing::get(analytics::get_adherence),
        )
        .route(
            "/api/analytics/cadence",
            axum::routing::get(analytics::get_cadence),
        )
        .route(
            "/api/analytics/post/{id}",
            axum::routing::get(analytics::get_post),
        )
        .route(
            "/api/events/recent",
            axum::routing::get(analytics::get_recent_events),
        )
        // v24-4: Brand profile (upsert + get).
        .route(
            "/api/profile",
            axum::routing::get(profile::get_profile).put(profile::update_profile),
        )
        .route(
            "/api/tags",
            axum::routing::get(tags::list).post(tags::create),
        )
        .route(
            "/api/tags/{id}",
            axum::routing::get(tags::get)
                .put(tags::update)
                .delete(tags::delete),
        )
        .route(
            "/api/signatures",
            axum::routing::get(signatures::list).post(signatures::create),
        )
        .route(
            "/api/signatures/{id}",
            axum::routing::put(signatures::update).delete(signatures::delete),
        )
        .route(
            "/api/signatures/{id}/set-default",
            axum::routing::post(signatures::set_default),
        )
        .route(
            "/api/signatures/default",
            axum::routing::get(signatures::get_default),
        )
        .route(
            "/api/developer/api-keys",
            axum::routing::get(developer::list).post(developer::create),
        )
        .route(
            "/api/developer/api-keys/{id}",
            axum::routing::delete(developer::revoke),
        )
        .route(
            "/api/developer/api-keys/{id}/regenerate",
            axum::routing::post(developer::regenerate),
        )
        // Public API v1 — Bearer sf_… keys or session cookie, same as above.
        .merge(v1::router())
        .route(
            "/api/webhooks",
            axum::routing::get(webhooks::list).post(webhooks::create),
        )
        .route(
            "/api/webhooks/{id}",
            axum::routing::get(webhooks::get)
                .put(webhooks::update)
                .delete(webhooks::delete),
        )
        .route(
            "/api/webhooks/{id}/test",
            axum::routing::post(webhooks::test),
        )
        .route(
            "/api/webhooks/{id}/deliveries",
            axum::routing::get(webhooks::deliveries),
        )
        .route(
            "/api/notifications",
            axum::routing::get(notifications::list),
        )
        .route(
            "/api/notifications/unread-count",
            axum::routing::get(notifications::unread_count),
        )
        .route(
            "/api/notifications/prefs",
            axum::routing::get(notifications::get_prefs).put(notifications::update_prefs),
        )
        .route(
            "/api/notifications/{id}/read",
            axum::routing::put(notifications::mark_read),
        )
        .route(
            "/api/notifications/read-all",
            axum::routing::put(notifications::mark_all_read),
        )
        .route(
            "/api/notifications/{id}",
            axum::routing::delete(notifications::delete),
        )
        // RSS autopost
        .route(
            "/api/rss/feeds",
            axum::routing::get(rss::list_feeds).post(rss::create_feed),
        )
        .route(
            "/api/rss/feeds/{id}",
            axum::routing::delete(rss::delete_feed),
        )
        .route(
            "/api/rss/feeds/{id}/toggle",
            axum::routing::put(rss::toggle_feed),
        )
        .route(
            "/api/rss/feeds/{id}/poll",
            axum::routing::post(rss::poll_feed),
        )
        .route(
            "/api/rss/feeds/{id}/items",
            axum::routing::get(rss::list_feed_items),
        )
        .route(
            "/api/rss/feeds/{id}/items/{guid}/import",
            axum::routing::post(rss::import_item),
        )
        .route(
            "/api/sets",
            axum::routing::get(sets::list_sets).post(sets::create_set),
        )
        .route("/api/sets/{id}", axum::routing::delete(sets::delete_set))
        // DMs
        .route(
            "/api/dms/conversations",
            axum::routing::get(dms::list_conversations),
        )
        .route("/api/dms/send", axum::routing::post(dms::send_dm))
        .route(
            "/api/dms/{conversation_id}/messages",
            axum::routing::get(dms::get_messages),
        )
        // Automation
        .route(
            "/api/automation/rules",
            axum::routing::get(automation::list_rules).post(automation::create_rule),
        )
        .route(
            "/api/automation/rules/{id}",
            axum::routing::put(automation::update_rule).delete(automation::delete_rule),
        )
        .route(
            "/api/automation/rules/{id}/logs",
            axum::routing::get(automation::get_logs),
        )
        // F8 (audit §5.1b): MCP streamable-HTTP on the daemon — remote
        // agents connect by URL (POST {APP_URL}/mcp) with zero processes
        // spawned, behind the same auth as the REST API (Authorization:
        // Bearer sf_… API key, or the sf_session cookie). rmcp's tower
        // service hosts it; each MCP session gets a fresh in-process
        // SwitchpostMcpServer (thin-MCP coexistence: the stdio `switchpost
        // mcp` path is unchanged).
        .route_service(
            "/mcp",
            {
                let mcp_state = state.clone();
                let mut mcp_cfg =
                    rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default();
                // rmcp's default Host allow-list is loopback-only (DNS-
                // rebinding guard). Admit the configured APP_URL host so a
                // networked deployment stays reachable; loopback remains
                // allowed by default.
                if let Some(host) = url::Url::parse(&state.config.app_url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                {
                    mcp_cfg.allowed_hosts.push(host);
                }
                rmcp::transport::streamable_http_server::StreamableHttpService::new(
                    move || Ok(crate::mcp::SwitchpostMcpServer::new(mcp_state.clone())),
                    std::sync::Arc::new(
                        rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
                    ),
                    mcp_cfg,
                )
            },
        )
        // CSRF defense-in-depth: validates Origin (or Referer fallback)
        // on all state-changing requests. SameSite=Lax already blocks
        // cross-site POST/PUT/DELETE cookie sends, but this catches:
        //   (a) any future GET-with-side-effects route (Lax allows GET),
        //   (b) attacks if SameSite is later relaxed to None for HTTPS.
        // The check is a no-op for same-origin requests (which is the
        // only legitimate origin in single-user mode).
        .layer(middleware::from_fn_with_state(
            CsrfState {
                allowed_origin: state.config.frontend_url.clone(),
            },
            csrf_origin_check,
        ))
        // Auth middleware: validates an `Authorization: Bearer sf_…` API
        // key against `api_keys`, else the `sf_session` cookie against the
        // JWT secret derived from `APP_PASSWORD`. Injects
        // `AuthenticatedUser { user_id: DEFAULT_USER_ID }` on success.
        .layer(middleware::from_fn_with_state(
            AuthState {
                session_secret: state.config.jwt_secret.clone(),
                db: state.db.clone(),
            },
            auth_middleware,
        ));

    // Upload route lives outside the global 10 MB transport cap (applied
    // below): the handler streams to disk and enforces MAX_FILE_SIZE
    // (50 MB) incrementally, so a transport cap must not pre-empt it.
    // Every other protection is re-applied here in the same relative order
    // (trace > rate-limit > CORS > auth > CSRF), only the size cap is out.
    let upload_route = Router::new()
        .route("/api/media", axum::routing::post(media::upload))
        .layer(middleware::from_fn_with_state(
            CsrfState {
                allowed_origin: state.config.frontend_url.clone(),
            },
            csrf_origin_check,
        ))
        .layer(middleware::from_fn_with_state(
            AuthState {
                session_secret: state.config.jwt_secret.clone(),
                db: state.db.clone(),
            },
            auth_middleware,
        ))
        .layer(cors_layer.clone())
        .layer(middleware::from_fn(rate_limit::enforce))
        .layer(TraceLayer::new_for_http());

    // Global middleware stack
    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(cors_layer)
        // Per-client rate limit on login, provider connect, and media
        // upload. Declared after `cors_layer` so CORS stays outermost —
        // a 429 raised here must still carry `Access-Control-Allow-Origin`
        // or the browser reports an opaque network error instead of 429.
        .layer(middleware::from_fn(rate_limit::enforce))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(
            10 * 1024 * 1024, // 10 MB limit
        ))
        // Merged after the transport cap so POST /api/media bypasses it;
        // the handler enforces its own streaming cap instead.
        .merge(upload_route)
        .with_state(state);

    // ── Frontend serving ──────────────────────────────────────────────────
    // Priority order:
    //   1. SERVE_FRONTEND=false  → disabled entirely (API-only mode)
    //   2. FRONTEND_DIR env var  → serve from filesystem path (dev mode / custom path)
    //   3. Embedded assets       → serve from binary-embedded frontend/build (default)
    //
    // In all disabled/missing cases, /api/* routes still function normally.

    let serve_frontend = std::env::var("SERVE_FRONTEND")
        .map(|v| v.to_lowercase() != "false" && v != "0")
        .unwrap_or(true);

    if !serve_frontend {
        return app;
    }

    // Check for explicit filesystem override first (dev mode)
    let fs_override = std::env::var("FRONTEND_DIR").ok().filter(|d| !d.is_empty());

    if let Some(dir) = fs_override {
        let frontend_path = std::path::Path::new(&dir);
        if frontend_path.join("index.html").exists() {
            let index = frontend_path.join("index.html");
            return app
                .fallback_service(ServeDir::new(frontend_path).fallback(ServeFile::new(index)));
        }
        tracing::warn!("FRONTEND_DIR={dir} has no index.html — falling back to embedded assets");
    }

    // Use embedded assets (compiled into the binary)
    if FrontendAssets::get("index.html").is_some() {
        app.fallback(embedded_frontend_handler)
    } else {
        tracing::info!("No frontend assets embedded or found — running in API-only mode");
        app
    }
}

/// Serve embedded frontend assets via rust-embed.
/// Falls back to `index.html` for unknown paths (SPA client-side routing).
async fn embedded_frontend_handler(req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().trim_start_matches('/');

    // ─── The /api/* mask ──────────────────────────────────────
    // A request to an /api/* path that NO route matched used to reach this
    // SPA fallback and was answered 200 text/html. That single behaviour
    // made every real backend failure unreadable:
    //
    //   - a URL typo in a frontend API client looks exactly like a broken
    //     provider (the client's JSON.parse fails on HTML)
    //   - a MISSING route looks exactly like a route that exists and 500s
    //   - an OAuth callback hitting a mistyped provider path answers 200,
    //     so the connect flow "succeeds" while nothing was stored
    //
    // Six production defects were diagnosed from one message,
    // "Invalid JSON response from /api/…", and this fallback was the
    // reason every one of them looked the same.
    //
    // So an /api/* path is answered with a JSON 404 that NAMES the
    // missing path. The HTML fallback is for the app's own client-side
    // routes and must keep serving index.html for them — this only
    // fires for the API namespace, which is never client-routed.
    let is_api_path = req
        .uri()
        .path()
        .split('/')
        .find(|seg| !seg.is_empty())
        .map(|seg| seg.eq_ignore_ascii_case("api"))
        .unwrap_or(false);
    if is_api_path {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(format!(
                "{{\"error\":\"No such endpoint: {}\",\"code\":\"not_found\"}}",
                req.uri().path()
            )))
            .expect("static json 404 is valid");
    }

    // Try exact path first, then with index.html appended (for directory routes)
    let asset = FrontendAssets::get(path).or_else(|| {
        if path.is_empty() || path.ends_with('/') {
            FrontendAssets::get(&format!("{path}index.html"))
        } else {
            None
        }
    });

    match asset {
        Some(file) => {
            // Use the resolved asset key for MIME guessing, not the original
            // request path — empty path "/" would guess octet-stream instead
            // of text/html for the index.html fallback.
            let resolved_key = if FrontendAssets::get(path).is_some() {
                path.to_string()
            } else if path.is_empty() || path.ends_with('/') {
                format!("{path}index.html")
            } else {
                path.to_string()
            };
            let mime = mime_guess::from_path(&resolved_key).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime.as_ref())
                // Cache immutable hashed assets aggressively; HTML never
                .header(
                    "cache-control",
                    if path.contains("/_app/immutable/") {
                        "public, max-age=31536000, immutable"
                    } else {
                        "no-cache"
                    },
                )
                .body(Body::from(file.data.into_owned()))
                .unwrap_or_else(|_| not_found())
        }
        // SPA fallback — serve index.html for any unknown path (client-side routing)
        None => match FrontendAssets::get("index.html") {
            Some(index) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/html; charset=utf-8")
                .header("cache-control", "no-cache")
                .body(Body::from(index.data.into_owned()))
                .unwrap_or_else(|_| not_found()),
            None => not_found(),
        },
    }
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not found"))
        .expect("static 404 response is valid")
}

/// Health check endpoint — liveness probe (no DB call, returns 200 if
/// the process is up and the router is dispatching).
async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Global drain flag — set to `true` when the process receives
/// SIGTERM/SIGINT. The `/ready` endpoint checks this and returns 503
/// so the load balancer stops routing traffic during the shutdown
/// drain window. This prevents new HTTP requests from creating posts
/// that will be killed mid-publish when the process exits.
static DRAINING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Mark the process as draining — called from main.rs on shutdown signal.
pub fn set_draining() {
    DRAINING.store(true, std::sync::atomic::Ordering::SeqCst);
    tracing::info!("Process marked as draining — /ready will return 503");
}

/// Readiness check — pings the DB with `SELECT 1` to verify the
/// connection pool is healthy. Container orchestrators should probe
/// `/ready` (not `/health`) before routing traffic, so a database
/// outage takes the instance out of rotation without killing the
/// process (which would prevent it from reconnecting).
///
/// Returns 503 if the process is draining (shutdown in progress) so
/// the load balancer stops sending new traffic. Existing in-flight
/// requests continue to be served — only new connections are
/// de-registered by the LB.
async fn ready_check(
    db: SqlitePool,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    // If we're draining, return 503 immediately — don't even ping the DB.
    if DRAINING.load(std::sync::atomic::Ordering::SeqCst) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "draining",
                "database": true,
                "version": env!("CARGO_PKG_VERSION"),
            })),
        ));
    }

    // §6.3: scheduler liveness. A panicked scheduler task used to leave
    // /ready green while NO publishes ran. If the last tick is older than
    // 3 poll periods the process is up but not publishing — report 503 so
    // the operator (or systemd watchdog) notices.
    let scheduler_ok =
        crate::scheduler::last_tick_age_secs() <= crate::scheduler::TICK_STALE_AFTER_SECS as i64;

    let db_ok = match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&db)
        .await
    {
        Ok(1) => true,
        Ok(_) => false,
        Err(e) => {
            tracing::warn!("Readiness check DB ping failed: {e}");
            false
        }
    };
    if !scheduler_ok {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "scheduler_stalled",
                "database": db_ok,
                "last_tick_age_secs": crate::scheduler::last_tick_age_secs(),
                "version": env!("CARGO_PKG_VERSION"),
            })),
        ));
    }
    let status = if db_ok { "ok" } else { "degraded" };
    Ok(axum::Json(serde_json::json!({
        "status": status,
        "database": db_ok,
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

/// Metrics endpoint — returns per-platform publish counts, queue
/// depth, and token-expiry status. Designed for Prometheus scrape
/// (JSON now; can add `text/plain; version=0.0.4` format later).
///
/// This is the operator's primary observability surface for the
/// scheduler. Without it, the only way to see "X failed 47 times in
/// the last hour" was to grep logs.
///
/// Uses runtime `sqlx::query` (not the `query!` macro) so the build
/// doesn't require a live DB or the .sqlx offline cache for these
/// new aggregate queries.
async fn metrics_check(db: SqlitePool) -> axum::Json<serde_json::Value> {
    #[derive(Default, sqlx::FromRow)]
    struct PostCounts {
        draft: i64,
        queued: i64,
        publishing: i64,
        published: i64,
        error: i64,
    }

    #[derive(Default, sqlx::FromRow)]
    struct IntegrationCounts {
        total: i64,
        disabled: i64,
        refresh_needed: i64,
        expiring_soon: i64,
    }

    #[derive(Default, sqlx::FromRow)]
    struct AttemptCounts {
        success: i64,
        failed: i64,
    }

    let post_counts: PostCounts = sqlx::query_as(
        "SELECT
            COUNT(*) FILTER (WHERE state = 'draft') as draft,
            COUNT(*) FILTER (WHERE state = 'queued') as queued,
            COUNT(*) FILTER (WHERE state = 'publishing') as publishing,
            COUNT(*) FILTER (WHERE state = 'published') as published,
            COUNT(*) FILTER (WHERE state = 'error') as error
           FROM posts",
    )
    .fetch_one(&db)
    .await
    .unwrap_or_default();

    let integration_counts: IntegrationCounts = sqlx::query_as(
        "SELECT
            COUNT(*) as total,
            COUNT(*) FILTER (WHERE disabled = true) as disabled,
            COUNT(*) FILTER (WHERE refresh_needed = true) as refresh_needed,
             COUNT(*) FILTER (WHERE token_expires_at IS NOT NULL AND token_expires_at < unixepoch() + 86400) as expiring_soon
           FROM integrations",
    )
    .fetch_one(&db)
    .await
    .unwrap_or_default();

    let recent_attempts: AttemptCounts = sqlx::query_as(
        "SELECT
            COUNT(*) FILTER (WHERE status = 'success') as success,
            COUNT(*) FILTER (WHERE status = 'failed') as failed
           FROM publish_attempts
           WHERE started_at > unixepoch() - 3600",
    )
    .fetch_one(&db)
    .await
    .unwrap_or_default();

    axum::Json(serde_json::json!({
        "posts": {
            "draft": post_counts.draft,
            "queued": post_counts.queued,
            "publishing": post_counts.publishing,
            "published": post_counts.published,
            "error": post_counts.error,
        },
        "integrations": {
            "total": integration_counts.total,
            "disabled": integration_counts.disabled,
            "refresh_needed": integration_counts.refresh_needed,
            "expiring_soon": integration_counts.expiring_soon,
        },
        "publish_attempts_last_1h": {
            "success": recent_attempts.success,
            "failed": recent_attempts.failed,
        },
        "draining": DRAINING.load(std::sync::atomic::Ordering::SeqCst),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Streak endpoint — returns the user's current posting streak.
/// Used by the frontend flame icon in the top bar.
///
/// THE ONE STREAK MEASURE: computed through `analytics::calculate_streak`
/// (consecutive founder's-zone days ending today or yesterday), the same
/// number the cadence card shows. It used to read `users.streak_days`,
/// the scheduler's separate 24h-window counter, so the two "streak"
/// numbers on screen could disagree across a midnight boundary.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct StreakQuery {
    /// The founder's zone as an IANA name. The badge sends it, so the
    /// TopBar number matches the cadence card in the same zone.
    pub tz: Option<String>,
}

pub(crate) async fn streak_check(
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthenticatedUser,
    Query(query): Query<StreakQuery>,
) -> Result<axum::Json<serde_json::Value>, crate::error::AppError> {
    use crate::error::AppError;
    let zone = match query.tz.as_deref() {
        Some(name) => Some(crate::api::calendar::parse_zone(name).map_err(AppError::BadRequest)?),
        None => None,
    };
    // Computed, not stored: the scheduler's counter stays as a cheap
    // hint, but the DISPLAYED streak is this one so there is only ever
    // one number the founder can read.
    // THE SESSION'S user, not the hard-coded DEFAULT_USER_ID: the
    // constant is only the JWT's subject convention; on an instance
    // seeded with a different users row the constant owns NO posts and
    // the badge reads 0 forever. `AuthenticatedUser` resolves the
    // session to the founder, exactly like the cadence handler.
    let streak = crate::api::analytics::calculate_streak(&state.db, auth.user_id, zone).await;
    Ok(axum::Json(serde_json::json!({
        "streak_days": streak,
        "streak_since": null,
    })))
}

/// State carried by the CSRF middleware.
#[derive(Clone)]
struct CsrfState {
    /// The single allowed origin for state-changing requests.
    /// Defaults to `frontend_url` from config (which itself defaults
    /// to `app_url`). Set to `*` to disable the check (not recommended).
    allowed_origin: String,
}

/// CSRF defense-in-depth: for any non-GET/non-HEAD/non-OPTIONS request,
/// verify that the `Origin` header (or `Referer` as fallback) matches
/// the configured allowed origin. Rejects with 403 otherwise.
///
/// This is a defense-in-depth layer on top of SameSite=Lax cookies:
/// Lax already blocks cross-site POST/PUT/DELETE cookie sends, but
/// (a) GET requests are still allowed cross-site under Lax, and any
/// future GET-with-side-effects route would be vulnerable, and
/// (b) if SameSite is relaxed to `None` for HTTPS deployment, this
/// becomes the primary CSRF defense.
///
/// Safe methods (GET, HEAD, OPTIONS) are not checked — they should
/// not have side effects. If a future GET route ever does have a
/// side effect, fix the route, not this middleware.
async fn csrf_origin_check(
    State(csrf): State<CsrfState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    use axum::http::{header, Method};

    // Skip check for safe methods.
    let method = req.method().clone();
    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }

    // /mcp is a non-browser JSON-RPC endpoint (F8, audit §5.1b): MCP
    // clients send no Origin/Referer at all, so this check would reject
    // every legitimate one. Its cookie-CSRF class is already covered by
    // SameSite=Lax (which blocks cross-site POST cookie sends), and the
    // auth middleware still gates it — Bearer API key or session cookie.
    // Server-to-server webhooks send no Origin either: Meta signs its
    // payloads instead (whatsapp_webhook.rs).
    if matches!(req.uri().path(), "/mcp" | "/api/webhooks/whatsapp") {
        return next.run(req).await;
    }

    // A Bearer key is never sent by a browser on its own (no ambient
    // credential), so a cross-site page cannot forge one. Non-browser clients
    // send no Origin, and the auth middleware still validates the key.
    if req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer "))
    {
        return next.run(req).await;
    }

    // If allowed_origin is `*`, the check is disabled (operator opted out).
    if csrf.allowed_origin == "*" || csrf.allowed_origin.is_empty() {
        return next.run(req).await;
    }

    // Check Origin header first, then fall back to Referer.
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let referer = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Extract just the origin part of Referer (scheme://host[:port])
    // by parsing it as a URL and reading the origin string.
    let referer_origin = referer.as_deref().and_then(|r| {
        url::Url::parse(r)
            .ok()
            .map(|u| u.origin().ascii_serialization())
    });

    let request_origin = origin.clone().or(referer_origin.clone());

    // v23-9: support comma-separated allowed origins. Previously the
    // operator could only set a single origin, so a deployment reachable
    // via multiple origins (e.g. https://switchpost.example.com AND
    // http://localhost:6543 for local dev) had to set CSRF_ALLOWED_ORIGIN=*
    // (disabling the check entirely). Now the operator can set
    // FRONTEND_URL=https://a.com,http://localhost:6543 and each is checked.
    let allowed_origins: Vec<&str> = csrf
        .allowed_origin
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let allowed = match request_origin.as_deref() {
        Some(o) => allowed_origins
            .iter()
            .any(|&allowed| o == allowed || o == allowed.trim_end_matches('/')),
        None => false,
    };

    if !allowed {
        tracing::warn!(
            "CSRF check failed: origin={origin:?} referer={referer:?} allowed={}",
            csrf.allowed_origin
        );
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "Cross-origin request blocked",
                    "code": "csrf_origin_mismatch",
                })
                .to_string(),
            ))
            .expect("static 403 response is valid");
    }

    next.run(req).await
}
