use switchpost::config::Config;
use switchpost::crypto;
use switchpost::db;
use switchpost::db::queries;
use switchpost::social::calendar::CalendarProvider;
use switchpost::social::drive::DriveProvider;
use switchpost::social::github::GithubProvider;
use switchpost::social::gmail::GmailProvider;
use switchpost::social::youtube::YoutubeProvider;
use switchpost::social::SocialProvider;

mod common;
use common::skip_on_environment_failure;

fn get_config() -> Config {
    switchpost::config::load_dotenv_for_live_tests();
    Config::from_env().expect("Failed to load config")
}

fn decrypt_token(raw: &str) -> Option<String> {
    if !raw.is_empty() && !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(raw.to_string());
    }
    let cfg = get_config();
    let key = crypto::decode_hex_key(cfg.token_encryption_key.as_ref()?).ok()?;
    crypto::decrypt_string(raw, &key).ok()
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

/// Why the Google suite cannot run, in the founder's terms.
///
/// The three causes are genuinely different and the operator can act on
/// each one, so they are reported separately rather than collapsed into
/// "Google token not found" - a message that blames a credential when the
/// real cause is a missing ACCOUNT or a decode failure.
fn google_skip_reason(account: &str, user_found: bool, has_google: bool) -> String {
    if !user_found {
        format!(
            "no account {account} in this database - set LIVE_TEST_EMAIL to the \
             account that carries the Google integration (the default is a \
             fixture address no real installation uses)"
        )
    } else if !has_google {
        format!("the account {account} is in the database but has no google integration - connect one first")
    } else {
        "the google integration has no readable access token - reconnect the channel".to_string()
    }
}

/// The Google access token, or the reason there isn't one.
async fn get_google_token() -> Result<String, String> {
    let cfg = get_config();
    let pool = db::create_pool(&cfg.database_url)
        .await
        .map_err(|e| format!("could not open the database: {e}"))?;
    let email = live_test_email();
    // The shared helper already declares `id as "id!: Uuid"`. The bare
    // `SELECT id` this used to run inferred TEXT from the column's DECLARED
    // type and failed at RUNTIME with "mismatched types ... not compatible
    // with SQL type BLOB" (AGENTS.md 5.1.1 / 9.44). The `.ok()??` chain
    // swallowed that error and reported "Google token not found" - naming a
    // missing credential when the real cause was a decode failure.
    let user = queries::get_user_by_email(&pool, &email)
        .await
        .map_err(|e| format!("could not look up {email}: {e}"))?
        .ok_or_else(|| google_skip_reason(&email, false, false))?;
    let row = sqlx::query!(
        "SELECT access_token FROM integrations WHERE user_id = ? AND provider_identifier = ? LIMIT 1",
        user.id, "google"
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| format!("could not read the google integration: {e}"))?
    .ok_or_else(|| google_skip_reason(&email, true, false))?;
    decrypt_token(&row.access_token)
        .ok_or_else(|| google_skip_reason(&email, true, true))
}

const GH_OWNER: &str = "ishan-parihar";
const GH_REPO: &str = "switchpost";

#[tokio::test]
async fn test_gh_user() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    let Some(r) = skip_on_environment_failure("GitHub get_user", gh.get_user(t, GH_OWNER).await)
    else {
        return;
    };
    assert_eq!(r["login"], GH_OWNER);
    println!("OK gh_user: {GH_OWNER}");
}

#[tokio::test]
async fn test_gh_auth_user() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    let Some(r) = skip_on_environment_failure(
        "GitHub get_authenticated_user",
        gh.get_authenticated_user(t).await,
    ) else {
        return;
    };
    println!("OK gh_auth: {}", r["login"].as_str().unwrap_or("?"));
}

#[tokio::test]
async fn test_gh_repos() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    let Some(r) =
        skip_on_environment_failure("GitHub list_repos", gh.list_repos(t, GH_OWNER, 50).await)
    else {
        return;
    };
    println!(
        "OK gh_repos: {}",
        r.as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_gh_issues() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_issues(t, GH_OWNER, GH_REPO, "all", 50).await {
        Ok(r) => println!(
            "OK gh_issues: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- gh_issues: {e}"),
    }
}

#[tokio::test]
async fn test_gh_prs() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_pull_requests(t, GH_OWNER, GH_REPO, "all", 50).await {
        Ok(r) => println!("OK gh_prs: {}", r.as_array().map(|a| a.len()).unwrap_or(0)),
        Err(e) => println!("-- gh_prs: {e}"),
    }
}

#[tokio::test]
async fn test_gh_commits() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_commits(t, GH_OWNER, GH_REPO, 50).await {
        Ok(r) => println!(
            "OK gh_commits: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- gh_commits: {e}"),
    }
}

#[tokio::test]
async fn test_gh_branches() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_branches(t, GH_OWNER, GH_REPO, 50).await {
        Ok(r) => println!(
            "OK gh_branches: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- gh_branches: {e}"),
    }
}

#[tokio::test]
async fn test_gh_releases() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_releases(t, GH_OWNER, GH_REPO, 50).await {
        Ok(r) => println!(
            "OK gh_releases: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- gh_releases: {e}"),
    }
}

#[tokio::test]
async fn test_gh_search_repos() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    let r = gh.search_repos(t, "rust", 10).await.unwrap();
    println!(
        "OK gh_search: {}",
        r["items"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_gh_search_code() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    let r = gh.search_code(t, "fn main", 50).await.unwrap();
    println!(
        "OK gh_code: {}",
        r["items"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_gh_contributors() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.list_contributors(t, GH_OWNER, GH_REPO, 50).await {
        Ok(r) => println!(
            "OK gh_contributors: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- gh_contributors: {e}"),
    }
}

#[tokio::test]
async fn test_gh_content() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.get_repo_content(t, GH_OWNER, GH_REPO, "README.md").await {
        Ok(r) => println!("OK gh_content: {} bytes", r["size"].as_u64().unwrap_or(0)),
        Err(e) => println!("-- gh_content: {e}"),
    }
}

#[tokio::test]
async fn test_gh_repo() {
    let c = get_config();
    let gh = GithubProvider::new(&c);
    let t = c.github_token.as_deref().unwrap_or("");
    match gh.get_repo(t, GH_OWNER, GH_REPO).await {
        Ok(r) => println!(
            "OK gh_repo: {} {} stars",
            r["full_name"].as_str().unwrap_or("?"),
            r["stargazers_count"]
        ),
        Err(e) => println!("-- gh_repo: {e}"),
    }
}

// Google Suite — None when Google is not connected, so tests skip instead of
// panicking on a box without live credentials. F2: each API is exercised
// through its canonical provider module (youtube/gmail/calendar/drive) —
// the same types tools_google dispatches to.
async fn google_token() -> Result<String, String> {
    get_google_token().await
}

#[tokio::test]
async fn test_goog_search() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let r = p.search_videos(&t, "Rust", 3).await.unwrap();
    println!(
        "OK yt_search: {}",
        r["items"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_channel() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let pages: Vec<switchpost::social::PageInfo> = SocialProvider::pages(&p, &t).await.unwrap();
    if let Some(page) = pages.first() {
        let r = p.get_channel_stats(&t, &page.id).await.unwrap();
        let s = &r["items"][0]["statistics"];
        println!(
            "OK channel: {} subs {}",
            s["subscriberCount"], s["videoCount"]
        );
    }
}

#[tokio::test]
async fn test_goog_video() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let s = p.search_videos(&t, "Rust", 1).await.unwrap();
    if let Some(v) = s["items"].as_array().and_then(|a| a.first()) {
        let vid = v["id"]["videoId"].as_str().unwrap_or("");
        let r = p.get_video(&t, vid).await.unwrap();
        println!(
            "OK video: {}",
            r["items"][0]["snippet"]["title"].as_str().unwrap_or("?")
        );
    }
}

#[tokio::test]
async fn test_goog_playlists() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let pages: Vec<switchpost::social::PageInfo> = SocialProvider::pages(&p, &t).await.unwrap();
    if let Some(page) = pages.first() {
        let r = p.get_playlists(&t, &page.id, 5).await.unwrap();
        println!(
            "OK playlists: {}",
            r["items"].as_array().map(|a| a.len()).unwrap_or(0)
        );
    }
}

#[tokio::test]
async fn test_goog_comments() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let s = p.search_videos(&t, "Rust", 1).await.unwrap();
    if let Some(v) = s["items"].as_array().and_then(|a| a.first()) {
        let vid = v["id"]["videoId"].as_str().unwrap_or("");
        match p.get_comments(&t, vid, 3).await {
            Ok(r) => println!(
                "OK comments: {}",
                r["items"].as_array().map(|a| a.len()).unwrap_or(0)
            ),
            Err(e) => println!("-- comments: {e}"),
        }
    }
}

#[tokio::test]
async fn test_goog_find() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    match p.find_creators(&t, "architecture", None, Some(3)).await {
        Ok(r) => println!(
            "OK creators: {}",
            r.as_array().map(|a| a.len()).unwrap_or(0)
        ),
        Err(e) => println!("-- find: {e}"),
    }
}

#[tokio::test]
async fn test_goog_analytics() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = YoutubeProvider::new(&get_config());
    let pages: Vec<switchpost::social::PageInfo> = SocialProvider::pages(&p, &t).await.unwrap();
    if let Some(page) = pages.first() {
        match p
            .get_analytics(
                &t,
                &page.id,
                "views,estimatedMinutesWatched",
                "2025-01-01",
                "2025-05-15",
            )
            .await
        {
            Ok(r) => println!(
                "OK analytics: {} rows",
                r["rows"].as_array().map(|a| a.len()).unwrap_or(0)
            ),
            Err(e) => println!("-- analytics: {e}"),
        }
    }
}

#[tokio::test]
async fn test_goog_gmail() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = GmailProvider::new(&get_config());
    let r = p.get_profile(&t).await.unwrap();
    println!(
        "OK gmail: {} {} msgs",
        r["emailAddress"], r["messagesTotal"]
    );
}

#[tokio::test]
async fn test_goog_messages() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = GmailProvider::new(&get_config());
    let r = p.list_messages(&t, 5, None).await.unwrap();
    println!(
        "OK msgs: {}",
        r["messages"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_labels() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = GmailProvider::new(&get_config());
    let r = p.list_labels(&t).await.unwrap();
    println!(
        "OK labels: {}",
        r["labels"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_cal_list() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = CalendarProvider::new(&get_config());
    let r = p.list_calendars(&t).await.unwrap();
    println!(
        "OK calendars: {}",
        r["items"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_events() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = CalendarProvider::new(&get_config());
    let r = p.list_events(&t, "primary", 5, None, None).await.unwrap();
    println!(
        "OK events: {}",
        r["items"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_cal_crud() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = CalendarProvider::new(&get_config());
    let created = p
        .create_event(
            &t,
            "primary",
            "Test",
            "2026-05-16T10:00:00",
            "2026-05-16T11:00:00",
            Some("test"),
        )
        .await
        .unwrap();
    let eid = created["id"].as_str().unwrap_or("").to_string();
    let _ = p
        .update_event(&t, "primary", &eid, Some("Updated"), Some("desc"))
        .await;
    p.delete_event(&t, "primary", &eid).await.unwrap();
    println!("OK cal_crud");
}

#[tokio::test]
async fn test_goog_drive_files() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = DriveProvider::new(&get_config());
    let r = p.list_files(&t, 10, None).await.unwrap();
    println!(
        "OK drive: {} files",
        r["files"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_drive_folders() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = DriveProvider::new(&get_config());
    let r = p.list_folders(&t, 10).await.unwrap();
    println!(
        "OK folders: {}",
        r["files"].as_array().map(|a| a.len()).unwrap_or(0)
    );
}

#[tokio::test]
async fn test_goog_drive_meta() {
    let t = match google_token().await {
        Ok(t) => t,
        Err(reason) => {
            println!("⏭ skipped: {reason}");
            return;
        }
    };
    let p = DriveProvider::new(&get_config());
    let files = p.list_files(&t, 1, None).await.unwrap();
    if let Some(f) = files["files"].as_array().and_then(|a| a.first()) {
        let m = p
            .get_file_metadata(&t, f["id"].as_str().unwrap_or(""))
            .await
            .unwrap();
        println!("OK meta: {} ({})", m["name"], m["mimeType"]);
    }
}
