<script lang="ts">
  // ─── WHY THIS COMPONENT EXISTS ──────────────────────────────────
  //
  // The Telegram USER channel signs in through Telegram's own MTProto
  // network, which needs an API id and hash from https://my.telegram.org.
  // There is no token to paste and no OAuth screen — the credentials ARE
  // the setup.
  //
  // The founder's report was the dead end this component removes:
  //
  //   "Telegram is not configured on this server ... set TELEGRAM_API_ID
  //    and TELEGRAM_API_HASH in ~/.switchpost/.env and restart."
  //
  // That message names a FILE on a SERVER and asks for a RESTART. The
  // founder is in a browser, on their laptop, and cannot do any of
  // those three things. The credentials are right there on the same
  // screen at my.telegram.org.
  //
  // So the credentials are collected HERE, and the server does the rest:
  // it writes them to the config file (so they survive a restart) and
  // swaps them into the live client (so they work NOW).
  import { integrationsApi } from "$lib/api/integrations";
  import Modal from "$lib/ui/Modal.svelte";
  import Button from "$lib/ui/Button.svelte";
  import Spinner from "$lib/ui/Spinner.svelte";

  let {
    show = false,
    onClose,
    onSuccess,
  }: {
    show?: boolean;
    onClose?: () => void;
    onSuccess?: () => void;
  } = $props();

  let apiId = $state("");
  let apiHash = $state("");
  let submitting = $state(false);
  let errorMsg = $state("");
  let savedTo = $state("");

  function reset() {
    apiId = "";
    apiHash = "";
    errorMsg = "";
    submitting = false;
    savedTo = "";
  }

  async function handleSubmit() {
    // The server validates too — this is a fast local check so the
    // founder is not round-tripped to learn that a field is empty.
    if (!apiId.trim()) {
      errorMsg = "Enter the App ID from my.telegram.org. It is a number, shown next to 'App api_id'.";
      return;
    }
    if (!apiHash.trim()) {
      errorMsg = "Enter the App hash from my.telegram.org. It is the 32-character hex string next to 'App api_hash'.";
      return;
    }

    submitting = true;
    errorMsg = "";
    try {
      const r = await integrationsApi.telegramUserCredentials({
        api_id: apiId.trim(),
        api_hash: apiHash.trim(),
      });
      if (r.error) {
        errorMsg = r.error;
      } else {
        savedTo = r.data?.saved_to ?? "";
        onSuccess?.();
      }
    } catch (e: unknown) {
      errorMsg = e instanceof Error ? e.message : "Could not save the Telegram credentials.";
    }
    submitting = false;
  }

  function handleClose() {
    if (!submitting) {
      reset();
      onClose?.();
    }
  }
</script>

<Modal open={show} title="Set up Telegram User" onclose={handleClose}>
  <div class="space-y-4">
    <div class="bg-background-input border border-line rounded-lg p-3 space-y-1">
      <p class="text-sm text-content">
        Telegram's user channel needs an <span class="font-medium">App ID</span> and
        <span class="font-medium">App hash</span> from
        <a
          href="https://my.telegram.org"
          target="_blank"
          rel="noreferrer"
          class="text-accent hover:text-accent-strong underline"
        >my.telegram.org</a
        >.
      </p>
      <ol class="text-sm text-muted list-decimal list-inside space-y-0.5">
        <li>Sign in to my.telegram.org with your phone number.</li>
        <li>Open <span class="font-mono text-xs">API development tools</span>.</li>
        <li>Create an app — any name works; it is for your own account.</li>
        <li>Copy the two values below.</li>
      </ol>
    </div>

    <form onsubmit={handleSubmit} class="space-y-4">
      <div>
        <label for="tg-api-id" class="block text-xs font-medium text-muted mb-1">
          App ID <span class="text-error">*</span>
        </label>
        <input
          id="tg-api-id"
          type="text"
          inputmode="numeric"
          bind:value={apiId}
          placeholder="e.g. 21102518"
          autocomplete="off"
          class="min-h-11 w-full px-3 py-2 bg-background-input border border-line rounded-lg text-sm text-content placeholder:text-faint focus:outline-none focus:border-accent transition-colors"
        />
      </div>

      <div>
        <label for="tg-api-hash" class="block text-xs font-medium text-muted mb-1">
          App hash <span class="text-error">*</span>
        </label>
        <input
          id="tg-api-hash"
          type="password"
          bind:value={apiHash}
          placeholder="32-character hex string"
          autocomplete="off"
          class="min-h-11 w-full px-3 py-2 bg-background-input border border-line rounded-lg text-sm text-content placeholder:text-faint focus:outline-none focus:border-accent transition-colors"
        />
      </div>

      {#if errorMsg}
        <div class="text-sm text-error bg-error/10 border border-error/20 rounded-lg px-3 py-2" role="alert">
          {errorMsg}
        </div>
      {/if}

      {#if savedTo}
        <div class="text-sm text-success bg-success/10 border border-success/20 rounded-lg px-3 py-2" role="status">
          Saved. Telegram User is ready — continue with your phone number.
          <span class="block text-xs text-muted mt-1">Stored in {savedTo}</span>
        </div>
      {/if}

      <div class="flex justify-end gap-2 pt-2">
        <Button variant="secondary" onclick={handleClose} disabled={submitting}>Cancel</Button>
        <Button variant="primary" onclick={handleSubmit} disabled={submitting}>
          {#if submitting}
            <span class="flex items-center gap-2">
              <Spinner size="sm" /> Saving...
            </span>
          {:else}
            Save
          {/if}
        </Button>
      </div>
    </form>
  </div>
</Modal>
