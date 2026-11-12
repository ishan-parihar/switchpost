# Recipes — multi-step workflows

Every command verified against the real CLI. Loaded on demand from SKILL.md.
Integration UUIDs come from `switchpost mcp-call integrations_list --args '{}'`.

## Instagram publish (no one-shot CLI)

Instagram needs a container → (poll) → publish flow. Two routes:

**Route A — the unified pipeline (preferred, one draft + publish):**

```bash
# stage a draft with media (media must be a PUBLIC URL the platform can fetch)
switchpost post "Caption text" --platforms instagram --media https://example.com/photo.jpg --first-comment "Link in bio!"
# -> {"staged":[{...,"state":"draft","post_id":"<ID>"}]}
switchpost posts publish <POST_ID>
```

**Route B — the raw container flow (fine-grained, MCP):**

```bash
switchpost mcp-call ig_create_container --args '{"ig_id":"<ACCOUNT_ID>","image_url":"https://example.com/photo.jpg","caption":"Caption"}'
# -> {"id":"<CONTAINER_ID>","status_code":"FINISHED"}
switchpost mcp-call ig_poll_container --args '{"ig_id":"<ACCOUNT_ID>","container_id":"<CONTAINER_ID>"}'   # only if not FINISHED
switchpost mcp-call ig_publish_container --args '{"ig_id":"<ACCOUNT_ID>","container_id":"<CONTAINER_ID>"}'
# Reels: pass "media_type":"REELS" and "video_url" on create; poll 2-3x with 5s delays for video
```

Local files: upload first (`switchpost media upload ./photo.jpg`), then use
the returned URL. `instagram-standalone` uses the same flow with the `ias_*`
tools.

## Carousel

```bash
switchpost carousel "5 Tips for Devs" --integration <INTEGRATION_UUID> --media https://a.jpg,https://b.jpg
# minimum 2 media URLs
```

## Deckmill autopost (render → upload → post, one command)

```bash
switchpost autopost --slides slides.json --platforms instagram --content "Three things we learned" --brand-name "Acme" --topic "Release" --url https://example.com --hashtags "ai,automation" --publish
```

`slides.json` is a JSON array of Deckmill slide specs:
`[{"type": "hero", "params": {"headline": "…"}}]`. Run
`deckmill slide-info <type>` for a type's params. Four flags are
required by the renderer (each renders a slide overlay): `--brand-name`,
`--topic`, `--url`, `--hashtags`. Other flags: `--preset` (default
instagram_portrait), `--aspect-ratio` (default 4:5), `--primary-color`,
`--brand-handle`, `--work-dir`, `--settings`.

**Recurring series** (DB-backed; the scheduler publishes — no loop process):

```bash
switchpost autopost --slides slides.json --platforms x --content "Weekly notes" --schedule 2026-11-01T09:00:00Z --repeat-days 7 --repeat-until 2026-12-01T09:00:00Z --url https://example.com --hashtags "ai"
```

## Schedule & slots

```bash
switchpost posts find-slot                       # next free slot (all accounts)
switchpost posts find-slot --integration <UUID>  # for one account → {ok, scheduled_at, integration_id}
switchpost posts create "text" --integrations <UUID> --schedule "2026-11-01T09:00:00Z"
switchpost posts create "text" --integrations <UUID> --first-comment "Context thread…"  # auto-comment right after publish — works on providers that implement comment (FB/Reddit/Slack/Dev.to/Hashnode/Medium/GMB); a failed first comment NEVER fails the post
switchpost posts get <POST_ID>                   # one post: id, content, state, integration_id, scheduled_at, …
switchpost posts schedule <POST_ID> "2026-11-02T09:00:00Z"    # reschedule
switchpost posts update <POST_ID> --content "new text"        # omitted fields kept
switchpost posts tags <POST_ID> <TAG_ID1> <TAG_ID2>           # replace tags (none = clear)
switchpost posts validate "text" --integrations <UUID>       # check limits w/o creating
```

## Failed-post recovery

```bash
switchpost posts list --state error            # what failed. STATE VOCAB: draft|queued|published|error — the CLI's help says "errored" but the filter value is `error`, and a wrong value SILENTLY LISTS EVERYTHING (no error)
switchpost posts get <POST_ID>                  # read error_message for the cause
switchpost posts publish <POST_ID>              # retry immediately (editor's fixes first: posts update)
switchpost posts delete <POST_ID>              # or drop it
switchpost posts delete <POST_ID> --from-platform   # ALSO delete the live post on the platform (the platform delete runs first; a failure keeps the row for retry)
```

The scheduler retries `RateLimited`/`Network` failures with exponential
backoff automatically; a post only lands in `error` after the retries are
exhausted. `posts update <ID> --content "fixed"` then `posts publish` is
the recovery loop.

MCP extras (posts_* tools via mcp-call): `posts_repeat` —
`{"post_id":"…","interval_days":7,"end_date":"2026-11-08T23:59:59Z"}`
(note the param is `end_date`, a FULL RFC3339 timestamp — a date-only
string is rejected; returns `group_id` + `post_ids` + `scheduled_dates`),
`posts_best_times` (`{"integration_id":"…"}` → per-account best
weekday/hour slots), `posts_thread` — `{"content_parts":["t1","t2"],
"integration_ids":["…"],"scheduled_at":"…"}` (creates a linked thread:
returns `group_id` + ordered `post_ids`; the scheduler auto-links each
part as a reply to the previous one), `posts_stage` v2 with
`overrides` (per-provider content), `dry_run` (readiness only),
`force` (stage despite readiness errors), `scheduled_at: "next-slot"`,
`group_id` (campaign). `posts_delete` takes `delete_from_platform: true` to
also remove the published post upstream (X/LinkedIn/FB).

## RSS → autopost

```bash
switchpost rss add https://example.com/feed.xml <INTEGRATION_UUID> --ai-summary --title "My feed"
switchpost rss list
switchpost rss items <FEED_ID>          # what a feed has produced
switchpost rss poll <FEED_ID>           # fetch now
switchpost rss toggle <FEED_ID>         # pause/resume
switchpost rss delete <FEED_ID>
```

## Automation rules (auto-reply)

```bash
switchpost automation create <INTEGRATION_UUID> "Auto-thanks" comment "Thanks for commenting!" fixed
# trigger: comment | dm | mention | follow;  response: fixed | template | ai_generated
switchpost automation list
switchpost automation update <RULE_ID> "New name" "New response" true   # optional positional args: NAME RESPONSE_TEMPLATE IS_ACTIVE(true|false)
switchpost automation logs <RULE_ID> --limit 50
switchpost automation delete <RULE_ID>
```

## Webhooks

```bash
switchpost webhooks create https://example.com/hook --name "my-hook"
switchpost webhooks list
switchpost webhooks get <WEBHOOK_ID>                # one hook: id, name, url, is_active, created_at
switchpost webhooks test <WEBHOOK_ID>
switchpost webhooks update <WEBHOOK_ID>
switchpost webhooks delete <WEBHOOK_ID>
```

## AI writing helpers

```bash
switchpost ai generate "Rust error handling" --tone professional --length short
switchpost ai bulk "Rust tips" --format one_short --count 3   # formats: one_short|one_long|thread_short|thread_long
switchpost ai improve "rough text"
switchpost ai summarize "long paragraph"
switchpost ai hashtags "a post about Rust"
switchpost ai tone "some text" casual
```

## Analytics (cross-platform aggregates)

```bash
switchpost analytics get <PROVIDER>          # account-level series: {label, data:[{total,date}], percentage_change}
switchpost analytics post <POST_ID>          # one post's own engagement series (live fetch)
switchpost streak --tz America/New_York      # consecutive-day publishing streak: {streak_days, streak_since}
```

Per-platform analytics (richer: insights, reach, audience) live in the
platform shims — see [platform-matrices.md](platform-matrices.md).
`analytics post` / `analytics_get_post` take the INTERNAL post UUID
(from `posts list`), NOT the platform's post id and NOT a feed-import
row's id — an external_posts id returns "Post not found". For an
imported/external post's engagement, read the platform directly (e.g.
`mcp-call x_tweet_detail`). NOTE: `x_get_post_analytics` is OAuth-only
— on a cookie-auth X account it 403s; use the timeline/tweet-detail
reads instead.

## Campaigns / signatures / tags / notifications

```bash
switchpost campaigns create "Launch week" --goal "Ship v1" --start-date 2026-11-01 --end-date 2026-11-08 --progress-metric posts --progress-target 10
switchpost campaigns list                # includes archived
switchpost campaigns update <CAMPAIGN_ID> --status paused   # omitted fields kept; --clear-<field> clears
switchpost campaigns delete <CAMPAIGN_ID>

switchpost signatures create "Sign-off" "— Acme" --provider x   # omit --provider for all
switchpost signatures list
switchpost signatures update <SIGNATURE_ID>
switchpost signatures delete <SIGNATURE_ID>

switchpost tags create "work" --color "#ff0000"
switchpost tags list
switchpost tags get <TAG_ID>                     # one tag: id, name, color, created_at, updated_at
switchpost tags update <TAG_ID>
switchpost tags delete <TAG_ID>

switchpost notifications list --limit 10
switchpost notifications read <ID>
switchpost notifications read-all
switchpost notifications create "Title" "Body"
```

## Media

```bash
switchpost media upload ./photo.jpg --alt "Description"
switchpost media upload https://example.com/image.png    # from URL
switchpost media upload-batch ./a.jpg ./b.jpg
switchpost media list --limit 20
switchpost media download <URL> --output ./out.jpg
# No CLI delete subcommand — use the MCP bridge. NOTE the param is
# media_id (NOT id, unlike most tools):
switchpost mcp-call media_delete --args '{"media_id":"<UUID>"}'
```

## UTM tagging

Put `"utm_campaign": "…"` in a post's `--settings` — every plain URL in the
content gets `utm_source=<platform>&utm_medium=social&utm_campaign=<campaign>`
appended at publish time.
