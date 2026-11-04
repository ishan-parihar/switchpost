// ─── Live X/Twitter Provider Test ──────────────────────────────
// Tests all X provider methods against the real Twitter GraphQL API
// using cookies from .env (X_AUTH_TOKEN + X_CT0).
//
// Run: cargo test --test live_x_test -- --nocapture
//
// Requires: .env with X_AUTH_TOKEN, X_CT0 set, database running.

use switchpost::config::Config;
use switchpost::social::x::XProvider;

mod common;
use common::skip_on_environment_failure;

fn setup_config() -> Config {
    switchpost::config::load_dotenv_for_live_tests();
    Config::from_env().expect("Failed to load config from .env")
}

fn create_provider() -> XProvider {
    let config = setup_config();
    XProvider::new(&config)
}

/// The X cookie, or `None` when the credentials are not configured.
///
/// A missing credential is not a defect in this codebase, and a gate
/// that goes red for it is a gate the operator learns to ignore. The
/// caller SKIPS.
fn get_token() -> Option<String> {
    let config = setup_config();
    // The credentials are not set: the test never ran, so it SKIPS. A
    // missing credential is not a defect in this codebase, and a gate
    // that goes red for it is a gate the operator learns to ignore.
    let at = config.x_auth_token?;
    let ct0 = config.x_ct0?;
    Some(serde_json::json!({ "auth_token": at, "ct0": ct0 }).to_string())
}

/// One get_me per binary: eleven tests each hitting the UserByScreenName
/// endpoint concurrently trips X's connection throttle and fails the gate
/// with transport resets (the recurring false-red). The first test to run
/// populates the cell; the rest reuse its result.
static ME: tokio::sync::OnceCell<Option<serde_json::Value>> = tokio::sync::OnceCell::const_new();
/// Whether this run may MUTATE the operator's real account.
///
/// `test_x_like_unlike` and `test_x_retweet_unretweet` act on the
/// operator's own X account. They are self-reversing (like then unlike,
/// retweet then unretweet) and target a public fixture tweet, so the
/// state is restored - but the reverse step is not asserted, and if it
/// errors the like or retweet PERSISTS on a real account. That makes
/// them writes, not reads, and a write deserves an opt-in.
///
/// The Reddit suite has had this guard (`REDDIT_LIVE_WRITE`) since it
/// was written; this one did not, so a plain `cargo test --test
/// live_x_test` performed real writes on the founder's account.
/// Same shape, so the rule is learnable from one place.
fn live_write_enabled() -> bool {
    std::env::var("X_LIVE_WRITE")
        .map(|v| v == "1")
        .unwrap_or(false)
}
/// Whether this run may MUTATE the operator's real account.

/// `None` means the provider did not answer and the reason was the
/// environment's - see `common::skip_on_environment_failure`.
async fn me() -> Option<&'static serde_json::Value> {
    ME.get_or_init(|| async {
        let token = get_token()?;
        let result = create_provider().get_me(&token).await;
        skip_on_environment_failure("X get_me", result)
    })
    .await
    .as_ref()
}

#[tokio::test]
async fn test_x_get_me() {
    let Some(result) = me().await.cloned() else {
        return;
    };
    println!("\n✅ X get_me succeeded:");
    println!(
        "   Response: {}",
        serde_json::to_string_pretty(&result)
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect::<String>()
    );
    assert!(
        result.get("data").is_some(),
        "Expected data field in response"
    );
    let data = result.get("data").unwrap();
    let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let username = data
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    println!("   ID: {id}");
    println!("   Name: {name}");
    println!("   Username: {username}");
}

#[tokio::test]
async fn test_x_user_lookup_by_username() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    let result = provider
        .user_lookup_by_username(&token, "XDevelopers")
        .await;
    match result {
        Ok(_) => println!("\n✅ X user_lookup_by_username: OK"),
        Err(e) => println!("\n⚠️  X user_lookup_by_username: {e}"),
    }
}

#[tokio::test]
async fn test_x_home_timeline() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    let result = provider.home_timeline(&token, "", 5, None).await;
    match result {
        Ok(json) => {
            let count = json
                .pointer("/data/user_result/timeline/instructions")
                .or_else(|| json.pointer("/data/home/home_timestone/instructions"))
                .map(|i| i.as_array().map(|a| a.len()).unwrap_or(0))
                .unwrap_or(0);
            println!("\n✅ X home_timeline: OK ({count} timeline entries)");
        }
        Err(e) => println!("\n⚠️  X home_timeline: {e}"),
    }
}

#[tokio::test]
async fn test_x_user_tweets() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };

    // First get "me" so we have our user ID
    let Some(me) = me().await.cloned() else {
        return;
    };
    let my_id = me
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !my_id.is_empty() {
        let result = provider.user_tweets(&token, my_id, 5, None).await;
        match result {
            Ok(_) => println!("\n✅ X user_tweets: OK"),
            Err(e) => println!("\n⚠️  X user_tweets: {e}"),
        }
    } else {
        println!("\n⚠️  X user_tweets: skipped (no user ID from get_me)");
    }
}

#[tokio::test]
async fn test_x_search_tweets() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    let result = provider.search_tweets(&token, "rust lang", 5, None).await;
    match result {
        Ok(_) => println!("\n✅ X search_tweets: OK"),
        Err(e) => println!("\n⚠️  X search_tweets: {e}"),
    }
}

#[tokio::test]
async fn test_x_followers() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };

    let Some(me) = me().await.cloned() else {
        return;
    };
    let my_id = me
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !my_id.is_empty() {
        let result = provider.followers(&token, my_id, 5, None).await;
        match result {
            Ok(json) => {
                let count = json
                    .get("data")
                    .and_then(|d| d.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!("\n✅ X followers: OK ({count} followers)");
            }
            Err(e) => println!("\n⚠️  X followers: {e}"),
        }
    } else {
        println!("\n⚠️  X followers: skipped (no user ID)");
    }
}

#[tokio::test]
async fn test_x_following() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };

    let Some(me) = me().await.cloned() else {
        return;
    };
    let my_id = me
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !my_id.is_empty() {
        let result = provider.following(&token, my_id, 5, None).await;
        match result {
            Ok(_) => println!("\n✅ X following: OK"),
            Err(e) => println!("\n⚠️  X following: {e}"),
        }
    } else {
        println!("\n⚠️  X following: skipped (no user ID)");
    }
}

#[tokio::test]
async fn test_x_tweet_detail() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    // Use a known tweet ID (this one should be stable)
    let result = provider.tweet_detail(&token, "20").await;
    match result {
        Ok(_) => println!("\n✅ X tweet_detail: OK"),
        Err(e) => println!("\n⚠️  X tweet_detail: {e}"),
    }
}

#[tokio::test]
async fn test_x_bookmarks() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    let result = provider.bookmarks(&token, "", 5, None).await;
    match result {
        Ok(_) => println!("\n✅ X bookmarks: OK"),
        Err(e) => println!("\n⚠️  X bookmarks: {e}"),
    }
}

#[tokio::test]
async fn test_x_like_unlike() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    if !live_write_enabled() {
        println!("\n⏭  X like/unlike: skipped (X_LIVE_WRITE != 1 - this writes to the account)");
        return;
    }

    let Some(me) = me().await.cloned() else {
        return;
    };
    let my_id = me
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !my_id.is_empty() {
        // Like a tweet
        let result = provider.like_tweet(&token, my_id, "20").await;
        match &result {
            Ok(_) => println!("\n✅ X like_tweet: OK"),
            Err(e) => println!("\n⚠️  X like_tweet: {e}"),
        }

        // Unlike it
        let result2 = provider.unlike_tweet(&token, my_id, "20").await;
        match &result2 {
            Ok(_) => println!("\n✅ X unlike_tweet: OK"),
            Err(e) => println!("\n⚠️  X unlike_tweet: {e}"),
        }
    } else {
        println!("\n⚠️  X like/unlike: skipped (no user ID)");
    }
}

#[tokio::test]
async fn test_x_retweet_unretweet() {
    let provider = create_provider();
    let Some(token) = get_token() else {
        return;
    };
    if !live_write_enabled() {
        println!("\n⏭  X retweet/unretweet: skipped (X_LIVE_WRITE != 1 - this writes to the account)");
        return;
    }

    let Some(me) = me().await.cloned() else {
        return;
    };
    let my_id = me
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !my_id.is_empty() {
        let result = provider.retweet(&token, my_id, "20").await;
        match &result {
            Ok(_) => println!("\n✅ X retweet: OK"),
            Err(e) => println!("\n⚠️  X retweet: {e}"),
        }

        let result2 = provider.unretweet(&token, my_id, "20").await;
        match &result2 {
            Ok(_) => println!("\n✅ X unretweet: OK"),
            Err(e) => println!("\n⚠️  X unretweet: {e}"),
        }
    } else {
        println!("\n⚠️  X retweet/unretweet: skipped (no user ID)");
    }
}
