// ─── MCP Setup/Onboarding Tools ──────────────────────────────
// Tools for AI agents to guide users through switchpost setup:
// - Check overall setup status
// - Get guided setup instructions per provider
// - Import browser cookies programmatically
// - Set configuration values

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

// ── Input/Output Types ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatusInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatusOutput {
    pub ready: bool,
    pub database_ok: bool,
    pub user_exists: bool,
    pub connected_providers: Vec<String>,
    pub cookie_providers: CookieProviderStatus,
    pub env_providers: Vec<EnvProviderStatus>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CookieProviderStatus {
    pub x_browser_cookies: bool,
    pub reddit_browser_cookies: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EnvProviderStatus {
    pub name: String,
    pub env_key: String,
    pub configured: bool,
    pub connected: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportCookiesInput {
    /// Provider to import cookies for ("x", "reddit", or "all")
    pub provider: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportCookiesOutput {
    pub results: Vec<ImportResult>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportResult {
    pub provider: String,
    pub status: String,
    pub name: Option<String>,
    pub source: Option<String>,
    pub error: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSetInput {
    /// Environment variable name (e.g. BLUESKY_HANDLE, GITHUB_TOKEN)
    pub key: String,
    /// Value to set
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfigGetInput {
    /// Environment variable name
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfigListOutput {
    pub path: String,
    pub exists: bool,
    pub entries: Vec<ConfigEntry>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetupGuideInput {
    /// Optional provider name to get specific guidance. If omitted, returns all providers.
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProviderGuide {
    pub identifier: String,
    pub name: String,
    pub auth_method: String,
    pub description: String,
    pub env_vars: Vec<EnvVarInfo>,
    pub setup_steps: Vec<String>,
    pub credential_request_url: Option<String>,
    pub oauth_redirect_uri: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EnvVarInfo {
    pub name: String,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetupGuideOutput {
    pub providers: Vec<ProviderGuide>,
}

// ── Tool Implementations ────────────────────────────────────

/// Get setup guidance for connecting social media providers.
/// Returns what credentials are needed, where to get them, and how to configure them.
/// AI agents use this to guide users through the full onboarding process.
pub fn setup_guide(
    state: &AppState,
    input: &SetupGuideInput,
) -> Result<Json<SetupGuideOutput>, String> {
    let app_url = &state.config.app_url;

    let all_guides = vec![
        // ── Cookie-based providers ──────────────────────────────
        ProviderGuide {
            identifier: "x".into(),
            name: "X (Twitter)".into(),
            auth_method: "browser_cookies".into(),
            description: "Full access via browser session cookies. Supports posting, liking, retweeting, bookmarks, DMs, and more.".into(),
            env_vars: vec![],
            setup_steps: vec![
                "1. Log into x.com in Chrome, Brave, Firefox, or Zen browser".into(),
                "2. Run 'switchpost connect x' to auto-import cookies".into(),
                "3. Or use the MCP import_cookies tool with provider='x'".into(),
                "4. Alternatively, set X_AUTH_TOKEN + X_CT0 in ~/.switchpost/.env".into(),
                format!("5. Or visit the web UI at {app_url}/api/public/connect/x-cookies for manual entry"),
            ],
            credential_request_url: None,
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "reddit".into(),
            name: "Reddit".into(),
            auth_method: "browser_cookies".into(),
            description: "Full access via browser session cookies. Supports browsing, posting, commenting, voting, saving, moderation, and inbox.".into(),
            env_vars: vec![],
            setup_steps: vec![
                "1. Log into reddit.com in Chrome, Brave, Firefox, or Zen browser".into(),
                "2. Run 'switchpost connect reddit' to auto-import cookies".into(),
                "3. Or use the MCP import_cookies tool with provider='reddit'".into(),
                format!("4. Or visit the web UI at {app_url}/api/public/connect/reddit-cookies for manual entry"),
            ],
            credential_request_url: None,
            oauth_redirect_uri: None,
        },

        // ── OAuth providers (Meta) ──────────────────────────────
        ProviderGuide {
            identifier: "facebook".into(),
            name: "Facebook Pages".into(),
            auth_method: "oauth".into(),
            description: "Manage Facebook Pages: post, comment, react, insights, messaging, albums.".into(),
            env_vars: vec![
                EnvVarInfo { name: "FACEBOOK_CLIENT_ID".into(), description: "Meta App ID from developers.facebook.com".into(), required: true },
                EnvVarInfo { name: "FACEBOOK_CLIENT_SECRET".into(), description: "Meta App Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create a Meta app at https://developers.facebook.com/apps/".into(),
                "2. Set FACEBOOK_CLIENT_ID and FACEBOOK_CLIENT_SECRET in ~/.switchpost/.env".into(),
                "3. Run 'switchpost connect facebook' or visit the web UI".into(),
                "4. Authorize the Facebook Pages OAuth flow".into(),
            ],
            credential_request_url: Some("https://developers.facebook.com/apps/".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "instagram".into(),
            // "Instagram Business" is what the MCP guide used to call this,
            // and it is the name that made the founder report the option as
            // MISSING: the onboarding text and the AI-agent guide said
            // "Instagram Business", the Channels grid card says "Instagram",
            // so the name they were sent to find does not exist in the place
            // they were sent to look. Follow the LinkedIn pair instead, where
            // the personal mode is "LinkedIn Personal" and the card in the
            // grid is the same company's product: this entry is the
            // business-mode Instagram reached through the Facebook login
            // flow, and the standalone variant is its own card. Same
            // product, same qualification, one spelling across surfaces.
            name: "Instagram (Business)".into(),
            auth_method: "oauth".into(),
            description: "Instagram Business/Creator accounts, connected through the Facebook login flow: media, reels, stories, insights, comments, hashtags, mentions.".into(),
            env_vars: vec![
                EnvVarInfo { name: "INSTAGRAM_CLIENT_ID".into(), description: "Meta App ID (same as Facebook)".into(), required: true },
                EnvVarInfo { name: "INSTAGRAM_CLIENT_SECRET".into(), description: "Meta App Secret (same as Facebook)".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create a Meta app at https://developers.facebook.com/apps/".into(),
                "2. Enable Instagram Graph API product".into(),
                "3. Set INSTAGRAM_CLIENT_ID and INSTAGRAM_CLIENT_SECRET in ~/.switchpost/.env".into(),
                "4. Run 'switchpost connect instagram' or visit the web UI".into(),
            ],
            credential_request_url: Some("https://developers.facebook.com/apps/".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "threads".into(),
            name: "Threads (Meta)".into(),
            auth_method: "oauth".into(),
            description: "Threads posting, insights, replies, and analytics.".into(),
            env_vars: vec![
                EnvVarInfo { name: "THREADS_APP_ID".into(), description: "Meta App ID".into(), required: true },
                EnvVarInfo { name: "THREADS_APP_SECRET".into(), description: "Meta App Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create a Meta app at https://developers.facebook.com/apps/".into(),
                "2. Enable Threads API product".into(),
                "3. Set THREADS_APP_ID and THREADS_APP_SECRET in ~/.switchpost/.env".into(),
            ],
            credential_request_url: Some("https://developers.facebook.com/apps/".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },

        // ── OAuth providers (LinkedIn) ──────────────────────────
        ProviderGuide {
            identifier: "linkedin".into(),
            name: "LinkedIn Personal".into(),
            auth_method: "oauth".into(),
            description: "LinkedIn personal profile: post, get profile, analytics, reactions, comments.".into(),
            env_vars: vec![
                EnvVarInfo { name: "LINKEDIN_CLIENT_ID".into(), description: "LinkedIn OAuth Client ID from linkedin.com/developers".into(), required: true },
                EnvVarInfo { name: "LINKEDIN_CLIENT_SECRET".into(), description: "LinkedIn OAuth Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://www.linkedin.com/developers/apps".into(),
                "2. Request the 'Sign In with LinkedIn using OpenID Connect' product".into(),
                "3. Set LINKEDIN_CLIENT_ID and LINKEDIN_CLIENT_SECRET in ~/.switchpost/.env".into(),
                "4. Run 'switchpost connect linkedin' or visit the web UI".into(),
            ],
            credential_request_url: Some("https://www.linkedin.com/developers/apps".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "linkedin-page".into(),
            name: "LinkedIn Company Pages".into(),
            auth_method: "oauth".into(),
            description: "LinkedIn company pages: post, analytics, followers, comments.".into(),
            env_vars: vec![
                EnvVarInfo { name: "LINKEDIN_CLIENT_ID".into(), description: "LinkedIn OAuth Client ID".into(), required: true },
                EnvVarInfo { name: "LINKEDIN_CLIENT_SECRET".into(), description: "LinkedIn OAuth Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://www.linkedin.com/developers/apps".into(),
                "2. Request 'Share on LinkedIn' and 'Sign In with LinkedIn' products".into(),
                "3. Set LINKEDIN_CLIENT_ID and LINKEDIN_CLIENT_SECRET".into(),
                "4. Authorize via the web UI — pages are auto-discovered".into(),
            ],
            credential_request_url: Some("https://www.linkedin.com/developers/apps".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },

        // ── Direct credential providers ─────────────────────────
        ProviderGuide {
            identifier: "bluesky".into(),
            name: "Bluesky".into(),
            auth_method: "env_vars".into(),
            description: "Bluesky posting, timeline, search, and feed.".into(),
            env_vars: vec![
                EnvVarInfo { name: "BLUESKY_HANDLE".into(), description: "Your Bluesky handle (e.g. user.bsky.social)".into(), required: true },
                EnvVarInfo { name: "BLUESKY_APP_PASSWORD".into(), description: "Bluesky app password (generate in Settings > Advanced > App Passwords)".into(), required: true },
            ],
            setup_steps: vec![
                "1. Go to Bluesky Settings > Advanced > App Passwords".into(),
                "2. Generate a new app password".into(),
                "3. Set BLUESKY_HANDLE and BLUESKY_APP_PASSWORD in ~/.switchpost/.env".into(),
            ],
            credential_request_url: None,
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "github".into(),
            name: "GitHub".into(),
            auth_method: "env_vars".into(),
            description: "GitHub repos, issues, PRs, commits, branches, releases, search.".into(),
            env_vars: vec![
                EnvVarInfo { name: "GITHUB_TOKEN".into(), description: "GitHub Personal Access Token (fine-grained recommended)".into(), required: true },
            ],
            setup_steps: vec![
                "1. Go to https://github.com/settings/tokens".into(),
                "2. Generate a fine-grained Personal Access Token".into(),
                "3. Grant permissions: repo, read:org, read:user".into(),
                "4. Set GITHUB_TOKEN in ~/.switchpost/.env".into(),
            ],
            credential_request_url: Some("https://github.com/settings/tokens".into()),
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "telegram-bot".into(),
            name: "Telegram Bot".into(),
            auth_method: "env_vars".into(),
            description: "Telegram bot messaging: send messages, photos, documents, polls, manage groups.".into(),
            env_vars: vec![
                EnvVarInfo { name: "TELEGRAM_BOT_TOKENS".into(), description: "Bot token(s) from @BotFather, comma-separated for multiple bots".into(), required: true },
            ],
            setup_steps: vec![
                "1. Message @BotFather on Telegram: /newbot".into(),
                "2. Copy the bot token".into(),
                "3. Set TELEGRAM_BOT_TOKENS in ~/.switchpost/.env".into(),
                "4. For multiple bots, comma-separate tokens".into(),
            ],
            credential_request_url: Some("https://t.me/BotFather".into()),
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "discord".into(),
            name: "Discord".into(),
            auth_method: "oauth".into(),
            description: "Discord server management: channels, messages, reactions, forum posts, threads.".into(),
            env_vars: vec![
                EnvVarInfo { name: "DISCORD_CLIENT_ID".into(), description: "Discord Application Client ID".into(), required: true },
                EnvVarInfo { name: "DISCORD_CLIENT_SECRET".into(), description: "Discord Application Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://discord.com/developers/applications".into(),
                "2. Set DISCORD_CLIENT_ID and DISCORD_CLIENT_SECRET in ~/.switchpost/.env".into(),
                "3. Add bot to your server with required permissions".into(),
            ],
            credential_request_url: Some("https://discord.com/developers/applications".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "slack".into(),
            name: "Slack".into(),
            auth_method: "oauth".into(),
            description: "Slack workspace: channels, messages, users, search.".into(),
            env_vars: vec![
                EnvVarInfo { name: "SLACK_CLIENT_ID".into(), description: "Slack App Client ID".into(), required: true },
                EnvVarInfo { name: "SLACK_CLIENT_SECRET".into(), description: "Slack App Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://api.slack.com/apps".into(),
                "2. Set SLACK_CLIENT_ID and SLACK_CLIENT_SECRET in ~/.switchpost/.env".into(),
                "3. Add OAuth scopes: channels:read, chat:write, users:read".into(),
                "4. Install to workspace via the web UI".into(),
            ],
            credential_request_url: Some("https://api.slack.com/apps".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "pinterest".into(),
            name: "Pinterest".into(),
            auth_method: "oauth".into(),
            description: "Pinterest boards, pins, analytics, search.".into(),
            env_vars: vec![
                EnvVarInfo { name: "PINTEREST_CLIENT_ID".into(), description: "Pinterest App ID".into(), required: true },
                EnvVarInfo { name: "PINTEREST_CLIENT_SECRET".into(), description: "Pinterest App Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://developers.pinterest.com/apps/".into(),
                "2. Set PINTEREST_CLIENT_ID and PINTEREST_CLIENT_SECRET".into(),
                "3. Complete OAuth via the web UI".into(),
            ],
            credential_request_url: Some("https://developers.pinterest.com/apps/".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "tiktok".into(),
            name: "TikTok".into(),
            auth_method: "oauth".into(),
            description: "TikTok video posting, profile, and video listing.".into(),
            env_vars: vec![
                EnvVarInfo { name: "TIKTOK_CLIENT_ID".into(), description: "TikTok Developer Client Key".into(), required: true },
                EnvVarInfo { name: "TIKTOK_CLIENT_SECRET".into(), description: "TikTok Developer Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create an app at https://developers.tiktok.com/".into(),
                "2. Set TIKTOK_CLIENT_ID and TIKTOK_CLIENT_SECRET".into(),
                "3. Complete OAuth via the web UI".into(),
            ],
            credential_request_url: Some("https://developers.tiktok.com/".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "mastodon".into(),
            name: "Mastodon".into(),
            auth_method: "oauth".into(),
            description: "Mastodon posting, timeline, search. Works with any Mastodon instance.".into(),
            env_vars: vec![
                EnvVarInfo { name: "MASTODON_CLIENT_ID".into(), description: "Mastodon app client ID".into(), required: true },
                EnvVarInfo { name: "MASTODON_CLIENT_SECRET".into(), description: "Mastodon app client secret".into(), required: true },
                EnvVarInfo { name: "MASTODON_INSTANCE_URL".into(), description: "Mastodon instance URL (e.g. https://mastodon.social)".into(), required: true },
            ],
            setup_steps: vec![
                "1. Register an app on your Mastodon instance: https://YOUR_INSTANCE/api/v1/apps".into(),
                "2. Set MASTODON_CLIENT_ID, MASTODON_CLIENT_SECRET, MASTODON_INSTANCE_URL".into(),
                "3. Complete OAuth via the web UI".into(),
            ],
            credential_request_url: None,
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "youtube".into(),
            name: "YouTube".into(),
            auth_method: "oauth".into(),
            description: "YouTube videos, playlists, comments, analytics, channel stats.".into(),
            env_vars: vec![
                EnvVarInfo { name: "YOUTUBE_CLIENT_ID".into(), description: "Google Cloud OAuth Client ID".into(), required: true },
                EnvVarInfo { name: "YOUTUBE_CLIENT_SECRET".into(), description: "Google Cloud OAuth Client Secret".into(), required: true },
            ],
            setup_steps: vec![
                "1. Create a project at https://console.cloud.google.com/".into(),
                "2. Enable YouTube Data API v3".into(),
                "3. Create OAuth 2.0 credentials".into(),
                "4. Set YOUTUBE_CLIENT_ID and YOUTUBE_CLIENT_SECRET".into(),
            ],
            credential_request_url: Some("https://console.cloud.google.com/apis/credentials".into()),
            oauth_redirect_uri: Some(format!("{app_url}/api/auth/callback")),
        },
        ProviderGuide {
            identifier: "wordpress".into(),
            name: "WordPress".into(),
            auth_method: "per_site".into(),
            description: "WordPress posting and management via REST API with Application Passwords.".into(),
            env_vars: vec![],
            setup_steps: vec![
                "1. In WordPress Admin > Users > Edit Profile, enable Application Passwords".into(),
                "2. Create an application password".into(),
                "3. Connect via the web UI with site URL, username, and application password".into(),
            ],
            credential_request_url: None,
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "medium".into(),
            name: "Medium".into(),
            auth_method: "env_vars".into(),
            description: "Medium article publishing.".into(),
            env_vars: vec![
                EnvVarInfo { name: "MEDIUM_ACCESS_TOKEN".into(), description: "Medium API integration token".into(), required: true },
            ],
            setup_steps: vec![
                "1. Go to https://medium.com/me/settings/security".into(),
                "2. Create an integration token".into(),
                "3. Set MEDIUM_ACCESS_TOKEN in ~/.switchpost/.env".into(),
            ],
            credential_request_url: Some("https://medium.com/me/settings/security".into()),
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "devto".into(),
            name: "Dev.to".into(),
            auth_method: "env_vars".into(),
            description: "Dev.to article publishing.".into(),
            env_vars: vec![
                EnvVarInfo { name: "DEVTO_API_KEY".into(), description: "Dev.to API key".into(), required: true },
            ],
            setup_steps: vec![
                "1. Go to https://dev.to/settings/extensions".into(),
                "2. Generate a new API key".into(),
                "3. Set DEVTO_API_KEY in ~/.switchpost/.env".into(),
            ],
            credential_request_url: Some("https://dev.to/settings/extensions".into()),
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "hashnode".into(),
            name: "Hashnode".into(),
            auth_method: "env_vars".into(),
            description: "Hashnode article publishing.".into(),
            env_vars: vec![
                EnvVarInfo { name: "HASHNODE_API_KEY".into(), description: "Hashnode Personal Access Token".into(), required: true },
            ],
            setup_steps: vec![
                "1. Go to https://hashnode.com/settings/integrations".into(),
                "2. Generate a Personal Access Token".into(),
                "3. Set HASHNODE_API_KEY in ~/.switchpost/.env".into(),
            ],
            credential_request_url: Some("https://hashnode.com/settings/integrations".into()),
            oauth_redirect_uri: None,
        },
        ProviderGuide {
            identifier: "skool".into(),
            name: "Skool".into(),
            auth_method: "chrome_extension".into(),
            description: "Skool community posting and browsing via Chrome extension cookie extraction.".into(),
            env_vars: vec![],
            setup_steps: vec![
                "1. Install the Skool Chrome extension".into(),
                "2. Log into skool.com in your browser".into(),
                "3. The extension extracts session cookies automatically".into(),
            ],
            credential_request_url: None,
            oauth_redirect_uri: None,
        },
    ];

    // Filter by provider if specified
    let guides = if let Some(ref requested) = input.provider {
        all_guides
            .into_iter()
            .filter(|g| g.identifier == *requested)
            .collect()
    } else {
        all_guides
    };

    Ok(Json(SetupGuideOutput { providers: guides }))
}

/// Check the overall setup status of switchpost.
/// Returns database health, user existence, connected providers, and next actions.
pub async fn setup_status(
    state: &AppState,
    _input: &SetupStatusInput,
) -> Result<Json<SetupStatusOutput>, String> {
    // Database check
    let database_ok = sqlx::query("SELECT 1")
        .fetch_optional(&state.db)
        .await
        .is_ok();

    // User check
    let user_exists = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .is_some();

    // Connected providers
    let user_id_opt = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

    let connected_providers: Vec<String> = if let Some(uid) = user_id_opt {
        crate::db::queries::list_integrations(&state.db, uid)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.provider_identifier)
            .collect()
    } else {
        Vec::new()
    };

    // Cookie browser status
    let cookie_providers = CookieProviderStatus {
        x_browser_cookies: crate::social::x_cookies::extract_x_cookies().is_some(),
        reddit_browser_cookies: crate::social::reddit_cookies::extract_reddit_cookies().is_some(),
    };

    // Env-var providers
    let env_providers = vec![
        EnvProviderStatus {
            name: "X/Twitter (cookies)".into(),
            env_key: "browser_cookies".into(),
            configured: crate::social::x_cookies::extract_x_cookies().is_some(),
            connected: connected_providers.contains(&"x".to_string()),
        },
        EnvProviderStatus {
            name: "Reddit (cookies)".into(),
            env_key: "browser_cookies".into(),
            configured: crate::social::reddit_cookies::extract_reddit_cookies().is_some(),
            connected: connected_providers.contains(&"reddit".to_string()),
        },
        EnvProviderStatus {
            name: "LinkedIn".into(),
            env_key: "LINKEDIN_CLIENT_ID".into(),
            configured: state.config.linkedin_client_id.is_some(),
            connected: connected_providers
                .iter()
                .any(|p| p.starts_with("linkedin")),
        },
        EnvProviderStatus {
            name: "Facebook".into(),
            env_key: "FACEBOOK_CLIENT_ID".into(),
            configured: state.config.facebook_client_id.is_some(),
            connected: connected_providers.contains(&"facebook".to_string()),
        },
        EnvProviderStatus {
            name: "Instagram".into(),
            env_key: "INSTAGRAM_CLIENT_ID".into(),
            configured: state.config.instagram_client_id.is_some(),
            connected: connected_providers.contains(&"instagram".to_string()),
        },
        EnvProviderStatus {
            name: "Bluesky".into(),
            env_key: "BLUESKY_HANDLE".into(),
            configured: state.config.bluesky_handle.is_some(),
            connected: connected_providers.contains(&"bluesky".to_string()),
        },
        EnvProviderStatus {
            name: "GitHub".into(),
            env_key: "GITHUB_TOKEN".into(),
            configured: state.config.github_token.is_some(),
            connected: connected_providers.contains(&"github".to_string()),
        },
        EnvProviderStatus {
            name: "Telegram Bot".into(),
            env_key: "TELEGRAM_BOT_TOKENS".into(),
            configured: state.config.telegram_bot_tokens.is_some(),
            connected: connected_providers.contains(&"telegram-bot".to_string()),
        },
        EnvProviderStatus {
            name: "Discord Bot".into(),
            env_key: "DISCORD_BOT_TOKEN".into(),
            configured: state.config.discord_bot_token.is_some(),
            connected: connected_providers.contains(&"discord".to_string()),
        },
        EnvProviderStatus {
            name: "Dev.to".into(),
            env_key: "DEVTO_API_KEY".into(),
            configured: state.config.devto_api_key.is_some(),
            connected: connected_providers.contains(&"devto".to_string()),
        },
        EnvProviderStatus {
            name: "Medium".into(),
            env_key: "MEDIUM_ACCESS_TOKEN".into(),
            configured: state.config.medium_access_token.is_some(),
            connected: connected_providers.contains(&"medium".to_string()),
        },
        EnvProviderStatus {
            name: "Hashnode".into(),
            env_key: "HASHNODE_API_KEY".into(),
            configured: state.config.hashnode_api_key.is_some(),
            connected: connected_providers.contains(&"hashnode".to_string()),
        },
    ];

    // Next actions
    let mut next_actions = Vec::new();
    if !user_exists {
        next_actions.push("Register a user via the web UI or API (POST /api/auth/register)".into());
    }
    if !cookie_providers.x_browser_cookies && !connected_providers.contains(&"x".to_string()) {
        next_actions.push(
            "Log into x.com in your browser, then use the import_cookies tool for provider='x'"
                .into(),
        );
    }
    if !cookie_providers.reddit_browser_cookies
        && !connected_providers.contains(&"reddit".to_string())
    {
        next_actions.push("Log into reddit.com in your browser, then use the import_cookies tool for provider='reddit'".into());
    }
    let unconfigured_oauth: Vec<&str> = env_providers
        .iter()
        .filter(|p| {
            !p.connected && matches!(p.name.as_str(), "LinkedIn" | "Facebook" | "Instagram")
        })
        .filter(|p| !p.configured)
        .map(|p| p.name.as_str())
        .collect();
    if !unconfigured_oauth.is_empty() {
        next_actions.push(format!("Set OAuth credentials for: {}. Use 'switchpost config set KEY VALUE' or visit the web UI.", unconfigured_oauth.join(", ")));
    }

    let ready = database_ok && user_exists && !connected_providers.is_empty();

    Ok(Json(SetupStatusOutput {
        ready,
        database_ok,
        user_exists,
        connected_providers,
        cookie_providers,
        env_providers,
        next_actions,
    }))
}

/// Import browser cookies for cookie-based providers (X/Twitter, Reddit).
/// Automatically detects Chrome, Brave (including Origin-Beta), Firefox, and Zen browsers.
pub async fn import_cookies(
    state: &AppState,
    input: &ImportCookiesInput,
) -> Result<Json<ImportCookiesOutput>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let mut results = Vec::new();

    let providers_to_import = if input.provider == "all" {
        vec!["x", "reddit"]
    } else {
        vec![input.provider.as_str()]
    };

    for provider in providers_to_import {
        match provider {
            "x" => {
                // Check if already connected
                let integrations = crate::db::queries::list_integrations(&state.db, user_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if integrations.iter().any(|i| i.provider_identifier == "x") {
                    results.push(ImportResult {
                        provider: "x".into(),
                        status: "already_connected".into(),
                        name: None,
                        source: None,
                        error: None,
                        hint: Some("X is already connected. Use x_get_me to verify.".into()),
                    });
                    continue;
                }

                match crate::social::x_cookies::extract_x_cookies() {
                    Some(cookies) => {
                        let token_str = crate::social::x_cookies::build_cookie_token(
                            &cookies.auth_token,
                            &cookies.ct0,
                            Some(&cookies.cookie_string),
                        );
                        let mut provider_obj = crate::social::x::XProvider::new(&state.config);
                        provider_obj.prepare_from_token(&token_str);
                        match provider_obj.get_me(&token_str).await {
                            Ok(json) => {
                                let data = json.get("data");
                                let name = data
                                    .and_then(|d| d.get("name"))
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("X User")
                                    .to_string();
                                let id = data
                                    .and_then(|d| d.get("id"))
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let avatar = data
                                    .and_then(|d| d.get("profile_image_url"))
                                    .and_then(|s| s.as_str());
                                match crate::db::queries::create_integration(
                                    &state.db,
                                    user_id,
                                    "x",
                                    "X (Twitter)",
                                    &id,
                                    &token_str,
                                    None,
                                    None,
                                    Some(&name),
                                    avatar,
                                    None,
                                    None,
                                    Some("cookie"), // §6: explicit auth_mode
                                    state.token_key,
                                )
                                .await
                                {
                                    Ok(_) => results.push(ImportResult {
                                        provider: "x".into(),
                                        status: "connected".into(),
                                        name: Some(name),
                                        source: Some(cookies.source),
                                        error: None,
                                        hint: None,
                                    }),
                                    Err(e) => results.push(ImportResult {
                                        provider: "x".into(),
                                        status: "error".into(),
                                        name: None,
                                        source: Some(cookies.source),
                                        error: Some(format!(
                                            "Cookie import succeeded but DB write failed: {e}"
                                        )),
                                        hint: Some(
                                            "Check database connectivity and try again.".into(),
                                        ),
                                    }),
                                }
                            }
                            Err(e) => {
                                results.push(ImportResult {
                                    provider: "x".into(),
                                    status: "error".into(),
                                    name: None,
                                    source: Some(cookies.source),
                                    error: Some(format!("Cookie import succeeded but validation failed: {e}. Cookies may be expired.")),
                                    hint: Some("Log into x.com in your browser and try again.".into()),
                                });
                            }
                        }
                    }
                    None => {
                        results.push(ImportResult {
                            provider: "x".into(),
                            status: "no_cookies".into(),
                            name: None,
                            source: None,
                            error: Some("No X/Twitter cookies found in any browser.".into()),
                            hint: Some("Log into x.com in Chrome, Brave, Firefox, or Zen browser, then try again.".into()),
                        });
                    }
                }
            }
            "reddit" => {
                let integrations = crate::db::queries::list_integrations(&state.db, user_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if integrations
                    .iter()
                    .any(|i| i.provider_identifier == "reddit")
                {
                    results.push(ImportResult {
                        provider: "reddit".into(),
                        status: "already_connected".into(),
                        name: None,
                        source: None,
                        error: None,
                        hint: Some("Reddit is already connected.".into()),
                    });
                    continue;
                }

                match crate::social::reddit_cookies::extract_reddit_cookies() {
                    Some(cookies) => {
                        let token_str = crate::social::reddit_cookies::build_cookie_token(
                            &cookies.reddit_session,
                            cookies.token_v2.as_deref(),
                            Some(&cookies.cookie_string),
                        );
                        let mut provider_obj =
                            crate::social::reddit::RedditProvider::new(&state.config);
                        provider_obj.prepare_from_token(&token_str);
                        match provider_obj.get_www("/api/me.json", &[]).await {
                            Ok(json) => {
                                let name = json["data"]["name"]
                                    .as_str()
                                    .unwrap_or("Reddit User")
                                    .to_string();
                                let id = json["data"]["id"].as_str().unwrap_or("").to_string();
                                let icon = json["data"]["icon_img"]
                                    .as_str()
                                    .and_then(|s| s.split('?').next())
                                    .map(String::from);
                                match crate::db::queries::create_integration(
                                    &state.db,
                                    user_id,
                                    "reddit",
                                    "Reddit",
                                    &id,
                                    &token_str,
                                    None,
                                    None,
                                    Some(&name),
                                    icon.as_deref(),
                                    None,
                                    None,
                                    Some("cookie"), // §6: explicit auth_mode
                                    state.token_key,
                                )
                                .await
                                {
                                    Ok(_) => results.push(ImportResult {
                                        provider: "reddit".into(),
                                        status: "connected".into(),
                                        name: Some(name),
                                        source: Some(cookies.source),
                                        error: None,
                                        hint: None,
                                    }),
                                    Err(e) => results.push(ImportResult {
                                        provider: "reddit".into(),
                                        status: "error".into(),
                                        name: None,
                                        source: Some(cookies.source),
                                        error: Some(format!(
                                            "Cookie import succeeded but DB write failed: {e}"
                                        )),
                                        hint: Some(
                                            "Check database connectivity and try again.".into(),
                                        ),
                                    }),
                                }
                            }
                            Err(e) => {
                                results.push(ImportResult {
                                    provider: "reddit".into(),
                                    status: "error".into(),
                                    name: None,
                                    source: Some(cookies.source),
                                    error: Some(format!("Cookie import succeeded but validation failed: {e}. Cookies may be expired.")),
                                    hint: Some("Log into reddit.com in your browser and try again.".into()),
                                });
                            }
                        }
                    }
                    None => {
                        results.push(ImportResult {
                            provider: "reddit".into(),
                            status: "no_cookies".into(),
                            name: None,
                            source: None,
                            error: Some("No Reddit cookies found in any browser.".into()),
                            hint: Some("Log into reddit.com in Chrome, Brave, Firefox, or Zen browser, then try again.".into()),
                        });
                    }
                }
            }
            _ => {
                results.push(ImportResult {
                    provider: provider.into(),
                    status: "unsupported".into(),
                    name: None,
                    source: None,
                    error: Some(format!("Provider '{provider}' does not support browser cookie import. Supported: x, reddit, all")),
                    hint: None,
                });
            }
        }
    }

    Ok(Json(ImportCookiesOutput { results }))
}

/// Set a configuration value in ~/.switchpost/.env.
/// Creates the file if it doesn't exist. Overwrites existing values for the same key.
pub fn config_set(
    _state: &AppState,
    input: &ConfigSetInput,
) -> Result<Json<serde_json::Value>, String> {
    let dir = crate::config::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
    let env_path = dir.join(".env");

    let content = if env_path.exists() {
        std::fs::read_to_string(&env_path).map_err(|e| format!("Failed to read .env: {e}"))?
    } else {
        String::new()
    };

    let key_upper = input.key.to_uppercase();
    let new_line = format!("{}={}", key_upper, input.value);

    let mut found = false;
    let new_content: String = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(&format!("{key_upper}="))
                || trimmed.starts_with(&format!("# {key_upper}="))
                || trimmed.starts_with(&format!("#{key_upper}="))
            {
                found = true;
                new_line.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    if found {
        std::fs::write(&env_path, &new_content)
            .map_err(|e| format!("Failed to write .env: {e}"))?;
    } else {
        let mut to_write = new_content;
        if !to_write.ends_with('\n') {
            to_write.push('\n');
        }
        to_write.push_str(&new_line);
        to_write.push('\n');
        std::fs::write(&env_path, &to_write).map_err(|e| format!("Failed to write .env: {e}"))?;
    }

    Ok(Json(serde_json::json!({
        "status": "set",
        "key": key_upper,
        "path": env_path.display().to_string(),
        "message": format!("Set {key_upper}. Restart switchpost to apply."),
    })))
}

/// Get a configuration value (redacts secrets).
pub fn config_get(
    _state: &AppState,
    input: &ConfigGetInput,
) -> Result<Json<serde_json::Value>, String> {
    let key_upper = input.key.to_uppercase();
    // Try process env first, then fall back to reading the .env file directly
    let val = std::env::var(&key_upper).ok().or_else(|| {
        let env_path = crate::config::config_dir().join(".env");
        if env_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&env_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('#') || trimmed.is_empty() {
                        continue;
                    }
                    if let Some((k, v)) = trimmed.split_once('=') {
                        if k.trim().to_uppercase() == key_upper {
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }
        None
    });
    match val {
        Some(val) => {
            let is_secret = key_upper.contains("SECRET")
                || key_upper.contains("PASSWORD")
                || key_upper.contains("TOKEN")
                || key_upper.contains("KEY")
                || key_upper.contains("PRIVATE");
            let display = if is_secret {
                if val.len() > 8 {
                    format!("{}...{}", &val[..4], &val[val.len() - 4..])
                } else {
                    "****".into()
                }
            } else {
                val
            };
            Ok(Json(serde_json::json!({
                "key": key_upper,
                "value": display,
                "is_secret": is_secret,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "key": key_upper,
            "value": null,
            "error": format!("'{key_upper}' is not set."),
        }))),
    }
}

/// List all configuration entries in ~/.switchpost/.env (redacts secrets).
pub fn config_list(_state: &AppState) -> Result<Json<ConfigListOutput>, String> {
    let dir = crate::config::config_dir();
    let env_path = dir.join(".env");

    let mut entries = Vec::new();
    if env_path.exists() {
        let content =
            std::fs::read_to_string(&env_path).map_err(|e| format!("Failed to read .env: {e}"))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let k = k.trim().to_uppercase();
                let is_secret = k.contains("SECRET")
                    || k.contains("PASSWORD")
                    || k.contains("TOKEN")
                    || k.contains("KEY")
                    || k.contains("PRIVATE");
                let display = if is_secret {
                    if v.len() > 8 {
                        format!("{}...{}", &v[..4], &v[v.len() - 4..])
                    } else if !v.is_empty() {
                        "****".into()
                    } else {
                        "(empty)".into()
                    }
                } else {
                    v.to_string()
                };
                entries.push(ConfigEntry {
                    key: k,
                    value: display,
                    is_secret,
                });
            }
        }
    }

    Ok(Json(ConfigListOutput {
        path: env_path.display().to_string(),
        exists: env_path.exists(),
        entries,
    }))
}
