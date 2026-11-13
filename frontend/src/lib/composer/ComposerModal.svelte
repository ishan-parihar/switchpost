<script lang="ts">
/**
 * The settings key each provider's TARGET goes in, for the providers
 * whose targets the founder can TYPE.
 *
 * The provider's `targets()` call is the normal way to get a
 * subreddit, a board or a Discord channel: the API lists them. But the
 * list can be empty - Reddit's `subreddits` endpoint needs a scope the
 * app may not have, a Discord bot may be in no channels, a Telegram
 * session may not be readable.
 *
 * And an empty list meant the "Posting Targets" panel was HIDDEN (the
 * panel only rendered when the list had something in it), so the
 * founder saw no way to set the target. The server's required-settings
 * check then said "Reddit needs \"subreddit\" - pick it in Posting
 * Targets before posting" - pointing at a panel that was not on the
 * screen.
 *
 * So the composer can ALWAYS take a typed target, and the key it goes
 * in is the key the provider reads.
 */
const MANUAL_TARGET: Record<string, { key: string; label: string; placeholder: string }> = {
  reddit: { key: 'subreddit', label: 'Subreddit', placeholder: 'rust' },
  pinterest: { key: 'board', label: 'Board', placeholder: 'my-board' },
  discord: { key: 'channel', label: 'Channel', placeholder: 'general' },
};

/**
 * The PROVIDER-SPECIFIC fields whose providers ALREADY read them out of
 * the post's settings.
 *
 * A control in the composer is a PROMISE (AGENTS.md 9.28): if the
 * composer collects a value, something must send it and something must
 * read it. So this table is EXACTLY the set of keys the providers read -
 * no more - and the founder's "postiz has a full range of them" is
 * answered with the range that actually publishes.
 *
 * It lives at the top of the script, not next to the target tables
 * inside `buildPayload`, because the TEMPLATE reads it too: the panels
 * below render one card per selected channel out of this same table, so
 * a field the payload writes cannot be a field the screen never shows.
 *
 * `kind` is the control: `text` for a one-line string, `boolean` for a
 * checkbox.
 *
 * `numeric` marks a key the provider reads as a NUMBER. WordPress's
 * `featured_media` is `settings.get("featured_media").and_then(as_i64)`
 * (src/social/wordpress.rs), so a STRING would be dropped at publish
 * with nothing on screen to say so. The box stays a text box - the id is
 * longer than a spinner holds - and the write parses it, and the panel
 * says so when what was typed is not a number.
 *
 * `altKey` marks the OTHER key the same control writes, for the provider
 * that reads either: Reddit reads `flair_id` and falls back to
 * `flair_text` (src/social/reddit.rs).
 */
const PROVIDER_FIELDS: Record<
  string,
  Array<{
    key: string;
    label: string;
    kind: 'text' | 'boolean';
    placeholder?: string;
    numeric?: boolean;
    altKey?: string;
  }>
> = {
  pinterest: [{ key: 'link', label: 'Link URL', kind: 'text', placeholder: 'https://example.com/your-post' }],
  reddit: [
    { key: 'flair_id', label: 'Flair (id:... or text:...)', kind: 'text', placeholder: 'id:abc123 or text:News', altKey: 'flair_text' },
  ],
  mastodon: [
    { key: 'spoiler_text', label: 'Content warning', kind: 'text', placeholder: 'Spoiler' },
    { key: 'sensitive', label: 'Sensitive content', kind: 'boolean' },
    // Mastodon also reads `settings["language"]` (mastodon.rs:573) and puts
    // it on the post as the content language.
    { key: 'language', label: 'Content language', kind: 'text', placeholder: 'en' },
  ],
  tiktok: [
    { key: 'disable_duet', label: 'Disable duets', kind: 'boolean' },
    { key: 'disable_comment', label: 'Disable comments', kind: 'boolean' },
    { key: 'disable_stitch', label: 'Disable stitches', kind: 'boolean' },
  ],
  wordpress: [
    { key: 'featured_media', label: 'Featured media ID', kind: 'text', numeric: true, placeholder: '123' },
  ],
  hashnode: [{ key: 'publication_id', label: 'Publication ID', kind: 'text', placeholder: 'publication id' }],
  skool: [{ key: 'groupId', label: 'Group ID', kind: 'text', placeholder: 'group id' }],
};

  // ComposerModal - the modal-based composer (Phase 2).
  //
  // This is the keystone of the v18 refactor: it replaces the full-page
  // /posts/new and /posts/[id] edit flows with a single modal that opens
  // in-place from anywhere (calendar, dashboard, posts list, keyboard 'n').
  //
  // Design decisions (per docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md phases F1-F5):
  //   - Full-screen modal (max-w-[1400px], h-[90vh]) - like postiz-app
  //   - Two-column layout on lg+: editor left, preview right (580px)
  //   - Single column on mobile (preview collapses to a toggle)
  //   - askClose: true - confirm before closing if content is non-empty
  //   - Reuses existing composer widgets (ChannelSelector, RichTextEditor,
  //     MediaUpload, SchedulePicker, TagPicker, etc.) - no rewrite
  //   - Create and edit share the same UI; edit mode pre-fills from API
  //
  // The composer store (lib/stores/composer.svelte.ts) drives open/close
  // and entry-point state (presetDate, editingPostId, etc.).

  import { onMount, onDestroy } from 'svelte';
  import { postsApi, type PostDetail, type PostSummary } from '$lib/api/posts';
  import { integrationsApi, type Integration } from '$lib/api/integrations';
  import { toast } from '$lib/stores/toast';
  import { realtime } from '$lib/stores/realtime';
  import { modals } from '$lib/stores/modals.svelte';
  import { composer } from '$lib/stores/composer.svelte';
  import { goto } from '$app/navigation';
  import ChannelSelector from '$lib/composer/ChannelSelector.svelte';
  import RichTextEditor from '$lib/composer/RichTextEditor.svelte';
  import MediaUpload from '$lib/composer/MediaUpload.svelte';
  import SchedulePicker from '$lib/composer/SchedulePicker.svelte';
  import TagPicker from '$lib/composer/TagPicker.svelte';
  import { campaignsApi, type Campaign } from '$lib/api/campaigns';
  import PostSetModal from '$lib/composer/PostSetModal.svelte';
  import ThreadBuilder from '$lib/composer/ThreadBuilder.svelte';
  import FirstComment from '$lib/composer/FirstComment.svelte';
  import AiAssistant from '$lib/composer/AiAssistant.svelte';
  import AiHashtagSuggestions from '$lib/composer/AiHashtagSuggestions.svelte';
  import MusicPicker from '$lib/composer/MusicPicker.svelte';
  import PerPlatformCharCount from '$lib/composer/PerPlatformCharCount.svelte';
  import PlatformPreviewPane from '$lib/composer/PlatformPreviewPane.svelte';
  import SelectCurrent from '$lib/composer/SelectCurrent.svelte';
  import { blockersFor, specFor, type PlatformBlocker } from '$lib/composer/platforms';
  import type { MediaItem } from '$lib/api/media';
  import TargetPicker from '$lib/composer/TargetPicker.svelte';
  import type { TargetInfo } from '$lib/api/integrations';
  import { signaturesApi } from '$lib/api/signatures';
  import { setProviderCharLimits } from '$lib/providers';
  import { wallClockToUtc } from '$lib/composer/wall-clock';
  // `formatDateTime` for the per-channel schedule list - the app's one
  // formatter, so the composer does not add a second way to render a time
  // (AGENTS.md 9.17/9.30: never `new Date(...)` in calendar code).
  import { formatDateTime } from '$lib/calendar/utils';
  import { timezone } from '$lib/stores/timezone.svelte';

  // Props passed by ModalManager (it injects a `close` function).
  let { close: closeProp } = $props<{ close: (confirmed?: boolean) => void }>();

  /**
   * Close the composer.
   *
   * This calls the MODAL STORE's close, which runs the modal's
   * `onClose` - the ONE place that asks "Discard this post?".
   *
   * The component must reach the store BEFORE it flips
   * `composer.open`. `composer.close()` sets `open = false`, the
   * layout's effect then calls `modals.close(id)`, and the store's
   * `onClose` sees `composer.open` already false - so the guard that
   * suppresses the SECOND prompt also suppresses the FIRST one, and
   * the × button discards a dirty draft with no question at all.
   *
   * So the order is: ask the store, the store asks, and only the
   * store's own `onClose` calls `composer.close()`.
   *
   * The default is `true` - "the composer is being closed, there is
   * nothing to ask" - so a programmatic close (a failed load) does not
   * prompt. The × button and Escape pass `false` to ASK.
   */
  function close(confirmed: boolean = true) {
    closeProp(confirmed);
  }

  // ── Form state (mirrors routes/posts/new/+page.svelte) ──────────
  let content = $state('');
  let title = $state('');
  let selectedIntegrations = $state<string[]>([]);

  /**
   * Whether there is unsaved work.
   *
   * The store holds it so the modal's `onClose` - which runs OUTSIDE
   * the component - can read it and ask before discarding.
   */
  let dirty = $derived(
    content.trim().length > 0 ||
      title.trim().length > 0 ||
      selectedIntegrations.length > 0
  );

  $effect(() => {
    composer.dirty = dirty;
  });
  let allIntegrations = $state<Integration[]>([]);
  let integrationProviders = $derived(new Map(allIntegrations.map(i => [i.id, i.provider_identifier])));
  let integrationNames = $derived(new Map(allIntegrations.map(i => [i.id, i.provider_name])));
  let mediaItems = $state<MediaItem[]>([]);
  let scheduledAt = $state<string | null>(null);
  // The schedule as it was when the post was loaded. A save compares
  // against this to know whether the user actually changed the schedule.
  let originalScheduledAt = $state<string | null>(null);
  // Phase 6 (per-channel date and target view): each channel's OWN schedule,
  // keyed by integration id, alongside `providerOverride` which already
  // carries each channel's own body.
  //
  // The map holds only the channels the founder actually DIVERGED on. A
  // channel absent from it uses the shared `scheduledAt`, which is what
  // makes the default behaviour unchanged: one post to two channels with no
  // per-channel edits still publishes both at the shared time.
  let channelSchedule = $state<Map<string, string>>(new Map());
  let recurring = $state<{ intervalDays: number; endDate: string } | null>(null);
  let selectedTagIds = $state<string[]>([]);
  // Phase 6: write the schedule the picker is showing. In the global view
  // that is the shared time (unchanged behaviour); in a channel's view it is
  // that channel's own time.
  //
  // Clearing to null in a channel's view DELETES the channel's divergence
  // rather than writing a null time - a channel with no stored time is a
  // channel that follows the shared schedule, which is the semantics the API
  // and the payload both read.
  function setScheduleInView(iso: string | null) {
    if (editingMode === 'global') {
      scheduledAt = iso;
      return;
    }
    const intId = editingMode.split(':')[1];
    if (iso) {
      channelSchedule.set(intId, iso);
      channelSchedule = new Map(channelSchedule);
    } else {
      channelSchedule.delete(intId);
      channelSchedule = new Map(channelSchedule);
    }
  }

  // Phase 6: drop one channel's own time so it follows the shared schedule
  // again. This is the UNDO for "I set 09:00 for X and I cannot remember
  // why" - a divergence with no visible way back is a setting that traps
  // the founder (AGENTS.md 9.28).
  function clearChannelSchedule(intId: string) {
    channelSchedule.delete(intId);
    channelSchedule = new Map(channelSchedule);
  }

  // Phase 8: campaign selector in composer.
  let allCampaigns = $state<Campaign[]>([]);
  let selectedCampaignId = $state<string | null>(null);
  let editingMode = $state<string>('global');
  // The schedule in view: the channel being edited, or the shared one for
  // the global view. Declared after `editingMode` because it reads it.
  //
  // This is what ONE SchedulePicker renders, and it is why the picker does
  // not need to become one-per-channel: switching to a channel's tab makes
  // the same picker show that channel's time.
  const scheduleInView = $derived(
    editingMode !== 'global'
      ? (channelSchedule.get(editingMode.split(':')[1]) ?? scheduledAt)
      : scheduledAt
  );
  // Which channels have a time of their own.
  const divergedSchedules = $derived(new Set(channelSchedule.keys()));
  let submitting = $state(false);
  let error = $state<string | null>(null);
  let firstComment = $state('');
  let showAi = $state(false);
  let showMusicPicker = $state(false);
  let selectedMusic = $state<{ id: string; title: string; artist: string } | null>(null);
  // The post's stored settings, loaded when editing.
  //
  // The update path REPLACES the settings wholesale, so a save that
  // omits them DELETES the provider settings - the Reddit subreddit, the
  // Pinterest board, the Discord channel, the Instagram audio id. The
  // composer's payload carries them forward so an edit only changes
  // what the founder changed.
  let storedSettings = $state<Record<string, unknown>>({});
  let providerOverride = $state<Map<string, string>>(new Map());
  let showPostSets = $state(false);
  let loading = $state(true);
  // Phase 8: mobile preview toggle - on < lg screens, the preview pane
  // is hidden by default and can be toggled with a button in the footer.
  let showPreviewMobile = $state(false);

  // Phase v21: group-editing state. When editing a post that has a
  // group_id (i.e., it's part of a thread or has a first-comment child),
  // we fetch all sibling posts via postsApi.getGroup() and show a tab
  // strip at the top of the editor so the user can switch between parts.
  // The currently-edited part's content/title/media is what gets saved
  // on submit. Switching tabs saves the current draft to a per-part
  // cache and loads the newly-selected part.
  let groupPosts = $state<Array<{ id: string; sequence: number; content: string; title?: string; media?: { id?: string; url: string; mime_type: string; alt?: string }[] }>>([]);
  let editingGroupPostId = $state<string | null>(null);
  // Per-part draft cache so switching tabs doesn't lose unsaved edits.
  let groupDraftCache = $state<Map<string, { content: string; title: string; mediaItems: MediaItem[] }>>(new Map());

  // v25 F2: draft auto-save indicator is a three-state machine, not a boolean.
  // A boolean could only say "something happened recently"; the composer
  // actually needs to distinguish "idle, autosave armed" from "writing to
  // localStorage right now" from "written". Showing the wrong one teaches
  // people not to trust it.
  type DraftState = 'idle' | 'saving' | 'saved';
  let draftState = $state<DraftState>('idle');
  let draftTimer: ReturnType<typeof setTimeout>;
  const DRAFT_KEY = 'switchpost-composer-draft';

  // Target selection
  //
  // These are PLAIN Map/Set, and they are reassigned after every
  // mutation - the idiom this file already uses for `selectedTargets`,
  // `providerOverride` and `groupDraftCache`.
  //
  // Svelte 5's `proxy()` returns a Map or a Set UNCHANGED (their
  // prototype is neither Object.prototype nor Array.prototype), so
  // `$state(new Map())` is NOT a reactive Map: mutating it in place
  // (`targetsLoading.add(id)`, `integrationTargets.set(id, ...)`)
  // notifies nobody. The template guard
  // `{#if integrationTargets.has(intId)}` is a derived with ZERO
  // reactive dependencies, so it is evaluated once at mount - when the
  // Map is empty - and never recomputed.
  //
  // The result was that the "Posting Targets" panel NEVER rendered: a
  // Reddit subreddit, a Pinterest board or a Facebook Page could not be
  // picked, and the post then failed at publish with "Missing
  // subreddit in settings" - a requirement the founder had no UI to
  // satisfy. The fetch also looped: the `$effect` that calls
  // `fetchTargets` had no dependencies either, so it never re-ran.
  let integrationTargets = $state<Map<string, TargetInfo[]>>(new Map());
  let selectedTargets = $state<Map<string, string[]>>(new Map());
  let targetsLoading = $state<Set<string>>(new Set());
  // A target the founder TYPED, per integration. The provider's own
  // target list is the normal source; this is the fallback for when
  // the list is empty (or the target the founder wants is not in it).
  let manualTargets = $state<Map<string, string>>(new Map());

  // The PROVIDER-SPECIFIC field values, per integration and then per
  // settings key: `{ [integrationId]: { [settingsKey]: value } }`.
  //
  // Keyed by integration because the settings are written PER CHANNEL: a
  // subreddit's flair is not a Skool group's, and one post can go to
  // both.
  //
  // A plain object rather than a Map because `$state` proxies a plain
  // object DEEPLY, so the nested read in the template is tracked
  // (AGENTS.md 9.22: a Map is NOT proxied, and mutating it in place
  // notifies nobody). Every mutation REASSIGNS anyway - the same idiom
  // this file uses for `selectedTargets`, `providerOverride` and
  // `groupDraftCache`.
  let providerFields = $state<Record<string, Record<string, string | boolean>>>({});

  function setProviderField(intId: string, key: string, value: string | boolean) {
    providerFields = {
      ...providerFields,
      [intId]: { ...(providerFields[intId] ?? {}), [key]: value },
    };
  }

  let hasXIntegration = $derived(
    selectedIntegrations.some(id => integrationProviders.get(id) === 'x')
  );
  // v26-4: primary provider for the ThreadBuilder's char limit display.
  // Uses the first selected integration's provider. This is a best-effort
  // UX hint - the actual per-platform validation happens on submit.
  let primaryProvider = $derived(
    selectedIntegrations.length > 0
      ? (integrationProviders.get(selectedIntegrations[0]) || 'x')
      : 'x'
  );
  let hasInstagramIntegration = $derived(
    selectedIntegrations.some(id => {
      const p = integrationProviders.get(id);
      return p === 'instagram' || p === 'instagram-standalone';
    })
  );

  // ── v25 F2: what the composer is about to publish ─────────────────

  let imageCount = $derived(mediaItems.filter(m => m.mime_type?.startsWith('image/')).length);
  let videoCount = $derived(mediaItems.length - imageCount);

  /** Selected channels that will not publish without a title, for the label. */
  let titleRequiredChannels = $derived.by(() => {
    const names: string[] = [];
    for (const id of selectedIntegrations) {
      const p = integrationProviders.get(id);
      if (p && specFor(p)?.titleRequired) names.push(specFor(p)!.label);
    }
    return [...new Set(names)];
  });

  /**
   * Every reason this post cannot be published, across all selected channels.
   *
   * F2's headline change: these BLOCK submit instead of being discovered by the
   * server after a round-trip. The server check in submit() is deliberately
   * still there - this is a UX affordance, that is the authority, and the two
   * read their limits from the same table so they should agree.
   */
  let blockers = $derived.by(() => {
    const out: PlatformBlocker[] = [];
    const seen = new Set<string>();
    for (const id of selectedIntegrations) {
      const p = integrationProviders.get(id);
      if (!p) continue;
      for (const b of blockersFor(p, providerOverride.get(id) ?? content, {
        title,
        mediaCount: mediaItems.length,
        imageCount,
        videoCount,
      })) {
        // Two accounts on one platform produce the identical blocker; showing
        // it twice is noise, not information.
        const key = `${b.provider}|${b.message}`;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push(b);
      }
    }
    return out;
  });
  // A post may carry no body IF it carries media: Instagram, TikTok,
  // YouTube and Pinterest all accept a photo with no caption - the
  // NORMAL case for a photo. The server's rule says exactly this
  // ("An empty body is only an error when the post has no media"), and
  // the composer's own `blockersFor` implements it, so the guard here
  // was the last place still blocking on it: the Schedule and Post Now
  // buttons were greyed out for a photo with no caption.
  let hasContentOrMedia = $derived(content.trim().length > 0 || mediaItems.length > 0);
  let canPublish = $derived(
    selectedIntegrations.length > 0 && hasContentOrMedia && blockers.length === 0,
  );

  /** Pinterest board / YT channel labels for the previews, by integration. */
  let targetNames = $derived.by(() => {
    const m = new Map<string, string>();
    for (const [intId, ids] of selectedTargets) {
      const names = (integrationTargets.get(intId) || [])
        .filter(t => ids.includes(t.id))
        .map(t => t.name);
      if (names.length > 0) m.set(intId, names.join(', '));
    }
    return m;
  });

  let unsubscribers: (() => void)[] = [];

  async function loadIntegrations() {
    const r = await integrationsApi.list();
    if (r.data) {
      allIntegrations = r.data.integrations.filter(i => !i.disabled);
      const validIds = new Set(allIntegrations.map(i => i.id));
      selectedIntegrations = selectedIntegrations.filter(id => validIds.has(id));
    }
  }

  async function loadEditingPost(postId: string) {
    const r = await postsApi.get(postId);
    if (r.error || !r.data) {
      toast(`Failed to load post: ${r.error || 'unknown'}`, 'error');
      close();
      return;
    }
    const post = r.data;
    content = post.content || '';
    title = post.title || '';
    // The stored SETTINGS, restored so a save does not wipe them.
    //
    // The update path REPLACES the settings wholesale, and the composer
    // used to send `{}` - because the detail response had no settings
    // field to restore from. So opening a scheduled Reddit post to fix
    // a typo and saving it DELETED the subreddit, and the post failed
    // at publish with "Missing subreddit in settings". The post was
    // working thirty seconds earlier.
    storedSettings = (post.settings || {}) as Record<string, unknown>;
    // The provider-specific fields, prefilled from what the post already
    // carries. A box that starts EMPTY on a post that already has a
    // flair would show nothing while the stored value still publishes -
    // and a checkbox nobody could tick OFF is worse still: a false read
    // from an empty box is indistinguishable from "the founder never
    // touched this", so a stored `true` could never be turned off.
    const editFields = PROVIDER_FIELDS[integrationProviders.get(post.integration_id) ?? ''];
    if (editFields) {
      const seeded: Record<string, string | boolean> = {};
      for (const f of editFields) {
        const stored = storedSettings[f.key];
        // One box, two keys the provider reads: seed the one that is set
        // so the founder can see which of the two this post carries.
        if (f.altKey) {
          if (typeof stored === 'string') seeded[f.key] = `id:${stored}`;
          else if (typeof storedSettings[f.altKey] === 'string') {
            seeded[f.key] = `text:${storedSettings[f.altKey]}`;
          }
          continue;
        }
        if (f.kind === 'boolean') {
          if (typeof stored === 'boolean') seeded[f.key] = stored;
          continue;
        }
        if (typeof stored === 'string') seeded[f.key] = stored;
        // WordPress's media id is read as a NUMBER and stored as one.
        else if (typeof stored === 'number') seeded[f.key] = String(stored);
      }
      if (Object.keys(seeded).length > 0) {
        providerFields = { ...providerFields, [post.integration_id]: seeded };
      }
    }
    selectedIntegrations = [post.integration_id];
    scheduledAt = post.scheduled_at || null;
    // The schedule as LOADED, so a save can tell "the user changed the
    // schedule" from "the schedule is the same". Without this the
    // reschedule step could not tell a new date from an unchanged one.
    originalScheduledAt = post.scheduled_at || null;
    firstComment = post.first_comment || '';
    selectedTagIds = post.tags?.map(t => t.id) || [];
    mediaItems = (post.media || []).map(m => ({
      // The SERVER'S media id, kept.
      //
      // It used to be a fresh `crypto.randomUUID()`, which threw the id
      // away. The server stores `posts.media` as raw JSON, so the id
      // the composer sent comes back when the post is loaded - and
      // discarding it meant the composer had no way to refer to the
      // stored media row. The alt-text PATCH then addressed a UUID that
      // did not exist and 404'd, so the alt text was unreachable
      // through the EDIT flow - which is how a founder actually uses
      // it: open a scheduled post, add alt text, save.
      //
      // The fallback covers media JSON written before the id existed.
      id: m.id || crypto.randomUUID(),
      url: m.url,
      mime_type: m.mime_type,
      // The alt text lives in `alt`. It used to be stuffed into
      // `original_name`, so the edit payload carried `alt: undefined`
      // and every save DROPPED the alt text the founder had written -
      // and the alt-text indicator in the media strip, which compares
      // `original_name` against the file name, never showed.
      alt: m.alt || '',
      original_name: 'media',
      file_size: 0,
    }));
    editingGroupPostId = post.id;

    // Phase v21: if this post is part of a group (thread or first-comment
    // chain), fetch all sibling posts and populate the group tab strip.
    // The user can then switch between parts without leaving the composer.
    //
    // The integrations are loaded in edit mode too: they are what the
    // per-platform validation, the character rings, the media rules and
    // the preview pane read. Without them, edit mode silently skipped
    // every one of those checks and the preview fell back to "x".
    if (post.group_id) {
      const groupRes = await postsApi.getGroup(post.group_id);
      if (groupRes.data && groupRes.data.length > 1) {
        // Sort by sequence so the tab strip shows parts in order.
        //
        // The part's MEDIA is carried too. It was dropped here, and
        // `switchGroupPart` loads an uncached part with `mediaItems =
        // []` - so switching to a part that had an image showed no
        // image, and saving the part then published it without the
        // image the founder had attached.
        groupPosts = groupRes.data
          .map(p => ({
            id: p.id,
            sequence: p.sequence ?? 0,
            content: p.content,
            title: p.title,
            media: p.media ?? [],
          }))
          .sort((a, b) => a.sequence - b.sequence);
      } else {
        groupPosts = [];
      }
    } else {
      groupPosts = [];
    }
  }

  /**
   * Phase v21: switch the editor to a different part of the thread group.
   * Caches the current part's unsaved edits, then loads the selected part.
   * The cache is keyed by post id and holds {content, title, mediaItems}.
   */
  function switchGroupPart(targetId: string) {
    if (targetId === editingGroupPostId) return;
    // Save current state to cache.
    if (editingGroupPostId) {
      groupDraftCache.set(editingGroupPostId, {
        content,
        title,
        mediaItems: [...mediaItems],
      });
      groupDraftCache = new Map(groupDraftCache);
    }
    // Load target from cache or from the groupPosts snapshot.
    const cached = groupDraftCache.get(targetId);
    if (cached) {
      content = cached.content;
      title = cached.title;
      mediaItems = [...cached.mediaItems];
    } else {
      const part = groupPosts.find(p => p.id === targetId);
      if (part) {
        content = part.content || '';
        title = part.title || '';
        mediaItems = (part.media ?? []).map((m) => ({
          // The server's media id, kept - the same reason as the
          // single-post edit path: a fresh UUID addresses nothing.
          id: m.id || crypto.randomUUID(),
          url: m.url,
          mime_type: m.mime_type,
          alt: m.alt || '',
          original_name: 'media',
          file_size: 0,
        }));
      }
    }
    editingGroupPostId = targetId;
  }

  onMount(async () => {
    loading = true;
    // Pre-fill from composer store entry-point state.
    if (composer.presetDate) {
      // A calendar slot click means "09:00 on that day, in MY time".
      // The old code hard-coded 09:00 UTC, so a user in Berlin got 11:00
      // local - while every other scheduling surface in the app (the
      // SchedulePicker) converted the wall clock through the selected
      // zone. The conversion is shared, so all three entry points agree.
      //
      // The TIME is the one the founder clicked. A ghost slot says
      // "X (Twitter) usually posts at this time" and sits in a row;
      // opening the composer at 09:00 when the row was 08:00 threw
      // away the value they picked.
      scheduledAt = wallClockToUtc(
        composer.presetDate,
        composer.presetTime || '09:00',
        timezone.value
      );
    } else if (composer.mode === 'create') {
      // Restore draft if present.
      const saved = localStorage.getItem(DRAFT_KEY);
      if (saved) {
        try {
          const d = JSON.parse(saved);
          if (d.content) content = d.content;
          if (d.title) title = d.title;
          if (d.selectedIntegrations) selectedIntegrations = d.selectedIntegrations;
          if (d.scheduledAt) scheduledAt = d.scheduledAt;
          if (d.firstComment) firstComment = d.firstComment;
          // The media and the tags were saved but never restored, so a
          // draft the founder came back to had the text and none of the
          // image they had attached or any of the tags they had set.
          if (Array.isArray(d.mediaItems) && d.mediaItems.length > 0) {
            mediaItems = d.mediaItems;
          }
          if (Array.isArray(d.selectedTagIds)) selectedTagIds = d.selectedTagIds;
          // Phase 6: restore the per-channel times. The save writes them as
          // a plain object (a Map does not survive JSON); rebuilding the
          // Map here is what makes a custom 17:00 for Reddit stay custom
          // across a reload. Without this the divergence is gone and the
          // founder re-does it without ever knowing it was lost.
          if (d.channelSchedule && typeof d.channelSchedule === 'object') {
            channelSchedule = new Map(
              Object.entries(d.channelSchedule).filter(([, v]) => typeof v === 'string') as [string, string][]
            );
          }
        } catch {}
      }
    }
    if (composer.presetIntegrationIds.length > 0) {
      selectedIntegrations = [...composer.presetIntegrationIds];
    }
    if (composer.prefilledContent) {
      content = composer.prefilledContent;
    }
    // Phase v21/5.7: load prefilled media items (from repurpose flow).
    // The media URLs already point to uploaded/proxied assets, so we just
    // wrap them in MediaItem-shaped objects - no upload needed.
    if (composer.prefilledMedia && composer.prefilledMedia.length > 0) {
      mediaItems = composer.prefilledMedia.map(m => ({
        // The repurpose flow copies media from another post, so the
        // source may or may not carry a server id. Keep it when it
        // does; fall back when it does not.
        id: m.id || crypto.randomUUID(),
        url: m.url,
        mime_type: m.mime_type,
        alt: m.alt ?? '',
        original_name: m.original_name || m.url.split('/').pop() || 'media',
        file_size: 0,
      }));
    }
    // Phase v21/v22: auto-append the user's default signature when
    // creating a new post (postiz-app pattern). Only fires if:
    //   - we're in create mode (not edit - editing shouldn't re-append)
    //   - no draft was restored (draft takes precedence - user already
    //     has the signature from their last session)
    //   - no prefilled content was set (repurpose/duplicate flows bring
    //     their own content; appending a signature would be noise)
    // The signature is fetched from GET /api/signatures/default, which
    // falls back to the global default if no provider-specific default
    // exists. A 404 (no default set) is silently ignored.
    if (composer.mode === 'create' && !content && !composer.prefilledContent) {
      try {
        const sigRes = await signaturesApi.getDefault();
        if (sigRes.data && sigRes.data.content) {
          // Append on a new line so the signature sits below the post body.
          content = '\n' + sigRes.data.content;
        }
      } catch {
        // 404 or network error - no default signature set, silently skip.
      }
    }
    // The character limits come from the server: the provider
    // implementation owns the limit, and the frontend used to hardcode a
    // second copy that had drifted (X showed 280 while the server
    // enforced 4000, so a 300-character post was blocked and the Post
    // button disabled).
    try {
      const provRes = await integrationsApi.listProviders();
      if (provRes.data) {
        const limits: Record<string, number> = {};
        for (const p of provRes.data) limits[p.identifier] = p.char_limit;
        setProviderCharLimits(limits);
      }
    } catch {
      // The static table is the fallback; a failed fetch just means the
      // composer uses it.
    }

    // The integrations are needed in BOTH modes: they carry the provider
    // for each channel, which is what the per-platform validation, the
    // character ring, the media rules and the preview pane all read. The
    // `if/else` used to load one or the other, so edit mode had an empty
    // integration list and every one of those checks was skipped.
    await loadIntegrations();
    if (composer.mode === 'edit' && composer.editingPostId) {
      await loadEditingPost(composer.editingPostId);
    }

    // Phase 8: load campaigns for the campaign selector.
    const campRes = await campaignsApi.list();
    if (campRes.data) allCampaigns = campRes.data;

    // Realtime: refresh integrations if connected/disconnected in another tab.
    unsubscribers.push(realtime.on('integration_connected', loadIntegrations));
    unsubscribers.push(realtime.on('integration_disconnected', loadIntegrations));
    // Server-side "you missed events" signal - the channel selector is the
    // composer's most staled list, so refetch that.
    unsubscribers.push(realtime.on('lagged', loadIntegrations));

    // Composer keyboard shortcuts (U-4): Cmd+Enter=post, Cmd+S=save draft.
    function onKeydown(e: KeyboardEvent) {
      const isMod = e.metaKey || e.ctrlKey;
      if (!isMod) return;
      if (e.key === 'Enter') {
        e.preventDefault();
        if (!submitting) postNow();
      } else if (e.key === 's' || e.key === 'S') {
        e.preventDefault();
        saveDraftNow();
      }
    }
    window.addEventListener('keydown', onKeydown);
    unsubscribers.push(() => window.removeEventListener('keydown', onKeydown));

    loading = false;
  });

  onDestroy(() => {
    unsubscribers.forEach(fn => fn());
  });

  // Auto-save draft (create mode only). F2: emits idle → saving → saved → idle
  // so the header indicator can name the state it is actually in.
  $effect(() => {
    if (composer.mode !== 'create') return;
    // The dependency list must cover EVERY field the serialised payload
    // reads. `mediaItems` and `selectedTagIds` were serialised but not
    // tracked, and they are read only inside the timer callback -
    // which runs later, outside the effect's tracked body - so they
    // were not dependencies either.
    //
    // The effect: the founder attaches an image and tags the post, then
    // types nothing more. `content` and `title` did not change, so the
    // effect never re-ran, so the draft was NEVER written - and the
    // image and the tags were gone on reload, while the UI said
    // "Autosave on".
    const _ = [content, title, selectedIntegrations, scheduledAt, firstComment, mediaItems, selectedTagIds, channelSchedule];
    clearTimeout(draftTimer);
    // A DRAFT may carry media and no words: a photo with no caption is
    // legal, and the server's own emptiness rule is
    // `clean.is_empty() && media.is_empty()`. The draft guard checked
    // only the words, so a media-only draft was never autosaved and the
    // image was gone on reload.
    if (!content && !title && mediaItems.length === 0) {
      draftState = 'idle';
      return;
    }
    draftState = 'saving';
    const timer = setTimeout(() => {
      localStorage.setItem(DRAFT_KEY, JSON.stringify({
        content, title, selectedIntegrations, scheduledAt, firstComment,
        mediaItems, selectedTagIds,
        // Phase 6: the per-channel times are the SHAPE the bug in this
        // family always takes - a field the save writes, or a field the
        // restore never reads. Both halves are here, or a custom 17:00 for
        // Reddit silently reverts to the shared time on reload while the
        // UI said "Autosave on".
        channelSchedule: Object.fromEntries(channelSchedule),
      }));
      draftState = 'saved';
      // Back to idle (autosave armed) so the indicator does not lie about a
      // save that is no longer happening.
      setTimeout(() => { if (draftState === 'saved') draftState = 'idle'; }, 2500);
    }, 1500);
    return () => clearTimeout(timer);
  });

  function saveDraftNow() {
    if (composer.mode !== 'create') return;
    if (content || title || mediaItems.length > 0) {
      localStorage.setItem(DRAFT_KEY, JSON.stringify({
        content, title, selectedIntegrations, scheduledAt, firstComment,
        mediaItems, selectedTagIds,
        // Phase 6: the per-channel times are the SHAPE the bug in this
        // family always takes - a field the save writes, or a field the
        // restore never reads. Both halves are here, or a custom 17:00 for
        // Reddit silently reverts to the shared time on reload while the
        // UI said "Autosave on".
        channelSchedule: Object.fromEntries(channelSchedule),
      }));
      draftState = 'saved';
      clearTimeout(draftTimer);
      setTimeout(() => { if (draftState === 'saved') draftState = 'idle'; }, 2500);
      toast('Draft saved', 'success');
    }
  }

  function insertAiText(text: string) {
    content = content + (content ? '\n\n' : '') + text;
  }

  function addHashtag(tag: string) {
    content = content + (content.endsWith(' ') ? '' : ' ') + tag;
  }

  async function fetchTargets(integrationId: string) {
    targetsLoading.add(integrationId);
    // Reassign so the template learns the set changed.
    targetsLoading = new Set(targetsLoading);
    try {
      const r = await integrationsApi.listTargets(integrationId);
      // Record the fetch EVEN WHEN THE LIST IS EMPTY.
      //
      // The map's presence means "we have asked this channel for its
      // targets", not "this channel has targets". Most providers return
      // an empty list - the trait default is `Ok(vec![])` - so a guard
      // of the form "not fetched yet" keyed on a non-empty list would
      // re-fetch on every effect run, forever.
      const targets = r.data?.targets ?? [];
      integrationTargets.set(integrationId, targets);
      // Reassign so the template learns the map changed - this is
      // what makes the "Posting Targets" panel appear.
      integrationTargets = new Map(integrationTargets);
    } catch (e) {
      // The fetch failed. Record it anyway, so a provider that errors
      // is not retried on every effect run.
      integrationTargets.set(integrationId, []);
      integrationTargets = new Map(integrationTargets);
      toast('Failed to fetch targets: ' + (e instanceof Error ? e.message : String(e)), 'error');
    } finally {
      targetsLoading.delete(integrationId);
      targetsLoading = new Set(targetsLoading);
    }
  }

  function toggleTarget(integrationId: string, targetId: string) {
    const current = selectedTargets.get(integrationId) || [];
    if (current.includes(targetId)) {
      selectedTargets.set(integrationId, current.filter(t => t !== targetId));
    } else {
      selectedTargets.set(integrationId, [...current, targetId]);
    }
    selectedTargets = new Map(selectedTargets);
  }

  /**
   * v25 F2: load a channel's posting targets (LinkedIn Page, Facebook pages,
   * Reddit subreddits, YouTube channels, Pinterest boards).
   *
   * `fetchTargets` existed and was never called from anywhere, so
   * `integrationTargets` stayed empty and the TargetPicker block below it was
   * dead code - a user could not pick the Facebook Page a post goes to, nor
   * the Pinterest board, which is a hard requirement on that platform. F2
   * wires it: whenever the selected set changes, fetch the missing ones.
   *
   * The guard is "not fetched yet", and `integrationTargets` records EVERY
   * fetch - including one that returned an empty list, and one that
   * errored. Most providers have no posting targets (the trait default is
   * an empty list), so a guard keyed on "the list is non-empty" would
   * re-fetch on every effect run, forever.
   */
  $effect(() => {
    for (const id of selectedIntegrations) {
      if (!integrationTargets.has(id) && !targetsLoading.has(id)) {
        fetchTargets(id);
      }
    }
  });

  function handlePostSetLoad(set: { content: string; channelIds: string[]; scheduledAt?: string | null }) {
    if (set.content) content = set.content;
    if (set.channelIds.length > 0) selectedIntegrations = set.channelIds;
    if (set.scheduledAt) scheduledAt = set.scheduledAt;
  }

  function buildPayload() {
    const overridesObj: Record<string, { content?: string; settings?: Record<string, unknown>; scheduled_at?: string | null }> = {};
    for (const [id, text] of providerOverride) {
      if (text && text !== content) {
        overridesObj[id] = { content: text };
      }
    }

    // Phase 6: the per-channel times ride in the SAME overrides map. The API
    // takes the override branch only when `overrides` is present, so this is
    // also what makes a per-channel schedule reach the create path at all -
    // a divergence with no entry here would be silently dropped.
    //
    // Only the channels that DIVERGED are written. Every other channel keeps
    // the shared `scheduled_at`, which is the whole point: a post to four
    // channels with one custom time is three shared rows and one custom row.
    for (const [id, iso] of channelSchedule) {
      if (!overridesObj[id]) overridesObj[id] = {};
      overridesObj[id].scheduled_at = iso;
    }

    // Per-integration PROVIDER settings.
    //
    // The providers read FLAT, PROVIDER-NAMED keys out of the settings
    // object: Reddit wants `settings["subreddit"]` and
    // `settings["title"]`, Pinterest wants `settings["board"]`. The
    // composer used to send `{integrationId: {target_ids: [...]}}`,
    // which no provider reads - so every Reddit post failed at publish
    // with "Missing subreddit in settings" and every Pinterest post
    // with "Missing board in settings", and Pinterest published the pin
    // with no title at all.
    //
    // The target the user picked (a subreddit, a board, a Facebook page)
    // goes in under the key ITS provider reads, and the title goes in
    // too - the title is stored in `posts.title`, which no publish path
    // reads, so it has to be in the settings for the providers that
    // publish a title.
    // The settings key each target type is written to MUST be the key
    // the provider READS at publish time. The two lists drifted apart:
    // the composer wrote `channel_id` and `page_id`, and NO provider
    // reads either. Discord reads `settings["channel"]` and errors with
    // "Missing channel in settings" - so every Discord post failed at
    // publish, with the founder's chosen channel never used.
    //
    // Facebook and YouTube are not in this table on purpose: they
    // resolve their page and channel from the CONNECTED INTEGRATION's
    // token (`resolve_page_id`, `_internal_id`), not from the post's
    // settings. Writing a `page_id` for them was a dead key.
    //
    // So the table below is exactly the providers that read a target
    // out of the post's settings: Reddit (subreddit) and Pinterest
    // (board). Discord's `channel` is here too.
    const providerKeyForTarget: Record<string, string> = {
      subreddit: 'subreddit',
      board: 'board',
      channel: 'channel',
      // Telegram-user's targets are its dialogs; `target_type` is
      // "peer" or "group" and the id IS the peer id the provider sends
      // to.
      peer: 'target',
      group: 'target',
    };

    // The providers that publish a TITLE, and read it out of the
    // post's settings.
    //
    // The composer wrote `settings["title"]` only for a `subreddit` or
    // a `board` target, so a title typed for Dev.to, Medium,
    // WordPress or Hashnode was DISCARDED - and each of those
    // providers falls back to the first line of the body. The founder
    // typed "How we cut our build time by 40%" and the published
    // article was titled with the first line of the body.
    //
    // The list is here, in the composer, because the composer is what
    // collects the title. `posts.title` is not read by any publish
    // path.
    const TITLE_PROVIDERS = new Set([
      'reddit',
      'pinterest',
      'devto',
      'medium',
      'wordpress',
      'hashnode',
      'skool',
      'youtube',
    ]);

    // The providers that publish a DRAFT unless told otherwise.
    //
    // Dev.to's `published` defaults to false, Medium's `publish_status`
    // to "draft" and WordPress's `status` to "draft". The composer
    // offered no way to change them, so a founder who clicked "Post
    // Now" got the toast "Post published" and a DRAFT on the
    // platform - invisible to the audience, with nothing in the UI
    // saying otherwise.
    //
    // The founder's only actions are "Post Now" and "Schedule", and
    // both mean "publish it". So the composer says so explicitly.
    const PUBLISH_STATUS: Record<string, Record<string, unknown>> = {
      devto: { published: true },
      medium: { publish_status: 'public' },
      wordpress: { status: 'publish' },
    };


    for (const [intId, targets] of selectedTargets) {
      if (targets.length === 0) continue;
      const target = integrationTargets.get(intId)?.find((t) => t.id === targets[0]);
      if (!target) continue;
      const key = providerKeyForTarget[target.target_type];
      if (!key) continue;

      const entry = overridesObj[intId] ?? {};
      // The override's settings REPLACE the post's settings (the server
      // treats an explicit object as a replace, the same as the create
      // path does for a multi-channel post). So the override has to
      // carry the STORED settings forward, or picking a new target on
      // an existing post drops everything else the provider needs -
      // a Reddit post's `title` next to its `subreddit`.
      const perIntegration: Record<string, unknown> = {
        ...storedSettings,
        ...(entry.settings ?? {}),
      };
      perIntegration[key] = target.id;
      entry.settings = perIntegration;
      overridesObj[intId] = entry;
    }

    // A TYPED target, for the providers whose target list was empty.
    //
    // The provider's `targets()` call is the normal source. When it
    // returns nothing, the "Posting Targets" panel is hidden, so the
    // founder had no way to set the target - and the server's
    // required-settings check said "pick it in Posting Targets",
    // pointing at a panel that was not on the screen.
    //
    // The typed value goes in under the key the PROVIDER reads, which
    // is the same key the picked target would have used.
    for (const [intId, value] of manualTargets) {
      if (!value.trim()) continue;
      if (!selectedIntegrations.includes(intId)) continue;
      const provider = integrationProviders.get(intId) ?? '';
      const spec = MANUAL_TARGET[provider];
      if (!spec) continue;
      const entry = overridesObj[intId] ?? {};
      const perIntegration: Record<string, unknown> = {
        ...storedSettings,
        ...(entry.settings ?? {}),
      };
      perIntegration[spec.key] = value.trim();
      entry.settings = perIntegration;
      overridesObj[intId] = entry;
    }

    // The TITLE, for every provider that publishes one.
    //
    // It is not in `posts.title` on the publish path - every provider
    // that publishes a title reads it out of the post's SETTINGS. So
    // the title has to be written into the settings of the providers
    // that need it, and nowhere else.
    //
    // It used to be written only for a `subreddit` or a `board`
    // target, so a title typed for Dev.to, Medium, WordPress or
    // Hashnode was DISCARDED and the provider fell back to the first
    // line of the body.
    for (const intId of selectedIntegrations) {
      const provider = integrationProviders.get(intId) ?? '';
      if (!TITLE_PROVIDERS.has(provider)) continue;
      if (!title.trim()) continue;
      const entry = overridesObj[intId] ?? {};
      const perIntegration: Record<string, unknown> = {
        ...storedSettings,
        ...(entry.settings ?? {}),
      };
      perIntegration.title = title.trim();
      entry.settings = perIntegration;
      overridesObj[intId] = entry;
    }

    // The PUBLISH STATUS, for the providers that default to a draft.
    //
    // The founder's only actions are "Post Now" and "Schedule", and
    // both mean "publish it". Without this the post went out as a
    // draft on Dev.to, Medium and WordPress, and the UI said
    // "Post published".
    for (const intId of selectedIntegrations) {
      const provider = integrationProviders.get(intId) ?? '';
      const status = PUBLISH_STATUS[provider];
      if (!status) continue;
      const entry = overridesObj[intId] ?? {};
      const perIntegration: Record<string, unknown> = {
        ...storedSettings,
        ...(entry.settings ?? {}),
        ...status,
      };
      entry.settings = perIntegration;
      overridesObj[intId] = entry;
    }

    // The PROVIDER-SPECIFIC fields - the ones the providers ALREADY
    // read out of the post's settings.
    //
    // Each control is flattened to the (key, value) pairs its provider
    // reads, and only a value the founder actually gave is written:
    //
    //   - a text box with nothing in it writes NOTHING. An empty string
    //     would overwrite a stored value with "", and the founder who
    //     opened the post to fix the caption would have wiped the flair
    //     they could not see (AGENTS.md 9.29).
    //   - a checkbox nobody touched is ABSENT, so a stored value
    //     survives. A checkbox the founder UN-ticked writes `false`,
    //     because skipping it would leave a `true` they had just turned
    //     off - a control that lies about the post.
    //   - a key the provider reads as a NUMBER (WordPress's media id)
    //     is written as a NUMBER. A string there is dropped at publish,
    //     silently, and a `NaN` does not survive the JSON at all.
    for (const intId of selectedIntegrations) {
      const provider = integrationProviders.get(intId) ?? '';
      const fields = PROVIDER_FIELDS[provider];
      if (!fields) continue;
      const values = providerFields[intId];
      if (!values) continue;

      const writes: Array<[string, unknown]> = [];
      for (const field of fields) {
        const value = values[field.key];
        if (field.altKey && typeof value === 'string') {
          // Reddit's flair: ONE box, and the provider reads `flair_id`
          // and falls back to `flair_text`. The typed value names the
          // key - `id:<id>` is an id, anything else is text. A made-up
          // id is rejected by Reddit, so the default is the text: what
          // the founder can SEE is what they get.
          const typed = value.trim();
          if (!typed) continue;
          if (typed.startsWith('id:')) writes.push([field.key, typed.slice(3).trim()]);
          else writes.push([field.altKey, typed.startsWith('text:') ? typed.slice(5).trim() : typed]);
          continue;
        }
        if (field.kind === 'boolean') {
          if (typeof value === 'boolean') writes.push([field.key, value]);
          continue;
        }
        if (typeof value !== 'string') continue;
        const typed = value.trim();
        if (!typed) continue;
        if (field.numeric) {
          const n = Number(typed);
          if (!Number.isInteger(n)) continue;
          writes.push([field.key, n]);
          continue;
        }
        writes.push([field.key, typed]);
      }
      // Nothing for this channel? Then its settings are left alone
      // entirely - an empty override would replace a stored value.
      if (writes.length === 0) continue;

      const entry = overridesObj[intId] ?? {};
      const perIntegration: Record<string, unknown> = {
        ...storedSettings,
        ...(entry.settings ?? {}),
      };
      for (const [key, value] of writes) {
        if (value === '') continue;
        perIntegration[key] = value;
      }
      entry.settings = perIntegration;
      overridesObj[intId] = entry;
    }

    const settings: Record<string, unknown> = { ...storedSettings };
    if (selectedMusic) {
      settings.audio_id = selectedMusic.id;
      settings.audio_title = selectedMusic.title;
    }
    return {
      integration_ids: selectedIntegrations,
      content,
      // `null` means "the user cleared this". `undefined` means "not
      // mentioned, keep the stored value" - and the backend reads the
      // difference, so emptying the title box actually removes the title
      // instead of silently keeping the old one.
      title: title || null,
      scheduled_at: scheduledAt || undefined,
      tag_ids: selectedTagIds,
      first_comment: firstComment || null,
      // v22 Phase 7: include alt text in the payload (was `alt: undefined`
      // - a no-op that silently dropped all alt text on submit).
      media:
        mediaItems.length > 0
          ? mediaItems.map((m) => ({ id: m.id, url: m.url, mime_type: m.mime_type, alt: m.alt }))
          : null,
      overrides: Object.keys(overridesObj).length > 0 ? overridesObj : undefined,
      // The campaign the post is filed under.
      //
      // The composer's campaign dropdown set a variable that was NEVER
      // read, so the id was dropped on submit - and the kanban's drag
      // PRESERVED the existing value rather than setting one. So there
      // was no UI path that could put a post into a campaign: the
      // campaign's post count was always 0 and its Posts tab was
      // permanently empty.
      campaign_id: selectedCampaignId || undefined,
      settings,
      // Phase v21: wire recurring schedule into the payload so it actually
      // gets sent to the backend. Previously SchedulePicker wrote to
      // `recurring` state but buildPayload() never included it - the field
      // was silently dropped and no repeat series was ever created.
      recurring: recurring ? { interval_days: recurring.intervalDays, end_date: recurring.endDate } : undefined,
    };
  }

  /**
   * Phase v21 fix: edit-mode branch.
   *
   * Previously `submit()`/`postNow()`/`saveAsDraft()` all unconditionally
   * called `postsApi.create()`, even when `composer.mode === 'edit'`. This
   * meant editing a post and clicking "Schedule" created a DUPLICATE post
   * instead of updating the existing one. The `postsApi.update()` method
   * existed but was never called from the UI.
   *
   * Now we branch on `composer.mode === 'edit'` and call
   * `postsApi.update(editingPostId, payload)` instead. The update endpoint
   * accepts content/title/media/settings; it does NOT change scheduled_at
   * (use the separate reschedule endpoint for that) and does NOT change
   * integration_ids (you can't move a post to a different channel after
   * creation - that's a duplicate-and-delete flow).
   *
   * Recurring schedule is wired into the create flow only - for edit mode,
   * the user must use the dedicated "Repeat" action on the post (outside
   * the composer) because the repeat endpoint requires an existing post id.
   */
  async function submit() {
    if (submitting) return;
    if (selectedIntegrations.length === 0) { error = 'Please select at least one channel'; return; }
    if (!content.trim() && mediaItems.length === 0) { error = 'Please write some content or attach a media item'; return; }
    // F2: client-side gate. The server still re-validates below; this stops the
    // request that we already know will fail, and the banner has already told
    // the user why.
    if (blockers.length > 0) { error = 'Fix the highlighted channel issues before scheduling'; return; }
    submitting = true;
    error = null;
    try {
      const payload = buildPayload();
      const valRes = await postsApi.validate(payload);
      if (valRes.data && !valRes.data.valid && valRes.data.errors.length > 0) {
        // Phase v21: show ALL per-platform validation errors as toasts,
        // not just the first one. Format: "X (@handle): post is too long".
        for (const err of valRes.data.errors) {
          toast(`${err.provider_name}: ${err.message}`, 'error');
        }
        const firstErr = valRes.data.errors[0];
        error = firstErr.provider_name + ': ' + firstErr.message;
        return;
      }

      if (composer.mode === 'edit' && composer.editingPostId) {
        // ── Edit mode: update existing post ──
        // v23-5: include tag_ids + first_comment in the update payload
        // (were silently dropped before the v23-5 backend fix).
        const r = await postsApi.update(currentEditingPostId, {
          content: payload.content,
          title: payload.title,
          media: payload.media,
          settings: payload.settings,
          tag_ids: payload.tag_ids,
          first_comment: payload.first_comment,
          // The per-channel body and the per-channel provider settings.
          // The update path used to drop them, so a per-channel override
          // or a Reddit subreddit set in the composer was silently lost
          // on every edit.
          overrides: payload.overrides,
          // The campaign the post is filed under.
          //
          // The update path never wrote the campaign column, so
          // changing the campaign on an EDIT did nothing - the
          // campaign was read-only after creation.
          campaign_id: payload.campaign_id ?? null,
        });
        if (r.error) { error = r.error; return; }

        // Save the OTHER parts of a thread.
        //
        // The composer caches each part's unsaved edits when the user
        // switches tabs, but only the CURRENT part was ever written -
        // so editing part 1, switching to part 2, editing it, and
        // saving kept part 1's original text. The on-screen hint
        // claimed "Switching tabs preserves your unsaved edits per
        // part".
        if (groupDraftCache.size > 0) {
          for (const [partId, draft] of groupDraftCache) {
            if (partId === currentEditingPostId) continue;
            const pr = await postsApi.update(partId, {
              content: draft.content,
              title: draft.title || null,
              media: draft.mediaItems.length > 0
                ? draft.mediaItems.map((m) => ({
                    id: m.id,
                    url: m.url,
                    mime_type: m.mime_type,
                    alt: m.alt,
                  }))
                : null,
            });
            if (pr.error) { error = pr.error; return; }
          }
          groupDraftCache = new Map();
        }

        // Reschedule when the SCHEDULE changed, not when the response
        // happens to carry one. The old gate was
        // `payload.scheduled_at && r.data?.scheduled_at && ...`, so:
        //   - a post that had no schedule (r.data.scheduled_at is null)
        //     never got one, even though the user just picked a date: the
        //     schedule was silently discarded and the toast said
        //     "Post updated";
        //   - a post whose schedule was UNCHANGED still hit the
        //     reschedule endpoint on every save.
        // Compare against the schedule the composer LOADED, which is the
        // user's intent.
        if (payload.scheduled_at !== originalScheduledAt) {
          if (payload.scheduled_at) {
            const rs = await postsApi.reschedule(currentEditingPostId, payload.scheduled_at, false);
            if (rs.error) { error = rs.error; return; }
          } else if (originalScheduledAt) {
            // The user cleared the schedule: put the post back to a draft.
            const us = await postsApi.unschedule(currentEditingPostId);
            if (us.error) { error = us.error; return; }
          }
        }

        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        toast('Post updated', 'success');
      } else {
        // ── Create mode: create new post(s) ──
        // "Schedule" without an instant would create a DRAFT, not a
        // scheduled post - the composer would toast "Post scheduled" and
        // the post would never publish. The SchedulePicker emits `null`
        // when the toggle is on but no date/time is filled in, so this is
        // reachable by clicking Schedule straight after ticking the box.
        if (!payload.scheduled_at) {
          error = 'Pick a date and time to schedule, or use "Post Now" to publish immediately';
          return;
        }
        const r = await postsApi.create(payload);
        if (r.error) { error = r.error; return; }

        // Phase v21: wire recurring schedule. After successful creation,
        // if recurring is set, call postsApi.repeat() to spawn the series.
        // Each channel gets its own recurring series, so repeat every created post.
        if (recurring && r.data?.posts?.length) {
          let copies = 0;
          const failures: string[] = [];
          for (const p of r.data.posts) {
            const repeatRes = await postsApi.repeat(p.id, recurring.intervalDays, recurring.endDate);
            if (repeatRes.error) failures.push(`${p.integration_name}: ${repeatRes.error}`);
            else copies += repeatRes.data?.count || 0;
          }
          if (failures.length) {
            toast(`Repeat setup failed for ${failures.join('; ')}`, 'error');
          } else {
            toast(`Post scheduled + ${copies} recurring copies created`, 'success');
          }
        }

        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        if (!recurring) toast('Post scheduled', 'success');
      }
    } catch (e: unknown) {
      error = (e instanceof Error ? e.message : String(e)) || 'Failed to schedule';
    } finally {
      submitting = false;
    }
  }

  async function postNow() {
    if (submitting) return;
    if (selectedIntegrations.length === 0) { error = 'Please select at least one channel'; return; }
    if (!content.trim() && mediaItems.length === 0) { error = 'Please write some content or attach a media item'; return; }
    if (blockers.length > 0) { error = 'Fix the highlighted channel issues before posting'; return; }
    submitting = true;
    error = null;
    try {
      const payload = buildPayload();
      const valRes = await postsApi.validate(payload);
      if (valRes.data && !valRes.data.valid && valRes.data.errors.length > 0) {
        for (const err of valRes.data.errors) {
          toast(`${err.provider_name}: ${err.message}`, 'error');
        }
        const firstErr = valRes.data.errors[0];
        error = firstErr.provider_name + ': ' + firstErr.message;
        return;
      }

      if (composer.mode === 'edit' && composer.editingPostId) {
        // Edit mode: update content, then publish immediately.
        const r = await postsApi.update(currentEditingPostId, {
          content: payload.content,
          title: payload.title,
          media: payload.media,
          settings: payload.settings,
        });
        if (r.error) { error = r.error; return; }
        const pub = await postsApi.publish(currentEditingPostId);
        if (pub.error) { error = pub.error; return; }
        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        toast('Post published', 'success');
      } else {
        // Create mode: create + publish.
        const r = await postsApi.create({ ...payload, scheduled_at: undefined });
        if (r.error) { error = r.error; return; }
        // One post per selected channel: publish every one, not just the first.
        const failures: string[] = [];
        for (const p of r.data?.posts ?? []) {
          const pub = await postsApi.publish(p.id);
          if (pub.error) failures.push(`${p.integration_name}: ${pub.error}`);
        }
        if (failures.length) { error = failures.join('; '); return; }
        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        toast('Post published', 'success');
      }
    } catch (e: unknown) {
      error = (e instanceof Error ? e.message : String(e)) || 'Failed to post';
    } finally {
      submitting = false;
    }
  }

  async function saveAsDraft() {
    if (submitting) return;
    // Media counts: a photo with no caption is a legal draft, and the
    // server's emptiness rule is `clean.is_empty() && media.is_empty()`.
    if (!content.trim() && !title.trim() && mediaItems.length === 0) {
      error = 'Please write some content, add a title, or attach a media item';
      return;
    }
    submitting = true;
    error = null;
    try {
      const payload = buildPayload();
      // Save as draft: no scheduled_at, state will default to draft
      if (composer.mode === 'edit' && composer.editingPostId) {
        // v24-1: edit-mode saveAsDraft now properly transitions the post
        // back to draft state via the new /unschedule endpoint (was a
        // TODO that used the "100 years in the future" hack). Also
        // includes tag_ids + first_comment in the update (v23-5 fix).
        const updateR = await postsApi.update(currentEditingPostId, {
          content: payload.content,
          title: payload.title,
          media: payload.media,
          settings: payload.settings,
          tag_ids: payload.tag_ids,
          first_comment: payload.first_comment,
        });
        if (updateR.error) { error = updateR.error; return; }
        // If the post was scheduled (queued state), unschedule it back to draft.
        if (updateR.data?.state === 'queued' || updateR.data?.scheduled_at) {
          const unschedR = await postsApi.unschedule(currentEditingPostId);
          if (unschedR.error) {
            // Non-fatal - the content was updated; just warn.
            console.warn('Failed to unschedule:', unschedR.error);
          }
        }
        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        toast('Draft updated', 'success');
      } else {
        // A DRAFT has no schedule, in BOTH places. The shared
        // `scheduled_at` was already stripped here; Phase 6's per-channel
        // times were not, so "Save as Draft" on a post whose Reddit copy
        // was set for 17:00 produced a row that was still QUEUED for 17:00
        // - the composer said "Draft saved" and the calendar still showed
        // a scheduled post.
        //
        // The channel overrides are kept for content and settings (the
        // Reddit subreddit and the title live there), so only the time is
        // dropped - the same three-state read as everywhere else: no
        // `scheduled_at` in the override means "follow the post", and a
        // draft's post has no time.
        const draftOverrides: typeof payload.overrides = payload.overrides
          ? Object.fromEntries(
              Object.entries(payload.overrides).map(([k, v]) => {
                const { scheduled_at: _drop, ...rest } = v;
                return [k, rest];
              })
            )
          : undefined;
        const r = await postsApi.create({ ...payload, scheduled_at: undefined, overrides: draftOverrides });
        if (r.error) { error = r.error; return; }
        localStorage.removeItem(DRAFT_KEY);
        composer.close();
        toast('Draft saved', 'success');
      }
    } catch (e: unknown) {
      error = (e instanceof Error ? e.message : String(e)) || 'Failed to save draft';
    } finally {
      submitting = false;
    }
  }

  // askClose: if content is non-empty, confirm before closing.
  /**
   * The DISCARD CONFIRMATION lives in the modal's `onClose` (in
   * +layout.svelte): the store calls it before it closes, and it is the
   * ONE place that asks "Discard this post?".
   *
   * The component used to have its own copy of the question. Two
   * questions for one close meant the second one's promise was nobody's
   * to await, and the pending confirm was left set FOREVER - so the
   * next Escape on any page in the app was swallowed.
   */

  let isDirty = $derived(!!content.trim() || !!title.trim() || selectedIntegrations.length > 0);

  // Phase v21: the post id to send update/publish/reschedule calls to.
  // Falls back to composer.editingPostId when the user hasn't switched
  // tabs (or when there's no group - single-post edit).
  let currentEditingPostId = $derived(editingGroupPostId || composer.editingPostId);

  // Phase 3: compute which integrations have diverged from global.
  // An integration "diverged" when it has an override in providerOverride
  // AND that override differs from the current global content.
  // This drives the pink dot on the SelectCurrent pill strip.
  let divergedIntegrations = $derived.by(() => {
    const set = new Set<string>();
    for (const [intId, text] of providerOverride) {
      if (text && text !== content) {
        set.add(intId);
      }
    }
    return set;
  });

  // Phase 3: when the user switches to a per-channel tab for the first
  // time (no override exists yet), clone the global content into the
  // override so the editor starts from global and then diverges.
  // This matches postiz-app's addRemoveInternal() clone-on-first-switch.
  function handleCurrentChange(tab: string) {
    if (tab !== 'global' && !providerOverride.has(tab)) {
      // Clone global → internal[tab] so the editor has something to edit.
      providerOverride.set(tab, content);
      providerOverride = new Map(providerOverride);
    }
    editingMode = tab === 'global' ? 'global' : `internal:${tab}`;
  }

  // Phase 3: remove a per-channel override (reset to global).
  // Called by SelectCurrent's X button.
  function handleRemoveOverride(integrationId: string) {
    providerOverride.delete(integrationId);
    providerOverride = new Map(providerOverride);
    // If we were editing this channel, switch back to global.
    if (editingMode === `internal:${integrationId}`) {
      editingMode = 'global';
    }
  }
</script>

<div class="flex flex-col h-full">
  <!-- Header -->
  <div class="flex items-center justify-between px-5 py-3 border-b border-line shrink-0 gap-3">
    <!-- v25 F5: `flex-wrap` + `min-w-0` on the left group. In the steady state
         this row measures ~235px inside the 328px the composer gets at a 360px
         viewport, but the `saving` branch adds a spinner plus a label and
         pushes it to ~333px - so the header overflowed during every autosave.
         Wrapping is the right fix rather than shortening the label: all three
         states are load-bearing, and the steady state is unaffected. -->
    <div class="flex items-center gap-3 flex-wrap min-w-0">
      <h2 class="text-xl font-semibold">
        {composer.mode === 'edit' ? 'Edit Post' : 'Create Post'}
      </h2>
      {#if composer.mode === 'edit'}
        <span class="text-xs px-2 py-0.5 rounded-full bg-accent-fill/20 text-accent">Editing</span>
      {/if}
      <!-- F2: the autosave indicator names its state instead of blinking. -->
      {#if draftState === 'saving'}
        <span class="text-xs text-muted flex items-center gap-1.5" role="status">
          <span class="inline-block w-2.5 h-2.5 border-2 border-line border-t-accent rounded-full animate-spin" aria-hidden="true"></span>
          Saving draft
        </span>
      {:else if draftState === 'saved'}
        <span class="text-xs text-success flex items-center gap-1" role="status">Draft saved</span>
      {:else}
        <span class="text-xs text-faint" title="Drafts are kept in this browser and restored when you open a new post.">Autosave on</span>
      {/if}
    </div>
    <div class="flex items-center gap-3">
      <span class="hidden lg:inline text-[10px] text-faint" title="Keyboard shortcuts">
        <kbd class="px-1 py-0.5 bg-surface-hover rounded">⌘</kbd>+<kbd class="px-1 py-0.5 bg-surface-hover rounded">↵</kbd> post ·
        <kbd class="px-1 py-0.5 bg-surface-hover rounded">⌘</kbd>+<kbd class="px-1 py-0.5 bg-surface-hover rounded">S</kbd> save
      </span>
      <button
        onclick={() => close(false)}
        class="text-muted hover:text-content text-2xl leading-none -mt-1 min-h-11 min-w-11 flex items-center justify-center"
        aria-label="Close composer"
      >&times;</button>
    </div>
  </div>

  {#if loading}
    <div class="flex-1 flex items-center justify-center">
      <div class="text-sm text-muted">Loading...</div>
    </div>
  {:else}
    <!-- Two-column body: editor left, preview right. On <lg the flex direction
         is already column, so the sections stack in DOM order.
         `min-h-0` is REQUIRED: a flex item's default `min-height: auto`
         means it will not shrink below its content's height, so the
         columns' `overflow-y-auto` never engaged - the body grew past
         the panel and the action bar sat ON TOP of the content. At
         375px the "Campaign" select was cut in half by the footer. -->
    <div class="flex-1 min-h-0 flex flex-col lg:flex-row overflow-hidden">
      <!-- Left column: editor (scrollable) -->
      <div class="flex-1 min-w-0 min-h-0 overflow-y-auto p-4 sm:p-5 space-y-4">
        {#if error}
          <div class="bg-error/10 border border-error/30 text-error text-sm rounded-lg p-3" role="alert">{error}</div>
        {/if}

        <!-- F2: every blocking reason at once, not one per attempt. -->
        {#if blockers.length > 0}
          <div class="bg-warning/10 border border-warning/30 rounded-lg p-3" role="alert">
            <p class="text-sm font-medium text-warning">
              Not ready to publish ({blockers.length} issue{blockers.length > 1 ? 's' : ''})
            </p>
            <ul class="mt-1.5 space-y-0.5">
              {#each blockers as b (`${b.provider}-${b.message}`)}
                <li class="text-xs text-content-secondary">
                  <span class="text-muted">{b.label}</span> - {b.message}
                </li>
              {/each}
            </ul>
          </div>
        {/if}

        <!-- Phase v21: group/thread tab strip. Shows one tab per post in
             the group when editing a multi-part thread. Switching tabs
             caches the current part's edits and loads the selected part. -->
        {#if composer.mode === 'edit' && groupPosts.length > 1}
          <div class="bg-surface border border-line rounded-xl p-2">
            <div class="text-xs text-muted mb-1.5 px-1">Thread parts ({groupPosts.length})</div>
            <div class="flex gap-1 flex-wrap">
              {#each groupPosts as part, i (part.id)}
                <button
                  onclick={() => switchGroupPart(part.id)}
                  class="px-2.5 py-1 text-xs rounded-lg transition-colors {part.id === editingGroupPostId ? 'bg-accent-fill text-accent-fg' : 'bg-surface-hover text-muted hover:text-content'}"
                  title={part.content ? (part.content.slice(0, 80) + (part.content.length > 80 ? '…' : '')) : `Part ${i + 1}`}
                >
                  {i === 0 ? 'Main post' : `Part ${i + 1}`}
                </button>
              {/each}
            </div>
            <p class="text-[10px] text-faint mt-1.5 px-1">Editing part {groupPosts.findIndex(p => p.id === editingGroupPostId) + 1} of {groupPosts.length}. Switching tabs preserves your unsaved edits per part.</p>
          </div>
        {/if}

        <!-- Title. F2: the label used to say "(optional)" unconditionally, which
             became a lie once the blocking banner taught us that YouTube,
             Pinterest and Reddit reject an untitled post. It now names the
             channels that need it instead. -->
        <div>
          <label for="composer-title" class="text-sm text-muted block mb-1">
            Title{#if titleRequiredChannels.length > 0}
              <span class="text-faint">- required for {titleRequiredChannels.join(', ')}</span>
            {/if}
          </label>
          <input id="composer-title" type="text" bind:value={title} placeholder="Post title..."
            class="w-full px-3 py-2 min-h-11 bg-background-input border border-line rounded-lg text-sm focus:border-accent outline-none" />
        </div>

        <!-- Channel Selection -->
        <div class="bg-surface border border-line rounded-xl p-4">
          <h3 class="text-sm font-semibold mb-3">Post to</h3>
          <ChannelSelector
            selected={selectedIntegrations}
            onToggle={(id) => {
              if (selectedIntegrations.includes(id)) {
                selectedIntegrations = selectedIntegrations.filter(i => i !== id);
              } else {
                selectedIntegrations = [...selectedIntegrations, id];
              }
            }}
          />
        </div>

        <!-- Posting Targets -->
        {#each selectedIntegrations as intId (intId)}
          <!--
            The panel appears when the channel HAS targets, OR when the
            provider takes a TYPED target.

            The map's presence means "we have asked", so the emptiness
            has to be checked here - most providers return an empty
            list, and a "Posting Targets for X" panel with nothing in
            it is noise.

            But a provider that takes a target (Reddit's subreddit,
            Pinterest's board, Discord's channel) ALWAYS needs a way to
            set one. When the list came back empty the panel was
            hidden, and the founder had no way to set the target at
            all - the server's check said "pick it in Posting Targets",
            pointing at a panel that was not on the screen.
          -->
          {@const targetList = integrationTargets.get(intId) || []}
          {@const provider = integrationProviders.get(intId) ?? ''}
          {@const manualSpec = MANUAL_TARGET[provider]}
          {#if targetList.length > 0 || manualSpec}
            <div class="bg-surface border border-line rounded-xl p-4">
              <h3 class="text-sm font-semibold mb-3">
                Posting Targets for {integrationNames.get(intId)}
                {#if targetsLoading.has(intId)}
                  <span class="ml-2 text-xs text-muted">Loading...</span>
                {/if}
              </h3>
              {#if targetList.length > 0}
                <TargetPicker
                  targets={targetList}
                  selectedTargets={selectedTargets.get(intId) || []}
                  onToggle={(targetId) => toggleTarget(intId, targetId)}
                />
              {/if}
              {#if manualSpec}
                <div class={targetList.length > 0 ? 'mt-3' : ''}>
                  <label class="block text-xs text-muted mb-1" for={`manual-target-${intId}`}>
                    {manualSpec.label}
                  </label>
                  <input
                    id={`manual-target-${intId}`}
                    type="text"
                    placeholder={manualSpec.placeholder}
                    value={manualTargets.get(intId) ?? ''}
                    oninput={(e) => {
                      const v = (e.currentTarget as HTMLInputElement).value;
                      const next = new Map(manualTargets);
                      if (v.trim()) next.set(intId, v);
                      else next.delete(intId);
                      manualTargets = next;
                    }}
                    class="w-full px-3 py-2 min-h-11 rounded-lg text-sm bg-background-input border border-line text-content placeholder:text-faint outline-none focus:border-accent transition-colors"
                  />
                  <p class="text-[11px] text-muted mt-1">
                    {targetList.length > 0
                      ? 'Or type a target the list does not show.'
                      : 'The provider did not list any - type the one you want.'}
                  </p>
                </div>
              {/if}
            </div>
          {/if}
        {/each}

        <!-- Provider-specific fields, one card per selected channel.

             A control in the composer is a PROMISE (AGENTS.md 9.28), so
             this renders exactly the settings keys the providers already
             read - the `PROVIDER_FIELDS` table, one card per channel, and
             nothing for a provider that reads none of them. It sits here,
             with the other per-channel settings forms, because the value
             is written into THAT channel's settings: a Reddit flair is not
             a Mastodon content warning, and one post can carry both. -->
        {#each selectedIntegrations as intId (intId)}
          {@const channelFields = PROVIDER_FIELDS[integrationProviders.get(intId) ?? '']}
          {#if channelFields}
            <div class="bg-surface border border-line rounded-xl p-4">
              <h3 class="text-sm font-semibold mb-3">
                {integrationNames.get(intId) || 'Channel'} settings
              </h3>
              <div class="space-y-2">
                {#each channelFields as field (field.key)}
                  {@const fieldId = `provider-field-${intId}-${field.key}`}
                  {@const fieldValue = providerFields[intId]?.[field.key]}
                  {#if field.kind === 'boolean'}
                    <label class="flex items-center gap-2 min-h-11 cursor-pointer select-none">
                      <input
                        id={fieldId}
                        type="checkbox"
                        checked={fieldValue === true}
                        onchange={(e) => setProviderField(intId, field.key, e.currentTarget.checked)}
                        class="w-4 h-4 rounded border-line bg-surface-hover text-accent focus:ring-accent focus:ring-offset-0"
                      />
                      <span class="text-sm text-content">{field.label}</span>
                    </label>
                  {:else}
                    <div>
                      <label class="block text-xs text-muted mb-1" for={fieldId}>{field.label}</label>
                      <input
                        id={fieldId}
                        type="text"
                        placeholder={field.placeholder ?? ''}
                        value={typeof fieldValue === 'string' ? fieldValue : ''}
                        oninput={(e) => setProviderField(intId, field.key, e.currentTarget.value)}
                        class="w-full px-3 py-2 min-h-11 rounded-lg text-sm bg-background-input border border-line text-content placeholder:text-faint outline-none focus:border-accent transition-colors"
                      />
                      {#if field.altKey}
                        <p class="text-[11px] text-muted mt-1">
                          Type <code class="text-content-secondary">id:&lt;id&gt;</code> for the flair id, or
                          <code class="text-content-secondary">text:&lt;text&gt;</code> for the flair text.
                        </p>
                      {/if}
                      {#if field.numeric && typeof fieldValue === 'string' && fieldValue.trim() && !Number.isInteger(Number(fieldValue.trim()))}
                        <p class="text-[11px] text-warning mt-1">
                          The media id has to be a number - WordPress cannot read anything else.
                        </p>
                      {/if}
                    </div>
                  {/if}
                {/each}
              </div>
            </div>
          {/if}
        {/each}

        <!-- ONE card where two stood. Tags and Campaign are both small
             metadata controls, and they used to be two separate bordered
             cards stacked above the body - which pushed the content editor,
             the thing the composer exists for, down past four cards. Two
             controls that each answer one line are one card.

             The plan (Phase 4) asked for Tags behind a "more" disclosure
             with the picker inside; that is more machinery than the problem
             and hides a control the founder opens the composer reaching
             for. A shared card keeps both a single tap away and costs one
             card of vertical space instead of two. -->
        <div class="bg-surface border border-line rounded-xl p-4 space-y-3">
          <h3 class="text-sm font-semibold">Details</h3>
          <div>
            <label class="block text-xs text-muted mb-1.5" for="composer-campaign">Campaign</label>
            {#if allCampaigns.length > 0}
              <select
                id="composer-campaign"
                value={selectedCampaignId || ''}
                onchange={(e) => selectedCampaignId = e.currentTarget.value || null}
                class="w-full px-3 py-2 min-h-11 bg-background-input border border-line rounded-lg text-sm focus:border-accent outline-none"
              >
                <option value="">No campaign</option>
                {#each allCampaigns as c (c.id)}
                  <option value={c.id}>{c.name}</option>
                {/each}
              </select>
            {:else}
              <p class="text-xs text-muted">No campaigns yet.</p>
            {/if}
          </div>
          <div>
            <span class="block text-xs text-muted mb-1.5">Tags</span>
            <TagPicker
              selected={selectedTagIds}
              onToggle={(id) => {
                if (selectedTagIds.includes(id)) {
                  selectedTagIds = selectedTagIds.filter(t => t !== id);
                } else {
                  selectedTagIds = [...selectedTagIds, id];
                }
              }}
            />
          </div>
        </div>

        <!-- Global vs Internal channel editor toggle (Phase 3: SelectCurrent) -->
        {#if selectedIntegrations.length > 1}
          <div class="bg-surface border border-line rounded-xl p-3">
            <SelectCurrent
              {selectedIntegrations}
              {integrationProviders}
              {integrationNames}
              current={editingMode === 'global' ? 'global' : editingMode.split(':')[1]}
              {divergedIntegrations}
              onCurrentChange={handleCurrentChange}
              onRemoveIntegration={handleRemoveOverride}
            />
          </div>
        {/if}

        <!-- Content Editor -->
        <div class="bg-surface border border-line rounded-xl p-4">
          <h3 class="text-sm font-semibold mb-3">
            {#if editingMode === 'global'}
              Content (shared across all channels)
            {:else}
              Content for {integrationNames.get(editingMode.split(':')[1]) || 'this channel'}
            {/if}
          </h3>
          {#if editingMode === 'global'}
            <RichTextEditor {content} onUpdate={(text) => content = text} integrationId={selectedIntegrations[0]} />
            {#if selectedIntegrations.length > 0}
              <div class="mt-3 pt-3 border-t border-line">
                <PerPlatformCharCount
                  {content}
                  {selectedIntegrations}
                  {integrationProviders}
                  {integrationNames}
                />
              </div>
            {/if}
          {:else}
            <RichTextEditor
              content={providerOverride.get(editingMode.split(':')[1]) || content}
              onUpdate={(text) => {
                const intId = editingMode.split(':')[1];
                providerOverride.set(intId, text);
                providerOverride = new Map(providerOverride);
              }}
              integrationId={editingMode.split(':')[1]}
            />
            <div class="mt-3 pt-3 border-t border-line">
              <PerPlatformCharCount
                content={providerOverride.get(editingMode.split(':')[1]) || content}
                selectedIntegrations={[editingMode.split(':')[1]]}
                {integrationProviders}
                {integrationNames}
              />
            </div>
          {/if}
        </div>

        <!-- AI Assistant -->
        {#if showAi}
          <AiAssistant {content} onInsert={insertAiText} />
        {/if}

        <!-- AI Hashtag Suggestions -->
        <AiHashtagSuggestions {content} onAddHashtag={addHashtag} />

        <!-- Media -->
        <div class="bg-surface border border-line rounded-xl p-4">
          <h3 class="text-sm font-semibold mb-3">Media</h3>
          <MediaUpload items={mediaItems}
            onAdd={(item) => mediaItems = [...mediaItems, item]}
            onRemove={(id) => mediaItems = mediaItems.filter(m => m.id !== id)}
            onReorder={(newItems) => mediaItems = newItems}
          />
        </div>

        <!-- Music (Instagram only) -->
        {#if hasInstagramIntegration}
          <div class="bg-surface border border-line rounded-xl p-4">
            <div class="flex items-center justify-between mb-2">
              <h3 class="text-sm font-semibold">🎵 Music</h3>
              <button
                onclick={() => showMusicPicker = true}
                class="text-xs px-3 py-1.5 bg-surface-hover hover:bg-line-hover rounded-lg text-muted hover:text-content transition-colors"
              >
                {selectedMusic ? 'Change' : 'Browse'}
              </button>
            </div>
            {#if selectedMusic}
              <div class="flex items-center gap-3 bg-surface-hover rounded-lg p-2">
                <div class="w-8 h-8 rounded bg-accent-fill/20 flex items-center justify-center text-accent text-sm">🎵</div>
                <div class="flex-1 min-w-0">
                  <p class="text-sm font-medium truncate">{selectedMusic.title}</p>
                  <p class="text-xs text-muted truncate">{selectedMusic.artist}</p>
                </div>
                <button onclick={() => selectedMusic = null} class="text-muted hover:text-error text-sm min-h-11">&times;</button>
              </div>
            {:else}
              <p class="text-xs text-muted">Add trending music from Instagram's library to your Reel.</p>
            {/if}
          </div>
        {/if}

        <!-- v26-4: Thread Builder (generic - works for any provider, not just X).
             Shows when at least 1 integration is selected. Per-row delay
             (delay_minutes) is passed to the backend createThread API.

             It is HIDDEN in edit mode. The builder's action is
             `createThread`, which always INSERTs new posts with a fresh
             group id - so opening an existing 3-part thread for editing
             and clicking "Post Thread (3)" created THREE NEW POSTS and
             left the original untouched: a duplicate thread published
             from an edit screen. The thread path also drops media, title,
             tags, first_comment and the per-channel overrides, because
             `CreateThreadRequest` has no such fields. -->
        {#if selectedIntegrations.length > 0 && composer.mode !== 'edit'}
          <ThreadBuilder
            {content}
            provider={primaryProvider}
            onCreateThread={async (parts, delayMinutes) => {
              if (submitting) return;
              if (selectedIntegrations.length === 0) { error = 'Please select at least one channel'; return; }
              submitting = true;
              error = null;
              try {
                const r = await postsApi.createThread({
                  content_parts: parts,
                  integration_ids: selectedIntegrations,
                  scheduled_at: scheduledAt || undefined,
                  delay_minutes: delayMinutes,
                });
                if (r.error) { error = r.error; return; }
                localStorage.removeItem(DRAFT_KEY);
                composer.close();
                toast('Thread created', 'success');
              } catch (e: unknown) {
                error = (e instanceof Error ? e.message : String(e)) || 'Failed to create thread';
              } finally {
                submitting = false;
              }
            }}
            {submitting}
          />
        {/if}

        <!-- First Comment (LinkedIn/Facebook only) -->
        <FirstComment
          {selectedIntegrations}
          {integrationProviders}
          firstComment={firstComment}
          onFirstCommentChange={(v: string) => firstComment = v}
        />
      </div>

      <!-- Right column: preview + schedule. F2 mobile behaviour: below lg it is
           a full-width stack AFTER the editor (same order, no absolute
           positioning), toggled from the footer, so a phone user scrolls
           editor → preview instead of a 400px column squeezed into 375px. -->
      <div class="lg:w-[420px] lg:shrink-0 lg:min-h-0 lg:border-l lg:border-line lg:bg-background/50 overflow-y-auto p-4 sm:p-5 space-y-4
        {showPreviewMobile ? 'block' : 'hidden lg:block'}">
        <!-- Mobile: close preview button -->
        <div class="lg:hidden flex justify-end">
          <button onclick={() => showPreviewMobile = false} class="text-muted hover:text-content text-sm min-h-11">Close preview</button>
        </div>
        <!-- F2: one frame per selected channel, each with the full post
             context (title, media, first comment, schedule, TikTok sound,
             Pinterest board) so the preview is frame-accurate rather than a
             text-only approximation.

             Phase 7: the preview gets `scheduleInView`, NOT the shared
             `scheduledAt`, so the time shown on Reddit's tab is Reddit's
             own time. It used to receive the shared value while the
             component was already told which channel it was rendering - so
             a founder on Reddit's tab with a custom 23:30 saw the shared
             09:00 and had no way to tell the preview described someone
             else's post. That is AGENTS.md 9.28's shape exactly: a value
             the UI collects and then shows the WRONG one for.
             `scheduleInView` already resolves this for the picker, and the
             preview must use the SAME derivation - two rules for one
             question is how they drift (9.18). -->
        <PlatformPreviewPane
          {content}
          overrides={providerOverride}
          current={editingMode === 'global' ? 'global' : editingMode.split(':')[1]}
          {selectedIntegrations}
          {integrationProviders}
          {integrationNames}
          media={mediaItems}
          {title}
          {firstComment}
          scheduleFor={(id: string) => channelSchedule.get(id) ?? scheduledAt}
          audioTitle={selectedMusic?.title || ''}
          {targetNames}
        />

        <!-- Schedule -->
        <div class="bg-surface border border-line rounded-xl p-4">
          <h3 class="text-sm font-semibold mb-3">
            {#if editingMode === 'global'}
              Schedule
            {:else}
              Schedule for {integrationNames.get(editingMode.split(':')[1]) || 'this channel'}
              {#if channelSchedule.has(editingMode.split(':')[1])}
                <span class="text-xs font-normal text-accent ml-1">custom</span>
              {/if}
            {/if}
          </h3>
          <!-- Phase 6: ONE picker, reading the schedule IN VIEW. When the
               founder switches to a channel's tab the picker shows THAT
               channel's time (or the shared one when the channel has not
               diverged), and changing it writes that channel's own time.
               Nothing here knows whether the channel diverged before -
               the `scheduleInView` derived resolves it.

               The resolve button still sends the FIRST integration, which
               is the shared behaviour and is unchanged. -->
          <SchedulePicker
            scheduledAt={scheduleInView}
            onChange={(iso: string | null) => setScheduleInView(iso)}
            {recurring}
            onRecurringChange={(r: { intervalDays: number; endDate: string } | null) => recurring = r}
            integrationId={editingMode !== 'global' ? editingMode.split(':')[1] : selectedIntegrations[0]}
          />
          {#if divergedSchedules.size > 0}
            <!-- The per-channel times, and how to undo one. A founder who
                 set 09:00 for X six minutes ago and cannot see WHY the
                 preview says 17:00 has to be able to find it. -->
            <div class="mt-3 pt-3 border-t border-line space-y-1">
              {#each selectedIntegrations as intId (intId)}
                {#if channelSchedule.has(intId)}
                  <div class="flex items-center justify-between gap-2 text-xs">
                    <span class="text-muted truncate">{integrationNames.get(intId) || 'channel'} — {formatDateTime(channelSchedule.get(intId)!)}</span>
                    <button
                      onclick={() => clearChannelSchedule(intId)}
                      class="text-accent hover:underline min-h-11 px-2 shrink-0"
                    >Use shared</button>
                  </div>
                {/if}
              {/each}
            </div>
          {/if}
        </div>

        <!-- Post Sets -->
        <button
          onclick={() => showPostSets = true}
          class="w-full px-3 py-2 text-sm text-muted hover:text-content border border-line rounded-lg transition-colors min-h-11"
        >Post Sets</button>
      </div>
    </div>

    <!-- Footer: actions. F2: on <lg the bar wraps to two rows so the primary
         CTA is never pushed off-screen by the secondary controls, and the two
         publish buttons are disabled with a reason while blockers stand. -->
    <div class="border-t border-line px-4 sm:px-5 py-3 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 shrink-0 bg-surface">
      <div class="flex items-center gap-2 shrink-0">
        <button
          onclick={() => showAi = !showAi}
          class="px-3 py-1.5 min-h-11 min-w-11 text-sm border border-line rounded-lg transition-colors {showAi ? 'bg-accent-soft text-accent border-accent/40' : 'text-muted hover:text-content'}"
        >AI</button>
        <!-- Preview toggle: on mobile the pane is a full-width stack below the
             editor, so this reveals it in place instead of side by side. -->
        <button
          onclick={() => showPreviewMobile = !showPreviewMobile}
          class="lg:hidden px-3 py-1.5 min-h-11 text-sm border border-line rounded-lg transition-colors text-muted hover:text-content"
          aria-expanded={showPreviewMobile}
        >{showPreviewMobile ? 'Hide preview' : 'Preview'}</button>
      </div>
      <div class="flex items-center gap-2 flex-wrap sm:justify-end">
        {#if composer.mode === 'edit'}
          <!-- Edit mode: no "save draft" (post already exists) -->
        {:else}
          <button
            onclick={saveAsDraft}
            disabled={submitting}
            class="px-3 py-1.5 min-h-11 text-sm text-muted hover:text-content border border-line rounded-lg disabled:opacity-50 transition-colors"
          >Save Draft</button>
        {/if}
        <!-- TWO filled buttons said NEITHER was primary: Post Now in green
             and Schedule in accent, side by side, both claiming the same
             weight. A footer with two primaries has no primary at all.

             Schedule became the one filled action. It is the reason the
             founder opens a scheduler, and "Post Now" is the expedient
             exception - so the exception is the bordered secondary and the
             routine is the filled one. Both stay visible: burying Post Now
             in a dropdown costs a click on the action someone reaches for
             when they want something out NOW, and the failure mode of a
             hidden control is not "a slightly busy footer" but "I could
             not find it".

             The ORDER is now secondary→primary, so the filled button is
             the last thing in the row and sits closest to where a
             right-to-left tap finishes. -->
        <button
          onclick={postNow}
          disabled={submitting || !canPublish}
          title={blockers.length > 0 ? blockers[0].message : ''}
          class="px-3 py-1.5 min-h-11 text-sm text-content border border-line rounded-lg hover:bg-surface-hover disabled:opacity-50 transition-colors font-medium"
        >{submitting ? 'Posting...' : 'Post Now'}</button>
        <button
          onclick={submit}
          disabled={submitting || !canPublish}
          title={blockers.length > 0 ? blockers[0].message : ''}
          class="px-3 py-1.5 min-h-11 bg-accent-fill hover:bg-accent-fill-hover text-accent-fg disabled:opacity-50 rounded-lg text-sm font-medium transition-colors"
        >{submitting ? 'Scheduling...' : 'Schedule'}</button>
      </div>
    </div>
  {/if}
</div>

{#if showMusicPicker}
  <MusicPicker integrationId={selectedIntegrations.find(id => integrationProviders.get(id) === 'instagram' || integrationProviders.get(id) === 'instagram-standalone') || ''} onSelect={(m) => { selectedMusic = m; showMusicPicker = false; }} onclose={() => showMusicPicker = false} />
{/if}

{#if showPostSets}
  <!--
    `currentChannelIds` was never passed, so a saved post set always
    stored `channel_ids: []` and loading it back restored the text and
    the schedule but not the channels. The prop existed; the call site
    just did not supply it.
  -->
  <PostSetModal
    open={true}
    onclose={() => showPostSets = false}
    currentContent={content}
    currentChannelIds={selectedIntegrations}
    onLoad={handlePostSetLoad}
  />
{/if}
