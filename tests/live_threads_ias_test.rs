// ─── Live Integration Tests: Threads + Instagram Standalone ──
// Run: cargo test --test live_threads_ias_test -- --nocapture
// Requires: SQLite file, .env with INSTAGRAM_APP_ID,
//   THREADS_APP_ID, and real OAuth tokens in the DB for dev@switchpost.dev.

use switchpost::config::Config;
use switchpost::db;
use switchpost::db::queries;
use switchpost::social::instagram_standalone::InstagramStandaloneProvider;
use switchpost::social::threads::ThreadsProvider;
use switchpost::social::SocialProvider;

// ── Threads Tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_threads_get_profile() {
    let (provider, token) = match get_threads_creds().await {
        Ok(v) => v,
        Err(reason) => {
            println!("⏭ threads_get_profile skipped: {reason}");
            return;
        }
    };
    let result = provider.get_profile(&token).await;
    println!("Threads get_profile: {:?}", result);
    assert!(result.is_ok());
    let json = result.unwrap();
    assert!(json["id"].as_str().is_some());
    println!(
        "Threads user: id={:?}, username={:?}",
        json["id"], json["username"]
    );
}

#[tokio::test]
async fn test_threads_social_provider_meta() {
    let (provider, _) = match get_threads_creds().await {
        Ok(v) => v,
        Err(reason) => {
            println!("⏭ threads_social_provider_meta skipped: {reason}");
            return;
        }
    };
    assert_eq!(provider.identifier(), "threads");
    assert_eq!(provider.name(), "Threads");
    assert!(provider.uses_oauth());
    let scopes = provider.scopes();
    assert!(scopes.contains(&"threads_basic".to_string()));
    println!("Threads scopes: {:?}", scopes);
}

#[tokio::test]
async fn test_ias_get_media() {
    let (provider, token, internal_id) = match get_ias_creds().await {
        Ok(v) => v,
        Err(reason) => {
            println!("⏭ ias_get_media skipped: {reason}");
            return;
        }
    };
    let result = provider.get_media(&token, &internal_id, 5).await;
    println!("IAS get_media: {:?}", result);
    if let Ok(json) = &result {
        println!(
            "IAS media count: {:?}",
            json["data"].as_array().map(|a| a.len())
        );
    }
}

#[tokio::test]
async fn test_ias_social_provider_meta() {
    let (provider, _, _) = match get_ias_creds().await {
        Ok(v) => v,
        Err(reason) => {
            println!("⏭ ias_social_provider_meta skipped: {reason}");
            return;
        }
    };
    assert_eq!(provider.identifier(), "instagram-standalone");
    assert_eq!(provider.name(), "Instagram (Standalone)");
    assert!(provider.max_content_length() > 0);
    assert!(provider.uses_oauth());
    let scopes = provider.scopes();
    assert!(scopes.contains(&"instagram_business_content_publish".to_string()));
    println!("IAS scopes: {:?}", scopes);
}

// ── Helpers ──────────────────────────────────────────────────

fn get_config() -> Config {
    switchpost::config::load_dotenv_for_live_tests();
    Config::from_env().expect("Failed to load config")
}

fn get_threads_provider(config: &Config) -> ThreadsProvider {
    ThreadsProvider::new(config)
}

/// The account this suite looks up when LIVE_TEST_EMAIL is not set.
///
/// It is a FIXTURE address, deliberately: a test that defaulted to the
/// operator's real account would run against a real account by accident.
const LIVE_TEST_EMAIL_DEFAULT: &str = "dev@switchpost.dev";

/// The account this run looks up: the operator's override, or the fixture.
fn live_test_email() -> String {
    std::env::var("LIVE_TEST_EMAIL").unwrap_or_else(|_| LIVE_TEST_EMAIL_DEFAULT.to_string())
}

fn get_ias_provider(config: &Config) -> InstagramStandaloneProvider {
    InstagramStandaloneProvider::new(config)
}

/// Why the suite cannot run, in the founder's terms.
///
/// The account being absent and the integration being absent are
/// DIFFERENT problems with different fixes, and the old message
/// ("no threads integration in DB") named the second while the common
/// cause was the first - the fixture account belongs to nobody.
fn skip_reason(email: &str, user_found: bool, provider_identifier: &str) -> String {
    if !user_found {
        format!(
            "no account {email} in this database - set LIVE_TEST_EMAIL to the \
             account that carries the {provider_identifier} channel (the default \
             is a fixture address no real installation uses)"
        )
    } else {
        format!(
            "the account {email} is in the database but has no \
             {provider_identifier} integration - connect one first"
        )
    }
}

async fn get_token(
    config: &Config,
    pool: &sqlx::SqlitePool,
    provider_identifier: &str,
) -> Result<(String, String), String> {
    let email = live_test_email();
    let user = queries::get_user_by_email(pool, &email)
        .await
        .expect("DB query")
        .ok_or_else(|| skip_reason(&email, false, provider_identifier))?;

    let integrations = queries::list_integrations(pool, user.id)
        .await
        .expect("list integrations");

    let integration = integrations
        .iter()
        .find(|i| i.provider_identifier == provider_identifier)
        .ok_or_else(|| skip_reason(&email, true, provider_identifier))?;

    let token = config
        .token_encryption_key
        .as_ref()
        .and_then(|k| {
            let key = switchpost::crypto::decode_hex_key(k).ok()?;
            switchpost::crypto::decrypt_string(&integration.access_token, &key).ok()
        })
        .unwrap_or_else(|| integration.access_token.clone());

    Ok((token, integration.internal_id.clone()))
}

async fn get_threads_creds() -> Result<(ThreadsProvider, String), String> {
    let config = get_config();
    let provider = get_threads_provider(&config);
    let pool = db::create_pool(&config.database_url)
        .await
        .expect("DB pool");
    let (token, _) = get_token(&config, &pool, "threads").await?;
    Ok((provider, token))
}

async fn get_ias_creds() -> Result<(InstagramStandaloneProvider, String, String), String> {
    let config = get_config();
    let provider = get_ias_provider(&config);
    let pool = db::create_pool(&config.database_url)
        .await
        .expect("DB pool");
    let (token, internal_id) = get_token(&config, &pool, "instagram-standalone").await?;
    Ok((provider, token, internal_id))
}
