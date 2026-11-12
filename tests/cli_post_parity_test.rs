// ─── CLI create parity ─────────────────────────────────────────
// The CLI's `posts create` used to call `PostService::create` directly, which
// checks only "the body is not empty" and "the body is under 2000 chars".
// Every other rule — a disabled integration, a missing required setting, the
// provider's character limit, a rejected media type — lived in the REST
// handler. So the CLI could create a post the browser refuses, and the
// failure surfaced hours later in the scheduler.
//
// The CLI now creates through `api::posts::create` (AGENTS §9.21: the CLI is
// a CLIENT of the REST surface), so the two surfaces agree by construction.
// These tests pin that agreement.
//
// Run: cargo test --test cli_post_parity_test
// Uses the isolated test DB from `load_dotenv_for_tests` (never data/switchpost.db).

use switchpost::api::AppState;
use switchpost::config::Config;
use switchpost::db;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;
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

/// A connected X account.
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
    .bind(switchpost::auth::middleware::DEFAULT_USER_ID)
    .bind(id.to_string())
    .execute(&state.db)
    .await
    .expect("seed the integration");
    id
}

#[tokio::test]
async fn the_cli_refuses_a_post_the_browser_would_refuse() {
    // The limit is the PROVIDER's, read from the registry — not a number
    // copied into the test. ASCII counts one byte and one weighted unit, so
    // a body one unit over the limit is over it under either rule.
    let state = state().await;
    let integration = seed_integration(&state).await;
    let limit = state
        .providers
        .get("x")
        .expect("the x provider is registered")
        .max_content_length();
    let long = "x".repeat(limit + 1);

    let err = switchpost::cli::run::handle_posts_create_helper(
        &state,
        switchpost::auth::middleware::DEFAULT_USER_ID,
        &long,
        &integration.to_string(),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect_err("an over-long post must be refused at create time");

    let message = err.to_string();
    assert!(
        message.contains("validation")
            || message.contains("char")
            || message.contains(&limit.to_string()),
        "the error must name the rule, got: {message}"
    );

    // And nothing was written FOR THIS CHANNEL. (The test DB is a file
    // shared by the suite, so the count is scoped to the integration this
    // test seeded.)
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE integration_id = ?")
        .bind(integration)
        .fetch_one(&state.db)
        .await
        .expect("count");
    assert_eq!(count, 0, "a refused create must not leave a row");
}

#[tokio::test]
async fn the_cli_refuses_a_disabled_integration() {
    let state = state().await;
    let integration = seed_integration(&state).await;
    sqlx::query("UPDATE integrations SET disabled = 1 WHERE id = ?")
        .bind(integration)
        .execute(&state.db)
        .await
        .expect("disable");

    let err = switchpost::cli::run::handle_posts_create_helper(
        &state,
        switchpost::auth::middleware::DEFAULT_USER_ID,
        "hello",
        &integration.to_string(),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect_err("a disabled integration must be refused");

    assert!(
        err.to_string().to_lowercase().contains("disabled"),
        "got: {err}"
    );
}

#[tokio::test]
async fn the_cli_cannot_create_a_post_in_the_published_state() {
    let state = state().await;
    let integration = seed_integration(&state).await;

    let err = switchpost::cli::run::handle_posts_create_helper(
        &state,
        switchpost::auth::middleware::DEFAULT_USER_ID,
        "hello",
        &integration.to_string(),
        None,
        None,
        None,
        None,
        None,
        Some("published"),
    )
    .await
    .expect_err("a post cannot be created as published");

    assert!(
        err.to_string().to_lowercase().contains("published"),
        "got: {err}"
    );
}
