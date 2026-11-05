<script lang="ts">
  // PerPlatformCharCount - the composer's per-channel character rings (v25 F2).
  //
  // F2 replaced the flat text badges with a ring per platform. A badge that
  // says "248/280" is a number the user has to mentally compare; a ring is the
  // comparison already drawn. It also removes the "am I nearly done?" question
  // for the platforms with a very different budget (X 4000, Facebook 63206) -
  // the ring is 98% full for X and 0.4% full for Facebook at the same moment,
  // and that difference is the entire point of a multi-platform composer.
  //
  // The counting itself moved to ./platforms so this component, the blocking
  // banner in ComposerModal, and the previews cannot disagree about whether an
  // emoji costs 1 or 2. X uses weighted length; everything else uses plain.
  //
  // Colors come from the F1 token layer (app.css → tailwind `stroke-*`), so the
  // ring rethemes with the rest of the app in light and dark.

  import { charLimitFor, countFor, plainText, specFor } from './platforms';

  let { content, selectedIntegrations, integrationProviders, integrationNames }: {
    content: string;
    selectedIntegrations: string[];
    integrationProviders: Map<string, string>;
    integrationNames: Map<string, string>;
  } = $props();

  // Plain-text length of the content (HTML tags stripped).
  let text = $derived(plainText(content));

  const R = 14;
  const C = 2 * Math.PI * R;

  interface Ring {
    key: string;
    label: string;
    account: string;
    limit: number;
    count: number;
    isOver: boolean;
    isWarning: boolean;
    /** Remaining characters, floored at 0. */
    left: number;
    /** Stroke-dashoffset, i.e. how much of the ring is still empty. */
    offset: number;
  }

  let rings = $derived.by(() => {
    const seen = new Set<string>();
    const out: Ring[] = [];
    for (const intId of selectedIntegrations) {
      const provider = integrationProviders.get(intId);
      if (!provider) continue;
      // Two accounts on the same platform share a limit, so one ring covers
      // both; the account name is listed so neither feels unrepresented.
      if (seen.has(provider)) continue;
      seen.add(provider);

      const limit = charLimitFor(provider);
      const count = countFor(provider, text);
      const ratio = limit > 0 ? Math.min(count / limit, 1) : 0;
      out.push({
        key: provider,
        label: specFor(provider)?.label ?? provider,
        account: integrationNames.get(intId) || '',
        limit,
        count,
        isOver: count > limit,
        isWarning: count > limit * 0.9 && count <= limit,
        left: Math.max(limit - count, 0),
        offset: C * (1 - ratio),
      });
    }
    return out;
  });

  let overCount = $derived(rings.filter(r => r.isOver).length);
  /** X's weighted count means the ring's number is not String.length. */
  function countNote(label: string): string {
    return label === 'X' ? 'weighted' : 'chars';
  }
</script>

{#if rings.length > 0}
  <div class="space-y-2">
    <div class="flex flex-wrap gap-2">
      {#each rings as r (r.key)}
        <div
          class="flex items-center gap-2 pl-1.5 pr-2.5 py-1 rounded-lg border
            {r.isOver
              ? 'border-error/40 bg-error/10'
              : r.isWarning
                ? 'border-warning/40 bg-warning/10'
                : 'border-line bg-surface-hover'}"
          title="{r.label} (weighted): {r.count} of {r.limit} {countNote(r.label)}"
        >
          <!-- Ring. rotate(-90) so 0% starts at 12 o'clock instead of 3. -->
          <svg width="30" height="30" viewBox="0 0 32 32" class="shrink-0 -rotate-90" aria-hidden="true">
            <circle cx="16" cy="16" r={R} fill="none" stroke-width="3" class="stroke-line" />
            <circle cx="16" cy="16" r={R} fill="none" stroke-width="3" stroke-linecap="round"
              stroke-dasharray={C}
              stroke-dashoffset={r.offset}
              class={r.isOver ? 'stroke-error' : r.isWarning ? 'stroke-warning' : 'stroke-accent'} />
          </svg>
          <div class="min-w-0 leading-tight">
            <div class="text-xs font-medium truncate max-w-[110px]">
              {r.label}{#if r.account}<span class="text-faint"> · {r.account}</span>{/if}
            </div>
            <div class="text-[10px] font-mono {r.isOver ? 'text-error' : r.isWarning ? 'text-warning' : 'text-muted'}">
              {#if r.isOver}
                {r.left === 0 ? `${r.count} - over by ${r.count - r.limit}` : `${r.count}/${r.limit}`}
              {:else}
                {r.count}/{r.limit}
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </div>

    {#if overCount > 0}
      <p class="text-[11px] text-error">
        {overCount} channel{overCount > 1 ? 's' : ''} over limit - trim the body or drop {overCount > 1 ? 'those channels' : 'that channel'} to publish.
      </p>
    {/if}
  </div>
{/if}
