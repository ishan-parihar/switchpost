<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { focusTrap } from '$lib/ui/focus-trap';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import ErrorState from '$lib/ui/ErrorState.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import { commentsApi, type Comment } from "$lib/api/comments";
  import { integrationsApi, type Integration } from "$lib/api/integrations";
  import { providerIcon } from "$lib/providers";
  import { toast } from "$lib/stores/toast";
  import { realtime } from "$lib/stores/realtime";

  let comments = $state<Comment[]>([]);
  let connectedIntegrations = $state<Integration[]>([]);
  let filterPlatform = $state("all");
  let filterStatus = $state("all");
  let loading = $state(true);
  let error = $state<string | null>(null);
  let replyModal = $state<{ comment: Comment; text: string } | null>(null);
  let sending = $state(false);

  const statuses = ["all", "new", "resolved"];

  // Build platform list from connected integrations
  let platforms = $derived.by(() => {
    const connected = connectedIntegrations
      .filter(i => !i.disabled)
      .map(i => i.provider_identifier);
    const unique = [...new Set(connected)];
    return ["all", ...unique];
  });

  async function load() {
    loading = true;
    error = null;
    const r = await commentsApi.list({
      ...(filterPlatform !== "all" && { platform: filterPlatform }),
      ...(filterStatus !== "all" && { status: filterStatus }),
    });
    if (r.data) {
      comments = r.data.comments;
    } else {
      error = r.error || "Failed to load comments";
    }
    loading = false;
  }

  async function resolveComment(id: string) {
    const r = await commentsApi.resolve(id);
    if (r.error) {
      toast("Failed to resolve: " + r.error, "error");
    } else {
      toast("Comment resolved", "success");
      load();
    }
  }

  async function sendReply() {
    if (!replyModal || !replyModal.text.trim()) return;
    sending = true;
    const r = await commentsApi.reply(replyModal.comment.id, replyModal.text);
    if (r.error) {
      toast("Reply failed: " + r.error, "error");
    } else {
      toast("Reply sent", "success");
      replyModal = null;
    }
    sending = false;
  }

  // Same rule as the DM page: the curated table is the ONE source of
  // provider glyphs (AGENTS.md 9.18). This page had a hand-maintained copy of
  // 5 entries that also fell back to the first two letters of the id, so a
  // google account rendered "GO" and the collision the founder reported
  // appeared here too.
  function platformIcon(p: string): string {
    return providerIcon(p);
  }

  let commentsUnsubscribers: (() => void)[] = [];

  onMount(async () => {
    const integRes = await integrationsApi.list();
    if (integRes.data) connectedIntegrations = integRes.data.integrations;
    load();
    // Refresh when a new comment arrives (realtime SSE event) - this is the
    // event the backend actually broadcasts. The previous subscription to
    // `post_published`/`post_created` never fired for comment activity.
    for (const evt of ['comment_received', 'post_published', 'lagged']) {
      commentsUnsubscribers.push(realtime.on(evt, () => load()));
    }
  });

  onDestroy(() => {
    commentsUnsubscribers.forEach(fn => fn());
  });
</script>

<div class="page-enter space-y-6">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <h1 class="text-xl font-semibold">Comments</h1>
    <button onclick={load} class="px-3 py-1.5 text-sm text-muted hover:text-content border border-line rounded-lg transition-colors min-h-11">Refresh</button>
  </div>

  <!-- Filters -->
  <div class="flex items-center gap-4 flex-wrap">
    <!-- Platform filter -->
    <div class="flex gap-2 flex-wrap">
      <span class="text-xs text-muted self-center">Platform:</span>
      {#each platforms as p}
        <button
          onclick={() => { filterPlatform = p; load(); }}
          class="px-3 py-1.5 text-xs capitalize rounded-lg transition-colors {filterPlatform === p ? 'bg-accent-fill text-accent-fg' : 'bg-surface text-muted hover:text-content border border-line'} min-h-11 min-w-11 inline-flex items-center justify-center"
        >{p}</button>
      {/each}
    </div>
    <!-- Phase 4: Status filter (was declared but never rendered) -->
    <div class="flex gap-2 flex-wrap">
      <span class="text-xs text-muted self-center">Status:</span>
      {#each statuses as s}
        <button
          onclick={() => { filterStatus = s; load(); }}
          class="px-3 py-1.5 text-xs capitalize rounded-lg transition-colors {filterStatus === s ? 'bg-accent-fill text-accent-fg' : 'bg-surface text-muted hover:text-content border border-line'} min-h-11 min-w-11 inline-flex items-center justify-center"
        >{s}</button>
      {/each}
    </div>
  </div>

  <!-- Content -->
  {#if error}
    <ErrorState message={error} actionLabel="Retry" onaction={load} />
  {:else if loading}
    <Skeleton variant="row" rows={4} />
  {:else if comments.length === 0}
    <EmptyState
      icon="comment"
      title="No comments found"
      description="Comments come from connected platforms that expose a comments API - Instagram, Facebook, LinkedIn."
    />
  {:else}
    <div class="bg-surface border border-line rounded-xl overflow-hidden">
      <div class="grid grid-cols-[40px_1fr_1.5fr_100px_100px_90px] gap-3 px-4 py-2 border-b border-line bg-background-input text-xs text-muted">
        <span></span><span>Post</span><span>Comment</span><span>Author</span><span>Date</span><span>Status</span>
      </div>
      {#each comments as c (c.id)}
        <div class="grid grid-cols-[40px_1fr_1.5fr_100px_100px_90px] gap-3 px-4 py-3 border-b border-line last:border-0 hover:bg-surface-hover transition-colors items-center">
          <span class="text-sm text-accent">{platformIcon(c.platform)}</span>
          <span class="text-sm truncate text-muted" title={c.post_content}>{c.post_content?.slice(0, 50) || c.post_id}</span>
          <span class="text-sm text-content-secondary truncate">{c.content}</span>
          <span class="text-xs text-muted truncate">{c.author || 'Unknown'}</span>
          <span class="text-xs text-muted">{new Date(c.created_at).toLocaleDateString()}</span>
          <div class="flex items-center gap-2">
            {#if c.status !== 'resolved'}
              <span class="px-2 py-0.5 text-xs rounded bg-warning/20 ring-1 ring-inset ring-warning/40 text-content">New</span>
              <button onclick={() => resolveComment(c.id)} class="text-xs text-muted hover:text-success min-h-11 min-w-11 inline-flex items-center justify-center" title="Resolve">✓</button>
              <button onclick={() => replyModal = { comment: c, text: "" }} class="text-xs text-muted hover:text-accent min-h-11 min-w-11 inline-flex items-center justify-center" title="Reply">↩</button>
            {:else}
              <span class="px-2 py-0.5 text-xs rounded bg-success/20 ring-1 ring-inset ring-success/40 text-content">Resolved</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- Reply Modal -->
{#if replyModal}
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
      onclick={() => { replyModal = null }}
    ></button>
<div class="relative bg-background-input border border-line rounded-xl p-6 w-full max-w-md"
      role="dialog"
      aria-modal="true"
      aria-labelledby="comment-reply-title"
      tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); replyModal = null; } }}
      use:focusTrap
    >
      <h2 id="comment-reply-title" class="text-lg font-semibold mb-2">Reply to {replyModal.comment.author || 'Unknown'}</h2>
      <p class="text-sm text-muted mb-4 truncate">{replyModal.comment.content}</p>
      <textarea
        bind:value={replyModal.text}
        placeholder="Write your reply..."
        rows="4"
        class="min-h-11 w-full px-3 py-2 bg-surface-hover border border-line rounded text-sm mb-4"
      ></textarea>
      <div class="flex gap-3 justify-end">
        <button onclick={() => replyModal = null} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
        <button onclick={sendReply} disabled={sending || !replyModal.text.trim()} class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded disabled:opacity-50 min-h-11">
          {sending ? "Sending..." : "Reply"}
        </button>
      </div>
    </div>
  </div>
{/if}
