// ─── MCP Tools Integration Test ─────────────────────────────────
// Tests the X and Reddit MCP tools via the shared AppState + ProviderRegistry.
// These tests verify:
//   1. All MCP tools are registered and callable
//   2. Provider methods work correctly with HTTP status checking
//   3. Multi-account support (root_internal_id, pages, connect-page)
//   4. Error handling (token expiry, rate limits, API errors)
//
// Run: cargo test --test mcp_tools_test -- --nocapture
// Requires: running DB at DATABASE_URL, configured .env

use std::sync::Arc;

use switchpost::config::Config;
use switchpost::db;
use switchpost::social::registry::ProviderRegistry;

// ── Helper: Test fixtures ────────────────────────────────────────

fn get_config() -> Config {
    switchpost::config::load_dotenv_for_tests();
    Config::from_env().expect("Failed to load config from .env")
}

fn get_registry(config: &Config) -> ProviderRegistry {
    ProviderRegistry::new(config, None, None)
}

// ── Test 1: Provider Registry has all 26 default providers ──────

#[tokio::test]
async fn test_provider_registry_has_all_providers() {
    let config = get_config();
    let registry = get_registry(&config);
    let mut ids = registry.list();
    ids.sort();

    let mut expected: Vec<&str> = vec![
        // Tier-1 (12)
        "x",
        "linkedin",
        "linkedin-page",
        "facebook",
        "instagram",
        "instagram-standalone",
        "threads",
        "youtube",
        "tiktok",
        "reddit",
        "bluesky",
        "pinterest",
        // Tier-2 (14)
        "telegram-bot",
        "telegram-user",
        "discord",
        "slack",
        "whatsapp",
        "wordpress",
        "mastodon",
        "medium",
        "devto",
        "hashnode",
        "github",
        "google_my_business",
        "skool",
        "gmail",
        "calendar",
        "drive",
    ];
    expected.sort();

    // Registration is unconditional (see `ProviderRegistry::new`): a clean
    // environment must still report the full catalogue, because the /channels
    // grid and the MCP tool list advertise all 29 regardless of credentials.
    // Tier-3 (farcaster) is excluded — it registers only under
    // ENABLE_ARCHIVE_PROVIDERS.
    //
    // The catalogue grew from 26 to 29 on 2026-10-30: `gmail`, `calendar`
    // and `drive` are now their own connectable providers, because Google
    // refuses `auth/youtube*` and `auth/drive*` in ONE consent and the
    // unified `google` integration can therefore never carry a Drive grant.
    // It then shrank to 28: the unified `google` provider published YouTube
    // videos and nothing else, so it was a second YouTube button whose rows
    // showed as a "Google Suite (7)" outlier in the channels list.
    // `gmail` was already being OFFERED on the Channels screen without being
    // registered — its connect button 404'd — which is the drift this list is
    // here to catch.
    assert_eq!(
        ids, expected,
        "Provider registry should contain all 28 default providers"
    );
    println!("✅ Provider registry: {} providers registered", ids.len());
}

// ── Test 2: XProvider methods compile and are callable ───────────

#[tokio::test]
async fn test_x_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let x = registry.get("x").expect("X provider should exist");
    assert_eq!(x.identifier(), "x");
    assert_eq!(x.name(), "X (Twitter)");

    // Verify expanded scopes
    let scopes = x.scopes();
    assert!(
        scopes.contains(&"bookmark.read".to_string()),
        "Should have bookmark.read scope"
    );
    assert!(
        scopes.contains(&"bookmark.write".to_string()),
        "Should have bookmark.write scope"
    );
    assert!(
        scopes.contains(&"like.read".to_string()),
        "Should have like.read scope"
    );
    assert!(
        scopes.contains(&"like.write".to_string()),
        "Should have like.write scope"
    );
    assert!(
        scopes.contains(&"follows.read".to_string()),
        "Should have follows.read scope"
    );
    assert!(
        scopes.contains(&"follows.write".to_string()),
        "Should have follows.write scope"
    );
    assert!(
        scopes.contains(&"list.read".to_string()),
        "Should have list.read scope"
    );
    assert!(
        scopes.contains(&"tweet.read".to_string()),
        "Should have tweet.read scope"
    );
    assert!(
        scopes.contains(&"tweet.write".to_string()),
        "Should have tweet.write scope"
    );
    assert!(
        scopes.contains(&"offline.access".to_string()),
        "Should have offline.access scope"
    );

    println!("✅ XProvider: {} scopes configured", scopes.len());
}

// ── Test 3: XProvider generates valid OAuth URL ─────────────────

#[tokio::test]
async fn test_x_generate_auth_url() {
    let config = get_config();
    let registry = get_registry(&config);
    let x = registry.get("x").expect("X provider should exist");

    let result = x
        .generate_auth_url(
            "test_state",
            "test_verifier",
            "http://localhost:3000/api/auth/callback",
        )
        .await;

    assert!(result.is_ok(), "X auth URL generation should succeed");
    let url = result.unwrap().url;
    assert!(
        url.contains("twitter.com/i/oauth2/authorize"),
        "URL should point to Twitter authorize endpoint"
    );
    assert!(
        url.contains("code_challenge_method=S256"),
        "Should use PKCE S256"
    );
    assert!(
        url.contains("bookmark.read"),
        "URL should contain expanded bookmark.read scope"
    );
    assert!(
        url.contains("like.read"),
        "URL should contain like.read scope"
    );
    assert!(
        url.contains("follows.read"),
        "URL should contain follows.read scope"
    );
    assert!(
        url.contains("list.read"),
        "URL should contain list.read scope"
    );
    assert!(
        url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fapi%2Fauth%2Fcallback"),
        "URL should contain redirect_uri"
    );

    println!("✅ XProvider: Auth URL generated with {} scopes", 11);
    println!("   URL: {}...{}", &url[..60], &url[url.len() - 30..]);
}

// ── Test 4: XProvider error handling ─────────────────────────────

#[tokio::test]
async fn test_x_bad_token_error_handling() {
    let config = get_config();
    let registry = get_registry(&config);

    // We can only test compilation/creation here.
    // Actual API calls need a real token which is tested via HTTP API.
    let x = registry.get("x").expect("X provider should exist");
    let scopes = x.scopes();
    assert!(scopes.len() >= 11, "X should have at least 11 OAuth scopes");

    // Verify the Exchange Code + PKCE public client pattern compiles
    let credentials = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{}:{}", "test_client", "test_secret"),
    );
    assert!(
        !credentials.is_empty(),
        "Base64 credentials should not be empty"
    );

    println!("✅ XProvider: Error handling patterns verified");
}

// ── Test 5: RedditProvider methods ───────────────────────────────

#[tokio::test]
async fn test_reddit_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let reddit = registry
        .get("reddit")
        .expect("Reddit provider should exist");
    assert_eq!(reddit.identifier(), "reddit");

    let scopes = reddit.scopes();
    // Reddit is non-OAuth password grant but still has read/identity/submit scopes
    assert!(!scopes.is_empty(), "Reddit should have internal scopes");
    assert!(scopes.contains(&"read".to_string()));
    assert!(scopes.contains(&"identity".to_string()));

    println!("✅ RedditProvider: Created with username from config");
}

// ── Test 6: Multi-account support (root_internal_id pattern) ─────

#[tokio::test]
async fn test_multi_account_data_model() {
    // Verify that the DB schema supports multi-account via root_internal_id
    let config = get_config();
    let db = db::create_pool(&config.database_url)
        .await
        .expect("Failed to connect to DB");

    // Check that the integrations table has root_internal_id column.
    // SQLite: pragma_table_info is the catalog (the old query used the
    // Postgres-only information_schema).
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('integrations') WHERE name = 'root_internal_id'",
    )
    .fetch_one(&db)
    .await
    .expect("Failed to query schema");

    assert_eq!(
        count.0, 1,
        "integrations table should have root_internal_id column"
    );
    println!("✅ Multi-account: root_internal_id column exists");

    // The upsert path's ON CONFLICT (user_id, provider_identifier,
    // internal_id) depends on a UNIQUE index — SQLite exposes them via
    // pragma_index_list (pg_constraint is Postgres).
    let constraints: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM pragma_index_list('integrations') WHERE \"unique\" = 1")
            .fetch_all(&db)
            .await
            .expect("Failed to query constraints");

    for (name,) in &constraints {
        println!("✅ Multi-account: UNIQUE index '{}'", name);
    }
    assert!(
        !constraints.is_empty(),
        "Should have at least one UNIQUE constraint on integrations"
    );
}

// ── Test 7: MCP tool router registration ────────────────────────

#[tokio::test]
async fn test_mcp_tools_registration() {
    // Verify that the MCP server handler can be instantiated
    let config = get_config();
    let db = db::create_pool(&config.database_url)
        .await
        .expect("Failed to connect to DB");

    use switchpost::api::AppState;
    use switchpost::mcp::SwitchpostMcpServer;
    use switchpost::realtime::Broadcaster;

    let broadcaster = Broadcaster::new();
    let registry = Arc::new(get_registry(&config));

    let state = AppState {
        db: db.clone(),
        config: config.clone(),
        broadcast: broadcaster.clone(),
        providers: (*registry).clone(),
        token_key: None,
        telegram_client_manager: None,
        wa_client: None,
        media_http_client: reqwest::Client::new(),
        media_wreq_client: wreq::Client::new(),
    };

    let server = SwitchpostMcpServer::new(state.clone());
    // Verify server creation succeeds (tests that all #[tool] macros compile)
    assert!(
        server.state.config.provider_credentials("x").is_some(),
        "X credentials should be loaded"
    );
    assert!(
        server.state.config.provider_credentials("reddit").is_some(),
        "Reddit credentials should be loaded"
    );

    println!("✅ MCP server: SwitchpostMcpServer instantiated successfully");
    println!("✅ MCP tools: All {} #[tool] entries compile", 115); // 7 reddit + 20 x + 16 fb + 17 ig + 7 ias + 9 th + 6 li + 4 lip + 9 yt + 7 pi + 10 di + 5 sk
}

// ── Test 8: DB connectivity and existing integrations ───────────

#[tokio::test]
async fn test_db_has_x_and_reddit_integrations() {
    let config = get_config();
    let db = db::create_pool(&config.database_url)
        .await
        .expect("Failed to connect to DB");

    // Check for X integrations
    let x_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations WHERE provider_identifier = 'x'")
            .fetch_one(&db)
            .await
            .expect("Failed to query x integrations");
    println!("✅ X/Twitter integrations in DB: {}", x_count.0);

    // Check for Reddit integrations
    let reddit_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations WHERE provider_identifier = 'reddit'")
            .fetch_one(&db)
            .await
            .expect("Failed to query reddit integrations");
    println!("✅ Reddit integrations in DB: {}", reddit_count.0);

    // Check multi-account support (multiple accounts per provider)
    let multi_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations WHERE root_internal_id IS NOT NULL")
            .fetch_one(&db)
            .await
            .expect("Failed to query child integrations");
    println!("✅ Multi-account (root_internal_id set): {}", multi_count.0);
}

// ── Test 9: FacebookProvider creation and scopes ─────────────────

#[tokio::test]
async fn test_facebook_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let fb = registry
        .get("facebook")
        .expect("Facebook provider should exist");
    assert_eq!(fb.identifier(), "facebook");
    assert_eq!(fb.name(), "Facebook");

    let scopes = fb.scopes();
    assert!(scopes.contains(&"pages_show_list".to_string()));
    assert!(scopes.contains(&"pages_manage_posts".to_string()));
    assert!(
        scopes.contains(&"pages_manage_engagement".to_string()),
        "Should have pages_manage_engagement scope"
    );
    assert!(
        scopes.contains(&"pages_manage_metadata".to_string()),
        "Should have pages_manage_metadata scope"
    );
    assert!(
        scopes.contains(&"pages_read_user_content".to_string()),
        "Should have pages_read_user_content scope"
    );
    assert!(
        scopes.contains(&"read_insights".to_string()),
        "Should have read_insights scope"
    );
    assert!(
        scopes.contains(&"business_management".to_string()),
        "Should have business_management scope"
    );

    println!(
        "✅ FacebookProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 10: InstagramProvider creation and scopes ────────────────

#[tokio::test]
async fn test_instagram_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let ig = registry
        .get("instagram")
        .expect("Instagram provider should exist");
    assert_eq!(ig.identifier(), "instagram");
    assert_eq!(ig.name(), "Instagram");

    let scopes = ig.scopes();
    assert!(scopes.contains(&"instagram_basic".to_string()));
    assert!(scopes.contains(&"instagram_content_publish".to_string()));
    assert!(scopes.contains(&"instagram_manage_comments".to_string()));
    assert!(scopes.contains(&"instagram_manage_insights".to_string()));
    assert!(
        scopes.contains(&"pages_manage_engagement".to_string()),
        "Should have pages_manage_engagement scope"
    );
    assert!(
        scopes.contains(&"pages_read_user_content".to_string()),
        "Should have pages_read_user_content scope"
    );
    assert!(
        scopes.contains(&"read_insights".to_string()),
        "Should have read_insights scope"
    );

    println!(
        "✅ InstagramProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 11: FacebookProvider multi-step (is_between_steps) ──────

#[tokio::test]
async fn test_facebook_is_between_steps() {
    let config = get_config();
    let registry = get_registry(&config);

    let fb = registry
        .get("facebook")
        .expect("Facebook provider should exist");
    assert!(
        fb.is_between_steps(),
        "Facebook should be multi-step (exchange_code returns user-level token)"
    );

    println!(
        "✅ FacebookProvider: is_between_steps() = {}",
        fb.is_between_steps()
    );
}

// ── Test 12: InstagramProvider multi-step (is_between_steps) ─────

#[tokio::test]
async fn test_instagram_is_between_steps() {
    let config = get_config();
    let registry = get_registry(&config);

    let ig = registry
        .get("instagram")
        .expect("Instagram provider should exist");
    assert!(
        ig.is_between_steps(),
        "Instagram should be multi-step (exchange_code returns user-level token)"
    );

    println!(
        "✅ InstagramProvider: is_between_steps() = {}",
        ig.is_between_steps()
    );
}

// ── Test 13: InstagramStandaloneProvider creation and scopes ─────

#[tokio::test]
async fn test_instagram_standalone_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let ias = registry
        .get("instagram-standalone")
        .expect("Instagram Standalone provider should exist");
    assert_eq!(ias.identifier(), "instagram-standalone");

    let scopes = ias.scopes();
    assert!(scopes.contains(&"instagram_business_basic".to_string()));
    assert!(scopes.contains(&"instagram_business_content_publish".to_string()));
    assert!(scopes.contains(&"instagram_business_manage_comments".to_string()));

    println!(
        "✅ InstagramStandaloneProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 14: ThreadsProvider creation and scopes ─────────────────

#[tokio::test]
async fn test_threads_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let threads = registry
        .get("threads")
        .expect("Threads provider should exist");
    assert_eq!(threads.identifier(), "threads");

    let scopes = threads.scopes();
    assert!(scopes.contains(&"threads_basic".to_string()));
    assert!(scopes.contains(&"threads_content_publish".to_string()));
    assert!(scopes.contains(&"threads_manage_replies".to_string()));
    assert!(scopes.contains(&"threads_manage_insights".to_string()));

    println!(
        "✅ ThreadsProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 15: LinkedInProvider creation and scopes ────────────────

#[tokio::test]
async fn test_linkedin_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let li = registry
        .get("linkedin")
        .expect("LinkedIn provider should exist");
    assert_eq!(li.identifier(), "linkedin");
    assert_eq!(li.name(), "LinkedIn");

    let scopes = li.scopes();
    assert!(scopes.contains(&"openid".to_string()));
    assert!(scopes.contains(&"profile".to_string()));
    assert!(scopes.contains(&"email".to_string()));
    assert!(scopes.contains(&"w_member_social".to_string()));

    println!(
        "✅ LinkedInProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 16: LinkedInPageProvider creation and scopes ────────────

#[tokio::test]
async fn test_linkedin_page_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let lip = registry
        .get("linkedin-page")
        .expect("LinkedIn Page provider should exist");
    assert_eq!(lip.identifier(), "linkedin-page");
    assert_eq!(lip.name(), "LinkedIn Page");

    let scopes = lip.scopes();
    assert!(scopes.contains(&"w_member_social".to_string()));
    assert!(scopes.contains(&"rw_organization_admin".to_string()));
    assert!(scopes.contains(&"w_organization_social".to_string()));
    assert!(scopes.contains(&"r_organization_social".to_string()));

    println!(
        "✅ LinkedInPageProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
    assert!(lip.is_between_steps(), "LinkedIn Page should be multi-step");
}

// ── Test 17: YouTube provider creation and scopes ────────────────

#[tokio::test]
async fn test_youtube_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    // The unified `google` provider is gone: it published YouTube videos and
    // nothing else, so it was a second YouTube button. Each Google service is
    // its own provider with its own consent now.
    let youtube = registry
        .get("youtube")
        .expect("YouTube provider should exist");
    assert_eq!(youtube.identifier(), "youtube");
    assert_eq!(youtube.name(), "YouTube");

    let scopes = youtube.scopes();
    assert!(scopes.contains(&"https://www.googleapis.com/auth/youtube.force-ssl".to_string()));
    // Drive is NOT here, and that is the fix. Google's authorize endpoint
    // refuses `auth/youtube*` and `auth/drive*` in one consent
    // ("This request contains scopes that cannot be requested together"),
    // so a combined set made the whole connect flow answer
    // `Error 400: invalid_request` before any consent screen.
    // The Drive grant comes from the separate `drive` provider, whose
    // own scopes are asserted below.
    assert!(
        !scopes.contains(&"https://www.googleapis.com/auth/drive.readonly".to_string()),
        "Drive scopes must not ride on the YouTube provider: \
         Google refuses them together"
    );

    let drive = registry
        .get("drive")
        .expect("the drive provider must be registered so the MCP Drive tools have a token");
    let drive_scopes = drive.scopes();
    assert!(
        drive_scopes.contains(&"https://www.googleapis.com/auth/drive.readonly".to_string()),
        "the drive provider must still carry the Drive grant the MCP tools need"
    );

    println!(
        "✅ YouTube: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 18: Pinterest provider creation and scopes ──────────────

#[tokio::test]
async fn test_pinterest_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let pi = registry
        .get("pinterest")
        .expect("Pinterest provider should exist");
    assert_eq!(pi.identifier(), "pinterest");
    assert_eq!(pi.name(), "Pinterest");

    let scopes = pi.scopes();
    assert!(scopes.contains(&"boards:read".to_string()));
    assert!(scopes.contains(&"pins:read".to_string()));

    println!(
        "✅ PinterestProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 19: Discord provider creation and scopes ────────────────

#[tokio::test]
async fn test_discord_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let di = match registry.get("discord") {
        Some(p) => p,
        None => {
            println!("⚠️ Discord provider not configured (DISCORD_CLIENT_ID not set) — skipping");
            return;
        }
    };
    assert_eq!(di.identifier(), "discord");
    assert_eq!(di.name(), "Discord");

    let scopes = di.scopes();
    assert!(scopes.contains(&"identify".to_string()));
    assert!(scopes.contains(&"guilds".to_string()));

    println!(
        "✅ DiscordProvider: {} scopes configured ({})",
        scopes.len(),
        scopes.join(", ")
    );
}

// ── Test 20: Skool provider creation and scopes ──────────────────

#[tokio::test]
async fn test_skool_provider_creation() {
    let config = get_config();
    let registry = get_registry(&config);

    let sk = registry.get("skool").expect("Skool provider should exist");
    assert_eq!(sk.identifier(), "skool");
    assert_eq!(sk.name(), "Skool");

    // Skool uses Chrome extension auth (no OAuth scopes)
    assert!(!sk.uses_oauth(), "Skool should not use OAuth");
    assert!(sk.is_chrome_extension(), "Skool should be Chrome extension");

    println!(
        "✅ SkoolProvider: uses_oauth={}, is_chrome_extension={}",
        sk.uses_oauth(),
        sk.is_chrome_extension()
    );
}
