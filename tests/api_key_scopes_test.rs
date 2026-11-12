// ─── API key scopes ────────────────────────────────────────────
// A key used to be exactly as powerful as the password session: it could
// publish, delete, and — because `POST /api/developer/api-keys` only asked
// for a session — mint MORE keys. So one leaked `sf_…` key was equivalent
// to the password, and revoking it did not evict an attacker who had
// already minted another one.
//
// These tests drive the real router: the key is created through the REST
// handler with a scope, then used against the routes its scope covers.
//
// Run: cargo test --test api_key_scopes_test
// Uses the isolated test DB from `load_dotenv_for_tests` (never data/switchpost.db).

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use serde_json::Value;
use switchpost::api::{build_router, AppState};
use switchpost::auth::jwt::create_token;
use switchpost::auth::middleware::{DEFAULT_USER_ID, SESSION_COOKIE};
use switchpost::config::Config;
use switchpost::db;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;
use tower::ServiceExt;

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

/// Create a key through the real handler with a scope, and return the raw key.
async fn create_key(state: &AppState, scope: Option<&str>) -> String {
    let token = create_token(DEFAULT_USER_ID, &state.config.jwt_secret).expect("token");
    let mut body = serde_json::json!({ "name": format!("scope probe {scope:?}") });
    if let Some(s) = scope {
        body["scope"] = Value::String(s.to_string());
    }
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/developer/api-keys")
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
    assert_eq!(
        status,
        StatusCode::OK,
        "key creation failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    let json: Value = serde_json::from_slice(&bytes).expect("json");
    json["full_key"].as_str().expect("full_key").to_string()
}

/// Call a route with an API key and return the status.
async fn call_with_key(
    state: &AppState,
    key: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> StatusCode {
    let body = body
        .map(|b| Body::from(b.to_string()))
        .unwrap_or_else(|| Body::empty());
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("router");
    res.status()
}

#[tokio::test]
async fn a_read_key_can_read_and_cannot_write() {
    let state = state().await;
    let key = create_key(&state, Some("read")).await;

    let read = call_with_key(&state, &key, Method::GET, "/api/tags", None).await;
    assert_eq!(read, StatusCode::OK, "a read key must be able to read");

    let write = call_with_key(
        &state,
        &key,
        Method::POST,
        "/api/tags",
        Some(serde_json::json!({ "name": "should not exist" })),
    )
    .await;
    assert_eq!(
        write,
        StatusCode::FORBIDDEN,
        "a read key must not be able to write"
    );
}

#[tokio::test]
async fn a_write_key_can_write_but_not_touch_the_key_console() {
    let state = state().await;
    let key = create_key(&state, Some("write")).await;

    let write = call_with_key(
        &state,
        &key,
        Method::POST,
        "/api/tags",
        Some(serde_json::json!({ "name": format!("write scope probe {}", uuid::Uuid::new_v4()) })),
    )
    .await;
    assert_eq!(write, StatusCode::OK, "a write key must be able to write");

    let console = call_with_key(&state, &key, Method::GET, "/api/developer/api-keys", None).await;
    assert_eq!(
        console,
        StatusCode::FORBIDDEN,
        "the key console needs the session, not a key"
    );
}

#[tokio::test]
async fn no_key_can_mint_another_key() {
    let state = state().await;
    for scope in [Some("read"), Some("write"), Some("admin"), None] {
        let key = create_key(&state, scope).await;
        let status = call_with_key(
            &state,
            &key,
            Method::POST,
            "/api/developer/api-keys",
            Some(serde_json::json!({ "name": "self-minted" })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a key with scope {scope:?} must not be able to mint another key"
        );
    }
}

#[tokio::test]
async fn an_admin_key_reaches_the_rest_of_the_developer_surface() {
    let state = state().await;
    let key = create_key(&state, Some("admin")).await;

    let status = call_with_key(&state, &key, Method::GET, "/api/developer/info", None).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "an admin key must reach the developer surface, got {status}"
    );
}

#[tokio::test]
async fn an_unknown_scope_is_refused_at_creation() {
    let state = state().await;
    let token = create_token(DEFAULT_USER_ID, &state.config.jwt_secret).expect("token");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/developer/api-keys")
        .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
        .header(header::ORIGIN, &state.config.frontend_url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "name": "bad scope", "scope": "superuser" }).to_string(),
        ))
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("router");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
