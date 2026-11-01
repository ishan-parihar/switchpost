// ─── Feed Refresher ─────────────────────────────────────────────

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::crypto;
use crate::realtime::Broadcaster;
use crate::social::registry::ProviderRegistry;
use crate::social::{ProviderError, SocialProvider};
use sqlx::SqlitePool;

/// Default interval between full refresh cycles (seconds)
const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Default interval between engagement poll cycles (seconds)
const DEFAULT_ENGAGEMENT_INTERVAL_SECS: u64 = 1800; // 30 minutes

/// How many recent posts to fetch per integration per cycle
const RECENT_POSTS_LIMIT: u32 = 200;

/// How many recent posts to fetch comments for, per integration, per
/// comment-refresh cycle. Capped to keep provider API usage reasonable —
/// the comments cache only needs to cover posts the user is likely to
/// be reading comments on, not the entire history.
const COMMENTS_REFRESH_POST_LIMIT: i64 = 20;

/// Start the feed refresher background task.
/// Polls all integrations for new posts and periodically fetches engagement data.
pub fn start_feed_refresher(
    db: SqlitePool,
    providers: Arc<ProviderRegistry>,
    broadcaster: Broadcaster,
    token_key: Option<[u8; 32]>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let db1 = db.clone();
    let mut shutdown1 = shutdown.clone();
    tokio::spawn(async move {
        let interval_secs = std::env::var("FEED_REFRESH_INTERVAL_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_REFRESH_INTERVAL_SECS);
        tracing::info!(
            "Feed refresher started (interval: {interval_secs}s, engagement interval: {DEFAULT_ENGAGEMENT_INTERVAL_SECS}s)"
        );
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut engagement_interval =
            tokio::time::interval(Duration::from_secs(DEFAULT_ENGAGEMENT_INTERVAL_SECS));
        engagement_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = shutdown1.changed() => {
                    if *shutdown1.borrow() {
                        tracing::info!("Feed refresher shutting down...");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(e) = refresh_all_posts(&db1, &providers, &broadcaster, token_key).await {
                        tracing::error!("Feed refresh error: {e}");
                    }
                }
                _ = engagement_interval.tick() => {
                    if let Err(e) = refresh_all_engagement(&db1, &providers, &broadcaster, token_key).await {
                        tracing::error!("Feed engagement refresh error: {e}");
                    }
                    // Refresh the comments cache on the same 30-min cycle.
                    // Comments change less frequently than posts, so this
                    // cadence is sufficient and keeps provider API usage low.
                    if let Err(e) = refresh_all_comments(&db1, &providers, &broadcaster, token_key).await {
                        tracing::error!("Comments refresh error: {e}");
                    }
                }
            }
        }
    });
    // Shutdown subscription stays alive through `shutdown`
}

/// Attempt to auto-refresh an expired token and re-fetch posts.
/// Returns Some(posts) on success, None if refresh fails.
///
/// THE DEFECT-B SEAM. `try_refresh_and_fetch` was the load-bearing half of the
/// 5c09a5f fix and had NO test behind it: the only reference to it in tests/
/// was a doc comment. Its regression is invisible to read, because the failure
/// is behavioural - a written-then-discarded token - so the only way to pin it
/// is to drive the function with a provider that can tell the two tokens apart
/// and see which one the fetch was actually given.
async fn try_refresh_and_fetch(
    db: &SqlitePool,
    provider: &Arc<dyn SocialProvider>,
    integration: &crate::db::models::Integration,
    token_key: Option<[u8; 32]>,
) -> Option<Vec<crate::social::ExternalPostData>> {
    try_refresh_and_fetch_for_test(db, provider, integration, token_key).await
}

/// The test-visible wrapper. Same body, same order of operations - the wrapper
/// exists so a test can drive the real path rather than a copy of it
/// (AGENTS.md 9.18), and it is the integration seam for defect B.
pub async fn try_refresh_and_fetch_for_test(
    db: &SqlitePool,
    provider: &Arc<dyn SocialProvider>,
    integration: &crate::db::models::Integration,
    token_key: Option<[u8; 32]>,
) -> Option<Vec<crate::social::ExternalPostData>> {
    tracing::info!(
        "Token expired for integration {} ({}), attempting auto-refresh...",
        integration.id,
        integration.provider_identifier
    );
    let refresh_token = integration.refresh_token.as_ref()?;
    let rt = crypto::maybe_decrypt_token(refresh_token, token_key.as_ref());
    let new_token = match provider.refresh_token(&rt).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                "Token refresh failed for {} ({}): {e}",
                integration.id,
                integration.provider_identifier
            );
            return None;
        }
    };
    let new_access = new_token.access_token.clone();
    let expires_at = new_token
        .expires_in
        .map(|e| chrono::Utc::now() + chrono::Duration::seconds(e as i64));
    // B8: single encrypting writer — pass plaintext + key; it never
    // downgrades to plaintext on failure.
    if let Err(e) = crate::db::queries::update_integration_token(
        db,
        integration.id,
        &new_access,
        new_token.refresh_token.as_deref(),
        expires_at,
        token_key,
    )
    .await
    {
        tracing::warn!(
            "Failed to update refreshed token for {}: {e}",
            integration.id
        );
        return None;
    }
    tracing::info!(
        "Token refreshed successfully for {} ({})",
        integration.id,
        integration.provider_identifier
    );
    // Defect B: the refreshed token was written, then DROPPED on the floor -
    // the fetch below used `integration.access_token` (the STALE, expired
    // token) instead of `new_access`. So every proactive refresh logged
    //     Auth error: Missing access_token
    // ...and `None`, which the caller read as "refresh failed" and used as a
    // reason to skip the channel for another cycle. The token WAS good; the
    // bug was that nobody used it.
    //
    // It also means the channel that WAS flagged `refresh_needed` never got
    // cleared, because clearing happens only on a fetch that actually
    // succeeds. Both halves are fixed here: use the new token, then clear the
    // flag when the fetch works.
    match provider
        .get_recent_posts(&new_access, &integration.internal_id, RECENT_POSTS_LIMIT)
        .await
    {
        Ok(p) => {
            if integration.refresh_needed {
                crate::db::queries::clear_integration_refresh_needed(db, integration.id)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            "Failed to clear refresh_needed for {}: {e}",
                            integration.id
                        );
                    });
            }
            Some(p)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch posts after refresh for {} ({}): {e}",
                integration.id,
                integration.provider_identifier
            );
            None
        }
    }
}

/// One rule, one place: what a failed post fetch MEANS for the channel.
///
/// WHY THIS EXISTS (measured, not assumed)
///
/// Every LinkedIn channel on the live deployment showed "needs reconnect"
/// while its token was VALID - the personal `linkedin` row had a
/// `token_expires_at` months out and the `linkedin-page` rows were being
/// rewritten nightly, so the refresh was succeeding and writing a good token
/// every time. The trigger was a log line, not a schedule:
///
///     WARN feed: LinkedIn get_posts failed ... 404: No virtual resource found.
///
/// "No virtual resource found" is LinkedIn's phrasing for "the page/resource
/// you named is not there". It is NOT a scope error, and it is definitely NOT
/// a token error - the same token had just been used successfully.
///
/// But the feed read that string, concluded "lacks API scope", and called
/// `mark_integration_refresh_needed`. That flag is what
/// `get_integrations_needing_refresh` filters on:
/// `AND refresh_needed = false`. So the act of asking for posts once, badly,
/// PERMANENTLY excluded the channel from the scheduler's 24h proactive token
/// refresh, and from the analytics cache refresh. Reconnecting gives a new
/// token, which 404s again, which sets the flag again - the founder's reconnect
/// does not fix it and cannot, because the flag was never the problem.
///
/// WHAT THE FLAG DOES NOT DO (measured, so this comment cannot overclaim):
/// `get_due_posts` does NOT read `refresh_needed` — its only integration
/// filter is `i.disabled = false`. So the flag does not stop a SCHEDULED
/// UNSENT POST from publishing. It stops the channel's token from ever being
/// proactively refreshed, which is what turns a fine token into an expired
/// one: the token is 59 days out at the time of the 404 and dies un-refreshed,
/// and only THEN does the post start failing. `handle_feed_failure` must
/// therefore leave the flag alone so the channel keeps its ticket to that window.
///
/// The functions named above are the authority, not a line number: they
/// move independently as the file grows, and a stale citation is a claim that
/// reads as evidence. All three were re-measured on 2026-11-01 by reading the
/// body (`get_due_posts` filters on `disabled` only, `refresh_needed` absent);
/// re-read before trusting either.
///
/// THE RULE
///
/// A 404 (or LinkedIn's `RESOURCE_NOT_FOUND` envelope) means the TARGET is
/// gone: the page was deleted, the ad account was removed, the URN is wrong.
/// None of those are fixed by a token refresh, so the flag must not be set.
///
/// An auth/permission failure is the case the flag exists for: 401 expired,
/// 403 missing scope. Those ARE fixed by a reconnect.
///
/// This lives as one function so the two call sites in this file (and any
/// future third) cannot drift back apart - AGENTS.md 9.18, "one
/// implementation per rule, on both sides of the wire".
pub fn classify_feed_failure(msg: &str) -> FeedFailureKind {
    let m = msg.to_lowercase();
    // LinkedIn's 404 phrasing, kept verbatim because that is the string the
    // live logs contain. Read as a LOWERCASE comparison so a provider that
    // writes "Resource Not Found" or "No virtual resource" matches too.
    let resource_gone = m.contains("no virtual resource")
        || m.contains("resource_not_found")
        || m.contains("not found")
        || m.contains("404");
    // DEFECT D: LinkedIn's READ path (src/social/linkedin.rs:192) replaces its
    // raw 403 with a message that names the FIX - "not granted
    // 'r_member_social' ... reconnect LinkedIn from the Channels screen" - and
    // contains NONE of the tokens the original list looked for. It classified
    // as Transient, so the flag was never set and no reconnect was ever
    // offered, on a channel that can never read a post again. Its own comment
    // calls it "the reconnection the founder actually has to do", so the
    // phrases it uses are the authority. Parentheses are EXPLICIT: `&&` binds
    // tighter than `||`, and an unparenthesised chain is a trap for the next
    // reader (and for a mutation that looks right but is not).
    let grant_miss = m.contains("not granted")
        || m.contains("granted without")
        || m.contains("missing grant")
        || (m.contains("grant") && m.contains("missing"));
    let fixes_with_reconnect = m.contains("enable") && m.contains("reconnect");
    let auth_lost = m.contains("401")
        || m.contains("unauthorized")
        || m.contains("token expired")
        || m.contains("403")
        || m.contains("forbidden")
        || m.contains("permission")
        || m.contains("scope")
        || m.contains("grant required")
        || grant_miss
        || fixes_with_reconnect;
    if auth_lost {
        FeedFailureKind::NeedsReconnect
    } else if resource_gone {
        // The founder's channel is reachable, the posts just are not.
        FeedFailureKind::TargetGone
    } else {
        FeedFailureKind::Transient
    }
}

/// What a failed post fetch means, from the channel's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedFailureKind {
    /// Token or scope: a reconnect fixes this, so the UI should offer one.
    NeedsReconnect,
    /// The page/account/resource the integration points at is gone. A
    /// reconnect does NOT fix it and must not be presented as the fix.
    TargetGone,
    /// Anything else - a provider outage, a timeout, a rate limit. Retry on
    /// the next cycle and do not touch the channel's flag.
    Transient,
}

/// Log a failed post fetch the honest way, and touch the channel's flag
/// ONLY when a reconnect would actually fix it.
///
/// The one thing it must never do is set `refresh_needed` for a
/// [`FeedFailureKind::TargetGone`] - see [`classify_feed_failure`] for the
/// measured production consequence of that mistake.
///
/// The `TargetGone` arm is deliberately LOUD (warn) but non-destructive:
/// the founder needs to know their page is gone, and the message has to say
/// "reconnect will not fix this one" instead of the old claim that a scope
/// was missing.
///
/// This is the integration-test seam for the whole classifier: the tests
/// drive THIS function (not a copy of the rule) against a real DB, so a
/// mutation to the flag logic goes red.
pub async fn handle_feed_failure_for_test(
    db: &SqlitePool,
    integration_id: uuid::Uuid,
    provider_identifier: &str,
    msg: &str,
) -> anyhow::Result<()> {
    handle_feed_failure_inner(db, integration_id, provider_identifier, msg).await
}

async fn handle_feed_failure_inner(
    db: &SqlitePool,
    integration_id: uuid::Uuid,
    provider_identifier: &str,
    msg: &str,
) -> anyhow::Result<()> {
    match classify_feed_failure(msg) {
        FeedFailureKind::NeedsReconnect => {
            tracing::warn!(
                "Integration {} ({}) lost its token or scope — marking refresh_needed: {msg}",
                integration_id,
                provider_identifier
            );
            crate::db::queries::mark_integration_refresh_needed(db, integration_id)
                .await
                .map_err(|e| anyhow::anyhow!("mark refresh_needed failed: {e}"))?;
        }
        FeedFailureKind::TargetGone => {
            // NOT a scope problem and NOT a token problem. Log it so the
            // operator can see it, and leave the flag alone: a channel that
            // can still publish must keep publishing.
            tracing::warn!(
                "Integration {} ({}) points at a resource no longer served ({msg}). \
                 The channel's token is fine, so it stays publishable; the target is gone.",
                integration_id,
                provider_identifier
            );
        }
        FeedFailureKind::Transient => {
            tracing::warn!(
                "Failed to fetch posts for integration {} ({}): {msg}",
                integration_id,
                provider_identifier
            );
        }
    }
    Ok(())
}

/// Poll all non-disabled integrations for a specific user and import their recent posts.
/// Returns the count of newly imported posts.
pub async fn refresh_user_posts(
    db: &SqlitePool,
    user_id: Uuid,
    providers: &ProviderRegistry,
    broadcaster: &Broadcaster,
    token_key: Option<[u8; 32]>,
) -> anyhow::Result<u32> {
    let integrations = crate::db::queries::list_integrations(db, user_id).await?;
    let mut total_new = 0u32;

    for integration in &integrations {
        if integration.disabled {
            continue;
        }

        let provider = match providers.get(&integration.provider_identifier) {
            Some(p) => p,
            None => {
                tracing::debug!(
                    "Skipping integration {}: provider '{}' not in registry",
                    integration.id,
                    integration.provider_identifier
                );
                continue;
            }
        };

        let token = crypto::maybe_decrypt_token(&integration.access_token, token_key.as_ref());

        let posts = match provider
            .get_recent_posts(&token, &integration.internal_id, RECENT_POSTS_LIMIT)
            .await
        {
            Ok(p) => p,
            Err(ProviderError::TokenExpired) => {
                match try_refresh_and_fetch(db, &provider, integration, token_key).await {
                    Some(p) => p,
                    None => continue,
                }
            }
            Err(ref e) => {
                let msg = e.to_string();
                if let Err(e) = handle_feed_failure_inner(
                    db,
                    integration.id,
                    &integration.provider_identifier,
                    &msg,
                )
                .await
                {
                    tracing::warn!(
                        "Failed to record the feed failure for integration {}: {e}",
                        integration.id
                    );
                }
                continue;
            }
        };

        let mut new_count = 0u32;
        for post in &posts {
            let media_val = serde_json::to_value(&post.media).unwrap_or_default();
            let metadata_val = post.metadata.clone().unwrap_or_default();
            match crate::db::queries::insert_external_post(
                db,
                integration.user_id,
                &integration.provider_identifier,
                &post.platform_post_id,
                &post.text,
                post.author_name.as_deref(),
                post.author_handle.as_deref(),
                post.author_avatar.as_deref(),
                post.created_at,
                post.url.as_deref(),
                &media_val,
                &metadata_val,
            )
            .await
            {
                Ok(Some(_)) => new_count += 1,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Failed to import post {}: {e}", post.platform_post_id);
                }
            }
        }

        if new_count > 0 {
            total_new += new_count;
            tracing::info!(
                "Feed: imported {new_count} new {} post(s) for user {}",
                integration.provider_identifier,
                integration.user_id,
            );
        }
    }

    if total_new > 0 {
        broadcaster.send("feed:new_posts", &serde_json::json!({ "count": total_new }));
    }

    Ok(total_new)
}

/// Poll all non-disabled integrations for new posts and import them.
async fn refresh_all_posts(
    db: &SqlitePool,
    providers: &ProviderRegistry,
    broadcaster: &Broadcaster,
    token_key: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    let integrations = crate::db::queries::list_all_integrations_across_users(db).await?;
    let mut total_new = 0u32;

    for integration in &integrations {
        if integration.disabled {
            continue;
        }

        let provider = match providers.get(&integration.provider_identifier) {
            Some(p) => p,
            None => {
                tracing::debug!(
                    "Skipping integration {}: provider '{}' not in registry",
                    integration.id,
                    integration.provider_identifier
                );
                continue;
            }
        };

        // Resolve token (decrypt if needed)
        let token = crypto::maybe_decrypt_token(&integration.access_token, token_key.as_ref());

        // Fetch recent posts from provider
        let posts = match provider
            .get_recent_posts(&token, &integration.internal_id, RECENT_POSTS_LIMIT)
            .await
        {
            Ok(p) => p,
            Err(ProviderError::TokenExpired) => {
                match try_refresh_and_fetch(db, &provider, integration, token_key).await {
                    Some(p) => p,
                    None => continue,
                }
            }
            Err(ref e) => {
                let msg = e.to_string();
                if let Err(e) = handle_feed_failure_inner(
                    db,
                    integration.id,
                    &integration.provider_identifier,
                    &msg,
                )
                .await
                {
                    tracing::warn!(
                        "Failed to record the feed failure for integration {}: {e}",
                        integration.id
                    );
                }
                continue;
            }
        };

        let mut new_count = 0u32;
        for post in &posts {
            let media_val = serde_json::to_value(&post.media).unwrap_or_default();
            let metadata_val = post.metadata.clone().unwrap_or_default();
            tracing::debug!(
                "FEED INSERT DEBUG: provider={} post_id={} name={:?} handle={:?} avatar={} media_len={} media_json_len={}",
                integration.provider_identifier,
                post.platform_post_id,
                post.author_name,
                post.author_handle,
                post.author_avatar.is_some(),
                post.media.len(),
                media_val.as_array().map(|a| a.len()).unwrap_or(0),
            );
            match crate::db::queries::insert_external_post(
                db,
                integration.user_id,
                &integration.provider_identifier,
                &post.platform_post_id,
                &post.text,
                post.author_name.as_deref(),
                post.author_handle.as_deref(),
                post.author_avatar.as_deref(),
                post.created_at,
                post.url.as_deref(),
                &media_val,
                &metadata_val,
            )
            .await
            {
                Ok(Some(_)) => new_count += 1,
                Ok(None) => {} // already exists
                Err(e) => {
                    tracing::warn!("Failed to import post {}: {e}", post.platform_post_id);
                }
            }
        }

        if new_count > 0 {
            total_new += new_count;
            tracing::info!(
                "Feed: imported {new_count} new {} post(s) for user {}",
                integration.provider_identifier,
                integration.user_id,
            );
        }
    }

    if total_new > 0 {
        broadcaster.send("feed:new_posts", &serde_json::json!({ "count": total_new }));
    }

    Ok(())
}

/// Poll engagement data for existing external posts using the canonical fetch_engagement() pipeline.
/// Uses the SocialProvider::fetch_engagement() method to get normalized EngagementData,
/// then upserts into the post_engagement table.
async fn refresh_all_engagement(
    db: &SqlitePool,
    providers: &ProviderRegistry,
    broadcaster: &Broadcaster,
    token_key: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    use crate::social::EngagementRow;

    let integrations = crate::db::queries::list_all_integrations_across_users(db).await?;
    let mut updated_count = 0u32;

    for integration in &integrations {
        if integration.disabled {
            continue;
        }

        let provider = match providers.get(&integration.provider_identifier) {
            Some(p) => p,
            None => continue,
        };

        let token = crypto::maybe_decrypt_token(&integration.access_token, token_key.as_ref());

        // Fetch recent posts from DB to get their platform_post_ids
        let recent_posts = crate::db::queries::list_external_posts(
            db,
            integration.user_id,
            &integration.provider_identifier,
            50,
        )
        .await?;

        for post in &recent_posts {
            // Fetch normalized engagement data from provider
            let engagement = match provider
                .fetch_engagement(&token, &post.platform_post_id)
                .await
            {
                Ok(Some(e)) => e,
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!(
                        "Engagement fetch failed for {} post {}: {e}",
                        integration.provider_identifier,
                        post.platform_post_id
                    );
                    continue;
                }
            };

            // Convert to DB row and upsert into post_engagement table
            let row: EngagementRow = engagement.into();
            match crate::db::queries::upsert_post_engagement(db, post.id, &row).await {
                Ok(_) => updated_count += 1,
                Err(e) => {
                    tracing::warn!("Failed to upsert engagement for post {}: {e}", post.id);
                }
            }
        }
    }

    if updated_count > 0 {
        broadcaster.send(
            "feed:engagement",
            &serde_json::json!({ "updated": updated_count }),
        );
    }

    Ok(())
}

/// Refresh the cached_comments table for all integrations (B-3).
///
/// For each non-disabled integration, fetches the most recent
/// `COMMENTS_REFRESH_POST_LIMIT` external posts from the DB, then calls
/// `provider.get_post_comments()` for each and upserts the results into
/// `cached_comments`. The comments list endpoint reads from this cache
/// instead of doing 50 sequential API calls per page load.
///
/// Failures are logged and skipped — one provider being down shouldn't
/// prevent the cache from being refreshed for other providers.
async fn refresh_all_comments(
    db: &SqlitePool,
    providers: &ProviderRegistry,
    broadcaster: &Broadcaster,
    token_key: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    let integrations = crate::db::queries::list_all_integrations_across_users(db).await?;
    let mut total_cached = 0u32;

    for integration in &integrations {
        if integration.disabled {
            continue;
        }

        let provider = match providers.get(&integration.provider_identifier) {
            Some(p) => p,
            None => continue,
        };

        let token = crypto::maybe_decrypt_token(&integration.access_token, token_key.as_ref());

        // Fetch the most recent N posts for this integration from the DB.
        // We cache comments only for recent posts — older posts' comments
        // are unlikely to be actively read.
        let recent_posts = crate::db::queries::list_external_posts(
            db,
            integration.user_id,
            &integration.provider_identifier,
            COMMENTS_REFRESH_POST_LIMIT,
        )
        .await?;

        for post in &recent_posts {
            // Fetch comments from the provider.
            let comments = match provider
                .get_post_comments(&token, &post.platform_post_id)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    // Don't log at warn for every post — many providers
                    // don't implement get_post_comments (B-4) and return
                    // an error. Debug level is enough.
                    tracing::debug!(
                        "Comment fetch failed for {} post {}: {e}",
                        integration.provider_identifier,
                        post.platform_post_id
                    );
                    continue;
                }
            };

            if comments.is_empty() {
                continue;
            }

            // Convert provider comments to the tuple shape upsert_cached_comments expects:
            // (comment_id, text, author_name, author_handle, author_avatar, created_at)
            // Note: CommentData doesn't have author_handle, so we pass None for that.
            type CacheRow = (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                chrono::DateTime<chrono::Utc>,
            );
            let batch: Vec<CacheRow> = comments
                .into_iter()
                .map(|c| {
                    (
                        c.id,
                        c.text,
                        c.author_name,
                        None, // author_handle — provider CommentData doesn't expose it
                        c.author_avatar,
                        c.created_at,
                    )
                })
                .collect();

            match crate::db::queries::upsert_cached_comments(
                db,
                integration.user_id,
                post.id,
                &integration.provider_identifier,
                &batch,
            )
            .await
            {
                Ok(_) => total_cached += batch.len() as u32,
                Err(e) => {
                    tracing::warn!("Failed to upsert cached comments for post {}: {e}", post.id);
                }
            }
        }
    }

    if total_cached > 0 {
        tracing::info!("Comments cache: refreshed {total_cached} comment(s)");
        // Phase 4: broadcast so the comments page realtime subscription lights up.
        broadcaster.send(
            "comment_received",
            &serde_json::json!({ "count": total_cached }),
        );
    }

    Ok(())
}
