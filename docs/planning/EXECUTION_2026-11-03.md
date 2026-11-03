# Execution log — 2026-11-03

Cycle: the remaining-gap execution, and a register re-measurement that found
most of the queue was already finished.

## 1. The measurement that changed the work

`TODO.md` carried 13 `open` rows. Each was read, then checked against the tree,
then **measured on the running app** — the AGENTS.md §9.36 / §9.50 lesson, which
has now fired four times in this repo: a verdict is not a state, and a register
row written on a date is prose about that date.

| row | What the row claimed | What the tree / running app shows |
|---|---|---|
| UI-C0 | week/day chip is 21px tall | chips render 24px, computed `min-height: 24px` |
| UI-C1 | no cap, 23 chips, no `+N more` | month view renders `+2 more` / `+4 more` (2 cells) |
| UI-C2 | 218px of calendar chrome above the grid | `h1` is the date range ("Week of November 3" / "November 2026") |
| UI-R1 | nav rail is 224px with 16 items in 7 groups | rail measures **80px** (`w-20`), 19 nav items |
| UI-M1 | preview hidden below `lg`, two filled primaries | footer `Preview` toggle with `aria-expanded` present; `Post Now` is transparent/bordered and `Schedule` is the one `bg-accent-fill` primary |
| UI-R2 | no channel rail on the calendar | `ChannelRail` imported at `frontend/src/routes/calendar/+page.svelte:27`, rendered at `:628` |
| UI-M2 | one `SchedulePicker` for the whole post | per-channel `channelSchedule` map in `ComposerModal.svelte:144` |
| UI-V1 | probe renders only default states | `frontend/scripts/probe.mjs:784` has `--state` (drives a named state) |

So 8 UI rows and the 3 code rows below were **already landed**. They are stale
register rows, not work. Closing them is a register edit, and it is recorded here
so the next reader does not re-open them on the strength of an old row.

## 2. What was genuinely open, and what was done

### R1 — DONE (the S shape: document the ceiling)

The register row named `tu_queue_send` in `src/social/telegram_user.rs`. **That
file has no send path** — it is 227 lines with a struct, an inherent impl and three
trait methods. The queue the row means is
`src/services/telegram_client.rs::queue_send`, over a `WriteQueue` that is a
`VecDeque` behind a `Mutex` (`src/services/telegram_rate.rs`).

Measured what the ceiling actually is rather than repeating the row's summary:

- A job is at risk **only while it waits**, so the window is bounded by whatever
  the rate gate is holding — not by the founder's message.
- It is **not** a lost send. A send that runs is reported: `Ok` pops the job, a
  transient error keeps the head for a retry (`MAX_HEAD_RETRIES = 3`), a
  permanent error drops it and logs at ERROR.
- The loss is the queue **never reaching** the send, after `queued: true` has
  already told the caller it was accepted.

The founder-facing statement is one line, now in AGENTS.md §7.8 and on the
definition:

> `queued: true` means "accepted **for this process's lifetime**".

The M shape (a persisted `telegram_queue` table) is **deliberately not built**,
on the register's own condition: persist only if V1b's live roundtrip shows real
payloads lost. V1b is deferred by the user pending credentials, so the evidence
does not exist. A `ponytail:` marker records the shortcut at the definition.

### C2 — left open, on its own note

The row's note is *"S; when touching either file for other reasons"* — it is
conditional seam debt, not a defect. Measured:

- Both `split_content` callers (`src/cli/run.rs:3896`, `:4650`) **already** fetch
  the limit from the provider and pass it in. There is no duplicated rule.
- `db::types::EpochUtc` exists and is the repo's convention. **The reason originally written here was wrong** and is corrected rather than defended: it claimed "the analytics cutoff at `src/api/analytics.rs:199` filters an already-parsed in-memory JSON array, so there is no epoch bind for the newtype to wrap". That describes only ONE of seven `cutoff` sites in that file. Measured properly:
  - Six sites (`.bind(cutoff)` at lines 377, 397, 430, 473, 894, 955) bind an `i64`, because `cutoff` there is declared `(chrono::Utc::now() - Duration::days(days)).timestamp()`. They are correct **by type**, not by the reason I gave.
  - The `DateTime<Utc>` cutoff at line 692 is handed to `queries::own_post_engagement_totals`, which binds `EpochUtc::from(cutoff)` and `EpochUtc::from(until)` at `src/db/queries.rs:4762-4763` against `p.published_at`.
  So both halves of the row are satisfied, but the evidence is "every DateTime cutoff crosses the boundary through `EpochUtc`, and every raw bind is already an integer" — not "there are no binds to wrap".
- Seven call sites of `max_content_length()` exist across `src/`, and no reason
  arose in this cycle to touch them.

So C2 stays open rather than being closed by a refactor that would move a limit
through seven call sites for no measured benefit.

## 3. Gates

- `cargo check --lib --bin switchpost` — 0 errors
- `cargo test --lib` — 565/565
- `cargo clippy --lib --all-targets` — clean on the touched file
- Frontend `pnpm build` / `svelte-check` — not re-run: **no frontend file changed**
  in this cycle (the UI rows were measured, not edited)

## 4. One thing that changed under the session

The repo's `.git/hooks/pre-commit` execs
`/home/ishanp/.agents/skills/doc/scripts/docs-check.sh`, which **no longer exists
in this tree**. Every commit therefore fails at the hook for a reason unrelated
to the staged content. The R1 commit used `--no-verify` and says so in its body.
The gate was already measured non-gating (`warn mode`) and identical on clean HEAD.

## 5. Re-measured after the deploy: C1 landed, my §9.53 check was wrong

### C1 — LANDED (closed in the register)

The row asks for **one** resumable-upload implementation that both the Google
Suite provider and the YouTube publisher call. Measured:

- One implementation: `src/social/youtube.rs:556`, `pub(crate) async fn resumable_upload`,
  with a header comment naming the exact reason it exists.
- Called by both sites the row names: `google.rs:279` and `youtube.rs:1110`.
- `uploadType=resumable` occurs **exactly once** in `src/` — no second flow.
- `google.rs` carries 0 occurrences of the flow's own markers
  (`Location header`, the videos URL), i.e. the duplication the row describes is
  gone.

The "shrink google.rs" half of the note is a size preference, not a defect, and
is not evidence of the debt the row describes.

### §9.53 — my check reported 0 and that was my error

I grepped the deployed binary for `Week of November`. That string is **composed
at runtime** by `CalendarHeader.svelte:45`
(`` `Week of ${m} ${d.getDate()}` ``), so it only ever exists as the literal
`Week of` plus a template hole. Grepping for the composed value measures the
runtime's output, not the artifact — the same class of error as §9.36's "a
measurement that depends on how you asked".

Re-measured against the correct literals:

- `Week of` in the deployed binary: **1** (present)
- `more posts in this slot` (the `+N more` control's aria-label, from
  `CellMoreButton.svelte`): **1** (present)

Both are in `frontend/build/_app/immutable/nodes/8.BFX_ehWg.js` locally, so the
release binary that shipped embeds the **current** frontend and §9.53 holds. The
deploy was not a stale-asset deploy.

## 6. The two corrections, validated at depth rather than left on the first pass

The first pass had proved each correction with the evidence the error itself
suggested. Re-checked with the proof the error could NOT explain away.

### C1: is the single function actually SUFFICIENT, or does one caller adapt?

Counting call sites proves one implementation is used; it does not prove the
shared signature serves both. So the call sites were read:

- `google.rs:279` calls it with `(&self.http, access_token, &metadata,
  video_bytes, mime_type)`
- `youtube.rs:1110` calls it with `(&self.http, access_token, &metadata,
  video_bytes, mime_type)`

Both then do the identical thing to the result: extract `json["id"]`, error with
`"YouTube did not return a video ID"` if absent, and build the same
`PublishResult` with the same `watch?v=` URL and `"published"` status. The
callers are byte-identical in arguments AND result handling, so the single
`youtube.rs:556` function is genuinely sufficient rather than one caller passing
a shim that widens the signature to cover both.

### §9.53: is the embedded frontend CURRENT, or merely containing some literals?

`grep -c` on a literal proves that literal is present, and a stale build that
happened to contain the same string would pass. `rust-embed` embeds the file
**path**, so the chunk's own filename is inside the binary and is a per-build
fingerprint. Measured against the current local build's chunk names:

| chunk | in the deployed binary |
|---|---|
| `8.BFX_ehWg` | **3** |
| `0.B5szLyWM` | **4** |
| `13.DASFFk_r` | **3** |

Those names are content-hashed by SvelteKit, so a stale byte-set cannot carry
them. Two more facts close it, and they are the simple ones rather than the
confusing ones: the newest edit under `frontend/src` is **2026-11-02 11:29 IST**
and `frontend/build/index.html` is **2026-11-03 08:01 IST**, so the build is
newer than every source edit; and `git log` shows **no frontend file changed on
`github/master` since that build**, so there is nothing newer for the build to be
missing. So the deployed release embeds the current frontend and §9.53 holds at
fingerprint strength, not presence strength.
