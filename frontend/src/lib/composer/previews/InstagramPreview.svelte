<script lang="ts">
  // InstagramPreview - IG-style preview card (Phase 4).
  //
  // Distinctive IG chrome:
  //   - Avatar + name + "•••" menu in header
  //   - Square image area (or carousel if multiple media)
  //   - Heart / Comment / Share / Save icon row
  //   - Caption with mention highlighting below the image
  //
  // Inspired by postiz-app's providers/instagram/instagram.preview.tsx.

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
  let charLimit = $derived(charLimitFor('instagram'));
  let previewCrop = $derived(cropForPreview('instagram', plainText, { accent: '#13315c' }));
  let isOverLimit = $derived(previewCrop.over);
  let previewCount = $derived(previewCrop.count);

  let highlightedHtml = $derived(
    cropForPreview('instagram', plainText, { accent: '#13315c' }).html
  );
</script>

<div class="bg-white text-black rounded-xl overflow-hidden max-w-[400px] mx-auto" style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;">
  <!-- Header: avatar + name + menu -->
  <div class="flex items-center gap-2 p-3">
    <div class="w-8 h-8 rounded-full p-0.5 bg-gradient-to-br from-yellow-400 via-pink-500 to-purple-600">
      {#if authorAvatar}
        <img src={authorAvatar} alt={authorName} class="w-full h-full rounded-full object-cover border-2 border-white" />
      {:else}
        <div class="w-full h-full rounded-full bg-white flex items-center justify-center text-pink-500 font-bold text-xs">
          {authorName.charAt(0).toUpperCase()}
        </div>
      {/if}
    </div>
    <span class="text-sm font-semibold flex-1 truncate">{authorHandle || authorName.toLowerCase().replace(/\s/g, '')}</span>
    <span class="text-black text-lg">•••</span>
  </div>

  <!-- Square image area -->
  {#if media.length > 0}
    <div class="relative aspect-square bg-gray-100">
      {#if media[0].mime_type.startsWith('image/')}
        <img src={media[0].url} alt="" class="w-full h-full object-cover" />
      {:else}
        <div class="w-full h-full flex items-center justify-center text-gray-500 text-sm">video</div>
      {/if}
      {#if media.length > 1}
        <div class="absolute top-2 right-2 bg-black/60 text-white text-[10px] px-1.5 py-0.5 rounded">
          1/{media.length}
        </div>
      {/if}
    </div>
  {:else}
    <div class="aspect-square bg-gray-100 flex items-center justify-center text-gray-600 text-sm border border-gray-200">
      No image - IG posts require media
    </div>
  {/if}

  <!-- Action row -->
  <div class="flex items-center gap-4 px-3 py-2">
    <svg class="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>
    <svg class="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>
    <svg class="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
    <svg class="w-6 h-6 ml-auto" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/></svg>
  </div>

  <!-- Caption -->
  <div class="px-3 pb-3 text-sm">
    <span class="font-semibold">{authorHandle || authorName.toLowerCase().replace(/\s/g, '')}</span>
    <span class="ml-1">{@html highlightedHtml}</span>
    {#if isOverLimit}
      <p class="text-[10px] text-red-500 mt-1">{previewCount}/{charLimit} - text beyond {charLimit} will be cropped</p>
    {/if}
  </div>
</div>
