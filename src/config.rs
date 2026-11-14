// ─── Configuration ─────────────────────────────────────────────
// Loads from environment variables (12-factor app style).
// Priority: env vars > .env (cwd) > ~/.switchpost/.env (user config)

use serde::Deserialize;

/// Returns the user config directory: ~/.switchpost/
pub fn config_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".switchpost")
}

/// Load .env files in priority order (later values don't override earlier ones)
pub fn load_dotenv() {
    // 1. CWD .env (highest priority for local dev)
    dotenvy::dotenv().ok();
    // 2. ~/.switchpost/.env (user-level config)
    let user_env = config_dir().join(".env");
    if user_env.exists() {
        dotenvy::from_path(&user_env).ok();
    }
}

/// Fallback when `DATABASE_URL` is unset. Relative to the process CWD, which
/// is the install/serve directory. Must stay in sync with the value written by
/// `switchpost init`, `.env.example`, and the installer.
pub const DEFAULT_DATABASE_URL: &str = "sqlite://data/switchpost.db";

/// Where the WhatsApp Web (wa-rs) session lives when WHATSAPP_STORE_DIR is
/// unset. Relative to the process CWD, which is the install/serve directory.
///
/// The provider already defaulted to this path (`social::whatsapp`), while the
/// config loader left the field `None` — so the startup gate in `main.rs` /
/// `cli::run.rs` refused to build the client at all and the WhatsApp channel
/// was dead on a default install despite the provider having a working
/// default. The default is resolved HERE, at config load, so there is ONE
/// place that decides it and the gate that reads the config sees a real path.
pub const DEFAULT_WHATSAPP_STORE_DIR: &str = "./data/whatsapp";

/// Where the Telegram USER session (Grammers/MTProto) lives when
/// TELEGRAM_SESSION_DIR is unset. Relative to the process CWD.
///
/// The same dead-default shape as the WhatsApp one above, on the same channel:
/// `main.rs` and `cli::run.rs` both fall back to `./data/telegram` when they
/// build the client manager, while the config loader left the field `None` -
/// so the CLI's own connect gate (`connect telegram-user`) demanded a THIRD
/// variable the server had never needed, and answered `not_configured` on an
/// instance that would have worked. Resolved once, here, so every reader agrees.
pub const DEFAULT_TELEGRAM_SESSION_DIR: &str = "./data/telegram";

/// Resolve the Telegram user session dir from an env value.
///
/// Blank means "unset" (dotenvy returns the empty string for a cleared line),
/// and falls through to the default - the same rule as the WhatsApp store dir,
/// because it is the same failure: a config field that is `None` on a default
/// install turns a working channel into "not configured".
fn resolve_telegram_session_dir(provided: Option<String>) -> String {
    provided
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TELEGRAM_SESSION_DIR.into())
}

/// Resolve the WhatsApp store dir from an env value.
///
/// A blank or whitespace-only value means "unset" (dotenvy hands back the
/// empty string for `KEY=`, and an operator who clears the line has not
/// asked for a different folder), so it falls through to the default. That is
/// the behaviour the WhatsApp channel's availability depends on: with no
/// default the startup gate in `main.rs` never builds the wa-rs client and the
/// channel is unreachable on a default install.
fn resolve_whatsapp_store_dir(provided: Option<String>) -> String {
    provided
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_WHATSAPP_STORE_DIR.into())
}

/// SQLite-only guard for `DATABASE_URL`.
///
/// `sqlx`'s `SqliteConnectOptions::from_str` just `trim_start_matches("sqlite:")`
/// and treats *everything else* as a bare filename — so a stale
/// `postgres://user:pass@host:5432/db` is not rejected by sqlx, it becomes a
/// path and fails much later with an opaque filesystem error. Switchpost
/// links no Postgres driver, so reject the URL here, at config load, before
/// anything opens a connection or a socket.
///
/// The offending value is deliberately not echoed back: a database URL
/// normally carries credentials in its userinfo section.
fn ensure_sqlite_url(database_url: &str) -> anyhow::Result<()> {
    let url = database_url.trim();
    // sqlx only understands `sqlite:`; a value with no `://` is read as a bare
    // filename, which keeps working, so it passes through untouched.
    if url.starts_with("sqlite:") || !url.contains("://") {
        return Ok(());
    }
    let scheme = url
        .split("://")
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Err(anyhow::anyhow!(
        "DATABASE_URL uses the `{scheme}` scheme, but Switchpost is SQLite-only \
         (no Postgres driver is linked into this binary).\n\
         Set it to a SQLite file URL, e.g. {DEFAULT_DATABASE_URL}\n\
         Sources are tried in order: environment, then ./.env, then ~/.switchpost/.env.\n\
         Fix it with:  switchpost config set DATABASE_URL {DEFAULT_DATABASE_URL}\n\
         or edit the file directly."
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// SQLite connection string. Defaults to a local file so the binary
    /// runs with no Postgres to provision. Any non-sqlite scheme is rejected
    /// by `ensure_sqlite_url` at load time.
    pub database_url: String,
    /// HMAC secret for signing session cookies AND OAuth state tokens.
    /// If unset, derived from `app_password` at startup (see `Config::from_env`).
    pub jwt_secret: String,
    /// Single password gate for the WebUI. If unset, a random one is
    /// generated and persisted to `~/.switchpost/.env` on first run.
    pub app_password: String,
    pub app_url: String,
    pub frontend_url: String,

    // Social provider credentials
    pub x_client_id: Option<String>,
    pub x_client_secret: Option<String>,
    pub x_auth_token: Option<String>,
    pub x_ct0: Option<String>,
    pub linkedin_client_id: Option<String>,
    pub linkedin_client_secret: Option<String>,
    pub bluesky_handle: Option<String>,
    pub bluesky_app_password: Option<String>,
    pub facebook_client_id: Option<String>,
    pub facebook_client_secret: Option<String>,
    pub instagram_client_id: Option<String>,
    pub instagram_client_secret: Option<String>,
    pub threads_app_id: Option<String>,
    pub threads_app_secret: Option<String>,
    pub youtube_client_id: Option<String>,
    pub youtube_client_secret: Option<String>,
    pub reddit_client_id: Option<String>,
    pub reddit_client_secret: Option<String>,
    pub reddit_username: Option<String>,
    pub reddit_password: Option<String>,
    pub reddit_access_token: Option<String>,
    pub reddit_refresh_token: Option<String>,
    /// REDDIT_COOKIE_TOKEN: the JSON cookie blob
    /// ({reddit_session, token_v2?, cookie_string?}) the manual paste
    /// flow and `connect reddit`'s env fallback accept.
    pub reddit_cookie_token: Option<String>,
    pub discord_client_id: Option<String>,
    pub discord_client_secret: Option<String>,
    pub discord_bot_token: Option<String>,
    pub telegram_bot_tokens: Option<String>,
    pub telegram_api_id: Option<String>,
    pub telegram_api_hash: Option<String>,
    pub telegram_session_dir: Option<String>,
    pub pinterest_client_id: Option<String>,
    pub pinterest_client_secret: Option<String>,
    pub instagram_app_id: Option<String>,
    pub instagram_app_secret: Option<String>,
    pub whatsapp_store_dir: Option<String>,
    // WhatsApp Cloud API (E6) — business-number path alongside the wa-rs
    // Web session. Token is a Meta system-user access token.
    pub whatsapp_cloud_token: Option<String>,
    pub whatsapp_cloud_phone_number_id: Option<String>,
    pub whatsapp_cloud_waba_id: Option<String>,
    /// Webhook subscription verification token (Meta App Dashboard).
    pub whatsapp_cloud_verify_token: Option<String>,
    /// App secret for X-Hub-Signature-256 webhook verification.
    pub whatsapp_cloud_app_secret: Option<String>,

    // Slack
    pub slack_client_id: Option<String>,
    pub slack_client_secret: Option<String>,

    // TikTok
    pub tiktok_client_id: Option<String>,
    pub tiktok_client_secret: Option<String>,

    // Mastodon
    pub mastodon_client_id: Option<String>,
    pub mastodon_client_secret: Option<String>,
    pub mastodon_instance_url: Option<String>,

    // Medium (API key-based)
    pub medium_access_token: Option<String>,

    // Dev.to (API key-based)
    pub devto_api_key: Option<String>,

    // Hashnode (API key-based)
    pub hashnode_api_key: Option<String>,

    // GitHub (PAT-based)
    pub github_token: Option<String>,

    // Neynar (Farcaster) API key — Tier-3 archive (v25 §1), only read
    // when ENABLE_ARCHIVE_PROVIDERS is set.
    pub neynar_api_key: Option<String>,

    // Token encryption at rest
    pub token_encryption_key: Option<String>,

    // Media storage
    pub media_dir: String,

    // AI/LLM endpoint for automation (auto-reply, content generation).
    // If unset, AI automation features are disabled.
    pub llm_endpoint: Option<String>,
    pub llm_model: Option<String>,

    // Short-link configuration (Dub.co API).
    // If DUB_CO_API_KEY is set, URLs in post content are auto-shortened
    // before publishing. STRIP_LINKS_FROM_X removes URLs from X posts
    // entirely (X downranks posts with external links).
    pub dub_co_api_key: Option<String>,
    pub dub_co_workspace: Option<String>,
    pub strip_links_from_x: bool,
}

/// Scratch database for test processes. Under `target/`, so `cargo test` never
/// migrates or writes the operator's `data/switchpost.db`.
///
/// The prefix for the per-binary file. See [`test_database_url`].
pub const TEST_DATABASE_URL: &str = "sqlite://target/test-data/switchpost-test.db";

/// The scratch database for THIS test binary.
///
/// `cargo test` runs every integration test binary as its own PROCESS, and
/// they run in PARALLEL. They used to share one file, and the shared file
/// was the cause of a flaky failure: "database is locked" in
/// `composer_contract_test.rs`, in about one run in three.
///
/// SQLite serialises writers. 45 processes each running a transactional
/// test against one file means one of them is always waiting on another's
/// write lock, and the 15s `busy_timeout` is a long time to hold a suite
/// up. Two processes can also deadlock against each other when one holds
/// a read and the other wants to upgrade it.
///
/// The fix is one file per binary: the binary's own name is in its
/// `current_exe` path, so the file is unique per test target and no two
/// processes ever touch the same database.
///
/// The file is still under `target/`, so it is cleaned with the build and
/// never the operator's data.
pub fn test_database_url() -> String {
    // `target/debug/deps/composer_contract_test-abc123` -> the file stem
    // before the hash, so the name is stable across rebuilds (a hash
    // change would leave a stale file behind, and the name would churn
    // in CI logs).
    let stem = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let stem = stem.split('-').next().unwrap_or("unknown").to_string();
    format!("sqlite://target/test-data/{stem}.db")
}

/// Load `.env` for an integration or live test, then point `DATABASE_URL` at
/// this binary's own scratch file. The repo `.env` names the real database,
/// which the suite otherwise migrated and wrote. Overrides any exported
/// `DATABASE_URL`.
///
/// Each test binary gets its OWN file (see [`test_database_url`]), so the
/// binaries can run in parallel without fighting over SQLite's write lock.
pub fn load_dotenv_for_tests() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        load_dotenv();
        std::env::set_var("DATABASE_URL", test_database_url());
    });
}

/// For live tests that read tokens already stored in the operator's database
/// (see `get_google_token`). They keep the `.env` `DATABASE_URL` on purpose.
pub fn load_dotenv_for_live_tests() {
    load_dotenv();
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let app_url = opt("APP_URL").unwrap_or_else(|| "https://localhost:6543".into());
        let frontend_url = opt("FRONTEND_URL").unwrap_or_else(|| app_url.clone());

        // ── Single-user password gate ────────────────────────────
        // Priority: APP_PASSWORD env var > persisted ~/.switchpost/.env
        // > generate-and-persist a fresh random one (first run).
        let app_password = match opt("APP_PASSWORD") {
            Some(p) if !p.is_empty() => p,
            _ => {
                // Try to load a previously-persisted password.
                let user_env = config_dir().join(".env");
                let persisted = if user_env.exists() {
                    std::fs::read_to_string(&user_env)
                        .ok()
                        .and_then(|s| {
                            s.lines().find_map(|l| {
                                l.strip_prefix("APP_PASSWORD=")
                                    .map(|v| v.trim().to_string())
                            })
                        })
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };

                if let Some(p) = persisted {
                    p
                } else {
                    let generated = generate_random_password(32);
                    if let Err(e) = persist_app_password(&generated) {
                        tracing::warn!("Failed to persist APP_PASSWORD: {e}");
                    }
                    // Do NOT log the password value — it would leak into
                    // container logs, journald, shell history, etc. Point
                    // the user at the persisted file instead.
                    let env_path = config_dir().join(".env");
                    tracing::warn!("┌──────────────────────────────────────────────────────────┐");
                    tracing::warn!("│ No APP_PASSWORD set. Generated a random one and persisted │");
                    tracing::warn!(
                        "│ it to {}|",
                        format!("{:<48}", env_path.display().to_string()).trim_end()
                    );
                    tracing::warn!("│                                                          │");
                    tracing::warn!("│ To view it:  cat ~/.switchpost/.env                    │");
                    tracing::warn!("│ To change it: edit that file or set APP_PASSWORD in env  │");
                    tracing::warn!("└──────────────────────────────────────────────────────────┘");
                    generated
                }
            }
        };

        // Derive the session/JWT secret from the password if not explicitly set.
        // This way one env var (`APP_PASSWORD`) is sufficient to secure both
        // the cookie signing and the OAuth state tokens.
        let jwt_secret = match opt("JWT_SECRET") {
            Some(s) if !s.is_empty() => s,
            _ => format!("sf-session-{}", app_password),
        };

        let database_url = opt("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());
        ensure_sqlite_url(&database_url)?;

        Ok(Self {
            database_url,
            jwt_secret,
            app_password,
            app_url,
            frontend_url,
            x_client_id: opt("X_CLIENT_ID"),
            x_client_secret: opt("X_CLIENT_SECRET"),
            x_auth_token: opt("X_AUTH_TOKEN"),
            x_ct0: opt("X_CT0"),
            linkedin_client_id: opt("LINKEDIN_CLIENT_ID"),
            linkedin_client_secret: opt("LINKEDIN_CLIENT_SECRET"),
            bluesky_handle: opt("BLUESKY_HANDLE"),
            bluesky_app_password: opt("BLUESKY_APP_PASSWORD"),
            facebook_client_id: opt("FACEBOOK_CLIENT_ID"),
            facebook_client_secret: opt("FACEBOOK_CLIENT_SECRET"),
            instagram_client_id: opt("INSTAGRAM_CLIENT_ID"),
            instagram_client_secret: opt("INSTAGRAM_CLIENT_SECRET"),
            threads_app_id: opt("THREADS_APP_ID"),
            threads_app_secret: opt("THREADS_APP_SECRET"),
            youtube_client_id: opt("YOUTUBE_CLIENT_ID"),
            youtube_client_secret: opt("YOUTUBE_CLIENT_SECRET"),
            reddit_client_id: opt("REDDIT_CLIENT_ID"),
            reddit_client_secret: opt("REDDIT_CLIENT_SECRET"),
            reddit_username: opt("REDDIT_USERNAME"),
            reddit_password: opt("REDDIT_PASSWORD"),
            reddit_access_token: opt("REDDIT_ACCESS_TOKEN"),
            reddit_refresh_token: opt("REDDIT_REFRESH_TOKEN"),
            reddit_cookie_token: opt("REDDIT_COOKIE_TOKEN"),
            discord_client_id: opt("DISCORD_CLIENT_ID"),
            discord_client_secret: opt("DISCORD_CLIENT_SECRET"),
            discord_bot_token: opt("DISCORD_BOT_TOKEN"),
            telegram_bot_tokens: opt("TELEGRAM_BOT_TOKENS"),
            telegram_api_id: opt("TELEGRAM_API_ID"),
            telegram_api_hash: opt("TELEGRAM_API_HASH"),
            telegram_session_dir: Some(resolve_telegram_session_dir(opt("TELEGRAM_SESSION_DIR"))),
            pinterest_client_id: opt("PINTEREST_CLIENT_ID"),
            pinterest_client_secret: opt("PINTEREST_CLIENT_SECRET"),
            instagram_app_id: opt("INSTAGRAM_APP_ID"),
            instagram_app_secret: opt("INSTAGRAM_APP_SECRET"),
            whatsapp_store_dir: Some(resolve_whatsapp_store_dir(opt("WHATSAPP_STORE_DIR"))),
            whatsapp_cloud_token: opt("WHATSAPP_CLOUD_TOKEN"),
            whatsapp_cloud_phone_number_id: opt("WHATSAPP_CLOUD_PHONE_NUMBER_ID"),
            whatsapp_cloud_waba_id: opt("WHATSAPP_CLOUD_WABA_ID"),
            whatsapp_cloud_verify_token: opt("WHATSAPP_CLOUD_VERIFY_TOKEN"),
            whatsapp_cloud_app_secret: opt("WHATSAPP_CLOUD_APP_SECRET"),
            slack_client_id: opt("SLACK_CLIENT_ID"),
            slack_client_secret: opt("SLACK_CLIENT_SECRET"),
            tiktok_client_id: opt("TIKTOK_CLIENT_ID"),
            tiktok_client_secret: opt("TIKTOK_CLIENT_SECRET"),
            mastodon_client_id: opt("MASTODON_CLIENT_ID"),
            mastodon_client_secret: opt("MASTODON_CLIENT_SECRET"),
            mastodon_instance_url: opt("MASTODON_INSTANCE_URL"),
            medium_access_token: opt("MEDIUM_ACCESS_TOKEN"),
            devto_api_key: opt("DEVTO_API_KEY"),
            hashnode_api_key: opt("HASHNODE_API_KEY"),
            github_token: opt("GITHUB_TOKEN"),
            neynar_api_key: opt("NEYNAR_API_KEY"),
            token_encryption_key: opt("TOKEN_ENCRYPTION_KEY"),
            media_dir: opt("MEDIA_DIR").unwrap_or_else(|| "./uploads".into()),

            llm_endpoint: opt("LLM_ENDPOINT"),
            llm_model: opt("LLM_MODEL"),
            dub_co_api_key: opt("DUB_CO_API_KEY"),
            dub_co_workspace: opt("DUB_CO_WORKSPACE"),
            strip_links_from_x: std::env::var("STRIP_LINKS_FROM_X")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        })
    }

    /// Returns (client_id, client_secret) for a given provider
    /// The api id the Telegram client is built with: the operator's own when
    /// set, otherwise Telegram Desktop's public one (see
    /// [`DEFAULT_TELEGRAM_API_ID`]). Never fails — that is the point.
    pub fn telegram_api_id_resolved(&self) -> i32 {
        self.telegram_api_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(DEFAULT_TELEGRAM_API_ID)
    }

    /// The api hash the Telegram client is built with, same rule as
    /// [`Self::telegram_api_id_resolved`].
    pub fn telegram_api_hash_resolved(&self) -> String {
        self.telegram_api_hash
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_TELEGRAM_API_HASH)
            .to_string()
    }

    /// True when the operator has not supplied their own Telegram app
    /// credentials, so the client runs on Telegram Desktop's public pair.
    /// Logged once at startup, exactly as `tg-lyr` warns.
    pub fn telegram_uses_default_credentials(&self) -> bool {
        self.telegram_api_id_resolved() == DEFAULT_TELEGRAM_API_ID
            && self.telegram_api_hash_resolved() == DEFAULT_TELEGRAM_API_HASH
    }

    pub fn provider_credentials(&self, provider: &str) -> Option<(String, String)> {
        match provider {
            "x" => Some((self.x_client_id.clone()?, self.x_client_secret.clone()?)),
            "linkedin" => Some((
                self.linkedin_client_id.clone()?,
                self.linkedin_client_secret.clone()?,
            )),
            "linkedin-page" => Some((
                self.linkedin_client_id.clone()?,
                self.linkedin_client_secret.clone()?,
            )),
            "bluesky" => Some((
                self.bluesky_handle.clone()?,
                self.bluesky_app_password.clone()?,
            )),
            "facebook" => Some((
                self.facebook_client_id.clone()?,
                self.facebook_client_secret.clone()?,
            )),
            "instagram" => Some((
                self.instagram_client_id.clone()?,
                self.instagram_client_secret.clone()?,
            )),
            "instagram-standalone" => Some((
                self.instagram_app_id.clone()?,
                self.instagram_app_secret.clone()?,
            )),
            "threads" => Some((
                self.threads_app_id.clone()?,
                self.threads_app_secret.clone()?,
            )),
            "youtube" => Some((
                self.youtube_client_id.clone()?,
                self.youtube_client_secret.clone()?,
            )),
            "reddit" => {
                let id = self.reddit_client_id.clone()?;
                let secret = self.reddit_client_secret.clone()?;
                Some((id, secret))
            }
            "discord" => Some((
                self.discord_client_id.clone()?,
                self.discord_client_secret.clone()?,
            )),
            "telegram-bot" => Some(("bot".into(), self.telegram_bot_tokens.clone()?)),
            // telegram-user is MTProto (Grammers). It needs the API
            // id/hash from my.telegram.org to build a client at ALL — there is
            // no token to paste — so an absent pair is reported as UNCONFIGURED
            // instead of the dummy pair this arm used to return. The dummy
            // made /setup and the REST connect handler offer a Connect button
            // that could only fail at request-code time with a raw 502, and
            // it made `integrations.providers` claim the channel was ready.
            // Always configured: the manager falls back to Telegram Desktop's
            // public credentials, so a phone number and a code are enough.
            "telegram-user" => Some(("user".into(), "mtproto".into())),
            "whatsapp" => self
                .whatsapp_store_dir
                .as_ref()
                .map(|d| ("whatsapp".into(), d.clone())),
            "pinterest" => Some((
                self.pinterest_client_id.clone()?,
                self.pinterest_client_secret.clone()?,
            )),
            "skool" => Some(("skool".into(), "chrome_extension".into())),
            "slack" => Some((
                self.slack_client_id.clone()?,
                self.slack_client_secret.clone()?,
            )),
            "mastodon" => Some((
                self.mastodon_client_id.clone()?,
                self.mastodon_client_secret.clone()?,
            )),
            "tiktok" => Some((
                self.tiktok_client_id.clone()?,
                self.tiktok_client_secret.clone()?,
            )),
            "medium" => Some(("api-key".into(), self.medium_access_token.clone()?)),
            "devto" => Some(("api-key".into(), self.devto_api_key.clone()?)),
            "hashnode" => Some(("api-key".into(), self.hashnode_api_key.clone()?)),
            "github" => Some((self.github_token.clone()?, self.github_token.clone()?)),
            "google_my_business" | "gmail" | "calendar" | "drive" => Some((
                self.youtube_client_id.clone()?,
                self.youtube_client_secret.clone()?,
            )),
            "farcaster" => Some(("neynar".into(), self.neynar_api_key.clone()?)),
            _ => None,
        }
    }

    /// Returns Reddit username for password grant auth
    pub fn reddit_username(&self) -> Option<String> {
        self.reddit_username.clone()
    }

    /// Returns Reddit password for password grant auth
    pub fn reddit_password(&self) -> Option<String> {
        self.reddit_password.clone()
    }

    /// Returns Reddit access token (pre-authorized via auth code flow)
    pub fn reddit_access_token(&self) -> Option<String> {
        self.reddit_access_token.clone()
    }

    /// Returns Reddit refresh token (obtained via auth code flow)
    pub fn reddit_refresh_token(&self) -> Option<String> {
        self.reddit_refresh_token.clone()
    }
}

/// Telegram Desktop's public API credentials.
///
/// `tg-lyr` (the reference implementation the founder named) ships these as
/// the built-in default: a user who has never been to my.telegram.org can
/// still log in with a phone number and a code, because Telegram accepts the
/// Desktop client's well-known id/hash pair. `TG_API_ID` / `TG_API_HASH`
/// remain optional overrides for anyone who wants their own app identity.
pub const DEFAULT_TELEGRAM_API_ID: i32 = 2040;
pub const DEFAULT_TELEGRAM_API_HASH: &str = "b18441a1ff607e10a989891a5462e627";

fn opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

/// Generate a random URL-safe password of the given length.
pub(crate) fn generate_random_password(len: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                             abcdefghijklmnopqrstuvwxyz\
                             0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Persist `APP_PASSWORD=<pw>` to `~/.switchpost/.env`, creating
/// the directory if needed. Idempotent — replaces any existing line.
///
/// Returns the write error instead of only warning so callers that
/// report the outcome to a human (e.g. the CLI) can fail loudly.
pub(crate) fn persist_app_password(password: &str) -> anyhow::Result<std::path::PathBuf> {
    persist_env_vars(&[("APP_PASSWORD", password)])
}

/// Persist one or more `KEY=VALUE` pairs into `~/.switchpost/.env`,
/// creating the directory if needed. Idempotent — an existing line for a
/// key is REPLACED, and a key that is not in the file is appended.
///
/// ─── WHY THIS EXISTS ────────────────────────────────────────
/// The Telegram USER channel needs `TELEGRAM_API_ID` and
/// `TELEGRAM_API_HASH` from https://my.telegram.org before it can
/// build an MTProto client at all. Until now the only way to supply
/// them was to edit the file on the server by hand and restart, so the
/// founder's report was "Telegram is not configured on this server"
/// with no way forward from the UI.
///
/// This is the same seam `persist_app_password` already used, generalised
/// to N keys, so a credential the founder pastes into the app is written
/// to the SAME file the process reads at startup and survives a restart.
///
/// The value is written VERBATIM, so a value containing spaces or `#`
/// would be mis-parsed by dotenvy on the next load. The callers pass
/// values that are either numeric or a hex/bot-token string, and the
/// writer refuses a value with a newline (which would inject a second
/// line into the file) rather than writing something that parses as a
/// different key.
pub(crate) fn persist_env_vars(pairs: &[(&str, &str)]) -> anyhow::Result<std::path::PathBuf> {
    for (key, value) in pairs {
        if !key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            || key.is_empty()
        {
            anyhow::bail!("Refusing to write an env var with an invalid name: {key:?}");
        }
        if value.contains('\n') || value.contains('\r') {
            anyhow::bail!("Refusing to write {key}: the value contains a newline.");
        }
    }

    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let env_path = dir.join(".env");
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();

    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    for (key, value) in pairs {
        let prefix = format!("{key}=");
        // Replace the first occurrence and drop any duplicates, so the
        // file never ends up with two lines for one key (dotenvy would
        // take the first, and a stale first line is a silent surprise).
        let mut replaced = false;
        let mut kept: Vec<String> = Vec::with_capacity(lines.len() + 1);
        for line in lines.drain(..) {
            if line.starts_with(&prefix) {
                if !replaced {
                    kept.push(format!("{key}={value}"));
                    replaced = true;
                }
                // A duplicate is dropped rather than kept.
            } else {
                kept.push(line);
            }
        }
        if !replaced {
            kept.push(format!("{key}={value}"));
        }
        lines = kept;
    }

    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    std::fs::write(&env_path, body)?;
    Ok(env_path)
}

/// Does any origin in the comma-separated `FRONTEND_URL`/`CSRF` allow-list
/// cover the port this server is actually bound to?
///
/// The browser sends `Origin` with the port the page is ACTUALLY served from,
/// and `csrf_origin_check` compares it against the allow-list verbatim. A
/// serve on any other port makes every state-changing route 403
/// (`csrf_origin_mismatch`) — login included — which reads as "the app is
/// broken" rather than "the env is stale". This bit twice before
/// (worklog 2026-10-22; AGENTS.md §10's FRONTEND_URL note documents the trap).
///
/// A scheme-default URL (no explicit `:port`, i.e. `https://…` = 443) counts
/// as covering only a 443 bind.
pub fn frontend_origins_cover_port(frontend_url: &str, bind_port: u16) -> bool {
    frontend_url
        .split(',')
        .filter_map(|o| url::Url::parse(o.trim()).ok())
        .any(|u| u.port() == Some(bind_port) || (u.port().is_none() && bind_port == 443))
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_sqlite_url, frontend_origins_cover_port, Config, DEFAULT_TELEGRAM_API_HASH,
        DEFAULT_TELEGRAM_API_ID, DEFAULT_TELEGRAM_SESSION_DIR, DEFAULT_WHATSAPP_STORE_DIR,
    };

    #[test]
    fn accepts_sqlite_urls_and_bare_paths() {
        for ok in [
            "sqlite://data/switchpost.db",
            "sqlite://data/switchpost.db?mode=rwc",
            "sqlite::memory:",
            // sqlx reads a scheme-less value as a filename — still valid.
            "data/switchpost.db",
            "/var/lib/switchpost.db",
        ] {
            assert!(
                ensure_sqlite_url(ok).is_ok(),
                "expected `{ok}` to be accepted"
            );
        }
    }

    #[test]
    fn rejects_non_sqlite_schemes() {
        for bad in [
            "postgres://switchpost:pw@localhost:5433/switchpost",
            "postgresql://localhost/db",
            "mysql://root@localhost:3306/switchpost",
        ] {
            let err = ensure_sqlite_url(bad)
                .expect_err(&format!("expected `{bad}` to be rejected"))
                .to_string();
            assert!(err.contains("SQLite-only"), "unhelpful error: {err}");
            assert!(err.contains("sqlite://data/switchpost.db"));
        }
    }

    /// Credentials live in the URL's userinfo section, so the error must name
    /// the scheme without echoing the value.
    #[test]
    fn error_does_not_echo_credentials() {
        let err = ensure_sqlite_url("postgres://user:hunter2@db:5432/app")
            .expect_err("postgres must be rejected")
            .to_string();
        assert!(!err.contains("hunter2"), "leaked password: {err}");
        assert!(err.contains("postgres"), "scheme not named: {err}");
    }

    #[test]
    fn port_coverage_accepts_matching_port() {
        assert!(frontend_origins_cover_port("https://localhost:6544", 6544));
    }

    #[test]
    fn port_coverage_rejects_stale_port() {
        // The exact drift that 403'd every browser write: env pinned to
        // 6543, serve running on 6544.
        assert!(!frontend_origins_cover_port("https://localhost:6543", 6544));
    }

    #[test]
    fn port_coverage_accepts_any_matching_origin_in_list() {
        // v23-9 comma-separated allow-list: one matching entry is enough.
        assert!(frontend_origins_cover_port(
            "https://a.com, https://localhost:6544",
            6544
        ));
    }

    #[test]
    fn port_coverage_scheme_default_only_covers_443() {
        assert!(frontend_origins_cover_port("https://example.com", 443));
        assert!(!frontend_origins_cover_port("https://example.com", 6544));
    }

    #[test]
    fn port_coverage_ignores_unparseable_entries() {
        assert!(frontend_origins_cover_port(
            "not a url, https://localhost:6544",
            6544
        ));
        // Nothing parseable at all — never claim coverage.
        assert!(!frontend_origins_cover_port("*", 6544));
    }

    // ── The three channel-login regressions ───────────────────

    /// The founder's WhatsApp connect failed with "Set WHATSAPP_SESSION_PATH"
    /// — a variable this binary has never read. The real one is
    /// WHATSAPP_STORE_DIR, and it had NO default, so the startup gate in
    /// main.rs refused to build the wa-rs client and the channel was dead on a
    /// default install even though the provider had a working default of its own.
    /// A default resolved at config load means ONE place decides it.
    #[test]
    fn whatsapp_store_dir_defaults_so_the_channel_is_reachable() {
        use super::resolve_whatsapp_store_dir;

        assert_eq!(
            resolve_whatsapp_store_dir(None),
            "./data/whatsapp",
            "an unset WHATSAPP_STORE_DIR must still yield a usable path, or the \
             startup gate never builds the wa-rs client and the channel is dead"
        );

        // A cleared line is "unset", not "use the empty string".
        assert_eq!(
            resolve_whatsapp_store_dir(Some("".into())),
            "./data/whatsapp"
        );
        assert_eq!(
            resolve_whatsapp_store_dir(Some("   ".into())),
            "./data/whatsapp"
        );

        // A real choice is honoured verbatim.
        assert_eq!(
            resolve_whatsapp_store_dir(Some("/var/lib/wa".into())),
            "/var/lib/wa"
        );

        // And the constant the two log sites print is the same default.
        assert_eq!(DEFAULT_WHATSAPP_STORE_DIR, "./data/whatsapp");
    }

    /// The Telegram USER session has the same dead default, on the same
    /// reported channel. The server and the CLI both fell back to `./data/telegram`
    /// when building the client manager while the config field stayed `None` - so
    /// the CLI's own `connect telegram-user` gate demanded a THIRD variable the
    /// server had never needed and answered `not_configured` on an instance that
    /// would have connected. One default, resolved once.
    #[test]
    fn telegram_session_dir_defaults_so_the_channel_is_reachable() {
        use super::resolve_telegram_session_dir;

        assert_eq!(
            resolve_telegram_session_dir(None),
            "./data/telegram",
            "an unset TELEGRAM_SESSION_DIR must still yield a usable path, or the \
             CLI's connect gate reports not_configured on a working instance"
        );
        assert_eq!(
            resolve_telegram_session_dir(Some("".into())),
            "./data/telegram"
        );
        assert_eq!(
            resolve_telegram_session_dir(Some("  ".into())),
            "./data/telegram"
        );
        assert_eq!(
            resolve_telegram_session_dir(Some("/var/lib/tg".into())),
            "/var/lib/tg"
        );
        assert_eq!(DEFAULT_TELEGRAM_SESSION_DIR, "./data/telegram");

        // The field is always `Some`, which is the invariant every reader relies
        // on - so no reader has to invent a default. The actual VALUE comes from
        // the environment (this repo's .env sets a different one, which is
        // honoured), so this asserts the SHAPE, not a specific path.
        let config = Config::from_env().expect("config");
        let resolved = config
            .telegram_session_dir
            .as_deref()
            .expect("telegram_session_dir must always resolve to a usable path");
        assert!(
            !resolved.trim().is_empty(),
            "the resolved session dir must not be blank, got {resolved:?}"
        );
    }

    /// telegram-user is MTProto: without the id/hash the client manager is
    /// Telegram Desktop's public credentials are the DEFAULT (see
    /// [`DEFAULT_TELEGRAM_API_ID`] and tg-lyr): a phone number and a code are
    /// enough, so the channel is configured even when the operator has never
    /// been to my.telegram.org. Their own pair, when set, wins.
    #[test]
    fn telegram_user_credentials_need_no_env_vars() {
        let mut config = Config::from_env().expect("config");

        // No id/hash: still configured, on Telegram Desktop's public pair.
        config.telegram_api_id = None;
        config.telegram_api_hash = None;
        assert_eq!(config.telegram_api_id_resolved(), DEFAULT_TELEGRAM_API_ID);
        assert_eq!(config.telegram_api_hash_resolved(), DEFAULT_TELEGRAM_API_HASH);
        assert!(
            config.provider_credentials("telegram-user").is_some(),
            "telegram-user must be connectable with a phone number alone"
        );
        assert!(config.telegram_uses_default_credentials());

        // A blank pair is "not set" - the CLI treats empty strings that way,
        // so config must agree rather than parse "" as an id.
        config.telegram_api_id = Some("  ".into());
        config.telegram_api_hash = Some("".into());
        assert_eq!(config.telegram_api_id_resolved(), DEFAULT_TELEGRAM_API_ID);
        assert!(config.telegram_uses_default_credentials());

        // The operator's own app identity wins.
        config.telegram_api_id = Some("12345".into());
        config.telegram_api_hash = Some("abc".into());
        assert_eq!(config.telegram_api_id_resolved(), 12345);
        assert_eq!(config.telegram_api_hash_resolved(), "abc");
        assert!(
            !config.telegram_uses_default_credentials(),
            "a custom pair must not be reported as the default"
        );
    }
}
