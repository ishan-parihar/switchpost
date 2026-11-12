import type { PostSummary } from "$lib/api/posts";
import type { CalendarPost } from "$lib/api/calendar";
import type { Tag } from "$lib/api/tags";
import { utcToWallClock } from "$lib/composer/wall-clock";

export type CalendarView = "month" | "week" | "day" | "list";

export interface CalendarEvent {
  id: string;
  date: string;         // YYYY-MM-DD
  time?: string;       // HH:mm (optional)
  title: string;
  content: string;      // preview text
  state: string;
  platform: string;
  integrationName: string;
  // The integration row's id - present on every post the API
  // returns (`integration_id`). The stats modal needs it to offer
  // an inline token refresh when the provider token expired.
  integrationId?: string;
  postUrl?: string;
  error?: string;
  tags?: Array<{ id: string; name: string; color: string }>;
  repeatIntervalDays?: number | null;
  repeatEndDate?: string | null;
  groupId?: string | null;
  // Phase 8: campaign awareness for calendar.
  campaignId?: string | null;
  campaignColor?: string | null;
  // Engagement metrics (optional - populated from analytics_cache)
  likes?: number | null;
  comments?: number | null;
  shares?: number | null;
  impressions?: number | null;
}

export interface WeekDay {
  date: Date;
  dateStr: string;      // YYYY-MM-DD
  isToday: boolean;
  isCurrentMonth: boolean;
  events: CalendarEvent[];
}

/**
 * The wall-clock date and time of a UTC instant, in `zone`.
 *
 * This is the calendar's name for the composer's `utcToWallClock` — the
 * SAME conversion, so a post the founder set at 09:00 in their zone reads
 * as 09:00 on the calendar grid.
 *
 * It used to be a SECOND copy of the conversion (the same `Intl`
 * `formatToParts` dance, written out again), and the copies drifted: this
 * one accepted a null timestamp and returned an empty pair, the other did
 * not. The calendar now calls the shared one — `$lib/composer/wall-clock`,
 * which is pinned against the server by `tests/wall_clock_parity_test.rs`.
 */
export function wallClock(
  iso: string | null | undefined,
  zone: string,
): { date: string; time: string } {
  if (!iso) return { date: "", time: "" };
  return utcToWallClock(iso, zone);
}

/**
 * A post as the calendar or the posts list delivers it.
 *
 * The two endpoints differ: the posts list nests the engagement under
 * `engagement`, the calendar endpoint returns it as four flat fields.
 * The calendar view is fed by both (the grid from `/api/calendar`, the
 * list view from `/api/posts`), so the mapper accepts either.
 */
export type CalendarPostLike = (PostSummary | CalendarPost) & {
  engagement?: { likes?: number | null; comments?: number | null; shares?: number | null; impressions?: number | null };
  likes?: number | null;
  comments?: number | null;
  shares?: number | null;
  impressions?: number | null;
};

export function toCalendarEvent(
  post: CalendarPostLike,
  zone = "UTC",
  campaignColor?: string | null,
): CalendarEvent {
  // For published posts, use the actual published_at timestamp
  // For queued/draft posts, use the scheduled_at timestamp
  const ts = post.state === "published" && post.published_at ? post.published_at : post.scheduled_at;
  const wall = wallClock(ts, zone);
  return {
    id: post.id,
    date: wall.date,
    time: wall.time || undefined,
    title: post.title || post.content.slice(0, 60),
    content: post.content,
    state: post.state,
    platform: post.integration_name,
    integrationName: post.integration_name,
    integrationId: post.integration_id,
    postUrl: post.platform_post_url,
    error: post.error_message,
    tags: post.tags?.map(t => ({ id: t.id, name: t.name, color: t.color })),
    repeatIntervalDays: post.repeat_interval_days ?? null,
    repeatEndDate: post.repeat_end_date ?? null,
    groupId: post.group_id ?? null,
    // v23-4 shipped the campaign chip colour, but the mapping never copied
    // `campaign_id` across, so `campaignColor` was always undefined and the
    // chip fell back to the first tag's colour. The backend does return
    // `campaign_id`; the colour comes from the campaign list the page
    // already holds.
    campaignId: post.campaign_id ?? null,
    campaignColor: post.campaign_id ? (campaignColor ?? null) : null,
    likes: post.engagement?.likes ?? post.likes ?? null,
    comments: post.engagement?.comments ?? post.comments ?? null,
    shares: post.engagement?.shares ?? post.shares ?? null,
    impressions: post.engagement?.impressions ?? post.impressions ?? null,
  };
}
