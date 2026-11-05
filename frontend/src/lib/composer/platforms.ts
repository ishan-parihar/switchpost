// Platform posting rules - v25 F2.
//
// ONE table that answers "what does this platform actually accept?", consumed by
// three callers that previously each carried their own partial copy of the
// answer:
//
//   PerPlatformCharCount  → the ring needs `charLimit` + the counting mode
//   ComposerModal         → submit blocking needs "is this post legal yet?"
//   PlatformPreviewPane  → the preview needs to know if media is required
//
// Character limits are NOT restated here. `providerMeta().charLimit` in
// $lib/providers is the authority (it tracks the backend's
// `Provider::max_content_length()`), so this table only carries what that
// shape does not model: media rules, required fields, and the X weighted
// counting mode.
//
// Media limits track the 2026 public API docs for each platform's
// Content Publishing surface. They are a UX gate, not a security boundary -
// `POST /api/posts/validate` re-checks server-side and ComposerModal still
// calls it on submit. A stale number here shows a warning too early or too
// late; it can never let an illegal post through.

import { providerMeta, charLimitFor as backendCharLimitFor } from '$lib/providers';

// The post-body conversion lives in its own module so the parity test can
// run the REAL implementation under node. It is re-exported here because
// this is where the composer imports it from.
import { plainText } from './post-text';
export { plainText };

export interface PlatformSpec {
  /** Provider key as registered in $lib/providers. */
  provider: string;
  /** Human label, delegated so the two tables cannot disagree. */
  label: string;
  /**
   * How to count characters against the limit.
   * - 'weighted' → X's twitter-text rules: emoji and CJK count 2.
   * - 'plain'    → JavaScript string length (everything else).
   */
  counting: 'weighted' | 'plain';
  /** Max images in one post. `Infinity` when the platform does not cap it. */
  maxImages: number;
  /** Max videos in one post. `Infinity` when uncapped. */
  maxVideos: number;
  /**
   * Max MEDIA ITEMS in one post, counting images and videos together.
   *
   * The server enforces this separately from the per-kind caps: X takes
   * 4 items TOTAL, Bluesky 4, Mastodon 4, Facebook 10, Reddit 20,
   * Threads 20. The composer only checked images and videos SEPARATELY,
   * so 8 images + 8 videos passed the composer's checks and was
   * rejected by the server on save.
   *
   * `Infinity` when the platform does not cap the total.
   */
  maxTotal: number;
  /** True when images and videos cannot be mixed in the same post. */
  exclusiveMedia: boolean;
  /** True when the platform rejects a post with no media at all. */
  mediaRequired: boolean;
  /** True when the platform rejects a post with an empty title. */
  titleRequired: boolean;
  /** Handle decoration the platform shows, e.g. '@' for X. Empty = none. */
  handlePrefix: string;
  /** True when the platform's preview chrome is a 9:16 vertical frame. */
  vertical: boolean;
}

/**
 * The 12 Tier-1 platforms, in the v25 §1 order. `linkedin-page` and
 * `instagram-standalone` are separate registry keys that share chrome with
 * their personal-account siblings, so both appear.
 *
 * Tier-2 platforms are deliberately absent: they are publish-maintained and
 * get the GeneralPreview, which is the correct level of fidelity for a
 * platform we do not claim depth on. Tier-3 (farcaster) is absent entirely -
 * it is not in the default registry, so it can never be selected.
 */
export const TIER1: readonly string[] = [
  'x',
  'linkedin',
  'linkedin-page',
  'facebook',
  'instagram',
  'instagram-standalone',
  'threads',
  'youtube',
  'tiktok',
  'reddit',
  'bluesky',
  'pinterest',
];

const SPECS: Record<string, PlatformSpec> = {
  x: {
    provider: 'x', label: providerMeta('x').label, counting: 'weighted',
    maxImages: 4, maxTotal: 4, maxVideos: 1, exclusiveMedia: true,
    mediaRequired: false, titleRequired: false, handlePrefix: '@', vertical: false,
  },
  // Personal and Page accounts post through the same UGC surface with the
  // same media rules; only the identity chrome differs.
  linkedin: {
    provider: 'linkedin', label: providerMeta('linkedin').label, counting: 'plain',
    maxImages: 20, maxTotal: 20, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: false, titleRequired: false, handlePrefix: '', vertical: false,
  },
  'linkedin-page': {
    provider: 'linkedin-page', label: 'LinkedIn Page', counting: 'plain',
    maxImages: 20, maxTotal: 20, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: false, titleRequired: false, handlePrefix: '', vertical: false,
  },
  facebook: {
    provider: 'facebook', label: providerMeta('facebook').label, counting: 'plain',
    maxImages: 10, maxTotal: 10, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: false, titleRequired: false, handlePrefix: '', vertical: false,
  },
  instagram: {
    provider: 'instagram', label: providerMeta('instagram').label, counting: 'plain',
    maxImages: 10, maxTotal: 10, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: true, titleRequired: false, handlePrefix: '@', vertical: false,
  },
  'instagram-standalone': {
    provider: 'instagram-standalone', label: 'Instagram (standalone)', counting: 'plain',
    maxImages: 10, maxTotal: 10, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: true, titleRequired: false, handlePrefix: '@', vertical: false,
  },
  threads: {
    provider: 'threads', label: providerMeta('threads').label, counting: 'plain',
    maxImages: 20, maxTotal: 20, maxVideos: 1, exclusiveMedia: false,
    mediaRequired: false, titleRequired: false, handlePrefix: '@', vertical: false,
  },
  // A YouTube upload IS the video. A text-only post cannot publish, so the
  // gate is "one video" and the title is mandatory metadata.
  youtube: {
    provider: 'youtube', label: providerMeta('youtube').label, counting: 'plain',
    maxImages: 0, maxTotal: 1, maxVideos: 1, exclusiveMedia: true,
    mediaRequired: true, titleRequired: true, handlePrefix: '@', vertical: true,
  },
  tiktok: {
    provider: 'tiktok', label: providerMeta('tiktok').label, counting: 'plain',
    maxImages: 0, maxTotal: 1, maxVideos: 1, exclusiveMedia: true,
    mediaRequired: true, titleRequired: false, handlePrefix: '@', vertical: true,
  },
  reddit: {
    provider: 'reddit', label: providerMeta('reddit').label, counting: 'plain',
    maxImages: 20, maxTotal: 20, maxVideos: 0, exclusiveMedia: true,
    mediaRequired: false, titleRequired: true, handlePrefix: 'u/', vertical: false,
  },
  bluesky: {
    provider: 'bluesky', label: providerMeta('bluesky').label, counting: 'plain',
    maxImages: 4, maxTotal: 4, maxVideos: 1, exclusiveMedia: true,
    mediaRequired: false, titleRequired: false, handlePrefix: '@', vertical: false,
  },
  pinterest: {
    provider: 'pinterest', label: providerMeta('pinterest').label, counting: 'plain',
    maxImages: 1, maxTotal: 1, maxVideos: 1, exclusiveMedia: true,
    mediaRequired: true, titleRequired: true, handlePrefix: '', vertical: true,
  },
};

/**
 * X's weighted character count (twitter-text v3 rules, as the v2 API applies
 * them): characters in emoji and CJK presentation ranges cost 2, everything
 * else costs 1. `String.length` charges 1 per UTF-16 code unit, which
 * undercounts an emoji-heavy post and lets it pass the provider's weighted
 * gate that the API will then reject. (That gate is whatever
 * `charLimitFor(provider)` says - 4000 for X as of the provider
 * implementation, not the 280 that X's own free-tier UI suggests.)
 *
 * Ranges are the same ones the X weighted-length helper used before F2 moved
 * it here; the behaviour is unchanged, only the owner is.
 */
export function weightedLength(text: string): number {
  let count = 0;
  for (const char of text) {
    const cp = char.codePointAt(0) ?? 0;
    const heavy =
      (cp >= 0x3040 && cp <= 0x30ff) ||  // Hiragana + Katakana
      (cp >= 0x3400 && cp <= 0x4dbf) ||  // CJK Extension A
      (cp >= 0x4e00 && cp <= 0x9fff) ||  // CJK Unified Ideographs
      (cp >= 0xf900 && cp <= 0xfaff) ||  // CJK Compatibility Ideographs
      (cp >= 0xac00 && cp <= 0xd7af) ||  // Hangul Syllables
      (cp >= 0x1f300 && cp <= 0x1f9ff) || // Emoji blocks
      (cp >= 0x2600 && cp <= 0x27bf);    // Misc Symbols + Dingbats
    count += heavy ? 2 : 1;
  }
  return count;
}


/**
 * The counting rule for a provider, with `twitter` mapped onto `x` because
 * both registry keys resolve to the same platform.
 */
export function countingMode(provider: string): 'weighted' | 'plain' {
  return specFor(provider)?.counting ?? 'plain';
}

/**
 * Count `text` the way `provider` counts it.
 *
 * The plain rule is BYTES, not `String.length`. The server measures
 * `clean.len()` - UTF-8 bytes - and the two have to agree, because the
 * composer shows this number and the server enforces it. `String.length`
 * counts UTF-16 code units, so a 200-emoji Bluesky post read
 * "200/300" in the composer and was rejected by the server as
 * "800 chars, max 300" - the user was told the post was 800
 * characters when the composer had said 200.
 */
export function countFor(provider: string, text: string): number {
  return countingMode(provider) === 'weighted'
    ? weightedLength(text)
    : new TextEncoder().encode(text).length;
}

/** The spec for a provider, or undefined when it is not a Tier-1 platform. */
export function specFor(provider: string): PlatformSpec | undefined {
  // `twitter` is a legacy alias for `x` in the provider registry; treat the
  // two as one platform everywhere or they drift apart on every rule.
  const key = provider === 'twitter' ? 'x' : provider;
  return SPECS[key];
}

/** True when this provider is one of the 12 depth platforms. */
export function isTier1(provider: string): boolean {
  return specFor(provider) !== undefined;
}

/** The character limit, delegated to $lib/providers (the authority).
 *
 *  $lib/providers prefers the limit the SERVER published
 *  (`Provider::max_content_length()`) and falls back to its static table
 *  before `/api/providers` has answered. */
export function charLimitFor(provider: string): number {
  return backendCharLimitFor(provider);
}

/** A single reason this post cannot be submitted to `provider` yet. */
export interface PlatformBlocker {
  provider: string;
  label: string;
  /** Short, human, actionable. Shown verbatim in the blocking banner. */
  message: string;
}

/**
 * Everything that stops this content going out on `provider` - as a list, so
 * the banner can show all of them at once instead of making the user fix one,
 * re-submit, and discover the next.
 *
 * `text` is the platform's own content (its override if it has one, else the
 * global body) and `html` is the same string before tag-stripping; pass the
 * raw HTML and this function strips it, because the platform API receives the
 * stripped text and the limit applies to that.
 */
export function blockersFor(
  provider: string,
  html: string,
  opts: { title?: string; mediaCount?: number; videoCount?: number; imageCount?: number } = {},
): PlatformBlocker[] {
  const spec = specFor(provider);
  if (!spec) return []; // Tier-2: no depth gate, the server decides.
  const out: PlatformBlocker[] = [];
  const text = plainText(html);
  const limit = charLimitFor(provider);
  const count = countFor(provider, text);

  // An empty body is only an error when there is no media to carry the
  // post. Instagram, TikTok, YouTube and Pinterest all accept a post
  // that is a photo with no caption - which is the NORMAL case for a
  // photo. The rule used to fire before the media rules, so the
  // composer blocked "Post body is empty" for a single image with no
  // caption, and the server agreed ("Content is empty for Instagram").
  const media = opts.mediaCount ?? 0;
  if (text.length === 0 && media === 0) {
    out.push({ provider, label: spec.label, message: 'Post body is empty' });
  } else if (count > limit) {
    const over = count - limit;
    out.push({
      provider,
      label: spec.label,
      message: `${spec.counting === 'weighted' ? 'Weighted count' : 'Character count'} is ${count}/${limit} - ${over} over`,
    });
  }
  if (spec.titleRequired && !(opts.title || '').trim()) {
    out.push({ provider, label: spec.label, message: 'Title is required' });
  }
  if (spec.mediaRequired && media === 0) {
    out.push({
      provider,
      label: spec.label,
      message: spec.maxImages === 0
        ? 'Requires a video before it can publish'
        : spec.maxVideos === 0
          ? 'Requires an image before it can publish'
          : 'Requires media before it can publish',
    });
  }
  const images = opts.imageCount ?? 0;
  const videos = opts.videoCount ?? 0;
  if (images > spec.maxImages) {
    out.push({
      provider, label: spec.label,
      message: `${images} images - ${spec.label} accepts at most ${spec.maxImages}`,
    });
  }
  if (videos > spec.maxVideos) {
    out.push({
      provider, label: spec.label,
      message: `${videos} videos - ${spec.label} accepts at most ${spec.maxVideos}`,
    });
  }
  // The TOTAL, not just the per-kind counts. The server caps the total
  // separately (X 4, Bluesky 4, Mastodon 4, Facebook 10, Reddit 20,
  // Threads 20), so 8 images + 8 videos passed every check above and
  // was rejected by the server on save.
  const total = images + videos;
  if (total > spec.maxTotal) {
    out.push({
      provider, label: spec.label,
      message: `${total} media items - ${spec.label} accepts at most ${spec.maxTotal} in one post`,
    });
  }
  if (spec.exclusiveMedia && images > 0 && videos > 0) {
    out.push({
      provider, label: spec.label,
      message: 'Images and video cannot be mixed in one post on this platform',
    });
  }
  return out;
}
