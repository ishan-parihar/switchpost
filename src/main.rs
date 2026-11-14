// ─── Switchpost ──────────────────────────────────────────────
// Social Media Scheduling Platform — Rust implementation.
//
// Architecture:
//   - axum HTTP server: REST API for SvelteKit frontend
//   - rmcp MCP server: stdio-based tools for AI agents
//   - In-process scheduler: handles post publishing
//   - SSE broadcast: real-time updates for both frontend and agents
//
// Two kinds of end users:
//   1. Humans via SvelteKit frontend (HTTP + SSE)
//   2. AI agents via MCP tools (stdio/SSE)

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use switchpost::api;
use switchpost::cli::{self, Cli, Command};
use switchpost::config;
use switchpost::db;
use switchpost::mcp;
use switchpost::rss;
use switchpost::scheduler;

use anyhow::Context;

use switchpost::api::AppState;
use switchpost::realtime::Broadcaster;
use switchpost::services::telegram_client::TelegramClientManager;
use switchpost::social::registry::ProviderRegistry;
use switchpost::wa::WhaClient;

/// Start the six background pollers. One call site at boot. Loop bodies
/// are untouched. There is no leader gate: SQLite is a single-writer
/// store, so this binary is the only process that can hold the write
/// lock, and running the pollers unconditionally is what makes a
/// single instance behave exactly as it did before.
#[allow(clippy::too_many_arguments)]
fn spawn_pollers(
    db: db::SqlitePool,
    providers_arc: Arc<ProviderRegistry>,
    broadcaster: Broadcaster,
    token_key: Option<[u8; 32]>,
    config: config::Config,
    shutdown_tx: &tokio::sync::watch::Sender<bool>,
) -> tokio::sync::watch::Receiver<bool> {
    let scheduler_rx = shutdown_tx.subscribe();
    let scheduler_drained = scheduler::start_scheduler(
        db.clone(),
        providers_arc.clone(),
        broadcaster.clone(),
        token_key,
        config.clone(),
        scheduler_rx,
    );

    // ── Start RSS poller ─────────────────────────────────────
    let rss_rx = shutdown_tx.subscribe();
    rss::start_rss_poller(
        db.clone(),
        providers_arc.clone(),
        Arc::new(config.clone()),
        rss_rx,
    );

    // ── Start analytics cache refresh ─────────────────────────
    let cache_db = db.clone();
    let cache_providers = providers_arc.clone();
    let cache_shutdown = shutdown_tx.subscribe();
    let cache_token_key = token_key;
    tokio::spawn(async move {
        scheduler::run_analytics_cache_refresh(
            cache_db,
            cache_providers,
            cache_token_key,
            cache_shutdown,
        )
        .await;
    });

    // ── Start feed refresher ────────────────────────────────────
    let feed_rx = shutdown_tx.subscribe();
    switchpost::feed::start_feed_refresher(
        db.clone(),
        providers_arc.clone(),
        broadcaster.clone(),
        token_key,
        feed_rx,
    );

    // ── Start streak reset checker ────────────────────────────
    let streak_rx = shutdown_tx.subscribe();
    scheduler::start_streak_reset(db.clone(), streak_rx);

    scheduler_drained
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install rustls crypto provider for TLS
    let _ = rustls::crypto::ring::default_provider().install_default();

    // AXI §8: Content-first home view — show live state when no args provided
    if std::env::args().len() <= 1 {
        cli::home::handle_home();
        return Ok(());
    }

    let cli_args = Cli::parse();

    // Extract port for server mode
    let port = match &cli_args.command {
        Command::Serve { port } => *port,
        _ => 6543,
    };

    // Dispatch non-server commands to CLI handler
    match &cli_args.command {
        Command::Serve { .. } | Command::Mcp => {
            // Fall through to server startup below
        }
        _ => {
            return cli::run_cli(cli_args).await;
        }
    }

    // ── Init logging ──────────────────────────────────────────
    switchpost::config::load_dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,switchpost=debug".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // ── Load config ───────────────────────────────────────────
    let config = config::Config::from_env()?;
    tracing::info!("Configuration loaded");

    // ── §5.1(a): thin MCP ──────────────────────────────
    // `switchpost mcp` used to boot the FULL server — writer lock,
    // HTTP, all 6 pollers, Telegram/WhatsApp clients — so it died on
    // the lock whenever the systemd `serve` daemon was running, and
    // without systemd each agent session ran its own rival scheduler.
    // Probe the lock first: held by a live daemon → thin mode (pool
    // read-write, MCP on stdio only — the daemon's scheduler picks up
    // the rows). Free (or stale-adopted) → merged mode below behaves
    // as before. NOTE: claim_writer_role is check-and-acquire — the
    // probe's successful claim IS the merge-mode guard; re-claiming at
    // line 176 self-collided and killed every `mcp` start after a
    // kill -9 left a stale lock (observed 2026-10-17).
    let probe_claim = if matches!(cli_args.command, Command::Mcp) {
        match db::claim_writer_role(&config.database_url) {
            Ok(guard) => Some(guard),
            Err(_) => {
                tracing::info!(
                    "Writer lock held by the serve daemon — thin MCP mode: \
                     no scheduler, no HTTP, no messaging clients"
                );
                return run_mcp_thin(config).await;
            }
        }
    } else {
        None
    };

    // ── Database ──────────────────────────────────────────────
    // Single-writer discipline (G-08): this process runs the publish scheduler,
    // so it must be the ONLY long-lived writer on `database_url`. Claim the
    // lock BEFORE opening the pool — a second `serve` on the same DATABASE_URL
    // then fails loudly here instead of silently double-publishing.
    let _writer_lock = match probe_claim {
        Some(guard) => guard,
        None => db::claim_writer_role(&config.database_url)
            .context("Failed to claim the SQLite single-writer lock")?,
    };

    let db = db::create_pool(&config.database_url)
        .await
        .context("Failed to create database pool")?;
    tracing::info!("Database connected (single-writer lock held)");

    // Ensure the single local user row exists (needed for FK constraints)
    if let Err(e) = db::ensure_local_user(&db).await {
        tracing::warn!("Failed to ensure local user row: {e} — DB inserts may fail");
    }

    // ── Realtime broadcaster ──────────────────────────────────
    let broadcaster = Broadcaster::new();

    // ── Telegram user client (Grammers) ──────────────────────
    //
    // ─── THE MANAGER ALWAYS EXISTS ────────────────────────────
    // It used to be `None` when TELEGRAM_API_ID/TELEGRAM_API_HASH were
    // unset, and every route that needed it answered "Telegram is not
    // configured on this server ... set the env vars and restart". The
    // founder could not act on that from the app: the only way to supply
    // the pair was to edit the server's `.env` by hand and restart the
    // service. That is the exact dead end the login report described.
    //
    // Now the manager is built either way. With credentials it behaves
    // exactly as before; without them it starts UNCONFIGURED and the
    // channels screen offers a paste-in form whose endpoint writes the
    // pair to `~/.switchpost/.env` and swaps them into this manager in
    // place — no restart.
    //
    // A malformed TELEGRAM_API_ID (non-numeric) is still a startup
    // error rather than a silent "unconfigured": the operator set the
    // variable and it is wrong, which is a different fault from not
    // setting it.
    let telegram_session_dir = PathBuf::from(
        config
            .telegram_session_dir
            .clone()
            .unwrap_or_else(|| config::DEFAULT_TELEGRAM_SESSION_DIR.into()),
    );
    let telegram_client_manager = {
        // Telegram Desktop's public credentials are the default (see
        // `Config::DEFAULT_TELEGRAM_API_ID` and tg-lyr): a phone number and a
        // code are enough, and TELEGRAM_API_ID/HASH are optional overrides.
        let api_id = config.telegram_api_id_resolved();
        let api_hash = config.telegram_api_hash_resolved();
        if config.telegram_uses_default_credentials() {
            tracing::warn!(
                "TELEGRAM_API_ID / TELEGRAM_API_HASH not set — using Telegram Desktop's public \
                 credentials (api_id {api_id}). Set your own pair for a separate app identity."
            );
        }
        Some(Arc::new(TelegramClientManager::new(
            api_id,
            api_hash,
            telegram_session_dir,
        )))
    };


    let wa_client: switchpost::wa::OptionalWhaClient = if let Some(dir) = &config.whatsapp_store_dir
    {
        let store_dir = PathBuf::from(dir);
        match WhaClient::new(store_dir).await {
            Ok(client) => {
                tracing::info!("WhatsApp Web client (wa-rs): initialized");
                Some(Arc::new(tokio::sync::Mutex::new(client)))
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WhatsApp client: {e}");
                None
            }
        }
    } else {
        tracing::info!(
            "WHATSAPP_STORE_DIR unset — WhatsApp Web client at the default {}",
            config::DEFAULT_WHATSAPP_STORE_DIR
        );
        None
    };

    // ── Provider registry ─────────────────────────────────────
    let providers =
        ProviderRegistry::new(&config, telegram_client_manager.clone(), wa_client.clone());
    let providers_arc = Arc::new(providers);

    // ── Shared app state ─────────────────────────────────────
    // Parse optional token encryption key (64 hex chars = 32 bytes)
    let token_key = config
        .token_encryption_key
        .as_ref()
        .and_then(|k| switchpost::crypto::decode_hex_key(k).ok());
    if token_key.is_some() {
        tracing::info!("Token encryption at rest: ENABLED");
    } else {
        tracing::warn!(
            "Token encryption at rest: DISABLED (set TOKEN_ENCRYPTION_KEY for production)"
        );
    }

    // ── Re-encrypt tokens written before the encrypting writer ──
    //
    // `create_integration` became the one encrypting writer in v12. Rows
    // written before that landed can still hold a PLAINTEXT token (every
    // connect path except the OAuth callback did). They keep working —
    // `maybe_decrypt_token` falls back to the raw value — but the file on
    // disk is the leak. This pass encrypts them once, at boot.
    if let Some(key) = token_key {
        match switchpost::db::queries::reencrypt_plaintext_tokens(&db, key).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                "Token encryption at rest: re-encrypted {n} plaintext integration token(s)"
            ),
            Err(e) => tracing::warn!("Startup token re-encryption failed: {e}"),
        }
    }

    // ── Build shared state (clone for MCP if needed) ──────────
    // Media clients are the shared statics (F1): `media_http_client` keeps
    // its 10s connect timeout for CDN fetching, while `media_wreq_client`
    // keeps Chrome131 emulation for video.twimg.com.
    let media_http_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        // No total timeout — needed for streaming large video files via proxy.
        // The browser manages video playback timeout independently.
        .build()
        .unwrap_or_else(|_| switchpost::social::shared_http());

    // wreq client with Chrome TLS fingerprinting for X/Twitter CDN.
    // video.twimg.com blocks requests that don't match a real browser's TLS fingerprint.
    let media_wreq_client = wreq::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .emulation(wreq_util::Emulation::Chrome131)
        .gzip(true)
        .brotli(true)
        .build()
        .unwrap_or_else(|_| switchpost::social::shared_wreq());

    let state = AppState {
        db: db.clone(),
        config: config.clone(),
        broadcast: broadcaster.clone(),
        providers: (*providers_arc).clone(),
        token_key,
        telegram_client_manager: telegram_client_manager.clone(),
        wa_client,
        media_http_client,
        media_wreq_client,
    };
    let state_for_mcp = state.clone();

    // ── F4: X queryId discovery ─────────────────────
    // Hydrate the in-memory map from the DB, then re-scan x.com bundles if
    // the DB copy is stale. Best-effort background task — the static
    // fallback table keeps publish working if it never completes.
    {
        let db_for_qids = db.clone();
        tokio::spawn(async move {
            switchpost::social::x_queryids::hydrate_from_db(&db_for_qids).await;
            switchpost::social::x_queryids::refresh_if_stale(
                &db_for_qids,
                &switchpost::social::shared_wreq(),
            )
            .await;
        });
    }

    // ── Start scheduler ───────────────────────────────────────
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let scheduler_drained = spawn_pollers(
        db.clone(),
        providers_arc.clone(),
        broadcaster.clone(),
        token_key,
        config.clone(),
        &shutdown_tx,
    );

    // ── Build HTTP router ─────────────────────────────────────
    let app = api::build_router(state);

    // ── Start HTTP/HTTPS server ─────────────────────────────
    // SECURITY: bind to loopback by default. The app is documented
    // as a single-user local-deployment tool; binding 0.0.0.0 by
    // default would expose the (intentionally light) auth layer
    // to anyone on the LAN. Operators who want LAN exposure (e.g.
    // to access from another machine on the home network) can opt
    // in explicitly with `BIND_HOST=0.0.0.0`.
    let bind_host = std::env::var("BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    if bind_host == "0.0.0.0" {
        tracing::warn!("⚠️  BIND_HOST=0.0.0.0 — server is reachable from the network.");
        tracing::warn!("   Ensure APP_PASSWORD is strong and the port is firewalled.");
    }
    let http_addr = format!("{bind_host}:{port}");
    let use_tls = config.app_url.starts_with("https://");

    // CSRF vs bind-port drift guard — the predicate and its rationale live in
    // `config::frontend_origins_cover_port`. Warn at boot, naming the actual
    // port, so a relaunch on a different port surfaces as a one-line env fix
    // instead of mysterious browser 403s on every state-changing route.
    if !config::frontend_origins_cover_port(&config.frontend_url, port) {
        tracing::warn!(
            "⚠️  FRONTEND_URL={:?} does not include this server's port {port}. \
             Browsers will send Origin https://localhost:{port} and every \
             state-changing route will 403 (csrf_origin_mismatch) — including \
             login. Fix: APP_URL/FRONTEND_URL=https://localhost:{port} (or \
             serve --port to match the configured URL).",
            config.frontend_url
        );
    }

    // Shared shutdown signal — fires once on SIGINT (Ctrl+C) OR SIGTERM.
    // Both the HTTP server (via `with_graceful_shutdown`) and the
    // background-task drain logic below subscribe to this same signal,
    // so a single SIGTERM cleanly stops everything in the right order:
    //   1. axum stops accepting new connections
    //   2. axum drains in-flight requests (default 5s)
    //   3. shutdown_tx broadcast → scheduler/RSS/feed stop iterating
    //   4. up to 10s grace, then process exits
    let shutdown_tx_signal = Arc::new(tokio::sync::Notify::new());
    let shutdown_rx_signal = shutdown_tx_signal.clone();
    tokio::spawn(async move {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        let sigterm = async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut s) = signal(SignalKind::terminate()) {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = ctrl_c => tracing::info!("Shutdown signal: Ctrl+C (SIGINT)"),
            _ = sigterm => tracing::info!("Shutdown signal: SIGTERM"),
        }
        shutdown_tx_signal.notify_waiters();
    });

    if use_tls {
        // Generate self-signed cert for localhost (or use mkcert certs if available)
        let cert_path = std::path::Path::new("data/tls/cert.pem");
        let key_path = std::path::Path::new("data/tls/key.pem");

        if !cert_path.exists() {
            std::fs::create_dir_all("data/tls")?;
            let mut params =
                rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])?;
            params
                .subject_alt_names
                .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
                    std::net::Ipv4Addr::LOCALHOST,
                )));
            let key_pair = rcgen::KeyPair::generate()?;
            let cert = params.self_signed(&key_pair)?;
            std::fs::write(cert_path, cert.pem())?;
            std::fs::write(key_path, key_pair.serialize_pem())?;
            tracing::info!("Generated self-signed TLS cert at data/tls/");
            tracing::warn!("⚠️  For browsers: trust the cert or use mkcert. See README.");
        }

        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
            .await
            .context("Failed to load TLS config")?;

        let addr: std::net::SocketAddr = http_addr
            .parse()
            .with_context(|| format!("invalid HTTP bind address {http_addr:?}"))?;
        let notify = shutdown_rx_signal.clone();
        let handle = axum_server::Handle::new();
        let handle_for_shutdown = handle.clone();
        tokio::spawn(async move {
            // When the shutdown signal fires, tell axum_server to drain.
            notify.notified().await;
            handle_for_shutdown.shutdown();
        });
        tokio::spawn(async move {
            if let Err(e) = axum_server::bind_rustls(addr, tls_config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await
            {
                tracing::error!("HTTPS server error: {e}");
            }
        });
        tracing::info!("HTTPS server: https://localhost:{port}/ (bound to {bind_host})");
    } else {
        let listener = tokio::net::TcpListener::bind(&http_addr)
            .await
            .context("Failed to bind HTTP listener")?;
        let notify = shutdown_rx_signal.clone();
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    notify.notified().await;
                })
                .await
            {
                tracing::error!("HTTP server error: {e}");
            }
        });
        tracing::info!("HTTP server: http://{http_addr}/");
    }

    let scheme = if use_tls { "https" } else { "http" };
    tracing::info!("Dashboard: {scheme}://localhost:{port}/");
    tracing::info!("Setup: {scheme}://localhost:{port}/setup");
    tracing::info!("OAuth redirect_uri: {}/api/auth/callback", config.app_url);

    // ── MCP mode: also start MCP stdio server ────────────────
    if matches!(cli_args.command, Command::Mcp) {
        tracing::info!("Starting in MERGED mode: HTTP on {http_addr} + MCP on stdio");
        tokio::spawn(async move {
            tracing::info!("MCP server started on stdio");
            if let Err(e) = mcp::run_mcp_stdio(state_for_mcp).await {
                tracing::error!("MCP server error: {e}");
            }
        });
    }

    // ── Wait for shutdown signal ────────────────────────────
    // Block until SIGINT/SIGTERM fires the shared notify. Once it
    // does, broadcast to all background tasks (scheduler, RSS, feed,
    // analytics cache) via the existing `shutdown_tx` watch channel
    // and give them a bounded grace period to finish in-flight work
    // before exiting.
    tracing::info!("Server running. Press Ctrl+C to shut down.");
    shutdown_rx_signal.notified().await;

    // Mark the process as draining — /ready returns 503 so the load
    // balancer stops routing new traffic. Existing in-flight requests
    // continue to be served by axum's graceful shutdown.
    api::set_draining();

    tracing::info!("Broadcasting shutdown to background tasks…");
    let _ = shutdown_tx.send(true);

    // Wait for the scheduler engine to finish draining its JoinSet (25s
    // budget), capped at 30s wall clock — exits the moment the last
    // in-flight publish lands instead of always sleeping the full window.
    // The cap still covers the other pollers and a wedged engine alike.
    let mut scheduler_drained = scheduler_drained;
    let drained_done = async {
        loop {
            if scheduler_drained.changed().await.is_err() {
                break; // sender dropped — engine task already gone
            }
            if *scheduler_drained.borrow() {
                break;
            }
        }
    };
    tokio::select! {
        _ = drained_done => {
            tracing::info!("Scheduler drained — in-flight publishes settled");
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
            tracing::warn!("Drain window elapsed (30s) — exiting with tasks possibly in flight");
        }
    }

    tracing::info!("Server shut down gracefully");
    Ok(())
}

/// §5.1(a): thin MCP session — pool + MCP stdio, nothing else.
///
/// Runs when the `serve` daemon already owns the writer lock: the agent's
/// process opens the SQLite pool (WAL handles the short writes) and serves
/// the MCP tool surface on stdio. No pollers, no HTTP port, no Telegram /
/// WhatsApp clients, no second scheduler — the daemon picks up the rows
/// this session writes. ~15 MB RSS instead of a full server.
async fn run_mcp_thin(config: switchpost::config::Config) -> anyhow::Result<()> {
    let db = switchpost::db::create_pool(&config.database_url)
        .await
        .context("thin mcp: failed to create database pool")?;
    if let Err(e) = switchpost::db::ensure_local_user(&db).await {
        tracing::warn!("thin mcp: ensure_local_user: {e} — DB inserts may fail");
    }

    let token_key = config
        .token_encryption_key
        .as_ref()
        .and_then(|k| switchpost::crypto::decode_hex_key(k).ok());

    let media_http_client = switchpost::social::shared_http();
    let media_wreq_client = switchpost::social::shared_wreq();

    let state = switchpost::api::AppState {
        db,
        config: config.clone(),
        broadcast: switchpost::realtime::Broadcaster::new(),
        providers: switchpost::social::registry::ProviderRegistry::new(&config, None, None),
        token_key,
        telegram_client_manager: None,
        wa_client: None,
        media_http_client,
        media_wreq_client,
    };

    switchpost::mcp::run_mcp_stdio(state).await
}
