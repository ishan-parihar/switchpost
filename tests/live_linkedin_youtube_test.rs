// ─── LinkedIn & YouTube Live Integration Tests ─────────────────────
// Tests LinkedIn (personal + page) and YouTube providers with
// real OAuth tokens from the database.
//
// Run: cargo test --test live_linkedin_youtube_test -- --nocapture
// Requires: running DB at DATABASE_URL, configured .env with
//   TOKEN_ENCRYPTION_KEY and OAuth client credentials.
//
// Connects to the DB, looks up the dev user (dev@switchpost.dev),
// fetches encrypted tokens for linkedin / linkedin-page / youtube,
// decrypts them, and calls every public provider method.

use std::sync::OnceLock;
use uuid::Uuid;

use switchpost::config::Config;
use switchpost::crypto;
use switchpost::db;
use switchpost::db::queries;
use switchpost::social::linkedin::LinkedInProvider;
use switchpost::social::linkedin_page::LinkedInPageProvider;
use switchpost::social::youtube::YoutubeProvider;
use switchpost::social::SocialProvider;

static CONFIG: OnceLock<Config> = OnceLock::new();

fn get_config() -> &'static Config {
    CONFIG.get_or_init(|| {
        switchpost::config::load_dotenv_for_live_tests();
        Config::from_env().expect("Failed to load config from env")
    })
}

fn log_result(name: &str, ok: bool, detail: &str) {
    let mark = if ok { "✅" } else { "❌" };
    println!("  {mark} {name:45} {detail}");
}

fn log_skip(name: &str, reason: &str) {
    println!("  ⏭️  {name:45} SKIPPED: {reason}");
}

/// Decrypt an encrypted DB token using the configured TOKEN_ENCRYPTION_KEY.
/// If the token is already plaintext (not hex-encrypted), returns it as-is.
fn decrypt_token(raw: &str) -> Option<String> {
    // Detect plaintext tokens: e.g. "EAAMx0ZAX...", "ya29...", "AQWqoQlE...", JSON blobs
    if !raw.is_empty() && !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(raw.to_string());
    }
    let config = get_config();
    let hex_key = config.token_encryption_key.as_ref()?;
    let key = crypto::decode_hex_key(hex_key).ok()?;
    crypto::decrypt_string(raw, &key).ok()
}

/// The account this suite looks up when LIVE_TEST_EMAIL is not set.
///
/// It is a FIXTURE address, deliberately: a test that defaulted to the
/// operator's real account would run against a real account by accident.
const LIVE_TEST_EMAIL_DEFAULT: &str = "[REDACTED:email]";

/// The account this run looks up: the operator's override, or the fixture.
fn live_test_email() -> String {
    std::env::var("LIVE_TEST_EMAIL").unwrap_or_else(|_| LIVE_TEST_EMAIL_DEFAULT.to_string())
}

/// Why the suite cannot run, in the founder's terms.
///
/// The old message was "dev user not in DB - visit the onboarding page
/// first", and it was WRONG in the common case: the operator's database
/// HAS a user and HAS integrations (github, reddit, x), so telling them to
/// go complete onboarding they already completed sends them to do work that
/// cannot help. The two real causes are (a) the account this test looks up
/// is not the account in the database, and (b) that account has no
/// linkedin/youtube integration. This returns the ACTUAL cause, so the
/// message names it.
fn skip_reason(user_missing: bool, email: &str) -> String {
    if user_missing {
        format!(
            "no account {email} in this database - set LIVE_TEST_EMAIL to the \
             account that carries the linkedin/youtube channels (the default \
             is a fixture address no real installation uses)"
        )
    } else {
        "the account is in the database but has no linkedin or youtube \
         integration - connect one first"
            .to_string()
    }
}

/// Fetch all integrations for the live-test account from the DB.
///
/// The account is configurable: `LIVE_TEST_EMAIL` selects it, because the
/// default fixture address belongs to nobody, and a suite that can only ever
/// run on a fresh install cannot test an operator's real setup.
async fn get_dev_user_integrations() -> Option<(Vec<db::models::Integration>, Uuid)> {
    let config = get_config();
    let pool = db::create_pool(&config.database_url)
        .await
        .expect("Failed to connect to DB");

    // Look up the live-test account.
    let email = std::env::var("LIVE_TEST_EMAIL")
        .unwrap_or_else(|_| LIVE_TEST_EMAIL_DEFAULT.to_string());
    let Some(user) = queries::get_user_by_email(&pool, &email).await.expect("DB error") else {
        return None;
    };

    let integrations = queries::list_integrations(&pool, user.id)
        .await
        .expect("Failed to list integrations");

    Some((integrations, user.id))
}

// ═════════════════════════════════════════════════════════════════
// LINKEDIN PERSONAL
// ═════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_li_get_profile() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_get_profile", "no linkedin integration in DB");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_get_profile", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    match provider.get_profile(&token).await {
        Ok(data) => log_result("li_get_profile", true, &format!("{:.120}", data)),
        Err(e) => log_result("li_get_profile", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_li_get_user_id() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_get_user_id", "no linkedin integration");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_get_user_id", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    match provider.get_user_id(&token).await {
        Ok(id) => log_result("li_get_user_id", true, &format!("user_id={id}")),
        Err(e) => log_result("li_get_user_id", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_li_get_posts() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_get_posts", "no linkedin integration");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_get_posts", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    // The author URN is the internal_id from DB (e.g., "MzKoMN58Rn")
    let author_urn = &li.internal_id;
    match provider.get_posts(&token, author_urn, 5).await {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            let preview = serde_json::to_string(&data).unwrap_or_default();
            log_result(
                "li_get_posts",
                true,
                &format!("{arr} posts, preview={:.120}", preview),
            );
        }
        Err(e) => log_result("li_get_posts", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_li_get_post_detail() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_get_post_detail", "no linkedin integration");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_get_post_detail", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    // First try to get posts to find a real post_urn
    let author_urn = &li.internal_id;
    let post_urn = match provider.get_posts(&token, author_urn, 1).await {
        Ok(data) => {
            data.as_array()
                .and_then(|a| a.first())
                .and_then(|p| p.as_object())
                .and_then(|o| {
                    // Try common field names for post URN
                    o.get("id")
                        .or_else(|| o.get("urn"))
                        .or_else(|| o.get("post_urn"))
                })
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
        Err(_) => None,
    };

    let post_urn = match post_urn {
        Some(p) => p,
        None => {
            log_skip("li_get_post_detail", "no posts found to get detail from");
            return;
        }
    };

    match provider.get_post_detail(&token, &post_urn).await {
        Ok(data) => log_result("li_get_post_detail", true, &format!("{:.120}", data)),
        Err(e) => log_result("li_get_post_detail", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_li_get_post_comments() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_get_post_comments", "no linkedin integration");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_get_post_comments", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    // Get posts first to find a real post_urn for comment lookup
    let author_urn = &li.internal_id;
    let post_urn = match provider.get_posts(&token, author_urn, 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_object())
            .and_then(|o| {
                o.get("id")
                    .or_else(|| o.get("urn"))
                    .or_else(|| o.get("post_urn"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let post_urn = match post_urn {
        Some(p) => p,
        None => {
            log_skip("li_get_post_comments", "no posts found");
            return;
        }
    };

    match provider.get_post_comments(&token, &post_urn).await {
        Ok(data) => {
            let arr = data.len();
            log_result("li_get_post_comments", true, &format!("{arr} comments"));
        }
        Err(e) => log_result("li_get_post_comments", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_li_create_comment() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin");
    let Some(li) = li else {
        log_skip("li_create_comment", "no linkedin integration");
        return;
    };
    let token = match decrypt_token(&li.access_token) {
        Some(t) => t,
        None => {
            log_result("li_create_comment", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInProvider::new(get_config());

    // Get posts first to find a real post_urn
    let author_urn = &li.internal_id;
    let post_urn = match provider.get_posts(&token, author_urn, 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_object())
            .and_then(|o| {
                o.get("id")
                    .or_else(|| o.get("urn"))
                    .or_else(|| o.get("post_urn"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let (post_urn, person_urn) = match post_urn {
        Some(p) => (p, li.internal_id.clone()),
        None => {
            log_skip("li_create_comment", "no posts found to comment on");
            return;
        }
    };

    match provider
        .create_comment(
            &token,
            &post_urn,
            &person_urn,
            "Test comment from integration test — please ignore",
        )
        .await
    {
        Ok(data) => log_result("li_create_comment", true, &format!("{:.120}", data)),
        Err(e) => log_result("li_create_comment", false, &format!("{e}")),
    }
}

// ═════════════════════════════════════════════════════════════════
// LINKEDIN PAGE
// ═════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_lip_get_page_posts() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let pages: Vec<_> = integrations
        .iter()
        .filter(|i| i.provider_identifier == "linkedin-page")
        .collect();
    if pages.is_empty() {
        log_skip("lip_get_page_posts", "no linkedin-page integrations in DB");
        return;
    }
    let provider = LinkedInPageProvider::new(get_config());

    for page in pages {
        let name = page.profile_name.as_deref().unwrap_or("?");
        let token = match decrypt_token(&page.access_token) {
            Some(t) => t,
            None => {
                log_result(
                    &format!("lip_get_page_posts[{name}]"),
                    false,
                    "token decryption failed",
                );
                continue;
            }
        };

        // Try with the page's internal_id as the page_id
        match provider.get_page_posts(&token, &page.internal_id, 5).await {
            Ok(data) => {
                let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
                log_result(
                    &format!("lip_get_page_posts[{name}]"),
                    true,
                    &format!("{arr} posts"),
                );
            }
            Err(e) => log_result(
                &format!("lip_get_page_posts[{name}]"),
                false,
                &format!("{e}"),
            ),
        }
    }
}

#[tokio::test]
async fn test_lip_pages() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let page = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin-page");
    let Some(page) = page else {
        log_skip("lip_pages", "no linkedin-page integration");
        return;
    };
    let token = match decrypt_token(&page.access_token) {
        Some(t) => t,
        None => {
            log_result("lip_pages", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInPageProvider::new(get_config());

    match provider.pages(&token).await {
        Ok(data) => {
            let preview = serde_json::to_string(&data).unwrap_or_default();
            log_result(
                "lip_pages",
                true,
                &format!("{} pages: {:.120}", data.len(), preview),
            );
        }
        Err(e) => log_result("lip_pages", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_lip_fetch_page_info() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let pages: Vec<_> = integrations
        .iter()
        .filter(|i| i.provider_identifier == "linkedin-page")
        .collect();
    if pages.is_empty() {
        log_skip("lip_fetch_page_info", "no linkedin-page integrations");
        return;
    }
    let provider = LinkedInPageProvider::new(get_config());

    for page in pages {
        let name = page.profile_name.as_deref().unwrap_or("?");
        let token = match decrypt_token(&page.access_token) {
            Some(t) => t,
            None => {
                log_result(
                    &format!("lip_fetch_page_info[{name}]"),
                    false,
                    "token decryption failed",
                );
                continue;
            }
        };

        match provider.fetch_page_info(&token, &page.internal_id).await {
            Ok(data) => log_result(
                &format!("lip_fetch_page_info[{name}]"),
                true,
                &format!("{:?}", data),
            ),
            Err(e) => log_result(
                &format!("lip_fetch_page_info[{name}]"),
                false,
                &format!("{e}"),
            ),
        }
    }
}

#[tokio::test]
async fn test_lip_analytics() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let page = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin-page");
    let Some(page) = page else {
        log_skip("lip_analytics", "no linkedin-page integration");
        return;
    };
    let token = match decrypt_token(&page.access_token) {
        Some(t) => t,
        None => {
            log_result("lip_analytics", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInPageProvider::new(get_config());

    match provider.analytics(&token, &page.internal_id, 7).await {
        Ok(data) => log_result("lip_analytics", true, &format!("{} entries", data.len())),
        Err(e) => log_result("lip_analytics", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_lip_post_analytics() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let page = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin-page");
    let Some(page) = page else {
        log_skip("lip_post_analytics", "no linkedin-page integration");
        return;
    };
    let token = match decrypt_token(&page.access_token) {
        Some(t) => t,
        None => {
            log_result("lip_post_analytics", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInPageProvider::new(get_config());

    // First try getting posts to find a post ID for analytics
    let post_id = match provider.get_page_posts(&token, &page.internal_id, 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_object())
            .and_then(|o| {
                o.get("id")
                    .or_else(|| o.get("post_id"))
                    .or_else(|| o.get("activity"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let post_id = match post_id {
        Some(p) => p,
        None => {
            log_skip("lip_post_analytics", "no page posts found");
            return;
        }
    };

    match provider.post_analytics(&token, &post_id).await {
        Ok(data) => log_result(
            "lip_post_analytics",
            true,
            &format!("{} entries", data.len()),
        ),
        Err(e) => log_result("lip_post_analytics", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_lip_create_comment() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let page = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin-page");
    let Some(page) = page else {
        log_skip("lip_create_comment", "no linkedin-page integration");
        return;
    };
    let name = page.profile_name.as_deref().unwrap_or("?");
    let token = match decrypt_token(&page.access_token) {
        Some(t) => t,
        None => {
            log_result("lip_create_comment", false, "token decryption failed");
            return;
        }
    };
    let provider = LinkedInPageProvider::new(get_config());

    // Try to find a post to comment on
    let post_urn = match provider.get_page_posts(&token, &page.internal_id, 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_object())
            .and_then(|o| {
                o.get("id")
                    .or_else(|| o.get("urn"))
                    .or_else(|| o.get("activity"))
                    .or_else(|| o.get("post_urn"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let (post_urn, org_urn) = match post_urn {
        Some(p) => (p, page.internal_id.clone()),
        None => {
            log_skip(
                &format!("lip_create_comment[{name}]"),
                "no posts found to comment on",
            );
            return;
        }
    };

    match provider
        .create_comment(
            &token,
            &post_urn,
            &org_urn,
            "Test page comment from integration test — please ignore",
        )
        .await
    {
        Ok(data) => log_result(
            &format!("lip_create_comment[{name}]"),
            true,
            &format!("{:.120}", data),
        ),
        Err(e) => log_result(
            &format!("lip_create_comment[{name}]"),
            false,
            &format!("{e}"),
        ),
    }
}

// ═════════════════════════════════════════════════════════════════
// YOUTUBE
// ═════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_yt_search_videos() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_search_videos", "no youtube integration in DB");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_search_videos", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider.search_videos(&token, "rust programming", 5).await {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            log_result("yt_search_videos", true, &format!("{arr} results"));
        }
        Err(e) => log_result("yt_search_videos", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_video() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_video", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_video", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    // Search for a video to get a real video ID
    let video_id = match provider.search_videos(&token, "rust programming", 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_object())
            .and_then(|o| o.get("id").or_else(|| o.get("video_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let video_id = match video_id {
        Some(v) => v,
        None => {
            log_skip("yt_get_video", "no videos found via search");
            return;
        }
    };

    match provider.get_video(&token, &video_id).await {
        Ok(data) => log_result("yt_get_video", true, &format!("{:.120}", data)),
        Err(e) => log_result("yt_get_video", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_playlists() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_playlists", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_playlists", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    // Use the internal_id as channel_id
    match provider.get_playlists(&token, &yt.internal_id, 5).await {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            let preview = serde_json::to_string(&data).unwrap_or_default();
            log_result(
                "yt_get_playlists",
                true,
                &format!("{arr} playlists: {:.100}", preview),
            );
        }
        Err(e) => log_result("yt_get_playlists", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_playlist_items() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_playlist_items", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_playlist_items", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    // Get playlists first to find a real playlist ID
    let playlist_id = match provider.get_playlists(&token, &yt.internal_id, 5).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_object())
            .and_then(|o| o.get("id").or_else(|| o.get("playlist_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let playlist_id = match playlist_id {
        Some(p) => p,
        None => {
            log_skip("yt_get_playlist_items", "no playlists found");
            return;
        }
    };

    match provider.get_playlist_items(&token, &playlist_id, 5).await {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            log_result("yt_get_playlist_items", true, &format!("{arr} items"));
        }
        Err(e) => log_result("yt_get_playlist_items", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_comments() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_comments", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_comments", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    // Search  to find a real video ID to get comments from
    let video_id = match provider.search_videos(&token, "rust programming", 1).await {
        Ok(data) => data
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_object())
            .and_then(|o| o.get("id").or_else(|| o.get("video_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };

    let video_id = match video_id {
        Some(v) => v,
        None => {
            log_skip("yt_get_comments", "no videos found");
            return;
        }
    };

    match provider.get_comments(&token, &video_id, 5).await {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            log_result("yt_get_comments", true, &format!("{arr} comments"));
        }
        Err(e) => log_result("yt_get_comments", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_channel_stats() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_channel_stats", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_channel_stats", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider.get_channel_stats(&token, &yt.internal_id).await {
        Ok(data) => log_result("yt_get_channel_stats", true, &format!("{:.120}", data)),
        Err(e) => log_result("yt_get_channel_stats", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_analytics() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_analytics", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_analytics", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    // YouTube Analytics API requires specific scopes — may fail if scopes missing
    match provider
        .get_analytics(
            &token,
            &yt.internal_id,
            "views,estimatedMinutesWatched",
            "2025-01-01",
            "2025-12-31",
        )
        .await
    {
        Ok(data) => log_result("yt_get_analytics", true, &format!("{:.120}", data)),
        Err(e) => log_result("yt_get_analytics", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_get_subscriptions() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_get_subscriptions", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_get_subscriptions", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider
        .get_subscriptions(&token, &yt.internal_id, 10)
        .await
    {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            log_result(
                "yt_get_subscriptions",
                true,
                &format!("{arr} subscriptions"),
            );
        }
        Err(e) => log_result("yt_get_subscriptions", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_find_creators() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_find_creators", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_find_creators", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider
        .find_creators(&token, "tech", Some(1000), Some(5))
        .await
    {
        Ok(data) => {
            let arr = data.as_array().map(|a| a.len()).unwrap_or(0);
            log_result("yt_find_creators", true, &format!("{arr} creators"));
        }
        Err(e) => log_result("yt_find_creators", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_pages() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_pages", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_pages", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider.pages(&token).await {
        Ok(data) => {
            log_result("yt_pages", true, &format!("{} channels", data.len()));
        }
        Err(e) => log_result("yt_pages", false, &format!("{e}")),
    }
}

#[tokio::test]
async fn test_yt_fetch_page_info() {
    let Some((integrations, _uid)) = get_dev_user_integrations().await else {
        log_skip("suite", &skip_reason(true, &live_test_email()));
        return;
    };
    let yt = integrations
        .iter()
        .find(|i| i.provider_identifier == "youtube");
    let Some(yt) = yt else {
        log_skip("yt_fetch_page_info", "no youtube integration");
        return;
    };
    let token = match decrypt_token(&yt.access_token) {
        Some(t) => t,
        None => {
            log_result("yt_fetch_page_info", false, "token decryption failed");
            return;
        }
    };
    let provider = YoutubeProvider::new(get_config());

    match provider.fetch_page_info(&token, &yt.internal_id).await {
        Ok(data) => log_result("yt_fetch_page_info", true, &format!("{:?}", data)),
        Err(e) => log_result("yt_fetch_page_info", false, &format!("{e}")),
    }
}
