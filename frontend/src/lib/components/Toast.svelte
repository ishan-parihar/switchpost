<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { onToast, type Toast } from '$lib/stores/toast';

  let toasts = $state<Toast[]>([]);
  let unsubscribe: (() => void) | null = null;

  onMount(() => {
    unsubscribe = onToast(t => {
      if (t.message) toasts = [...toasts, t];
      else toasts = toasts.filter(x => x.id !== t.id);
    });
  });

  onDestroy(() => {
    if (unsubscribe) unsubscribe();
  });
</script>

{#each toasts as t (t.id)}
  <!--
    Each type is a FILL and its matching INK, never `bg-X text-X`.

    `bg-error text-error` binds the SAME `--error-rgb` var to both the
    background and the text, so an error toast is red text on red with a
    contrast ratio of 1.00:1 in every theme -- the message is invisible. The
    same shape was true of the warning toast. The `*-fg` tokens exist for
    exactly this: dark-theme --warning-rgb is yellow-500 (234 179 8), which
    needs the DARK surface ink (11 14 20) rather than more yellow.

    The info toast used `bg-hue-neutral/90 text-hue-neutral`, the same
    self-pairing, on a wash instead of a fill.
  -->
  <div class="fixed bottom-4 right-4 z-50 px-4 py-3 rounded-lg shadow-lg text-sm max-w-sm animate-slide-up
    {t.type === 'success' ? 'bg-success text-success-fg' : ''}
    {t.type === 'error' ? 'bg-error text-error-fg' : ''}
    {t.type === 'warning' ? 'bg-warning text-warning-fg' : ''}
    {t.type === 'info' ? 'bg-info/90 text-info-fg' : ''}"
  >
    {t.type === 'success' ? '✅ ' : t.type === 'error' ? '❌ ' : t.type === 'warning' ? '⚠️ ' : t.type === 'info' ? 'ℹ️ ' : ''}
    {t.message}
  </div>
{/each}

<style>
  @keyframes slide-up { from { transform: translateY(20px); opacity: 0; } to { transform: translateY(0); opacity: 1; } }
  .animate-slide-up { animation: slide-up 0.3s ease-out; }
</style>
