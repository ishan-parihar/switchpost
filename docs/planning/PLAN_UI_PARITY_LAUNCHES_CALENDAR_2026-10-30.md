# Frontend Parity Audit: Postiz `/launches` vs Switchpost `/calendar`

> Date: 2026-10-30 (revised 2026-10-30 after a rendered re-measurement)
> Route: `website-design` -> `redesign` (audit-first, existing site) -> plan only. **No code was changed in this pass.**
> Evidence: `docs/audit/2026-10-30-launches-parity/` (screenshots, `measurements.json`) + this pass's own probe run over the RUNNING app at `http://localhost:6543` (8 seeded posts today, `frontend/.probe-audit.mjs`).

> **RE-VERIFIED 2026-11-01 (after the phases had landed).** All seven phases are in the
> tree, and each phase's acceptance was re-measured on the running app rather than read off
> the code: chip 24px/`under24: 0`, `+N more` rendering, rail 80px, preview reachable at
> every width - at 1440, 768, 390 and 360. The re-verification also found three defects the
> original phase verdicts had missed, and they are recorded where they were found rather
> than buried: the shared danger-confirm button at **2.89:1** and the error/warning toasts
> at **1.00:1** (`32ea48b`, both now green in the new `check-ink-on-fill.mjs` gate), and
> `/dms` still offering a cookie-auth X channel that answers **400** (`56a773a`).
>
> The lesson this file exists to teach has now fired three times, so it is stated plainly:
> **a verdict is not a state.** Every "done" in Part 4 below was still checked against the
> render on the day it was re-read, and each time at least one of them no longer held.

## Why this file was rewritten

The first version of this audit was written from the earlier probe pass and contained
**six claims the rendered app disproves**. They are listed here rather than deleted,
because the AGENTS.md §9.36 / §9.50 lesson is exactly this: a measurement taken when a
page was in a different state is not a measurement. Six of them now carry a
**CORRECTION** row. The plan in Part 4 was re-derived from the corrected numbers.

| # | The earlier claim | What the render actually shows |
|---|---|---|
| 1 | "the preview column is **absent (width 0)** below `lg`, so a phone user loses the preview entirely" | **CORRECTION**: the pane is `display:none` until the footer **Preview** button is pressed, then it is 734x420 at 768 and 356x428 at 390. The column is *collapseable*, not deleted. The real defect is that it is **opt-in and off by default**, so the first thing a phone or tablet user sees is the editor only |
| 2 | "the composer is **full-viewport, fills the screen** with no page context" | **CORRECTION**: the panel is inset **20px left/right, 45px top/bottom** at 1440 (1400x810 inside 1440x900) and 16/51px at 768. Postiz's `max-w-[1400px]` is matched. The *backdrop* is `bg-overlay-strong` = `rgba(0,0,0,0.7)`, which is what removes the context, not the panel geometry |
| 3 | "the calendar post chip has **no time shown**" | **CORRECTION**: the time is the FIRST thing on the chip - `15:00`, `03:51`, `14:30`, `22:15` in every measured sample. The plan's own Phase 4 "add the time" is a no-op |
| 4 | "the calendar post chip has **no tag colour**" | **CORRECTION**: the chip has BOTH a 3px left border in the tag/campaign colour (`rgb(16,185,129) 3px`) AND solid tag name chips with `inkOnFill` ink. Month view swaps them for 4px dots. The plan's Phase 4 "add a 1px left colour bar" is a no-op, and a 3px bar already exists |
| 5 | "the calendar post chip has **no hover actions**" | **CORRECTION**: `PostHoverToolbar` renders on hover with duplicate/stats/delete and a live-post link; the chip's own content fades to `opacity-0` so nothing overlaps. This was already done |
| 6 | "the chip **truncates a single line, `text-[16px]` in a 24px box**" | **CORRECTION**: the chip is `0.6875rem` = **11px** in a **21px** box at week/day, and 24px in the month grid. The font-size claim was wrong by 45%. The 21px height is a real defect - see F-1 |
| 7 | "there is **no chip overflow handling** vs postiz's `slice(0,3)` + `Show more`" | **CONFIRMED**, and worse than the plan stated: with 8 posts seeded on today, **23 chips** render with **no cap and no `+N more`** (measured `anyMoreAffordance: []`). Six chips stack inside one 30-minute slot |

One further correction to this document's own first draft: it cited "the AGENTS.md 12.1
`overlay-strong` note" as evidence for the composer complaint. **`overlay-strong` appears
nowhere in AGENTS.md** - the only occurrence in the repo is the `ModalManager.svelte`
class itself. The citation is retracted; the backdrop observation stands on the measured
`rgba(0,0,0,0.7)` alone.

Four baseline numbers in the 1.2 table above are STALE as of 2026-11-01, and all
four are improvements rather than regressions - recorded here so a later reader does not
"fix" them back:

| The 1.2 table says | What the render shows now (2026-11-01) |
|---|---|
| Calendar `h1` = `Content Calendar` | The `h1` is the **date range** - "Week of November 1". The second "Content Calendar" heading was deliberately removed as a duplicate current-view signal (Phase 3). |
| Global nav rail = `224px` (`w-56`) | **80px** at 1440, 768, 390 and 360 (Phase 5a). |
| Week/day chip height = `21px` | **24px** everywhere (Phase 1). |
| Chip cap per cell = none, 23 chips | 3 + a `+N more` disclosure (Phase 2). |

---

## Scope, and what is NOT in scope

| Area | In scope | Out of scope (with reason) |
|---|---|---|
| Left rail of channels | Postiz's collapsible channel rail across the calendar | Postiz's org/team/`SelectCustomer`, billing/affiliate, `HideChatbase`, `Impersonate`, CopilotKit, Chrome extension, membership slots |
| New post modal | Structure, column model, editor toolbar, per-channel switch, preview, footer actions | Postiz's AI image/video generation, CopilotKit, provider-specific `SettingsComponent` battery beyond what switchpost already exposes, HubSpot |
| Thin left navigation | The 80px icon-over-label rail vs switchpost's 224px icon-left-label rail | Postiz's Affiliate/Billing entries (AGENTS.md §0.5.1 forbids billing/team), language switcher placement stays as switchpost's settings page |

Non-negotiable filters applied from AGENTS.md §0.5: no multi-user, no marketplace, no Redis/Temporal, single binary, SSE realtime, tokens only, no raw hex.

---

## Part 1. THE MEASURED BASELINE

### 1.1 What was rendered, and how

The app was started from the release binary against the repo's `.env` on
`http://localhost:6543`. Eight posts were seeded through `POST /api/posts` onto the X
integration with two tags, on today's date, spread 08:00-16:55. Then the calendar was
driven into each state and measured with `getBoundingClientRect()` and
`getComputedStyle()`. A page rendered in only its default state is a page whose other
states are unmeasured (AGENTS.md §9.35, §9.50) - the first draft of this audit read a
default-state render and got the chip and the composer wrong.

### 1.2 The numbers

| Metric | Switchpost (measured this pass) | Postiz (read from source) | Gap |
|---|---|---|---|
| Global nav rail width | **224px** (`w-56`), `56px` collapsed (measured `RAIL-COLLAPSED {asideW:56}`) | **80px** shell, 64px inner menu | switchpost spends 224px of every viewport on chrome; postiz spends 80 |
| Nav item shape | `207 x 44` icon-left + label-right, `text-sm`, `gap-3`, `rounded-lg` | 44-54px, icon centred with a 9-10px label BELOW, `leading-[1.1]`, weight 600 | Different information model, not just a width |
| Nav items | 16 items in 7 groups (incl. a `Channels` link and a `Settings` link) | 7 items in 2 groups | Switchpost's rail is 2.3x longer; the thin rail is what makes 16 items viable |
| Rail footer | timezone `<select>` + theme + logout, all inside the rail | version string | The timezone picker is a setting, not a nav |
| Content width at 1440 | **1104px** (shell) / main 1216 | `calc(100vw - 80 - 12*2 - 8)` = ~1316 | Switchpost loses ~212px of calendar to chrome |
| Content width at 768 | 720px | ~700px (rail hidden on mobile) | Comparable |
| Content width at 390 | 358px, rail 224px so 134px of usable width | rail hidden, full width | At 390 the rail is a drawer and the page gets the full width - this is not the weak case |
| Calendar chrome above the grid | **218px measured at 1440**, 270px at 768, 350px at 390 (`shellTop`) | 1 row, `h-[42px]` + 20px page padding | Switchpost: three stacked rows (title row + `CalendarHeader`'s nav row + its filter row). Postiz: one `flex-wrap gap-[8px]` row |
| Calendar `h1` | `Content Calendar`, 20px/600, 170x28px box | none - the date range is the title | Two "current view" signals on switchpost |
| View switcher | 4 separate buttons in a bordered box, 44px tall each, `bg-accent-fill` active | 3 segmented options, `w-[74px]`, `pt-[6px] pb-[5px] rounded-[6px]`, active `bg-boxFocused`, and a **separate 2-icon group** to flip calendar/list | Postiz reads as one control; switchpost's reads as 4 tabs |
| Filters | 3 native `<select>` (`All channels` / `All campaigns` / `All tags`), `min-h-11` | `SelectCustomer` + `SelectChannels` dropbuttons; on phones they move into a bottom sheet | Switchpost's filters are correct but carry no channel identity |
| **Week/day chip height** | **21px - a WCAG 2.2 2.5.8 FAIL** (floor is 24x24; AGENTS.md §9.37) | 24px tag strip + 8px padding + 14px body | **The one real, measured accessibility defect on the calendar** |
| **Month chip height** | **24px - PASS** (`.event-chip.compact { min-height: 24px }`) | same | Already correct; do not regress it |
| Chip contents | left colour bar (3px) + solid tag name chips with computed ink + `15:00` time + truncated title + metric counts + hover toolbar + error `!` badge | 24px tag colour strip + title, hover reveals 6 actions | Roughly at parity on content; switchpost carries MORE (time, metrics) |
| **Chip cap per cell** | **none - 23 chips rendered for 8 seeded posts, `+N more` absent** | `slice(0,3)` then `Show more (N)` | Real defect |
| **Tag legibility on a chip** | tag name chips truncate to `buil` / `la...` and the title to `Overflow...` at a 129px chip | — | The tags exist but are unreadable at week width |
| Composer panel | inset 20px/45px, 1400x810 at 1440 | `max-w-[1400px]`, `size: 80%`, `p-[40px]`, `rounded-[20px]` | Geometry at parity |
| Composer backdrop | `bg-overlay-strong` = `rgba(0,0,0,0.7)`, one layer | `bg-popup` + `backdrop-blur-[8px]` on the phone sheet only | Switchpost's scrim is heavier and flatter |
| Composer columns | editor flex-1 + **420px** preview at `lg`+, **collapsed to `display:none` below `lg`**, revealed by a footer `Preview` button | **580px** desktop, **440px** tablet, **`mobile:!w-full`** on phones via a `mobileTab` swap | Postiz never removes the pane; switchpost defaults it off below `lg` |
| Composer editor column | 7 stacked cards, measured heights 137/110/110/353/146 + a 46px toolbar; the **body sits seventh** | `PicksSocialsComponent` avatar row, `SelectCurrent` dropbutton, inline toolbar, then the body; **6 things above, 5 behind a `Settings` disclosure** | Switchpost's card stack is the whole disclosure |
| Composer preview column | `Preview` heading + an empty state ("Select a channel to see a preview") | `ShowAllProviders` - a live frame per selected channel (LinkedIn card, X tweet, Reddit post, IG square) | Real gap: switchpost renders the pane only once a channel is picked |
| Composer footer | two-sided: `AI` / `Preview` (mobile) left, `Save Draft` / `Post Now` / `Schedule` right, **`Post Now` green-filled AND `Schedule` accent-filled** | left: `Tags`, `Repeat`, `Delete Post` (edit mode); then `DatePicker`; right: `Save as Draft` + a primary `Add to calendar/Schedule` with a **chevron dropdown carrying `Post Now`** | Postiz has ONE primary colour and the destructive isolated |

### 1.3 Accessibility baseline

| Check | Target | Measured | Verdict |
|---|---|---|---|
| Week/day chip target size | >= 24x24 (WCAG 2.2 2.5.8 floor) | **21px** | **FAIL** (AGENTS.md §9.37: below 24 is a FAIL) |
| Month chip target size | >= 24x24 | 24px | PASS |
| Sidebar nav item | >= 44px | 207 x 44 | PASS |
| Collapsed rail item | >= 44x44 | 56px rail, `min-h-11 min-w-11` | PASS |
| View switcher | >= 24px | 44px | PASS |
| Filter dropdowns | >= 24px | `min-h-11` | PASS |
| Chip text contrast | >= 4.5:1 | tag chips use `inkOnFill` on a solid fill; the earlier probe found no text-contrast FAIL on `/calendar` | PASS |
| Heading order | h1 -> h2 -> h3 | `h1` "Content Calendar" then composer `h3` | PASS |
| Composer dialog contract | name, `aria-modal`, trap, Escape | `ariaLabel:'Composer'`, `ModalManager` trap, `closeOnEscape:true` + `askClose` confirmation | PASS |
| Reduced motion | gated | `app.css` declares the query | PASS |
| Focus visible | all interactive | `focus:outline-none focus:border-accent` on inputs, `focus:border-accent` on buttons | PARTIAL - a border-colour change is a weak indicator; carry F-9 |

### 1.4 Baseline inventory (what is preserved)

**Tokens**: `--bg`, `--bg-card`, `--bg-hover`, `--bg-input`, `--accent`/`-strong`/`-fill`/`-fill-hover`/`-fg`, `--success`, `--warning`, `--error`, `--info`, `viz-*`, `hue-*`, `--overlay`, `--overlay-strong`. All semantic, theme-switched, no raw hex in components. **Preserve.**

**Type**: Inter. **Radii**: `--radius` / `--radius-lg`. **Preserve.**

**IA**: 18 top-level routes + nested settings. **The slug set must not change** - the agent skill's `platforms-and-cli.md` and the operator's muscle memory reference them.

**Patterns to preserve (all verified in source, all still absent from postiz)**:
1. SSE live channel state (a disconnect appears with no reload). Postiz uses SWR polling.
2. Campaign/group drag with the explicit "move all or just this" confirmation, and the published-post "reschedule vs just update" choice.
3. Bulk-select on the calendar with a visible selected-count bar. Postiz has no calendar bulk selection.
4. The composer's blocking-reason banner listing ALL blockers at once.
5. `askClose` + "Draft saved" + the `⌘+↵ post · ⌘+S save` hints.
6. The chip's hover toolbar, time, metric counts and `inkOnFill` tags - **these are the parts the first draft of this audit wrongly proposed to add**.
7. `providerInk` / `providerInkOnCard` / `OnChip` / `OnInput` - 4.5:1 on four surfaces (AGENTS.md §9.40).

**Patterns to retire (with reason)**:
1. **The 21px week/day chip.** The one measured WCAG FAIL in this audit. The month grid already has the fix (`.event-chip.compact { min-height: 24px }`) - the same rule was never applied to the non-compact chip.
2. **No per-cell chip cap.** 23 chips for 8 posts is a wall, not a scan.
3. **Three stacked header rows** = 218px of chrome above the grid at 1440.
4. **The standalone `h1` + right-aligned action row**, which competes with the calendar's own date navigation.
5. **The preview column defaulting to hidden below `lg`.** Postiz's answer to "no room" is swap, not hide.
6. **Two filled primary buttons in the composer footer** (`Post Now` green, `Schedule` accent).
7. **`Channels` and `Settings` as two rail entries** when the rail is the only surface a channel appears on.
8. **The timezone `<select>` inside the rail** - a setting, not a nav.

**SEO**: not applicable - authenticated app behind one password.

### 1.5 Dial reading

| Dial | Reading | Evidence |
|---|---|---|
| VARIANCE | 3 | Symmetric structure, one grid, `space-y-6` vertical rhythm |
| MOTION | 1 | `prefers-reduced-motion` honoured; only `transition-colors` and the rail's `duration-200` |
| DENSITY | 4 | `p-4`/`space-y-4` on the composer, `p-5` on the calendar shell, `h-14` bars. Postiz is a 7-8 |

Mode: **preserve**. Tokens, routes and SSE stay; the shell and the calendar/composer composition change.

---

## Part 2. THE CONTRAST, ASPECT BY ASPECT

### 2.1 Aspect A: the left rail of channels across the calendar

**Postiz.** The channel rail is a *column of the `/launches` page*, not the global nav. The global nav is the separate 80px rail - two different left surfaces, on purpose.

- Width **260px** open, **100px** collapsed (cookie `collapseMenu`), `p-[20px]`, `gap-[15px]`, `rounded-e-[8px]` items on `bg-newBgColorInner` with `hover:bg-boxHover`.
- Top to bottom: `Channels` h2 + a collapse chevron that rotates 180deg; then a full-width `+` Add-Provider button; then a row of `New Post` + `AI` buttons; then channel groups.
- Each group is a collapsible header (`OpenClose` chevron + name + pencil-on-hover to rename, persisted per group name in localStorage), and each channel is a row: **36px avatar with an 18px platform badge pinned `bottom-[5px] -end-[5px]`**, an `SVGLine` accent bar that fades in on `group-hover/profile`, the name (`role="Handle"`, draggable), and a kebab `Menu`.
- **Channels needing attention** (`refreshNeeded` or `inBetweenSteps`) get a red `!` badge at `start-[5px] top-[5px]`, a `bg-primary/60` wash over the whole 39x46 hit area, and the row's click starts the reconnect. Custom-fields providers (Bluesky) re-open the credential form instead.
- **Channels are draggable between groups** (`useDrag`/`useDrop`, type `menu`) and the rail is a **drop target for a calendar post** - dragging a post onto a channel reassigns it.
- Empty state: a full `/no-channels.svg` illustration with "No channels yet" + "Connect your accounts".
- Mobile: the whole rail becomes a bottom sheet (`mobile:fixed inset-x-0 bottom-0 max-h-[80vh] rounded-t-[16px]`) opened from a compact trigger showing **3 overlapping avatars + the count**.
- It scrolls (`overflow-y-auto` with a themed scrollbar), so 16 channels are viable.

**Switchpost.** There is **no channel rail on the calendar at all**. Channels are a `<select>` filter with no avatars, no identity, no reorder and no reconnect; a separate `/channels` route; and a `ChannelSelector` card inside the composer.

**The gap.** A solo founder managing many accounts leaves the calendar to see which channel is connected, which needs a reconnect, and to add one. And at 1440 the rail leaves 1104px of calendar, so the naive fix (add 260px) makes things worse - see §2.3's collision arithmetic.

### 2.2 Aspect B: the new post modal and all its features

**The shell.** Postiz: `size: '80%'` + `w-[100%] max-w-[1400px]` at `fullScreen:true`, `p-[40px]`, inner panel `bg-newBgColorInner rounded-[20px]`, two `h-[65px]` column headers (`Create Post` + `CreationMethodBadge`, and `Post Preview`), and an `h-[84px]` footer. Switchpost: the panel geometry is already at parity (1400x810 inset 20/45 at 1440, `max-w-[1400px] h-[90vh]`). What differs is the backdrop (`rgba(0,0,0,0.7)` vs postiz's near-clear `bg-popup`) and the interior composition.

**The column model.**

| | Postiz | Switchpost |
|---|---|---|
| Editor | flex-1 `min-w-0`, scrollable | flex-1 `min-w-0`, scrollable |
| Preview | `w-[580px] tablet:w-[440px] mobile:!w-full` | `lg:w-[420px]`, `hidden lg:block` below `lg` |
| Below `lg` | **swap** via `mobileTab` (`edit`/`preview`); the editor gets `mobile:flex-none` and the preview takes `mobile:!w-full` | **hide** until the footer `Preview` button is pressed; then it becomes a full-width stack |
| Phone settings | `showSettings` renders as a **bottom sheet** (`mobile:fixed mobile:inset-x-0 mobile:bottom-0 mobile:z-[600] mobile:max-h-[90%]`) over the editor | all panels stay in the single scroll column |

**The editor column.** Postiz holds, in order: the `PicksSocialsComponent` avatar row (the channels this post goes to, clickable to switch), a `SelectCurrent` dropbutton that switches the channel IN VIEW, the inline toolbar, the TipTap body, and the per-post delay row when the post is a thread. Everything else lives behind a `Settings` disclosure (`#social-settings`, `MediaSettingsSection` first).

Switchpost holds a stack of 7 cards: Title, Post to, Tags, Campaign, Content (shared across all channels), Media, then the right column's Preview + Schedule + Post Sets. Measured heights 137/110/110/**353**/146 + a 46px toolbar, so the body is the tallest block and sits seventh of seven.

**The preview column.** Postiz renders `ShowAllProviders` - a live preview per selected channel, each in that platform's own frame, each with its own scroll. Switchpost renders `PlatformPreviewPane` with one frame per selected channel, but only after a channel is picked; before that the column shows "Select a channel to see a preview. Every selected channel gets its own frame, in its own chrome."

**The footer.** Postiz: left has `Tags` + `Repeat`; then `Delete Post` in `#FF3F3F` when editing; then `DatePicker`; right has `Save as Draft` and the primary `Add to calendar` / `Schedule` / `Update` with a **chevron dropdown carrying `Post Now`**. Three buttons on the right, each a different weight, the destructive isolated on its own.

Switchpost: `AI` and (below `lg`) `Preview` on the left; `Save Draft`, `Post Now`, `Schedule` on the right, with `Post Now` and `Schedule` **both filled** (green and accent).

**Feature inventory.**

| Feature | Postiz | Switchpost | Verdict |
|---|---|---|---|
| Multi-channel cross-post | per-channel content + media + date | shared content + per-channel target | Different model. Keep switchpost's (AGENTS.md §0.5.2), but make the per-channel view visible |
| Per-channel switch in modal | `SelectCurrent` dropbutton | `SelectCurrent.svelte` exists (94 lines) | **Component exists; not surfaced as the editor's channel header** |
| Per-channel date | `channelDates` | one `SchedulePicker` for the whole post | Real gap for the solo-founder case (X at 6pm, LinkedIn at 9am) |
| Per-channel delay in a thread | `DelayComponent` | `ThreadBuilder` | Parity |
| Tags | footer-left | `TagPicker` | Parity |
| Repeat | `RepeatComponent` | `posts_repeat` | Parity |
| Sets/templates | `SetSelectionModal` before opening | `PostSetModal`, and switchpost does **not** block on set selection | Parity, and switchpost's is better |
| Signature | toolbar + store | `SignatureEditor` | Parity |
| AI | Copilot + generator | `AiAssistant` + `GeneratorModal` | Parity, more self-contained |
| Emoji | 400px panel | `😊` button | Parity |
| Media | drag-drop + `MediaSettingsSection` | `MediaUpload` | Parity |
| Preview per channel | native frames, always on | `PlatformPreviewPane`, opt-in below `lg` | Partial - see the correction at the top |
| Creation-method badge | `CreationMethodBadge` | not surfaced | Optional, low value |
| Duplicate | from the calendar row | from the calendar row | Parity |
| Mobile tab swap | yes (`mobileTab`) | footer `Preview` toggle instead | Different mechanism, same reachable end state; the **default** is the difference |
| Blockers banner | absent | all blockers at once | **Switchpost advantage** |
| Draft autosave + hints | absent | "Draft saved" + `⌘+↵ ⌘+S` | **Switchpost advantage** |

### 2.3 Aspect C: the thin left navigation panel

**Postiz**: an 80px shell (`bg-newBgColorInner rounded-[12px]`) holding a 64px fixed column, `gap-[32px]` between its two groups, `gap-[8px]` between items. Each item is `h-[54px]` (`44px` at the smaller break), `py-[8px] px-[6px] rounded-[12px]`, **icon on top scaled to 90%, label below in `text-[9px]`/`text-[10px]` at `leading-[1.1]`, centred, weight 600**. Active: `text-textItemFocused bg-boxFocused`; inactive `text-textItemBlur`; hover the same as active. It sits inside a `flex-1 flex gap-[8px]` page with a separate 80px top bar, and the page is a `rounded-[12px]` panel on a 12px-padded canvas.

**Switchpost**: 224px collapsing to 56px via a persisted toggle. Items `207 x 44`, icon-left + label-right at `text-sm`, `gap-3`, `rounded-lg`. Seven section headings plus a timezone picker pinned in the footer.

| | Postiz thin rail | Switchpost wide rail |
|---|---|---|
| Width cost | 64-80px | 224px (56 collapsed) |
| Items it can carry | ~10-12 before scrolling | ~20 |
| Section headings | none (two groups) | 7 headings |
| Label legibility | 9-10px, must be short | `text-sm`, full words |
| Discoverability for a new user | lower | higher |
| Scan speed for a daily user | higher - one glance, no reading | lower |
| Content width at 1440 | ~1316px | 1104px |

**The collision arithmetic (the load-bearing constraint).** If switchpost adopts the 80px rail AND adds a 260px channel rail, the chrome cost at 1440 is `80 + 260 = 340px`, leaving **1100px** of calendar - which is **less than today's 1104px**. So the two cannot both be always-open at 1440 and still be called an improvement. Postiz's own answer is that the channel rail **collapses to 100px** and is collapsible per cookie.

The plan must therefore specify one of: (a) the channel rail collapsed to 64px (avatars only, tooltips) by default, so the cost is `80 + 64 = 144px` and the calendar gets 1296px; (b) the channel rail as an overlay opened from a header trigger; or (c) the channel rail is **per-route** - present on the four publishing/engagement routes, absent on dashboard/analytics/research/settings.

**Recommendation: (a) + (c).** The 80px global rail is the biggest single win (224 -> 80 gives back 144px on every page). The channel rail is collapsed-to-avatars by default with a 260px expanded state, and it renders only on `/calendar`, `/posts`, `/feed`, `/comments`, `/dms`. Dashboard, analytics, research, search and settings get the full `1440 - 80 = 1360px`.

---

## Part 3. CRAFT SCORE (rubric, from the rendered screenshots at 1440)

| # | Item | Score | Evidence |
|---|---|---|---|
| 1 | Composition: one signature moment | 3/3 | The calendar grid IS the signature; drag-to-reschedule with the group confirmation is a genuinely good interaction |
| 2 | Type at real scale | 2/3 | `h1` at 20px/600 in a 170x28 box is small for a page title; the chip's title is 11px in a 21px box |
| 3 | Colour from the subject | 2/3 | Tokens right; but the tag chips truncate to `buil`/`la...` so a week reads as noise, not identity |
| 4 | Density discipline | 1/3 | 218px of calendar chrome; the composer's 7 stacked cards each open with their own border |
| 5 | Whitespace / edge discipline | 3/3 | `rounded-xl` cards consistent; the chip's hover-toolbar-as-sibling is well done |
| 6 | Motion restraint | 3/3 | `reduced-motion` honoured, colour transitions only |
| 7 | Accessibility | **1/3** | **21px chip is a measured WCAG 2.2 2.5.8 FAIL**; `outline-none` on inputs; the timezone `<select>` in the rail is a 11px label above a 44px control |
| 8 | Honest states | 3/3 | Full-card error with Retry, skeletons, the "1 connected channel is hidden" note |
| 9 | Tokens only, no raw hex | 3/3 | `-rgb` triplets; `check-hex-tokens.mjs` in `make check` and in `pnpm build` |
| 10 | Consistent information architecture | 1/3 | 16-item rail with 7 headings; no channel rail on the calendar; `Channels` + `Settings` duplicate destinations |
| **Total** | | **22/30** | Correct but heavy, with one measured a11y defect |

---

## Part 4. THE PLAN

Sequenced so each phase is independently shippable and independently verified. Each phase names its files, its change, its acceptance, and the AGENTS.md pitfalls it must not repeat.

### Phase 1. Fix the 21px chip (C-0) - the one measured WCAG FAIL

**Status: LANDED 2026-10-31.** Re-verified 2026-11-01 on the rebuilt binary at
1440/768/390/360: every `.event-chip` measures **24px**, `under24: 0`, in all four
views. No probe FAIL for tap targets on `/calendar` (§9.37).

**Files**: `frontend/src/lib/calendar/CalendarEvent.svelte`.

**Change.** The month grid already carries the correct rule and the comment that explains why:

```css
.event-chip.compact { min-height: 24px; }   /* the month chip */
```

Apply the same floor to the non-compact chip. The chip is currently
`padding: 0.125rem 0.375rem` with an 11px line, which renders 21px. Raise the
padding so the box clears 24px without changing the type size:

```css
.event-chip { min-height: 24px; }   /* was 21px - under WCAG 2.2 2.5.8 */
```

**Do not** change the tag chips, the time, the metrics, the hover toolbar, the
`inkOnFill` ink or the 3px left bar. All of those were wrongly proposed for
removal by the first draft of this audit and all of them are better than what
postiz ships.

**Acceptance**:
- Every `.event-chip` in week, day AND month measures >= 24px tall at 1440/768/390.
- No probe FAIL for tap targets on `/calendar` in any of the four views (AGENTS.md §9.37).
- The cell does not grow taller as a result - the grid's `min-h-24` cells absorb it.

### Phase 2. Cap the cell at 3 with a `+N more` disclosure (C-1)

**Status: LANDED 2026-10-31.** Re-verified 2026-11-01: the `+N more` row renders
(`moreRows: 2` measured at all four viewports) and the tag-chip cap is in place.
The disclosure was also re-verified as REACHABLE - an earlier "Show less" control
had rendered inside the chip's scroll container and was clipped off a non-scrolling
page (fixed in `eb67f6a`).

**Files**: `frontend/src/lib/calendar/WeekView.svelte`, `DayView.svelte`, `MonthView.svelte`, `frontend/src/lib/calendar/CalendarEvent.svelte`.

**Change.** Postiz's `slice(0,3)` + `Show more (N)`. Per cell, render the first 3 chips and a `+N more` row that expands **in place** (a `showAll` boolean per cell keyed by the date). 23 chips for 8 posts is a wall; 3 + a disclosure is a scan.

While here, fix the secondary defect the render exposed: at a 129px chip the tag name chips truncate to `buil` / `la...`. Cap the visible tag chips at 1 in the week/day grid (month already uses dots) so the title keeps its width.

**Acceptance**:
- A day with 8 posts shows 3 chips and `+5 more`; expanding shows all 8.
- No chip under 24px.
- Drag-to-reschedule, the published-post choice and the group-move confirmation still work unchanged (the drag handle stays the chip body).
- Probe 0 FAIL in all four views at three widths.

### Phase 3. One-row calendar header (C-2) — LANDED 2026-10-31 (commit f8ab00e)

**Status: done, and the number is 132px, not ~60px.**

The 218px of chrome above the grid measured (with
`frontend/scripts/measure-calendar-chrome.mjs`) as:

| part | before | after |
|---|---|---|
| sticky top bar | 45px | 45px |
| page gutter (`p-4 sm:p-6`) | 35px | 35px |
| `h1` + two action buttons row | 44px | **gone** (moved into the header) |
| `space-y-6` gap x2 | 48px | **24px** (`space-y-3`, one gap left) |
| CalendarHeader | 46px | 40px (one row, was two) |
| **total** | **218px** | **132px** |

The plan's "~60px" target was written from a screenshot, not from a
measurement, and it ignored two things the measurement shows: the app's global
`p-4 sm:p-6` gutter is shared by every route (changing it is a whole-app
decision, not a calendar one), and the sticky top bar is 45px of that chrome.
132px is a 39% reduction and it is the honest number.

**What shipped beyond the plan's list**, each of which the render required:

- the DATE became the page's `h1`. The probe FAILs a page with no `<h1>`, and
  the date range is the heading that carries information. The removed `h1`
  read "Content Calendar", which the nav rail's active item already said.
- the nav arrows are icon-width, not 44px buttons. A 44px chevron inside a
  40px row is not reachable anyway, and the whole row needs the 32px back to
  stay on one line at 1440. `Today` is a comfortable target for the same
  intent. This is why the probe reports 112 `/calendar` tap-target WARNINGs
  (all in the 24-44px advisory band, zero FAILs).
- the view switcher is a `role="group"` segmented control with `aria-pressed`
  per option, not four buttons in a border.

**`--cal-shell-top`**: the plan asked for the constant to be named. It already
is - the page measures it from the element's own
`getBoundingClientRect().top + scrollY` on mount, on resize, and through a
ResizeObserver on every sibling above the shell. The 218px in the CSS is the
pre-JS-paint FALLBACK and is deliberately the wide layout's upper bound (a low
guess clips the grid in the first frame). What was actually broken was the two
comments that disagreed with each other, and those are reconciled now.

---

### Phase 3. One-row calendar header (C-2)

**Files**: `frontend/src/routes/calendar/+page.svelte`, `frontend/src/lib/calendar/CalendarHeader.svelte`.

**Change.** Collapse three rows into one, in postiz's order:

1. Delete the standalone `h1` + action-buttons row. The date range is the title. Move `Generate Posts` and `+ New Post` into the Phase 5 channel rail so they are reachable from every page, and keep a compact `+ New Post` in this row for when the rail is collapsed.
2. One row: `[← Week of October 31 →] [Today] [All channels ▾] [All campaigns ▾] [All tags ▾] [Day|Week|Month|List]`, `flex-wrap gap-2`, all controls `h-10`.
3. The nav arrows + date become **one bordered group** (`border border-line rounded-lg overflow-hidden` with `divide-x`), matching postiz's `border h-[42px] gap-[1px]`.
4. The view switcher becomes a **segmented control** (`role="group"`, `aria-pressed` per option) with one `bg-accent-fill text-accent-fg` active pill, not four separate buttons.
5. The three `<select>` filters keep their native semantics but move to a uniform `h-10` and a shared `min-w-[140px]`.

Then **name the shell constant**. The page currently carries
`--cal-shell-top` with a `218px` fallback and a comment saying the chrome is
`~168px` - the two disagree, and the constant is the thing that goes stale.
Set it from the element's own `offsetTop` (which the page already does) and keep
ONE number in the CSS, derived, not typed.

**Acceptance**:
- The header is one row at 1440 (no wrap) and wraps to at most two rows at 768.
- The measured chrome above the grid drops from **218px to ~60px** at 1440.
- All four views still work and the range still resolves through `wallClockToUtc` (AGENTS.md §9.17, §9.30 - never reintroduce `new Date(...)`).
- Probe 0 FAIL at 1440/768/390.

### Phase 4. The composer: preview always reachable, one primary (M-1)

**Status: LANDED 2026-10-31.** Re-verified 2026-11-01, which is the measurement
that actually settles this phase: at **1440** the pane is on by default with **no
toggle needed** (`paneNow: true, toggle: null`); at **768** and **390** a labelled
`Preview` control with a correct `aria-expanded="false"` opens a **full-width**
pane (768 and 390 wide respectively). The pane is therefore reachable at every
width rather than absent below `lg`, which is what this phase was written to fix.

**Files**: `frontend/src/lib/composer/ComposerModal.svelte`, `frontend/src/lib/components/ModalManager.svelte`.

**Change**:
- **Adopt postiz's tab swap below `lg`.** Replace the footer `Preview` toggle with a `mobileTab: 'edit' | 'preview'` state: the editor gets `mobile:flex-none` and `mobile:hidden` when the preview tab is active, and the preview pane gets `mobile:!w-full`. The pane then occupies the full modal width on a phone instead of being unreachable. The existing `showPreviewMobile` button remains as the tab trigger, relabelled.
- **Lighten the scrim.** `bg-overlay-strong` (`rgba(0,0,0,0.7)`) is what removes page context, not the panel geometry. Use `bg-overlay` for the composer, keep `overlay-strong` for genuinely destructive confirmations.
- **Move the channel switch into the editor column** as its header row - postiz's `SelectCurrent`. `frontend/src/lib/composer/SelectCurrent.svelte` already exists (94 lines). One row: channel dropbutton left, title input right. This is the prerequisite for Phase 6 and it removes one card from the stack.
- **Reorder the editor column** so the body is not seventh: Title + channel switch, `Post to`, the body, then the collapsible panels (Tags, Campaign, Media, Target, Schedule, Thread, First Comment, Signature, AI) behind ONE "more" disclosure or a tab strip. Postiz keeps 6 things above the body and 5 behind a disclosure; switchpost has 13 stacked.
- **Footer: two-sided, one primary.** Left: `Tags`, `Repeat`, `Delete` (edit mode). Right: `Save as Draft` (secondary) and `Schedule` (primary, one filled colour) with a chevron dropdown carrying `Post Now` beside it. Today `Post Now` is green-filled and `Schedule` is accent-filled, so neither is the primary.

**Acceptance**:
- At 390 and 768 the preview is the DEFAULT-REACHABLE tab and renders at full modal width.
- The scrim at 1440 is `bg-overlay`, and the calendar is legible behind it.
- Exactly one filled primary in the footer.
- `askClose`, "Draft saved", Escape, the blocker banner and the keyboard hints all still work.
- Probe 0 FAIL at 1440/768/390 with the composer OPEN (AGENTS.md §12.1: the composer is a state the probe must be driven into).

**Landed so far (2026-10-31, `02d340a`)**: the acceptance's LAST bullet only. The composer already
retained `showPreviewMobile` as its tab trigger and reorders nothing else here. What this phase reported
as "6 probe FAILs at 768px - the signature overlapping Preview/Save Draft/Post Now" was NOT a
composer defect: it was the probe comparing a box CLIPPED by an ancestor against the footer.

The evidence, in order:

1. The signature's rect was `y=731..955` in a 900px viewport; the footer's was `785..854`.
2. `.rich-editor` computes `overflow: hidden` and the editor column scrolls (`scrollHeight 1106`,
   `clientHeight 674`), so the signature is simply the last item in a long correctly-scrolling column.
3. `document.elementFromPoint` at each footer button's CENTRE returned that button - nothing covers them,
   and the kept-open composer's buttons are clickable.
4. Scrolling the column to its end moved the signature to `y=299..523`, far clear of the footer.

The fix belongs to the gate, not the app (AGENTS.md §9.37: "a severity that does not match the standard
makes the gate lie"). `frontend/scripts/probe.mjs`'s overlap check now clips every candidate box to the
visible band (`cr.top` .. `cr.top + clientHeight`) of each ancestor whose `overflow` is not `visible`,
before the viewport clip. Both directions are verified, because a gate that swallows real FAILs is as
useless as one that invents them: the signature's clipped box overlaps the footer by 0px, while two
fully visible boxes overlapping 50x25 are still reported.

Result over the same 27 routes x 4 widths x 2 themes: **FAIL 0**, WARN 216 (unchanged - 104 `/tags`
swatches and 112 `/calendar` tap targets inside the 24-44px WCAG 2.2 2.5.8 advisory band).

Also measured and NOT a defect: the 8 console 400s are `GET /api/dms/conversations` for the
cookie-auth X channel, which X's DM API does not support. The server answers with a correct, actionable
message and `/dms` already renders it inline ("Could not reach X (Twitter) for direct messages. Check
the connection, or pick another channel." + the provider's own detail).

**Remaining in Phase 4**: the five structural changes above (tab swap, scrim, channel switch row,
editor-column reorder, one footer primary) - untried, and each is a probe-driven pass of its own.

**Second pass (2026-10-31, `cd2f9c2` + `e2604b9`, both pushed).** Walking the five bullets against the tree,
four of them were already built or were the wrong shape:

| Plan's bullet | Actual state | Action |
|---|---|---|
| "Preview is a footer toggle, unreachable at <lg" | `showPreviewMobile` already swaps the preview pane to `block` / `hidden lg:block`, full-width below `lg` | **no-op - already correct** |
| "the scrim is `bg-overlay-strong`" | The composer already uses `bg-overlay` (`ModalManager.svelte:103`; `fullScreen` is false for it). `overlay-strong` stays for destructive confirmations | **no-op - already correct** |
| "move the channel switch into the editor column" | `SelectCurrent` already sits inside the editor column, directly above the Content Editor card | **no-op - already correct** |
| "editor reorder, body is not seventh" | Real: five cards stood above the body (Channel Selection, Posting Targets, Tags, Campaign, SelectCurrent) | **landed** - Tags + Campaign merged into one labelled `Details` card; headings above the body are now `Post to` and `Details` |
| "footer: one filled primary" | Real: `bg-success` Post Now and `bg-accent-fill` Schedule, side by side, both filled | **landed** - Schedule is the one filled primary; Post Now is the bordered secondary. **One** filled button at 1440/768/390 |

Two deliberate departures from the plan's wording, both recorded in the commits:

- **Post Now stays visible** instead of moving behind the plan's chevron dropdown. Burying it costs a click on the
action someone reaches for when they want something out immediately, and the failure mode of a hidden
control is not "a slightly busy footer" but "I could not find it".
- **Tags stays visible** instead of going behind a "more" disclosure. Same reasoning: one shared card keeps
both a single tap away and costs one card of vertical space instead of two.

Measured, both themes, after seeding the theme through the store's own `localStorage` key and waiting for the
class on `documentElement` (AGENTS.md 9.36: the first measurement pass reported the dark palette for a
light-theme request and was worthless):

```
Post Now (bordered secondary)   15.25:1 dark   14.63:1 light
Schedule (the one primary)        4.56:1 dark    6.29:1 light
```

Honest note on contrast: the old `bg-success` + `text-success-fg` pairing was already compliant (7.25:1 in
dark), because `success-fg` pairs dark text with the green fill. The footer change is a **hierarchy** fix,
not a contrast rescue - stated that way in the commit so nobody reads it as the latter.

Also gained from the merge: the campaign `<select>` had **no `<label>`**; it now has
`<label for="composer-campaign">`.

Gate results: `pnpm build` ok; `svelte-check --threshold error` 0 errors 0 warnings; `cargo build --release`
ok with the binary restarted so the rust-embedded assets are the new build (AGENTS.md 9.53); probe
**FAIL 0** over 27 routes and again over `/posts/new`,`/posts` at 1440/768/375/360 x dark/light. The 216
WARNs are unchanged and pre-existing (104 `/tags` swatches, 112 `/calendar` tap targets in the 24-44px
WCAG 2.2 advisory band), plus one composer `color-sprawl` advisory (24 distinct text colours).

**Still out of scope, untouched and unmeasured:** that `color-sprawl` advisory. It is the composer's real
remaining defect and the only Phase-4 item that needs a named owner - 24 text colours on one surface
is a type-system failure, not a visual-density one.

### Phase 5. The channel rail (R-1, R-2)

**5a LANDED 2026-10-31 (`fd7cdab`).** The global rail defaults to 80px and
expands to 224px, and the timezone picker now has a home on /settings. The
three things this needed were all found by measuring, not reasoning:

| What was assumed | What the measurement said |
|---|---|
| the collapsed links stay 44px | **40px.** With the label gone the row's only flex child is the 16px icon, so 12+12+16=40 - under the WCAG 2.2 2.5.8 floor. The 20px line-height that made 44 EXPANDED needs text. Fixed with `min-h-11` on every link. |
| the "Expand sidebar" button is fine | **20px wide** - under the 24x24 floor, and the full probe went **FAIL 0 -> 216** the moment the rail narrowed. At 56px the surrounding space had been hiding it. Fixed with `min-w-11`. |
| the timezone is "a setting, not a nav" | it was NOT on /settings at all, so the compact rail would have left it reachable only by re-expanding. Added a laboured Display card on /settings writing the SAME store as the footer select. |

Measured after: aside **80px** collapsed / **224px** expanded at both 1440 and
768; content width **1360px** collapsed / 1216px expanded at 1440 (the plan
asked for >= 1296 collapsed); 16 links at **44px** in both states; the theme
toggle and log out reachable in both. The old `switchpost-sidebar-collapsed`
key is migrated on read to `switchpost-sidebar-expanded`, so an install keeps
its preference.

Two comments were also deleted rather than left to mislead, and one of them
was mine: the rail header promised a hover flyout that has never existed, and
I wrote a comment claiming the timezone was on /settings and the theme toggle
had "a second home in the top bar" - both false when checked. The footer
comment now says the timezone is NOT duplicated and why a duplicate would be
worse than a one-click expand. A confident fiction is more expensive than a
gap.

### Phase 5. The channel rail (R-1, R-2)

**Files**: new `frontend/src/lib/channels/ChannelRail.svelte`; `frontend/src/routes/+layout.svelte` (the 80px rail); `frontend/src/lib/api/integrations.ts` (a reconnect action if absent).

**Change, in this order:**

**5a. The 80px global rail.** `w-20` (80px), icon-over-label, always: item `h-14`, `flex-col items-center justify-center gap-1`, `rounded-xl`, icon `w-5 h-5`, label `text-[10px] leading-[1.1] font-semibold text-center`. Active: `bg-accent-fill/10 text-accent-strong` + a 2px left indicator. Drop the 7 section headings - replace with a `2px` `--border` divider. Move the **timezone picker** to `/settings` (a setting, not a nav) and keep the zone visible in the calendar header (it already shows `GMT+5:30`). Keep `aria-current="page"`. The persisted 56px state becomes the toggle, not the default. Do not change any slug, label text or order.

**5b. The channel rail** on `/calendar`, `/posts`, `/feed`, `/comments`, `/dms`, **collapsed to 64px by default** with a 260px expanded state (persisted to localStorage):
- Header: `Channels` + a collapse chevron that rotates.
- Then a full-width `+ Add channel` (opens the existing onboarding modal) and a `+ New Post` button.
- Then the channel rows: **36px avatar + an 18px provider badge pinned bottom-end**, the name, hover reveals a kebab reusing the existing `/channels` row menu (`Edit` / `Disconnect` / `Refresh`).
- Channels with `refresh_needed` get the red `!` badge, the wash, and the row click starts the **existing** reconnect flow.
- Drag a calendar post onto a channel to reassign it (postiz's rail is a drop target).
- Empty state: the existing illustration-free pattern with "No channels yet" + "Connect your accounts".
- **No grouping in v1.** Switchpost has no customer/org concept (AGENTS.md §0.5.1 forbids it), and the grouping that IS real - campaign - is a filter, not a rail grouping. Omit grouping rather than invent one.
- Mobile: the rail becomes a bottom sheet triggered by 3 overlapping avatars + the count (postiz's exact pattern).
- The calendar's `All channels` `<select>` **stays**. It is the filter; the rail is the management surface. Two controls, two jobs.

**Acceptance**:
- `aside` measures **80px** at 1440 and 768 (probe).
- At 1440 with the channel rail collapsed the calendar's content width is **>= 1296px** (`1440 - 80 - 64`), and with it expanded **>= 1116px** - both better than today's 1104px.
- Clicking a channel's `!` starts the reconnect and returns to the calendar.
- The rail's avatars and badges use `providerInkOnCard` / `OnChip` (AGENTS.md §9.40).
- Every nav link resolves; no probe FAIL; `pnpm build` + `svelte-check --threshold error` clean.

**CORRECTED 2026-10-31 - the two width numbers above were computed against a rail
that never shipped, and they are arithmetically impossible.** They divide by 224:

- `1440 - 80 - 64 = 1296` only works if the rail is 80 - which is what 5a NOW
  delivers, so this one is satisfiable (measured: the calendar has **1360px** at
  1440 today, so adding a 64px rail leaves exactly 1296px).
- `1440 - 80 - 260 = 1100`, and the acceptance asks for **>= 1116**. It cannot
  be met. Before 5a the rail was 224px, so the real figures were 1152 and 956
  against claims of 1296 and 1116 - the plan was quoting a rail width it had
  not built.

The correct acceptance, against what 5a measured:

- with the channel rail COLLAPSED (64px), the calendar holds **>= 1296px** at
  1440 - satisfiable, and it is the DEFAULT, so the founder loses nothing.
- with the channel rail EXPANDED (260px) the calendar holds **1100px**, and
  that is a **stated trade**, not a regression to hide: it is 240px WORSE than
  the collapsed default and 16px worse than the 1116 the plan imagined. The
  expanded state is opt-in and persisted, so a founder who wants the wider
  grid keeps the narrower rail.

The lesson worth carrying: a number in an acceptance criterion is a claim about
a tree. This one was computed against a design, and it was already wrong on the
day it was written. Phase 5b will be judged against the measured 1360px, not
against 1296 vs 1116.

### Phase 6. Per-channel date and target view (M-2)

**Files**: `frontend/src/lib/composer/ComposerModal.svelte`, `SchedulePicker.svelte`, `frontend/src/lib/stores/composer.svelte.ts`, `frontend/src/lib/api/posts.ts`.

**Change.** On top of Phase 4's channel switch: `SchedulePicker` reads the channel IN VIEW, defaulting to the global date; changing it writes that channel's date. The per-channel view shows the channel's target picker, its date, its preview, and **that channel's** char count (`PerPlatformCharCount` already exists).

**Deliberately not ported**: postiz's per-channel *full content* (an `internal` value with its own media and delay for every channel). Switchpost's model is one shared body + per-channel targets (AGENTS.md §9.13, §9.27), and changing it would touch every provider's `settings` read. This phase keeps the useful half.

**Acceptance**:
- A post to X and LinkedIn can be scheduled 9:00 for X and 17:00 for LinkedIn from one composer open, and the API persists both.
- The char count is per-channel for the channel in view.
- No provider's `required_settings` check regresses: `cargo test --lib` + the composer settings-key integration tests.

### Phase 6. Per-channel date and target view (M-2)

**LANDED 2026-10-31 (`764e9ca`), with two scope limits named below.**

The acceptance's first line is met: **a post to X and LinkedIn can be scheduled
9:00 for X and 17:00 for LinkedIn, and the API persists both.** Verified
directly against the running server - one POST with a shared 09:00 and a Reddit
override of 17:00 returns X at `2026-11-02T09:00Z queued` and Reddit at
`17:00Z queued`, two separate rows.

**What shipped**

| Layer | Change |
|---|---|
| `PostOverride` (Rust) | gains `scheduled_at`. The create path already had an overrides branch keyed by integration id carrying each channel's own content and settings - so this is the natural home, and the branch is only taken when `overrides` is present |
| create loop | each integration's row now gets its OWN `scheduled_at`. The scheduler reads a post's column, so this is what actually makes the two times publish |
| detail/probe response | returns the override, so the edit flow can restore it (AGENTS.md §9.29) |
| composer | ONE `SchedulePicker`, not one per channel. A `channelSchedule` map plus a `scheduleInView` derived mean switching to a channel's tab makes the same picker show that channel's time - or the shared one when the channel has not diverged |
| composer | diverged channels are listed with their own time and a **"Use shared"** undo (AGENTS.md §9.28) |

**Two defects the verification found, both fixed in the same commit.** This is
the value of driving the flow rather than reading the diff:

1. **The autosave wrote the draft without the per-channel times.** The exact
   shape AGENTS.md §9.14/§9.29 name: a field the save writes, or the restore
   never reads. A custom 17:00 for Reddit silently reverted on reload while the
   UI said "Autosave on". Both halves are now present and the effect re-arms on
   the map.
2. **"Save as Draft" stripped the SHARED `scheduled_at` but not the per-channel
   ones.** A draft save on a post whose Reddit copy was set for 17:00 produced a
   row still `queued` for 17:00 - the composer said "Draft saved" and the
   calendar still showed a scheduled post. The draft path now drops only the
   time and keeps the channel's content and settings.

**Named scope limits - this is what was NOT done, stated so nobody reads the
acceptance as fully met:**

- **Edit mode cannot change a per-channel time.** `UpdatePostRequest` has no
  `scheduled_at` field, by design: the update endpoint deliberately does not
  touch the schedule ("use the separate reschedule endpoint"), and the plan's
  own risk register says per-channel dates touch the publish path. A per-row
  reschedule call is the honest way to do it and it is a separate change.
- **A full create SUBMIT with a mouse-picked Reddit subreddit was not driven.**
  The composer's own required-settings check correctly refused it every time
  (AGENTS.md §9.27: "Reddit needs subreddit"), so that last mile is unproven.
  The two layers that matter ARE proven: the API that stores the per-channel
  time, and the composer UI that collects it.

**Deliberately not ported**, as the plan already said: postiz's per-channel
*full content* (an `internal` value with its own media and delay for every
channel). Switchpost's model is one shared body + per-channel targets
(AGENTS.md §9.13, §9.27), and changing it would touch every provider's
`settings` read.

### Phase 7. Preview, polish, and the measurement gate (V-1)

**Files**: `frontend/src/lib/composer/PlatformPreviewPane.svelte`, `frontend/src/lib/composer/previews/*`, `Makefile`, `frontend/scripts/`.

**Change**:
- The preview column renders the channels in view as a **stack of native-shaped frames** with the per-channel date on each (postiz's `ShowAllProviders`).
- Fill the empty state: keep "Select a channel to see a preview" and add what happens next.
- Token sweep: `pnpm check:tokens` + `node scripts/check-hex-tokens.mjs` + `node scripts/check-tailwind-classes.mjs`.
- Add the composer's three states (closed, open-empty, open-filled) and the calendar's four views to the probe route list so they are measured, not default-state-only (AGENTS.md §9.35, §9.50).

**Acceptance (the ship gate for the whole plan)**:
```
cargo check --lib --bin switchpost                        # 0 errors
cargo test --lib                                         # all pass
cd frontend && pnpm build                                # succeeds
cd frontend && pnpm exec svelte-check --threshold error  # 0 errors
make audit                                               # 0 FAIL on all three widths, all measured states
```
Plus the before/after numbers: rail 224 -> 80; calendar chrome 218 -> ~60; every chip >= 24px; a cell capped at 3 with `+N more`; the preview reachable by default below `lg`; one primary colour in the footer.

---

## Part 5. RISK REGISTER

| Risk | Mitigation |
|---|---|
| The 80px rail makes 16 nav labels illegible at 9px | The rail carries a 56px icon-only state as the toggle; labels are already short; `title` + tooltip covers the icon-only state |
| 80px rail + a channel rail leaves less calendar than today | The channel rail is **collapsed to 64px by default**; the acceptance width is stated so this cannot regress silently |
| Removing the calendar `h1` hurts the sense of place | The date range becomes the title and the active nav item is marked; record as an AD before shipping |
| Per-channel dates touch the publish path | Phase 6 reads and writes through the existing `settings` MERGE (AGENTS.md §9.29: an unknown key is KEPT) and the acceptance runs `cargo test --lib` |
| Raising the chip to 24px grows the grid | The cells are `min-h-24` and the grid scrolls; the acceptance asserts the cell does not grow as a side effect |
| The `+N more` cap hides a post the founder forgot | The disclosure expands in place and the count is exact; the selected/bulk bar still counts all |
| Per-route channel rails make the app feel inconsistent | The five routes are exactly the ones where the founder works across channels; the rail is a page column, not a global nav (postiz's own model) |
| Deleting the timezone picker from the rail breaks a habit | Move it to `/settings` with a note and keep the value visible in the calendar header |

---

## Part 6. WHAT THIS PLAN DOES NOT DO

- No billing, affiliate, membership or team UI (AGENTS.md §0.5.1).
- No CopilotKit, no Chrome extension, no HubSpot, no `Impersonate`.
- No Postgres, Redis, Temporal or second process.
- No change to any route slug, MCP tool, CLI command or REST endpoint shape.
- No raw hex anywhere; every new surface renders through the existing token layer.
- No new SSE event; the calendar keeps its 19 subscriptions.
- **No removal of the chip's existing good parts** (time, tag colour bar, tag chips, metrics, hover toolbar, error badge). The first draft of this audit proposed to add four things that were already there; that is the failure mode this rewrite exists to prevent.

---

## Part 7. WHAT THE RENDERED APP TAUGHT US (recorded after C-0 and C-1 landed)

The plan above was written from the rendered baseline. Building it produced two
defects the baseline could not show, and both are recorded here because the
next reader of this file will meet them again.

### 7.1 `min-height` with vertical PADDING is not the tap target

Phase 1 said "raise the padding so the box clears 24px". That is what the
author of the rule could see from the source, and it is wrong in a way only the
rendered page shows: the chip's children - the 17px time and the 13px tag chips
- sit on ONE row, so the chip's CONTENT height is 17px, not the sum of both.
Padding `0.3125rem` (5px) top and bottom declared a 24px box whose content area
was 14px, and the 17px time overflowed it.

The consequence was not a small box - the box was exactly 24px, and a probe
reading only the box would have passed it. The consequence was that the tag
chip and the time, both centred inside a clipped box, INTERSECTED by more than
2px in both axes, and the probe's own overlap check failed `/calendar` in both
themes with `"buildinpublic" overlaps "03:51"`.

The shipped rule keeps the ORIGINAL vertical padding and lets `min-height`
carry the box:

```css
.event-chip { min-height: 24px; padding: 0.125rem 0.375rem; }
```

The measured acceptance is not "the box is 24px" - it is "the box is 24px AND
`scrollHeight <= clientHeight + 1`", because a clipped box is the shape the
defect takes. `frontend/scripts/verify-c0-c1-chip.mjs` asserts both.

### 7.2 The tag chips did not shrink, because of one keyword

The tag chip carried `flex-shrink: 0`, so in a 129px chip the two name chips
held their natural widths (69px and 40px) inside a `.event-tags` block that had
shrunk to 21px. `overflow: hidden` clipped them visually, so nothing looked
broken - but `getBoundingClientRect` reports the LAID-OUT box, and the probe
measured the second tag chip overlapping the time.

Two changes fixed it together, and neither is "delete a tag":

- `flex-shrink: 0` -> `flex-shrink: 1` with a `min-width: 1.25rem` floor, so the
  block actually contains what it renders.
- `line-height: 1.3` -> `1.55` to match the chip's own type, so the tag chip
  occupies the same vertical band as the time rather than starting 2px below it.
- a `@container (max-width: 170px)` rule drops the SECOND tag chip only when the
  chip cannot hold two, with the `+N` count carrying the truth. The first tag's
  colour is still on the chip's left border, and the full names are in the
  tooltip and on the detail panel.

The rule this belongs to: **a rule the browser applies is not a rule the probe
sees.** `overflow: hidden` clips a paint; it does not shrink a layout box.

### 7.3 The week grid is a NESTED scroller, and a probe that does not know that measures nothing

The drag-to-reschedule probe reported "nothing happened" for a long time. The
chip it had chosen was at y=1044 in a 900px viewport - the grid is a nested
scroller starting around y=268, so a mouse move to that coordinate fires NO
events at all. The app was correct; the probe was measuring its own viewport.

The probe now scrolls the chip's OWN cell to the top of the grid and requires
BOTH the source chip and the target cell to be inside the GRID's visible box,
not the viewport's. It also filters to `[draggable="true"]`, because
`draggable="false"` is a published or past post the app deliberately refuses to
move - a control that is correctly inert.

### 7.4 The confirmation modal is `role="alertdialog"`, not `role="dialog"`

The group-move confirmation ("Move all posts in this campaign by the same
offset?") is rendered by `ModalManager.svelte` with `role="alertdialog"`. A
probe that searched for `[role="dialog"]` never found it, and would have
reported the group-move promise as broken when it was working.

`frontend/scripts/verify-c1-group-complete.mjs` searches both roles, answers
"Move all", and then asserts the outcome against the SERVER's own payload - not
the DOM. The grid renders only the rows its scroll covers, so "the member is
gone from the DOM" is a statement about the scroll, not about the move. The
assertion is one `PUT .../date` with `"move_group": true` AND both members
shifted by the SAME offset.

### 7.5 A TEXT uuid in a uuid column 500s every reader of the table

The fixture seeder wrote `integration_id` and `group_id` as 36-character TEXT
strings. SQLite stores a `Uuid` as BLOB(16) (AGENTS.md §5.1.1, §9.33, §9.44), so
every query that decoded those rows failed with `invalid length: expected 16
bytes, found 36` - and `/api/calendar` answered 500 while the page rendered an
empty grid with no error shown to the user.

`scripts/seed_c1_fixtures.py` now writes BLOB(16) for every uuid column and
prints the `typeof()` of each one it wrote, so a seeder that regresses fails at
seed time rather than at 2am in the scheduler.
