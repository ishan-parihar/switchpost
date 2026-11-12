pub mod autopost;
pub mod browser;
pub mod home;
pub mod oauth_browser;
pub mod platforms;
pub mod run;
pub mod update;
pub use run::run_cli;

use clap::{Parser, Subcommand};

/// Switchpost — Dual-Interface Social Media Scheduling Engine
#[derive(Parser, Debug)]
#[command(name = "switchpost", version, about, long_about = None)]
pub struct Cli {
    /// Output as compact JSON (default: TOON, ~40% fewer tokens for agents)
    #[arg(long, global = true, default_value_t = false)]
    pub json: bool,

    /// Pretty-print JSON output for human readability
    #[arg(long, global = true, default_value_t = false)]
    pub pretty: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Options for `switchpost autopost`.
#[derive(clap::Args, Debug)]
pub struct AutopostOptions {
    /// Path to the slide-spec JSON file.
    #[arg(long)]
    pub slides: String,
    /// Comma-separated provider identifiers to post to.
    #[arg(long)]
    pub platforms: String,
    /// The post copy.
    #[arg(long)]
    pub content: String,
    /// Deckmill platform preset (default: instagram_portrait).
    #[arg(long, default_value = "instagram_portrait")]
    pub preset: String,
    /// Deckmill aspect ratio (default: 4:5).
    #[arg(long, default_value = "4:5")]
    pub aspect_ratio: String,
    /// Brand colour for the slides (Deckmill requires one).
    #[arg(long, default_value = "#2D5BFF")]
    pub primary_color: String,
    /// Brand name rendered on every slide.
    #[arg(long, default_value = "")]
    pub brand_name: String,
    /// Brand handle rendered on every slide.
    #[arg(long, default_value = "")]
    pub brand_handle: String,
    /// Topic rendered on every slide.
    #[arg(long, default_value = "")]
    pub topic: String,
    /// URL rendered in the slide overlay (Deckmill requires it).
    #[arg(long, default_value = "")]
    pub url: String,
    /// Hashtags rendered in the slide overlay (Deckmill requires them).
    #[arg(long, default_value = "")]
    pub hashtags: String,
    /// Per-provider settings, e.g. '{"reddit":{"subreddit":"rust"}}'.
    #[arg(long)]
    pub settings: Option<String>,
    /// Publish immediately instead of staging a draft.
    #[arg(long)]
    pub publish: bool,
    /// ISO8601 timestamp to schedule for (implies queued, not draft).
    #[arg(long)]
    pub schedule: Option<String>,
    /// Directory for the rendered slides (default: a temp dir).
    #[arg(long)]
    pub work_dir: Option<String>,
    /// Repeat every N days until --repeat-until. Creates a recurring
    /// series the scheduler publishes — no loop process needed.
    #[arg(long)]
    pub repeat_days: Option<i32>,
    /// End of the recurring series (ISO8601). Required with --repeat-days.
    #[arg(long)]
    pub repeat_until: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check for and install a verified release update (alias: upgrade)
    #[command(alias = "upgrade")]
    Update(update::UpdateArgs),

    /// Start the HTTP REST server
    Serve {
        /// Port to listen on. `APP_URL` and `FRONTEND_URL` must use this port,
        /// or state-changing requests fail the CSRF origin check (403).
        #[arg(long, default_value_t = 6543)]
        port: u16,
    },

    /// Start the MCP stdio server
    Mcp,

    /// Call one MCP tool by name, in-process — the documented shell path
    /// for agents (skills/switchpost-agent). Args are a JSON object.
    McpCall {
        /// Tool name as advertised by tools/list (e.g. posts_stage, x_create_tweet)
        tool: String,

        /// JSON object of tool arguments
        #[arg(long, default_value = "{}")]
        args: String,
    },

    /// List every MCP tool with its description — the documented shell
    /// path for agents (`switchpost mcp-tools --pretty`).
    McpTools {
        /// Aligned "name — description" table plus a count (plain lists names only)
        #[arg(long)]
        pretty: bool,

        /// Narrow to one toolset (e.g. `x`, `reddit`, `core`). Mirrors
        /// SWITCHPOST_TOOLSETS filtering so an agent can size a session
        /// before enabling it. Omit for the full catalog.
        #[arg(long)]
        toolset: Option<String>,
    },

    /// Initialize ~/.switchpost/ config directory with default .env template
    Init,

    /// List connected social providers
    Providers,

    /// Enable or disable a connected integration without deleting it
    Integrations {
        #[command(subcommand)]
        action: IntegrationsAction,
    },

    /// Connect a provider (auto-imports browser cookies for X/Reddit, shows OAuth URL for others)
    Connect {
        /// Provider name (x, reddit, linkedin, facebook, instagram, bluesky, github)
        provider: String,
        /// For OAuth/one-time-token providers: poll until the login
        /// completes (the callback lands or the code is redeemed), then
        /// print the resulting account. Designed for AI-agent loops:
        /// run with --wait, drive a browser to the auth_url, and the
        /// command exits 0 the moment the credential lands.
        #[arg(long)]
        wait: bool,
        /// Seconds to wait with --wait before giving up (default 300).
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
        /// Drive the platform's consent page in the operator's real browser
        /// (browseros-neo) and complete the login end to end.
        ///
        /// The CLI opens the authorization URL, waits for the provider's
        /// redirect, and redeems the authorization code IN-PROCESS — the
        /// server does not have to be running.
        #[arg(long)]
        browser: bool,
        /// browseros-neo MCP endpoint (default: http://127.0.0.1:9012/mcp).
        #[arg(long, default_value = crate::cli::oauth_browser::DEFAULT_BROWSER_ENDPOINT)]
        browser_endpoint: String,
    },

    /// Render a carousel with Deckmill and post it — one command, no
    /// copy-paste between tools.
    ///
    /// The slide specs are Deckmill's own contract: a JSON array of
    /// `{"type": "<slide type>", "params": {...}}` (run
    /// `deckmill slide-info <type>` for a type's params). The
    /// renderer binary is found on PATH, or via DECKMILL_BIN.
    Autopost(AutopostOptions),

    /// Print what this CLI can do for an AI agent: every command, the
    /// MCP tools, the connected providers and their auth state.
    Agent,

    /// Check health of all connected providers and report status
    Doctor,

    /// Read-only audit: disabled integrations, --platforms prefix collisions,
    /// and the live single-writer/busy-timeout posture of the SQLite pool
    Audit,

    /// Install/uninstall the systemd user service for the serve daemon
    /// (§6.4: no sudo — a user unit plus `loginctl enable-linger`)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    /// Full guided onboarding: check status, import cookies, connect providers
    Setup,

    /// Auto-import browser cookies for all cookie-based providers (X, Reddit)
    ConnectAll,

    /// Manage configuration values in ~/.switchpost/.env
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// X (Twitter) operations
    X {
        #[command(subcommand)]
        action: XAction,
    },

    /// Reddit operations
    Reddit {
        #[command(subcommand)]
        action: RedditAction,
    },

    /// LinkedIn personal profile operations
    Linkedin {
        #[command(subcommand)]
        action: LinkedinAction,
    },

    /// LinkedIn company page operations
    LinkedinPage {
        #[command(subcommand)]
        action: LinkedinPageAction,
    },

    /// Facebook page operations
    Facebook {
        #[command(subcommand)]
        action: FacebookAction,
    },

    /// Instagram account operations
    Instagram {
        #[command(subcommand)]
        action: InstagramAction,
    },

    /// YouTube operations
    Youtube {
        #[command(subcommand)]
        action: YoutubeAction,
    },

    /// Bluesky operations
    Bluesky {
        #[command(subcommand)]
        action: BlueskyAction,
    },

    /// Mastodon operations
    Mastodon {
        #[command(subcommand)]
        action: MastodonAction,
    },

    /// Import recent posts from a provider into local store
    Import {
        /// Provider name (x, reddit, bluesky)
        provider: String,
        /// Number of recent posts to import
        #[arg(long, default_value_t = 20)]
        count: u32,
    },

    /// Show unified feed of imported posts across all providers
    Feed {
        /// Filter by provider name (x, reddit, etc.)
        #[arg(long)]
        provider: Option<String>,
        /// Number of posts to show
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Comment operations
    Comment {
        #[command(subcommand)]
        action: CommentAction,
    },

    /// RSS autopost feeds — each new feed item becomes a post
    Rss {
        #[command(subcommand)]
        action: RssAction,
    },

    /// Campaigns — grouped posts with a goal and a progress metric
    Campaigns {
        #[command(subcommand)]
        action: CampaignAction,
    },

    /// Post signatures — text appended to posts
    Signatures {
        #[command(subcommand)]
        action: SignatureAction,
    },

    /// AI writing helpers (generate, improve, summarise, hashtags, tone)
    Ai {
        #[command(subcommand)]
        action: AiAction,
    },

    /// Current posting streak (consecutive days with a published post)
    Streak {
        /// The founder's timezone as an IANA name (e.g. America/New_York)
        #[arg(long)]
        tz: Option<String>,
    },

    /// Direct message operations
    Dm {
        #[command(subcommand)]
        action: DmAction,
    },

    /// Automation rules management
    Automation {
        #[command(subcommand)]
        action: AutomationAction,
    },

    /// Posts management
    Posts {
        #[command(subcommand)]
        action: PostsAction,
    },

    // ── Unified Commands (mirror MCP tools) ──────────────────
    /// Unified post command — post to any platform with auto content-splitting
    Post {
        /// Post text content
        text: String,
        /// Target platforms (comma-separated: x,bluesky,linkedin,reddit,...). Uses all connected if omitted.
        ///
        /// EXACT identifier match wins: `instagram` targets ONLY `instagram`
        /// (not `instagram-standalone`), `linkedin` ONLY `linkedin` (not
        /// `linkedin-page`). To hit a suffixed variant name it explicitly
        /// (`instagram,instagram-standalone`) or use `--integrations <uuid,...>`.
        #[arg(long)]
        platforms: Option<String>,
        /// Media URLs to attach (comma-separated)
        #[arg(long)]
        media: Option<String>,
        /// Schedule for later (ISO8601 datetime)
        #[arg(long)]
        schedule: Option<String>,
        /// First comment (for platforms that support it, e.g. Instagram)
        #[arg(long)]
        first_comment: Option<String>,
        /// Provider settings as a JSON object.
        ///
        /// Some providers cannot publish without a target, and they read
        /// it out of the post's settings: Reddit needs `subreddit` and
        /// `title`, Pinterest needs `board` and `title`, Discord needs
        /// `channel`, Telegram-user needs `target`, Hashnode needs
        /// `publication_id`.
        ///
        /// The composer writes these for you; from the shell you pass
        /// them here. Without them the post is created, queued, and then
        /// fails at publish with "Missing subreddit in settings".
        #[arg(long)]
        settings: Option<String>,
    },

    /// Stage a post across multiple platforms with auto content-splitting
    Stage {
        /// Post text content
        text: String,
        /// Target integration IDs (comma-separated UUIDs). Uses all connected if omitted.
        /// Bypasses platform-name resolution entirely — the precise way to
        /// target one of a colliding pair.
        #[arg(long, conflicts_with = "platforms")]
        integrations: Option<String>,
        /// Target platforms (comma-separated: x,linkedin,bluesky,...). Filters by provider name.
        ///
        /// EXACT identifier match wins. `instagram` targets ONLY `instagram`
        /// and `linkedin` ONLY `linkedin` — the `-standalone` / `-page`
        /// variants are separate providers and are NOT swept in. Name them
        /// explicitly (`--platforms instagram,instagram-standalone`) or use
        /// `--integrations` with the exact UUIDs. `switchpost audit` reports
        /// which colliding pairs are currently connected.
        #[arg(long, conflicts_with = "integrations")]
        platforms: Option<String>,
        /// Media URLs to attach (comma-separated)
        #[arg(long)]
        media: Option<String>,
        /// Schedule for later (ISO8601 datetime)
        #[arg(long)]
        schedule: Option<String>,
        /// Preview splits without creating drafts
        #[arg(long, default_value_t = false)]
        preview: bool,
        /// First comment (for platforms that support it, e.g. Instagram)
        #[arg(long)]
        first_comment: Option<String>,
        /// Provider settings as a JSON object.
        ///
        /// Some providers cannot publish without a target, and they read
        /// it out of the post's settings: Reddit needs `subreddit` and
        /// `title`, Pinterest needs `board` and `title`, Discord needs
        /// `channel`, Telegram-user needs `target`, Hashnode needs
        /// `publication_id`.
        ///
        /// The composer writes these for you; from the shell you pass
        /// them here. Without them the post is created, queued, and then
        /// fails at publish with "Missing subreddit in settings".
        #[arg(long)]
        settings: Option<String>,
    },

    /// Create a carousel post with multiple images
    Carousel {
        /// Post text content
        text: String,
        /// Target integration ID (UUID)
        #[arg(long)]
        integration: String,
        /// Media URLs (comma-separated, minimum 2)
        #[arg(long)]
        media: String,
        /// Optional post title
        #[arg(long)]
        title: Option<String>,
        /// Schedule for later (ISO8601 datetime)
        #[arg(long)]
        schedule: Option<String>,
    },

    /// Media management — upload, list, download
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },

    /// Content split preview — see how content will be split across platforms
    SplitPreview {
        /// Content to preview splitting for
        text: String,
        /// Platforms to preview (comma-separated). Shows all if omitted.
        #[arg(long)]
        platforms: Option<String>,
    },

    /// TikTok operations
    Tiktok {
        #[command(subcommand)]
        action: TiktokAction,
    },
    /// Threads operations
    Threads {
        #[command(subcommand)]
        action: ThreadsAction,
    },
    /// Discord operations
    Discord {
        #[command(subcommand)]
        action: DiscordAction,
    },
    /// Slack operations
    Slack {
        #[command(subcommand)]
        action: SlackAction,
    },
    /// Telegram Bot operations
    TelegramBot {
        #[command(subcommand)]
        action: TelegramBotAction,
    },
    /// Telegram User operations
    TelegramUser {
        #[command(subcommand)]
        action: TelegramUserAction,
    },
    /// WhatsApp operations
    Whatsapp {
        #[command(subcommand)]
        action: WhatsappAction,
    },
    /// Pinterest operations
    Pinterest {
        #[command(subcommand)]
        action: PinterestAction,
    },
    /// GitHub operations
    Github {
        #[command(subcommand)]
        action: GithubAction,
    },
    /// WordPress operations
    Wordpress {
        #[command(subcommand)]
        action: WordpressAction,
    },
    /// Hashnode operations
    Hashnode {
        #[command(subcommand)]
        action: HashnodeAction,
    },
    /// Medium blog operations
    MediumBlog {
        #[command(subcommand)]
        action: MediumBlogAction,
    },
    /// Dev.to operations
    Devto {
        #[command(subcommand)]
        action: DevtoAction,
    },
    /// Skool operations
    Skool {
        #[command(subcommand)]
        action: SkoolAction,
    },
    /// Google services (YouTube)
    Google {
        #[command(subcommand)]
        action: GoogleAction,
    },
    /// Google Drive operations
    Gdrive {
        #[command(subcommand)]
        action: DriveAction,
    },
    /// Google Calendar operations
    Gcal {
        #[command(subcommand)]
        action: GcalAction,
    },
    /// Gmail operations
    GmailOps {
        #[command(subcommand)]
        action: GmailAction,
    },
    /// Webhook management
    Webhooks {
        #[command(subcommand)]
        action: WebhooksAction,
    },
    /// Notification management
    Notifications {
        #[command(subcommand)]
        action: NotificationsAction,
    },
    /// Tag management
    Tags {
        #[command(subcommand)]
        action: TagsAction,
    },
    /// Analytics
    Analytics {
        #[command(subcommand)]
        action: AnalyticsAction,
    },
}

// ─── Comment Actions ────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum CommentAction {
    /// Get comments for a post
    Get {
        /// Integration ID
        integration_id: String,
        /// Post ID
        post_id: String,
        /// Number of comments to fetch
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Reply to a comment
    Reply {
        /// Integration ID
        integration_id: String,
        /// Comment ID
        comment_id: String,
        /// Reply content
        content: String,
    },
}

// ─── DM Actions ─────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum DmAction {
    /// Send a direct message
    Send {
        /// Integration ID
        integration_id: String,
        /// Recipient ID
        recipient: String,
        /// Message content
        content: String,
    },
    /// List DM conversations
    List {
        /// Integration ID
        integration_id: String,
        /// Number of conversations to fetch
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get messages in a conversation
    Messages {
        /// Integration ID
        integration_id: String,
        /// Conversation ID
        conversation_id: String,
        /// Number of messages to fetch
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

// ─── Automation Actions ─────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum AutomationAction {
    /// Create an automation rule
    Create {
        /// Integration ID
        integration_id: String,
        /// Rule name
        name: String,
        /// Trigger type (comment, dm, mention, follow)
        trigger_type: String,
        /// Response template
        response_template: String,
        /// Response type (fixed, template, ai_generated)
        response_type: String,
    },
    /// List automation rules
    List {
        /// Integration ID (optional)
        integration_id: Option<String>,
    },
    /// Update an automation rule
    Update {
        /// Rule ID
        rule_id: String,
        /// Rule name (optional)
        name: Option<String>,
        /// Response template (optional)
        response_template: Option<String>,
        /// Is active (optional)
        is_active: Option<bool>,
    },
    /// Delete an automation rule
    Delete {
        /// Rule ID
        rule_id: String,
    },
    /// Get execution logs
    Logs {
        /// Rule ID
        rule_id: String,
        /// Number of logs to fetch
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

// ─── Posts Actions ────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum PostsAction {
    /// List posts (draft, queued, published, error)
    List {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Get a single post by ID
    Get { id: String },
    /// Create a new post
    Create {
        content: String,
        #[arg(long)]
        integrations: String,
        #[arg(long)]
        schedule: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        media: Option<String>,
        #[arg(long)]
        first_comment: Option<String>,
        #[arg(long)]
        settings: Option<String>,
        #[arg(long)]
        state: Option<String>,
    },
    /// Re-schedule an existing post
    Schedule {
        id: String,
        scheduled_at: String,
        /// A PUBLISHED post needs this to be re-scheduled: it resets the
        /// post to `queued` and the scheduler publishes it AGAIN — a NEW
        /// post on the platform, next to the one that is already live.
        ///
        /// Without the flag the command refuses, because the old answer
        /// ("Post not found after scheduling") was a lie: the post exists,
        /// it is simply published, and the founder was left hunting for a
        /// post that was never missing.
        #[arg(long)]
        republish: bool,
        /// Move the whole thread/campaign group by the same delta.
        #[arg(long)]
        move_group: bool,
    },
    /// Publish a queued or errored post immediately via the provider
    Publish { id: String },
    /// Delete a post
    Delete {
        id: String,
        /// Also delete the PUBLISHED post on the platform.
        ///
        /// Without this the local row is removed and the tweet stays live
        /// — so the dashboard says the post is gone while the audience
        /// still sees it. The platform delete happens FIRST, so a failure
        /// leaves the row (and its platform id) for a retry.
        #[arg(long)]
        from_platform: bool,
    },
    /// Find the next available posting slot (optionally for one integration)
    FindSlot {
        #[arg(long)]
        integration: Option<String>,
    },
    /// Update a post's content, title, media, or settings (omitted fields are kept)
    Update {
        id: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Media as a JSON array, or comma-separated URLs
        #[arg(long)]
        media: Option<String>,
        /// Settings as a JSON object
        #[arg(long)]
        settings: Option<String>,
    },
    /// Replace a post's tags with these tag UUIDs (no IDs clears them)
    Tags { id: String, tag_ids: Vec<String> },
    /// Validate content against integrations without creating a post
    Validate {
        content: String,
        /// Comma-separated integration UUIDs
        #[arg(long)]
        integrations: String,
        #[arg(long)]
        title: Option<String>,
        /// Media as a JSON array, or comma-separated URLs
        #[arg(long)]
        media: Option<String>,
        /// Settings as a JSON object
        #[arg(long)]
        settings: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum IntegrationsAction {
    /// Stop publishing through an integration (keeps its tokens and posts)
    Disable { id: String },
    /// Resume publishing through a disabled integration
    Enable { id: String },
}

// ─── Media Actions ──────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum MediaAction {
    /// Upload a media file (image/video) for post attachments
    Upload {
        /// File path to upload
        path: String,
        /// Alt text for accessibility
        #[arg(long)]
        alt: Option<String>,
    },
    /// Batch upload multiple media files from local paths
    UploadBatch {
        /// File paths to upload (space-separated)
        paths: Vec<String>,
    },
    /// List uploaded media files
    List {
        /// Number of items to show
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Search filter
        #[arg(long)]
        search: Option<String>,
    },
    /// Download media from a URL to local file
    Download {
        /// URL to download
        url: String,
        /// Output file path
        #[arg(long, default_value = "./download")]
        output: String,
    },
}

// ─── RSS Actions ──────────────────────────────────────────────────────────

/// RSS autopost feeds.
///
/// These are the FEED-DRIVEN half of autonomous auto-posting: the poller
/// turns each new feed item into a post. The same handlers back the MCP
/// tools, so the CLI and the MCP surface cannot drift.
#[derive(Subcommand, Debug)]
pub enum RssAction {
    /// List the configured feeds
    List,
    /// Add a feed that autoposts to a channel
    Add {
        /// Feed URL (RSS or Atom)
        feed_url: String,
        /// Channel (integration) ID that receives the posts
        integration_id: String,
        /// Display title for the feed
        #[arg(long)]
        title: Option<String>,
        /// Draft an AI summary for each item
        #[arg(long)]
        ai_summary: bool,
        /// Create the feed PAUSED (do not poll it yet)
        #[arg(long)]
        disabled: bool,
    },
    /// Delete a feed
    Delete {
        /// Feed ID
        feed_id: String,
    },
    /// Pause or resume polling a feed
    Toggle {
        /// Feed ID
        feed_id: String,
    },
    /// Poll a feed now for new items
    Poll {
        /// Feed ID
        feed_id: String,
    },
    /// List the items a feed has produced
    Items {
        /// Feed ID
        feed_id: String,
    },
}

// ─── Campaign Actions ─────────────────────────────────────────────────────

/// Campaigns group posts around a goal and track a progress metric.
#[derive(Subcommand, Debug)]
pub enum CampaignAction {
    /// List campaigns (archived ones included)
    List,
    /// Create a campaign
    Create {
        /// Campaign name
        name: String,
        /// What the campaign is for
        #[arg(long)]
        description: Option<String>,
        /// Hex colour
        #[arg(long)]
        color: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        start_date: Option<String>,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end_date: Option<String>,
        /// The goal the campaign works toward
        #[arg(long)]
        goal: Option<String>,
        /// active | paused | archived | completed
        #[arg(long)]
        status: Option<String>,
        /// posts | engagement | reach | followers | custom
        #[arg(long)]
        progress_metric: Option<String>,
        /// Target value for the progress metric
        #[arg(long)]
        progress_target: Option<i32>,
    },
    /// Update a campaign. Omitted fields are kept; --clear-<field> clears one.
    Update {
        /// Campaign ID
        campaign_id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// Clear the description
        #[arg(long, conflicts_with = "description")]
        clear_description: bool,
        /// New goal
        #[arg(long)]
        goal: Option<String>,
        /// Clear the goal
        #[arg(long, conflicts_with = "goal")]
        clear_goal: bool,
        /// active | paused | archived | completed
        #[arg(long)]
        status: Option<String>,
    },
    /// Delete (archive) a campaign
    Delete {
        /// Campaign ID
        campaign_id: String,
    },
}

// ─── Signature Actions ────────────────────────────────────────────────────

/// Signatures are text appended to posts.
#[derive(Subcommand, Debug)]
pub enum SignatureAction {
    /// List signatures
    List,
    /// Create a signature
    Create {
        /// Name for identification
        name: String,
        /// The signature text
        content: String,
        /// Provider it applies to (e.g. x). Omit for all providers.
        #[arg(long)]
        provider: Option<String>,
    },
    /// Update a signature
    Update {
        /// Signature ID
        signature_id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New content
        #[arg(long)]
        content: Option<String>,
        /// New provider scope
        #[arg(long)]
        provider: Option<String>,
    },
    /// Delete a signature
    Delete {
        /// Signature ID
        signature_id: String,
    },
}

// ─── AI Actions ───────────────────────────────────────────────────────────

/// AI writing helpers. They call the same handlers the MCP tools call.
#[derive(Subcommand, Debug)]
pub enum AiAction {
    /// Draft a post from a topic
    Generate {
        /// What the post is about
        topic: String,
        /// Tone (e.g. casual, professional)
        #[arg(long, default_value = "professional")]
        tone: String,
        /// Length (e.g. short, long)
        #[arg(long, default_value = "short")]
        length: String,
    },
    /// Generate several posts on one topic
    Bulk {
        /// What the posts are about
        topic: String,
        /// one_short | one_long | thread_short | thread_long
        #[arg(long, default_value = "one_short")]
        format: String,
        /// personal | company
        #[arg(long, default_value = "professional")]
        tone: String,
        /// How many posts (max 5)
        #[arg(long, default_value_t = 3)]
        count: u32,
    },
    /// Improve the wording of existing text
    Improve {
        /// The text to improve
        content: String,
    },
    /// Summarise text
    Summarize {
        /// The text to summarise
        content: String,
    },
    /// Suggest hashtags for text
    Hashtags {
        /// The text to work on
        content: String,
    },
    /// Rewrite text in a different tone
    Tone {
        /// The text to rewrite
        content: String,
        /// The target tone
        tone: String,
    },
}

// ─── Config Management ─────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Write the hardened user unit, daemon-reload, enable --now, and
    /// enable-linger so it survives logout.
    Install,
    /// Stop and disable the user unit (leaves linger and data alone).
    Uninstall,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Set a configuration value in ~/.switchpost/.env
    Set {
        /// Environment variable name (e.g. BLUESKY_HANDLE, GITHUB_TOKEN)
        key: String,
        /// Value to set
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Environment variable name
        key: String,
    },
    /// List all configured values (redacts secrets)
    List,
    /// Reset the single-user WebUI password (APP_PASSWORD in ~/.switchpost/.env)
    ResetPassword {
        /// New password. Omit to generate a strong random one (printed once).
        password: Option<String>,
    },
}

// ─── X (Twitter) ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum XAction {
    /// Post a tweet
    Post {
        /// Tweet text content
        text: String,
    },

    /// View home timeline
    Timeline {
        /// Number of tweets to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },

    /// Search tweets
    Search {
        /// Search query
        query: String,
    },

    /// Like a tweet
    Like {
        /// Tweet ID to like
        tweet_id: String,
    },

    /// Retweet a tweet
    Retweet {
        /// Tweet ID to retweet
        tweet_id: String,
    },

    /// Delete a tweet
    Delete {
        /// Tweet ID to delete
        tweet_id: String,
    },

    /// Bookmark a tweet
    Bookmark {
        /// Tweet ID to bookmark
        tweet_id: String,
    },

    /// Get a user's profile
    User {
        /// Username to look up
        username: String,
    },

    /// Reply to a tweet
    Reply {
        /// Tweet ID to reply to
        tweet_id: String,
        /// Reply text
        text: String,
    },

    /// Send a direct message
    Dm {
        /// Recipient user ID
        recipient: String,
        /// Message text
        text: String,
    },

    /// List DM conversations
    DmList {
        /// Number of conversations to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },

    /// Get messages in a DM conversation
    DmMessages {
        /// Conversation ID
        conversation_id: String,
        /// Number of messages to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
}

// ─── Reddit ─────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum RedditAction {
    /// Browse a subreddit
    Browse {
        /// Subreddit name (without r/)
        subreddit: String,

        /// Sort order (hot, new, top, rising)
        #[arg(long, default_value = "hot")]
        sort: String,

        /// Number of posts to fetch
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },

    /// Search Reddit
    Search {
        /// Search query
        query: String,

        /// Limit search to a subreddit
        #[arg(long)]
        subreddit: Option<String>,

        /// Sort order (relevance, hot, top, new, comments)
        #[arg(long, default_value = "relevance")]
        sort: String,
    },

    /// Submit a post to a subreddit
    Post {
        /// Post title
        #[arg(long)]
        title: String,

        /// Self-text body
        #[arg(long)]
        text: Option<String>,

        /// Link URL (for link posts)
        #[arg(long)]
        url: Option<String>,

        /// Target subreddit (omit to pick interactively)
        #[arg(long)]
        target: Option<String>,

        /// Multiple posting targets (comma-separated). Creates one post per target.
        #[arg(long)]
        targets: Option<String>,
    },

    /// Comment on a post or reply to a comment
    Comment {
        /// Parent thing ID (t3_ for post, t1_ for comment)
        thing_id: String,

        /// Comment text
        text: String,
    },

    /// Vote on a post or comment
    Vote {
        /// Thing ID to vote on
        thing_id: String,

        /// Vote direction (up, down, none)
        direction: String,
    },

    /// Save a post or comment
    Save {
        /// Thing ID to save
        thing_id: String,
    },

    /// Unsave a post or comment
    Unsave {
        /// Thing ID to unsave
        thing_id: String,
    },

    /// Delete a post or comment
    Delete {
        /// Thing ID to delete
        thing_id: String,
    },

    /// Get a user's profile
    User {
        /// Username to look up
        username: String,
    },

    /// View inbox messages
    Inbox {
        /// Inbox folder (inbox, unread, sent)
        #[arg(long, default_value = "inbox")]
        folder: String,
    },

    /// Moderator actions
    Mod {
        #[command(subcommand)]
        action: RedditModAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum RedditModAction {
    /// Remove a post or comment
    Remove {
        /// Thing ID to remove
        thing_id: String,

        /// Mark as spam
        #[arg(long)]
        spam: bool,
    },

    /// Approve a post or comment
    Approve {
        /// Thing ID to approve
        thing_id: String,
    },

    /// Lock a post or comment
    Lock {
        /// Thing ID to lock
        thing_id: String,
    },

    /// Unlock a post or comment
    Unlock {
        /// Thing ID to unlock
        thing_id: String,
    },
}

// ─── LinkedIn ───────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum LinkedinAction {
    /// View your LinkedIn profile
    Profile,

    /// List your recent posts
    Posts {
        /// Number of posts to fetch
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },

    /// Create a new post
    Post {
        /// Post text content
        text: String,
    },

    /// Delete a post
    Delete {
        /// Post URN to delete
        post_urn: String,
    },

    /// View reactions on a post
    Reactions {
        /// Post URN
        post_urn: String,
    },

    /// View profile analytics
    Analytics,

    /// Reply to a LinkedIn comment
    Reply {
        /// Comment URN
        comment_id: String,
        /// Reply content
        content: String,
    },

    /// Send a direct message on LinkedIn
    Dm {
        /// Recipient URN
        recipient: String,
        /// Message content
        content: String,
    },

    /// List LinkedIn message conversations
    DmList {
        /// Number of conversations to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },

    /// Get messages in a LinkedIn conversation
    DmMessages {
        /// Conversation ID
        conversation_id: String,
        /// Number of messages to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
}

// ─── LinkedIn Page ──────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum LinkedinPageAction {
    /// List managed company pages
    List,

    /// Post on behalf of a company page
    Post {
        /// Page ID
        page_id: String,

        /// Post text content
        text: String,
    },

    /// View page analytics
    Analytics {
        /// Page ID
        page_id: String,
    },

    /// View page followers
    Followers {
        /// Page ID
        page_id: String,
    },
    /// Get page details
    Page {
        /// Page ID
        page_id: String,
    },
    /// Get page feed
    Feed {
        /// Page ID
        page_id: String,
        /// Number of posts
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Create a comment on a post
    CreateComment {
        /// Post URN
        post_urn: String,
        /// Page URN
        page_urn: String,
        /// Comment text
        text: String,
    },
    /// Delete a post
    Delete {
        /// Post URN
        post_urn: String,
    },
    /// Get reactions on a post
    Reactions {
        /// Post URN
        post_urn: String,
    },
    /// Get shares on a post
    Shares {
        /// Post URN
        post_urn: String,
    },
    /// Get analytics for a specific post
    PostAnalytics {
        /// Post URN
        post_urn: String,
    },
}

// ─── Facebook ───────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum FacebookAction {
    /// List posts on a page
    Posts {
        /// Facebook Page ID
        page_id: String,
    },

    /// View page insights
    Insights {
        /// Facebook Page ID
        page_id: String,

        /// Comma-separated metrics (e.g. page_impressions,page_engaged_users,page_fans).
        /// Default: page_impressions,page_engaged_users,page_fans
        #[arg(long, default_value = "page_impressions,page_engaged_users,page_fans")]
        metric: String,
    },
    /// Comment on a post
    Comment {
        /// Post ID to comment on
        post_id: String,
        /// Comment text
        text: String,
    },
    /// Create a post on a page
    Create {
        /// Facebook Page ID
        page_id: String,
        /// Post message text
        message: String,
    },
    /// Create a photo post
    Photo {
        /// Facebook Page ID
        page_id: String,
        /// Photo URL
        url: String,
        /// Optional caption
        #[arg(long)]
        caption: Option<String>,
    },
    /// Create a video post
    Video {
        /// Facebook Page ID
        page_id: String,
        /// Video file URL
        url: String,
        /// Optional title
        #[arg(long)]
        title: Option<String>,
    },
    /// Delete a post
    Delete {
        /// Post ID to delete
        post_id: String,
    },
    /// React to a post
    React {
        /// Post ID
        post_id: String,
        /// Reaction type (LIKE, LOVE, HAHA, WOW, SAD, ANGRY)
        reaction_type: String,
    },
    /// List conversations (inbox)
    Conversations {
        /// Facebook Page ID
        page_id: String,
    },
    /// Send a message to a conversation
    Send {
        /// Facebook Page ID
        page_id: String,
        /// Conversation ID
        conversation_id: String,
        /// Message text
        text: String,
    },
    /// Search for public pages
    Pages {
        /// Search query
        query: String,
    },
    /// Get page insights
    PageInsights {
        /// Facebook Page ID
        page_id: String,
        /// Comma-separated metrics
        #[arg(long, default_value = "page_impressions,page_engaged_users")]
        metric: String,
    },
    /// Get page albums
    Albums {
        /// Facebook Page ID
        page_id: String,
    },
}

// ─── Instagram ──────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum InstagramAction {
    /// List recent posts
    Posts {
        /// Instagram account ID
        account_id: String,
    },

    /// View account insights
    Insights {
        /// Instagram account ID
        account_id: String,

        /// Comma-separated metrics (e.g. reach,follower_count,profile_views).
        /// Default: reach,follower_count
        #[arg(long, default_value = "reach,follower_count")]
        metric: String,
    },

    /// Comment on a media post
    Comment {
        /// Media ID to comment on
        media_id: String,

        /// Comment text
        text: String,
    },

    /// Send a direct message on Instagram
    Dm {
        /// Account ID
        account_id: String,
        /// Recipient ID
        recipient: String,
        /// Message content
        content: String,
    },

    /// List Instagram message conversations
    DmList {
        /// Account ID
        account_id: String,
        /// Number of conversations to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },

    /// Get messages in an Instagram conversation
    DmMessages {
        /// Account ID
        account_id: String,
        /// Conversation ID
        conversation_id: String,
        /// Number of messages to fetch
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
    /// Get media details
    MediaDetail {
        /// Instagram account ID
        account_id: String,
        /// Media ID
        media_id: String,
    },
    /// Search hashtags
    Hashtag {
        /// Instagram account ID
        account_id: String,
        /// Hashtag query
        query: String,
    },
    /// Get reels
    Reels {
        /// Instagram account ID
        account_id: String,
    },
    /// Get stories
    Stories {
        /// Instagram account ID
        account_id: String,
    },
    /// Get followers
    Followers {
        /// Instagram account ID
        account_id: String,
    },
    /// Get audience insights
    InsightsAudience {
        /// Instagram account ID
        account_id: String,
    },
    /// Get mentions
    Mentions {
        /// Instagram account ID
        account_id: String,
    },
    /// Poll container status
    PollContainer {
        /// Instagram account ID
        account_id: String,
        /// Creation ID
        creation_id: String,
    },
}

// ─── YouTube ────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum YoutubeAction {
    /// Reply to a YouTube comment
    Reply {
        /// Comment ID
        comment_id: String,
        /// Reply content
        content: String,
    },

    /// Search YouTube videos
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },

    /// Get video details
    Video {
        /// Video ID
        video_id: String,
    },
    /// List playlists
    Playlists {
        /// Channel ID
        channel_id: String,
        /// Max results
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Get channel stats
    Stats {
        /// Channel ID
        channel_id: String,
    },
    /// Get analytics
    Analytics {
        /// Channel ID
        channel_id: String,
        /// Metrics (comma-separated)
        #[arg(long, default_value = "views,likes,comments")]
        metrics: String,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        start_date: String,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end_date: String,
    },
    /// Get subscriptions
    Subscriptions {
        /// Channel ID
        channel_id: String,
        /// Max results
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Find creators by topic
    Creators {
        /// Search query
        query: String,
        /// Max results
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
}

// ─── Bluesky ────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum BlueskyAction {
    /// Reply to a Bluesky post
    Reply {
        /// Post URI
        post_uri: String,
        /// Reply content
        content: String,
    },

    /// Get user profile
    Profile {
        /// Handle or DID
        handle: String,
    },

    /// Get timeline
    Timeline {
        /// Number of posts to fetch
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Search posts
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Create a new post
    Post {
        /// Post text content
        text: String,
        /// Image URLs (comma-separated)
        #[arg(long)]
        images: Option<String>,
    },
    /// Get the feed
    Feed {
        /// Feed type (popular, etc.)
        #[arg(long)]
        feed_type: Option<String>,
        /// Number of posts
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

// ─── Mastodon ───────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum MastodonAction {
    /// Reply to a Mastodon status
    Reply {
        /// Status ID
        status_id: String,
        /// Reply content
        content: String,
    },

    /// Get user info
    Whoami,

    /// Get timeline
    Timeline {
        /// Timeline type (home, local, public)
        #[arg(long, default_value = "home")]
        kind: String,
        /// Number of posts to fetch
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Search
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Create a new post (toot)
    Post {
        /// Post content (max 500 chars)
        text: String,
        /// Visibility (public, unlisted, private, direct)
        #[arg(long, default_value = "public")]
        visibility: String,
    },
    /// Get a specific post by ID
    Get {
        /// Status ID
        status_id: String,
    },
}

// ─── TikTok

#[derive(Subcommand, Debug)]
pub enum TiktokAction {
    /// Get TikTok profile
    Profile,
    /// Post a video
    Post {
        /// Video caption text
        text: String,
        /// Video URL to upload
        #[arg(long)]
        video_url: Option<String>,
    },
    /// List recent videos
    Videos {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

// ─── Threads

#[derive(Subcommand, Debug)]
pub enum ThreadsAction {
    /// Get your Threads profile
    Profile {
        /// Threads account ID
        threads_id: String,
    },
    /// List your threads (posts)
    List {
        /// Threads account ID
        threads_id: String,
        /// Max results
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Create a new thread
    Post {
        /// Threads account ID
        threads_id: String,
        /// Post text
        text: String,
        /// Optional image/video URL
        #[arg(long)]
        media_url: Option<String>,
    },
    /// Reply to a thread
    Reply {
        /// Threads account ID
        threads_id: String,
        /// Media ID to reply to
        media_id: String,
        /// Reply text
        text: String,
    },
    /// Delete a thread
    Delete {
        /// Threads account ID
        threads_id: String,
        /// Media ID to delete
        media_id: String,
    },
    /// Get insights/analytics
    Insights {
        /// Threads account ID
        threads_id: String,
        /// Metric (e.g. impression_count)
        metric: String,
        /// Period (day, etc.)
        #[arg(long)]
        period: Option<String>,
    },
}

// ─── Discord

#[derive(Subcommand, Debug)]
pub enum DiscordAction {
    /// List channels in a guild
    Channels {
        /// Guild ID
        guild_id: String,
    },
    /// Get messages from a channel
    Messages {
        /// Channel ID
        channel_id: String,
        /// Max messages
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Send a message to a channel
    Send {
        /// Channel ID
        channel_id: String,
        /// Message text
        text: String,
    },
    /// Get server/guild info
    Server {
        /// Guild ID
        guild_id: String,
    },
    /// Create a forum post
    Forum {
        /// Channel ID (forum channel)
        channel_id: String,
        /// Post title
        title: String,
        /// Post content
        content: String,
    },
}

// ─── Slack

#[derive(Subcommand, Debug)]
pub enum SlackAction {
    /// List channels
    Channels,
    /// Get channel history
    History {
        /// Channel ID
        channel_id: String,
        /// Max messages
        #[arg(long, default_value_t = 50)]
        limit: i32,
    },
    /// Send a message
    Send {
        /// Channel ID
        channel_id: String,
        /// Message text
        text: String,
    },
    /// List users
    Users,
}

// ─── Telegram Bot

#[derive(Subcommand, Debug)]
pub enum TelegramBotAction {
    /// Send a message
    Send {
        /// Bot token index (0-based)
        #[arg(long, default_value_t = 0)]
        bot_index: usize,
        /// Chat ID
        chat_id: String,
        /// Message text
        text: String,
    },
    /// Send a photo
    Photo {
        #[arg(long, default_value_t = 0)]
        bot_index: usize,
        chat_id: String,
        /// Photo URL
        url: String,
        #[arg(long)]
        caption: Option<String>,
    },
    /// Send a document
    Document {
        #[arg(long, default_value_t = 0)]
        bot_index: usize,
        chat_id: String,
        /// Document URL or path
        path: String,
        #[arg(long)]
        caption: Option<String>,
    },
    /// Get chat info
    Chat {
        #[arg(long, default_value_t = 0)]
        bot_index: usize,
        chat_id: String,
    },
    /// Get updates
    Updates {
        #[arg(long, default_value_t = 0)]
        bot_index: usize,
    },
}

// ─── Telegram User

#[derive(Subcommand, Debug)]
pub enum TelegramUserAction {
    /// Send a message
    Send {
        /// Peer (username, phone, or chat ID)
        peer: String,
        /// Message text
        text: String,
    },
    /// List dialogs/conversations
    Dialogs {
        /// Max results
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List contacts
    Contacts,
    /// Search messages
    Search {
        /// Search query
        query: String,
    },
}

// ─── WhatsApp

#[derive(Subcommand, Debug)]
pub enum WhatsappAction {
    /// Send a text message
    Send {
        /// Recipient phone number
        to: String,
        /// Message text
        text: String,
    },
    /// List chats
    Chats {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List contacts
    Contacts {
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List groups
    Groups,
    /// Create a group
    CreateGroup {
        /// Group name
        name: String,
        /// Participant phone numbers
        participants: Vec<String>,
    },
    /// Get invite link for a group
    InviteLink {
        /// Group JID
        group_jid: String,
    },
}

// ─── Pinterest

#[derive(Subcommand, Debug)]
pub enum PinterestAction {
    /// Get user account info
    Profile {
        /// Board ID (used for token resolution)
        board_id: String,
    },
    /// Get board details
    Board { board_id: String },
    /// Get pins on a board
    Pins {
        board_id: String,
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
    /// Get a single pin
    Pin { board_id: String, pin_id: String },
    /// Search pins
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get board analytics
    BoardAnalytics {
        board_id: String,
        #[arg(long)]
        start_date: String,
        #[arg(long)]
        end_date: String,
    },
    /// Get pin analytics
    PinAnalytics {
        board_id: String,
        pin_id: String,
        #[arg(long)]
        start_date: String,
        #[arg(long)]
        end_date: String,
    },
}

// ─── GitHub

#[derive(Subcommand, Debug)]
pub enum GithubAction {
    /// Get authenticated user info
    Me,
    /// Get my repos
    MyRepos {
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Get a user's profile
    User { login: String },
    /// List repos for a user
    Repos {
        username: String,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// List issues
    Issues {
        owner: String,
        repo: String,
        #[arg(long)]
        state_filter: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// List pull requests
    Prs {
        owner: String,
        repo: String,
        #[arg(long)]
        state_filter: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Create an issue
    CreateIssue {
        owner: String,
        repo: String,
        title: String,
        #[arg(long)]
        body: Option<String>,
    },
    /// Close an issue
    CloseIssue {
        owner: String,
        repo: String,
        number: u32,
    },
    /// List commits
    Commits {
        owner: String,
        repo: String,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// List branches
    Branches {
        owner: String,
        repo: String,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Search repos
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// List releases
    Releases {
        owner: String,
        repo: String,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
}

// ─── WordPress

#[derive(Subcommand, Debug)]
pub enum WordpressAction {
    /// Create a post
    Post {
        title: String,
        content: String,
        #[arg(long)]
        status: Option<String>,
    },
    /// List posts
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Get a post by ID
    Get { id: i32 },
    /// List categories
    Categories,
}

// ─── Hashnode

#[derive(Subcommand, Debug)]
pub enum HashnodeAction {
    /// Create a post
    Post {
        publication_id: String,
        title: String,
        content: String,
    },
    /// List posts
    List {
        publication_id: String,
        #[arg(long)]
        page: Option<i32>,
    },
    /// Get a post
    Get { post_id: String },
}

// ─── Medium

#[derive(Subcommand, Debug)]
pub enum MediumBlogAction {
    /// Create a post
    Post {
        #[arg(long)]
        title: Option<String>,
        content: String,
        #[arg(long)]
        tags: Option<Vec<String>>,
    },
    /// List posts
    List,
    /// Get a post
    Get { id: String },
}

// ─── Dev.to

#[derive(Subcommand, Debug)]
pub enum DevtoAction {
    /// Create an article
    Post {
        #[arg(long)]
        title: Option<String>,
        content: String,
        #[arg(long)]
        tags: Option<Vec<String>>,
        /// Publish immediately (default: save as draft)
        #[arg(long, default_value_t = false)]
        publish: bool,
    },
    /// List articles
    List,
    /// Get an article
    Get { id: String },
}

// ─── Skool

#[derive(Subcommand, Debug)]
pub enum SkoolAction {
    /// Publish a post
    Post {
        group_id: String,
        title: String,
        content: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Get community info
    Info { community_slug: String },
    /// List posts
    Posts {
        community_slug: String,
        #[arg(long)]
        page: Option<u32>,
        #[arg(long)]
        sort: Option<String>,
    },
    /// Create a comment
    Comment {
        post_id: String,
        group_id: String,
        content: String,
    },
}

// ─── Google (YouTube)

#[derive(Subcommand, Debug)]
pub enum GoogleAction {
    /// Search YouTube videos
    YoutubeSearch {
        channel_id: String,
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Get video details
    Video {
        channel_id: String,
        video_id: String,
    },
    /// List playlists
    Playlists {
        channel_id: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Get channel stats
    ChannelStats { channel_id: String },
}

// ─── Google Drive

#[derive(Subcommand, Debug)]
pub enum DriveAction {
    /// List files
    Files {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get a file
    File { file_id: String },
    /// Search files
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List folders
    Folders {
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Get file metadata
    Metadata { file_id: String },
    /// Export a file
    Export { file_id: String, mime_type: String },
}

// ─── Google Calendar

#[derive(Subcommand, Debug)]
pub enum GcalAction {
    /// List calendars
    Calendars,
    /// List events
    Events {
        #[arg(long)]
        calendar_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get a specific event
    Event {
        #[arg(long)]
        calendar_id: Option<String>,
        event_id: String,
    },
    /// Create an event
    Create {
        #[arg(long)]
        calendar_id: Option<String>,
        title: String,
        start: String,
        end: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// Update an event
    Update {
        #[arg(long)]
        calendar_id: Option<String>,
        event_id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Delete an event
    Delete {
        #[arg(long)]
        calendar_id: Option<String>,
        event_id: String,
    },
}

// ─── Gmail

#[derive(Subcommand, Debug)]
pub enum GmailAction {
    /// Get Gmail profile
    Profile,
    /// List messages
    Messages {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        query: Option<String>,
    },
    /// Get a message
    Message { id: String },
    /// Send an email
    Send {
        to: String,
        subject: String,
        body: String,
    },
    /// List labels
    Labels,
    /// Get a thread
    Thread { id: String },
    /// Search messages
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

// ─── Webhooks

#[derive(Subcommand, Debug)]
pub enum WebhooksAction {
    /// List webhooks
    List,
    /// Create a webhook
    Create {
        /// Webhook URL
        url: String,
        /// Webhook name
        #[arg(long, default_value = "webhook")]
        name: String,
    },
    /// Delete a webhook
    Delete {
        /// Webhook ID
        id: String,
    },
    /// Get a webhook by ID
    Get {
        /// Webhook ID
        id: String,
    },
    /// Update a webhook
    Update {
        /// Webhook ID
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New URL
        #[arg(long)]
        url: Option<String>,
        /// Enable/disable
        #[arg(long)]
        active: Option<bool>,
    },
    /// Test a webhook
    Test {
        /// Webhook ID
        id: String,
    },
}

// ─── Notifications

#[derive(Subcommand, Debug)]
pub enum NotificationsAction {
    /// List notifications
    List {
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Mark a notification as read
    Read { id: String },
    /// Mark all notifications as read
    ReadAll,
    /// Create a notification
    Create {
        /// Title
        title: String,
        /// Body
        body: String,
    },
}

// ─── Tags

#[derive(Subcommand, Debug)]
pub enum TagsAction {
    /// List tags
    List,
    /// Create a tag
    Create {
        name: String,
        #[arg(long)]
        color: Option<String>,
    },
    /// Delete a tag
    Delete { id: String },

    /// Get a tag by ID
    Get {
        /// Tag ID
        id: String,
    },
    /// Update a tag
    Update {
        /// Tag ID
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New color
        #[arg(long)]
        color: Option<String>,
    },
}

// ─── Analytics

#[derive(Subcommand, Debug)]
pub enum AnalyticsAction {
    /// Get analytics for a provider
    Get {
        /// Provider name (instagram, facebook, etc.)
        provider: String,
        /// Number of days
        #[arg(long, default_value_t = 7)]
        days: i32,
    },
    /// Get analytics for a specific post
    Post {
        /// Post ID (UUID)
        post_id: String,
    },
}
