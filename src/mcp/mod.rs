// ─── MCP Server ───────────────────────────────────────────────
// Model Context Protocol server exposing all Switchpost operations as tools.
// Designed for AI agents to schedule, manage, and monitor posts.
//
// Uses rmcp crate with ServerHandler + #[tool_router] pattern.
// Same business logic as the REST API — shared via AppState.

use rmcp::{
    handler::server::wrapper::Parameters, schemars::JsonSchema, tool, tool_handler, tool_router,
    transport::stdio, Json, ServiceExt,
};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

pub mod auth; // pub: tests/resolve_local_user_test.rs drives resolve_first_user (the ~80-site seam)
pub(crate) mod rest_bridge;
pub mod tools_admin;
pub mod tools_ai;
pub mod tools_analytics;
pub mod tools_automation;
pub mod tools_bluesky;
pub mod tools_calendar; // tests drive the loud-refusal contract directly (9.21 parity)
pub mod tools_campaigns;
pub mod tools_comments;
pub mod tools_devto;
pub mod tools_discord;
pub mod tools_dm;
pub mod tools_facebook;
pub mod tools_feed;
pub mod tools_github;
pub mod tools_google;
pub mod tools_hashnode;
pub mod tools_inbox;
pub mod tools_instagram;
pub mod tools_instagram_standalone;
pub mod tools_integrations;
pub mod tools_linkedin;
pub mod tools_linkedin_page;
pub mod tools_mastodon;
pub mod tools_media;
pub mod tools_medium;
pub mod tools_notifications;
pub mod tools_pinterest;
pub mod tools_posts;
pub mod tools_reddit;
pub mod tools_rss;
pub mod tools_sets;
pub mod tools_setup;
pub mod tools_skool;
pub mod tools_slack;
pub mod tools_tags;
pub mod tools_telegram_bot;
pub mod tools_telegram_user;
pub mod tools_threads;
pub mod tools_tiktok;
pub mod tools_webhooks;
pub mod tools_whatsapp;
pub mod tools_wordpress;
pub mod tools_x;
pub mod tools_youtube;

// ══════════════════════════════════════════════════════════════
// SHARED TYPES
// ══════════════════════════════════════════════════════════════

/// MCP-compatible wrapper around serde_json::Value for tool output.
/// The MCP spec requires outputSchema to have root type \"object\",
/// but schemars generates `true` (any-value schema) for serde_json::Value.
/// This wrapper provides a valid `{\"type\": \"object\"}` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpJsonValue(pub serde_json::Value);

impl McpJsonValue {
    pub fn value(self) -> serde_json::Value {
        self.0
    }
}

impl JsonSchema for McpJsonValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("OutputObject")
    }
    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        serde_json::from_value(serde_json::json!({"type": "object"}))
            .expect("OutputObject schema literal is valid JSON Schema")
    }
}

// ══════════════════════════════════════════════════════════════
// TOOL ROUTER
// ══════════════════════════════════════════════════════════════

// §5.5: input structs for the meta tools.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ToolsSearchInput {
    /// Case-insensitive substring matched against tool names and descriptions.
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ToolsEnableInput {
    /// Toolsets to enable for this session: platform names (x, reddit,
    /// linkedin, facebook, instagram, threads, tiktok, pinterest, youtube,
    /// bluesky, mastodon, telegram, whatsapp, slack, discord, skool,
    /// wordpress, medium, devto, hashnode, github, google) or "all".
    pub sets: Vec<String>,
}

/// std Mutex isn't Clone; the cheap answer for a per-session set is Arc —
/// clones share the SAME session state, which is what thin-mode and
/// mcp-call both want (one session, one set).
#[derive(Clone, Default)]
pub struct Toolsets(std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>);

impl Toolsets {
    pub fn new(sets: std::collections::HashSet<String>) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(sets)))
    }

    pub fn get(&self) -> std::collections::HashSet<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn insert(&self, set: String) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(set);
    }
}

#[derive(Clone)]
pub struct SwitchpostMcpServer {
    pub state: AppState,
    /// §5.5: enabled toolsets for THIS session. Boot default comes from
    /// SWITCHPOST_TOOLSETS (default `core`); `tools_enable` adds sets at
    /// runtime. Guards the per-session prompt cost: an agent only advertises
    /// the toolsets it works with instead of all 329 schemas.
    pub toolsets: Toolsets,
}

/// The parameter type for a tool that takes NO arguments.
///
/// rmcp's `Parameters<P>` extractor does `arguments.take().unwrap_or_default()`
/// — an absent argument map becomes `{}` — and then deserializes it as
/// `P`. `P = ()` cannot be built from a map: serde answers "invalid type:
/// map, expected unit", so every such tool failed in EVERY MCP client
/// (stdio, and the CLI's `mcp-call`) with a params error.
///
/// This type accepts the empty map (and any map), so a no-arg tool
/// actually runs.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct EmptyParams {
    #[serde(flatten)]
    #[schemars(skip)]
    _ignored: std::collections::BTreeMap<String, serde_json::Value>,
}

// Helper: get DB pool from state

#[tool_router]
impl SwitchpostMcpServer {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            toolsets: Toolsets::new(Self::boot_toolsets()),
        }
    }

    /// §5.5: SWITCHPOST_TOOLSETS (comma-separated). Default: `core`
    /// (posts/stage/media/tags/notifications/setup/integrations/analytics/
    /// feed + the meta tools). `all` disables filtering.
    fn boot_toolsets() -> std::collections::HashSet<String> {
        std::env::var("SWITCHPOST_TOOLSETS")
            .unwrap_or_else(|_| "core".into())
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// §5.5: the meta tools are never filtered - an agent must always be
    /// able to search and enable more toolsets.
    pub(crate) const META_TOOLS: [&str; 2] = ["tools_search", "tools_enable"];

    /// §5.5: which set a tool belongs to. Providers map to their platform
    /// name; cross-platform lifecycle tools map to `core`.
    pub fn toolset_of(name: &str) -> String {
        match name {
            "get_comments"
            | "reply_to_comment"
            | "send_dm"
            | "get_dm_messages"
            | "list_dm_conversations" => "core".into(),
            _ => match name.split('_').next().unwrap_or("") {
                "posts" | "stage" | "media" | "tag" | "notif" | "setup" | "integrations"
                | "signatures" | "calendar" | "analytics" | "feed" | "auth" | "inbox" => {
                    "core".into()
                }
                "gh" => "github".into(),
                "li" | "lip" => "linkedin".into(),
                "fb" => "facebook".into(),
                "ig" | "ias" => "instagram".into(),
                "tb" | "tu" => "telegram".into(),
                "di" => "discord".into(),
                "ms" => "mastodon".into(),
                "bs" => "bluesky".into(),
                "yt" => "youtube".into(),
                "sk" => "skool".into(),
                "wa" => "whatsapp".into(),
                "wh" => "webhooks".into(),
                "sl" => "slack".into(),
                // x, reddit, th(reads), tt, pi, wp, md, dv, hn + anything
                // unknown map to their own prefix name.
                other => other.into(),
            },
        }
    }

    /// §5.5: the ONE membership rule for a toolset filter. The session
    /// router, `tools_search` and the CLI's `mcp-tools --toolset` all
    /// use it, so a shell listing and a live session advertise the same
    /// tools.
    ///
    ///   - `all` means NO filtering;
    ///   - the meta tools are always present (an agent must always be
    ///     able to search and enable more toolsets);
    ///   - otherwise the tool's own set must match.
    pub fn tool_in_toolset(tool_name: &str, set: &str) -> bool {
        let set = set.trim().to_ascii_lowercase();
        if set == "all" {
            return true;
        }
        if Self::META_TOOLS.contains(&tool_name) {
            return true;
        }
        Self::toolset_of(tool_name) == set
    }

    /// The router this session advertises: every tool whose set is not
    /// enabled is hidden (list) and rejected (call). Meta tools stay up.
    pub(crate) fn filtered_tool_router(
        &self,
    ) -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        let mut router = Self::tool_router();
        let enabled = self.toolsets.get();
        if enabled.contains("all") {
            return router;
        }
        let names: Vec<String> = router
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        for name in names {
            // Meta tools are never filtered — an agent must always be able
            // to search and enable more toolsets.
            if Self::META_TOOLS.contains(&name.as_str()) {
                continue;
            }
            let set = Self::toolset_of(&name);
            if !enabled.contains(&set) {
                router.disable_route(name);
            }
        }
        router
    }

    // ── Auth Tools ──────────────────────────────────────────
    //
    // Switchpost is a single-user local-deployment app — there is
    // no register/login flow exposed over MCP. The WebUI is gated by
    // a single password (`APP_PASSWORD` env var); MCP stdio is local
    // (shell access already implies trust) and bypasses the gate.
    // This single tool lets an AI agent confirm the server is up
    // and identify the local user.

    #[tool(
        description = "Return the local user id. Single-user mode — no login required over MCP."
    )]
    async fn auth_status(&self) -> Result<Json<McpJsonValue>, String> {
        Ok(Json(McpJsonValue(serde_json::json!({
            "user_id": crate::auth::middleware::DEFAULT_USER_ID.to_string(),
            "mode": "single_user",
            "note": "MCP runs locally over stdio; the WebUI is gated by APP_PASSWORD."
        }))))
    }

    // ── Calendar Tools ──────────────────────────────────────

    #[tool(
        description = "Get posts grouped by LOCAL day for a date range, matching the web calendar. start/end are RFC3339; tz is the founder's IANA zone (e.g. America/New_York) so day keys match the calendar they see, DST-correct. Legacy: tz_offset in seconds."
    )]
    async fn calendar_get(
        &self,
        params: Parameters<tools_calendar::CalendarInput>,
    ) -> Result<Json<tools_calendar::CalendarOutput>, String> {
        tools_calendar::get_calendar(&self.state, &params.0).await
    }

    // ── Integration Tools ────────────────────────────────────

    #[tool(description = "List all available social media providers that can be connected.")]
    async fn integrations_list_providers(
        &self,
        params: Parameters<tools_integrations::ListProvidersInput>,
    ) -> Result<Json<tools_integrations::ListProvidersOutput>, String> {
        tools_integrations::list_providers(&self.state, &params.0).await
    }

    #[tool(
        description = "List all connected social media integrations for the authenticated user."
    )]
    async fn integrations_list(
        &self,
        params: Parameters<tools_integrations::ListIntegrationsInput>,
    ) -> Result<Json<tools_integrations::ListIntegrationsOutput>, String> {
        tools_integrations::list_integrations(&self.state, &params.0).await
    }

    #[tool(description = "Get OAuth URL to connect a social media channel.")]
    async fn integrations_connect(
        &self,
        params: Parameters<tools_integrations::ConnectInput>,
    ) -> Result<Json<tools_integrations::ConnectOutput>, String> {
        tools_integrations::connect_integration(&self.state, &params.0).await
    }

    #[tool(
        description = "Complete OAuth connection after authorizing in browser. Pass code and state from the callback URL."
    )]
    async fn integrations_connect_complete(
        &self,
        params: Parameters<tools_integrations::ConnectCompleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_integrations::complete_connect_integration(&self.state, &params.0).await
    }

    #[tool(description = "Disconnect/remove a social media integration by its ID.")]
    async fn integrations_disconnect(
        &self,
        params: Parameters<tools_integrations::DisconnectInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_integrations::disconnect_integration(&self.state, &params.0).await
    }

    #[tool(
        description = "List discoverable posting targets (channels, groups, subreddits, peers) for a connected integration."
    )]
    async fn integrations_list_targets(
        &self,
        params: Parameters<tools_integrations::ListTargetsInput>,
    ) -> Result<Json<tools_integrations::ListTargetsOutput>, String> {
        tools_integrations::list_targets(&self.state, &params.0).await
    }

    // ── Setup/Onboarding Tools ─────────────────────────────────

    #[tool(
        description = "Check overall setup status: database, user, connected providers, and next actions for an AI-agent guided setup."
    )]
    async fn setup_status(
        &self,
        _params: Parameters<tools_setup::SetupStatusInput>,
    ) -> Result<Json<tools_setup::SetupStatusOutput>, String> {
        tools_setup::setup_status(&self.state, &tools_setup::SetupStatusInput {}).await
    }

    #[tool(
        description = "Import browser cookies for X/Twitter and/or Reddit. Auto-detects Chrome, Brave, Firefox, Zen. Use provider='all' to import both."
    )]
    async fn setup_import_cookies(
        &self,
        params: Parameters<tools_setup::ImportCookiesInput>,
    ) -> Result<Json<tools_setup::ImportCookiesOutput>, String> {
        tools_setup::import_cookies(&self.state, &params.0).await
    }

    #[tool(
        description = "Get setup guidance for connecting social media providers. Returns what credentials are needed, where to get them, and how to configure them. Call without provider to see all providers."
    )]
    async fn setup_guide(
        &self,
        params: Parameters<tools_setup::SetupGuideInput>,
    ) -> Result<Json<tools_setup::SetupGuideOutput>, String> {
        tools_setup::setup_guide(&self.state, &params.0)
    }

    #[tool(
        description = "Set a configuration value in ~/.switchpost/.env. Creates file if needed. Restart switchpost after setting."
    )]
    async fn setup_config_set(
        &self,
        params: Parameters<tools_setup::ConfigSetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_setup::config_set(&self.state, &params.0).map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a configuration value (redacts secrets)")]
    async fn setup_config_get(
        &self,
        params: Parameters<tools_setup::ConfigGetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_setup::config_get(&self.state, &params.0).map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all configuration entries in ~/.switchpost/.env.")]
    async fn setup_config_list(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<Json<tools_setup::ConfigListOutput>, String> {
        tools_setup::config_list(&self.state)
    }

    // ── Post Tools ───────────────────────────────────────────

    #[tool(description = "Create a new post. Set scheduled_at to publish later, omit for draft.")]
    async fn posts_create(
        &self,
        params: Parameters<tools_posts::CreatePostInput>,
    ) -> Result<Json<tools_posts::CreatePostOutput>, String> {
        tools_posts::create_post(&self.state, &params.0).await
    }

    #[tool(description = "List posts with optional state filter (draft|queued|published|error).")]
    async fn posts_list(
        &self,
        params: Parameters<tools_posts::ListPostsInput>,
    ) -> Result<Json<tools_posts::ListPostsOutput>, String> {
        tools_posts::list_posts(&self.state, &params.0).await
    }

    #[tool(description = "Get a single post by ID.")]
    async fn posts_get(
        &self,
        params: Parameters<tools_posts::GetPostInput>,
    ) -> Result<Json<tools_posts::GetPostOutput>, String> {
        tools_posts::get_post(&self.state, &params.0).await
    }

    #[tool(description = "Schedule a post for publishing at a specific time (RFC3339 format).")]
    async fn posts_schedule(
        &self,
        params: Parameters<tools_posts::SchedulePostInput>,
    ) -> Result<Json<tools_posts::SchedulePostOutput>, String> {
        tools_posts::schedule_post(&self.state, &params.0).await
    }

    #[tool(description = "Publish a post immediately. Accepts queued or errored posts.")]
    async fn posts_publish(
        &self,
        params: Parameters<tools_posts::GetPostInput>,
    ) -> Result<Json<tools_posts::SchedulePostOutput>, String> {
        tools_posts::publish_post(&self.state, &params.0).await
    }

    #[tool(description = "Delete a post by ID.")]
    async fn posts_delete(
        &self,
        params: Parameters<tools_posts::DeletePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::delete_post(&self.state, &params.0).await
    }

    #[tool(
        description = "Create a thread: ordered parts sharing one group_id, linked at publish time. Mirrors POST /api/posts/thread."
    )]
    async fn posts_thread(
        &self,
        params: Parameters<tools_posts::ThreadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::create_thread(&self.state, &params.0).await
    }

    #[tool(
        description = "Move a queued/draft/error post back to draft (unschedule). Mirrors POST /api/posts/{id}/unschedule."
    )]
    async fn posts_unschedule(
        &self,
        params: Parameters<tools_posts::UnscheduleInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::unschedule(&self.state, &params.0).await
    }

    #[tool(
        description = "Reschedule a post, or its whole thread/campaign group with move_group. For published posts pass action: 'schedule' (re-publish) or 'update' (re-date). Mirrors PUT /api/posts/{id}/date."
    )]
    async fn posts_reschedule(
        &self,
        params: Parameters<tools_posts::RescheduleInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::reschedule(&self.state, &params.0).await
    }

    #[tool(
        description = "Fetch every post in a campaign/thread group by group_id. Mirrors GET /api/posts/group/{id}."
    )]
    async fn posts_group(
        &self,
        params: Parameters<tools_posts::GroupInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::get_group(&self.state, &params.0).await
    }

    #[tool(
        description = "Validate a post against every integration WITHOUT creating it (publish-time rules). Mirrors POST /api/posts/validate."
    )]
    async fn posts_validate(
        &self,
        params: Parameters<tools_posts::ValidateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::validate(&self.state, &params.0).await
    }

    #[tool(description = "Find the next available free time slot for scheduling.")]
    async fn posts_find_slot(
        &self,
        params: Parameters<tools_posts::FindSlotInput>,
    ) -> Result<Json<tools_posts::FindSlotOutput>, String> {
        tools_posts::find_slot(&self.state, &params.0).await
    }

    #[tool(description = "Update a post's content, title, media, or settings by ID.")]
    async fn posts_update(
        &self,
        params: Parameters<tools_posts::UpdatePostInput>,
    ) -> Result<Json<tools_posts::UpdatePostOutput>, String> {
        tools_posts::update_post(&self.state, &params.0).await
    }

    #[tool(
        description = "Upload media (image/video) for post attachments. Pass base64-encoded file data."
    )]
    async fn posts_media_upload(
        &self,
        params: Parameters<tools_media::MediaUploadInput>,
    ) -> Result<Json<tools_media::MediaUploadOutput>, String> {
        tools_media::upload_media(&self.state, &params.0).await
    }

    #[tool(description = "List uploaded media files.")]
    async fn posts_media_list(
        &self,
        params: Parameters<tools_media::MediaListInput>,
    ) -> Result<Json<tools_media::MediaListOutput>, String> {
        tools_media::list_media(&self.state, &params.0).await
    }

    #[tool(
        description = "Upload media from a local file path. More efficient than base64 for large files. Returns the media URL for use in posts."
    )]
    async fn posts_media_upload_from_path(
        &self,
        params: Parameters<tools_media::MediaUploadFromPathInput>,
    ) -> Result<Json<tools_media::MediaUploadOutput>, String> {
        tools_media::upload_from_path_mcp(&self.state, &params.0).await
    }

    #[tool(
        description = "Batch upload multiple media files from local file paths in a single call. Optimized for carousel workflows. Returns all uploaded media URLs."
    )]
    async fn posts_media_upload_batch(
        &self,
        params: Parameters<tools_media::MediaUploadBatchInput>,
    ) -> Result<Json<tools_media::MediaUploadBatchOutput>, String> {
        tools_media::upload_batch(&self.state, &params.0).await
    }

    #[tool(
        description = "Download media from an external URL and store it locally. Useful when you have image URLs but need them in the local media library for posting."
    )]
    async fn posts_media_upload_from_url(
        &self,
        params: Parameters<tools_media::MediaUploadFromUrlInput>,
    ) -> Result<Json<tools_media::MediaUploadOutput>, String> {
        tools_media::upload_from_url(&self.state, &params.0).await
    }

    #[tool(
        description = "Stage a post across multiple platforms. Auto-splits content per platform character limits. Creates draft posts for each integration."
    )]
    async fn posts_stage(
        &self,
        params: Parameters<tools_posts::StagePostInput>,
    ) -> Result<Json<tools_posts::StagePostOutput>, String> {
        tools_posts::stage_post(&self.state, &params.0).await
    }

    #[tool(
        description = "Create a carousel post with multiple images. Requires at least 2 media URLs. Supported by Instagram, LinkedIn, and Facebook."
    )]
    async fn posts_create_carousel(
        &self,
        params: Parameters<tools_posts::CreateCarouselInput>,
    ) -> Result<Json<tools_posts::CreateCarouselOutput>, String> {
        tools_posts::create_carousel(&self.state, &params.0).await
    }

    #[tool(
        description = "Best time to post per account, from stored engagement history (E5): a ranked weekday×hour histogram of engagement on published posts. Small samples are noisy — treat as a hint."
    )]
    pub async fn posts_best_times(
        &self,
        params: Parameters<tools_posts::PostsBestTimesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_posts::handle_posts_best_times(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Unified inbox (E3): recent comments (swept over each account's latest published posts) plus DM conversations, merged across every connected provider, newest first. One reply verb: inbox_reply."
    )]
    pub async fn inbox_list(
        &self,
        params: Parameters<tools_inbox::InboxListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_inbox::handle_inbox_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Answer an inbox item with one verb (E3). For a comment: provider + kind='comment' + comment_id. For a DM: provider + kind='dm' + recipient. Text is the reply body."
    )]
    pub async fn inbox_reply(
        &self,
        params: Parameters<tools_inbox::InboxReplyInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_inbox::handle_inbox_reply(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Comment Tools ───────────────────────────────────────

    #[tool(
        description = "Get comments for a post on any platform, given the integration and the post ID."
    )]
    async fn get_comments(
        &self,
        params: Parameters<tools_comments::GetCommentsInput>,
    ) -> Result<Json<tools_comments::GetCommentsOutput>, String> {
        tools_comments::get_comments(&self.state, &params.0).await
    }

    #[tool(
        description = "Reply to a comment on any platform, given the integration, the comment ID, and the reply content."
    )]
    async fn reply_to_comment(
        &self,
        params: Parameters<tools_comments::ReplyToCommentInput>,
    ) -> Result<Json<tools_comments::ReplyToCommentOutput>, String> {
        tools_comments::reply_to_comment(&self.state, &params.0).await
    }

    // ── DM Tools ────────────────────────────────────────────

    #[tool(
        description = "Send a direct message on any platform, given the integration, the recipient, and the message content."
    )]
    async fn send_dm(
        &self,
        params: Parameters<tools_dm::SendDmInput>,
    ) -> Result<Json<tools_dm::SendDmOutput>, String> {
        tools_dm::send_dm(&self.state, &params.0).await
    }

    #[tool(description = "List DM conversations on any platform for a connected integration.")]
    async fn list_dm_conversations(
        &self,
        params: Parameters<tools_dm::ListDmInput>,
    ) -> Result<Json<tools_dm::ListDmOutput>, String> {
        tools_dm::list_dm_conversations(&self.state, &params.0).await
    }

    #[tool(description = "Get messages in a DM conversation by conversation ID.")]
    async fn get_dm_messages(
        &self,
        params: Parameters<tools_dm::GetDmInput>,
    ) -> Result<Json<tools_dm::GetDmOutput>, String> {
        tools_dm::get_dm_messages(&self.state, &params.0).await
    }

    // ── Automation Tools ─────────────────────────────────────

    #[tool(
        description = "Create an automation rule that auto-replies to comments or DMs on a connected integration."
    )]
    async fn create_automation_rule(
        &self,
        params: Parameters<tools_automation::CreateRuleInput>,
    ) -> Result<Json<tools_automation::CreateRuleOutput>, String> {
        tools_automation::create_rule(&self.state, &params.0).await
    }

    #[tool(description = "List automation rules, optionally filtered by integration.")]
    async fn list_automation_rules(
        &self,
        params: Parameters<tools_automation::ListRulesInput>,
    ) -> Result<Json<tools_automation::ListRulesOutput>, String> {
        tools_automation::list_rules(&self.state, &params.0).await
    }

    #[tool(description = "Update an automation rule by ID; omitted fields keep their value.")]
    async fn update_automation_rule(
        &self,
        params: Parameters<tools_automation::UpdateRuleInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_automation::update_rule(&self.state, &params.0).await
    }

    #[tool(description = "Delete an automation rule by ID.")]
    async fn delete_automation_rule(
        &self,
        params: Parameters<tools_automation::DeleteRuleInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_automation::delete_rule(&self.state, &params.0).await
    }

    #[tool(description = "Get execution logs for an automation rule by ID.")]
    async fn get_automation_logs(
        &self,
        params: Parameters<tools_automation::GetLogsInput>,
    ) -> Result<Json<tools_automation::GetLogsOutput>, String> {
        tools_automation::get_logs(&self.state, &params.0).await
    }

    // ── Reddit Read/Query Tools ──────────────────────────────────

    #[tool(description = "Browse a subreddit and list posts.")]
    async fn reddit_browse(
        &self,
        params: Parameters<tools_reddit::RedditBrowseInput>,
    ) -> Result<Json<tools_reddit::RedditBrowseOutput>, String> {
        tools_reddit::reddit_browse(&self.state, &params.0).await
    }

    #[tool(description = "Search Reddit. Optionally restrict to a specific subreddit.")]
    async fn reddit_search(
        &self,
        params: Parameters<tools_reddit::RedditSearchInput>,
    ) -> Result<Json<tools_reddit::RedditSearchOutput>, String> {
        tools_reddit::reddit_search(&self.state, &params.0).await
    }

    #[tool(description = "Get a Reddit post's full content with comments and metadata.")]
    async fn reddit_post_detail(
        &self,
        params: Parameters<tools_reddit::RedditPostDetailInput>,
    ) -> Result<Json<tools_reddit::RedditPostDetailOutput>, String> {
        tools_reddit::reddit_post_detail(&self.state, &params.0).await
    }

    #[tool(description = "Get Reddit user info with optional posts and comments.")]
    async fn reddit_user_info(
        &self,
        params: Parameters<tools_reddit::RedditUserInfoInput>,
    ) -> Result<Json<tools_reddit::RedditUserInfoOutput>, String> {
        tools_reddit::reddit_user_info(&self.state, &params.0).await
    }

    #[tool(description = "Send a direct message to a Reddit user.")]
    async fn reddit_send_dm(
        &self,
        params: Parameters<tools_reddit::RedditSendDmInput>,
    ) -> Result<Json<tools_reddit::RedditSendDmOutput>, String> {
        tools_reddit::reddit_send_dm(&self.state, &params.0).await
    }

    #[tool(
        description = "Read Reddit inbox. Folders: inbox, unread, sent, messages, mentions, comments, selfreply."
    )]
    async fn reddit_inbox(
        &self,
        params: Parameters<tools_reddit::RedditInboxInput>,
    ) -> Result<Json<tools_reddit::RedditInboxOutput>, String> {
        tools_reddit::reddit_inbox(&self.state, &params.0).await
    }

    #[tool(description = "Get comments for a Reddit post with the given post ID.")]
    async fn reddit_get_comments(
        &self,
        params: Parameters<tools_reddit::RedditGetCommentsInput>,
    ) -> Result<Json<tools_reddit::RedditGetCommentsOutput>, String> {
        tools_reddit::reddit_get_comments(&self.state, &params.0).await
    }

    #[tool(description = "Create a new Reddit post (text or link) to a subreddit.")]
    pub async fn reddit_create_post(
        &self,
        params: Parameters<tools_reddit::RedditCreatePostInput>,
    ) -> Result<Json<tools_reddit::RedditCreatePostOutput>, String> {
        tools_reddit::handle_reddit_create_post(&self.state, &params.0).await
    }

    #[tool(description = "Create a comment or reply on Reddit.")]
    pub async fn reddit_create_comment(
        &self,
        params: Parameters<tools_reddit::RedditCreateCommentInput>,
    ) -> Result<Json<tools_reddit::RedditCreateCommentOutput>, String> {
        tools_reddit::handle_reddit_create_comment(&self.state, &params.0).await
    }

    #[tool(description = "Get karma breakdown by subreddit for the authenticated user.")]
    pub async fn reddit_get_karma(
        &self,
    ) -> Result<Json<tools_reddit::RedditGetKarmaOutput>, String> {
        tools_reddit::handle_reddit_get_karma(&self.state).await
    }

    #[tool(description = "Vote on a Reddit post or comment.")]
    pub async fn reddit_vote(
        &self,
        params: Parameters<tools_reddit::RedditVoteInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_vote(&self.state, &params.0).await
    }

    #[tool(description = "Save a Reddit post or comment.")]
    pub async fn reddit_save(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_save(&self.state, &params.0).await
    }

    #[tool(description = "Unsave a Reddit post or comment.")]
    pub async fn reddit_unsave(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_unsave(&self.state, &params.0).await
    }

    #[tool(description = "Hide a Reddit post from your feed.")]
    pub async fn reddit_hide(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_hide(&self.state, &params.0).await
    }

    #[tool(description = "Subscribe or unsubscribe to a subreddit.")]
    pub async fn reddit_subscribe(
        &self,
        params: Parameters<tools_reddit::RedditSubscribeInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_subscribe(&self.state, &params.0).await
    }

    #[tool(description = "Edit a Reddit post or comment text.")]
    pub async fn reddit_edit(
        &self,
        params: Parameters<tools_reddit::RedditEditInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_edit(&self.state, &params.0).await
    }

    #[tool(description = "Delete a Reddit post or comment.")]
    pub async fn reddit_delete(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_delete(&self.state, &params.0).await
    }

    #[tool(description = "Moderation: remove a post/comment.")]
    pub async fn reddit_mod_remove(
        &self,
        params: Parameters<tools_reddit::RedditModRemoveInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_remove(&self.state, &params.0).await
    }

    #[tool(description = "Moderation: approve a post/comment.")]
    pub async fn reddit_mod_approve(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_approve(&self.state, &params.0).await
    }

    #[tool(
        description = "Moderation: distinguish a comment (how: 'yes', 'no', 'admin', 'special'). Requires mod cookie auth."
    )]
    pub async fn reddit_mod_distinguish(
        &self,
        params: Parameters<tools_reddit::RedditModDistinguishInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_distinguish(&self.state, &params.0).await
    }

    #[tool(description = "Moderation: sticky or unsticky a post in a subreddit you moderate.")]
    pub async fn reddit_mod_sticky(
        &self,
        params: Parameters<tools_reddit::RedditModStickyInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_sticky(&self.state, &params.0).await
    }

    #[tool(description = "Moderation: lock a post/comment thread. Requires mod cookie auth.")]
    pub async fn reddit_mod_lock(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_lock(&self.state, &params.0).await
    }

    #[tool(description = "Moderation: unlock a post/comment thread. Requires mod cookie auth.")]
    pub async fn reddit_mod_unlock(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mod_unlock(&self.state, &params.0).await
    }

    // ── Reddit Parity Tools (P6.5, reddit-lyr port) ──────────

    #[tool(
        description = "Report a Reddit post or comment to the moderators. Provide at least one reason."
    )]
    pub async fn reddit_report(
        &self,
        params: Parameters<tools_reddit::RedditReportInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_report(&self.state, &params.0).await
    }

    #[tool(description = "Block a Reddit user (they can no longer message you or see your posts).")]
    pub async fn reddit_block_user(
        &self,
        params: Parameters<tools_reddit::RedditUsernameInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_block_user(&self.state, &params.0).await
    }

    #[tool(description = "List your Reddit friends.")]
    pub async fn reddit_list_friends(
        &self,
    ) -> Result<Json<tools_reddit::RedditDataOutput>, String> {
        tools_reddit::handle_reddit_list_friends(&self.state).await
    }

    #[tool(description = "Add a Reddit friend.")]
    pub async fn reddit_add_friend(
        &self,
        params: Parameters<tools_reddit::RedditUsernameInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_add_friend(&self.state, &params.0).await
    }

    #[tool(description = "Remove a Reddit friend.")]
    pub async fn reddit_remove_friend(
        &self,
        params: Parameters<tools_reddit::RedditUsernameInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_remove_friend(&self.state, &params.0).await
    }

    #[tool(description = "Mark your own Reddit post as NSFW.")]
    pub async fn reddit_mark_nsfw(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mark_nsfw(&self.state, &params.0).await
    }

    #[tool(description = "Remove the NSFW tag from your own Reddit post.")]
    pub async fn reddit_unmark_nsfw(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_unmark_nsfw(&self.state, &params.0).await
    }

    #[tool(description = "Mark your own Reddit post as a spoiler.")]
    pub async fn reddit_mark_spoiler(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_mark_spoiler(&self.state, &params.0).await
    }

    #[tool(description = "Remove the spoiler tag from your own Reddit post.")]
    pub async fn reddit_unmark_spoiler(
        &self,
        params: Parameters<tools_reddit::RedditThingInput>,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_unmark_spoiler(&self.state, &params.0).await
    }

    #[tool(description = "Get a subreddit's rules.")]
    pub async fn reddit_get_rules(
        &self,
        params: Parameters<tools_reddit::RedditSubredditInput>,
    ) -> Result<Json<tools_reddit::RedditDataOutput>, String> {
        tools_reddit::handle_reddit_get_rules(&self.state, &params.0).await
    }

    #[tool(
        description = "List a subreddit's link flairs — the id pairs usable as reddit_create_post's flair_id."
    )]
    pub async fn reddit_list_flairs(
        &self,
        params: Parameters<tools_reddit::RedditSubredditInput>,
    ) -> Result<Json<tools_reddit::RedditDataOutput>, String> {
        tools_reddit::handle_reddit_list_flairs(&self.state, &params.0).await
    }

    #[tool(description = "List your saved Reddit posts and comments.")]
    pub async fn reddit_get_saved(
        &self,
        params: Parameters<tools_reddit::RedditLimitInput>,
    ) -> Result<Json<tools_reddit::RedditDataOutput>, String> {
        tools_reddit::handle_reddit_get_saved(&self.state, &params.0).await
    }

    #[tool(description = "Mark all your Reddit inbox messages as read.")]
    pub async fn reddit_read_all_messages(
        &self,
    ) -> Result<Json<tools_reddit::RedditActionOutput>, String> {
        tools_reddit::handle_reddit_read_all_messages(&self.state).await
    }

    #[tool(description = "Crosspost an existing Reddit post into a subreddit.")]
    pub async fn reddit_crosspost(
        &self,
        params: Parameters<tools_reddit::RedditCrosspostInput>,
    ) -> Result<Json<tools_reddit::RedditCreatePostOutput>, String> {
        tools_reddit::handle_reddit_crosspost(&self.state, &params.0).await
    }

    // ── X/Twitter Read Tools ─────────────────────────────────────

    #[tool(description = "Get the authenticated X/Twitter user's profile.")]
    async fn x_get_me(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<Json<tools_x::XGetMeOutput>, String> {
        tools_x::x_get_me(&self.state).await
    }

    #[tool(
        description = "Get X/Twitter home timeline (reverse chronological). Shows recent tweets from people you follow."
    )]
    async fn x_home_timeline(
        &self,
        params: Parameters<tools_x::XHomeTimelineInput>,
    ) -> Result<Json<tools_x::XHomeTimelineOutput>, String> {
        tools_x::x_home_timeline(&self.state, &params.0).await
    }

    #[tool(description = "Lookup an X/Twitter user by their numeric user ID.")]
    async fn x_user_lookup(
        &self,
        params: Parameters<tools_x::XUserLookupInput>,
    ) -> Result<Json<tools_x::XUserLookupOutput>, String> {
        tools_x::x_user_lookup(&self.state, &params.0).await
    }

    #[tool(description = "Lookup an X/Twitter user by their @username.")]
    async fn x_user_lookup_by_username(
        &self,
        params: Parameters<tools_x::XUserLookupByUsernameInput>,
    ) -> Result<Json<tools_x::XUserLookupByUsernameOutput>, String> {
        tools_x::x_user_lookup_by_username(&self.state, &params.0).await
    }

    #[tool(description = "Get tweets from a specific X/Twitter user by their user ID.")]
    async fn x_user_tweets(
        &self,
        params: Parameters<tools_x::XUserTweetsInput>,
    ) -> Result<Json<tools_x::XUserTweetsOutput>, String> {
        tools_x::x_user_tweets(&self.state, &params.0).await
    }

    #[tool(description = "Get a single X/Twitter tweet with full details by tweet ID.")]
    async fn x_tweet_detail(
        &self,
        params: Parameters<tools_x::XTweetDetailInput>,
    ) -> Result<Json<tools_x::XTweetDetailOutput>, String> {
        tools_x::x_tweet_detail(&self.state, &params.0).await
    }

    #[tool(
        description = "Search recent X/Twitter tweets. Query supports standard Twitter search syntax (from:user, #hashtag, etc.)."
    )]
    async fn x_search_tweets(
        &self,
        params: Parameters<tools_x::XSearchTweetsInput>,
    ) -> Result<Json<tools_x::XSearchTweetsOutput>, String> {
        tools_x::x_search_tweets(&self.state, &params.0).await
    }

    #[tool(description = "Get followers of an X/Twitter user.")]
    async fn x_followers(
        &self,
        params: Parameters<tools_x::XFollowersInput>,
    ) -> Result<Json<tools_x::XFollowersOutput>, String> {
        tools_x::x_followers(&self.state, &params.0).await
    }

    #[tool(description = "Get who an X/Twitter user is following.")]
    async fn x_following(
        &self,
        params: Parameters<tools_x::XFollowingInput>,
    ) -> Result<Json<tools_x::XFollowingOutput>, String> {
        tools_x::x_following(&self.state, &params.0).await
    }

    #[tool(description = "Get bookmarked X/Twitter tweets.")]
    async fn x_bookmarks(
        &self,
        params: Parameters<tools_x::XBookmarksInput>,
    ) -> Result<Json<tools_x::XBookmarksOutput>, String> {
        tools_x::x_bookmarks(&self.state, &params.0).await
    }

    // ── X/Twitter Write Tools ────────────────────────────────────

    #[tool(description = "Delete an X/Twitter tweet by its ID.")]
    async fn x_delete_tweet(
        &self,
        params: Parameters<tools_x::XDeleteTweetInput>,
    ) -> Result<Json<tools_x::XDeleteTweetOutput>, String> {
        tools_x::x_delete_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Like a tweet on X/Twitter.")]
    async fn x_like_tweet(
        &self,
        params: Parameters<tools_x::XLikeTweetInput>,
    ) -> Result<Json<tools_x::XLikeTweetOutput>, String> {
        tools_x::x_like_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Unlike a tweet on X/Twitter.")]
    async fn x_unlike_tweet(
        &self,
        params: Parameters<tools_x::XUnlikeTweetInput>,
    ) -> Result<Json<tools_x::XUnlikeTweetOutput>, String> {
        tools_x::x_unlike_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Retweet a tweet on X/Twitter.")]
    async fn x_retweet(
        &self,
        params: Parameters<tools_x::XRetweetInput>,
    ) -> Result<Json<tools_x::XRetweetOutput>, String> {
        tools_x::x_retweet(&self.state, &params.0).await
    }

    #[tool(description = "Unretweet (remove retweet of) a tweet on X/Twitter.")]
    async fn x_unretweet(
        &self,
        params: Parameters<tools_x::XUnretweetInput>,
    ) -> Result<Json<tools_x::XUnretweetOutput>, String> {
        tools_x::x_unretweet(&self.state, &params.0).await
    }

    #[tool(description = "Bookmark a tweet on X/Twitter.")]
    async fn x_bookmark_tweet(
        &self,
        params: Parameters<tools_x::XBookmarkTweetInput>,
    ) -> Result<Json<tools_x::XBookmarkTweetOutput>, String> {
        tools_x::x_bookmark_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Remove a bookmark from a tweet on X/Twitter.")]
    async fn x_unbookmark_tweet(
        &self,
        params: Parameters<tools_x::XUnbookmarkTweetInput>,
    ) -> Result<Json<tools_x::XUnbookmarkTweetOutput>, String> {
        tools_x::x_unbookmark_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Follow a user on X/Twitter by their user ID.")]
    async fn x_follow_user(
        &self,
        params: Parameters<tools_x::XFollowUserInput>,
    ) -> Result<Json<tools_x::XFollowUserOutput>, String> {
        tools_x::x_follow_user(&self.state, &params.0).await
    }

    #[tool(description = "Unfollow a user on X/Twitter by their user ID.")]
    async fn x_unfollow_user(
        &self,
        params: Parameters<tools_x::XUnfollowUserInput>,
    ) -> Result<Json<tools_x::XUnfollowUserOutput>, String> {
        tools_x::x_unfollow_user(&self.state, &params.0).await
    }

    #[tool(description = "Get tweets from an X/Twitter List by list ID.")]
    async fn x_list_timeline(
        &self,
        params: Parameters<tools_x::XListTimelineInput>,
    ) -> Result<Json<tools_x::XListTimelineOutput>, String> {
        tools_x::x_list_timeline(&self.state, &params.0).await
    }

    #[tool(
        description = "Create and immediately publish a new tweet on X/Twitter. For media tweets, first upload via media_upload_from_path, then pass returned URLs in media_urls."
    )]
    async fn x_create_tweet(
        &self,
        params: Parameters<tools_x::XCreateTweetInput>,
    ) -> Result<Json<tools_x::XCreateTweetOutput>, String> {
        tools_x::x_create_tweet(&self.state, &params.0).await
    }

    #[tool(
        description = "Get account-level analytics for the authenticated X/Twitter user (followers, following, tweet count)"
    )]
    async fn x_get_analytics(
        &self,
        params: Parameters<tools_x::XGetAnalyticsInput>,
    ) -> Result<Json<tools_x::XGetAnalyticsOutput>, String> {
        tools_x::x_get_analytics(&self.state, &params.0).await
    }

    #[tool(
        description = "Get engagement metrics (likes, retweets, replies, impressions) for a specific tweet."
    )]
    async fn x_get_post_analytics(
        &self,
        params: Parameters<tools_x::XGetPostAnalyticsInput>,
    ) -> Result<Json<tools_x::XGetPostAnalyticsOutput>, String> {
        tools_x::x_get_post_analytics(&self.state, &params.0).await
    }

    #[tool(description = "Reply to an X/Twitter tweet by tweet ID.")]
    async fn x_reply_tweet(
        &self,
        params: Parameters<tools_x::XReplyTweetInput>,
    ) -> Result<Json<tools_x::XReplyTweetOutput>, String> {
        tools_x::x_reply_tweet(&self.state, &params.0).await
    }

    #[tool(description = "Send a direct message on X/Twitter to a recipient user ID.")]
    async fn x_send_dm(
        &self,
        params: Parameters<tools_x::XSendDmInput>,
    ) -> Result<Json<tools_x::XSendDmOutput>, String> {
        tools_x::x_send_dm(&self.state, &params.0).await
    }

    #[tool(description = "List X/Twitter DM conversations for the authenticated user.")]
    async fn x_list_dms(
        &self,
        params: Parameters<tools_x::XListDmsInput>,
    ) -> Result<Json<tools_x::XListDmsOutput>, String> {
        tools_x::x_list_dms(&self.state, &params.0).await
    }

    #[tool(description = "Get messages in an X/Twitter DM conversation by conversation ID.")]
    async fn x_get_dm_conversation(
        &self,
        params: Parameters<tools_x::XGetDmConversationInput>,
    ) -> Result<Json<tools_x::XGetDmConversationOutput>, String> {
        tools_x::x_get_dm_conversation(&self.state, &params.0).await
    }

    // ── Facebook Tools ──────────────────────────────────────────────

    #[tool(description = "Get a Facebook page's feed (recent posts).")]
    pub async fn fb_get_feed(
        &self,
        params: Parameters<tools_facebook::FbGetFeedInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_get_feed(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Facebook post by ID.")]
    pub async fn fb_get_post(
        &self,
        params: Parameters<tools_facebook::FbGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get comments on a Facebook post.")]
    pub async fn fb_get_comments(
        &self,
        params: Parameters<tools_facebook::FbGetCommentsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_get_comments(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a text/link post on a Facebook page.")]
    pub async fn fb_create_post(
        &self,
        params: Parameters<tools_facebook::FbCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a photo post on a Facebook page.")]
    pub async fn fb_create_photo(
        &self,
        params: Parameters<tools_facebook::FbCreatePhotoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_create_photo(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a video post on a Facebook page.")]
    pub async fn fb_create_video(
        &self,
        params: Parameters<tools_facebook::FbCreateVideoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_create_video(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a Facebook post by ID.")]
    pub async fn fb_delete_post(
        &self,
        params: Parameters<tools_facebook::FbDeletePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_delete_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Comment on a Facebook post.")]
    pub async fn fb_comment(
        &self,
        params: Parameters<tools_facebook::FbCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "React to a Facebook post (LIKE, LOVE, WOW, HAHA, SAD, ANGRY).")]
    pub async fn fb_react(
        &self,
        params: Parameters<tools_facebook::FbReactInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_react(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get insights/analytics for a Facebook page.")]
    pub async fn fb_page_insights(
        &self,
        params: Parameters<tools_facebook::FbPageInsightsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_page_insights(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get conversations (inbox) for a connected Facebook page.")]
    pub async fn fb_conversations(
        &self,
        params: Parameters<tools_facebook::FbConversationsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_conversations(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get messages in a Facebook conversation.")]
    pub async fn fb_conversation_messages(
        &self,
        params: Parameters<tools_facebook::FbConversationMsgsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_conversation_msgs(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Send a message to a Facebook conversation.")]
    pub async fn fb_send_message(
        &self,
        params: Parameters<tools_facebook::FbSendMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_send_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search for public Facebook pages by name.")]
    pub async fn fb_search_pages(
        &self,
        params: Parameters<tools_facebook::FbSearchPagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_search_pages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get albums from a Facebook page.")]
    pub async fn fb_albums(
        &self,
        params: Parameters<tools_facebook::FbAlbumsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_facebook::handle_fb_albums(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Instagram Tools ─────────────────────────────────────────────

    #[tool(description = "Get Instagram media (posts/reels) for a business account.")]
    pub async fn ig_get_media(
        &self,
        params: Parameters<tools_instagram::IgGetMediaInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_media(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a specific Instagram media item.")]
    pub async fn ig_get_media_detail(
        &self,
        params: Parameters<tools_instagram::IgGetMediaDetailInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_media_detail(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get comments on an Instagram media post by media ID.")]
    pub async fn ig_get_comments(
        &self,
        params: Parameters<tools_instagram::IgGetCommentsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_comments(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search for Instagram hashtags by name.")]
    pub async fn ig_search_hashtag(
        &self,
        params: Parameters<tools_instagram::IgSearchHashtagInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_search_hashtag(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get recent media for an Instagram hashtag.")]
    pub async fn ig_get_hashtag_media(
        &self,
        params: Parameters<tools_instagram::IgGetHashtagMediaInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_hashtag_media(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get insights for an Instagram business account.")]
    pub async fn ig_get_insights(
        &self,
        params: Parameters<tools_instagram::IgGetInsightsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_insights(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get media where the Instagram account is tagged.")]
    pub async fn ig_get_tagged(
        &self,
        params: Parameters<tools_instagram::IgGetTaggedInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_tagged(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create an Instagram media container (step 1 of publish).")]
    pub async fn ig_create_container(
        &self,
        params: Parameters<tools_instagram::IgCreateContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_create_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Publish an Instagram media container (step 2 of publish).")]
    pub async fn ig_publish_container(
        &self,
        params: Parameters<tools_instagram::IgPublishContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_publish_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to an Instagram comment.")]
    pub async fn ig_reply_to_comment(
        &self,
        params: Parameters<tools_instagram::IgReplyToCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_reply_to_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Instagram reels for a business account.")]
    pub async fn ig_get_reels(
        &self,
        params: Parameters<tools_instagram::IgGetReelsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_reels(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Instagram stories for a business account.")]
    pub async fn ig_get_stories(
        &self,
        params: Parameters<tools_instagram::IgGetStoriesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_stories(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get followers of an Instagram business account.")]
    pub async fn ig_get_followers(
        &self,
        params: Parameters<tools_instagram::IgGetFollowersInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_followers(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Discover an Instagram business account by username.")]
    pub async fn ig_business_discovery(
        &self,
        params: Parameters<tools_instagram::IgBusinessDiscoveryInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_business_discovery(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get recent mentions of an Instagram business account.")]
    pub async fn ig_get_mentions(
        &self,
        params: Parameters<tools_instagram::IgGetMentionsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_mentions(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Poll the status of an Instagram container (before publishing).")]
    pub async fn ig_poll_container(
        &self,
        params: Parameters<tools_instagram::IgPollContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_poll_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get audience insights for an Instagram business account.")]
    pub async fn ig_get_insights_audience(
        &self,
        params: Parameters<tools_instagram::IgGetInsightsAudienceInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_insights_audience(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Send a direct message on Instagram to a recipient user ID.")]
    pub async fn ig_send_dm(
        &self,
        params: Parameters<tools_instagram::IgSendDmInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_send_dm(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Instagram message conversations for a business account.")]
    pub async fn ig_list_conversations(
        &self,
        params: Parameters<tools_instagram::IgListConversationsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_list_conversations(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get messages in an Instagram conversation by conversation ID.")]
    pub async fn ig_get_messages(
        &self,
        params: Parameters<tools_instagram::IgGetMessagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram::handle_ig_get_messages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Create and publish an Instagram media post in a single call. Handles the full container flow internally (create + poll + publish). Pass ig_id, caption, media_url, and media_type (IMAGE/VIDEO/REELS)."
    )]
    pub async fn ig_post(
        &self,
        params: Parameters<tools_instagram::IgPostInput>,
    ) -> Result<Json<tools_instagram::IgPostOutput>, String> {
        tools_instagram::handle_ig_post(&self.state, &params.0).await
    }

    // ── Instagram Standalone (Basic Display API) Tools ─────────────────

    #[tool(
        description = "Get the Instagram media feed for a personal account (Basic Display API)."
    )]
    pub async fn ias_get_media(
        &self,
        params: Parameters<tools_instagram_standalone::IasGetMediaInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_get_media(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a specific Instagram media item (Basic Display API).")]
    pub async fn ias_get_media_detail(
        &self,
        params: Parameters<tools_instagram_standalone::IasGetMediaDetailInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_get_media_detail(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get comments on an Instagram media post by media ID.")]
    pub async fn ias_get_comments(
        &self,
        params: Parameters<tools_instagram_standalone::IasGetCommentsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_get_comments(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to an Instagram comment (Basic Display API).")]
    pub async fn ias_reply_to_comment(
        &self,
        params: Parameters<tools_instagram_standalone::IasReplyToCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_reply_to_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Create an Instagram media container (step 1 of publish, Basic Display API)."
    )]
    pub async fn ias_create_container(
        &self,
        params: Parameters<tools_instagram_standalone::IasCreateContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_create_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Publish an Instagram media container (step 2 of publish, Basic Display API)."
    )]
    pub async fn ias_publish_container(
        &self,
        params: Parameters<tools_instagram_standalone::IasPublishContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_publish_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Poll Instagram container publish status (Basic Display API).")]
    pub async fn ias_poll_container(
        &self,
        params: Parameters<tools_instagram_standalone::IasPollContainerInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_instagram_standalone::handle_ias_poll_container(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Google Suite Tools ─────────────────────────────────────────
    // YouTube, Gmail, Calendar, Drive — all use Google OAuth single sign-on

    #[tool(description = "Search YouTube videos by query.")]
    pub async fn goog_search_videos(
        &self,
        params: Parameters<tools_google::YtSearchVideosInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_search_videos(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a specific YouTube video by video ID.")]
    pub async fn goog_get_video(
        &self,
        params: Parameters<tools_google::YtGetVideoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_video(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List playlists for a YouTube channel.")]
    pub async fn goog_list_playlists(
        &self,
        params: Parameters<tools_google::YtListPlaylistsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_playlists(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get items in a YouTube playlist.")]
    pub async fn goog_get_playlist_items(
        &self,
        params: Parameters<tools_google::YtGetPlaylistItemsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_playlist_items(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get comments on a YouTube video.")]
    pub async fn goog_get_comments(
        &self,
        params: Parameters<tools_google::YtGetCommentsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_comments(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get statistics for a YouTube channel (subscribers, views, videos).")]
    pub async fn goog_get_channel_stats(
        &self,
        params: Parameters<tools_google::YtGetChannelStatsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_channel_stats(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get YouTube Analytics reports for a channel by date range.")]
    pub async fn goog_get_analytics(
        &self,
        params: Parameters<tools_google::YtGetAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get subscriptions for a YouTube channel.")]
    pub async fn goog_get_subscriptions(
        &self,
        params: Parameters<tools_google::YtGetSubscriptionsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_subscriptions(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Find YouTube creators by topic.")]
    pub async fn goog_find_creators(
        &self,
        params: Parameters<tools_google::YtFindCreatorsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_find_creators(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to a YouTube comment by comment ID on a channel.")]
    pub async fn yt_reply_comment(
        &self,
        params: Parameters<tools_youtube::YtReplyCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_youtube::handle_yt_reply_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Upload and publish a video to YouTube immediately. Uses YouTube's resumable upload API. Pass video_url (from media_upload), title, description, and channel_id."
    )]
    pub async fn yt_upload_video(
        &self,
        params: Parameters<tools_youtube::YtUploadVideoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_youtube::handle_yt_upload_video(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Gmail profile info.")]
    pub async fn goog_get_profile(&self) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_profile(&self.state, &())
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Gmail messages with optional search query.")]
    pub async fn goog_list_messages(
        &self,
        params: Parameters<tools_google::GmListMessagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_messages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Gmail message by ID.")]
    pub async fn goog_get_message(
        &self,
        params: Parameters<tools_google::GmGetMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Send an email via Gmail.")]
    pub async fn goog_send_message(
        &self,
        params: Parameters<tools_google::GmSendMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_send_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Gmail labels.")]
    pub async fn goog_list_labels(&self) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_labels(&self.state, &())
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a Gmail thread by ID.")]
    pub async fn goog_get_thread(
        &self,
        params: Parameters<tools_google::GmGetThreadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_thread(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search Gmail messages with a query.")]
    pub async fn goog_search_messages(
        &self,
        params: Parameters<tools_google::GmSearchMessagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_search_messages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Google calendars.")]
    pub async fn goog_list_calendars(
        &self,
        params: Parameters<tools_google::GcalListCalendarsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_calendars(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List calendar events.")]
    pub async fn goog_list_events(
        &self,
        params: Parameters<tools_google::GcalListEventsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_events(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single calendar event.")]
    pub async fn goog_get_event(
        &self,
        params: Parameters<tools_google::GcalGetEventInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_event(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a new calendar event.")]
    pub async fn goog_create_event(
        &self,
        params: Parameters<tools_google::GcalCreateEventInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_create_event(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Update a calendar event.")]
    pub async fn goog_update_event(
        &self,
        params: Parameters<tools_google::GcalUpdateEventInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_update_event(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a calendar event.")]
    pub async fn goog_delete_event(
        &self,
        params: Parameters<tools_google::GcalDeleteEventInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_delete_event(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List files in Google Drive.")]
    pub async fn goog_list_files(
        &self,
        params: Parameters<tools_google::DrListFilesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_files(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a file from Google Drive by ID.")]
    pub async fn goog_get_file(
        &self,
        params: Parameters<tools_google::DrGetFileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_file(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search files in Google Drive.")]
    pub async fn goog_search_files(
        &self,
        params: Parameters<tools_google::DrSearchFilesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_search_files(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List folders in Google Drive.")]
    pub async fn goog_list_folders(
        &self,
        params: Parameters<tools_google::DrListFoldersInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_list_folders(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get file metadata from Google Drive.")]
    pub async fn goog_get_file_metadata(
        &self,
        params: Parameters<tools_google::DrGetFileMetadataInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_get_file_metadata(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Export a Google Drive file to a specific MIME type.")]
    pub async fn goog_export_file(
        &self,
        params: Parameters<tools_google::DrExportFileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_google::handle_goog_export_file(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Pinterest Tools ──────────────────────────────────────────

    #[tool(description = "Get Pinterest user account info.")]
    pub async fn pi_get_user_account(
        &self,
        params: Parameters<tools_pinterest::PiGetUserAccountInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_user_account(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Pinterest board details.")]
    pub async fn pi_get_board(
        &self,
        params: Parameters<tools_pinterest::PiGetBoardInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_board(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get pins on a Pinterest board.")]
    pub async fn pi_get_board_pins(
        &self,
        params: Parameters<tools_pinterest::PiGetBoardPinsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_board_pins(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Pinterest pin by pin ID.")]
    pub async fn pi_get_pin(
        &self,
        params: Parameters<tools_pinterest::PiGetPinInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_pin(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get analytics for a Pinterest board.")]
    pub async fn pi_get_board_analytics(
        &self,
        params: Parameters<tools_pinterest::PiGetBoardAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_board_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get analytics for a Pinterest pin by pin ID.")]
    pub async fn pi_get_pin_analytics(
        &self,
        params: Parameters<tools_pinterest::PiGetPinAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_get_pin_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search Pinterest pins by keyword using Pinterest API v5.")]
    pub async fn pi_search_pins(
        &self,
        params: Parameters<tools_pinterest::PiSearchPinsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_search_pins(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Create and publish a new Pinterest pin immediately. Supports image and video pins. Pass board_id, title, content, and media_urls."
    )]
    pub async fn pi_create_pin(
        &self,
        params: Parameters<tools_pinterest::PiCreatePinInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_pinterest::handle_pi_create_pin(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Discord Tools ───────────────────────────────────────────

    #[tool(description = "Get Discord channel info.")]
    pub async fn di_get_channel(
        &self,
        params: Parameters<tools_discord::DiGetChannelInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_channel(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get messages from a Discord channel.")]
    pub async fn di_get_messages(
        &self,
        params: Parameters<tools_discord::DiGetMessagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_messages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Discord guild/server info.")]
    pub async fn di_get_guild(
        &self,
        params: Parameters<tools_discord::DiGetGuildInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_guild(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get thread members in a Discord channel.")]
    pub async fn di_get_thread_members(
        &self,
        params: Parameters<tools_discord::DiGetThreadMembersInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_thread_members(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Send a message to a Discord channel.")]
    pub async fn di_send_message(
        &self,
        params: Parameters<tools_discord::DiSendMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_send_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a message from a Discord channel.")]
    pub async fn di_delete_message(
        &self,
        params: Parameters<tools_discord::DiDeleteMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_delete_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Add a reaction (emoji) to a Discord message.")]
    pub async fn di_add_reaction(
        &self,
        params: Parameters<tools_discord::DiAddReactionInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_add_reaction(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all channels in a Discord guild/server.")]
    pub async fn di_get_guild_channels(
        &self,
        params: Parameters<tools_discord::DiGetGuildChannelsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_guild_channels(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get detailed info about a Discord guild/server including member counts.")]
    pub async fn di_get_server_info(
        &self,
        params: Parameters<tools_discord::DiGetServerInfoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_get_server_info(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a forum post in a Discord forum channel.")]
    pub async fn di_create_forum_post(
        &self,
        params: Parameters<tools_discord::DiCreateForumPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_create_forum_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Discord Read Tools (E7-read, discord-cli port) ───────

    #[tool(
        description = "Fetch deep channel history with cursor pagination (100/request). Returns messages chronologically. Pass `before` for backfill or `after` for incremental sync."
    )]
    pub async fn di_history(
        &self,
        params: Parameters<tools_discord::DiHistoryInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_history(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Search messages across a Discord guild (and optionally one channel) using Discord's built-in search."
    )]
    pub async fn di_search(
        &self,
        params: Parameters<tools_discord::DiSearchInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_search(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Tail a Discord channel: latest messages snapshot. Pass `after` = the previous call's newest_id to fetch only what arrived since."
    )]
    pub async fn di_tail(
        &self,
        params: Parameters<tools_discord::DiTailInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_tail(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Post a message via a Discord webhook URL — zero setup, no bot or OAuth needed. Optionally override username/avatar; wait=true returns the sent message id."
    )]
    pub async fn di_webhook_post(
        &self,
        params: Parameters<tools_discord::DiWebhookPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_webhook_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Inspect a Discord webhook URL: validates it and returns the name, channel and guild it posts to. Use before di_webhook_post to verify a URL."
    )]
    pub async fn di_webhook_info(
        &self,
        params: Parameters<tools_discord::DiWebhookInfoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_discord::handle_di_webhook_info(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── WhatsApp Tools ───────────────────────────────────────────

    #[tool(description = "Check WhatsApp authentication status.")]
    pub async fn wa_auth_status(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<Json<tools_whatsapp::WaAuthStatusOutput>, String> {
        tools_whatsapp::handle_wa_auth_status(&self.state).await
    }

    #[tool(description = "Send a text message via WhatsApp.")]
    pub async fn wa_send_text(
        &self,
        params: Parameters<tools_whatsapp::WaSendTextInput>,
    ) -> Result<Json<tools_whatsapp::WaSendTextOutput>, String> {
        tools_whatsapp::handle_wa_send_text(&self.state, &params.0).await
    }

    #[tool(description = "List WhatsApp chats.")]
    pub async fn wa_chats(
        &self,
        params: Parameters<tools_whatsapp::WaChatsInput>,
    ) -> Result<Json<tools_whatsapp::WaChatsOutput>, String> {
        tools_whatsapp::handle_wa_chats(&self.state, &params.0).await
    }

    #[tool(description = "List WhatsApp contacts.")]
    pub async fn wa_contacts(
        &self,
        params: Parameters<tools_whatsapp::WaContactsInput>,
    ) -> Result<Json<tools_whatsapp::WaContactsOutput>, String> {
        tools_whatsapp::handle_wa_contacts(&self.state, &params.0).await
    }

    #[tool(description = "Edit a previously sent WhatsApp message (within 48h).")]
    pub async fn wa_edit_message(
        &self,
        params: Parameters<tools_whatsapp::WaEditMessageInput>,
    ) -> Result<Json<tools_whatsapp::WaEditMessageOutput>, String> {
        tools_whatsapp::handle_wa_edit_message(&self.state, &params.0).await
    }

    #[tool(description = "Delete/revoke a WhatsApp message for everyone in the chat.")]
    pub async fn wa_revoke_message(
        &self,
        params: Parameters<tools_whatsapp::WaRevokeMessageInput>,
    ) -> Result<Json<tools_whatsapp::WaRevokeMessageOutput>, String> {
        tools_whatsapp::handle_wa_revoke_message(&self.state, &params.0).await
    }

    #[tool(description = "List the WhatsApp groups the user belongs to.")]
    pub async fn wa_list_groups(&self) -> Result<Json<tools_whatsapp::WaListGroupsOutput>, String> {
        tools_whatsapp::handle_wa_list_groups(&self.state).await
    }

    #[tool(description = "Create a new WhatsApp group with the given name and participants.")]
    pub async fn wa_create_group(
        &self,
        params: Parameters<tools_whatsapp::WaCreateGroupInput>,
    ) -> Result<Json<tools_whatsapp::WaCreateGroupOutput>, String> {
        tools_whatsapp::handle_wa_create_group(&self.state, &params.0).await
    }

    #[tool(description = "Get the invite link for a WhatsApp group.")]
    pub async fn wa_group_invite_link(
        &self,
        params: Parameters<tools_whatsapp::WaGroupInviteLinkInput>,
    ) -> Result<Json<tools_whatsapp::WaGroupInviteLinkOutput>, String> {
        tools_whatsapp::handle_wa_group_invite_link(&self.state, &params.0).await
    }

    // ── WhatsApp Cloud API (E6) ───────────────────────────────

    #[tool(description = "List WhatsApp Cloud message templates (business numbers).")]
    pub async fn wc_list_templates(
        &self,
        params: Parameters<tools_whatsapp::WcListTemplatesInput>,
    ) -> Result<Json<tools_whatsapp::WcListTemplatesOutput>, String> {
        tools_whatsapp::handle_wc_list_templates(&self.state, &params.0).await
    }

    #[tool(
        description = "Create a WhatsApp Cloud message template (Marketing/Utility/Authentication; components is the raw Meta component array)."
    )]
    pub async fn wc_create_template(
        &self,
        params: Parameters<tools_whatsapp::WcCreateTemplateInput>,
    ) -> Result<Json<tools_whatsapp::WcCreateTemplateOutput>, String> {
        tools_whatsapp::handle_wc_create_template(&self.state, &params.0).await
    }

    #[tool(description = "Delete a WhatsApp Cloud message template by name.")]
    pub async fn wc_delete_template(
        &self,
        params: Parameters<tools_whatsapp::WcDeleteTemplateInput>,
    ) -> Result<Json<tools_whatsapp::WcDeleteTemplateOutput>, String> {
        tools_whatsapp::handle_wc_delete_template(&self.state, &params.0).await
    }

    #[tool(
        description = "Send a WhatsApp Cloud template message (the compliant outbound path outside the 24h window)."
    )]
    pub async fn wc_send_template(
        &self,
        params: Parameters<tools_whatsapp::WcSendTemplateInput>,
    ) -> Result<Json<tools_whatsapp::WcSendTemplateOutput>, String> {
        tools_whatsapp::handle_wc_send_template(&self.state, &params.0).await
    }

    // ── WordPress Tools ───────────────────────────────────────────────

    #[tool(
        description = "Create a new WordPress post. Requires a connected WordPress integration (site URL + Application Password)."
    )]
    pub async fn wp_create_post(
        &self,
        params: Parameters<tools_wordpress::WpCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_wordpress::handle_wp_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List WordPress posts with optional status filter and pagination.")]
    pub async fn wp_list_posts(
        &self,
        params: Parameters<tools_wordpress::WpListPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_wordpress::handle_wp_list_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single WordPress post by ID.")]
    pub async fn wp_get_post(
        &self,
        params: Parameters<tools_wordpress::WpGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_wordpress::handle_wp_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List categories for a WordPress site.")]
    pub async fn wp_list_categories(
        &self,
        params: Parameters<tools_wordpress::WpListCategoriesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_wordpress::handle_wp_list_categories(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Threads Tools ─────────────────────────────────────────────────

    #[tool(description = "Get your Threads profile.")]
    pub async fn th_get_profile(
        &self,
        params: Parameters<tools_threads::ThreadsGetProfileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_get_profile(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List the Threads posts on the connected account.")]
    pub async fn th_get_threads(
        &self,
        params: Parameters<tools_threads::ThreadsGetThreadsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_get_threads(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a specific Threads thread (post).")]
    pub async fn th_get_thread_detail(
        &self,
        params: Parameters<tools_threads::ThreadsGetThreadDetailInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_get_thread_detail(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get replies on a Threads thread.")]
    pub async fn th_get_replies(
        &self,
        params: Parameters<tools_threads::ThreadsGetRepliesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_get_replies(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to a Threads thread.")]
    pub async fn th_reply_to_thread(
        &self,
        params: Parameters<tools_threads::ThreadsReplyToThreadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_reply_to_thread(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create and publish a new Threads post (text, image, or video).")]
    pub async fn th_create_thread(
        &self,
        params: Parameters<tools_threads::ThreadsCreateThreadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_create_thread(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a Threads thread (post).")]
    pub async fn th_delete_thread(
        &self,
        params: Parameters<tools_threads::ThreadsDeleteThreadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_delete_thread(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get the insights and analytics for the connected Threads account.")]
    pub async fn th_get_insights(
        &self,
        params: Parameters<tools_threads::ThreadsGetInsightsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_get_insights(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Poll publish status of a Threads creation ID.")]
    pub async fn th_poll_publish_status(
        &self,
        params: Parameters<tools_threads::ThreadsPollStatusInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_threads::handle_th_poll_publish_status(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── LinkedIn Personal Tools ─────────────────────────────────

    #[tool(description = "Get your LinkedIn profile information (name, headline, avatar).")]
    pub async fn li_get_profile(
        &self,
        params: Parameters<tools_linkedin::LiGetProfileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_profile(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "List LinkedIn posts for an author URN (e.g. urn:li:person:abc). Use this to fetch your recent posts."
    )]
    pub async fn li_get_posts(
        &self,
        params: Parameters<tools_linkedin::LiGetPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get details of a specific LinkedIn post by its URN (e.g. urn:li:activity:xyz)."
    )]
    pub async fn li_get_post_detail(
        &self,
        params: Parameters<tools_linkedin::LiGetPostDetailInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_post_detail(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get comments on a LinkedIn post by post URN.")]
    pub async fn li_get_comments(
        &self,
        params: Parameters<tools_linkedin::LiGetCommentsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_comments(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a comment on a LinkedIn post.")]
    pub async fn li_create_comment(
        &self,
        params: Parameters<tools_linkedin::LiCreateCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_create_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Create a new LinkedIn post (text-only). Content will be published immediately to your LinkedIn profile."
    )]
    pub async fn li_create_post(
        &self,
        params: Parameters<tools_linkedin::LiCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a LinkedIn post by its URN (e.g., urn:li:share:123456).")]
    pub async fn li_delete_post(
        &self,
        params: Parameters<tools_linkedin::LiDeletePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_delete_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get reactions (likes) on a LinkedIn post by its URN.")]
    pub async fn li_get_reactions(
        &self,
        params: Parameters<tools_linkedin::LiGetReactionsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_reactions(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get shares (reposts) of a LinkedIn post by its URN.")]
    pub async fn li_get_shares(
        &self,
        params: Parameters<tools_linkedin::LiGetSharesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_shares(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get analytics for your LinkedIn personal profile (connections count).")]
    pub async fn li_get_analytics(
        &self,
        params: Parameters<tools_linkedin::LiGetAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get analytics for a specific LinkedIn post (likes, comments, shares counts)."
    )]
    pub async fn li_get_post_analytics(
        &self,
        params: Parameters<tools_linkedin::LiGetPostAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_post_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to a LinkedIn comment by comment ID.")]
    pub async fn li_reply_comment(
        &self,
        params: Parameters<tools_linkedin::LiReplyCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_reply_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Send a direct message on LinkedIn to a recipient URN.")]
    pub async fn li_send_dm(
        &self,
        params: Parameters<tools_linkedin::LiSendDmInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_send_dm(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List LinkedIn message conversations.")]
    pub async fn li_list_conversations(
        &self,
        params: Parameters<tools_linkedin::LiListConversationsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_list_conversations(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get messages in a LinkedIn conversation by conversation ID.")]
    pub async fn li_get_messages(
        &self,
        params: Parameters<tools_linkedin::LiGetMessagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin::handle_li_get_messages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── LinkedIn Page Tools ────────────────────────────────────

    #[tool(
        description = "List LinkedIn company pages you administer. Returns page IDs and names needed for page operations."
    )]
    pub async fn lip_list_pages(
        &self,
        params: Parameters<tools_linkedin_page::LipListPagesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_list_pages(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a LinkedIn company page by its page ID.")]
    pub async fn lip_get_page(
        &self,
        params: Parameters<tools_linkedin_page::LipGetPageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_get_page(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get posts from a LinkedIn company page by page ID.")]
    pub async fn lip_get_page_posts(
        &self,
        params: Parameters<tools_linkedin_page::LipGetPagePostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_get_page_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Create a comment on a LinkedIn post as a company page. Provide the page URN, post URN, and message."
    )]
    pub async fn lip_create_comment(
        &self,
        params: Parameters<tools_linkedin_page::LipCreateCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_create_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a post as a LinkedIn Page (organization).")]
    pub async fn lip_create_post(
        &self,
        params: Parameters<tools_linkedin_page::LipCreatePostInput>,
    ) -> Result<Json<tools_linkedin_page::LipCreatePostOutput>, String> {
        tools_linkedin_page::handle_lip_create_post(&self.state, &params.0).await
    }

    #[tool(
        description = "Get analytics (impressions, clicks, likes, followers) for a LinkedIn Page."
    )]
    pub async fn lip_get_analytics(
        &self,
        params: Parameters<tools_linkedin_page::LipGetAnalyticsInput>,
    ) -> Result<Json<tools_linkedin_page::LipGetAnalyticsOutput>, String> {
        tools_linkedin_page::handle_lip_get_analytics(&self.state, &params.0).await
    }

    #[tool(description = "Get analytics for a specific LinkedIn Page post.")]
    pub async fn lip_get_post_analytics(
        &self,
        params: Parameters<tools_linkedin_page::LipGetPostAnalyticsInput>,
    ) -> Result<Json<tools_linkedin_page::LipGetAnalyticsOutput>, String> {
        tools_linkedin_page::handle_lip_get_post_analytics(&self.state, &params.0).await
    }

    #[tool(description = "Get follower count for a LinkedIn Page.")]
    pub async fn lip_get_followers(
        &self,
        params: Parameters<tools_linkedin_page::LipGetFollowersInput>,
    ) -> Result<Json<tools_linkedin_page::LipGetFollowersOutput>, String> {
        tools_linkedin_page::handle_lip_get_followers(&self.state, &params.0).await
    }

    #[tool(description = "Delete a LinkedIn Page post by its post ID.")]
    pub async fn lip_delete_post(
        &self,
        params: Parameters<tools_linkedin_page::LipDeletePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_delete_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get reactions (likes) on a LinkedIn Page post by its URN.")]
    pub async fn lip_get_reactions(
        &self,
        params: Parameters<tools_linkedin_page::LipGetReactionsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_get_reactions(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get shares (reposts) of a LinkedIn Page post by its URN.")]
    pub async fn lip_get_shares(
        &self,
        params: Parameters<tools_linkedin_page::LipGetSharesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_linkedin_page::handle_lip_get_shares(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Telegram Bot Tools ───────────────────────────────────────
    #[tool(description = "Send a message via Telegram Bot API using a configured bot token.")]
    pub async fn tb_send_message(
        &self,
        params: Parameters<tools_telegram_bot::TbSendMessageInput>,
    ) -> Result<Json<tools_telegram_bot::TbSendMessageOutput>, String> {
        tools_telegram_bot::handle_tb_send_message(&self.state, &params.0).await
    }

    #[tool(description = "Get updates from Telegram Bot API for the connected bot.")]
    pub async fn tb_get_updates(
        &self,
        params: Parameters<tools_telegram_bot::TbGetUpdatesInput>,
    ) -> Result<Json<tools_telegram_bot::TbGetUpdatesOutput>, String> {
        tools_telegram_bot::handle_tb_get_updates(&self.state, &params.0).await
    }

    #[tool(
        description = "Discover chats/channels the bot is in by sweeping pending updates (E8). Returns distinct chats with id, type and title. Consumes pending updates like tb_get_updates; tu_list_dialogs (user session) lists all."
    )]
    pub async fn tb_list_chats(
        &self,
        params: Parameters<tools_telegram_bot::TbListChatsInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_list_chats(&self.state, &params.0).await
    }

    #[tool(
        description = "Get Telegram bot info (username, name, capabilities) for a bot by token_index."
    )]
    pub async fn tb_get_me(
        &self,
        params: Parameters<tools_telegram_bot::TbTokenInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_get_me(&self.state, &params.0).await
    }

    #[tool(description = "Get chat details (title, type, member count) for a Telegram chat.")]
    pub async fn tb_get_chat(
        &self,
        params: Parameters<tools_telegram_bot::TbChatInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_get_chat(&self.state, &params.0).await
    }

    #[tool(description = "Get member count of a Telegram chat/channel.")]
    pub async fn tb_get_chat_member_count(
        &self,
        params: Parameters<tools_telegram_bot::TbChatInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_get_chat_member_count(&self.state, &params.0).await
    }

    #[tool(description = "Send a photo via Telegram Bot to a chat by chat ID.")]
    pub async fn tb_send_photo(
        &self,
        params: Parameters<tools_telegram_bot::TbSendPhotoInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_send_photo(&self.state, &params.0).await
    }

    #[tool(description = "Send a document/file via Telegram Bot to a chat.")]
    pub async fn tb_send_document(
        &self,
        params: Parameters<tools_telegram_bot::TbSendDocumentInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_send_document(&self.state, &params.0).await
    }

    #[tool(description = "Forward a message from one Telegram chat to another.")]
    pub async fn tb_forward_message(
        &self,
        params: Parameters<tools_telegram_bot::TbForwardInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_forward_message(&self.state, &params.0).await
    }

    #[tool(description = "Pin a message in a Telegram chat.")]
    pub async fn tb_pin_message(
        &self,
        params: Parameters<tools_telegram_bot::TbPinInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_pin_message(&self.state, &params.0).await
    }

    #[tool(description = "Unpin a message in a Telegram chat.")]
    pub async fn tb_unpin_message(
        &self,
        params: Parameters<tools_telegram_bot::TbPinInput>,
    ) -> Result<Json<tools_telegram_bot::TbApiOutput>, String> {
        tools_telegram_bot::handle_tb_unpin_message(&self.state, &params.0).await
    }

    // ── Telegram User Tools ──────────────────────────────────────

    #[tool(
        description = "Check Telegram user client authentication status (via telegram-cli daemon)."
    )]
    pub async fn tu_auth_status(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<Json<tools_telegram_user::TuAuthStatusOutput>, String> {
        tools_telegram_user::handle_tu_auth_status(&self.state).await
    }

    #[tool(description = "Send a message via Telegram user client (telegram-cli daemon).")]
    pub async fn tu_send_message(
        &self,
        params: Parameters<tools_telegram_user::TuSendMessageInput>,
    ) -> Result<Json<tools_telegram_user::TuSendMessageOutput>, String> {
        tools_telegram_user::handle_tu_send_message(&self.state, &params.0).await
    }

    #[tool(description = "List dialogs/conversations via the Telegram user client.")]
    pub async fn tu_list_dialogs(
        &self,
        params: Parameters<tools_telegram_user::TuListDialogsInput>,
    ) -> Result<Json<tools_telegram_user::TuListDialogsOutput>, String> {
        tools_telegram_user::handle_tu_list_dialogs(&self.state, &params.0).await
    }

    #[tool(description = "List contacts via Telegram user client.")]
    pub async fn tu_list_contacts(
        &self,
        params: Parameters<tools_telegram_user::TuListContactsInput>,
    ) -> Result<Json<tools_telegram_user::TuListContactsOutput>, String> {
        tools_telegram_user::handle_tu_list_contacts(&self.state, &params.0).await
    }

    #[tool(description = "Search Telegram user client (contacts, dialogs, messages).")]
    pub async fn tu_search(
        &self,
        params: Parameters<tools_telegram_user::TuSearchInput>,
    ) -> Result<Json<tools_telegram_user::TuSearchOutput>, String> {
        tools_telegram_user::handle_tu_search(&self.state, &params.0).await
    }

    #[tool(description = "Request a login code for Telegram user account (Grammers MTProto).")]
    pub async fn tu_request_code(
        &self,
        params: Parameters<tools_telegram_user::TuRequestCodeInput>,
    ) -> Result<Json<tools_telegram_user::TuRequestCodeOutput>, String> {
        tools_telegram_user::handle_tu_request_code(&self.state, &params.0).await
    }

    #[tool(description = "Sign in to Telegram user account with phone number and code.")]
    pub async fn tu_sign_in(
        &self,
        params: Parameters<tools_telegram_user::TuSignInInput>,
    ) -> Result<Json<tools_telegram_user::TuSignInOutput>, String> {
        tools_telegram_user::handle_tu_sign_in(&self.state, &params.0).await
    }

    // ── Telegram User-Session Parity (P6.4, tg-cli port) ───────

    #[tool(
        description = "Fetch a Telegram chat's message history (chronological). Pass offset_id = the previous page's oldest_id to continue back in time."
    )]
    pub async fn tu_history(
        &self,
        params: Parameters<tools_telegram_user::TuHistoryInput>,
    ) -> Result<Json<tools_telegram_user::TuHistoryOutput>, String> {
        tools_telegram_user::handle_tu_history(&self.state, &params.0).await
    }

    #[tool(
        description = "Tail a Telegram chat: latest messages, optionally only newer than after_id (the previous call's newest_id)."
    )]
    pub async fn tu_tail(
        &self,
        params: Parameters<tools_telegram_user::TuTailInput>,
    ) -> Result<Json<tools_telegram_user::TuTailOutput>, String> {
        tools_telegram_user::handle_tu_tail(&self.state, &params.0).await
    }

    #[tool(description = "Edit a message you sent in a Telegram chat.")]
    pub async fn tu_edit(
        &self,
        params: Parameters<tools_telegram_user::TuEditMessageInput>,
    ) -> Result<Json<tools_telegram_user::TuEditMessageOutput>, String> {
        tools_telegram_user::handle_tu_edit_message(&self.state, &params.0).await
    }

    #[tool(description = "Delete a message you sent in a Telegram chat.")]
    pub async fn tu_delete(
        &self,
        params: Parameters<tools_telegram_user::TuDeleteMessageInput>,
    ) -> Result<Json<tools_telegram_user::TuDeleteMessageOutput>, String> {
        tools_telegram_user::handle_tu_delete_message(&self.state, &params.0).await
    }

    #[tool(
        description = "Queue a Telegram send through the rate-limited write queue (paced per Telegram limits; use for bursts). The queue is in-memory — a server restart drops pending sends; use tu_send_message for must-not-be-lost sends."
    )]
    pub async fn tu_queue_send(
        &self,
        params: Parameters<tools_telegram_user::TuQueueSendInput>,
    ) -> Result<Json<tools_telegram_user::TuQueueSendOutput>, String> {
        tools_telegram_user::handle_tu_queue_send(&self.state, &params.0).await
    }

    #[tool(description = "Check the Telegram write queue depth (pending sends).")]
    pub async fn tu_queue_status(
        &self,
    ) -> Result<Json<tools_telegram_user::TuHistoryOutput>, String> {
        tools_telegram_user::handle_tu_queue_status(&self.state).await
    }

    // ── Skool Tools ─────────────────────────────────────────────────

    #[tool(description = "Publish a post to a Skool group.")]
    pub async fn sk_publish(
        &self,
        params: Parameters<tools_skool::SkPublishInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_skool::handle_sk_publish(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get Skool community info (name, description, member count) for a community slug."
    )]
    pub async fn sk_get_info(
        &self,
        params: Parameters<tools_skool::SkGetInfoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_skool::handle_sk_get_info(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List posts in a Skool community with the given group ID.")]
    pub async fn sk_list_posts(
        &self,
        params: Parameters<tools_skool::SkListPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_skool::handle_sk_list_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Skool post by community slug and post slug.")]
    pub async fn sk_get_post(
        &self,
        params: Parameters<tools_skool::SkGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_skool::handle_sk_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a comment on a Skool post.")]
    pub async fn sk_create_comment(
        &self,
        params: Parameters<tools_skool::SkCreateCommentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_skool::handle_sk_create_comment(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Slack Tools ─────────────────────────────────────────────────

    #[tool(description = "Send a message to a Slack channel.")]
    pub async fn sl_send_message(
        &self,
        params: Parameters<tools_slack::SlSendMessageInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_slack::handle_sl_send_message(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all public and private channels in the Slack workspace.")]
    pub async fn sl_list_channels(
        &self,
        params: Parameters<tools_slack::SlListChannelsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_slack::handle_sl_list_channels(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get message history for a Slack channel. Specify channel ID and optional limit."
    )]
    pub async fn sl_channel_history(
        &self,
        params: Parameters<tools_slack::SlChannelHistoryInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_slack::handle_sl_channel_history(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all users in the Slack workspace.")]
    pub async fn sl_list_users(
        &self,
        params: Parameters<tools_slack::SlListUsersInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_slack::handle_sl_list_users(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Bluesky Tools ─────────────────────────────────────────────────

    #[tool(description = "Get a Bluesky user's profile by handle or DID.")]
    pub async fn bs_profile(
        &self,
        params: Parameters<tools_bluesky::BsProfileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_profile(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get the Bluesky home timeline.")]
    pub async fn bs_timeline(
        &self,
        params: Parameters<tools_bluesky::BsTimelineInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_timeline(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a new Bluesky post (text with optional media).")]
    pub async fn bs_create_post(
        &self,
        params: Parameters<tools_bluesky::BsCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search Bluesky for posts and users.")]
    pub async fn bs_search(
        &self,
        params: Parameters<tools_bluesky::BsSearchInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_search(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get the Bluesky feed (popular/recent posts).")]
    pub async fn bs_feed(
        &self,
        params: Parameters<tools_bluesky::BsFeedInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_feed(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to a Bluesky post by post URI.")]
    pub async fn bs_reply(
        &self,
        params: Parameters<tools_bluesky::BsReplyInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_reply(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get account-level analytics for a Bluesky user (followers, following, posts, engagement)"
    )]
    pub async fn bs_get_analytics(
        &self,
        params: Parameters<tools_bluesky::BsGetAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_bluesky::handle_bs_get_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── TikTok Tools ─────────────────────────────────────────────────

    #[tool(description = "Get TikTok user profile info.")]
    pub async fn tt_profile(
        &self,
        params: Parameters<tools_tiktok::TtProfileInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tiktok::handle_tt_profile(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create and publish a TikTok video.")]
    pub async fn tt_create_post(
        &self,
        params: Parameters<tools_tiktok::TtCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tiktok::handle_tt_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List TikTok videos for the connected account.")]
    pub async fn tt_list_videos(
        &self,
        params: Parameters<tools_tiktok::TtListVideosInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tiktok::handle_tt_list_videos(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get account-level analytics for TikTok (followers, likes, video engagement)"
    )]
    pub async fn tt_get_analytics(
        &self,
        params: Parameters<tools_tiktok::TtGetAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tiktok::handle_tt_get_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Mastodon Tools ───────────────────────────────────────────────

    #[tool(
        description = "Create a new Mastodon post (toot) with optional media, visibility settings, and content warnings."
    )]
    pub async fn ms_create_post(
        &self,
        params: Parameters<tools_mastodon::MsCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get Mastodon timeline (home, local, trending, or public).")]
    pub async fn ms_get_timeline(
        &self,
        params: Parameters<tools_mastodon::MsGetTimelineInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_get_timeline(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Mastodon post/status by status ID.")]
    pub async fn ms_get_post(
        &self,
        params: Parameters<tools_mastodon::MsGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Search across Mastodon for posts, users, accounts, statuses, and hashtags."
    )]
    pub async fn ms_search(
        &self,
        params: Parameters<tools_mastodon::MsSearchInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_search(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Reply to a Mastodon status by status ID.")]
    pub async fn ms_reply(
        &self,
        params: Parameters<tools_mastodon::MsReplyInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_reply(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get account-level analytics for Mastodon (followers, following, statuses, engagement)"
    )]
    pub async fn ms_get_analytics(
        &self,
        params: Parameters<tools_mastodon::MsGetAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_mastodon::handle_ms_get_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Medium Tools ─────────────────────────────────────────────────

    #[tool(description = "Create a new Medium story (post).")]
    pub async fn md_create_post(
        &self,
        params: Parameters<tools_medium::MdCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_medium::handle_md_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Medium posts for the connected account.")]
    pub async fn md_list_posts(
        &self,
        params: Parameters<tools_medium::MdListPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_medium::handle_md_list_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Medium post by post ID.")]
    pub async fn md_get_post(
        &self,
        params: Parameters<tools_medium::MdGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_medium::handle_md_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Hashnode Tools ────────────────────────────────────────────────

    #[tool(description = "Create a new Hashnode story (post).")]
    pub async fn hn_create_post(
        &self,
        params: Parameters<tools_hashnode::HnCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_hashnode::handle_hn_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Hashnode posts for a publication by publication ID.")]
    pub async fn hn_list_posts(
        &self,
        params: Parameters<tools_hashnode::HnListPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_hashnode::handle_hn_list_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Hashnode post by post ID.")]
    pub async fn hn_get_post(
        &self,
        params: Parameters<tools_hashnode::HnGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_hashnode::handle_hn_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Dev.to Tools ─────────────────────────────────────────────────

    #[tool(description = "Create a new Dev.to article (post).")]
    pub async fn dv_create_post(
        &self,
        params: Parameters<tools_devto::DvCreatePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_devto::handle_dv_create_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List Dev.to articles for the connected account.")]
    pub async fn dv_list_posts(
        &self,
        params: Parameters<tools_devto::DvListPostsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_devto::handle_dv_list_posts(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single Dev.to article by article ID.")]
    pub async fn dv_get_post(
        &self,
        params: Parameters<tools_devto::DvGetPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_devto::handle_dv_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Analytics Tools ──────────────────────────────────────────────

    #[tool(description = "Get analytics data for a connected social provider.")]
    pub async fn analytics_get(
        &self,
        params: Parameters<tools_analytics::AnalyticsGetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_analytics::handle_analytics_get(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get analytics data for a specific post.")]
    pub async fn analytics_get_post(
        &self,
        params: Parameters<tools_analytics::AnalyticsPostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_analytics::handle_analytics_get_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Feed Tools ──────────────────────────────────────

    #[tool(
        description = "List imported external posts (unified feed). Cursor-paginated. Optionally filter by provider."
    )]
    async fn feed_list(
        &self,
        params: Parameters<tools_feed::FeedListInput>,
    ) -> Result<Json<tools_feed::FeedListOutput>, String> {
        tools_feed::handle_feed_list(&self.state, &params.0).await
    }

    #[tool(
        description = "Trigger immediate import of recent posts from all connected providers. Returns count of new posts imported."
    )]
    async fn feed_import(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<Json<tools_feed::FeedImportOutput>, String> {
        tools_feed::handle_feed_import(&self.state).await
    }

    // ── Agent-surface parity (REST-only resources) ─────────────────────

    #[tool(description = "List campaigns, including archived ones.")]
    pub async fn campaign_list(
        &self,
        params: Parameters<tools_campaigns::CampaignListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_campaigns::handle_campaign_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a campaign with name, dates, goal, status and KPIs.")]
    pub async fn campaign_create(
        &self,
        params: Parameters<tools_campaigns::CampaignCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_campaigns::handle_campaign_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Update a campaign; omitted fields keep their value.")]
    pub async fn campaign_update(
        &self,
        params: Parameters<tools_campaigns::CampaignUpdateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_campaigns::handle_campaign_update(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Archive and soft-delete a campaign.")]
    pub async fn campaign_delete(
        &self,
        params: Parameters<tools_campaigns::CampaignDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_campaigns::handle_campaign_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List post sets (reusable templates bound to channels)")]
    pub async fn set_list(
        &self,
        params: Parameters<tools_sets::SetListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_sets::handle_set_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a post set bound to one or more channels.")]
    pub async fn set_create(
        &self,
        params: Parameters<tools_sets::SetCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_sets::handle_set_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a post set.")]
    pub async fn set_delete(
        &self,
        params: Parameters<tools_sets::SetDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_sets::handle_set_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List RSS autopost feeds.")]
    pub async fn rss_feed_list(
        &self,
        params: Parameters<tools_rss::RssFeedListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Add an RSS feed that autoposts to a channel.")]
    pub async fn rss_feed_create(
        &self,
        params: Parameters<tools_rss::RssFeedCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete an RSS feed.")]
    pub async fn rss_feed_delete(
        &self,
        params: Parameters<tools_rss::RssFeedIdInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Pause or resume an RSS feed.")]
    pub async fn rss_feed_toggle(
        &self,
        params: Parameters<tools_rss::RssFeedIdInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_toggle(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Poll an RSS feed now for new items.")]
    pub async fn rss_feed_poll(
        &self,
        params: Parameters<tools_rss::RssFeedIdInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_poll(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List the items an RSS feed produced and their import state.")]
    pub async fn rss_feed_items(
        &self,
        params: Parameters<tools_rss::RssFeedIdInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_rss::handle_rss_feed_items(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Draft a post with AI from a topic, tone and length.")]
    pub async fn ai_generate_post(
        &self,
        params: Parameters<tools_ai::AiGeneratePostInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_generate_post(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Improve the wording of post text with AI.")]
    pub async fn ai_improve_writing(
        &self,
        params: Parameters<tools_ai::AiContentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_improve_writing(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Suggest hashtags for post text with AI.")]
    pub async fn ai_suggest_hashtags(
        &self,
        params: Parameters<tools_ai::AiContentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_suggest_hashtags(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Rewrite post text in another tone with AI.")]
    pub async fn ai_change_tone(
        &self,
        params: Parameters<tools_ai::AiChangeToneInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_change_tone(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Summarise text with AI.")]
    pub async fn ai_summarize(
        &self,
        params: Parameters<tools_ai::AiContentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_summarize(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Generate several posts on one topic with AI.")]
    pub async fn ai_generate_bulk(
        &self,
        params: Parameters<tools_ai::AiGenerateBulkInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_ai::handle_ai_generate_bulk(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Mark a comment resolved in the local inbox (platform untouched)")]
    pub async fn comment_resolve(
        &self,
        params: Parameters<tools_comments::CommentResolveInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_comments::handle_comment_resolve(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List recent delivery attempts for a webhook.")]
    pub async fn webhook_deliveries(
        &self,
        params: Parameters<tools_webhooks::WhDeliveriesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_deliveries(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get the current posting streak, in consecutive days.")]
    pub async fn streak_get(
        &self,
        params: Parameters<tools_analytics::StreakGetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_analytics::handle_streak_get(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Tags Tools ──────────────────────────────────────────────────

    #[tool(description = "Create a new tag.")]
    pub async fn tag_create(
        &self,
        params: Parameters<tools_tags::TagCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tags::handle_tag_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all tags for the authenticated user.")]
    pub async fn tag_list(
        &self,
        params: Parameters<tools_tags::TagListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tags::handle_tag_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single tag by ID.")]
    pub async fn tag_get(
        &self,
        params: Parameters<tools_tags::TagGetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tags::handle_tag_get(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Update an existing tag.")]
    pub async fn tag_update(
        &self,
        params: Parameters<tools_tags::TagUpdateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tags::handle_tag_update(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a tag.")]
    pub async fn tag_delete(
        &self,
        params: Parameters<tools_tags::TagDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_tags::handle_tag_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Webhooks Tools ──────────────────────────────────────────────

    #[tool(description = "Create a new outgoing webhook.")]
    pub async fn wh_create(
        &self,
        params: Parameters<tools_webhooks::WhCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all outgoing webhooks.")]
    pub async fn wh_list(
        &self,
        params: Parameters<tools_webhooks::WhListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a single webhook by ID.")]
    pub async fn wh_get(
        &self,
        params: Parameters<tools_webhooks::WhGetInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_get(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Update an existing webhook.")]
    pub async fn wh_update(
        &self,
        params: Parameters<tools_webhooks::WhUpdateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_update(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a webhook.")]
    pub async fn wh_delete(
        &self,
        params: Parameters<tools_webhooks::WhDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Test a webhook by sending a sample event.")]
    pub async fn wh_test(
        &self,
        params: Parameters<tools_webhooks::WhTestInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_webhooks::handle_wh_test(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Notification Tools ─────────────────────────────────────

    #[tool(description = "List notifications with optional limit and offset.")]
    async fn notif_list(
        &self,
        params: Parameters<tools_notifications::NotifListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_notifications::handle_notif_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Mark a notification as read by its ID.")]
    async fn notif_mark_read(
        &self,
        params: Parameters<tools_notifications::NotifMarkReadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_notifications::handle_notif_mark_read(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Mark all notifications as read.")]
    async fn notif_mark_all_read(
        &self,
        params: Parameters<tools_notifications::NotifMarkAllReadInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_notifications::handle_notif_mark_all_read(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a notification (for testing or manual alerts).")]
    async fn notif_create(
        &self,
        params: Parameters<tools_notifications::NotifCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_notifications::handle_notif_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── GitHub Tools ─────────────────────────────────────────────

    #[tool(description = "Get the currently authenticated GitHub user.")]
    pub async fn gh_get_authenticated_user(&self) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_authenticated_user(&self.state, &())
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a GitHub user by login username.")]
    pub async fn gh_get_user(
        &self,
        params: Parameters<tools_github::GhGetUserInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_user(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List repositories for a GitHub user or organization.")]
    pub async fn gh_list_repos(
        &self,
        params: Parameters<tools_github::GhListReposInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_repos(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get details of a specific GitHub repository.")]
    pub async fn gh_get_repo(
        &self,
        params: Parameters<tools_github::GhGetRepoInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_repo(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List issues for a GitHub repository.")]
    pub async fn gh_list_issues(
        &self,
        params: Parameters<tools_github::GhListIssuesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_issues(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a specific GitHub issue.")]
    pub async fn gh_get_issue(
        &self,
        params: Parameters<tools_github::GhGetIssueInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_issue(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a new issue in a GitHub repository.")]
    pub async fn gh_create_issue(
        &self,
        params: Parameters<tools_github::GhCreateIssueInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_create_issue(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List pull requests for a GitHub repository.")]
    pub async fn gh_list_pull_requests(
        &self,
        params: Parameters<tools_github::GhListPullRequestsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_pull_requests(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get a specific GitHub pull request.")]
    pub async fn gh_get_pull_request(
        &self,
        params: Parameters<tools_github::GhGetPullRequestInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_pull_request(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List commits for a GitHub repository.")]
    pub async fn gh_list_commits(
        &self,
        params: Parameters<tools_github::GhListCommitsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_commits(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List branches for a GitHub repository.")]
    pub async fn gh_list_branches(
        &self,
        params: Parameters<tools_github::GhListBranchesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_branches(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List releases for a GitHub repository.")]
    pub async fn gh_list_releases(
        &self,
        params: Parameters<tools_github::GhListReleasesInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_releases(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search GitHub repositories by query.")]
    pub async fn gh_search_repos(
        &self,
        params: Parameters<tools_github::GhSearchReposInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_search_repos(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Search code on GitHub by query.")]
    pub async fn gh_search_code(
        &self,
        params: Parameters<tools_github::GhSearchCodeInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_search_code(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List contributors for a GitHub repository.")]
    pub async fn gh_list_contributors(
        &self,
        params: Parameters<tools_github::GhListContributorsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_contributors(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Get file or directory contents from a GitHub repository by path.")]
    pub async fn gh_get_repo_content(
        &self,
        params: Parameters<tools_github::GhGetRepoContentInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_get_repo_content(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Close a GitHub issue.")]
    pub async fn gh_close_issue(
        &self,
        params: Parameters<tools_github::GhCloseIssueInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_close_issue(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List repositories for the authenticated GitHub user.")]
    pub async fn gh_list_my_repos(
        &self,
        params: Parameters<tools_github::GhListMyReposInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_github::handle_gh_list_my_repos(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── Admin/Parity Tools (v9 MCP parity wrappers) ────────────────

    #[tool(
        description = "Set up a recurring/evergreen post. The scheduler creates a new queued copy every interval_days days until end_date. Wraps POST /api/posts/{id}/repeat."
    )]
    pub async fn posts_repeat(
        &self,
        params: Parameters<tools_admin::PostsRepeatInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_posts_repeat(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Attach tags to a post. Replaces any existing tags. Wraps PUT /api/posts/{id}/tags."
    )]
    pub async fn posts_set_tags(
        &self,
        params: Parameters<tools_admin::PostsSetTagsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_posts_set_tags(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Delete a media file (DB row + file on disk). Wraps DELETE /api/media/{id}."
    )]
    pub async fn media_delete(
        &self,
        params: Parameters<tools_admin::MediaDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_media_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Manually refresh the OAuth access token for an integration. Wraps POST /api/integrations/{id}/refresh."
    )]
    pub async fn integrations_refresh(
        &self,
        params: Parameters<tools_admin::IntegrationsRefreshInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_integrations_refresh(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "List all per-platform signatures. Wraps GET /api/signatures.")]
    pub async fn signatures_list(
        &self,
        params: Parameters<tools_admin::SignatureListInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_signatures_list(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Create a new per-platform signature. Wraps POST /api/signatures.")]
    pub async fn signatures_create(
        &self,
        params: Parameters<tools_admin::SignatureCreateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_signatures_create(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Update an existing signature. Wraps PUT /api/signatures/{id}.")]
    pub async fn signatures_update(
        &self,
        params: Parameters<tools_admin::SignatureUpdateInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_signatures_update(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(description = "Delete a signature. Wraps DELETE /api/signatures/{id}.")]
    pub async fn signatures_delete(
        &self,
        params: Parameters<tools_admin::SignatureDeleteInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_signatures_delete(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get aggregate analytics summary: total posts, published/failed/draft/queued counts. Wraps GET /api/analytics/summary."
    )]
    pub async fn analytics_summary(
        &self,
        params: Parameters<tools_admin::AnalyticsSummaryInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_analytics_summary(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    #[tool(
        description = "Get feed-level engagement analytics: total likes/comments/shares/impressions across all imported feed posts. Wraps GET /api/feed/analytics."
    )]
    pub async fn feed_analytics(
        &self,
        params: Parameters<tools_admin::FeedAnalyticsInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        tools_admin::handle_feed_analytics(&self.state, &params.0)
            .await
            .map(|Json(v)| Json(McpJsonValue(v)))
    }

    // ── §5.5 Meta Tools (always enabled) ──────────────────────

    #[tool(
        description = "Search ALL available MCP tools by name/description (including ones not enabled for this session). Use this before tools_enable to discover toolsets."
    )]
    pub async fn tools_search(
        &self,
        params: Parameters<ToolsSearchInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        let q = params.0.query.to_ascii_lowercase();
        let enabled = self.toolsets.get();
        let hits: Vec<serde_json::Value> = Self::tool_router()
            .list_all()
            .into_iter()
            .filter(|t| {
                t.name.to_ascii_lowercase().contains(&q)
                    || t.description
                        .as_ref()
                        .is_some_and(|d| d.to_ascii_lowercase().contains(&q))
            })
            .map(|t| {
                let set = Self::toolset_of(&t.name);
                serde_json::json!({
                    "name": t.name,
                    "set": set,
                    "enabled": enabled.contains("all")
                        || enabled.contains(&set)
                        || Self::META_TOOLS.contains(&t.name.as_ref()),
                    "description": t.description.unwrap_or_default(),
                })
            })
            .collect();
        Ok(Json(McpJsonValue(serde_json::json!({
            "query": params.0.query,
            "matches": hits.len(),
            "tools": hits,
        }))))
    }

    #[tool(
        description = "Enable additional toolsets for this session (e.g. [\"x\", \"reddit\"]). Discover sets with tools_search. Ads are per-session; the boot default is SWITCHPOST_TOOLSETS (default: core)."
    )]
    pub async fn tools_enable(
        &self,
        params: Parameters<ToolsEnableInput>,
    ) -> Result<Json<McpJsonValue>, String> {
        let mut sets_ref = self.toolsets.0.lock().unwrap_or_else(|e| e.into_inner());
        for s in &params.0.sets {
            let s = s.trim().to_ascii_lowercase();
            if !s.is_empty() {
                sets_ref.insert(s);
            }
        }
        let enabled: Vec<String> = sets_ref.iter().cloned().collect();
        Ok(Json(McpJsonValue(serde_json::json!({
            "enabled": enabled,
            "note": "Newly enabled tools are advertised on the next tools/list call.",
        }))))
    }
}

// §5.5: the ServerHandler dispatches through the per-session FILTERED
// router — hidden toolsets disappear from tools/list and tools/call rejects
// them by name.
#[tool_handler(router = self.filtered_tool_router())]
impl rmcp::ServerHandler for SwitchpostMcpServer {}

// ══════════════════════════════════════════════════════════════
// RUNNER
// ══════════════════════════════════════════════════════════════

/// Crate-visible accessor for the (privately generated) tool router —
/// `mcp-call` (§5.3) dispatches tools in-process through it.
impl SwitchpostMcpServer {
    pub fn router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        Self::tool_router()
    }
}

/// Start the MCP server on stdio (for AI clients that spawn the binary)
pub async fn run_mcp_stdio(state: AppState) -> anyhow::Result<()> {
    let server = SwitchpostMcpServer::new(state);
    let service = server.serve(stdio()).await?;
    tracing::info!("MCP server started on stdio");
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EmptyParams;

    // A no-arg MCP tool must accept the EMPTY ARGUMENT MAP. rmcp's
    // `Parameters<P>` extractor turns absent arguments into `{}` and
    // deserializes THAT as `P`, so `P = ()` failed every call with
    // "invalid type: map, expected unit" — in every MCP client, not just
    // the CLI. This pins the contract that the empty map is accepted.
    #[test]
    fn empty_params_accepts_the_empty_argument_map() {
        let parsed: EmptyParams =
            serde_json::from_value(serde_json::json!({})).expect("empty map must deserialize");
        assert!(parsed._ignored.is_empty());
    }

    #[test]
    fn empty_params_ignores_stray_arguments() {
        // An agent that passes an extra key must not break the call.
        let parsed: EmptyParams = serde_json::from_value(serde_json::json!({"stray": 1}))
            .expect("stray key must deserialize");
        assert_eq!(parsed._ignored.len(), 1);
    }
}
