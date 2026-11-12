<script lang="ts">
  // GeneralPreview - default Twitter/X-like preview card (Phase 4).
  //
  // This is the fallback preview used when no platform-specific preview
  // exists, and also the preview shown when the user is in 'global'
  // editing mode (no specific channel selected).
  //
  // Inspired by postiz-app's general.preview.component.tsx:
  //   - Avatar + name + verified badge + @handle
  //   - Content with mention (@user) and hashtag (#tag) highlighting
  //   - Character-crop: text beyond the platform's limit is wrapped in
  //     <mark class="bg-red-500/30"> with title "This text will be cropped"
  //   - Media grid: 1 image full-width, 2-3 side by side, 4+ in 2-col grid
  //
  // Security: all content is rendered via {@html} after escaping, so
  // user input cannot inject HTML. Mentions/hashtags are wrapped in
  // <span> tags with explicit colors.

  import { providerMeta } from '$lib/providers';
  import { plainText as plainTextOf, charLimitFor } from '$lib/composer/platforms';
  import { cropForPreview } from './crop';
  import type { MediaItem } from '$lib/api/media';

  let {
    content = '',
    provider = 'x',
    authorName = 'Your Brand',
    authorHandle = 'yourbrand',
    authorAvatar = '',
    media = [] as MediaItem[],
  }: {
    content?: string;
    provider?: string;
    authorName?: string;
    authorHandle?: string;
    authorAvatar?: string;
    media?: MediaItem[];
  } = $props();

  // Strip HTML tags to get plain text for the preview.
  // The content is already plain text (the editor speaks plain text
  // at its boundary, the same contract the server and the providers use).
  // The strip is kept as a safety net for content that arrived from
  // somewhere else, and it is the SHARED helper - it used to be a
  // per-file copy of the same regex.
  let plainText = $derived(plainTextOf(content));

  // The provider's rule, measured by the shared counter (not `length`, which
  // counts UTF-16 code units) and cropped on the PLAIN text (not the escaped
  // string) — see $lib/composer/preview-crop.
  let charLimit = $derived(charLimitFor(provider));

  // Highlight mentions, hashtags and URLs in the plain text.
  // Returns HTML with <span> wrappers for colored text.
  let highlightedHtml = $derived(
    cropForPreview(provider, plainText, {
      accent: '#a855f7',
      highlightUrls: true,
      urlAccent: '#6366f1',
      markerTitle: 'This text will be cropped',
    }).html
  );

  let overCount = $derived(cropForPreview(provider, plainText, { accent: '#a855f7' }));

  // Media grid layout: 1=full, 2=side-by-side, 3=side-by-side, 4+=2-col grid
  let mediaGridClass = $derived(
    media.length === 1 ? 'grid-cols-1' :
    media.length === 2 ? 'grid-cols-2' :
    media.length === 3 ? 'grid-cols-3' :
    'grid-cols-2'
  );
</script>

<div class="bg-surface border border-line rounded-xl overflow-hidden">
  <!-- Header: avatar + name + handle -->
  <div class="flex items-start gap-3 p-4">
    {#if authorAvatar}
      <img src={authorAvatar} alt={authorName} class="w-10 h-10 rounded-full object-cover flex-shrink-0" />
    {:else}
      <div class="w-10 h-10 rounded-full bg-indigo-500/20 flex items-center justify-center text-indigo-400 font-bold text-sm flex-shrink-0">
        {authorName.charAt(0).toUpperCase()}
      </div>
    {/if}
    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-1">
        <span class="text-sm font-semibold text-content truncate">{authorName}</span>
        <svg class="w-4 h-4 text-indigo-400 flex-shrink-0" viewBox="0 0 24 24" fill="currentColor">
          <path d="M22.5 12.5c0-1.58-.875-2.95-2.148-3.6.154-.435.238-.905.238-1.4 0-2.21-1.71-3.998-3.818-3.998-.47 0-.92.084-1.336.25C14.818 2.415 13.51 1.5 12 1.5s-2.816.917-3.437 2.25c-.415-.165-.866-.25-1.336-.25-2.11 0-3.818 1.79-3.818 4 0 .494.083.964.237 1.4-1.272.65-2.147 2.018-2.147 3.6 0 1.495.782 2.798 1.942 3.486-.02.17-.032.34-.032.514 0 2.21 1.708 4 3.818 4 .47 0 .92-.086 1.335-.25.62 1.334 1.926 2.25 3.437 2.25 1.512 0 2.818-.916 3.437-2.25.415.163.865.248 1.336.248 2.11 0 3.818-1.79 3.818-4 0-.174-.012-.344-.033-.513 1.16-.687 1.943-1.99 1.943-3.484zm-6.616-3.334l-4.334 6.5c-.145.217-.382.334-.625.334-.143 0-.288-.04-.416-.126l-.115-.094-2.415-2.415c-.293-.293-.293-.768 0-1.06s.768-.294 1.06 0l1.77 1.767 3.825-5.74c.23-.345.696-.436 1.04-.207.346.23.44.696.21 1.04z"/>
        </svg>
        <span class="text-muted text-sm">@{authorHandle}</span>
        <span class="text-muted text-sm ml-auto text-xs">{providerMeta(provider).label}</span>
      </div>
    </div>
  </div>

  <!-- Content -->
  <div class="px-4 pb-3">
    <p class="text-sm text-content whitespace-pre-wrap break-words leading-relaxed">{@html highlightedHtml}</p>
    {#if overCount.over}
      <p class="text-[10px] text-red-400 mt-1">{overCount.count}/{overCount.limit} - text beyond {overCount.limit} will be cropped</p>
    {/if}
  </div>

  <!-- Media grid -->
  {#if media.length > 0}
    <div class="grid {mediaGridClass} gap-0.5 border-t border-line">
      {#each media.slice(0, 4) as item (item.id)}
        {#if item.mime_type.startsWith('image/')}
          <img src={item.url} alt={item.original_name || ''} class="w-full aspect-square object-cover" />
        {:else}
          <div class="w-full aspect-square bg-surface-hover flex items-center justify-center text-muted text-xs">
            video
          </div>
        {/if}
      {/each}
    </div>
    {#if media.length > 4}
      <p class="text-[10px] text-muted px-4 py-1">+{media.length - 4} more</p>
    {/if}
  {/if}

  <!-- Engagement icons row -->
  <div class="flex items-center justify-between px-4 py-2 border-t border-line text-muted text-xs">
    <span class="flex items-center gap-1">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>
    </span>
    <span class="flex items-center gap-1">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></svg>
    </span>
    <span class="flex items-center gap-1">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></svg>
    </span>
    <span class="flex items-center gap-1">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/></svg>
    </span>
  </div>
</div>
