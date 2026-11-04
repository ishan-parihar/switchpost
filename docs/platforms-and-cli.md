# Platforms and CLI

Moved out of the README to keep it short. Content is unchanged.

## Contents

- [Targeting: `--platforms` collisions and `--integrations`](#targeting---platforms-collisions-and---integrations)
- [Single-writer rule](#single-writer-rule)
- [Logging a provider in (for AI agents)](#logging-a-provider-in-for-ai-agents)
- [Toolsets and the shell path](#toolsets-and-the-shell-path)
- [Updating the binary](#updating-the-binary)
- [Supported Platforms](#supported-platforms)
- [Project Structure](#project-structure)

### Targeting: `--platforms` collisions and `--integrations`

Two registered providers are a strict prefix of another, so platform **names are
matched exactly**:

| `--platforms` value | Targets | Does NOT also target |
|---|---|---|
| `instagram` | `instagram` | `instagram-standalone` |
| `linkedin` | `linkedin` | `linkedin-page` |

```bash
# Exact names only — no double drafts
switchpost post "hello" --platforms instagram

# Name a suffixed variant explicitly to include it
switchpost post "hello" --platforms instagram,instagram-standalone

# Or skip name resolution entirely and pick exact accounts
switchpost mcp-call integrations_list --args '{}'   # get integration UUIDs (id field)
switchpost stage "hello" --integrations 6f1c...-...     # mutually exclusive with --platforms
```

`telegram` still fans out to `telegram-bot` and `telegram-user`, because no
integration is registered under the bare name `telegram`.

### Single-writer rule

SQLite admits one writer per file. `serve` and `mcp` both run the publish
scheduler, so exactly one long-lived process may own a given `DATABASE_URL` —
the second one fails at boot on an exclusive lock file rather than silently
double-publishing. Short CLI reads (`providers`, `posts list`, `audit`) never
take the lock. `switchpost audit` reports the live `journal_mode` /
`busy_timeout` alongside the disabled-integration inventory.

Full detail: [`docs/planning/TARGETING_AND_WRITER_DISCIPLINE.md`](planning/TARGETING_AND_WRITER_DISCIPLINE.md).

### Logging a provider in (for AI agents)

`switchpost connect <provider>` is the one entry point. Three shapes:

|Provider kind|What `connect` does|What the agent does next|
|---|---|---|
|Cookie (x, reddit)|Imports browser cookies, then falls back to `X_AUTH_TOKEN`+`X_CT0` / `REDDIT_COOKIE_TOKEN` env; validates against the live API; writes the row|Nothing — it either prints `connected` or names what is missing|
|Token env (github, telegram-bot)|Validates the env token against the live API (`GET /user`, `getMe`) and writes the row NOW|Nothing — `connected`, or the exact validation error|
|Session env (bluesky)|Creates a real session with the app password (`createSession`), stores the accessJwt + did|Nothing — `connected`, or the exact error|
|OAuth (linkedin, facebook, instagram, threads, discord, slack, pinterest, tiktok, mastodon, youtube, google)|Emits a **provider-native** `auth_url` (the platform's own consent page, e.g. `https://www.threads.net/oauth/authorize?...`) when the client credentials are configured; `not_configured` + `requires` when they are not|Opens the URL in a browser, completes the platform login; the provider redirects to `/api/auth/callback` and the server writes the row|
|One-time-token (telegram user)|Prints the code + instructions|Sends the code to the bot|
|QR scan (telegram-user)|`qr_login_required` + the env names|Scans the QR in the dashboard's Telegram connect flow|
|Per-site / QR / extension (wordpress, whatsapp, skool)|Names the exact web-UI path|Follows it in the dashboard|

**The URL is usable with NO session cookie.** The CLI mints the
authorization URL itself (code_verifier + state + `oauth_states` row +
the provider's `generate_auth_url`), exactly as the web UI does. An agent
driving its own browser goes straight to the platform's consent page —
it never needs the founder's `sf_session` cookie, a `?token=` JWT, or a
popup. The provider redirects to `{APP_URL}/api/auth/callback`, an
auth-exempt route, and the server exchanges the code there.

**The wait loop.** `connect <provider> --wait [--timeout-secs N]` (default
300) turns the OAuth/OTT shapes into a single blocking command: it prints
the URL (or code), then polls every 2s and exits the moment the credential
row lands — `status: connected` and exit 0. Agent loop, end to end:

```sh
switchpost connect linkedin --wait &     # emits auth_url, polls
# drive a browser to the auth_url, complete the login
# -> the CLI prints: status: connected, accounts: [...]
```

`--wait` prints the `auth_url` BEFORE it starts polling, so the agent
never has to run the command twice.

**`redirect_uri` is printed alongside the URL.** When a platform answers
`redirect_uri_mismatch`, the URI the provider's developer app must have
registered is in the CLI's own output — no guessing:

```sh
switchpost connect linkedin --json
# {"status":"not_connected","auth_url":"https://www.linkedin.com/oauth/v2/authorization?...",
#  "redirect_uri":"https://localhost:6544/api/auth/callback", ...}
```

**Output format.** TOON is the default (~40% fewer tokens for an agent).
`--json` emits compact JSON and `--pretty` emits indented JSON — both
are global flags and apply to every command.

### Toolsets and the shell path

An MCP session boots with the `core` toolset only, so a session that
touches one platform does not pay the full tool-schema cost. The filter
is on the ADVERTISED schema (`tools/list`), not on capability.

|Shell command|Effect of the session filter|
|---|---|
|`switchpost mcp-tools`|Lists every tool (389 at the time of writing)|
|`switchpost mcp-tools --toolset x`|Lists the `x` set, plus the two meta tools|
|`switchpost mcp-tools --toolset all`|Lists every tool — `all` means no filtering|
|`switchpost mcp-call <tool> --args '<json>'`|Reaches EVERY tool, filtered or not|

`mcp-call` names one tool explicitly and pays no `tools/list` cost, so
the filter does not apply to it. A live `switchpost mcp` session DOES
need `tools_enable` — that is how a toolset is turned on there.

The membership rule is one function (`tool_in_toolset`): `all` keeps
everything, the meta tools (`tools_search`, `tools_enable`) are always
present, and otherwise the tool's own set must match. The session
router, `tools_search`'s `enabled` flag and the CLI all call it, so a
shell listing and a live session never disagree.

Seed rows never count as connected: a `seed-token` placeholder (from
`scripts/seed_demo.py`) is treated as absent, so `connect` re-imports and
replaces it instead of answering `already_connected` forever.

#### Browser-driven OAuth: `connect <provider> --browser`

`--browser` completes the WHOLE login — the agent does not have to read a
URL, drive a browser by hand, or poll:

```sh
switchpost connect linkedin --browser --pretty
# -> {"status":"connected","provider":"linkedin","account":"…",...}
```

What it does, in order:

1. Mints the provider's authorization URL (state + code_verifier stored in
   `oauth_states`) — the same URL the web UI mints.
2. Opens it in the operator's real browser through **browseros-neo**
   (`http://127.0.0.1:9012/mcp` by default; override with
   `--browser-endpoint`). The browser may already hold the platform
   session, so most providers skip the login form entirely.
3. Waits for the provider's redirect to `{APP_URL}/api/auth/callback`.
   browseros-neo preserves the requested URL even when the navigation
   fails, so the code is recoverable whether or not the server is running.
4. **Redeems the authorization code in-process** via the same
   `IntegrationService::complete_connect` the server's callback uses, and
   writes the encrypted token through the one shared writer.

The server does **not** need to be running for `--browser`. `--wait` does
need it (it polls for the row the callback writes); `--browser` does not.

Progress is written to **stderr** while the wait is on ("Opened the
`<provider>` consent page in the operator's browser (page N)…"), so stdout
stays clean for the JSON result.

On success the consent tab is closed. On **timeout** it is deliberately
LEFT OPEN — the operator may still be mid-consent, and closing it would
throw their work away — and the error names the page plus the command to
run once they finish (`switchpost connect <provider> --wait`).

#### Deckmill autopost: `switchpost autopost`

One command that renders a carousel with
[Deckmill](https://github.com/ishan-parihar/deckmill) and posts it — no
copy-paste of image paths between tools:

```sh
switchpost autopost --slides slides.json --platforms instagram --content "Three things we learned shipping agents" --brand-name "Acme" --brand-handle "@acme" --topic "Release" --url "https://example.com" --hashtags "ai,automation" --publish --pretty
```

`slides.json` is a JSON array of Deckmill's own slide contract:

```json
[
  {"type": "hero", "params": {"headline": "Ship slides in minutes", "subheadline": "Deckmill + switchpost"}},
  {"type": "split_features", "params": {"title": "Why it works", "features": ["One pipeline", "Token-efficient", "Autonomous"]}},
  {"type": "qr_destination", "params": {"heading": "Start now", "destination_url": "https://example.com", "cta_text": "Scan to sign up"}}
]
```

Run `deckmill slide-info <type>` for a slide type's required params.

The command renders the slides, uploads every PNG through the same media
path the dashboard uses, and then calls switchpost's own staging service —
so the readiness checks, the provider settings and the publish pipeline
are the ones the dashboard uses. `--publish` publishes immediately;
without it the post is staged as a draft. `--schedule <ISO8601>` queues it.

`--url` and `--hashtags` are REQUIRED by Deckmill's renderer (they draw
the slide overlays); pass them even when the brand does not use them.

**Recurring / autonomous posting.** `--repeat-days N --repeat-until <ISO8601>`
turns the post into a RECURRING SERIES:

```sh
switchpost autopost --slides slides.json --platforms instagram --content "Weekly release notes" --brand-name "Acme" --topic "Release" --url "https://example.com" --hashtags "ai,automation" --schedule 2026-11-01T09:00:00Z --repeat-days 7 --repeat-until 2026-12-01T09:00:00Z
```

This creates the spaced copies in the DATABASE and lets the switchpost
scheduler publish them. It is deliberately NOT a loop process:

- the series is visible in the dashboard (the calendar shows every copy),
- it obeys the single-writer rule — no second publisher can race it,
- it cannot double-post against another scheduler the way a
  `while true; do …; sleep …; done` loop can,
- and nothing stays running when the series ends.

`--repeat-days` needs both `--schedule` (the series counts from the first
due time) and `--repeat-until`. At most 100 copies per series.

For a feed-driven loop instead of a fixed series, use the RSS feeds
(`switchpost rss add <url> <integration-id>`): each new feed item
becomes a post, which can then be made recurring with the same
`--repeat-days` mechanism.

#### `switchpost agent`

One command that reports what an agent can do here: the command list, the
two workflows above, every provider with its auth kind, and which
accounts are connected. It is the cheapest possible orientation call.

```sh
switchpost agent --pretty
```

---

### Updating the binary

```bash
switchpost update --check
switchpost update
switchpost --json update
switchpost update --restart
```

The command works for direct release binaries installed by `scripts/install.sh`;
older installs without an ownership receipt must rerun that script once.
It selects the matching Linux or macOS release asset, verifies `SHA256SUMS`, probes
the staged binary, and atomically replaces the executable. `--restart` only restarts
an active `switchpost.service`; a failed service restart restores the previous binary.
Without that flag, active processes continue on the old executable image. `--version`
pins a release, `--channel preview` selects prereleases, and downgrades require
`--allow-downgrade`. Package-manager installations remain owned by their package manager.

Checksums are a transitional verification path; the suite-wide TUF metadata service
and offline signing root have not been provisioned yet.

### Supported Platforms

Providers are tiered: **Tier-1** gets full depth (analytics, engagement, comments, mentions, target discovery); **Tier-2** is publish-maintained. Tier membership is defined once in `src/social/tier.rs` and reported as a `tier` field by `/api/providers`. Full rationale: `docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md` §1.

#### Tier-1 — depth (12)

| Platform | OAuth | Cookie Auth | CLI | MCP Tools |
|----------|:-----:|:-----------:|:---:|:---------:|
| X / Twitter | ✅ | ✅ (GraphQL) | ✅ | 27 |
| Reddit | ✅ | ✅ (www + modhash) | ✅ | 23 |
| LinkedIn (Personal) | ✅ | — | ✅ | 15 |
| LinkedIn (Page) | ✅ | — | ✅ | 11 |
| Facebook | ✅ | — | ✅ | 15 |
| Instagram | ✅ | — | ✅ | 21 |
| Instagram (Standalone) | ✅ | — | — | 7 |
| Threads | ✅ | — | — | 9 |
| YouTube | ✅ | — | — | 2 |
| TikTok | ✅ | — | — | 4 |
| Pinterest | ✅ | — | — | 8 |
| Bluesky | App Password | — | — | 7 |

#### Tier-2 — publish-maintained (14)

| Platform | OAuth | Cookie Auth | CLI | MCP Tools |
|----------|:-----:|:-----------:|:---:|:---------:|
| Discord | ✅ | — | — | 10 |
| Slack | ✅ | — | — | 4 |
| Telegram (Bot) | Token | — | — | 10 |
| Telegram (User) | Session | — | — | 7 |
| WhatsApp | QR | — | — | 9 |
| Mastodon | ✅ | — | — | 6 |
| WordPress | ✅ | — | — | 4 |
| Medium | API Key | — | — | 3 |
| Dev.to | API Key | — | — | 3 |
| Hashnode | API Key | — | — | 3 |
| GitHub | PAT | — | — | 18 |
| Google (Gmail/Calendar/Drive) | ✅ | — | ✅ | 28 |
| Google My Business | ✅ | — | — | 0 |
| Skool | ✅ (Chrome ext) | — | — | 5 |

**29 platforms by default** (12 Tier-1 + 17 Tier-2). `kick`, `vk`, `whop` and `lemmy` were removed outright (v25 §1).

#### Tier-3 — archive (1)

| Platform | Enabled by | Notes |
|----------|-----------|-------|
| Farcaster | `ENABLE_ARCHIVE_PROVIDERS=1` | Web3/Neynar. Code kept under `src/social/archive/`; absent from `/api/providers` and the MCP tool list by default. |

*Mastodon custom instances use `MASTODON_INSTANCE_URL` — no separate provider. TikTok business accounts publish through the same Content Posting API as creator accounts — no separate provider.*

*Google My Business has no MCP tools of its own — it is reachable through the REST API and the channels UI. Per-platform tool counts are the `#[tool(…)]` names in `src/mcp/`; the total is the `#[tool(` count across that directory.*

*Platform list verified against `src/social/tier.rs` + `src/social/registry.rs` (26 default + 1 archived) and the `src/mcp/tools_*.rs` handler modules (42 modules).*

---

## Project Structure

```
switchpost/
├── src/
│   ├── main.rs              # Entry point, CLI dispatch
│   ├── cli/                 # CLI subcommands (clap, 64 commands)
│   │   ├── mod.rs           # Command definitions
│   │   ├── run.rs           # Cross-platform handler implementations
│   │   └── platforms/       # 31 thin shims (call shared MCP handlers)
│   ├── api/                 # REST API (axum routes)
│   │   ├── mod.rs           # Router + AppState
│   │   ├── onboard.rs       # OAuth flows + cookie forms
│   │   └── integrations.rs  # CRUD for connected accounts
│   ├── mcp/                 # MCP server (389 tools, 44 modules)
│   │   ├── mod.rs           # Tool registry
│   │   ├── tools_x.rs       # X/Twitter tools
│   │   ├── tools_reddit.rs  # Reddit tools
│   │   └── ...              # Per-provider tool modules
│   ├── social/              # Provider implementations
│   │   ├── mod.rs           # SocialProvider trait
│   │   ├── x.rs             # X/Twitter (GraphQL + OAuth)
│   │   ├── x_cookies.rs     # Browser cookie extraction
│   │   ├── reddit.rs        # Reddit (dual-path)
│   │   ├── reddit_cookies.rs
│   │   ├── linkedin.rs
│   │   ├── tier.rs           # Tier-1/2/3 membership (v25 §1)
│   │   └── ...              # 29 providers (+ archive/ behind flag)
│   ├── db/                  # Database (sqlx, migrations)
│   ├── scheduler/           # Background post publisher
│   └── config.rs            # Environment configuration
├── frontend/                # SvelteKit dashboard
│   ├── src/
│   │   ├── routes/          # Pages (posts, setup, settings)
│   │   ├── lib/             # Components, API client
│   │   └── app.html         # HTML shell
│   └── package.json
├── Dockerfile               # Multi-stage: downloads pre-built binary
├── .env.example             # All config variables documented
└── Cargo.toml               # Rust dependencies
```

Planning, audit, and decision records are indexed in [`docs/planning/README.md`](planning/README.md).

---
