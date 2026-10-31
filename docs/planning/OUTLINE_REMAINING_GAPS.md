# OUTLINE — Remaining implementation gaps, batched for approval

Generated 2026-10-31 (second pass, after `OUTLINE_TOKEN_REFRESH.md` landed).

Every item below is measured against the tree at `b7cf00a` and carries the
file and the reason it is a gap. Nothing in it is applied. This is the list
the operator is being asked to approve, so the sizing is honest: a phase
marked S is a small, self-contained change with a clear acceptance test, not
an estimate of "won't take long".

**How to read the priority column.** Priority is `now` / `next` / `deferred`,
and the ordering is by *consequence to the founder*, not by size:

- `now` — a user is losing something today (a dropped post, a locked-out
  channel, a wrongly-labelled number).
- `next` — a real gap whose cost is a slower or more confusing workflow.
- `deferred` — blocked on a decision or on something outside this repo.

---

## The queue

| # | Item | Where | Priority | Size | What is actually wrong |
|---|---|---|---|---|---|
| 1 | **G-01+G-02: a good token is thrown away by a profile** | `linkedin.rs:728-735`, `instagram_standalone.rs:210-220`, `threads.rs:204-214` | **now** | S ×3 | the reported bug; see §1 |
| 2 | **G-04: one `profile_optional` for all three, + `linkedin_page`** | new helper + 4 call sites | **now** | S (net delete) | stops the 4th copy being written wrong |
| 3 | **G-05: a post is dropped on the pre-publish path** | `scheduler/mod.rs:1824` (`resolve_token`) | **now** | S | worse than #1: a lost post, not a locked channel |
| 4 | **UI-C0: the calendar chip is 21px, WCAG floor is 24** | `CalendarEvent.svelte` | **next** | S | one CSS rule, acceptance measured by the probe |
| 5 | **UI-C2: 218px of calendar chrome, and its constant lies** | calendar `+page.svelte` | next | M | the page's own comment disagrees with its fallback |
| 6 | **UI-C1: no `+N more`, 23 chips in one 30-min slot** | WeekView/DayView/MonthView | next | M | the calendar becomes unusable at 8+ posts a day |
| 7 | **UI-M1: the preview column hides below `lg`; two filled primaries** | `ComposerModal.svelte` | next | M | blocks UI-M2 |
| 8 | **UI-R1 / UI-R2: the rail is 224px, and there is no channel rail** | `+layout.svelte` + new rail | next | L | returns 144px on every route |
| 9 | **UI-M2: one SchedulePicker for a whole cross-post** | `SelectCurrent.svelte` | deferred | M | deliberate, per AGENTS.md §9.13/§9.27 |
| 10 | **UI-V1: the probe never renders the other states** | `probe.mjs` | next | M | six audit claims were false for this reason |
| 11 | **Google-Suite grouping (undecided)** | connect flow | blocked | M | a *platform* constraint, not a code mess |
| 12 | **C1: one resumable-upload implementation** | `google.rs:286-326`, `youtube.rs:1023-1073` | next | S–M | two copies of an upload protocol |
| 13 | **C2: seam debt** | splitter signature, `EpochUtc` | next | S | only when touching either file |
| 14 | **A11-D001-D005: one orphan doc** | `PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md` | next | S | the last hard docs-gate failure |
| 15 | V1a–V1d, C3, LIVE-X-FLAKE, R1 | see §6 | deferred | — | blocked on credentials / a decision |

---

## §1 — G-01 + G-02: a good token is discarded (the reported bug)

**The mechanism, in one sentence.** `refresh_token` succeeds, then a profile
read attached to it fails, and `?` turns the whole call into an error, so the
scheduler's failure arm marks the channel "needs reconnect" and the good
access token is never written.

**The three sites, each pinned by reading (not grepping):**

- `src/social/linkedin.rs:728-735` — `refresh_token` fetches `/v2/userinfo`
  and propagates both the transport and the JSON failure. This is the
  operator's reported bug.
- `src/social/instagram_standalone.rs:210-220` — same shape on
  `graph.instagram.com/.../me`.
- `src/social/threads.rs:204-214` — same shape on `graph.threads.net/.../me`.

**The correct behaviour is already in the tree one file over.**
`linkedin_page.rs:252-262`:

```rust
// Fetch profile info so it's not lost on refresh.
// Use async match — page-scoped tokens may not have openid/profile scope.
let profile = match self.http.get(...).send().await {
    Ok(r) => r.json::<serde_json::Value>().await.ok(),
    Err(_) => None,
};
```

That is a recorded design decision present on one LinkedIn provider and
absent on the other.

**The consequence chain, live:** refresh returns `Ok(valid_token)` → the
profile read 5xx's or is refused for scope → `?` produces
`ProviderError::Http` → `proactive_token_refresh` (`scheduler/mod.rs:561-575`)
takes its `Err` arm → `mark_integration_refresh_needed` → the 24h window
query excludes the row forever → the channel shows "needs reconnect" → the
founder re-authorizes by hand while a valid token is discarded.

### The test, and why it is a source-shape pin

This is the one part of the plan that needs saying out loud, because the
obvious test cannot be written:

**A behavioural test over a mock HTTP server is impossible here.** The
provider constructors take only `&Config`, and every URL is a hardcoded
`&'static str` (`linkedin.rs`'s `token_url()`, the Graph hosts). There is no
injectable base URL and no mock-server dependency in the tree. So there is no
seam at which a test can point LinkedIn at `127.0.0.1` and watch the fix
happen.

The honest option is the precedent this repo already uses for exactly this
situation — `tests/linkedin_e2e_test.rs:933`
(`test_linkedin_token_calls_quote_the_providers_own_error_body`), which reads
the source and pins the *arrangement*: that both token calls check
`resp.status()` and read `resp.text()` before parsing, and never
`resp.json()` directly. It is RED on the pre-fix shape and GREEN after, and
it was accepted here.

So for G-01/G-02 the test is: for each of the three providers, extract
`refresh_token`'s body from its own source file and assert it does **not**
contain a bare `?` on the profile read — specifically, that the profile fetch
resolves to an `Option` (`.ok()`, `match`, or an early `return None`) rather
than `?`.

**And the trap this test must be run against.** The standing lesson, hit 5+
times this cycle: a test that asserts the *spelling* of the fix passes its own
mutation. Two mutations prove this one:

- **Delete the profile fetch entirely.** The test must still pass — a refresh
  that returns a valid token and no profile is exactly the desired degrade.
  (This mutation also happens to be a legitimate fix, which is the point:
  the test pins the *property*, not the code.)
- **Re-introduce `?` on the profile read.** The test must go RED.

If the second mutation does not turn it RED, the test is asserting a sentence
and must be rewritten against the whitespace-agnostic property.

### What each file changes

`linkedin.rs:727-735` becomes the `linkedin_page` shape: a `match` on the
send, `.ok()` on the JSON, and the `AuthToken` fields read through
`profile.as_ref()`. The four field reads (`provider_user_id`, `name`,
`username`, `picture`) become `profile.get("sub")...`-style, defaulting to
empty — which is the correct degrade: an unknown avatar is cosmetic, a
dropped channel is not.

Same for the other two, at their own field names.

---

## §2 — G-04: one helper, four call sites

Ponytail. Three copies of the rule in three files, with `linkedin_page`'s
`match` being a fourth correct idiom, is how the 5th copy gets written wrong.

Extract into `src/social/common.rs` (which is the existing home for the
cross-provider helpers):

```rust
/// A provider's profile read is BOOKKEEPING. A failure here must never
/// fail the token refresh that carries it: a valid token with an unknown
/// avatar is recoverable, a discarded token is a manual reconnect.
pub async fn profile_optional(
    http: &reqwest::Client,
    url: &str,
    bearer: &str,
) -> Option<serde_json::Value>
```

All four providers call it. Net line count goes down. This is the same shape
as the `google_oauth` unification (`a1ac73f`) and belongs beside it for that
reason — that commit is the precedent for "one implementation, one place, and
a test that pins the shape".

**Acceptance:** the only place in `src/social/` that reads a `/me` or
`userinfo` body inside a `refresh_token` is through `profile_optional`. A
greppable, single-source property — and a test that goes RED if a provider
reintroduces a direct `?`.

---

## §3 — G-05: the same defect drops a POST

`src/scheduler/mod.rs:1824`, `resolve_token`. When a post is about to publish
and its token is inside `TOKEN_REFRESH_BUFFER_SECS` of expiry, this refreshes
and **propagates** any `Err`. No `refresh_needed` write, no "try the old
token anyway" path.

So: a post that was perfectly publishable with the token it already holds is
failed because a *profile* call inside the refresh errored. That is a
customer-visible dropped post, which is strictly worse than the channel row
in §1.

**This is the priority argument**, and it is why §1 and §3 land together:
the §1 fix is *also* the §3 fix, because the failure stops being an `Err` at
all. But §3 deserves its own test, because the property it guards is
different — the post must still publish, not merely the channel stay clean.

**Test:** for the same three providers, the refresh returns a token, the
profile read fails, and a post in the buffer window must still publish. With
the current code that is impossible to drive (§1's seam problem), so the
honest form is the same source-shape pin on `resolve_token`: it must not
convert a provider `Err` into a publish failure for a token that was itself
valid. If that cannot be pinned without a behavioural seam, the phase's
acceptance is the §1 test plus a manual note, and it is reported as such
rather than over-claimed.

**Do not** add a "retry with the old token" path as the only fix. It hides the
defect and lets the next provider skip the §1 fix.

---

## §4 — The UI gaps (register rows UI-C0 … UI-V1)

These are all already scoped in `docs/planning/PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md`
and carried as open rows in `TODO.md`. The outline does not re-derive them; it
*orders* them, because the dependencies are load-bearing and the register
rows do not carry them:

**Order, with the reason:**

1. **UI-C0 first** — one CSS rule (`min-height: 24px` on the non-compact
   chip, matching what the month grid already does), acceptance measured by
   the rendered probe over the running app: every chip ≥ 24px in all four
   views at three widths, 0 FAIL, and no cell growth as a side effect. The
   register row already names what NOT to touch (the tag chips, the time,
   the metrics, the hover toolbar, the ink and the 3px bar) — the first draft
   of the audit wrongly proposed all four and they are already there. This is
   a self-contained win and it is the only one that is a pure CSS rule.

2. **UI-C2 before UI-C1.** C2 is "218px of chrome above the grid" and the
   load-bearing detail is that the page's `--cal-shell-top` fallback says
   218px while its own comment says ~168px, so the page already disagrees with
   itself. Fix the constant first, because C1's `+N more` affordance has to
   live inside whatever height C2 decides.

3. **UI-C1 after C2.** Port postiz's `slice(0,3)` + `Show more (N)` to
   WeekView/DayView/MonthView with an in-place expand keyed by date. Note the
   register's secondary finding: at a 129px chip the tag name chips truncate
   to `buil`/`la...`, so week/day visible tags cap at 1 (month already uses
   dots) to give the title its width back.

4. **UI-M1 after the calendar.** The preview column is `display:none` below
   `lg`, so a tablet user sees only the editor, and the footer has TWO filled
   primaries (`Post Now` = `bg-success`, `Schedule` = `bg-accent-fill`) so
   neither is the primary. The register row also corrects the first draft's
   claim: it is the 0.7-alpha `bg-overlay-strong` scrim that removes page
   context, NOT the panel geometry — the panel is already inset 20/45px and
   matches postiz's `max-w-[1400px]`. postiz's answer to "no room" is SWAP
   (`mobileTab` edit/preview), not hide.
   **UI-M1 is the prerequisite for UI-M2**, which is why the order matters.

5. **UI-R1 + UI-R2 last.** 224px → 80px returns 144px on every route, but the
   channel rail that goes with it is constrained: 80px rail + a 260px
   always-open channel rail leaves 1100px of calendar at 1440, which is LESS
   than today's 1104px. So the channel rail is collapsed to 64px by default
   and is per-route (`/calendar`, `/posts`, `/feed`, `/comments`, `/dms`).
   No grouping in v1 — switchpost has no customer/org concept and AGENTS.md
   §0.5.1 forbids inventing one. Move the timezone picker to `/settings` (it
   is a setting, not a nav) and keep the zone visible in the calendar header.

**UI-M2 stays deferred.** One `SchedulePicker` for the whole post means a
cross-post cannot carry X at 09:00 and LinkedIn at 17:00. `SelectCurrent.svelte`
exists (94 lines) but is not surfaced as the editor's channel header. The
register row records the deliberate limit: switchpost's model is one shared
body + per-channel targets (AGENTS.md §9.13/§9.27), and porting postiz's
per-channel full content would touch every provider's `settings` read. Land
UI-M1 first, then decide whether per-channel *times* alone (not content) is
worth the change.

**UI-V1 belongs to all of them.** The probe renders each route in its
DEFAULT state only, so the composer and the calendar's other views are
surfaces nobody measures. The 2026-10-30 re-measurement is the proof of
value: six claims in the first draft of that audit were false because the app
had not been driven into the state being described. A UI phase is not done
until its new states are in the probe's route list —
`docs/audit/2026-10-30-launches-parity/probe-audit-render.mjs` is a working
example of a state-driving probe.

---

## §5 — Google-Suite grouping: blocked on a PLATFORM constraint, not code

Recorded here so it is not re-investigated from scratch.

**The ask was** one unified login flow for Google, Gmail, Calendar, Drive and
YouTube. **It cannot be satisfied as asked.** `src/social/scope_guard.rs:40-46`
and `src/social/google.rs:42` document that Google's authorize endpoint
**refuses** two of the pairs outright:

| Pair | Google's answer |
|---|---|
| YouTube Data API + Drive | `Error 400: invalid_request` (live, 2026-10-30) |
| YouTube scope + `calendar.events` | refused the same way |

All five providers already share `YOUTUBE_CLIENT_ID/SECRET` (registry.rs:386-390),
so this is not a credential problem either. The conflict is enforced by Google
in the consent request and pinned by a test in
`tests/connect_flow_redteam_test.rs` (per `scope_guard`'s own doc comment).

**So the only buildable shape is:**

- **Group A** (mutually compatible): `google` + `gmail` + `calendar` + `drive`,
  one consent screen, one connect button.
- **Separate**: `youtube`, necessarily its own flow, because pairing it with
  either Drive or calendar is refused before consent.

**What is still open is UI, not backend:** the grouping is not built in the
connect flow. Three options were presented earlier and none was chosen:

1. One button per group, with a per-service toggle row inside the consent
   group (postiz's pattern).
2. Keep per-service tiles and add a "connect all of Google" affordance that
   walks the founder through both consents in sequence.
3. Do nothing — four tiles plus one YouTube tile, each its own consent.

This needs the operator's decision. It is marked blocked, not deferred,
because neither of the two builds is small enough to guess at.

**Nothing about §0.5's single-user preference is at stake here** — this is a
Google-side grouping constraint, not a team feature.

---

## §6 — Credential-gated and decision-gated items

Not work to do now; carried so the register stays complete.

| Item | Blocked on |
|---|---|
| **V1a** `live_reddit_test` | a working Reddit cookie/OAuth blob. Cheapest creds of the four. |
| **V1b** `live_telegram_test` | completing `tu_request_code`/`tu_sign_in` once. MUST stay one sequential test (gramers sessions are not concurrent-client safe). |
| **V1c** `live_discord_test` | `DISCORD_BOT_TOKEN` + a test guild. |
| **V1d** `live_whatsapp_cloud_test` | the Meta developer-app dance. |
| **C3** skill eval loop | a token-cost decision. Owner: user. The configuration check is already done: 90 commands parse, 7 mcp-call names resolve in 389 tools, 0 stale. |
| **LIVE-X-FLAKE** | transport-error flake in the full run. Retry transport errors in the live helper, or keep it strict. Not decided. |
| **R1** Telegram write-queue window | a decision, not a bug: document the ceiling (S) or persist a table (M) — persist only if V1b shows real payloads. |
| **B12** Linux Chromium cookie decrypt | deliberately not implemented; unblock = evidence the Firefox/Zen plaintext + paste paths are insufficient. |
| **WA-inbox** | unblock = the first consumer that needs an inbox table. |

---

## §7 — Already resolved, recorded so it is not re-opened

Two items from the previous pass that are **no longer open**:

- **The `SQLX_OFFLINE` error at `queries.rs:119`** (`create_integration`) is
  **fixed**. The `.sqlx/` cache has 60 entries, `SQLX_OFFLINE=true cargo check
  --lib` exits 0, and the entry is committed (`278e742`). No `cargo sqlx prepare`
  run is needed. This was listed as a follow-up in the last outline; the
  follow-up is closed.
- **The docs gate D003 for `OUTLINE_TOKEN_REFRESH.md`** is closed (`ae35795`).
  The one remaining hard failure is `PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md`
  from `eda6828`, an earlier commit — that one IS in this queue (item 14).

Both were carried as open in the previous outline's closing sentence. Both
were measured again rather than assumed; the measurement is what changed the
verdict.
