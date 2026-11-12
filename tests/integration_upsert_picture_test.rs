// ─── Red-team: a re-connect must not DESTROY the channel's avatar ─────
//
// The operator's x, reddit and github rows all have `profile_picture`
// NULL. This session fixed the REpair path (refresh writes
// `COALESCE(?, profile_picture)`, which is why the avatars stayed gone
// on providers hardcoding `picture: None`).
//
// But the repair path was never the only write. `create_integration` is
// the CONNECT path, and it upserts on
// `(user_id, provider_identifier, internal_id)` — so reconnecting an
// existing channel runs its ON CONFLICT arm. That arm reads:
//
//     ON CONFLICT ... DO UPDATE SET
//         access_token = ?, refresh_token = ?, token_expires_at = ?,
//         profile_name = ?, profile_picture = ?, profile_url = ?,
//         root_internal_id = COALESCE(?, integrations.root_internal_id),
//         auth_method = ?, refresh_needed = false, ...
//
// `root_internal_id`, in the SAME statement, is protected by COALESCE —
// someone learned that lesson there and then did not apply it one
// binding earlier. So the connect path behaves OPPOSITELY to the
// refresh path: refresh keeps a value the provider does not supply,
// and connect WIPES a value the provider does not supply.
//
// Which means the fix to the repair path is not sufficient on its own.
// A Google-family provider that returned no `picture` on one exchange —
// a scoped-down token, a fresh consent screen the user dismissed, an
// exchange whose userinfo call failed after the OAuth code succeeded —
// silently loses the avatar it had already captured, on a reconnect
// that had nothing to do with avatars.
//
// This test pins the invariant at the DB layer: reconnecting a channel
// with no avatar must leave the avatar that is already there.
//
// Run: cargo test --test integration_upsert_picture_test

use switchpost::api::AppState;
use switchpost::config::Config;
use switchpost::db;
use switchpost::db::queries;
use switchpost::mcp::auth::resolve_first_user;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;

async fn state(name: &str) -> AppState {
    switchpost::config::load_dotenv_for_tests();
    let mut config = Config::from_env().expect("config");
    let base = config.database_url.trim_end_matches(".db").to_string();
    config.database_url = format!("{base}-{name}.db");
    let pool = db::create_pool(&config.database_url).await.expect("pool");
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

const PICTURE: &str = "https://lh3.googleusercontent.com/a/real-avatar";

/// The re-connect of a channel the app already has. This is the exact
/// shape of the page picker's second connect, an OAuth re-authorize,
/// and the PAT re-paste on the Channels page.
#[tokio::test]
async fn reconnecting_with_no_avatar_keeps_the_stored_one() {
    let state = state("upsert_picture_keeps").await;
    db::ensure_local_user(&state.db).await.expect("local user");
    let user_id = resolve_first_user(&state).await.expect("user");

    queries::create_integration(
        &state.db,
        user_id,
        "gmail",
        "Gmail",
        "gmail-user-1",
        "access-token-1",
        None,
        None,
        Some("The Founder"),
        Some(PICTURE),
        None,
        None,
        Some("oauth"),
        None,
    )
    .await
    .expect("first connect, WITH a picture");

    // Re-authorise the same channel. The provider's exchange comes back
    // without a picture this time — a scoped token, dismissed consent, or
    // a userinfo call that failed after the code was already exchanged.
    queries::create_integration(
        &state.db,
        user_id,
        "gmail",
        "Gmail",
        "gmail-user-1",
        "access-token-2",
        None,
        None,
        Some("The Founder"),
        None,
        None,
        None,
        Some("oauth"),
        None,
    )
    .await
    .expect("second connect, WITHOUT a picture");

    let stored = queries::list_integrations(&state.db, user_id)
        .await
        .expect("integrations")
        .into_iter()
        .find(|i| i.internal_id == "gmail-user-1")
        .expect("the channel exists");

    assert_eq!(
        stored.profile_picture.as_deref(),
        Some(PICTURE),
        "the reconnect wiped the avatar that was already stored"
    );

    // The token DID update, so the conflict arm ran — this is not the
    // INSERT arm incidentally skipping anything.
    assert_eq!(stored.access_token, "access-token-2");
}

/// The other half of the invariant: a NEW avatar on a reconnect must
/// still land. COALESCE must protect an absent value, not defend a
/// stale one against every update.
#[tokio::test]
async fn reconnecting_with_a_new_avatar_updates_it() {
    let state = state("upsert_picture_updates").await;
    db::ensure_local_user(&state.db).await.expect("local user");
    let user_id = resolve_first_user(&state).await.expect("user");

    queries::create_integration(
        &state.db,
        user_id,
        "gmail",
        "Gmail",
        "gmail-user-2",
        "access-token-1",
        None,
        None,
        Some("The Founder"),
        Some("https://example.com/old-avatar"),
        None,
        None,
        Some("oauth"),
        None,
    )
    .await
    .expect("first connect");

    queries::create_integration(
        &state.db,
        user_id,
        "gmail",
        "Gmail",
        "gmail-user-2",
        "access-token-2",
        None,
        None,
        Some("The Founder"),
        Some(PICTURE),
        None,
        None,
        Some("oauth"),
        None,
    )
    .await
    .expect("second connect");

    let stored = queries::list_integrations(&state.db, user_id)
        .await
        .expect("integrations")
        .into_iter()
        .find(|i| i.internal_id == "gmail-user-2")
        .expect("the channel exists");

    assert_eq!(
        stored.profile_picture.as_deref(),
        Some(PICTURE),
        "a genuinely new avatar was discarded"
    );
}

/// The x-cookie connect path's promise, pinned at the layer that can be
/// driven: the connector reads `profile_image_url_https` out of X's
/// GraphQL `UserByScreenName` response and rewrites `_normal` to the
/// larger size. That value must (a) survive the write and (b) come back
/// out of the read path the Channels page renders, or the "missing
/// avatar" report reproduces at the seam that is actually reachable.
///
/// A full end-to-end here needs real X cookies - `connect_x_cookie`
/// POSTs to x.com with the caller's own session and has no injection
/// point - so what is pinned is every layer after the network call.
#[tokio::test]
async fn the_x_cookie_picture_survives_the_round_trip_the_ui_reads() {
    let state = state("upsert_picture_x_round_trip").await;
    db::ensure_local_user(&state.db).await.expect("local user");
    let user_id = resolve_first_user(&state).await.expect("user");

    // The exact shape `connect_x_cookie` builds: the `_normal` suffix
    // replaced for the larger file, and a cookie-blob access token.
    let x_picture = "https://pbs.twimg.com/profile_images/1234/abc_400x400.jpg";
    queries::create_integration(
        &state.db,
        user_id,
        "x",
        "X (Twitter)",
        "1234567890",
        r#"{"auth_token":"at","ct0":"ct0","cookie_string":"auth_token=at; ct0=ct0;"}"#,
        None,
        None,
        Some("The Founder"),
        Some(x_picture),
        Some("@thefounder"),
        None,
        Some("cookie"),
        None,
    )
    .await
    .expect("x-cookie connect");

    let stored = queries::list_integrations(&state.db, user_id)
        .await
        .expect("integrations")
        .into_iter()
        .find(|i| i.internal_id == "1234567890")
        .expect("the channel exists");

    assert_eq!(
        stored.profile_picture.as_deref(),
        Some(x_picture),
        "the picture the connector builds does not survive the write"
    );

    // The serialization the Channels page consumes.
    let public: switchpost::db::models::IntegrationPublic = stored.into();
    assert_eq!(
        public.profile_picture.as_deref(),
        Some(x_picture),
        "the picture does not survive the serialization the UI reads"
    );
}

// ══════════════════════════════════════════════════════════════
// The SAME bug class, in the refresh handler's own write
// ══════════════════════════════════════════════════════════════

/// The refresh endpoint writes the profile fields too, and its picture was
/// guarded while its name was not.
///
/// `reconnect`'s default DEGRADES on a channel-shaped provider (Reddit, X,
/// Discord, Instagram-standalone, ...) by returning the channel's own identity
/// with an EMPTY name and no picture: that is what makes a channel refreshable
/// without a per-provider override. Written bare, that blank is destructive:
///
///     UPDATE integrations SET profile_name = $1,
///            profile_picture = COALESCE($2, profile_picture), ...
///
/// The picture survives a provider that supplies nothing. The name does not.
///
/// Proven live before the fix, on a running server: a row seeded with
/// `profile_name = 'u/cooluser r/NICE SUB'` and a real avatar came back from
/// `POST /api/integrations/:id/refresh` with the avatar intact and
/// `profile_name = ''`. The Channels page then rendered an unlabelled channel,
/// with no error, no toast, and no way to recover the label by hand.
///
/// So this is the ASYMMETRY this whole file exists for, one statement later:
/// the COALESCE lesson was learned for one column in a write and not applied
/// to its neighbour in the same write. The two must be guarded together, or
/// the write blanks one of them.
#[test]
fn the_refresh_write_guards_profile_name_as_well_as_the_picture() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/api/integrations.rs"),
    )
    .expect("read src/api/integrations.rs");

    // The refresh handler's write, isolated: from the sqlx::query that names
    // profile_name to the end of its execute. Not the whole file, because
    // `connect_page` and the connect handlers legitimately write a bare
    // profile_name - they are writing a value the provider DID supply.
    let start = src
        .find("UPDATE integrations SET profile_name")
        .unwrap_or_else(|| panic!("the refresh handler's write is not in integrations.rs"));
    let tail = &src[start..];
    let end = start
        + tail
            .find(".await?;")
            .expect("the refresh write has no .await? terminator")
        + ".await?;".len();
    let write = &src[start..end];

    assert!(
        write.contains("profile_name = COALESCE("),
        "the refresh write assigns profile_name directly, so a channel-shaped \
         provider (whose reconnect DEGRADES to an empty name) loses the label \
         the Channels page renders. It just lost its avatar fix to a blank \
         name; the write was: {write}"
    );
    assert!(
        write.contains("profile_picture = COALESCE("),
        "the refresh write no longer guards the picture, which is the original \
         avatar bug this file pins. Write was: {write}"
    );

    // And the name must be bound as an OPTION, not as the raw String. A
    // `Some(info.name)` makes the COALESCE above inert: an empty name then
    // arrives as Some("") rather than NULL, and COALESCE preserves it.
    //
    // Mutation-proven: adding `COALESCE($1, profile_name)` to the SQL while
    // binding `info.name.clone()` still passes a SQL-only check. This is the
    // whole reason the assertion is on the BIND and not just the statement.
    assert!(
        write.contains(".bind(info_name)"),
        "the refresh write does not bind the name as an Option, so the COALESCE \
         above it is inert: an empty name arrives as Some(\"\") and COALESCE \
         keeps it. The blank must become NULL at the bind. Write was: {write}"
    );
}
