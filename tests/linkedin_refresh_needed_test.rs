//! Defect A: a 404 from the provider is not a scope error.
//!
//! Every LinkedIn channel on the live deployment showed "needs reconnect"
//! while its token was VALID. The measured cause is in
//! `src/feed/mod.rs`: the post fetch read the error string, saw
//! "No virtual resource found" (LinkedIn's phrasing for a deleted page), and
//! concluded "lacks API scope" — then called
//! `mark_integration_refresh_needed`. That flag is what
//! `get_integrations_needing_refresh` filters on (`AND refresh_needed = false`,
//! db/queries.rs:519), so the channel lost its ticket to the scheduler's 24h
//! proactive token refresh (scheduler/mod.rs:508) for ever — and a token no
//! refresh keeps warm is the token that expires and starts failing later.
//! The UI told the founder to reconnect an account whose token was fine.
//!
//! The flag does NOT stop `get_due_posts` (that filter reads only
//! `i.disabled = false`), so a scheduled unsent post still went out; the
//! damage is the lost refresh, not an overnight stop.
//!
//! These tests drive the real classifier and the real DB. Each one is
//! mutation-proven: reverting the fix turns it red.

use sqlx::SqlitePool;
use uuid::Uuid;

/// An integration row shaped exactly like the live `linkedin` row: valid
/// token, months of life left, currently flagged (the bug already fired).
///
/// `expires_in_days` is a parameter because the reachability test needs a
/// token INSIDE the scheduler's 24h window, while the classifier tests need
/// one that is not (a 404 arriving on a fresh channel, as it did on the live
/// box, where the token had 59 days of life).
///
/// `internal_id` is a parameter because a founder with several LinkedIn
/// pages has several rows that differ ONLY by URN — the schema's uniqueness
/// is on `(user_id, provider_identifier, internal_id)`, so a test that wants
/// two channels has to give them two URNs, exactly as the real connect flow
/// does.
async fn seed_linkedin_with_urn(pool: &SqlitePool, urn: &str, flagged: bool, expires_in_days: i64) -> Uuid {
    let user_id = switchpost::auth::middleware::DEFAULT_USER_ID;
    switchpost::db::ensure_local_user(pool).await.unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO integrations
            (id, user_id, provider_identifier, provider_name, internal_id,
             access_token, refresh_token, token_expires_at,
             disabled, refresh_needed, posting_times, auth_method)
         VALUES (?, ?, 'linkedin', 'LinkedIn', ?, 'A validT0ken',
                 'refresh-tok', ?, 0, ?, '[]', 'oauth')",
    )
    .bind(id)
    .bind(user_id)
    .bind(urn)
    .bind(
        chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(expires_in_days))
            .unwrap()
            .timestamp(),
    )
    .bind(flagged)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// The single-channel fixture: the personal LinkedIn row, with the live
/// production token's remaining life (59 days out when the 404 arrived).
async fn seed_linkedin(pool: &SqlitePool, flagged: bool) -> Uuid {
    seed_linkedin_with_urn(pool, "urn:li:person:abc", flagged, 59).await
}

async fn is_flagged(pool: &SqlitePool, id: Uuid) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT refresh_needed FROM integrations WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
        == 1
}

// ── The classifier rule, unit level ─────────────────────────────

/// The string the live logs actually contain, from LinkedIn's `get_posts`.
const LIVE_404: &str = "API error: LinkedIn get_posts failed: HTTP 404 — No virtual resource found";

/// LinkedIn's alternate 404 envelope.
const LIVE_404_ENVELOPE: &str = "API error: RESOURCE_NOT_FOUND";

/// A 401 in LinkedIn's shape: the case the flag EXISTS for.
const LIVE_401: &str = "API error: HTTP 401 Unauthorized";

/// A 403: LinkedIn's missing-scope shape.
const LIVE_403: &str = "API error: HTTP 403 Forbidden — missing scope r_organization_social";

// ── Defect A: 404 must not set the flag ─────────────────────────

#[tokio::test]
async fn a_linkedin_404_is_not_a_scope_error() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    let kind = switchpost::feed::classify_feed_failure(LIVE_404);

    assert_eq!(
        kind,
        switchpost::feed::FeedFailureKind::TargetGone,
        "LinkedIn's 404 phrasing means the page is gone, not that a scope is missing"
    );

    // The end-to-end assertion: driving the failure leaves the channel
    // PUBLISHABLE. Pre-fix this was true — the flag got set, and
    // `get_integrations_for_refresh`'s `AND refresh_needed = false`
    // (db/queries.rs) excluded it from the refresh window for ever.
    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", LIVE_404)
        .await
        .unwrap();
    assert!(
        !is_flagged(&pool, id).await,
        "a 404 must NOT mark the channel refresh_needed — it stays publishable"
    );
}

#[tokio::test]
async fn the_resource_not_found_envelope_is_also_not_a_scope_error() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", LIVE_404_ENVELOPE)
        .await
        .unwrap();
    assert!(
        !is_flagged(&pool, id).await,
        "LinkedIn's RESOURCE_NOT_FOUND envelope is the same 404, not a scope error"
    );
}

/// The negative-direction guard: the classifier is not vacuous. A real
/// auth failure MUST still set the flag — that is what the flag is for, and
/// a "fix" that never sets it would break the reconnect prompt entirely.
#[tokio::test]
async fn a_real_auth_failure_still_marks_refresh_needed() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", LIVE_401)
        .await
        .unwrap();
    assert!(
        is_flagged(&pool, id).await,
        "a 401 is a genuine token failure and must still offer the reconnect"
    );
}

#[tokio::test]
async fn a_missing_scope_forbidden_still_marks_refresh_needed() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", LIVE_403)
        .await
        .unwrap();
    assert!(
        is_flagged(&pool, id).await,
        "a 403 missing-scope must still offer the reconnect — the flag is correct here"
    );
}

/// A transient failure (rate limit, outage) must not touch the flag at all.
#[tokio::test]
async fn a_rate_limit_does_not_touch_the_flag() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    switchpost::feed::handle_feed_failure_for_test(
        &pool,
        id,
        "linkedin",
        "API error: 429 Too Many Requests",
    )
    .await
    .unwrap();
    assert!(
        !is_flagged(&pool, id).await,
        "a 429 is not a reconnect prompt — the channel is fine"
    );
}

/// Precedence: a message carrying BOTH a scope word and a 404. The
/// pre-fix code checked the 404 string FIRST, so any 404 whose body happened
/// to mention a scope was still misclassified. Auth wins, because a token
/// problem plus a missing resource is still a token problem the operator
/// can act on.
#[tokio::test]
async fn an_auth_word_beats_a_404_in_the_same_message() {
    let kind = switchpost::feed::classify_feed_failure(
        "API error: 404 Not Found — the token may lack permission for this organization",
    );
    assert_eq!(
        kind,
        switchpost::feed::FeedFailureKind::NeedsReconnect,
        "an auth word in the same message is the actionable failure"
    );
}

// ── The real consequence: the channel keeps publishing ─────────

/// The flag's whole point is the exclusion. Assert against the REAL
/// query the scheduler uses, so the test fails if the flag is ever
/// re-pointed at a different filter.
#[tokio::test]
async fn a_404_channel_still_reaches_the_scheduler() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    // Inside the scheduler's 24h window: this is what a token about to be
    // proactively refreshed looks like. A 59-day token would not appear at
    // all, which would make BOTH assertions below vacuous.
    let id = seed_linkedin_with_urn(&pool, "urn:li:person:abc", false, 6).await;

    // The flag must start clear, or the assertion is vacuous.
    assert!(!is_flagged(&pool, id).await);

    // The production window the scheduler uses (src/scheduler/mod.rs: 24h).
    let window = switchpost::scheduler::PROACTIVE_REFRESH_WINDOW_HOURS;
    let reachable =
        switchpost::db::queries::get_integrations_needing_refresh(&pool, "linkedin", window)
            .await
            .unwrap();
    assert!(
        reachable.iter().any(|i| i.id == id),
        "a linkedin channel with a token inside the 24h window must be reachable by the proactive refresh"
    );

    // Now 404 the channel, and check it is STILL reachable.
    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", LIVE_404)
        .await
        .unwrap();
    let still_reachable =
        switchpost::db::queries::get_integrations_needing_refresh(&pool, "linkedin", window)
            .await
            .unwrap();
    assert!(
        still_reachable.iter().any(|i| i.id == id),
        "the 404 must not remove the channel from the scheduler's refresh window"
    );
}

// ── Defect B: the flag is clearable ───────────────────────────

/// `mark_integration_refresh_needed` had no counterpart: a one-way door.
/// A channel that gets flagged can be recovered by a reconnect (which goes
/// through `update_integration_token`, clearing the flag at queries.rs:439)
/// but NOT by a proven-healthy token, because nothing mirrored the write.
#[tokio::test]
async fn a_flags_channel_can_be_cleared_by_refresh_success() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin_with_urn(&pool, "urn:li:person:abc", true, 6).await;
    assert!(is_flagged(&pool, id).await, "fixture starts flagged");

    switchpost::db::queries::clear_integration_refresh_needed(&pool, id)
        .await
        .unwrap();

    assert!(
        !is_flagged(&pool, id).await,
        "a successful refresh is proof of health and must clear the flag"
    );
    // And it comes back into the window.
    let window = switchpost::scheduler::PROACTIVE_REFRESH_WINDOW_HOURS;
    let reachable =
        switchpost::db::queries::get_integrations_needing_refresh(&pool, "linkedin", window)
            .await
            .unwrap();
    assert!(
        reachable.iter().any(|i| i.id == id),
        "clearing the flag must restore the channel to the refresh window"
    );
}

/// Clearing is idempotent — the caller in `try_refresh_and_fetch` only checks
/// the flag but the write must be safe on an already-clear row.
#[tokio::test]
async fn clearing_an_unflagged_channel_is_a_no_op_not_an_error() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, false).await;

    switchpost::db::queries::clear_integration_refresh_needed(&pool, id)
        .await
        .expect("clearing an already-clear flag must not error");
    assert!(!is_flagged(&pool, id).await);
}

/// The channel the fix changed must be a DIFFERENT row: clearing one
/// channel's flag must not clear another's.
#[tokio::test]
async fn clearing_is_scoped_to_one_channel() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let a = seed_linkedin_with_urn(&pool, "urn:li:person:aaa", true, 59).await;
    let b = seed_linkedin_with_urn(&pool, "urn:li:organization:bbb", true, 59).await;

    switchpost::db::queries::clear_integration_refresh_needed(&pool, a)
        .await
        .unwrap();

    assert!(!is_flagged(&pool, a).await);
    assert!(
        is_flagged(&pool, b).await,
        "clearing channel A must not clear channel B"
    );
}

/// Every refresh path writes through `update_integration_token`, which
/// already clears the flag (queries.rs:439). This asserts a reconnect
/// therefore recovers a flagged channel, so the recovery strategy a founder
/// is offered is the one that works.
#[tokio::test]
async fn a_reconnect_recovers_a_flagged_channel() {
    let pool = switchpost::db::create_pool("sqlite::memory:").await.unwrap();
    let id = seed_linkedin(&pool, true).await;

    // reconnect routes through update_integration_token (AGENTS.md §6.3.1).
    switchpost::db::queries::update_integration_token(
        &pool,
        id,
        "fresh-access",
        Some("fresh-refresh"),
        Some(chrono::Utc::now() + chrono::Duration::days(59)),
        None,
    )
    .await
    .unwrap();

    assert!(
        !is_flagged(&pool, id).await,
        "a reconnect must recover a flagged channel"
    );
}
