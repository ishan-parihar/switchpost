<script lang="ts">
  // ChannelRail - the channel management surface beside the publishing routes.
  //
  // Phase 5b of the UI parity plan. Postiz keeps a column of channels beside
  // /launches with avatars, provider badges and a per-row kebab; switchpost
  // had no such surface at all, so a founder managing many accounts left the
  // calendar to see which channel was connected, which needed a reconnect, or
  // to add one at all.
  //
  // WHAT THIS DELIBERATELY DOES NOT DO: drag a calendar post onto a channel to
  // reassign it. The plan asked for it, and it cannot be built honestly -
  // `UpdatePostRequest` (src/api/posts.rs) has no field to change a post's
  // `integration_id`, and it should not: a post's channel is the channel it
  // published to. A rail that accepted a drop and wrote nothing would be
  // AGENTS.md 9.28's shape exactly - a control that looks like it worked. If
  // reassignment is ever wanted it needs a backend capability first, and this
  // comment is the record of why it is absent rather than forgotten.
  //
  // Everything here reuses code that already exists: ChannelQuickMenu is the
  // same kebab /channels uses, ConnectFlow is the one connect entry point, and
  // channelStatus is the single source of truth for "needs reconnect". A second
  // copy of any of those is how the two surfaces drift.
  //
  // Collapsed by default (64px, avatars only) because §2.3 of the plan measured
  // the collision: at 1440 the global rail is 80px, so an always-open 260px
  // channel rail would leave the calendar 1100px - LESS than the 1104px it had
  // before the whole exercise. 64px leaves 1296px. The expanded state is opt-in
  // and persisted, so a founder who wants the roomier rows keeps the smaller grid.

  import {
    integrationsApi,
    type Integration,
  } from '$lib/api/integrations';
  import { providerColor, providerLabel } from '$lib/providers';
  import { channelStatus } from '$lib/channels/channel-status';
  import ChannelQuickMenu from './ChannelQuickMenu.svelte';
  import ConnectFlow from './ConnectFlow.svelte';
  // The provider badge reuses ProviderIcon rather than rendering
  // `providerMeta().icon` as a text glyph. The glyph was set at 8px to fit an
  // 18px badge, and the probe's tiny-text floor (any element with its own text
  // node under 10px) flagged all three channels on every route - a FAIL, not a
  // warn. A glyph set at 10px would not fit an 18px circle either, so the
  // honest fix was to stop drawing an icon as text and use the component that
  // already solves this (brand fill + contrast ink, as SVG).
  import ProviderIcon from './ProviderIcon.svelte';
  import { toast } from '$lib/stores/toast';

  let {
    integrations = [] as Integration[],
    onSelect = (id: string | null) => {},
    selected = null as string | null,
    onChannelsChanged = () => {},
    variant = 'full' as 'full' | 'calendar',
  }: {
    integrations?: Integration[];
    onSelect?: (id: string | null) => void;
    selected?: string | null;
    onChannelsChanged?: () => void;
    /** 'calendar' hides the New Post button (the calendar header owns it). */
    variant?: 'full' | 'calendar';
  } = $props();

  // Persisted, so a founder who widened it does not re-widen it every visit.
  const STORE_KEY = 'switchpost-channel-rail-expanded';
  let expanded = $state(readExpanded());

  function readExpanded(): boolean {
    try {
      return localStorage.getItem(STORE_KEY) === '1';
    } catch {
      // A private-mode or storage-disabled browser must not break the rail.
      return false;
    }
  }

  function toggleExpanded() {
    expanded = !expanded;
    try {
      localStorage.setItem(STORE_KEY, expanded ? '1' : '0');
    } catch {
      // The toggle still works for this visit; only the persistence fails.
    }
  }

  let menuFor = $state<string | null>(null);
  let connectProvider = $state<string | null>(null);
  let reconnecting = $state<string | null>(null);
  // The phone bottom sheet. Not persisted: a sheet that was open when the
  // browser closed should not cover the calendar on the next visit.
  let sheetOpen = $state(false);

  /** Disabled channels cannot publish, so they are noise in a publish rail. */
  let live = $derived(integrations.filter(i => !i.disabled));

  function select(id: string) {
    onSelect(selected === id ? null : id);
  }

  // The reconnect is the EXISTING refresh endpoint. No new backend call.
  async function reconnect(int: Integration) {
    reconnecting = int.id;
    try {
      const r = await integrationsApi.refresh(int.id);
      if (r.error) {
        toast(`Reconnect failed: ${r.error}`, 'error');
      } else {
        toast(`${int.provider_name} reconnected`, 'success');
        onChannelsChanged();
      }
    } catch (e: unknown) {
      toast(`Reconnect failed: ${e instanceof Error ? e.message : String(e)}`, 'error');
    } finally {
      reconnecting = null;
    }
  }

  // ─── WHY THIS FILE DERIVES NOTHING ───────────────────────────
  //
  // Every avatar here is a `ProviderIcon`, which paints the provider's logo
  // (or its curated 2-letter code) on the SOLID brand colour. There is no
  // self-made glyph rule in this file, and there must not be one:
  //
  //   - `initials(provider_name)` gave every Google-family account a "GO"
  //     glyph - three indistinguishable ones, reported by the founder as
  //     conflictingly integrated tags and confirmed by the probe as
  //     "GO" overlaps "GO" on /calendar and /search.
  //   - `glyph()`, which read the curated code instead, then collided with
  //     the ProviderIcon badge BESIDE it: "GS" overlaps "GS".
  //   - A 10px tinted-letter variant of it measured 4.27:1 - 4.36:1, under
  //     the AA floor in both themes, on the calendar channel trigger.
  //
  // All three were the same mistake: a second copy of an identity that
  // `ProviderIcon` already carries (AGENTS.md 9.18). The functions were
  // DELETED rather than left behind, so the next reader cannot wire one back.
</script>

{#if variant === 'calendar' && live.length === 0}
  <!-- Empty state. Illustrated-free, matching the pattern the channels page uses. -->
  <div class="w-16 shrink-0 border-r border-line bg-surface/40 p-2">
    <p class="text-[10px] text-muted text-center leading-tight">No channels yet</p>
  </div>
{:else}
  <!-- BELOW lg the rail is a BOTTOM SHEET, not a narrow side column.

       MeASURED 2026-10-31 at 375px: a side rail of any width cost the
       calendar more than a phone calendar can give up - 64px left the grid
       279px, and a persisted 260px expanded state left it 83px, which is not
       a usable calendar. That second case is why the sheet decided it: a
       founder who widened the rail on a laptop carries the preference to
       their phone via the persisted key and would get the 83px grid. So the
       expanded width is desktop-only by construction rather than by luck.

       The sheet is triggered by 3 overlapping avatars + the count, which is
       the plan's own specified pattern and is a real target (not the rail's
       44px chevron, which a thumb cannot reach at the bottom of a phone). -->
  <div class="lg:hidden fixed inset-x-0 bottom-0 z-40">
    {#if !sheetOpen}
      <button
        type="button"
        onclick={() => (sheetOpen = true)}
        class="min-h-11 min-w-11 mx-auto mb-2 flex items-center gap-2 rounded-full bg-surface border border-line px-3 py-2 elev-lg"
        aria-label={`Show ${live.length} channels`}
      >
        <span class="flex {live.length > 1 ? '-space-x-2' : ''}">
          {#each live.slice(0, 3) as int (int.id)}
            <!-- MEASURED 2026-11-02 at 375/360px, both themes: a 10px glyph on
                 a 28px TINTED circle with a 2px border measures 4.27:1-4.36:1
                 against the composited wash - under the 4.5:1 AA floor for
                 EVERY Google-family glyph (GS 4.27, GM 4.36, CA 4.27), on the
                 calendar's channel trigger. A 10px label in a 24px inner box
                 cannot clear the floor for a washed brand tint; the ink math in
                 providerInkOnTint is correct, the geometry is not.

                 ProviderIcon paints the logo on the SOLID brand colour, where
                 the ink is chosen to clear the floor (white, or black on
                 Google blue) and the glyph is an SVG rather than 10px type -
                 and it is the same treatment every other avatar in this file
                 uses, so the rail is one rule again (AGENTS.md 9.18). -->
            <span class="shrink-0 rounded-full ring-2 ring-surface">
              {#if int.profile_picture}
                <img src={int.profile_picture} alt="" class="w-7 h-7 rounded-full object-cover" />
              {:else}
                <ProviderIcon provider={int.provider_identifier} size="sm" />
              {/if}
            </span>
          {/each}
        </span>
        <span class="text-xs font-medium text-content-secondary">{live.length}</span>
      </button>
    {:else}
      <div class="bg-surface border-t border-line rounded-t-2xl max-h-[70vh] overflow-y-auto p-3 space-y-2 elev-lg">
        <div class="flex items-center justify-between">
          <h2 class="text-sm font-semibold">Channels</h2>
          <button type="button" onclick={() => (sheetOpen = false)}
            class="min-h-11 min-w-11 inline-flex items-center justify-center rounded-lg text-muted hover:text-content"
            aria-label="Close channel list">
            <span aria-hidden="true">×</span>
          </button>
        </div>
        {#each live as int (int.id)}
          {@const status = channelStatus(int)}
          <div class="flex items-center gap-2">
            <button type="button" onclick={() => { select(int.id); sheetOpen = false; }}
              class="flex-1 min-h-11 flex items-center gap-2 rounded-lg px-2 {selected === int.id ? 'bg-accent-fill/15' : 'hover:bg-surface-hover'}"
              aria-pressed={selected === int.id}
            >
              <span class="relative shrink-0">
                {#if int.profile_picture}
                  <img src={int.profile_picture} alt="" class="w-9 h-9 rounded-full object-cover" />
                {:else}
                  <!-- Tint only. The ProviderIcon badge below is the identity
                       and it reads the same curated glyph as `glyph()`, so
                       rendering both put the same two letters on screen
                       twice, one on top of the other - the probe's
                       "GS" overlaps "GS". -->
                  <span class="w-9 h-9 rounded-full"
                    style="background-color: {providerColor(int.provider_identifier)}22"></span>
                {/if}
                <span class="absolute -bottom-0.5 -end-0.5 w-[18px] h-[18px] rounded-full border-2 border-surface overflow-hidden flex items-center justify-center" aria-hidden="true">
                  <ProviderIcon provider={int.provider_identifier} size="sm" />
                </span>
                {#if status.actionable}
                  <span class="absolute -top-0.5 -end-0.5 w-4 h-4 rounded-full bg-error flex items-center justify-center" aria-hidden="true">
                    <span class="w-1.5 h-1.5 rounded-full bg-white"></span>
                  </span>
                {/if}
              </span>
              <span class="min-w-0 flex-1 text-start">
                <span class="block text-xs font-medium truncate">{int.provider_name}</span>
                <span class="block text-[10px] text-muted truncate">{providerLabel(int.provider_identifier)}</span>
              </span>
            </button>
            {#if status.actionable}
              <button type="button" onclick={() => reconnect(int)} disabled={reconnecting === int.id}
                class="min-h-11 min-w-11 inline-flex items-center justify-center rounded-lg text-error hover:bg-error/10"
                aria-label={`Reconnect ${int.provider_name}`}>
                {reconnecting === int.id ? '…' : '!'}
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <aside
    class="hidden lg:flex shrink-0 border-r border-line bg-surface/40 flex-col transition-[width] duration-200
      {expanded ? 'w-[260px]' : 'w-16'}"
    aria-label="Channels"
  >
    <!-- Header: the label only earns the space when there IS space for it. -->
    <div class="flex items-center gap-2 px-2 py-2 border-b border-line {expanded ? 'justify-between' : 'justify-center'}">
      {#if expanded}
        <h2 class="text-xs font-semibold text-content-secondary uppercase tracking-wide">Channels</h2>
      {/if}
      <button
        type="button"
        onclick={toggleExpanded}
        class="min-h-11 min-w-11 inline-flex items-center justify-center rounded-lg text-muted hover:text-content hover:bg-surface-hover transition-colors"
        aria-expanded={expanded}
        aria-label={expanded ? 'Collapse channel rail' : 'Expand channel rail'}
        title={expanded ? 'Collapse channel rail' : 'Expand channel rail'}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-4 h-4 transition-transform {expanded ? 'rotate-180' : ''}" aria-hidden="true">
          <path d="M9 6l6 6-6 6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
    </div>

    {#if variant === 'full'}
      <div class="p-2 space-y-2 border-b border-line">
        <button
          type="button"
          onclick={() => (connectProvider = 'picker')}
          class="w-full min-h-11 inline-flex items-center justify-center gap-1.5 rounded-lg border border-line bg-surface hover:bg-surface-hover text-content text-xs font-medium transition-colors"
          title="Add channel"
        >
          <span aria-hidden="true">+</span>
          {#if expanded}<span>Add channel</span>{/if}
        </button>
      </div>
    {/if}

    <!-- Channel rows. Collapsed shows the avatar only; expanded adds the name. -->
    <nav class="flex-1 overflow-y-auto p-2 space-y-1" aria-label="Channel list">
      {#each live as int (int.id)}
        {@const status = channelStatus(int)}
        <!-- One flex row: the select button, then the actions. The actions are
             IN FLOW, not absolutely positioned. An absolute cluster does not
             participate in flex sizing, so `min-w-0 flex-1` on the name block
             could never be shrunk by it, and the buttons sat ON TOP of the
             channel name - the two-rows-of-buttons distortion the founder
             reported. In flow, the cluster takes its 92px and the name's
             `truncate` does its job. -->
        <div class="flex items-center gap-1">
          <button
            type="button"
            onclick={() => select(int.id)}
            class="min-w-0 flex-1 min-h-11 rounded-lg flex items-center gap-2 px-1.5 transition-colors
              {selected === int.id ? 'bg-accent-fill/15 ring-1 ring-accent/40' : 'hover:bg-surface-hover'}
              {expanded ? '' : 'justify-center'}"
            aria-pressed={selected === int.id}
            title={int.provider_name}
          >
            <!-- Avatar + provider badge bottom-end, the plan's row anatomy. -->
            <span class="relative shrink-0">
              {#if int.profile_picture}
                <img src={int.profile_picture} alt="" class="w-9 h-9 rounded-full object-cover" />
              {:else}
                <!-- TINT ONLY. The 18px ProviderIcon badge below already names
                     the provider, and it reads the SAME curated glyph as
                     `glyph()` - so rendering the text here too put the same two
                     letters on screen twice, one on top of the other. That is
                     what the probe reported as "GS" overlaps "GS" (and, before
                     the glyph fix, "GO" overlaps "GO"): a second copy of an
                     identity already on screen, not a harmless redundancy. -->
                <span
                  class="w-9 h-9 rounded-full"
                  style="background-color: {providerColor(int.provider_identifier)}22"
                ></span>
              {/if}
              <span class="absolute -bottom-0.5 -end-0.5 w-[18px] h-[18px] rounded-full border-2 border-surface overflow-hidden flex items-center justify-center" aria-hidden="true">
                <ProviderIcon provider={int.provider_identifier} size="sm" />
              </span>
              {#if status.actionable}
                <span class="absolute -top-0.5 -end-0.5 w-4 h-4 rounded-full bg-error flex items-center justify-center" title={status.hint} aria-hidden="true">
                  <span class="w-1.5 h-1.5 rounded-full bg-white"></span>
                </span>
              {/if}
            </span>

            {#if expanded}
              <span class="min-w-0 flex-1 text-start">
                <span class="block text-xs font-medium truncate">{int.provider_name}</span>
                <span class="block text-[10px] text-muted truncate">{providerLabel(int.provider_identifier)}</span>
              </span>
            {/if}
          </button>

          {#if expanded}
            <div class="flex items-center gap-1">
              <!-- Reconnect is one call and it is the action the ! badge promises. -->
              {#if status.actionable}
                <button
                  type="button"
                  onclick={() => reconnect(int)}
                  disabled={reconnecting === int.id}
                  class="min-h-11 min-w-11 inline-flex items-center justify-center rounded-lg text-error hover:bg-error/10 transition-colors"
                  title={status.hint}
                  aria-label={`Reconnect ${int.provider_name}`}
                >
                  {#if reconnecting === int.id}
                    <span class="text-[10px]">…</span>
                  {:else}
                    <span class="text-xs font-bold">!</span>
                  {/if}
                </button>
              {/if}
              <button
                type="button"
                onclick={() => (menuFor = menuFor === int.id ? null : int.id)}
                class="min-h-11 min-w-11 inline-flex items-center justify-center rounded-lg text-muted hover:text-content hover:bg-surface-hover transition-colors"
                aria-label={`Actions for ${int.provider_name}`}
                aria-expanded={menuFor === int.id}
              >
                <span aria-hidden="true">⋯</span>
              </button>
            </div>
          {/if}
        </div>
      {/each}
    </nav>

    {#if menuFor}
      {@const int = live.find(i => i.id === menuFor)}
      {#if int}
        <div class="absolute inset-0 z-20" role="presentation" onclick={() => (menuFor = null)}></div>
        <div class="absolute start-2 end-2 bottom-2 z-30">
          <ChannelQuickMenu integration={int} onclose={() => (menuFor = null)} />
        </div>
      {/if}
    {/if}
  </aside>
{/if}

{#if connectProvider === 'picker'}
  <!-- The provider chooser is ConnectFlow's job; a rail that opened its own
       picker would be a second one, and two pickers drift (§9.18). -->
  <ConnectFlow
    provider=""
    show={true}
    onSuccess={() => {
      connectProvider = null;
      onChannelsChanged();
    }}
    onClose={() => (connectProvider = null)}
  />
{/if}
