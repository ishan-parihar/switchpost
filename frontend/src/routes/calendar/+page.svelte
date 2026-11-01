<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { calendarApi } from "$lib/api/calendar";
  import { postsApi } from "$lib/api/posts";
  import { integrationsApi, type Integration } from "$lib/api/integrations";
  import { tagsApi, type Tag } from "$lib/api/tags";
  import { campaignsApi, type Campaign } from "$lib/api/campaigns";
  import { calendarState } from "$lib/stores/calendar.svelte";
  import { timezone } from "$lib/stores/timezone.svelte";
  import { wallClockToUtc } from "$lib/composer/wall-clock";
  import { toast } from "$lib/stores/toast";
  import { realtime } from "$lib/stores/realtime";
  import { confirmModal } from "$lib/stores/modals.svelte";
  import { modals } from "$lib/stores/modals.svelte";
  import { composer } from "$lib/stores/composer.svelte";
  import PostStatsModal from "$lib/calendar/PostStatsModal.svelte";
  import GeneratorModal from "$lib/composer/GeneratorModal.svelte";
  import { toCalendarEvent, type CalendarEvent, type CalendarView } from "$lib/calendar/types";
  import { formatDateKey } from "$lib/calendar/utils";
  import CalendarHeader from "$lib/calendar/CalendarHeader.svelte";
  import MonthView from "$lib/calendar/MonthView.svelte";
  import WeekView from "$lib/calendar/WeekView.svelte";
  import DayView from "$lib/calendar/DayView.svelte";
  import ListView from "$lib/calendar/ListView.svelte";
  import PostDetail from "$lib/calendar/PostDetail.svelte";
  import ChannelRail from "$lib/channels/ChannelRail.svelte";
  import { goto } from "$app/navigation";

  let events = $state<CalendarEvent[]>([]);
  let allTags = $state<Tag[]>([]);
  let allIntegrations = $state<Integration[]>([]);
  // Phase 5b: the rail manages ALL channels, including disabled ones - a
  // disabled channel is exactly the one a founder came to the rail to fix,
  // so hiding it from the management surface would hide the problem. The
  // publish views keep filtering on `allIntegrations`, because a disabled
  // channel cannot publish.
  let allIntegrationsRaw = $state<Integration[]>([]);
  let allCampaigns = $state<Campaign[]>([]);
  let selectedTagId = $state<string | null>(null);
  // v24-2: server-side calendar filters (channel + campaign). These are
  // passed to calendarApi.get() which sends them as query params to the
  // backend (v23-4). The backend filters in SQL, so the client doesn't
  // need to filter post-hoc.
  let selectedIntegrationId = $state<string | null>(null);
  let selectedCampaignId = $state<string | null>(null);
  let filteredEvents = $derived(
    selectedTagId
      ? events.filter(e => e.tags?.some(t => t.id === selectedTagId))
      : events
  );
  let selectedEvent = $state<CalendarEvent | null>(null);
  let duplicating = $state(false);
  let deleting = $state(false);
  let loading = $state(false);
  let refreshing = $state(false);
  let fetchError = $state<string | null>(null);

  // Bulk selection
  let selected = $state<Set<string>>(new Set());
  let bulkScheduleDate = $state("");
  let bulkScheduleTime = $state("09:00");
  let showBulkSchedule = $state(false);
  let bulkProcessing = $state(false);

  function toggleSelect(id: string, e: Event) {
    e.stopPropagation();
    const s = new Set(selected);
    if (s.has(id)) s.delete(id); else s.add(id);
    selected = s;
  }

  function toggleAll() {
    if (selected.size === events.length) selected = new Set();
    else selected = new Set(events.map(e => e.id));
  }

  async function bulkDelete() {
    // Phase v21: replace native confirm() with modals.areYouSure for
    // consistent UX (postiz-style confirmation dialog with i18n + keyboard).
    const ok = await modals.areYouSure({
      title: `Delete ${selected.size} post(s)?`,
      message: 'This will soft-delete the selected posts. They will be hidden from the calendar and posts list, but can be recovered from the Trash (coming in v22).',
      confirmLabel: 'Delete',
      cancelLabel: 'Cancel',
      danger: true,
    });
    if (!ok) return;
    bulkProcessing = true;
    for (const id of selected) { await postsApi.delete(id); }
    selected = new Set();
    bulkProcessing = false;
    refresh();
  }

  async function bulkReschedule() {
    if (!bulkScheduleDate) return;
    bulkProcessing = true;
    // The wall clock the user typed, read in the APP's selected
    // timezone - the same conversion a drag uses. `new Date(...)` parsed
    // it in the browser's timezone, so a bulk reschedule meant a
    // different instant than the same time entered in the composer.
    const iso = wallClockToUtc(bulkScheduleDate, bulkScheduleTime, timezone.value);
    // The response is CHECKED. The schedule endpoint refuses a post
    // that is not schedulable in its current state (a published one),
    // and the 404 was swallowed - so a bulk reschedule that included
    // a published post moved the others and silently left it alone.
    // The founder saw four posts move and one not, with no reason.
    //
    // The failures are collected and reported, and the posts that DID
    // move stay selected so the founder can see which ones did not.
    const failed: string[] = [];
    for (const id of selected) {
      const r = await postsApi.schedule(id, iso);
      if (r.error) failed.push(id);
    }
    if (failed.length > 0) {
      toast(
        `${failed.length} of ${selected.size} posts could not be rescheduled - a published post cannot be rescheduled`,
        'error'
      );
      // Keep only the failed ones selected, so the founder can see
      // exactly which did not move.
      selected = new Set(failed);
    } else {
      selected = new Set();
    }
    showBulkSchedule = false;
    bulkProcessing = false;
    refresh();
  }

  /**
   * The campaign colour for a post, looked up from the campaign list the
   * page already holds. The calendar chip is coloured by campaign, so the
   * colour has to travel with the event.
   */
  function campaignColorOf(id: string | null | undefined): string | null {
    if (!id) return null;
    return allCampaigns.find((c) => c.id === id)?.color ?? null;
  }

  /**
   * The founder's zone, as an IANA NAME.
   *
   * The server buckets a post into the day it belongs to, and the day
   * a post belongs to is a LOCAL day. The server resolves the name
   * per-instant (it now embeds the IANA database via chrono-tz), so a
   * window spanning a DST transition is bucketed correctly - the
   * client no longer resolves the offset at all. The old constant
   * offset sent ONE value per window and was wrong across a
   * transition.
   */

  async function fetchEvents(start: string, end: string) {
    loading = true;
    fetchError = null;
    // v24-2: pass server-side filters (integration_ids, campaign_id)
    // to the calendar API. The backend filters in SQL.
    const filters: { integration_ids?: string[]; campaign_id?: string } = {};
    if (selectedIntegrationId) filters.integration_ids = [selectedIntegrationId];
    if (selectedCampaignId) filters.campaign_id = selectedCampaignId;
    // The ZONE NAME, not a constant offset: the server resolves the
    // offset per-instant (DST-correct), so a window spanning a
    // transition no longer buckets the post-transition posts an hour
    // late. The client no longer resolves the offset at all.
    const r = await calendarApi.get(
      start,
      end,
      Object.keys(filters).length > 0 ? filters : undefined,
      timezone.value
    );
    if (r.data) {
      events = r.data.days.flatMap((d) =>
        d.posts.map((p) => toCalendarEvent(p, timezone.value, campaignColorOf(p.campaign_id))),
      );
    } else {
      // Phase v21: improved error UX - show a full-card error state with a
      // Retry button + emit a toast so the user notices even if they've
      // scrolled past the calendar. Previously this was a tiny red <div>
      // with no retry, easy to miss.
      fetchError = r.error || "Failed to load calendar events";
      toast(`Calendar error: ${fetchError}`, 'error');
    }
    loading = false;
  }

  /** Phase v21: retry handler for the calendar error state. */
  function retryFetch() {
    const range = calendarState.state.view === 'list'
      ? null
      : calendarState.state.view === 'month' ? getMonthRange()
      : calendarState.state.view === 'week' ? getWeekRange()
      : getDayRange();
    if (range) {
      fetchEvents(range.start, range.end);
    } else {
      fetchListEvents();
    }
  }

  // Phase 4: fetch posts for the list view using the posts API with
  // state filter + pagination. This is separate from fetchEvents because
  // the list view uses a different endpoint (posts, not calendar) and
  // supports server-side pagination + state filter.
  async function fetchListEvents() {
    loading = true;
    fetchError = null;
    const limit = 100;
    const offset = (calendarState.state.listPage - 1) * limit;
    const stateFilter = calendarState.state.listState === 'all'
      ? undefined
      : calendarState.state.listState === 'scheduled' ? 'queued' : calendarState.state.listState;
    // The channel and campaign filters apply to the list view too. The
    // list view used to fetch EVERY post and ignore both filters, so
    // the dropdown had no effect in one of the four views - the user
    // picked "Twitter", switched to List, and saw every channel.
    const r = await postsApi.list({
      limit,
      offset,
      ...(stateFilter && { state: stateFilter }),
      ...(selectedIntegrationId && { integration_ids: [selectedIntegrationId] }),
      ...(selectedCampaignId && { campaign_id: selectedCampaignId }),
    });
    if (r.data) {
      events = r.data.posts.map((p) =>
        toCalendarEvent(p, timezone.value, campaignColorOf(p.campaign_id)),
      );
      calendarState.setListTotalPages(Math.max(1, Math.ceil(r.data.total / limit)));
    } else {
      fetchError = r.error || "Failed to load posts";
      events = [];
    }
    loading = false;
  }

  // The range is sent as UTC INSTANTS, not as date-only strings.
  //
  // A date-only range is resolved by the server to UTC midnight and
  // UTC end-of-day, while the grid's cells are the APP's timezone
  // days. Two different definitions of "a day" - and the posts in the
  // gap fall off the calendar entirely.
  //
  // The concrete case: app timezone America/New_York, a post
  // scheduled Oct 31 20:00 EDT = Nov 1 00:00 UTC. October's month
  // view sends start=2026-10-01&end=2026-10-31, the server resolves
  // `end` to Oct 31 23:59:59 UTC, and the post (Nov 1 00:00 UTC) is
  // EXCLUDED. November's month view fetches it, but the grid has no
  // Oct 31 cell, so it is dropped on the floor. The post appeared on
  // NO month grid, in either month.
  //
  // So the range is computed with `wallClockToUtc` - the ONE
  // conversion the app has - and the server's exact-datetime path is
  // used. The grid and the filter then agree by construction.
  function getMonthRange() {
    const d = calendarState.state.currentDate;
    const first = formatDateKey(new Date(d.getFullYear(), d.getMonth(), 1));
    const last = formatDateKey(new Date(d.getFullYear(), d.getMonth() + 1, 0));
    return rangeAsInstants(first, last);
  }

  function getWeekRange() {
    const d = calendarState.state.currentDate;
    const start = new Date(d);
    start.setDate(start.getDate() - ((start.getDay() + 6) % 7)); // Monday
    const end = new Date(start);
    end.setDate(end.getDate() + 6);
    return rangeAsInstants(formatDateKey(start), formatDateKey(end));
  }

  /** Phase v21: day-view range helper (for retryFetch). */
  function getDayRange() {
    const d = calendarState.state.currentDate;
    const key = formatDateKey(d);
    return rangeAsInstants(key, key);
  }

  /**
   * A date range in the app's timezone, as UTC instants.
   *
   * `start` is the first instant of the start DAY in the app's zone,
   * and `end` is the last instant of the end DAY in the app's zone.
   */
  function rangeAsInstants(startDate: string, endDate: string) {
    const zone = timezone.value;
    return {
      start: wallClockToUtc(startDate, '00:00', zone),
      // The end is the LAST instant of the end day, so it is the
      // following day's midnight minus a second.
      //
      // The time carries seconds: `wallClockToUtc` keeps them, so
      // this is `23:59:59` and the instant is a valid RFC 3339
      // string. Before, the helper appended its own `:00` to a time
      // that already had seconds, and the server answered "Invalid
      // end date" for the whole range.
      end: wallClockToUtc(endDate, '23:59:59', zone),
    };
  }

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      const st = calendarState.state;
      if (st.view === "list") {
        // Phase 4: list view uses the posts API with state filter + pagination.
        await fetchListEvents();
      } else {
        const r = st.view === "week" ? getWeekRange() : getMonthRange();
        await fetchEvents(r.start, r.end);
      }
    } finally {
      refreshing = false;
    }
  }

  function handleViewChange(v: CalendarView) {
    calendarState.setView(v);
    refresh();
  }
  function handlePrev() { calendarState.goBackward(); refresh(); }
  function handleNext() { calendarState.goForward(); refresh(); }
  function handleToday() { calendarState.goToday(); refresh(); }

  async function handleDrop(eventId: string, newDate: string, newHour?: string) {
    const event = events.find(e => e.id === eventId);
    if (!event) return;

    // Phase v21: published-post safety modal (postiz-inspired).
    // When the user drags a published post, ask what they want:
    //   - "Reschedule the post" → action: 'schedule' → backend resets
    //     state to queued + clears release fields → scheduler re-publishes
    //     at the new time (creates a NEW post on the platform).
    //   - "Just update the post details" → action: 'update' → backend
    //     changes scheduled_at only, leaves state + release fields alone
    //     (archive-style re-date, no re-publish).
    let action: 'schedule' | 'update' | undefined;
    if (event.state === 'published') {
      const choice = await confirmModal({
        title: 'This post is already published',
        message: 'What do you want to do?',
        confirmLabel: 'Reschedule the post',
        cancelLabel: 'Just update the post details',
        danger: false,
      });
      // choice === true  → reschedule (re-publish at new time)
      // choice === false → just update (change scheduled_at without re-publishing)
      action = choice ? 'schedule' : 'update';
    }

    // Phase 1: hour-precision. If a newHour was passed (from WeekView),
    // use it; otherwise fall back to the event's existing time.
    const time = newHour || event.time || "09:00";
    // The wall clock the user dropped the post on, read in the APP's
    // selected timezone.
    //
    // `new Date(`${newDate}T${time}:00`)` parsed it in the BROWSER's
    // timezone, which is not the same thing: an app whose timezone is
    // America/New_York, opened on a machine set to UTC, dropped a post
    // on the 09:00 slot and the server received 09:00 UTC - 5am local.
    // The read path (`toCalendarEvent`) already used the app's zone, so
    // the post appeared in the WRONG slot until the next render.
    //
    // `wallClockToUtc` is the one conversion - it is the same helper the
    // composer's SchedulePicker uses, so a time entered by dragging and
    // a time entered by typing mean the same instant.
    const iso = wallClockToUtc(newDate, time, timezone.value);

    let moveGroup = false;
    if (event.groupId) {
      moveGroup = await confirmModal({
        title: 'Move campaign group?',
        message: 'Move all posts in this campaign by the same offset? Cancel to move only this post.',
        confirmLabel: 'Move all',
        cancelLabel: 'Just this post',
      });
    }

    const r = await postsApi.reschedule(eventId, iso, moveGroup, action);
    if (r.error) {
      toast("Failed to reschedule: " + r.error, "error");
    } else {
      const count = r.data?.count;
      if (moveGroup && count) {
        toast("Rescheduled " + count + " posts in group", "success");
      } else if (action === 'schedule') {
        toast("Post re-scheduled for re-publishing", "success");
      } else if (action === 'update') {
        toast("Post date updated (no re-publish)", "success");
      } else {
        toast("Post rescheduled", "success");
      }
      refresh();
    }
  }

  async function handleDuplicate(eventId: string) {
    if (duplicating) return;
    duplicating = true;
    try {
      const detail = await postsApi.get(eventId);
      if (detail.error || !detail.data) {
        toast(`Failed to fetch post: ${detail.error || 'unknown'}`, "error");
        return;
      }
      const post = detail.data;

      const slot = await postsApi.findSlot(post.integration_id);
      if (slot.error || !slot.data?.date) {
        toast(`Failed to find slot: ${slot.error || 'unknown'}`, "error");
        return;
      }

      // Preserve media, tags, and first_comment when duplicating -
      // the previous version only copied content + title + scheduled_at,
      // losing all the rich context the user had attached to the original.
      await postsApi.create({
        integration_ids: [post.integration_id],
        content: post.content,
        title: post.title,
        scheduled_at: slot.data.date,
        tag_ids: post.tags?.map(t => t.id) || [],
        first_comment: post.first_comment || undefined,
        media: (post.media || []).map(m => ({
          id: crypto.randomUUID(),
          url: m.url,
          mime_type: m.mime_type,
          alt: m.alt,
        })),
      });

      refresh();
    } catch (e) {
      toast(`Failed to duplicate post: ${e instanceof Error ? e.message : 'unknown'}`, "error");
    } finally {
      duplicating = false;
    }
  }

  function handleStats(eventId: string) {
    const post = events.find(e => e.id === eventId);
    modals.open(PostStatsModal, {
      postId: eventId,
      postTitle: post?.title || 'Post',
      integrationId: post?.integrationId,
      integrationName: post?.integrationName,
      onclose: () => {},
    }, {
      title: 'Post Statistics',
      size: 'max-w-2xl',
    });
  }

  async function handleDelete(eventId: string) {
    if (deleting) return;
    // Phase v21: replace native confirm() with modals.areYouSure for
    // consistent UX with the bulk-delete flow above.
    const ok = await modals.areYouSure({
      title: 'Delete this post?',
      message: 'The post will be soft-deleted. It will be hidden from the calendar and posts list, but can be recovered from the Trash (coming in v22).',
      confirmLabel: 'Delete',
      cancelLabel: 'Cancel',
      danger: true,
    });
    if (!ok) return;
    deleting = true;
    try {
      const r = await postsApi.delete(eventId);
      if (r.error) toast(`Failed to delete post: ${r.error}`, "error");
      else refresh();
    } finally {
      deleting = false;
    }
  }

  let calUnsubscribers: (() => void)[] = [];

  // Phase 4: re-fetch when list-view state filter or page changes.
  $effect(() => {
    // Track these reactive reads so the effect re-runs when they change.
    const _listState = calendarState.state.listState;
    const _listPage = calendarState.state.listPage;
    const _view = calendarState.state.view;
    // untrack: refresh() reads and writes `refreshing`; tracking that here
    // made the effect re-run on its own write, refetching in a loop.
    if (_view === 'list') {
      untrack(() => refresh());
    }
  });

  onMount(async () => {
    // v28: measure the chrome above the calendar card and feed the
    // shell's height (see the style block). Re-measured whenever the
    // header's own height CHANGES — a window resize alone misses the
    // filter row reflowing onto a second line (font swap, scrollbar
    // appearance, viewport emulation), so the header wrapper itself is
    // observed. ponytail: one getBoundingClientRect per observed change.
    const measureShell = () => {
      const shell = document.querySelector('.calendar-view-shell');
      if (shell) {
        const top = shell.getBoundingClientRect().top + window.scrollY;
        (shell as HTMLElement).style.setProperty('--cal-shell-top', `${Math.round(top)}px`);
      }
    };
    measureShell();
    window.addEventListener('resize', measureShell);
    let headerObserver: ResizeObserver | undefined;
    try {
      headerObserver = new ResizeObserver(measureShell);
      const shell = document.querySelector('.calendar-view-shell');
      // Observe every sibling BEFORE the shell — any of them growing
      // pushes the shell down. (The header rows wrap; the shell must
      // follow.)
      let sib = shell?.previousElementSibling;
      while (sib) {
        headerObserver.observe(sib as Element);
        sib = sib.previousElementSibling;
      }
    } catch { /* ResizeObserver unavailable: resize listener still covers window changes */ }
    onDestroy(() => {
      window.removeEventListener('resize', measureShell);
      headerObserver?.disconnect();
    });

    // Phase v21: read ?display= and ?date= from the URL on mount so the
    // calendar is deep-linkable. URL params win over localStorage (which
    // is the persisted default). This mirrors postiz-app's calendar URL sync.
    const url = new URL(window.location.href);
    const urlDisplay = url.searchParams.get('display');
    if (urlDisplay === 'month' || urlDisplay === 'week' || urlDisplay === 'day' || urlDisplay === 'list') {
      calendarState.setView(urlDisplay);
    }
    const urlDate = url.searchParams.get('date');
    if (urlDate) {
      const d = new Date(urlDate);
      if (!isNaN(d.getTime())) {
        calendarState.setDate(d);
      }
    }
    refresh();
    // Fetch tags for the filter dropdown
    const tagRes = await tagsApi.list();
    if (tagRes.data) allTags = tagRes.data;
    // Phase 2: fetch integrations for DayView ghost slots
    const integRes = await integrationsApi.list();
    if (integRes.data) {
      allIntegrationsRaw = integRes.data.integrations;
      allIntegrations = allIntegrationsRaw.filter(i => !i.disabled);
    }
    // v24-2: fetch campaigns for the campaign filter dropdown.
    const campRes = await campaignsApi.list();
    if (campRes.data) allCampaigns = campRes.data;
    const events = ['post_created', 'post_scheduled', 'post_published', 'post_failed', 'post_deleted', 'post_updated', 'post_stage_changed', 'lagged'];
    for (const evt of events) {
      calUnsubscribers.push(realtime.on(evt, () => refresh()));
    }
  });

  // Phase 5b: the rail's reconnect / connect / disconnect actions change the
  // channel set, and the rail must not re-fetch silently behind the calendar's
  // back - it hands control back so both lists change together.
  async function reloadIntegrations() {
    const r = await integrationsApi.list();
    if (r.data) {
      allIntegrationsRaw = r.data.integrations;
      allIntegrations = allIntegrationsRaw.filter(i => !i.disabled);
    }
    refresh();
  }

  // v24-2: re-fetch events when the channel or campaign filter changes.
  $effect(() => {
    selectedIntegrationId;
    selectedCampaignId;
    // Only re-fetch if we've already loaded once (avoid double-fetch on mount).
    if (allIntegrations.length > 0 || allCampaigns.length > 0) {
      untrack(() => refresh());
    }
  });

  // Phase v21: sync view + date to URL via replaceState (no history
  // pollution - back button doesn't get a stack entry per week nav).
  // Postiz-app uses the same pattern: window.history.replaceState.
  $effect(() => {
    // Read these so the effect re-runs when they change.
    const view = calendarState.state.view;
    const currentDate = calendarState.state.currentDate;
    if (typeof window === 'undefined') return;
    const url = new URL(window.location.href);
    url.searchParams.set('display', view);
    url.searchParams.set('date', formatDateKey(currentDate));
    // Native history call: SvelteKit's replaceState throws if this effect
    // runs before the app root mounts (`$set` of undefined on first load).
    // Keep the router's own history.state so back/forward still resolve.
    history.replaceState(history.state, '', url.pathname + url.search + url.hash);
  });

  onDestroy(() => {
    calUnsubscribers.forEach(fn => fn());
  });
</script>

<!-- `space-y-3`, not `space-y-6`. The 24px vertical gap was sized for a
     page with a title row, a header row and a filter bar between them; with
     one header row it was dead space between the header and the grid, and
     every pixel of it comes off the working hours the week grid shows. -->
<!-- Phase 5b: the channel rail sits BESIDE the calendar content, not above
     it, because the plan measured the collision (2.3). At 1440 the global rail is
     80px, so an always-open 260px channel rail would leave the grid 1100px -
     LESS than the 1104px it had before the whole exercise.

     The rail is NOT a second channel filter: the header's `All channels` select
     stays. Two controls, two jobs - the select is "show me these posts", the rail
     is "manage these accounts".

     MEASURED 2026-10-31 at 1440, and the plan's own arithmetic is off: the
     calendar's content box is capped by the layout's `max-w-6xl` at 1104px, so
     the "1296px available for the channel rail" figure was never reachable - the
     container is the binding constraint, not the rails. Collapsed, the rail
     costs 64px and there is NO page-level gap between it and the calendar (the
     rail carries its own border and inner padding), so the calendar measures
     1040px rather than the 1104px it had. That 64px is the honest price and the
     default; the 260px expanded state costs 260px and is opt-in and persisted.
     Widening `max-w-6xl` to make the plan's number true was rejected: it lives
     in +layout.svelte and would reshape every other route for one page's
     arithmetic. -->
<div class="page-enter flex">
  <ChannelRail
    integrations={allIntegrationsRaw}
    selected={selectedIntegrationId}
    onSelect={(id) => (selectedIntegrationId = id)}
    onChannelsChanged={reloadIntegrations}
    variant="calendar"
  />

  <div class="space-y-3 min-w-0 flex-1">
  <!-- C-2: this row was a standalone `h1` plus the two action buttons, and
       it existed only to carry those buttons. Its 44px, plus the `space-y-6`
       gap above and below it, was more than half of the measured 218px of
       chrome between the top bar and the grid - chrome that pushed the week
       grid's first working hour below the fold.

       Both actions moved INTO the header row, which is where postiz puts them
       (next to the view switch, on the line that carries everything else). The
       date range is the title now; a second `h1` reading "Content Calendar"
       said nothing the nav rail's active item and the date range do not.

       The nav rail's "Calendar" item remains the route's landmark, so the page
       still has one accessible name for assistive technology. -->

  <CalendarHeader
    view={calendarState.state.view}
    currentDate={calendarState.state.currentDate}
    onPrev={handlePrev}
    onNext={handleNext}
    onToday={handleToday}
    onViewChange={handleViewChange}
    tags={allTags}
    {selectedTagId}
    onTagFilter={(id) => selectedTagId = id}
    integrations={allIntegrations}
    {selectedIntegrationId}
    onIntegrationFilter={(id) => selectedIntegrationId = id}
    campaigns={allCampaigns}
    {selectedCampaignId}
    onCampaignFilter={(id) => selectedCampaignId = id}
    onNewPost={() => composer.openCreate()}
    onGenerate={() => modals.open(GeneratorModal, {}, { title: 'AI Post Generator', size: 'max-w-lg' })}
  />

  {#if selected.size > 0}
    <div class="flex items-center gap-3 bg-accent-fill/10 border border-accent/30 rounded-lg px-4 py-2 flex-wrap">
      <span class="text-sm text-accent-strong">{selected.size} selected</span>
      <button onclick={() => showBulkSchedule = !showBulkSchedule} disabled={bulkProcessing} class="px-3 py-1 text-xs bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded disabled:opacity-50 min-h-11">Reschedule</button>
      <button onclick={bulkDelete} disabled={bulkProcessing} class="px-3 py-1 text-xs bg-error hover:bg-error/90 text-error-fg rounded disabled:opacity-50 min-h-11">Delete</button>
      <button onclick={() => selected = new Set()} class="sm:ml-auto text-xs text-muted hover:text-content min-h-11">Clear</button>
    </div>
    {#if showBulkSchedule}
      <div class="flex items-center gap-2 bg-background-input border border-line rounded-lg p-3 flex-wrap">
        <input type="date" aria-label="Bulk reschedule date" bind:value={bulkScheduleDate} class="min-h-11 px-2 py-1 bg-surface border border-line rounded text-sm text-content-secondary" />
        <input type="time" aria-label="Bulk reschedule time" bind:value={bulkScheduleTime} class="min-h-11 px-2 py-1 bg-surface border border-line rounded text-sm text-content-secondary" />
        <button onclick={bulkReschedule} disabled={bulkProcessing || !bulkScheduleDate} class="px-3 py-1 bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded text-xs disabled:opacity-50 min-h-11">Apply</button>
      </div>
    {/if}
  {/if}

  <!-- v28: the view card takes the REST of the viewport, so the week
       and day hour grids scroll INSIDE the card (postiz's shape) and the
       month grid scrolls as a whole with its weekday header pinned.

       The chrome above is ONE row now (C-2): it measured 218px at 1440
       before this phase - a 44px `h1`+actions row, a 46px CalendarHeader and
       two 24px `space-y-6` gaps, on top of the sticky top bar. It is one
       40px row plus its gutter now. The number that matters is not in this
       comment: `.calendar-view-shell` below takes its height from
       `--cal-shell-top`, which the page MEASURES from the element's own
       `getBoundingClientRect().top` - so a comment that says "~168px" and a
       CSS fallback that says "218px" can both be stale without breaking
       anything, which is exactly how they drifted. -->
  <div class="calendar-view-shell">
  {#if fetchError}
    <!-- Phase v21: postiz-style full-card error state with Retry button.
         Previously this was a tiny red <div class="text-center py-4 text-sm text-error">
         that was easy to miss and offered no recovery path. -->
    <div class="flex flex-col items-center justify-center py-16 px-4 text-center">
      <div class="w-12 h-12 rounded-full bg-error/10 border border-error/30 flex items-center justify-center mb-3">
        <svg class="w-6 h-6 text-error" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      </div>
      <p class="text-sm font-medium text-content mb-1">Couldn't load calendar</p>
      <p class="text-xs text-muted mb-4 max-w-md">{fetchError}</p>
      <button
        onclick={retryFetch}
        class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded-lg transition-colors flex items-center gap-2"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
        Retry
      </button>
    </div>
  {:else if loading}
    <div class="grid grid-cols-7 gap-px">
      {#each Array(35) as _, i (i)}
        <Skeleton variant="card" rows={2} />
      {/each}
    </div>
  {:else if calendarState.state.view === "month"}
    <MonthView
      year={calendarState.state.currentDate.getFullYear()}
      month={calendarState.state.currentDate.getMonth()}
      events={filteredEvents}
      {selected}
      onEventClick={(id) => selectedEvent = events.find(e => e.id === id) || null}
      onDateClick={(date, hour) => composer.openCreate(date, undefined, undefined, undefined, hour)}
      onDrop={handleDrop}
      onDuplicate={handleDuplicate}
      onStats={handleStats}
      onDelete={handleDelete}
      onToggleSelect={toggleSelect}
    />
  {:else if calendarState.state.view === "week"}
    <WeekView
      referenceDate={calendarState.state.currentDate}
      events={filteredEvents}
      {selected}
      onEventClick={(id) => selectedEvent = events.find(e => e.id === id) || null}
      onDateClick={(date, hour) => composer.openCreate(date, undefined, undefined, undefined, hour)}
      onDrop={handleDrop}
      onDuplicate={handleDuplicate}
      onStats={handleStats}
      onDelete={handleDelete}
      onToggleSelect={toggleSelect}
    />
  {:else if calendarState.state.view === "day"}
    <DayView
      date={calendarState.state.currentDate}
      events={filteredEvents}
      {selected}
      integrations={allIntegrations}
      integrationFilter={selectedIntegrationId}
      onEventClick={(id) => selectedEvent = events.find(e => e.id === id) || null}
      onDateClick={(date, hour) => composer.openCreate(date, undefined, undefined, undefined, hour)}
      onDrop={handleDrop}
      onDuplicate={handleDuplicate}
      onStats={handleStats}
      onDelete={handleDelete}
      onToggleSelect={toggleSelect}
    />
  {:else if calendarState.state.view === "list"}
    <ListView
      events={filteredEvents}
      {selected}
      onEventClick={(id) => selectedEvent = events.find(e => e.id === id) || null}
      onDuplicate={handleDuplicate}
      onStats={handleStats}
      onDelete={handleDelete}
      onToggleSelect={toggleSelect}
      showActions={true}
    />
  {/if}
  </div>

  <PostDetail event={selectedEvent} onclose={() => selectedEvent = null} onDuplicate={handleDuplicate} {duplicating} />
  </div>
</div>

<style>
  /* The height the calendar card gets: from the card's own top edge
     to the bottom of the viewport, MEASURED, not a magic rems constant.

     The chrome above WRAPS - the filter row (channel + campaign + tag
     selects + the view switch + the two actions) reflows onto a second line
     at 768 and a third at 390 - so a fixed `calc(100dvh - Xrem)` would
     overshoot the fold by the wrapped rows' height and the page would scroll
     instead of the grid. `100dvh` tracks mobile URL bars.

     The shell is sized by the element's own position: the page sets
     `--cal-shell-top` from `getBoundingClientRect().top + scrollY` on mount,
     on resize, and through a ResizeObserver on every sibling BEFORE the
     shell (any of them growing pushes the shell down). So the ONE number
     that is authoritative is measured at runtime.

     The 218px FALLBACK below is pre-JS-paint only, and it is deliberately
     the WIDE layout's upper bound rather than a measured value - a fallback
     that guesses low clips the grid in the first frame. It used to disagree
     with the comment above it ("~168px"), which is the shape a constant
     takes when nobody measures it. If you change the header, re-measure with
     `frontend/scripts/measure-calendar-chrome.mjs` and update the text, not
     the fallback - the fallback's job is to be safe before JS runs. */
  .calendar-view-shell {
    height: calc(100dvh - var(--cal-shell-top, 218px));
    min-height: 420px;
    display: flex;
    flex-direction: column;
  }
  .calendar-view-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  @media (max-width: 639px) {
    .calendar-view-shell {
      height: auto;
      min-height: 0;
    }
    .calendar-view-shell > :global(*) {
      flex: none;
    }
  }
</style>
