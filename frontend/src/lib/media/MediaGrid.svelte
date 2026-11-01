<script lang="ts">
  // v25 F4 - media grid keyboard + a11y pass.
  //
  // What was wrong before:
  //  - File-type placeholders were emoji. Emoji render at different sizes per
  //    platform, ignore `currentColor` so they do not retheme, and read as
  //    noise next to the app's line-icon set. The repo already has
  //    $lib/ui/Icon.svelte for exactly this.
  //  - The tile was a `<div role="button">`. That is announced as a button but
  //    is not one: no native Enter/Space handling, no pressed state, and a
  //    delete button nested inside a button, which is invalid HTML and makes
  //    the tab order ambiguous.
  //  - The delete control was 24x24 revealed only on hover. Unreachable by
  //    keyboard, invisible on touch, and below the 44px minimum target. The
  //    two-step confirm was hover-only too, so a keyboard user could neither
  //    complete nor cancel it.
  //  - No list semantics, so a screen reader never announced an item count.
  import type { MediaItem } from "$lib/api/media";
  import Icon from "$lib/ui/Icon.svelte";

  let {
    items = [],
    loading = false,
    selectable = false,
    selectedIds = [],
    onSelect,
    onDelete,
  }: {
    items?: MediaItem[];
    loading?: boolean;
    selectable?: boolean;
    selectedIds?: string[];
    onSelect?: (item: MediaItem) => void;
    onDelete?: (id: string) => void;
  } = $props();

  let deleting = $state<string | null>(null);

  $effect(() => {
    if (deleting !== null) {
      const onClick = (e: MouseEvent) => {
        // Only close when the click landed outside the tile, so a click on the
        // confirm button itself is not immediately undone.
        const target = e.target as HTMLElement | null;
        if (!target?.closest("[data-media-tile]")) deleting = null;
      };
      // v25 F4: Escape lives on the document while the confirm is armed rather
      // than on the trigger button. The armed trigger is hidden (the overlay
      // owns the confirm), so focus may be anywhere, and a key handler on one
      // element only fires when that element holds focus.
      const onKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") deleting = null;
      };
      document.addEventListener("click", onClick);
      document.addEventListener("keydown", onKey);
      return () => {
        document.removeEventListener("click", onClick);
        document.removeEventListener("keydown", onKey);
      };
    }
  });

  function confirmDelete(id: string) {
    if (deleting === id) {
      onDelete?.(id);
      deleting = null;
    } else {
      deleting = id;
    }
  }

  function cancelDelete() {
    deleting = null;
  }

  function isSelected(id: string) {
    return selectedIds.includes(id);
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + " KB";
    return (bytes / 1048576).toFixed(1) + " MB";
  }

  function formatDate(iso: string | undefined): string {
    if (!iso) return "";
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const days = Math.floor(diff / 86400000);
    if (days === 0) return "Today";
    if (days === 1) return "Yesterday";
    if (days < 7) return `${days} days ago`;
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  /** Line-icon name per file kind. Replaces the emoji set. */
  function fileIcon(mime: string): string {
    if (mime.startsWith("video/")) return "media";
    if (mime.startsWith("audio/")) return "comment-bubble";
    if (mime.includes("pdf")) return "post";
    return "inbox";
  }

  function kindLabel(mime: string): string {
    if (mime.startsWith("image/")) return "Image";
    if (mime.startsWith("video/")) return "Video";
    if (mime.startsWith("audio/")) return "Audio";
    if (mime.includes("pdf")) return "PDF";
    return "File";
  }
</script>

<!--
  Svelte 5 snippet: the tile body is shared by the selectable (<button>) and
  browse (plain container) variants so the two can never drift visually.
-->
{#snippet TileBody(item: MediaItem)}
  {#if item.mime_type.startsWith("image/")}
    <img
      src={item.url}
      alt={item.original_name}
      class="w-full aspect-square object-cover"
      loading="lazy"
    />
  {:else}
    <div class="w-full aspect-square bg-surface-hover flex items-center justify-center text-muted">
      <Icon name={fileIcon(item.mime_type)} class="w-8 h-8" />
    </div>
  {/if}

  <div class="p-2">
    <p class="text-xs text-content-secondary truncate" title={item.original_name}>{item.original_name}</p>
    <p class="text-[10px] text-muted">
      {kindLabel(item.mime_type)} &middot; {formatSize(item.file_size)} &middot; {formatDate(item.created_at)}
    </p>
  </div>
{/snippet}

{#if loading}
  <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3" aria-busy="true" aria-label="Loading media">
    {#each Array(10) as _, i (i)}
      <div class="bg-surface border border-line rounded-xl overflow-hidden animate-pulse">
        <div class="w-full aspect-square bg-surface-hover"></div>
        <div class="p-2 space-y-1.5">
          <div class="h-3 bg-surface-hover rounded w-3/4"></div>
          <div class="h-2.5 bg-surface-hover rounded w-1/2"></div>
        </div>
      </div>
    {/each}
  </div>
{:else if items.length === 0}
  <div class="text-center py-16 text-sm text-muted">
    <div class="inline-flex items-center justify-center w-12 h-12 rounded-full bg-surface-hover mb-3">
      <Icon name="inbox" class="w-5 h-5 text-muted" />
    </div>
    <p>No media uploaded yet</p>
  </div>
{:else}
  <!--
    `group` sits on the tile so `group-hover` can reveal the delete control.
    `focus-visible:opacity-100` is what makes it reachable by keyboard: tabbing
    to the button is equivalent to hovering the tile.
  -->
  <ul
    class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3"
    aria-label="Media library, {items.length} items"
  >
    {#each items as item (item.id)}
      <li
        data-media-tile
        class="relative group bg-surface border rounded-xl overflow-hidden transition-all duration-150"
        class:border-accent={selectable && isSelected(item.id)}
        class:border-line={!selectable || !isSelected(item.id)}
        class:ring-1={selectable && isSelected(item.id)}
        class:ring-accent={selectable && isSelected(item.id)}
      >
        <!--
          In selectable mode the tile IS a control, so it is a real <button>:
          native Enter/Space, a real focus ring, and a pressed state exposed via
          aria-pressed. In browse mode it stays a plain container, because a
          clickable div with no behaviour is a lie to assistive tech.
        -->
        {#if selectable}
          <button
            type="button"
            aria-pressed={isSelected(item.id)}
            aria-label="{isSelected(item.id) ? 'Deselect' : 'Select'} {item.original_name}"
            onclick={() => onSelect?.(item)}
            class="block w-full text-left cursor-pointer"
          >
            {@render TileBody(item)}
          </button>
        {:else}
          {@render TileBody(item)}
        {/if}

        {#if selectable && isSelected(item.id)}
          <span class="absolute top-1.5 left-1.5 w-5 h-5 bg-accent-fill rounded-full flex items-center justify-center pointer-events-none" aria-hidden="true">
            <Icon name="check" class="w-3 h-3 text-accent-fg" />
          </span>
        {/if}

        {#if onDelete}
          <!--
            44x44 hit area from padding around a visually smaller dot, so the
            target meets the touch minimum without a large red blob on every
            tile. Hidden while the confirm is armed: the overlay below owns the
            decision then, and a second confirm control reading as a tick mark
            is ambiguous.
          -->
          {#if deleting !== item.id}
            <button
              type="button"
              aria-label="Delete {item.original_name}"
              onclick={(e) => { e.stopPropagation(); confirmDelete(item.id); }}
              class="absolute top-0 right-0 w-11 h-11 p-2 flex items-start justify-end opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity rounded-bl-xl"
            >
              <span class="w-6 h-6 rounded-full flex items-center justify-center bg-error text-error-fg">
                <Icon name="close" class="w-3.5 h-3.5" />
              </span>
            </button>
          {/if}

          {#if deleting === item.id}
            <div
              class="absolute inset-0 bg-overlay flex flex-col items-center justify-center gap-2 p-3 text-center"
              role="alertdialog"
              aria-label="Confirm deleting {item.original_name}"
            >
              <p class="text-xs text-content">Delete this file?</p>
              <div class="flex gap-2">
                <button
                  type="button"
                  onclick={() => confirmDelete(item.id)}
                  class="px-3 py-1.5 text-xs font-medium rounded-md bg-error/25 text-content border border-error/50 hover:bg-error/35"
                >
                  Delete
                </button>
                <button
                  type="button"
                  onclick={cancelDelete}
                  class="min-h-11 px-3 py-1.5 text-xs rounded-md bg-surface-hover text-content border border-line"
                >
                  Cancel
                </button>
              </div>
            </div>
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
{/if}
