<script lang="ts">
  // ModalManager - renders the global modal stack (Phase 0).
  //
  // Mount this once at the +layout.svelte level (authenticated branch).
  // It reads from the `modals` store and renders each modal in the stack
  // with the correct z-index. Only the topmost modal is interactive;
  // lower modals get pointer-events-none.
  //
  // Also handles:
  //   - Body scroll lock when stack is non-empty
  //   - Escape key closes the topmost modal (with askClose confirm if set)
  //   - Backdrop click closes the topmost (with askClose confirm if set)
  //   - The pending confirm dialog (from modals.areYouSure())

  import { onMount, onDestroy } from 'svelte';
  import { modals } from '$lib/stores/modals.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import { focusTrap } from '$lib/ui/focus-trap';

  // Body scroll lock: when any modal is open, lock body overflow.
  $effect(() => {
    if (modals.stack.length > 0) {
      const prev = document.body.style.overflow;
      document.body.style.overflow = 'hidden';
      return () => {
        document.body.style.overflow = prev;
      };
    }
  });

  // Escape key: close the topmost modal (with askClose confirm if set).
  function handleKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    // If a confirm dialog is pending, Escape cancels it (resolves false).
    if (modals._pendingConfirm) {
      modals._pendingConfirm.resolve(false);
      return;
    }
    if (modals.stack.length === 0) return;
    const top = modals.stack[modals.stack.length - 1];
    if (top.options.closeOnEscape === false) return;
    modals.closeCurrent(false);
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
  });
  // `onDestroy` also runs during SSR, where `window` does not exist - the bare
  // `window.removeEventListener` here used to throw `window is not defined`
  // and 500 every server-rendered route in dev. The listener is only ever
  // added from onMount (client-only), so skipping the removal on the server is
  // correct, not just defensive.
  onDestroy(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener('keydown', handleKeydown);
    }
  });

  // Resolve a size string to a CSS max-width.
  function sizeToMaxWidth(size: string | undefined): string {
    if (!size) return 'max-w-lg';
    // If it's a Tailwind max-w-* class, use it directly.
    if (size.startsWith('max-w-')) return size;
    // If it ends with %, px, rem, em - use as inline style value.
    if (/(\d+(\.\d+)?)(%|px|rem|em|vw|vh)$/.test(size)) return size;
    // Default: treat as a Tailwind class.
    return size;
  }
  function isInlineSize(size: string): boolean {
    return /(\d+(\.\d+)?)(%|px|rem|em|vw|vh)$/.test(size);
  }

  // Confirm dialog handlers.
  function resolveConfirm(ok: boolean) {
    if (modals._pendingConfirm) {
      modals._pendingConfirm.resolve(ok);
    }
  }
</script>

<!-- Render the modal stack. Each modal is a full-viewport overlay with
     its own backdrop. Lower modals are pointer-events-none so only the
     topmost responds to clicks. -->
{#each modals.stack as entry, idx (entry.id)}
  <div
    class="fixed inset-0 flex items-center justify-center p-4 {entry.isTop ? '' : 'pointer-events-none'}"
    style="z-index: {entry.zIndex}"
    role="dialog"
    aria-modal="true"
    aria-labelledby={entry.options.title ? `mm-title-${entry.id}` : undefined}
    aria-label={entry.options.title ? undefined : (entry.options.ariaLabel || 'Dialog')}
  >
    <!-- Backdrop -->
    {#if entry.isTop}
      <!-- v26: a full-screen modal (composer) gets the HEAVY TRANSLUCENT scrim,
           not an opaque one. The opaque bg-surface backdrop hid the page
           completely - the "modals make everything opaque" report: the
           founder could not tell the dialog sat on the calendar they were
           working in. The page stays visible but dimmed, so the composer
           keeps its context, and the scrim still hides the page detail well
           enough that the probe does not read overlapping/dimmed text. -->
      <div
        class="absolute inset-0 {entry.options.fullScreen ? 'bg-overlay-strong' : 'bg-overlay'} {entry.options.closeOnClickOutside !== false ? 'cursor-pointer' : ''}"
        aria-hidden="true"
        onclick={() => {
          if (entry.options.closeOnClickOutside !== false) {
            modals.close(entry.id, false);
          }
        }}
      ></div>
    {:else}
      <!-- Lower modals keep the standard scrim so the top modal stands out.
           (The old comment said "bg-overlay/60" but the class below was plain
           bg-overlay - a bare-var token cannot take an alpha modifier, so the
           commented class was never what rendered.) -->
      <div class="absolute inset-0 bg-overlay pointer-events-none" aria-hidden="true"></div>
    {/if}

    <!-- Modal panel -->
    <div
      class="relative bg-surface border border-line rounded-xl shadow-lg flex flex-col max-h-[90vh] w-full
        {entry.options.fullScreen ? 'max-w-[1400px] h-[90vh]' : ''}
        {entry.options.panelClass || ''}
        {entry.isTop ? '' : 'opacity-90'}"
      style={entry.options.fullScreen
        ? ''
        : (entry.options.size && isInlineSize(sizeToMaxWidth(entry.options.size))
            ? `max-width: ${sizeToMaxWidth(entry.options.size)}; width: 100%;`
            : '')}
      class:max-w-lg={!entry.options.size && !entry.options.fullScreen}
      class:max-w-4xl={entry.options.size === 'max-w-4xl'}
      class:max-w-2xl={entry.options.size === 'max-w-2xl'}
      class:max-w-6xl={entry.options.size === 'max-w-6xl'}
      tabindex="-1"
      use:focusTrap={{ enabled: entry.isTop }}
    >
      <!-- Header (only if title or close button) -->
      {#if entry.options.title || entry.options.withCloseButton !== false}
        <div class="flex items-center justify-between px-5 py-3 border-b border-line shrink-0">
          {#if entry.options.title}
            <h3 id={`mm-title-${entry.id}`} class="text-lg font-semibold">{entry.options.title}</h3>
          {:else}
            <span></span>
          {/if}
          {#if entry.options.withCloseButton !== false}
            <button
              onclick={() => modals.close(entry.id, true)}
              aria-label="Close dialog"
              class="text-muted hover:text-content text-2xl leading-none -mt-1"
            >&times;</button>
          {/if}
        </div>
      {/if}

      <!-- Body: either the component or the snippet -->
      <div class="flex-1 overflow-y-auto {entry.options.fullScreen ? '' : 'p-5'}">
        {#if entry.component}
          {@const C = entry.component}
          <C {...entry.props} close={(confirmed: boolean = false) => modals.close(entry.id, confirmed)} />
        {:else if entry.snippet}
          {@render entry.snippet()}
        {/if}
      </div>
    </div>
  </div>
{/each}

<!-- Pending confirm dialog (from modals.areYouSure). Rendered on top of
     everything else with the highest z-index. -->
{#if modals._pendingConfirm}
  {@const pc = modals._pendingConfirm}
  <div
    class="fixed inset-0 flex items-center justify-center p-4"
    style="z-index: {500 + modals.stack.length}"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby={pc.opts.title ? 'mm-confirm-title' : undefined}
    aria-describedby={pc.opts.message ? 'mm-confirm-msg' : undefined}
    aria-label={pc.opts.title ? undefined : 'Confirm'}
  >
    <div
      class="absolute inset-0 bg-overlay"
      aria-hidden="true"
      onclick={() => resolveConfirm(false)}
    ></div>
    <div
      class="relative bg-surface border border-line rounded-xl shadow-lg max-w-sm w-full p-6"
      tabindex="-1"
      use:focusTrap={{ focusSelf: false }}
    >
      {#if pc.opts.title}
        <h3 id="mm-confirm-title" class="text-lg font-semibold mb-2">{pc.opts.title}</h3>
      {/if}
      {#if pc.opts.message}
        <p id="mm-confirm-msg" class="text-sm text-muted mb-5">{pc.opts.message}</p>
      {/if}
      <div class="flex justify-end gap-2">
        <button
          onclick={() => resolveConfirm(false)}
          class="px-4 py-2 text-sm text-muted hover:text-content border border-line rounded-lg transition-colors"
        >
          {pc.opts.cancelLabel || 'Cancel'}
        </button>
        <button
          onclick={() => resolveConfirm(true)}
          class="px-4 py-2 text-sm rounded-lg transition-colors {pc.opts.danger ? 'text-error-fg bg-error hover:bg-error' : 'text-accent-fg bg-accent-fill hover:bg-accent-fill-hover'}"
        >
          {pc.opts.confirmLabel || 'Confirm'}
        </button>
      </div>
    </div>
  </div>
{/if}
