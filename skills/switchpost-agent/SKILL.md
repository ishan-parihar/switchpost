---
name: switchpost-agent
description: >
  Use Switchpost to manage social media accounts via CLI. Trigger this skill whenever the user wants to post, read, schedule, comment, DM, or manage any social media — X/Twitter, Reddit, LinkedIn, Facebook, Instagram, YouTube, Bluesky, Mastodon, TikTok, Threads, Discord, Slack, Telegram, WhatsApp, Pinterest, GitHub, WordPress, Hashnode, Medium, Dev.to, Skool, Google (Drive/Calendar/Gmail), or any connected provider. Also trigger on "post to X", "check my LinkedIn", "browse Reddit", "manage my socials", "schedule a tweet", "what's on my feed", "get Instagram insights", "post to Facebook page", "send a Telegram message", "manage Discord", "check WhatsApp", "carousel", "autopost", or any social media operation. Also trigger for engagement workflows (like, retweet, reply to comments, send DMs), analytics and performance questions (followers, reach, streak, "how did my post do"), thread/recurring scheduling ("post this thread", "repost every Monday"), failed-post recovery, multi-platform cross-posting, and RSS/blog cross-publishing.
---

# Switchpost — AI Agent Guide

One Rust binary, 28 social platforms, three interfaces (CLI / REST / MCP). For an
agent, the CLI is the interface. `mcp-call` bridges to all 389 MCP tools
in-process — no separate server needed.

**Error contract** (live-verified, four distinct paths):

| Path | Streams | Exit |
|---|---|---|
| Success | TOON/JSON on **stdout** | 0 |
| Usage/arg error (clap) | clap text on **stderr** | 2 |
| Runtime error, core command (`output_error`) | `{error, help?}` TOON/JSON on **stdout** | 2 |
| Runtime error, platform shim (`emit_result`) | `{error, hint}` TOON/JSON on **stdout** | 2 |
| `anyhow` propagation (e.g. `No X integration found`) | `Error: <msg>` on **stderr** | 1 |

So **check stdout for the payload, stderr only for clap/anyhow** — a
nonzero exit with empty stderr means the error object is on stdout.
Never trust exit code alone: a platform lookup of a nonexistent user/tweet
returns exit 0 with empty-shaped data (`data: {id: ""}`), not an error.

**Output**: default is TOON (compact human-readable, ~40% fewer tokens). Use
default or `--json` when parsing. `--pretty` only for showing a human.
`--json`/`--pretty` are GLOBAL and reach every command, including platform
shims (`x timeline --json` emits real JSON).

## Two ID systems (read this before anything else)

Switchpost has TWO kinds of ID, and confusing them is the #1 failure:

| ID | Where it comes from | Used by |
|---|---|---|
| **Integration UUID** | `switchpost mcp-call integrations_list --args '{}'` → `id` | cross-platform lifecycle: `posts create --integrations`, `stage --integrations`, `carousel --integration`, `dm send`, `automation create`, `rss add` |
| **Platform account ID** (`internal_id`) | `switchpost providers --json` → `internal_id` (e.g. X user id `3301263462`), or the platform's own `list` command | platform-specific commands: `linkedin-page post <PAGE_ID>`, `facebook create <PAGE_ID>`, `instagram posts <ACCOUNT_ID>`, `threads post <THREADS_ID>`, `hashnode post <PUB_ID>` |

An integration UUID passed where a platform ID belongs (or vice versa) fails
with "not connected" / "Invalid integration_id format". If a command rejects
your ID, you swapped the columns.

**Get all integration UUIDs in one call:**

```bash
switchpost mcp-call integrations_list --args '{}'
# {"integrations":[{"id":"506f20cf-...","provider":"github","profile_name":"vyreagent",...}, ...]}
```

Many single-platform MCP tools also accept an optional `integration` param of
`"provider:Name"` (e.g. `"x:alt-account"`) or a UUID; omitted → first connected
account.

## Step Zero — orientation

```bash
switchpost agent --json          # commands + workflows + connected providers (one call)
switchpost providers --json      # connected accounts + internal_id + capability flags
switchpost doctor                # health-check every provider
```

If nothing is connected: `switchpost connect <provider>` (prints a URL), or
`switchpost connect <provider> --browser` (drives the consent page in the
operator's signed-in browser — works without the server running), or
`switchpost setup` (guided). Cookie-based providers (X, Reddit) auto-import
browser cookies; `connect-all` does every cookie provider at once.

## Core routes (90% of jobs)

| Job | Command |
|---|---|
| Post NOW to one platform | `switchpost x post "text"` (or `linkedin`, `reddit post`, `facebook create`, `bluesky post`, `mastodon post`, …) — see routing table below |
| Post NOW to many platforms | `switchpost post "text" --platforms x,linkedin` — stages per-platform copies; then `posts publish <ID>` each, OR schedule |
| Draft/schedule | `switchpost posts create "text" --integrations <UUID> --schedule <ISO8601>` |
| Stage drafts for review | `switchpost stage "text" --integrations <UUID1>,<UUID2>` (use `--preview` to see splits without writing) |
| See splits first | `switchpost split-preview "text" --platforms x,linkedin` |
| Publish a draft/queued post | `switchpost posts publish <POST_ID>` |
| List posts | `switchpost posts list --state queued --limit 10` |
| Cancel | `switchpost posts delete <POST_ID>` |
| Read a feed | `switchpost x timeline --count 5` / `switchpost reddit browse rust` / `switchpost feed` (unified, local) |
| Analytics (account + per-post) | `switchpost analytics get <provider>` / `switchpost analytics post <POST_UUID>` / `switchpost streak --tz <zone>` |
| Engage (like/comment/DM) | `switchpost x like <TWEET_ID>` / `comment reply <UUID> <COMMENT_ID> "text"` / `dm send <UUID> <RECIPIENT> "text"` — full set in platform-matrices.md |
| Thread / recurring | `mcp-call posts_thread` (content_parts) / `posts_repeat` (interval_days + FULL-RFC3339 end_date) — exact params in recipes.md |
| Fix a failed post | `switchpost posts list --state error` → `posts get <ID>` (read error_message) → `posts update` + `posts publish` |
| Any MCP tool | `switchpost mcp-call <tool> --args '<json>'` |
| Discover tools | `switchpost mcp-tools` (all 389) / `switchpost mcp-tools --toolset x` (one set) |

**Unified `post`/`stage` semantics** (both go through the same pipeline):
with `--schedule <ISO8601>` posts land **queued** (the scheduler publishes
them); without it they land **draft** — they do NOT go live until you
`posts publish <ID>`. Provider-native commands (`x post`, `bluesky post`, …)
publish immediately.

**Multi-platform targeting rule**: `--platforms` matches provider
identifiers EXACTLY — `linkedin` is the personal profile, `linkedin-page` the
company page, `instagram` ≠ `instagram-standalone`. To hit one of a colliding
pair, name it explicitly or use `--integrations <uuid,...>`.

## Platform routing table (posting)

| Platform | Post command (immediate) |
|---|---|
| X/Twitter | `switchpost x post "<text>"` |
| LinkedIn personal | `switchpost linkedin post "<text>"` |
| LinkedIn page | `switchpost linkedin-page post <PAGE_ID> "<text>"` |
| Reddit | `switchpost reddit post --title "<t>" [--text <b> \| --url <u>] --target <sub>` |
| Facebook | `switchpost facebook create <PAGE_ID> "<msg>"` (also `photo`/`video <PAGE_ID> <URL>`) |
| Bluesky | `switchpost bluesky post "<text>"` |
| Mastodon | `switchpost mastodon post "<text>"` |
| TikTok | `switchpost tiktok post "<caption>"` |
| Threads | `switchpost threads post <THREADS_ID> "<text>"` |
| Discord | `switchpost discord send <CHANNEL_ID> "<text>"` |
| Slack | `switchpost slack send <CHANNEL_ID> "<text>"` |
| Telegram Bot | `switchpost telegram-bot send <CHAT_ID> "<text>"` |
| Telegram User | `switchpost telegram-user send <PEER> "<text>"` |
| WhatsApp | `switchpost whatsapp send <PHONE> "<text>"` |
| WordPress | `switchpost wordpress post "<title>" "<content>"` |
| Dev.to | `switchpost devto post "<content>"` |
| Hashnode | `switchpost hashnode post <PUB_ID> "<title>" "<content>"` |
| Medium | `switchpost medium-blog post "<content>"` |
| Skool | `switchpost skool post <GROUP_ID> "<title>" "<content>"` |
| Instagram | No one-shot CLI. Stage with `switchpost post "…" --platforms instagram --media <url>` (draft) then `posts publish <ID>`, or the MCP container flow (see recipes.md § Instagram) |

Provider-specific settings (Reddit `subreddit`, Pinterest `board`, Discord
`channel`, Hashnode `publication_id`, Telegram-user `target`) go in
`--settings '<json>'` on `post`/`posts create`/`stage`. Without them the post
is created and then fails at publish with "Missing subreddit in settings".

## MCP tools from the shell

```bash
switchpost mcp-call <tool> --args '<json>'   # reaches EVERY tool (filter is list-side only)
switchpost mcp-tools                          # all 389 names
switchpost mcp-tools --toolset <set>          # one set, e.g. x, reddit, core
switchpost mcp-call tools_search --args '{"query":"keyword"}'   # find tools + their sets
```

Toolset names: `core` (cross-platform: posts/stage/media/tags/notifications/
analytics/feed/setup/integrations/signatures/calendar/inbox/auth/comment/dm),
`google`, `github`, `linkedin`, `facebook`, `instagram`, `telegram`,
`discord`, `mastodon`, `bluesky`, `youtube`, `skool`, `whatsapp`, `wc`
(WhatsApp Cloud templates), `webhooks`, `slack`, `x`, `reddit`, `th`
(Threads), `tt` (TikTok), `pi` (Pinterest), `wp` (WordPress), `md` (Medium),
`dv` (Dev.to), `hn` (Hashnode).

A live `switchpost mcp` session boots `core` only; `SWITCHPOST_TOOLSETS=x,reddit`
pre-enables sets at boot, `tools_enable` adds them at runtime. `mcp-call` is
NOT filtered — it names one tool and pays no list cost.

## Where to look next

| Need | Read |
|---|---|
| Read/engage/DM/comment/analytics per platform, full CRUD families | [references/platform-matrices.md](references/platform-matrices.md) |
| Instagram container flow, carousel, autopost (Deckmill), threads/recurring (posts_thread/posts_repeat params), failed-post recovery, analytics, RSS, automation, webhooks, AI helpers, campaigns | [references/recipes.md](references/recipes.md) |
| MCP tool catalogue by category, staging v2 contracts, toolset budgeting | [references/mcp-catalogue.md](references/mcp-catalogue.md) |
| Install/ops: serve daemon, systemd, config, audit, browser OAuth | [references/ops.md](references/ops.md) |
| Unknown flag | `switchpost <command> --help` — never guess |

## Error recovery

| Symptom | Meaning | Fix |
|---|---|---|
| `Invalid integration_id format` / `At least one valid integration UUID required` | You passed a platform ID (or vice versa) | `mcp-call integrations_list` for UUIDs; `providers` for platform IDs |
| `LinkedIn Page 'X' not connected` | Wrong PAGE_ID form | `switchpost linkedin-page list` for the platform page ids |
| `Missing subreddit in settings` (at publish) | Provider target not set | add `--settings '{"subreddit":"rust"}'` (or `board`, `channel`, …) |
| `tool not found` (mcp-call) | Tool name typo | `mcp-tools` to list; `tools_search` to find by keyword |
| `No X/Twitter integration found` etc. | Provider not connected | `switchpost connect <provider>` / `doctor` for health |
| `csrf_origin_mismatch` (REST only) | Server port ≠ FRONTEND_URL port | run `serve --port` matching FRONTEND_URL, or fix the env |
| `Rate limit exceeded` / HTTP 429 / `X API rate limit` | Provider quota (X: cookie-path hits the account-level cap; OAuth path is billed per post by X) | wait for reset; `doctor` shows provider health; scheduler retries with exponential backoff automatically |
| Reddit 403 `whoa there, pardner` | Reddit's www endpoints block datacenter IPs — the token is NOT the problem | `doctor` confirms the account is healthy; treat as environment, not auth; other Reddit paths may still work |
| `Token expired` | OAuth token expired | `switchpost connect <provider>` again; auto-refresh usually handles it |
| `unknown flag` / `invalid value` | CLI syntax | `switchpost <cmd> --help` |
| `redirect_uri_mismatch` (connect) | Provider app's registered URI differs | register the `redirect_uri` the CLI printed in the provider's developer console |
| DB `database is locked` | Two writers on one DB | only one `serve` (or one writer) per DATABASE_URL; `mcp-call` is fine alongside |

**Always run `switchpost doctor` first** when a provider call fails
unexpectedly — it distinguishes "not connected" from "token expired" from
"provider down".

## Auth model (per provider)

1. DB-stored cookie tokens (X, Reddit) — richest access: voting, moderation, GraphQL
2. Browser cookie extraction (Chrome/Brave/Firefox/Zen) — auto-detected by `connect`
3. OAuth tokens (LinkedIn, Facebook, Instagram, Threads, TikTok, YouTube, …)
4. API keys / one-time tokens (Dev.to, Hashnode, Medium, Skool, WordPress)

`connect <provider>` picks the best available automatically.
