<script lang="ts">
  import { toast } from "$lib/stores/toast";
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import ErrorState from '$lib/ui/ErrorState.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import { onMount, onDestroy } from "svelte";
  import { realtime } from "$lib/stores/realtime";
  import { dmsApi, type Conversation, type DmMessage } from "$lib/api/dms";
  import { integrationsApi, type Integration } from "$lib/api/integrations";

  let conversations = $state<Conversation[]>([]);
  let integrations = $state<Integration[]>([]);
  let selectedIntegrationId = $state<string | null>(null);
  let selectedId = $state<string | null>(null);
  let messages = $state<DmMessage[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let newMessage = $state("");
  let sending = $state(false);

  let selected = $derived(conversations.find(c => c.id === selectedId));

  async function loadIntegrations() {
    const r = await integrationsApi.list();
    if (r.data) {
      integrations = r.data.integrations.filter(i => !i.disabled);
      if (integrations.length > 0 && !selectedIntegrationId) {
        // Provider capability comes from the SERVER, not from a list here.
        //
        // This page used to carry `DM_CAPABLE = ['x','instagram','linkedin']`
        // and pick the first match as the default channel. On an instance
        // whose only X channel is COOKIE-auth that shipped a 400 on a page
        // load, because X's DM API is bearer-only and a cookie channel has
        // no bearer. MEASURED 2026-11-01: /dms opened with the X channel
        // selected and the server answering
        //   "X direct messages need an OAuth-connected channel. Cookie-auth
        //    channels (auth_token + ct0) cannot use X's DM API."
        //
        // The first attempt at a fix guessed again, from `auth_method`, and
        // the same page still OFFERED X in the same selector - a second
        // implementation of a rule the server owns, which is the shape
        // 9.20 is about (the duplicated char_limit table that drifted).
        // `/api/providers` now publishes `dm_supported`, read from the
        // provider implementation, so there is ONE rule.
        //
        // TWO levels, and the page needs both:
        //   dm_supported  - does the PROVIDER have a DM surface (server),
        //   auth_method   - can THIS channel reach one (a COOKIE X cannot).
        // The provider default below uses both. The channel is still
        // offered in the selector if the founder picks it - but no accident
        // puts it there, and the API remains the authority if they do.
        const provRes = await integrationsApi.listProviders();
        const dmProviders = new Set(
          (provRes.data ?? []).filter(p => p.dm_supported).map(p => p.identifier)
        );
        const dmCapable = integrations.filter(
          i => dmProviders.has(i.provider_identifier) &&
            (i.auth_method || 'oauth') === 'oauth'
        );
        if (dmCapable.length > 0) {
          selectedIntegrationId = dmCapable[0].id;
        } else {
          selectedIntegrationId = integrations[0].id;
        }
      }
    }
  }

  async function load() {
    if (!selectedIntegrationId) {
      loading = false;
      return;
    }
    loading = true;
    error = null;
    const r = await dmsApi.listConversations(selectedIntegrationId!);
    if (r.data) {
      conversations = r.data.conversations;
    } else {
      // The message names the CHANNEL and says what to do.
      //
      // The raw provider error was shown verbatim, and for X it is a
      // POST-LOOKUP message - "X post or user not found, it may be
      // deleted or protected" - shown for a CONVERSATION-LIST failure.
      // The founder's first impression of the DMs page was a message
      // about a deleted post, and a Retry button that looped the same
      // failure forever.
      //
      // The provider's own text is kept - it is the detail - but it is
      // no longer the whole message, and the thing the founder can
      // actually DO is named.
      const channel = integrations.find((i) => i.id === selectedIntegrationId);
      const name = channel?.profile_name || channel?.provider_name || 'this channel';
      error = `Could not reach ${name} for direct messages. Check the connection, or pick another channel.\n\n${r.error || ''}`;
    }
    loading = false;
  }

  async function loadMessages(convId: string) {
    if (!selectedIntegrationId) return;
    const r = await dmsApi.getMessages(convId, selectedIntegrationId);
    if (r.data) {
      messages = r.data.messages;
    }
  }

  async function sendMessage() {
    if (!selectedId || !newMessage.trim() || !selectedIntegrationId || !selected) return;
    sending = true;
    const recipient = selected.participant_name || selected.participant || "";
    const r = await dmsApi.send(selectedIntegrationId!, recipient, newMessage);
    if (r.error) {
      toast("Error: " + r.error, "error");
    } else {
      newMessage = "";
      await loadMessages(selectedId);
    }
    sending = false;
  }

  function selectConversation(id: string) {
    selectedId = id;
    messages = [];
    loadMessages(id);
  }

  function platformIcon(p: string): string {
    const icons: Record<string, string> = { x: "X", reddit: "R", linkedin: "in", facebook: "f", instagram: "IG", telegram: "TG", whatsapp: "WA", discord: "DC" };
    return icons[p] || "•";
  }

  let dmsUnsubscribers: (() => void)[] = [];

  onMount(async () => {
    await loadIntegrations();
    load();
    dmsUnsubscribers.push(realtime.on('integration_connected', () => load()));
    dmsUnsubscribers.push(realtime.on('integration_disconnected', () => load()));
    dmsUnsubscribers.push(realtime.on('dm_received', () => {
      if (selectedId) loadMessages(selectedId);
      load();
    }));
    // Server-side "you missed events" signal - same refetch as dm_received.
    dmsUnsubscribers.push(realtime.on('lagged', () => {
      if (selectedId) loadMessages(selectedId);
      load();
    }));
  });

  onDestroy(() => {
    dmsUnsubscribers.forEach(fn => fn());
  });
</script>

<div class="page-enter space-y-6">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <h1 class="text-xl font-semibold">Direct Messages</h1>
    <div class="flex gap-2 items-center">
      {#if integrations.length > 0}
        <select
          bind:value={selectedIntegrationId}
          onchange={load}
          class="min-h-11 px-3 py-1.5 text-sm bg-surface border border-line rounded-lg text-content"
        >
          {#each integrations as int (int.id)}
            <!--
              The PROFILE name, not the internal id.

              The dropdown showed `x (18239447)` - the provider's
              internal account id, which is a number the founder has
              never seen and cannot act on. The channels page already
              labels a connection with `profile_name || provider_name`,
              and the DM list is the same list of accounts: the
              founder connected three X accounts and has to tell them
              apart.

              The internal id stays as the fallback for an account
              whose profile name was never captured.
            -->
            <option value={int.id}>
              {int.profile_name || int.provider_name}{int.profile_name ? ` (${int.provider_identifier})` : ''}
            </option>
          {/each}
        </select>
      {/if}
      <button onclick={load} class="px-3 py-1.5 text-sm text-muted hover:text-content border border-line rounded-lg transition-colors min-h-11">Refresh</button>
    </div>
  </div>

  {#if error}
    <ErrorState message={error} actionLabel="Retry" onaction={load} />
  {:else if loading}
    <Skeleton variant="row" rows={5} />
  {:else if conversations.length === 0}
    <EmptyState
      icon="dm"
      title="No conversations found"
      description="DMs need a DM-capable provider (X, Instagram, or LinkedIn) on an API tier that allows messaging. Pick a different channel above to try another provider."
    />
  {:else}
    <div class="flex gap-4 h-[calc(100vh-200px)]">
      <!-- Conversation list -->
      <div class="w-80 bg-surface border border-line rounded-xl overflow-hidden flex flex-col shrink-0">
        <div class="p-3 border-b border-line">
          <span class="text-xs text-muted">{conversations.length} conversations</span>
        </div>
        <div class="flex-1 overflow-y-auto">
          {#each conversations as conv (conv.id)}
            <button
              onclick={() => selectConversation(conv.id)}
              class="w-full px-3 py-3 border-b border-line hover:bg-surface-hover transition-colors text-left {selectedId === conv.id ? 'bg-surface-hover' : ''}"
            >
              <div class="flex items-center gap-2 mb-1">
                <span class="text-xs text-accent">{platformIcon(conv.platform)}</span>
                <span class="text-sm font-medium truncate">{conv.participant_name || conv.participant}</span>
                {#if conv.unread_count > 0}
                  <span class="ml-auto px-1.5 py-0.5 text-[10px] bg-accent-fill text-accent-fg rounded-full">{conv.unread_count}</span>
                {/if}
              </div>
              <div class="flex items-center gap-2">
                <span class="text-xs text-muted truncate flex-1">{conv.last_message || '(no messages)'}</span>
                {#if conv.last_message_at}
                  <span class="text-[10px] text-muted shrink-0">{new Date(conv.last_message_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
                {/if}
              </div>
            </button>
          {/each}
        </div>
      </div>

      <!-- Message thread -->
      <div class="flex-1 bg-surface border border-line rounded-xl flex flex-col">
        {#if selected}
          <div class="px-4 py-3 border-b border-line">
            <div class="flex items-center gap-2">
              <span class="text-xs text-accent">{platformIcon(selected.platform)}</span>
              <span class="text-sm font-medium">{selected.participant_name || selected.participant}</span>
            </div>
          </div>

          <div class="flex-1 overflow-y-auto p-4 space-y-3">
            {#each messages as msg (msg.id)}
              <div class="flex {msg.read ? 'justify-end' : 'justify-start'}">
                <div class="page-enter max-w-[70%] {msg.read ? 'bg-accent-fill/30' : 'bg-line'} rounded-xl px-3 py-2">
                  <p class="text-sm">{msg.content}</p>
                  <span class="text-[10px] text-muted">{new Date(msg.created_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
                </div>
              </div>
            {/each}
          </div>

          <div class="p-3 border-t border-line">
            <div class="flex gap-2">
              <input
                type="text"
                bind:value={newMessage}
                placeholder="Type a message..."
                class="min-h-11 flex-1 px-3 py-2 bg-background-input border border-line rounded-lg text-sm"
                onkeydown={(e) => e.key === "Enter" && sendMessage()}
              />
              <button
                onclick={sendMessage}
                disabled={sending || !newMessage.trim()}
                class="px-4 py-2 bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded-lg text-sm disabled:opacity-50 transition-colors"
              >
                {sending ? "..." : "Send"}
              </button>
            </div>
          </div>
        {:else}
          <div class="flex-1 flex items-center justify-center text-sm text-muted">
            Select a conversation
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>
