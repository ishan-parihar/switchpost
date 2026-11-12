<script lang="ts">
  // FacebookPreview - Facebook-style preview card (Phase 4).
  //
  // Distinctive FB chrome:
  //   - Avatar + name + "•••" menu + timestamp
  //   - Content (no visual char limit, but crop highlighted)
  //   - Like / Comment / Share row
  //
  // Inspired by postiz-app's providers/facebook/facebook.preview.tsx.

  import { providerMeta } from '$lib/providers';
  import { plainText as plainTextOf, charLimitFor } from '$lib/composer/platforms';
  import { cropForPreview } from './crop';
  import type { MediaItem } from '$lib/api/media';

  let {
    content = '',
    authorName = 'Your Brand',
    authorHandle = 'yourbrand',
    authorAvatar = '',
    media = [] as MediaItem[],
  }: {
    content?: string;
    authorName?: string;
    authorHandle?: string;
    authorAvatar?: string;
    media?: MediaItem[];
  } = $props();

  // The content is already plain text (the editor speaks plain text
  // at its boundary, the same contract the server and the providers use).
  // The strip is kept as a safety net for content that arrived from
  // somewhere else, and it is the SHARED helper - it used to be a
  // per-file copy of the same regex.
  let plainText = $derived(plainTextOf(content));
  // The provider's rule, measured by the shared counter (not `length`, which
  // counts UTF-16 code units) and cropped on the PLAIN text (not the escaped
  // string) — see $lib/composer/preview-crop.
  let charLimit = $derived(charLimitFor('facebook'));
  let previewCrop = $derived(cropForPreview('facebook', plainText, { accent: '#1877f2' }));
  let isOverLimit = $derived(previewCrop.over);
  let previewCount = $derived(previewCrop.count);

  let highlightedHtml = $derived(
    cropForPreview('facebook', plainText, { accent: '#1877f2' }).html
  );
</script>

<div class="bg-white text-black rounded-xl overflow-hidden max-w-[500px] mx-auto border border-gray-200" style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;">
  <!-- Header -->
  <div class="flex items-start gap-2 p-3">
    {#if authorAvatar}
      <img src={authorAvatar} alt={authorName} class="w-10 h-10 rounded-full object-cover flex-shrink-0" />
    {:else}
      <div class="w-10 h-10 rounded-full bg-blue-600 flex items-center justify-center text-white font-bold flex-shrink-0">
        {authorName.charAt(0).toUpperCase()}
      </div>
    {/if}
    <div class="flex-1 min-w-0">
      <div class="text-sm font-semibold text-black">{authorName}</div>
      <div class="text-xs text-gray-500">Just now · 🌐</div>
    </div>
    <span class="text-gray-500 text-lg">•••</span>
  </div>

  <!-- Content -->
  <div class="px-3 pb-3">
    <p class="text-sm text-gray-800 whitespace-pre-wrap break-words leading-relaxed">{@html highlightedHtml}</p>
    {#if isOverLimit}
      <p class="text-[10px] text-red-500 mt-1">{previewCount}/{charLimit} - text beyond {charLimit} will be cropped</p>
    {/if}
  </div>

  <!-- Media -->
  {#if media.length > 0}
    <div class="border-t border-gray-200">
      {#if media[0].mime_type.startsWith('image/')}
        <img src={media[0].url} alt="" class="w-full max-h-[300px] object-cover" />
      {:else}
        <div class="w-full h-[200px] bg-gray-100 flex items-center justify-center text-gray-600 text-sm">video</div>
      {/if}
    </div>
  {/if}

  <!-- Likes count -->
  <div class="flex items-center justify-between px-3 py-2 border-t border-gray-200 text-xs text-gray-500">
    <div class="flex items-center gap-1">
      <span class="w-4 h-4 rounded-full bg-blue-600 flex items-center justify-center text-white text-[8px]">👍</span>
      <span>8</span>
    </div>
    <span>2 comments</span>
  </div>

  <!-- Action buttons -->
  <div class="flex items-center justify-around border-t border-gray-200 py-2 text-xs text-gray-600 font-medium">
    <span class="flex items-center gap-1 hover:bg-gray-100 px-3 py-1 rounded cursor-default">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/></svg>
      Like
    </span>
    <span class="flex items-center gap-1 hover:bg-gray-100 px-3 py-1 rounded cursor-default">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>
      Comment
    </span>
    <span class="flex items-center gap-1 hover:bg-gray-100 px-3 py-1 rounded cursor-default">
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></svg>
      Share
    </span>
  </div>
</div>
