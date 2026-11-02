<script lang="ts">
  import Icon from "$lib/ui/Icon.svelte";
  import { focusTrap } from '$lib/ui/focus-trap';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { onMount, onDestroy } from "svelte";
  import { feedApi, proxyMediaUrl, type FeedPost, type FeedAccount } from "$lib/api/feed";
  import { integrationsApi, type Integration } from "$lib/api/integrations";
  import { toast } from "$lib/stores/toast";
  import { realtime } from "$lib/stores/realtime";
  import { timezone } from "$lib/stores/timezone.svelte";
  import { providerIcon, providerInk, providerInkOnChip, providerLabel } from "$lib/providers";
  import EngagementCard from "$lib/components/EngagementCard.svelte";
  import MediaCarousel from "$lib/media/MediaCarousel.svelte";

  let posts = $state<FeedPost[]>([]);
  let accounts = $state<FeedAccount[]>([]);
  let connectedIntegrations = $state<Integration[]>([]);
  let loading = $state(true);
  let importing = $state(false);
  let searchQuery = $state("");
  let selectedProvider = $state("all");
  let hasMore = $state(false);
  let nextCursor = $state<string | null>(null);
  // Debounce timer for server-side ?q= search.
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  // Tracks the most recent query we sent to the backend so we can ignore
  // stale responses that arrive after the user has typed more.
  let lastSentQuery = $state<string>("");

  // Build platform list dynamically from connected integrations,
  // using the central provider metadata (R-8) for labels and icons.
  let platforms = $derived.by(() => {
    const connected = connectedIntegrations
      .filter(i => !i.disabled)
      .map(i => i.provider_identifier);
    const unique = [...new Set(connected)];
    return [
      { value: "all", label: "All" },
      ...unique.map(p => ({ value: p, label: providerLabel(p) })),
    ];
  });

  // Filtered results: when the user types a search query, the backend
  // already filters via ?q= (server-side ILIKE across text/author fields),
  // so we only need client-side provider filtering on the response set.
  // When no query is typed, we still apply provider filtering client-side
  // for the "All" tab to avoid an extra round-trip.
  let filteredPosts = $derived(
    posts.filter(p => {
      const matchesProvider = selectedProvider === "all" || p.provider === selectedProvider;
      return matchesProvider;
    })
  );

  // Saved searches (localStorage)
  let savedSearches = $state<string[]>([]);
  let showSaveDialog = $state(false);
  let newSearchName = $state("");

  function loadSavedSearches() {
    try {
      const stored = localStorage.getItem("sf_saved_searches");
      if (stored) savedSearches = JSON.parse(stored);
    } catch { /* ignore */ }
  }

  function saveSearch() {
    if (!searchQuery.trim()) return;
    const entry = searchQuery.trim();
    if (!savedSearches.includes(entry)) {
      savedSearches = [...savedSearches, entry];
      localStorage.setItem("sf_saved_searches", JSON.stringify(savedSearches));
      toast("Search saved", "success");
    }
    showSaveDialog = false;
  }

  function removeSearch(term: string) {
    savedSearches = savedSearches.filter(s => s !== term);
    localStorage.setItem("sf_saved_searches", JSON.stringify(savedSearches));
  }

  async function load() {
    loading = true;
    const q = searchQuery.trim() || undefined;
    // Track which query this request corresponds to so we can ignore
    // stale responses (user typed more before the first response arrived).
    lastSentQuery = q || "";
    const r = await feedApi.list(undefined, undefined, undefined, 100, q);
    if (r.data) {
      // Ignore stale responses - only apply if the user hasn't typed more.
      if (lastSentQuery !== (q || "")) {
        loading = false;
        return;
      }
      posts = r.data.posts;
      hasMore = r.data.has_more;
      nextCursor = r.data.next_cursor;
    } else if (r.error) {
      toast(`Failed to load feed: ${r.error}`, "error");
    }
    loading = false;
  }

  // Debounced search trigger - fires 350ms after the user stops typing.
  // Coalesces rapid keystrokes so we don't spam the backend on every char.
  function scheduleSearch() {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      searchTimer = null;
      nextCursor = null;
      load();
    }, 350);
  }

  function onSearchInput() {
    scheduleSearch();
  }

  async function loadAccounts() {
    const r = await feedApi.accounts();
    if (r.data) {
      accounts = r.data;
    }
  }

  async function importFeed() {
    importing = true;
    const r = await feedApi.import();
    if (r.data) {
      toast(`Imported ${r.data.imported} posts`, "success");
      await load();
    } else if (r.error) {
      toast(`Import failed: ${r.error}`, "error");
    }
    importing = false;
  }

  async function loadMore() {
    if (!nextCursor) return;
    // Pass the same search query through to the next page so pagination
    // stays within the result set of the current search.
    const q = searchQuery.trim() || undefined;
    const r = await feedApi.list(nextCursor, undefined, undefined, 50, q);
    if (r.data) {
      posts = [...posts, ...r.data.posts];
      hasMore = r.data.has_more;
      nextCursor = r.data.next_cursor;
    }
  }

  // Provider icon/color now come from the central $lib/providers module
  // (R-8) - no more local duplicates.

  function formatTime(iso: string): string {
    // Render in the user's selected timezone (F-7) instead of the
    // browser's local timezone. Relative-time strings ("just now",
    // "5h ago") are timezone-independent so they stay as-is.
    const d = new Date(iso);
    const now = new Date();
    const diffH = Math.floor((now.getTime() - d.getTime()) / 3600000);
    if (diffH < 1) return 'just now';
    if (diffH < 24) return `${diffH}h ago`;
    const diffD = Math.floor(diffH / 24);
    if (diffD < 7) return `${diffD}d ago`;
    return timezone.formatDate(iso);
  }

  let unsubscribers: (() => void)[] = [];

  onMount(async () => {
    loadSavedSearches();
    loadAccounts();
    load();
    const integRes = await integrationsApi.list();
    if (integRes.data) connectedIntegrations = integRes.data.integrations;
    // Realtime: when the feed refresher pulls new posts in the
    // background, refresh the search results so they appear without
    // a manual reload. Also refresh on integration changes (new
    // account connected → new feed source available).
    unsubscribers.push(realtime.on('integration_connected', () => {
      loadAccounts();
      load();
    }));
    unsubscribers.push(realtime.on('integration_disconnected', () => {
      loadAccounts();
      load();
    }));
    // Server-side "you missed events" signal - refetch the stale results.
    unsubscribers.push(realtime.on('lagged', () => {
      loadAccounts();
      load();
    }));
  });

  onDestroy(() => {
    unsubscribers.forEach(fn => fn());
    if (searchTimer) clearTimeout(searchTimer);
  });
</script>

<div class="page-enter space-y-6">
  <!-- Header -->
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-xl font-semibold">Search & Discovery</h1>
      <p class="text-sm text-muted mt-1">Search across your connected social media feeds</p>
    </div>
    <button
      onclick={importFeed}
      disabled={importing}
      class="px-3 py-1.5 text-sm whitespace-nowrap bg-surface-hover hover:bg-line min-h-11 inline-flex items-center border border-line rounded-lg transition-colors disabled:opacity-50"
    >
      {importing ? "Importing..." : "Import Feed"}
    </button>
  </div>

  <!-- Search Bar -->
  <div class="flex gap-3">
    <div class="relative flex-1">
      <input
        type="text"
        bind:value={searchQuery}
        oninput={onSearchInput}
        placeholder="Search posts, authors, hashtags..."
        class="min-h-11 w-full px-4 py-2.5 pl-10 bg-background-input border border-line rounded-lg text-sm text-content-secondary placeholder-muted focus:border-accent outline-none transition-colors"
      />
      <span class="absolute left-3 top-1/2 -translate-y-1/2 text-muted"><Icon name="search" class="w-4 h-4" /></span>
    </div>
    <button
      onclick={() => showSaveDialog = true}
      disabled={!searchQuery.trim()}
      class="px-3 py-2 text-sm bg-surface-hover hover:bg-line border border-line rounded-lg transition-colors disabled:opacity-30"
      title="Save this search"
    >
      <Icon name="bookmark" class="w-3.5 h-3.5 inline" /> Save
    </button>
  </div>

  <!-- Saved Searches -->
  {#if savedSearches.length > 0}
    <div class="flex gap-2 flex-wrap">
      {#each savedSearches as term}
        <div class="flex items-center gap-1 px-3 py-1 bg-surface border border-line rounded-full text-xs">
          <button onclick={() => { searchQuery = term; onSearchInput(); }} class="text-muted hover:text-accent transition-colors min-h-11">
            {term}
          </button>
          <button onclick={() => removeSearch(term)} class="text-faint hover:text-error transition-colors ml-1 min-h-11">
            ✕
          </button>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Platform Tabs -->
  <!-- MEASURED 2026-11-02 at 375px/360px: these chips carried a glyph AND a
       label - "GS" + "Google Suite" - and at a 44px min-width each, six of
       them cannot fit 375px. The probe reported "DR Drive" overlapping "YT"
       at both narrow widths, in both themes. They render horizontally with
       `overflow-x-auto`, so they SCROLL rather than wrap - but a chip that
       cannot shrink below its min-width still pushes its neighbours out of
       the scrollport's first screenful.

       The label is already the identity; the glyph prefix was a second copy
       of it (AGENTS.md 9.18), and it is what Overflowed. It goes.

       `min-w-6` (24px), NOT `min-w-11` (44px): six chips cannot fit 375px at
       44px each. 24px is the WCAG 2.2 "Target Size (Minimum)" 2.5.8 floor
       (AGENTS.md 9.37), so the tap target stays legal while the strip
       actually fits the phone. -->
  <div class="flex gap-1 bg-surface border border-line rounded-lg p-1 overflow-x-auto">
    {#each platforms as p}
      <button
        onclick={() => selectedProvider = p.value}
        class="px-3 py-1.5 text-xs rounded-md transition-colors min-h-11 min-w-6 inline-flex items-center justify-center whitespace-nowrap
          {selectedProvider === p.value ? 'bg-accent-fill text-accent-fg' : 'text-muted hover:bg-surface-hover'}"
      >
        {p.label}
      </button>
    {/each}
  </div>

  <!-- Results count -->
  <div class="flex items-center justify-between">
    <span class="text-xs text-muted">
      {filteredPosts.length} result{filteredPosts.length !== 1 ? 's' : ''}
      {searchQuery.trim() ? ` for "${searchQuery}"` : ''}
    </span>
    {#if accounts.length > 0}
      <span class="text-xs text-faint">{accounts.length} accounts tracked</span>
    {/if}
  </div>

  <!-- Results -->
  {#if loading}
    <div class="space-y-3">
      {#each [1, 2, 3] as _}
        <Skeleton height="8rem" rounded="lg" />
      {/each}
    </div>
  {:else if filteredPosts.length === 0}
    <div class="text-center py-16">
      <Icon name="search" class="w-8 h-8 text-muted" />
      <p class="text-content-secondary mt-3 mb-1">
        {#if posts.length === 0}
          No feed data yet. Click "Refresh Feed" to import posts from your connected accounts.
        {:else}
          No results found{searchQuery.trim() ? ` for "${searchQuery}"` : ''}.
        {/if}
      </p>
      {#if posts.length === 0}
        <button onclick={importFeed} disabled={importing} class="mt-3 px-4 py-2 bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded-lg text-sm transition-colors disabled:opacity-50 min-h-11">
          {importing ? "Importing..." : "Import Feed"}
        </button>
      {/if}
    </div>
  {:else}
    <div class="space-y-3">
      {#each filteredPosts as post (post.id)}
        <div class="bg-surface border border-line rounded-xl p-4 hover:border-line transition-colors">
          <div class="flex items-start gap-3">
            <!-- Avatar -->
            {#if post.author_avatar}
              <img src={proxyMediaUrl(post.author_avatar)} alt="" class="w-10 h-10 rounded-full flex-shrink-0 object-cover" />
            {:else}
              <div class="w-10 h-10 rounded-full bg-line flex items-center justify-center flex-shrink-0">
                <!-- The glyph is painted ON the `bg-line` circle, so the ink
                     has to clear 4.5:1 against `--line` (light) /
                     `--bg-hover` (dark) - not against the page background.
                     Against the page it measured 3.67:1 (LinkedIn) and
                     3.82:1 (X) in the light theme, under the AA floor. -->
                <span class="text-sm" style="color: {providerInkOnChip(post.provider)}">{providerIcon(post.provider)}</span>
              </div>
            {/if}

            <div class="flex-1 min-w-0">
              <!-- Author + time -->
              <div class="flex items-center gap-2 mb-1">
                <span class="text-sm font-medium text-content truncate min-w-0">{post.author_name || 'Unknown'}</span>
                {#if post.author_handle}
                  <span class="text-xs text-muted truncate min-w-0">@{post.author_handle}</span>
                {/if}
                <!-- MEASURED 2026-11-02 at 375px: a SECOND provider glyph here -
                     duplicating the 40px avatar circle 15 lines above - left
                     name + handle + glyph + timestamp competing for one row,
                     and the probe reported "DR Drive" overlapping "YT". The row
                     is `min-w-0` on the two truncating spans already, so the
                     only element that could not shrink was the glyph, which is
                     also the one carrying no information the avatar lacks. -->
                <span class="text-xs text-faint ml-auto whitespace-nowrap">{formatTime(post.created_at)}</span>
              </div>

              <!-- Text -->
              <p class="text-sm text-content-secondary mb-2 whitespace-pre-wrap break-words">{post.text}</p>

              <!-- Media -->
              {#if post.media.length > 0}
                <div class="mb-2">
                  <MediaCarousel items={post.media} />
                </div>
              {/if}

              <!-- Engagement + link -->
              <div class="flex items-center gap-4">
                {#if post.engagement}
                  <EngagementCard engagement={post.engagement} provider={post.provider} compact={true} />
                {/if}
                {#if post.url}
                  <!-- min-h-11 gives the link a 44px target. The
                       `ml-auto` keeps it right-aligned; the height
                       does not push the row apart because the row
                       already has the engagement card's height. -->
                  <a href={post.url} target="_blank" rel="noopener" class="text-xs text-accent hover:text-accent-strong transition-colors ml-auto inline-flex items-center min-h-11">
                    View original →
                  </a>
                {/if}
              </div>
            </div>
          </div>
        </div>
      {/each}
    </div>

    <!-- Load more -->
    {#if hasMore}
      <div class="text-center py-4">
        <button onclick={loadMore} class="px-4 py-2 text-sm bg-surface-hover hover:bg-line border border-line rounded-lg transition-colors min-h-11">
          Load More
        </button>
      </div>
    {/if}
  {/if}
</div>

<!-- Save Search Dialog -->
{#if showSaveDialog}
    <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
    <!--
  The scrim is a separate, aria-hidden BUTTON and the panel is its
  SIBLING.

  The wrapper used to carry `aria-hidden="true"`, which made the whole
  dialog invisible to a screen reader, and the shared focus trap skips
  anything in an aria-hidden subtree - so it found no focusable child
  and the focus never entered the dialog. See AGENTS.md 9.39.

  The scrim carries the aria-hidden and the tabindex="-1": a screen
  reader should not offer a nameless "close" button, and Escape
  already closes the dialog.
-->
    <button
      type="button"
      class="absolute inset-0 bg-overlay cursor-default"
      aria-hidden="true"
      tabindex="-1"
      onclick={() => { showSaveDialog = false }}
    ></button>
<div class="relative bg-background-input border border-line rounded-xl p-6 w-full max-w-sm"
      role="dialog"
      aria-modal="true"
      aria-labelledby="save-search-title"
      tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); showSaveDialog = false; } }}
      use:focusTrap
    >
      <h2 id="save-search-title" class="text-lg font-semibold mb-2">Save Search</h2>
      <p class="text-sm text-muted mb-4">Save "{searchQuery}" for quick access later</p>
      <div class="flex gap-3 justify-end">
        <button onclick={() => showSaveDialog = false} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
        <button onclick={saveSearch} class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded-lg min-h-11">Save</button>
      </div>
    </div>
  </div>
{/if}
