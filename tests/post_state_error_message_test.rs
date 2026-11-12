// ─── Red-team: a state change must not ERASE why a post failed ─────────
//
// `posts.error_message` is the only place the founder can read WHY a post
// did not go out. Four writers set it as part of saying "this failed":
//
//   - `mark_post_error` (scheduler): the exhausted-retry path
//   - `mark_thread_rest_failed` (scheduler): predecessor failed
//   - `reclaim_stuck_publishing` (queries): stuck in `publishing` too long
//   - `reset_post_for_republish`: a deliberate clear, on re-publish
//
// One writer does NOT mean failure: `update_post_state`, which the manual
// publish path uses to release a claim it did not use, and the scheduler
// uses to push a thread part back to `queued`.
//
// That writer took an `error_message: Option<&str>` parameter and wrote it
// BARE:
//
//     UPDATE posts SET state = ?, published_at = COALESCE(?, published_at),
//        platform_post_id = COALESCE(?, platform_post_id),
//        platform_post_url = COALESCE(?, platform_post_url),
//        error_message = ?, updated_at = unixepoch() WHERE id = ?
//
// Four of its five value columns are COALESCE-guarded, and the fifth is
// not. Every one of its three call sites passes `None`, so the bare write
// binds NULL and BLANKS the column.
//
// Which matters, because one of those call sites restores a state the post
// was already in - `post.state.clone()` - and `PostState::Error` is an
// accepted state for a manual publish. So:
//
//   1. a post fails, and the founder can see why
//   2. they retry a manual publish of the same post
//   3. the thread predecessor is not published yet, so the publish is
//      SKIPPED and nothing is sent
//   4. the claim is released with `state = post.state` (Error) and
//      `error_message = None`
//   5. the failure reason is GONE, and the state still says `error`
//
// The post now reports itself as failed with no explanation, which is
// worse than either outcome: the founder cannot act on it and the UI shows
// an empty reason. Same shape as the avatar and profile_name bugs in this
// cycle, one statement later: the guard was learned for four columns and
// not applied to the fifth.
//
// Run: cargo test --test post_state_error_message_test

use switchpost::api::AppState;
use switchpost::config::Config;
use switchpost::db;
use switchpost::db::models::PostState;
use switchpost::db::queries;
use switchpost::mcp::auth::resolve_first_user;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;

/// A real integration row, because `posts.integration_id` is a foreign key
/// and SQLite enforces it. The provider is never contacted by these tests.
async fn seed_integration(pool: &db::SqlitePool, user_id: uuid::Uuid) -> uuid::Uuid {
    queries::create_integration(
        pool,
        user_id,
        "x",
        "X",
        &format!("acct-{}", uuid::Uuid::new_v4()),
        "token",
        None,
        None,
        Some("Seeded account"),
        None,
        None,
        None,
        Some("oauth"),
        None,
    )
    .await
    .expect("seed an integration for the FK")
    .id
}

/// This binary's own database, named from the test calling it. AGENTS.md
/// 9.42: a shared scratch file makes 45 processes serialise on one write
/// lock, so each binary gets one, and 9.52 adds that a per-binary file is
/// still not per-RUN - these tests seed and read only their own rows, so
/// previous runs' rows cannot satisfy an assertion.
async fn state(name: &str) -> AppState {
    switchpost::config::load_dotenv_for_tests();
    let mut config = Config::from_env().expect("config");
    let base = config.database_url.trim_end_matches(".db").to_string();
    config.database_url = format!("{base}-{name}.db");
    let pool = db::create_pool(&config.database_url).await.expect("pool");
    db::ensure_local_user(&pool).await.expect("local user");
    let registry = ProviderRegistry::new(&config, None, None);
    AppState {
        db: pool,
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

/// Release a claim on a post that is in `error`, the way the manual publish
/// path does when a thread publish is skipped. The stored failure reason
/// must survive: the state is being restored, not the post being forgiven.
#[tokio::test]
async fn releasing_a_claim_on_a_failed_post_keeps_its_failure_reason() {
    let state = state("post_state_msg_keeps").await;
    let pool = state.db.clone();
    let user_id = resolve_first_user(&state).await.expect("local user id");
    let integration_id = seed_integration(&pool, user_id).await;

    let post = queries::create_post(
        &pool,
        user_id,
        // A real integration: `posts.integration_id` is a foreign key and
        // SQLite enforces it. No provider is ever contacted.
        integration_id,
        "the body of a post that did not go out",
        None,
        &serde_json::json!([]),
        &serde_json::json!({}),
        None,
        Some(PostState::Error),
        None,
        0,
        None,
    )
    .await
    .expect("seed a failed post");

    // The reason the founder was given for the failure.
    let reason = "Thread predecessor is not published yet — publish the earlier part first";
    sqlx::query("UPDATE posts SET state = 'error', error_message = ?, updated_at = unixepoch() WHERE id = ?")
        .bind(reason)
        .bind(post.id)
        .execute(&pool)
        .await
        .expect("record the failure reason");

    let before = queries::get_post(&pool, post.id, user_id)
        .await
        .expect("read before")
        .expect("the post exists");
    assert_eq!(
        before.error_message.as_deref(),
        Some(reason),
        "the seed should hold the failure reason, otherwise this test proves \
         nothing about preservation"
    );

    // The claim release: the state the post was already in, with no new
    // message and no new platform id.
    queries::update_post_state(&pool, post.id, PostState::Error, None, None, None)
        .await
        .expect("release the claim");

    let after = queries::get_post(&pool, post.id, user_id)
        .await
        .expect("read after")
        .expect("the post still exists");
    assert_eq!(
        after.state,
        PostState::Error,
        "the state should be unchanged - this is the state being restored"
    );
    assert_eq!(
        after.error_message.as_deref(),
        Some(reason),
        "the failure reason was ERASED. The post still says `error` and now \
         says nothing about why, so the founder cannot act on it and the \
         activity log shows an empty reason. Four of this write's five value \
         columns were COALESCE-guarded; `error_message` was not."
    );
}

/// A post that NEVER had a failure reason must not acquire one. The guard
/// cannot be a blanket "always keep", or a NULL column would become a
/// sticky value no writer can clear.
#[tokio::test]
async fn a_post_that_never_failed_stays_without_a_reason() {
    let state = state("post_state_msg_none").await;
    let pool = state.db.clone();
    let user_id = resolve_first_user(&state).await.expect("local user id");
    let integration_id = seed_integration(&pool, user_id).await;

    let post = queries::create_post(
        &pool,
        user_id,
        integration_id,
        "a post that has never failed",
        None,
        &serde_json::json!([]),
        &serde_json::json!({}),
        None,
        Some(PostState::Queued),
        None,
        0,
        None,
    )
    .await
    .expect("seed a fresh post");

    queries::update_post_state(&pool, post.id, PostState::Queued, None, None, None)
        .await
        .expect("release the claim");

    let after = queries::get_post(&pool, post.id, user_id)
        .await
        .expect("read after")
        .expect("the post exists");
    assert_ne!(
        after.error_message.as_deref(),
        Some(reason()),
        "a post that never failed must not be given one - a sticky reason \
         would be worse than the erase, because no writer could clear it"
    );
}

fn reason() -> &'static str {
    "Thread predecessor is not published yet — publish the earlier part first"
}

/// The publish path must still be able to clear a reason deliberately.
/// `reset_post_for_republish` is the writer that means it, and this pins
/// that the guard above has not silently disabled the real clear.
#[tokio::test]
async fn republishing_clears_the_reason_on_purpose() {
    let state = state("post_state_msg_clear").await;
    let pool = state.db.clone();
    let user_id = resolve_first_user(&state).await.expect("local user id");
    let integration_id = seed_integration(&pool, user_id).await;

    let post = queries::create_post(
        &pool,
        user_id,
        integration_id,
        "a post that will be republished",
        None,
        &serde_json::json!([]),
        &serde_json::json!({}),
        None,
        Some(PostState::Error),
        None,
        0,
        None,
    )
    .await
    .expect("seed");

    sqlx::query("UPDATE posts SET state = 'error', error_message = ?, updated_at = unixepoch() WHERE id = ?")
        .bind("the old failure")
        .bind(post.id)
        .execute(&pool)
        .await
        .expect("record the failure reason");

    queries::reset_post_for_republish(
        &pool,
        post.id,
        user_id,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("republish");

    let after = queries::get_post(&pool, post.id, user_id)
        .await
        .expect("read after")
        .expect("the post exists");
    assert_eq!(
        after.state,
        PostState::Queued,
        "a republish requeues the post"
    );
    let msg = after.error_message.clone().unwrap_or_default();
    assert!(
        msg.trim().is_empty(),
        "republishing is the deliberate clear, and it must still clear - a \
         guard that makes `error_message` sticky breaks the reschedule \
         flow. Found: {msg:?}"
    );
}
