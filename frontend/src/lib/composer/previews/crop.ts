// ─── The preview crop (v12) ─────────────────────────────────────
//
// Four preview panes (General, Facebook, Instagram, LinkedIn) each measured
// with `plainText.length` — UTF-16 CODE UNITS — and then sliced the ESCAPED
// string. Both halves of that were wrong:
//
//   1. the count the composer showed was not the count the server
//      enforced. A 600-emoji Instagram caption read "600/2200" while the
//      server counted 2400 bytes and rejected it (AGENTS §9.19 / §9.20).
//   2. the crop index was applied to the escaped string, so a caption of
//      `&`s (each becoming `&`, five characters) had its "will be
//      cropped" marker ~400 characters early — and a slice landing inside
//      an injected mention `<span>` closed the tag in the wrong place.
//
// The other five previews already did this right (`countFor` + slicing the
// plain text). This is the ONE helper that does it for the four that did
// not, so the crop is computed on the same text the count measured.

import { charLimitFor, countFor } from '../platforms';

export interface PreviewCropOptions {
  /** The accent colour for @mentions and #hashtags. */
  accent: string;
  /** Also highlight URLs (the General preview does). */
  highlightUrls?: boolean;
  /** The URL accent colour. */
  urlAccent?: string;
  /** The `title` on the overflow marker. */
  markerTitle?: string;
}

export interface PreviewCrop {
  /** The rendered HTML: the text, with the overflow wrapped in a marker. */
  html: string;
  /** The provider's count of the whole text. */
  count: number;
  /** The provider's limit. */
  limit: number;
  /** True when `count` is over `limit`. */
  over: boolean;
}

/**
 * Escape for HTML and highlight @mentions / #hashtags.
 *
 * Escape FIRST, then wrap: the mention pattern must run against the escaped
 * text so a mention cannot introduce a tag.
 */
export function highlightPreviewText(
  text: string,
  options: PreviewCropOptions,
): string {
  let out = text
    .replace(/&/g, '&')
    .replace(/</g, '<')
    .replace(/>/g, '>');
  out = out.replace(
    /@(\w+)/g,
    `<span style="color: ${options.accent}; font-weight: 500;">@$1</span>`,
  );
  out = out.replace(
    /#(\w+)/g,
    `<span style="color: ${options.accent}; font-weight: 500;">#$1</span>`,
  );
  if (options.highlightUrls) {
    out = out.replace(
      /(https?:\/\/[^\s]+)/g,
      `<span style="color: ${options.urlAccent ?? options.accent};">$1</span>`,
    );
  }
  return out;
}

/**
 * Crop `text` for the preview of `provider`.
 *
 * The crop index is a CHARACTER index (the same one the five correct
 * previews use): the count is the rule and it is exact — the provider's
 * own `countFor` — while the crop point marks where the text stops being
 * readable. For the weighted providers (X) the two units differ, and the
 * marker is allowed to be approximate; the number beside it is not.
 *
 * The crop is applied to the PLAIN TEXT and the two halves are escaped
 * separately, so the marker lands where the founder's text actually ends
 * and no injected tag is cut in half.
 */
export function cropForPreview(
  provider: string,
  text: string,
  options: PreviewCropOptions,
): PreviewCrop {
  const limit = charLimitFor(provider);
  const count = countFor(provider, text);
  const over = count > limit;
  const before = over ? text.slice(0, limit) : text;
  const after = over ? text.slice(limit) : '';
  const beforeHtml = highlightPreviewText(before, options);
  if (!after) {
    return { html: beforeHtml, count, limit, over };
  }
  const afterHtml = highlightPreviewText(after, options);
  const title = options.markerTitle ? ` title="${options.markerTitle}"` : '';
  return {
    html: `${beforeHtml}<mark style="background-color: rgba(239, 68, 68, 0.2); color: #f87171;"${title}>${afterHtml}</mark>`,
    count,
    limit,
    over,
  };
}
