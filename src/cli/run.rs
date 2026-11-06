// ─── CLI Handler Implementations ─────────────────────────────────
// Dispatches CLI subcommands to provider methods, resolving tokens
// the same way MCP tools do (DB > browser extraction > env vars).

use uuid::Uuid;

use rmcp::ServiceExt as _;

use crate::api::AppState;
use crate::config::Config;
use crate::crypto;
use crate::db;
use crate::realtime::Broadcaster;
use crate::social::registry::ProviderRegistry;

use super::{
    AiAction, AutomationAction, CampaignAction, Cli, Command, CommentAction, ConfigAction,
    DmAction, IntegrationsAction, MediaAction, PostsAction, RssAction, SignatureAction,
};
use crate::db::models::Integration;
use crate::social::TargetInfo;

// ── TOON Output (AXI §1/§3) ───────────────────────────────────
// Formerly the `toon-helper` crate: three functions, one call site each.
// A workspace member for this much code cost more than it carried.

/// TOON-encode a value for AI-agent token-efficiency, falling back to
/// compact JSON if the encoder errors.
pub(crate) fn toon_encode<T: serde::Serialize>(value: &T) -> String {
    toon_format::encode_default(value)
        .unwrap_or_else(|_| serde_json::to_string(value).unwrap_or_default())
}

/// Serialize to the requested format ("toon" | "json"). TOON is the default
/// for ~40% token savings (AXI §1).
pub(crate) fn format_text<T: serde::Serialize>(value: &T, format: &str) -> String {
    if format == "json" {
        serde_json::to_string_pretty(value).unwrap_or_default()
    } else {
        toon_encode(value)
    }
}

/// AXI §3: Recursively truncate long string fields to `max_chars`, appending
/// a total-length indicator so the agent knows the field was clipped.
pub(crate) fn truncate_json_strings(
    value: &serde_json::Value,
    max_chars: usize,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), truncate_json_strings(v, max_chars)))
                .collect(),
        ),
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| truncate_json_strings(v, max_chars))
                .collect(),
        ),
        serde_json::Value::String(s) if s.len() > max_chars => {
            let total = s.len();
            let truncated: String = s.chars().take(max_chars).collect();
            serde_json::json!(format!(
                "{}... (truncated, {} chars total)",
                truncated, total
            ))
        }
        other => other.clone(),
    }
}

// ── Output Helpers ───────────────────────────────────────────

/// Output format for every command that prints a result.
///
/// 0 = TOON (default: ~40% fewer tokens for an agent), 1 = JSON, 2 = pretty JSON.
///
/// The `--json` / `--pretty` flags are GLOBAL and were declared but never
/// honored — `output_json` hardcoded TOON, so an agent that asked for JSON
/// got TOON and could not parse the result. Set once at startup from the
/// parsed flags.
static OUTPUT_FORMAT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub(crate) fn set_output_format(json: bool, pretty: bool) {
    let value = if pretty {
        2
    } else if json {
        1
    } else {
        0
    };
    OUTPUT_FORMAT.store(value, std::sync::atomic::Ordering::Relaxed);
}

fn current_output_format() -> u8 {
    OUTPUT_FORMAT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Render a value in the format the global --json/--pretty flags
/// selected. THE one renderer for every CLI result — emit_result and
/// output_json both route through it, so no shim can bypass the flag.
pub(crate) fn render_output(value: &serde_json::Value) -> String {
    match current_output_format() {
        1 => serde_json::to_string(value).unwrap_or_default(),
        2 => serde_json::to_string_pretty(value).unwrap_or_default(),
        _ => format_text(value, "toon"),
    }
}

pub(crate) fn output_json(value: &serde_json::Value) {
    println!("{}", render_output(value));
}

pub(crate) fn output_error(msg: &str) -> anyhow::Result<()> {
    let err = serde_json::json!({"error": msg});
    output_json(&err);
    std::process::exit(2);
}

pub(crate) fn output_error_with_hint(msg: &str, hint: &str) -> ! {
    let err = serde_json::json!({"error": msg, "help": hint});
    output_json(&err);
    std::process::exit(2);
}

// ── Target Discovery Helpers ─────────────────────────────────

pub(crate) async fn fetch_targets(
    state: &AppState,
    integration: &Integration,
) -> anyhow::Result<Vec<TargetInfo>> {
    let provider_obj = state
        .providers
        .get(&integration.provider_identifier)
        .ok_or_else(|| anyhow::anyhow!("Provider not found in registry"))?;

    let token =
        crate::crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

    let targets = provider_obj
        .targets(&token)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch targets: {}", e))?;

    Ok(targets)
}

pub(crate) async fn find_integration(
    state: &AppState,
    user_id: Uuid,
    provider: &str,
) -> anyhow::Result<Integration> {
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    integrations
        .into_iter()
        .find(|i| i.provider_identifier == provider)
        .ok_or_else(|| anyhow::anyhow!("No {} integration found", provider))
}

pub(crate) fn pick_target_interactive(
    targets: &[TargetInfo],
    provider: &str,
) -> anyhow::Result<String> {
    if targets.is_empty() {
        return Err(anyhow::anyhow!(
            "No posting targets found for this {} account",
            provider
        ));
    }

    eprintln!("\nAvailable {} targets:", provider);
    for (i, t) in targets.iter().enumerate() {
        let type_label = if !t.target_type.is_empty() {
            format!(" [{}]", t.target_type)
        } else {
            String::new()
        };
        eprintln!("  {}. {}{}", i + 1, t.name, type_label);
    }
    eprint!("\nSelect target (1-{}): ", targets.len());

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| anyhow::anyhow!("Failed to read input: {}", e))?;

    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid selection: {}", input.trim()))?;

    if choice < 1 || choice > targets.len() {
        return Err(anyhow::anyhow!(
            "Selection out of range (1-{})",
            targets.len()
        ));
    }

    Ok(targets[choice - 1].id.clone())
}

// ── Platform → integration targeting ──────────────────────────

/// Provider identifiers that are a strict prefix of another registered
/// provider. `--platforms instagram` used to match BOTH `instagram` and
/// `instagram-standalone` (and `--platforms linkedin` both `linkedin` and
/// `linkedin-page`), which silently staged TWO drafts per target — the
/// double-draft bug. Reported by `switchpost audit`.
pub const PLATFORM_COLLISIONS: &[(&str, &str)] = &[
    ("instagram", "instagram-standalone"),
    ("linkedin", "linkedin-page"),
];

/// Resolve a `--platforms` list to integration IDs.
///
/// Exact identifier match always wins. A prefix match (`telegram` →
/// `telegram-bot` / `telegram-user`) applies ONLY to names that have no exact
/// match, so a real provider is never shadowed by its own `-suffix` variant.
/// See [`PLATFORM_COLLISIONS`].
pub(crate) fn resolve_platform_integrations<'a, I>(integrations: I, platforms: &str) -> Vec<Uuid>
where
    I: Iterator<Item = &'a crate::db::models::Integration>,
{
    let all: Vec<&crate::db::models::Integration> = integrations.collect();
    let requested: Vec<&str> = platforms
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let exact: Vec<&str> = requested
        .iter()
        .copied()
        .filter(|p| all.iter().any(|i| i.provider_identifier == *p))
        .collect();

    all.iter()
        .filter(|i| {
            requested.iter().any(|p| {
                if i.provider_identifier == *p {
                    return true;
                }
                // Prefix fallback, but never for a name that resolved exactly
                // to a different provider.
                !exact.contains(p) && i.provider_identifier.starts_with(&format!("{p}-"))
            })
        })
        .map(|i| i.id)
        .collect()
}

// ── Lightweight State Init ───────────────────────────────────

async fn init_state() -> anyhow::Result<AppState> {
    crate::config::load_dotenv();
    let config = Config::from_env()?;
    let db = db::create_pool(&config.database_url).await?;
    let broadcaster = Broadcaster::new();
    let token_key = config
        .token_encryption_key
        .as_ref()
        .and_then(|k| crypto::decode_hex_key(k).ok());
    let providers = ProviderRegistry::new(&config, None, None);
    // The CLI/mcp-call path previously hard-wired both to None — every
    // tu_* / wa_* tool was unreachable from the CLI surface even with
    // creds set (parity bug caught during the V1b live pass). Mirror
    // main.rs's construction, but degrade with a warn instead of
    // failing: a malformed TELEGRAM_API_ID must not break `posts list`.
    //
    // The manager ALWAYS exists, exactly as in `main.rs`. "No
    // credentials" is a state it can be moved out of at runtime, so a
    // CLI invocation on an unconfigured instance can still report the
    // channel as needing setup rather than pretending the feature does
    // not exist.
    let telegram_session_dir = std::path::PathBuf::from(
        config
            .telegram_session_dir
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_TELEGRAM_SESSION_DIR.into()),
    );
    let telegram_client_manager = match (&config.telegram_api_id, &config.telegram_api_hash) {
        (Some(id_str), Some(hash)) if !id_str.trim().is_empty() && !hash.trim().is_empty() => {
            match id_str.trim().parse::<i32>() {
                Ok(api_id) => Some(std::sync::Arc::new(
                    crate::services::telegram_client::TelegramClientManager::new(
                        api_id,
                        hash.trim().to_string(),
                        telegram_session_dir,
                    ),
                )),
                Err(_) => {
                    tracing::warn!(
                        "Invalid TELEGRAM_API_ID — Telegram user client starts unconfigured"
                    );
                    Some(std::sync::Arc::new(
                        crate::services::telegram_client::TelegramClientManager::unconfigured(
                            telegram_session_dir,
                        ),
                    ))
                }
            }
        }
        _ => Some(std::sync::Arc::new(
            crate::services::telegram_client::TelegramClientManager::unconfigured(
                telegram_session_dir,
            ),
        )),
    };
    let wa_client: crate::wa::OptionalWhaClient = if let Some(dir) = &config.whatsapp_store_dir {
        let store_dir = std::path::PathBuf::from(dir);
        match crate::wa::WhaClient::new(store_dir).await {
            Ok(client) => {
                tracing::info!("WhatsApp Web client (wa-rs): initialized");
                Some(std::sync::Arc::new(tokio::sync::Mutex::new(client)))
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WhatsApp client: {e}");
                None
            }
        }
    } else {
        tracing::warn!("WHATSAPP_STORE_DIR not set -- WhatsApp tools disabled");
        None
    };
    let state = AppState {
        db: db.clone(),
        config,
        broadcast: broadcaster,
        providers,
        token_key,
        telegram_client_manager,
        wa_client,
        media_http_client: crate::social::shared_http(),
        media_wreq_client: crate::social::shared_wreq(),
    };
    // F4: load the discovered X queryIds from the DB so one-shot CLI X
    // commands use the freshest cached ids. No network here — the daemon
    // owns discovery; the static table is the final fallback.
    crate::social::x_queryids::hydrate_from_db(&db).await;
    Ok(state)
}

// ── systemd user service (§6.4) ─────────────────────

const UNIT_NAME: &str = "switchpost.service";

/// `switchpost service install` — a USER unit, no sudo:
/// ~/.config/systemd/user/switchpost.service with hardening
/// (NoNewPrivileges, ProtectSystem=strict, ReadWritePaths=<data>,
/// PrivateTmp, TimeoutStopSec=60), then daemon-reload, enable --now, and
/// `loginctl enable-linger`. The scheduler's sd_notify pairs with
/// `Type=notify` and `WatchdogSec=120` (§6.3).
fn handle_service_install() -> anyhow::Result<()> {
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("cannot resolve the binary path: {e}"))?;
    let cwd = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("cannot resolve the working directory: {e}"))?;
    let home = std::env::var("HOME")
        .map_err(|_| anyhow::anyhow!("$HOME is not set — a user unit needs a home"))?;

    let unit_dir = format!("{home}/.config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit_path = format!("{unit_dir}/{UNIT_NAME}");

    // ReadWritePaths covers the data dirs the .env's DATABASE_URL actually
    // uses — default is <cwd>/data and <cwd>/uploads.
    let data_dir = format!("{}/data", cwd.display());
    let uploads_dir = format!("{}/uploads", cwd.display());

    let unit = format!(
        "# Generated by `switchpost service install` — edit freely.\n\
         [Unit]\n\
         Description=Switchpost daemon (scheduler + dashboard)\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=notify\n\
         WatchdogSec=120\n\
         ExecStart={exe} serve\n\
         WorkingDirectory={cwd}\n\
         TimeoutStopSec=60\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ReadWritePaths={data_dir} {uploads_dir}\n\
         PrivateTmp=true\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display(),
        cwd = cwd.display(),
        data_dir = data_dir,
        uploads_dir = uploads_dir,
    );
    std::fs::write(&unit_path, unit)?;
    println!("wrote {unit_path}");

    systemctl(&["--user", "daemon-reload"])?;
    systemctl(&["--user", "enable", "--now", UNIT_NAME])?;

    // enable-linger: the user manager starts at boot, so the unit runs
    // without anyone logged in. Absent linger, everything above only
    // lasts until logout.
    let linger = std::process::Command::new("loginctl")
        .args(["enable-linger"])
        .status();
    match linger {
        Ok(s) if s.success() => println!("linger enabled — the service now starts at boot"),
        st => {
            println!(
                "note: `loginctl enable-linger` failed ({st:?}) — the service stops at logout."
            );
        }
    }

    println!(
        "\nstatus:\t`systemctl --user status {UNIT_NAME}`\nlogs:\t`journalctl --user -u {UNIT_NAME} -f`"
    );
    Ok(())
}

/// `switchpost service uninstall` — stop + disable; data and linger
/// are left alone.
fn handle_service_uninstall() -> anyhow::Result<()> {
    systemctl(&["--user", "disable", "--now", UNIT_NAME])?;
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("$HOME is not set"))?;
    let unit_path = format!("{home}/.config/systemd/user/{UNIT_NAME}");
    if std::path::Path::new(&unit_path).exists() {
        std::fs::remove_file(&unit_path)?;
        println!("removed {unit_path}");
        systemctl(&["--user", "daemon-reload"])?;
    }
    println!("uninstalled. (linger left as-is; disable with `loginctl disable-linger`)");
    Ok(())
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("systemctl not available: {e}"))?;
    if !status.success() {
        anyhow::bail!("`systemctl {}` failed ({})", args.join(" "), status);
    }
    Ok(())
}

// ── MCP tool dispatch (§5.3) ────────────────────────

/// `switchpost mcp-tools [--pretty]` — the shell path the agent skill
/// documents (`switchpost mcp-tools --pretty`) and that did not exist
/// until now. Plain output lists names (one per line, scriptable);
/// --pretty aligns "name — description" and prints the count.
async fn handle_mcp_tools_with_state(
    state: &AppState,
    pretty: bool,
    toolset: Option<&str>,
) -> anyhow::Result<()> {
    let _ = state; // the tool router is static — no DB needed.
    let mut tools = crate::mcp::SwitchpostMcpServer::router().list_all();
    // B3: same set-membership rule as the per-session MCP filter
    // (§5.5) so shell discovery matches what a session would advertise.
    if let Some(set) = toolset.map(|s| s.trim().to_ascii_lowercase()) {
        tools.retain(|t| crate::mcp::SwitchpostMcpServer::tool_in_toolset(t.name.as_ref(), &set));
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    if pretty {
        let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
        for t in &tools {
            println!(
                "{:<width$}  {}",
                t.name,
                t.description.as_deref().unwrap_or("")
            );
        }
        println!("\n{} tools", tools.len());
    } else {
        for t in &tools {
            println!("{}", t.name);
        }
    }
    Ok(())
}

/// Call one MCP tool by name, in-process — the shell path the agent
/// skill documents (`switchpost mcp-call <tool> --args '<json>'`,
/// ×116 references in skills/switchpost-agent) and that did not
/// exist until now.
async fn handle_mcp_call_with_state(
    state: &AppState,
    tool: &str,
    args: &str,
) -> anyhow::Result<()> {
    let arguments: serde_json::Map<String, serde_json::Value> = if args.trim().is_empty() {
        Default::default()
    } else {
        match serde_json::from_str::<serde_json::Value>(args) {
            Ok(serde_json::Value::Object(map)) => map,
            Ok(_) => anyhow::bail!("--args must be a JSON object, got: {args}"),
            Err(e) => anyhow::bail!("--args must be a JSON object: {e}"),
        }
    };

    let server = crate::mcp::SwitchpostMcpServer::new(state.clone());
    // §5.5: the per-session toolset filter cuts the ADVERTISED schema
    // (the tools/list payload an agent pays for in context). It is NOT a
    // capability switch: `mcp-call` names ONE tool explicitly and pays
    // no list cost, so filtering it would reject tools the agent can
    // see in `mcp-tools` with "tool not found". The filter stays on the
    // live session (stdio/HTTP), where tools_enable is how a toolset is
    // turned on.
    server.toolsets.insert("all".into());
    let router = crate::mcp::SwitchpostMcpServer::router();
    if router.get(tool).is_none() {
        let mut names: Vec<String> = router
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        anyhow::bail!(
            "unknown tool '{tool}' — {} tools available: {}",
            names.len(),
            names.join(", ")
        );
    }

    // In-process dispatch: a client and the real server over a duplex
    // channel — the same handshake a spawned MCP session performs, with
    // no private constructors. The server side fully implements
    // ServerHandler, so the initialize handshake succeeds.
    struct NoopClient;
    impl rmcp::ClientHandler for NoopClient {}

    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    // Spawn the server handshake — serve() does not resolve until the client
    // sends initialize, so awaiting it inline before building the client
    // deadlocks. The task hands back the RunningService, which must stay
    // ALIVE for the duration of the call (dropping it closes the transport).
    let server_task = tokio::spawn(async move { server.serve(server_io).await });

    let client = NoopClient
        .serve(client_io)
        .await
        .map_err(|e| anyhow::anyhow!("mcp-call init failed: {e}"))?;
    let server_svc = server_task
        .await
        .map_err(|e| anyhow::anyhow!("mcp-call server task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("mcp-call server init failed: {e}"))?;

    // A tool whose params type is `()` rejects an arguments MAP — even an
    // empty one ("invalid type: map, expected unit"). So the arguments are
    // attached only when there are some; otherwise the request carries
    // none, which is what a no-arg tool expects.
    let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
    if !arguments.is_empty() {
        params = params.with_arguments(arguments);
    }

    let result = client
        .call_tool(params)
        .await
        .map_err(|e| anyhow::anyhow!("mcp-call '{tool}' failed: {e}"))?;

    drop(server_svc);

    let mut text_out = String::new();
    for content in &result.content {
        match &content.raw {
            rmcp::model::RawContent::Text(t) => {
                text_out.push_str(&t.text);
                text_out.push('\n');
            }
            other => {
                // Json tool outputs arrive as structured/resource contents —
                // serialize them so mcp-call's stdout stays one JSON blob.
                if let Ok(s) = serde_json::to_string(other) {
                    text_out.push_str(&s);
                    text_out.push('\n');
                }
            }
        }
    }
    // The tool's output is itself a JSON document (rmcp Json outputs are
    // serialized above; Text outputs carry JSON too). Parse it and render
    // through the shared formatter so the global --json/--pretty flags and
    // the list truncation apply to mcp-call like every other command.
    // Unparseable output (plain text) is printed verbatim.
    match serde_json::from_str::<serde_json::Value>(text_out.trim()) {
        Ok(value) => println!("{}", render_output(&truncate_json_strings(&value, 500))),
        Err(_) => print!("{text_out}"),
    }

    if result.is_error.unwrap_or(false) {
        anyhow::bail!("tool '{tool}' returned an error result");
    }
    Ok(())
}

// ── User Resolution ──────────────────────────────────────────

pub(crate) async fn resolve_user(state: &AppState) -> anyhow::Result<Uuid> {
    let user = sqlx::query_scalar::<_, Uuid>(
        "SELECT u.id FROM users u WHERE EXISTS (SELECT 1 FROM integrations i WHERE i.user_id = u.id) LIMIT 1"
    )
    .fetch_optional(&state.db)
    .await?;

    if let Some(id) = user {
        return Ok(id);
    }

    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No user registered"))
}

// ── Token Resolution ─────────────────────────────────────────

pub(crate) async fn find_x_token(
    state: &AppState,
    user_id: Uuid,
) -> anyhow::Result<(String, String)> {
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let x_integrations: Vec<_> = integrations
        .into_iter()
        .filter(|i| i.provider_identifier == "x")
        .collect();

    if let Some(preferred) = x_integrations.iter().find(|i| {
        // §6: explicit column first; shape sniff as legacy-row fallback.
        i.auth_method == "cookie" || i.access_token.starts_with('{')
    }) {
        let token = preferred.access_token.clone();
        let token = crate::crypto::maybe_decrypt_token(&token, state.token_key.as_ref());
        return Ok((token, preferred.internal_id.clone()));
    }

    if let (Some(auth_token), Some(ct0)) = (&state.config.x_auth_token, &state.config.x_ct0) {
        let token = serde_json::json!({"auth_token": auth_token, "ct0": ct0}).to_string();
        return Ok((token, String::new()));
    }

    // B12: no implicit browser extraction — explicit import only
    // (/api/public/connect/x-cookies or env vars).

    if let Some(oauth) = x_integrations.first() {
        let token = oauth.access_token.clone();
        let token = crate::crypto::maybe_decrypt_token(&token, state.token_key.as_ref());
        return Ok((token, oauth.internal_id.clone()));
    }

    anyhow::bail!("No X/Twitter integration found")
}

pub(crate) async fn find_reddit_token(state: &AppState, user_id: Uuid) -> anyhow::Result<String> {
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let reddit_integrations: Vec<_> = integrations
        .into_iter()
        .filter(|i| i.provider_identifier == "reddit")
        .collect();

    if let Some(preferred) = reddit_integrations.iter().find(|i| {
        // §6: explicit column first; shape sniff as legacy-row fallback.
        i.auth_method == "cookie" || i.access_token.starts_with('{')
    }) {
        let token = preferred.access_token.clone();
        let token = crate::crypto::maybe_decrypt_token(&token, state.token_key.as_ref());
        return Ok(token);
    }

    // B12: no implicit browser extraction — explicit import only
    // (/api/public/connect/reddit-cookies).

    if let Some(oauth) = reddit_integrations.first() {
        let token = oauth.access_token.clone();
        let token = crate::crypto::maybe_decrypt_token(&token, state.token_key.as_ref());
        return Ok(token);
    }

    anyhow::bail!("No Reddit integration found")
}

pub(crate) async fn find_linkedin_token(
    state: &AppState,
    user_id: Uuid,
) -> anyhow::Result<(String, String)> {
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let li = integrations
        .iter()
        .find(|i| i.provider_identifier == "linkedin")
        .ok_or_else(|| anyhow::anyhow!("No LinkedIn account connected"))?;
    let token = li.access_token.clone();
    let token = crate::crypto::maybe_decrypt_token(&token, state.token_key.as_ref());
    Ok((token, li.internal_id.clone()))
}

pub(crate) async fn find_linkedin_page_token(
    state: &AppState,
    user_id: Uuid,
    page_id: &str,
) -> anyhow::Result<(String, String)> {
    let lip = state
        .integration_by_internal_id(user_id, "linkedin-page", page_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("LinkedIn Page '{}' not connected", page_id))?;
    let token = crate::crypto::maybe_decrypt_token(&lip.access_token, state.token_key.as_ref());
    Ok((token, lip.internal_id.clone()))
}

pub(crate) async fn find_facebook_page_token(
    state: &AppState,
    user_id: Uuid,
    page_id: &str,
) -> anyhow::Result<String> {
    let page = state
        .integration_by_internal_id(user_id, "facebook", page_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Facebook page '{}' not connected", page_id))?;
    let token = crate::crypto::maybe_decrypt_token(&page.access_token, state.token_key.as_ref());
    Ok(token)
}

// ── Audit Handler ────────────────────────────────────────────

/// Read-only health audit for the two failure modes that are invisible from
/// the happy path: integrations that exist but are disabled (so drafts can
/// never publish), and platform names that fan out to more than one provider
/// (so `--platforms X` stages two drafts). Also reports the live
/// single-writer/busy-timeout posture of the SQLite pool.
async fn handle_audit_with_state(state: &AppState) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;

    let disabled: Vec<serde_json::Value> = integrations
        .iter()
        .filter(|i| i.disabled)
        .map(|i| {
            serde_json::json!({
                "provider": i.provider_identifier,
                "name": i.profile_name,
                "integration_id": i.id.to_string(),
                "impact": "drafts staged against this integration cannot publish",
            })
        })
        .collect();
    let enabled: Vec<&str> = integrations
        .iter()
        .filter(|i| !i.disabled)
        .map(|i| i.provider_identifier.as_str())
        .collect();

    // Which connected providers would a bare `--platforms X` fan out to.
    let collisions: Vec<serde_json::Value> = PLATFORM_COLLISIONS
        .iter()
        .map(|(base, variant)| {
            let base_connected = integrations
                .iter()
                .any(|i| i.provider_identifier == *base && !i.disabled);
            let variant_connected = integrations
                .iter()
                .any(|i| i.provider_identifier == *variant && !i.disabled);
            serde_json::json!({
                "platform": base,
                "also_matches": variant,
                "connected": [base_connected, variant_connected],
                "both_connected": base_connected && variant_connected,
                "guidance": format!(
                    "--platforms {base} targets ONLY {base}. To target both, pass \
                     --platforms {base},{variant} — or use --integrations <uuid,...> \
                     to pick exact integrations."
                ),
            })
        })
        .collect();
    let live_collisions = collisions
        .iter()
        .filter(|c| c["both_connected"] == serde_json::json!(true))
        .count();

    // Live single-writer posture, read back from the open pool.
    let (journal_mode, busy_timeout_ms): (String, i64) = sqlx::query_as(
        "SELECT (SELECT * FROM pragma_journal_mode) AS jm, \
         (SELECT * FROM pragma_busy_timeout) AS bt",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or_else(|e| (format!("unavailable: {e}"), -1));

    let findings: Vec<serde_json::Value> = [
        if disabled.is_empty() {
            serde_json::json!({"id": "disabled_integrations", "status": "ok",
                "detail": "no disabled integrations"})
        } else {
            serde_json::json!({"id": "disabled_integrations", "status": "warn",
                "detail": format!("{} integration(s) disabled=1 — drafts against them will not publish",
                    disabled.len()),
                "integration_ids": disabled.iter().map(|d| d["integration_id"].clone()).collect::<Vec<_>>()})
        },
        if live_collisions == 0 {
            serde_json::json!({"id": "platform_collisions", "status": "ok",
                "detail": "no prefix-colliding platform pair is simultaneously connected"})
        } else {
            serde_json::json!({"id": "platform_collisions", "status": "warn",
                "detail": format!("{live_collisions} colliding pair(s) both connected — \
                    name the variant explicitly or use --integrations")})
        },
        if busy_timeout_ms >= 1000 {
            serde_json::json!({"id": "writer_discipline", "status": "ok",
                "detail": format!("journal_mode={journal_mode} busy_timeout={busy_timeout_ms}ms max_connections=1")})
        } else {
            serde_json::json!({"id": "writer_discipline", "status": "fail",
                "detail": format!("busy_timeout={busy_timeout_ms}ms — concurrent writers will fail fast with SQLITE_BUSY")})
        },
    ]
    .into_iter()
    .collect();

    let warn_count = findings.iter().filter(|f| f["status"] != "ok").count();

    output_json(&serde_json::json!({
        "status": if warn_count == 0 { "clean" } else { "warn" },
        "findings": findings,
        "counts": {
            "integrations": integrations.len(),
            "enabled": enabled.len(),
            "disabled": disabled.len(),
            "colliding_pairs_both_connected": live_collisions,
        },
        "disabled_integrations": disabled,
        "enabled_providers": enabled,
        "platform_collisions": collisions,
        "writer_discipline": {
            "max_connections": 1,
            "journal_mode": journal_mode,
            "busy_timeout_ms": busy_timeout_ms,
            "note": "SQLite is single-writer. Run one long-lived process (serve or mcp) \
                     per database; short CLI invocations are fine alongside it.",
        },
        "help": [
            "Run 'switchpost providers' for the full integration list",
            "Pass --integrations <uuid,...> to target an exact integration",
        ],
    }));
    Ok(())
}

// ── Providers / Connect ──────────────────────────────────────

/// Timestamp for the readiness matrix. ISO-8601 UTC so an agent can diff two
/// runs without parsing.
fn checked_at() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The two readiness flags that need no network and no credential.
///
/// `live_verified` is deliberately absent: it is only ever set from an
/// authenticated round-trip, so there is no code path where a provider looks
/// verified because a flag was on disk. Returns `(draft_ok, publish_capable,
/// reason_when_not_publish_capable)`.
fn readiness_flags(registry: &ProviderRegistry, provider_id: &str) -> (bool, bool, Option<String>) {
    let Some(provider) = registry.get(provider_id) else {
        return (
            false,
            false,
            Some(format!(
                "{provider_id} is not registered in this build — nothing can publish to it"
            )),
        );
    };
    let publish_capable = provider.publish_capable();
    (
        true,
        publish_capable,
        (!publish_capable).then(|| {
            format!(
                "{provider_id} has no post-publish surface (content limit 0) — staged drafts \
                 are stored but never go live"
            )
        }),
    )
}

/// Env var names each provider reads. Names only.
///
/// The audit reports *where* a credential comes from so a "configured"
/// claim can be traced to the DB row or the env file it was read from,
/// without this function ever returning a value to the caller.
fn credential_env_names(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "x" => &["X_CLIENT_ID", "X_CLIENT_SECRET", "X_AUTH_TOKEN", "X_CT0"],
        "linkedin" | "linkedin-page" => &["LINKEDIN_CLIENT_ID", "LINKEDIN_CLIENT_SECRET"],
        "bluesky" => &["BLUESKY_HANDLE", "BLUESKY_APP_PASSWORD"],
        "facebook" | "instagram" => &["FACEBOOK_CLIENT_ID", "FACEBOOK_CLIENT_SECRET"],
        "instagram-standalone" => &["INSTAGRAM_APP_ID", "INSTAGRAM_APP_SECRET"],
        "threads" => &["THREADS_APP_ID", "THREADS_APP_SECRET"],
        "reddit" => &[
            "REDDIT_CLIENT_ID",
            "REDDIT_CLIENT_SECRET",
            "REDDIT_USERNAME",
            "REDDIT_PASSWORD",
            "REDDIT_ACCESS_TOKEN",
            "REDDIT_REFRESH_TOKEN",
        ],
        "youtube" | "google" | "google_my_business" => {
            &["YOUTUBE_CLIENT_ID", "YOUTUBE_CLIENT_SECRET"]
        }
        "discord" => &[
            "DISCORD_CLIENT_ID",
            "DISCORD_CLIENT_SECRET",
            "DISCORD_BOT_TOKEN",
        ],
        "telegram-bot" => &["TELEGRAM_BOT_TOKENS"],
        "telegram-user" => &[
            "TELEGRAM_API_ID",
            "TELEGRAM_API_HASH",
            "TELEGRAM_SESSION_DIR",
        ],
        "pinterest" => &["PINTEREST_CLIENT_ID", "PINTEREST_CLIENT_SECRET"],
        "tiktok" => &["TIKTOK_CLIENT_ID", "TIKTOK_CLIENT_SECRET"],
        "mastodon" => &[
            "MASTODON_CLIENT_ID",
            "MASTODON_CLIENT_SECRET",
            "MASTODON_INSTANCE_URL",
        ],
        "medium" => &["MEDIUM_ACCESS_TOKEN"],
        "devto" => &["DEVTO_API_KEY"],
        "hashnode" => &["HASHNODE_API_KEY"],
        "github" => &["GITHUB_TOKEN"],
        "whatsapp" => &["WHATSAPP_STORE_DIR"],
        "slack" => &["SLACK_CLIENT_ID", "SLACK_CLIENT_SECRET"],
        _ => &[],
    }
}

/// Resolution order is DB row → browser extraction → env, so a connected
/// account with env vars also set reports `db`: the token actually used is
/// the one in the row.
fn credential_source(provider_id: &str, db_rows: usize, config: &Config) -> &'static str {
    if db_rows > 0 {
        "db"
    } else if config.provider_credentials(provider_id).is_some() {
        "env"
    } else {
        "none"
    }
}

/// Pre-check stored media against APP_URL reachability.
///
/// Fails fast, and structurally: no server is started and no request is made.
/// The failure this exists to catch — a platform that cannot fetch
/// `http://localhost:6543/api/media/<id>` — is decided by the URL shape, not
/// by whether anything answers right now, so a network probe would only add
/// a way to be wrong.
async fn media_preflight(
    state: &AppState,
    user_id: Uuid,
    integrations: &[Integration],
) -> serde_json::Value {
    let app_url = state.config.app_url.trim_end_matches('/').to_string();
    let app_host = url::Url::parse(&app_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    // An unparseable APP_URL is treated as local: that is the safe direction,
    // since it must not be reported as publicly reachable.
    let app_public = app_host
        .as_deref()
        .is_some_and(|h| !crate::social::is_host_local(h));

    let fetchers: Vec<&str> = integrations
        .iter()
        .map(|i| i.provider_identifier.as_str())
        .filter(|p| crate::social::fetches_media_server_side(p))
        .collect();
    let media_count = crate::db::queries::list_media(&state.db, user_id, 200, 0, None)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut problems: Vec<serde_json::Value> = Vec::new();
    if media_count > 0 && !fetchers.is_empty() {
        if !app_public {
            problems.push(serde_json::json!({
                "kind": "app_url_not_public",
                "detail": format!(
                    "{media_count} media object(s) stored and {} fetch media server-side, but \
                     APP_URL is `{app_url}` — a host-local address the platform cannot reach",
                    fetchers.join(", ")
                ),
                "fix": "Set APP_URL to a publicly reachable https:// URL (tunnel or reverse \
                        proxy) and restart; media URLs are built as {APP_URL}/api/media/{id}",
            }));
        }
        problems.push(serde_json::json!({
            "kind": "relative_media_url",
            "detail": format!(
                "stored media is served as the relative path `/api/media/{{id}}`; server-side \
                 fetchers receive it verbatim and cannot resolve it against {app_url}"
            ),
            "fix": "pass absolute https:// URLs in the post's media array, or serve uploads from a \
                    public host",
        }));
    }

    let ok = problems.is_empty();
    serde_json::json!({
        "status": if ok { "ok" } else { "unreachable" },
        "ok": ok,
        "checked_at": checked_at(),
        "app_url": app_url,
        "app_host": app_host,
        "app_url_public": app_public,
        "media_objects": media_count,
        "server_fetch_providers": fetchers,
        "problems": problems,
    })
}

async fn handle_providers_with_state(state: &AppState) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let db_counts: std::collections::HashMap<&str, usize> = integrations.iter().fold(
        std::collections::HashMap::new(),
        |mut acc: std::collections::HashMap<&str, usize>, i| {
            *acc.entry(i.provider_identifier.as_str()).or_default() += 1;
            acc
        },
    );

    // `configured` was the only word this surface had, and it is the wrong
    // one: a row in `integrations` proves a form was filled in, not that the
    // token authenticates. Each row now separates what the build knows
    // (draft_ok / publish_capable) from what only a live call can prove
    // (live_verified), and this command makes no calls — so
    // `live_verified` is always false here with a reason, never guessed.
    let started_at = checked_at();
    let mut list: Vec<serde_json::Value> = Vec::new();
    for i in &integrations {
        let (draft_ok, publish_capable, reason) =
            readiness_flags(&state.providers, &i.provider_identifier);
        let mut row = serde_json::json!({
            "provider": i.provider_identifier,
            "name": i.profile_name,
            "internal_id": i.internal_id,
            "disabled": i.disabled,
            "draft_ok": draft_ok,
            "publish_capable": publish_capable,
            "live_verified": false,
            "live_verified_at": serde_json::Value::Null,
            "checked_at": started_at,
            "credential_source": credential_source(
                &i.provider_identifier,
                db_counts.get(i.provider_identifier.as_str()).copied().unwrap_or(0),
                &state.config,
            ),
        });
        if let Some(reason) = reason {
            row["reason"] = serde_json::Value::String(reason);
        }
        list.push(row);
    }
    let active = list.iter().filter(|p| p["disabled"] != true).count();
    let disabled = list.len() - active;
    let draft_ok = list.iter().filter(|p| p["draft_ok"] == true).count();
    let publish_capable = list.iter().filter(|p| p["publish_capable"] == true).count();
    // AXI §5: Definitive empty state. An empty list here means "no account is
    // connected", which is the single most common reason a publish silently
    // does nothing — so it is a warning, not just a message.
    if list.is_empty() {
        output_json(&serde_json::json!({
            "providers": [],
            "count": 0,
            "draft_ok": 0,
            "publish_capable": 0,
            "live_verified": 0,
            "warnings": ["No provider is connected: a staged post has nowhere to go. \
                          Run 'switchpost connect <provider>' or visit /setup."],
            "message": "No providers connected. Run 'switchpost connect <provider>' or visit /setup to get started.",
            "help": "Run 'switchpost connect --help' to see all supported providers.",
        }));
    } else {
        let mut warnings: Vec<String> = Vec::new();
        if publish_capable < list.len() {
            warnings.push(format!(
                "{}/{} connected account(s) have no post-publish surface — drafts stored \
                 against them will never go live.",
                list.len() - publish_capable,
                list.len()
            ));
        }
        warnings.push(
            "live_verified is false for every row: this command makes no platform calls. \
             Run 'switchpost doctor' for an authenticated round-trip."
                .into(),
        );
        // AXI §4: Pre-computed aggregates
        output_json(&serde_json::json!({
            "count": list.len(),
            "active": active,
            "disabled": disabled,
            "draft_ok": draft_ok,
            "publish_capable": publish_capable,
            "live_verified": 0,
            "providers": list,
            "warnings": warnings,
        }));
    }
    Ok(())
}

fn handle_init() -> anyhow::Result<()> {
    let dir = crate::config::config_dir();
    std::fs::create_dir_all(&dir)?;
    let env_path = dir.join(".env");
    if env_path.exists() {
        output_json(&serde_json::json!({
            "status": "exists",
            "path": env_path.display().to_string(),
            "message": "Config already exists. Edit it with your preferred editor."
        }));
    } else {
        let template = r#"# ─── Switchpost — created by `switchpost init` ───────────
# The binary reads $CWD/.env, then ~/.switchpost/.env, and
# `switchpost config set` writes to this file. Keep it as the single
# source of truth — every name below is one the binary actually reads.

# ── Core ───────────────────────────────────────────────────────
# SQLite only. The file is created and migrated on first run.
DATABASE_URL=sqlite://data/switchpost.db?mode=rwc

# Single-user WebUI password. Auto-generated and persisted here on first
# run when unset. Re-roll it with: switchpost config reset-password
# APP_PASSWORD=

# Public URL of this instance, used for OAuth redirect URIs
# ({APP_URL}/api/auth/callback). Instagram-Standalone and Threads require
# https:// — the server self-signs a cert when APP_URL starts with https.
APP_URL=https://localhost:6543

# CORS origin + CSRF allow-list. Defaults to APP_URL.
# FRONTEND_URL=https://localhost:6543

# Network interface to bind: 127.0.0.1 (default, loopback only) or 0.0.0.0.
# BIND_HOST=127.0.0.1

# Session cookie / OAuth state secret. Derived from APP_PASSWORD if unset.
# JWT_SECRET=

# 64 hex chars (32 bytes) — encrypts provider tokens at rest.
# TOKEN_ENCRYPTION_KEY=

# Embedded web UI: SERVE_FRONTEND=false runs API-only, FRONTEND_DIR serves
# a SvelteKit build from disk (dev).
# SERVE_FRONTEND=true
# FRONTEND_DIR=./frontend/build

# Media uploads directory.
# MEDIA_DIR=./uploads

# ── X / Twitter ────────────────────────────────────────────────
# Cookie auth (recommended — unlocks the GraphQL API)
# X_AUTH_TOKEN=
# X_CT0=
# OAuth (API v2 only)
# X_CLIENT_ID=
# X_CLIENT_SECRET=

# ── Reddit ────────────────────────────────────────────────────
# Cookie auth is auto-imported from your browser (`switchpost connect reddit`)
# REDDIT_CLIENT_ID=
# REDDIT_CLIENT_SECRET=
# REDDIT_USERNAME=
# REDDIT_PASSWORD=
# REDDIT_ACCESS_TOKEN=
# REDDIT_REFRESH_TOKEN=

# ── LinkedIn (personal + company pages) ───────────────────────
# LINKEDIN_CLIENT_ID=
# LINKEDIN_CLIENT_SECRET=

# ── Facebook ──────────────────────────────────────────────────
# FACEBOOK_CLIENT_ID=
# FACEBOOK_CLIENT_SECRET=

# ── Instagram ─────────────────────────────────────────────────
# Graph API provider (`instagram`) reads the CLIENT_ID pair — normally the
# same Meta app id/secret as FACEBOOK_CLIENT_ID / FACEBOOK_CLIENT_SECRET.
# INSTAGRAM_CLIENT_ID=
# INSTAGRAM_CLIENT_SECRET=
# Standalone provider (`instagram-standalone`) reads the APP_ID pair.
# INSTAGRAM_APP_ID=
# INSTAGRAM_APP_SECRET=

# ── Threads ───────────────────────────────────────────────────
# THREADS_APP_ID=
# THREADS_APP_SECRET=

# ── YouTube / Google (Gmail, Calendar, Drive) ─────────────────
# YOUTUBE_CLIENT_ID=
# YOUTUBE_CLIENT_SECRET=

# ── Bluesky ───────────────────────────────────────────────────
# BLUESKY_HANDLE=
# BLUESKY_APP_PASSWORD=

# ── Discord ───────────────────────────────────────────────────
# DISCORD_CLIENT_ID=
# DISCORD_CLIENT_SECRET=
# DISCORD_BOT_TOKEN=

# ── Slack ─────────────────────────────────────────────────────
# SLACK_CLIENT_ID=
# SLACK_CLIENT_SECRET=

# ── Telegram ──────────────────────────────────────────────────
# Bot API (comma-separated tokens for multi-bot)
# TELEGRAM_BOT_TOKENS=
# User client (MTProto — personal account automation)
# TELEGRAM_API_ID=
# TELEGRAM_API_HASH=
# TELEGRAM_SESSION_DIR=./data/telegram

# ── TikTok ────────────────────────────────────────────────────
# TIKTOK_CLIENT_ID=
# TIKTOK_CLIENT_SECRET=

# ── Pinterest ─────────────────────────────────────────────────
# PINTEREST_CLIENT_ID=
# PINTEREST_CLIENT_SECRET=

# ── Mastodon ──────────────────────────────────────────────────
# MASTODON_CLIENT_ID=
# MASTODON_CLIENT_SECRET=
# MASTODON_INSTANCE_URL=

# ── API-key providers ─────────────────────────────────────────
# MEDIUM_ACCESS_TOKEN=
# DEVTO_API_KEY=
# HASHNODE_API_KEY=
# GITHUB_TOKEN=

# ── WhatsApp (native client — pair by QR in /setup) ───────────
# WHATSAPP_STORE_DIR=./data/whatsapp

# ── Farcaster (Tier-3 archive — off unless the flag is set) ───
# ENABLE_ARCHIVE_PROVIDERS=1
# NEYNAR_API_KEY=

# ── Automation / housekeeping ─────────────────────────────────
# Auto-reply and content generation need an OpenAI-compatible endpoint.
# LLM_ENDPOINT=
# LLM_MODEL=
# Shorten links in post bodies via Dub.co, and strip them from X posts.
# DUB_CO_API_KEY=
# DUB_CO_WORKSPACE=
# STRIP_LINKS_FROM_X=false
# FEED_REFRESH_INTERVAL_SECONDS=900
# EVENTS_LOG_RETENTION_DAYS=30
"#;
        std::fs::write(&env_path, template)?;
        output_json(&serde_json::json!({
            "status": "created",
            "path": env_path.display().to_string(),
            "message": "Config created. DATABASE_URL is set; add APP_PASSWORD and platform credentials as needed.",
            "help": "Set a value any time with 'switchpost config set KEY VALUE'. Never commit this file."
        }));
    }
    Ok(())
}

/// An integration row that can actually authenticate: enabled and holding a
/// credential. Placeholder rows with an empty `access_token` (and any row the
/// user disabled) must NOT make `connect` report `already_connected` — that
/// blocks reconnect forever with no way out short of SQL.
///
/// A SEED token is not a credential either (AGENTS §0.5: seed_demo.py rows
/// are fixtures, not accounts). Recognizing them here lets `connect x`
/// replace the stub with the real browser/env cookies instead of answering
/// `already_connected` for a row whose token is `{"auth_token":"seed"}`.
pub(crate) fn is_credential_row(i: &crate::db::models::Integration) -> bool {
    if i.disabled || i.access_token.is_empty() {
        return false;
    }
    let t = i.access_token.as_str();
    if t == "seed-token" || t.contains("\"seed\"") {
        return false;
    }
    true
}

/// The --wait login loop for AI agents (Command::Connect --wait).
///
/// 1. Run the one-shot connect: for cookie providers (x/reddit) this
///    IS the whole flow — it imports, validates and returns.
/// 2. For OAuth/one-time-token providers the one-shot emits an auth_url
///    (or the /connect code instructions). This loop then polls the
///    integration list every 2s until the provider gains a REAL
///    credential row (seed tokens do not count — `is_credential_row`),
///    or the timeout elapses.
///
/// Agent loop, end to end:
///   switchpost connect linkedin --wait --pretty
///   -> read the auth_url from stdout
///   -> drive a browser to it, complete the platform's login
///   -> the callback creates the integration row
///   -> this command prints `status: connected` and exits 0.
async fn handle_connect_wait(
    state: &AppState,
    provider: &str,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    use std::time::{Duration, Instant};

    let user_id = resolve_user(state).await?;

    let started = Instant::now();

    // Step 1: report the CURRENT state before asking for anything.
    let existing: Vec<_> = crate::db::queries::list_integrations(&state.db, user_id)
        .await?
        .into_iter()
        .filter(|i| i.provider_identifier == provider && is_credential_row(i))
        .collect();
    if !existing.is_empty() {
        output_json(&serde_json::json!({
            "status": "already_connected",
            "provider": provider,
            "count": existing.len(),
            "accounts": existing.iter().map(|i| serde_json::json!({
                "name": i.profile_name,
                "internal_id": i.internal_id,
            })).collect::<Vec<_>>(),
            "hint": "Already connected.",
        }));
        return Ok(());
    }

    // Step 2: hand the agent the NEXT ACTION.
    //
    // For an OAuth provider that is the provider's own authorization URL —
    // minted here, so the agent gets it NOW and can drive its browser
    // straight to the platform's consent page. (The one-shot connect
    // swallowed its output before, so `connect --wait` printed no URL at
    // all and only revealed the problem at timeout.)
    //
    // For a cookie/credential provider the one-shot connect IS the action:
    // it imports from the browser or the environment.
    let oauth_provider = state
        .providers
        .get(provider)
        .map(|p| p.uses_oauth())
        .unwrap_or(false);
    if oauth_provider {
        match oauth_auth_url_with_redirect(state, provider).await {
            Ok((url, redirect_uri)) => {
                // ─── THE PROVIDER'S OWN PREFLIGHT ──────────────────
                // The same question the REST route and the MCP tool ask,
                // for the same reason: Meta's Threads consent gate
                // redirects the BROWSER to its own page and never comes
                // back to our callback, so the failure is invisible to a
                // shell user too. The agent driving the CLI needs the
                // reason to tell the founder what to fix.
                let warning = match state.providers.get(provider) {
                    Some(p) => p.connect_preflight().await,
                    None => None,
                };
                let mut payload = serde_json::json!({
                    "status": "waiting_for_oauth",
                    "provider": provider,
                    "method": "oauth",
                    "auth_url": url,
                    "redirect_uri": redirect_uri,
                    "hint": "Open the auth_url in a browser, complete the login, and this command will report the connection. If the platform says redirect_uri_mismatch, register the redirect_uri above in the provider's developer console.",
                });
                if let Some(w) = warning {
                    payload["warning"] = serde_json::Value::String(w);
                }
                output_json(&payload);
            }
            Err(e) => {
                // Not configured, or the provider refused to build a URL —
                // let the one-shot print the actionable diagnostic.
                let _ = handle_connect_with_state(state, provider).await;
                output_json(&serde_json::json!({
                    "status": "error",
                    "provider": provider,
                    "error": format!("{e}"),
                    "hint": "Check the provider's credentials in ~/.switchpost/.env, then run this command again.",
                }));
                return Ok(());
            }
        }
    } else {
        let _ = handle_connect_with_state(state, provider).await;
    }

    // Step 3: did the import land already?
    if crate::db::queries::list_integrations(&state.db, user_id)
        .await?
        .iter()
        .any(|i| i.provider_identifier == provider && is_credential_row(i))
    {
        return Ok(());
    }

    // Step 4: poll until the credential row appears or the timeout hits.
    let deadline = started + Duration::from_secs(timeout_secs);
    loop {
        if Instant::now() >= deadline {
            output_json(&serde_json::json!({
                "status": "timeout",
                "provider": provider,
                "waited_secs": started.elapsed().as_secs(),
                "hint": "No credential landed before --timeout-secs. Open the auth_url (printed above) in a browser and complete the login, then run again.",
            }));
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let connected: Vec<_> = crate::db::queries::list_integrations(&state.db, user_id)
            .await?
            .into_iter()
            .filter(|i| i.provider_identifier == provider && is_credential_row(i))
            .collect();
        if !connected.is_empty() {
            output_json(&serde_json::json!({
                "status": "connected",
                "provider": provider,
                "waited_secs": started.elapsed().as_secs(),
                "accounts": connected.iter().map(|i| serde_json::json!({
                    "name": i.profile_name,
                    "internal_id": i.internal_id,
                    "type": i.provider_identifier,
                })).collect::<Vec<_>>(),
            }));
            return Ok(());
        }
    }
}

/// Mint a provider-NATIVE OAuth authorization URL for an agent-driven login.
///
/// The old shape emitted `{app_url}/api/public/connect/{provider}`, a SERVER
/// route gated by the browser's `sf_session` cookie (or a `?token=` JWT).
/// An agent driving its own browser hits a 401 dead end there, because the
/// agent's browser session is not the founder's web session.
///
/// This mints the same URL the web UI mints — code_verifier + state +
/// `oauth_states` row + the provider's own `generate_auth_url` — so the
/// browser goes straight to the platform's consent page and the provider
/// redirects back to `{app_url}/api/auth/callback`, where the server
/// exchanges the code and writes the integration row that `connect --wait`
/// polls for. No cookie, no JWT, no popup required.
/// The redirect URI every OAuth provider's app must have registered.
fn oauth_redirect_uri_for(app_url: &str) -> String {
    format!("{}/api/auth/callback", app_url.trim_end_matches('/'))
}

fn oauth_redirect_uri(state: &AppState) -> String {
    oauth_redirect_uri_for(&state.config.app_url)
}

async fn oauth_auth_url(state: &AppState, provider: &str) -> anyhow::Result<String> {
    Ok(oauth_auth_url_with_redirect(state, provider).await?.0)
}

/// The same URL, plus the `redirect_uri` the provider's developer console
/// must have registered.
///
/// Every OAuth provider rejects the request with `redirect_uri_mismatch`
/// when its app does not list this exact URI, and the operator cannot see
/// the URI in the provider's error page — they have to read it out of the
/// URL we generated. Handing it over turns a dead end into one setting.
async fn oauth_auth_url_with_redirect(
    state: &AppState,
    provider: &str,
) -> anyhow::Result<(String, String)> {
    // The SERVICE owns the OAuth initiation, including how the state's
    // redirect_uri column is packed ("<user_id>:<url>"). The CLI used to
    // re-implement the flow and stored the bare URL, so every
    // CLI-started OAuth flow died at the callback with "Invalid user ID"
    // and the code exchange never ran. One implementation, one rule.
    let user_id = resolve_user(state).await?;
    let result = crate::services::integrations::IntegrationService::initiate_connect(
        &state.db,
        &state.providers,
        user_id,
        provider,
        state.config.app_url.trim_end_matches('/'),
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to build {provider} authorization URL: {e}"))?;
    Ok((result.auth_url, result.redirect_uri))
}

async fn handle_connect_with_state(state: &AppState, provider: &str) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;

    match provider {
        // ── X/Twitter: auto-import from browser ────────────────
        "x" => {
            // Check if already connected
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "x" && is_credential_row(i))
            {
                let existing: Vec<_> = integrations
                    .iter()
                    .filter(|i| i.provider_identifier == "x" && is_credential_row(i))
                    .collect();
                output_json(&serde_json::json!({
                    "status": "already_connected",
                    "provider": "x",
                    "count": existing.len(),
                    "accounts": existing.iter().map(|i| serde_json::json!({
                        "name": i.profile_name,
                        "internal_id": i.internal_id,
                    })).collect::<Vec<_>>(),
                    "hint": "Already connected. Use 'switchpost x timeline' to verify.",
                }));
                return Ok(());
            }

            // Try auto-import from the browser first, then env-var cookies
            // (X_AUTH_TOKEN + X_CT0). Both paths share the same get_me
            // validation — a credential that cannot authenticate never becomes
            // an integration row.
            let imported = crate::social::x_cookies::extract_x_cookies().map(|cookies| {
                (
                    crate::social::x_cookies::build_cookie_token(
                        &cookies.auth_token,
                        &cookies.ct0,
                        Some(&cookies.cookie_string),
                    ),
                    "browser-import".to_string(),
                    cookies.source,
                )
            });

            let (token_str, method, source) = match imported {
                Some(v) => v,
                None => {
                    let (at, ct0) = (&state.config.x_auth_token, &state.config.x_ct0);
                    match (at, ct0) {
                        (Some(at), Some(ct0)) if !at.is_empty() && !ct0.is_empty() => (
                            crate::social::x_cookies::build_cookie_token(at, ct0, None),
                            "env-cookies".to_string(),
                            "X_AUTH_TOKEN+X_CT0".to_string(),
                        ),
                        _ => {
                            output_json(&serde_json::json!({
                                "status": "no_browser_cookies",
                                "provider": "x",
                                "error": "No X/Twitter cookies found in any browser or .env.",
                                "hints": [
                                    "Log into x.com in Chrome, Brave, Firefox, or Zen browser",
                                    "Then run 'switchpost connect x' again",
                                    "Or set X_AUTH_TOKEN + X_CT0 in ~/.switchpost/.env",
                                    "Or visit http://localhost:6543/api/public/connect/x-cookies for manual entry",
                                ],
                            }));
                            return Ok(());
                        }
                    }
                }
            };

            // Validate by calling get_me
            {
                let mut provider_obj = crate::social::x::XProvider::new(&state.config);
                provider_obj.prepare_from_token(&token_str);
                match provider_obj.get_me(&token_str).await {
                    Ok(json) => {
                        let data = json.get("data");
                        let name = data
                            .and_then(|d| d.get("name"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("X User");
                        let username = data
                            .and_then(|d| d.get("username"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("");
                        let avatar = data
                            .and_then(|d| d.get("profile_image_url"))
                            .and_then(|s| s.as_str());
                        let id = data
                            .and_then(|d| d.get("id"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();

                        crate::db::queries::create_integration(
                            &state.db,
                            user_id,
                            "x",
                            "X (Twitter)",
                            &id,
                            &token_str,
                            None,
                            None,
                            Some(name),
                            None,
                            avatar,
                            None,
                            Some("cookie"), // §6: explicit auth_mode
                        )
                        .await?;
                        // Drop the seed_demo fixture row (if any) so the
                        // channel list shows the real account, not the
                        // fictional seed one beside it.
                        let seeds =
                            crate::db::queries::delete_seed_integrations(&state.db, user_id, "x")
                                .await
                                .unwrap_or(0);

                        output_json(&serde_json::json!({
                            "status": "connected",
                            "provider": "x",
                            "method": method,
                            "source": source,
                            "name": name,
                            "username": username,
                            "id": id,
                            "seed_rows_replaced": seeds,
                        }));
                    }
                    Err(e) => {
                        output_json(&serde_json::json!({
                            "status": "error",
                            "provider": "x",
                            "error": format!("Cookie import succeeded but validation failed: {e}. The cookies may be expired."),
                            "hint": "Log into x.com in your browser (or refresh X_AUTH_TOKEN + X_CT0 in .env), then run this command again.",
                        }));
                    }
                }
            }
        }

        // ── Reddit: auto-import from browser ──────────────────
        "reddit" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "reddit" && is_credential_row(i))
            {
                let existing: Vec<_> = integrations
                    .iter()
                    .filter(|i| i.provider_identifier == "reddit" && is_credential_row(i))
                    .collect();
                output_json(&serde_json::json!({
                    "status": "already_connected",
                    "provider": "reddit",
                    "count": existing.len(),
                    "accounts": existing.iter().map(|i| serde_json::json!({
                        "name": i.profile_name,
                        "internal_id": i.internal_id,
                    })).collect::<Vec<_>>(),
                    "hint": "Already connected. Use 'switchpost reddit browse rust' to verify.",
                }));
                return Ok(());
            }

            // Browser cookie import first, then the REDDIT_COOKIE_TOKEN env
            // var (the same JSON blob the manual paste flow accepts:
            // {reddit_session, token_v2?, cookie_string?}). Both paths
            // share the same get_www validation — a credential that cannot
            // authenticate never becomes an integration row.
            let env_token = state
                .config
                .reddit_cookie_token
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(|t| {
                    (
                        t.to_string(),
                        "env-cookies".to_string(),
                        "REDDIT_COOKIE_TOKEN".to_string(),
                    )
                });
            let imported = crate::social::reddit_cookies::extract_reddit_cookies().map(|cookies| {
                (
                    crate::social::reddit_cookies::build_cookie_token(
                        &cookies.reddit_session,
                        cookies.token_v2.as_deref(),
                        Some(&cookies.cookie_string),
                    ),
                    "browser-import".to_string(),
                    cookies.source,
                )
            });
            let imported = imported.or(env_token);
            match imported {
                Some((token_str, method, source)) => {
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

                            crate::db::queries::create_integration(
                                &state.db,
                                user_id,
                                "reddit",
                                "Reddit",
                                &id,
                                &token_str,
                                None,
                                None,
                                Some(&name),
                                None,
                                icon.as_deref(),
                                None,
                                Some("cookie"), // §6: explicit auth_mode
                            )
                            .await?;
                            // Drop the seed_demo fixture row (if any) — see
                            // the X branch for why.
                            let seeds = crate::db::queries::delete_seed_integrations(
                                &state.db, user_id, "reddit",
                            )
                            .await
                            .unwrap_or(0);

                            output_json(&serde_json::json!({
                                "status": "connected",
                                "provider": "reddit",
                                "method": method,
                                "source": source,
                                "name": name,
                                "id": id,
                                "seed_rows_replaced": seeds,
                            }));
                        }
                        Err(e) => {
                            output_json(&serde_json::json!({
                                "status": "error",
                                "provider": "reddit",
                                "error": format!("Cookie import succeeded but validation failed: {e}. The cookies may be expired."),
                                "hint": "Log into reddit.com in your browser (or refresh REDDIT_COOKIE_TOKEN in .env), then run this command again.",
                            }));
                        }
                    }
                }
                None => {
                    output_json(&serde_json::json!({
                        "status": "no_browser_cookies",
                        "provider": "reddit",
                        "error": "No Reddit cookies found in any browser or .env.",
                        "hints": [
                            "Log into reddit.com in Chrome, Brave, Firefox, or Zen browser",
                            "Then run 'switchpost connect reddit' again",
                            "Or set REDDIT_COOKIE_TOKEN in ~/.switchpost/.env",
                            "Or visit http://localhost:6543/api/public/connect/reddit-cookies for manual entry",
                        ],
                    }));
                }
            }
        }

        // ── OAuth providers: check status and provide URL ──────
        "linkedin" | "linkedin-page" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            let connected: Vec<_> = integrations
                .iter()
                .filter(|i| {
                    (i.provider_identifier == "linkedin"
                        || i.provider_identifier == "linkedin-page")
                        && is_credential_row(i)
                })
                .collect();
            // An auth_url without configured client credentials 400s on
            // /api/public/connect — an agent following it hits a dead end.
            // Report the missing env vars instead, like the other OAuth arms.
            let configured = state.config.provider_credentials("linkedin").is_some();
            let (auth_url, redirect_uri) = if configured {
                let (u, r) = oauth_auth_url_with_redirect(state, provider).await?;
                (Some(u), Some(r))
            } else {
                (None, None)
            };
            output_json(&serde_json::json!({
                "status": if !connected.is_empty() { "already_connected" }
                    else if configured { "not_connected" }
                    else { "not_configured" },
                "provider": provider,
                "count": connected.len(),
                "accounts": connected.iter().map(|i| serde_json::json!({
                    "name": i.profile_name,
                    "internal_id": i.internal_id,
                    "type": i.provider_identifier,
                })).collect::<Vec<_>>(),
                "method": "oauth",
                "auth_url": auth_url.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
                "redirect_uri": redirect_uri.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
                "requires": if configured { serde_json::Value::Null } else { serde_json::json!(["LINKEDIN_CLIENT_ID", "LINKEDIN_CLIENT_SECRET"]) },
                "hint": if connected.is_empty() && configured {
                    "Open the auth_url in a browser to complete OAuth authorization. If the platform says redirect_uri_mismatch, register the redirect_uri above in the provider's developer console."
                } else if connected.is_empty() {
                    "Set LINKEDIN_CLIENT_ID + LINKEDIN_CLIENT_SECRET in ~/.switchpost/.env, restart the server, then connect again."
                } else {
                    "Already connected."
                },
            }));
        }
        "facebook" | "instagram" | "instagram-standalone" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            let connected: Vec<_> = integrations
                .iter()
                .filter(|i| i.provider_identifier == provider && is_credential_row(i))
                .collect();
            // Same gate as linkedin: no client creds, no auth_url.
            let configured = state.config.provider_credentials(provider).is_some();
            let (auth_url, redirect_uri) = if configured {
                let (u, r) = oauth_auth_url_with_redirect(state, provider).await?;
                (Some(u), Some(r))
            } else {
                (None, None)
            };
            output_json(&serde_json::json!({
                "status": if !connected.is_empty() { "already_connected" }
                    else if configured { "not_connected" }
                    else { "not_configured" },
                "provider": provider,
                "count": connected.len(),
                "accounts": connected.iter().map(|i| serde_json::json!({
                    "name": i.profile_name,
                    "internal_id": i.internal_id,
                })).collect::<Vec<_>>(),
                "method": "oauth",
                "auth_url": auth_url.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
                "redirect_uri": redirect_uri.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
                "requires": if configured { serde_json::Value::Null } else { serde_json::json!(["FACEBOOK_CLIENT_ID", "FACEBOOK_CLIENT_SECRET"]) },
                "hint": if connected.is_empty() && configured {
                    "Open the auth_url in a browser to complete OAuth authorization. If the platform says redirect_uri_mismatch, register the redirect_uri above in the provider's developer console."
                } else if connected.is_empty() {
                    "Set FACEBOOK_CLIENT_ID + FACEBOOK_CLIENT_SECRET in ~/.switchpost/.env, restart, then connect again."
                } else {
                    "Already connected."
                },
            }));
        }

        // ── Direct-connect providers ──────────────────────────
        // ── Env-var credential providers ─────────────────────
        "bluesky" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "bluesky" && is_credential_row(i))
            {
                let existing: Vec<_> = integrations
                    .iter()
                    .filter(|i| i.provider_identifier == "bluesky" && is_credential_row(i))
                    .collect();
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "bluesky", "count": existing.len()}),
                );
            } else if let (Some(handle), Some(app_pw)) = (
                state
                    .config
                    .bluesky_handle
                    .as_deref()
                    .filter(|h| !h.is_empty()),
                state
                    .config
                    .bluesky_app_password
                    .as_deref()
                    .filter(|p| !p.is_empty()),
            ) {
                // Eager connect: create a session with the app password and
                // store the accessJwt + did as the integration row — the
                // MCP/CLI tools read the row's access_token as the SESSION
                // JWT (they do not fall back to env config), so a real
                // session is what makes them work.
                let client = reqwest::Client::new();
                let resp = client
                    .post("https://bsky.social/xrpc/com.atproto.server.createSession")
                    .json(&serde_json::json!({
                        "identifier": handle,
                        "password": app_pw,
                    }))
                    .send()
                    .await;
                match resp {
                    Ok(r) if r.status().as_u16() == 200 => {
                        match r.json::<serde_json::Value>().await {
                            Ok(session) => {
                                let jwt = session["accessJwt"].as_str().unwrap_or("");
                                let did = session["did"].as_str().unwrap_or("").to_string();
                                let handle_owned =
                                    session["handle"].as_str().unwrap_or(handle).to_string();
                                let display = session["email"]
                                    .as_str()
                                    .or(session["handle"].as_str())
                                    .unwrap_or(handle);
                                if jwt.is_empty() || did.is_empty() {
                                    output_json(&serde_json::json!({
                                        "status": "error",
                                        "provider": "bluesky",
                                        "error": "createSession returned no accessJwt/did",
                                        "hint": "Check BLUESKY_HANDLE + BLUESKY_APP_PASSWORD in ~/.switchpost/.env",
                                    }));
                                } else {
                                    crate::db::queries::create_integration(
                                        &state.db,
                                        user_id,
                                        "bluesky",
                                        "Bluesky",
                                        &did,
                                        jwt,
                                        None,
                                        None,
                                        Some(display),
                                        None,
                                        None,
                                        None,
                                        Some("app_password"), // §6: explicit auth_mode
                                    )
                                    .await?;
                                    // Bluesky sessions are short-lived;
                                    // refresh_token re-creates one from the
                                    // env BLUESKY_HANDLE/APP_PASSWORD, so no
                                    // extra meta is stored here.
                                    let seeds = crate::db::queries::delete_seed_integrations(
                                        &state.db, user_id, "bluesky",
                                    )
                                    .await
                                    .unwrap_or(0);
                                    output_json(&serde_json::json!({
                                        "status": "connected",
                                        "provider": "bluesky",
                                        "method": "env-app-password",
                                        "source": "BLUESKY_HANDLE+BLUESKY_APP_PASSWORD",
                                        "name": display,
                                        "handle": handle_owned,
                                        "id": did,
                                        "seed_rows_replaced": seeds,
                                    }));
                                }
                            }
                            Err(e) => {
                                output_json(&serde_json::json!({
                                    "status": "error",
                                    "provider": "bluesky",
                                    "error": format!("createSession response unreadable: {e}"),
                                }));
                            }
                        }
                    }
                    Ok(r) => {
                        let status = r.status().as_u16();
                        let body = r.text().await.unwrap_or_default();
                        output_json(&serde_json::json!({
                            "status": "error",
                            "provider": "bluesky",
                            "error": format!("createSession HTTP {status}: {}", body.chars().take(200).collect::<String>()),
                            "hint": "Check BLUESKY_HANDLE + BLUESKY_APP_PASSWORD (an app password, not the account password) in ~/.switchpost/.env",
                        }));
                    }
                    Err(e) => {
                        output_json(&serde_json::json!({
                            "status": "error",
                            "provider": "bluesky",
                            "error": format!("createSession request failed: {e}"),
                        }));
                    }
                }
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "bluesky", "requires": ["BLUESKY_HANDLE", "BLUESKY_APP_PASSWORD"], "hint": "Set these in ~/.switchpost/.env. Get an app password at Bluesky Settings > Advanced > App Passwords."}),
                );
            }
        }
        "github" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "github" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "github"}),
                );
            } else if let Some(token) = state
                .config
                .github_token
                .as_deref()
                .filter(|t| !t.is_empty())
            {
                // Eager connect: validate the PAT against the live API and
                // create the row NOW, so 'connect github' is a complete
                // step for an agent (not a promise that something will
                // happen "on first use").
                let provider_obj = crate::social::github::GithubProvider::new(&state.config);
                match provider_obj.get_authenticated_user(token).await {
                    Ok(user) => {
                        let login = user["login"].as_str().unwrap_or("github-user");
                        let id = user["id"]
                            .as_i64()
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let avatar = user["avatar_url"].as_str();
                        let name = user["name"].as_str().unwrap_or(login);
                        crate::db::queries::create_integration(
                            &state.db,
                            user_id,
                            "github",
                            "GitHub",
                            &id,
                            token,
                            None,
                            None,
                            Some(name),
                            None,
                            avatar,
                            None,
                            Some("token"), // §6: explicit auth_mode
                        )
                        .await?;
                        let seeds = crate::db::queries::delete_seed_integrations(
                            &state.db, user_id, "github",
                        )
                        .await
                        .unwrap_or(0);
                        output_json(&serde_json::json!({
                            "status": "connected",
                            "provider": "github",
                            "method": "env-token",
                            "source": "GITHUB_TOKEN",
                            "name": name,
                            "username": login,
                            "id": id,
                            "seed_rows_replaced": seeds,
                        }));
                    }
                    Err(e) => {
                        output_json(&serde_json::json!({
                            "status": "error",
                            "provider": "github",
                            "error": format!("GITHUB_TOKEN validation failed: {e}"),
                            "hint": "Check the PAT at https://github.com/settings/tokens and refresh GITHUB_TOKEN in ~/.switchpost/.env",
                        }));
                    }
                }
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "github", "requires": ["GITHUB_TOKEN"], "hint": "Create a PAT at https://github.com/settings/tokens and set GITHUB_TOKEN in ~/.switchpost/.env"}),
                );
            }
        }
        "telegram-bot" | "telegram" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "telegram-bot" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "telegram-bot"}),
                );
            } else if let Some(tokens) = state
                .config
                .telegram_bot_tokens
                .as_deref()
                .filter(|t| !t.is_empty())
            {
                // Eager connect: validate the FIRST bot token via getMe and
                // create the row now (the multi-bot registry still reads
                // TELEGRAM_BOT_TOKENS; the row makes the dashboard show it).
                let first = tokens.split(',').next().unwrap_or("").trim();
                let client = reqwest::Client::new();
                match client
                    .get(format!("https://api.telegram.org/bot{first}/getMe"))
                    .send()
                    .await
                {
                    Ok(r) => {
                        let status = r.status().as_u16();
                        let body = r.text().await.unwrap_or_default();
                        let ok = status == 200
                            && serde_json::from_str::<serde_json::Value>(&body)
                                .ok()
                                .and_then(|v| v["ok"].as_bool())
                                .unwrap_or(false);
                        if ok {
                            let me = serde_json::from_str::<serde_json::Value>(&body)
                                .ok()
                                .and_then(|v| v["result"].clone().into())
                                .unwrap_or_default();
                            let username = me["username"].as_str().unwrap_or("telegram-bot");
                            let id = me["id"].as_i64().map(|v| v.to_string()).unwrap_or_default();
                            crate::db::queries::create_integration(
                                &state.db,
                                user_id,
                                "telegram-bot",
                                "Telegram",
                                &id,
                                first,
                                None,
                                None,
                                Some(username),
                                None,
                                None,
                                None,
                                Some("bot_token"), // §6: explicit auth_mode
                            )
                            .await?;
                            let seeds = crate::db::queries::delete_seed_integrations(
                                &state.db,
                                user_id,
                                "telegram-bot",
                            )
                            .await
                            .unwrap_or(0);
                            output_json(&serde_json::json!({
                                "status": "connected",
                                "provider": "telegram-bot",
                                "method": "env-token",
                                "source": "TELEGRAM_BOT_TOKENS",
                                "name": username,
                                "id": id,
                                "seed_rows_replaced": seeds,
                            }));
                        } else {
                            output_json(&serde_json::json!({
                                "status": "error",
                                "provider": "telegram-bot",
                                "error": format!("getMe HTTP {status}: {}", body.chars().take(200).collect::<String>()),
                                "hint": "Check the bot token from @BotFather and refresh TELEGRAM_BOT_TOKENS in ~/.switchpost/.env",
                            }));
                        }
                    }
                    Err(e) => {
                        output_json(&serde_json::json!({
                            "status": "error",
                            "provider": "telegram-bot",
                            "error": format!("getMe request failed: {e}"),
                        }));
                    }
                }
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "telegram-bot", "requires": ["TELEGRAM_BOT_TOKENS"], "hint": "Message @BotFather on Telegram to create a bot, get the token, and set TELEGRAM_BOT_TOKENS in ~/.switchpost/.env"}),
                );
            }
        }
        "discord" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "discord" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "discord"}),
                );
            } else if state.config.discord_client_id.is_some()
                && state.config.discord_client_secret.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "discord", "method": "oauth", "auth_url": oauth_auth_url(state, "discord").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "discord", "requires": ["DISCORD_CLIENT_ID", "DISCORD_CLIENT_SECRET"], "hint": "Create an app at https://discord.com/developers/applications and set credentials in ~/.switchpost/.env"}),
                );
            }
        }
        "slack" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "slack" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "slack"}),
                );
            } else if state.config.slack_client_id.is_some()
                && state.config.slack_client_secret.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "slack", "method": "oauth", "auth_url": oauth_auth_url(state, "slack").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "slack", "requires": ["SLACK_CLIENT_ID", "SLACK_CLIENT_SECRET"], "hint": "Create an app at https://api.slack.com/apps and set credentials in ~/.switchpost/.env"}),
                );
            }
        }
        "pinterest" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "pinterest" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "pinterest"}),
                );
            } else if state.config.pinterest_client_id.is_some()
                && state.config.pinterest_client_secret.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "pinterest", "method": "oauth", "auth_url": oauth_auth_url(state, "pinterest").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "pinterest", "requires": ["PINTEREST_CLIENT_ID", "PINTEREST_CLIENT_SECRET"], "hint": "Create an app at https://developers.pinterest.com/apps/ and set credentials in ~/.switchpost/.env"}),
                );
            }
        }
        "tiktok" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "tiktok" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "tiktok"}),
                );
            } else if state.config.tiktok_client_id.is_some()
                && state.config.tiktok_client_secret.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "tiktok", "method": "oauth", "auth_url": oauth_auth_url(state, "tiktok").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "tiktok", "requires": ["TIKTOK_CLIENT_ID", "TIKTOK_CLIENT_SECRET"], "hint": "Create an app at https://developers.tiktok.com/ and set credentials in ~/.switchpost/.env"}),
                );
            }
        }
        "mastodon" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            if integrations
                .iter()
                .any(|i| i.provider_identifier == "mastodon" && is_credential_row(i))
            {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "mastodon"}),
                );
            } else if state.config.mastodon_client_id.is_some()
                && state.config.mastodon_client_secret.is_some()
                && state.config.mastodon_instance_url.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "mastodon", "method": "oauth", "auth_url": oauth_auth_url(state, "mastodon").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "mastodon", "requires": ["MASTODON_CLIENT_ID", "MASTODON_CLIENT_SECRET", "MASTODON_INSTANCE_URL"], "hint": "Register an app on your Mastodon instance and set credentials in ~/.switchpost/.env"}),
                );
            }
        }
        "youtube" | "google" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            let connected = integrations.iter().any(|i| {
                (i.provider_identifier == "youtube" || i.provider_identifier == "google")
                    && is_credential_row(i)
            });
            if connected {
                output_json(
                    &serde_json::json!({"status": "already_connected", "provider": "youtube"}),
                );
            } else if state.config.youtube_client_id.is_some()
                && state.config.youtube_client_secret.is_some()
            {
                output_json(
                    &serde_json::json!({"status": "not_connected", "provider": "youtube", "method": "oauth", "auth_url": oauth_auth_url(state, "youtube").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "youtube", "requires": ["YOUTUBE_CLIENT_ID", "YOUTUBE_CLIENT_SECRET"], "hint": "Create a Google Cloud project, enable YouTube Data API v3, create OAuth credentials, and set in ~/.switchpost/.env"}),
                );
            }
        }
        "medium" => {
            if state.config.medium_access_token.is_some() {
                output_json(
                    &serde_json::json!({"status": "configured", "provider": "medium", "method": "env_vars"}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "medium", "requires": ["MEDIUM_ACCESS_TOKEN"], "hint": "Get an integration token at https://medium.com/me/settings/security"}),
                );
            }
        }
        "devto" => {
            if state.config.devto_api_key.is_some() {
                output_json(
                    &serde_json::json!({"status": "configured", "provider": "devto", "method": "env_vars"}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "devto", "requires": ["DEVTO_API_KEY"], "hint": "Generate an API key at https://dev.to/settings/extensions"}),
                );
            }
        }
        "hashnode" => {
            if state.config.hashnode_api_key.is_some() {
                output_json(
                    &serde_json::json!({"status": "configured", "provider": "hashnode", "method": "env_vars"}),
                );
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "hashnode", "requires": ["HASHNODE_API_KEY"], "hint": "Generate a PAT at https://hashnode.com/settings/integrations"}),
                );
            }
        }
        "wordpress" => {
            output_json(
                &serde_json::json!({"status": "per_site", "provider": "wordpress", "hint": "WordPress uses per-site Application Passwords. Connect via the web UI with site URL, username, and app password."}),
            );
        }
        "skool" => {
            output_json(
                &serde_json::json!({"status": "chrome_extension", "provider": "skool", "hint": "Skool uses Chrome extension cookie extraction. Install the Skool Chrome extension, log into skool.com, and cookies are auto-extracted."}),
            );
        }
        "farcaster" => {
            output_json(
                &serde_json::json!({"status": "per_user", "provider": provider, "hint": "Farcaster is a Tier-3 archive provider (v25 §1) — set ENABLE_ARCHIVE_PROVIDERS=1 to register it. Uses per-user credentials stored in the integration record; connect via the web UI."}),
            );
        }
        "whatsapp" => {
            output_json(
                &serde_json::json!({"status": "native_client", "provider": "whatsapp", "hint": "WhatsApp uses a native client. Connect via the web UI to scan the QR code."}),
            );
        }
        "threads" => {
            if state.config.threads_app_id.is_some() && state.config.threads_app_secret.is_some() {
                // ─── THE PROVIDER'S OWN PREFLIGHT ──────────────────
                // The same question the REST route, the MCP tool and
                // `connect --wait` ask, for the same reason: Meta's
                // consent gate redirects the BROWSER to its own page and
                // never comes back to our callback, so a shell user sees
                // the dead end too. An agent driving this command needs
                // the reason to tell the founder what to fix.
                let warning = match state.providers.get(provider) {
                    Some(p) => p.connect_preflight().await,
                    None => None,
                };
                let mut payload = serde_json::json!({"status": "not_connected", "provider": "threads", "method": "oauth", "auth_url": oauth_auth_url(state, "threads").await?, "redirect_uri": oauth_redirect_uri(state), "hint": "Open the auth_url in a browser to complete OAuth authorization."});
                if let Some(w) = warning {
                    payload["warning"] = serde_json::Value::String(w);
                }
                output_json(&payload);
            } else {
                output_json(
                    &serde_json::json!({"status": "not_configured", "provider": "threads", "requires": ["THREADS_APP_ID", "THREADS_APP_SECRET"], "hint": "Threads uses Meta credentials. Set THREADS_APP_ID and THREADS_APP_SECRET in ~/.switchpost/.env"}),
                );
            }
        }

        "telegram-user" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            let connected: Vec<_> = integrations
                .iter()
                .filter(|i| i.provider_identifier == "telegram-user" && is_credential_row(i))
                .collect();
            if !connected.is_empty() {
                output_json(&serde_json::json!({
                    "status": "already_connected",
                    "provider": "telegram-user",
                    "count": connected.len(),
                    "accounts": connected.iter().map(|i| serde_json::json!({
                        "name": i.profile_name,
                        "internal_id": i.internal_id,
                    })).collect::<Vec<_>>(),
                    "hint": "Already connected.",
                }));
            } else if state.config.telegram_api_id.is_some()
                && state.config.telegram_api_hash.is_some()
                && state.config.telegram_session_dir.is_some()
            {
                // A user-session login needs the phone's QR scan, so the
                // action is a SCAN, not a URL. The CLI tells the agent
                // exactly that instead of claiming it will auto-connect.
                output_json(&serde_json::json!({
                    "status": "qr_login_required",
                    "provider": "telegram-user",
                    "method": "qr",
                    "requires": ["TELEGRAM_API_ID", "TELEGRAM_API_HASH", "TELEGRAM_SESSION_DIR"],
                    "hint": "Telegram user accounts log in by QR scan: run the server's Telegram user connect flow (web UI: Channels -> Telegram -> QR), or use the telegram_user_* CLI commands to log in interactively, then re-run this command.",
                }));
            } else {
                output_json(&serde_json::json!({
                    "status": "not_configured",
                    "provider": "telegram-user",
                    "requires": ["TELEGRAM_API_ID", "TELEGRAM_API_HASH", "TELEGRAM_SESSION_DIR"],
                    "hint": "Get API id/hash from my.telegram.org and set TELEGRAM_API_ID, TELEGRAM_API_HASH and TELEGRAM_SESSION_DIR in ~/.switchpost/.env",
                }));
            }
        }
        "google_my_business" => {
            let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
            let connected: Vec<_> = integrations
                .iter()
                .filter(|i| i.provider_identifier == "google_my_business" && is_credential_row(i))
                .collect();
            if !connected.is_empty() {
                output_json(&serde_json::json!({
                    "status": "already_connected",
                    "provider": "google_my_business",
                    "count": connected.len(),
                    "accounts": connected.iter().map(|i| serde_json::json!({
                        "name": i.profile_name,
                        "internal_id": i.internal_id,
                    })).collect::<Vec<_>>(),
                    "hint": "Already connected.",
                }));
            } else if state
                .config
                .provider_credentials("google_my_business")
                .is_some()
            {
                output_json(&serde_json::json!({
                    "status": "not_connected",
                    "provider": "google_my_business", "method": "oauth", "auth_url": oauth_auth_url(state, "google_my_business").await?, "redirect_uri": oauth_redirect_uri(state),
                    "hint": "Open the auth_url in a browser to complete OAuth authorization.",
                }));
            } else {
                output_json(&serde_json::json!({
                    "status": "not_configured",
                    "provider": "google_my_business",
                    "requires": ["YOUTUBE_CLIENT_ID", "YOUTUBE_CLIENT_SECRET"],
                    "hint": "Google uses OAuth client credentials. Set YOUTUBE_CLIENT_ID and YOUTUBE_CLIENT_SECRET in ~/.switchpost/.env",
                }));
            }
        }
        _ => {
            output_json(&serde_json::json!({
                "status": "unknown",
                "provider": provider,
                "error": format!("Unknown provider '{provider}'. Use 'switchpost connect --help' for supported providers."),
                "hint": "Try: x, reddit, linkedin, facebook, instagram, bluesky, github, telegram, discord, slack, pinterest, tiktok, mastodon, youtube, medium, devto, hashnode, wordpress, threads, skool",
            }));
        }
    }

    Ok(())
}

// ── Doctor: health check for all providers ───────────────────

async fn handle_doctor_with_state(state: &AppState) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list integrations: {e}"))?;

    let started_at = checked_at();
    let mut checks: Vec<serde_json::Value> = Vec::new();

    // Check each connected provider by making a lightweight API call
    for integration in &integrations {
        let provider_id = &integration.provider_identifier;
        let name = integration.profile_name.as_deref().unwrap_or("Unknown");
        let token =
            crate::crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

        let (status, detail) = match provider_id.as_str() {
            "x" => {
                // `prepare_from_token` silently no-ops on anything that is not
                // a cookie pair, which then surfaces as an opaque HTTP error
                // from GraphQL. Name the actual cause instead.
                if !crate::social::x::XProvider::is_cookie_auth_static(&token) {
                    ("error".to_string(), Some("stored token is not an X cookie pair — it must carry both auth_token and ct0. Re-import with 'switchpost connect x'".to_string()))
                } else {
                    let mut p = crate::social::x::XProvider::new(&state.config);
                    p.prepare_from_token(&token);
                    match p.get_me(&token).await {
                        Ok(_) => ("healthy".to_string(), None),
                        Err(e) => ("error".to_string(), Some(format!("{e}"))),
                    }
                }
            }
            "reddit" => {
                let mut p = crate::social::reddit::RedditProvider::new(&state.config);
                p.prepare_from_token(&token);
                match p.get_www("/api/me.json", &[]).await {
                    Ok(_) => ("healthy".to_string(), None),
                    Err(e) => ("error".to_string(), Some(format!("{e}"))),
                }
            }
            "github" => {
                // GITHUB_TOKEN was never read back, so a configured-but-dead
                // PAT reported as healthy for as long as it stayed in the env
                // file. `/user` is the cheapest call that proves the token
                // authenticates; fall back to config because the provider
                // itself sources the PAT from env, not from the DB row.
                let pat = if token.is_empty() {
                    state.config.github_token.clone().unwrap_or_default()
                } else {
                    token.clone()
                };
                if pat.is_empty() {
                    ("error".to_string(), Some("no GitHub PAT found — set GITHUB_TOKEN in ~/.switchpost/.env".to_string()))
                } else {
                    let url = "https://api.github.com/user";
                    match crate::social::shared_http()
                        .get(url)
                        .header("Authorization", format!("Bearer {pat}"))
                        .header("Accept", "application/vnd.github+json")
                        .header("User-Agent", "switchpost-doctor")
                        .send().await
                    {
                        Ok(r) if r.status().is_success() => ("healthy".to_string(), None),
                        Ok(r) => {
                            let status = r.status();
                            let body = r.text().await.unwrap_or_default();
                            ("error".to_string(), Some(format!("HTTP {}: {}", status, body.chars().take(200).collect::<String>())))
                        }
                        Err(e) => ("error".to_string(), Some(format!("{e}"))),
                    }
                }
            }
            "linkedin" => {
                let p = crate::social::linkedin::LinkedInProvider::new(&state.config);
                match p.get_profile(&token).await {
                    Ok(_) => ("healthy".to_string(), None),
                    Err(e) => ("error".to_string(), Some(format!("{e}"))),
                }
            }
            "linkedin-page" => {
                let p = crate::social::linkedin_page::LinkedInPageProvider::new(&state.config);
                match p.get_page_posts(&token, &integration.internal_id, 1).await {
                    Ok(_) => ("healthy".to_string(), None),
                    Err(e) => ("error".to_string(), Some(format!("{e}"))),
                }
            }
            "facebook" => {
                let url = format!("https://graph.facebook.com/{}/{}?fields=name", crate::social::GRAPH_VERSION, integration.internal_id);
                match crate::social::shared_http().get(&url).bearer_auth(&token).send().await {
                    Ok(r) if r.status().is_success() => ("healthy".to_string(), None),
                    Ok(r) => {
                        let status = r.status();
                        let body = r.text().await.unwrap_or_default();
                        ("error".to_string(), Some(format!("HTTP {}: {}", status, body.chars().take(200).collect::<String>())))
                    }
                    Err(e) => ("error".to_string(), Some(format!("{e}"))),
                }
            }
            "instagram" | "instagram-standalone" => {
                let url = format!("https://graph.facebook.com/{}/{}?fields=id,name", crate::social::GRAPH_VERSION, integration.internal_id);
                match crate::social::shared_http().get(&url).bearer_auth(&token).send().await {
                    Ok(r) if r.status().is_success() => ("healthy".to_string(), None),
                    Ok(r) => {
                        let status = r.status();
                        let body = r.text().await.unwrap_or_default();
                        ("error".to_string(), Some(format!("HTTP {}: {}", status, body.chars().take(200).collect::<String>())))
                    }
                    Err(e) => ("error".to_string(), Some(format!("{e}"))),
                }
            }
            _ => ("unverified".to_string(), Some("No live health check exists for this provider — its credential is unread, not proven".to_string())),
        };

        let live_verified = status == "healthy";
        let (draft_ok, publish_capable, reason) = readiness_flags(&state.providers, provider_id);
        let mut check = serde_json::json!({
            "provider": provider_id,
            "name": name,
            "internal_id": integration.internal_id,
            "status": status,
            "draft_ok": draft_ok,
            "publish_capable": publish_capable,
            "live_verified": live_verified,
            "live_verified_at": if live_verified {
                serde_json::Value::String(started_at.clone())
            } else {
                serde_json::Value::Null
            },
            "checked_at": started_at,
        });
        if let Some(d) = detail {
            check["detail"] = serde_json::Value::String(d);
        }
        if let Some(r) = reason {
            check["reason"] = serde_json::Value::String(r);
        }
        checks.push(check);
    }

    // Check for missing providers that have env vars configured
    //
    // The check is `provider_credentials(id).is_some()` for ALL THREE, not a
    // per-provider hand-written pair of fields. It was three copies of a rule
    // and one of them had drifted: `instagram` was gated on
    // `facebook_client_id`/`facebook_client_secret`, which is the OTHER
    // provider's credential (the backend's `provider_credentials("instagram")`
    // reads `instagram_client_id`/`instagram_client_secret`). Two failure
    // modes, both silent and both in an audit whose entire job is telling the
    // operator what is configured:
    //   - configured Instagram only -> never told Instagram needs connecting
    //   - configured Facebook only  -> told Instagram needs connecting, and
    //     `switchpost connect instagram` then fails with "not configured"
    // One call, one source of truth. `linkedin` and `facebook` below were
    // already correct; the rule is now enforced by construction rather than by
    // remembering to keep three copies in step.
    let missing_providers: Vec<serde_json::Value> = [
        ("linkedin", "LinkedIn Personal"),
        ("facebook", "Facebook Pages"),
        ("instagram", "Instagram"),
    ]
    .into_iter()
    .filter(|(id, _)| !integrations.iter().any(|i| i.provider_identifier == *id))
    // `provider_credentials` answers presence for every provider in one table,
    // so a new provider is correct the day it is added to `config.rs`.
    .filter(|(id, _)| state.config.provider_credentials(id).is_some())
    .map(|(id, name)| serde_json::json!({
        "provider": id,
        "name": name,
        "status": "needs_oauth",
        "hint": format!("Credentials configured but not connected. Run 'switchpost connect {id}' or visit the onboarding page.")
    }))
    .collect();

    let healthy = checks.iter().filter(|c| c["status"] == "healthy").count();
    let errored = checks.iter().filter(|c| c["status"] == "error").count();
    let live_verified = checks.iter().filter(|c| c["live_verified"] == true).count();
    let unverified = checks.iter().filter(|c| c["live_verified"] != true).count();
    let draft_ok = checks.iter().filter(|c| c["draft_ok"] == true).count();
    let publish_capable = checks
        .iter()
        .filter(|c| c["publish_capable"] == true)
        .count();

    // ── Credential audit (names only) ───────────────────────────
    // `configured` could never be traced: it said a credential existed but
    // not whether it came from the DB row or the env file, so a stale value
    // in one place looked identical to a fresh one in the other. This lists
    // the source per provider and the env var *names* it would read. No value
    // is read into the output — `provider_credentials().is_some()` answers
    // presence without unwrapping anything.
    let db_counts: std::collections::HashMap<&str, usize> =
        integrations
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, i| {
                *acc.entry(i.provider_identifier.as_str()).or_default() += 1;
                acc
            });
    let credentials: Vec<serde_json::Value> = integrations
        .iter()
        .map(|i| {
            let id = i.provider_identifier.as_str();
            let db_rows = db_counts.get(id).copied().unwrap_or(0);
            let env_vars = credential_env_names(id);
            let source = credential_source(id, db_rows, &state.config);
            serde_json::json!({
                "provider": id,
                "source": source,
                "db_rows": db_rows,
                "env_vars": env_vars,
                "env_present": env_vars
                    .iter()
                    .filter(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
                    .count(),
                "note": match source {
                    "none" if env_vars.is_empty() =>
                        "per-account credential only (URL + app password, or a browser-extension cookie) — lives on the integration row".to_string(),
                    "none" => format!(
                        "no credential found: set {} in ~/.switchpost/.env, or connect the account",
                        env_vars.join(" / ")
                    ),
                    _ => "source only — this audit never reads or prints a credential value".to_string(),
                },
            })
        })
        .collect();

    let media = media_preflight(state, user_id, &integrations).await;

    let mut warnings: Vec<String> = Vec::new();
    if checks.is_empty() {
        warnings.push(
            "No provider is connected, so nothing here has been verified against a platform."
                .into(),
        );
    }
    if unverified > 0 {
        warnings.push(format!(
            "{unverified}/{} connected account(s) are live_verified=false — a stored credential is \
             not proof the platform will accept it.",
            checks.len()
        ));
    }
    if publish_capable < draft_ok {
        warnings.push(format!(
            "{}/{} connected account(s) have no post-publish surface.",
            draft_ok - publish_capable,
            draft_ok
        ));
    }
    if media["ok"] != true {
        warnings.push(format!(
            "media reachability pre-check failed: {}",
            media["problems"][0]["kind"]
        ));
    }

    output_json(&serde_json::json!({
        "checked_at": started_at,
        "healthy": healthy,
        "errors": errored,
        "connected": checks.len(),
        "draft_ok": draft_ok,
        "publish_capable": publish_capable,
        "live_verified": live_verified,
        "providers": checks,
        "credentials": credentials,
        "media": media,
        "missing_oauth": missing_providers,
        "warnings": warnings,
    }));
    Ok(())
}

// ── Setup: Full Guided Onboarding ──────────────────────────

async fn handle_setup() -> anyhow::Result<()> {
    crate::config::load_dotenv();
    let mut steps: Vec<serde_json::Value> = Vec::new();

    // Step 1: Config file
    let env_path = crate::config::config_dir().join(".env");
    let config_exists = env_path.exists();
    let has_database_url = std::env::var("DATABASE_URL").is_ok();
    steps.push(serde_json::json!({
        "step": 1,
        "name": "config",
        "status": if config_exists && has_database_url { "ok" } else { "action_needed" },
        "detail": if !config_exists {
            "Run 'switchpost init' to create ~/.switchpost/.env"
        } else if !has_database_url {
            "DATABASE_URL not found. Set it in ~/.switchpost/.env or environment."
        } else {
            "Config file exists and DATABASE_URL is set."
        },
        "action": if !config_exists { Some("switchpost init") } else { None },
    }));

    // Step 2: Database (graceful — don't crash if DB isn't configured)
    let db_result = init_state().await;
    let db_ok = db_result.is_ok();
    steps.push(serde_json::json!({
        "step": 2,
        "name": "database",
        "status": if db_ok { "ok" } else { "error" },
        "detail": if db_ok { "SQLite connection successful." } else { "Cannot open the SQLite database. Check DATABASE_URL in ~/.switchpost/.env" },
    }));

    // If DB failed, we can still report config + cookie status
    let state = match db_result {
        Ok(s) => s,
        Err(e) => {
            // Add remaining steps with limited info
            let x_cookies = crate::social::x_cookies::extract_x_cookies();
            let reddit_cookies = crate::social::reddit_cookies::extract_reddit_cookies();
            steps.push(serde_json::json!({
                "step": 3, "name": "user", "status": "skipped",
                "detail": format!("Skipped: {e}"),
            }));
            steps.push(serde_json::json!({
                "step": 4, "name": "browser_cookies", "status": "info",
                "detail": format!("X cookies: {}. Reddit cookies: {}.",
                    if x_cookies.is_some() { "found" } else { "not_found" },
                    if reddit_cookies.is_some() { "found" } else { "not_found" }),
            }));
            output_json(&serde_json::json!({
                "status": "setup_incomplete",
                "steps": steps,
                "next_actions": {
                    "fix_db": "Set DATABASE_URL in ~/.switchpost/.env to a writable SQLite path (e.g. sqlite://data/switchpost.db?mode=rwc)",
                    "init": "switchpost init",
                },
            }));
            return Ok(());
        }
    };

    // Step 3: User
    let user_result = resolve_user(&state).await;
    let user_ok = user_result.is_ok();
    steps.push(serde_json::json!({
        "step": 3,
        "name": "user",
        "status": if user_ok { "ok" } else { "action_needed" },
        "detail": if user_ok { "User account exists." } else { "No user registered. Register via the web UI or API." },
    }));

    // Step 4: Cookie-based providers (X, Reddit)
    let x_cookies = crate::social::x_cookies::extract_x_cookies();
    let reddit_cookies = crate::social::reddit_cookies::extract_reddit_cookies();
    let x_cookie_status = if x_cookies.is_some() {
        "found"
    } else {
        "not_found"
    };
    let reddit_cookie_status = if reddit_cookies.is_some() {
        "found"
    } else {
        "not_found"
    };
    steps.push(serde_json::json!({
        "step": 4,
        "name": "browser_cookies",
        "status": "info",
        "detail": format!("X cookies: {}. Reddit cookies: {}.", x_cookie_status, reddit_cookie_status),
        "hint": "Run 'switchpost connect-all' to auto-import cookies from your browser.",
    }));

    // Step 5: Existing integrations
    if let Ok(uid) = user_result {
        let integrations = crate::db::queries::list_integrations(&state.db, uid)
            .await
            .unwrap_or_default();
        let provider_summary: Vec<serde_json::Value> = integrations
            .iter()
            .map(|i| {
                serde_json::json!({
                    "provider": i.provider_identifier,
                    "name": i.profile_name,
                })
            })
            .collect();
        steps.push(serde_json::json!({
            "step": 5,
            "name": "integrations",
            "status": if provider_summary.is_empty() { "action_needed" } else { "ok" },
            "connected_count": provider_summary.len(),
            "connected": provider_summary,
        }));
    }

    // Step 6: Env-var providers status
    let env_providers: Vec<serde_json::Value> = vec![
        serde_json::json!({"name": "Bluesky", "key": "BLUESKY_HANDLE", "configured": state.config.bluesky_handle.is_some()}),
        serde_json::json!({"name": "GitHub", "key": "GITHUB_TOKEN", "configured": state.config.github_token.is_some()}),
        serde_json::json!({"name": "Telegram Bot", "key": "TELEGRAM_BOT_TOKENS", "configured": state.config.telegram_bot_tokens.is_some()}),
        serde_json::json!({"name": "Discord Bot", "key": "DISCORD_BOT_TOKEN", "configured": state.config.discord_bot_token.is_some()}),
        serde_json::json!({"name": "Dev.to", "key": "DEVTO_API_KEY", "configured": state.config.devto_api_key.is_some()}),
        serde_json::json!({"name": "Medium", "key": "MEDIUM_ACCESS_TOKEN", "configured": state.config.medium_access_token.is_some()}),
        serde_json::json!({"name": "Hashnode", "key": "HASHNODE_API_KEY", "configured": state.config.hashnode_api_key.is_some()}),
    ];
    let configured_count = env_providers
        .iter()
        .filter(|p| p["configured"] == true)
        .count();
    steps.push(serde_json::json!({
        "step": 6,
        "name": "env_providers",
        "status": "info",
        "configured_count": configured_count,
        "total": env_providers.len(),
        "providers": env_providers,
        "hint": "Use 'switchpost config set KEY VALUE' to add API keys.",
    }));

    // Summary
    let action_needed = steps
        .iter()
        .any(|s| s["status"] == "action_needed" || s["status"] == "error");
    output_json(&serde_json::json!({
        "status": if action_needed { "setup_incomplete" } else { "ready" },
        "steps": steps,
        "next_actions": {
            "connect_all": "switchpost connect-all  (import browser cookies for X + Reddit)",
            "doctor": "switchpost doctor  (health-check all providers)",
            "config_set": "switchpost config set KEY VALUE  (add API keys)",
        },
    }));
    Ok(())
}

// ── Connect All: Bulk Cookie Import ──────────────────────────

async fn handle_connect_all_with_state(state: &AppState) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let mut results: Vec<serde_json::Value> = Vec::new();

    // ── X/Twitter ─────────────────────────────────────────────
    let x_result = {
        let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
        if integrations.iter().any(|i| i.provider_identifier == "x") {
            serde_json::json!({"provider": "x", "status": "already_connected"})
        } else {
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
                                .unwrap_or("X User");
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
                                Some(name),
                                None,
                                avatar,
                                None,
                                None,
                            )
                            .await
                            {
                                Ok(_) => {
                                    serde_json::json!({"provider": "x", "status": "connected", "name": name, "source": cookies.source})
                                }
                                Err(e) => {
                                    serde_json::json!({"provider": "x", "status": "error", "error": format!("DB write failed: {e}")})
                                }
                            }
                        }
                        Err(e) => {
                            serde_json::json!({"provider": "x", "status": "error", "error": format!("{e}")})
                        }
                    }
                }
                None => {
                    serde_json::json!({"provider": "x", "status": "no_cookies", "hint": "Log into x.com in your browser first."})
                }
            }
        }
    };
    results.push(x_result);

    // ── Reddit ────────────────────────────────────────────────
    let reddit_result = {
        let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
        if integrations
            .iter()
            .any(|i| i.provider_identifier == "reddit")
        {
            serde_json::json!({"provider": "reddit", "status": "already_connected"})
        } else {
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
                                None,
                                icon.as_deref(),
                                None,
                                None,
                            )
                            .await
                            {
                                Ok(_) => {
                                    serde_json::json!({"provider": "reddit", "status": "connected", "name": name, "source": cookies.source})
                                }
                                Err(e) => {
                                    serde_json::json!({"provider": "reddit", "status": "error", "error": format!("DB write failed: {e}")})
                                }
                            }
                        }
                        Err(e) => {
                            serde_json::json!({"provider": "reddit", "status": "error", "error": format!("{e}")})
                        }
                    }
                }
                None => {
                    serde_json::json!({"provider": "reddit", "status": "no_cookies", "hint": "Log into reddit.com in your browser first."})
                }
            }
        }
    };
    results.push(reddit_result);

    let connected = results
        .iter()
        .filter(|r| r["status"] == "connected")
        .count();
    let already = results
        .iter()
        .filter(|r| r["status"] == "already_connected")
        .count();
    let failed = results.iter().filter(|r| r["status"] == "error").count();
    let no_cookies = results
        .iter()
        .filter(|r| r["status"] == "no_cookies")
        .count();

    output_json(&serde_json::json!({
        "connected": connected,
        "already_connected": already,
        "errors": failed,
        "no_cookies": no_cookies,
        "results": results,
    }));
    Ok(())
}

// ── Config: Manage ~/.switchpost/.env ──────────────────────

fn set_env_value(env_path: &std::path::Path, key_upper: &str, value: &str) -> anyhow::Result<()> {
    let content = if env_path.exists() {
        std::fs::read_to_string(env_path)?
    } else {
        String::new()
    };

    let new_line = format!("{key_upper}={value}");
    let prefix = format!("{key_upper}=");
    let commented_prefix = format!("# {prefix}");
    let hashed_prefix = format!("#{key_upper}=");

    let mut found = false;
    let new_content: String = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(&prefix)
                || trimmed.starts_with(&commented_prefix)
                || trimmed.starts_with(&hashed_prefix)
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
        std::fs::write(env_path, &new_content)?;
    } else {
        let mut to_write = new_content;
        if !to_write.ends_with('\n') {
            to_write.push('\n');
        }
        to_write.push_str(&new_line);
        to_write.push('\n');
        std::fs::write(env_path, &to_write)?;
    }
    Ok(())
}

fn handle_config(action: ConfigAction) -> anyhow::Result<()> {
    crate::config::load_dotenv();
    let dir = crate::config::config_dir();
    std::fs::create_dir_all(&dir)?;
    let env_path = dir.join(".env");

    // `Config::from_env` derives the session secret from APP_PASSWORD when
    // JWT_SECRET is unset, so a reset invalidates every existing session cookie.
    let jwt_secret_set = std::env::var("JWT_SECRET").is_ok_and(|v| !v.is_empty());

    match action {
        ConfigAction::Set { key, value } => {
            let key_upper = key.to_uppercase();
            set_env_value(&env_path, &key_upper, &value)?;

            output_json(&serde_json::json!({
                "status": "set",
                "key": key_upper,
                "path": env_path.display().to_string(),
                "message": format!("Set {key_upper}. Restart switchpost to apply."),
            }));
        }
        ConfigAction::Get { key } => {
            let key_upper = key.to_uppercase();
            match std::env::var(&key_upper) {
                Ok(val) => {
                    // Redact secrets
                    let display = if key_upper.contains("SECRET")
                        || key_upper.contains("PASSWORD")
                        || key_upper.contains("TOKEN")
                        || key_upper.contains("KEY")
                        || key_upper.contains("PRIVATE")
                    {
                        if val.len() > 8 {
                            format!("{}...{}", &val[..4], &val[val.len() - 4..])
                        } else {
                            "****".into()
                        }
                    } else {
                        val
                    };
                    output_json(&serde_json::json!({
                        "key": key_upper,
                        "value": display,
                        "is_secret": key_upper.contains("SECRET") || key_upper.contains("PASSWORD") ||
                                       key_upper.contains("TOKEN") || key_upper.contains("KEY") ||
                                       key_upper.contains("PRIVATE"),
                    }));
                }
                Err(_) => {
                    output_json(&serde_json::json!({
                        "key": key_upper,
                        "value": null,
                        "error": format!("'{key_upper}' is not set."),
                    }));
                }
            }
        }
        ConfigAction::ResetPassword { password } => {
            let generated = password.as_deref().is_none_or(|p| p.is_empty());
            let new_pw = match password.filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => crate::config::generate_random_password(32),
            };
            if new_pw.len() < 8 {
                return output_error("Password must be at least 8 characters.");
            }
            let written = crate::config::persist_app_password(&new_pw)?;
            let mut out = serde_json::json!({
                "status": "reset",
                "key": "APP_PASSWORD",
                "path": written.display().to_string(),
                "generated": generated,
                "sessions_invalidated": !jwt_secret_set,
                "message": "Password updated. Restart switchpost to apply.",
            });
            if generated {
                out["password"] = serde_json::json!(new_pw);
            }
            output_json(&out);
        }
        ConfigAction::List => {
            // Read from .env file to show all keys (without loading into env)
            let mut entries: Vec<serde_json::Value> = Vec::new();
            if env_path.exists() {
                let content = std::fs::read_to_string(&env_path)?;
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
                        entries.push(serde_json::json!({
                            "key": k,
                            "value": display,
                            "is_secret": is_secret,
                        }));
                    }
                }
            }
            output_json(&serde_json::json!({
                "path": env_path.display().to_string(),
                "exists": env_path.exists(),
                "count": entries.len(),
                "entries": entries,
            }));
        }
    }
    Ok(())
}

// ── X (Twitter) Handler ──────────────────────────────────────

// ── Import Handler ───────────────────────────────────────────

async fn handle_import_with_state(
    state: &AppState,
    provider_name: &str,
    count: u32,
) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integration = find_integration(state, user_id, provider_name).await?;
    let token =
        crate::crypto::maybe_decrypt_token(&integration.access_token, state.token_key.as_ref());

    let provider = state
        .providers
        .get(provider_name)
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found in registry", provider_name))?;

    let posts = provider
        .get_recent_posts(&token, &integration.internal_id, count)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get posts from {provider_name}: {e}"))?;

    let mut imported = 0u32;
    for post in &posts {
        let media_val = serde_json::to_value(&post.media).unwrap_or_default();
        let metadata_val = post.metadata.clone().unwrap_or_default();
        match db::queries::insert_external_post(
            &state.db,
            user_id,
            provider_name,
            &post.platform_post_id,
            &post.text,
            post.author_name.as_deref(),
            post.author_handle.as_deref(),
            post.author_avatar.as_deref(),
            post.created_at,
            post.url.as_deref(),
            &media_val,
            &metadata_val,
        )
        .await
        {
            Ok(Some(_)) => imported += 1,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Failed to import post {}: {e}", post.platform_post_id);
            }
        }
    }

    let truncated: Vec<serde_json::Value> = posts
        .into_iter()
        .map(|p| {
            let val = serde_json::to_value(&p).unwrap_or_default();
            truncate_json_strings(&val, 500)
        })
        .collect();
    output_json(&serde_json::json!({
        "provider": provider_name,
        "imported": imported,
        "total": truncated.len(),
        "posts": truncated,
    }));
    Ok(())
}

// ── Feed Handler ────────────────────────────────────────────

async fn handle_feed_with_state(
    state: &AppState,
    provider: Option<&str>,
    limit: u32,
) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let limit = limit.min(100) as i64;

    let posts =
        crate::db::queries::list_all_external_posts(&state.db, user_id, provider, None, limit)
            .await?;

    // AXI §5: Definitive empty state + §3: truncate long post text
    if posts.is_empty() {
        let filter = provider.unwrap_or("all");
        output_json(&serde_json::json!({
            "posts": [],
            "count": 0,
            "provider": provider,
            "message": format!("No feed posts found for {filter}. Run 'switchpost import <provider> --count 10' to pull recent posts."),
        }));
    } else {
        let truncated: Vec<serde_json::Value> = posts
            .into_iter()
            .map(|p| {
                let val = serde_json::to_value(&p).unwrap_or_default();
                truncate_json_strings(&val, 500)
            })
            .collect();
        output_json(&serde_json::json!({
            "provider": provider,
            "count": truncated.len(),
            "posts": truncated,
        }));
    }
    Ok(())
}

// ── Instagram Handler ────────────────────────────────────────

// ── Comment Handler ──────────────────────────────────────────

/// Campaigns, signatures, AI helpers and the posting streak.
///
/// All of them call the SAME handlers the MCP tools call, so the CLI and
/// the MCP surface cannot drift — the repo's rule for a second surface
/// over one operation (§9.21).
async fn handle_campaigns_with_state(
    state: &AppState,
    action: CampaignAction,
) -> anyhow::Result<()> {
    use crate::mcp::tools_campaigns as c;

    let value = match action {
        CampaignAction::List => c::handle_campaign_list(state, &c::CampaignListInput {})
            .await
            .map(|r| r.0),
        CampaignAction::Create {
            name,
            description,
            color,
            start_date,
            end_date,
            goal,
            status,
            progress_metric,
            progress_target,
        } => {
            let input = c::CampaignCreateInput {
                name,
                description,
                color,
                start_date,
                end_date,
                goal,
                status,
                progress_metric,
                progress_target,
                audience_persona: None,
                content_pillars: None,
                budget_cents: None,
                kpi_targets: None,
            };
            c::handle_campaign_create(state, &input).await.map(|r| r.0)
        }
        CampaignAction::Update {
            campaign_id,
            name,
            description,
            clear_description,
            goal,
            clear_goal,
            status,
        } => {
            let input = c::CampaignUpdateInput {
                campaign_id,
                name,
                description: double_state(description, clear_description),
                color: None,
                start_date: None,
                end_date: None,
                goal: double_state(goal, clear_goal),
                status,
                progress_metric: None,
                progress_target: None,
                audience_persona: None,
                content_pillars: None,
                budget_cents: None,
                kpi_targets: None,
            };
            c::handle_campaign_update(state, &input).await.map(|r| r.0)
        }
        CampaignAction::Delete { campaign_id } => {
            let input = c::CampaignDeleteInput { campaign_id };
            c::handle_campaign_delete(state, &input).await.map(|r| r.0)
        }
    };
    output_json(&truncate_json_strings(
        &value.map_err(|e| anyhow::anyhow!("{e}"))?,
        500,
    ));
    Ok(())
}

/// Turn a `--field value` / `--clear-field` pair into the three-state
/// shape the update handler needs: absent = keep, `null` = clear.
fn double_state(value: Option<String>, clear: bool) -> Option<Option<String>> {
    if clear {
        Some(None)
    } else {
        value.map(Some)
    }
}

async fn handle_signatures_with_state(
    state: &AppState,
    action: SignatureAction,
) -> anyhow::Result<()> {
    use crate::mcp::tools_admin as a;

    let value = match action {
        SignatureAction::List => a::handle_signatures_list(state, &a::SignatureListInput {})
            .await
            .map(|r| r.0),
        SignatureAction::Create {
            name,
            content,
            provider,
        } => {
            let input = a::SignatureCreateInput {
                name,
                content,
                provider,
            };
            a::handle_signatures_create(state, &input)
                .await
                .map(|r| r.0)
        }
        SignatureAction::Update {
            signature_id,
            name,
            content,
            provider,
        } => {
            let input = a::SignatureUpdateInput {
                signature_id,
                name,
                content,
                provider,
            };
            a::handle_signatures_update(state, &input)
                .await
                .map(|r| r.0)
        }
        SignatureAction::Delete { signature_id } => {
            let input = a::SignatureDeleteInput { signature_id };
            a::handle_signatures_delete(state, &input)
                .await
                .map(|r| r.0)
        }
    };
    output_json(&truncate_json_strings(
        &value.map_err(|e| anyhow::anyhow!("{e}"))?,
        500,
    ));
    Ok(())
}

async fn handle_ai_with_state(state: &AppState, action: AiAction) -> anyhow::Result<()> {
    use crate::mcp::tools_ai as ai;

    let value = match action {
        AiAction::Generate {
            topic,
            tone,
            length,
        } => {
            let input = ai::AiGeneratePostInput {
                topic,
                tone,
                length,
            };
            ai::handle_ai_generate_post(state, &input)
                .await
                .map(|r| r.0)
        }
        AiAction::Bulk {
            topic,
            format,
            tone,
            count,
        } => {
            let input = ai::AiGenerateBulkInput {
                topic,
                format,
                tone,
                count: Some(count),
            };
            ai::handle_ai_generate_bulk(state, &input)
                .await
                .map(|r| r.0)
        }
        AiAction::Improve { content } => {
            let input = ai::AiContentInput { content };
            ai::handle_ai_improve_writing(state, &input)
                .await
                .map(|r| r.0)
        }
        AiAction::Summarize { content } => {
            let input = ai::AiContentInput { content };
            ai::handle_ai_summarize(state, &input).await.map(|r| r.0)
        }
        AiAction::Hashtags { content } => {
            let input = ai::AiContentInput { content };
            ai::handle_ai_suggest_hashtags(state, &input)
                .await
                .map(|r| r.0)
        }
        AiAction::Tone { content, tone } => {
            let input = ai::AiChangeToneInput { content, tone };
            ai::handle_ai_change_tone(state, &input).await.map(|r| r.0)
        }
    };
    output_json(&truncate_json_strings(
        &value.map_err(|e| anyhow::anyhow!("{e}"))?,
        500,
    ));
    Ok(())
}

/// RSS autopost feeds.
///
/// These call the SAME handlers the MCP tools call, so the CLI and the
/// MCP surface cannot drift — that is the repo's rule for a second
/// surface over one operation (§9.21).
async fn handle_rss_with_state(state: &AppState, action: RssAction) -> anyhow::Result<()> {
    use crate::mcp::tools_rss as rss_tools;

    let value = match action {
        RssAction::List => rss_tools::handle_rss_feed_list(state, &rss_tools::RssFeedListInput {})
            .await
            .map(|r| r.0),
        RssAction::Add {
            feed_url,
            integration_id,
            title,
            ai_summary,
            disabled,
        } => {
            let input = rss_tools::RssFeedCreateInput {
                feed_url,
                integration_id,
                title,
                use_ai_summary: Some(ai_summary),
                enabled: Some(!disabled),
            };
            rss_tools::handle_rss_feed_create(state, &input)
                .await
                .map(|r| r.0)
        }
        RssAction::Delete { feed_id } => {
            rss_tools::handle_rss_feed_delete(state, &rss_tools::RssFeedIdInput { feed_id })
                .await
                .map(|r| r.0)
        }
        RssAction::Toggle { feed_id } => {
            rss_tools::handle_rss_feed_toggle(state, &rss_tools::RssFeedIdInput { feed_id })
                .await
                .map(|r| r.0)
        }
        RssAction::Poll { feed_id } => {
            rss_tools::handle_rss_feed_poll(state, &rss_tools::RssFeedIdInput { feed_id })
                .await
                .map(|r| r.0)
        }
        RssAction::Items { feed_id } => {
            rss_tools::handle_rss_feed_items(state, &rss_tools::RssFeedIdInput { feed_id })
                .await
                .map(|r| r.0)
        }
    };
    let value = value.map_err(|e| anyhow::anyhow!("{e}"))?;
    output_json(&truncate_json_strings(&value, 500));
    Ok(())
}

async fn handle_comment_with_state(state: &AppState, action: CommentAction) -> anyhow::Result<()> {
    match action {
        CommentAction::Get {
            integration_id,
            post_id,
            limit,
        } => {
            let input = crate::mcp::tools_comments::GetCommentsInput {
                integration_id,
                post_id,
                limit: Some(limit),
            };
            let result = match crate::mcp::tools_comments::get_comments(state, &input).await {
                Ok(r) => r,
                Err(e) => output_error_with_hint(
                    &format!("Failed to get comments: {}", e),
                    "Run `switchpost doctor` to check provider health.",
                ),
            };
            let val =
                truncate_json_strings(&serde_json::to_value(result.0).unwrap_or_default(), 500);
            output_json(&val);
        }
        CommentAction::Reply {
            integration_id,
            comment_id,
            content,
        } => {
            let input = crate::mcp::tools_comments::ReplyToCommentInput {
                integration_id,
                comment_id,
                content,
            };
            let result = match crate::mcp::tools_comments::reply_to_comment(state, &input).await {
                Ok(r) => r,
                Err(e) => output_error_with_hint(
                    &format!("Failed to reply to comment: {}", e),
                    "Ensure the integration is connected. Run `switchpost doctor` to check.",
                ),
            };
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
    }
    Ok(())
}

// ── DM Handler ───────────────────────────────────────────────

async fn handle_dm_with_state(state: &AppState, action: DmAction) -> anyhow::Result<()> {
    match action {
        DmAction::Send {
            integration_id,
            recipient,
            content,
        } => {
            let input = crate::mcp::tools_dm::SendDmInput {
                integration_id,
                recipient,
                content,
            };
            let result = match crate::mcp::tools_dm::send_dm(state, &input).await {
                Ok(r) => r,
                Err(e) => output_error_with_hint(
                    &format!("Failed to send DM: {}", e),
                    "Ensure the DM integration is connected. Run `switchpost doctor` to check.",
                ),
            };
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        DmAction::List {
            integration_id,
            limit,
        } => {
            let input = crate::mcp::tools_dm::ListDmInput {
                integration_id,
                limit: Some(limit),
            };
            let result = match crate::mcp::tools_dm::list_dm_conversations(state, &input).await {
                Ok(r) => r,
                Err(e) => output_error_with_hint(
                    &format!("Failed to list DM conversations: {}", e),
                    "Run `switchpost doctor` to check provider health.",
                ),
            };
            let val =
                truncate_json_strings(&serde_json::to_value(result.0).unwrap_or_default(), 500);
            output_json(&val);
        }
        DmAction::Messages {
            integration_id,
            conversation_id,
            limit,
        } => {
            let input = crate::mcp::tools_dm::GetDmInput {
                integration_id,
                conversation_id,
                limit: Some(limit),
            };
            let result = match crate::mcp::tools_dm::get_dm_messages(state, &input).await {
                Ok(r) => r,
                Err(e) => output_error_with_hint(
                    &format!("Failed to get DM messages: {}", e),
                    "Ensure the conversation ID is valid.",
                ),
            };
            let val =
                truncate_json_strings(&serde_json::to_value(result.0).unwrap_or_default(), 500);
            output_json(&val);
        }
    }
    Ok(())
}

// ── Automation Handler ───────────────────────────────────────

async fn handle_automation_with_state(
    state: &AppState,
    action: AutomationAction,
) -> anyhow::Result<()> {
    match action {
        AutomationAction::Create {
            integration_id,
            name,
            trigger_type,
            response_template,
            response_type,
        } => {
            let input = crate::mcp::tools_automation::CreateRuleInput {
                integration_id,
                name,
                trigger_type,
                trigger_filter: serde_json::json!({}),
                response_template,
                response_type,
                ai_model: None,
                cooldown_minutes: None,
                max_responses_per_hour: None,
            };
            let result = crate::mcp::tools_automation::create_rule(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        AutomationAction::List { integration_id } => {
            let input = crate::mcp::tools_automation::ListRulesInput { integration_id };
            let result = crate::mcp::tools_automation::list_rules(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let val =
                truncate_json_strings(&serde_json::to_value(result.0).unwrap_or_default(), 500);
            output_json(&val);
        }
        AutomationAction::Update {
            rule_id,
            name,
            response_template,
            is_active,
        } => {
            let input = crate::mcp::tools_automation::UpdateRuleInput {
                rule_id,
                name,
                trigger_filter: None,
                response_template,
                response_type: None,
                ai_model: None,
                is_active,
                cooldown_minutes: None,
                max_responses_per_hour: None,
            };
            let result = crate::mcp::tools_automation::update_rule(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        AutomationAction::Delete { rule_id } => {
            let input = crate::mcp::tools_automation::DeleteRuleInput { rule_id };
            let result = crate::mcp::tools_automation::delete_rule(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        AutomationAction::Logs { rule_id, limit } => {
            let input = crate::mcp::tools_automation::GetLogsInput {
                rule_id,
                limit: Some(limit as i64),
            };
            let result = crate::mcp::tools_automation::get_logs(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let val =
                truncate_json_strings(&serde_json::to_value(result.0).unwrap_or_default(), 500);
            output_json(&val);
        }
    }
    Ok(())
}

// Unified Post Handler
/// Parse the `--settings` JSON argument.
///
/// Some providers cannot publish without a target, and they read it out
/// of the post's settings: Reddit needs `subreddit` and `title`,
/// Pinterest needs `board` and `title`, Discord needs `channel`,
/// Telegram-user needs `target`, Hashnode needs `publication_id`.
///
/// The CLI used to send an EMPTY settings object, so a Reddit post from
/// the shell was created, queued, and then failed at publish with
/// "Missing subreddit in settings" - with no way to supply it.
fn parse_cli_settings(raw: Option<&str>) -> anyhow::Result<serde_json::Value> {
    let Some(raw) = raw else {
        return Ok(serde_json::json!({}));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| anyhow::anyhow!("--settings must be a JSON object: {e}"))?;
    if !value.is_object() {
        return Err(anyhow::anyhow!(
            "--settings must be a JSON object, got {}",
            value
        ));
    }
    Ok(value)
}

async fn handle_post_with_state(
    state: &AppState,
    text: &str,
    platforms: Option<&str>,
    media: Option<&str>,
    schedule: Option<&str>,
    first_comment: Option<&str>,
    settings: Option<&str>,
) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;

    let integration_ids: Vec<String> = if let Some(platforms_str) = platforms {
        resolve_platform_integrations(integrations.iter(), platforms_str)
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    } else {
        integrations.iter().map(|i| i.id.to_string()).collect()
    };

    if integration_ids.is_empty() {
        return Err(anyhow::anyhow!(
            "No matching integrations found. Use 'switchpost providers' to see connected accounts."
        ));
    }

    let input = crate::mcp::tools_posts::StagePostInput {
        content: text.to_string(),
        media: media.map(|m| {
            serde_json::json!(m
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|u| serde_json::json!({"url": u}))
                .collect::<Vec<_>>())
        }),
        integration_ids,
        settings: Some(parse_cli_settings(settings)?),
        scheduled_at: schedule.map(String::from),
        first_comment: first_comment.map(String::from),
        overrides: None,
        dry_run: false,
        force: false,
        group_id: None,
    };

    let result = crate::mcp::tools_posts::stage_post(state, &input)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    output_json(&serde_json::to_value(result.0).unwrap_or_default());
    Ok(())
}

// Stage Handler
#[expect(
    clippy::too_many_arguments,
    reason = "7 arguments required to handle stage command with all optional input fields"
)]
async fn handle_stage_with_state(
    state: &AppState,
    text: &str,
    integrations_str: Option<&str>,
    platforms_str: Option<&str>,
    media: Option<&str>,
    schedule: Option<&str>,
    preview_only: bool,
    first_comment: Option<&str>,
    settings: Option<&str>,
) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let all_integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let integration_ids: Vec<Uuid> = if let Some(iids) = integrations_str {
        iids.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Uuid::parse_str(s).map_err(|_| anyhow::anyhow!("Invalid integration_id: {s}")))
            .collect::<Result<_, _>>()?
    } else if let Some(plats) = platforms_str {
        resolve_platform_integrations(all_integrations.iter(), plats)
    } else {
        all_integrations.iter().map(|i| i.id).collect()
    };
    if integration_ids.is_empty() {
        return Err(anyhow::anyhow!(
            "No integration IDs specified and no connected accounts found."
        ));
    }
    let media_json = match media {
        Some(m) => {
            let urls: Vec<String> = m
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            serde_json::json!(urls
                .iter()
                .map(|u| serde_json::json!({"url": u}))
                .collect::<Vec<_>>())
        }
        None => serde_json::json!([]),
    };
    let scheduled_at = match schedule {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| anyhow::anyhow!("Invalid date format, use ISO8601"))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    if preview_only {
        let previews: Vec<serde_json::Value> = integration_ids.iter().filter_map(|iid| {
            let integration = all_integrations.iter().find(|i| i.id == *iid)?;
            let limit = state.providers.get(&integration.provider_identifier).map_or(0, |p| p.max_content_length());
            let segments = crate::services::content_splitter::split_content(text, &integration.provider_identifier, limit);
            Some(serde_json::json!({
                "provider": integration.provider_identifier,
                "limit": limit,
                "segments": segments.len(),
                "posts": segments.iter().map(|s| serde_json::json!({"sequence": s.sequence, "total": s.total, "content": s.content, "char_count": s.content.len()})).collect::<Vec<_>>(),
            }))
        }).collect();
        output_json(
            &serde_json::json!({"preview": true, "total_integrations": previews.len(), "integrations": previews}),
        );
        return Ok(());
    }
    let request = crate::services::staging::StagingRequest {
        content: text.to_string(),
        media: media_json,
        integration_ids,
        settings: parse_cli_settings(settings)?,
        scheduled_at,
        first_comment: first_comment.map(String::from),
        overrides: serde_json::Value::Null,
        dry_run: false,
        force: false,
        next_slot: false,
        group_id: None,
    };
    crate::services::staging::validate_staging_request(&request)
        .map_err(|e| anyhow::anyhow!("Validation failed: {e}"))?;
    let result = crate::services::staging::stage_post(
        &state.db,
        &state.providers,
        &state.config.app_url,
        user_id,
        request,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Staging failed: {e}"))?;
    let staged: Vec<serde_json::Value> = result.staged.into_iter().map(|s| {
        serde_json::json!({"post_id": s.post_id.to_string(), "provider": s.provider, "sequence": s.sequence, "total_segments": s.total_segments, "state": s.state})
    }).collect();
    output_json(
        &serde_json::json!({"status": "staged", "total_posts": result.total_posts, "warnings": result.warnings, "staged": staged}),
    );
    Ok(())
}

// Carousel Handler
async fn handle_carousel_with_state(
    state: &AppState,
    text: &str,
    integration_id_str: &str,
    media_str: &str,
    title: Option<&str>,
    schedule: Option<&str>,
) -> anyhow::Result<()> {
    let media_urls: Vec<String> = media_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if media_urls.len() < 2 {
        return Err(anyhow::anyhow!("Carousel requires at least 2 media URLs"));
    }

    let scheduled_at = match schedule {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| anyhow::anyhow!("Invalid date format, use ISO8601"))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };

    let media_json = serde_json::json!(media_urls);
    let user_id = resolve_user(state).await?;
    let integration_id = Uuid::parse_str(integration_id_str)
        .map_err(|_| anyhow::anyhow!("Invalid integration_id format"))?;

    let post = crate::services::posts::PostService::create(
        &state.db,
        &state.broadcast,
        crate::services::posts::CreatePostInput {
            user_id,
            integration_id,
            content: text.to_string(),
            title: title.map(String::from),
            media_urls: media_json,
            scheduled_at,
            settings: serde_json::json!({}),
            first_comment: None,
            state: None,
            campaign_id: None,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to create carousel: {e}"))?;

    output_json(&serde_json::json!({
        "id": post.id.to_string(),
        "state": post.state.to_string(),
        "scheduled_at": post.scheduled_at.map(|d| d.to_rfc3339()),
        "created_at": post.created_at.to_rfc3339(),
    }));
    Ok(())
}

// Media Handler
async fn handle_media_with_state(state: &AppState, action: MediaAction) -> anyhow::Result<()> {
    match action {
        MediaAction::Upload { path, alt: _ } => {
            let result = crate::mcp::tools_media::upload_from_path(state, &path)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        MediaAction::List { limit, search } => {
            let input = crate::mcp::tools_media::MediaListInput {
                limit: Some(limit.min(200) as i64),
                search,
            };
            let result = crate::mcp::tools_media::list_media(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
        MediaAction::Download { url, output } => {
            let resp = crate::social::shared_http()
                .get(&url)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to download: {e}"))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(anyhow::anyhow!("Download failed with HTTP {}", status));
            }
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read response: {e}"))?;
            let out_path = if std::path::Path::new(&output).is_dir() {
                let filename = url.rsplit('/').next().unwrap_or("download");
                let filename = filename.split('?').next().unwrap_or(filename);
                std::path::Path::new(&output).join(filename)
            } else {
                std::path::PathBuf::from(&output)
            };
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;
            }
            std::fs::write(&out_path, &bytes)
                .map_err(|e| anyhow::anyhow!("Failed to write file: {e}"))?;
            output_json(&serde_json::json!({
                "status": "downloaded",
                "url": url,
                "path": out_path.display().to_string(),
                "size": bytes.len() as i64,
                "content_type": content_type,
            }));
        }
        MediaAction::UploadBatch { paths } => {
            if paths.is_empty() {
                return Err(anyhow::anyhow!("At least one file path is required"));
            }
            let input = crate::mcp::tools_media::MediaUploadBatchInput { paths, alt: None };
            let result = crate::mcp::tools_media::upload_batch(state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            output_json(&serde_json::to_value(result.0).unwrap_or_default());
        }
    }
    Ok(())
}

// ── Posts Handler ─────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn handle_posts_create_helper(
    state: &crate::api::AppState,
    user_id: uuid::Uuid,
    content: &str,
    integrations: &str,
    schedule: Option<&str>,
    title: Option<&str>,
    media: Option<&str>,
    first_comment: Option<&str>,
    settings: Option<&str>,
    state_override: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let integration_ids: Vec<uuid::Uuid> = integrations
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<uuid::Uuid>().ok())
        .collect();
    if integration_ids.is_empty() {
        return Err(anyhow::anyhow!(
            "At least one valid integration UUID required"
        ));
    }

    let scheduled_at = match schedule {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| anyhow::anyhow!("Invalid schedule date, use ISO 8601 / RFC 3339"))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };

    let media_json = media
        .map(parse_media_arg)
        .unwrap_or_else(|| serde_json::json!([]));

    let settings_json: serde_json::Value = match settings {
        Some(s) => serde_json::from_str::<serde_json::Value>(s)
            .map_err(|e| anyhow::anyhow!("Invalid settings JSON: {e}"))?,
        None => serde_json::json!({}),
    };

    let target_state = match state_override {
        Some("draft") => Some(crate::db::models::PostState::Draft),
        Some("queued") => Some(crate::db::models::PostState::Queued),
        Some("published") => Some(crate::db::models::PostState::Published),
        Some("error") => Some(crate::db::models::PostState::Error),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid --state '{other}'. Use draft|queued|published|error."
            ))
        }
        None if scheduled_at.is_some() => Some(crate::db::models::PostState::Queued),
        None => None,
    };

    let first_comment_owned = first_comment.and_then(|s| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    });
    let title_owned = title.map(String::from);

    fn build(
        iid: uuid::Uuid,
        user_id: uuid::Uuid,
        content: &str,
        title_owned: Option<String>,
        media_json: serde_json::Value,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        settings_json: serde_json::Value,
        first_comment_owned: Option<String>,
        target_state: Option<crate::db::models::PostState>,
    ) -> crate::services::posts::CreatePostInput {
        crate::services::posts::CreatePostInput {
            user_id,
            integration_id: iid,
            content: content.to_string(),
            title: title_owned,
            media_urls: media_json,
            scheduled_at,
            settings: settings_json,
            first_comment: first_comment_owned,
            state: target_state,
            campaign_id: None,
        }
    }

    let mut created: Vec<serde_json::Value> = Vec::new();
    let count = integration_ids.len();
    for &iid in &integration_ids {
        let input = build(
            iid,
            user_id,
            content,
            title_owned.clone(),
            media_json.clone(),
            scheduled_at,
            settings_json.clone(),
            first_comment_owned.clone(),
            target_state.clone(),
        );
        match crate::services::posts::PostService::create(&state.db, &state.broadcast, input).await
        {
            Ok(post) => created.push(serde_json::json!({
                "id": post.id.to_string(),
                "integration_id": iid.to_string(),
                "state": post.state.to_string(),
                "scheduled_at": post.scheduled_at.map(|d| d.to_rfc3339()),
                "created_at": post.created_at.to_rfc3339(),
            })),
            Err(e) => {
                if count == 1 {
                    return Err(anyhow::anyhow!("Create failed: {e}"));
                }
                tracing::warn!("Best-effort create for integration {iid} failed: {e}");
                created.push(serde_json::json!({
                    "integration_id": iid.to_string(),
                    "error": e,
                }));
            }
        }
    }
    Ok(serde_json::json!({
        "ok": true,
        "count": created.len(),
        "posts": created,
    }))
}

/// `--media` takes a JSON array or a comma-separated list of URLs.
fn parse_media_arg(m: &str) -> serde_json::Value {
    if m.trim_start().starts_with('[') {
        serde_json::from_str(m).unwrap_or_else(|_| serde_json::json!([]))
    } else {
        let urls: Vec<String> = m
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        serde_json::json!(urls)
    }
}

fn parse_settings_arg(s: &str) -> anyhow::Result<serde_json::Value> {
    serde_json::from_str(s).map_err(|e| anyhow::anyhow!("Invalid settings JSON: {e}"))
}

async fn handle_posts_update_helper(
    state: &crate::api::AppState,
    user_id: uuid::Uuid,
    id: &str,
    content: Option<String>,
    title: Option<String>,
    media: Option<&str>,
    settings: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let post_id: uuid::Uuid = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid post ID: {id}"))?;
    // The CLI is a CLIENT of the REST surface too (AGENTS.md §9.21), so
    // it edits through the same function: the `Publishing` guard, the
    // three-state fields and the provider validation all apply.
    let body = crate::api::posts::UpdatePostRequest {
        content: content.map(|c| c.to_string()),
        // Three-state: the CLI's `--title` is only sent when the
        // flag is present, so "not given" must mean "keep the
        // stored title" rather than "clear it".
        title: title.map(|t| Some(t.to_string())),
        media: media.map(|m| Some(parse_media_arg(m))),
        settings: match settings {
            Some(s) => Some(Some(parse_settings_arg(s)?)),
            None => None,
        },
        tag_ids: None,
        first_comment: None,
        // The CLI's post-edit flags do not include a campaign, so the
        // stored one is kept: absent = keep.
        campaign_id: None,
        overrides: None,
    };
    let post = crate::api::posts::update_post(state, user_id, post_id, body)
        .await
        .map_err(|e| anyhow::anyhow!("Update failed: {e}"))?;
    Ok(serde_json::json!({
        "ok": true,
        "id": post.id.to_string(),
        "state": post.state,
        "content": post.content,
        "title": post.title,
        "scheduled_at": post.scheduled_at,
    }))
}

async fn handle_posts_tags_helper(
    state: &crate::api::AppState,
    user_id: uuid::Uuid,
    id: &str,
    tag_ids: &[String],
) -> anyhow::Result<serde_json::Value> {
    let post_id: uuid::Uuid = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid post ID: {id}"))?;
    crate::db::queries::get_post(&state.db, post_id, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Post {id} not found"))?;
    let tags: Vec<uuid::Uuid> = tag_ids
        .iter()
        .map(|t| {
            t.parse()
                .map_err(|_| anyhow::anyhow!("Invalid tag ID: {t}"))
        })
        .collect::<anyhow::Result<_>>()?;
    crate::api::verify_tag_ownership(&state.db, &tags, user_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::db::queries::set_post_tags(&state.db, post_id, &tags).await?;
    Ok(serde_json::json!({
        "ok": true,
        "id": post_id.to_string(),
        "tag_ids": tags.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
    }))
}

async fn handle_posts_validate_helper(
    state: &crate::api::AppState,
    user_id: uuid::Uuid,
    integrations: &str,
    content: &str,
    title: Option<&str>,
    media: Option<&str>,
    settings: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let integration_ids: Vec<uuid::Uuid> = integrations
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse()
                .map_err(|_| anyhow::anyhow!("Invalid integration UUID: {s}"))
        })
        .collect::<anyhow::Result<_>>()?;
    if integration_ids.is_empty() {
        return Err(anyhow::anyhow!("At least one integration UUID required"));
    }
    let mut rows = Vec::with_capacity(integration_ids.len());
    for id in &integration_ids {
        let integ = crate::db::queries::get_integration(&state.db, *id, user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Integration {id} not found"))?;
        rows.push(integ);
    }
    let media_json = media.map(parse_media_arg);
    let attachments: Vec<crate::social::MediaAttachment> =
        serde_json::from_value(media_json.clone().unwrap_or(serde_json::json!([])))
            .unwrap_or_default();
    let body = crate::api::CreatePostRequest {
        integration_ids,
        content: content.to_string(),
        title: title.map(String::from),
        media: media_json,
        settings: settings.map(parse_settings_arg).transpose()?,
        scheduled_at: None,
        tag_ids: None,
        first_comment: None,
        sequence: None,
        overrides: None,
        campaign_id: None,
    };
    let errors = crate::api::validate_posts_for_integrations(state, &body, &rows, &attachments);
    Ok(serde_json::json!({
        "valid": errors.is_empty(),
        "errors": errors,
    }))
}

async fn handle_integrations_with_state(
    state: &AppState,
    action: IntegrationsAction,
) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let (id, disabled) = match action {
        IntegrationsAction::Disable { id } => (id, true),
        IntegrationsAction::Enable { id } => (id, false),
    };
    let result = match id.parse::<uuid::Uuid>() {
        Ok(integration_id) => {
            match crate::db::queries::set_integration_disabled(
                &state.db,
                user_id,
                integration_id,
                disabled,
            )
            .await
            {
                Ok(true) => Ok(serde_json::json!({"ok": true, "id": id, "disabled": disabled})),
                Ok(false) => Err(anyhow::anyhow!("Integration {id} not found")),
                Err(e) => Err(anyhow::anyhow!("DB error: {e}")),
            }
        }
        Err(_) => Err(anyhow::anyhow!("Invalid integration ID: {id}")),
    };
    match result {
        Ok(v) => output_json(&v),
        Err(e) => return output_error(&e.to_string()),
    }
    Ok(())
}

async fn handle_posts_with_state(state: &AppState, action: PostsAction) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;

    let result: anyhow::Result<serde_json::Value> = match action {
        PostsAction::List {
            state: post_state,
            limit,
            offset,
        } => {
            let limit = limit.min(200) as i64;
            let offset = offset as i64;
            match crate::services::posts::PostService::list(
                &state.db,
                user_id,
                post_state.as_deref(),
                limit,
                offset,
                true,
            )
            .await
            {
                Ok((posts, total)) => {
                    let summaries: Vec<_> = posts
                        .iter()
                        .map(|p| {
                            let mut val = serde_json::json!({
                                "id": p.id,
                                "content": p.content,
                                "state": p.state,
                                "title": p.title,
                                "integration_id": p.integration_id,
                                "scheduled_at": p.scheduled_at.map(|dt| dt.to_rfc3339()),
                                "published_at": p.published_at.map(|dt| dt.to_rfc3339()),
                                "platform_post_id": p.platform_post_id,
                                "platform_post_url": p.platform_post_url,
                                "error_message": p.error_message,
                                "first_comment": p.first_comment,
                                "media": p.media,
                                "settings": p.settings,
                                "created_at": p.created_at.to_rfc3339(),
                                "updated_at": p.updated_at.to_rfc3339(),
                            });
                            // AXI §3: truncate long strings in list view
                            val = truncate_json_strings(&val, 500);
                            val
                        })
                        .collect();
                    Ok(serde_json::json!({
                        "total": total.unwrap_or(0),
                        "limit": limit,
                        "offset": offset,
                        "count": summaries.len(),
                        "hint": if total.unwrap_or(0) > limit { Some(format!("Showing {} of {} posts. Use --offset {} for next page.", summaries.len(), total.unwrap_or(0), offset + limit)) } else { None },
                        "posts": summaries,
                    }))
                }
                Err(e) => Err(anyhow::anyhow!("DB error: {e}")),
            }
        }
        PostsAction::Get { id } => match id.parse::<uuid::Uuid>() {
            Ok(post_id) => match crate::db::queries::get_post(&state.db, post_id, user_id).await {
                Ok(Some(p)) => Ok(serde_json::json!({
                    "id": p.id,
                    "content": p.content,
                    "state": p.state,
                    "title": p.title,
                    "integration_id": p.integration_id,
                    "scheduled_at": p.scheduled_at.map(|dt| dt.to_rfc3339()),
                    "published_at": p.published_at.map(|dt| dt.to_rfc3339()),
                    "platform_post_id": p.platform_post_id,
                    "platform_post_url": p.platform_post_url,
                    "error_message": p.error_message,
                    "first_comment": p.first_comment,
                    "media": p.media,
                    "settings": p.settings,
                    "created_at": p.created_at.to_rfc3339(),
                    "updated_at": p.updated_at.to_rfc3339(),
                })),
                Ok(None) => Err(anyhow::anyhow!("Post {id} not found")),
                Err(e) => Err(anyhow::anyhow!("DB error: {e}")),
            },
            Err(_) => Err(anyhow::anyhow!("Invalid post ID: {id}")),
        },
        PostsAction::Create {
            content,
            integrations,
            schedule,
            title,
            media,
            first_comment,
            settings,
            state: state_override,
        } => {
            handle_posts_create_helper(
                state,
                user_id,
                &content,
                &integrations,
                schedule.as_deref(),
                title.as_deref(),
                media.as_deref(),
                first_comment.as_deref(),
                settings.as_deref(),
                state_override.as_deref(),
            )
            .await
        }
        PostsAction::Schedule { id, scheduled_at } => {
            let post_id = match id.parse::<uuid::Uuid>() {
                Ok(id) => id,
                Err(_) => return output_error("Invalid post ID"),
            };
            let dt = chrono::DateTime::parse_from_rfc3339(&scheduled_at)
                .map_err(|e: chrono::ParseError| anyhow::anyhow!("Invalid scheduled_at: {}", e))?
                .with_timezone(&chrono::Utc);
            crate::services::posts::PostService::schedule(
                &state.db,
                &state.broadcast,
                user_id,
                post_id,
                dt,
            )
            .await
            .map(|post| {
                serde_json::json!({
                    "ok": true,
                    "id": post.id.to_string(),
                    "state": post.state.to_string(),
                    "scheduled_at": post.scheduled_at.map(|d| d.to_rfc3339()),
                })
            })
            .map_err(|e| anyhow::anyhow!("Schedule failed: {e}"))
        }
        PostsAction::Publish { id } => {
            let post_id = match id.parse::<uuid::Uuid>() {
                Ok(id) => id,
                Err(_) => return output_error("Invalid post ID"),
            };
            crate::services::posts::PostService::publish(
                &state.db,
                &state.providers,
                &state.broadcast,
                &state.config,
                user_id,
                post_id,
                state.token_key,
            )
            .await
            .map(|platform_url| {
                serde_json::json!({
                    "ok": true,
                    "id": id,
                    "state": "published",
                    "platform_post_url": platform_url,
                })
            })
            .map_err(|e| anyhow::anyhow!("Publish failed: {e}"))
        }
        PostsAction::Delete { id } => {
            let post_id = match id.parse::<uuid::Uuid>() {
                Ok(id) => id,
                Err(_) => return output_error("Invalid post ID"),
            };
            crate::services::posts::PostService::delete(
                &state.db,
                &state.broadcast,
                user_id,
                post_id,
            )
            .await
            .map(|deleted| serde_json::json!({"ok": deleted, "id": id}))
            .map_err(|e| anyhow::anyhow!("Delete failed: {e}"))
        }
        PostsAction::FindSlot { integration } => {
            let integration_id = integration
                .as_deref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            crate::services::posts::PostService::find_slot(&state.db, user_id, integration_id)
                .await
                .map(|slot| {
                    serde_json::json!({
                        "ok": true,
                        "scheduled_at": slot.to_rfc3339(),
                        "integration_id": integration,
                    })
                })
                .map_err(|e| anyhow::anyhow!("Find-slot failed: {e}"))
        }
        PostsAction::Update {
            id,
            content,
            title,
            media,
            settings,
        } => {
            handle_posts_update_helper(
                state,
                user_id,
                &id,
                content,
                title,
                media.as_deref(),
                settings.as_deref(),
            )
            .await
        }
        PostsAction::Tags { id, tag_ids } => {
            handle_posts_tags_helper(state, user_id, &id, &tag_ids).await
        }
        PostsAction::Validate {
            content,
            integrations,
            title,
            media,
            settings,
        } => {
            handle_posts_validate_helper(
                state,
                user_id,
                &integrations,
                &content,
                title.as_deref(),
                media.as_deref(),
                settings.as_deref(),
            )
            .await
        }
    };

    match result {
        Ok(v) => output_json(&v),
        Err(e) => return output_error(&e.to_string()),
    }
    Ok(())
}

// Split Preview Handler
fn handle_split_preview(text: &str, platforms: Option<&str>) -> anyhow::Result<()> {
    crate::config::load_dotenv();
    let providers = ProviderRegistry::new(&Config::from_env()?, None, None);
    let provider_list: Vec<&str> = match platforms {
        Some(p) => p
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect(),
        None => vec![
            "x",
            "bluesky",
            "linkedin",
            "reddit",
            "mastodon",
            "facebook",
            "instagram",
            "youtube",
        ],
    };
    let preview: Vec<serde_json::Value> = provider_list.iter().map(|&provider| {
        let limit = providers.get(provider).map_or(0, |p| p.max_content_length());
        let segments = crate::services::content_splitter::split_content(text, provider, limit);
        let needs_split = crate::services::content_splitter::needs_splitting(text, limit);
        serde_json::json!({
            "provider": provider, "char_limit": limit, "content_length": text.len(), "needs_splitting": needs_split, "segments": segments.len(),
            "posts": segments.iter().map(|s| serde_json::json!({"sequence": s.sequence, "total": s.total, "content": s.content, "char_count": s.content.len()})).collect::<Vec<_>>(),
        })
    }).collect();
    output_json(&serde_json::json!({"content_length": text.len(), "platforms": preview}));
    Ok(())
}

// ── Main CLI Dispatcher ──────────────────────────────────────

pub async fn run_cli(cli: Cli) -> anyhow::Result<()> {
    // The global --json/--pretty flags select the output format for every
    // command. Set them BEFORE any handler runs.
    set_output_format(cli.json, cli.pretty);

    // Commands that don't need AppState — handle early, skip init_state().
    match cli.command {
        Command::Serve { .. } | Command::Mcp => {
            return Err(anyhow::anyhow!(
                "Serve/Mcp are handled in main.rs before calling run_cli — this is a bug"
            ));
        }
        Command::Init => return handle_init(),
        Command::Update(options) => return crate::cli::update::run(options, cli.json).await,
        Command::Service { action } => {
            // No AppState needed — this manages the systemd unit only.
            return match action {
                crate::cli::ServiceAction::Install => handle_service_install(),
                crate::cli::ServiceAction::Uninstall => handle_service_uninstall(),
            };
        }
        Command::Config { action } => return handle_config(action),
        Command::SplitPreview { text, platforms } => {
            return handle_split_preview(&text, platforms.as_deref());
        }
        _ => {}
    }

    // Initialize AppState once for all commands that need it.
    let state = init_state().await?;

    match cli.command {
        Command::Init
        | Command::Update(_)
        | Command::Config { .. }
        | Command::Service { .. }
        | Command::SplitPreview { .. }
        | Command::Serve { .. }
        | Command::Mcp => {
            unreachable!("handled above")
        }

        Command::Providers => handle_providers_with_state(&state).await,
        Command::McpCall { tool, args } => handle_mcp_call_with_state(&state, &tool, &args).await,
        Command::McpTools { pretty, toolset } => {
            handle_mcp_tools_with_state(&state, pretty, toolset.as_deref()).await
        }
        Command::Audit => handle_audit_with_state(&state).await,
        Command::Connect {
            provider,
            wait,
            timeout_secs,
            browser,
            browser_endpoint,
        } => {
            if browser {
                handle_connect_browser(&state, &provider, &browser_endpoint, timeout_secs).await
            } else if wait {
                handle_connect_wait(&state, &provider, timeout_secs).await
            } else {
                handle_connect_with_state(&state, &provider).await
            }
        }
        Command::Autopost(options) => handle_autopost(&state, &options).await,
        Command::Agent => handle_agent(&state).await,
        Command::Doctor => handle_doctor_with_state(&state).await,
        Command::Setup => handle_setup().await,
        Command::ConnectAll => handle_connect_all_with_state(&state).await,

        // ── Core Platform Handlers ──────────────────────────
        Command::X { action } => crate::cli::platforms::x::handle(action, &state).await,
        Command::Reddit { action } => crate::cli::platforms::reddit::handle(action, &state).await,
        Command::Linkedin { action } => {
            crate::cli::platforms::linkedin::handle(action, &state).await
        }
        Command::LinkedinPage { action } => {
            crate::cli::platforms::linkedin_page::handle(action, &state).await
        }
        Command::Facebook { action } => {
            crate::cli::platforms::facebook::handle(action, &state).await
        }
        Command::Instagram { action } => {
            crate::cli::platforms::instagram::handle(action, &state).await
        }
        Command::Youtube { action } => crate::cli::platforms::youtube::handle(action, &state).await,
        Command::Bluesky { action } => crate::cli::platforms::bluesky::handle(action, &state).await,
        Command::Mastodon { action } => {
            crate::cli::platforms::mastodon::handle(action, &state).await
        }

        // ── Unified Commands ────────────────────────────────
        Command::Import { provider, count } => {
            handle_import_with_state(&state, &provider, count).await
        }
        Command::Feed { provider, limit } => {
            handle_feed_with_state(&state, provider.as_deref(), limit).await
        }
        Command::Comment { action } => handle_comment_with_state(&state, action).await,
        Command::Rss { action } => handle_rss_with_state(&state, action).await,
        Command::Campaigns { action } => handle_campaigns_with_state(&state, action).await,
        Command::Signatures { action } => handle_signatures_with_state(&state, action).await,
        Command::Ai { action } => handle_ai_with_state(&state, action).await,
        Command::Streak { tz } => {
            use crate::mcp::tools_analytics as an;
            let input = an::StreakGetInput { tz };
            let value = an::handle_streak_get(&state, &input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            output_json(&truncate_json_strings(&value.0, 500));
            Ok(())
        }
        Command::Dm { action } => handle_dm_with_state(&state, action).await,
        Command::Automation { action } => handle_automation_with_state(&state, action).await,
        Command::Posts { action } => handle_posts_with_state(&state, action).await,
        Command::Integrations { action } => handle_integrations_with_state(&state, action).await,
        Command::Post {
            text,
            platforms,
            media,
            schedule,
            first_comment,
            settings,
        } => {
            handle_post_with_state(
                &state,
                &text,
                platforms.as_deref(),
                media.as_deref(),
                schedule.as_deref(),
                first_comment.as_deref(),
                settings.as_deref(),
            )
            .await
        }
        Command::Stage {
            text,
            integrations,
            platforms,
            media,
            schedule,
            preview,
            first_comment,
            settings,
        } => {
            handle_stage_with_state(
                &state,
                &text,
                integrations.as_deref(),
                platforms.as_deref(),
                media.as_deref(),
                schedule.as_deref(),
                preview,
                first_comment.as_deref(),
                settings.as_deref(),
            )
            .await
        }
        Command::Carousel {
            text,
            integration,
            media,
            title,
            schedule,
        } => {
            handle_carousel_with_state(
                &state,
                &text,
                &integration,
                &media,
                title.as_deref(),
                schedule.as_deref(),
            )
            .await
        }
        Command::Media { action } => handle_media_with_state(&state, action).await,

        // ── New Platform Handlers (delegate to platform modules) ──
        Command::Tiktok { action } => crate::cli::platforms::tiktok::handle(action, &state).await,
        Command::Threads { action } => crate::cli::platforms::threads::handle(action, &state).await,
        Command::Discord { action } => crate::cli::platforms::discord::handle(action, &state).await,
        Command::Slack { action } => crate::cli::platforms::slack::handle(action, &state).await,
        Command::TelegramBot { action } => {
            crate::cli::platforms::telegram_bot::handle(action, &state).await
        }
        Command::TelegramUser { action } => {
            crate::cli::platforms::telegram_user::handle(action, &state).await
        }
        Command::Whatsapp { action } => {
            crate::cli::platforms::whatsapp::handle(action, &state).await
        }
        Command::Pinterest { action } => {
            crate::cli::platforms::pinterest::handle(action, &state).await
        }
        Command::Github { action } => crate::cli::platforms::github::handle(action, &state).await,
        Command::Wordpress { action } => {
            crate::cli::platforms::wordpress::handle(action, &state).await
        }
        Command::Hashnode { action } => {
            crate::cli::platforms::hashnode::handle(action, &state).await
        }
        Command::MediumBlog { action } => {
            crate::cli::platforms::medium_blog::handle(action, &state).await
        }
        Command::Devto { action } => crate::cli::platforms::devto::handle(action, &state).await,
        Command::Skool { action } => crate::cli::platforms::skool::handle(action, &state).await,
        Command::Google { action } => crate::cli::platforms::google::handle(action, &state).await,
        Command::Gdrive { action } => crate::cli::platforms::drive::handle(action, &state).await,
        Command::Gcal { action } => crate::cli::platforms::gcal::handle(action, &state).await,
        Command::GmailOps { action } => crate::cli::platforms::gmail::handle(action, &state).await,
        Command::Webhooks { action } => {
            crate::cli::platforms::webhooks::handle(action, &state).await
        }
        Command::Notifications { action } => {
            crate::cli::platforms::notifications::handle(action, &state).await
        }
        Command::Tags { action } => crate::cli::platforms::tags::handle(action, &state).await,
        Command::Analytics { action } => {
            crate::cli::platforms::analytics::handle(action, &state).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The 500-char clip is the one behaviour agents depend on but cannot
    // observe from the value alone, so it gets a check.
    #[test]
    fn truncate_marks_clipped_fields_with_total_length() {
        let val = serde_json::json!({
            "body": "a".repeat(600),
            "title": "short",
            "nested": {"inner": "b".repeat(700)},
        });
        let out = truncate_json_strings(&val, 500);
        assert_eq!(
            out["body"].as_str().unwrap(),
            format!("{}... (truncated, 600 chars total)", "a".repeat(500))
        );
        assert_eq!(out["title"], "short");
        assert!(out["nested"]["inner"]
            .as_str()
            .unwrap()
            .ends_with("(truncated, 700 chars total)"));
    }

    #[test]
    fn format_text_switches_between_toon_and_json() {
        let val = serde_json::json!({"key": "value"});
        assert!(format_text(&val, "json").starts_with('{'));
        assert!(!format_text(&val, "toon").starts_with('{'));
    }

    // The platform shims' emit_result hardcoded TOON, so every
    // `switchpost x timeline`-style command silently ignored the
    // global --json flag. This drives the REAL render path the shims
    // print from and fails if it stops honoring the flags.
    #[test]
    fn platform_shim_output_honors_json_flag() {
        // OUTPUT_FORMAT is a process-global atomic and lib tests run in
        // parallel threads — snapshot and restore so this test cannot
        // change what the other format tests observe mid-run.
        let saved = current_output_format();
        set_output_format(true, false);
        let ok_text = crate::cli::platforms::render_emit(&Ok(serde_json::json!({"a": 1})));
        assert!(
            ok_text.starts_with('{'),
            "--json must reach platform shims, got: {ok_text}"
        );
        let err_text = crate::cli::platforms::render_emit::<serde_json::Value>(&Err("boom".into()));
        assert!(
            err_text.starts_with('{'),
            "--json must reach error arm, got: {err_text}"
        );
        set_output_format(false, true);
        let pretty_text = crate::cli::platforms::render_emit(&Ok(serde_json::json!({"a": 1})));
        assert!(
            pretty_text.starts_with('{'),
            "--pretty must reach platform shims"
        );
        set_output_format(false, false);
        let toon_text = crate::cli::platforms::render_emit(&Ok(serde_json::json!({"a": 1})));
        assert!(!toon_text.starts_with('{'), "default must stay TOON");
        OUTPUT_FORMAT.store(saved, std::sync::atomic::Ordering::Relaxed);
    }

    // An OAuth provider's auth_url must be the PROVIDER's own consent
    // page. The old shape emitted `{app_url}/api/public/connect/{p}`,
    // a server route gated by the browser's session cookie — an agent
    // driving its own browser hit a 401 there and never reached the
    // platform. The redirect URI is the one the provider's app must
    // have registered; a trailing slash in APP_URL would produce a
    // double slash that the provider rejects as a mismatch.
    #[test]
    fn oauth_redirect_uri_is_the_callback_route() {
        assert_eq!(
            oauth_redirect_uri_for("https://localhost:6544"),
            "https://localhost:6544/api/auth/callback"
        );
        assert_eq!(
            oauth_redirect_uri_for("https://localhost:6544/"),
            "https://localhost:6544/api/auth/callback"
        );
    }
}

/// `connect --browser`: drive the platform's consent page in the
/// operator's real browser and complete the login end to end.
async fn handle_connect_browser(
    state: &AppState,
    provider: &str,
    endpoint: &str,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let result =
        crate::cli::oauth_browser::drive_oauth_login(state, provider, endpoint, timeout_secs)
            .await?;
    output_json(&serde_json::json!({
        "status": "connected",
        "provider": result.provider,
        "account": result.account_name,
        "internal_id": result.internal_id,
        "final_url": result.final_url,
        "hint": "The credential is stored. Verify with 'switchpost providers'.",
    }));
    Ok(())
}

/// `autopost`: render a Deckmill carousel, upload it, and stage (or
/// publish) it on the given platforms.
async fn handle_autopost(state: &AppState, options: &super::AutopostOptions) -> anyhow::Result<()> {
    // 1. The slide specs — Deckmill's own contract, read verbatim.
    let raw = tokio::fs::read_to_string(&options.slides)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", options.slides))?;
    let specs: Vec<serde_json::Value> = serde_json::from_str(&raw).map_err(|e| {
        anyhow::anyhow!("{} is not a JSON array of slide specs: {e}", options.slides)
    })?;

    // 2. The target integrations.
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;
    let integration_ids = resolve_platform_integrations(integrations.iter(), &options.platforms);
    if integration_ids.is_empty() {
        return Err(anyhow::anyhow!(
            "No connected integration for '{}'. Run 'switchpost providers' to see what is connected.",
            options.platforms
        ));
    }

    // 3. Render. Deckmill requires all four overlay fields non-empty
    // (its own error names each); with the CLI's "" defaults the
    // failure surfaced mid-render as a deckmill error string. Fail
    // here with the flags named instead.
    let missing = [
        ("--brand-name", options.brand_name.as_str()),
        ("--topic", options.topic.as_str()),
        ("--url", options.url.as_str()),
        ("--hashtags", options.hashtags.as_str()),
    ]
    .iter()
    .filter(|(_, v)| v.trim().is_empty())
    .map(|(flag, _)| *flag)
    .collect::<Vec<_>>()
    .join(", ");
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "autopost: {missing} must be non-empty (Deckmill renders each as a slide overlay). Add them and retry."
        ));
    }
    let work_dir = match &options.work_dir {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::temp_dir().join(format!("switchpost-autopost-{}", uuid::Uuid::new_v4())),
    };
    let rendered = crate::cli::autopost::render_carousel(
        &specs,
        &options.preset,
        &options.aspect_ratio,
        &options.primary_color,
        &options.brand_name,
        &options.brand_handle,
        &options.topic,
        &options.url,
        &options.hashtags,
        &work_dir,
    )
    .await?;

    // 4. Upload.
    let media = crate::cli::autopost::upload_slides(state, &rendered.slides).await?;

    // 5. Stage.
    let scheduled_at = match &options.schedule {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|e| anyhow::anyhow!("Invalid --schedule timestamp '{raw}': {e}"))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    // A recurring series needs a first due time to count from.
    let repeat_days = options.repeat_days;
    if repeat_days.is_some() && options.repeat_until.is_none() {
        return Err(anyhow::anyhow!("--repeat-days also needs --repeat-until"));
    }
    if let Some(days) = repeat_days {
        if days <= 0 {
            return Err(anyhow::anyhow!("--repeat-days must be positive"));
        }
        if scheduled_at.is_none() {
            return Err(anyhow::anyhow!(
                "--repeat-days also needs --schedule (the series counts from the first due time)"
            ));
        }
    }
    let settings = parse_cli_settings(options.settings.as_deref())?;
    let overrides = serde_json::json!({});
    let staged = crate::cli::autopost::stage(
        state,
        &options.content,
        media,
        integration_ids,
        scheduled_at,
        settings,
        overrides,
    )
    .await?;

    // 6. Publish now, if asked.
    let mut published = Vec::new();
    if options.publish {
        for post in &staged.staged {
            match crate::services::posts::PostService::publish(
                &state.db,
                &state.providers,
                &state.broadcast,
                &state.config,
                user_id,
                post.post_id,
                state.token_key,
            )
            .await
            {
                Ok(url) => published.push(serde_json::json!({
                    "post_id": post.post_id.to_string(),
                    "provider": post.provider,
                    "url": url,
                })),
                Err(e) => published.push(serde_json::json!({
                    "post_id": post.post_id.to_string(),
                    "provider": post.provider,
                    "error": e,
                })),
            }
        }
    }

    // 7. Recurrence — the DB-backed series, not a loop process.
    //
    // The scheduler publishes the spaced copies; nothing stays running.
    // Reusing `set_post_recurring_with_copies` means the series is
    // visible in the dashboard, respects the single-writer rule, and
    // cannot double-publish against another scheduler.
    let mut recurring = serde_json::Value::Null;
    if let (Some(days), Some(until_raw)) = (repeat_days, options.repeat_until.as_ref()) {
        let until = chrono::DateTime::parse_from_rfc3339(until_raw)
            .map_err(|e| anyhow::anyhow!("Invalid --repeat-until timestamp '{until_raw}': {e}"))?
            .with_timezone(&chrono::Utc);
        let first = scheduled_at.expect("checked above");
        if until <= first {
            return Err(anyhow::anyhow!("--repeat-until must be after --schedule"));
        }
        let max_copies = ((until - first).num_days() / days as i64) + 1;
        if max_copies > 100 {
            return Err(anyhow::anyhow!(
                "--repeat-days x --repeat-until would create {max_copies} posts (the limit is 100)"
            ));
        }
        let mut series = Vec::new();
        for staged in &staged.staged {
            let (post_ids, dates) = crate::db::queries::set_post_recurring_with_copies(
                &state.db,
                staged.post_id,
                user_id,
                days,
                &until,
                staged.group_id,
                &first,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create the recurring series: {e}"))?;
            series.push(serde_json::json!({
                "post_id": staged.post_id.to_string(),
                "copies": post_ids.len(),
                "copy_ids": post_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                "scheduled_dates": dates,
            }));
        }
        recurring = serde_json::json!({
            "interval_days": days,
            "until": until.to_rfc3339(),
            "series": series,
            "note": "The scheduler publishes the copies; no loop process is left running.",
        });
    }

    output_json(&serde_json::json!({
        "status": if options.publish { "published" } else { "staged" },
        "slides": rendered.slides.len(),
        "html": rendered.html.to_string_lossy(),
        "group_id": staged.group_id.map(|g| g.to_string()),
        "staged": staged.staged.iter().map(|s| serde_json::json!({
            "post_id": s.post_id.to_string(),
            "provider": s.provider,
            "state": s.state,
        })).collect::<Vec<_>>(),
        "total_posts": staged.total_posts,
        "warnings": staged.warnings,
        "errors": staged.errors,
        "published": published,
        "recurring": recurring,
    }));
    Ok(())
}

/// `agent`: one command that tells an AI agent what it can do here —
/// the commands, the MCP tools, and the live connection state.
async fn handle_agent(state: &AppState) -> anyhow::Result<()> {
    let user_id = resolve_user(state).await?;
    let integrations = crate::db::queries::list_integrations(&state.db, user_id).await?;

    let connected: Vec<_> = integrations
        .iter()
        .filter(|i| is_credential_row(i))
        .map(|i| {
            serde_json::json!({
                "provider": i.provider_identifier,
                "name": i.profile_name,
                "internal_id": i.internal_id,
            })
        })
        .collect();

    // Which providers could be connected, and how each one authenticates.
    let providers: Vec<_> = state
        .providers
        .list()
        .into_iter()
        .map(|id| {
            let provider = state.providers.get(id);
            serde_json::json!({
                "id": id,
                "tier": state.providers.tier(id).as_str(),
                "oauth": provider.as_ref().map(|p| p.uses_oauth()).unwrap_or(false),
                "one_time_token": provider.as_ref().map(|p| p.one_time_token()).unwrap_or(false),
                "connected": integrations
                    .iter()
                    .any(|i| i.provider_identifier == id && is_credential_row(i)),
            })
        })
        .collect();

    output_json(&serde_json::json!({
        "commands": [
            "connect <provider> --browser      drive the OAuth consent page in a real browser",
            "connect <provider> --wait        poll until the credential lands",
            "autopost --slides f.json --platforms x --content '...'   render a Deckmill carousel and post it",
            "posts list|create|publish         the post lifecycle",
            "media upload <path>               upload a file",
            "mcp-call <tool> --args '<json>'  call any MCP tool in-process",
            "mcp-tools                         list the MCP tools",
            "providers                         the connected accounts",
            "doctor                            health-check every provider",
        ],
        "workflows": {
            "oauth_login": "switchpost connect linkedin --browser",
            "deckmill_autopost": "switchpost autopost --slides slides.json --platforms instagram --brand-name 'Acme' --topic 'Release' --content '...' --url https://example.com --hashtags ai",
        },
        "providers": providers,
        "connected": connected,
    }));
    Ok(())
}

#[cfg(test)]
mod tests_cli {
    use super::*;

    // ── G-10: --platforms must not fan out to a colliding variant ───────────

    fn fake_integration(provider: &str) -> crate::db::models::Integration {
        crate::db::models::Integration {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            provider_identifier: provider.to_string(),
            provider_name: provider.to_string(),
            internal_id: provider.to_string(),
            access_token: "{}".to_string(),
            refresh_token: None,
            token_expires_at: None,
            profile_name: None,
            profile_picture: None,
            profile_url: None,
            disabled: false,
            refresh_needed: false,
            root_internal_id: None,
            posting_times: serde_json::json!({}),
            auth_method: "test".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn providers(integrations: &[crate::db::models::Integration], platforms: &str) -> Vec<String> {
        let mut out: Vec<String> = resolve_platform_integrations(integrations.iter(), platforms)
            .into_iter()
            .map(|id| {
                integrations
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.provider_identifier.clone())
                    .unwrap_or_default()
            })
            .collect();
        out.sort();
        out
    }

    /// The double-draft regression: `instagram` must NOT also select
    /// `instagram-standalone`, and `linkedin` must NOT also select
    /// `linkedin-page`, even with both connected.
    #[test]
    fn platforms_exact_match_does_not_sweep_the_variant() {
        let igs = [
            fake_integration("instagram"),
            fake_integration("instagram-standalone"),
        ];
        assert_eq!(providers(&igs, "instagram"), vec!["instagram".to_string()]);

        let lis = [
            fake_integration("linkedin"),
            fake_integration("linkedin-page"),
        ];
        assert_eq!(providers(&lis, "linkedin"), vec!["linkedin".to_string()]);

        // Every documented collision pair behaves the same way.
        for (base, variant) in PLATFORM_COLLISIONS {
            let both = [fake_integration(base), fake_integration(variant)];
            assert_eq!(
                providers(&both, base),
                vec![base.to_string()],
                "--platforms {base} must target only {base}"
            );
            // Naming the variant explicitly still works.
            assert_eq!(
                providers(&both, &format!("{base},{variant}")).len(),
                2,
                "explicit '{base},{variant}' must target both"
            );
        }
    }

    /// The prefix fallback must survive for names with NO exact match —
    /// otherwise `--platforms telegram` reaches nothing.
    #[test]
    fn platforms_prefix_fallback_still_fans_out_when_no_exact_match() {
        let tgs = [
            fake_integration("telegram-bot"),
            fake_integration("telegram-user"),
        ];
        assert_eq!(providers(&tgs, "telegram").len(), 2);
    }

    /// A prefix match must not leak across an unrelated request: only the
    /// requested platform is selected.
    #[test]
    fn platforms_selection_is_scoped_to_the_request() {
        let all = [
            fake_integration("x"),
            fake_integration("bluesky"),
            fake_integration("reddit"),
        ];
        assert_eq!(providers(&all, "x"), vec!["x".to_string()]);
        assert_eq!(providers(&all, "x,bluesky").len(), 2);
        assert!(
            providers(&all, "tiktok").is_empty(),
            "no match → no targets"
        );
    }

    /// Whitespace and empty entries in the list must not create blank targets.
    #[test]
    fn platforms_tolerates_whitespace_and_empty_entries() {
        let all = [fake_integration("x"), fake_integration("bluesky")];
        assert_eq!(providers(&all, " x , , bluesky ").len(), 2);
        assert!(providers(&all, " , ").is_empty());
    }

    // ── The three-state update shape ───────────────────────────────────────

    /// `--field value` replaces, `--clear-field` clears, neither keeps.
    ///
    /// This is the difference between `Option<T>` and `Option<Option<T>>`:
    /// a two-state field cannot express "leave it alone" AND "clear it".
    #[test]
    fn a_value_replaces_and_a_clear_clears() {
        assert_eq!(
            double_state(Some("new".into()), false),
            Some(Some("new".into()))
        );
        assert_eq!(double_state(None, true), Some(None));
    }

    /// Neither flag means "keep" — the field is absent from the request.
    #[test]
    fn neither_value_nor_clear_keeps_the_field() {
        assert_eq!(double_state(None, false), None);
    }
}
