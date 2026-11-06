<script lang="ts">
  // v25 F4 - OAuth path of the unified connect flow.
  //
  // The old inline implementation in routes/channels/+page.svelte ran a
  // `setInterval(..., 1000)` to notice the popup closing. That was polling a
  // DOM property to guess whether a connect had landed, and it re-fetched the
  // whole integration list once a second while the user was authorising in
  // another window. v25 plan §4 keeps SSE as the only realtime transport, so
  // this component waits for the two things that actually mean "done":
  //   1. the popup's `oauth-connected` postMessage, and
  //   2. nothing else - the page's `integration_connected` SSE subscription
  //      already refreshes the list, so a closed popup needs no timer.
  import { integrationsApi } from "$lib/api/integrations";
  import Modal from "$lib/ui/Modal.svelte";
  import Button from "$lib/ui/Button.svelte";
  import Spinner from "$lib/ui/Spinner.svelte";
  import Icon from "$lib/ui/Icon.svelte";
  import { providerLabel } from "$lib/providers";

  let {
    provider = "",
    show = false,
    onClose,
    onSuccess,
  }: {
    provider?: string;
    show?: boolean;
    onClose?: () => void;
    onSuccess?: () => void;
  } = $props();

  type Phase = "idle" | "starting" | "awaiting" | "bot_code" | "done";

  let phase = $state<Phase>("idle");
  let errorMsg = $state("");
  let botInstructions = $state("");
  let botCode = $state("");
  let verifying = $state(false);
  let popupBlocked = $state(false);

  // ─── THE PROVIDER'S SIGN-IN WARNING ────────────────────────────
  // Some providers can refuse a sign-in at a step OUR server never sees.
  // Meta's Threads is the one we have measured: its consent gate
  // redirects the BROWSER to Meta's own privacy/consent page with
  // `error_code=1349003` and never comes back to our callback, so the
  // founder is left on a Meta screen with nothing to do about it.
  //
  // The connect route asks the provider whether it knows of such a gate
  // and returns the answer alongside the URL. This shows it, so the
  // founder sees the reason and the fix BEFORE they hit the wall.
  let warning = $state("");

  function reset() {
    phase = "idle";
    errorMsg = "";
    botInstructions = "";
    botCode = "";
    verifying = false;
    popupBlocked = false;
    warning = "";
  }

  function handleClose() {
    if (verifying) return;
    reset();
    onClose?.();
  }

  /** Listen for the popup's success broadcast. Returns an unsubscribe. */
  function watchPopup(): () => void {
    const onMessage = (e: MessageEvent) => {
      if (e.data?.type === "oauth-connected") {
        // ─── THE POPUP'S VERDICT IS NOT ALWAYS SUCCESS ──────────
        // The callback page posts this one event for BOTH outcomes, with
        // a `success` flag. The old listener only looked at the TYPE, so a
        // failed OAuth (a provider rejection, a redirect_uri mismatch, an
        // expired consent) reported "Connected" to the founder — which is
        // exactly what the Threads login-error report described: nothing
        // happened, and no error was shown anywhere.
        //
        // Only a `success: true` closes the dialog and calls onSuccess. A
        // failure keeps the dialog open and shows the provider's own
        // message, so the next attempt starts from what actually went
        // wrong.
        cleanup();
        if (e.data.success) {
          phase = "done";
          onSuccess?.();
        } else {
          phase = "idle";
          errorMsg = e.data.error || "The sign-in did not complete. Close this and try again.";
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }

  let cleanup: (() => void) | null = null;

  async function start() {
    phase = "starting";
    errorMsg = "";
    popupBlocked = false;
    try {
      const r = await integrationsApi.connect(provider);
      if (r.error) {
        errorMsg = r.error;
        phase = "idle";
        return;
      }
      const url = r.data?.url;
      if (!url) {
        errorMsg = "The server did not return an authorization URL.";
        phase = "idle";
        return;
      }

      // The provider's own warning, if it named one.
      warning = r.data?.warning ?? "";

      // Server completed the flow itself (env-var credentials already present).
      if (r.data?.state === "auto") {
        phase = "done";
        onSuccess?.();
        return;
      }

      // Chat platforms hand back a one-time code to run inside the app itself.
      if (r.data?.state === "one-time-token") {
        botInstructions = url;
        botCode = url.match(/\/connect\s+(\S+)/)?.[1] ?? "";
        phase = "bot_code";
        return;
      }

      const popup = window.open(url, "_blank", "width=600,height=700");
      if (!popup) {
        // Popup blockers are common and the failure mode is invisible otherwise.
        popupBlocked = true;
        errorMsg = "Your browser blocked the sign-in window. Allow popups for this site, then try again.";
        phase = "idle";
        return;
      }
      cleanup = watchPopup();
      phase = "awaiting";
    } catch (e: unknown) {
      errorMsg = e instanceof Error ? e.message : "Could not start the connection.";
      phase = "idle";
    }
  }

  async function verifyBotCode() {
    if (!botCode) return;
    verifying = true;
    errorMsg = "";
    try {
      const r = await integrationsApi.verifyOneTimeToken(provider, botCode);
      if (r.error) {
        errorMsg = r.error;
      } else {
        phase = "done";
        onSuccess?.();
      }
    } catch (e: unknown) {
      errorMsg = e instanceof Error ? e.message : "Verification failed";
    }
    verifying = false;
  }

  $effect(() => {
    if (!show) {
      cleanup?.();
      cleanup = null;
    }
  });

  $effect(() => {
    if (show) reset();
  });

  // Release the message listener when the component goes away entirely.
  $effect(() => {
    return () => {
      cleanup?.();
      cleanup = null;
    };
  });
</script>

<Modal open={show} title="Connect {providerLabel(provider)}" onclose={handleClose}>
  <div class="space-y-4">
    <!--
      THE WARNING IS ABOVE THE PHASE BLOCK, NOT INSIDE IT.

      The warning arrives with the authorization URL, which is the moment
      the phase flips from "idle" to "awaiting". A warning rendered only
      in the idle branch therefore disappears EXACTLY when the founder
      is about to hit Meta's consent wall — which is the moment it
      matters. It stays up for the whole attempt instead.

      It is a block of TEXT, not markup: the server builds it, and a
      provider's answer is not HTML. Rendering it as text means a
      provider can never inject markup into the page.
    -->
    {#if warning}
      <div
        class="text-sm text-content bg-warning/10 border border-warning/20 rounded-lg px-3 py-2 whitespace-pre-line"
        role="status"
      >
        {warning}
      </div>
    {/if}

    {#if phase === "bot_code"}
      <p class="text-sm text-muted">1. Open this bot:</p>
      <a
        href="https://t.me/{botInstructions.split('\n')[0]?.replace('@', '') ?? ''}"
        target="_blank"
        rel="noreferrer"
        class="block text-center text-accent hover:text-accent-strong font-medium break-all"
      >
        {botInstructions.split('\n')[0]}
      </a>
      <p class="text-sm text-muted">2. Send this command to the bot, or to any group it is in:</p>
      <div class="bg-background-input border border-line rounded-lg p-3 text-center">
        <code class="text-sm text-content font-mono break-all">{botInstructions.split('\n')[1] ?? ""}</code>
      </div>
      <p class="text-sm text-muted">3. Confirm it here.</p>

      {#if errorMsg}
        <div class="text-sm text-error bg-error/10 border border-error/20 rounded-lg px-3 py-2" role="alert">
          {errorMsg}
        </div>
      {/if}

      <div class="flex justify-end gap-2 pt-2">
        <Button variant="secondary" onclick={handleClose} disabled={verifying}>Cancel</Button>
        <Button variant="primary" onclick={verifyBotCode} disabled={verifying || !botCode}>
          {#if verifying}
            <span class="flex items-center gap-2"><Spinner size="sm" /> Verifying...</span>
          {:else}
            Verify
          {/if}
        </Button>
      </div>
    {:else if phase === "awaiting"}
      <div class="flex items-start gap-3 text-sm">
        <Spinner size="md" />
        <div>
          <p class="font-medium">Finish signing in the pop-up window</p>
          <p class="text-muted mt-1">
            This page updates on its own the moment the connection lands. You can close this
            dialog and carry on. Nothing is lost.
          </p>
        </div>
      </div>
      <div class="flex justify-end pt-2">
        <Button variant="secondary" onclick={handleClose}>Close</Button>
      </div>
    {:else if phase === "done"}
      <div class="flex items-center gap-3 text-sm text-success">
        <Icon name="check" class="w-5 h-5" />
        <span>Connected. {providerLabel(provider)} is ready to post.</span>
      </div>
      <div class="flex justify-end pt-2">
        <Button variant="primary" onclick={handleClose}>Done</Button>
      </div>
    {:else}
      <p class="text-sm text-muted">
        {providerLabel(provider)} uses standard OAuth 2.0. A sign-in window opens; this page
        updates as soon as you approve access there.
      </p>

      {#if errorMsg}
        <div class="text-sm text-error bg-error/10 border border-error/20 rounded-lg px-3 py-2" role="alert">
          {errorMsg}
        </div>
      {/if}

      <div class="flex justify-end gap-2 pt-2">
        <Button variant="secondary" onclick={handleClose}>Cancel</Button>
        <Button variant="primary" onclick={start} disabled={phase === "starting"}>
          {#if phase === "starting"}
            <span class="flex items-center gap-2"><Spinner size="sm" /> Starting...</span>
          {:else if popupBlocked}
            Try again
          {:else}
            Continue
          {/if}
        </Button>
      </div>
    {/if}
  </div>
</Modal>
