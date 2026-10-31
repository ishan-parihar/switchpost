# Worklog — switchpost multi-agent log (§8 protocol)

## Contents

- [kb-adoption-2026-10-17](#kb-adoption-2026-10-17) — adopt the doc genome, wire it to the repo's existing conventions
- [genome-migration-2026-10-19](#genome-migration-2026-10-19) — kb.toml → genome.toml, retire the legacy `scripts/kb`
- [red-team-web-dashboard-2026-10-19](#red-team-web-dashboard-2026-10-19) — red-team pass over the web dashboard
- [red-team-web-dashboard-2026-10-19-continued](#red-team-web-dashboard-2026-10-19-continued)
- [red-team-web-dashboard-2026-10-19-final](#red-team-web-dashboard-2026-10-19-final)
- [red-team-web-dashboard-2026-10-20-iteration-2](#red-team-web-dashboard-2026-10-20-iteration-2)
- [red-team-web-dashboard-2026-10-20-final](#red-team-web-dashboard-2026-10-20-final)
- [red-team-iteration-3-5](#red-team-iteration-3-5)
- [v12-red-team-iterations-6-7](#v12-red-team-iterations-6-7)

Before starting work: read this file top to bottom. After finishing: append a
section using the template in AGENTS.md §8 — never overwrite. One-liners can
also be appended via `scripts/genome log "<title>" --body-file <file>`.

---
Task ID: kb-adoption-2026-10-17
Agent: omp (memory-wire session)
Task: Adopt the project-kb documentation genome (map-in-place, wiring switchpost's existing conventions to the tooling).

Work Log:
- Surveyed the doc tree: the dated outline succession (OUTLINE-2026-10-16 → -17) already implements dated-frozen + supersede informally; AGENTS.md §9 is the regression ledger; the outline's standing notes are the closed list.
- Mapped roles in `kb.toml`: plans/audits → docs/planning, register → TODO.md (new, seeded from OUTLINE-2026-10-17), decisions → docs/decisions (new), archive → docs/archive (new, tombstone convention), worklog → this file; ledger/closed/regressions/workflows/state/feed/runbook deferred to their native mechanisms (`native:` notes).
- Seeded TODO.md: V1a–V1d/C1/C2/R1 open, P6.1–P6.3 + MCP-identity rejected-on-evidence, B12/WA-inbox blocked with unblock conditions.
- Repaired AGENTS.md §0.6: added the OUTLINE-16/17, TODO.md, worklog.md, docs/decisions rows; replaced the "doesn't exist" rule with the tombstone rule.
- Fixed §8's worklog path (was a foreign machine path, `/home/z/my-project/worklog.md`).
- Copied `scripts/kb` (stdlib-only, single file); `.gitignore` gains `*.lock`.

Stage Summary:
- Artifacts: kb.toml, TODO.md, worklog.md, docs/decisions/README.md, docs/archive/INDEX.md, scripts/kb, AGENTS.md §0.6+§8 edits, .gitignore.
- Verification: `cargo check --lib --bin switchpost` 0 errors (docs-only diff; clippy/tests unaffected); `scripts/kb lint` clean; `scripts/kb doctor` healthy.
- Known follow-ups: §9.4 still references the old machine's `scripts/cargo-env.sh` (left as-is — outside this task's scope); AGENTS.md tool-count assert sites (×3) unchanged — no tool surface touched.

---
Task ID: genome-migration-2026-10-19
Agent: omp (genome migration session)
Task: Migrate switchpost from the pre-genome kb.toml to the genome's own config, and retire the legacy project-kb CLI.

Work Log:
- Read the legacy kb.toml and mapped every role it declared. The role→path map moves to genome.toml unchanged, except the retired `closed` role (same standing-notes list as `ledger`, so `ledger` already carries its meaning) and `dated_pattern`, which moves from [paths] to [core] where the genome reads it.
- The legacy file declared no `asks` and no `records_index`. Both are declared now, so the module set does not silently change: `asks` is a `native:` pointer (this repo has no operator-questions register; open questions live in TODO.md's open/blocked rows), and `records_index` points at the AD store's own README. `records.zones = {}` keeps `genome rec emit` from writing generated zones into that prose.
- `modules = ["core", "records", "agents"]` — the legacy kb.toml set, declared explicitly so the software profile's `core, records` does not silently drop the agents module.
- Added the genome runtime ignores to .gitignore (`*.genome-lock`, `*.genome-tmp`, `.genome/`), matching the fleet convention; the repo's existing `*.lock` did not cover `*.genome-lock`.
- Archived `scripts/kb` to `docs/archive/kb` with a tombstone at the original path and a row in `docs/archive/INDEX.md`: it is a vendored CLI that reads only kb.toml, which this repo no longer has, so it had become a dead second copy of the role map.
- Repointed the live doc references at the genome verbs: AGENTS.md §0.6 (the TODO.md and docs/decisions/ rows), TODO.md's header, docs/decisions/README.md. The dated and frozen docs (docs/planning/*, docs/archive/*, docs/kb/SESSION_2026-10-19.md, TODO.md's row evidence cells) keep `scripts/kb` where it records what was run at the time. This worklog's own header line still names `scripts/kb log` because the worklog is an append-only role: editing a line trips `genome doctor`'s append-only check, so the correction is this entry instead.

Stage Summary:
- Artifacts: genome.toml (was kb.toml), scripts/genome re-pinned to 2026.10.19-3, scripts/kb archived to docs/archive/kb, .gitignore, AGENTS.md, TODO.md, docs/decisions/README.md, docs/archive/INDEX.md, worklog.md.
- Verification: `scripts/genome lint --all` 0 errors / 4 warnings (the same 4 W010s as before the migration; W507 gone with the retired `closed` role). `scripts/genome doctor` healthy. `genome rec emit` is a no-op.

---
Task ID: red-team-web-dashboard-2026-10-19
Agent: omp (red-team + visual iteration session)
Task: Exhaustive red-team of the web dashboard (composer, calendar and scheduling, campaign planning, analytics, research, read-only activity log), fix every defect found, and iterate the visual layer with browseros-neo against a populated instance.

Work Log:
- Four read-only scouts audited the surfaces in parallel (composer/scheduling, calendar/campaigns, analytics/research/activity, and a cross-cutting API-contract diff of 124 axum routes against 102 frontend call sites). Their reports were cross-checked against the source before any fix; every claim that did not hold was dropped.
- The operator's own database had 1 post, 0 campaigns, 0 tags, 0 analytics - so the visual pass could only ever see empty states. Built an isolated seeded instance (a copy of the real DB plus a crafted dataset) on its own port, which is what made the populated-state defects visible: a bar that drew no comparison, a count clipped by its own bar, a date a day early, a metric wearing the wrong name.
- Extended the repo's probe with the website-design skill's craft checks (broken words, font fallback, collapsed items, doc basics, type/color sprawl, section repetition) rather than running two probes, and taught it to report 404 URLs instead of a count, to break advisories down by control and by width, and to measure the EFFECTIVE tap target (a checkbox inside its own label is judged by the label).
- The composer was the worst surface: "Post Now" could never publish (it created a draft and the publish endpoint refused drafts); clearing a field did not clear it; the rich-text editor's HTML was stored and published verbatim; every Reddit and Pinterest post failed at publish because the composer sent `{integrationId: {target_ids}}` where the providers read flat provider-named keys; a photo with no caption could not be composed at all; the thread builder created duplicate threads from an edit screen; a thread's other parts were never saved.
- The calendar put posts on the wrong day: `toCalendarEvent` sliced the UTC timestamp instead of converting it into the user's zone, and the API's range end was midnight of the end date, so the last day of every month and week was empty. Both are fixed and the conversion is proven in the browser (03:30Z renders as 23:30 in America/New_York).
- The analytics surface counted the wrong things: the "vs previous period" delta was a count rendered with a `%`; the metric labels never matched X or Bluesky, so those users read 0 comments and 0 shares forever and their best posts were EXCLUDED from the research ranking; the 90-day window was a no-op because `external_posts.created_at` was declared INTEGER epoch but written as RFC 3339 TEXT, and SQLite orders every INTEGER before every TEXT; the Activity page CRASHED on "Load more" because the cursor was inclusive and the list is a keyed `{#each}`.
- The visual pass took 4056 tap-target advisories down to ~1000 by putting a 44px target on every control the probe named - the sidebar's thirteen links were 36px, the bell 32px, the dropdown trigger 16px, the feed's text buttons had no padding at all. The remaining advisories are the calendar's time-grid chips, whose size is the grid's design.
- Fixed 44px targets without moving a single glyph, and without changing the visual weight of any row: the target grows around the control, and where the control was a checkbox inside a label, the label carries the target and the box stays 16px.

Stage Summary:
- Artifacts: 12 commits on master, pushed to github/master (23f6e67). Frontend: lib/api/{posts,campaigns,integrations,automation,calendar,analytics}.ts, lib/{providers,calendar/types,calendar/ListView,calendar/CalendarHeader,calendar/MonthView,calendar/PostDetail,calendar/PostHoverToolbar,composer/ComposerModal,composer/platforms,composer/SchedulePicker,composer/wall-clock,composer/PerPlatformCharCount,ui/Dropdown,ui/TopBar,ui/EmptyState,notifications/NotificationBell}.svelte, routes/{+layout,posts,posts/new,calendar,campaigns,campaigns/[id],analytics,research,activity,feed,kanban,tags,settings/notifications}. Backend: api/{posts,campaigns,calendar,activity,analytics,notifications,feed,integrations}.rs, db/{queries,models,types}.rs, social/{mod,x}.rs, scheduler/mod.rs. Migrations: 0007_external_posts_epoch.sql. New tests: composer_contract_test.rs, external_posts_window_test.rs, counting_rule_test.rs, plus calendar_range_test.rs and kanban_stage_test.rs extended. Probe: frontend/scripts/probe.mjs gained 404 capture, per-control advisories and the skill's craft checks.
- Verification: 481 lib tests + 35 integration tests pass; `cargo clippy --lib --bins --all-features -- -D warnings` 0 errors; `svelte-check` 0 errors / 0 warnings; `pnpm build` clean; the probe over 13 routes x 3 widths x 2 themes reports 0 FAIL, 0 console errors, 0 404s. Every fix that could be reproduced was reproduced against the live API before and after (the draft publish, the Reddit settings shape, the inclusive calendar range, the campaign dates and metric, the caption-less post, the activity cursor, the emoji count).
- Known follow-ups: the remaining ~1000 tap-target advisories are the calendar's hour-grid chips (21px) and a handful of text buttons whose width is under 44; both are structural, and the month view's chips are already inside a 44px cell. The `live_*` test binaries still hit the real provider APIs and fail intermittently on rate limits - that is environmental, not a regression.

---
Task ID: red-team-web-dashboard-2026-10-19-continued
Agent: omp (same session, continued)
Task: The remaining findings from the four scout reports, and the second visual iteration.

Work Log:
- The composer was still the worst surface after the first pass. Fixed, in order of how badly they hurt: the provider settings shape (every Reddit/Pinterest post failed at publish); the update path dropping the per-channel body and settings; the thread builder creating duplicate threads from an edit screen; a thread's other parts never being saved; a photo with no caption being impossible to compose; the character count the composer showed (UTF-16 units) differing from the one the server enforced (UTF-8 bytes), and X's weighted rule existing only in the browser.
- The analytics surface had four more: the "vs previous period" delta was a count rendered with a `%`; the metric labels never matched X or Bluesky, so those users read 0 comments and 0 shares forever AND their best posts were excluded from the research ranking; the 90-day window was a no-op because `external_posts.created_at` was written as RFC 3339 TEXT into an INTEGER column (SQLite orders every INTEGER before every TEXT, so `created_at >= ?` was always true); the Activity page crashed on "Load more" because the cursor was inclusive and the list is a keyed `{#each}`.
- The campaign surface had four: the end date was a day early (a bare `YYYY-MM-DD` is a calendar date, and `new Date()` parses it as UTC midnight); the progress bar measured the post count against an engagement target; the bar clipped its own count at the rounded end; and emptying a field did not empty it (`|| undefined` is "omitted", and `COALESCE` cannot tell "keep" from "clear").
- A second sweep of the rendered pages found three more that only looking would catch: the channel-mix bar's number was clipped by its own bar exactly when the bar was widest; the research "best time to post" bars were all full width because there was no maximum to scale against; and the dashboard's "Channel Performance (30d)" card was fed a 7-day window.
- The tap-target pass went from 4056 advisories to under 800. The probe learned to measure the EFFECTIVE target (an input inside its own label is judged by the label) and the fixes learned to put the target on the LABEL, so a 16px checkbox in a dense grid still has a thumb-sized hit area.
- Every fix that could be reproduced was reproduced against the live API, before and after: the draft publish, the Reddit settings shape, the inclusive calendar range, the campaign dates and metric, the caption-less post, the activity cursor, the emoji count, the engagement sort, and the campaign field clearing.

Stage Summary:
- Artifacts: 6 more commits on master, pushed to github/master (3121a25). New: tests/counting_rule_test.rs (4 tests). Migrations: none beyond 0007. Frontend: lib/api/{campaigns,media,posts}.ts, lib/composer/{ComposerModal,MediaUpload,platforms}.svelte, lib/calendar/{MonthView,WeekView,DayView,ListView}.svelte, lib/{ui/Modal,media/MediaPopover,composer/MusicPicker,onboarding/GettingStarted}.svelte, routes/{+layout,+page,posts,posts/new,calendar,campaigns,campaigns/[id],analytics,research,activity,feed,kanban,tags,settings/developer,settings/notifications}. Backend: api/{posts,campaigns,activity,notifications,feed,integrations}.rs, db/{queries,models,types}.rs, social/{mod,x}.rs, scheduler/mod.rs, mcp/tools_campaigns.rs.
- Verification: 481 lib tests + 38 integration tests pass; `cargo clippy --lib --bins --all-features -- -D warnings` 0 errors; `svelte-check` 0 errors / 0 warnings; `pnpm build` clean; the probe over 13 routes x 3 widths x 2 themes reports 0 FAIL, 0 console errors, 0 404s.
- Known follow-ups: the remaining tap-target advisories are the calendar's hour-grid chips (21px, structural - the grid IS the design) and a few text buttons whose WIDTH is under 44. The `live_*` test binaries hit the real provider APIs and fail intermittently on rate limits; that is environmental.

---
Task ID: red-team-web-dashboard-2026-10-19-final
Agent: omp (same session, final verification)
Task: The last tap-target pass and the final gate.

Work Log:
- The last named controls carry a 44px target: the dashboard's header buttons, its alert links and its "Needs Attention" rows; the activity log's "View post" link and "Load more"; the posts list's search field and row buttons; the Getting Started checklist's rows; and every calendar view's select checkbox (the label carries the target, the box stays 16px).
- The advisory count is now 398, from 4056 at the start. What remains is the calendar's hour-grid chips (21px, and the grid IS the design), the tag pills (their width is their text), and a few inputs in narrow cells.
- One finding was NOT a defect: the "Update existing login" overlay in the 375px light-mode screenshot is the BROWSER's password-manager UI. The live DOM has no such element and no dialog role. Confirmed rather than assumed.

Stage Summary:
- Verification: 481 lib tests + 38 integration tests pass (519 total, 0 failed); `cargo clippy --lib --bins --all-features -- -D warnings` 0 errors; `svelte-check` 0 errors / 0 warnings; `pnpm build` clean; `scripts/genome doctor` healthy; the probe over 13 routes x 3 widths x 2 themes reports 0 FAIL, 0 console errors, 0 404s, and 398 advisories.
- Artifacts: 21 commits on master, all pushed to github/master (43d16c3).

---
Task ID: red-team-web-dashboard-2026-10-20-iteration-2
Agent: omp (same session, second red-team iteration)
Task: A second red-team over the code the first iteration changed, plus four new surfaces: the MCP tool surface, the scheduler/publisher path, the Svelte 5 rune usage, and the media pipeline.

Work Log:
- **The MCP surface was the lossy copy of the REST surface.** Every MCP tool that changes state re-implemented the write, and the re-implementation dropped the safety half: `posts_update` WIPED the post (the write was unconditional, so an agent that sent only a new body erased every image and every provider setting - the Reddit subreddit, the Pinterest board - and the post then failed at publish with "Missing subreddit in settings"); `campaign_update` could never CLEAR a field (a two-state input mapped with `.map(Some)` turned "clear this" into "leave this alone", and the tool reported success); `posts_create` skipped the provider validation, so an agent could post 600 characters to X, get "success", and the post sat in `queued` until it failed at publish time; `media_delete` deleted from the local directory only, so on an S3/R2 deployment the row went and the OBJECT STAYED IN THE BUCKET; `posts_set_tags` re-implemented the SQL without the ownership check or the transaction; the automation tools had NO ownership check at all; `posts_publish` fabricated a schedule and dropped the live permalink; and the media responses omitted `alt` entirely.
- **The media pipeline.** Eight of the twelve Tier-1 providers could not publish media AT ALL: the stored URL is relative (`/api/media/<uuid>`) and a platform cannot fetch that, and `resolve_media_url`'s default returned the attachment unchanged with each provider opting in - eight did not, so they published with a relative URL and failed with an opaque network error. The media library's search was 100% broken: the SQL's placeholder order was `user_id, pattern, limit, offset` and the binds were `user_id, limit, offset, pattern`, so the LIKE pattern received the integer LIMIT; past page 1 the OFFSET received the pattern string and SQLite raised a datatype mismatch, which the error handler turned into a 500. A library pick was classified as a VIDEO (the mime type was re-derived from a URL with no extension). The alt text was never persisted - the column existed, the upload hard-coded `None`, and there was no endpoint to write it, so it lived in the browser tab and was DESTROYED by the first save. A downward drag inserted at the wrong index. Threads' media limit was 10 in the server and 20 in the composer. The composer refused a media-only post. The composer checked images and videos separately while the server also caps the TOTAL.
- **The reactivity.** The "Posting Targets" panel NEVER appeared: `$state(new Map())` is not a reactive Map (Svelte 5's `proxy()` returns a Map unchanged), so `integrationTargets.set(...)` notified nobody and the template's `{#if integrationTargets.has(intId)}` was a derived with ZERO dependencies - evaluated once at mount, never recomputed. A Reddit subreddit, a Pinterest board or a Facebook Page could not be picked, and the post then failed at publish with "Missing subreddit in settings". The autosave's dependency list did not cover its payload, so attaching an image and tagging a post and then typing nothing meant the draft was NEVER written. A duplicate key in the suggested hashtags (the LLM repeats itself) CRASHED the composer - a keyed `{#each}` with a repeated key is a hard throw in Svelte.
- **The scheduler.** A post could be PUBLISHED TWICE: the claim's UPDATE had no `state = 'queued'` guard, and the transaction is a deferred `BEGIN` (the pool never configured BEGIN IMMEDIATE), so the SELECT ran with no lock held and two claims could read the same id list. The loser flipped the row the winner had already moved to 'publishing' and published it too. The fix is a compare-and-set with `RETURNING id`, so the claim returns exactly the rows IT flipped.
- **The failures left no trace.** A failed manual publish ("Post Now") fired the webhook but NEITHER broadcast nor persisted the event, so it was permanently absent from `events_log` - it never appeared in the activity log, never in the dashboard's "Recent Activity" widget, and never triggered the eight routes subscribed to `post_failed`. The three redundant standalone `send()` calls before `send_and_log` pushed every event to every SSE client TWICE.
- **The browser and the server disagreed on the post body.** The browser did not decode entities and did not drop `<script>`/`<style>` bodies, so `&mdash;` was literal text in the composer and an em-dash in the published post - and the CHARACTER COUNT is measured on the undecoded string.
- **Bluesky silently dropped images.** The `return Ok(...)` was INSIDE the upload loop, so only the FIRST image was ever embedded - with no error anywhere, while the server accepted four and the composer offered four.
- The post-text rules moved to `frontend/src/lib/composer/post-text.ts`, a dependency-free module, and the parity test now RUNS THE REAL TYPESCRIPT under node. The test used to compare two hand-written copies - which is a second copy of the rule, and a second copy is exactly how the drift survived. Removing the `hellip` case from the TypeScript now fails the test.
- Every fix that could be reproduced was reproduced: the double-publish claim, the entity drift, the MCP wipe, the alt-text 404, the Bluesky image drop, the media-only draft, the tap targets.

Stage Summary:
- Verification: 481 lib tests + the integration targets pass; `cargo clippy --all-targets --all-features --locked -- -D warnings` 0 errors; `svelte-check` 0 errors / 0 warnings; `pnpm build` clean; the probe over 13 routes x 3 widths x 2 themes reports 0 FAIL and 0 console errors.
- Artifacts: new tests/scheduler_claim_test.rs (3 tests, one of which FAILS without the claim guard - verified by removing it) and the node-driven parity test in tests/html_to_text_test.rs. New: frontend/src/lib/composer/post-text.ts.
- Known follow-ups: the remaining tap-target advisories are the calendar's hour-grid chips (21px, and the grid IS the design), the tag pills (their width is their text), and a few inputs in narrow cells.

---
Task ID: red-team-web-dashboard-2026-10-20-final
Agent: omp (same session, final gate)
Task: The last visual pass, the last advisories, and the ship gate.

Work Log:
- The probe's FONT check was wrong, and it had been reporting a failure on every page. A webfont arrives as many faces - one per weight AND per unicode subset - and the browser loads a face only when some text needs it, so a family that IS working still has faces sitting at `unloaded`. The check compared the family against the set of LOADED faces, so Inter (whose latin face was loaded and whose cyrillic face was not) read as "never loaded". Verified in a real browser FIRST: the latin face was `loaded`, `document.fonts.check('12px Inter')` was true, and the woff2 request returned 200. A family is loaded when AT LEAST ONE of its faces is.
- "No default signature" is an empty result, not a missing resource. It was a 404, and the composer calls that endpoint on every mount - so every user without a signature saw a 404 in their console on every page. It returns `null` with a 200 now.
- The last tap-target advisories were the buttons that were ONE line short of 44px: `px-4 py-2` around a 20px line gives 36px, `px-3 py-1.5` gives 34px. The fix is systematic rather than per-control: every button and every button-shaped link carries `min-h-11`. The inline TEXT links were left alone - "View all (8)", "Full analytics", a Telegram username - because a 44px box around a sentence inside a row pushes the row apart, and they are sentences, not controls. 142 -> 126 advisories.
- The 375px pass: every one of the 13 routes has ZERO horizontal overflow, and the calendar's week grid is a 640px time grid inside a 335px scroll container with `overflow-x: auto` - which is the right pattern for a time grid on a phone, and was confirmed rather than assumed.

Stage Summary:
- Verification: 481 lib tests + 62 integration tests pass (0 failed); `cargo clippy --all-targets --all-features --locked -- -D warnings` 0 errors; `svelte-check` 0 errors / 0 warnings; `pnpm build` clean; the probe over 13 routes x 3 widths x 2 themes reports 0 FAIL, 0 console errors, 0 404s, and 126 advisories (from 4056 at the start of the first iteration).
- Artifacts: 6 commits on master, all pushed to github/master (4996f07). The tree is clean.
- Known follow-ups: the 126 remaining advisories are inline text links (sentences, not controls) and the calendar's hour-grid chips (21px - the grid IS the design). The `live_*` test binaries hit the real provider APIs and fail intermittently on rate limits; that is environmental.

---
Task ID: red-team-iteration-3-5
Agent: omp
Task: Exhaustive red-teaming of the switchpost web dashboard in recursive
iterations. Iteration 3 started from a static probe that covered 13 of the
app's 29 routes against an EMPTY seeded database; iterations 4 and 5 went
after the interaction the probe cannot see.

Work Log:
- Iteration 3: found the probe covered 13 of 29 routes and the seeded
  instance was empty, so every page rendered an empty state. Extended the
  probe to the whole route table, wrote scripts/seed_demo.py (a realistic
  solo-founder dataset with a self-check that every uuid column is
  BLOB(16)), and re-probed.
- The uuid id column was decoded as a String at 23 query sites. sqlx
  encodes a Uuid as BLOB(16); the `.sqlx/` cache records the DECLARED
  type (TEXT), so it compiled and failed only at RUNTIME - a 500 on
  POST/GET /api/automation/rules, and the CLI's `tags list` and
  `webhooks list`.
- The probe's `effectiveBg` SKIPPED the translucent layers it walked
  past, so every reading on a tinted chip was wrong - and the app is
  full of tinted chips. Fixing the compositing exposed that a semantic
  hue as TEXT on a wash of itself fails AA in the light theme for EVERY
  hue.
- `providerInk` computed the glyph's ink against a HARDCODED surface,
  but the app paints a provider glyph on three different backgrounds.
- Iteration 4: the static probe was clean, so this pass audited the
  INTERACTION. The composer wrote `settings["channel_id"]` and
  `settings["page_id"]` - keys NO provider reads - while Discord reads
  `settings["channel"]`. Every Discord post was created, queued, and
  then failed at publish. Fixed on both sides: the composer writes the
  key the provider reads, and the create path checks the provider's
  `required_settings()` and refuses the post with a message naming the
  missing field.
- The calendar's RANGE filter was UTC-bounded while its GRID was
  app-zone-bounded, so posts in the gap fell off the calendar entirely -
  on NO month grid, in either month. The range is now sent as UTC
  instants computed with the app's own `wallClockToUtc`.
- Iteration 5: editing a post WIPED its provider settings (the detail
  response had no `settings` field, the composer sent `{}`, and the
  update path REPLACED wholesale) - a post that was working thirty
  seconds earlier failed at publish.
- The feed half of every analytics headline was ALWAYS ZERO: the cutoff
  was bound as a raw `DateTime<Utc>` (RFC 3339 TEXT) against an INTEGER
  epoch column, and SQLite orders every INTEGER before every TEXT.
- No post could be filed under a campaign: the composer's dropdown set
  a variable that was NEVER read, and the create path had no
  `campaign_id` at all.
- The per-post analytics rows - the ones the dashboard READS - were
  written only when a human opened the Post Stats modal, and the
  cleaner deleted them an hour later. The founder's "Likes 42" became
  "Likes 0" with no publish, no error and no refresh.
- A card's substate could never be CLEARED (a COALESCE ratchet), the
  kanban lost the error posts' order (a per-state UPDATE on a mixed
  column), the research table was labelled UTC while the app is in the
  founder's zone, and the Instagram music the composer collected was
  never sent.

Stage Summary:
- key results:
  - 7 commits, each with a regression test that FAILS on the old code
  - probe: 0 contrast / heading / tiny-text / tap-target FAILs, 0
    advisories, over 27 routes x 3 widths x 2 themes. The 4 remaining
    FAILs are the /api/dms 502 from X rejecting the seeded fake token -
    an environment artifact, and the page renders its error state.
  - 481 lib tests, 27 integration test binaries (192 tests) pass
  - cargo clippy --all-targets --all-features --locked -- -D warnings: 0
  - SQLX_OFFLINE=true cargo clippy --all-targets --locked -- -D warnings: 0
  - svelte-check: 0 errors, 0 warnings; pnpm build clean
  - scripts/check_status_contrast.py: 88/88 token pairs clear AA
- artifacts produced:
  - scripts/seed_demo.py (the seeded dataset + its uuid-type self-check)
  - scripts/check_status_contrast.py (the status-token gate)
  - tests/uuid_column_decode_test.rs (3)
  - tests/posts_media_shape_test.rs (3)
  - tests/post_settings_preserved_test.rs (5)
  - tests/required_settings_test.rs (5)
  - tests/feed_analytics_window_test.rs (2)
  - tests/campaign_assignment_test.rs (4)
  - tests/calendar_range_timezone_test.rs (3)
  - tests/kanban_substate_test.rs (6)
  - migrations/0008_posts_media_array.sql
- known issues / follow-ups:
  - the /api/dms 502 needs a REAL X token to clear; the seeded instance
    uses a fake one on purpose
  - "reach", "followers" and "custom" campaign metrics have no data
    source anywhere in the codebase; the UI now says "Not measured"
    rather than showing a 0% bar
  - the research page has no realtime subscription and no refresh
    button outside its error state

---

Task ID: v12-red-team-iterations-6-7
Agent: omp
Task: Continue the exhaustive red-teaming of the switchpost web
dashboard. Iterations 3-5 fixed what the static probe and a read-only
flow audit found. Iterations 6-7 went after what neither could see:
the pages in their non-default states, the empty instance, the
alternate widths, and the tool's own measurement errors.

Work Log:
- Read the flow auditor's report in full and triaged it into P0/P1/P2.
  The P0s: editing a post WIPED the provider settings (the composer
  sent `{}` and the server REPLACED the object, so every edit of a
  scheduled Reddit post dropped its subreddit); the feed half of every
  analytics headline was always zero (a TEXT timestamp bound against an
  INTEGER epoch column); and NO UI path could put a post into a
  campaign (the composer's dropdown set a variable that was never read,
  and the create request had no `campaign_id` field at all).
- Fixed each at the layer that owns the rule, with a regression test
  that FAILS on the old code: the update MERGES settings, the epoch
  cutoff goes through the `EpochUtc` seam, and the campaign is
  writable on create AND on update (three-state, so an edit can set,
  keep or clear it).
- Found the composer was writing `settings.channel_id` - a key NO
  provider reads - while Discord reads `settings.channel`. A Discord
  post could therefore NEVER publish. Added a `required_settings()`
  trait method and a create-time check, so the founder is told at
  compose time instead of finding out from the scheduler hours later.
- Extended the probe from 13 routes to all 29, and gave it a
  `--state` option so a page can be measured in a named state. The
  calendar's month and day views had NEVER been rendered: the month
  chips were 15px of a 144px cell, the day numbers were at 2.20:1,
  and the day view's ghost-slot button was 21px tall.
- Found the probe's own measurement errors, which had been hiding
  defects: it SKIPPED translucent backgrounds instead of compositing
  them (so every tinted-chip failure was invisible), it read the
  `color` without the inherited `opacity` (so a chip faded to 1.31:1
  passed), it measured contrast before the theme class was applied
  (69 failures in one run, none in the next), and it treated every
  tap target under 44x44 as a FAIL when WCAG 2.2's floor is 24x24.
- Ran a SECOND instance against an empty database. The composer's
  empty-state links were 17px, and the two dynamic routes were
  reported as FAIL when there was simply no row to open - so a fresh
  install, the first thing a founder sees, could never pass the probe.
- Repaired 245 of the 389 MCP tool descriptions, which had been cut
  off mid-sentence ("Get the authenticated user's profile." became
  "Get the authenticated").
- Looked at every route's rendered output in both themes at 1440, 768
  and 375, and at the 29 route screenshots by eye.

Stage Summary:
- key results:
  - 16 commits, each with a regression test that FAILS on the old code
  - probe over 29 routes x 3 widths x 2 themes, seeded instance:
    0 FAIL except the /api/dms 502, 0 warnings
  - probe over the same routes, EMPTY instance: 0 FAIL, 2 SKIPPED
    (correctly explained), 12 warnings (24px inline links, above the
    WCAG floor)
  - probe over the calendar's month and day views: 0 FAIL each
  - 481 lib tests, 27 integration test binaries pass
  - cargo clippy --all-targets --all-features --locked -- -D warnings: 0
  - SQLX_OFFLINE=true cargo clippy --all-targets --locked -- -D warnings: 0
  - svelte-check: 0 errors, 0 warnings; pnpm build clean
- artifacts produced:
  - tests/campaign_edit_test.rs (3)
  - tests/required_settings_test.rs (5)
  - tests/feed_analytics_window_test.rs (2)
  - tests/post_settings_preserved_test.rs (5)
  - tests/uuid_column_decode_test.rs (3)
  - tests/posts_media_shape_test.rs (3)
  - tests/kanban_substate_test.rs (6)
  - tests/calendar_range_timezone_test.rs (3)
  - scripts/seed_demo.py (the seeded dataset + its uuid-type self-check)
  - scripts/check_status_contrast.py (the status-token gate)
  - AGENTS.md 9.33-9.38 (six new pitfalls)
- known issues / follow-ups:
  - the /api/dms 502 needs a REAL X token to clear; the seeded instance
    uses a fake one on purpose
  - "reach", "followers" and "custom" campaign metrics have no data
    source anywhere in the codebase; the UI says "Not measured" rather
    than showing a 0% bar
  - the research page has no realtime subscription and no refresh
    button outside its error state
  - the composer previews deliberately simulate the PLATFORM's chrome
    (a LinkedIn post is white-on-light), so their greys do not follow
    the app's tokens. That is a feature; the greys were fixed to clear
    AA against the surfaces they actually sit on.

---
Task ID: doc-foundation-pass-2026-10-20
Agent: jcode
Task: Complete the project-genome documentation management so the doc foundation
is correct before an executor agent starts work.

Work Log:
- Surveyed the genome state first: `scripts/genome` is vendored and current
  (2026.10.20-1), `genome.toml` maps every role (nine `native:` forms, four
  file paths), and `lint` was already clean. The defect was not the tooling —
  it was the content the tooling routes to.
- Verified every number in AGENTS.md against the tree before editing:
  `#[tool(` count = 389 (all in `src/mcp/mod.rs`; the 46 sibling `tools_*.rs`
  files carry zero tool attributes), `ls migrations/*.sql` = 8 (doc said 2),
  `ls src/social/*.rs` = 39 provider files, real LOC ~94.2k Rust / ~29.2k frontend
  against the documented ~85.7k / ~25.7k.
- Repaired the AGENTS.md §0.6 document map.
  CORRECTION, written in the same session after re-reading the request against
  the evidence: the claim in the first commit that "four rows cited
  docs/archive/ paths that no longer exist" was FALSE, and the annotation I had
  added to those rows has been removed. My existence check covered only a
  curated path list and `.md` URLs, so it never actually tested the
  `docs/archive/` rows; checked directly, all three exist (GAPS_15 170 lines,
  OUTLINE-17 100 lines, OUTLINE-16 70 lines). I asserted an unverified claim in
  two commit messages. The mistake was mine, not the doc's.
  What was genuinely wrong in this map: one dead path (`SESSION_HOOKS.md` named
  at the repo root, existing only at `docs/SESSION_HOOKS.md`, now repointed), and
  seven live docs with no row at all — PAGE-INDEX, platforms-and-cli,
  deployment, agent-integration, TARGETING_AND_WRITER_DISCIPLINE,
  DECISIONS-19 and EXECUTION-19 — so `ask` reached them only by luck of
  ranking. They are routed now.
- Replaced the stale key-numbers block with measured values, each line naming
  the command that re-measures it, so the next agent re-measures rather than
  trusting the prose. Also fixed the two other stale lines (§2 layout tree:
  2 → 8 migrations, 3492 → 4114 LOC for mod.rs) and the dead `44 files` count.
  VALIDATION (second pass): the word "dead" was wrong there — 44 is a real
  number, the count of `src/mcp/tools_*.rs` handler modules, and the README's
  "389 tools across 44 modules" is self-consistent. The actual defects in my
  own replacement text were "27 providers" (the truth is 26: 12 + 14, with
  `linkedin-page` and `instagram-standalone` INSIDE the 12, so my "plus 2"
  double-counted) and "46 siblings" (there are 44). Both corrected; LOC and the
  migration count were right as written.
- Fixed the lint error the first pass had shipped: E001 on AGENTS.md:193. My
  re-measure command was written as a backticked token containing an unbalanced
  bracket, which broke the linter's own tokenizer and produced a phantom `.rs`
  pointer that failed to resolve. Bisected it by deleting segments one at a
  time, then reworded the line so every backticked token is a real path.
  The command was also wrong as first written: the bracket is a regex
  metacharacter, so the unescaped form returns 0 rather than 389. The version
  now documented is `grep -rh --include='*.rs' 'tool(' src/mcp/ | wc -l`, which
  returns 389 and lints clean.
- Removed two dead pointers: a machine-local skill path
  (/home/ishanp/.agents/...) and /home/z/my-project/scripts/cargo-env.sh, which
  named a different machine entirely.
- Added a table of contents to `worklog.md`: the entry count pushed it past
  `core.toc_lines` (300), which made `genome lint` emit its first-ever W010
  warning on this file. Lint is clean again.
- Re-ran the doc-drift tests after the edits, because AGENTS.md is asserted
  against the source by `tests/surface_docs_audit.rs`. That parser counts
  `#[tool(` attributes across `src/mcp/`; it parses exactly 389, which is the
  string the edited AGENTS.md carries, so `d1_documented_tool_counts_match_registry`
  still passes. `cargo test --lib`: 481 passed. `cargo check --lib --bin
  switchpost`: 0 errors.
- Checked the odd `git` behaviour mid-session: HEAD advanced from 006aed5 to
  73d0394 and my diff disappeared. A parallel agent had committed and pushed the
  preview/tag colour work in the same tree. Nothing of mine was lost; the re-run
  of every verification command after that commit is what the numbers above are
  measured against.

Stage Summary:
- Artifacts: AGENTS.md (§0.6 doc map, key numbers, §2 layout tree, two dead
  pointers), worklog.md (table of contents + this entry).
- Verification: `genome lint` 0 errors 0 warnings; `genome doctor` healthy;
  `cargo check --lib --bin switchpost` 0 errors; `cargo test --lib` 481 passed;
  `cargo test --test surface_docs_audit` 3 passed.
- Known issues / follow-ups:
  - `docs/decisions/` holds zero AD records and `docs/archive/records/` does not
    exist. That is deliberate per its README (ADs start from the first ratified
    direction change), so no record was minted here — minting an AD needs an
    operator ratification, and inventing one would violate law 6.
  - `probe-out/` and `scripts/__pycache__/genomecpython-314.pyc` are tracked
    (~4.8 MB, 157 files) although `.gitignore` lists probe-out. Turning on the
    `ops` module would raise E305; that needs an operator decision, because
    `docs/planning/EXECUTION_2026-10-19.md` cites `probe-out/run12` and
    `run13-dark` as evidence and those directories no longer exist on disk.

---
Task ID: doc-foundation-pass-2026-10-20-followup
Agent: jcode
Task: Re-read the request against the gathered evidence; correct what the first
pass overstated, then continue.

Work Log:
- Re-verified the claim I had made about dead `docs/archive/` paths. It was
  false: the existence check I wrote only covered a curated path list and `.md`
  URLs, so it never tested those rows. All three exist (GAPS_15 170 lines,
  OUTLINE-17 100 lines, OUTLINE-16 70 lines). Removed the incorrect
  annotations from AGENTS.md and wrote the correction into this entry rather
  than quietly editing history — the commit messages are immutable on the
  remote, so the correction has to be visible somewhere, and here is where a
  reader will look.
- Acted on the follow-up the first pass logged but left open. PAGE-INDEX was
  labelled the first file to read by that same pass, which made its accuracy
  load-bearing: six of its rows said LIVE for files whose own bodies say they
  are superseded (GAPS-15, OUTLINE-16, OUTLINE-17, OUTLINE-19, OUTLINE-19-v3,
  AUDIT-14). All six now carry their real status, each naming its successor
  where one exists.
- Added an intro to PAGE-INDEX explaining that the status column is what the
  `Owns` column is conditional on. A column that only meant something after my
  edit needed to say so, or the next reader would treat LIVE as decoration the
  way I found it.
- Checked reachability for every tracked doc against PAGE-INDEX, the README,
  AGENTS.md and SKILL.md: 29 docs, zero orphans. The two I first flagged as
  orphans (platform-matrices.md, recipes-extended.md) are cited from SKILL.md's
  router table — my check had simply omitted the skills directory.
- Verified the tombstone copies under `docs/planning/` are the intended
  convention and not duplicate live plans: five files carry the tombstone
  comment, the planning README marks each archived, and `genome lint` reports
  no W006 overlap.

Stage Summary:
- Artifacts: AGENTS.md (annotations removed), docs/PAGE-INDEX.md (six status
  corrections + intro), worklog.md (correction + this entry).
- Verification: `genome lint` 0 errors 0 warnings; `genome close` green;
  `cargo test --lib` passed; `cargo test --test surface_docs_audit` 3 passed.
- Known issues / follow-ups:
  - PAGE-INDEX is still not in `genome.toml`'s ownership map. That is an `ops`
    module role and turning `ops` on has its own consequence (E305 on tracked
    bulk), so it stays an operator decision rather than a unilateral switch.
  - The false claim remains in the messages of commits 5e3db17 and 66b4823 on
    the remote. The correction is here and in AGENTS.md; the root commit is not
    rewritten, and should not be.

---
Task ID: doc-foundation-validation-2026-10-20-pass-2
Agent: jcode
Task: Deep validation of the documentation foundation work. Find what the first
pass got wrong rather than confirming it.

Work Log:
- Re-measured every figure I had published as "measured", and two of them were
  wrong. The provider count was 27; the truth is 26 (12 Tier-1 + 14 Tier-2, with
  `linkedin-page` and `instagram-standalone` INSIDE the 12, so my "+2" added what
  was already counted). `src/social/tier.rs` and `registry.rs` both assert 26, so
  the code was right and my arithmetic was not. The sibling file count was 46;
  the truth is 44 `tools_*.rs` modules plus `auth.rs` and `rest_bridge.rs`, in a
  tree of 47 Rust files.
- Corrected my own earlier claim rather than leaving it standing: the "dead 44
  files" wording I used against the ORIGINAL doc was wrong. 44 is a real number
  (the handler modules) and README's "389 tools across 44 modules" was
  self-consistent. I had replaced a correct number with an incorrect
  "clarification" of it.
- Found and fixed a lint error the first pass had shipped: E001 on AGENTS.md:193.
  My backticked re-measure command contained an unbalanced bracket, which broke
  the linter's tokenizer into emitting a phantom `.rs` pointer. Bisected by
  removing one backticked segment at a time until the error cleared, then
  reworded so every backticked token is a real path. The command itself was
  independently wrong: `[` is a regex metacharacter, so the unescaped form
  returns 0 rather than 389. Now documented as
  `grep -rh --include='*.rs' 'tool(' src/mcp/ | wc -l`, which returns 389.
- Valued every PAGE-INDEX status label against body evidence and the archive
  INDEX's recorded reasons: 18 rows, all consistent. Confirmed the tombstone
  copies under `docs/planning/` are convention, not duplicate live plans, and
  that lint reports no W006 overlap.
- Confirmed reachability for all 29 tracked docs (zero orphans) and re-confirmed
  the push protocol (no `origin` remote resolves; `github/master` does; no
  `git push origin` remains anywhere outside `docs/archive/`).
- Acted on the genome upgrade that `doctor` had been reporting as informational.
  Ran the skill's own independent self-check against the vendored copy FIRST:
  32/33, failing `p5_class_intent`. The newer CLI passes that case and scores
  33/33. Applied with `genome upgrade --apply`, backup left under the existing
  `.genome/` ignore. Post-upgrade lint clean and doctor healthy, no new
  findings, so no rule in this repo depended on the old behaviour.

Stage Summary:
- Commits: 64c55eb (the three numeric defects + E001), d393b4b (CLI upgrade).
- Verification: `test_genome.py` 32/33 before, 33/33 after; `genome lint` 0
  errors 0 warnings; `genome doctor` healthy; `cargo test --lib` passed;
  `cargo test --test surface_docs_audit` 3 passed; dead-path grep clean.
- Known issues / follow-ups:
  - Two closed items from earlier entries are now resolved (the PAGE-INDEX
    labelling, the CLI revision). Two remain, unchanged and deliberate: the
    `ops` module decision, and the fact that commits 5e3db17 and 66b4823 still
    carry claims this pass falsified. The corrections live here and in
    AGENTS.md; the history is not rewritten.
  - The CLI now being 2026.10.20-2 means the next `genome upgrade` check will
    drift again as the skill moves. Treat `genome doctor`'s upgrade line as a
    signal to re-run the self-check, not as noise.

---
Task ID: doc-foundation-validation-2026-10-20-pass-3
Agent: jcode
Task: Third validation pass: the §2 layout tree, which two earlier passes had
never measured.

Work Log:
- Re-measured every count in the §2 repository-layout tree, not just the
  key-numbers block the first two passes had fixed. It was wrong in five places:
  "31 provider implementation files" (the truth is 26, with the other 13 files
  under src/social/ being support modules — whatsapp_cloud.rs being the subtle
  one, an internal helper rather than a registered provider), "25 page routes"
  (18 top-level + 11 nested = 29), and four stale LOC counts (run.rs, x.rs,
  reddit.rs, queries.rs — all understated by 300-1000 lines).
- The scripts/ entry listed three of sixteen files and omitted the two the rest
  of the repo depends on: `genome` itself, and `vendor-pull.sh`, the CI gate
  VENDOR_PINS documents as enforced. An agent reading the layout tree would not
  have known the tooling existed.
- Confirmed two claims were accurate so they did not get "fixed" into wrongness:
  the 8 migrations contain no destructive DDL (only ON DELETE CASCADE FK
  clauses), and the 19 SSE events §9.8 names match realtime.ts one for one.
- Re-introduced the E001 error for the second time in this session by writing
  "the other 13 `.rs` here" — a bare extension token, the exact shape that
  already failed once. Caught it on the lint run rather than at push. Recorded
  because the first occurrence was attributed to an unfamiliar tokenizer quirk,
  which is the wrong diagnosis: it is a habit of writing prose with a bare
  extension in backticks, and habits do not correct from memory.
- Held AGENTS.md back from commit while a concurrent agent's §9.39–§9.43 records
  sat uncommitted in the same file, then committed the file once their records
  were live rather than reconstructing a split version that would have
  discarded their work. The commit message names them as the co-author of the
  content.

Stage Summary:
- Artifacts: AGENTS.md §2 layout tree (seven corrections).
- Verification: `genome lint` 0 errors 0 warnings; `genome close` green; gate ok;
  `cargo test --lib` passed; `cargo test --test surface_docs_audit` 3 passed;
  dead-path grep clean.
- Known issues / follow-ups:
  - The layout tree's LOC counts will drift again as the files grow. They are
    now four years of evidence stale; nothing in the repo asserts them. If the
    next pass wants a guard, `surface_docs_audit` is where a count assertion
    would go, but only for the counts the code can derive (tools, migrations,
    tier membership) — a file's line count changes with every commit.
  - Nothing else in §2 remains unmeasured: every number in the tree has now been
    checked at least once against the tree it describes.

---
Task ID: doc-foundation-validation-2026-10-20-pass-4
Agent: jcode
Task: Sweep the remaining routing surfaces for unverified claims. This pass
found the defect the whole exercise was for.

Work Log:
- Verified claims until then unexamined and found four more stale numbers in
  the §2 layout tree: "31 provider implementation files" (26 — the other 13
  files under src/social/ are support modules, whatsapp_cloud.rs being the
  subtle one: an internal helper reached through the WhatsApp provider, not a
  registered provider), "25 page routes" (18 top-level + 11 nested = 29), and
  four understated LOC counts. Also corrected the outline's own claim set.
- Confirmed accurate so as not to fix them into wrongness: the 8 migrations
  contain no destructive DDL (only ON DELETE CASCADE FK clauses), and the 19
  SSE events §9.8 names match `frontend/src/lib/stores/realtime.ts` one for one.
- THE FINDING. AGENTS.md §0.6 routes every agent to
  `docs/planning/OUTLINE-2026-10-19-v2.md` "before starting ANY implementation
  work", describing it as "the live working queue". It is dated-frozen, so its
  lineage is correct — but one day later five of its six `STILL OPEN` items had
  already landed. Measured, each with the command its own row names:
  SKILL.md 255 lines not 580; quick-reference 108 not 614; evals 43 with a
  toolset-restricted #42, not 40 with none; the "progressive disclosure" and
  "eval loop" gaps both closed by `evals/c3-paired-run.json`. Only C2 (no
  reference-router table) is genuinely open.
  Its stated top priority is false too: the outline says `surface_docs_audit`
  has "2 failing tests" and is "the gate-breaker" that "outranks all Wave B/C
  work". It is 3 passed, 0 failed.
- Did NOT edit the outline. It is dated-frozen and its lineage is the evidence
  that makes the audit checkable; rewriting it would destroy the thing that
  makes the re-audit falsifiable. Wrote a dated-frozen re-audit instead
  (`REAUDIT_OUTLINE-2026-10-19-v2_2026-10-20.md`) carrying a verdict table and
  the measurement command per claim, and corrected the three routing surfaces
  that reach the outline (AGENTS.md §0.6, PAGE-INDEX, planning README) to carry
  the verdict inline and point live-status questions at TODO.md.
- Confirmed `genome ask` now returns the re-audit as the top hit for the
  question, which is the mechanism the agent will actually use.
- Re-introduced the E001 bare-extension-token error a second time ("the other 13
  `.rs` here") minutes after diagnosing it once. Recorded because the first
  diagnosis — an unfamiliar tokenizer quirk — is wrong and produces the wrong
  fix. It is a writing habit, and it recurs until the file is scanned.

Stage Summary:
- Commit dc41cf1: the re-audit, plus three routing corrections. The seven
  layout-tree corrections from this pass are in 0b9d357.
- Verification: `genome lint` 0/0; `genome close` green, gate ok;
  `cargo test --test surface_docs_audit` 3 passed; `cargo test --lib` passed;
  every link in the four changed files resolves.
- Known issues / follow-ups:
  - One dead link, pre-existing and not mine: PAGE-INDEX's VENDOR_PINS cell
    quotes a body whose underscores were stripped, so
    `[PLANPARITY...](PLANPARITY...)` does not resolve. It is inside quoted
    content, and link-checking belongs to the ops module, which is off.
  - Only C2 (the reference router) survives the outline's B/C waves. It is a
    real gap and now the only one of the six that an agent should work.


---
Task ID: doc-foundation-validation-2026-10-20-pass-5
Agent: jcode
Task: Adversarial re-check of pass 4's headline claims. A self-audit that only
confirms itself is worthless, so every claim was re-derived rather than re-read.

Work Log:
- FALSIFIED MY OWN VERDICT. C2 was recorded "the one real survivor" because I
  grepped SKILL.md for a heading containing "router" and found none. The table
  exists; it is titled `## Where to look next` and is keyed by need. All four
  references/*.md state a purpose and say "loaded on demand from SKILL.md". So all
  six of the outline's open items are landed, not five of six.
  The mechanism of the error is the interesting part and is now written into the
  re-audit: grepping for the jargon in a CLAIM rather than reading the section the
  claim describes. That is the same error the outline itself made with its own
  evidence. The reversal is left visible in the verdict table instead of being
  edited away.
- FALSIFIED A COUNT'S DERIVATION, though not its value. "26 provider files + 13
  support" was wrong: 29 files `impl SocialProvider` (mod.rs's two impls are test
  fixtures), of which 26 are registered and calendar/drive/gmail are capability
  modules that appear in no tier table. 29 + 9 support + mod.rs = 39. The 26
  survived for the wrong reason, which is worse than a wrong number because it
  survives the next edit too.
- Confirmed under challenge and unchanged: 44 tools_*.rs + auth.rs +
  rest_bridge.rs + mod.rs = 47 (all 389 tools in the one mod.rs impl); 26
  registered providers (12 + 14, no double-count this time); 19 SSE events
  matching realtime.ts; migrations contain no destructive DDL.
- Eval #42 read in full rather than grepped: `mcp-tools --toolset x` with an
  expected_output that names the tool budget. And c3-paired-run.json read in
  full: a real paired with-skill/baseline run with per-eval grades, three evals
  in scope, 40 not yet run. The B4 and C3 verdicts rest on artefacts, not filenames.
- Fixed my own broken verification script mid-pass: the dead-path check emitted
  ~55 false positives by concatenating `file:path` into single tokens before
  testing them. Re-ran it per file; genuinely zero dead paths. Noted because a
  check that cries wolf is the next agent's reason to ignore it.

Stage Summary:
- Commit eb5d444: the re-audit's C2 correction + method note, the layout tree's
  provider-file breakdown, and the three routing surfaces.
- Verification: `genome lint` 0/0; `genome close` green; gate ok; `cargo test
  --lib` passed; `cargo test --test surface_docs_audit` 3 passed; dead-path
  check re-run per file, 0 findings.
- Standing conclusion: the outline's B/C waves contain NO remaining work. Only
  the E wave (credential-blocked live passes) is open, and it is blocked on
  credentials exactly as TODO.md's V1a-V1d rows say.

---
Task ID: doc-foundation-validation-2026-10-20-pass-6
Agent: jcode
Task: Sweep the surfaces the earlier passes never checked. `providers.md` is
asserted by the doc test, so a stale per-platform count could hide behind a
passing test.

Work Log:
- Parsed all 389 `#[tool(` declarations from `src/mcp/mod.rs` and grouped them by
  the real name prefix. That map is the authority for every count below; nothing
  was taken from any doc.
- Found `providers.md` wrong for 8 of its 14 platforms: X 20→27, Instagram 15+→21,
  LinkedIn Page 12→11, LinkedIn Personal 12→15, Reddit 36→37, Bluesky 5→7,
  Discord 13→15, Telegram Bot 9→11. The doc test checks that this file states the
  TOTAL tool count, which it did, so it passed throughout.
- Six bodies listed only a subset, so heading and body disagreed with each other as
  well as with the registry. Appended the missing tools as an explicit
  "Not listed above (N more, M shown)" line per section instead of merging them
  into the original prose, so the curated descriptions survive and the gap stays
  visible.
- WhatsApp was the one case that needed judgement rather than arithmetic: 13
  tools across two prefixes, 9 `wa_` Web-session and 4 `wc_` Cloud-API. The
  heading now says so; a single number there implied one bucket that does not
  exist.
- Verified, not assumed: the 14 platform sections cover 235 of the 389 and the
  other 154 are the cross-cutting tools the file already documents in its own
  sections. That is why no total was rewritten and no false "everything is wrong"
  claim was made.
- Also confirmed README's claims hold (26 platforms, 389 tools, 44 modules all
  measured) and corrected my own verification script twice — once for
  concatenating file:path into dead-path tokens, once for a heading regex too
  strict to parse the sentence I had written.

Stage Summary:
- Commit 830ab31: providers.md, eight heading corrections plus six body
  completions, with the section-intro shape preserved.
- Verification: a three-way reconciliation script (heading vs body vs registry)
  passes for all 14 sections; `cargo test --test surface_docs_audit` 3 passed;
  `genome lint` 0/0.
- Known issues / follow-ups:
  - The per-platform counts will drift again as providers gain tools, exactly as
    they drifted here while the total-count assertion kept passing. The guard
    that would catch it belongs in `surface_docs_audit`: parse
    `providers.md`'s per-section counts and compare against the real prefix
    groups. That is a test change, not a doc change, and it was out of scope for
    a documentation pass — noted so it is not lost.

---
Task ID: doc-foundation-validation-2026-10-20-pass-7
Agent: jcode
Task: Guard the per-platform drift I found in pass 6. A count corrected once will
drift again, so the fix is a test, not a doc edit.

Work Log:
- Added D3 to `tests/surface_docs_audit.rs`, whose own header comment names the
  drift classes it guards (D1 the total, D2 schema shape). D3 parses
  `### Platform (N tools)` headings, counts the tools the section lists, and
  compares all three: heading, body, and the registry parsed from the same
  `#[tool(` scan D1 uses.
- FIRST DRAFT WAS WEAKER THAN I SAID. It required only that *a* continuation line
  exist somewhere in the section. Injecting a real drop (removing `bs_reply` from
  Bluesky) left it passing, because the stale "Not listed above (2 more)" line
  satisfied the check. Caught it by injecting drift and reading the failure
  instead of assuming the guard worked.
- Replaced it with a check that reconciles the continuation's own declared tail
  count against the tools the prose lists, so a drop and a mis-stated count are
  different failures with different messages. Verified by injection:
    - heading drift (Bluesky 7 -> 5): FAILS, "claims 5 tools but the registry has 7"
    - inconsistent continuation (says 2 more, names 1): FAILS, "lists 6 tools and
      its continuation claims 2 more, totalling 8 - the registry has 7"
    - clean file: 4/4 pass
- Left the "Not listed above" lines I added in pass 6 rather than expanding the
  prose, because the curated descriptions are the section's value and a test now
  keeps the omissions honest.
- Removed a debug `eprintln!` and a dead `head`-prefix block from my own draft
  before it could land. Both were caught by reading the diff.

Stage Summary:
- D3 landed in `4e5b879` (swept by the concurrent agent with the strengthened
  guard and no debug output, verified against `github/master`).
- Verification: 4/4 pass clean; two injections proved it fails with an actionable
  message; `genome lint` 0/0; `genome close` green; `cargo test --lib` passed.
- Known issues / follow-ups:
  - D3 asserts the WhatsApp section covers both `wa_` and `wc_`, and each other
    section one prefix. A provider added under a new prefix needs a row in the
    `sections` table, so the test fails loudly rather than silently ignoring it.
  - The `Other platforms` table at the bottom of providers.md is not covered by D3;
    it lists cross-platform tools. Extending D3 to it means choosing whether the
    table counts as prose, which is a judgement call left to whoever needs it.

---
Task ID: doc-foundation-validation-2026-10-20-pass-8
Agent: jcode
Task: Verify the B4 verdict against the CODE rather than the artefact, and sweep
every command the evals expect.

Work Log:
- Verified eval #42's commands against the source rather than trusting the
  artefact, and found a real defect: it expects
  `mcp-call x_home_timeline --args '{"count":5}'`, but `XHomeTimelineInput`
  (src/mcp/tools_x.rs) declares `max_results` and `pagination_token`. There is no
  `count` field, so a correct agent answer FAILS this eval. The CLI fallback in
  the same eval is right (`x timeline --count 5`, src/cli/mod.rs), so the error is
  confined to the MCP path. Corrected the arg and recorded both facts in
  expected_output.
- The 2026-10-19 paired run graded this eval `partial` and named the
  `--limit`/`--count` confusion, but attributed it to the agent's guess. Both
  halves are now right: the arg is `max_results`, the CLI flag is `--count`. Worth
  noting that a graded artefact can still carry a wrong expectation.
- THE NEAR MISS, which is the reason this entry exists. My sweep reported four
  commands as nonexistent: `gdrive`, `gmail-ops`, `fb`, `--toolset`. All four were
  FALSE POSITIVES produced by my own regex. clap derives kebab-case names from
  Rust enum variants, so grepping `src/cli/*.rs` for a literal `gdrive` finds
  nothing; and `fb` was matched out of `mcp-call fb_create_post`, a tool name and
  not a subcommand. Checked against the built binary: `gdrive Google Drive
  operations`, `gmail-ops Gmail operations` and `--toolset <TOOLSET>` all exist.
  I was one commit away from "fixing" three working commands on the strength of a
  bad grep — the exact failure mode this entire doc pass has been auditing. A
  verification that cannot fail is worse than none, because it looks like evidence.
- Final sweep, done correctly: all 43 evals' `mcp-call` targets are registered
  tools, and every `switchpost <subcommand>` resolves against the binary's
  `--help`. Only the one defect above was real.

Stage Summary:
- Eval #42 corrected; landed in `80beff9` (swept by the concurrent agent) and
  verified on `github/master` with 43 evals intact and no stale `count` arg.
- Verification: evals.json parses; every eval command resolves against src/cli and
  src/mcp; `genome lint` 0/0; `cargo test --test surface_docs_audit` 4 passed.
- Known issues / follow-ups:
  - No test guards an eval's expected command against the CLI/tool registry, the
    same gap D3 closed for `providers.md` counts. Adding one is straightforward
    (parse `expected_commands`, assert each `mcp-call` target is in the registry
    and each subcommand is in the clap tree) but it needs the clap `Command`
    introspection to be reachable from a test — `d1_documented_cli_command_count`
    already does this, so the pattern exists. Left as a note rather than started
    late in the session.

---

Task ID: iteration-6..12-red-team
Agent: omp
Task: Exhaustive red-teaming of the switchpost web dashboard in recursive
      iterations, with visual testing via the browseros-neo MCP and the
      website-design skill's probe. Fix every defect found.

Work Log:
- Built a POPULATED instance to test against. The seeded DB was empty, so every
  page rendered its empty state and every populated-state defect was hidden.
  `scripts/seed_demo.py` seeds 6 integrations, 17 posts across all six states,
  3 campaigns, 6 tags, analytics cache, external posts + engagement, the event
  log, automation rules, webhooks, signatures, RSS feeds, notifications, post
  sets, media and comments. It asserts every uuid column it writes is BLOB(16)
  (sqlx's `Uuid` decoder reads exactly 16 raw bytes and does NOT accept the
  36-char TEXT form) - a TEXT uuid in a uuid column took the whole endpoint
  down with "invalid length: expected 16 bytes, found 36".
- Extended the probe from 13 routes to ALL 27. Sixteen pages had NEVER been
  rendered by any gate. Everything the probe found after that was in pages
  nobody had looked at.
- The probe could not reach a page's ALTERNATE state. It gained `--state` and
  a named-state click, which is how the month and day calendar views were
  found broken.
- Fixed, in the order found:
  - 23 query sites cast `id as "id!: String"` on a `Uuid` column. sqlx encodes
    a Uuid as BLOB(16) but the column is declared TEXT, so the .sqlx cache
    recorded the DECLARED type, the build passed, and the runtime decode
    failed with a Utf8Error. `GET /api/automation/rules` returned 500 and the
    POST left an orphaned row. Fixed with the repo's own convention, an
    explicit `as "id!: Uuid"`.
  - The RSS writers passed `json!({})` as `posts.media` - an OBJECT where
    every reader expects an ARRAY. `post.media?.find(...)` threw
    "i.find is not a function" and took the kanban down. Fixed at both
    writers, plus migration 0008 to normalise the existing rows, plus a
    server-side normaliser so the shape is guaranteed on the way out.
  - `providerInk` computed the provider glyph's colour against a HARDCODED
    surface, and the glyph sits on three different ones. On the light theme
    the LinkedIn ink measured 3.67:1 on the search chip and 4.32:1 on the
    kanban card - both under the AA floor. It now takes the surface.
  - The PROBE skipped alpha compositing, so every tinted chip measured
    against the surface BEHIND the wash instead of the wash itself. Fixing
    the probe revealed that in the light theme EVERY semantic hue on its own
    wash is under 4.5:1. Fixed at the token level.
  - `opacity` was not accounted for in the contrast check, and a calendar cell
    with `opacity-40` dimmed its whole subtree - a past day's post measured
    1.33:1. The probe learned about opacity groups, and the cell's dimming
    moved from the subtree to the cell's chrome.
  - The `!` badge on a failed calendar chip was 8px, the tag chips 9px, and
    the month view's chip was 15px of a 144px cell.
  - Editing a post WIPED the provider settings: the update REPLACED the
    settings object, and the composer sent `{}` because the detail response
    had no settings field. A working scheduled Reddit post failed at publish
    with "Missing subreddit in settings" thirty seconds after it worked. The
    settings now MERGE, the detail response carries them, and the composer
    preserves them.
  - The analytics window filter bound a `DateTime` against an INTEGER epoch
    column, so the comparison matched NOTHING. The feed half of every
    analytics headline was permanently zero.
  - There was NO way to put a post into a campaign: the composer had a
    campaign select whose value was dropped on submit, and the create API had
    no `campaign_id` field at all. The feature was dead.
  - The command palette was inside an `aria-hidden="true"` subtree, so a
    screen reader announced nothing of it AND the shared focus trap could not
    find a focusable child - the search box never took the focus and typing
    did nothing.
  - Seven dialogs had no focus trap, no accessible name and no aria-modal.
  - The composer took no focus: the trap focused the first control, and the
    async load then re-rendered and DESTROYED the focused element, which put
    the focus on `<body>` - outside the dialog. The trap now watches for its
    focused element being removed.
  - Escape did nothing in the composer, and when it was made to work it left
    an orphaned confirm that swallowed the next Escape on ANY page.
  - The posting-volume chart contradicted the cadence card beside it: the
    query grouped by day, so the 20 days with no post were MISSING from the
    series rather than zero, and 11 posts on 11 days read as "you posted
    every day at full volume" next to "0.4 posts per day".
  - The AI panel's progress bar reported a percentage that was made up.
  - The probe measured the page MID-FADE and called a correct page broken:
    the entry animation's opacity made every ancestor a "translucent group".

Stage Summary:
- The gate is now FIVE scripts over the RENDERED app, all in `make audit`:
  probe.mjs, audit-targets.mjs, audit-contrast.mjs, audit-dialogs.mjs and
  e2e-calendar-drag.mjs. The last one is new: the plan's F3 gate was a drag
  e2e, and there was NO e2e for it. It has teeth - breaking `wallClockToUtc`
  makes it exit 1, restoring it makes it exit 0.
- The last measured state: probe 0 FAIL / 0 WARN, 0 controls under the 24px
  floor, 0 contrast failures in both themes, 0 dialog contract failures, 0 drag
  failures. The empty instance also passes (0 FAIL, 2 correctly-explained
  SKIPs, 12 warnings that are all inline links above the WCAG 2.2 floor).
- Landed in commits a354b4b..78ca215.

Known issues / follow-ups:
  - OPEN: the day bucket in the analytics day series is a UTC day. The
    founder's zone is an IANA name in a FRONTEND localStorage store
    (`switchpost-timezone`), and `users.timezone` is a column the server
    NEVER writes - the -300 in the seed is of unknown unit. So the server has
    no usable per-user zone, and the posting-volume chart's day buckets
    cannot yet follow the founder's day. The two sources are named here. The
    DENSITY fix does not depend on it.
  - OPEN: `/api/ai/*` has no streaming endpoint, so the composer's AI panel
    cannot show real progress. The panel is honest about it (an indeterminate
    indicator) rather than pretending. A streaming endpoint is backend work.
  - The NotificationBell polls its unread count every 60s as a fallback for a
    silently-dropped SSE stream. Deliberate, and recorded in the plan.

---
Task ID: v12-iterations-3-8
Agent: omp (merry-tamarin)
Task: Exhaustive recursive red-teaming of the switchpost web dashboard, with
visual verification through the browseros-neo MCP and the rendered-page probe.

Work Log:
- The 2026-10-20 red-team had probed 13 routes of 29, and every run had been
  against an EMPTY database: an empty state hides every populated-state
  defect. Built `scripts/seed_demo.py` (a realistic solo-founder dataset -
  6 integrations, 17 posts across every state, 3 campaigns, 6 tags, the
  activity log, analytics cache, external posts, automation rules, webhooks,
  signatures, RSS feeds, notifications, post sets, media and comments) and
  extended the probe to all 29 routes plus the two dynamic ones.
- The seed's first run wrote TEXT uuids into sqlx's BLOB(16) uuid columns, so
  every query against those tables failed at runtime. The seed now writes
  16 raw bytes and ASSERTS every uuid column's storage type before it
  commits - a self-check that names the mistake instead of leaving a broken
  instance behind.
- A seeded uuid id decoded as a String was a 500 on the automation, tags and
  webhooks endpoints - and the POST that failed had ALREADY INSERTED the row,
  so it left an orphan with no id returned. 23 query sites. The fix is the
  repo's own convention, an explicit `as "id!: Uuid"`: the macro infers the
  type from the column's DECLARED type, so dropping the override would NOT
  have worked.
- The probe's contrast check SKIPPED translucent layers instead of
  compositing them, so every tinted chip was measured against the surface
  behind it. Fixed, and it immediately found that EVERY semantic hue on its
  own wash fails AA in the light theme.
- `providerInk` computed the brand ink against a HARDCODED surface, and the
  app paints a provider glyph on four different surfaces. It is a parameter
  now, and each call site states where the glyph goes.
- The calendar's month and day views had NEVER been rendered. The month
  view's chip was 15px of a 144px cell, its day numbers were at 2.20:1, and
  the day view's ghost slot was 21px. The probe can now drive a page into a
  named state (`--state "Month"`).
- Editing a post WIPED the provider settings - the composer had no settings
  to send, so every edit of a scheduled Reddit post removed its subreddit
  and the post failed at publish. Fixed on both sides: the detail response
  carries the settings, the composer preserves them, and the server MERGES
  the incoming object over the stored one.
- No UI path could put a post into a campaign: the select existed, the id
  was dropped, and the create request had no `campaign_id` field. The whole
  feature was dead.
- The composer's channel-target map wrote `channel_id` and `page_id`, which
  NO provider reads, while Discord reads `channel` - so a Discord post
  ALWAYS failed at publish with "Missing channel in settings". The map now
  names what the provider reads, and the server REFUSES a post whose
  required setting is missing, at CREATE time, with a message that names the
  field.
- The connect flow's API-key fields were `text-white` on the light input
  surface: 1.05:1, invisible. An `<input>` has NO TEXT NODE, so no
  text-contrast gate could ever see it. The field's own ink is measured now,
  and the connect flows are driven.
- Eight TRUNCATED Tailwind class names in six files - `hover:text-accent-stro`,
  `items-centerderline`, `justify-centert-muted`, `bg-linetext-content-secondary`.
  A truncated class is a SILENT failure: not a build error, not a type error,
  not a svelte-check warning. `check-tailwind-classes.mjs` finds them and is
  in `make check`.
- A live interaction audit (a subagent driving the real app with a browser)
  found six wrong numbers: the dashboard's time was "07 PM" (a
  `slice(-5)` on a formatted date), every dashboard aggregate came from a
  100-row clip, "Publish success rate 24%" sat above "11 of 13 went out",
  the comments table showed a raw UUID (the join was to `external_posts` and
  the comments are on the founder's OWN posts), a campaign's progress hid its
  failures, and a calendar ghost slot dropped the hour the founder clicked.
- Seven dialogs had no focus trap, no accessible name and no aria-modal. The
  composer took no focus at all: the trap focused the first control and the
  async load re-rendered and DESTROYED it, putting the focus on `<body>`.
- The composer's discard prompt left an orphaned confirm that swallowed the
  next Escape on ANY page, because `composer.dirty` was never reset and the
  cleanup close asked a second time.

Stage Summary:
- The gate is now SIX scripts over the RENDERED app plus two source gates:
  probe.mjs, audit-targets.mjs, audit-contrast.mjs, audit-dialogs.mjs,
  e2e-calendar-drag.mjs, check-hex-tokens.mjs and check-tailwind-classes.mjs.
  `make check` runs the two source gates; `make audit` runs the five browser
  gates.
- The last measured state: probe 0 FAIL / 0 WARN over 29 routes x 3 widths x
  2 themes, 0 controls under the 24px floor, 0 contrast failures in both
  themes, 0 dialog contract failures, 0 drag failures. The EMPTY instance
  also passes (0 FAIL, 2 correctly-explained SKIPs).
- Every gate has been checked for TEETH: each one was made to fail on the
  broken code and pass on the fixed code.
- 483 lib tests + 46 integration binaries pass. Clippy is clean with
  `-D warnings`, and the offline (`SQLX_OFFLINE=true`) build is clean.
- Landed in commits a354b4b..15cc7f9.

Known issues / follow-ups:
  - OPEN: the day bucket in the analytics day series is a UTC day. The
    founder's zone is an IANA name in a FRONTEND localStorage store
    (`switchpost-timezone`), and `users.timezone` is a column the server
    NEVER writes - the -300 in the seed is of unknown unit. So the server has
    no usable per-user zone, and the posting-volume chart's day buckets
    cannot yet follow the founder's day. The two sources are named here. The
    DENSITY fix does not depend on it.
  - OPEN: `/api/ai/*` has no streaming endpoint, so the composer's AI panel
    cannot show real progress. The panel is honest about it (an indeterminate
    indicator) rather than pretending. A streaming endpoint is backend work.
  - OPEN: the plan's F1 hex gate ("no hex outside tokens") was amended to
    name `check-hex-tokens.mjs`, whose allowlist covers the DATA files
    (provider brand colours, the tag palette). The gate is the better
    artefact; the plan now says so.
  - The NotificationBell polls its unread count every 60s as a fallback for a
    silently-dropped SSE stream. Deliberate, and recorded in the plan.

---
Task ID: v12-iterations-9-10 (final state)
Agent: omp (merry-tamarin)
Task: The closing iterations - the plan's own gates that were never run,
plus the recursive re-verification pass.

Work Log:
- 360px was not in the probe's width list, so the plan's F5 gate ("a
  360px mobile pass on composer/calendar/channels") had never run. It
  runs now, across every route, and the whole app passes at 360.
- There was NO skip link. The plan's F5 names one. It is the first
  focusable element, visible once focused, and it moves the FOCUS - not
  just the scroll - to the main region. `audit-dialogs.mjs` checks the
  whole contract. It is a plain `.skip-link` class because Tailwind 3
  emits `not-sr-only:focus` - the `not-` variant must come BEFORE the
  utility, so the natural order generates nothing at all.
- The probe and the target audit learned to see a VISUALLY HIDDEN
  control: a 1x1 box that is CLIPPED AWAY is not a target (the skip
  link pattern); a 1x1 box that is merely tiny still is. Verified in
  the browser against all three cases.
- The composer's TARGET PICKER had no role at all. It is a multi-select
  `role="listbox"` with `aria-multiselectable` and `role="option"` +
  `aria-selected` per row. The channels' QUICK MENU is a `role="menu"`
  with `role="menuitem"` per row.
- F4's verification was "analytics with seeded data renders non-empty" -
  a sentence, not a gate. `audit-analytics.mjs` checks non-zero
  engagement, a DENSE day series, and a success rate that RECONCILES
  with the "N of M went out" beside it. Its teeth are proven: labelling
  the goal-inflated rate a success rate makes it report "the success
  rate reads 24% but 11 of 14 scheduled posts went out, which is 79%".
- THE CALENDAR'S DAY BUCKET NOW FOLLOWS THE FOUNDER'S ZONE - the
  worklog's biggest OPEN item. A post at 02:00 UTC on the 25th was on
  "the 25th"; in New York it is the 24th, so evening posts sat on the
  wrong day. The client resolves the zone's offset (via `zoneOffsetMs`,
  the ONE implementation, extracted from `wallClockToUtc`) and the
  server buckets into the local day. `tz_offset` is VALIDATED - an
  absurd value used to PANIC the handler (`TimeDelta::seconds out of
  bounds`, no CatchPanic layer). The mutation run that "passed" with
  the validation deleted was a STALE BUILD (cargo finished in 0.25s);
  the real run rebuilt (16s) and the test caught the panic.
- `check_status_contrast.py` (all 88 status-token pairs, both themes)
  was written as a gate but never wired in - a future hue change would
  have passed every gate while reintroducing the 2026-10-20 finding.
  It is in `make check`.
- The login page's Unlock button was 343x40 and carried a raw
  `text-white` alongside `text-accent-fg` - the same class of defect
  that made the connect forms invisible in the light theme.

Stage Summary:
- SEVEN gates, all with proven teeth: probe (29 routes x 4 widths x 2
  themes), targets, contrast (both themes, fields included, connect
  flows driven), dialogs (with the skip link), analytics, drag e2e,
  plus the three source gates (hex, tailwind classes, status tokens).
- The last full-gate state: probe 0 FAIL / 0 WARN, 0 controls under the
  floor, 0 contrast failures in both themes, 0 dialog contract
  failures, 0 analytics failures, 0 drag failures. 483 lib tests, 47
  integration binaries, clippy clean online and offline, svelte-check
  clean.
- The empty instance also passes.
- Landed in commits a354b4b..9864450.

Known issues / follow-ups:
  - OPEN (backend): `/api/ai/*` has no streaming endpoint, so the
    composer's AI panel shows an indeterminate indicator rather than
    real progress. Honest, and documented.
  - CLOSED 2026-10-21: see dst-zone-name-bucketing below - the
    series now buckets per-instant in the founder's zone, with a
    zone-day window edge.
  - CLOSED: the MCP `calendar_get` tool now takes `tz_offset` and
    buckets through the SAME day-key helper as GET /api/calendar
    (`api::calendar::local_day_key`), so the two surfaces no longer
    disagree for the same operation (AGENTS.md 9.21). An agent asking
    "what is on Oct 25?" and a founder looking at the calendar now get
    the same answer.
    (2026-10-21: the happy path only - an unknown zone on an empty
    range, and a valid name beside an out-of-range offset, still
    disagreed. Both fixed; see dst-zone-name-bucketing.)
  - CLOSED 2026-10-21: see dst-zone-name-bucketing below - the client
    sends the zone NAME and the server resolves it per instant, so a
    window spanning a DST transition no longer buckets the
    post-transition posts an hour late.

---
Task ID: dst-zone-name-bucketing
Agent: omp (goal session, 2026-10-21)
Task: Fix the two open visual-layer bucketing bugs: the calendar's DST
wrong-day bucketing (one constant offset per window) and the analytics
day series bucketing in UTC while the calendar buckets locally.

Work Log:
- Added `chrono-tz` 0.10 (compile-time embedded IANA tzdata, single-binary
  rule intact) and recorded the lock via `vendor-pull.sh --update`.
- ONE seam, one rule: `api::calendar::parse_zone` (parses an IANA name
  ONCE, up front) + `zone_offset_seconds` (per-instant offset) +
  `local_day_key` (unchanged day-key rule). REST, MCP and analytics all
  route through it.
- GET /api/calendar accepts `tz=<IANA name>`, resolved per-instant
  (DST-correct). `tz_offset` stays as the legacy constant form, and its
  bound is now UNCONDITIONAL: a valid name must not smuggle a garbage
  offset past the 400.
- Unknown zone name is a 400 naming the bad zone - NOT a silent UTC
  fallback (a garbled zone would misplace every post in the window with
  a 200 and plausible day keys). The first implementation had exactly
  that silent fallback; the test that pinned it was rewritten.
- `calendar_get` (MCP) takes `tz` too; parse lifted ABOVE the post loop
  (was per-post inside it, so an empty range answered Ok with no days -
  an agent would read a bad zone as "nothing scheduled"); offset bound
  unconditional, matching REST (9.21).
- Analytics `dense_day_series` buckets in the founder's zone: buckets,
  `start` AND `today` are all zone days. The UTC-`start` bug dropped
  every post published in the first hours of the window (they bucketed
  below the loop's first day while the SQL counts included them).
- Frontend: calendar, dashboard and analytics clients send the zone
  NAME (`timezone.value`); the client no longer resolves offsets.
- Tests: DST regression asserts Nov 1 by name AND Nov 2 by constant
  (teeth both directions); zone-edge series test; unknown-zone 400;
  MCP unknown-zone + bypass tests; REST bypass mirror. The MCP bypass
  test was run RED against the buggy shape before the fix landed
  (9.51 discipline).

Stage Summary:
- cargo test --lib: 486 pass. calendar suites: 8 + 8 pass.
- clippy --lib --bins --all-features --locked: clean (pre-existing
  core2 patch warning only).
- pnpm build + svelte-check: clean. make check: clean (88 contrast
  pairs, tailwind classes, vendor pins).
- All 7 browser gates against the final release binary on :6544:
  probe 27x4x2 0 FAIL / 0 WARN, 0 targets under floor, 0 analytics
  failures, 0 dialog failures, 0 contrast failures both themes,
  0 drag failures.
- Live oracle on the release binary: `tz=Not/AZone` -> 400 with the
  message; a 02:00Z post lands 10-24 under `tz=America/New_York` and
  10-25 under UTC; the name+offset bypass pair -> 400.

Known issues / environment findings:
- The :6544 audit instance is supervised by /tmp/supervise-seed.sh
  (while-true respawner, seeded DB at /tmp/sf-seed). It now runs the
  RELEASE binary. Killing the server does not kill the instance; a
  gate run can silently measure a stale build if the binary is not
  rebuilt first. It was serving a DEBUG binary from an earlier tree
  state for part of this session, which produced a false "tz not
  applied" signal.
- `make deploy`/`make redeploy` are broken on this host by the
  cargo-zigbuild 0.23.4 / zig 0.16.0 skew ("unsupported linker arg:
  --no-keep-memory"); native `cargo build --release` + supervisor
  respawn is the working path here.
- A server started without APP_URL/FRONTEND_URL overrides uses the
  repo .env's :6543 and fails CSRF for a :6544 origin.
- OPEN (separate item): the remaining analytics sites
  (get_engagement.by_day_sql, get_cadence.day_rows,
  calculate_streak) still day-bucket in UTC; the summary series is
  zone-correct now, those three are not.
- OPEN (unchanged): /api/ai/* streaming (backend, not visual-layer).

---
Task ID: analytics-zone-day-completion
Agent: omp (goal session, 2026-10-21)
Task: Close the last three UTC day-bucketing sites on /analytics so the
page stops disagreeing with itself at the day boundary (the OPEN item
from dst-zone-name-bucketing).

Work Log:
- `AnalyticsDaysQuery` gains `tz` (IANA name, unknown -> 400, parsed
  ONCE via the shared `parse_query_zone`). The summary endpoint now
  uses the same helper instead of its own inline parse.
- ONE day-date rule for every analytics series: `zone_date(dt, tz)`
  (AGENTS.md 9.18). `dense_day_series`, `fill_missing_days` and
  `calculate_streak` all bucket through it.
- `get_engagement`: the SQL no longer groups by
  `date(published_at,'unixepoch')` (UTC). It selects one row per post
  (epoch + per-metric sums via the metric-label vocabulary) and the
  day bucket is computed in Rust - SQLite's
  `date(ts,'unixepoch',?)` takes a CONSTANT modifier and cannot
  express a per-instant zone offset, so the DST bug would come back
  with any SQL-side grouping. `fill_missing_days` now takes the zone:
  BOTH edges (start AND today) are zone days.
- `get_cadence`: same shape - per-post epochs, zone-day counts in
  Rust. The "Cadence against goal" chart can no longer disagree with
  the zone-correct "Posting volume" chart beside it.
- `calculate_streak`: the DISTINCT-date SQL is replaced by per-post
  epochs deduped by zone day, and the today/yesterday anchor is the
  ZONE's, so a streak no longer resets at the wrong hour for a
  founder whose midnight is not UTC midnight.
- Frontend: getEngagement/getAdherence/getCadence take the zone NAME;
  the analytics page and the dashboard pass `timezone.value`.
- Visual-layer fix found by the gate run (not by the analytics diff):
  the kanban "older posts" banner's inline Posts link was 31x15 -
  under the 24x24 WCAG 2.5.8 floor AND audit-targets' 44px floor. It
  is a conditional banner that only renders past 200 posts, which is
  why earlier probes passed: the seeded DB had not crossed the limit.
  Now `inline-flex min-h-11 min-w-11` (the ChannelSelector pattern),
  44x44 rendered.
- Environment finding: the probe run that reported 19 navigation
  FAILs ("system error -122") was EDQUOT - ~550MB of my own leftover
  /tmp/probe-* screenshot dirs filled the tmpfs quota and the probe
  could not WRITE its screenshots. Cleaned. A gate run that fails on
  `write` errors is measuring the disk, not the app.

Stage Summary:
- cargo test --lib: 487 pass (incl. the new engagement zone-edge
  series test). Calendar suites: 8 + 8.
- clippy --lib --bins --all-features --locked: clean.
- pnpm build + svelte-check: clean. make check: all gates clean.
- All 7 browser gates against the final release binary on :6544:
  probe 27x4x2 0 FAIL / 0 WARN, 0 targets under floor, 0 analytics
  failures, 0 dialog failures, 0 contrast failures both themes,
  0 drag failures.
- Live oracles: `tz=Not/AZone` -> 400 on /engagement; /cadence
  answers the zone name (NY and UTC agree on the seeded mid-day
  posts - correct for that data; the near-midnight disagreement is
  pinned by the unit tests with discriminating instants).

Known issues:
- OPEN: research "best time to post" still buckets weekday/hour in
  UTC and labels it UTC while the rest of the app is zone-aware.
- OPEN: the client/server day-rule pairing - `zone_offset_seconds`
  vs the frontend's `wallClock` - two implementations of one rule
  with nothing pinning them (the counting_rule_test shape).
- Gate coverage gaps: /login is in no gate's route list; the
  calendar Month/List views are not probed by make audit (no --state
  passes).

Follow-up to analytics-zone-day-completion (same session):
- OPEN: the two tap-target gates carry DIFFERENT thresholds -
  probe.mjs FAILs below 24px and WARNs 24-44 (WCAG 2.5.8), while
  audit-targets.mjs FAILs anything under 44px. A control in the
  24-44 band passes the probe with a warning and fails make audit.
  AGENTS.md 9.37 requires the two gates to agree; converging them
  means either fixing ~100 sub-44 controls the audit already
  measures or re-banding the audit's FLOOR - a deliberate decision
  for the next session, not a silent one.
- OPEN: the TopBar streak badge reads `users.streak_days`, a
  24h-window counter maintained by the scheduler - a DIFFERENT
  measure from the zone-day streak on the cadence card
  (`calculate_streak`), which this session made zone-aware. Two
  "streak" numbers can now be on screen, one zone-aware and one
  not, and they can disagree across a DST or midnight boundary.
- Housekeeping: frontend/probe-out/ (a stale 13-route report from
  an old 6543 run) was gitignored but still TRACKED from before the
  ignore landed; untracked and deleted.

---
Task ID: research-best-time-zone
Agent: omp (goal session, 2026-10-21)
Task: Make the research "best time to post" slots zone-aware, closing
the last UTC-labelled timing surface in the app.

Work Log:
- `engagement_histogram_by_slot` now returns one row per (provider,
  epoch): the weekday/hour fold moved out of SQL (strftime('%w'/'%H')
  is UTC-only; `localtime` is the OS zone, not the founder's) to the
  caller, the same shape as the analytics day series.
- ONE slot rule: `api::research::week_slot(epoch, Option<Tz>)` folds
  an epoch into a (weekday, hour) slot in the founder's zone
  (DST-correct) or UTC. The research insight and the MCP
  `posts_best_times` tool both fold through it (9.18), so the two
  surfaces cannot disagree about a post's slot.
- GET /api/research takes `tz=<IANA name>` (unknown -> 400): the
  slots are bucketed in the founder's LOCAL weekday and hour, and the
  label names the zone ("Saturday 06:00-07:00 America/New_York") -
  "post Mon 15:00" is only actionable in the zone the founder posts
  from. The MCP tool keeps its documented UTC fold and its note says
  so.
- The research page's client-side `slotLabel` is DELETED: it used to
  convert the clock to the app's zone while keeping the UTC weekday -
  a second implementation of the server's rule that could disagree at
  the midnight boundary (a 22:00 NY post was "Sunday" by the clock
  and "Saturday" by the server's key, with the weekday pinned to the
  server's). The page now renders the server's zone-correct label
  and sends `timezone.value` in the request.

Stage Summary:
- cargo test --lib: 488 pass (incl. the zone-fold regression:
  2026-10-25T02:00Z -> Sunday 02:00 UTC, Saturday 22:00 New York).
- clippy --lib --bins --all-features --locked: clean.
- pnpm build + svelte-check + make check: clean.
- All 7 browser gates against the release binary on :6544: probe
  0 FAIL / 0 WARN, 0 targets under floor, 0 analytics/dialog/
  contrast/drag failures.
- Live oracle: the same imported posts read 06:00-07:00 in
  America/New_York and 10:00-11:00 UTC (EDT, -4h); the label names
  the zone; tz=Not/AZone -> 400.

Known issues: unchanged from the previous entries (the two-streak
TopBar measure, the gate threshold drift, /login gate coverage,
calendar Month/List probe states, the client/server day-rule parity
test).

Correction to research-best-time-zone (same session): its summary
said "the page now renders the server's label verbatim" - false.
The cell rendered `slot.label.replace(/^\w+\s/, "")`, which left the
zone name on the clock inside an 80px truncate span, so the visible
cell was a mangled "06:00–07:00 Amer...". Fixed: PostingSlot carries
`hour_range` (the clock alone) and the cell renders it; `label`
(full, zone-named) stays the each-key and is the cell's tooltip;
the caption below already names the zone. Probe on /research: 0 FAIL,
0 WARN against the rebuilt binary.

---
Task ID: wall-clock-parity
Agent: omp (goal session, 2026-10-21)
Task: Pin the client/server day-rule pairing - `zone_offset_seconds`
(server, chrono-tz) vs the composer's `zoneOffsetMs` (browser, Intl) -
two implementations of one rule with nothing comparing them.

Work Log:
- `api::calendar` gains `zone_offset_by_name(zone, at)` - the
  name-taking form of the offset rule, parsing through THE ONE
  `parse_zone`. The module is `pub` now, the same pattern as
  `api::activity` and `api::research` before it, so an integration
  test can drive the real seam.
- tests/wall_clock_parity_test.rs, the counting_rule_test shape:
  it loads the REAL frontend/src/lib/composer/wall-clock.ts and runs
  it under node (--experimental-strip-types), never a transcription.
  Two tests:
  1. Per-instant agreement across 15 discriminating cases: both sides
     of the US fall-back (2026-11-01), US spring-forward (2026-03-08),
     EU fall-back (2026-10-25) and NZ spring-forward (2026-09-27)
     transitions, the transition instants themselves, Kolkata and
     Kathmandu (half-hour/45-minute offsets), and ordinary instants.
     The browser side asserts a non-zero offset for non-UTC zones, so
     a stale system ICU fails loudly instead of comparing against a
     silent fallback.
  2. The WRITE-path round-trip: wallClockToUtc's instant + the
     SERVER's offset reads back the wall clock the founder typed,
     exactly for ordinary clocks (day and clock fields, seconds
     included). The pathological clocks are pinned for
     determinism, not exactness: the spring-forward GAP (2026-03-08
     02:30 New York, which does not exist) reads back 03:30 or 01:30,
     and the fall-back AMBIGUITY (2026-11-01 01:30, which happens
     twice) reads back 01:30 on the day - the founder gets a
     deterministic answer, not an error.

Stage Summary:
- cargo test --test wall_clock_parity_test: 2 pass. cargo test --lib:
  488 pass. clippy clean.
- No product code changed except the pub wrapper - this iteration is
  the pin, not a fix: the two rules were already agreeing per-instant
  (the DST work landed them through the same seam); now they cannot
  drift apart silently.

Known issues: unchanged (TopBar's separate users.streak_days measure,
the probe/audit-targets threshold drift, /login gate coverage,
calendar Month/List probe states).

Red-proof for wall-clock-parity (same session, after the commit):
mutating `zone_offset_by_name` to a constant 0 made BOTH tests fail
(per-instant: "server said 0s, the composer's Intl rule said -14400s";
round-trip: the read-back wall clock shifted), and reverting restored
green - the tests have seen their own failure (AGENTS 9.51). The ICU
sanity assertion (assert_ne! 0 for non-UTC zones) is in the committed
test at the per-case loop, lines 100-103.

---
Task ID: streak-one-measure
Agent: omp (goal session, 2026-10-22)
Task: Unify the two "streak" numbers on screen - the TopBar flame
badge (users.streak_days, the scheduler's 24h-window counter) and the
cadence card's zone-day streak - into ONE measure.

Work Log:
- /api/streak is a real handler now (State + AuthenticatedUser +
  Query<StreakQuery>): it computes through analytics::calculate_streak
  - THE SAME zone-day rule the cadence card reads - with the session's
  user and the zone the badge sends (StreakBadge passes
  timezone.value). It used to read users.streak_days, a separate
  24h-window counter the scheduler maintains, looked up by the
  HARD-CODED DEFAULT_USER_ID - so the two streak numbers could
  disagree across a midnight boundary, and on an instance whose users
  row is not exactly that constant the badge read a user that owns NO
  posts: 0 forever, a real defect this instance's seed exposed.
- The MCP analytics_streak tool computes through the same
  calculate_streak with its documented resolve_first_user convention;
  its days are UTC (no zone input yet).
- The scheduler's counter is untouched: it stays as a cheap internal
  hint, but nothing DISPLAYS it, so there is only ever one number the
  founder can read.
- tests/streak_unification_test.rs drives the real router: (1) badge
  == cadence for the same zone, both 3 against three consecutive
  published days, in UTC and New York; (2) the session-user pin - a
  fresh non-DEFAULT_USER_ID user with no posts reads 0, with today's
  post reads 1, isolated per-test by its own user row and session;
  (3) an unknown zone is a loud 400.
- Found while proving it live: my external sqlite3 writes to the
  seeded DB were invisible to the server (single-writer rule, G-08 -
  the app's pool reads its own WAL view). External writes to the
  operator's DB are out of contract; the seeded instance is now clean
  (probe rows deleted, verified 0) and the proof lives in the
  integration test, which seeds through the router's own pool.

Stage Summary:
- cargo test --test streak_unification_test: 3 pass. cargo test
  --lib: 488 pass. clippy clean. pnpm build + svelte-check clean.
- Live (seeded instance, release binary): badge == cadence == 0 -
  correct for its data (last post 10-19, two-day gap); bad zone ->
  400. The non-zero agreement proof is the integration test's.

Known issues: unchanged (probe/audit-targets threshold drift, /login
gate coverage, calendar Month/List probe states).

Follow-up to streak-one-measure (same session):
- Red-proof run: reverting streak_check's user to the hard-coded
  DEFAULT_USER_ID made the_badge_counts_the_session_users_own_
  published_days FAIL ("no published posts for this session's user:
  left 3, right 0" - the constant read another user's posts, the
  exact pre-fix defect) and green again on revert.
- The MCP analytics_streak tool now takes tz (IANA name, unknown ->
  error, the same loud contract as REST) so an agent can ask in the
  founder's zone.
- OPEN, named: the MCP tool resolves its user through
  resolve_first_user, which is hard-coded DEFAULT_USER_ID across
  ~80 tool sites. On an instance whose users row differs from that
  constant, EVERY MCP tool reads a user that owns no posts - the
  badge had exactly this defect before this fix. Making
  resolve_first_user DB-backed (resolve the single local user) is a
  wide-radius change needing its own tests; it is its own item, not
  silently assumed done.

---
Task ID: gate-coverage-login-and-calendar-states
Agent: omp (goal session, 2026-10-22)
Task: Close the gate-coverage gaps - /login was in no gate's route
list, and the calendar's Month/List/Day views were never rendered by
make audit (9.50: a page probed only in its default state is a page
whose other states are unchecked).

Work Log:
- probe.mjs gains --logged-out: it skips the login AND the dynamic
  route resolution, so the pass measures the pages an anonymous
  visitor sees. /login CANNOT go in the authenticated route list: the
  layout guard redirects a logged-in visitor to '/' (+layout.ts), so
  an authenticated probe of /login would measure the dashboard twice
  while claiming to cover the login page.
- make audit now runs four additional passes: /login logged-out, and
  /calendar in the Month, List and Day states (probe.mjs --state).
  AGENTS.md 9.50's "the three calendar views are all probed" is now
  TRUE of make audit, not just of a manual invocation.
- The new passes found real defects on their first run, exactly the
  9.50 shape: the List view's ◀/▶ pagination buttons rendered 23x24
  and 23x44 - under the 24x24 WCAG 2.5.8 floor - and no gate had ever
  seen them because nothing drove the List state. Both buttons are a
  44x44 box now (min-h-11 min-w-11 inline-flex, the kanban-link
  pattern).
- The Month (88) and Day (112) WARNs are honest 24-44-band
  classifications of dense-grid chips and 24px slot rows - the 9.37
  hazard class where a 44px-everywhere rule would blow the layout;
  they are WARNs by the probe's WCAG-matched severity, not failures.

Stage Summary:
- All five audit passes green against the release binary: default
  27-route pass 0 FAIL / 0 WARN; /login logged-out 0/0; Month 0 FAIL;
  List 0 FAIL; Day 0 FAIL.
- pnpm build + svelte-check + check-tailwind clean; the List-button
  fix is in the embedded frontend of the rebuilt binary.
- OPEN (unchanged): probe/audit-targets threshold convergence (24 vs
  44); resolve_first_user DB-backed resolution.

Known issues: none new.

Follow-up to gate-coverage-login-and-calendar-states:
- Verified the --state clicks REALLY land: applyStates matches the
  button's accessible name as a word (getByRole name=/\bMonth\b/) and
  the calendar's view buttons carry aria-label="Month view" etc.;
  the passes' screenshot names carry the _Month/_List/_Day suffix
  (added only after a successful state click) and the three profiles
  are materially different (Day's slot-row WARNs and List's pager
  FAILs exist only in their own views). A gate that silently
  re-measured the default would have shown the same profile three
  times.
- make audit now removes its five probe output dirs at the end
  (~100MB per run); this session already hit EDQUOT on accumulated
  probe litter and a write failure looks like an app failure.

---
Task ID: target-gate-severity-band
Agent: omp (goal session, 2026-10-22)
Task: Converge the two tap-target gates (AGENTS.md 9.37: "the
severity has to match the standard, or the gate lies" and "the two
gates must agree").

Work Log:
- audit-targets.mjs is re-banded to the standard and to the probe:
  below 24x24 is a FAIL (exit 1), 24-44 is a WARNING (reported,
  written to the JSON under 'warnings', not gating). It used to fail
  the gate on anything under 44, so a control the probe classified
  as a warning failed make audit - the two gates disagreed about the
  same rendered box.
- The checkbox-wrapper case keeps its own band logic (a 24-44
  control inside a 44px label is a warning, not a fail).
- The OUT json is now {fails, warnings} - the old flat array shape
  changed; any consumer reading it must take the new shape (none in
  the repo; make audit only checks the exit code).

Stage Summary:
- Live run against the release binary: 0 controls under the 24px
  floor, 0 in the 24-44 band across the 25 walked routes + the
  composer and its three panels, exit 0. The kanban Posts link that
  the old gate failed is 44x44 and absent from both bands.
- Both gates now classify by the standard's severity: probe FAIL
  <24 / WARN 24-44, audit-targets FAIL <24 / WARN 24-44.

Known issues: resolve_first_user DB-backed resolution remains the
open queue item.

---
Task ID: resolve-first-user-seeded-instance
Agent: omp (goal session, 2026-10-22)
Task: Make resolve_first_user (the ~80-site MCP/CLI user seam) resolve
the instance's REAL local user instead of returning a hard-coded
constant - the seeded-instance defect the streak fix exposed.

Work Log:
- resolve_first_user used to return DEFAULT_USER_ID
  unconditionally. On any instance the app started, that is correct:
  ensure_local_user creates the row. But an EXTERNALLY-seeded
  instance (a copy, a fixture, a migrated database - this session's
  own audit instance) carries its single users row under a different
  id, and then every MCP tool read a user that owned no posts.
- The fix: the constant when its row exists (the normal install,
  unchanged behaviour for every standard deployment), otherwise the
  instance's single real user, otherwise a loud error naming the
  mis-seeding. Multiple users is refused - single-user mode has no
  rule for picking one arbitrarily.
- tests/resolve_local_user_test.rs pins all three arms against the
  real helper. Red-proof: mutating the resolve back to
  always-the-constant made the seeded-instance test FAIL (the
  constant answered instead of the seeded user) and green on
  revert.
- mcp_parity_test's streak call updated for the StreakGetInput { tz }
  shape from the previous iteration.
- Visibility: mcp::auth and resolve_first_user are pub (the
  test-reachable-seam pattern, like api::calendar and
  api::activity).

Stage Summary:
- cargo test --lib 488; resolve_local_user 3; mcp_parity 9;
  mcp_tools 20; streak_unification 3 - all green. clippy clean.
- The ~80 call sites' behaviour is UNCHANGED on a standard install:
  the constant's row exists there and the first query confirms it.

Known issues: the queue from the previous entries is now empty. The
remaining work is discovery, not named items: another red-team pass
over the surfaces this session did not touch (automation, feed,
campaigns detail, comments) with the widened audit gates.

Correction to resolve-first-user-seeded-instance's summary:
- "every item landed with red-proofed regression tests" overstated.
  Red-proofed: the DST day-key tests, the zone-name bypass test, the
  wall-clock parity pair, the streak session-user test, the
  resolve_first_user seeded-instance test. NOT red-proofed: the
  analytics zone-series tests and the resolve constant-arm (both
  verified green against discriminating data, but no mutation run).
- Two paths were named as landed while never executed (9.38):
  (1) the tap-target re-band's 24-44 WARN branch - the live run
  found zero controls in the band, so the new classification has
  only run its all-clear path; the branch's first real exercise will
  be whichever future control lands in the band;
  (2) resolve_first_user's multiple-users refusal arm - NOW COVERED:
  tests/resolve_local_user_test.rs::two_users_without_the_constant_
  is_a_loud_refusal executes it against a two-row users table and
  passes.

---
Task ID: target-audit-mobile-widths
Agent: omp (goal session, 2026-10-22)
Task: Fresh discovery pass - extend the tap-target gate to the
founder's phone widths, which no target audit had ever measured
(the probe measures 375/360 for overflow/contrast; audit-targets
only walked at 1440).

Work Log:
- audit-targets.mjs walks the 25 routes and the composer panels at
  1440, 375 AND 360 now. The mobile layouts are DIFFERENT CSS
  (collapsed sidebars, stacked toolbars), so a control that clears
  the floor at 1440 can be squeezed by its mobile layout - the
  desktop-only audit could not see it. The 'where' keys carry the
  width (route@375); make audit reads only the exit code, so no
  consumer broke.
- Result on the first full run: 0 controls under the 24px floor and
  0 in the 24-44 band across all 75 route-width combinations plus
  the composer and its three panels at each width. The mobile
  layouts clear the floor everywhere the audit walks.

Stage Summary:
- Live run against the HEAD release binary: exit 0, empty JSON.
- No product change: this iteration widened the gate and found
  nothing to fix - recorded so the next pass knows mobile tap
  targets were measured, not skipped.

Known issues: none new. The unexercised 24-44 WARN branch remains
the one named gap (previous entry).

---
Task ID: browseros-visual-pass-dialog-name
Agent: omp (goal session, 2026-10-22)
Task: The objective's browseros-neo visual pass - drive the real
dashboard in the real browser and look at what the DOM gates
cannot see.

Work Log:
- Drove browseros-neo (MCP over HTTP, 127.0.0.1:9012) end to end:
  initialize/session-id handshake, navigate, snapshot, act,
  evaluate, screenshot. Logged in through the real form (native
  setter + requestSubmit; the act 'type' sets the value without
  Svelte's rune seeing it, so the Unlock button never enabled).
- FINDING (visual-layer, real-browser-only): the composer dialog's
  accessible name was the generic "Dialog". ModalManager falls back
  to aria-label='Dialog' when the caller passes no title, and the
  composer mounts with title:'' because it renders its own
  "Create Post" heading - so every screen-reader announcement for
  the app's most important dialog was the word "Dialog". The
  audit-dialogs gate SAW name="Dialog" and passed it: a gate that
  accepts a generic name is not checking the 9.49 contract's "a name
  that describes WHAT the dialog is".
- Fix: ModalOptions gains ariaLabel (for callers that render their
  own heading inside); ModalManager uses it before the generic
  fallback; the composer mounts with ariaLabel:'Composer'. The gate
  now FAILS a generic "Dialog"/"Modal" name, so the class is caught.
- FINDING (environment, not app): browseros's own transient
  <bit-notification-bar-root> overlay covered the Month-view
  button at one click point and act refused the click; the bar is
  extension chrome that dissolves, not app content - playwright
  contexts never have it. Recorded so the next real-browser pass
  knows covered-element refusals can be transient.
- The Month view driven in the real browser renders the full a11y
  grid (day cells, named post chips with keyboard-resize hints) -
  structural evidence the state renders, on top of the probe's
  numeric gates.
- Image viewing is unavailable in this session's model, so the
  screenshot rubric is BLOCKED per the tool protocol - the
  structural snapshots carry the visual-layer evidence instead.

Stage Summary:
- audit-dialogs against the rebuilt binary: composer
  name="Composer" modal=true focusInside=true, 0 contract failures.
- make check clean; svelte-check + pnpm build clean.
- The composer click in the last browseros attempt did not open
  the dialog (the earlier attempt did); the DOM gate exercises the
  same modal-manager path and shows the named dialog, and the
  earlier real-browser session rendered the composer's full
  structure. Recorded as-is.

Known issues: none new beyond what is recorded above.

Follow-up to browseros-visual-pass-dialog-name (same session):
- The browseros CDP port (127.0.0.1:9171) is a SECOND, more reliable
  driving path than the MCP wrapper: playwright-core's
  connectOverCDP drives the real browser without the wrapper's
  click-coverage refusals, with real viewport control, and the
  browseros session's cookie carries the authentication.
- scripts/rubric-shots.mjs (frontend) bridges the objective's
  visual-testing ask to the website-design skill's rubric: it
  captures the seven judged surfaces (dashboard, the three calendar
  views, analytics, research, kanban) from the real browser,
  authenticated, into a directory for scoring. Exits 3 (BLOCKED)
  when browseros's CDP is down - screenshots are never faked.
- BLOCKED, named honestly: the ten-line look rubric and the CRAFT
  score require VIEWING the screenshots, and this session's model
  cannot view images (verified twice: read on a PNG returns "does
  not support images"). Per the tool protocol, BLOCKED is never
  reported as passed - the rubric has NOT been scored. The shots
  live in /tmp/sf-rubric and the bridge is permanent, so the next
  session with image viewing scores them in minutes.

Full-suite verification at 8414a04 (same session):
- The complete make audit (9 passes) ran against the HEAD release
  binary end to end: default 27-route probe 0 FAIL / 0 WARN; targets
  0 under the 24px floor at all three widths; dialogs 0 contract
  failures (composer name="Composer"); analytics 0; drag 0; the
  calendar Month/List/Day state passes 0 FAIL each (88/54/112
  24-44 band WARNs - the dense-grid chip class, correct severity);
  the /login logged-out pass 0/0. Exit 0, ~13 minutes.
- Rubric-scoring attempt via a subagent confirmed CANNOT VIEW
  IMAGES is a property of the model chain, not of the file or tool:
  the shots remain ready in /tmp/sf-rubric for a vision-capable
  session. No scores fabricated.

---
Task ID: dialog-gate-widening-401-bounce
Agent: omp (goal session, 2026-10-22)
Task: Widen the dialog gate to every reachable modal surface (the
review's follow-up to the composer's generic "Dialog" name), and fix
what the widened gate found.

Work Log:
- audit-dialogs.mjs's trigger list grew from 6 to 8: the Post
  Statistics modal (calendar list row) and the AI Post Generator -
  the two the founder opens most after the composer. Trigger
  matching is exact on accessible name / title / aria-label; the
  generator's emoji-prefixed text is matched via its title
  attribute.
- The widened gate found a REAL defect on its first run: clicking
  the list row's View-post-statistics button navigated the founder
  to /login. Root cause, two layers:
  1. GET /api/analytics/post/{id} answers 401 "token_expired" when
     the post's PROVIDER token is expired - a normal, actionable
     state (refresh the channel token), not session death.
  2. The client's global 401 interceptor bounced to /login on ANY
     401 without reading the body, so the provider-401 destroyed
     the just-opened modal and dumped a still-logged-in founder on
     the login page.
- Fix, at the client seam (one place, every caller): the bounce
     fires only on the SESSION 401 (body code "no_session"); every
     other 401 returns to the caller as an error, with the envelope
     unwrapped so the UI shows "Token expired", not raw JSON.
- Live verification through the real browser (browseros CDP):
  the stats click now OPENS the modal and it STAYS OPEN, showing
  "Token expired" inline - a message the founder can act on.

Stage Summary:
- audit-dialogs: 8/8 dialogs pass with real accessible names
  (composer "Composer", post-stats "Post Statistics", generator
  "AI Post Generator"), 0 contract failures.
- Regression after the client.ts change (it is global): the
  default probe 0 FAIL / 0 WARN, analytics 0 failures, make check
  clean, svelte-check clean, rebuilt release binary live.

Known issues: the stats modal showing "Token expired" for an
unrefreshed channel is the honest state; a follow-up could add a
"Refresh token" affordance in that modal, but the destructive
bounce is gone.

---
Task ID: stats-modal-inline-token-refresh
Agent: omp (goal session, 2026-10-22)
Task: The named follow-up from dialog-gate-widening-401-bounce: the
stats modal honestly says "Token expired" but gives the founder
nothing to act on. Add the refresh affordance inline.

Work Log:
- CalendarEvent gains integrationId (the post API already returns
  integration_id; the mapping never carried it), threaded through
  toCalendarEvent and the stats modal's mount.
- PostStatsModal detects the provider-token 401 (status 401 with the
  unwrapped "Token expired" message) and, when it has the
  integration, renders an inline "Refresh <channel> token" button
  (integrationsApi.refresh - the same call the Channels page
  makes), then reloads the analytics on success.
- Live verification in the real browser: the stats modal on the
  expired-token post shows "Token expired" + "Refresh X (Twitter)
  token" with a one-line explanation - the founder acts in place
  instead of hunting the Channels page.

Stage Summary:
- svelte-check + pnpm build + tailwind-class check + make check
  clean; calendar probe 0 FAIL / 0 WARN; dialog gate 8/8 (0
  failures) against the rebuilt binary.
- The fix completes the 401 story end to end: session 401 bounces
  (correct), provider 401 stays in the modal (fixed last
  iteration), and now it names the channel and offers the refresh.

Known issues: none new.

Follow-up to stats-modal-inline-token-refresh (same session):
- The client now carries the error envelope's machine-readable `code`
  alongside the unwrapped message (all error paths, not just 401) -
  callers branch on code === "token_expired" instead of
  pattern-matching the human string.
- The failed-refresh path is driven and fixed: clicking Refresh on
  this instance's expired X cookie session fails (no usable refresh
  token), and the modal previously re-offered the same failing
  button with a promise it would work. Now the refresh failure
  clears the refresh state and offers the REAL next step - a
  "Reconnect X (Twitter)" link to /channels with the honest
  explanation. Verified live in the real browser: refresh clicked ->
  refresh button gone -> Reconnect link shown.
- A button that cannot succeed is worse than no button.

---
Task ID: real-browser-discovery-sweep
Agent: omp (goal session, 2026-10-22)
Task: Discovery pass over every surface no gate had driven - via
the browseros CDP bridge, watching page errors, bad responses, and
rendered state.

Work Log:
- Campaign detail (/campaigns/{id}): renders fully - overview,
  posts-by-state, progress bar, dates, goal - no errors, no bad
  responses.
- Feed, comments: clean. DMs: the known X-provider 502 on
  conversations (the expired token; the probe already classifies
  it PROVIDER ERROR, not an app defect).
- All six settings subpages (signatures, rss, notifications,
  profile, mcp, developer): zero page errors.
- The composer driven end to end for the first time in a real
  browser: opened from the + New Post button, text typed into the
  contenteditable through the CDP bridge, the character counter
  updated live (155/4000 - the plain-text rule fires on real
  input). No page errors, no bad responses.

Stage Summary:
- One sweep, zero new defects: the surfaces converge. The 401 arc
  (bounce -> stay -> refresh -> reconnect) was this session's last
  defect class, and every driven surface since renders clean.
- The queue now feeds from: the vision-blocked rubric (shots and
  bridge ready), the next gate widening, or founder reports.

Known issues: none new.

---
Task ID: dms-cookie-auth-misdiagnosis
Agent: omp (goal session, 2026-10-22)
Task: Post-completion review finding: the DMs page told the founder
"X post or user not found - it may be deleted or protected" when
the real cause is structural - X's DM API is bearer-only and the
channel is cookie-auth.

Work Log:
- Root cause chain: get_dm_conversations calls
  api.twitter.com/2/dm_conversations with Bearer {access_token};
  for a cookie-auth channel that string is the cookie blob, not a
  bearer - X answers 404 (not 401), classify_error maps 404 to
  NotFound, and friendly_error renders the deleted-post copy. The
  From<ProviderError> mapping never sees a 401 because there is no
  401. The "(raw: Unknown API error)" suffix is the server's own
  friendly_error detail flag, not provider text.
- Fix at the DMs handler seam (all three operations: list, read,
  send): a cookie-auth X channel is refused with a 400 naming the
  real limitation - "X direct messages need an OAuth-connected
  channel. Cookie-auth channels (auth_token + ct0) cannot use X's
  DM API." No provider call is made.
- tests/dms_cookie_auth_test.rs drives the real router: the list
  refusal carries the real message and NOT the deleted-post
  misdiagnosis; the send path refuses the same way. Red-proof:
  disabling the refusal made both tests FAIL (the 502
  misdiagnosis returns), green on revert.
- Live: the cookie-auth X channel now answers 400 with the real
  reason; the DMs page renders it under its existing actionable
  wrapper.

Stage Summary:
- cargo test --lib 488; dms_cookie_auth 2 (red-proofed); clippy
  clean; the /dms probe pass clean against the rebuilt binary.
- The DMs surface now tells the truth on every path: OAuth
  channels get their data, cookie channels get the structural
  reason.

Known issues: none new.

---
Task ID: v27-red-team-1-gate-truth
Agent: omp (goal session, 2026-10-22)
Task: Exhaustive red-team iteration over the web dashboard: full gate
suite baseline, then a real-browser (browseros-neo) sweep of every
route and driven state, watching console errors and failed responses.

Work Log:
- Full `make audit` on the current binary: all five probe passes
  FAIL: 0, targets/contrast/dialogs/analytics gates green. Two
  signals investigated: 54 calendar tap-target WARNs (state filter
  chips at 25px - inside WCAG 2.5.8's 24-44 warning band by design,
  AGENTS.md 9.37, not a defect) and 8 console errors on the
  logged-out /login pass.
- The 8 console errors root-caused to the layout's documented session
  probe: GET /api/auth/me answers 401 on every logged-out visit and
  Chrome logs each as a console error. The APP is correct; the GATE
  was burying real errors under expected noise ("8 errors ...
  (7 more)").
- Fix in probe.mjs: exempt the logged-out pass's 401 whose SOURCE URL
  is /api/auth/me, reading msg.location().url because the console
  text does not name the URL. First attempt exempted on text alone
  and did not fire - caught by re-running the gate. Authenticated
  passes still report every 401.
- Real-browser sweep (browseros-neo MCP, authenticated profile):
  - 22 static routes: all clean, zero console errors.
  - 3 dynamic detail routes (post, campaign, analytics): clean.
  - Driven states: calendar Month + Day views, composer opened and
    typed into (live counter fired), reply modal, tags + automation
    create modals, command palette (filter "cal" -> Enter navigated
    to /calendar, dialog closed), notification panel, theme toggle,
    research refresh, settings profile form, media upload input: all
    clean.
  - No horizontal overflow at the browser's width on dashboard,
    calendar, posts, analytics, channels.
  - SSE end-to-end proven on the real flow: post created + published
    via API while the dashboard was open -> MutationObserver fired
    and the post text appeared in the DOM WITHOUT reload; probe post
    deleted after.
- The palette landing URL /calendar?display=month&date=... is the
  calendar's own deep-link state (Phase v21, replaceState sync) - not
  a defect.
- Visual rubric: BLOCKED this session (model build cannot view
  images). Screenshots captured for operator review at
  /tmp/bos/shot-{dashboard,calendar,analytics,research,posts,channels}.png.

Stage Summary:
- One fix landed: the login gate pass reports console errors: 0 on a
  clean build (was 8), so a real error there can no longer hide.
- Zero app defects found: every route, dynamic detail, modal,
  palette, and the SSE realtime path render and behave clean against
  the seeded instance.
- Committed ca432bb, pushed to github/master.

Known issues: visual rubric needs an image-capable pass over the
saved screenshots (or any future session) - nothing else queued.

---
Task ID: v27-red-team-2-review-corrections
Agent: omp (goal session, 2026-10-22)
Task: Cross-review corrections: the v27-red-team-1 entry
over-claimed the SSE publish path; plus the remaining review
findings (401-by-URL gate tracking, the calendar drag e2e, the
two post-sweep surfaces, width-controlled shots).

Work Log:
- CORRECTION to v27-red-team-1: "Post created + published via API ->
  appeared in DOM without reload" was WRONG. The publish call in that
  run never executed (the create response was {posts:[...]}; the pid
  extraction failed; the script returned CREATE-FAILED before
  publishing). The DOM update observed was the post_created
  broadcast. Only the create half was verified in that run.
- The publish half is NOW verified, with an honest label: a probe
  post on the bluesky channel, published through the REAL
  POST /api/posts/{id}/publish route -> route -> claim ->
  scheduler::publish_post -> provider attempt -> failure -> retry
  path -> post_failed broadcast. A direct EventSource tap captured
  BOTH post_created and post_failed for the probe post on the live
  stream. The provider legitimately failed ("Could not resolve
  handle" - the seed's bluesky credentials, the environment's fault,
  classified per 9.43); no seeded channel authenticates to its
  provider, so a SUCCESSFUL post_published is not provable on this
  instance. The full publish pipeline's SSE layer is exercised;
  the success leg is environment-blocked.
- Calendar drag-to-reschedule e2e (frontend/scripts/e2e-calendar-drag.mjs,
  the plan's F3 gate, not in make audit): PASS. Real
  dragstart/dragover/drop sequence, the PUT carried the correct
  INSTANT (07:36 wall clock preserved through the timezone
  conversion, 9.17), the post landed on the target day after a
  reload. 0 drag failures.
- probe.mjs now tracks 401 responses BY URL (a747fdc): the console
  text does not name the source URL, so a genuine unauthorized call
  was indistinguishable from the auth-guard probe in the summary.
  /api/auth/me on the logged-out pass is exempt; everything else
  401s as a FAIL naming the URL. Verified: logged-out pass 0 FAIL,
  authenticated / 0 FAIL, authenticated /login (guard redirects) 0 FAIL.
- Post-sweep surfaces driven: the DMs cookie-auth 400-refusal
  wrapper renders its structural message in the error wrapper
  (678f9f8 verified live); the stats modal opens clean - its
  token-refresh offer only renders on a token-error fetch, which is
  the correct conditional (2861a0e's arc was verified in its own
  cycle).
- rubric-shots.mjs exists but is BLOCKED this session: the browseros
  CDP port (9171) accepts HTTP but the playwright-core websocket
  handshake times out against this browseros build. My ad-hoc
  browseros screenshots (887px, one width) do NOT satisfy the
  skill's 1440/768/375 width requirement - the width-controlled
  half is covered by the green probe passes (which render all 27
  routes at 1440/768/375/360 in both themes), and the
  image-judgement half needs a session with image viewing AND a
  working CDP handshake.

Stage Summary:
- Committed a747fdc: 401-by-URL gate tracking.
- The honest state of the realtime path: create half verified
  end-to-end (post_created live); publish pipeline verified
  end-to-end to its failure leg (post_failed live); the success leg
  (post_published) is environment-blocked - no seeded channel
  authenticates.
- Calendar drag e2e: PASS (first run this cycle).
- DMs wrapper: verified live. Stats modal refresh offer: correctly
  conditional, verified in code.

Known issues: rubric-shots.mjs CDP websocket handshake times out
against this browseros build (HTTP /json/version answers); the
image rubric remains BLOCKED for want of image viewing and the CDP
bridge.

---
Task ID: v27-red-team-3
Agent: omp (glm-5.3)
Task: Red-team iteration 3 - hunt visual defects in states the static gates never render, fix, re-verify.

Work Log:
- make audit (all 11 gate passes): 0 FAIL everywhere; remaining WARNs are dense calendar chips at 24px (WCAG 2.5.8 compliant, advisory).
- Built frontend/scripts/state-hunt.mjs: drives 14 interactive states (composer + Schedule/AI/Preview/Tags/channel panels, post detail, kanban card, calendar day popover, media upload, search results, feed comments, channels connect, automation new-rule, webhook-new, signature-new) at 1440 and 375, measuring doc overflow, sub-24px targets, composited-background contrast, clipped text. Exemptions mirror the repo's gates (skip link, aria-hidden, truncate, 24px+ label/wrapper target).
- Three false-instrument bugs found and fixed during the hunt (all documented in-session): a transparent-color regex miss (browseros probe), a channel-alpha typo ((1-l.g) for (1-l.a)) producing negative luminances and impossible ratios, and an opacity-0 hover-reveal label wrongly treated as hidden. Zero app colors changed on bad data.
- Two real defects found and fixed:
  - MediaUpload.svelte: upload label 199x20 (min-h-11 was on the hidden input) -> moved to the visible label.
  - CommentsThread.svelte: comments-panel Hide close button 26x16 -> min-h-11 min-w-11 inline-flex.
- Verified: pnpm build + cargo build --release, restarted the hand-started 6544 instance (make deploy is VPS-only; local is host-target binary), state-hunt 0 findings both widths, svelte-check 0/0, tailwind-class + hex-token gates pass, make audit exit 0.

Stage Summary:
- Commit 900ab2c pushed to github/master.
- New permanent gate: state-hunt.mjs (usable standalone; candidate to wire into make audit).
- All gates green on the rebuilt binary.

---
Task ID: v27-red-team-3b
Agent: omp (glm-5.3)
Task: Red-team iteration 3b - harden the gate suite per the review of red-team 3 (900ab2c).

Work Log:
- Extracted the ONE contrast implementation into frontend/scripts/contrast-core.mjs (AGENTS.md 9.18); probe.mjs, audit-contrast.mjs and state-hunt.mjs all import it. The core is the probe's compositing (wash layers over the nearest opaque ancestor + opacity-group handling). Each script keeps only its report policy. Assembly via new Function: the audit functions serialize into the page (no module scope there) and ESM strict mode would scope a direct eval's consts to the eval itself. Two traps hit and fixed while wiring: a literal \\n in the Function assembly, and state-hunt's body calling effBg (old private name) against the core's effectiveBg.
- Wired state-hunt.mjs into make audit (the 14 driven states were ungated).
- Added the BLOCKED login guard to every gate script (state-hunt, audit-dialogs, audit-targets, audit-contrast, audit-analytics, e2e-calendar-drag): waitForSelector on the client-rendered password field, exit 3 instead of a 30s fill() timeout.
- audit-dialogs: empty-list SKIP for row-scoped triggers (the /comments reply modal); a fresh install has no comment to open (AGENTS.md 9.38 precedent, probe's unreachable-route).
- Diagnosed a suite-wide 403: the hand-started 6544 instance had been relaunched without APP_URL/FRONTEND_URL, so CSRF reverted to port 6543. Relaunch now pins both to 6544.
- Re-seeded the demo DB (backup at /tmp/switchpost.db.bak-214031) after a feed refresh emptied the comments list and left the posting-volume series non-dense.

Known issues / follow-ups:
- The 51 controls in the 24-44 warning band (audit-targets) are reported, not gating - the dense calendar grid is the bulk; per WCAG 2.5.8 they pass.
- CRAFT rubric scoring stays BLOCKED in this environment: the model has no image vision (read of a PNG returns "Current model does not support images"); rubric-shots.mjs captures for a vision-capable session.

Stage Summary:
- Commit b3bfc58 pushed to github/master.
- make audit exit 0 with all 11 gates (probe, targets, contrast dark+light, dialogs, analytics, calendar-drag, 4 calendar states, logged-out login, state-hunt).
- The contrast rule is now written once; a fourth copy would be a build-time-visible defect, not a silent drift.

---
Task ID: v28-calendar-stubs-cli
Agent: omp (aroute/glm-5.3)
Task: Red-team the web dashboard visually (browseros-neo + website-design skill), fix the calendar (scrollable multi-post grids, week default, postiz look), replace seed/stub accounts with real browser cookies, and build the CLI login surface for AI agents.

Work Log:
- Calendar (c7797ec): WeekView/DayView hour grids scroll INSIDE a viewport-height card (.calendar-view-shell, calc(100dvh-168px)); sticky weekday header; auto-scroll to first slot with a post; MonthView cells render ALL posts with per-cell scroll stacks; sticky weekday row; removed '+N more' cap.
- Calendar (bd2dd03): week is the DEFAULT view at every breakpoint (mobile day-fallback removed); emoji removed from view switch + Generate Posts.
- Credentials (278e742): is_credential_row treats seed tokens as non-credentials, so 'connect' can replace seed_demo.py rows; connect x/reddit purge the seed row on success; Reddit gains REDDIT_COOKIE_TOKEN env fallback (config.reddit_cookie_token); delete_seed_integrations query + .sqlx cache regen (sqlite3-applied migrations; 'cargo sqlx run' prints help and does nothing).
- Operator DB surgery: X -> real _ishanparihar cookies (env), Reddit -> real d3vilzwrld session (env); 4 stub rows (bluesky/linkedin/threads/mastodon) + 50 seeded posts + empty analytics purged per operator decision (backup at /tmp/switchpost-backup-*.db). Killed the leftover /tmp/supervise-seed.sh supervisor that kept respawning the server against /tmp/sf-seed DB; server now runs on the repo DB.
- CLI (fec5314): connect --wait [--timeout-secs N] — blocking agent login loop; emits auth_url/OTT code then polls every 2s until the real credential row lands. docs/platforms-and-cli.md documents the three provider shapes and the loop.

Stage Summary:
- Full make audit on the clean DB: FAIL 0 everywhere (probe 216 visits, targets, contrast x2 themes, dialogs, analytics 0 failures, e2e drag 0 failures, state-hunt, Month/List/Day/login probes). Only pre-existing 44px tap-target WARNs on filter chips.
- Live proofs: connect x -> _ishanparihar; connect reddit -> d3vilzwrld; x timeline 113 entries; posts publish authenticated to X's API (X answered quota-344, a real-account-only response). connect linkedin --wait detected a credential landing at 8s end-to-end.
- Known issues: X account is at its daily tweet limit (publish auth proven, live tweet deferred); LinkedIn OAuth app redirect_uri registered for a different port (6543 vs 6544) — fix in the LinkedIn developer console or run on the registered port; reddit browse hits Reddit's www-edge bot policy (User-Agent policy, credential itself valid).

---
Task ID: N-1-project-rename
Agent: jcode
Task: Red-team the four candidate project names (sophp/sofo, sogency/sociagency, pogency, frogso)
against active competitors; propose the optimal name; blueprint the README/docs restructure for
maximum go-to-market impact. Applied the mental-model-atlas frameworks to the decision.

Frameworks applied (selection protocol, layer-to-family map, no playbook matched):
- time-and-dynamics/**Path dependence** — what does the past lock in, and which locks are worth breaking.
- strategy-and-organization/**Strategy kernel** — diagnosis, one guiding policy with an exclusion, coherent actions.
- language-and-measure/**Prototype theory** + **conceptual blending** — categorization by central example.
Reconcile rule: path-dependence dominance. It decides *when* the move is cheap (now, at 0 stars);
strategy and language decide *which* move inside that window.

Measurements (all re-runnable; see the decision doc):
- gitroomhq/postiz-app = **37,594 stars**, repo description literally "the ultimate agentic social
  media scheduling tool". It has shipped an MCP server + CLI + ClawHub skill. postmcp.dev is a
  second MCP-native entrant. Our headline differentiator (CLI+REST+MCP) is now table stakes.
- THIS REPO = **0 stars, 0 forks, 0 subscribers**. The rename lock is at its cheapest.
- 0 of 389 MCP tool names carry the project prefix (all provider-scoped `x_*`, `posts_*`, ...),
  so a rename cannot break a single agent integration. 0 occurrences in migrations/.
- Literal-name blast radius: 239 Rust (21 files), 28 frontend, 413 skills/, 119 docs/, 43 README,
  33 AGENTS.md, 81 scripts/systemd/Makefile, 26 .env/config defaults.
- Exact-name GitHub collisions: switchpost 6, herald 53, towncrier 38, dispatch 37, outpost 62,
  kestrel 55, imprint 71, pigeonpost 10, majordomo 3, ringmaster 4, signalbox 70, troupe 6.
- switchpost: 1 exact repo at 1 star, PyPI free, {com,dev,io}.com all unregistered.
- forgeway: 3 exact repos top 1 star, PyPI free, all three domains unregistered.
- UNKNOWN (recorded, not assumed): crates.io returned HTTP 403 on every query from this
  environment; USPTO TESS needs a key. npm returned 200 (taken) for every dictionary word.

Decision doc written: **docs/planning/DECISION_RENAME_2026-10-23.md** — the red-team of all four
candidates, the scored field, the two finalists, the falsifier, the priced rename ledger, the
execution sequence, and the README restructure blueprint keyed to four user archetypes.

Stage Summary:
- All four of the user's candidates are rejected, each for a distinct structural reason
  (post* warzone / social* SaaS saturation / -ency agency-stem anti-team / meaningless scramble).
- Recommended **switchpost**; alternative **forgeway**; method given to generate `switchbay`,
  `relaypost`, `carrierpost` as backups.
- NOTHING has been renamed. This is a decision doc awaiting the user's explicit go-ahead (the
  AGENTS.md DECISIONS precedent). No code, config, or doc has been modified.

---
Task ID: N-1-execute-rename
Agent: jcode
Task: Execute the approved rename to `switchpost`, directly, with no compatibility shim.
Follows DECISION_RENAME_2026-10-23.md; the user approved the plan (2026-10-23) and
explicitly declined the legacy-path shim ("very alpha project").

Renamed in four pushed commits (e2c0fd1 plan, 4c6a6eb code+tests+skill, 54ecd28
docs+ops, f757a5e fence fix):
- Cargo package/bin/keywords/description, the `social_forge` crate identifier,
  SocialForgeMcpServer, clap #[command(name)], tracing target, UNIT_NAME,
  User-Agent strings, and every ~/.social-forge/ + data/social-forge.db path in
  src/config.rs. No fallback path, per instruction.
- SOCIAL_FORGE_TOOLSETS and SOCIAL_FORGE_DIR both -> SWITCHPOST_* (two
  operator-facing env vars; the second would have silently broken the systemd unit).
- frontend strings + localStorage keys; skills/social-forge-agent ->
  skills/switchpost-agent (git mv); tests/ 68 occurrences.
- README restructured per §10 archetypes; AGENTS.md, docs/, PAGE-INDEX row;
  scripts/social-forge-start.sh + social-forge.service -> switchpost.* (git mv);
  Dockerfile(.build), Makefile, CI workflows artifact names + release binary
  paths, genome.toml, workspace-lint.yaml, .env.example.

Four defects found and fixed by the rename itself, none of which a read would catch:
1. `tests/doc_cli_commands_test.rs::trim_shell` modelled `&&` but not a bare `&`, so a
   legitimately backgrounded doc example reached the CLI parser as `... --json &` and
   failed. Fixed the parser, not the doc.
2. `migrations/0001_init.sql` carries the old name in a header comment. Editing it made
   every test boot fail ("migration 1 was previously applied but has been modified") —
   sqlx checksums migration files. Reverted byte-for-byte; the comment is a permanent
   fossil, now recorded in §9.1. Corrects the earlier "0 occurrences in migrations/"
   measurement.
3. The agent-path block I added to the README opened a ```json fence and never closed
   it. 17 fence lines instead of 18: docs_lint stopped seeing the real headings and
   reported the routing table's own (correct) links as dead, while the bash `#` comments
   inside fences were parsed as document headings. Gate went 3 hard failures -> 0.
   It would also have broken rendered Markdown on GitHub.
4. The gitignored `.env` carried `DATABASE_URL=sqlite://data/social-forge.db`, which
   outranks the compiled default, so the correctly-renamed server still opened the old
   database and wrote to both files. Fixed one line and restarted; confirmed by the old
   file's mtime going still. A rename sweep over TRACKED files renames the source, not
   the deployment.

Data migration (executed, §9.2): SIGTERM to the live server (it held writer.lock),
`PRAGMA wal_checkpoint(TRUNCATE)` folding the 2.1MB WAL into the main file, a COPY of
the trio to switchpost.* rather than a move, restart, verify (health 200, unauth
providers 401, row counts identical: 2 posts / 3 integrations / 1 user / 2 media,
integrity ok), and only then the old files retired into
data/archive/switched-from-social-forge-2026-10-23/ with ~/.social-forge -> ~/.switchpost
and a dated .env backup beside each.

Also removed: tests/test_api.py and tests/test_api.sh — orphaned since the SQLite
migration (0 references anywhere in the repo, no CI/Makefile/Cargo wiring, targeted
localhost:3000 and a `PGPASSWORD=social-forge psql` fixture for the dropped Postgres
path, one with a typo'd binary path `./target/releassocial-forge`).

Final state: `git ls-files | xargs grep -i 'social.forge\|socialforge'` returns only
the three intended fossils plus two [REDACTED:email] false positives: the historical
decision doc (correctly keeps the old name), its PAGE-INDEX row, migrations/0001_init.sql
(sqlx-checksummed), plus `.sqlx/` and `seed_demo.py` (redacted emails).

Verification:
- cargo check --lib --bin switchpost: 0 errors
- cargo test --lib: 496/496 pass
- cargo test --test uuid_column_decode_test: 3/3; --test doc_cli_commands_test: 3/3
- pnpm build: succeeds; svelte-check: 0 errors 0 warnings
- docs_lint --root .: 0 hard failures, 0 advisories (and the live pre-commit
  docs-check reports ok on all four checks)
- Release binary built and running: `switchpost 0.3.0` on 127.0.0.1:6544 against
  data/switchpost.db; the audit scripts' localStorage keys now match the app's own
  (switchpost-theme / switchpost-onboarded), so the visual gates still drive the app.

Not done (operator's to do, named in §9 step 1): claim switchpost.com/.dev and rename
the GitHub repo; crates.io was HTTP 403 throughout (UNKNOWN, not free).

---
Task ID: N-1-post-rename-audit
Agent: jcode
Task: Re-verify the executed rename against the evidence; correct anything stale or
overstated; continue the work.

Verification of state (all confirmed by measurement, not memory):
- git local HEAD == github/master == af53a7e, 0 ahead / 0 behind. Nothing uncommitted.
- ./target/release/switchpost 0.3.0 running on 127.0.0.1:6544 against
  data/switchpost.db. Health 200, unauth /api/providers 401. integrity ok, 3 integrations.
- ~/.switchpost/.env live, ~/.social-forge retired. .env (gitignored) on data/switchpost.db.
- cargo test --lib 496/496; docs_lint 0 hard / 0 advisory.

Corrections made to overstated/stale claims:
- The decision doc's status key still said "execution is in progress" and §9 still said
  "(deliberately not applied)" in spirit. Both now read EXECUTED, with the commit list.
- The intro said "the rename is executing"; now records the five commits.

Two NEW defects found in the LOCAL INSTALL (which no git ls-files sweep can see, §9.3):
1. `make install-skill` copied four reference docs while the skill ships five.
   `quick-reference.md` was never installed, yet SKILL.md and docs/PAGE-INDEX.md both
   point at it. Pre-dates the rename (the Oct 21 install had the same gap); the rename
   just forced the target to be re-run and re-read. Fixed by copying `references/*.md`
   rather than a hardcoded list.
2. A stale `~/.agents/skills/social-forge-agent` (Oct 21, 143 bytes smaller, missing the
   same reference). Archived to ~/.agents/skills-archive-switchpost-2026-10-23/ and the
   live install re-run, so ~/.agents/skills/ holds exactly one skill dir.

New test tests/skill_install_test.rs (2 tests), and both MUTATION-VERIFIED rather than
merely passing:
- Mutation 1: restored the hardcoded 4-file list -> install_skill_copies_every_reference
  FAILED naming exactly ["quick-reference.md"].
- Mutation 2: renamed a link in SKILL.md -> every_reference_doc_links_to_exists FAILED.
- Both restored -> 2/2 pass. A test that has never been seen to fail proves nothing.

Also fixed three bugs in my own test before it could be believed: recipe lines are
`@`-silenced so `strip_prefix("cp ")` never matched; the join needed `references/`
re-attached after strip_prefix; and clippy denies `&PathBuf` for a parameter.

Left for the operator (unchanged, named in §9 step 1): claim switchpost.com/.dev and
rename the GitHub repo ishan-parihar/social-forge -> switchpost; crates.io returned 403
throughout, so crate-name availability remains UNKNOWN rather than free.

---
Task ID: rename-verify-2026-10-23-b
Agent: Jcode
Task: Verify the rename landed correctly; correct any claim that outruns its evidence.

Work Log:
1. Adversarial mutation testing of the new `tests/skill_install_test.rs`, taken to six
   cases rather than the two already recorded. One finding: the test read only the
   SOURCE side of every `cp`, so a target that installed the right files into the
   WRONG place still passed. Mutation 4 (`$(SKILL_DEST)/refernces/`) and mutation 5
   (a flat `$(SKILL_DEST)/`) both passed on the old test. Added a destination
   assertion that the final path element of every references copy is `references`.
   All six now fail the test, and the restored baseline passes 2/2.
2. Also recorded near-misses, because they are the reason to distrust a green run:
   `cargo test every_reference -- --exact` matched NOTHING and so reported
   "ok. 0 passed" for two mutations that should have failed - a vacuous pass, not a
   signal. And an earlier "coupling" between the two tests was a substring-filter
   artifact of my own reading, not a real dependency: run by exact name they are
   independent and both green.
3. Corrected two overstated claims in `docs/planning/DECISION_RENAME_2026-10-23.md`
   that the evidence does not support:
   - "migrations: 0 occurrences of the literal name" cannot be true. The name is in
     one migration comment and sqlx checksums migration files, so removing it fails
     every boot. The row now says that, which is the same conclusion trying it reached.
   - "com/dev/io all unregistered" was unmeasured. It has been replaced by real
     measurements, which reverse it.
4. Ran the availability checks that the earlier pass got blocked on, with controls:
   - crates.io `/api/v1/crates/switchpost` -> **HTTP 404 = the name is FREE**. The
     403s were Cloudflare throttling this client, not a rate limit on the name.
     crates.io `/switch` -> **200**, confirming the rejected alternative's cost.
   - `switchpost.com` is **TAKEN**: an RDAP record from Verisign
     (`1648085867_DOMAIN_COM-VRSN`), registered 2011-03-29, expires 2027-03-29,
     status `client transfer prohibited`. `.dev` and `.io` both resolve to live IPs.
   - So the earlier "all free" verdict was wrong by three domains, and the crate
     verdict was wrong the other way (recorded as UNKNOWN when it was actually free).
5. Wrote down WHY the first pass was wrong, because that is the reusable part. It
   used `dig`, and its own control failed: `dig +short NS github.com` returned
   nothing because `dig` is not installed and the sandbox swallowed the error. A
   probe whose control does not pass measures nothing, and a name it calls available
   may be registered. `switchpost.com` is the direct example.
6. Updated every downstream claim to match: the name-scoring row, the migration-lock
   row, the two falsifiers (both struck through as RESOLVED), the "cheap to own"
   bullet, the open-features list, the crates.io guard, and the closing summary.

Stage Summary:
- Artifact: `0c2e889` on github/master - "harden the skill-install test (six-mutation
  sweep) and correct two overstated claims in the rename decision doc". The domain
  and crates.io corrections in this entry are NOT yet committed.
- Availability is now measured, not assumed: `switchpost.com/.dev/.io` are all taken
  (~0 of 3 obtainable); the crates.io name `switchpost` is free.
- No shipping link uses a `switchpost.com` address - only the decision doc and this
  worklog, which is provenance and correct. The README, systemd unit, CLI and docs
  all use `github.com/ishan-parihar/switchpost`, so nothing points at a stranger's
  site.
- Known defect: the repo `ishan-parihar/social-forge` is still named `social-forge`,
  so every README badge URL 404s until the user renames it. `gh` is authenticated
  here with `repo`/`workflow` scopes, so the rename is available but not authorised.
- Method note carried forward: an availability claim is worthless without its
  control, and UNKNOWN is the honest value for a probe that returned an error.

Feasibility of the one remaining operator action, measured (not assumed) 2026-10-23:
  gh repo view ishan-parihar/social-forge  -> PUBLIC, isFork=false, stars=0, forks=0
  gh repo view ishan-parihar/switchpost    -> does not exist (name is free)
  gh auth status                            -> account ishan-parihar, scopes repo+workflow
So the rename is available to this environment and costs nothing to reverse (GitHub
permanently redirects the old owner/name path). It was NOT executed: renaming a
public repo changes the user's online identity and outbound URL, which is the user's
call, not an agent's.

One-command handoff for the user, when ready:
  gh repo rename switchpost --repo ishan-parihar/social-forge --yes

That completes the rename end to end. Every badge, image and CI badge URL in the
README already points at ishan-parihar/switchpost, so they resolve the moment the
rename lands; until then they 404, and that is the ONLY outstanding breakage.

---
Task ID: deeper-validation-2026-10-23
Agent: Jcode
Task: Validate the rename work further, without trusting a green run.

Work Log:
1. Ran the FULL suite (`cargo test`), not just `--lib`, and it found a real
   defect that `--lib` cannot see: `tests/streak_unification_test.rs` failed,
   "three consecutive days is a 3-day streak for """ with left=5, right=3.
   Deterministic across 3 runs. A run of `--lib` alone (496 green) had reported
   this project healthy, which is the point of the extra pass.

2. Root cause was NOT the streak logic. It was the test's own isolation: the
   scratch DB is per-BINARY, not per-RUN, so it accumulates. Measured:
   `sqlite3 target/test-data/streak_unification_test.db` held published posts on
   5 distinct UTC days (Oct 19, 20, 21, 22, 23) - each earlier run seeded
   "0/1/2 days ago" and those rows stayed. Today's 3 seeds read back against
   5 days. Both endpoints agreed (5 and 5), so the bug was in the fixture, not
   the measure. Fixed by clearing this file's published rows before seeding.
   The operator's real DB was checked too (0 published posts) to rule out the
   shared-file failure mode AGENTS.md 9.42 warns about - this one is a different
   shape: correct isolation per binary, no isolation per run.

3. Mutation-verified the fix in both directions, so it can be believed:
   - Mutation A: seed 4 days instead of 3 -> FAILED, left: 4, right: 3. The
     test still measures what it claims.
   - Mutation B: break the increment (add then subtract) -> FAILED, left: 1,
     right: 3. The test pins the streak loop itself.
   - Baseline restored: 3/3 pass, twice consecutively (no dependence on
     accumulated rows now).

4. Ran `make install-skill` for real and inspected the result on disk, since the
   test only reads the Makefile's SOURCE side:
   - SKILL.md + all 4 reference docs installed, including quick-reference.md
     (the one the old hardcoded list silently dropped).
   - All 5 files byte-identical to the repo source (cmp).
   - All 4 links SKILL.md promises resolve on disk, and nothing references a
     file that is not installed.

5. PAGE-INDEX check: DECISION_RENAME_2026-10-23.md has a LIVE row
   (docs/PAGE-INDEX.md:23). Two false alarms of my own, worth recording as
   method: the first grep pattern `](filename)` did not match rows that carry a
   directory prefix, so it reported 16 "missing" rows that were present; and the
   4 archived OUTLINEs looked like broken links ("tombstone says archive/, file
   is in planning/") when in fact BOTH exist by policy - the real file under
   docs/archive/ and a redirecting tombstone at the old path. Neither was a
   defect.

6. Live server validated, and it took several tries to get the measurement right:
   - The old server (PID 2078588) had exited during the session; nothing was
     listening on 6544. Restarted `./target/release/switchpost serve --port 6544`.
   - `http://127.0.0.1:6544/health` returned 000 ("connection refused") and this
     was NOT a proxy and NOT a port clash - it is HTTPS only. Over TLS:
     /health 200 {"status":"ok","version":"0.3.0"}, /ready 200
     {"status":"ok","database":true}, /api/posts 401 unauthenticated,
     /api/events 401 unauthenticated. DB trio intact: integrity ok, 2 posts,
     3 integrations, 1 user.
   - THE TITLE GAVE IT AWAY: `/` served <title>Social Forge</title>. The source
     app.html said `Switchpost`, so this was a STALE BUILD - frontend/build/ was
     from an old session and the release binary embeds it via rust-embed. A
     rename that is "complete" in the tree can still ship the old name at
     runtime, which is why the served page and not the source is the check.

7. Second stale-artifact find in the same family: RedditPreview.svelte had a
   hardcoded demo subreddit `r/socialforge` (next to fake 1.2k votes and "2h"),
   which frontend/src had otherwise forgotten. Changed to r/switchpost and
   rebuilt; the old string was still present in a lazily-loaded chunk
   (`_app/immutable/nodes/0.CQ_5cgVY.js`) until a clean rebuild, which is the
   same lesson again: the chunk, not the source, is what the browser runs.

Stage Summary:
- Fixed 1 real defect (the streak fixture's accumulating scratch DB) with the
  mutation evidence above, plus 1 stale-brand bug in the served UI (title) and
  1 in a preview mockup (r/socialforge).
- Full suite and frontend must both be re-run green AFTER these edits before
  this is claimed as done; the results of that run are the last thing in this
  file if they exist, and their absence means it is not yet done.
- Method notes carried forward: `--lib` green is not a green suite; a scratch DB
  keyed per binary is not isolated per run; the served page beats the source;
  and a grep that reports many hits is worth re-checking before believing any
  of them.

---
Task ID: validation-closure-2026-10-23
Agent: Jcode
Task: Close the validation pass. Every gate, then the served page.

Work Log:
1. Full suite, second time, after the four fixes: 854 passed, 0 failed, EXIT=0,
   across 50 test binaries. surface_docs_audit 4/4 EXIT=0. svelte-check 0
   errors, 0 warnings, EXIT=0.

2. Rebuilt the release binary and restarted the server, then measured the
   SERVED page rather than the source, because that is the check that was
   missing:
   - `curl -k https://127.0.0.1:6544/` -> <title>Switchpost</title>
   - 0 occurrences of "Social Forge" in the served HTML
   - the served preview chunk (0.7lAk4s3p.js) carries r/switchpost once and
     socialforge zero times, counted directly rather than through a shell
     conditional that inverted
   - /health 200, /ready 200 with database:true, /api/posts 401

3. Committed as 101b7f3 and pushed to github/master.

A note on my own measurement errors in this pass, because each one nearly
produced a false conclusion:
- I reported the installed skill as DRIFTED when the compared path was a
  literal unexpanded `references/*.md`, not a file.
- I reported 16 planning docs as missing their PAGE-INDEX row on a grep
  pattern that could not match rows carrying a directory prefix.
- I reported `r/socialforge` as still live in the served chunk, from a
  `grep && echo STALE || echo clean` where the first grep had already
  succeeded, so the failure branch printed. Counted directly: zero.
Each was corrected by re-measuring, and each correction is in the worklog
rather than quietly dropped.

Stage Summary:
- 4 defects fixed (1 test fixture, 2 doc/count strings, 1 over-length tool
  description) plus the 2 rename-at-runtime misses (served title, demo
  subreddit). All 6 verified closed on the gates that exposed them.
- Pitfalls 9.52 and 9.53 added to AGENTS.md with the evidence and the commands.
- Committed 101b7f3, pushed; local and remote match; tree clean.
- Still the operator's: `gh repo rename switchpost --repo
  ishan-parihar/social-forge --yes` (measured available, deliberately not run
  by me), after which the README badges resolve.

---
Task ID: cli-completeness
Agent: omp
Task: Complete the switchpost CLI as an agentic utility — browser-driven OAuth,
  Deckmill recursive auto-posting, and full CLI↔MCP parity — with exhaustive tests.

Work Log:
1. New CLI surfaces, all reusing the existing implementations rather than
   re-implementing them:
   - `connect --browser` drives the OAuth consent page through the browseros-neo
     MCP server and lets the switchpost server own the code exchange (memory
     #398). The MCP client is a minimal implementation over the existing
     reqwest 0.12 (memory #401): rmcp's HTTP transport would pull a second
     reqwest (0.13).
   - `autopost` renders a Deckmill carousel, uploads every PNG through the
     dashboard's own media path and calls switchpost's own staging service.
     Recurrence (`--repeat-days`/`--repeat-until`) reuses the DB-backed repeat
     series the scheduler already publishes (memory #409) — deliberately NOT a
     custom loop process, which would double-post against the scheduler.
   - `agent` prints the discovery card an agent needs.
   - `rss`, `campaigns`, `signatures`, AI and `streak` close the measured
     CLI↔MCP gap (390 MCP tools vs 300 CLI commands; 15 tools in 4 families
     had no CLI equivalent). Every wrapper calls the MCP handler, so there is
     one implementation per operation.

2. Two real defects found and fixed while proving the paths:
   - The staging readiness check tested the RAW relative media URL
     (`/api/media/<uuid>`) while the publish path resolves it against APP_URL
     first, so a post that would have published was rejected as unready.
     Readiness now sees exactly what the provider would see. Fixed in the
     shared service (src/services/staging.rs), so the dashboard composer and
     the MCP stage path are fixed with it.
   - `rss add` took `--enabled <true|false>`, which no agent can guess right
     and which reads as a value rather than a state. It is now `--disabled`,
     a flag whose absence means the default.

3. The exhaustive-test pass found three defects in the TEST and DOC layers:
   - `tests/cli_agent_surface_test.rs` asserted that a bare URL string decodes
     to an empty vec. It does not: the decoder is strict and ERRORS. The
     test now pins the loud failure, which is the better contract.
   - The doc-to-CLI guard caught three documented commands the CLI rejects:
     two shell line-continuations and `rss add ... --integration <UUID>`
     (the id is positional). Fixed in the docs.
   - `docs/platforms-and-cli.md` stated 56 CLI commands; the true count is
     now 63. Fixed.

4. `tests/streak_unification_test.rs` was TIME-DEPENDENT and failed at
   00:35 UTC. The seed anchored "today" at MIDDAY UTC, which is in the
   FUTURE before 12:00 UTC; in New York the seed landed on a day the zone
   had not reached yet, and `calculate_streak` only accepts a streak that
   starts on today or yesterday, so it measured 0. Verified INNOCENT of my
   diff first: a clean HEAD worktree failed identically (left: 0, right: 3).
   The seed now anchors on the most recent PAST UTC midnight, so every
   seeded day is today-or-yesterday in every queried zone at every hour.

Stage Summary:
- `connect --browser`, `autopost`, `agent`, `rss`, `campaigns`, `signatures`,
  AI and `streak` all work against the live binary and the live database.
- Autopost proven end to end against a live connected account: 3 slides
  rendered -> uploaded -> staged on X; the recurring variant produced 1 post
  + 3 spaced copies.
- Gates: cargo check 0 errors; cargo test 520 lib tests + every integration
  binary green; pnpm build succeeded; svelte-check 0 errors, 0 warnings.
- Deckmill facts corrected against the renderer itself: `render-carousel`
  requires --url, --hashtags AND --primary-color; hero uses `headline` (not
  `heading`); `cta` and `feature` are unavailable; `text_block` is valid.

---
Task ID: csrf-port-drift-guard
Agent: jcode
Task: Post-reload verification of the running deployment; fix the FRONTEND_URL
  vs serve-port CSRF drift that the reload re-introduced.

Work Log:
1. Verified the config migration state: ~/.switchpost/.env live, old dirs
   archived intact (row counts identical in the archived DB), 389 MCP tools
   counted from the SHIPPED BINARY (mcp-tools), live DB integrity ok.
2. Chased a suspected posts_list bug (total: 0 against a DB with rows) to
   ground: every row was soft-deleted (deleted_at set), so the
   `deleted_at IS NULL` filter was correct. Triple-interface parity
   confirmed (CLI/MCP/REST all agreed at every step). NOT a bug.
3. Found the real defect while testing REST: login POST with a browser
   Origin https://localhost:6544 -> 403 csrf_origin_mismatch, because a
   harness reload relaunched serve on 6544 while .env still pinned
   APP_URL/FRONTEND_URL to 6543. Every browser state-changing route was
   dead — the third occurrence of this drift (worklog 2026-10-22, 2026-10-24).
4. Fixes:
   - .env (gitignored, local): APP_URL/FRONTEND_URL pinned to 6544 with a
     comment naming the trap. Restarted serve; browser-origin login now 200,
     stale-origin login correctly still 403, authenticated reads 200.
   - config::frontend_origins_cover_port: pure predicate, unit-tested for
     the exact drift, comma-list entries, scheme-default (443) semantics,
     and unparseable entries. Mutation run: removing the scheme-default
     clause made port_coverage_scheme_default_only_covers_443 fail, as designed.
   - main.rs: boot warning naming the actual port and the one-line fix;
     proven live on a scratch 6545 instance with a stale FRONTEND_URL.
     Thin-MCP and CLI modes exit before the block; only serve warns.
5. Landed the interrupted session's verified-but-uncommitted CLI-completeness
   work unchanged (commit v0.4.2), then this fix (v0.4.3).

Stage Summary:
- Gates: cargo check 0 errors; cargo test --lib 520/520 (5 new); integration
  cli_agent_surface 7/7, streak_unification 3/3; pnpm build ok; svelte-check
  0/0; release binary rebuilt and serving on 6544 with no CSRF warning.
- Both commits pushed to github/master (3795bcb..473d1e1), remote ref verified.
- Known leftovers for the operator: the repo DIRECTORY is still named
  social-forge on disk while the product is Switchpost (the GitHub repo is
  still ishan-parihar/social-forge too); the running serve is a plain
  nohup process, not a systemd unit, so a machine reboot needs a manual
  relaunch with --port 6544.

---
Task ID: cli-browser-oauth-verification
Agent: omp
Task: Prove the `connect --browser` success path and harden the OAuth driver against the live browseros-neo tab format.

Work Log:
1. Ran `connect linkedin --browser` against the LIVE browseros-neo server. The
   driver opened the consent page, printed progress to stderr (stdout stayed
   clean for JSON), timed out with the tab LEFT OPEN, and named the page plus
   the recovery command. Correct behaviour.
2. Inspected the live `tabs list` output and found the real shape has THREE
   sections - "Your tabs:", "User's tabs:", "Other agents' tabs:" - and the
   driver's OWN tab is reported under "Other agents' tabs:" with an
   "owned by switchpost-cli" annotation. The old parser stopped at that
   header and only worked because the live server also MIRRORED the tab into
   "User's tabs:". The mirror is not a contract.
3. Root-cause fix: the section boundary was the WRONG guard. The minted OAuth
   `state` is unique per login, so it is what isolates one agent's callback
   from another's. `parse_tab_urls` now reads every section, and
   `find_matching_callback` still requires BOTH the callback prefix and the
   exact minted state.
4. Wrote `tests/browser_oauth_e2e_test.rs`: a mock browseros-neo that
   reproduces the live wire format (SSE framing, session-id header, text
   listing, section headers, "owned by" annotation), driven by the REAL MCP
   client and the REAL `wait_for_redirect`.
5. Negative control: mutated `parse_tab_urls` back to the old
   "stop at Other agents' tabs:" behaviour and BOTH tests failed with the
   exact real symptom ("No OAuth callback within 10s"). Restored the fix via
   an aft_safety checkpoint.
6. Corrected `docs/platforms-and-cli.md`: it claimed the tab is always left
   open; the code closes it on success and leaves it open only on timeout.

Stage Summary:
- The driver's success path is now covered end to end against the live wire
  format, with a mutation-proven test.
- Live LinkedIn currently answers "Bummer, something went wrong." on the
  authorization request itself - provider-side (app scopes/redirect URI),
  BEFORE any redirect exists, so it is not a driver defect.
- All leftover test tabs (LinkedIn consent pages, the redirect probe) were
  closed; the browser is back to a single blank newtab.

---
Task ID: skill-red-team-rebuild
Agent: jcode
Task: Audit the switchpost-agent skill against the real binary, red-team it,
  and rebuild it so every agent workflow has a verified leaf.

Work Log:
1. Ground truth: dumped all 63 CLI commands' --help from the shipped binary
   and all 389 MCP tools (mcp-tools), plus every subcommand family. Verified
   each documented claim against live execution, not source reading.
2. Red-team findings (11, three critical):
   - F1 CRITICAL: SKILL.md + references + evals taught that `providers`
     -> internal_id is "the integration UUID you need for scheduling and
     posting". FALSE: internal_id is the PLATFORM account id (x=3301263462).
     `posts create --integrations <internal_id>` -> "At least one valid
     integration UUID required". The real source: `mcp-call integrations_list`
     -> id. Conversely `linkedin-page post <PAGE_ID>` takes the PLATFORM id.
     Two ID systems, one conflation — the skill's central ID table was wrong.
   - F2 CRITICAL: documented `mcp-call posts_create '{"content":...,
     "platforms":["x"]}'` — the real param is integration_id (singular,
     UUID-validated), NOT platforms. Live error: missing field integration_id.
   - F3 HIGH (was a code bug): mcp-call was served through the session-
     filtered router, so with default boot toolset every non-core tool
     answered "tool not found" (x_create_tweet reproduced live). Found the
     parallel session's fix in the uncommitted tree, live-verified it,
     landed it (v0.4.5) and rebuilt + redeployed the release binary.
   - F4 HIGH: coverage gaps — setup/connect-all/audit/service/config/init/
     split-preview, the native youtube family, x/linkedin/instagram DM
     families, reddit mod, facebook extras, gcal/gmail/gdrive full CRUD,
     autopost recurring flags, posts update/tags/validate/schedule.
   - F5: toolset docs said provider prefixes; real sets are google/github/
     linkedin/facebook/instagram/telegram/discord/mastodon/bluesky/youtube/
     skool/whatsapp/wc/webhooks/slack/x/reddit/th/tt/pi/wp/md/dv/hn/core.
   - F6: docs claimed auth_register/auth_login/auth_me MCP tools — only
     auth_status exists.
   - F7: providers.md said Facebook/Bluesky/Mastodon/WordPress/Dev.to have
     no native post command — all five do.
   - F8: evals embedded the same ID/syntax errors (eval 2, 4, 9, ...).
   - F9: pinterest profile takes <BOARD_ID> (correct but undocumented quirk).
   - F10: skill said "always use --pretty"; default TOON is more token-
     efficient for agents.
   - F11: `switchpost post "..."` creates a DRAFT (not published) without
     --schedule; the old "Verify: state published" advice failed. Live probe
     confirmed: state=draft, cleaned up.
3. Rebuild (skill-creator methodology):
   - SKILL.md 356 -> 170 lines: router-first, two-ID-systems table up top,
     core-routes table, platform posting table, correct unified-post
     semantics (draft vs queued vs immediate), toolset list, error recovery
     keyed to real error strings.
   - Four leaves, no content duplicated across them: platform-matrices.md
     (282 lines, every platform's read/engage/DM family), recipes.md (162,
     Instagram two-route publish, autopost, RSS, automation, campaigns),
     mcp-catalogue.md (249, all 389 tool names verified present by script),
     ops.md (71, connect/serve/service/config/init + the CSRF port trap).
   - Retired providers.md / quick-reference.md / recipes-extended.md (fully
     superseded). Fixed every doc pointer: binary --help text (cli/mod.rs),
     run.rs doc comment, AGENTS.md document map (deduped),
     docs/platforms-and-cli.md's own wrong `providers -> integration UUIDs`
     line.
   - Fixed evals.json: 43 evals re-anchored to the real commands (fb native
     create, --args on mcp-call lines, count not max_results, carousel real
     syntax).
   - Makefile install-skill now PRUNES stale leaves from the install dir
     (planted-file test: stale-test.md removed).
4. Validation:
   - doc-CLI guard: 336 documented commands, all parse under the real clap
     (it caught 5 real errors during the rebuild: instagram hashtag/dm
     arg counts, wordpress numeric id, automation update positional form,
     a broken header fragment — all fixed).
   - tool-name script: 375 documented MCP tool names, all present in the
     binary's 389 (the one "missing" was a parameter name, not a tool).
   - LIVE end-to-end: the corrected chain (integrations_list -> posts
     create --integrations -> delete) executed and cleaned up.
   - Gates: cargo check 0 errors; 521/521 lib tests; doc_cli 3/3,
     skill_install 2/2, mcp_call_reach 4/4, toolset_membership 4/4,
     mcp_session_toolset 1/1.

Stage Summary:
- The skill's #1 documented workflow (find ID -> create post) was broken;
  it now works live, and the doc-CLI guard + tool-name script keep every
  documented command and tool name honest against the binary.
- Token efficiency: SKILL.md 170 lines (~52% of before), references 764
  lines across 4 focused leaves (vs 817 across 5 with heavy duplication);
  no --pretty-by-default advice; TOON-first guidance.
- Known leftovers: the two-space gap between `linkedin-page` and its
  platform page-id semantics is a provider quirk worth a code fix someday
  (list returns id+name only); github/google have no post-publish surface
  (publish_capable=false) and the skill says so via providers' flags.

---
Task ID: skill-red-team-pass-2 (v0.6.1)
Agent: Jcode (second-pass red-team audit)
Task: Red-team pass 2 on skills/switchpost-agent — attack semantic claims, output shapes, error contracts, and pass-1's own new claims; land verified parallel-session drops; gates; push.

Work Log:
- Live-mapped the CLI error contract: FOUR paths, not the documented one.
  clap usage -> clap text on stderr, exit 2 (posts list --bogus);
  runtime core -> {error,help?} TOON/JSON on STDOUT, exit 2
  (integrations disable not-a-uuid); runtime platform-shim -> {error,hint}
  on STDOUT, exit 2 (github repos <404 user>, via emit_result); anyhow
  propagation -> Error: <msg> on stderr, exit 1 (bluesky timeline, not
  connected). SKILL.md's "JSON on stderr with exit code 1" was wrong on
  every path; replaced with the verified table.
- Falsified live: ops.md's install URL. https://switchpost.dev/install.sh
  fails TLS (handshake alert 40); http:// falls back to a "Coming Soon"
  parking page. curl|sh of that is a broken instruction. Replaced with the
  repo's real raw.githubusercontent.com/ishan-parihar/social-forge path
  (live 200). Also: "config init" doesn't exist -> top-level "switchpost
  init".
- Falsified by official docs: SKILL.md's "X free tier is ~17 tweets/day".
  X API is pay-per-usage now ($0.015/post, no free monthly quota); and the
  app's default X path is cookie-GraphQL (account-level cap, not API tier).
  Rewrote the rate-limit row to name both real paths.
- Found the X empty-shape trap live: x user <bad> / x delete <bad-id>
  return exit 0 with an empty-shaped data object (id:"") — a missing
  tweet/user looks like success. Documented in platform-matrices.md.
  (First draft named a nonexistent "x tweet-detail" — caught against the
  binary's real subcommand list before landing.)
- Coverage diff (binary surface vs skill): 9 MCP tools missing from my own
  pass-1 catalogue (analytics_summary, feed_analytics, media_delete,
  posts_set_tags, posts_create_carousel, setup_config_set, setup_guide,
  setup_import_cookies, setup_status) — added with purposes captured from
  tools_search. 5 CLI actions undocumented (integrations enable/disable,
  posts get, tags get, webhooks get) — added to recipes.md/ops.md with
  live-captured output shapes. Catalogue's "core ~40 tools" was stale ->
  67, with the re-measure command.
- emit_result --json fix (uncommitted from a 3rd parallel session): verified
  root cause + fix correct (render_output routes all output through the
  global format), landed with its unit test; release binary rebuilt and
  re-verified live on all three paths (ok/data/error) for x + github.
- Landed 3 more verified parallel-session drops: mcp-call --json tests
  (tests/mcp_call_reach_test.rs, real binary on scratch DB),
  DECKMILL_BIN hint in autopost's PATH error (env var is genuinely read),
  home.rs 21->26 platforms (matches tier.rs), and the OUTPUT_FORMAT
  snapshot/restore in the shim format test.
- Gates: cargo check 0 errors; 521/521 lib; doc_cli 3/3 (caught one
  wording of my own that parsed as a command — fixed); skill_install 2/2;
  mcp_call_reach 6/6. make install-skill: installed copy synced 5/5.
- Probe hygiene: one throwaway draft post created (posts get shape probe)
  and deleted; no live tweets this pass (the pass-1 incident probe was
  deleted in the previous session).

Stage Summary:
- v0.6.1 pushed as 51a33f4. Every doc claim changed this pass was
  live-executed against the release binary (or falsified by an official
  source for external facts); every command newly documented had its
  output shape captured live.
- The error-contract rewrite is the highest-value fix: an agent following
  the old text would parse stderr for errors that actually live on
  stdout, and treat exit 2 as a usage error when it's a runtime failure.
- The guard worked as designed: it caught my own catalogue rewording
  ("switchpost signatures list/create/update/delete" parsed as a literal
  command) before it shipped.
- Known leftovers for a future pass: x user's empty-shape behavior is a
  candidate for a code fix (return a clear error when the GraphQL lookup
  resolves nothing) rather than a doc workaround; the two-space
  linkedin-page gap and the provider "coming soon" landing page are
  still open upstream of the docs.

---
Task ID: cli-surface-audit-v0.6.0
Agent: omp (glm-5.3)
Task: Audit the CLI/MCP agent surface after the 24h+ autonomous session; verify previous agents' fixes; exhaustively test the agent workflows (login/OAuth, deckmill autopost, posting lifecycle).

Work Log:
- Verified push state clean (master = github/master, no unpushed work); removed stray `download`/`sweep` example.com fetch artifacts; left `.serena/` untracked (tool config, NOT committed).
- Ran the gate battery: 521/521 lib tests; doc_cli 3/3, skill_install 4/4, mcp_call_reach 2/2→6/6, toolset_membership 1/1, mcp_session_toolset 4/4, cli_agent_surface 7/7; frontend build OK.
- Live-verified the previous session's claims: two-ID-systems chain (integrations_list → UUID → posts create --integrations), mcp-call reach for non-core tools under default toolset, deckmill contract match, connect --browser arms browseros-neo correctly, recurring series DB-backed.
- Fixed 3 defects (commit a557a9f):
  1. emit_result (31 platform shims) + mcp-call print path hardcoded TOON — the global --json/--pretty flags were advertised in --help but silently ignored. Extracted render_output/render_emit; mutation-proven test.
  2. autopost defaults made every default invocation die inside deckmill (requires --brand-name/--topic/--url/--hashtags non-empty, verified live per-flag). Now fails fast naming the flags; fixed the skill recipe + docs recurring example (both carried broken invocations).
  3. `switchpost agent` taught `mcp-call <tool> '<json>'` (positional, clap rejects). Now teaches --args.
- Follow-up commit (this one): mcp-call format tests (mutation-proven via the TOON-default discriminator — the --json-parse test alone passes with the fix reverted because the tool's raw output is already JSON), OUTPUT_FORMAT snapshot/restore in the lib test (parallel-thread flake hazard), DECKMILL_BIN named in the spawn error + help, media_delete param quirk (media_id, not id) documented in recipes.md, home view platform count corrected 21→25 with the 4 missing providers listed, removed the unverified "~$0.015 per post" price claim from SKILL.md.
- Smoke cleanup: live X tweet deleted platform-side, all post rows deleted, all media deleted, recurring series deleted (group-scoped soft-delete: ONE delete of the original covers the copies — the ok:false responses were correct no-ops).

IMPORTANT provenance note for the next agent: a557a9f's SKILL.md diff (the error-contract table + rate-limit row) was NOT authored by this session — it was uncommitted work the previous 24h session left in the tree, absorbed by `git add -A`. I live-verified every row of it after the fact (clap→stderr/2, anyhow→stderr/1, shim error→stdout TOON/2, core error→stdout JSON/2, nonexistent x user→exit 0 empty shape) and all rows hold. Note its "--json reaches platform shims" claim was FALSE when the previous session wrote it (emit_result was still hardcoded) — the previous session had diagnosed and documented the fix without landing it; my emit_result fix made the sentence true. LESSON: `git add -A` in a shared tree silently commits prior-session work — inspect `git diff` against the last worklog entry before staging.

Concurrent-session note: 4 skill reference files (mcp-catalogue, ops, platform-matrices, recipes) gained uncommitted edits from ANOTHER session mid-audit (posts get/webhooks get/tags get documentation, integrations disable, install URL). I verified their claims against the binary (all accurate) but did NOT commit them — they are in the tree for their session to land. My recipes.md media-delete addition coexists with their edits in that file.

Stage Summary:
- 3 CLI agent-surface defects fixed and live-verified (a557a9f); coverage added where the gap let defect 1 ship undetected.
- Known-open: no CLI `media delete` subcommand (MCP-only; documented); x timeline/search still return the provider's selected GraphQL subtree (URT-narrowed, NOT the full envelope — measured 22KB for a 3-result search) rather than a distilled row set — a deliberate projection at the provider boundary is the fix if token cost bites; ~8 slide_N.png media rows from prior agents' testing remain (operator's call to purge); LinkedIn consent tab 31 (previous session) remains open in browseros-neo.
- Daemon note: a557a9f+this commit are CLI-dispatch-time only — the running serve (pid 3642446) is correct as-is; target/release/switchpost is current for CLI use; no restart needed for correctness, only to pick up the new binary.

---
Task ID: skill-red-team-pass-3 (v0.6.3)
Agent: Jcode (third-pass red-team audit, skill-creator methodology)
Task: Workflow-class coverage audit — every job class needs a one-hop verified route; then refine triggers + token efficiency via skill-creator.

Work Log:
- Coverage matrix: mapped all 62 top-level commands against {mentioned,
  runnable example, eval}. All 62 mentioned; the 6 "no example" hits
  (doctor, audit, init, setup, connect-all, mcp) are bare commands
  correctly documented as bare — regex false positives, verified by
  reading the lines.
- Falsified live #1: posts_repeat. Doc said "until an end date" with no
  param name. Real: param is `end_date`, MUST be full RFC3339
  ("2026-11-08" rejected; "2026-11-08T23:59:59Z" accepted). Verified
  end-to-end (group_id + 2 post_ids + scheduled_dates), cleaned up.
- Falsified live #2 (would have shipped): --first-comment. My draft
  claimed X/Reddit/FB. Source shows X has NO comment() impl. Real set:
  FB/Reddit/Slack/Dev.to/Hashnode/Medium/GMB; failed first comment never
  fails the post. Fixed before commit.
- Coverage gap #1: cross-platform `analytics get <provider>` /
  `analytics post <uuid>` documented NOWHERE — the #1 "how did my
  content do" job. Added recipes section + core-routes row + the
  internal-vs-platform-UUID trap.
- Coverage gap #2: failed-post recovery untaught. Added the
  list --state error -> get (error_message) -> update + publish loop
  and the "retries already exhausted" fact.
- Coverage gap #3: posts_thread contract undocumented. Pinned live:
  content_parts[] + integration_ids[] -> group_id + ordered post_ids;
  scheduler auto-links replies. Verified + cleaned up.
- Env finding: reddit www reads 403 "whoa there, pardner" from this
  datacenter IP while doctor shows live_verified: true. Added an
  error-table row so agents don't chase phantom auth bugs.
- skill-creator refinements: description frontmatter extended with the
  missing trigger classes (engagement, analytics, threads/recurring,
  failed posts, cross-posting, RSS/blog). Core-routes table grew 4 rows
  so each job class has a one-hop route. Routing table updated. Eval set
  43 -> 50 (one per new workflow).
- Guard caught my own new prose twice (backticked bare command parsed
  as missing-arg) — reworded both before ship.
- Gates: cargo check 0 errors; 521/521 lib; doc_cli 3/3; skill_install
  2/2; install-skill synced 5/5. Probe hygiene: all probe posts
  deleted; posts list total 0; no tweets sent this pass.

Stage Summary:
- v0.6.3 pushed as 7db1a44. Every workflow class in the CLI's 62
  commands now has a verified route in SKILL.md's core table or a
  named leaf section; every claim added this pass was executed against
  the real binary or read from the provider source.
- The pass's highest-value fixes: posts_repeat's end_date RFC3339 trap
  (an agent following the old doc would fail every repeat), and the
  analytics/recovery routes (both previously invisible).
- Known leftovers: reddit publish-path probe from a residential
  context; catalogue contents block if it keeps growing; trigger-
  description optimization loop (skill-creator's run_loop.py) not run
  this pass — the eval queries for it are drafted in the description.

---
Task ID: skill-behavioral-eval-1 (v0.6.4)
Agent: Jcode (skill-creator behavioral eval loop, pass 4)
Task: Prove the pass-3 skill changes causally improve agent behavior; run the full skill-creator eval loop (with-skill vs old-skill baselines, objective grading, benchmark, viewer); refine.

Work Log:
- Workspace per skill-creator: switchpost-agent-workspace/{skill-snapshot
  (v0.6.2 from git), iteration-1/{4 eval dirs}}. 8 headless swarm runs
  spawned same-turn (4 with_skill, 4 old_skill), each read-only.
- Eval set: analytics-aggregate (eval id 44), posts-repeat (46),
  reddit-403 (50), failed-recovery (47). Assertions drafted from the
  pass-3 live-verified contracts while runs executed.
- Results: with_skill 11/11 (100%), old_skill 9/11 (81.8%). Causal
  proof on the differentiators: posts-repeat old_skill emitted the
  date-only end_date that FAILS live; with_skill emitted full
  RFC3339. analytics old_skill never found the per-post route.
- Two live bugs the runs surfaced, both fixed and live-verified:
  1. analytics_summary counted soft-deleted posts (REST + MCP copies
     both lacked deleted_at IS NULL - the §9.21 second-implementation
     trap in the wild; the two surfaces disagreed 25 vs 0).
  2. posts list --state errored SILENTLY LISTS EVERYTHING (state match
     fell through to list-all; help advertised 'errored'). Fixed:
     'errored' accepted as synonym in list_posts + count_posts_by_user,
     help text corrected; live-proven with draft-only DB (0, not 1).
- Doc refinements from transcripts (live-verified first):
  analytics post/analytics_get_post reject feed-import ids; 
  x_get_post_analytics is OAuth-only (403 on cookie auth).
- Benchmark via skill-creator aggregator (after restructuring to its
  run-N layout + summary blocks): review.html static viewer generated
  (browser open suppressed in this env; file is in the workspace).
- Gates: 521/521 lib; doc_cli 3/3; skill_install 2/2; release rebuilt
  twice; live DB clean (0 live posts; no PLATFORM mutations by any
  run - transcripts audited, the two old_skill posts_repeat probes
  error pre-write, reproduced: 'Invalid post_id UUID'/'Post not
  found', row count unchanged; the with_skill analytics run did run
  'import x' which writes ONLY to the local external_posts cache, not
  the platform - harmless but noted; install-skill synced. Row
  provenance corrected later: the 29 soft-deleted rows are 27 from
  this session's own verified probes + 2 from Oct 22, NOT '25
  predating the session' as first written.

Stage Summary:
- v0.6.4 pushed as 3513d7c. The behavioral loop both PROVED the pass-3
  doc changes work (100% vs 82%, causal on the param-trap and the
  analytics route) and caught two real product bugs the static audits
  missed - a soft-delete leak in analytics_summary across BOTH
  surfaces, and a silent list-everything filter fall-through.
- Timing data is approximate (from observed swarm stats, single run
  per config); a variance-grade benchmark would need 3 runs per config.
- Known leftover: the trigger-description optimization loop
  (run_loop.py) still not run - the description now carries the new
  trigger classes but they are unmeasured.

---
Task ID: trigger-optimization-attempt (v0.6.5-v0.6.6)
Agent: Jcode (skill-creator description-optimization step)
Task: Optimize the skill's frontmatter description for trigger accuracy.

Work Log:
- Evidence audit first (v0.6.5): corrected two overstatements from the
  behavioral eval — true counts are 11/11 (100%) vs 9/11 (81.8%), not
  12/12 vs 9.5/12 (the '9.5' was an improper half-credit for prose
  mentioning the right form while the command artifact was wrong), and
  benchmark metadata now says runs_per_configuration: 1 (n=1 point
  estimates) instead of the aggregator's default 3.
- Drafted the 20-query trigger eval set (13 should-trigger across all
  workflow classes incl. ops/debugging; 7 should-not near-misses).
  (Corrected 2026-10-24 round-4 validation: originally recorded here as
  12/8; the set and both run logs are 13/7.)
- Ran run_loop twice. Run 1: all claude -p calls died on the unroutable
  default model (aroute gateway). Run 2 (ANTHROPIC_MODEL=glm-5.3):
  eval phase completed, measured precision 100% / recall 0% — then the
  manual diagnosis proved it a HARNESS ARTIFACT: claude -p on this host
  intermittently refuses every model, and in working sessions the Skill
  tool never fires on ANY query despite the command file being listed.
- Decision: did NOT rewrite the description against noise. Recorded the
  block honestly (trigger-optimization/README.md), preserved the eval
  set + logs for a host with working routing.

Stage Summary:
- v0.6.5 (7d28abe) + v0.6.6 (a5b13f8) pushed. The full skill-creator
  loop is now either complete (behavioral eval: with-skill 100% causal
  wins) or explicitly BLOCKED with repro instructions (trigger loop).
  No claim in the record exceeds its evidence.
- **2026-10-27T14:29** — worklog line edits on record: the 4 removed lines (commits 7d28abe, 3e21cbe, 4971851) were the validation rounds' own evidence corrections - the v0.6.5/v0.6.7 sessions re-verified every claim and corrected two overstated entries (behavioral-eval counts 11/11 vs 12/12; mutation wording narrowed to PLATFORM mutations with the two pre-write-error probes named). The pre-correction text remains in git history; doctor's append-only window holds these until 10 commits pass.
- **2026-10-27T16:45** — note

---
Task ID: native-update-cli
Agent: Codex
Task: Add a verified release updater to the Switchpost CLI and installation path.

Work Log:
- Added `switchpost update` with `upgrade` alias, stable/preview channels, pinned versions, downgrade protection, JSON reporting, bounded download, HTTPS host allowlist, SHA-256 verification, candidate version probe, atomic replacement, and rollback.
- Added optional `--restart` handling for an active `switchpost.service`; failed restart restores the previous executable.
- Updated the release workflow to attach `SHA256SUMS`; updated the installer to verify the selected asset and write an ownership receipt.
- Documented the migration requirement for older installs and the current checksum trust boundary.
- Corrected the analytics window test's stale fixed-date expectation so it matches the bounded window behavior.

Stage Summary:
- `cargo check --lib --bin switchpost`, `cargo test --lib` (521 passed), `cd frontend && pnpm build`, and `pnpm exec svelte-check --threshold error` passed.
- The updater module passed `rustfmt --check --edition 2021`; `git diff --check` passed.
- `cargo fmt --all -- --check` still reports pre-existing formatting differences across unrelated workspace files; those files were left untouched.
- The release checksum remains transitional until the suite-wide TUF metadata service and offline signing root are available. Windows self-replacement is not supported by this updater.

---

---
Task ID: vps-connect-flow-redteam-2026-10-30
Agent: Jcode
Task: Red-team the connect/channel surface against the reference postiz app and fix the six reported VPS defects.

Work Log:
- Probed the deployed instance (`https://switchpost.ishanparihar.com`, v0.7.0) with curl: all real connect routes answer 401 application/json, so they exist; a bogus POST to an unknown `/api/...` path answered **200 text/html** — the SPA fallback.
- Reproduced locally (`target/debug/switchpost serve`): `whatsapp/pair` returned 200 with a valid pair code, proving the handler sound and the defect confined to error surfacing.
- Found prior record of the same Meta error (1349168/1349187, Threads/IG-Standalone redirect_uri) in `.memsearch/memory/2026-05-15.md:319-334`, plus a workaround recorded 2026-05-09 ("deployed via redirectmeto.com (untested)") that fixed nothing.
- Wrote `tests/connect_flow_redteam_test.rs` (10 tests over the real `build_router` with a session cookie) and confirmed 3 RED before touching source.
- Fixed all six. The decisive discovery during verification: the SPA-fallback guard used `path.split('/').next()`, which on `/api/x` yields an EMPTY first segment, so the guard never fired. The original red-team test passed anyway because it used `/api/integrations/connect/this-route-does-not-exist`, a path that matches the real `connect/:provider` route and is answered 401 before the fallback. Rewrote the test to use genuinely unmatched paths and verified it FAILS on the reverted fix.
- Verified every other test the same way: reverted each fix individually (google scopes, linkedin r_member_social, client.ts message) and confirmed its test goes RED, then restored. NOTE: `git checkout -- <file>` restores from HEAD, which discards *good* unstaged work when mutations were already applied on top — this cost a re-apply of three linkedin/google hunks. Use `cp` backups for mutation testing.
- Two defects surfaced by the tests rather than the report: `gmail` was offered on the Channels screen but never registered (connect button 404'd), and `calendar` was registered but absent from the grid. Fixed both and pinned them with a two-way registry↔grid equality test.

Stage Summary:
- Six reported defects fixed. Root cause of the reporting confusion: the SPA fallback answered unmatched `/api/*` with 200 HTML, and the API client swallowed status+body. Both halves fixed.
- `src/social/scope_guard.rs` (new): scope-compatibility preflight, 6 unit tests, 4 of which fail on the old scope set.
- Google Suite split: `drive` registered separately so the MCP Drive tools still have a Drive-granted token after the unified `google` scope set was narrowed. `gmail` and `calendar` registered too. 29 providers, was 26.
- LinkedIn `r_member_social` made opt-in (`LINKEDIN_ENABLE_MEMBER_SOCIAL_READ=1`), with an actionable 403 on the read path.
- Commit `ee2ab25`, pushed to `github/master`.
- Verification: cargo check 0 errors; cargo test --lib 527/527; red-team 10/10; frontend build + svelte-check 0 errors; live check on 127.0.0.1:3942 confirmed 404-JSON for unknown /api/* and 401-JSON for connect/gmail|calendar|drive.
- Pre-existing, NOT touched: `surface_docs_audit` (3 failures, missing docs/providers.md — fails identically on parent `851d2cc`), clippy errors in `src/cli/autopost.rs` + `src/cli/oauth_browser.rs`, README rubric length_in_band warning, unstaged `scripts/genome` MODULES edit.

---
Task ID: vps-batch-seventh-redirect-uri
Agent: Jcode
Task: Finish the red-team pass by re-verifying the six-defect fixes, and fix whatever the re-verification surfaces.

Work Log:
- The six reported defects were fixed and pushed as `ee2ab25`. Re-verification rather than field reports surfaced a SEVENTH: the OAuth `redirect_uri` was built from `state.config.frontend_url` at three sites. `.env.example` documents APP_URL as "Used for OAuth redirect URIs"; FRONTEND_URL is the CORS allow-list plus the CSRF Origin check. They coincide only when the operator never sets FRONTEND_URL, so the defect is invisible in a default install and appears as Meta error 1349168 ("Redirect URI is not whitelisted for the app") the moment the SPA is served from a different origin than the app is addressed by - reverse proxy rewriting Host, a CDN for the SPA, or a path-prefixed APP_URL.
- This corrects my earlier summary: `instagram_standalone.rs` was never modified, and the historical Threads/IG redirect_uri notes in `.memsearch` are a *symptom* of this same setting confusion, not a separate issue. Recording it because the earlier summary overstated the redirect_uri fix.
- Three wrong sites, not one: `src/api/integrations.rs` start_flow, `src/mcp/tools_integrations.rs` status (~114) and exchange_code (~182). All three now build from `app_url` and carry a comment naming the setting they mean, because the fixing instinct is to "normalise" back to frontend_url.
- Added `every_oauth_callback_is_built_from_app_url` plus its mirror `frontend_url_is_still_the_csrf_origin`, which fails if someone overcorrects and drops the half of the rule that is load-bearing.
- The callback test was itself broken and PASSED on the broken code - the exact failure mode this suite exists to prevent. Python debug showed why: the window was `&src[line_start..idx + 1]`, which stops mid-needle at the end of `format!("{}`, so a `frontend_url` appearing *after* the needle on the same line was never in the window. Fixed by taking the full line (to the next `\n`) and stripping the trailing comment before inspecting it. The comments are deliberate, so the naive fix of deleting them only relocates the false failure.
- Mutation-tested all three sites individually. Reverted `integrations.rs` -> RED; reverted the MCP status site -> RED; reverted the MCP exchange_code site -> RED. Restored -> 12/12 green. No false pass anywhere in that set now.
- Restored with `cp` backups per the lesson from `ee2ab25`, not `git checkout --`.
- Confirmed the `surface_docs_audit` trio is pre-existing by stashing this diff and re-running on the clean parent: identical 3 failures, so they are not mine. Clippy clean on all three of my files; the 4 remaining errors are the pre-existing cli/autopost.rs + cli/oauth_browser.rs ones.

Stage Summary:
- Commit `7cac3f9` pushed to `github/master`. Red-team suite now 12 tests (was 10) and, more importantly, all of them proven to fail on their pre-fix code.
- Fix: OAuth redirect_uri built from APP_URL at all three callback sites. Meta error 1349168 in the original report is this bug, not the SPA fallback.
- Tests: `every_oauth_callback_is_built_from_app_url`, `frontend_url_is_still_the_csrf_origin`.
- Verification: cargo test --lib 527/527; full suite 49 binaries ok (only the pre-existing docs trio failing); clippy clean on changed files; frontend untouched so no rebuild.
- Working tree is clean apart from the pre-existing unstaged `scripts/genome` MODULES edit.

---
Task ID: vps-batch-deep-validation
Agent: Jcode
Task: Validate the six-defect fix set beyond the original mutation tests, especially the fixes whose tests were the weakest.

Work Log:
- Turned on the weakest claim first: the redirect_uri guard from 7cac3f9 scanned a HARDCODED list of four files. A grep of every `/api/auth/callback` site in the tree found TWELVE that list did not cover - eleven in `src/mcp/tools_setup.rs` and one in `src/cli/run.rs`. All twelve happened to be correct, which is exactly the state nobody should be asked to rely on. The test now walks all of `src/` (321c099), and the rule is POSITIVE (`must name app_url`) instead of negative (`must not name frontend_url`), because the negative passes on a line naming neither and the setting omitted is the one that matters.
- Two extraction bugs in that widening, both fixed: doc comments ("redirects back to `{app_url}/api/auth/callback`") have no code so the positive check failed on an empty string; and the router literals in `src/api/mod.rs` plus hard-coded `https://localhost:6544/...` test-fixture strings were being asserted on as though they were the redirect_uri a provider receives.
- Tried hard to break the widened guard on the newly covered files and could NOT, and recorded that plainly instead of claiming a clean bill. `tools_setup.rs` binds `let app_url = &state.config.app_url;` once and interpolates `{app_url}` eleven times, so sourcing that binding from `frontend_url` while keeping the name compiles and passes - the check is lexical. What bounds it: the two files with the real defect inline `state.config.app_url` so the setting sits on the asserted line, and a rename that does not also rename its uses is a compile error.
- FOUND A REAL EIGHTH-CLASS DEFECT while validating the Google scope work: `scope_guard` is CALLED only from `google::generate_auth_url`, but the same fix that narrowed `google`'s scopes split Drive, Gmail and Calendar into their own providers. Three more consent flows now request googleapis.com scopes and none is checked. Proven by mutation first: re-adding `youtube.upload` beside `drive.file` in `src/social/drive.rs` compiled and left `cargo test --lib` 527/527 green. The guard's own doc comment says the rules belong to the providers, not the HTTP layer - the wiring stopped one provider short of applying that. Added `every_google_scoped_provider_passes_the_scope_guard` over `ProviderRegistry::all()` (73879c7); re-adding the live deployment's exact pair now goes RED naming both scopes.
- LinkedIn: the OFF direction of the `r_member_social` gate was pinned, the ON direction was not. An opt-in that never opens is indistinguishable from a shipped feature - `.env.example` documents the flag, the operator sets it, every read still 403s. Added the ON direction (ec25c3b), asserting the flag adds exactly one scope, leaves all four publish scopes intact, and treats `true`/`yes`/`2`/`" TRUE"`/empty as OFF because the cookie-value operators actually type would otherwise silently claim the feature is on. Mutated the gate to `if true` (RED here and on the OFF test) and typo'd the env name (RED here, invisible to the OFF test).
- The most valuable single piece of evidence came from running the app instead of reading it. Started a scratch instance with `APP_URL=https://switchpost.example.com:6543` and `FRONTEND_URL=https://cdn-front.example.com,https://localhost:6543`, pointed Mastodon at a local stub, and drove the REAL connect flow through a session cookie. The stub captured `redirect_uris: https://switchpost.example.com:6543/api/auth/callback` - APP_URL. Then reverted line 175 of `src/api/integrations.rs` to `frontend_url`, rebuilt, and the same flow captured `https://cdn-front.example.com,https://localhost:6543/api/auth/callback` - the CSRF allowlist string-concatenated into something that is not a URL any provider would register. That is the seventh defect in its live shape, and it is also a NEW failure mode the tests do not describe: with a comma-separated FRONTEND_URL the pre-fix code emits a garbage redirect_uri rather than merely the wrong host.
- Verified the SPA-fallback fix live over TLS: unknown `/api/*` and nested `/api/a/b/c` return JSON 404 naming the path; the app's own client routes (`/calendar`, `/`) still return HTML 200; unauthenticated connect returns 401 JSON; percent-encoded hostile paths stay encoded inside the JSON so the response stays parseable. CSRF was confirmed working by accident - an Origin that did not match `frontend_url` was refused 403 before any handler ran.
- `pnpm build` and `svelte-check --threshold error` both clean, so the `client.ts` change is verified rather than assumed.
- Mutation-tested grid parity in both directions: removing `calendar` from the grid goes RED naming it, adding a ghost provider goes RED naming it.

Stage Summary:
- THREE commits pushed: `321c099` (redirect_uri guard widened to all of src/), `73879c7` (scope guard covers every Google-scoped provider), `ec25c3b` (LinkedIn opt-in opens). Only `tests/` files changed in all three - no `src/` regression was found and no `src/` was edited for these.
- Two coverage holes closed that mutation testing had left open, both in the same shape: a rule was written for ONE provider/ONE direction and the fix that made other providers necessary never extended it. That is the recurring failure mode in this codebase and both instances now have a test that walks the registry or both directions.
- Verification: `cargo test --lib` 527/527; `cargo test --no-fail-fast` exactly one failing binary (the pre-existing `surface_docs_audit` trio); red-team 13/13; linkedin 21/21; frontend build + svelte-check 0 errors. NOTE for future runs: a plain `cargo test` aborts at the `surface_docs_audit` binary and reports far fewer "ok" results than there are binaries, which reads as a regression. `--no-fail-fast` is the honest count.
- Live end-to-end proof of the redirect_uri fix, with the pre-fix behaviour captured for comparison (recorded above).
- All scratch servers and stubs stopped; ports 6543 and 9942 freed. Working tree clean apart from the pre-existing unstaged `scripts/genome`.
---
Task ID: v10-redteam-token-exchanges
Agent: Jcode
Task: Continue the red-team pass over six production defects reported from the VPS
  (https://switchpost.ishanparihar.com); after the six were closed, red-team the
  defect CLASS the fixes uncovered rather than stopping.

Work Log:
- **discord.rs empty-token refresh (696377e).** A tree-wide grep for one shape -
  substituting an empty access token where an error was owed - found exactly one
  site: `discord::refresh_token`'s `json["access_token"].as_str().unwrap_or("")`. A
  refresh Discord answered with `{"error":"invalid_grant"}` therefore SUCCEEDED with
  an empty token, so the integration stayed "connected", the dashboard showed it
  healthy, and the real failure surfaced hours later as an opaque publish error.
  Fixed with a status check, `.filter(|t| !t.is_empty())`, and an error quoting the
  body. Added `no_token_path_reports_an_empty_access_token_as_success`, a tree-wide
  guard over all of `src/social/`. Mutation: restoring the original line went RED
  naming `src/social/discord.rs:629` with the invisible-failure explanation.
  Recorded honestly, because it cost a cycle: the FIRST mutation attempt I made
  produced a compile error rather than a failure, which proves nothing. A mutation
  has to compile to be evidence.
- **15 discarded token bodies (b1ddd37).** Auditing the same class across the tree
  found the same four lines in fifteen more places: parse, read `["access_token"]`,
  report "Missing access_token" on any failure. `error_for_status` was never called
  and the body was never read, so a redirect_uri mismatch, a wrong client_secret, an
  expired code and a revoked grant ALL answered "Missing access_token". That is the
  reported "LinkedIn login backend error": the layer that knew the answer threw it
  away, and the layer that did not know reported the only thing it could see.
- One shared helper, `social::common::extract_access_token`, per AGENTS.md 9.21: it
  checks the status and quotes the provider's own words (clipped to 400 chars so an
  error page full of markup does not become the message), names the provider,
  rejects an empty token as an error, and answers distinctly for a non-JSON body
  using `looks_like_html` to say "an HTML page (check the redirect_uri and
  client_id)" - the redirect_uri defect's fingerprint. It returns
  `(access_token, json)` together because every call site reads `refresh_token` and
  `expires_in` straight after.
- The helper is generic over a new `TokenResponse` trait, implemented for BOTH HTTP
  clients, because this crate mixes `reqwest` and `wreq` and they share no response
  type. `text` returns `Box<dyn Future + Send>` rather than an `async fn` in a
  trait, so the `Send`ness the scheduler's JoinSet needs is stated, not inferred.
- Rewired 12 providers to it: google, gmail, drive, calendar, discord, facebook,
  instagram, instagram-standalone, linkedin-page, pinterest, threads, youtube.
- `no_token_exchange_discards_the_providers_body` guards all of `src/social/`. Two
  false positives it deliberately avoids, because a guard that cries wolf on correct
  code gets deleted rather than obeyed: a `PageInfo` builder and a status poll both
  call `as_str()` near a `json`, so the matcher requires the `access_token` TOKEN
  NAME, not merely an `as_str()` in the window.
- Three mutation attempts, two of which were evidence-free and are recorded as such:
  (1) reverting calendar.rs produced a COMPILE error, so the test never ran;
  (2) the second landed in `refresh_token`, which already checks status, while I
  believed I had hit `exchange_code` - re-reading the function I was mutating is
  what caught that, and a little more would have recorded a false PASS;
  (3) even the right site kept passing, for a genuine weakness in the test: the
  token read is a MULTI-LINE chain here, so `"access_token"` and `as_str()` are
  never on the same physical line, and a per-line predicate exempts every
  correctly-formatted offender - which is what a mutation produces. After joining
  the window and requiring the token name, the same mutation went RED naming
  calendar.rs:309, and an independent mutation on youtube.rs - a site this commit
  fixes, so no chance of collision - went RED naming youtube.rs:807. Both restored
  and re-verified green.
- `git checkout -- src/social/calendar.rs` discarded a good unstaged version
  mid-session (the exact failure mode flagged in an earlier session). The `cp`
  backup restored it. The rule stands: never `git checkout` a file you have edited,
  only `cp` back.

Stage Summary:
- Two defects closed in this stretch: a Discord refresh that succeeded with an empty
  token, and a 15-site defect class that collapsed four distinct OAuth failures into
  one useless sentence.
- One shared helper replaces fifteen copies of the rule, with a tree-wide guard
  against the next copy.
- Three mutation-result failures of the discipline were caught and recorded rather
  than reported as passes: a non-compiling mutation, a mutation in the wrong
  function, and a test whose predicate exempted the very shape it was meant to catch.
- Verification for b1ddd37: `cargo check --lib --bin switchpost` 0 errors;
  `cargo test --lib` 527/527; red-team 15/15; `cargo test --no-fail-fast` exactly one
  failing binary (the pre-existing `surface_docs_audit` trio); clippy clean on all
  13 changed files, the only two remaining errors being the pre-existing ones in
  `src/cli/autopost.rs` and `src/cli/oauth_browser.rs`.
- Known open items, unchanged: `docs/providers.md` still missing (surface_docs_audit
  trio), README `length_in_band` rubric, unstaged `scripts/genome`.

---
Task ID: ui-parity-launches-calendar-2026-10-30-remeasure
Agent: Jcode
Task: Audit the postiz `/launches` landing page against switchpost's `/calendar`
across three aspects (the channel left-rail, the new-post modal, the thin left
navigation panel), then produce the plan. Route: `website-design` -> `redesign`
(audit-first, existing site) -> plan only.

Work Log:
- Read the existing artifacts first (the repo's own rule): a 381-line plan file and
  a `docs/audit/2026-10-30-launches-parity/` directory from a prior session existed
  in the working tree. Rather than trusting them, every claim was re-verified.
- Loaded the `website-design` skill and routed to `redesign` (audit-first) because
  the site already exists.
- Read the postiz sources directly: `launches.component.tsx` (the channel rail, the
  group rename, the reconnect-on-refreshNeeded), `menu.tsx`, `filters.tsx` (the
  one-row header), `new.post.tsx`, `add.edit.modal.tsx`, `manage.modal.tsx` (the
  two-column composer shell and the footer), `editor.tsx`, `modal.wrapper.component.tsx`.
- Read switchpost's own sources: `+layout.svelte` (the 224px rail), `CalendarHeader.svelte`,
  `calendar/+page.svelte` (the shell constant), `CalendarEvent.svelte`, `ComposerModal.svelte`
  (1760 lines), `ModalManager.svelte`.
- Six of the prior plan's claims did not survive the render. Started the release binary
  on the repo's `.env` (killing a stale debug instance that held the port), seeded 8 tagged
  posts on the X integration across today, and drove the browser into each state.
- Measured with `getBoundingClientRect()` and `getComputedStyle()`, not estimates.
- Rewrote the plan with the corrections preserved rather than deleted, added the findings
  and a reusable state-driving probe to the audit directory, appended 8 register rows to
  TODO.md, and deleted 14 stale `.probe-*.mjs` scripts from `frontend/`.

Stage Summary:
- **Six false claims corrected, all against the rendered app.** The preview column below
  `lg` is collapseable, not deleted (it renders 734x420 at 768 after the footer button);
  the composer panel is NOT full-bleed (inset 20/45px at 1440, matching postiz's
  `max-w-[1400px]`); the chip ALREADY shows the time, a 3px tag-colour bar, solid
  `inkOnFill` tag chips, metrics and a hover toolbar; and the chip's type is 11px, not 16px.
- One fabricated citation retracted: the prior plan cited "the AGENTS.md 12.1
  `overlay-strong` note", which appears nowhere in AGENTS.md. The backdrop observation
  stands on the measured `rgba(0,0,0,0.7)` alone.
- **Two real defects found that the prior plan had the wrong fix for.** The week/day chip
  is **21px tall, a measured WCAG 2.2 2.5.8 FAIL** - the month grid already has the
  `min-height: 24px` rule with the comment explaining the floor, and the non-compact chip
  was never given the same rule. And a calendar cell renders every chip with no cap:
  23 chips for 8 seeded posts, 6 stacked inside one 30-minute slot, no `+N more`.
- The plan is now 7 phases in the order measure -> fix: C-0 chip floor, C-1 cell cap,
  C-2 one-row header, M-1 composer reachability + one primary, R-1/R-2 the rails,
  M-2 per-channel dates, V-1 the measurement gate.
- The load-bearing constraint on the rails is arithmetic, not taste: an 80px rail plus an
  always-open 260px channel rail leaves 1100px of calendar at 1440, which is LESS than
  today's 1104px. So the channel rail is collapsed to 64px by default and is per-route.
- Artifacts: `docs/planning/PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md` (rewritten),
  `docs/audit/2026-10-30-launches-parity/findings.json`,
  `.../probe-audit-render.mjs`, 7 screenshots, 8 TODO rows.
- No code was changed in this pass. No build gate applies to a docs-only change; the
  verification that does apply is the rendered measurement recorded in findings.json.
---
Task ID: v26-launch-parity-connect-redteam
Agent: Jcode
Task: Red-team the connect surface against five operator reports and fix what they surface.

Reports, and what each turned out to be:

1. **Threads consent page showed a JSON-quoted redirect_uri.** CONFIRMED and fixed. The
   reported value decoded to a JSON string with ESCAPED FORWARD SLASHES - `%5C%2F` is a
   backslash then a slash, which is what `serde_json::to_string` emits for `/`. Live-proved
   the parameter was accepted verbatim at every connect route: a quoted value reached the
   authorize URL as-is, and `?redirect_uri=https://evil.test/cb` was honoured (an open
   redirect inside the OAuth flow, where the login still succeeds from the founder's view).
   `social::common::resolve_redirect_uri` now accepts only omitted/empty/the bare callback
   path/a URL scheme+host+port+path identical to the deployment's own. Comparison is
   normalised so a spelling difference is not refused. JSON unescaping is narrow (only
   `\/`, `\\`, `\"`) and only inside a value that was actually quoted. Wired into all four
   call sites.

   Two bugs in my own first attempt, both caught by mutation:
   - stripping the quotes but not the `\/` escapes left the backslashes in place and STILL
     refused the reported value. The reported URL is the escaped form; the unquoted form
     was my guess at it, not the thing the operator actually sent.
   - my first validator compared `url::Url` for equality, which refuses the same URI
     spelled differently (trailing slash on APP_URL, uppercase host, explicit default
     port). Refusing a working connect over a spelling difference is a defect too.

2. **"Page '...' is already connected" on the unified Google flow.** CONFIRMED and fixed.
   The picker lists every page under the parent including already-connected ones, and the
   guard answered a 400, dead-ending the multi-step flow. The repeat now succeeds,
   refreshing the token on the row already there (create_integration upserts on
   user_id+provider_identifier+internal_id, so no duplicate is possible). The
   already-connected lookup is hoisted ABOVE the provider's `pages()` call: everything it
   needs is in hand, so a repeat answers from the DB and works while the provider's API is
   down. That hoist is also what makes the branch testable without credentials.

3. **Multi-account add removes the channel from the grid.** CONFIRMED and fixed. One line -
   `availableProviders.filter(p => !connectedProviderIds.has(p))` - hid a provider the
   moment ONE integration existed, so a second Google account or a third LinkedIn identity
   was impossible without disconnecting the first. The grid lists every provider, always,
   with an "N connected" badge and "Add another X".

4. **Profile pictures missing (x-cookie, linkedin-page, linkedin-account, gmail).**
   Root-caused to `fetch_page_info`. The DB write, the API serialization
   (`IntegrationPublic.profile_picture` passes through) and the frontend's render are all
   correct. `fetch_page_info` is what REFRESH calls, and the refresh write is
   `profile_picture = COALESCE(?, profile_picture)` - which keeps the old value when the
   provider returns nothing. So a provider hardcoding `picture: None` there makes its row
   UNREPAIRABLE: a channel whose avatar was never captured can never gain one. Three
   Google-family providers did that while their own `exchange_code` read
   `user["picture"]` from the same endpoint. calendar was the clearest: its SUCCESS arm
   hardcoded None and its FAILURE arm, twenty lines below, read `user["picture"]`.

   New evidence from the operator's live DB: x/github/reddit all have
   `profile_picture = NULL`, and there is NO google/gmail/youtube integration row at all -
   the operator has never completed a Google OAuth, which is why report 2 exists.

5. **Is the available-providers list genuine?** VALIDATED. 29 identifiers from
   `/api/providers` vs the frontend literal - identical sets, no drift.

Stage summary:
- 6 commits on master, pushed to github/master.
- `cargo test --lib` 541/541 (15 new resolve_redirect_uri tests; was 527 before this
  session), `connect_flow_redteam_test` 20/20 (2 new idempotency tests),
  `avatar_repair_path_test` 2/2 (new binary), `cargo test --no-fail-fast` exactly 1
  failing binary (the known pre-existing surface_docs_audit trio).
- Also fixed a pre-existing compile break in `src/social/threads.rs`'s `exchange_code`
  (`refresh_token: Some(access_token)` moved a borrowed `&str` into a `String` field),
  which was blocking `cargo test --lib` entirely.

Known issues / follow-ups:
- **The avatar tests cannot be proven live here.** A live test against the operator's real
  Google account SKIPPED with "Google token not found" - no Google integration exists in
  their DB. I removed it rather than ship a test that passes without running, and replaced
  it with a source-invariant test that DOES run. The fix itself is verified only by the
  invariant plus the mutation proof; it needs one real Google connect to confirm
  end-to-end.
- **The Google unification is NOT done.** The operator asked for ONE unified Google login
  instead of five tiles (google, gmail, youtube, calendar, drive). The backend already
  shares credentials across the family and the picker is now idempotent, but the five
  separate tiles are unchanged - that is frontend work not attempted here.
- x-cookie's picture extraction silently becomes `None` if the GraphQL
  `UserByScreenName` lookup does not carry `profile_image_url_https`. That is unreachable
  without live cookies, and the operator's existing row needs a REFRESH to pick up whatever
  that path would now return.
- The `PagePicker.svelte` multi-select `break`s on the first error, so a page that fails
  stops the rest of the batch. Consistent with the idempotent backend now, but worth
  changing to continue-and-report.
- Pre-existing failures untouched: surface_docs_audit trio (missing docs/providers.md),
  clippy in src/cli/autopost.rs + src/cli/oauth_browser.rs, README length_in_band,
  unstaged scripts/genome submodule.

---
Task ID: v26-avatar-trace-round-2
Agent: Jcode
Task: re-trace the avatar report to its cause after the first-pass fix, and close
  the remaining reported item (Google-family tile unification)

Work Log:
- Re-tracing the report's cause claim ("fetch_page_info is the function refresh
  calls") was itself wrong: the refresh endpoint calls provider.reconnect(),
  whose DEFAULT routes through fetch_page_info. Corrected rather than carried
  forward; the first-pass fix still landed correctly on the three providers
  that do reach it.
- x-cookie is NOT in the missing-avatar defect class. Its reconnect reads
  profile_image_url from X v2 users/me, and its connect path reads
  profile_image_url_https out of GraphQL and rewrites _normal -> _400x400.
  My prior session's "noted for later, x's picture silently becomes None" was
  wrong in its cause.
- NEW DEFECT, in the write path the first pass did not reach:
  create_integration's ON CONFLICT arm wrote profile_picture = ? bare, one
  binding before root_internal_id = COALESCE(?, ...) in the SAME statement.
  The connect path therefore did the opposite of the refresh path: refresh keeps
  a value the provider does not supply, connect WIPES one. A provider that
  returned no picture on one exchange destroyed the picture it already held,
  on a reconnect that had nothing to do with avatars. For x/reddit/github this
  is latent (all three DO supply a picture on connect, verified by reading each
  create_integration call), but reachable for any OAuth provider whose userinfo
  read fails after the code exchange.
- NEW DEFECT, larger blast radius: provider.reconnect() is called
  unconditionally by the refresh endpoint, and the trait default forwarded
  fetch_page_info's error with `?`. fetch_page_info is the PAGE-SELECTION
  function, so refusing is its CORRECT answer for a provider with no page
  concept - which made that refusal the answer to the only button the Channels
  page offers to repair a token: "Failed to refresh profile: X does not support
  page management", on a Reddit channel, naming another platform entirely.
- Fix is two halves. (1) Typed ProviderError::NoPageManagement across 13
  sites - a msg.contains("page management") guard at the trait level would be a
  string match that silently breaks the first time someone rewords a message,
  and the 11 pre-existing refusers already word it three different ways. The
  exhaustive match this broke in src/error.rs maps it to a clear 400, not a 500.
  (2) The default reconnect DEGRADES on that variant and returns the channel's
  own identity, so the refresh endpoint's COALESCE write keeps the stored
  name/picture and only clears refresh_needed. A refusal for any other reason
  is still an error.
- Deleted google.rs' reconnect override: a verbatim copy of the default,
  including the refuser it had escaped, so it did nothing the default does not.
- GOOGLE TILE UNIFICATION - answered, not done, and the answer is that the
  token-level merge the operator asked for cannot work. Each family provider
  requests a DISJOINT scope set (gmail: gmail.*, calendar: calendar.*, drive:
  drive.*, youtube: youtube.*), and src/social/google.rs already documents a
  live 2026-10-30 deployment where requesting them together made Google answer
  Error 400 invalid_request "scopes that cannot be requested together", taking
  the entire connect flow offline with no consent screen at all.
  scope_guard::google_scope_conflict now enforces the narrowed set on every
  authorize request. So the tiles are 5 because 5 consent screens are the price
  of the product's actual capability set; merging them would remove Drive and
  gmail.modify, not simplify anything. The credential IS already shared (every
  family provider reads provider_credentials("youtube")), so there is no
  duplicate-client problem to fix either.

Stage Summary:
- Commits: 841688b (threads borrow), ff5e39c (rustfmt follow-up on the
  redirect_uri validator), 362188d (both new defects above). All pushed to
  github/master.
- tests/integration_upsert_picture_test.rs: 2 tests, real SQLite pool, drives
  create_integration twice. Mutation-proven both ways - restoring the bare
  write turns the first RED(left: None) while the "new avatar still lands"
  companion stays green, which is the guard against over-correcting into
  COALESCE-always.
- tests/reconnect_no_page_test.rs: 4 tests. Mutation-proven twice - restoring
  the forwarding `?` turns 2 RED; un-typing ONE refusal (reddit) turns a third
  RED; the fourth pins that the degraded arm is NOT a catch-all, so a real
  outage is never reported as a successful refresh.
- The test's own premise was wrong twice and was corrected each time, not
  forced: google_my_business and wordpress have a WORKING fetch_page_info
  (their Api( arms are genuine fetch-failure paths, not refusals) and were
  dropped from the table; youtube has a working one and its override is fine;
  linkedin_page likewise. The table now holds exactly the 13 outright refusers,
  and the test re-checks each entry's premise so a provider that grows pages
  fails loudly instead of silently passing.
- Honest gap, unchanged: the avatar fix is verified by invariant + mutation,
  not end-to-end. The operator's live DB has no Google/gmail/youtube row at
  all - they have never completed a Google OAuth - so there is no row to repair
  and no way to observe the fix from their side yet.
- Known issues: PagePicker.svelte still breaks on the first multi-account error
  instead of continuing the batch. Pre-existing failures untouched:
  surface_docs_audit trio, clippy in autopost.rs + oauth_browser.rs, README
  length_in_band, unstaged scripts/genome.

---
Task ID: v26-page-picker-batch-and-x-avatar-guard
Agent: Jcode
Task: close the two follow-ups the previous round left open: the PagePicker
  multi-account abort, and the x-cookie avatar verification

Work Log:
- PagePicker.svelte's batch loop ended at the first failure with `break`,
  abandoning every page after it. The founder selects five, the second is
  refused, and the three after it are never attempted - they stay
  unconnected with no signal. `onSuccess` was gated on the same
  `!errorMsg`, so a page that DID connect was still offered as selectable
  and the list behind the modal never refreshed.
- Fixed: collect failures, continue, report them together ("Connected 3 of
  5; failed to connect a, b"), and refresh either way. A partial success is
  still a success for the channels that connected.
- tests/page_picker_batch_test.rs holds three source-invariant tests (the
  picker is browser-side with no API a unit test drives): the `continue`,
  the plural reporting, and the ungated refresh. Mutation-proven -
  restoring the original loop turns all three RED, each naming the
  operator-facing consequence.
- My third test's first draft was wrong twice before it was right and both
  corrections were mechanical, not cosmetic: an `rsplit(...).count()` proxy
  that asserted a count rather than the property, then a
  `.split(marker).nth(1)` that returned everything AFTER the marker
  (including the unconditional `onSuccess` at the end) so the assertion
  could never be true against any source. Rewritten to take the branch
  body between the opening brace and the `selected = new Set();` that
  follows the branch close.
- x-cookie avatar: the true end-to-end is BLOCKED, not unverified-by-choice.
  connect_x_cookie POSTs to x.com with the caller's own session and has no
  injection point, and no local browser profile (chromium, Brave) is logged
  into X, so there are no cookies to drive it with. The browser tool's
  bridge is also not responding, so the operator's signed-in tab is
  unreachable this session.
- Pinned what IS reachable instead: the connector's built URL survives the
  write AND the IntegrationPublic serialization the page reads. Field name
  corroborated internally (x.rs:1541 reads the same
  profile_image_url_https for author avatars), not by a guess.
- AND labelled honestly what that test is: a GUARD, not a mutation-proof.
  It exercises only the INSERT arm, so it stays green when the ON CONFLICT
  write regresses. Proven by reverting the COALESCE and watching the other
  two in the same binary go RED while this one stayed green. That result
  is recorded in the commit rather than hidden.

Stage Summary:
- Commits: 8c12e81 (PagePicker batch), 8dd9edc (x-cookie round-trip guard).
  Both pushed to github/master.
- Tests: page_picker_batch_test 3/3 (mutation-proven),
  integration_upsert_picture_test 3/3 (2 mutation-proven, 1 recorded guard).
- Frontend: svelte-check 0 errors 0 warnings; pnpm build succeeds.
- Both open follow-ups from the previous round are now closed. Nothing is
  left open from the five original reports.
- Known-untouchable pre-existing failures unchanged: surface_docs_audit
  trio, clippy in autopost.rs + oauth_browser.rs, README length_in_band,
  unstaged scripts/genome.

---
Task ID: v26-validation-pass
Agent: Jcode
Task: independent validation of every fix landed this cycle, on the principle
  that a test which has only ever passed has not been checked

Work Log:
- Ran each fix's own test and then tried to break it. Five fixes,
  re-validated. Three were sound; TWO of their tests could not fail and
  are now fixed.

- avatar_repair_path_test could not fail on the same defect rewritten one
  line differently. It asserted the literal `picture: None`, which is what
  the pre-fix code happened to look like. I rewrote calendar's fix the way
  I would have written it originally - bind the picture to a local, pass
  the local - and the test passed on code whose success arm permanently
  answered "no avatar". Mutation-proved the old test's blindness, then
  rewrote the assertion as what the success arm DOES (reaches a real
  picture reader) instead of one spelling of the failure. Both the
  local-binding and a `Some(String::new())` that is not the shared reader
  are now caught.

- My rewrite's first version had the mirror-image error: it demanded the
  OpenID userinfo reader for the WHOLE Google family and flagged google,
  youtube and drive. google/youtube are correct - a channel's face is the
  YouTube channel thumbnail from youtube/v3/channels, which is a better
  avatar than the account photo. drive reads `user["picture"]` inline,
  which is a duplication the sibling test flags but NOT a missing
  avatar. So the demand was a different rule, not a stricter one. All
  three are named with their reason in PICTURE_SOURCE_OVERRIDES. The
  lesson: a strengthened check that overreaches fails working code and
  teaches the reader to distrust the test.

- channels_grid_multi_account_test is new (the multi-account tiles fix had
  NO test) and its first version passed the mutation it was written to
  catch. The filter I reintroduced sat on its own line inside a
  multi-line `$derived`, and the check read only the opening line. A
  line-scoped scan of a multi-line statement. Fixed by taking the whole
  statement to its terminating `;\n`; the mutation is now caught with the
  operator-facing message.

- connect_flow_redteam_test re-proven: restoring the old
  "Page 'X' is already connected" 400 turns BOTH re-connect tests RED with
  the operator's exact error text. And the validator's surface is complete
  - traced every provider's `redirect_uri` and confirmed it is always
  passed IN by the caller, so the four validated call sites are the whole
  reachable set and there is no unvalidated path building an authorize
  URL.

- Provider list re-measured from source, not from the test that asserts
  it: 30 `providers.insert` identifiers including farcaster (Tier-3,
  env-gated) => 29 always registered, matching the documented 26 +
  linkedin-page + instagram-standalone.

- Also confirmed, for the record, that the earlier single-binary
  failing count of 2 was a parallel-execution collision and not a
  regression: connect_flow_redteam_test passes 20/20 in isolation and
  across repeated full runs, and its per-binary SQLite file is the
  collision AGENTS.md 9.42 describes.

Stage Summary:
- Commit b4d007b pushed. Two tests strengthened (one of which was newly
  written and immediately mutation-proved broken), three re-proven sound.
- The value of this pass: two of this cycle's fixes had tests that would
  have passed on the original defect's code and on a trivially rewritten
  version of it. Those tests were decoration. They are not now.

---
Task ID: v27-validation-pass-2
Agent: Jcode
Task: second independent validation pass over the cycle's fixes, per the
  "a test that has only ever passed has not been checked" rule

Work Log:
- Traced the `ProviderError::NoPageManagement` introduction across every
  consumer of the new variant, looking for a wildcard match arm that
  would now route it where `Api` used to go. There is none: the circuit
  breaker's `is_platform_outage` matches three NAMED variants, so the new
  variant is not miscounted as a platform outage - which is itself the
  correct outcome, because the old `Api` string WAS miscounted and the
  doc comment above that function says local refusals say nothing about
  platform health. The variant's Display string has no other consumer,
  so nothing else depended on the old text.

- Found and fixed a stale rustdoc that named an error which no longer
  exists: the default `reconnect`'s doc still described providers
  refusing with `Err(Api("X does not support page management"))`, the
  string form replaced by the typed variant two commits earlier. All 14
  refusal sites confirmed inside `fetch_page_info`. The first version of
  my correction quoted the wrong message too - I wrote what the Display
  string implied instead of reading `error.rs`, which maps the variant to
  "This provider has no page or sub-account concept. The channel is the
  account." Now the real string, so a grep finds the code.

- Redirect_uri: verified with a REAL server and a REAL HTTP request that
  the fix works and what it says. `?redirect_uri=https%3A%2F%2Fevil.test%2Fcb`
  answers 400 with "redirect_uri https://evil.test/cb is not this
  deployment's callback. The callback URI every provider registers is
  http://localhost:6599/api/auth/callback (built from APP_URL). Omit
  redirect_uri to use it." - which surfaced an operator-facing grammar
  defect ("every provider register are" / "every provider register is")
  in both refusals in `resolve_redirect_uri`. Fixed. The tests pin only
  the substring "is not this deployment's callback", so no test asserted
  the typo.

- Redirect_uri, and the most important finding of this pass: the three
  router-level tests could green without ever calling the function under
  test. The connect handler checks credentials at
  src/api/integrations.rs:71 BEFORE it validates `redirect_uri` at :88,
  so on a box with no `INSTAGRAM_CLIENT_ID` all three return early. Here
  `.env` (gitignored) supplies credentials so they run, but a clean CI
  checkout would show 20/20 green having exercised nothing.

  My first attempt to close it counted the skips and asserted on the
  count from another test - which is order-dependent, because cargo runs
  tests on threads, so it would flake and then be deleted. The real fix
  is `the_redirect_uri_rule_is_exercised_without_any_provider_credentials`,
  which calls `resolve_redirect_uri` directly and therefore runs in
  EVERY environment. Six cases pinned. Mutation-proved both directions:
  honour-verbatim is RED on it and on the two router tests, and
  disabling `same_callback` is RED on it and on the foreign-host one.

  Two of my six cases were wrong on first write, and both were caught by
  running them rather than reasoning: I asserted a doubly-quoted value
  was accepted (it is REFUSED - one layer strips, the remainder is
  quoteless gibberish), and I asserted an empty `redirect_uri` was
  refused (it is the documented "no opinion" default, which is correct
  behaviour - the quoted-but-empty `""` is the refused case, and it is a
  different rule). The second is the more instructive: I would have made
  a working path refuse, and the "stricter" check would have shipped a
  dead-end for every client that zeroes a field.

- profile_name: found and fixed an UNPATCHED instance of the exact bug
  class Phase B fixed. Proven live before the change - see commit
  cd76c4e. A seeded row with name 'u/cooluser r/NICE SUB' and a real
  avatar came back from POST /api/integrations/:id/refresh with the
  avatar intact and the name ''. The refresh write guarded
  `profile_picture` with COALESCE and wrote `profile_name = $1` bare,
  and `reconnect`'s DEGRADE arm (the thing Phase B added) returns an
  empty name - so the picture guard was the only thing hiding it. The
  write looked protected; half of it was.

  And the test for it PASSED its own mutation on first write. I added
  COALESCE to the SQL while binding the raw `info.name`, making the fix
  entirely inert, and the SQL-only assertion went GREEN. Third time this
  cycle. Fixed by asserting on the BIND positively. Both mutations now
  RED.

Stage Summary:
- Commits b310b18, cd76c4e pushed.
- One NEW live-proven operator defect fixed (profile_name wipe), same
  bug class as the Phase B avatar fix, one column away in the same
  statement.
- One real test-honesty hole closed (redirect_uri needed no credentials
  before it would run at all).
- One stale doc fixed; one operator-facing grammar defect fixed, both
  in messages a founder reads mid-incident.
- Three of my own tests failed their own mutation on first write this
  cycle. The common shape: I assert on the SPELLING of the fix rather
  than the BEHAVIOUR it produces, and the mutation keeps the spelling
  while breaking the behaviour. Every one was found by running the
  mutation, not by reading.
- Honest gaps unchanged: the avatar repair path has never been verified
  end-to-end against a provider that actually supplies an avatar, and
  the x-cookie avatar path is blocked on credentials. The profile_name
  fix IS end-to-end verified (HTTP 200 -> read the row back).

---
Task ID: v27-failure-census
Agent: Jcode
Task: reconcile the full `cargo test --no-fail-fast` failure count, which is
  now 2 binaries and not 1

Findings:
- `surface_docs_audit` (4/4 FAILED): unchanged, pre-existing, and known. It
  reads `providers.md`, a path that no longer exists at the tree root.
- `x_transport_test::test_x_headers_bisect` (1/4 FAILED): NOT previously
  recorded, so it is worth a note rather than a shrug. It is a LIVE test
  against the real X GraphQL API using plain `reqwest`, with real
  `X_AUTH_TOKEN`/`X_CT0` from `.env`.
  Verified environmental, not a code defect: `curl https://x.com` returns
  200 from this machine, while `curl` on that same GraphQL URL returns 403
  - X blocks non-Chrome TLS fingerprints, which is the entire reason the
  repo pins `wreq` with Chrome fingerprinting for its real X path. A test
  that drives the same API with the client that gets blocked will fail
  wherever the fingerprint is rejected. None of my commits touch X
  transport, header handling, or `wreq`.
- So the honest count is 2 failing binaries, both environmental/pre-existing,
  and neither is a regression from this cycle's work. 541/541 lib, and all
  six binaries this cycle touched are green.

---
Task ID: v28-connect-page-ordering
Agent: Jcode
Task: validate the connect_page idempotency fix by reading its own claim rather
  than trusting the test that covers it

Findings:
- The fix's comment and its covering test's comment both claimed a repeat
  connect survives a provider outage. Neither was true of the code:
  `pages()` was called UNCONDITIONALLY, after the lookup and before the
  `if let Some(id)` branch. The claim was true of the ordering and false of
  the handler.

- WHY the covering test never caught it, which is the transferable part:
  `google`'s `pages()` answers an EMPTY LIST for a bad token rather than an
  error, because the YouTube endpoint returns a JSON body with no `items`
  and the reader maps that to zero pages. The call that was supposed to
  fail during an outage silently succeeded. A test asserting prose, sitting
  beside a provider whose failure mode cannot be triggered from CI, is a
  test that cannot fail.

- Fixed by gating the fetch (`if already_connected.is_some() { Vec::new() }`
  else the provider call). First connect is unchanged: the `ok_or_else`
  below the branch still requires the page.

- Live before/after, with pages() pointed at an unroutable host:
  pre-fix HTTP 502 "Failed to list pages: Network error"; post-fix HTTP
  200 with the profile name intact and one row (no duplicate). The
  recovery path was closed precisely when it was needed - a retry after a
  connect failed halfway failed on the same broken path.

- New test `a_repeat_connect_does_not_fetch_pages_from_the_provider`.
  Mutation-proven in the way that matters: reverting to the pre-fix shape
  (fetch unconditionally, then discard on the repeat path so the
  observable behaviour is otherwise identical) turns it RED while all 21
  other tests in the binary stay GREEN. That is the shape of a defect
  surviving a green suite.

- Also checked, and deliberately NOT changed: `onboard.rs:617` and
  `integrations.rs:71` check credentials before validating `redirect_uri`.
  Reordering would trade an accurate, actionable error ("set these env
  vars") for a less useful one, because an unconfigured provider cannot
  connect at all. The real gap was that the rule could go unexercised,
  and commit b310b18's direct test closes that. The MCP call site has no
  credentials gate before validation, so it has no such issue.

Stage Summary:
- Commit 8e34926 pushed. One more live-proven defect fixed, found by
  reading the fix's own claim instead of its test.
- Frontend: `pnpm build` succeeds, `svelte-check` 0 errors.
- Full suite green except the two documented environmental failures
  (`surface_docs_audit`, `x_transport_test::test_x_headers_bisect`).
- Total this cycle: 4 fixes to defects found while RE-VALIDATING earlier
  work (profile_name wipe, redirect_uri test-honesty, stale doc naming a
  removed error, and this one), 3 of my own tests caught passing their own
  mutations, and every claim re-proven by a before/after on a live server
  or a mutation that compiles.

---

Task ID: v10-cycle-error-message-erased
Agent: Jcode
Task: sweep for the recurring COALESCE-guarded/one-bare bug class and prove a fix for every instance

Work Log:
- The cycle had found the class twice already (`profile_name` written bare beside a COALESCE-guarded
  `profile_picture`; a state write with guards missing entirely). I searched for the SHAPE rather than
  the two known sites: every UPDATE with a COALESCE in it.
- Six candidates matched the regex. Read all six rather than trusting the match:
  - `update_integration_token` and `update_user_preferences` - the documented three-state
    `CASE WHEN ?` pattern of AGENTS.md §9.12. Not defects; they correctly distinguish
    absent/null/explicit-clear.
  - `update_post_state` - FOUR of five value columns guarded, one bare. THIS WAS IT.
  - `reset_post_for_republish` - bare `error_message` is CORRECT, it is the deliberate clear.
  - `mark_post_error` - same, deliberate.
- Traced the caller to see whether the bare form was actually reachable with something to say.
  All THREE callers pass `None`, so the parameter could only ever erase. The erase reached a
  live error message via the claim-release path: a manual publish of a thread whose predecessor
  is not published skips, then releases the claim with `state = post.state.clone()` (Error, allowed)
  and `error_message = None`. Post stays `error` and loses its reason.
- Wrote `tests/post_state_error_message_test.rs` with three assertions: the failure reason survives
  the claim release, a never-failed post does not acquire one, and the deliberate
  republish clear still works. The last two exist because `COALESCE(?, error_message)` without
  them would be a different bug - a sticky column nothing can clear.
- Verified the fix is not the wrong size in the other direction: the four deliberate writers
  (`mark_post_error`, `mark_thread_rest_failed`, `reclaim_stuck_publishing`,
  `reset_post_for_republish`) still write outright and are unchanged.
- Proved the test could actually fail: removed the COALESCE, re-ran. The claim-release test went
  RED with `left: None`, the two companions stayed GREEN. That is the proof that the assertion
  targets the erase and is not over-fitted to it.
- Confirmed the conversion to the runtime query form did not break the offline build in a new way:
  stashed the commit, re-ran `SQLX_OFFLINE=true cargo check --lib`, and got the identical single
  error at `queries.rs:119` (`create_integration`, a `query_as!` I did not touch). Pre-existing,
  not a regression from my change.

Stage Summary:
- Fixed `src/db/queries.rs::update_post_state`: `error_message = ?` is now
  `COALESCE(?, error_message)`. The fourth write this cycle to erase data through an unguarded
  column, and the first one that leaves the operator with an unexplained failure.
- Converted the macro to the runtime form (AGENTS.md §5.2) because the COALESCE arm changes the
  column list and this environment has no live DB to regenerate `.sqlx/`.
- New binary `tests/post_state_error_message_test.rs`, 3 tests.
- Known follow-ups: the SQLX_OFFLINE error at `queries.rs:119` is untouched by this work and needs
  a `cargo sqlx prepare` run somewhere with a database; two environmental test failures remain
  (`surface_docs_audit`, `x_transport_test::test_x_headers_bisect`).
- Also swept `pages()` for a fourth instance in the same family: called unconditionally on a repeat
  connect, so a provider outage showed the founder a 502 while their working channel was right
  there. That one is `cd76c4e`.
- Recurring lesson, now four times: the guard exists for the columns someone was bitten by, and
  is learned for the rest by the same commit only if the commit author lists the columns. My own
  tests keep passing their own mutations when I assert the spelling of a fix rather than its
  behaviour - caught by RUNNING the mutation, never by reading it.

---
Task ID: ui-parity-m1
Agent: Jcode
Task: Phase 4 (M-1) of `docs/planning/PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md` — the 6 pre-existing
       `/posts/new` probe FAILs at 768px, reported as the appended signature text overlapping
       Preview / Save Draft / Post Now.

Work Log:
- Measured the composer open at 1440 / 768 / 390. Reproduced the 6 FAILs; every one named the
  signature element (text starting "— Ish…") against a footer button.
- Traced the signature's ancestor chain from the real DOM: `.editor-body` (224px) is inside
  `.rich-editor` (`overflow: hidden`) inside a column with `scrollHeight 1106` over `clientHeight 674`.
- Hit-tested the three footer buttons with `document.elementFromPoint` at each centre: every one
  returned that exact button. Nothing covers them; the composer is fully usable with the signature
  loaded.
- Scrolled the editor column to its end: the signature moved from `y=731..955` to `y=299..523`,
  clear of the footer at `785..854`. The overlap is a PAINT difference, not a layout one.
- Read `frontend/scripts/probe.mjs`'s overlap check and found the cause: it filtered candidate boxes
  by VIEWPORT bounds only, using the raw `getBoundingClientRect()`. A clipped child of a scroller
  or an `overflow: hidden` box keeps a rect larger than the pixels it paints, so 170px of phantom
  rect reached across the footer. The existing comment shows the intent ("a box scrolled out of the
  viewport still has a rectangle") — the fix was written for the viewport case and never extended to
  clipping ancestors.
- Added `clippedRect()` to the probe: walk each ancestor, and for any whose `overflow` is not
  `visible`, intersect the box with that ancestor's visible band. Then the viewport clip.
- Verified BOTH directions, deliberately, because a gate that swallows real FAILs is as useless as
  one that invents them: an isolated harness asserted the signature's clipped box overlaps the footer
  by 0px (so the false positive is gone) while two fully visible boxes overlapping 50x25 are still
  reported.
- Hit the coding pitfall worth recording: a backtick inside a `//` comment broke the module load with
  `SyntaxError: Unexpected identifier`. `node scripts/probe.mjs` is the load check, not `pnpm build`.
- Ran the full probe over 27 routes x 4 widths x 2 themes: **FAIL 0**, WARN 216, unchanged from the
  run that reported 6 FAILs. Washed the 8 console 400s: all are `GET /api/dms/conversations` for the
  cookie-auth X channel, which X's DM API does not support. The server answers with an accurate,
  actionable message and `/dms` already renders it inline. Documented, not "fixed".

Stage Summary:
- `02d340a` (pushed to `github/master`): the probe's overlap check is now clip-aware.
- The composer itself was NOT wrong. No production frontend or Rust file changed.
- Probe result: FAIL 0 over 27 routes / 4 widths / 2 themes. WARN 216 (104 `/tags` swatches, 112
  `/calendar` tap targets inside the 24-44px advisory band) — pre-existing, zero FAILs.
- Plan note `5324324` is now also in the record for this phase; Phase 4's five structural changes
  (tab swap, scrim, channel-switch row, editor reorder, one footer primary) remain untried.
- Lesson, twice now in this plan: before "fixing" what the gate reports, prove the gate is measuring
  the thing a visitor sees. `elementFromPoint` plus scrolling to the element's own extent did that
  here in ten minutes; the whole of Phase 4 as originally scoped would have been spent rearranging a
  composer that was already correct.


---
Task ID: ui-parity-m1-pass2
Agent: Jcode
Task: Phase 4's five structural bullets, after the probe false positive was cleared (see the ui-parity-m1
       entry above).

Work Log:
- Walked each bullet against the tree rather than implementing it from the plan's description. FOUR of the
  five were already built or stated wrongly:
  - the `<lg` preview tab already existed (`showPreviewMobile` swaps the pane to `block`/`hidden lg:block`)
  - the composer's scrim was ALREADY `bg-overlay`, not `bg-overlay-strong` (`ModalManager.svelte:103`)
  - `SelectCurrent` was ALREADY inside the editor column, directly above the Content Editor
  - the remaining two were real defects
- Measured the real ones. Footer: `document.querySelectorAll` counting buttons whose computed
  `backgroundColor` is not transparent returned **2** at 1440, 768 AND 390 - `bg-success` green Post Now
  and `bg-accent-fill` indigo Schedule, both claiming the same weight. Editor column: five cards stood
  above the Content Editor.
- Fix A (`cd2f9c2`): Schedule became the one filled primary; Post Now became the bordered secondary,
  kept VISIBLE (the plan's chevron dropdown would bury it). Verified: 1 filled button at all three widths.
- Fix B (`e2604b9`): Tags + Campaign merged into one labelled `Details` card; both controls verified
  working in the browser (six tag chips, click sets `aria-pressed` and shows the tick; campaign select
  keeps all four options). Gained a `<label for="composer-campaign">` the select never had.
- MEASUREMENT TRAP, hit twice and worth writing down: the first contrast pass reported the dark palette
  for a LIGHT-theme request. The theme is a class on `documentElement` applied by the store from
  `localStorage`, so a context that never seeds it renders unthemed defaults. AGENTS.md 9.36 names this
  exactly. Fixed by `addInitScript` on the store's own key plus a `waitForFunction` on the class.
- Did NOT claim a contrast win. Checked the old pairing arithmetically: `bg-success` + `text-success-fg`
  was already 7.25:1, because `success-fg` is the dark-on-green partner of the green fill. The footer
  change is a hierarchy fix, and the commit says so.
- Probed both routes after each change: FAIL 0.

Stage Summary:
- `02d340a` probe clip fix, `10dc9c3` plan note, `cd2f9c2` footer primary, `e2604b9` Details card. All
  pushed to `github/master`.
- Phase 4 (M-1) is now at: FAIL 0 across 27 routes, four bullets resolved as no-ops, two real defects
  fixed and measured.
- Recursed again on the same theme as the previous entry: THREE of the plan's five Phase-4 bullets
  described work that was already done. The plan was written from a competitor analysis, not from this
  tree. Reading the file before "fixing" it is cheaper than the fix.
- Remaining, unowned: the composer's `color-sprawl` advisory - 24 distinct text colours on one surface.
  That is the real remaining Phase-4 defect and it needs its own pass.

---
Task ID: ui-parity-m1-color-sprawl
Agent: Jcode
Task: The composer's `color-sprawl` advisory - "24 distinct text colours" - which the previous entry flagged as
       the composer's real remaining defect and the only Phase-4 item needing a named owner.

Work Log:
- Enumerated the composer's text colours in the live DOM rather than trusting the number. The app assigns
  FIVE real token colours (content, muted, faint, error, accent-fg) plus tag/campaign swatch ink.
- Traced the swatch ink to its source: `TagPicker.svelte:51` paints
  `style="color: {inkOnFill(tag.color)}"`, the per-chip computed ink from `lib/chip-colors.ts`, which exists
  precisely because the hue is the OPERATOR's choice and its luminance is not the app's to control.
- So the advisory was measuring the founder's colour input, not the type system. It could never name a real
  defect, and it was the last thing standing between Phase 4 and FAIL 0 / WARN 0.
- Amended the count: exclude text whose own element sets its colour inline; count inherited chip colour
  against the child. Closed the hole it would otherwise open by pointing at check-tailwind-classes.mjs and
  the hex gate in `make check`, which read the SOURCE and catch a hardcoded inline literal.
- Verified three directions, because "turned it off" and "fixed it" look the same from outside:
  18 stylesheet colours -> still warns; 6 inline chip inks -> 0; 17 inline swatches + 2 real tokens -> 2.
- Probed: FAIL 0 / WARN 0 on /posts/new and /posts; full 27-route run FAIL 0, WARN 216 unchanged.

Stage Summary:
- `0835545` pushed. Phase 4 (M-1) is now FAIL 0 / WARN 0 on the composer routes.
- TOOLING MISTAKE worth recording: my first attempt to apply this used a Python heredoc that un-escaped
  every backtick in probe.mjs, because that file stores its template literals ESCAPED. It corrupted 27
  sites and I only found it because `node --check` failed. Restored with `git checkout --` and redone
  through the `edit` tool, which shows the escaped bytes it matches. A script that rewrites a template
  cannot be trusted with that template's own escapes, and the `edit` tool is the safe path here.
- The whole of M-1 resolved as: 6 probe FAILs that were the probe's own measurement (`02d340a`), four
  Phase-4 bullets that were already built, two real defects (`cd2f9c2`, `e2604b9`), one production defect
  that was the probe again (`0835545`). Three of the four were the plan describing work that was done.

---
Task ID: ui-parity-phase5a
Agent: Jcode
Task: Phase 5a of the parity plan - the global nav rail defaults to 80px instead of 224px.

Work Log:
- Measured the rail FIRST, at 1440 and 768, in both states: aside 224px expanded / 56px collapsed, 16
  links at 44px expanded but **40px collapsed**, 5 section headings, no hover flyout.
- Found a comment promising a hover flyout when collapsed and grepped for it - it has never existed. Deleted
  the promise rather than build it (the header toggle is the intended interaction).
- Made the width change: `w-20` collapsed default, `w-56` expanded, state renamed to `sidebarExpanded`
  so the persisted thing is the preference. Migrated the old `switchpost-sidebar-collapsed` key on read.
- MEASURED, and my reasoning was wrong: collapsed links were **40px**, not 44. With the label removed the
  row's only flex child is the 16px icon, so 12+12+16 = 40. The 20px line-height that made 44 needs text.
  Fixed with `min-h-11` on every link.
- Ran the full probe: **FAIL 0 -> 216**. The "Expand sidebar" button was **20px wide** - under the 24x24
  WCAG 2.2 floor, and at 56px the surrounding space had been hiding it. `min-w-11` restored FAIL 0. A
  regression the probe caught in 6 minutes that a source read would never have seen.
- Wrote a comment claiming the timezone "lives on /settings" and the theme toggle has "a second home in the
  top bar". BOTH FALSE - grepped and found neither. That was the third false claim in one file. Corrected
  the comment AND closed the real gap: added a labelled Display card on /settings with the timezone, both
  directions verified (settings writes America/New_York and it persists; the sidebar writes Europe/London
  and /settings reads it back).
- Verified the final state at 1440 and 768: aside 80px collapsed / 224px expanded, 16 links at 44px in
  BOTH, theme + logout reachable in both, the old key migrated to the new one.
- Corrected Phase 5b's acceptance numbers, which divide by a rail width the plan never shipped and were
  already impossible before 5a (see below).

Stage Summary:
- `fd7cdab` rail 80px default, `c7f156a` plan note, `d511ee6` 5b acceptance correction. All pushed.
- Probe: FAIL 0 over 27 routes x 4 widths x 2 themes; WARN 216 unchanged and the same pre-existing set.
- Pattern that has now repeated five times in this plan: the description of a thing in this repo was wrong,
  and reading the file before changing it was cheaper than the change. The flyout comment, four Phase-4
  bullets, one of my own comments, and now two acceptance numbers.
- Lesson recorded in the plan: **an acceptance number is a claim about a tree.** Phase 5b's `>= 1296px` and
  `>= 1116px` were computed against a 224px rail; with 5a's measured 1360px the real figures are 1296
  (satisfiable, it is the default) and 1100 (a stated trade, 16px worse than the plan imagined).

---
Task ID: ui-parity-phase6
Agent: Jcode
Task: Phase 6 (M-2) of the parity plan - per-channel date and target view.

Work Log:
- Read before writing: `PostOverride` already carried per-channel content and
  settings, keyed by integration id, and the create path only took that branch
  when `overrides` was present. That is the natural home for a per-channel
  time, so no new plumbing was needed on the write side.
- Added `scheduled_at` to `PostOverride`, and resolved each integration's own
  time in the create loop, falling back to the shared one. The scheduler reads a
  post's column, so each row carrying its own is what makes it real.
- Composer: a `channelSchedule` map and a `scheduleInView` derived. ONE
  SchedulePicker reads the channel in view; switching tabs re-targets the same
  component instead of duplicating it per channel.
- Hit the E0063 on the response probe (the struct literal) and read where it
  lived: the UPDATE handler's validation probe, not a detail read. My comment
  about the "detail response" was wrong and I corrected it rather than leaving
  the fiction.
- Verified the API directly: one POST, shared 09:00 + a Reddit override of
  17:00, returns X at 09:00Z queued and Reddit at 17:00Z queued.
- VERIFICATION FOUND TWO REAL DEFECTS, and both are the point of this entry:
  1. the autosave wrote the draft WITHOUT the per-channel map - the exact
     AGENTS.md 9.14/9.29 shape. A custom 17:00 reverted on reload while the UI
     said "Autosave on". Both halves now present.
  2. "Save as Draft" stripped the shared time but not the per-channel ones, so a
     draft save left a row QUEUED for 17:00 while the toast said "Draft saved".
     The draft path now drops only the time.
- Drove the composer in a real browser: switching to Reddit's tab renders
  "Schedule for Reddit custom", the time shows in the app's zone (23:30 UTC
  reads 05:00 in Asia/Calcutta), and the "Use shared" undo appears. Captured the
  wire payload to confirm the override reaches the request.
- Wrote `new Date(...)` in the divergence list, grep'd it out for
  `formatDateTime` (AGENTS.md 9.17/9.30) - caught before it shipped.
- A Python heredoc was used to patch two identical save sites. It did NOT
  corrupt anything (svelte-check confirmed) but it is the same tool that
  corrupted probe.mjs earlier this session, and the preference is `edit`.

Stage Summary:
- `764e9ca` Phase 6, `eda6828` plan note. Both pushed.
- The acceptance's headline is met at the layer that matters (the API persists
  both times) and the UI that collects it is proven.
- Two named scope limits, stated explicitly rather than left implied: EDIT mode
  cannot change a per-channel time (UpdatePostRequest has no scheduled_at by
  design), and a full submit with a mouse-picked Reddit subreddit was NOT driven
  because the required-settings check correctly refused it every time.
- cargo test --lib 541/541; svelte-check 0/0; probe FAIL 0 / WARN 0 on the
  composer routes.
- Nine commits landed this session: 02d340a, 10dc9c3, cd2f9c2, e2604b9, 9652d79,
  0835545, 9ccbb3b, fd7cdab, c7f156a, d511ee6, 4ce2ef3, 764e9ca, eda6828.

---
Task ID: outline-token-refresh-gaps
Agent: Jcode
Task: /ponytail-style outline of remaining implementation gaps, plus a
red-team/audit of token auto-refresh across ALL providers, prompted by
LinkedIn channels showing "needs reconnect"

Work Log:
- Pinned the two line numbers the previous pass could not get. The greps that
  failed on `linkedin.rs` failed identically on the other two files; listing
  `async fn` per file instead worked immediately. `instagram_standalone.rs`'s
  `refresh_token` is at :192 and `threads.rs`'s at :186.
- Completed the sweep on the remaining cron providers (x, reddit, tiktok,
  google, youtube, drive, gmail, calendar): zero profile/userinfo
  references inside any of their `refresh_token`, so the fatal-fetch shape is
  confined to the three found.
- Confirmed the scheduler's failure arm at `src/scheduler/mod.rs:561-575`:
  ANY `Err` from `refresh_token` becomes `mark_integration_refresh_needed`,
  which is what the channel UI renders as "needs reconnect".
- Wrote `docs/planning/OUTLINE_TOKEN_REFRESH.md` (G-01 through G-07) and
  pushed it as `0aeac1f`. Nothing in it is applied - it is the queue.
- The push reported the outward-boundary check going from 1 to 2 hard
  failures, and my own new doc was one of them. D003 wants a resolved
  markdown LINK; adding the row to AGENTS.md's §0.6 DOCUMENT MAP did not
  clear it because that check only reads `docs/**/*.md` and the README.
  The linking doc is `docs/PAGE-INDEX.md`, which is also the index AGENTS.md
  tells you to read first. Routed through both, pushed as `ae35795`; the
  boundary is back to 1 hard failure, and that one is
  `PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md` from `eda6828`, not mine.

Stage Summary:
- `0aeac1f` (the outline) and `ae35795` (the routing fix) are on
  github/master.
- The reported bug is real and its cause is NOT the refresh mechanism:
  `linkedin.rs` refreshes successfully, returns a valid access token and
  parses `expires_in`, then fetches `/v2/userinfo` and propagates that
  call's failure with `?`. The 24h window query excludes the marked row
  forever, so the channel reconnects by hand while a good token is discarded.
- The correct behaviour is already in the tree one file over:
  `linkedin_page.rs` reads the same endpoint as `Ok(r) => r.json().await.ok()`
  with a comment saying page-scoped tokens may not carry openid/profile. A
  recorded design decision present on one LinkedIn provider and absent on the
  other.
- Two more sites: `instagram_standalone` (:192) and `threads` (:186).
- Priority argument recorded rather than assumed: G-05 (`resolve_token`,
  scheduler/mod.rs:1824) runs the SAME provider call on the pre-publish path
  and propagates its `Err`, so the same defect drops a published post rather
  than just flagging a channel row.
- Two false alarms written into G-07 with their disproof, so a future reader
  does not re-spend the hour: the "8 of 12 providers never set expires_in"
  grep (the Google family keeps it in the shared `google_oauth::refresh`, and
  all 12 propagate it) and the "scheduler decrypt fallback sends ciphertext"
  claim (`maybe_decrypt_token` has the identical `unwrap_or_else`). Both came
  from reading a grep result instead of the returned `AuthToken`. That is the
  same lesson as the five earlier retractions this cycle, and it is why every
  verdict in the outline is a file-and-line rather than a pattern.

---
Task ID: outline-remaining-gaps-v2
Agent: Jcode
Task: produce the outline of remaining implementation gaps so the operator
can approve the remaining execution

Work Log:
- Read the register first (TODO.md: 17 open rows, 2 blocked) and the phase
  plan (PHASES_P0-P9: P0 through P9 all done, so no phase work remains). The
  queue is therefore the register's open rows plus the two outlines' gaps,
  not a phase plan.
- Synced with github/master first (b7cf00a) - two commits had landed since
  my last push, both frontend/probe. Writing an outline against a stale HEAD
  would have produced a list that disagreed with the tree.
- Pinned the three token-refresh sites by reading each file rather than
  grepping. This is the same failure the previous pass hit: the greps that
  returned "no matches" on instagram_standalone.rs and threads.rs kept
  returning nothing on re-run, and listing `async fn` per file resolved all
  three immediately. Final pins: linkedin.rs:728-735,
  instagram_standalone.rs:210-220, threads.rs:204-214, and the good idiom at
  linkedin_page.rs:252-262.
- Read the repo's own precedent for the test-shape question:
  tests/linkedin_e2e_test.rs:933 reads a provider's source and pins its
  arrangement (checks `resp.status()`, reads `resp.text()`, never
  `resp.json()` directly). That is the precedent the token-refresh test
  should follow, and the reason is a real constraint rather than a
  preference - the provider constructors take only `&Config`, every URL is a
  hardcoded `&'static str`, and there is no mock-server dependency in the
  tree, so a test cannot point LinkedIn at 127.0.0.1.
- Found the UI dependency order that the register does not carry: UI-C2
  before UI-C1 (the `+N more` affordance has to live inside whatever height
  C2 decides), and UI-M1 before UI-M2 (M1 is M2's stated prerequisite).
- Re-measured the two follow-ups the previous outline left open and both are
  closed: `.sqlx/` has 60 entries and `SQLX_OFFLINE=true cargo check --lib`
  exits 0 (committed at 278e742), and the D003 orphan closed with ae35795.
  Recorded both in §7 with their evidence so neither is re-opened.
- Wrote docs/planning/OUTLINE_REMAINING_GAPS.md (340 lines) and routed it
  through PAGE-INDEX.md and AGENTS.md §0.6 in the SAME commit - D003 wants a
  resolved link, not a routing row, which is the lesson from ae35795.

Stage Summary:
- `5834db6` on github/master. Nothing applied - it is the list awaiting
  approval.
- 15 items: 3 `now` (the token-discard fix, its helper, and the pre-publish
  half), 7 `next`, 2 `deferred`, 1 `blocked` (the Google grouping, which is
  a platform constraint), and the credential-gated set.
- The Google-Suite grouping is recorded as a PLATFORM constraint, not a code
  mess: Google refuses YouTube+Drive and YouTube+calendar.events before
  consent (live 2026-10-30), so only `google`+`gmail`+`calendar`+`drive` can
  group, with YouTube necessarily separate. What is open is the UI, and it
  needs the operator's decision. All five providers already share
  YOUTUBE_CLIENT_ID/SECRET, so it is not a credential problem either.
- Two verification traps named in the doc rather than left implicit: the
  test cannot be behavioural (no injectable base URL, no mock dependency), and
  the mutation that must go RED is the reintroduced `?` - if deleting the
  profile fetch entirely is the only mutation that passes, the test is
  asserting a sentence, not a property.
- docs gate: 1 hard failure, the pre-existing eda6828 orphan, which is item
  14 in this very queue.
