# AGENTS.md — Switchpost Development Protocol

## Contents

- [0. THE GOLDEN RULES (read these first)](#0-the-golden-rules-read-these-first)
- [0.5. DESIGN PREFERENCES (non-negotiable — do NOT violate these)](#05-design-preferences-non-negotiable-do-not-violate-these)
  - [1. Single-user, solo-founder social-media management](#1-single-user-solo-founder-social-media-management)
  - [2. Solo-founder digital-marketing-agency level command center](#2-solo-founder-digital-marketing-agency-level-command-center)
  - [3. Local-deployment first, single binary](#3-local-deployment-first-single-binary)
  - [4. Triple interface: CLI + REST API + MCP server](#4-triple-interface-cli-rest-api-mcp-server)
  - [5. Security-first design](#5-security-first-design)
  - [6. Realtime-first frontend](#6-realtime-first-frontend)
  - [What this means for feature parity work](#what-this-means-for-feature-parity-work)
- [0.6. DOCUMENT MAP (what to read, in order)](#06-document-map-what-to-read-in-order)
  - [Rust style: rust-best-practices skill](#rust-style-rust-best-practices-skill)
- [2. Repository Layout](#2-repository-layout)
- [3. Build & Test Workflow](#3-build-test-workflow)
  - [3.1 Prerequisites](#31-prerequisites)
  - [3.2 Backend build](#32-backend-build)
  - [3.3 Frontend build](#33-frontend-build)
  - [3.4 Full deploy (on the target VPS)](#34-full-deploy-on-the-target-vps)
  - [3.5 Running locally](#35-running-locally)
- [4. Git Protocol](#4-git-protocol)
  - [4.1 Commit discipline](#41-commit-discipline)
  - [4.2 Commit message conventions](#42-commit-message-conventions)
  - [4.3 Branching](#43-branching)
- [5. SQL Query Rules (critical — read this)](#5-sql-query-rules-critical-read-this)
  - [5.1 The .sqlx/ offline cache problem](#51-the-sqlx-offline-cache-problem)
  - [5.2 When to use which form](#52-when-to-use-which-form)
  - [5.3 Runtime query pattern](#53-runtime-query-pattern)
  - [5.4 All SQL must be parameterized](#54-all-sql-must-be-parameterized)
- [6. Security Rules (non-negotiable)](#6-security-rules-non-negotiable)
  - [6.1 HTML interpolation](#61-html-interpolation)
  - [6.2 File upload security](#62-file-upload-security)
  - [6.3 Token encryption at rest](#63-token-encryption-at-rest)
  - [6.4 Auth gate](#64-auth-gate)
  - [6.5 CSRF defense](#65-csrf-defense)
  - [6.6 Bind address](#66-bind-address)
- [7. Scheduler Internals (read before touching src/scheduler/)](#7-scheduler-internals-read-before-touching-srcscheduler)
  - [7.1 Post state machine](#71-post-state-machine)
  - [7.2 How processdueposts works (v9-v10)](#72-how-processdueposts-works-v9-v10)
  - [7.3 Circuit breaker (v10)](#73-circuit-breaker-v10)
  - [7.4 Retry policy](#74-retry-policy)
  - [7.5 Thread linking (v10)](#75-thread-linking-v10)
  - [7.6 Webhook dispatch](#76-webhook-dispatch)
  - [7.7 Audit trail](#77-audit-trail)
- [8. Multi-Agent Worklog Protocol](#8-multi-agent-worklog-protocol)
- [9. Known Pitfalls (things that have burned previous agents)](#9-known-pitfalls-things-that-have-burned-previous-agents)
  - [9.1 wreq version pinning](#91-wreq-version-pinning)
  - [9.2 core2 yanked crate](#92-core2-yanked-crate)
  - [9.3 rmcp API](#93-rmcp-api)
  - [9.4 BoringSSL build requires cmake + libclang](#94-boringssl-build-requires-cmake-libclang)
  - [9.5 SQLX offline cache](#95-sqlx-offline-cache)
  - [9.6 Svelte 5 runes](#96-svelte-5-runes)
  - [9.7 Tailwind semantic colors](#97-tailwind-semantic-colors)
  - [9.8 Frontend realtime events](#98-frontend-realtime-events)
- [10. Environment Variables](#10-environment-variables)
- [11. Quick Reference: CLI + MCP Commands](#11-quick-reference-cli-mcp-commands)
  - [CLI (preferred for shell-based AI agents)](#cli-preferred-for-shell-based-ai-agents)
  - [MCP (for Claude Desktop / Cursor)](#mcp-for-claude-desktop-cursor)
  - [Toolset filtering (per-session tool budget)](#toolset-filtering-per-session-tool-budget)
  - [Key MCP tool categories (389 total)](#key-mcp-tool-categories-389-total)
- [12. When You're Done](#12-when-youre-done)

> **This document is the single source of truth for any AI agent (or human developer) working in this repository.** Read it completely before making any changes. It covers the architecture, the build/test workflow, the git protocol, the security model, and the known pitfalls that have burned previous iterations.

---

## 0. THE GOLDEN RULES (read these first)

1. **ALWAYS PUSH COMMITS AFTER EACH ITERATION.** After every meaningful unit of work — a bug fix, a feature, a refactor phase — you MUST `git commit` AND `git push github master`. The user tests locally on their machine by pulling from `github/master`. If you don't push, your work doesn't exist as far as the user is concerned. Do not batch 5 phases of work into one giant commit; commit each phase as you complete it.

2. **NEVER break the build.** Before committing, verify:
   - `cargo check --lib --bin switchpost` → 0 errors (warnings are OK)
   - `cd frontend && pnpm build` → succeeds
   - If you added Rust unit tests: `cargo test --lib` → all pass
   - If you can't verify (e.g. missing `cmake` for boringssl), say so explicitly in the commit message and explain what the user needs to verify locally.

3. **NEVER skip the security review.** Every change that touches HTML interpolation, file upload, auth, or SQL must be reviewed against the security rules in §6. The v7-v9 audit history is full of XSS and encryption-bypass bugs that were introduced by agents who didn't read this file.

4. **NEVER use `sqlx::query!` / `query_as!` macros for NEW queries** unless you can regenerate the `.sqlx/` offline cache against a local SQLite file. Use runtime `sqlx::query()` / `sqlx::query_as()` with `FromRow` structs instead. See §5 for details.

5. **NEVER commit secrets.** The `.env` file is gitignored. `.env.example` is the template. API keys, OAuth secrets, and `APP_PASSWORD` values must never appear in code or commits.

6. **ALWAYS read the worklog before starting.** If a `worklog.md` exists in the project root, read it to see what previous agents did. Append your own entry when you finish. (If the session uses the multi-agent worklog protocol — see §8.)

---

## 0.5. DESIGN PREFERENCES (non-negotiable — do NOT violate these)

These are the core design decisions that define switchpost's identity. They were established by the user and must not be changed, even when adding features inspired by competitors like postiz-app. If a feature addition conflicts with any of these, skip the feature rather than violate the preference.

### 1. Single-user, solo-founder social-media management
- **NO multi-user auth.** There is no user-registration, no user table lookups, no per-user permissions, no team roles, no team collaboration features. The WebUI is gated by a single `APP_PASSWORD` env var.
- **NO team collaboration features.** No "invite team member", no per-user roles (USER/ADMIN/SUPERADMIN), no slot-level comments, no approval queues, no impersonation. If postiz-app has a team feature, switchpost does NOT copy it.
- The `users` table exists only to satisfy FK constraints. The `password` column is unused (a random invalid hash is stored). All data is owned by `DEFAULT_USER_ID`.
- If a competitor analysis suggests adding "team collaboration" or "multi-tenant" features, SKIP that suggestion. It violates this preference.

### 2. Solo-founder digital-marketing-agency level command center
- The frontend is a **command center for one person managing many social accounts**, not a SaaS platform for multiple organizations.
- Features should help ONE person manage MANY channels efficiently. Examples: multi-platform cross-posting, per-platform content adaptation, drag-and-drop calendar, AI assistance, bulk operations.
- Features that add organizational complexity (marketplace, payouts, customer management, org-to-org submission) are OUT OF SCOPE.

### 3. Local-deployment first, single binary
- The app runs as a **single Rust binary** on a VPS or local machine. No microservices, no orchestrator process, no Temporal server, no Redis.
- **SQLite is the only supported database. There is no Postgres fallback, no server, and no Docker.** Storage is one local `.db` file, created and migrated automatically on first run.
- Do NOT suggest adding a Postgres server, Temporal.io, Redis, or splitting the app into multiple processes. The single-binary design is a feature, not a limitation.
- Background tasks (scheduler, RSS poller, feed refresher, analytics cache) run as `tokio::spawn` tasks within the single binary.

### 4. Triple interface: CLI + REST API + MCP server
- All three interfaces must have feature parity for core operations. If a feature is added to REST, it should be exposed via MCP (for AI agents) and CLI (for shell users).
- The MCP server is the primary interface for AI agents (Claude, Cursor). It must be comprehensive — 389 tools is the target, not a limitation.
- The CLI mirrors the MCP tools for shell-based automation.

### 5. Security-first design
- OAuth tokens are **encrypted at rest** with AES-256-GCM when `TOKEN_ENCRYPTION_KEY` is set. This is non-negotiable. Do NOT add code paths that store tokens in plaintext.
- The server binds to `127.0.0.1` by default. LAN exposure is opt-in via `BIND_HOST=0.0.0.0`.
- HTML interpolation always uses `html_escape()`. File uploads always validate MIME + magic bytes. CSRF defense-in-depth on all state-changing routes.
- If a competitor stores tokens in plaintext (postiz-app does), switchpost does NOT copy that pattern. Its encryption-at-rest is a security advantage, not a drawback.

### 6. Realtime-first frontend
- The frontend uses SSE (`/api/events`) for live updates. 9 routes subscribe to realtime events. This is a UX advantage over polling-based competitors (postiz-app uses SWR polling).
- When adding new pages, wire them to the realtime store. Do NOT add polling as a substitute.

### What this means for feature parity work

When implementing features inspired by postiz-app or other competitors, apply this filter:

| Competitor feature | Adopt? | Why |
|---|---|---|
| Drag-and-drop calendar | ✅ YES | Solo-founder UX, no team dependency |
| Global/Internal per-channel editor | ✅ YES | Solo-founder managing many channels |
| Day view calendar | ✅ YES | Content planning for one person |
| Per-thread-item delay | ✅ YES | Solo-founder scheduling optimization |
| AI post generator | ✅ YES | AI agent efficacy |
| Sets/Templates | ✅ YES | Solo-founder efficiency |
| Short-linking | ✅ YES | Content management |
| Timezone picker | ✅ YES | Solo-founder convenience |
| Tag-colored calendar items | ✅ YES | Visual organization |
| Team collaboration / roles | ❌ NO | Violates single-user preference |
| Marketplace / orders / payouts | ❌ NO | Violates solo-founder preference |
| Multi-tenant org switching | ❌ NO | Violates single-user preference |
| Impersonation | ❌ NO | Violates single-user preference |
| Approval workflows (team) | ❌ NO | Violates single-user preference |
| Slot-level team comments | ❌ NO | Violates single-user preference |
| Temporal.io workflow engine | ❌ NO | Violates single-binary preference |
| Redis dependency | ❌ NO | Violates single-binary preference |
| SWR polling instead of SSE | ❌ NO | Violates realtime-first preference |

---

## 0.6. DOCUMENT MAP (what to read, in order)

| Doc | Purpose | When to read |
|---|---|---|
| `AGENTS.md` (this file) | Build/test/git/security protocol | Always, first |
| `README.md` | User-facing overview, platform tiers, CLI/MCP reference | For product context |
| `docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md` | Live master plan: tier rationale (§1), parity phases | Before parity/feature work |
| `docs/PAGE-INDEX.md` | Every active doc with the question it owns | First — it is the routing index the doc map below summarises |
| `docs/platforms-and-cli.md` | Platform list and the full CLI reference | When you need a command or a platform's capabilities |
| `docs/deployment.md` | Deployment: env vars, TLS, reverse proxy, OAuth redirect URIs | Before deploying, or an OAuth redirect misbehaves |
| `docs/agent-integration.md` | MCP setup, Claude Code / Codex / OpenCode session hooks | When wiring an agent to this repo |
| `docs/audit/AUDIT_2026-10-14.md` | End-to-end audit: P0 bugs, cookie/API matrix, fragmentation, phased plan — executed through the 2026-10-15/16 cycles; **history** | For audit context on the P0 fixes and the cookie/API matrix |
| `docs/archive/GAPS_2026-10-15.md` | What the 13 landed audit commits did NOT close at the time — superseded by the OUTLINE succession; **history** | For what was still open after the 13 landed commits, and how it was tracked before the register |
| `docs/planning/TARGETING_AND_WRITER_DISCIPLINE.md` | Targeting, collisions, and the single-writer rule (G-08) — why one long-lived process owns the database | Before touching scheduling, the writer, or the deployment model |
| `docs/planning/VENDOR_PINS.md` | Pinned transitive deps enforced by `vendor-pull.sh --check` (CI gate) | Before touching `Cargo.toml` |
| `docs/planning/DECISIONS_2026-10-19.md` | Open decision points with recommendations — nothing in it is applied until the user approves | When a design-direction change is proposed |
| `docs/planning/EXECUTION_2026-10-19.md` | Cycle-3 execution log: what landed, the gate results, and what did not | As evidence for register transitions |
| `docs/planning/OUTLINE-2026-10-19-v2.md` | The working queue as of 2026-10-19 (audit-revised that day) — **dated-frozen and largely superseded by what has landed since.** Verified 2026-10-20: its six "STILL OPEN" B/C items are all false (SKILL.md is 255 lines, not 580; quick-reference 108 lines, not 614; evals 43 including toolset-shaped #42; the eval-loop file exists; the recipes block is gone). ALL of its open items have landed (verified by reading the sections, not grepping for the outline's jargon — see the re-audit's method note), and its "D-test redness" priority claim is false: `surface_docs_audit` is 3/3 green. Treat this file as provenance, and read `TODO.md` for live status | Read it for provenance and the tier rationale behind each item. Do NOT treat its STILL OPEN verdicts as work to do — they were true on 2026-10-19 and false on 2026-10-20 |
| `docs/planning/REAUDIT_OUTLINE-2026-10-19-v2_2026-10-20.md` | The dated re-audit of the live outline above: its verdicts re-measured against the tree, with the measurement commands | Before believing either the outline or this file, run the commands at the bottom of the re-audit — both are dated records, and only the tree is current |
| `docs/archive/OUTLINE-2026-10-17.md` | Superseded by the 2026-10-19 v2 outline — history, not queue. Its Wave A–C items are tracked in `TODO.md` | For provenance of the 2026-10-17 cycle |
| `docs/archive/OUTLINE-2026-10-16.md` | Executed + red-team-audited predecessor outline (execution `d5b9bbd..7873f84`, audit `ca63a9e`) — history, not queue | For provenance of what landed 2026-10-16/17 |
| `TODO.md` | The register: live status per work item (`scripts/genome status` is the supervised transition), seeded from the 2026-10-17 outline | To see or claim what's open |
| `worklog.md` | The multi-agent worklog — this protocol, §8 | Before starting work; after finishing |
| `docs/decisions/` | AD-NNN decision records, immutable once numbered (`scripts/genome rec new ad` mints them) | When a direction change is ratified |
| `docs/SESSION_HOOKS.md` | Session-start ambient-context setup (AXI) | When wiring agent hooks |
| `skills/switchpost-agent/SKILL.md` | Installed agent skill (via `make install-skill`) | When editing the skill |
| `docs/planning/PHASES_P0-P9.md` | The P0–P9 phase plan the objective names: each phase's acceptance criteria and its register-row mapping | Before parity or phase work; when claiming a phase is done |
| `docs/planning/OUTLINE_TOKEN_REFRESH.md` | The token auto-refresh audit: why a channel shows "needs reconnect" despite a good token (G-01 `linkedin`, G-02 `instagram_standalone` + `threads`), the pre-publish blast radius (G-05), and the sticky-flag policy question (G-03). Also records two false alarms disproved on evidence | Before touching any provider's `refresh_token`, or before adding a refresh retry policy |
| `docs/planning/OUTLINE_REMAINING_GAPS.md` | The approval queue: every remaining gap measured against the tree and ordered by consequence to the founder, not by size — the token-discard fix and its pre-publish half first, then the UI register rows in a dependency order the register itself does not carry, then the credential-gated rows. §7 records what it re-measured as already closed | When deciding what to work on next, or when a "this is still open" claim needs checking against the tree |
| `docs/planning/SESSION_2026-10-19.md` | Session self-reflection: what worked, every failure with its evidence, and the carry-forward checklist | Before a delegated or browser-driven session |
| `skills/switchpost-agent/references/` | The skill's leaves: `platform-matrices.md` (read/engage/DM per platform), `recipes.md` (multi-step workflows), `mcp-catalogue.md` (all 389 tools + toolsets), `ops.md` (install/daemon/config) | When editing the skill; `make install-skill` installs them |

Superseded docs move to `docs/archive/` with a tombstone at the original path — nothing is deleted. (2026-09-30 saw the last deletion of 14 stale docs; that practice is retired.) If a doc isn't in this table and has no tombstone, it doesn't exist — don't go looking for it.

### Rust style: `rust-best-practices` skill

Load the `rust-best-practices` skill (the Apollo handbook, 10 chapters) before writing or reviewing Rust: `&T` over `.clone()`, `Result` + `?` over `unwrap` (never `unwrap`/`expect` outside tests), `thiserror` for libs / `anyhow` for the binary, `cargo clippy --all-targets --all-features --locked -- -D warnings` clean, descriptive test names with one assertion each.

Switchpost is a **single-user, self-hosted social media management platform** designed for AI agents. It provides a triple interface — CLI, REST API, and MCP server — over 26 social platforms (X, Reddit, LinkedIn, Facebook, Instagram, YouTube, Threads, TikTok, Bluesky, Pinterest, Discord, Slack, Telegram, WhatsApp, WordPress, Medium, Dev.to, Hashnode, GitHub, etc.).

**Architecture in one paragraph:** A single Rust binary (`switchpost`) runs an axum HTTP server (REST API + embedded SvelteKit frontend), an rmcp MCP server (stdio, for AI agents like Claude/Cursor), an in-process scheduler (polls for due posts every 30s), an SSE broadcaster (realtime updates to the frontend), and background tasks (RSS poller, feed refresher, analytics cache refresher). All state lives in a local SQLite file. OAuth tokens are AES-256-GCM encrypted at rest when `TOKEN_ENCRYPTION_KEY` is set.

**Key numbers (measured 2026-10-20 against this commit; each line names where to re-measure it):**
- **389 MCP tools** (`grep -rh --include='*.rs' 'tool(' src/mcp/ | wc -l` = 389), all declared in the one `SwitchpostMcpServer` impl in `src/mcp/mod.rs`; the 44 sibling `src/mcp/tools_*.rs` files plus `src/mcp/auth.rs` and `src/mcp/rest_bridge.rs` are handler modules with no `tool(` attributes of their own — do not grep them for the tool count. That is 47 Rust files under `src/mcp/` in total.
- **26 providers registered by default** in `src/social/registry.rs` — 12 Tier-1 (depth) + 14 Tier-2 (publish-maintained). `linkedin-page` and `instagram-standalone` are Tier-1 members in their own right, not extras bolted onto a parent count. One more, Farcaster, is registered only when `ENABLE_ARCHIVE_PROVIDERS` is set (Tier-3, code in `src/social/archive/`). Tier membership lives in `src/social/tier.rs` (plan §1); the counts are asserted there and in `registry.rs`'s own tests, so a drift breaks the build rather than the doc. `kick`/`vk`/`whop`/`lemmy` were removed outright.
- **8 SQL migrations** in `migrations/` (`0001_init` … `0008_posts_media_array`), additive-only, no destructive ops.
- **~94,200 LOC of Rust** (`find src -name '*.rs' | xargs wc -l`) + **~29,200 LOC of Svelte/TS frontend** (`find frontend/src`).

---

## 2. Repository Layout

```
switchpost/
├── src/
│   ├── main.rs              # Entry point: arg parsing, server startup, graceful shutdown
│   ├── config.rs            # Config::from_env() — all env var loading
│   ├── crypto.rs            # AES-256-GCM encrypt/decrypt for tokens at rest
│   ├── error.rs             # AppError type
│   ├── lib.rs               # Module declarations
│   ├── api/                 # axum HTTP router + route handlers
│   │   ├── mod.rs           # Router builder, AppState struct, /health, /ready, CSRF middleware
│   │   ├── auth.rs          # POST /api/auth/login (password gate + session cookie)
│   │   ├── posts.rs         # CRUD for posts
│   │   ├── media.rs         # Upload/list/delete/serve media (MIME allowlist + magic-byte sniff)
│   │   ├── onboard.rs       # /setup page + OAuth/cookie connect flows (html_escape + js_escape helpers)
│   │   ├── integrations.rs  # List/connect/disconnect/refresh integrations
│   │   └── ...              # calendar, feed, comments, dms, automation, tags, signatures, webhooks, rss, analytics, developer, notifications
│   ├── auth/
│   │   ├── mod.rs           # JWT create/validate
│   │   ├── middleware.rs    # auth_middleware (validates sf_session cookie), AuthenticatedUser extractor, DEFAULT_USER_ID
│   │   └── jwt.rs           # hash_password / verify_password (argon2), create_token / validate_token (jsonwebtoken)
│   ├── cli/                 # CLI commands (mirrors MCP tools)
│   │   ├── run.rs           # Main CLI dispatcher (~4059 LOC)
│   │   └── platforms/       # 32 platform shims (thin wrappers around unified commands)
│   ├── mcp/                 # MCP server (389 tools, all #[tool] methods in mod.rs below)
│   │   ├── mod.rs           # SwitchpostMcpServer impl + all #[tool] method registration (~4114 LOC)
│   │   ├── tools_posts.rs   # Post CRUD MCP tools
│   │   ├── tools_admin.rs   # v9 parity wrappers (posts_repeat, posts_set_tags, media_delete, etc.)
│   │   ├── tools_x.rs       # X-specific tools (create_tweet, reply, like, retweet, search, timeline)
│   │   ├── tools_{platform}.rs  # Per-platform tool modules
│   │   └── auth.rs          # MCP auth (stdio = trusted local)
│   ├── social/              # Provider trait (in mod.rs) + 29 sibling files that `impl SocialProvider` — 26 registered as providers, plus calendar, drive and gmail, which are capability modules reached through the `google` provider. Nine further files are support, not providers: registry, tier, common, browser_cookies, reddit_cookies, x_cookies, x_queryids, google_oauth, whatsapp_cloud. `src/social/archive/` holds Farcaster (Tier-3, opt-in).
│   │   ├── mod.rs           # SocialProvider trait, ProviderError enum, PostContent, MediaAttachment
│   │   ├── registry.rs      # ProviderRegistry (provider lookup + per-provider Semaphore)
│   │   ├── x.rs             # X/Twitter (wreq + GraphQL cookie auth, ~3455 LOC)
│   │   ├── reddit.rs        # Reddit (OAuth + cookie auth, ~2511 LOC)
│   │   └── ...              # linkedin, facebook, instagram, threads, youtube, tiktok, bluesky, mastodon, pinterest, discord, slack, telegram_bot, telegram_user, whatsapp, wordpress, medium, devto, hashnode, github, google, skool, google_my_business, farcaster (see tier.rs for membership)
│   ├── db/
│   │   ├── mod.rs           # create_pool() — SqlitePoolOptions with env-configurable limits
│   │   ├── models.rs        # PostState enum (Draft/Queued/Publishing/Published/Error), Post, PostWithIntegration, Integration, etc.
│   │   └── queries.rs       # All SQL queries (~4984 LOC)
│   ├── scheduler/
│   │   └── mod.rs           # In-process scheduler: process_due_posts (JoinSet + Semaphore), publish_post (retry + backoff), proactive_token_refresh, analytics cache refresh
│   ├── services/
│   │   ├── posts.rs         # PostService (shared by API + MCP + CLI) — publish, resolve_token
│   │   ├── content_splitter.rs  # Split long content into platform-specific chunks
│   │   ├── webhook_dispatcher.rs  # send_webhook + dispatch_event
│   │   └── notifications.rs
│   ├── realtime/
│   │   └── mod.rs           # Broadcaster (SSE fan-out to frontend)
│   ├── feed/                # Feed import/refresh (background task)
│   ├── rss/                 # RSS auto-post (background task)
│   └── wa/                  # WhatsApp Web client (wa-rs)
├── frontend/                # SvelteKit 5 frontend
│   ├── src/
│   │   ├── routes/          # 18 top-level page routes (dashboard, posts, calendar, feed, analytics, search, channels, media, tags, activity, comments, dms, research, campaigns, kanban, login, auth, settings) + 11 nested pages under settings/ and the [id] detail routes — 29 rendered pages in total
│   │   ├── lib/
│   │   │   ├── api/         # API client modules (posts.ts, analytics.ts, integrations.ts, etc.)
│   │   │   ├── ui/          # Reusable components (Button, Modal, Badge, Icon, Spinner, Card, Dropdown)
│   │   │   ├── composer/    # Post composer components (RichTextEditor, ChannelSelector, SchedulePicker, etc.)
│   │   │   ├── stores/      # realtime.ts (SSE client), toast.ts, calendar.svelte.ts
│   │   │   └── channels/    # Channel connect flows (ApiKeyConnect, Web3Connect, TimeSlotEditor, etc.)
│   │   ├── app.css          # Global styles (Inter font, prefers-reduced-motion, skeleton shimmer)
│   │   └── app.html         # HTML shell (Google Fonts)
│   ├── tailwind.config.js   # Semantic color tokens (surface, line, muted, content, brand)
│   ├── svelte.config.js     # adapter-static (SPA fallback)
│   └── tsconfig.json        # Extends .svelte-kit/tsconfig.json
├── migrations/              # 8 SQL migrations (0001-0008, additive-only, no destructive ops)
├── scripts/
│   ├── install.sh           # One-line installer (downloads pre-built binary, sets up systemd)
│   ├── switchpost.service # systemd unit
│   ├── switchpost-start.sh
│   ├── genome               # The vendored project-genome CLI (wake / ask / close / lint / doctor). See genome.toml for the role→path map
│   └── vendor-pull.sh       # Verifies the pinned transitive deps in docs/planning/VENDOR_PINS.md (`--check` is a CI gate)
├── Dockerfile               # OPTIONAL container build (downloads a pre-built binary). SQLite-in-container is not a supported deploy path — §0.5.3 rules out Docker as a runtime dependency.
├── Makefile                 # build, deploy, redeploy, restart, logs, watch
├── Cargo.toml               # Rust dependencies
├── rust-toolchain.toml      # Pinned toolchain + clippy/rustfmt (CI lint gate)
├── .env.example             # Template for .env (all env vars documented)
└── AGENTS.md                # THIS FILE
```

---

## 3. Build & Test Workflow

### 3.1 Prerequisites

The backend requires `cmake` + `libclang` for `boring-sys` (BoringSSL, used by `wreq` for TLS fingerprinting). On Debian/Ubuntu:
```bash
sudo apt-get install -y cmake libclang-dev pkg-config libssl-dev libsqlite3-dev
```

The frontend requires Node 20+ and pnpm:
```bash
cd frontend && pnpm install
```

### 3.2 Backend build

```bash
# Fast check (no codegen) — use this during development:
cargo check --lib --bin switchpost

# Full release build:
cargo build --release

# If you get "SQLX_OFFLINE=true but there is no cached data for this query":
# Either (a) regenerate .sqlx/ against a local SQLite file (see §5.1), or
#        (b) use runtime sqlx::query() instead of sqlx::query! macro (see §5)

# Run unit tests (fast, no DB needed for most):
cargo test --lib

# Run a specific test:
cargo test --lib -- html_escape
```

**Note on boringssl build time:** The first `cargo build` will compile BoringSSL from source (~5-10 minutes). Subsequent builds use the cached artifacts in `target/debug/build/boring-sys2-*/`. If you delete `target/`, you'll wait again.

### 3.3 Frontend build

```bash
cd frontend
pnpm install          # first time only
pnpm build            # production build → frontend/build/
pnpm exec svelte-check --threshold error  # type-check (must be 0 errors)
```

The frontend is embedded into the Rust binary at compile time via `rust-embed` (`src/api/mod.rs:15-18`, `#[folder = "frontend/build"]`). So you must run `pnpm build` BEFORE `cargo build --release` if frontend assets changed.

### 3.4 Full deploy (on the target VPS)

```bash
make deploy      # builds frontend + Rust, installs skill, restarts systemd
# or the fast path (skip frontend rebuild):
make redeploy    # cargo zigbuild --release + install binary + restart
```

### 3.5 Running locally

```bash
# No database server to start — SQLite creates and migrates the .db file on first run.

# Run the server (HTTP, no TLS, loopback only):
BIND_HOST=127.0.0.1 APP_URL=http://localhost:6543 cargo run -- serve

# Run MCP server on stdio (for Claude Desktop / Cursor):
cargo run -- mcp

# Use the CLI:
cargo run -- posts list
cargo run -- x timeline
```

---

## 4. Git Protocol

### 4.1 Commit discipline

**ALWAYS PUSH AFTER EACH ITERATION.** The user pulls from `github/master` to test locally. The workflow is:

```bash
# 1. Make your changes
# 2. Verify the build (see §3.2 + §3.3)
cargo check --lib --bin switchpost   # must be 0 errors
cd frontend && pnpm build              # must succeed
cargo test --lib                       # must pass

# 3. Stage + commit with a descriptive message
git add -A
git commit -m "vN Phase X: <one-line summary>

<detailed description of what changed and why>

Verification:
- Backend cargo check: 0 errors
- Frontend pnpm build: succeeds
- Backend cargo test --lib: 53/53 pass"

# 4. PUSH IMMEDIATELY — do not batch multiple phases. The remote is named
# `github` (NOT `origin` — there is no `origin` remote in this repo).
git push github master

# 5. Confirm the push succeeded by checking the REMOTE ref, not the local HEAD
git log --oneline -1 github/master
```

### 4.2 Commit message conventions

- Start with a version prefix: `v9 Stage 1:`, `v9 Stage 2:`, `v10:`, etc.
- First line: imperative mood, ≤72 chars, summarize the phase
- Body: what changed, why, and the verification results
- Reference file paths + line numbers when fixing specific bugs
- If you couldn't verify the build (e.g. sandbox missing `cmake`), say so explicitly

### 4.3 Branching

This repo uses a single `master` branch. No feature branches, no PRs — commit directly to `master` and push. The user reviews the diff on GitHub and tests locally.

---

## 5. SQL Query Rules (critical — read this)

### 5.1 The `.sqlx/` offline cache problem

`sqlx::query!` and `sqlx::query_as!` macros require **either** a live SQLite connection at compile time (via `DATABASE_URL`) **or** pre-generated offline cache files in `.sqlx/`. To regenerate the cache against a throwaway SQLite file:

```bash
rm -f /tmp/sf-dev.db
DATABASE_URL="sqlite:///tmp/sf-dev.db?mode=rwc" cargo sqlx run --source migrations/
DATABASE_URL="sqlite:///tmp/sf-dev.db?mode=rwc" cargo sqlx prepare -- --all-targets
```

`--all-targets`, NOT `--lib --bins`. The narrower form SILENTLY DROPS the
cache entries for the test targets, and the build then fails offline with
"there is no cached data for this query" for a query that is in the
source and was in the cache before. (Found 2026-10-20: `cargo sqlx
prepare -- --lib --bins` dropped two entries under
`tests/live_github_google_test.rs` and broke the offline `--tests` build.)

If you add a new query using these macros without regenerating the cache, the build will fail with:

```
error: `SQLX_OFFLINE=true` but there is no cached data for this query
```

### 5.1.1 A uuid column is BLOB(16), never TEXT

`sqlx` encodes a `Uuid` as 16 raw bytes. A uuid column therefore holds a
BLOB, and there are TWO ways to get that wrong, both of which compile
cleanly and fail only at RUNTIME:

|What you write|What happens|
|---|---|
|`SELECT id as "id!: String"`|Decodes a BLOB as utf-8: "invalid utf-8 sequence of 1 bytes from index 2" - a 500|
|`SELECT id` (bare, in a `query_as!` macro)|The macro infers the type from the column's DECLARED type (TEXT), so it also decodes as a String - the same 500|

So the macro sites MUST carry an explicit `as "id!: Uuid"` override. This
is the repo's convention (30+ sites in `src/db/queries.rs`). The RUNTIME
`sqlx::query_as()` path is different: there the struct field drives the
decode, so a `Uuid` field is enough.

The `.sqlx/` cache records the DECLARED type, which is why the build
passes and the failure only appears when the endpoint is called. 23
sites had `id!: String` and the automation rules API 500'd on its own
create+list cycle; the CLI's `tags list` and `webhooks list` failed the
same way. `tests/uuid_column_decode_test.rs` drives the real router and
fails on the old shape.

### 5.2 When to use which form

| Situation | Use |
|---|---|
| Modifying an EXISTING `query!` macro query | Keep the macro form — the cache entry already exists |
| Adding a NEW query, and you ran `cargo sqlx prepare` | `sqlx::query!` macro + run `cargo sqlx prepare` to update `.sqlx/` |
| Adding a NEW query, and you DIDN'T regenerate `.sqlx/` | **Runtime `sqlx::query()` / `sqlx::query_as()` with `FromRow` structs** |

### 5.3 Runtime query pattern

```rust
use serde::{Deserialize, Serialize};

#[derive(Default, sqlx::FromRow)]
struct MyCounts {
    total: i64,
    published: i64,
}

let counts: MyCounts = sqlx::query_as(
    "SELECT COUNT(*) as total, COUNT(*) FILTER (WHERE state = 'published') as published
     FROM posts WHERE user_id = $1",
)
.bind(user_id)
.fetch_one(&db)
.await
.map_err(|e| format!("DB error: {e}"))?;
```

**Trade-off:** No compile-time column type checking. But the query compiles without a DB connection, which is essential in sandboxed CI environments.

### 5.4 All SQL must be parameterized

NEVER use `format!()` to interpolate values into SQL. Always use `$1, $2, ...` bind parameters. The only exception is building a `LIKE` pattern: `format!("%{}%", query)` is OK because the result is passed as a bind value, not interpolated into the SQL string.

---

## 6. Security Rules (non-negotiable)

### 6.1 HTML interpolation

**Every** value interpolated into HTML in `src/api/onboard.rs` (or anywhere else that emits HTML) MUST go through `html_escape()`. This includes:
- Query string parameters (`?connected=`, `?error=`, `?name=`, etc.)
- DB-sourced fields (`profile_name`, `profile_picture`, `provider_identifier`)
- Upstream API responses (Telegram bot API, etc.)

The `html_escape()` and `js_escape()` helpers live at the bottom of `src/api/onboard.rs`. Use them.

```rust
// CORRECT:
format!(r#"<strong>{name}</strong>"#, name = html_escape(name))

// WRONG (XSS):
format!(r#"<strong>{name}</strong>"#, name = name)
```

For values interpolated into JavaScript string literals (inside `<script>` blocks), use BOTH `js_escape()` AND `html_escape()`:
```rust
let iid_safe = html_escape(&js_escape(iid));
```

### 6.2 File upload security

`src/api/media.rs` enforces:
- MIME allowlist: `image/png`, `image/jpeg`, `image/webp`, `image/gif`, `video/mp4`, `video/quicktime`
- Magic-byte sniffing (`sniff_mime()`) — client-supplied `Content-Type` is never trusted alone
- `serve_media` always sets `X-Content-Type-Options: nosniff`
- Non-image MIME types get `Content-Disposition: attachment`

If you add a new upload endpoint, follow the same pattern.

### 6.3 Token encryption at rest

When `TOKEN_ENCRYPTION_KEY` is set (64 hex chars = 32 bytes), ALL paths that write OAuth tokens to the DB MUST encrypt them first, and an encryption failure MUST abort the write — never fall back to plaintext:

```rust
// Encrypt inside the one shared writer (see §6.3.1). If you are touching a
// refresh site directly and cannot route through it, fail loudly instead:
let enc = crate::crypto::encrypt_string(&plaintext, key)
    .map_err(|e| format!("token encryption failed for integration {id}: {e}"))?;
queries::update_integration_token(db, id, &enc, refresh.as_deref(), expires).await?;
```

**There is 1 shared writer — all refresh paths MUST route through it:**

1. `src/db/queries.rs::update_integration_token` (encrypts access + refresh token when a key is configured; encrypt failure returns `Err`). Every site below passes **plaintext** and lets this function encrypt.
2. `src/scheduler/mod.rs::publish_post` (mid-publish refresh)
3. `src/scheduler/mod.rs::resolve_token` (pre-publish refresh)
4. `src/services/posts.rs::resolve_token` (manual publish path)
5. `src/api/integrations.rs::refresh` (manual refresh endpoint)
6. `src/mcp/tools_admin.rs::handle_integrations_refresh` (MCP refresh tool)

If you add a 7th refresh path, call `update_integration_token` with plaintext — do not add a second encrypt site.

### 6.4 Auth gate

All endpoints except `/health`, `/ready`, `/api/auth/login`, `/api/auth/callback`, `/api/media/{id}` (serve), and `/api/webhooks/whatsapp` require the `sf_session` cookie (validated by `auth_middleware`). `/api/proxy-media` is gated too: it fetches upstream CDN URLs for the caller, and the frontend's `feed.ts` sends the cookie on same-origin requests. Note `/api/events` (SSE) is intentionally auth-gated (BUG #19 — prevents event leakage on networked deployments).

The `/setup` page and `/api/public/connect/*` routes accept EITHER the `sf_session` cookie OR a `?token=<jwt>` query param (via `resolve_authed_user()` in `onboard.rs`). They do NOT mint fresh JWTs for anonymous visitors — that was a security hole closed in v7.

### 6.5 CSRF defense

All state-changing routes (POST/PUT/DELETE) pass through `csrf_origin_check` middleware which validates the `Origin` (or `Referer` fallback) header against `FRONTEND_URL`. Safe methods (GET/HEAD/OPTIONS) are not checked.

### 6.6 Bind address

The server binds to `127.0.0.1` by default. To expose on the LAN, set `BIND_HOST=0.0.0.0` (with a startup warning). Never change the default back to `0.0.0.0`.

---

## 7. Scheduler Internals (read before touching `src/scheduler/`)

### 7.1 Post state machine

```
draft → queued → publishing → published
                    ↓
                  error
```

- `draft`: created but not scheduled
- `queued`: scheduled, waiting for `scheduled_at`
- `publishing`: scheduler has claimed it and is mid-API-call (added in v9 migration 018)
- `published`: successfully published
- `error`: failed after MAX_RETRIES (3)

### 7.2 How `process_due_posts` works (v9-v10)

1. `get_due_posts()` atomically transitions `queued → publishing` for due posts via a CTE (`UPDATE ... WHERE id IN (SELECT ... FOR UPDATE SKIP LOCKED) RETURNING`). This prevents dual-instance double-publish.
2. **Circuit breaker check (v10)**: for each claimed post, check the per-provider circuit breaker. If open, push the post back to `queued` state (skip publishing). This prevents cascading failures during platform outages.
3. For each post that passes the circuit breaker, acquire a per-provider `Semaphore` permit (default 1 for strict platforms, 3 for high-headroom).
4. Spawn the publish task on a **tracked `JoinSet`** (not detached `tokio::spawn`).
5. After each publish, call `circuit_breaker.record_success()` or `record_failure()` to update the breaker state.
6. Wait for all tasks to complete (with 5min timeout per `join_next`) before returning.
7. On startup, `reclaim_stuck_publishing()` marks posts stuck in `publishing` > 15min (`RECLAIM_AFTER_SECS` = 900 in `src/scheduler/mod.rs`) as `error`.

### 7.3 Circuit breaker (v10)

Each provider has a `CircuitBreaker` with three states:
- **Closed**: all requests pass through. Failure count tracked.
- **Open** (after 5 consecutive failures): all requests rejected for 60s. Posts pushed back to `queued`.
- **Half-open** (after cooldown): one request allowed. If success → closed. If fail → open.

Configurable via env vars:
- `PROVIDER_CB_THRESHOLD_{ID}` (default 5, range 1-50) — failures before opening
- `PROVIDER_CB_COOLDOWN_{ID}` (default 60s, range 10-3600) — open duration

### 7.4 Retry policy

- `TokenExpired`: refresh once, retry immediately. If refresh fails, mark `integration.refresh_needed = true`.
- `RateLimited`: exponential backoff `2^attempt × 5s ± 25% jitter`. Max 3 retries.
- `Network`: same exponential backoff (was: immediate fail — a bug fixed in v9).
- `Auth` / `Api` / `InvalidRequest`: no retry (won't succeed).

### 7.5 Thread linking (v10)

When the scheduler publishes a post with `group_id` and `sequence > 1`, it looks up the previous post in the same group (by `sequence - 1`) and passes its `platform_post_id` as `in_reply_to` in the `PostContent`. Providers that support threading (X, Bluesky, Mastodon, Threads) use this to create linked replies instead of standalone posts.

### 7.6 Webhook dispatch

`dispatch_webhook_background()` fires `post.published` and `post.failed` events to all active webhooks matching the event type. Runs in a detached task so it doesn't block the scheduler tick. Records delivery attempts in `webhook_deliveries` table.

### 7.7 Audit trail

Every publish attempt (success or failure) is recorded in the `publish_attempts` table via `record_publish_attempt()`. The operator can query this to see the full retry history.

---

## 8. Multi-Agent Worklog Protocol

If multiple agents are working on the repo in sequence, use the shared worklog at `worklog.md` in the repo root:

1. **Before starting work:** Read `worklog.md` to see what previous agents did.
2. **After finishing work:** Append a new section (do NOT overwrite) using this template:

```markdown
---
Task ID: <task id, e.g. "v9-stage-1">
Agent: <agent name>
Task: <what you were asked to do>

Work Log:
- <step 1>
- <step 2>
- ...

Stage Summary:
- <key results>
- <artifacts produced>
- <known issues / follow-ups>
```

---

## 9. Known Pitfalls (things that have burned previous agents)

### 9.1 `wreq` version pinning

`wreq` is pinned to `=6.0.0-rc.23` in `Cargo.toml`. Do NOT upgrade to `rc.29+` — it has breaking API changes. The `wreq` crate provides Chrome TLS fingerprinting for X/Twitter GraphQL API (regular `reqwest` gets blocked).

### 9.2 `core2` yanked crate

There's a local stub at `vendor/core2/` that re-exports `std::error`, with a `[patch.crates-io]` section in `Cargo.toml`. This works around the yanked `core2 0.4.0` dependency. Do NOT remove this.

### 9.3 `rmcp` API

`ServerHandler::call_tool` needs `RequestContext`. The CLI `mcp` command bridges to the stdio server (`src/cli/run.rs`). Don't try to call MCP tools from the CLI directly — use the shared `services/` layer.

### 9.4 BoringSSL build requires cmake + libclang

If you see `is cmake not installed?` or `Unable to find libclang`, install:
```bash
sudo apt-get install -y cmake libclang-dev
```

In sandboxed environments without root, extract the `.deb` files manually and point `CC`/`CMAKE`/`LIBCLANG_PATH` at the unpacked tree (the exact env-var set depends on your sandbox's layout).

### 9.5 SQLX offline cache

If you add a `sqlx::query!` macro and the build fails with "no cached data", either:
- Run `cargo sqlx prepare` against a local SQLite file (requires `cargo install sqlx-cli --no-default-features --features sqlite`), OR
- Convert the query to runtime `sqlx::query()` form (see §5.3)

### 9.6 Svelte 5 runes

The frontend uses Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`). Do NOT use Svelte 4 syntax (`let x = writable()`, `$: x = y`). The `svelte.config.js` suppresses `a11y_*` and `state_referenced_locally` warnings.

### 9.7 Tailwind semantic colors

Use semantic Tailwind classes (`bg-surface`, `text-muted`, `border-line`, `bg-brand-500`, etc.) defined in `frontend/tailwind.config.js`. Do NOT use hardcoded hex colors (`bg-[#1e2435]`) in CSS classes — the v8 audit replaced all of them. JS data values (provider color metadata, color picker palettes) are fine as hex strings.

### 9.8 Frontend realtime events

The SSE client (`frontend/src/lib/stores/realtime.ts`) subscribes to these 19 events:
`post_created`, `post_scheduled`, `post_published`, `post_failed`, `first_comment_failed`, `post_deleted`, `post_updated`, `post_stage_changed`, `lagged`, `integration_connected`, `integration_disconnected`, `notification_new`, `comment_received`, `dm_received`, `campaign_created`, `campaign_updated`, `campaign_deleted`, `feed:new_posts`, `feed:engagement`.

If you add a new broadcast event in the backend (`broadcaster.send(...)`), add it to the `realtime.ts` subscription list AND wire up listeners in the relevant routes.

### 9.9 Own-post engagement lives in `analytics_cache`, NOT `post_engagement`

`post_engagement.post_id` is a foreign key to **`external_posts(id)`**, not to `posts(id)`. It can therefore never hold a row for one of the founder's own published posts - it only holds engagement for IMPORTED competitor feed posts.

The founder's own engagement is in `analytics_cache`, keyed by `(user_id, platform_post_id)`, written by `/api/analytics/post/{id}` after a live provider fetch. Its payload is provider-shaped JSON: a list of `{label, data:[{total,date}]}` series.

Two consequences that have each broken a query:
- A `LEFT JOIN post_engagement pe ON pe.post_id = p.id` matches NOTHING for own posts, so the join looks like it works and every row scores zero.
- The provider's series LABELS are not a shared vocabulary. X reports "Retweets" and "Replies", Bluesky reports "Reposts" and "Replies", LinkedIn reports lowercase. Matching only `lower(label) IN ('likes','comments','shares')` reads 0 comments and 0 shares forever on X and Bluesky, and `WHERE engagement > 0` then EXCLUDES those posts from any ranking.

The vocabulary is mapped in ONE place - `db::queries::metric_label_matches` - and every reader uses it. Do not write a second label list.

### 9.10 A calendar range's `end` is an INCLUSIVE DATE

`end=2026-10-31` means "through the end of the 31st", not "at 00:00:00 on the 31st". The calendar endpoint resolves a date-only `end` to the last instant of that day, because the range is filtered with `scheduled_at <= end` and midnight excluded every post on the last day of the range.

The same shape appears in every "date range" parameter: a bare `YYYY-MM-DD` is a CALENDAR DATE. Rendering one with `new Date("2026-11-15")` parses it as UTC midnight and shows the PREVIOUS day in every negative-offset zone - the frontend's `formatDateOnly` in `lib/api/campaigns.ts` exists for this.

### 9.11 Timestamps: the storage type and the reader must agree

SQLite keeps TEXT in an INTEGER-affinity column, and orders every INTEGER before every TEXT. A column declared `INTEGER` that is written with a `DateTime<Utc>` (sqlx stores RFC 3339 TEXT) therefore compares WRONG against every INTEGER bound: `created_at >= ?` is always TRUE and the range filter is a no-op. `external_posts.created_at` was exactly this, which made the 90-day research window, the feed summary and the posting-time histogram let through posts of any age.

Write epoch columns through the `EpochUtc` seam in `db::types` (bind `EpochUtc::from(dt)`, read `EpochUtc`), and read them back as `DateTime<Utc>` via `From`. Migration 0007 rewrote the existing rows.

### 9.12 A partial update needs THREE states, not two

`Option<T>` collapses "absent" and "null", so a partial-update endpoint cannot express "leave it alone" versus "clear it". The composer sends `undefined` for a field the user emptied - and `undefined` means "omitted" on the wire, so emptying the title box left the old title on the post.

The three-state pattern is `Option<Option<T>>` with a `double_option` deserializer (see `api::campaigns::double_option`), and the SQL needs an explicit flag rather than `COALESCE` - `COALESCE(?, col)` cannot tell "keep" from "clear" either, so the write is gated: `CASE WHEN ? THEN ? ELSE col END`.

The same pattern applies to `campaign_id` on the stage endpoint, where a plain `Option<Uuid>` meant that omitting it WROTE NULL and silently dropped a post out of its campaign on every kanban drag.

### 9.13 The composer and the publish path must speak the same shape

The providers read FLAT, PROVIDER-NAMED keys out of `settings`: Reddit wants `settings["subreddit"]` and `settings["title"]`, Pinterest wants `settings["board"]`. The composer used to send `{integrationId: {target_ids}}`, which no provider reads - so every Reddit post failed at publish with "Missing subreddit in settings".

The `title` is also stored in `posts.title`, which NO publish path reads. A title for a platform that publishes one (Reddit, Pinterest) has to be in the settings.

### 9.14 The character count has one rule per provider, in two languages

The composer shows a count and the server enforces one. The plain rule is UTF-8 BYTES on both sides (the browser's `String.length` counts UTF-16 code units, so a 200-emoji post read "200/300" in the composer and was rejected as "800 chars"). X's rule is the WEIGHTED one (twitter-text v3: emoji and CJK cost 2), implemented on both sides in `social::weighted_length` and `composer/platforms.ts`.

`tests/counting_rule_test.rs` pins the server's rule against the composer's.

### 9.15 A date-only `end` is inclusive; a cursor must be EXCLUSIVE

A pagination cursor that is the oldest row's own timestamp, with a `created_at <= cursor` filter, returns that row AGAIN on the next page. The Activity page appends into a keyed `{#each rows as row (row.id)}`, and Svelte throws `each_key_duplicate` on a repeated key - so the page CRASHED on "Load more". The cursor is now one second earlier, and the append is idempotent so a page can never take the feed down.

### 9.16 The post body is PLAIN TEXT, everywhere

The editor speaks plain text at its boundary. The server stores plain text. The providers publish plain text. The rich text is an EDITING convenience; the body is text.

The editor used to speak HTML (`getHTML()` out, `setContent()` in) while the server stored text. So the server stripped the editor's HTML on the way in, and every path that read the stored TEXT back - on duplicate, on edit, and from the autosaved draft - handed plain text to an editor that parses its input as HTML. TipTap took the text's `<` for a tag, and a body of `---` came back as `--`.

The server is still a trust boundary, so HTML that arrives from the MCP tool, a webhook or an older API client is still converted. The discriminator is EVIDENCE of markup - a closing tag, an attribute, or a void tag - not a bare `<word>`, which is a word in angle brackets and exactly what a post about HTML is full of.

The post detail page renders the body as TEXT, not `{@html}`. It used to render `{@html post.content}`, and the server's stripping was the only thing hiding the XSS hole. The server is not the right layer for that defence; the page is.

### 9.17 A schedule is a WALL CLOCK in the app's timezone, not the browser's

`wallClockToUtc` (frontend/src/lib/composer/wall-clock.ts) is the one conversion. The composer, the calendar's drag-to-reschedule, the calendar's bulk reschedule and the composer's preset all use it.

The calendar's READ path used the app's selected timezone and its WRITE path used the browser's, so a post dropped on the 09:00 slot was stored as 09:00 UTC - five hours off in New York - and appeared in the wrong slot until the next render. `new Date(`${date}T${time}`)` parses in the browser's zone, which is not the same thing as the app's setting.

### 9.18 One implementation per rule, on both sides of the wire

A rule that both the browser and the server enforce needs ONE implementation on each side, and the two must be compared by a test.

Iteration 1 found the character count implemented twice with different rules (UTF-16 units in the browser, UTF-8 bytes on the server; the weighted rule for X only in the browser). Iteration 2 found the HTML-to-text conversion implemented EIGHT times - three in Rust, five in TypeScript - with three different behaviours, and the copy that was actually used deleted the text between any two angle brackets.

When a rule lives on both sides, `tests/counting_rule_test.rs` and `tests/html_to_text_test.rs` pin the two against each other. A new copy of a rule is a defect.

### 9.19 The character count is the TEXT, not the markup

`countFor` measures the plain text. The composer shows the number the provider will count, and the server enforces the number it will count. The preview shows the same text. A preview that strips the tags with its own regex can show a different string from the one the count was measured on - and a preview that strips too eagerly deletes a legitimate mention of a tag.

### 9.20 The composer validates against the SERVER's limits, not a copy

`Provider::max_content_length()` is the authority. The frontend used to keep its own table, and the two drifted: X showed 280 in the composer (blocking a 300-character post and disabling the Post button) while the server enforced 4000. `/api/providers` now publishes `char_limit` per provider and the composer reads it.

The same applies to which providers can be offered at all: a provider whose limit is 0 (Gmail, Drive, GitHub) has no "publish a post" surface, and offering it in the composer's channel grid produced a 400 at submit time ("Content is 42 chars, max 0 for Gmail").

### 9.21 The MCP surface is a CLIENT of the REST surface, not a second implementation

An MCP tool and the REST endpoint for the same operation must behave the same way. Where the MCP layer re-implemented the SQL, it dropped the safety half every time: the tag write lost its ownership check and its transaction, the automation update lost its `user_id` predicate, the media delete hard-coded the local directory instead of the active storage backend.

The fix is to CALL the REST handler in-process (see `mcp::rest_bridge`), so the validation and the ownership checks live in one place. A tool that cannot reach its handler (because the api module is private) makes that module `pub(crate)`.

The MCP input types are three-state too: `Option<Option<T>>` with `double_option`, so `null` means "clear" and absent means "keep". A two-state input mapped with `.map(Some)` turns "clear this" into "leave this alone" - and the tool reports success.

### 9.22 A plain Map or Set is NOT reactive state

Svelte 5's `proxy()` returns a `Map` or a `Set` UNCHANGED - their prototype is neither `Object.prototype` nor `Array.prototype`. So `$state(new Map())` is a plain collection with no reactive tracking, and mutating it in place notifies nobody.

The consequence is silent: a template guard like `{#if integrationTargets.has(intId)}` compiles to a derived with ZERO reactive dependencies, which Svelte evaluates ONCE at mount and never recomputes. The panel never appeared.

The fix is to REASSIGN after every mutation (`integrationTargets = new Map(integrationTargets)`) - the idiom this codebase already used for `selectedTargets`, `providerOverride` and `groupDraftCache`. Or use `SvelteMap`/`SvelteSet` from `svelte/reactivity`.

### 9.23 A keyed `{#each}` needs a key that CANNOT repeat

Svelte throws `each_key_duplicate` - a HARD throw, in dev and in prod - when a keyed each sees the same key twice, and it takes the whole page down.

The keys come from the data: an LLM's free-text response split on whitespace repeats itself constantly ("ai ai marketing"), and a provider's analytics series list can carry the same label twice. Dedupe the data, and make the key unique (`label + '#' + index`).

### 9.24 The claim is a compare-and-set, and the UPDATE is what makes it atomic

SQLite issues a deferred `BEGIN`, which takes the write lock at the FIRST WRITE, not at BEGIN. So the scheduler's `SELECT` of the due ids runs with NO lock held, and two claims can read the same id list.

The guard that keeps the claim exclusive is on the UPDATE: `WHERE id IN (...) AND state = 'queued'`, with `RETURNING id` so the claim returns EXACTLY the rows it flipped. Reading them back by `state = 'publishing'` does not work - a row another claim flipped is also 'publishing', and the loser would publish it too. A post published twice is unrecoverable on a real audience.

### 9.25 A media URL is stored RELATIVE, so the provider needs it ABSOLUTE

`MediaPublic.url` is `/api/media/<uuid>` - a relative path, because that is what the dashboard renders. A platform cannot fetch it. The trait's `resolve_media_url` default makes it absolute against the app's public base URL, because that is correct for EVERY provider.

The default used to return the attachment unchanged, with each provider opting in - and eight of the twelve Tier-1 providers did not have the override, so they published with a relative URL and failed, with the error surfacing as an opaque network failure rather than a media error.

### 9.26 `posts.media` is an ARRAY, and a `{}` takes the whole ROUTE down

`posts.media` holds an array of attachments. The RSS autopost path wrote a JSON OBJECT there - `create_post`'s SIXTH parameter is `media`, and it was passed `json!({})`:

```rust
create_post(db, ..., &serde_json::json!({}), &serde_json::json!({"rss_auto": true}), ...)
```

The frontend assumes an array. The kanban's `coverImage` calls `post.media.find(...)` directly, so `{}` - which has no `find` - crashed the whole ROUTE with "i.find is not a function". A pageerror, not one bad card. The calendar's `(post.media || []).map(...)` failed the same way, because `{}` is truthy so the `||` fallback never fired.

Three layers own this, and each has a test in `tests/posts_media_shape_test.rs`:

- the WRITER (both RSS paths) passes `json!([])`
- the MIGRATION (`0008_posts_media_array.sql`) repairs the rows that are already there - an operator's database holds them
- the READER (`db::models::media_as_array`) normalises anything that slipped through, so the response is an array whatever the column holds

The general rule: a JSON column that the frontend treats as an array must be normalised ON THE READ, in ONE place, because the column's history is longer than the frontend's assumption.

### 9.27 The composer and the server must agree on the settings KEY

A provider that needs a target reads it out of the post's `settings`
under a SPECIFIC key - Discord's `channel`, Reddit's `subreddit`,
Pinterest's `board`, Telegram-user's `target`, Hashnode's
`publication_id`, Skool's `groupId`.

The composer used to write `channel_id` and `page_id` - keys NO
provider reads - and never wrote Discord's `channel`. So every
Discord post was created, queued, and then failed at publish with
"Missing channel in settings".

There are TWO lists to keep in agreement, and they are in different
places:

- the composer writes the key (`ComposerModal.svelte`'s
  `providerKeyForTarget` for a target, `TITLE_PROVIDERS` for a title,
  `PUBLISH_STATUS` for a publish status)
- the provider reads the key (`src/social/<provider>.rs`)

And the server has the third list: `SocialProvider::required_settings()`,
the keys a provider cannot publish without. The create path checks it
and refuses the post with a message naming the missing field, so the
failure happens where the founder can act on it - not hours later in
the scheduler.

A NEW provider that reads a key out of `post.settings` must add that
key to `required_settings()`. A provider that publishes a title must
be in the composer's `TITLE_PROVIDERS` - `posts.title` is read by NO
publish path.

### 9.28 A setting that is COLLECTED and never READ is a silent failure

The composer's MusicPicker is shown for Instagram and the pick is
written to `settings.audio_id`. Nothing read it - so a Reel went out
silent while the composer showed a "Change/Browse" control that
implied it had been applied.

The same shape: the composer's title was written only for a
`subreddit` or a `board` target, so Dev.to, Medium, WordPress and
Hashnode fell back to the first line of the body. And Dev.to's
`published`, Medium's `publish_status` and WordPress's `status` default
to a DRAFT - so "Post Now" produced a draft and the toast said
"Post published".

The rule: a control in the composer is a PROMISE. If the composer
collects a value, something must send it, something must read it, and
the founder must be able to see the result. A value that is collected
and dropped is worse than no control at all, because it looks like it
worked.

### 9.29 The edit flow must not delete what the publish flow needs

The provider settings hold the target, and the provider reads them at
PUBLISH time.

The composer's edit flow deleted them: the detail response had NO
`settings` field to restore from, the composer sent `settings: {}`, and
the update path REPLACED the settings wholesale. So opening a
scheduled Reddit post to fix a typo and saving it DELETED the
subreddit - and the post failed at publish with "Missing subreddit in
settings". A post that was working thirty seconds earlier.

Two things hold this together:

- the READ path returns the settings, so the client can restore them
- the WRITE path MERGES the incoming settings over the stored ones, so
  a key the caller did not send is KEPT

The merge is the one that matters: a client that does not know about
the settings must not be able to delete them. The whole-object `null`
stays an explicit clear, and the validator and the write must read the
SAME merged settings - the validator prefers the override's over the
global ones, so reading the raw override there made it report a
setting as missing when the stored one was right there.

### 9.30 A range filter and the grid it fills must agree on "a day"

The calendar's GRID is built from the APP's timezone days. The RANGE
FILTER used to be UTC: the client sent date-only strings and the
server resolved them to UTC midnight and UTC end-of-day.

The posts in the gap fell off the calendar. With the app in
America/New_York, a post scheduled Oct 31 20:00 EDT = Nov 1 00:00 UTC
was EXCLUDED from October's range and then DROPPED by November's grid,
which has no Oct 31 cell. It appeared on NO month grid, in either
month.

So the range is sent as UTC INSTANTS, computed with
`wallClockToUtc` - the ONE conversion the app has. And
`wallClockToUtc` keeps the seconds it is given: it used to append its
own `:00` to a time that already had seconds, so `23:59:59` became
`23:59:59:00`, which is NOT RFC 3339, and the server answered
"Invalid end date" for the whole range.

The same zone discipline applies to the "is this cell today" checks:
they compare against the BROWSER's date while the cells are the APP's
days, so the grid was labelled with the browser's dates and the chips
fell into the neighbouring day's cell.

### 9.31 `COALESCE` cannot express "clear"

`kanban_substate = COALESCE(?, kanban_substate)` can express "keep"
and "set", and NOT "clear". So a card tagged "Blocked" was "Blocked"
for ever: the client had to refuse the clear and toast "Clearing
substate is not yet supported", and the "No substate" filter the
filter bar offered could never match a card the founder wanted to
clear.

The three-state pattern (`Option<Option<T>>` with `double_option`)
plus a SQL that uses an EXPLICIT FLAG:

```sql
kanban_substate = CASE WHEN ? THEN ? ELSE kanban_substate END
```

The flag is the "the caller is setting this" bit, and the value is
`NULL` when clearing. The same shape the `campaign_id` write already
used.

### 9.32 The refresh cycle must refresh what the dashboard READS

The per-post analytics rows - the ones the analytics page, the research
page, the campaign engagement and the per-post badges read - were
written ONLY when a human opened the Post Stats modal. The background
refresher wrote the ACCOUNT-level rows, and the cleaner deleted the
expired per-post rows every 30 minutes.

So a founder's "Likes 42" silently became "Likes 0" an hour later,
with no publish, no error and no refresh, and the research page's
"your top posts" went empty the same way.

The refresher refreshes the per-post rows too. The window is the recent
published posts, because a per-post fetch is one API call each.

### 9.33 A `Uuid` column is BLOB(16), and a `String` decode only reads TEXT

sqlx encodes a `Uuid` as BLOB(16) and the SQLite `Uuid` decoder reads
exactly 16 raw bytes. It does NOT accept the 36-character TEXT form.

So `SELECT id as "id!: String"` on a uuid column is a runtime failure
("invalid length: expected 16 bytes, found 36" or a UTF-8 decode
error) that the COMPILE-TIME check cannot see: the column is DECLARED
`TEXT` in the migration, so the `.sqlx/` cache records the declared
type and the build passes.

23 sites did this. The symptom was a 500 on `GET /api/automation/rules`
- and, because the failed POST still INSERTED the row, an orphaned
rule with no id returned.

The fix is the repo's own convention (30+ sites in `src/db/queries.rs`):
an explicit `as "id!: Uuid"` override. NOT dropping the override - the
`query!` MACRO infers the decode type from the DB describe, not from
the Rust struct field, so a bare `id` on a TEXT column still infers
`String`.

`tests/uuid_column_decode_test.rs` drives the real router and fails
on the old code.

### 9.34 A TEXT timestamp bound against an INTEGER epoch column matches NOTHING

SQLite keeps TEXT in an INTEGER-affinity column, and orders every
INTEGER before every TEXT. A column declared `INTEGER` that is written
with a `DateTime<Utc>` (sqlx stores RFC 3339 TEXT) compares WRONG
against every INTEGER bound: `created_at >= ?` is always TRUE, and
`created_at <= ?` is always FALSE.

`get_engagement_summary` bound a `DateTime<Utc>` cutoff against an
INTEGER epoch column, so the 30-day window matched ZERO rows and the
feed half of every analytics headline was "0.0 interactions per post"
against real posts with real engagement.

Write epoch columns through the `EpochUtc` seam in `db::types` (bind
`EpochUtc::from(dt)`, read `EpochUtc`).

### 9.35 A page that is only rendered in its DEFAULT state is a page whose other states are unchecked

The probe rendered the calendar in its default state - the WEEK view.
The month and day views are separate components with their own layout,
and NEITHER had ever been rendered by the probe. Both were broken: the
month view's chips were 15px of a 144px cell, the day numbers were at
2.20:1, and the day view's ghost-slot button was 21px tall.

The probe now drives a page into a named state (`--state "Month"`), and
the same hole was found in the tag modal's edit form.

### 9.36 A measurement that depends on WHEN it ran is not a measurement

One full probe run reported 69 contrast failures on the research page.
The next run of the same command reported none.

The theme is a class on `documentElement`, applied by the theme store
on mount. The probe seeds the choice through localStorage and the store
reads it on its first read - so there is a window in which the page is
rendered with NO theme class and the tokens resolve to the un-themed
defaults. Every failure in that run was a text tier measured against a
background it would never meet.

The audit waits for the theme class now.

### 9.37 A severity that does not match the standard makes the gate lie

The probe treated every tap target under 44x44 as a FAIL. But WCAG 2.2
"Target Size (Minimum)" (2.5.8) sets the floor at 24x24, and a FAIL
means "the probe is not clean" - so a page with a legitimate dense
month grid could never pass.

Below 24x24 is a FAIL. Between 24 and 44 is a WARNING. The severity
has to match the standard, or the gate lies about what is broken.

### 9.38 An empty list is not a failure

The probe reported the two dynamic routes as FAIL when the API
answered 200 with an EMPTY list - the page is not broken, there is no
row to open. A fresh install looks exactly like this, and a fresh
install is the FIRST thing a founder sees.

An empty list is a SKIP, and the report says so. An API that FAILS is
still a FAIL.

### 9.39 A DIALOG inside an `aria-hidden` subtree is invisible AND un-typeable

`aria-hidden="true"` means "a screen reader must not announce this".
Putting a dialog inside it makes the dialog invisible to assistive
technology - and, because the shared focus trap skips anything in an
`[aria-hidden="true"]` subtree, it also makes the trap find NO
focusable child, so the dialog node takes the focus instead of the
input.

The command palette was built that way. The founder pressed Ctrl+K, the
palette opened, and TYPING DID NOTHING - the input never had focus,
and a screen reader announced nothing at all.

The pattern the working dialogs use (`ModalManager.svelte`,
`Modal.svelte`): the SCRIM carries the aria-hidden, and the PANEL is
its SIBLING. The scrim is a `<button>` with `aria-hidden` and
`tabindex="-1"`, so it is a real click target that the tab order and
the accessibility tree both skip.

Two dialogs did not follow it, and they were the two that were broken.

### 9.40 The provider ink is computed against a SURFACE, and there are four

`providerInk(provider)` walks the brand hue's lightness until it clears
4.5:1 against ONE background. The app paints a provider glyph on at
least FOUR: the page, a card (`--bg-card`), a chip
(`--bg-surface-hover`) and an input (`--bg-input`). Each needs its own
ink, or the glyph is under the floor on three of the four.

`providerInk` is the page. `providerInkOnCard`, `providerInkOnChip` and
`providerInkOnInput` are the other three. Getting this wrong is
invisible in the source and invisible in the probe, because the probe
only renders a page's DEFAULT state.

### 9.41 A TAP TARGET is measured on the RENDERED box, and the RENDERED box is not the source

A `px-3 py-1.5` button is 30px tall, whatever the source says about
the padding, and the padding changes with the font. The composer, its
panels and the popovers are where the small targets hide, and the
probe never renders them.

`make audit` drives the real app and measures the rendered box.

### 9.42 The 45 test binaries must not share one SQLite file

`cargo test` runs every integration test binary as its own PROCESS, in
parallel. They used to share one scratch file, so 45 processes
serialised on one write lock and "database is locked" appeared in about
one run in three.

`config::test_database_url()` gives each binary its own file, named
from its own `current_exe`. The file is still under `target/`, so it is
cleaned with the build and never the operator's data.

### 9.43 A LIVE test that fails for the ENVIRONMENT's reason must SKIP

The live tests hit the providers' real APIs with the operator's
credentials. A transport reset, a timeout or a 429 is the
environment's fault, not a defect in this codebase - but it failed the
gate, and a gate that goes red for the wrong reason is a gate the
operator learns to ignore.

`tests/common/mod.rs` classifies the failure: a transport error, a
timeout, a 429, a 401, a 403 or the provider's own error envelope
SKIPS; a malformed response or a missing field still FAILS.
`tests/live_failure_classification_test.rs` pins the classification in
both directions, because a classifier that skips everything would make
the live tests worthless.

### 9.44 A `Uuid` COLUMN is BLOB(16), and a TEXT uuid in it is a runtime error

sqlx encodes a `Uuid` as 16 raw bytes. The SQLite `Uuid` decoder reads
exactly 16 bytes and does NOT accept the 36-character TEXT form - so a
TEXT uuid in a uuid column fails at runtime with "invalid length:
expected 16 bytes, found 36", and the whole query errors out.

This bit twice, in opposite directions:

  - 23 query sites cast the id `as "id!: String"`. The column is
    DECLARED `TEXT`, so the `.sqlx/` cache recorded TEXT, the macro
    inferred `String`, the build passed - and the runtime decode
    failed on the BLOB. The fix is the repo's own convention, an
    explicit `as "id!: Uuid"`, NOT dropping the override: the macro
    infers the type from the column's DECLARED type, not from the
    struct field, so a bare `id` still infers `String`.

  - A test that wrote a TEXT uuid "to be safe" then failed to decode
    against the BLOB the app had written. To write a TEXT row, bind a
    `String`; to write a BLOB, bind `id.as_bytes().to_vec()`.

`tests/uuid_column_decode_test.rs` drives the REAL ROUTER and asserts
HTTP 200, because a test that re-implements the query with its own
struct proves nothing about the production query.

### 9.45 An `<input>` has NO TEXT NODE, so a text-contrast gate cannot see it

The contrast audit looks for elements with their OWN TEXT and measures
those. A form field has no text node, so it was skipped - in every
theme, by every gate.

The connect flow's API-key field was `text-white` on the light input
surface: 1.05:1. The founder typed their API key and could not see a
single character, and every contrast gate in the repo passed.

So the field's own `color` is measured against its effective
background, and the connect flows are DRIVEN (a provider tile is
clicked, the form opens, its fields are measured). A provider whose
form cannot be reached is a FAILURE, not a skip: a gate that measures
nothing must say so.

### 9.46 A TRUNCATED Tailwind class is a SILENT failure

`hover:text-accent-strong` cut to `hover:text-accent-stro` is not a
build error, not a type error and not a svelte-check warning. Tailwind
's JIT does not generate a rule for a class it does not recognise, so
the element simply does not get the style - and a screenshot shows a
link with no hover colour and reads as "a design choice".

Eight were in the tree, in six files. Three had a class cut in HALF
("tex" from "text-muted", "hover:" with nothing after it, "border
border" with no colour) and one had a SPACE removed between two
classes ("bg-linetext-content-secondary").

`frontend/scripts/check-tailwind-classes.mjs` collects every Tailwind
class the source uses and confirms each one is in the GENERATED
stylesheet. It is in `make check`, beside the hex gate (both need the
BUILT frontend) and `scripts/check_status_contrast.py` (which reads the
SOURCE app.css and needs no build).

### 9.47 A NUMBER computed from a CLIPPED PAGE is a wrong number

The dashboard fetched `limit: 100` of everything and computed every
aggregate client-side. With 106 posts it said "Drafts 84" while the
Posts page said 89 - the same account, two answers, and which six
posts fell off the end depended on the sort order.

The COUNTS are the server's now (the list response carries the true
`total` for a filtered query) and the LISTS are fetched per state, so
a list and its count never describe different sets.

### 9.48 A RATE with a GOAL in the denominator is not a SUCCESS RATE

The adherence rate is `published / max(scheduled, goal x days)`. It
was labelled "Publish success rate" and sat directly above "11 of 13
scheduled posts went out" - which is 85%, against a displayed 24%.

A success rate has to be a success rate. The API returns both now
(`success_rate` and `adherence_rate`), and each surface shows the one
whose caption names the formula it uses.

### 9.49 A MODAL that is not in the modal manager needs its own contract

The onboarding tour was a full-screen `role="dialog"` with no
accessible name, no `aria-modal`, no focus trap and no Escape
handler, and a raw `bg-black/70` scrim that did not retheme. It is
not in the modal manager, so none of the shared machinery applied.

Every dialog needs: a name (a label, or an id pointing at a heading),
`aria-modal="true"`, a focus trap on the PANEL, an Escape handler on
the PANEL, and the `overlay` token for its scrim.

`frontend/scripts/audit-dialogs.mjs` checks the contract over the
real app, and the onboarding tour is dismissed AFTER the login - the
layout only raises it once the app is authenticated, so dismissing it
before the login does nothing.

### 9.50 A PROBE that renders ONE state is a probe that misses the other states

The calendar has THREE views - week, month and day - and each is its
own component with its own layout. The probe rendered the default
(week) and the month and day views were NEVER rendered. The month
view's chip was 15px of a 144px cell, and the day view's ghost slot
was 21px.

`--state "Month"` drives a page into a named state, and the three
calendar views are all probed. The same applies to the modals: a
dialog that only appears after a click is a dialog no route-level pass
ever rendered.

### 9.51 A query PARAMETER that reaches a date library PANICS the handler

`tz_offset` came off the query string as a raw `Option<i64>` and went
straight into `chrono::Duration::seconds`, which PANICS past
i64::MAX/1000. There is no CatchPanic layer in the router, so
`tz_offset=9223372036854775807` took the whole `/api/calendar` handler
down - and the same shape exists in ANY parameter that feeds a date
or duration library.

The bound has to be in the handler: a real UTC offset is within 18
hours, and anything beyond a DAY is refused with a 400 that names the
bound. The shared helper enforces it too, because a shared helper is
called from places that did not validate.

THE TEST TRAP, and it is subtle: a value the EXTRACTOR rejects proves
nothing about the handler. axum's `Query` deserializes the whole query
string BEFORE the handler runs, and its own 400 is
indistinguishable from the handler's unless the test asserts on the
BODY. A 20-digit `tz_offset` never reaches the handler at all, so a
test using one passed with the validation DELETED. Use a value that
DESERIALIZES and then panics (i64::MAX is 19 digits), and assert the
response contains the handler's own message.

And the mutation run itself has a trap: a "Finished in 0.25s" build
after editing an 85k-LOC lib is a STALE BUILD, and a test that has
never seen its own failure proves nothing. A real rebuild of the lib
takes seconds; the mutated run must PANIC (`TimeDelta::seconds out of
bounds`), not pass.

### 9.52 A scratch DB keyed per BINARY is not isolated per RUN

`config::test_database_url()` names the file from the test binary's own
exe stem, and nothing deletes it. So a test that asserts an ABSOLUTE
value against data it seeds is reading its OWN previous runs' rows too.

`streak_unification_test` seeded "0, 1 and 2 days ago" and asserted a
3-day streak. It returned 5, deterministically, because the same binary
had run on Oct 19, 20, 21 and 22 and those rows were still there:
`sqlite3 target/test-data/streak_unification_test.db` showed published
posts on five distinct days. Both endpoints agreed on 5, which is what
made it a fixture bug and not a logic bug - the measure was consistent,
the input was not controlled.

The fix is to delete the test's own rows before seeding them, and the
operator's real DB must be checked separately (`SELECT COUNT(*) ...
WHERE state='published'` returned 0 here) to tell this shape apart from
the shared-file mode in 9.42, which is the one that "database is locked"
warns about.

Two traps in this one bug. `cargo test --lib` was 496/496 green
throughout - the suite is where it lived, so `--lib` alone is not a green
project. And restoring a mutated file from a `/tmp` backup that had
silently never been written left the mutation live and produced a second
failure in `resolve_local_user_test` that was only a build-lock race from
parallel cargo runs; `git checkout -- <file>` is the restore that cannot
lie.

### 9.53 The release binary embeds the frontend, so a rename can ship the old name

`src/api/mod.rs` pulls `frontend/build` in with `rust-embed` at compile
time. The source can be renamed perfectly and the SERVED page can still
say the old name, because what the browser runs is a chunk that was built
before the edit.

This bit twice in one pass. `frontend/src/app.html` said `Switchpost`
while `curl https://127.0.0.1:6544/` returned `<title>Social Forge</title>`
- `frontend/build/` was from an old session, and the release binary had
embedded it. Then a hardcoded demo subreddit (`r/socialforge`, next to
fake "1.2k" votes and "2h") survived in a lazily-loaded chunk after a
rebuild, because the source edit landed after `pnpm build` had read it.
Only a clean rebuild cleared it.

So the check is the SERVED page over TLS, not the source: grep `app.html`
for the title, grep `frontend/build/` for the old literal AFTER a clean
`rm -rf build && pnpm build`, and only then rebuild the release binary so
it embeds the new assets. A rename is not complete until the running
process serves the new name.

---

## 10. Environment Variables

See `.env.example` for the full list. Critical ones:

| Var | Required | Default | Purpose |
|---|---|---|---|
| `DATABASE_URL` | No | `sqlite://data/switchpost.db` | SQLite file URL (SQLite-only; non-sqlite schemes are rejected at startup) |
| `APP_PASSWORD` | No | auto-generated | Single-user password gate (persisted to `~/.switchpost/.env`) |
| `APP_URL` | No | `https://localhost:6543` | Public URL (used for OAuth redirect URIs + TLS decision) |
| `BIND_HOST` | No | `127.0.0.1` | Network interface to bind (`0.0.0.0` for LAN) |
| `FRONTEND_URL` | No | = `APP_URL` | CORS allow-list + CSRF origin. Must match the browser origin including port; a `--port` other than the `APP_URL` port fails `csrf_origin_mismatch` on state-changing routes |
| `JWT_SECRET` | No | derived from `APP_PASSWORD` | HMAC key for session cookies |
| `TOKEN_ENCRYPTION_KEY` | No | — | 64 hex chars (32 bytes) for AES-256-GCM token encryption at rest |
| `DB_MAX_CONNECTIONS` | No | `1` | SqlitePool max connections (kept at 1 — the pool serializes writers) |
| `PROVIDER_CONCURRENCY_X` | No | `1` | Per-provider concurrent publish limit (override) |
| `PROVIDER_CB_THRESHOLD_X` | No | `5` | Per-provider circuit breaker failure threshold (v10) |
| `PROVIDER_CB_COOLDOWN_X` | No | `60` | Per-provider circuit breaker cooldown seconds (v10) |

---

## 11. Quick Reference: CLI + MCP Commands

### CLI (preferred for shell-based AI agents)
```bash
switchpost --help
switchpost providers               # List connected accounts
switchpost post "hello" --platforms x,linkedin
switchpost posts list
switchpost posts publish <id>
switchpost media upload ./photo.jpg
switchpost x timeline
switchpost reddit browse rust
switchpost automation list
```

### MCP (for Claude Desktop / Cursor)
```json
{
  "mcpServers": {
    "switchpost": {
      "command": "switchpost",
      "args": ["mcp"]
    }
  }
}
```

### Toolset filtering (per-session tool budget)

Boot default is `core` — the cross-platform lifecycle tools
(posts/stage/media/tags/notifications/analytics/setup/integrations/feed)
plus the two meta tools. A session that only touches X does not pay
389-schema context tax.

- `SWITCHPOST_TOOLSETS=x,reddit` (comma-separated, or `all`) — boot-time
  selection; `tools_enable` adds sets at runtime.
- `tools_search` searches ALL tools including disabled ones — use it
  before `tools_enable` to discover the set names (provider prefixes:
  `x`, `reddit`, `facebook`, `instagram`, `linkedin`, `telegram`,
  `discord`, `whatsapp`, `youtube`, `google`, `github`, `bluesky`,
  `mastodon`, `skool`, `slack`, `threads`, `tiktok`, `pinterest`,
  `wordpress`, `medium`, `devto`, `hashnode`, `webhooks`, …).
- Shell sizing a session before enabling:
  `switchpost mcp-tools --toolset x --pretty` (same membership rule
  as the in-session filter; plain output lists names only).

### Key MCP tool categories (389 total)
| Category | Sample tools |
|---|---|
| Posts | `posts_create`, `posts_list`, `posts_publish`, `posts_repeat`, `posts_set_tags`, `posts_stage` |
| Media | `posts_media_upload`, `posts_media_list`, `media_delete` |
| Engagement | `comments_get`, `comments_reply`, `dm_send`, `dm_list` |
| Per-platform | `x_create_tweet`, `reddit_search`, `bs_timeline`, `ms_get_analytics`, `yt_reply_comment` |
| Integrations | `integrations_list`, `integrations_refresh`, `integrations_disconnect` |
| Signatures | `signatures_list`, `signatures_create`, `signatures_update`, `signatures_delete` |
| Analytics | `analytics_summary`, `feed_analytics`, `analytics_get_post` |
| Automation | `automation_create`, `automation_list`, `automation_logs` |
| Webhooks | `wh_create`, `wh_list`, `wh_test` |

---

## 12. When You're Done

Before declaring a task complete, verify ALL of these:

- [ ] `cargo check --lib --bin switchpost` → 0 errors
- [ ] `cargo test --lib` → all tests pass
- [ ] `cd frontend && pnpm build` → succeeds
- [ ] `cd frontend && pnpm exec svelte-check --threshold error` → 0 errors
- [ ] `git add -A && git commit -m "..."` with a descriptive message
- [ ] **`git push github master`** ← THIS IS THE MOST IMPORTANT STEP
- [ ] Confirm the push succeeded: `git log --oneline -1 github/master` shows your commit on the remote ref

If you can't verify one of these (e.g. missing `cmake`), say so explicitly to the user and explain what they need to verify locally.

### 12.1 The visual gate - when the change touches the UI

A change that touches the UI is not done until it has been RENDERED and
MEASURED. Three gates, all over the running app, not the source:

```bash
make audit URL=https://localhost:6544 PASSWORD="$APP_PASSWORD"
```

| Gate | What it covers | Why the source is not enough |
|---|---|---|
| `frontend/scripts/probe.mjs` | 27 routes x 3 widths x 2 themes: overflow, text overlap, broken words, collapsed items, contrast, tap targets, font fallback, heading order, type scale | A source read cannot see a 0.5px collision or a 4.39:1 pair |
| `frontend/scripts/audit-targets.mjs` | the RENDERED target size, driving the real app | The probe only renders each page's DEFAULT state, so the composer, its panels and the popovers are never measured |
| `frontend/scripts/audit-contrast.mjs` | the RENDERED contrast against the EFFECTIVE background, same driving | The same: a chip on a tinted wash only exists once the panel is open |

Each exits **3 (BLOCKED)** when the browser is missing - BLOCKED is never
reported as passed - and 1 when it finds something.

**The two browser-side audits are the ones that find what the probe
cannot.** A page that is only ever rendered in its default state is a
page whose other states are never checked. When you add a panel, a
popover or a mode, add it to the audit's route list, or it is a surface
nobody measures.

Two exemptions both gates make, and both are WCAG's: a DISABLED control
is not content the reader can act on, and an `aria-hidden` element is
decoration. The two gates must agree on this - a copy of the measure
that drifts is a gate that lies.

