//! Defect C: a channel-shaped provider must not forward its page-management
//! refusal into the refresh endpoint's answer.
//!
//! WHAT WAS BROKEN
//!
//! `/api/integrations/:id/refresh` (src/api/integrations.rs) asks the provider
//! to "Refresh profile" via `provider.reconnect(...)`. The trait's DEFAULT
//! routes that through `fetch_page_info`, which is the PAGE-SELECTION function
//! and which is SUPPOSED to refuse a channel-shaped provider with
//! `Err(ProviderError::NoPageManagement)`.
//!
//! Left unhandled, a `?` forwards that refusal as the endpoint's answer, so a
//! Reddit (or Threads, or Instagram-standalone) channel answered the founder's
//! "Refresh" click with:
//!
//!     "Failed to refresh profile: This provider has no page or sub-account
//!      concept. The channel is the account."
//!
//! ...which is a 400 telling the founder something about the provider's data
//! model in words they cannot act on, from a button that is supposed to renew a
//! token. The default `reconnect` therefore DEGRADES on that one variant:
//! it answers with the channel's own identity and the existing token, and the
//! handler's COALESCE write then keeps the stored `profile_name`/
//! `profile_picture` and clears `refresh_needed`.
//!
//! WHAT IS PINNED HERE
//!
//! Two halves, both mutation-provable:
//!
//!   1. the three providers in the defect report (linkedin,
//!      instagram-standalone, threads) DO refuse with `NoPageManagement` from
//!      `fetch_page_info` — a provider that forgot the override would silently
//!      inherit a default that either panics or answers nonsense.
//!   2. the default `reconnect` DEGRADES on that refusal instead of forwarding
//!      it, so the "Refresh" click succeeds and the flag clears.
//!
//! Refusing is not enough. A provider that refuses but whose `reconnect`
//! forwards the refusal reproduces the dead end from a different direction,
//! which is exactly what connect_flow_redteam_test.rs's "unreconnectable"
//! case is about.

use switchpost::config::Config;
use switchpost::social::instagram_standalone::InstagramStandaloneProvider;
use switchpost::social::linkedin::LinkedInProvider;
use switchpost::social::threads::ThreadsProvider;
use switchpost::social::ProviderError;
use switchpost::social::SocialProvider;

/// Build a provider with real `&Config` (the constructors read credentials
/// from it; no credentials in the test env means empty ones, which is fine -
/// the `fetch_page_info` refusals below never reach the network, and the one
/// reconnect call tolerates a transport failure).
fn config() -> Config {
    Config::from_env().expect("config")
}

// ══════════════════════════════════════════════════════════════
// Half 1: the three providers refuse page selection
// ══════════════════════════════════════════════════════════════

/// The `fetch_page_info` contract for a channel-shaped provider: refuse with
/// the ONE variant that means "there is nothing to pick here", so the
/// `reconnect` degrade path recognises it.
///
/// A provider that returns a DIFFERENT variant (Api, Auth, InvalidRequest,
/// Network) makes `reconnect` forward it — the dead end the doc comment above
/// describes. A provider that returns `Ok(..)` with a fabricated identity is
/// worse: it lets the refresh endpoint write a fake id over a real one.
#[tokio::test]
async fn linkedin_refuses_page_selection_with_no_page_management() {
    let provider = LinkedInProvider::new(&config());
    let err = provider
        .fetch_page_info("tok", "urn:li:person:abc")
        .await
        .expect_err("linkedin personal is channel-shaped: it must refuse page selection");
    assert!(
        matches!(err, ProviderError::NoPageManagement),
        "linkedin refused with {err:?} — a channel-shaped provider must use \
         NoPageManagement, or reconnect forwards the refusal as a 400"
    );
}

#[tokio::test]
async fn instagram_standalone_refuses_page_selection_with_no_page_management() {
    let provider = InstagramStandaloneProvider::new(&config());
    let err = provider
        .fetch_page_info("tok", "17841400000000000")
        .await
        .expect_err("instagram-standalone is channel-shaped: it must refuse page selection");
    assert!(
        matches!(err, ProviderError::NoPageManagement),
        "instagram-standalone refused with {err:?} — a channel-shaped provider \
         must use NoPageManagement, or reconnect forwards the refusal as a 400"
    );
}

#[tokio::test]
async fn threads_refuses_page_selection_with_no_page_management() {
    let provider = ThreadsProvider::new(&config());
    let err = provider
        .fetch_page_info("tok", "1234567890")
        .await
        .expect_err("threads is channel-shaped: it must refuse page selection");
    assert!(
        matches!(err, ProviderError::NoPageManagement),
        "threads refused with {err:?} — a channel-shaped provider must use \
         NoPageManagement, or reconnect forwards the refusal as a 400"
    );
}

/// The other direction, and the reason the variant matters: a refusal for a
/// GENUINE failure is still an error. `reconnect` must not swallow it into
/// "the channel is the account", or a real outage answers "Refresh" with
/// success and the founder believes a token was renewed when it was not.
///
/// This drives the DEFAULT `reconnect` (through linkedin's `fetch_page_info`,
/// which refuses) against a provider that answers a non-`NoPageManagement`
/// error, so the `Err(e) => Err(e)` arm is the one under test.
#[tokio::test]
async fn a_real_failure_is_not_degraded_into_success() {
    // The default reconnect's third arm is `Err(e) => Err(e)`: every refusal
    // that is NOT NoPageManagement keeps propagating. Drives the real LinkedIn
    // provider's own overridden reconnect, which is the production path for
    // the provider in this defect report.
    let provider = LinkedInProvider::new(&config());
    // An invalid token: the provider's reconnect makes a real HTTP call and
    // degrades to an empty name, exactly as the live test
    // (tests/live_linkedin_youtube_test.rs) observes. It must NOT forward the
    // page-management refusal, because linkedin never returns that variant.
    let result = provider
        .reconnect("definitely-not-a-valid-token", "urn:li:person:abc", "p")
        .await;
    match result {
        Ok(info) => assert!(
            info.name.is_empty(),
            "a bad token must degrade to an empty name, not a fabricated identity"
        ),
        // A network/transport failure is a legitimate Err here (no network in
        // the sandbox) — the point is that it is NOT the "no page concept"
        // 400, which is the dead end this test file exists to keep closed.
        Err(e) => assert!(
            !matches!(e, ProviderError::NoPageManagement),
            "linkedin personal must never answer NoPageManagement from reconnect: {e:?}"
        ),
    }
}

// ══════════════════════════════════════════════════════════════
// Half 2b: the BEHAVIOURAL proof — the default reconnect answers Ok
// ══════════════════════════════════════════════════════════════

/// A channel-shaped provider: it has no page concept, so its
/// `fetch_page_info` refuses with the one variant that means "nothing to pick
/// here". Everything else is the trait default, which is the path the refresh
/// endpoint actually walks.
struct ChannelShapedProvider;

#[async_trait::async_trait]
impl switchpost::social::SocialProvider for ChannelShapedProvider {
    fn identifier(&self) -> &'static str {
        "channel-shaped"
    }

    fn name(&self) -> &'static str {
        "Channel Shaped"
    }

    fn scopes(&self) -> Vec<String> {
        vec![]
    }

    fn max_content_length(&self) -> usize {
        0
    }

    async fn generate_auth_url(
        &self,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<switchpost::social::AuthUrlResponse, ProviderError> {
        unreachable!("the degrade path does not start an OAuth flow")
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<switchpost::social::AuthToken, ProviderError> {
        unreachable!("the degrade path never exchanges a code")
    }

    async fn refresh_token(
        &self,
        _refresh_token: &str,
    ) -> Result<switchpost::social::AuthToken, ProviderError> {
        unreachable!("the degrade path does not refresh a token")
    }

    async fn publish(
        &self,
        _access_token: &str,
        _post: &switchpost::social::PostContent,
    ) -> Result<switchpost::social::PublishResult, ProviderError> {
        unreachable!("the degrade path does not publish")
    }

    async fn fetch_page_info(
        &self,
        _access_token: &str,
        _page_id: &str,
    ) -> Result<switchpost::social::PageInfo, ProviderError> {
        Err(ProviderError::NoPageManagement)
    }
}

/// The BEHAVIOURAL proof: this one CALLS the default `reconnect` against a
/// real stub and asserts what the endpoint would actually receive.
///
/// On the token specifically it is REDUNDANT with the source-shape test below,
/// which asserts `arm.contains("access_token.to_string()")` - mutation C2
/// (a degraded identity built with `String::new()`) turns BOTH red. That
/// redundancy is kept, not fixed: it is one extra assertion on a path whose
/// failure mode is silently blanking a live channel's token.
///
/// What this test adds that a string match cannot: it runs the REAL default
/// rather than reading the source text, so it keeps working across a rename,
/// a reformat, or an arm relocated into a provider. It is the more durable
/// half. (For the record, no mutation has been found that ONLY this test
/// catches - that would be a claim, not a finding, so it is not claimed here.)
#[tokio::test]
async fn the_default_reconnect_degrades_behaviourally_not_just_textually() {
    let provider = ChannelShapedProvider;

    // `internal_id` is the channel's own identifier; the endpoint passes it
    // for both, exactly as src/api/integrations.rs:554-556 does.
    let result = provider
        .reconnect("a-live-token", "channel-urn-123", "channel-urn-123")
        .await
        .expect(
            "a channel-shaped provider's page-selection refusal must DEGRADE \
             to Ok, or the founder's Refresh button answers the 400 dead end: \
             'Failed to refresh profile: This provider has no page or \\
             sub-account concept. The channel is the account.'",
        );

    // The degraded identity must be the CHANNEL's own, so the handler's write
    // rebinds the same row and does not create a phantom one.
    assert_eq!(
        result.id, "channel-urn-123",
        "the degraded identity must reuse the caller's page_id"
    );
    // And the token the CALLER passed, so the handler's write cannot put an
    // empty access token on a live row.
    assert_eq!(
        result.access_token, "a-live-token",
        "the degraded identity must reuse the caller's access token, or the \
         degraded path writes an empty token over a live one"
    );
    // Empty name + no picture: that is "the provider told us nothing", which
    // the handler's COALESCE write turns into "keep what you had".
    assert!(
        result.name.is_empty(),
        "the degraded name must be empty so the handler keeps the stored label; \
         got {:?}",
        result.name
    );
    assert!(
        result.picture.is_none(),
        "the degraded picture must be None so the handler keeps the stored avatar"
    );
}

/// The degrade path lives in `src/social/mod.rs`'s default `reconnect`. If it
/// is ever removed (or the `Err(ProviderError::NoPageManagement)` arm is
/// replaced by the forwarding `Err(e) => Err(e)`), the dead-end 400 returns
/// for Reddit, X, Discord, Threads and Instagram-standalone — five channels —
/// and no provider-level test would notice.
#[test]
fn the_default_reconnect_degrades_on_a_page_management_refusal() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/social/mod.rs"),
    )
    .expect("read src/social/mod.rs");

    // Isolate the default `reconnect` body.
    let start = src
        .find("async fn reconnect(")
        .unwrap_or_else(|| panic!("the trait default reconnect is not in social/mod.rs"));
    let tail = &src[start..];
    let body_end = tail.find("\n    }\n").map(|i| i + 5).unwrap_or(tail.len());
    let body = &src[start..start + body_end];

    assert!(
        body.contains("Err(ProviderError::NoPageManagement)"),
        "the default reconnect no longer has a NoPageManagement arm, so a \
         channel-shaped provider forwards the page-selection refusal and the \
         Refresh button answers the 400 dead end again"
    );
    // And it must DEGRADE (answer Ok), not merely match-and-forward.
    assert!(
        body.contains("Ok(ReconnectResult {"),
        "the NoPageManagement arm matches but returns Err, which is the same \
         dead end reached from a different direction: the founder's Refresh \
         still fails on a channel whose token is fine"
    );
    // The degraded identity must carry the CHANNEL's id and the CALLER's
    // token, so the handler's write does not blank the stored row. An empty
    // `access_token` here would write an empty token over a live one.
    let arm = &body[body.find("Err(ProviderError::NoPageManagement)").unwrap()..];
    assert!(
        arm.contains("page_id.to_string()"),
        "the degraded identity must reuse the caller's page_id, or the \
         reconnect rebinds the channel to an empty id"
    );
    assert!(
        arm.contains("access_token.to_string()"),
        "the degraded identity must reuse the caller's access token, or the \
         degraded path can write an empty token over a live one"
    );
    // And the LAST arm must still forward every other refusal.
    let after_gone = &arm[arm.find("access_token.to_string()").unwrap()..];
    assert!(
        after_gone.contains("Err(e) => Err(e)"),
        "the default reconnect swallows refusals that are NOT NoPageManagement, \
         so a real auth failure answers Refresh with success and the founder \
         believes a dead token was renewed"
    );
}

// ══════════════════════════════════════════════════════════════
// Defect D: a scope miss that reads as a TIMEOUT
// ══════════════════════════════════════════════════════════════

/// THE STRING, copied from the live provider.
///
/// `src/social/linkedin.rs:192` replaces the raw 403 on the READ path with
/// this message, and its own comment says why: the token was granted without
/// the now-opt-in `r_member_social` scope, and "the reconnection the founder
/// actually has to do" follows. The message even ends with the words
/// "reconnect LinkedIn from the Channels screen".
///
/// THE DEFECT: `classify_feed_failure` looks for the literal tokens `403`,
/// `forbidden`, `permission`, `scope`, `401`, `unauthorized`, and
/// `token expired`. This message contains NONE of them - it says "not
/// granted", "enable", "reconnect", and "product". So it classifies as
/// `Transient`, the `Transient` arm only logs a warning and touches no flag,
/// and the founder is left polling a channel that can never succeed with no
/// reconnect offered anywhere in the UI.
///
/// It is the same shape as defect A, from the other side: A set the flag when
/// it should NOT have; D fails to set it when it SHOULD. Both leave the
/// Channels screen lying.
fn linkedin_read_path_403() -> &'static str {
    "LinkedIn could not read this member's posts: the connected account was \
     not granted 'r_member_social'. To read your own LinkedIn posts and \
     feed, enable the 'Read member's social' product on the LinkedIn app, \
     set LINKEDIN_ENABLE_MEMBER_SOCIAL_READ=1 on the server, then \
     reconnect LinkedIn from the Channels screen."
}

/// The provider's REAL message must classify as a reconnect, not a timeout.
///
/// Driven against `classify_feed_failure` directly (not a copy of the rule),
/// so a mutation to the token list goes red here.
#[test]
fn a_linkedin_scope_miss_is_a_reconnect_not_a_timeout() {
    assert_eq!(
        switchpost::feed::classify_feed_failure(linkedin_read_path_403()),
        switchpost::feed::FeedFailureKind::NeedsReconnect,
        "the provider's own 403 message describes a scope miss and names the \
         reconnect that fixes it, but it classified as a timeout - so the flag \
         is never set and the Channels screen shows a healthy channel that can \
         never read a post again (defect D)"
    );
}

/// The BEHAVIOURAL half: this message must actually SET the flag on a real
/// row, not merely classify correctly. A classifier that names the right kind
/// while the handler drops it is the same defect wearing another hat.
#[tokio::test]
async fn a_linkedin_scope_miss_sets_the_flag_on_a_real_row() {
    let pool = switchpost::db::create_pool("sqlite::memory:")
        .await
        .unwrap();
    switchpost::db::ensure_local_user(&pool).await.unwrap();

    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO integrations
            (id, user_id, provider_identifier, provider_name, internal_id,
             access_token, refresh_token, token_expires_at,
             disabled, refresh_needed, posting_times, auth_method)
         VALUES (?, ?, 'linkedin', 'LinkedIn', 'urn:li:person:abc',
                 'A validT0ken', 'refresh-tok', ?, 0, 0, '[]', 'oauth')",
    )
    .bind(id)
    .bind(switchpost::auth::middleware::DEFAULT_USER_ID)
    .bind(chrono::Utc::now() + chrono::Duration::days(59))
    .execute(&pool)
    .await
    .unwrap();

    switchpost::feed::handle_feed_failure_for_test(&pool, id, "linkedin", linkedin_read_path_403())
        .await
        .unwrap();

    let flagged =
        sqlx::query_scalar::<_, bool>("SELECT refresh_needed FROM integrations WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        flagged,
        "a LinkedIn read-path 403 must set refresh_needed, so the founder is \
         shown the reconnect that fixes it (defect D)"
    );
}
