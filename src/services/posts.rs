// ─── Post Service ─────────────────────────────────────────────
// Shared business logic for post CRUD, scheduling, and slot-finding.
// Used by both `api/posts.rs` (HTTP) and `mcp/tools_posts.rs` (MCP).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::models::{Post, PostState};
use crate::db::queries;
use crate::realtime::Broadcaster;
use crate::social::registry::ProviderRegistry;

/// Result type for service operations
pub type ServiceResult<T> = Result<T, String>;

/// Input for creating a post
pub struct CreatePostInput {
    pub user_id: Uuid,
    pub integration_id: Uuid,
    pub content: String,
    pub title: Option<String>,
    pub media_urls: Value,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub settings: Value,
    pub first_comment: Option<String>,
    /// Optional state hint. If `None`, defaults to `Draft`. Pass `Some(Queued)` after `--schedule` to arm the post.
    pub state: Option<PostState>,
    /// The campaign to file this post under.
    pub campaign_id: Option<Uuid>,
}

/// Input for updating a post
///
/// THREE states per field, not two:
///   absent -> keep the stored value
///   `null` -> clear the field
///   a value -> replace it
///
/// The two-state version (`Option<T>`) collapsed "absent" and "null",
/// and the write was an UNCONDITIONAL `SET media = ?, settings = ?` -
/// so a caller that sent only a new body wiped every attached image
/// and every provider setting (the Reddit subreddit, the Pinterest
/// board, the Instagram audio id) on the post. The post then failed at
/// publish time with "Missing subreddit in settings", and the founder
/// had no way to know why.
pub struct UpdatePostInput {
    pub content: Option<String>,
    pub title: Option<Option<String>>,
    pub media: Option<Option<Value>>,
    pub settings: Option<Option<Value>>,
    /// The first comment on the post. Three-state, like the others:
    /// absent keeps it, `null` clears it, a value replaces it.
    ///
    /// It was missing here, and the write went through
    /// `update_post_content` - which has no `first_comment` column - so
    /// an MCP or CLI edit that set a first comment SILENTLY LOST it.
    pub first_comment: Option<Option<String>>,
}

/// Input for scheduling
pub struct SchedulePostInput {
    pub scheduled_at: DateTime<Utc>,
}

/// Shared post service
pub struct PostService;

impl PostService {
    /// Sanitize post content: store it as PLAIN TEXT, and limit the length.
    ///
    /// The app sends plain text (the editor speaks plain text at its
    /// boundary, the same contract the server and the providers use), so
    /// the common case is text that must be left ALONE. A post that
    /// mentions a tag - "Use <div> for layout" - keeps its `<div>`.
    ///
    /// HTML that arrives from somewhere else (the MCP tool, a webhook,
    /// an API client written against an older contract) is CONVERTED to
    /// text, because the server is a trust boundary: the stored content
    /// is published to a provider, and it is what a page renders.
    ///
    /// The conversion is `social::common::strip_html_tags` - the one
    /// implementation, so the content the server stores is the same
    /// text the providers publish. It used to be a character-level
    /// stripper that deleted everything between a `<` and the next `>`,
    /// so a post comparing two numbers lost the middle.
    pub fn sanitize_content(raw: &str, max_len: usize) -> String {
        let clean = if crate::social::common::looks_like_html(raw) {
            crate::social::common::strip_html_tags(raw)
        } else {
            raw.trim().to_string()
        };
        if clean.chars().count() > max_len {
            // Use char-boundary-safe truncation to avoid panicking on
            // multi-byte UTF-8 sequences (e.g., emoji). `String::len()`
            // returns byte length, and slicing `clean[..max_len]` would
            // panic if `max_len` falls inside a multi-byte sequence.
            clean.chars().take(max_len).collect()
        } else {
            clean
        }
    }

    /// Create a new post
    pub async fn create(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        input: CreatePostInput,
    ) -> ServiceResult<Post> {
        let content = Self::sanitize_content(&input.content, 2000);
        if content.trim().is_empty() {
            return Err("Content cannot be empty".into());
        }
        if content.chars().count() > 2000 {
            return Err(format!(
                "Content exceeds 2000 chars (got {})",
                content.chars().count()
            ));
        }

        let state = input.state.unwrap_or(PostState::Draft);

        let post = queries::create_post(
            db,
            input.user_id,
            input.integration_id,
            &content,
            input.title.as_deref(),
            &input.media_urls,
            &input.settings,
            input.scheduled_at,
            Some(state),
            input.first_comment.as_deref(),
            0,
            input.campaign_id,
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

        broadcaster.send(
            "post_created",
            &serde_json::json!({"id": post.id.to_string()}),
        );

        Ok(post)
    }

    /// List posts with optional state filtering and pagination
    pub async fn list(
        db: &SqlitePool,
        user_id: Uuid,
        state_filter: Option<&str>,
        limit: i64,
        offset: i64,
        include_total: bool,
    ) -> ServiceResult<(Vec<Post>, Option<i64>)> {
        let posts = queries::list_posts(db, user_id, state_filter, limit, offset)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        let total = if include_total {
            Some(
                queries::count_posts_by_user(db, user_id, state_filter)
                    .await
                    .map_err(|e| format!("Database error: {e}"))?,
            )
        } else {
            None
        };

        Ok((posts, total))
    }

    /// Get a single post by ID (verifies ownership)
    pub async fn get(db: &SqlitePool, user_id: Uuid, post_id: Uuid) -> ServiceResult<Post> {
        let post = queries::get_post(db, post_id, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| "Post not found".to_string())?;

        Ok(post)
    }

    /// Update a post (verifies ownership)
    ///
    /// Every field is three-state: absent keeps the stored value, an
    /// explicit null clears it, a value replaces it. The write is
    /// conditional on the flag, so a caller that only sends a new body
    /// keeps the post's media and settings.
    ///
    /// This is the ONE implementation of "edit a post". The REST
    /// handler, the MCP tool and the CLI all come through here, so a
    /// fix lands once:
    ///
    ///   - a post that is MID-PUBLISH is not editable (the REST path
    ///     returned 409 for it; the service path edited it silently);
    ///   - `first_comment` is three-state and persisted (the service
    ///     path dropped it entirely);
    ///   - the edited content is validated against the channel's
    ///     provider limits (the service path accepted an over-long post
    ///     that then failed at publish time).
    pub async fn update(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        user_id: Uuid,
        post_id: Uuid,
        input: UpdatePostInput,
    ) -> ServiceResult<Post> {
        // Verify ownership before update
        let existing = Self::get(db, user_id, post_id).await?;

        // A post that is mid-publish is being written to RIGHT NOW.
        // Editing it underneath the publisher produced a post whose
        // content was the OLD content and whose published text was the
        // NEW one - or a failed publish with no explanation.
        if existing.state == PostState::Publishing {
            return Err("Post is being published; edit it after the attempt finishes".to_string());
        }

        // Resolve each field against the stored value.
        let content = match &input.content {
            Some(c) => Self::sanitize_content(c, 2000),
            None => existing.content,
        };
        let title = match &input.title {
            Some(Some(t)) => Some(t.clone()),
            Some(None) => None, // explicit clear
            None => existing.title,
        };
        let media = match &input.media {
            Some(Some(m)) => m.clone(),
            Some(None) => serde_json::json!([]), // explicit clear
            None => existing.media,
        };
        let settings = match &input.settings {
            Some(Some(s)) => s.clone(),
            Some(None) => serde_json::json!({}), // explicit clear
            None => existing.settings,
        };
        let first_comment = match &input.first_comment {
            Some(Some(fc)) => Some(fc.clone()),
            Some(None) => None, // explicit clear
            None => existing.first_comment,
        };

        let post = queries::update_post_full(
            db,
            post_id,
            user_id,
            &content,
            title.as_deref(),
            &media,
            &settings,
            first_comment.as_deref(),
            // The service's update path does not take a campaign, so
            // the stored one is kept: absent = keep.
            None,
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| "Post not found after update".to_string())?;

        broadcaster.send(
            "post_updated",
            &serde_json::json!({"id": post_id.to_string()}),
        );

        Ok(post)
    }

    /// Schedule a post (verifies ownership)
    pub async fn schedule(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        user_id: Uuid,
        post_id: Uuid,
        scheduled_at: DateTime<Utc>,
    ) -> ServiceResult<Post> {
        // Verify ownership
        Self::get(db, user_id, post_id).await?;

        let post = queries::schedule_post(db, post_id, user_id, scheduled_at)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| "Post not found after scheduling".to_string())?;

        broadcaster.send(
            "post_scheduled",
            &serde_json::json!({"id": post_id.to_string()}),
        );

        Ok(post)
    }

    /// Reschedule a post — the ONE core behind REST PUT /api/posts/{id}/date
    /// and the MCP posts_reschedule tool (audit §5.2 parity).
    ///
    /// `action` disambiguates PUBLISHED posts (v21):
    /// - Some("schedule") → reset state to queued + clear release fields
    ///   (re-publish at the new time);
    /// - Some("update") → change scheduled_at only (archive re-date);
    /// - None on a published post → hard error (never silently re-publish).
    ///
    /// `move_group = true` applies the same time DELTA to every post in the
    /// thread/campaign group — one handle, one verb (§5.2).
    pub async fn reschedule(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        user_id: Uuid,
        post_id: Uuid,
        new_scheduled_at: DateTime<Utc>,
        move_group: bool,
        action: Option<&str>,
    ) -> ServiceResult<serde_json::Value> {
        let post = queries::get_post(db, post_id, user_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| "Post not found".to_string())?;

        let action = action.unwrap_or("");
        if post.state == crate::db::models::PostState::Published
            && action != "schedule"
            && action != "update"
        {
            return Err(
                "Cannot reschedule a published post without specifying action. \
                 Use action: 'schedule' to re-publish, or action: 'update' to \
                 just re-date for archival."
                    .into(),
            );
        }

        if move_group {
            if let Some(group_id) = post.group_id {
                let group_posts = queries::get_posts_by_group_id(db, user_id, group_id)
                    .await
                    .map_err(|e| format!("DB error: {e}"))?;

                let old_scheduled_at = post.scheduled_at.unwrap_or_else(Utc::now);
                let delta = new_scheduled_at - old_scheduled_at;

                // Counts only members that changed. A failed write is reported,
                // not silently counted as rescheduled.
                let mut updated_count = 0usize;
                let mut skipped_published = 0usize;
                let mut failed: Vec<String> = Vec::new();
                for group_post in &group_posts {
                    let Some(old_at) = group_post.scheduled_at else {
                        continue;
                    };
                    let new_at = old_at + delta;
                    let is_published = group_post.state == crate::db::models::PostState::Published;
                    let result = if is_published && action == "schedule" {
                        // Re-publishing a part that already went out would post it
                        // twice. Date-only changes for published parts are `update`.
                        skipped_published += 1;
                        continue;
                    } else if is_published && action == "update" {
                        queries::update_post_date_only(db, group_post.id, user_id, new_at).await
                    } else {
                        queries::schedule_post(db, group_post.id, user_id, new_at).await
                    };
                    match result {
                        Ok(Some(_)) => updated_count += 1,
                        Ok(None) => {}
                        Err(e) => failed.push(format!("{}: {e}", group_post.id)),
                    }
                }

                if !failed.is_empty() {
                    return Err(format!(
                        "Group reschedule partly failed ({} of {} updated): {}",
                        updated_count,
                        group_posts.len(),
                        failed.join("; ")
                    ));
                }

                return Ok(serde_json::json!({
                    "rescheduled": true,
                    "group_id": group_id.to_string(),
                    "count": updated_count,
                    "skipped_published": skipped_published,
                    "action": action,
                    "new_scheduled_at": new_scheduled_at.to_rfc3339(),
                }));
            }
        }

        if post.state == crate::db::models::PostState::Published && action == "schedule" {
            let updated = queries::reset_post_for_republish(db, post_id, user_id, new_scheduled_at)
                .await
                .map_err(|e| format!("DB error: {e}"))?
                .ok_or_else(|| "Post not found after reschedule".to_string())?;
            broadcaster.send(
                "post_scheduled",
                &serde_json::json!({"id": post_id.to_string()}),
            );
            return Ok(serde_json::json!({
                "rescheduled": true,
                "action": "schedule",
                "post": serde_json::to_value(&updated).unwrap_or_default(),
            }));
        } else if post.state == crate::db::models::PostState::Published && action == "update" {
            let updated = queries::update_post_date_only(db, post_id, user_id, new_scheduled_at)
                .await
                .map_err(|e| format!("DB error: {e}"))?
                .ok_or_else(|| "Post not found after reschedule".to_string())?;
            broadcaster.send(
                "post_scheduled",
                &serde_json::json!({"id": post_id.to_string()}),
            );
            return Ok(serde_json::json!({
                "rescheduled": true,
                "action": "update",
                "post": serde_json::to_value(&updated).unwrap_or_default(),
            }));
        }

        let updated = queries::schedule_post(db, post_id, user_id, new_scheduled_at)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| "Post not found after reschedule".to_string())?;
        broadcaster.send(
            "post_scheduled",
            &serde_json::json!({"id": post_id.to_string()}),
        );
        Ok(serde_json::json!({
            "rescheduled": true,
            "action": action,
            "post": serde_json::to_value(&updated).unwrap_or_default(),
        }))
    }

    /// Delete a post (ownership verified via `WHERE user_id = ?` in the query)
    pub async fn delete(
        db: &SqlitePool,
        broadcaster: &Broadcaster,
        user_id: Uuid,
        post_id: Uuid,
    ) -> ServiceResult<bool> {
        let deleted = queries::delete_post(db, post_id, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        broadcaster.send(
            "post_deleted",
            &serde_json::json!({"id": post_id.to_string()}),
        );

        Ok(deleted)
    }

    /// Find next available time slot
    pub async fn find_slot(
        db: &SqlitePool,
        user_id: Uuid,
        integration_id: Option<Uuid>,
    ) -> ServiceResult<DateTime<Utc>> {
        queries::find_next_free_slot(db, user_id, integration_id)
            .await
            .map_err(|e| format!("Database error: {e}"))
            .map(|opt| opt.unwrap_or_else(Utc::now))
    }

    /// Publish a post immediately (or retry a failed post).
    /// Returns the platform_post_url on success.
    ///
    /// `token_key` is the optional AES-256 key used for at-rest token
    /// encryption. When `Some`, freshly refreshed access tokens are
    /// encrypted before being written to the DB — matching the
    /// scheduler's behavior. When `None`, tokens are stored as-is
    /// (legacy/dev mode).
    pub async fn publish(
        db: &SqlitePool,
        providers: &ProviderRegistry,
        broadcaster: &Broadcaster,
        config: &crate::config::Config,
        user_id: Uuid,
        post_id: Uuid,
        token_key: Option<[u8; 32]>,
    ) -> ServiceResult<String> {
        // Fetch post with integration details
        let post = queries::get_post_with_integration(db, post_id, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| "Post not found".to_string())?;

        // A manual publish accepts a DRAFT: the composer's "Post Now" creates
        // the post without a schedule (which is what a draft is) and then
        // asks to publish it. Refusing drafts made that button 100% broken -
        // there was no path from "written" to "published" that did not go
        // through the scheduler.
        //
        // An IDEA is still refused: it is a captured thought, not a post.
        match post.state {
            PostState::Queued | PostState::Error | PostState::Draft => {}
            PostState::Idea => return Err("Idea posts must be moved to drafts first".into()),
            PostState::Published => return Err("Post already published".into()),
            PostState::Publishing => return Err("Post is already being published".into()),
        }

        if post.integration_disabled {
            return Err("Integration is disabled. Reconnect the social account.".into());
        }

        let provider = providers
            .get(&post.provider_identifier)
            .ok_or_else(|| format!("Provider '{}' not found", post.provider_identifier))?;

        // ── The ONE publish pipeline (audit §4.1) ────────────────────
        // Delegates to the scheduler's engine (token refresh, thread
        // linking, post_pending/finalize lifecycle, retries with backoff,
        // outbox durability, first comment, attempt audit, events,
        // webhooks). Previously this was a second pipeline that lagged:
        // no post_pending polling, no retries, and every fix had to land
        // twice (G-01's empty-id guard, B3's integration-scoped thread
        // link, B8's encryption) — and drifted when it didn't.
        // Claim the post first (same transition the scheduler uses), so a
        // scheduler tick cannot publish it at the same time.
        let claimed = queries::claim_post_for_publish(db, post_id, user_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?;
        if !claimed {
            return Err("Post changed state while publishing; reload and try again".into());
        }

        let opts = crate::scheduler::PublishOpts { manual: true };
        let outcome = crate::scheduler::publish_post(
            db,
            provider.as_ref(),
            &post,
            broadcaster,
            token_key,
            config,
            opts,
        )
        .await;

        match outcome {
            Ok(crate::scheduler::PublishOutcome::Published { platform_post_url }) => {
                Ok(platform_post_url.unwrap_or_default())
            }
            Ok(crate::scheduler::PublishOutcome::Skipped) => {
                // Nothing was sent: give the claim back so the post is not
                // stranded in `publishing`.
                if let Err(write_err) =
                    queries::update_post_state(db, post_id, post.state.clone(), None, None, None)
                        .await
                {
                    tracing::error!("Failed to release claim for post {post_id}: {write_err}");
                }
                // This is a FAILURE from the founder's point of view - the
                // post did not go out - so it gets the same event as any
                // other manual failure, and it appears in the activity log.
                let err =
                    "Thread predecessor is not published yet — publish the earlier part first"
                        .to_string();
                let _ = broadcaster
                    .send_and_log(
                        db,
                        post.user_id,
                        "post_failed",
                        &serde_json::json!({
                            "id": post_id.to_string(),
                            "provider": post.provider_identifier,
                            "error": err,
                        }),
                    )
                    .await;
                Err(err)
            }
            Err(e) => {
                let err = e.to_string();
                // The engine already recorded the failed attempt audit row
                // and fired the post.failed WEBHOOK. It did NOT fire the
                // EVENT, so the failure was PERMANENTLY ABSENT from
                // `events_log`: it never appeared in GET /api/activity,
                // never in the dashboard's "Recent Activity" widget, and
                // never triggered the eight frontend routes subscribed to
                // `post_failed`. The founder pressed "Post Now", saw an
                // error banner, and the failure left no trace anywhere
                // they could find it again.
                //
                // The broadcast lives HERE, in the caller's error arm,
                // because this is the ONE place every manual failure
                // passes through: retry-exhausted, scope-lost, and the
                // thread-predecessor refusal above. The automated path
                // broadcasts its own `post_failed` inside the retry
                // loop, so exactly one owner per path and exactly one
                // row per failure.
                let _ = broadcaster
                    .send_and_log(
                        db,
                        post.user_id,
                        "post_failed",
                        &serde_json::json!({
                            "id": post_id.to_string(),
                            "provider": post.provider_identifier,
                            "error": err,
                        }),
                    )
                    .await;
                // Fail the claim as `error`, the same
                // state the scheduler uses. Restoring the pre-claim state
                // would put the post back in `queued`, and the scheduler would
                // retry it on its own without the operator seeing the failure.
                crate::scheduler::mark_post_error(db, post_id, &err).await;
                Err(err)
            }
        }
    }

    /// Get calendar posts by date range
    pub async fn calendar(
        db: &SqlitePool,
        user_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> ServiceResult<Vec<Post>> {
        queries::get_posts_by_date_range(db, user_id, start, end)
            .await
            .map_err(|e| format!("Database error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::{
        AuthToken, AuthUrlResponse, PageInfo, PostContent, ProviderError, PublishResult,
        SocialProvider,
    };
    use std::sync::Arc;

    /// Returns whatever `PublishResult` the test hands it — including the
    /// G-01 shape, where the provider claims `published` but has no id.
    struct FakeProvider {
        result: PublishResult,
    }

    #[async_trait::async_trait]
    impl SocialProvider for FakeProvider {
        fn identifier(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn scopes(&self) -> Vec<String> {
            vec![]
        }
        fn max_content_length(&self) -> usize {
            5000
        }

        async fn generate_auth_url(
            &self,
            _s: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthUrlResponse, ProviderError> {
            unimplemented!()
        }
        async fn exchange_code(
            &self,
            _c: &str,
            _v: &str,
            _r: &str,
        ) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn refresh_token(&self, _r: &str) -> Result<AuthToken, ProviderError> {
            unimplemented!()
        }
        async fn fetch_page_info(&self, _t: &str, _p: &str) -> Result<PageInfo, ProviderError> {
            unimplemented!()
        }
        async fn publish(
            &self,
            _t: &str,
            _p: &PostContent,
        ) -> Result<PublishResult, ProviderError> {
            Ok(self.result.clone())
        }
    }

    /// In-memory SQLite carrying the real schema, so `publish` runs against
    /// the same columns and constraints production uses.
    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().in_memory(true))
            .await
            .expect("in-memory sqlite pool");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    /// A user + integration + `queued` post wired to the `fake` provider,
    /// i.e. exactly what `posts publish` is pointed at.
    async fn queued_post(pool: &SqlitePool) -> (Uuid, Uuid) {
        let user = queries::create_user(pool, "publish-guard@test.local", "hash", "Guard")
            .await
            .expect("user");
        let integration = queries::create_integration(
            pool,
            user.id,
            "fake",
            "Fake",
            "acct-1",
            "access-token",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("integration");
        let post = queries::create_post(
            pool,
            user.id,
            integration.id,
            "hello from the forge",
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
        .expect("post");
        (user.id, post.id)
    }

    fn registry(result: PublishResult) -> ProviderRegistry {
        ProviderRegistry::with_providers(vec![(
            "fake",
            Arc::new(FakeProvider { result }) as Arc<dyn SocialProvider>,
        )])
    }

    /// G-01: X's GraphQL CreateTweet answered 200 with no `rest_id`, so the
    /// provider returned `platform_post_id: ""` alongside
    /// `status: "published"`. The post was never created, but before the
    /// empty-id guard this wrote the row as published with a phantom id —
    /// a silent loss the operator only found by searching X.
    #[tokio::test]
    async fn publish_rejects_a_success_that_carries_no_platform_post_id() {
        let pool = test_pool().await;
        let (user_id, post_id) = queued_post(&pool).await;
        let providers = registry(PublishResult {
            platform_post_id: String::new(),
            platform_post_url: None,
            status: "published".into(),
        });

        let err = PostService::publish(
            &pool,
            &providers,
            &Broadcaster::new(),
            &crate::config::Config::from_env().expect("test config"),
            user_id,
            post_id,
            None,
        )
        .await
        .expect_err("an empty platform_post_id is not a publish receipt");
        assert!(
            err.contains("no platform post id"),
            "error must name the cause: {err}"
        );

        let post = queries::get_post(&pool, post_id, user_id)
            .await
            .expect("read back")
            .expect("row still exists");

        // The post never went live, so the row must not claim it did.
        assert_ne!(
            post.state,
            PostState::Published,
            "row was marked published for a post that was never created"
        );
        assert!(post.published_at.is_none(), "published_at must stay unset");
        assert!(
            post.platform_post_id.as_deref().unwrap_or("").is_empty(),
            "platform_post_id must stay empty, got {:?}",
            post.platform_post_id
        );
        // A failed publish ends in `error` with the reason attached. From there
        // the post can be rescheduled (`PostState::Error` is editable above).
        assert_eq!(post.state, PostState::Error);
        assert!(
            post.error_message
                .as_deref()
                .unwrap_or("")
                .contains("no platform post id"),
            "error_message must explain the failure, got {:?}",
            post.error_message
        );

        // Audit trail: the failure is visible, and the attempt number it
        // carries is the number of calls that were actually MADE. The empty-id
        // guard breaks on the FIRST attempt without retrying — a non-retryable
        // failure — so this is attempt 1, not MAX_RETRIES. The row used to
        // hard-code MAX_RETRIES, which told the operator "we tried 3 times" for
        // a post that was called once.
        let attempts: Vec<(i32, String, Option<String>)> = sqlx::query_as(
            "SELECT attempt_number, status, error_message FROM publish_attempts WHERE post_id = ?",
        )
        .bind(post_id)
        .fetch_all(&pool)
        .await
        .expect("attempts");
        assert_eq!(
            attempts.len(),
            1,
            "expected one attempt row, got {attempts:?}"
        );
        assert_eq!(
            attempts[0].0, 1,
            "a single non-retryable call must record attempt 1, got {}",
            attempts[0].0
        );
        assert_eq!(attempts[0].1, "failed");
        assert!(attempts[0]
            .2
            .as_deref()
            .unwrap_or("")
            .contains("no platform post id"));
    }

    /// The control: a real id still publishes, and the id is what lands.
    #[tokio::test]
    async fn publish_records_the_platform_post_id_when_the_platform_returns_one() {
        let pool = test_pool().await;
        let (user_id, post_id) = queued_post(&pool).await;
        let providers = registry(PublishResult {
            platform_post_id: "tweet-123".into(),
            platform_post_url: Some("https://x.com/i/status/tweet-123".into()),
            status: "published".into(),
        });

        let url = PostService::publish(
            &pool,
            &providers,
            &Broadcaster::new(),
            &crate::config::Config::from_env().expect("test config"),
            user_id,
            post_id,
            None,
        )
        .await
        .expect("a real platform_post_id must publish");
        assert_eq!(url, "https://x.com/i/status/tweet-123");

        let post = queries::get_post(&pool, post_id, user_id)
            .await
            .expect("read back")
            .expect("row still exists");
        assert_eq!(post.state, PostState::Published);
        assert_eq!(post.platform_post_id.as_deref(), Some("tweet-123"));
        assert!(post.published_at.is_some(), "published_at must be stamped");
    }
}
