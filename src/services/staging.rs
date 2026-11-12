// ─── Staging Service ──────────────────────────────────────────
// Orchestrates multi-platform post staging with content splitting.
// Takes a single post intent and creates appropriate posts per platform.
//
// v2 (audit 2026-10-14 §5.2, B4, B14):
// - per-provider `overrides` (content / settings / thread flag);
// - one campaign `group_id` for everything staged — one handle for the
//   whole multi-platform campaign;
// - splits are THREADS: ordered sequences in one group, linked by the
//   B3 predecessor gate at publish time (no more "3 unrelated tweets
//   in arbitrary order");
// - the whole stage is ONE transaction — half-staged campaigns are gone;
// - readiness errors (missing subreddit, over-limit content, bad media
//   URL) REJECT that integration unless `force`;
// - `dry_run` reports per-integration readiness and writes nothing.

use crate::db::queries;
use crate::services::content_splitter;
use crate::social::{self, registry::ProviderRegistry, MediaAttachment, PostContent};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug)]
pub struct StagingRequest {
    pub content: String,
    pub media: serde_json::Value,
    pub integration_ids: Vec<Uuid>,
    pub settings: serde_json::Value,
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// E4: queue each integration at its own next free posting slot
    /// (per-account posting queue) instead of `scheduled_at`.
    pub next_slot: bool,
    pub first_comment: Option<String>,
    // ── v2 ──
    /// Per-provider overrides: `{"reddit": {"settings": {"subreddit":
    /// "rust"}}, "x": {"content": "short copy"}}`. `content` replaces the
    /// base copy for that provider; `settings` shallow-merges over the
    /// base settings — a Reddit `title` no longer leaks into Medium.
    pub overrides: serde_json::Value,
    /// Report readiness for every integration, write nothing.
    pub dry_run: bool,
    /// Stage integrations whose readiness reported errors anyway.
    pub force: bool,
    /// Attach to an existing campaign group instead of minting one.
    pub group_id: Option<Uuid>,
}

#[derive(Debug)]
pub struct StagedPost {
    pub post_id: Uuid,
    pub provider: String,
    pub sequence: usize,
    pub total_segments: usize,
    pub state: String,
    /// Campaign handle shared by every post this stage created.
    pub group_id: Uuid,
}

#[derive(Debug)]
pub struct StagingResult {
    pub staged: Vec<StagedPost>,
    pub total_posts: usize,
    pub warnings: Vec<String>,
    /// Readiness errors, one line per rejected integration (B14).
    pub errors: Vec<String>,
    /// Providers whose integration was rejected by readiness (or
    /// force-staged despite it, when `force` is set).
    pub rejected: Vec<String>,
    /// Campaign handle: every lifecycle verb (reschedule, unschedule,
    /// publish-now, delete, status) accepts it.
    pub group_id: Option<Uuid>,
}

/// Read the request's `media` array into the shape `validate_media_limits`
/// and provider `validate_media` expect. Fields the caller omitted (the CLI
/// only ever supplies `url`) come through empty rather than being invented —
/// the pre-check is advisory, so a missing mime must not fabricate one.
fn parse_media(value: &serde_json::Value) -> Vec<MediaAttachment> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    Some(MediaAttachment {
                        url: m.get("url")?.as_str()?.to_string(),
                        mime_type: m
                            .get("mime_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        alt: m.get("alt").and_then(|v| v.as_str()).map(String::from),
                        poster_url: m
                            .get("poster_url")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Everything the scheduler would otherwise only discover at publish time.
///
/// One line per distinct failure rather than a single boolean, so a caller
/// can see *which* rule the post breaks instead of being told that it does.
fn publish_readiness(
    provider_id: &str,
    post: &PostContent,
    registry: &ProviderRegistry,
    app_url: &str,
) -> Vec<String> {
    let Some(provider) = registry.get(provider_id) else {
        return vec![format!(
            "{provider_id} is not registered in this build — the draft is stored but nothing can publish it"
        )];
    };

    let mut out = Vec::new();
    if let Err(e) = provider.validate_post(post) {
        out.push(format!("{provider_id}: {e}"));
    }
    if let Err(e) = social::validate_media_limits(provider_id, post) {
        out.push(format!("{provider_id}: {e}"));
    }
    // Readiness must see EXACTLY what would be published. The publish path
    // resolves a relative `/api/media/<uuid>` against APP_URL before the
    // provider sees it (`SocialProvider::resolve_media_url`), so the
    // readiness check has to resolve it the same way — otherwise a stored
    // media row was reported as "neither an http(s) URL nor an existing
    // local file" and a post that would have published was rejected.
    for attachment in &post.media {
        let resolved = provider.resolve_media_url(attachment, app_url);
        if let Some(problem) = social::media_url_problem(provider_id, &resolved.url) {
            let url = &attachment.url;
            out.push(format!("{provider_id}: media `{url}` — {problem}"));
        }
    }
    out
}

/// Shallow-merge `over` into `base` (`over` wins per key).
fn merge_settings(base: &serde_json::Value, over: &serde_json::Value) -> serde_json::Value {
    let mut merged = base.as_object().cloned().unwrap_or_default();
    if let Some(over_map) = over.as_object() {
        for (k, v) in over_map {
            merged.insert(k.clone(), v.clone());
        }
    }
    serde_json::Value::Object(merged)
}

/// The per-integration plan after overrides are applied.
struct IntegrationPlan {
    integration_id: Uuid,
    provider: String,
    parts: Vec<queries::ThreadPart>,
    readiness: Vec<String>,
}

/// Stage `request` against every integration, splitting content per platform.
///
/// The provider registry is required, not optional: a draft that cannot
/// publish is stored exactly like one that can, so the only place the
/// difference is visible is here — before the scheduler spends a retry
/// budget discovering it.
pub async fn stage_post(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    app_url: &str,
    user_id: Uuid,
    request: StagingRequest,
) -> Result<StagingResult, String> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut rejected = Vec::new();

    let media = parse_media(&request.media);
    let group_id = request.group_id.unwrap_or_else(Uuid::new_v4);

    // ── Plan: resolve every integration, apply overrides, split, and
    // check readiness BEFORE writing anything (dry_run stops here).
    let mut plans: Vec<IntegrationPlan> = Vec::new();
    let mut accepted: Vec<IntegrationPlan> = Vec::new();

    for integration_id in &request.integration_ids {
        let integration = queries::get_integration(pool, *integration_id, user_id)
            .await
            .map_err(|e| format!("Integration not found: {e}"))?
            .ok_or_else(|| format!("Integration {} not found", integration_id))?;
        let provider = integration.provider_identifier.clone();

        // Publish refuses a disabled integration (services/posts.rs). Staging
        // must too; `force` does not override it.
        if integration.disabled {
            errors.push(format!(
                "{provider} — rejected: integration is disabled. Reconnect the social account"
            ));
            rejected.push(provider);
            continue;
        }

        let ov = request
            .overrides
            .get(&provider)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let eff_content = ov
            .get("content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| request.content.clone());
        let eff_settings = merge_settings(
            &request.settings,
            ov.get("settings").unwrap_or(&serde_json::Value::Null),
        );
        let eff_first_comment = ov
            .get("first_comment")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .or_else(|| request.first_comment.clone());

        // Readiness sees exactly what would be published (overrides
        // included). B14: the WHOLE content is validated to catch what a
        // split cannot fix (missing subreddit, zero content limit) — but
        // a pure length failure that every split segment resolves is a
        // warning, not a rejection.
        let preflight = PostContent {
            content: eff_content.clone(),
            media: media.clone(),
            settings: eff_settings.clone(),
            in_reply_to: None,
            idempotency_key: None,
            delay_minutes: None,
        };
        let whole_readiness = publish_readiness(&provider, &preflight, registry, app_url);

        let limit = registry
            .get(&provider)
            .map_or(0, |p| p.max_content_length());
        let segments = content_splitter::split_content(&eff_content, &provider, limit);
        if segments.len() > 1 {
            warnings.push(format!(
                "Content split into {} posts for {} (limit: {} chars) — linked as a thread via group {group_id}",
                segments.len(),
                provider,
                limit
            ));
        }

        // If every split segment validates clean, whole-content failures
        // are split-solvable → warnings; anything else stays an error.
        let segments_clean = segments.iter().all(|seg| {
            publish_readiness(
                &provider,
                &PostContent {
                    content: seg.content.clone(),
                    media: media.clone(),
                    settings: eff_settings.clone(),
                    in_reply_to: None,
                    idempotency_key: None,
                    delay_minutes: None,
                },
                registry,
                app_url,
            )
            .is_empty()
        });
        let readiness = if segments_clean {
            for line in &whole_readiness {
                warnings.push(format!(
                    "{provider} (whole content, resolved by split): {line}"
                ));
            }
            vec![]
        } else {
            whole_readiness
        };

        // B4: every segment gets the campaign group + ordered sequence.
        // Media attaches to part 1 only (B15/X/IG attach once per thread).
        let parts: Vec<queries::ThreadPart> = segments
            .iter()
            .map(|seg| queries::ThreadPart {
                content: seg.content.clone(),
                media: if seg.sequence == 1 {
                    request.media.clone()
                } else {
                    serde_json::json!([])
                },
                settings: eff_settings.clone(),
                first_comment: if seg.sequence == 1 {
                    eff_first_comment.clone()
                } else {
                    None
                },
            })
            .collect();

        plans.push(IntegrationPlan {
            integration_id: *integration_id,
            provider: provider.clone(),
            parts,
            readiness,
        });
    }

    // ── B14: readiness errors reject the integration unless force.
    for plan in plans {
        let hard = !plan.readiness.is_empty();
        if hard && !request.force {
            for line in &plan.readiness {
                errors.push(format!("{} — rejected: {}", plan.provider, line));
            }
            rejected.push(plan.provider);
        } else {
            if hard {
                // force: keep the lines visible but stage anyway.
                for line in &plan.readiness {
                    errors.push(format!("{} — staged despite: {}", plan.provider, line));
                }
            }
            accepted.push(plan);
        }
    }

    if request.dry_run {
        let readiness: Vec<serde_json::Value> = accepted
            .iter()
            .map(|p| {
                serde_json::json!({
                    "integration_id": p.integration_id.to_string(),
                    "provider": p.provider,
                    "segments": p.parts.len(),
                    "ready": p.readiness.is_empty(),
                    "problems": p.readiness,
                })
            })
            .collect();
        // Dry run writes nothing; surface the report through warnings so
        // every caller (CLI/MCP/REST) sees it without a new wire type.
        warnings.push(
            serde_json::to_string(&serde_json::json!({
                "dry_run": true,
                "group_id": group_id.to_string(),
                "integrations": readiness,
            }))
            .unwrap_or_default(),
        );
        return Ok(StagingResult {
            staged: vec![],
            total_posts: 0,
            warnings,
            errors,
            rejected,
            group_id: None,
        });
    }

    if accepted.is_empty() {
        return Err(format!(
            "Every integration failed readiness: {}. Nothing was staged{}.",
            errors.join("; "),
            if request.force {
                " (even with force)"
            } else {
                " — pass force=true to stage anyway"
            }
        ));
    }

    // ── Write: ONE transaction for the whole stage (B4).
    let queued = request.scheduled_at.is_some() || request.next_slot;
    let state = if queued {
        crate::db::models::PostState::Queued
    } else {
        crate::db::models::PostState::Draft
    };
    let state_label = if queued { "queued" } else { "draft" };

    // E4: next-slot mode — each integration lands on ITS next free slot.
    // Resolved before the transaction opens: the pool holds one connection, so
    // a query made while `tx` is alive waits on the connection `tx` holds.
    let mut next_slots = std::collections::HashMap::new();
    if request.next_slot {
        for plan in &accepted {
            let slot = queries::find_next_free_slot(pool, user_id, Some(plan.integration_id))
                .await
                .map_err(|e| format!("Could not find a free slot for {}: {e}", plan.provider))?
                .unwrap_or_else(chrono::Utc::now);
            next_slots.insert(plan.integration_id, slot);
        }
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Begin failed: {e}"))?;
    let mut staged = Vec::new();
    for plan in accepted {
        // next-slot: the slot resolved above; explicit scheduled_at: one shared
        // time; draft: none.
        let plan_scheduled_at = if request.next_slot {
            next_slots.get(&plan.integration_id).copied()
        } else {
            request.scheduled_at
        };
        let posts = queries::insert_stage_tx(
            &mut tx,
            user_id,
            plan.integration_id,
            &plan.parts,
            plan_scheduled_at,
            state.clone(),
            group_id,
        )
        .await
        .map_err(|e| format!("Failed to create posts: {e}"))?;

        let total = plan.parts.len();
        for post in posts {
            staged.push(StagedPost {
                post_id: post.id,
                provider: plan.provider.clone(),
                sequence: post.sequence.max(1) as usize,
                total_segments: total,
                state: state_label.to_string(),
                group_id,
            });
        }
    }
    tx.commit()
        .await
        .map_err(|e| format!("Stage commit failed — nothing was written: {e}"))?;

    Ok(StagingResult {
        total_posts: staged.len(),
        staged,
        warnings,
        errors,
        rejected,
        group_id: Some(group_id),
    })
}

pub fn validate_staging_request(request: &StagingRequest) -> Result<(), String> {
    if request.content.trim().is_empty() {
        return Err("Content cannot be empty".into());
    }
    if request.integration_ids.is_empty() {
        return Err("At least one integration_id is required".into());
    }
    if request.content.len() > 100_000 {
        return Err("Content exceeds maximum length (100,000 chars)".into());
    }
    if !request.overrides.is_null() && !request.overrides.is_object() {
        return Err("overrides must be a JSON object keyed by provider".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries;
    use crate::social::registry::ProviderRegistry;
    use crate::social::{
        AuthToken, AuthUrlResponse, PageInfo, ProviderError, PublishResult, SocialProvider,
    };
    use std::sync::Arc;

    fn req(overrides: serde_json::Value, force: bool) -> StagingRequest {
        StagingRequest {
            content: "hello".into(),
            media: serde_json::json!([]),
            integration_ids: vec![Uuid::new_v4()],
            settings: serde_json::json!({}),
            scheduled_at: None,
            first_comment: None,
            overrides,
            dry_run: false,
            force,
            next_slot: false,
            group_id: None,
        }
    }

    #[test]
    fn overrides_settings_shallow_merge_base_wins_keys_not_in_over() {
        let merged = merge_settings(
            &serde_json::json!({"title": "base", "keep": 1}),
            &serde_json::json!({"title": "over", "added": true}),
        );
        assert_eq!(merged["title"], serde_json::json!("over"));
        assert_eq!(merged["keep"], serde_json::json!(1));
        assert_eq!(merged["added"], serde_json::json!(true));
    }

    #[test]
    fn overrides_must_be_an_object() {
        let err = validate_staging_request(&req(serde_json::json!(["x"]), false))
            .expect_err("array overrides must be rejected");
        assert!(err.contains("keyed by provider"), "got: {err}");
    }

    /// A provider mirroring Bluesky's splitter profile (max_content_length() = 300,
    /// thread-capable) so the real splitter produces multi-segment splits.
    struct SplitFake;

    #[async_trait::async_trait]
    impl SocialProvider for SplitFake {
        fn identifier(&self) -> &'static str {
            "bluesky"
        }
        fn name(&self) -> &'static str {
            "Bluesky"
        }
        fn scopes(&self) -> Vec<String> {
            vec![]
        }
        fn max_content_length(&self) -> usize {
            300
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
            Ok(PublishResult {
                platform_post_id: "split-1".into(),
                platform_post_url: None,
                status: "published".into(),
            })
        }
    }

    async fn test_pool() -> sqlx::SqlitePool {
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

    /// B4 regression: a stage split must produce ONE group_id and ordered
    /// sequences — not N unrelated rows. Pre-v2 the split rows had no
    /// group at all and went out as 3 arbitrary-order posts.
    #[tokio::test]
    async fn stage_split_produces_group_id_and_ordered_sequences() {
        let pool = test_pool().await;
        let user = queries::create_user(&pool, "stage@test.local", "hash", "Stage")
            .await
            .expect("user");
        let integration = queries::create_integration(
            &pool,
            user.id,
            "bluesky",
            "Bluesky",
            "acct-1",
            "access-token",
            None,
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
        let registry = ProviderRegistry::with_providers(vec![(
            "bluesky",
            Arc::new(SplitFake) as Arc<dyn SocialProvider>,
        )]);

        // 620 chars over Bluesky's 300-char limit → ≥3 thread segments.
        let long = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do ".repeat(10);
        let result = stage_post(
            &pool,
            &registry,
            "https://localhost:6544",
            user.id,
            StagingRequest {
                content: long,
                media: serde_json::json!([]),
                integration_ids: vec![integration.id],
                settings: serde_json::json!({}),
                scheduled_at: Some(chrono::Utc::now()),
                first_comment: None,
                overrides: serde_json::Value::Null,
                dry_run: false,
                force: false,
                next_slot: false,
                group_id: None,
            },
        )
        .await
        .expect("stage");

        assert!(
            result.staged.len() >= 2,
            "expected a split, got {}",
            result.staged.len()
        );
        let group = result.group_id.expect("campaign group_id returned");
        let mut last_seq = 0;
        for s in &result.staged {
            assert_eq!(s.group_id, group, "every split row shares the group");
            assert!(
                s.sequence > last_seq,
                "sequences must be ordered, got {} after {last_seq}",
                s.sequence
            );
            last_seq = s.sequence;
        }

        // The rows themselves carry the group + sequence.
        let rows: Vec<(Uuid, i32)> =
            sqlx::query_as("SELECT group_id, sequence FROM posts WHERE user_id = ?")
                .bind(user.id)
                .fetch_all(&pool)
                .await
                .expect("rows");
        assert_eq!(rows.len(), result.staged.len());
        for (gid, seq) in rows {
            assert_eq!(gid, group);
            assert!(seq >= 1);
        }
    }

    /// D2: a disabled integration is rejected even with force, matching publish.
    #[tokio::test]
    async fn stage_rejects_disabled_integration_even_when_forced() {
        let pool = test_pool().await;
        let user = queries::create_user(&pool, "disabled@test.local", "hash", "Disabled")
            .await
            .expect("user");
        let integration = queries::create_integration(
            &pool,
            user.id,
            "bluesky",
            "Bluesky",
            "acct-d",
            "access-token",
            None,
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
        sqlx::query("UPDATE integrations SET disabled = 1 WHERE id = ?")
            .bind(integration.id)
            .execute(&pool)
            .await
            .expect("disable");
        let registry = ProviderRegistry::with_providers(vec![(
            "bluesky",
            Arc::new(SplitFake) as Arc<dyn SocialProvider>,
        )]);

        let err = stage_post(
            &pool,
            &registry,
            "https://localhost:6544",
            user.id,
            StagingRequest {
                content: "hello".into(),
                media: serde_json::json!([]),
                integration_ids: vec![integration.id],
                settings: serde_json::json!({}),
                scheduled_at: Some(chrono::Utc::now()),
                first_comment: None,
                overrides: serde_json::Value::Null,
                dry_run: false,
                force: true,
                next_slot: false,
                group_id: None,
            },
        )
        .await
        .expect_err("a disabled integration must not stage, even with force");

        assert!(err.contains("integration is disabled"), "got: {err}");
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE user_id = ?")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(rows, 0, "no post row is written for a disabled integration");
    }

    /// E4: schedule="next-slot" queues each integration at ITS own next
    /// free slot — never a draft, never a missing scheduled_at.
    #[tokio::test]
    async fn next_slot_stages_queued_with_a_scheduled_time() {
        let pool = test_pool().await;
        let user = queries::create_user(&pool, "slot@test.local", "hash", "Slot")
            .await
            .expect("user");
        let integration = queries::create_integration(
            &pool,
            user.id,
            "bluesky",
            "Bluesky",
            "acct-1",
            "access-token",
            None,
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
        let registry = ProviderRegistry::with_providers(vec![(
            "bluesky",
            Arc::new(SplitFake) as Arc<dyn SocialProvider>,
        )]);

        let result = stage_post(
            &pool,
            &registry,
            "https://localhost:6544",
            user.id,
            StagingRequest {
                content: "slot me".into(),
                media: serde_json::json!([]),
                integration_ids: vec![integration.id],
                settings: serde_json::json!({}),
                scheduled_at: None,
                next_slot: true,
                first_comment: None,
                overrides: serde_json::Value::Null,
                dry_run: false,
                force: false,
                group_id: None,
            },
        )
        .await
        .expect("stage");

        assert_eq!(result.staged[0].state, "queued", "next-slot must queue");
        let (state, scheduled): (String, Option<i64>) =
            sqlx::query_as("SELECT state, scheduled_at FROM posts WHERE user_id = ?")
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "queued");
        assert!(
            scheduled.is_some(),
            "next-slot must write a concrete scheduled_at"
        );
    }
}
