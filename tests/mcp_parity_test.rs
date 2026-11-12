// ─── MCP parity tools (campaigns, sets, RSS, streak, resolve) ───
// The new MCP tools call the REST handlers in-process. These tests drive the
// MCP handler functions against a real (isolated) database and check the
// round trip and the argument validation.
//
// Run: cargo test --test mcp_parity_test
// Uses the scratch DB from `load_dotenv_for_tests` (never data/switchpost.db).

use switchpost::api::AppState;
use switchpost::config::Config;
use switchpost::db;
use switchpost::mcp::{tools_analytics, tools_campaigns, tools_rss, tools_sets};
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;

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

fn campaign_create(name: &str) -> tools_campaigns::CampaignCreateInput {
    tools_campaigns::CampaignCreateInput {
        name: name.into(),
        description: None,
        color: None,
        start_date: None,
        end_date: None,
        goal: None,
        status: None,
        progress_metric: None,
        progress_target: None,
        audience_persona: None,
        content_pillars: None,
        budget_cents: None,
        kpi_targets: None,
    }
}

fn campaign_id(v: &serde_json::Value) -> String {
    v["id"].as_str().expect("campaign id").to_string()
}

#[tokio::test]
async fn campaign_round_trip_through_mcp_handlers() {
    let state = state().await;
    let name = format!("parity-{}", uuid::Uuid::new_v4());

    let created = tools_campaigns::handle_campaign_create(&state, &campaign_create(&name))
        .await
        .expect("create");
    let id = campaign_id(&created.0);

    let listed =
        tools_campaigns::handle_campaign_list(&state, &tools_campaigns::CampaignListInput {})
            .await
            .expect("list");
    let rows = listed.0.as_array().expect("list is an array");
    assert!(
        rows.iter().any(|c| c["name"] == name.as_str()),
        "created campaign must appear in the list"
    );

    let update = tools_campaigns::CampaignUpdateInput {
        campaign_id: id.clone(),
        name: None,
        description: None,
        color: None,
        start_date: None,
        end_date: None,
        goal: None,
        status: Some("paused".into()),
        progress_metric: None,
        progress_target: None,
        audience_persona: None,
        content_pillars: None,
        budget_cents: None,
        kpi_targets: None,
    };
    let updated = tools_campaigns::handle_campaign_update(&state, &update)
        .await
        .expect("update");
    assert_eq!(updated.0["status"], "paused");
    assert_eq!(
        updated.0["name"],
        name.as_str(),
        "omitted fields keep their value"
    );

    tools_campaigns::handle_campaign_delete(
        &state,
        &tools_campaigns::CampaignDeleteInput { campaign_id: id },
    )
    .await
    .expect("delete");
}

#[tokio::test]
async fn parity_tools_reject_malformed_ids_before_touching_the_db() {
    let state = state().await;

    let err = tools_campaigns::handle_campaign_update(
        &state,
        &tools_campaigns::CampaignUpdateInput {
            campaign_id: "not-a-uuid".into(),
            name: None,
            description: None,
            color: None,
            start_date: None,
            end_date: None,
            goal: None,
            status: None,
            progress_metric: None,
            progress_target: None,
            audience_persona: None,
            content_pillars: None,
            budget_cents: None,
            kpi_targets: None,
        },
    )
    .await
    .err()
    .expect("bad campaign id");
    assert!(err.contains("campaign_id must be a UUID"), "got: {err}");

    let err = tools_sets::handle_set_create(
        &state,
        &tools_sets::SetCreateInput {
            name: "t".into(),
            description: None,
            content: serde_json::json!({}),
            channel_ids: vec!["nope".into()],
        },
    )
    .await
    .err()
    .expect("bad channel id");
    assert!(err.contains("channel_ids must be a UUID"), "got: {err}");

    let err = tools_rss::handle_rss_feed_items(
        &state,
        &tools_rss::RssFeedIdInput {
            feed_id: "nope".into(),
        },
    )
    .await
    .err()
    .expect("bad feed id");
    assert!(err.contains("feed_id must be a UUID"), "got: {err}");
}

#[tokio::test]
async fn streak_tool_reports_the_streak_fields() {
    let state = state().await;
    let out =
        tools_analytics::handle_streak_get(&state, &tools_analytics::StreakGetInput { tz: None })
            .await
            .expect("streak");
    assert!(
        out.0["streak_days"].is_number(),
        "streak_days present: {}",
        out.0
    );
    assert!(out.0.get("streak_since").is_some(), "streak_since present");
}

// Regression: these queries used Postgres-only SQL (`NOW()`, `make_interval`)
// and failed at runtime on SQLite. They must execute against the real schema.
#[tokio::test]
async fn admin_analytics_and_integration_toggle_run_on_sqlite() {
    use switchpost::mcp::tools_admin;

    let state = state().await;

    let summary = tools_admin::handle_analytics_summary(
        &state,
        &tools_admin::AnalyticsSummaryInput { days: Some(7) },
    )
    .await;
    assert!(
        summary.is_ok(),
        "analytics summary failed: {:?}",
        summary.err()
    );

    let feed = tools_admin::handle_feed_analytics(
        &state,
        &tools_admin::FeedAnalyticsInput { days: Some(7) },
    )
    .await;
    assert!(feed.is_ok(), "feed analytics failed: {:?}", feed.err());

    // An unknown integration id is a clean "no row" result, not a SQL error.
    let matched = db::queries::set_integration_disabled(
        &state.db,
        switchpost::auth::middleware::DEFAULT_USER_ID,
        uuid::Uuid::new_v4(),
        true,
    )
    .await
    .expect("toggle SQL runs on sqlite");
    assert!(!matched);
}

// ─── The MCP writes must not be the lossy copy ────────────────
//
// The MCP tools and the REST endpoints expose the SAME operations to
// the same founder - an agent driving the MCP server and a browser
// driving the dashboard are two clients of one application. Where they
// diverged, the MCP copy was the lossy one:
//
//   1. `posts_update` WIPED the post. The service took a two-state
//      input and wrote `SET media = ?, settings = ?` unconditionally, so
//      an agent that sent only a new body erased every attached image
//      and every provider setting (the Reddit subreddit, the Pinterest
//      board). The post then failed at publish time with "Missing
//      subreddit in settings", and the founder had no way to know why.
//
//   2. `campaign_update` could never CLEAR a field. The MCP input was
//      two-state and mapped with `.map(Some)`, which turned "clear
//      this" into "leave this alone": `campaign_update {goal: null}`
//      kept the goal and reported success.
//
//   3. The automation tools' UPDATE and DELETE had no `user_id`
//      predicate, so an agent could edit another user's rule.
//
// These tests call the MCP tool functions directly and read the row
// back, so they exercise the real write.

use switchpost::auth::middleware::DEFAULT_USER_ID;
use switchpost::mcp::{tools_automation, tools_posts};
use uuid::Uuid;

async fn seed_integration(state: &AppState) -> Uuid {
    let integration_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO integrations (id, user_id, provider_identifier, provider_name, internal_id)
         VALUES (?, ?, 'bluesky', 'Bluesky', ?)",
    )
    .bind(integration_id)
    .bind(DEFAULT_USER_ID)
    .bind(format!("did:plc:{integration_id}"))
    .execute(&state.db)
    .await
    .expect("seed integration");
    integration_id
}

/// A draft post with a title, an attachment and a provider setting.
async fn seed_post(state: &AppState, integration_id: Uuid) -> Uuid {
    let post_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO posts (id, user_id, integration_id, state, content, title, media, settings,
                            idempotency_key)
         VALUES (?, ?, ?, 'draft', 'original body', 'original title', ?, ?, ?)",
    )
    .bind(post_id)
    .bind(DEFAULT_USER_ID)
    .bind(integration_id)
    .bind(serde_json::json!([{ "url": "/api/media/abc", "mime_type": "image/png" }]))
    .bind(serde_json::json!({ "subreddit": "rust" }))
    // `idempotency_key` is a Uuid: sqlx stores it as a 16-byte BLOB, so
    // it is bound as the raw bytes, not as the dashed text.
    .bind(Uuid::new_v4().as_bytes().to_vec())
    .execute(&state.db)
    .await
    .expect("seed post");
    post_id
}

async fn post_fields(
    state: &AppState,
    id: Uuid,
) -> (String, Option<String>, serde_json::Value, serde_json::Value) {
    sqlx::query_as::<_, (String, Option<String>, serde_json::Value, serde_json::Value)>(
        "SELECT content, title, media, settings FROM posts WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .expect("post row")
}

#[tokio::test]
async fn mcp_posts_update_keeps_the_media_and_the_settings_it_was_not_asked_to_change() {
    let state = state().await;
    let integration_id = seed_integration(&state).await;
    let post_id = seed_post(&state, integration_id).await;

    // The agent fixes a typo in the body. Nothing else.
    let input = tools_posts::UpdatePostInput {
        id: post_id.to_string(),
        content: Some("the new body".to_string()),
        title: None,
        media: None,
        settings: None,
        // The campaign is not mentioned by these parity checks, so the
        // stored one is kept: absent = keep.
        campaign_id: None,
    };
    tools_posts::update_post(&state, &input)
        .await
        .expect("update");

    let (content, title, media, settings) = post_fields(&state, post_id).await;
    assert_eq!(content, "the new body", "the body must change");
    assert_eq!(
        title,
        Some("original title".into()),
        "the title must survive"
    );
    assert_eq!(
        media.as_array().map(|a| a.len()),
        Some(1),
        "the attached image must survive: {media}"
    );
    assert_eq!(
        settings.get("subreddit").and_then(|v| v.as_str()),
        Some("rust"),
        "the provider setting must survive: {settings}"
    );
}

#[tokio::test]
async fn mcp_posts_update_can_clear_a_field_on_purpose() {
    let state = state().await;
    let integration_id = seed_integration(&state).await;
    let post_id = seed_post(&state, integration_id).await;

    // The agent clears the title explicitly.
    let input = tools_posts::UpdatePostInput {
        id: post_id.to_string(),
        content: None,
        title: Some(None),
        media: None,
        settings: None,
        // The campaign is not mentioned by these parity checks, so the
        // stored one is kept: absent = keep.
        campaign_id: None,
    };
    tools_posts::update_post(&state, &input)
        .await
        .expect("update");

    let (_, title, _, _) = post_fields(&state, post_id).await;
    assert_eq!(title, None, "an explicit null must clear the title");
}

#[tokio::test]
async fn mcp_campaign_update_can_clear_a_field_on_purpose() {
    let state = state().await;
    let campaign_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO campaigns (id, user_id, name, goal, status, created_at, updated_at)
         VALUES (?, ?, 'Q3', 'Reach 10k people', 'active', unixepoch(), unixepoch())",
    )
    .bind(campaign_id)
    .bind(DEFAULT_USER_ID)
    .execute(&state.db)
    .await
    .expect("seed campaign");

    // The agent clears the goal explicitly.
    let input = tools_campaigns::CampaignUpdateInput {
        campaign_id: campaign_id.to_string(),
        name: None,
        description: None,
        color: None,
        start_date: None,
        end_date: None,
        goal: Some(None),
        status: None,
        progress_metric: None,
        progress_target: None,
        audience_persona: None,
        content_pillars: None,
        budget_cents: None,
        kpi_targets: None,
    };
    tools_campaigns::handle_campaign_update(&state, &input)
        .await
        .expect("update");

    let goal: Option<String> = sqlx::query_scalar("SELECT goal FROM campaigns WHERE id = ?")
        .bind(campaign_id)
        .fetch_one(&state.db)
        .await
        .expect("campaign row");
    assert_eq!(
        goal, None,
        "an explicit null must clear the goal, not leave it alone"
    );
}

#[tokio::test]
async fn mcp_campaign_update_keeps_a_field_it_was_not_asked_to_change() {
    let state = state().await;
    let campaign_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO campaigns (id, user_id, name, goal, status, created_at, updated_at)
         VALUES (?, ?, 'Q3', 'Reach 10k people', 'active', unixepoch(), unixepoch())",
    )
    .bind(campaign_id)
    .bind(DEFAULT_USER_ID)
    .execute(&state.db)
    .await
    .expect("seed campaign");

    // The agent renames the campaign. The goal must survive.
    let input = tools_campaigns::CampaignUpdateInput {
        campaign_id: campaign_id.to_string(),
        name: Some("Q3 (revised)".to_string()),
        description: None,
        color: None,
        start_date: None,
        end_date: None,
        goal: None,
        status: None,
        progress_metric: None,
        progress_target: None,
        audience_persona: None,
        content_pillars: None,
        budget_cents: None,
        kpi_targets: None,
    };
    tools_campaigns::handle_campaign_update(&state, &input)
        .await
        .expect("update");

    let goal: Option<String> = sqlx::query_scalar("SELECT goal FROM campaigns WHERE id = ?")
        .bind(campaign_id)
        .fetch_one(&state.db)
        .await
        .expect("campaign row");
    assert_eq!(
        goal,
        Some("Reach 10k people".into()),
        "an omitted field must keep its stored value"
    );
}

#[tokio::test]
async fn mcp_automation_update_refuses_a_rule_it_does_not_own() {
    let state = state().await;
    // A rule owned by a DIFFERENT user.
    let other_user = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, password, created_at) VALUES (?, ?, 'x', unixepoch())",
    )
    .bind(other_user)
    .bind(format!("{other_user}@example.test"))
    .execute(&state.db)
    .await
    .expect("seed user");
    let rule_id = Uuid::new_v4();
    let integration_id = seed_integration(&state).await;
    sqlx::query(
        "INSERT INTO automation_rules (id, user_id, integration_id, name, trigger_type,
                                      trigger_filter, response_template,
                                      response_type, is_active, created_at, updated_at)
         VALUES (?, ?, ?, 'Someone else''s rule', 'comment', '{}', 'hi', 'template', 1,
                 unixepoch(), unixepoch())",
    )
    .bind(rule_id)
    .bind(other_user)
    .bind(integration_id)
    .execute(&state.db)
    .await
    .expect("seed rule");

    let input = tools_automation::UpdateRuleInput {
        rule_id: rule_id.to_string(),
        name: Some("hijacked".to_string()),
        trigger_filter: None,
        response_template: None,
        response_type: None,
        ai_model: None,
        is_active: None,
        cooldown_minutes: None,
        max_responses_per_hour: None,
    };
    let result = tools_automation::update_rule(&state, &input).await;
    assert!(
        result.is_err(),
        "an update must not touch another user's rule"
    );

    let name: String = sqlx::query_scalar("SELECT name FROM automation_rules WHERE id = ?")
        .bind(rule_id)
        .fetch_one(&state.db)
        .await
        .expect("rule row");
    assert_eq!(name, "Someone else's rule", "the rule must be untouched");
}

// ─── The CLI/MCP parity claim, measured ────────────────────────
//
// AGENTS §4 requires the CLI, REST and MCP surfaces to have parity for core
// operations. The MCP side is 389 tools; the CLI side is ~60 commands
// grouped by resource. 380 of the 389 tools are platform-specific
// operations that have no CLI verb by design (the CLI is grouped by
// command, not by tool).
//
// The claim that can be MEASURED is the cross-platform core: every
// lifecycle family the MCP `core` toolset exposes must be reachable from
// the shell. It was never measured — so a family that lost its CLI command
// would silently break the parity claim.
//
// This test walks the CLI's own command tree (clap's `Command`), so it
// cannot drift from the CLI as shipped.

use clap::CommandFactory;
use switchpost::cli::Cli;

/// The top-level command names, as clap parses them (kebab-cased).
fn cli_commands() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect()
}

#[test]
fn every_core_mcp_family_has_a_cli_command() {
    let commands = cli_commands();
    // family -> the CLI command(s) that serve it.
    let required: [(&str, &[&str]); 18] = [
        ("posts", &["posts", "post", "stage", "carousel"]),
        ("media", &["media"]),
        ("tags", &["tags"]),
        ("sets", &["sets"]),
        ("campaigns", &["campaigns"]),
        ("signatures", &["signatures"]),
        ("webhooks", &["webhooks"]),
        ("rss", &["rss"]),
        ("feed", &["feed"]),
        ("integrations", &["integrations", "connect"]),
        ("notifications", &["notifications"]),
        ("analytics", &["analytics", "get"]),
        // The calendar has no CLI family of its own: its WRITE path is
        // `posts schedule` and its read path is the calendar endpoint.
        ("calendar", &["posts"]),
        ("comments", &["comment"]),
        ("dms", &["dm"]),
        ("automation", &["automation"]),
        ("ai", &["ai"]),
        ("streak", &["streak"]),
    ];
    for (family, aliases) in required {
        assert!(
            aliases.iter().any(|a| commands.iter().any(|c| c == a)),
            "the `{family}` family is in the MCP core toolset but has no CLI command \
             (looked for {aliases:?}, have {commands:?})"
        );
    }
}

#[test]
fn the_sets_command_has_all_three_verbs() {
    // `sets` was the hole this measurement found: the MCP has create / list /
    // / delete in its CORE toolset and the CLI had no verb at all.
    let commands = cli_commands();
    assert!(
        commands.contains(&"sets".to_string()),
        "the sets family must exist (have {commands:?})"
    );
    let sets = Cli::command()
        .get_subcommands()
        .find(|c| c.get_name() == "sets")
        .expect("the sets command")
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect::<Vec<_>>();
    for verb in ["list", "create", "delete"] {
        assert!(
            sets.contains(&verb.to_string()),
            "the sets command must have `{verb}` (has {sets:?})"
        );
    }
}

#[test]
fn every_registered_provider_is_reachable_from_the_shell() {
    // The CLI is one family per platform (`x`, `reddit`, …), and the
    // registry's providers are the ones the app can talk to. A provider
    // with no CLI family is still reachable through the unified
    // `post --platforms <identifier>` path (exact identifier match), so
    // THAT is what must hold.
    let commands = cli_commands();
    // The providers the app REGISTERS BY DEFAULT. Farcaster is opt-in
    // (`ENABLE_ARCHIVE_PROVIDERS`), so it is not in the registry on a
    // normal boot — and the CLI shim is not built for it either.
    switchpost::config::load_dotenv_for_tests();
    let config = switchpost::config::Config::from_env().expect("config");
    let registry = switchpost::social::registry::ProviderRegistry::new(&config, None, None);
    let providers: Vec<&'static str> = registry.list();
    let mut unreachable: Vec<String> = Vec::new();
    for p in &providers {
        let clap_name = p.replace('_', "-");
        let has_family = commands.contains(&clap_name);
        // The CLI names a few families differently from the registry
        // identifier. These are RENAMES, not gaps.
        let renamed = matches!(
            *p,
            "drive" | "gmail" | "calendar" // gdrive / gmail-ops / gcal
        ) || (*p == "medium" && commands.contains(&"medium-blog".to_string()));
        // These two have NO CLI family of their own. They are still
        // reachable — through the unified `post --platforms <identifier>`
        // path, which takes the registry identifier verbatim (the next
        // test proves it for every identifier). The list is here so a
        // THIRD provider without a family breaks the test and has to be
        // a decision, not an accident.
        const UNIFIED_PATH_ONLY: [&str; 2] = ["instagram-standalone", "google_my_business"];
        if !has_family && !renamed && !UNIFIED_PATH_ONLY.contains(p) {
            unreachable.push(format!("{p} (no `{clap_name}` command)"));
        }
    }
    assert!(
        unreachable.is_empty(),
        "these providers are registered but have no CLI command and no renamed family: \
         {unreachable:?} (have {commands:?})"
    );
}

#[test]
fn the_unified_post_command_accepts_an_exact_provider_identifier() {
    // This is the path every provider without its own CLI family rides on:
    // `--platforms` takes the registry identifier verbatim.
    //
    // The provider list is what the registry knows, so the test is a
    // property of the resolver, not of a list kept in sync by hand.
    use switchpost::cli::run::resolve_platform_integrations;
    for identifier in switchpost::social::tier::all_known_identifiers() {
        let integration = make_integration(identifier);
        let matched = resolve_platform_integrations(std::iter::once(&integration), identifier);
        assert_eq!(
            matched,
            vec![integration.id],
            "`--platforms {identifier}` must reach the {identifier} integration"
        );
    }
}

/// An integration row for a provider identifier, for the resolver tests.
fn make_integration(identifier: &str) -> switchpost::db::models::Integration {
    switchpost::db::models::Integration {
        id: uuid::Uuid::new_v4(),
        user_id: switchpost::auth::middleware::DEFAULT_USER_ID,
        provider_identifier: identifier.to_string(),
        provider_name: identifier.to_string(),
        internal_id: "me".to_string(),
        access_token: "tok".to_string(),
        refresh_token: None,
        token_expires_at: None,
        profile_name: None,
        profile_picture: None,
        profile_url: None,
        disabled: false,
        refresh_needed: false,
        root_internal_id: None,
        posting_times: serde_json::json!([]),
        auth_method: "oauth".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}
