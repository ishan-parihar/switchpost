<script lang="ts">
  import { focusTrap } from '$lib/ui/focus-trap';
  import { integrationsApi, type Integration } from "$lib/api/integrations";
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import ErrorState from '$lib/ui/ErrorState.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Icon from "$lib/ui/Icon.svelte";
  import { onMount, onDestroy } from "svelte";
  import { toast } from "$lib/stores/toast";
  import { realtime } from "$lib/stores/realtime";
  import { groupIntegrations, groupHeading } from "$lib/channels/group-integrations";
  import ChannelCard from "$lib/channels/ChannelCard.svelte";
  import ProviderIcon from "$lib/channels/ProviderIcon.svelte";
  import ConnectFlow from "$lib/channels/ConnectFlow.svelte";
  import TelegramCredentials from "$lib/channels/TelegramCredentials.svelte";
  import { channelStatus } from "$lib/channels/channel-status";

  let integrations = $state<Integration[]>([]);
  let loading = $state(true);
  let error = $state("");
  let connectProvider = $state<string | null>(null);
  let refreshing = $state<string | null>(null);

  // WhatsApp QR pairing and Telegram SMS sign-in are wizards, not credential
  // forms, so they stay on the page rather than in ConnectFlow. The unified
  // flow covers the five connect METHODS (oauth / cookie / api_key / pat /
  // web3), not device-pairing steps.
  let onboardDialog = $state<{
    provider: string;
    step: "phone" | "pair_code" | "polling" | "sms_code" | "bot_code" | "done";
    phone?: string;
    pairCode?: string;
    code?: string;
    instructions?: string;
  } | null>(null);
  let connecting = $state<string | null>(null);

  // ─── THE TELEGRAM CREDENTIALS GATE ──────────────────────────────
  // The Telegram USER channel needs MTProto credentials (an API id and
  // hash from my.telegram.org) before it can send a login code. Without
  // them the phone-number step is a dead end: the founder types a number
  // and the server answers "Telegram is not configured on this server …
  // set TELEGRAM_API_ID and TELEGRAM_API_HASH in ~/.switchpost/.env and
  // restart" — a file on a server, and a restart, neither of which the
  // founder can do from a browser.
  //
  // So the channel screen asks the server whether the credentials are
  // there, and offers the form when they are not. This is the whole
  // point of the status endpoint.
  let telegramCredsDialog = $state(false);

  // Provider labels come from the central $lib/providers module (R-8).
  // The channels page overrides a few labels for the connect dialog
  // (e.g. "X (Twitter)" instead of "X", "Instagram (Standalone)" instead
  // of "Instagram"). Those overrides live here so the central map stays
  // generic.
  import { providerLabel as centralProviderLabel, HIDDEN_PROVIDERS } from "$lib/providers";
  const PROVIDER_LABEL_OVERRIDES: Record<string, string> = {
    x: "X (Twitter)",
    "instagram-standalone": "Instagram (Standalone)",
    // The setup guide and the AI-agent onboarding call this one "Instagram
    // (Business)" - it is the business-mode account reached through the
    // Facebook login flow, and the standalone variant is the card above.
    // The grid used to say "Instagram", which is the name the operator was
    // never given, so they reported the business option as missing. Same
    // product, same qualification, one spelling across surfaces: this
    // mirrors the LinkedIn pair, where `linkedin-page` is labelled
    // "LinkedIn Page" rather than a bare "LinkedIn".
    instagram: "Instagram (Business)",
    "linkedin-page": "LinkedIn Page",
    "telegram-bot": "Telegram Bot",
    "telegram-user": "Telegram User",
    "google-my-business": "Google Business",
  };
  function providerLabel(provider: string): string {
    if (PROVIDER_LABEL_OVERRIDES[provider]) return PROVIDER_LABEL_OVERRIDES[provider];
    return centralProviderLabel(provider);
  }

  // Connectable providers - the same 26 the backend registry always registers
  // (`ProviderRegistry::new` in src/social/registry.rs, unconditional since
  // registration was decoupled from credentials). This literal previously
  // drifted: `youtube` was missing here while the registry, `$lib/providers`
  // and the onboarding modal all listed it, so the grid showed 25 cards for a
  // 26-platform product with no way to connect YouTube. Keep it in step with
  // the registry; the identifier spelling (notably `google_my_business` with an
  // underscore) is the registry's, not `PROVIDERS`' hyphenated alias.
  //
  // Tier membership is defined in src/social/tier.rs (v25 plan §1):
  // kick/vk/whop/lemmy were removed outright, farcaster is Tier-3 (archive).
  //
  // Tier-3 archive providers are hidden by default. To surface one, start the
  // backend with ENABLE_ARCHIVE_PROVIDERS=1 AND add the id below. The flag is a
  // server-side env var the frontend cannot read.
  const ARCHIVE_PROVIDERS: string[] = [];
  // F1: `.filter(...)` against the single HIDDEN_PROVIDERS set in
  // $lib/providers so the tier rule is enforced in code, not only in this
  // comment. Without it a future edit that re-adds kick/vk to the literal below
  // would ship a connect button for a platform the product no longer supports.
  const availableProviders = [
    "x", "facebook", "instagram", "instagram-standalone", "threads",
    "linkedin", "linkedin-page",
    "reddit", "bluesky", "discord", "pinterest",
    "tiktok", "youtube",
    "mastodon",
    "google_my_business", "slack",
    "gmail", "calendar", "drive",
    "telegram-bot", "telegram-user",
    "whatsapp",
    "wordpress", "medium", "devto", "hashnode",
    "github",
    "skool",
    ...ARCHIVE_PROVIDERS,
  ].filter((p) => !HIDDEN_PROVIDERS.has(p));

  let groups = $derived.by(() => groupIntegrations(integrations));

  // v25 F4: page-level status summary. The card states were already
  // individual; this answers the question the list cannot: is anything broken
  // right now? Reuses channelStatus so the two can never disagree.
  let statusCounts = $derived.by(() => {
    const counts = { connected: 0, expiring: 0, refresh_needed: 0, disabled: 0 };
    for (const i of integrations) counts[channelStatus(i).state]++;
    return counts;
  });
  let attentionCount = $derived(statusCounts.expiring + statusCounts.refresh_needed);

  // Already-connected providers keep their tile. The founder managing many
  // channels connects a second Google account, a third LinkedIn identity and a
  // second X handle as a matter of course, and the old rule - hide a provider
  // the moment ONE integration exists for it - made those impossible. Five
  // accounts connected left five tiles missing from this grid, and the only
  // way back was disconnecting the one you wanted to keep alongside.
  //
  // A tile that is already connected reads as "Add another", not "Connect",
  // because that is what the click does. The count comes from
  // `connectedCounts` below - a Set would say only "connected" and the
  // founder with three LinkedIn profiles should see the three.
  let connectedCounts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const i of integrations) m.set(i.provider_identifier, (m.get(i.provider_identifier) ?? 0) + 1);
    return m;
  });
  let connectableProviders = $derived(availableProviders);
  /**
   * Which providers can actually be connected on THIS deployment, and what
   * each one is missing.
   *
   * The grid used to offer a Connect button for a provider whose credentials
   * were absent, and the founder learned it was impossible only after
   * clicking - the refusal was honest, but it arrived one click too late.
   * `configured` is the LIVE state from the server (paste a token in the app
   * and it flips with no restart), and `requires` is the provider's own env
   * hint, so the tile names the exact variables.
   */
  let requiresById = $state<Record<string, { configured: boolean; requires: string }>>({});

  // A provider id the grid should render as the suite tile's representative is
  // `google`; the tile needs to know it is a group so it can show the note.

  async function load() {
    loading = true;
    error = "";
    try {
      const [r, p] = await Promise.all([
        integrationsApi.list(),
        integrationsApi.listProviders(),
      ]);
      if (r.data) integrations = r.data.integrations;
      if (p.data) {
        requiresById = Object.fromEntries(
          p.data.map((s) => [s.identifier, { configured: s.configured, requires: s.requires }]),
        );
      }
    } catch {
      error = "Failed to load channels";
    }
    loading = false;
  }

  async function disconnect(id: string) {
    try {
      await integrationsApi.disconnect(id);
      await load();
    } catch (e) {
      error = "Failed to disconnect channel";
      toast(`Disconnect failed: ${e instanceof Error ? (e instanceof Error ? e.message : String(e)) : "unknown"}`, "error");
    }
  }

  /**
   * v25 F4: opening the unified flow is now a one-liner. Every branch that used
   * to live here (the OAuth popup, the cookie dialog, the API-key dialog, the
   * PAT dialog, the method chooser) lives in ConnectFlow, which decides for
   * itself which one applies.
   */
  function initiateConnect(provider: string) {
    if (provider === "whatsapp") {
      onboardDialog = { provider, step: "phone", phone: "" };
      return;
    }
    if (provider === "telegram-user") {
      // The credential check is ASYNC, so this one path cannot be a
      // synchronous branch like the rest. It is deliberately the only
      // async one: WhatsApp needs no credentials, and every other
      // provider's readiness is already known to the server, which
      // refuses the connect with a message naming the env var.
      void startTelegramUserConnect();
      return;
    }
    connectProvider = provider;
  }

  /**
   * Start the Telegram USER connect, checking the credentials first.
   *
   * The order matters: asking for a phone number before the credentials
   * are known to be present sends the founder down a path that cannot
   * finish, and the only explanation arrives AFTER they have typed a
   * number they were asked for.
   */
  async function startTelegramUserConnect() {
    error = "";
    try {
      const r = await integrationsApi.telegramUserStatus();
      if (r.error) {
        // The status endpoint is the one thing that must answer. If it
        // does not, fall back to showing the phone step rather than
        // blocking the founder on an unreachable endpoint — the server
        // will still refuse the code request with its own message, and
        // that message names the fix.
        onboardDialog = { provider: "telegram-user", step: "phone", phone: "" };
        return;
      }
      if (r.data?.configured) {
        onboardDialog = { provider: "telegram-user", step: "phone", phone: "" };
        return;
      }
      telegramCredsDialog = true;
    } catch {
      onboardDialog = { provider: "telegram-user", step: "phone", phone: "" };
    }
  }

  /**
   * The credentials are saved — carry on to the phone number.
   *
   * The server has already swapped them into the live client, so the
   * next request code works without a restart.
   */
  function handleTelegramCredentialsSaved() {
    telegramCredsDialog = false;
    onboardDialog = { provider: "telegram-user", step: "phone", phone: "" };
  }

  async function handleChannelRefresh(id: string) {
    refreshing = id;
    try {
      const r = await integrationsApi.refresh(id);
      if (r.error) {
        error = `Refresh failed: ${r.error}`;
      } else {
        await load();
      }
    } catch (e: unknown) {
      error = `Refresh failed: ${(e instanceof Error ? e.message : undefined) || "Unknown error"}`;
    }
    refreshing = null;
  }

  async function handleReconnect(id: string) {
    refreshing = id;
    try {
      // Delete existing integration, then start fresh OAuth
      const int = integrations.find(i => i.id === id);
      if (!int) { error = "Integration not found"; refreshing = null; return; }
      await integrationsApi.disconnect(id);
      initiateConnect(int.provider_identifier);
    } catch (e: unknown) {
      error = `Reconnect failed: ${(e instanceof Error ? e.message : undefined) || "Unknown error"}`;
    }
    refreshing = null;
  }

  async function handleToggleDisableIntegration(id: string, disabled: boolean) {
    try {
      await integrationsApi.toggleDisable(id, disabled);
      await load();
    } catch (e) {
      error = "Failed to toggle channel";
      toast(`Toggle disable failed: ${e instanceof Error ? (e instanceof Error ? e.message : String(e)) : "unknown"}`, "error");
    }
  }

  async function onboardSubmitPhone() {
    if (!onboardDialog || !onboardDialog.phone) return;
    error = "";
    connecting = onboardDialog.provider;
    try {
      if (onboardDialog.provider === "whatsapp") {
        const r = await integrationsApi.whatsappPair(onboardDialog.phone);
        if (r.error) { error = r.error; connecting = null; return; }
        onboardDialog = { ...onboardDialog, step: "pair_code", pairCode: r.data?.pair_code ?? "" };
        connecting = null;
        pollWhatsAppAuth();
      } else if (onboardDialog.provider === "telegram-user") {
        const r = await integrationsApi.telegramUserRequestCode(onboardDialog.phone);
        if (r.error) { error = r.error; connecting = null; return; }
        onboardDialog = { ...onboardDialog, step: "sms_code", code: "" };
        connecting = null;
      }
    } catch (e: unknown) { error = (e instanceof Error ? e.message : undefined) || "Failed"; connecting = null; }
  }

  /**
   * WhatsApp pairing is the one connect path that polls, and it is not a
   * polling regression: the event being watched happens on the user's PHONE
   * (they enter the pair code under Linked Devices). The server has no way to
   * observe that, so there is no SSE event to subscribe to and
   * `/whatsapp/status` is the only oracle. Every other connect path in this
   * file is SSE-driven through the `integration_connected` subscription in
   * onMount.
   */
  async function pollWhatsAppAuth() {
    for (let i = 0; i < 60; i++) {
      await new Promise(r => setTimeout(r, 3000));
      if (!onboardDialog || onboardDialog.provider !== "whatsapp") return;
      try {
        const r = await integrationsApi.whatsappStatus();
        if (r.data?.authenticated) {
          const v = await integrationsApi.verifyOneTimeToken("whatsapp", "");
          if (v.error) { error = v.error; return; }
          onboardDialog = null;
          await load();
          return;
        }
      } catch { /* keep polling */ }
    }
    error = "Timed out waiting for WhatsApp authentication";
  }

  async function onboardSubmitCode() {
    if (!onboardDialog || !onboardDialog.code) return;
    error = "";
    connecting = onboardDialog.provider;
    try {
      if (onboardDialog.provider === "telegram-user") {
        const r = await integrationsApi.telegramUserSignIn(onboardDialog.code);
        if (r.error) { error = r.error; connecting = null; return; }
        onboardDialog = null;
        connecting = null;
        await load();
      } else {
        const r = await integrationsApi.verifyOneTimeToken(onboardDialog.provider, onboardDialog.code ?? "");
        if (r.error) { error = r.error; connecting = null; return; }
        onboardDialog = null;
        connecting = null;
        await load();
      }
    } catch (e: unknown) { error = (e instanceof Error ? e.message : undefined) || "Verification failed"; connecting = null; }
  }

  let chanUnsubscribers: (() => void)[] = [];

  onMount(() => {
    load();
    // v25 F4: this subscription is now the ONLY completion signal for every
    // connect method, including the OAuth pop-up. The pop-up used to run a 1s
    // setInterval to guess whether it had closed and then re-fetch the whole
    // integration list on a timer.
    // `lagged` is the server's synthetic "you missed events" signal - refetch
    // the list so a backgrounded tab doesn't show stale connections.
    const events = ['integration_connected', 'integration_disconnected', 'lagged'];
    for (const evt of events) {
      chanUnsubscribers.push(realtime.on(evt, () => load()));
    }
  });

  onDestroy(() => {
    chanUnsubscribers.forEach(fn => fn());
  });
</script>

<div class="page-enter space-y-6">
  <h1 class="text-xl font-semibold">Channel Management</h1>

  <!-- Connected channels -->
  <div class="bg-surface border border-line rounded-xl p-4">
    <div class="flex items-center justify-between mb-3 gap-3 flex-wrap">
      <h2 class="text-sm font-semibold text-muted uppercase tracking-wider">Connected Channels</h2>
      {#if !loading && integrations.length > 0}
        <!--
          F4: the page-level answer to "is anything wrong right now?". A list of
          rows each carrying its own chip is precise but not glanceable; this
          states the count up front.
        -->
        <div class="flex items-center gap-3 text-xs">
          <span class="text-muted">{integrations.length} total</span>
          {#if attentionCount > 0}
            <span class="inline-flex items-center gap-1 text-warning">
              <Icon name="alert" class="w-3.5 h-3.5" />
              {attentionCount} needing attention
            </span>
          {:else}
            <span class="inline-flex items-center gap-1 text-success">
              <Icon name="check" class="w-3.5 h-3.5" />
              All healthy
            </span>
          {/if}
        </div>
      {/if}
    </div>

    {#if loading}
      <Skeleton variant="row" rows={4} />
    {:else if error && integrations.length === 0}
      <ErrorState message={error} actionLabel="Retry" onaction={load} />
    {:else if integrations.length === 0}
      <EmptyState
        icon="channel"
        title="No channels connected yet"
        description="Connect a platform below and it becomes available everywhere: composer, calendar, and analytics."
      />
    {:else}
      {#each [...groups.entries()] as [name, ints] (name)}
        <div class="mb-4 last:mb-0">
          <div class="text-xs text-muted px-1 mb-1">{groupHeading(name, ints)} ({ints.length})</div>
          <!-- F4: <ul> because ChannelCard renders an <li>; the list semantics
               let a screen reader announce the item count before reading rows. -->
          <ul class="page-enter space-y-0.5">
            {#each ints as int (int.id)}
              <ChannelCard
                integration={int}
                timeslots={int.posting_times?.map((t: { time: number }) => ({ time: t.time })) || []}
                onDisconnect={disconnect}
                onRefresh={() => handleChannelRefresh(int.id)}
                onReconnect={() => handleReconnect(int.id)}
                onToggleDisable={handleToggleDisableIntegration}
                isRefreshing={refreshing === int.id}
              />
            {/each}
          </ul>
        </div>
      {/each}
    {/if}
  </div>

  <!-- Available providers grid -->
  <div class="bg-surface border border-line rounded-xl p-4">
    <h2 class="text-sm font-semibold text-muted uppercase tracking-wider mb-3">Available Providers</h2>
    {#if connectableProviders.length === 0}
      <p class="text-sm text-muted py-6 text-center">
        Every supported platform is connected. Add more accounts from a channel's context menu.
      </p>
    {:else}
      <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3">
        {#each connectableProviders as provider (provider)}
          {@const connCount = connectedCounts.get(provider) ?? 0}
          {@const info = requiresById[provider]}
          {@const unavailable = info !== undefined && !info.configured}
          {@const tileLabel = unavailable
            ? `${providerLabel(provider)} is not configured on this server. ${info?.requires}`
            : connCount > 0
              ? `Add another ${providerLabel(provider)} (${connCount} connected)`
              : providerLabel(provider)}
          <!--
            A wrapper `div` for the EVERY case, not just the suite: the
            chooser below is a sibling of the tile button, never a child.
            A `<button>` inside a `<button>` is invalid HTML - the browser
            repairs it on parse, which produces a hydration mismatch and a
            warning that hides real defects.
          -->
          <div class="relative">
            <button
              onclick={() => initiateConnect(provider)}
              disabled={connecting === provider || unavailable}
              title={unavailable ? info?.requires : undefined}
              aria-label={tileLabel}
              class="w-full h-full relative flex flex-col items-center gap-2 p-4 bg-background-input border border-line rounded-xl hover:border-accent/50 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {#if connCount > 0}
                <span class="absolute top-1.5 right-1.5 text-[10px] px-1.5 py-0.5 rounded-full border border-line text-muted bg-surface">
                  {connCount} connected
                </span>
              {/if}
              <ProviderIcon {provider} size="lg" />
              <span class="text-xs text-center">
                {connCount > 0 ? `Add another ${providerLabel(provider)}` : providerLabel(provider)}
              </span>
              {#if unavailable}
                <!--
                  The one thing the founder needs BEFORE clicking: this
                  provider has no credentials on this server, and what to
                  put in the env to get them. A refusal after the click is
                  too late to plan around.
                -->
                <span class="text-[10px] leading-tight text-center px-1 text-amber-600 dark:text-amber-400">
                  Needs credentials
                </span>
                <span class="text-[10px] leading-tight text-center px-1 text-muted">
                  {info?.requires}
                </span>
              {/if}
            </button>

          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<!-- v25 F4: the one connect entry point. Replaces the page's bespoke
     connectChoice / credDialog dialogs and the inline OAuth popup branch. -->
<ConnectFlow
  provider={connectProvider ?? ""}
  show={connectProvider !== null}
  onSuccess={() => { connectProvider = null; load(); }}
  onClose={() => { connectProvider = null; }}
/>

{#if onboardDialog}
<div class="fixed inset-0 bg-overlay flex items-center justify-center z-50 p-4" role="dialog" aria-modal="true" aria-label="Connect {providerLabel(onboardDialog.provider)}" tabindex="-1" use:focusTrap>
  <div class="bg-surface border border-line rounded-xl p-6 w-full max-w-sm elev-lg">
    <h2 class="text-lg font-semibold mb-3">Connect {providerLabel(onboardDialog.provider)}</h2>

    {#if onboardDialog.step === "phone"}
      <p class="text-sm text-muted mb-3">
        {#if onboardDialog.provider === "whatsapp"}Enter your phone number to get a pairing code.{:else}Enter your phone number to receive a login code via Telegram.{/if}
      </p>
      <label for="onboard-phone" class="sr-only">Phone number</label>
      <input id="onboard-phone" type="tel" bind:value={onboardDialog.phone} placeholder="+1234567890" class="min-h-11 w-full mb-4 px-3 py-2 bg-background-input border border-line rounded text-sm" />
      {#if error}<p class="text-error text-sm mb-3" role="alert">{error}</p>{/if}
      <div class="flex justify-end gap-2">
        <button onclick={() => { onboardDialog = null; error = ""; }} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
        <button onclick={onboardSubmitPhone} disabled={!!connecting} class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded disabled:opacity-50 min-h-11">
          {connecting ? "Sending…" : "Next"}
        </button>
      </div>

    {:else if onboardDialog.step === "pair_code"}
      <p class="text-sm text-muted mb-2">Open WhatsApp on your phone:</p>
      <p class="text-sm text-content-secondary mb-3">Settings → Linked Devices → Link a Device → Enter code</p>
      <div class="bg-background-input border border-line rounded-lg p-4 mb-4 text-center">
        <span class="text-2xl font-mono font-bold tracking-[0.3em] text-content">{onboardDialog.pairCode}</span>
      </div>
      <p class="text-xs text-muted mb-3 text-center">Waiting for you to enter the code on your phone…</p>
      <div class="flex justify-center" role="status" aria-label="Waiting for WhatsApp to confirm the pairing">
        <div class="animate-spin h-5 w-5 border-2 border-accent border-t-transparent rounded-full"></div>
      </div>
      <div class="flex justify-end mt-4">
        <button onclick={() => { onboardDialog = null; error = ""; }} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
      </div>

    {:else if onboardDialog.step === "sms_code"}
      <p class="text-sm text-muted mb-3">Enter the code sent to your Telegram app.</p>
      <label for="onboard-code" class="sr-only">Login code</label>
      <input id="onboard-code" type="text" bind:value={onboardDialog.code} placeholder="12345" class="min-h-11 w-full mb-4 px-3 py-2 bg-background-input border border-line rounded text-sm font-mono text-center text-lg tracking-widest" />
      {#if error}<p class="text-error text-sm mb-3" role="alert">{error}</p>{/if}
      <div class="flex justify-end gap-2">
        <button onclick={() => { onboardDialog = null; error = ""; }} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
        <button onclick={onboardSubmitCode} disabled={!!connecting} class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded disabled:opacity-50 min-h-11">
          {connecting ? "Signing in…" : "Sign In"}
        </button>
      </div>

    {:else if onboardDialog.step === "bot_code"}
      {@const parts = (onboardDialog.instructions ?? "").split("\n")}
      {@const botUsername = parts[0] ?? ""}
      {@const connectCmd = parts[1] ?? ""}
      <div class="page-enter space-y-3 mb-4">
        <p class="text-sm text-muted">1. Open this bot in Telegram:</p>
        <a href="https://t.me/{botUsername.replace('@','')}" target="_blank" rel="noreferrer" class="block text-center text-accent hover:text-accent-strong font-medium">{botUsername}</a>
        <p class="text-sm text-muted">2. Send this command to the bot or any group/channel it's in:</p>
        <div class="bg-background-input border border-line rounded-lg p-3 text-center">
          <code class="text-sm text-content font-mono break-all">{connectCmd}</code>
        </div>
        <p class="text-sm text-muted">3. Click Verify below.</p>
      </div>
      {#if error}<p class="text-error text-sm mb-3" role="alert">{error}</p>{/if}
      <div class="flex justify-end gap-2">
        <button onclick={() => { onboardDialog = null; error = ""; }} class="px-4 py-2 text-sm text-muted hover:text-content min-h-11">Cancel</button>
        <button onclick={onboardSubmitCode} disabled={!!connecting} class="px-4 py-2 text-sm bg-accent-fill hover:bg-accent-fill-hover text-accent-fg rounded disabled:opacity-50 min-h-11">
          {connecting ? "Verifying…" : "Verify"}
        </button>
      </div>
    {/if}
  </div>
</div>
{/if}

<!--
  The Telegram USER credentials form.

  It is a SEPARATE dialog from the phone-number wizard on purpose: the
  phone-number step assumes the credentials are already in place, and
  mixing the two would put two unrelated forms in one modal. The server
  writes the credentials to the config file and swaps them into the live
  client, so the phone step that follows works with no restart.
-->
<TelegramCredentials
  show={telegramCredsDialog}
  onSuccess={handleTelegramCredentialsSaved}
  onClose={() => { telegramCredsDialog = false; error = ""; }}
/>
