// ─── Post list filter + reschedule parity ──────────────────────
// Two parity findings from the 2026-11-12 audit:
//
// 1. A list filter with a TYPO silently listed EVERY post. The query fell
//    back to "no filter" for any state it did not recognise, so
//    `--state publishd` answered with the whole list — a silent wrong
//    answer, and the CLI's own recipes flagged it as a trap.
//
// 2. `switchpost posts schedule <published-id> <t>` answered "Post not
//    found after scheduling". The post was there; it was PUBLISHED, and the
//    reschedule UPDATE only touches `idea|draft|queued|error`. The message
//    sent the founder looking for a post that was never missing, and there
//    was no way to re-schedule a published post from the shell.
//
// The CLI and the MCP tool now call the REST handlers, so these tests pin
// the shared behaviour.
//
// Run: cargo test --test post_list_schedule_parity_test
// Uses the isolated test DB from `load_dotenv_for_tests` (never data/switchpost.db).

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use switchpost::api::{build_router, AppState};
use switchpost::auth::jwt::create_token;
use switchpost::auth::middleware::{DEFAULT_USER_ID, SESSION_COOKIE};
use switchpost::config::Config;
use switchpost::db;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;
use tower::ServiceExt;
use uuid::Uuid;

async fn state() -> AppState {
    switchpost::config::load_dotenv_for_tests();
    let config = Config::from_env().expect("config");
    let db = db::create_pool(&config.database_url).await.expect("pool");
    db::ensure_local_user(&db).await.expect("local user");
    let registry = ProviderRegistry::new(&config, None, None);
    AppState {
        db,
        config: config.clone(),
        broadcast: Broadcaster::new(),
        providers: registry,
        token_key: None,
        telegram_client_manager: None,
        wa_client: None,
        media_http_client: reqwest::Client::new(),
        media_wreq_client: wreq::Client::new(),
    }
}

async fn seed_integration(state: &AppState) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO integrations
            (id, user_id, provider_identifier, provider_name, internal_id,
             access_token, refresh_token, token_expires_at,
             disabled, refresh_needed, posting_times, auth_method)
         VALUES (?, ?, 'x', 'X', ?, 'tok', NULL, NULL, 0, 0, '[]', 'oauth')",
    )
    .bind(id)
    .bind(DEFAULT_USER_ID)
    .bind(id.to_string())
    .execute(&state.db)
    .await
    .expect("seed the integration");
    id
}

async fn seed_post(state: &AppState, integration_id: Uuid, state_name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO posts
            (id, user_id, integration_id, state, content, title, media, settings,
             scheduled_at, published_at, platform_post_id, platform_post_url,
             error_message, created_at, updated_at, repeat_interval_days,
             repeat_end_date, group_id, first_comment, sequence, idempotency_key)
         VALUES (?, ?, ?, ?, 'hello', NULL, '[]', '{}', NULL, NULL, NULL, NULL,
                 NULL, unixepoch(), unixepoch(), NULL, NULL, NULL, NULL, 0, ?)",
    )
    .bind(id)
    .bind(DEFAULT_USER_ID)
    .bind(integration_id)
    .bind(state_name)
    .bind(Uuid::new_v4())
    .execute(&state.db)
    .await
    .expect("seed the post");
    id
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let token = create_token(DEFAULT_USER_ID, &state.config.jwt_secret).expect("token");
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
        .header(header::ORIGIN, &state.config.frontend_url)
        .body(Body::empty())
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("router");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn reschedule(state: &AppState, post_id: Uuid, action: Option<&str>) -> (StatusCode, String) {
    let token = create_token(DEFAULT_USER_ID, &state.config.jwt_secret).expect("token");
    let mut body = serde_json::json!({ "scheduled_at": "2030-01-02T03:04:05Z" });
    if let Some(a) = action {
        body["action"] = serde_json::Value::String(a.to_string());
    }
    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/posts/{post_id}/date"))
        .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
        .header(header::ORIGIN, &state.config.frontend_url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("router");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn the_state_vocabulary_is_one_list() {
    for good in ["draft", "queued", "published", "error", "errored", "idea"] {
        assert!(
            switchpost::db::queries::parse_post_state_filter(good).is_some(),
            "{good} must be a valid filter"
        );
    }
    for bad in ["publishd", "eror", "", "DRAFTED"] {
        assert!(
            switchpost::db::queries::parse_post_state_filter(bad).is_none(),
            "{bad} must NOT be a valid filter"
        );
    }
}

#[tokio::test]
async fn a_typo_in_the_state_filter_is_refused_with_the_vocabulary() {
    let state = state().await;
    let (status, body) = get(&state, "/api/posts?state=publishd").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("draft") && body.contains("published"),
        "the error must name the vocabulary, got: {body}"
    );
}

#[tokio::test]
async fn a_valid_state_filter_still_works() {
    let state = state().await;
    let integration = seed_integration(&state).await;
    seed_post(&state, integration, "error").await;
    let (status, _) = get(&state, "/api/posts?state=error").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn rescheduling_a_published_post_without_an_action_is_refused_honestly() {
    let state = state().await;
    let integration = seed_integration(&state).await;
    let post_id = seed_post(&state, integration, "published").await;

    let (status, body) = reschedule(&state, post_id, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.to_lowercase().contains("published"),
        "the error must say the post is published, got: {body}"
    );
    assert!(
        !body.contains("Post not found"),
        "the error must NOT claim the post is missing — it exists: {body}"
    );
}

#[tokio::test]
async fn a_published_post_can_be_rescheduled_when_the_action_says_so() {
    let state = state().await;
    let integration = seed_integration(&state).await;
    let post_id = seed_post(&state, integration, "published").await;

    let (status, body) = reschedule(&state, post_id, Some("schedule")).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    let state_now: String = sqlx::query_scalar("SELECT state FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_one(&state.db)
        .await
        .expect("read");
    assert_eq!(
        state_now, "queued",
        "the post must be queued for a re-publish"
    );
}

#[tokio::test]
async fn a_queued_post_reschedules_without_an_action() {
    let state = state().await;
    let integration = seed_integration(&state).await;
    let post_id = seed_post(&state, integration, "queued").await;

    let (status, body) = reschedule(&state, post_id, None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
}
