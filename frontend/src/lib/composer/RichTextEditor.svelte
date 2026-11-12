<script lang="ts">
  import { plainText } from './post-text';
  import { byteCount } from './platforms';
  import { onMount, untrack } from 'svelte';
  import { createEditor, EditorContent } from 'svelte-tiptap';
  import type { Editor } from 'svelte-tiptap';
  import StarterKit from '@tiptap/starter-kit';
  import Placeholder from '@tiptap/extension-placeholder';
  import Link from '@tiptap/extension-link';
  import { ImageExtension } from './extensions/ImageExtension';
  import LinkEditor from './extensions/LinkEditor.svelte';
  import EmojiPicker from './extensions/EmojiPicker.svelte';
  import MentionPicker from './extensions/MentionPicker.svelte';
  import SignatureEditor from './SignatureEditor.svelte';

  let { content = "", placeholder = "Write your post...", onUpdate, integrationId }: {
    /**
     * The post body, as PLAIN TEXT.
     *
     * The editor used to take and return HTML, while the server stores
     * plain text (the providers publish plain text) and the server
     * converts. So the two disagreed at every boundary:
     *
     *   - the composer sent HTML and the server stripped it;
     *   - the composer read the stored PLAIN TEXT back - on duplicate,
     *     on edit, and from the autosaved draft - and handed it to the
     *     editor, which parses its input as HTML. TipTap then treated
     *     the text's `<` as the start of a tag and the result was
     *     corrupted: a post whose text was `---` came back as `--`,
     *     and a mention of `<div>` was parsed as an element.
     *
     * The editor now speaks plain text at its boundary, the same
     * contract the server and the providers use. The rich text is an
     * EDITING convenience; the post body is text.
     */
    content?: string; placeholder?: string;
    /** Called with the new body, as plain text. */
    onUpdate?: (text: string) => void;
    /** v26-5: integration ID for @mention suggestions. Optional - if omitted,
     * the MentionPicker won't activate. */
    integrationId?: string;
  } = $props();

  let editor = $state<Editor | null>(null);
  let charCount = $state(0);
  let showLinkEditor = $state(false);
  let showEmojiPicker = $state(false);
  let showImageInput = $state(false);
  let imageUrl = $state('');
  let syncing = false;
  /**
   * The plain text this editor last emitted.
   *
   * The sync effect compares the incoming `content` with the editor's
   * current HTML. Once the contract is plain text, that comparison is
   * between two different languages - so the guard is "is this the text
   * I just emitted?". If it is, the editor already shows it and
   * re-setting the content would reset the cursor to the start of the
   * document on every keystroke.
   */
  let lastEmitted = $state<string | null>(null);

  /**
   * Plain text -> the HTML TipTap edits.
   *
   * The text is ESCAPED, so a `<` in the text stays a `<` in the
   * document instead of opening an element. Blank lines become
   * paragraphs, so the author's line breaks survive the round trip.
   */
  function textToHtml(text: string): string {
    if (!text) return '';
    const escape = (s: string) =>
      s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    return text
      .split(/\n{2,}/)
      .map((block) => `<p>${escape(block).replace(/\n/g, '<br>')}</p>`)
      .join('');
  }


  // Note: Placeholder config is captured once at init - TipTap doesn't support reactive placeholder updates
  const extensions = [
    // StarterKit already bundles Link; disabled here so the explicit configure below is the only one.
    StarterKit.configure({ heading: { levels: [1, 2, 3] }, link: false }),
    Placeholder.configure({ placeholder: untrack(() => placeholder) }),
    Link.configure({
      openOnClick: false,
      HTMLAttributes: {
        rel: 'noopener noreferrer nofollow',
      },
    }),
    ImageExtension,
  ];

  function toggleLinkEditor() {
    showLinkEditor = !showLinkEditor;
    showEmojiPicker = false;
    showImageInput = false;
  }

  function toggleEmojiPicker() {
    showEmojiPicker = !showEmojiPicker;
    showLinkEditor = false;
    showImageInput = false;
  }

  function toggleImageInput() {
    showImageInput = !showImageInput;
    showLinkEditor = false;
    showEmojiPicker = false;
    if (!showImageInput) {
      imageUrl = '';
    }
  }

  // Guarded by {#if ... && editor} in the template - editor is always available when UI is visible
  function insertImage() {
    if (!editor || !imageUrl.trim()) return;
    editor.chain().focus().setImage({ src: imageUrl.trim() }).run();
    imageUrl = '';
    showImageInput = false;
  }

  // Guarded by {#if ... && editor} in the template - editor is always available when UI is visible
  function insertEmoji(emoji: string) {
    if (!editor) return;
    editor.chain().focus().insertContent(emoji).run();
  }

  function closeImageInput() {
    showImageInput = false;
    imageUrl = '';
  }

  function handleImageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      insertImage();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeImageInput();
    }
  }

  function handleImageBackdropClick(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('img-input-backdrop')) {
      closeImageInput();
    }
  }

  onMount(() => {
    const store = createEditor({
      extensions,
      content: textToHtml(content),
      onUpdate: ({ editor: ed }) => {
        if (syncing) return;
        // The SHARED conversion — the same one the server applies and the
        // same one the previews measure with. The editor used to carry its
        // own copy (a second HTML-to-text converter), and the copies
        // drifted: the editor counted one string and the server stored
        // another (AGENTS §9.18).
        const text = plainText(ed.getHTML());
        lastEmitted = text;
        onUpdate?.(text);
        charCount = byteCount(ed.getText());
      },
    });

    const unsub = store.subscribe(val => {
      editor = val;
      if (val) charCount = byteCount(val.getText());
    });

    return () => {
      unsub();
      editor?.destroy();
    };
  });

  // Sync external content changes into the editor (guarded to avoid loops)
  $effect(() => {
    if (editor && content && content !== lastEmitted && !syncing) {
      syncing = true;
      editor.commands.setContent(textToHtml(content));
      syncing = false;
    }
  });
</script>

<div class="rich-editor border border-line rounded-lg overflow-hidden bg-background-input relative">
  <div class="flex items-center gap-1 p-2 border-b border-line flex-wrap">
    <button aria-label="Bold" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleBold().run(); }} class="toolbar-btn min-h-11" class:active={editor?.isActive("bold")}>B</button>
    <button aria-label="Italic" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleItalic().run(); }} class="toolbar-btn italic min-h-11" class:active={editor?.isActive("italic")}>I</button>
    <!-- v24-6: Underline button (TipTap 3.x includes Underline in StarterKit) -->
    <button aria-label="Underline" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleUnderline().run(); }} class="toolbar-btn underline min-h-11" class:active={editor?.isActive("underline")}>U</button>
    <button aria-label="Heading 2" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleHeading({ level: 2 }).run(); }} class="toolbar-btn min-h-11" class:active={editor?.isActive("heading", { level: 2 })}>H2</button>
    <button aria-label="Bullet list" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleBulletList().run(); }} class="toolbar-btn min-h-11" class:active={editor?.isActive("bulletList")}>• List</button>
    <button aria-label="Ordered list" onmousedown={(e) => { e.preventDefault(); editor?.chain().focus().toggleOrderedList().run(); }} class="toolbar-btn min-h-11" class:active={editor?.isActive("orderedList")}>1. List</button>
    <span class="text-line-hover mx-1" aria-hidden="true">|</span>
    <button aria-label="Insert image" onmousedown={(e) => { e.preventDefault(); toggleImageInput(); }} class="toolbar-btn min-h-11" class:active={showImageInput}>🖼️</button>
    <button aria-label="Insert or edit link" onmousedown={(e) => { e.preventDefault(); toggleLinkEditor(); }} class="toolbar-btn min-h-11" class:active={showLinkEditor || editor?.isActive("link")}>🔗</button>
    <button aria-label="Insert emoji" onmousedown={(e) => { e.preventDefault(); toggleEmojiPicker(); }} class="toolbar-btn min-h-11" class:active={showEmojiPicker}>😊</button>
    <span class="text-line-hover mx-1" aria-hidden="true">|</span>
    <SignatureEditor onInsert={(content) => editor?.chain().focus().insertContent(content).run()} />

    <span class="text-xs text-muted ml-auto">{charCount} chars</span>
  </div>

  {#if showImageInput}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="img-input-backdrop fixed inset-0 z-40" role="none" onclick={handleImageBackdropClick}>
      <div
        class="img-input-popover"
        role="dialog"
        aria-label="Insert image URL"
        tabindex="-1"
        onclick={(e) => e.stopPropagation()}
      >
        <label for="img-url" class="block text-xs text-muted mb-1">Image URL</label>
        <div class="flex items-center gap-2">
          <input
            id="img-url"
            type="text"
            placeholder="https://example.com/image.png"
            bind:value={imageUrl}
            onkeydown={handleImageKeydown}
            class="min-h-11 flex-1 px-3 py-2 rounded text-sm bg-background-input border border-line text-content-secondary placeholder:text-faint outline-none focus:border-accent transition-colors"
          />
          <button
            onclick={insertImage}
            disabled={!imageUrl.trim()}
            class="px-3 py-2 rounded text-xs font-medium bg-accent-fill text-accent-fg hover:bg-accent-fill-hover disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            aria-label="Insert image"
          >
            Insert
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if showLinkEditor && editor}
    <LinkEditor {editor} onClose={() => showLinkEditor = false} />
  {/if}

  {#if showEmojiPicker}
    <EmojiPicker onSelect={insertEmoji} onClose={() => showEmojiPicker = false} />
  {/if}

  <!-- v26-5: @mention suggestion popup. Renders above everything; positions
       itself at the cursor via fixed positioning. Only active when
       integrationId is provided. -->
  <MentionPicker {editor} {integrationId} />

  <div class="editor-body p-3 min-h-[200px]">
    {#if editor}
      <EditorContent {editor} />
    {/if}
  </div>
</div>

<style>
  /* v24-6: use CSS variables so the editor rethemes in light mode. */
  .toolbar-btn {
    /* A 44px-tall target. The button is 26x29 at the old padding, which
       is under the WCAG 2.2 minimum. The extra height comes from the
       padding, so the GLYPH stays the same size and the row does not
       grow - the buttons simply have room to be hit. */
    min-height: 44px; min-width: 44px;
    padding: 0.25rem 0.5rem; font-size: 0.8rem; background: transparent;
    border: 1px solid transparent; border-radius: 0.25rem; cursor: pointer;
    color: var(--text-muted); font-weight: 500;
    display: inline-flex; align-items: center; justify-content: center;
  }
  .toolbar-btn:hover { background: var(--bg-hover); color: var(--text-secondary); }
  .toolbar-btn.active { background: var(--brand); color: white; border-color: var(--brand); }
  .italic { font-style: italic; }
  .underline { text-decoration: underline; }

  .img-input-popover {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    padding: 0.875rem;
    /* v25 F5: was a bare `width: 24rem` (384px). The popover is absolutely
       positioned and centred, and nothing between it and the composer's
       `fixed` overlay clips - so on a 360px phone this was the single widest
       element in the app and the one thing that could scroll the whole page
       sideways. `min()` keeps the desktop size and yields to the viewport. */
    width: min(24rem, calc(100vw - 2rem));
    box-shadow: var(--shadow-lg);
    z-index: 50;
  }

  .img-input-backdrop {
    background: var(--overlay);
  }

  /* v25 F1: the editor body was carrying `prose prose-invert`, but
     @tailwindcss/typography is not installed - both classes compile to nothing.
     The content was therefore rendering with browser-default styling: black text
     on the dark `--bg-input` surface, i.e. unreadable in dark mode. These rules
     replace the missing plugin with a scoped, token-driven equivalent: the post
     body retheme correctly and headings/lists/links/blockquote all get the
     contrast they need in either theme.
     Scoped with :global() because ProseMirror injects its own DOM. */
  .editor-body :global(.tiptap) {
    color: var(--text);
    font-size: 0.9375rem;
    line-height: 1.65;
    outline: none;
    min-height: 200px;
  }
  .editor-body :global(.tiptap p) { margin: 0 0 0.75rem; }
  .editor-body :global(.tiptap p:last-child) { margin-bottom: 0; }
  .editor-body :global(.tiptap h1),
  .editor-body :global(.tiptap h2),
  .editor-body :global(.tiptap h3) {
    color: var(--text);
    font-weight: 600;
    line-height: 1.3;
    margin: 1rem 0 0.5rem;
  }
  .editor-body :global(.tiptap h1) { font-size: 1.5rem; }
  .editor-body :global(.tiptap h2) { font-size: 1.25rem; }
  .editor-body :global(.tiptap h3) { font-size: 1.0625rem; }
  .editor-body :global(.tiptap ul),
  .editor-body :global(.tiptap ol) { margin: 0 0 0.75rem; padding-left: 1.5rem; }
  .editor-body :global(.tiptap ul) { list-style: disc; }
  .editor-body :global(.tiptap ol) { list-style: decimal; }
  .editor-body :global(.tiptap li) { margin-bottom: 0.25rem; }
  .editor-body :global(.tiptap a) { color: var(--brand); text-decoration: underline; }
  .editor-body :global(.tiptap blockquote) {
    border-left: 3px solid var(--brand);
    padding-left: 0.875rem;
    margin: 0 0 0.75rem;
    color: var(--text-secondary);
  }
  .editor-body :global(.tiptap code) {
    background: var(--bg-hover);
    color: var(--text);
    padding: 0.125rem 0.35rem;
    border-radius: 0.25rem;
    font-size: 0.875em;
  }
  .editor-body :global(.tiptap pre) {
    background: var(--bg-hover);
    border: 1px solid var(--border);
    padding: 0.75rem;
    border-radius: var(--radius-md);
    margin: 0 0 0.75rem;
    overflow-x: auto;
  }
  .editor-body :global(.tiptap pre code) { background: transparent; padding: 0; }
  .editor-body :global(.tiptap img) { max-width: 100%; border-radius: var(--radius-md); }
  .editor-body :global(.tiptap hr) { border: none; border-top: 1px solid var(--border); margin: 1rem 0; }
  /* TipTap's Placeholder extension renders this via a data attribute on the
     first empty node. It needs a real token too, or the prompt is invisible on
     a light surface. */
  .editor-body :global(.tiptap p.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    color: var(--text-faint);
    float: left;
    height: 0;
    pointer-events: none;
  }
  .editor-body :global(.tiptap-focused) { outline: none; }
  /* The editor surface drops its outline, so show the ring from the keyboard
     focus state of the editable inside it. */
  :global(.editor-body:has(:focus-visible)) {
    box-shadow: 0 0 0 var(--focus-ring-width) var(--focus-ring);
    border-radius: 0.5rem;
  }
</style>
