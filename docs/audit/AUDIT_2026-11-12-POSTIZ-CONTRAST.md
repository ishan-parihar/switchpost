# Audit 2026-11-12 — Implementation gaps against postiz-app, red-team, and lean pass

> Date: 2026-11-12
> Subject repos: `switchpost` (this repo) vs `postiz-app` (reference, `../external-repos/postiz-app`, HEAD `64f00fd6`, 2026-10-28)
> Load assumed: ONE founder, ONE process, ONE SQLite file, ~20 connected channels, a few thousand posts, plus AI-agent traffic over `/mcp` and the CLI. Nothing here assumes multi-user load.
> Output: this doc + the worklog entry. **No code changed.** Every finding was verified by reading the cited lines; anything only inferred is marked UNVERIFIED.

## 1. Counts (measured against this commit)

| | switchpost | postiz-app |
|---|---|---|
| code | 103,097 LOC Rust (`src/`, 192 files) + 32,687 LOC Svelte (29 routes) | 27,228 LOC of provider code (38 files) in a TS monorepo (NestJS + Next.js + a Temporal orchestrator) |
| providers | 29 registered (12 Tier-1 + 17 Tier-2), 1 Tier-3 behind a flag | 37 social providers |
| agent surface | 392 MCP tools, CLI, REST | one agent ("postiz") with ~23 tools; no CLI |
| interfaces | 3 (CLI + REST + MCP over stdio and HTTP) | 1 REST + CopilotKit chat |
| storage | SQLite, single binary | Postgres + Redis + Temporal + Elasticsearch (docker-compose) |
| tests | 71 files in `tests/` (23,753 LOC) + 583 inline in `src/`; 1,068 `#[test]`/`#[tokio::test]` total | jest, no count taken |
| data model | 8 migrations, additive-only | 88 Prisma models, 45 of them `mastra_*` |
| deps | 54 direct in `[dependencies]` + 1 patch | ~120 root deps incl. Stripe/OpenAI/AWS SDK |

## 2. The contrast, in one paragraph

Postiz is a multi-tenant SaaS: every scheduled post is a durable Temporal workflow, media lives on local disk or Cloudflare R2, analytics are a live provider call cached in Redis for an hour, and publishing, billing (Stripe), agency, and marketplace all hang off `Organization`. Switchpost is the opposite trade — one binary, one SQLite file, one process — and it is AHEAD of postiz on four things that matter to a solo founder: **provider comment read+reply (12/12 Tier-1 vs postiz's threaded team comments), a working DM surface (x/linkedin/instagram vs none), delete-published-on-platform (4 providers vs 1), and an agent-facing triple interface (392 MCP tools + CLI vs ~23 tools).** Postiz is ahead on: publish-failure classification formalised as a workflow contract, media upload hardening (byte-sniffed caps, random dated keys, range-capable serving), per-provider concurrency caps, and a resumable-upload abstraction switchpost already implements once (§ C1 in `TODO.md`).

### 2.1 What to steal from postiz (cheap, no new infra)

| From postiz | Where it would live here | Cost |
|---|---|---|
| **Publish failure classification with "unknown outcome => never retry"** (`apps/orchestrator/src/workflows/post-workflows/post.workflow.v1.1.2.ts:250-307`) | `src/scheduler/mod.rs` retry policy — POST-204-after-timeout must not be retried (switchpost already does this for `Network`, but not for a timeout-after-send) | ~20 lines + 1 test |
| **Refresh-before-publish: refresh once, retry, else mark `refresh_needed` and fail the post** (`refresh.integration.service.ts`; `post.workflow.v1.1.2.ts:190-208`) | `resolve_token` in `src/scheduler/mod.rs:1842` and `src/services/posts.rs` | ~15 lines |
| **Byte-sniffed MIME + per-type caps + random dated media keys + range-capable serving** (`upload/custom.upload.validation.ts`; `local.storage.ts`) | already present in `src/api/media.rs` — **extend it to the MCP/CLI upload path** (finding 8) | 0 new infra |
| **Per-provider concurrency caps** (`temporal.module.ts:52-82`) | `ProviderRegistry` Semaphore — already exists; apply it to the analytics refresher too (finding 8) | 0 new infra |
| **Anchored, resolvable post comments** (`schema.prisma:428-456`) | `post_engagement` / `comments` — a single-user variant, no assignee | small |

### 2.2 What NOT to copy (would break a stated non-negotiable)

Temporal, Redis, Postgres, Elasticsearch, Stripe, Cloudflare R2 (as a requirement), the org/role/billing/agency/marketplace half (45 `mastra_*` models + Orders/Messages/PayoutProblems), CopilotKit's chat shell, and postiz's 13 frozen copies of the publish workflow kept for in-flight executions. Postiz's own dead weight confirms the filter: five marketplace models have no code reference at all, and 45 `mastra_*` tables back one agent with 23 tools.

## 3. Per-platform gap matrix, re-measured today (Tier-1, 12)

Measured by which trait methods each provider overrides in `src/social/*.rs` (a default means the capability is absent).

| capability | Tier-1 coverage | gaps |
|---|---|---|
| `publish` | 12/12 | — |
| `post_analytics` | 12/12 | — |
| `get_post_engagement` | 12/12 | — |
| `get_post_comments` (read) | 12/12 | — |
| `reply_to_comment` | 10/12 | tiktok, pinterest |
| `comment` (post a comment) | **2/12** | x, linkedin(+page), instagram, instagram-standalone, threads, youtube, tiktok, bluesky, pinterest |
| `delete_published` | 4/12 | linkedin-page, facebook, linkedin, x |
| `edit_published` | **0/12** | all 12 (no provider overrides it; nothing calls it) |
| `search_mention` | 9/12 | linkedin-page, tiktok, pinterest |
| `targets` | 5/12 | x, threads, instagram, instagram-standalone, bluesky |
| `pages` | 6/12 | x, linkedin, threads, instagram-standalone, bluesky |
| `needs_cron_refresh` | **7/12** | x, facebook, instagram, bluesky, pinterest |
| `required_settings` | 3/12 | only reddit + pinterest declare a required key (discord/skool/hashnode are Tier-2) |
| `get_recent_posts` (feed import) | 11/12 | instagram-standalone |
| `map_error` | 11/12 | pinterest |
| `format_mention` | **0/12 overrides** | the default `@{id}` is used by every provider |

The v25 depth plan (`docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md` §2) is now ~80% closed: rows 1, 3, 4, 7 and 10 are done; rows 5 (comment write path), 8 (`needs_cron_refresh` on the expiring Meta tokens) and 6 (mentions on the remaining Tier-1) are the ones still open. Row 8 is not cosmetic — see finding 5/6.

## 4. Findings

### Must fix (wrong result, security, data loss)

**1. Tokens are written in plaintext on five connect paths, even with `TOKEN_ENCRYPTION_KEY` set**
- `src/db/queries.rs:102-172` (`create_integration` — no crypto call at all), `src/services/integrations.rs:144-164` (encrypts, then **falls back to the plaintext token on failure**), `src/api/integrations.rs:157-174` (direct connect), `src/api/onboard.rs:665-682` (public onboarding), `src/api/onboard.rs:1025-1040` (X cookies), `src/api/onboard.rs:1590-1604` (Telegram bot token), `src/api/integrations.rs:2008-2023` (page connect).
- What this is: `create_integration` writes the raw token; only the OAuth callback in `services/integrations.rs` encrypts, and only `update_integration_token` (the refresh writer) does it internally.
- Problem: the founder sets the key, connects a channel through the browser or the page picker, and `integrations.access_token` holds a working OAuth token / cookie jar in the clear. One `sqlite3 .dump` or one backup copy leaks every connected account. The plaintext-on-encrypt-failure fallback is the exact downgrade AGENTS §6.3 forbids.
- Fix: move the encryption inside `create_integration` (same shape as `update_integration_token`), delete the per-caller encryption, and return `Err` on encrypt failure. **Migrate**: re-encrypt existing plaintext rows on startup.
- If we skip it: the advertised "AES-256-GCM at rest" is true for one path out of seven.

**2. `/api/proxy-media` re-serves an attacker-hosted SVG inline on the app origin**
- `src/api/media.rs:837-849` (allowlist), `src/api/media.rs:1004-1025` (serving), contrast `src/api/media.rs:803-819` (`serve_media` sniffs bytes) and `src/api/mod.rs:245-247` (route is on the public router but the handler needs a session, so the victim's own cookie authenticates it).
- What this is: the proxy fetches a URL and answers with the upstream `Content-Type`; anything `image/*` is served INLINE, with no byte sniffing.
- Problem: the allowlist includes `files.catbox.moe` and `i.imgur.com` — anonymous, user-uploadable hosts. An attacker uploads `x.svg` with `<script>` and sends the founder `https://app/api/proxy-media?url=…/x.svg`. The browser navigates, gets `image/svg+xml` inline on the app origin, and the script runs with `sf_session` — same-origin, so it can call every `/api` route (publish, delete, list integrations) as the founder.
- Fix: sniff the bytes and serve only PNG/JPEG/WebP/GIF/MP4 inline; force `Content-Disposition: attachment` for anything else including `image/svg+xml`; drop user-upload hosts from the allowlist (`i.ytimg.com`, `pbs.twimg.com`, `video.twimg.com`, `www.instagram.com`, `scontent-*.fbcdn.*` are enough). Also tighten the `scontent-` rule: today it is `host.starts_with("scontent-") && host.contains(".fbcdn.")`, which `scontent-evil.fbcdn.attacker.com` satisfies.
- If we skip it: a link in a chat message is account takeover on the app origin.

**3. A webhook URL is an authenticated SSRF with the response returned to the caller**
- `src/api/webhooks.rs:271-275` (only "non-empty" is validated), `src/api/webhooks.rs:468-473` (test endpoint), `src/services/webhook_dispatcher.rs:54-89` (default reqwest client — follows redirects — and returns a capped body), `src/api/webhooks.rs:512-514` (the body lands in the JSON result).
- Problem: `POST /api/webhooks {"url":"http://169.254.169.254/latest/meta-data/iam/security-credentials/"}` then `POST /api/webhooks/{id}/test` returns the cloud metadata in the response; a redirect from a public URL to `127.0.0.1:6543` works too.
- Fix: the same guard `src/api/media.rs:854-880` already implements for redirects — `https` only, resolve the host, reject loopback/private/link-local, re-check every hop, cap and redact the returned body.
- If we skip it: the app is an open SSRF proxy with read-back for anyone holding the session or an API key.

**4. An RSS feed URL is fetched server-side with no host check, and its content is shown back**
- `src/api/rss.rs:80-88` (any `feed_url` is stored), `src/api/rss.rs:129` (`reqwest::get`), `src/rss/mod.rs:44` (same in the poller).
- Problem: `http://10.0.0.5:8080/internal.xml` — the server fetches it and the title/link/content surface in the feed items list. A redirect to an internal address bypasses a check that only looked at the original URL.
- Fix: share one host/IP guard with finding 3, applied at store time and at every fetch (including the poller's).
- If we skip it: LAN and cloud-metadata probing from a stolen API key.

**5. A token refresh on a connected Facebook/Instagram page channel silently redirects every later post to the FIRST page of the account**
- `src/api/integrations.rs:618-673` (manual refresh overwrites the stored token), `src/api/integrations.rs:2008-2023` (`connect_page` stores the page token + the parent's USER refresh token), `src/social/facebook.rs:241-259` (`refresh_token` returns a USER token), `src/social/facebook.rs:807-853` (`resolve_page` takes the first page of `/me/accounts`).
- Problem: the founder clicks Refresh (or an agent calls `integrations_refresh`, `src/mcp/tools_admin.rs:232`) on the page channel. The user token replaces the page token. From then on `/me/accounts` succeeds, `resolve_page` returns the **first** page, and every scheduled post lands on that page instead of the connected one — no error, no warning.
- Fix: never overwrite a page token with a user token. On refresh, re-derive the page token for the stored `internal_id` and store that; make `resolve_page` select the page whose id matches the post's `internal_id` instead of the first entry.
- If we skip it: posts go to the wrong audience with no signal.

**6. Facebook and Instagram page channels have no refresh path at all, so they hard-fail at Meta's ~60-day expiry**
- `src/api/integrations.rs:2008-2023` (`token_expires_at = None` for a page sub-account), `src/scheduler/mod.rs:1848-1851` (`needs_refresh` is false when the expiry is NULL), `src/social/facebook.rs` / `instagram.rs` (no `needs_cron_refresh` override, so the proactive refresher never selects them).
- Problem: nothing ever refreshes the page token. When Meta expires it, every publish fails with an auth error and the founder has to reconnect by hand. This is the same class as G-01/G-02 in `docs/planning/OUTLINE_TOKEN_REFRESH.md`, one layer down.
- Fix: store the page token's real expiry, or mark Meta providers `needs_cron_refresh = true` and make the cron refresher re-derive the page token (see finding 5) so a refresh cannot repoint the channel.
- If we skip it: every Facebook/Instagram page channel stops publishing within two months of connecting.

**7. LinkedIn Page publishes to the first organization the token can administer, not the page the founder chose**
- `src/social/linkedin_page.rs:988-994` (`resolve_org_id` = `pages().first()`), `src/social/linkedin_page.rs:446-452` (`publish` uses it for the `urn:li:organization:` author), `src/api/integrations.rs:2008-2023` (the chosen page IS stored in `integrations.internal_id` and never read here).
- Problem: the founder administers two orgs and connects the second; the post is published under the first — same wrong-destination shape as finding 5, on a different provider.
- Fix: pass the post's integration `internal_id` into `publish` (extend `PostContent` or the trait) and use it as the author URN.
- If we skip it: every LinkedIn Page post can land on the wrong organization, and its analytics read the wrong org too.

**8. MCP `tags_update` binds its SQL parameters in the wrong order, so it can never rename a tag**
- `src/mcp/tools_tags.rs:166-179` — SQL is `name = COALESCE(?), color = COALESCE(?), WHERE id = ? AND user_id = ?` but the binds are `tag_id, user_id, new_name, new_color`.
- Problem: name ← uuid bytes, color ← user uuid, id ← "Launch", user_id ← the new colour. The WHERE never matches, so every call answers "Tag not found". REST `PUT /api/tags/{id}` and `switchpost tags update` work, so only the agent surface is broken.
- Fix: reorder the binds to `new_name, new_color, tag_id, user_id`; add a test that renames a tag through the tool.
- If we skip it: every agent falls back to delete+create and loses the tag id on every post that references it.

### Should fix (risky, no test, wrong result in an edge case)

**9. MCP/CLI media upload bypasses the MIME allowlist, the magic-byte sniff and the storage backend — and can copy any local file**
- `src/mcp/tools_media.rs:59-111` (base64 upload writes bytes straight to `state.config.media_dir`, stores the caller's MIME verbatim), `src/mcp/tools_media.rs:122-195` (`upload_from_path_with_alt` copies ANY local file with an extension-derived MIME), `src/cli/run.rs:4052` (wires it), `src/api/media.rs:58-65,596-615,692-700` (the checks that exist) and `src/api/mod.rs:195` (`GET /api/media/{id}` is public).
- Problem: `switchpost media upload ~/.switchpost/.env` puts the secret file in the media store at a public URL. On an S3/R2 deployment (`src/api/media.rs:198,490`) the MCP upload writes to the local directory while `serve_media` reads through the S3 backend, so the agent's upload returns a URL that 404s and the image never reaches the platform.
- Fix: make the MCP and CLI uploads call the same handler `POST /api/media` uses (allowlist + sniff + storage backend); refuse a non-media local path explicitly.

**10. The CLI is the only surface that can create an invalid post**
- `src/cli/run.rs:4231` (calls `PostService::create` directly) vs `src/api/posts.rs:525-585` (checks `integration.disabled` + `validate_posts_for_integrations`) vs `src/mcp/tools_posts.rs:586` (calls `api::posts::create`), `src/services/posts.rs:106-147` (only a global 2000-char ceiling), `src/cli/run.rs:4169-4178` (`--state published` is accepted).
- Problem: `switchpost posts create "hi" --integrations <discord> --schedule …` prints `{"ok":true}` for a post that REST/MCP refuse for a missing `channel` in settings; a 400-emoji Bluesky post is created and fails later; `--state published` writes a row the dashboard counts as published that was never posted. The CLI is what goes in cron.
- Fix: route the CLI through `api::posts::create`; drop or validate `--state published`.

**11. Only MCP can delete a post on the platform**
- `src/mcp/tools_posts.rs:476,738` (`delete_from_platform`), `src/api/posts.rs:1191-1203` (REST delete is local-only), no CLI flag, no UI affordance.
- Problem: the founder deletes a published post from the dashboard or the CLI and the post stays live on the platform — the one delete path that matters to an audience is MCP-only. AGENTS §0.5.4 asks for parity on core operations.
- Fix: put the same guard + the platform call behind REST `DELETE /api/posts/{id}?from_platform=1` and a CLI flag, sharing one helper.

**12. The feed poller reports 200 "new" posts every cycle and rewrites every row**
- `src/db/queries.rs:3582-3635` (`insert_external_post` — an UPSERT with `RETURNING`, so `fetch_optional` is **always** `Some`; plus two `tracing::info!` per row), `src/feed/mod.rs:537-581` (`Ok(Some(_)) => new_count += 1` — the "already exists" branch is dead), `src/feed/mod.rs:41-44` (default interval 900s).
- Problem: at the assumed load (20 channels × `RECENT_POSTS_LIMIT`) every imported row is rewritten, `new_count` counts every fetched post, so the log says "imported 200 new posts" and the SSE `feed:new_posts` event fires every 15 minutes with no new post — the frontend refetches for nothing, and the info log emits ~8,000 lines per cycle on the single writer.
- Fix: read `total_changes()` / `RETURNING … WHERE` so the upsert is a no-op for unchanged rows, return `Some` only for a genuine insert, and drop the two per-row info logs to `debug!`.

**13. Four preview panes count with their own rule and crop the wrong text**
- `frontend/src/lib/composer/previews/{General,Facebook,Instagram,LinkedIn}Preview.svelte` (each `plainText.length` + slicing the escaped string) vs `frontend/src/lib/composer/platforms.ts countFor` (used by the other five previews).
- Problem: a 600-emoji Instagram caption reads "600/2200" while the server counts 2400 bytes and rejects it. A caption of `&`s gets the "will be cropped" marker ~400 characters early.
- Fix: call `countFor`/`charLimitFor` and slice the plain text, like the other five.

**14. The calendar deep link opens the previous day in every negative-offset timezone**
- `frontend/src/routes/calendar/+page.svelte:533-539` (`new Date(urlDate)` parses `YYYY-MM-DD` as UTC midnight) vs `frontend/src/lib/api/campaigns.ts:100-114` (the `formatDateOnly` fix that already exists).
- Problem: `/calendar?date=2026-11-15` opens Nov 14 in America/New_York, and line 591 then writes the wrong day back to the URL and to localStorage.
- Fix: parse into a local `Date` or reuse the campaigns helper.

**15. `posts_list` silently ignores an unknown state and both agent surfaces lack the REST filters**
- `src/db/queries.rs:879` (an unrecognised state falls back to "all posts"), `src/mcp/tools_posts.rs:627-640`, `src/cli/mod.rs:670-679` (only state/limit/offset) vs `src/api/posts.rs:371-444` (state, q, integration_ids, tag_ids, campaign_id, sort, limit 1..200).
- Problem: `posts_list state="Published"` (capital P) returns every draft too, and the agent believes it filtered. `limit=0` returns zero rows. "Which posts carry tag X" is unanswerable from the agent surface.
- Fix: reject an unrecognised state; give MCP and CLI the same filter set; clamp limit to 1..200 there too.

**16. The CLI cannot reschedule a post at all**
- `src/cli/mod.rs:670-742` (no reschedule command), `src/cli/run.rs:4545-4568` (schedule only) vs `src/services/posts.rs:307-332` and `POST/PUT`/`posts_reschedule`.
- Problem: `switchpost posts schedule <published-id> <t>` fails with "Post not found after scheduling" — the post exists, and the message lies. A group cannot be moved as a unit.
- Fix: add `posts reschedule <id> <at> [--move-group] [--action …]`.

**17. API keys have no scopes, and a key can mint more keys**
- `src/api/developer.rs:99-152`, `src/auth/middleware.rs:132-171` (a key resolves to the same `AuthenticatedUser` as the cookie), `migrations/0001_init.sql:270-280` (no scope column).
- Problem: one leaked `sf_…` key can POST `/api/developer/api-keys` for a fresh permanent key, so revoking the leaked one does not evict the attacker. A key is silently equivalent to the password.
- Fix: add a `scopes` column and enforce it; refuse key creation on a key-authenticated request (or require an explicit scope).

**18. Every API-key request does a SQLite WRITE**
- `src/auth/middleware.rs:82-99` (`UPDATE api_keys SET last_used_at … RETURNING user_id` on every request).
- Problem: an agent doing N tool calls performs N write transactions on the single writer, serialising against the scheduler.
- Fix: `SELECT` first; update `last_used_at` at most once a minute or in a background sweep.

**19. Two unbounded reads: the analytics refresher is 500 serial provider calls, the calendar is an N+1 per post**
- `src/scheduler/mod.rs:2109-2267` (per integration: 1 call + 1 upsert, then 25 posts × 1 call, all awaited serially), `src/api/calendar.rs:257-268` (one `get_tags_for_post` round-trip per post) + `src/db/queries.rs:2040-2085` (no LIMIT), `src/api/research.rs:459-472` (the whole corpus in memory, window up to 3650 days).
- Problem: 20 channels ≈ 500 sequential provider calls per 30-minute cycle (~17 minutes held serially at 2s/call); a year-wide calendar range opens 300 posts with 301 sequential queries.
- Fix: run the per-post fetches through the existing provider `Semaphore`; fetch tags with one `WHERE post_id IN (…)`; cap the calendar range and the research window.

**20. The frontend "scope lost" classification exists only on REST; agent surfaces lose it**
- `src/api/posts.rs:1557-1571` (409 `ScopeLost` with provider + integration id) vs `src/mcp/tools_posts.rs:844-880` and `src/cli/run.rs:4570-4596` (raw provider string).
- Problem: after a scope is revoked, an agent's `posts_publish` says "Publish failed: …" with no machine-readable "reconnect this channel", so the agent retries a dead grant and reports a provider outage.
- Fix: share one classifier across the three surfaces (`mcp::rest_bridge` already exists).

### Nice to have

**21. `SocialProvider::edit_published` is dead surface** — zero call sites anywhere in the tree (`src/social/mod.rs:692-703` and its docs, ~14 lines). Either implement it on the four providers that have `delete_published` or delete it. *(Verified by grep: no reference outside the trait definition.)*

**22. `format_mention` has one default and zero overrides** — every provider uses `@{id}` (`src/social/mod.rs:799-801`), and `search_mention` exists on only 9/12 Tier-1. Either fill the two gaps or state the limit in `docs/platforms-and-cli.md`.

**23. A session JWT travels in a query string and is re-minted into links**
- `src/api/onboard.rs:73-82` (`?token=<jwt>` accepted), `src/api/onboard.rs:109-117` (a fresh 30-day JWT embedded in provider-card links).
- Problem: a full 30-day session in a URL — browser history, access logs, the Referer of the OAuth redirect. AGENTS §0.6 already flags the redirect-chain requirement; a one-time exchange code is the smaller exposure.

**24. `insert_external_post` returns `Some` for a no-op update** (already covered by finding 12) and `undelete_post` / `unresolve_comment` are dead (finding 26).

## 5. Lean findings (with before/after counts)

**25. Fourteen dead public functions — 246 lines, 0 call sites**
`src/db/queries.rs`: `create_repeated_post:2632`, `create_user_with_id:39`, `get_media_user:2459`, `get_post_engagement_by_post_id:3020`, `set_post_recurring:2661`, `undelete_post:1560`, `unresolve_comment:3865`, `update_external_post_metadata:3638`; `src/social/reddit.rs`: `submit_www:400`, `comment_www:425`; `src/social/wordpress.rs`: `list_tags:172`; `src/social/registry.rs`: `state_str:228`; `src/social/browser_cookies.rs`: `copy_cookie_db:23`; `src/services/telegram_rate.rs`: `peer_key:72`. Every one greps to zero hits in `src/`, `tests/`, `frontend/src`, `scripts/`, `migrations/`, `.sqlx/`. `set_post_recurring_with_copies` IS live (`src/api/posts.rs:1509`, `src/cli/run.rs:5191`) and stays.
**Fix: delete all 14. 246 → 0 lines.**

**26. 19 of 26 providers hand-roll error classification instead of calling `classify_error`**
`src/social/mod.rs:1323` is the one classifier. Only 7 providers call it (reddit, tiktok, youtube, bluesky, instagram-standalone, linkedin, linkedin-page); the other 19 re-implement status 401/403/429/404 checks.
**Problem:** the scheduler retries `RateLimited` but not `Auth`, so the same upstream response produces a different retry on a different provider.
**Fix: route the 19 through `classify_error` and keep only the message copy. ~450 → ~60 lines (estimate, -390).**

**27. Five copies of "should this token be refreshed?"**
`src/scheduler/mod.rs:1842`, `src/services/posts.rs`, `src/api/integrations.rs:1958`, `src/mcp/tools_admin.rs:230`, `src/feed/mod.rs:94`. The write-back is correctly single (`update_integration_token`, 7 call sites); the *decision* is not, so a token can refresh in one path and be marked `refresh_needed` in another — exactly the shape of G-01/G-02/G-05 in `docs/planning/OUTLINE_TOKEN_REFRESH.md`.
**Fix: one helper beside `update_integration_token`. ~180 → ~90 lines (estimate, -90).**

**28. The editor carries a third character-count rule and a second HTML-to-text converter**
`frontend/src/lib/composer/RichTextEditor.svelte:186,192,227` (UTF-16 `length`; the server counts UTF-8 bytes, X is weighted) and `:90-94` (an inline regex strip that drops text between angle brackets — the bug AGENTS §9.18 records). The pinned pair is `src/social/common.rs:566` + `frontend/src/lib/composer/post-text.ts` (12 tests).
**Fix: call the shared helper and `countFor`/byte length. -3 lines, no behaviour change.**

**29. Two wall-clock converters in the browser**
`frontend/src/lib/composer/wall-clock.ts` (pinned, 2 tests) and `frontend/src/lib/calendar/types.ts:57`.
**Fix: import the shared one. ~25 → ~13 lines (estimate, -12).**

**30. 21 live env vars are undocumented in `.env.example`**
`MEDIA_S3_BUCKET`, `MEDIA_S3_ENDPOINT`, `MEDIA_S3_KEY`, `MEDIA_S3_SECRET`, `WHATSAPP_CLOUD_{TOKEN,PHONE_NUMBER_ID,WABA_ID,VERIFY_TOKEN,APP_SECRET}`, `LLM_ENDPOINT`, `LLM_MODEL`, `NEYNAR_API_KEY`, `DUB_CO_API_KEY`, `DUB_CO_WORKSPACE`, `FEED_REFRESH_INTERVAL_SECONDS`, `STRIP_LINKS_FROM_X`, `SWITCHPOST_TOOLSETS`, `EVENTS_LOG_RETENTION_DAYS`, `REDDIT_COOKIE_TOKEN`, `DECKMILL_BIN`. (The template documents 56 vars; 0 of them are unread by code — the gap is the other direction.)
**Fix: add the 21 lines. 166 → 187 lines.**

**31. Two tests are missing where a mistake costs the most**
`src/rss/mod.rs` (272 lines, 0 inline tests, no `tests/rss*` file) — a parser that mis-reads a feed creates wrong posts unattended; and the scheduler's backoff jitter bounds + circuit-breaker half-open single-allow path, which no named test covers (`tests/scheduler_claim_test.rs` covers the claim).
**Fix: 2 test files, ~80 lines.**

**32. 380 of 389 MCP tools have no CLI identifier**
Expected by design (the CLI is grouped by command), but the parity claim in AGENTS §11 stays unmeasured.
**Fix: assert per-command coverage in `tests/mcp_parity_test.rs` (~40 test lines).**

`Lean: -251 measured lines, -390/-90/-12 estimated by two unmeasured refactors, +151 test/template lines. 0 dependencies can be dropped (the 54 direct deps each earn their place: `wreq` for X's TLS fingerprinting, `grammers` for Telegram-user, `wa-rs` for WhatsApp Web, `sqlx` for the DB, `rmcp` for MCP).`

## 6. Not checked

- Runtime behaviour: nothing was executed (`cargo test`/`clippy` were not run this pass); the S3 404 and the tag-bind defect are read-level derivations that each need one test to pin.
- `src/api/{feed,comments,dms,analytics in full,activity,automation,campaigns,notifications,profile,sets,ai,sse,rate_limit,whatsapp_webhook}.rs` beyond the lines cited; 46 of the 47 `src/mcp/` handler modules; `src/social/*` beyond the providers named; `src/wa`, `src/realtime`, `src/services/media_transform.rs`.
- The 33k LOC frontend beyond the rule seams named above and the keyed-`{#each}` / `@html` / `$state(new Map/Set)` sweeps (both clean: `@html` escapes before highlighting, and the three `$state(new Set)` sites reassign after mutation).
- Provider `ErrorKind` variant reachability (never-returned variants not enumerated), deprecated crates, and column-level migration dependencies.
- postiz's `apps/frontend` (33k+ TSX) beyond the components cited, and its `apps/extension`.

## 7. Suggested order for the upgrade

1. **Security week (findings 1-4, 9, 17, 23)** — encryption inside `create_integration` with a migration for existing plaintext rows, one shared SSRF guard for webhooks + RSS, the proxy-media sniff, and API-key scopes. Each fix gets one test that fails before and passes after.
2. **Wrong-destination week (5, 6, 7, 12)** — the Meta/LinkedIn page-token contract: one shared "resolve the target this channel is connected to" rule, the page-token re-derivation on refresh, `internal_id` threaded into `publish`, and the honest `insert_external_post` return.
3. **Parity week (8, 10, 11, 15, 16, 20)** — the tag-bind fix, the CLI routed through the REST handlers, platform delete on all three surfaces, the filter/state parity, reschedule, and the shared error classifier.
4. **Frontend + load week (13, 14, 18, 19)** — the four previews, the calendar date parse, the `last_used_at` write, the semaphore on the analytics refresher, and the calendar's single tag query.
5. **Lean + tests (21-32)** — delete the 246 dead lines, route the 19 error copies through `classify_error`, extract the one refresh-decision helper, add the RSS + scheduler tests, and document the 21 env vars.

Every item above is small and independent; none adds a dependency, a process, or a new store.

### 7.1 What each week actually landed

The plan above was executed in five weeks, one commit group per week, each
pushed to `github/master`.

| Week | Findings | Landed in | The result the founder sees |
|---|---|---|---|
| Security | 1-4, 9, 17 | `2f4ad9c`, `c0cd99d`, `69021bd`, `03c10e1`, `ebd25ca`, `61176d1` | Tokens are encrypted at rest (with a migration for the rows that were already plaintext), one SSRF guard covers the webhooks and the RSS poller, the media proxy sniffs the bytes it serves, an API key has scopes and cannot mint another key, and the setup codes are single-use. |
| Wrong destination | 5, 6, 7, 12 | `d5a1a76` | A Facebook Page / Instagram Business / LinkedIn Page channel publishes to THE channel it is connected to, and one refresh path re-derives the page token instead of overwriting it. The feed importer says "new" only when a row is new. |
| Parity | 8, 10, 11, 15, 16, 20 | `eddbb07`, `87fd33d`, `2764e6d`, `56c023c`, `916d073` | The agent's `tag_update` renames a tag (it could not before), the CLI's `posts create` goes through the REST handler, a post can be deleted from the platform on all three surfaces, an unknown list filter is refused instead of silently ignored, a post can be re-scheduled, and ONE error classifier decides what a failure is — so a rate limit is retried everywhere and a 401 refreshes its token everywhere. |
| Frontend + load | 13, 14, 18, 19 | `5b6686d` | The composer counts the way the server counts (the four previews that did not were wrong), the calendar's deep link opens the day the URL names in every timezone, and the unbounded reads are bounded: the refresher's per-post fetches are concurrent and semaphore-bounded, the calendar's tags come from one query, and the research window is capped. |
| Lean + tests | 21-32 | the lean + tests commit | 260 dead lines are gone, the refresh DECISION is one rule with two look-aheads, the editor and the calendar no longer carry their own copies of a rule, the 21 undocumented env vars are documented, and the RSS parser, the backoff, the circuit breaker, the expiry rule and the CLI/MCP parity all have tests. |

### 7.2 What the parity measurement found

Finding 32 said the CLI/MCP parity claim "stays unmeasured". Measuring it
(`tests/mcp_parity_test.rs`) found a real hole: the MCP has `set_create` /
`set_list` / `set_delete` in its CORE toolset and the CLI had NO verb for
them at all. The CLI now has `sets list | create | delete`.

Two registered providers still have no CLI family of their own
(`instagram-standalone`, `google_my_business`). Both are reachable through
the unified `post --platforms <identifier>` path, and the test lists them
explicitly so a third one has to be a decision rather than an accident.
