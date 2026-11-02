<script lang="ts">
  import type { CalendarView } from "./types";
  import type { Tag } from "$lib/api/tags";
  import type { Integration } from "$lib/api/integrations";
  import type { Campaign } from "$lib/api/campaigns";

  let { view, currentDate, onPrev, onNext, onToday, onViewChange, tags = [], selectedTagId = null, onTagFilter,
         onNewPost, onGenerate,
         // v24-2: channel + campaign filter props.
         integrations = [], selectedIntegrationId = null, onIntegrationFilter,
         campaigns = [], selectedCampaignId = null, onCampaignFilter }: {
    view: CalendarView; currentDate: Date;
    onPrev: () => void; onNext: () => void; onToday: () => void;
    onViewChange: (v: CalendarView) => void;
    tags?: Tag[];
    selectedTagId?: string | null;
    onTagFilter?: (tagId: string | null) => void;
    /** Open the composer. Present when the header is the page's only route to it. */
    onNewPost?: () => void;
    /** Open the AI generator. */
    onGenerate?: () => void;
    // v24-2: channel + campaign filter props.
    integrations?: Integration[];
    selectedIntegrationId?: string | null;
    onIntegrationFilter?: (id: string | null) => void;
    campaigns?: Campaign[];
    selectedCampaignId?: string | null;
    onCampaignFilter?: (id: string | null) => void;
  } = $props();

  // v28: postiz's view switch carries no per-button glyphs - the label
  // is the whole control, and emoji rendered as color glyphs that fought
  // both themes. Text-only keeps the segmented control neutral.
  const views: { key: CalendarView; label: string }[] = [
    { key: "day", label: "Day" },
    { key: "week", label: "Week" },
    { key: "month", label: "Month" },
    { key: "list", label: "List" },
  ];

  let title = $derived.by(() => {
    const d = currentDate;
    const m = d.toLocaleDateString("en-US", { month: "long" });
    if (view === "month") return `${m} ${d.getFullYear()}`;
    if (view === "week") return `Week of ${m} ${d.getDate()}`;
    if (view === "day") return d.toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric" });
    return "Upcoming Posts";
  });

  // The select controls share one height and one minimum width so the row
  // reads as a set. `h-10` is 40px - verified MEASURED 2026-10-31 against the
  // nav group's own border box at 40px, and both the selects and the group
  // render 40px at 1440, so no control in this row sticks out of the line.
  //
  // A comment sat here recording "the row wraps anyway - 88px over TWO rows,
  // measured at 1440, 1600, 1920 and 2560... the equal 40px heights keep the
  // wrap clean (two aligned lines, not a ragged edge) - they do not prevent
  // it." That is a defect being described as a design property: three selects
  // each had a hard 120px floor and the two actions were `shrink-0`.
  //
  // MEASURED 2026-11-02 against the app's own compiled stylesheet, the box is
  // 1040px with the channel rail collapsed and 844px with it expanded (the
  // rails are siblings of `<main>` and consume viewport width; `max-w-6xl`
  // caps whatever is left at 1104). The row's content measures 962px at a
  // 120px select basis and 794px at the 64px basis below (three selects,
  // measured as three separate elements - an earlier probe concatenated its
  // markup and only ever rendered one, so its totals were short by two).
  //
  // A "fix" here once swapped the floor for `w-[7.5rem]` and left it at that.
  // app.css sets no `:root` font-size, so `rem` resolves against the 16px
  // browser default and 7.5rem computes to 120px - the floor it replaced. The
  // guard above the class is on PIXELS now, so a width that does not actually
  // change cannot pass.
  //
  // The floor is gone: `w-16` is a 64px BASIS with `min-w-0`, and the actions
  // carry `truncate`. Measured, that holds one line to 800px. Below that no
  // single control reaches, because the nav group (267px) and the view
  // switcher (147px) are `shrink-0` with their own content setting the floor -
  // releasing those is a design decision, not a class swap.
  const selectClass = "h-10 min-w-0 w-16 px-2 bg-background-input border border-line rounded-lg text-xs text-content focus:outline-none focus:border-accent";
</script>

<!-- C-2: ONE row, in postiz's order. The three rows this replaces were
     the `h1` + two action buttons (44px), this header (46px), and the two
     `space-y-6` gaps between them - 218px of chrome above the grid at 1440,
     which the page's own comment said was ~168px while the CSS fallback
     said 218px. Three numbers for one thing is how a constant goes stale.

     218px is HISTORY, NOT THE CURRENT VALUE. It describes what was removed.
     MEASURED 2026-10-31, this header occupies 88px of its own two rows and
     the grid starts at 180px from the top of the viewport - so this row, the
     page padding and the shell's own top margin account for 180px where the
     old three rows accounted for 218px. Anyone reading "218px" as a
     description of what is on screen now will be wrong by 38px; that is why
     the number is spelled out as past tense and the current number is too.

     What is left is the date range as the title, the nav, the filters, the
     view switcher, and the two actions - on ONE line, at the calendar's real
     width.

     MEASURED 2026-11-02, IN A REAL BROWSER AGAINST THIS APP'S OWN COMPILED
     STYLESHEET - because the numbers that used to sit here were wrong, and
     wrong by enough to ship a fix that did nothing.

     THE BOX. `max-w-6xl` (1104px) lives inside `<main>`, which is a SIBLING
     of the `<aside>` in the layout's `flex h-dvh` row - so the rails consume
     viewport width, and the calendar's box is 1104 minus them:
       1040px with the channel rail collapsed   (the measured figure)
        844px with the channel rail expanded     (the reported bug)
     An earlier revision of this comment said "about 700px", and "wraps at
     every viewport". Neither was true: the pre-fix row held one line down to
     900px and only wrapped below that.

     THE ROW'S CONTENT, measured with three selects rendered as three
     separate elements: 962px at the 120px floor, 794px at the 64px basis
     below. Part by part that is nav 267 + views 147 + 3 x select + Generate
     105 + New Post 82 + the gaps.

     THE TRAP THAT SHIPPED A NO-OP. A previous fix replaced the 120px FLOOR
     with `w-[7.5rem]` and called it fixed. `app.css` declares no font-size on
     `:root`, so `rem` resolves against the browser default of 16px and
     7.5rem computes to 120px - the exact value it replaced. The test guarding
     this row asserts on CLASS STRINGS, never on rendered geometry, so a
     change that preserves the width cannot be detected: it was green while
     shipped broken, and a small REGRESSION at 960px besides.

     THE BASIS IS NOW `w-16` (64px), MEASURED, NOT CHOSEN TO LOOK PRECISE.
     One line at 1040 and 844; the row only wraps below 800px, which no
     single control reaches because the nav group (267px) and the view
     switcher (147px) are `shrink-0` with their content setting the floor -
     the date is `whitespace-nowrap`. Releasing THAT is a design decision
     about what the founder loses, so it is named here rather than guessed.

     The three selects take a 64px basis with `min-w-0` instead of a 120px
     floor, and the two action buttons carry `min-w-0 truncate` instead of
     `shrink-0`. The nav group and the view switcher keep `shrink-0`
     deliberately: they hold "Week of October 31" and "Day/Week/Month", where
     truncating destroys the meaning rather than the overflow.

     Nothing here reaches for `new Date(...)`: the range still resolves
     through `wallClockToUtc` in the page (AGENTS.md 9.17). -->
<div class="flex items-center flex-wrap gap-x-2 gap-y-2">
  <!-- The nav arrows and the date are ONE bordered group, so the date reads
       as the thing being navigated rather than a title sitting between two
       loose buttons. `divide-x` gives the seams without a border per child.

       The date is the one part that must never be truncated - "Week of
       October 31" is what the page is for. The ARROWS are the part that can
       give way, so they are icon-width rather than 44px buttons: a 44px tap
       target for a chevron inside a 40px row is not reachable anyway, and
       this row has a `Today` that is a comfortable target for the same
       intent. The nav group measured 296px with generous arrows; trimming
       them keeps the whole header on one line at 1440. -->
  <div class="flex items-stretch border border-line rounded-lg overflow-hidden h-10 shrink-0">
    <button
      onclick={onPrev}
      aria-label="Previous"
      class="px-2 text-sm text-muted hover:text-content hover:bg-surface-hover transition-colors flex items-center justify-center"
    >&larr;</button>
    <button
      onclick={onNext}
      aria-label="Next"
      class="px-2 text-sm text-muted hover:text-content hover:bg-surface-hover transition-colors border-l border-line flex items-center justify-center"
    >&rarr;</button>
    <!-- The DATE is the page's h1. It was a span inside a loose row of
         buttons, with a separate `h1` reading "Content Calendar" above it -
         two headings for one idea, and the one that carried no information
         was the one marked up as the landmark. The date range says which week
         the founder is looking at, which is the thing the heading exists to
         say. The nav rail's active item carries the "Calendar" name, and the
         probe's own rule (no `<h1>` is a FAIL) is satisfied by the range. -->
    <h1 class="px-2.5 text-sm font-semibold border-l border-line flex items-center whitespace-nowrap">{title}</h1>
    <button
      onclick={onToday}
      class="px-2.5 text-xs text-muted hover:text-content hover:bg-surface-hover transition-colors border-l border-line flex items-center"
    >Today</button>
  </div>

  <!-- v24-2: channel filter. Native semantics kept; the geometry is a
       `w-[7.5rem]` BASIS with `min-w-0`, not a 120px floor - a floor is what
       set this row's minimum content width above the calendar's width, and
       forced the wrap. -->
  {#if integrations.length > 0 && onIntegrationFilter}
    <select
      value={selectedIntegrationId ?? ""}
      onchange={(e) => onIntegrationFilter(e.currentTarget.value || null)}
      class={selectClass}
      aria-label="Filter by channel"
    >
      <option value="">All channels</option>
      {#each integrations as integ (integ.id)}
        <option value={integ.id}>{integ.provider_name || integ.provider_identifier}</option>
      {/each}
    </select>
  {/if}
  <!-- v24-2: campaign filter -->
  {#if campaigns.length > 0 && onCampaignFilter}
    <select
      value={selectedCampaignId ?? ""}
      onchange={(e) => onCampaignFilter(e.currentTarget.value || null)}
      class={selectClass}
      aria-label="Filter by campaign"
    >
      <option value="">All campaigns</option>
      {#each campaigns as camp (camp.id)}
        <option value={camp.id}>{camp.name}</option>
      {/each}
    </select>
  {/if}
  {#if tags.length > 0 && onTagFilter}
    <select
      value={selectedTagId ?? ""}
      onchange={(e) => onTagFilter(e.currentTarget.value || null)}
      class={selectClass}
      aria-label="Filter by tag"
    >
      <option value="">All tags</option>
      {#each tags as tag}
        <option value={tag.id}>{tag.name}</option>
      {/each}
    </select>
  {/if}

  <!-- The view switcher is a SEGMENTED CONTROL: one bordered group, one
       accent pill for the active view, `aria-pressed` on every option so the
       state is announced rather than only drawn. It was four independent
       buttons inside a border, which is the same geometry but none of the
       semantics. -->
  <div class="flex items-center border border-line rounded-lg overflow-hidden h-10 shrink-0" role="group" aria-label="Calendar view">
    {#each views as v}
      <button
        onclick={() => onViewChange(v.key)}
        aria-pressed={view === v.key}
        class="px-2.5 text-xs font-medium transition-colors h-full
          {view === v.key ? 'bg-accent-fill text-accent-fg' : 'text-muted hover:text-content hover:bg-surface-hover'}"
      >{v.label}</button>
    {/each}
  </div>

  <!-- The two actions live here now, not in a second row above. They were on
       their own 44px line that existed only to carry them. They are
       `min-w-0 truncate`, NOT `shrink-0`: an action that cannot give way is
       what pushed this row onto a second line. The `title` keeps the full
       label reachable when the text is truncated. -->
  {#if onGenerate}
    <button
      onclick={onGenerate}
      class="h-10 px-2.5 text-xs border border-line rounded-lg text-muted hover:text-content hover:bg-surface-hover transition-colors min-w-0 truncate"
      title="Generate multiple posts from a topic using AI"
    >Generate Posts</button>
  {/if}
  {#if onNewPost}
    <button
      onclick={onNewPost}
      class="h-10 px-2.5 bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded-lg text-xs font-medium transition-colors min-w-0 truncate"
    >+ New Post</button>
  {/if}
</div>
