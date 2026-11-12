import type { WeekDay, CalendarEvent } from "./types";

/**
 * Today's calendar date in the APP's timezone, as `YYYY-MM-DD`.
 *
 * The calendar's cells are the app's timezone days - the events are
 * placed by `wallClock(iso, timezone)`. So "is this cell today?" and
 * "is this cell in the past?" have to be answered in the SAME zone.
 *
 * They used to be answered in the BROWSER's zone, from `new Date()`.
 * With the app in America/New_York and the browser in UTC, at 21:00
 * UTC the browser said "the 21st" while the app's zone said "the
 * 20th" - so the whole grid was labelled with the browser's dates
 * while the chips were placed by the app's, and the chips fell into
 * the neighbouring day's cell.
 *
 * The grid's own `Date` objects stay browser-local - they are the
 * GRID's shape, and `formatDateKey` reads their calendar fields. What
 * changes is the comparison: the day's KEY is compared against the
 * app's today key, not against a browser-local `Date`.
 */
export function todayKeyInZone(zone: string): string {
  try {
    const parts = new Intl.DateTimeFormat('en-CA', {
      timeZone: zone, year: 'numeric', month: '2-digit', day: '2-digit',
    }).formatToParts(new Date());
    const get = (t: string) => parts.find((p) => p.type === t)?.value ?? '';
    return `${get('year')}-${get('month')}-${get('day')}`;
  } catch {
    const t = new Date();
    return `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, '0')}-${String(t.getDate()).padStart(2, '0')}`;
  }
}

export function getMonthDays(year: number, month: number): Date[] {
  const first = new Date(year, month, 1);
  const last = new Date(year, month + 1, 0);
  const days: Date[] = [];
  // v22 Phase 7: Monday-start grid (matches getWeekDays + postiz).
  // Previously this was Sunday-start, which meant switching from week
  // view (Monday-start) to month view (Sunday-start) shifted the grid
  // by one column - confusing.
  // getDay() returns 0=Sun..6=Sat. Convert to 0=Mon..6=Sun:
  //   Mon=0, Tue=1, Wed=2, Thu=3, Fri=4, Sat=5, Sun=6
  const firstDayMon = (first.getDay() + 6) % 7;
  for (let i = firstDayMon; i > 0; i--) days.push(new Date(year, month, 1 - i));
  for (let d = 1; d <= last.getDate(); d++) days.push(new Date(year, month, d));
  // Trailing days to fill the last row (Monday-start).
  const lastDayMon = (last.getDay() + 6) % 7;
  const trailingDays = 6 - lastDayMon;
  for (let i = 1; i <= trailingDays; i++) days.push(new Date(year, month + 1, i));
  return days;
}

export function getWeekDays(date: Date): Date[] {
  const start = new Date(date);
  const day = start.getDay();
  const diff = day === 0 ? 6 : day - 1;
  start.setDate(start.getDate() - diff);
  const days: Date[] = [];
  for (let i = 0; i < 7; i++) {
    const d = new Date(start);
    d.setDate(start.getDate() + i);
    days.push(d);
  }
  return days;
}

export function getDayHours(): string[] {
  return Array.from({ length: 24 }, (_, i) =>
    `${String(i).padStart(2, "0")}:00`
  );
}

/**
 * Is this grid cell TODAY, in the app's timezone?
 *
 * The zone matters: the cells are the app's timezone days, so "today"
 * has to be the app's today. A browser-local comparison said "the
 * 21st" when the app's zone said "the 20th", and every cell was
 * mislabelled.
 */
export function isToday(d: Date, zone?: string): boolean {
  const today = zone ? todayKeyInZone(zone) : browserTodayKey();
  return formatDateKey(d) === today;
}

export function isCurrentMonth(d: Date, y: number, m: number): boolean {
  return d.getFullYear() === y && d.getMonth() === m;
}

/**
 * Phase v21: returns true if the given date is strictly before today
 * (i.e., yesterday or earlier). Used by calendar views to grey-out past
 * cells and disable drop targets (postiz-style UX cue: you can't
 * schedule in the past).
 *
 * Compares calendar days only - time-of-day is ignored. A cell for
 * "today" returns false even if the current time is 23:59.
 *
 * With a zone, the comparison is against the APP's today - the cells
 * are the app's timezone days.
 */
export function isPast(d: Date, zone?: string): boolean {
  const today = zone ? todayKeyInZone(zone) : browserTodayKey();
  // Both are `YYYY-MM-DD` keys, so a string comparison is the date
  // comparison - and it does not depend on the browser's offset.
  return formatDateKey(d) < today;
}

/** Today's date in the BROWSER's timezone, as `YYYY-MM-DD`. */
function browserTodayKey(): string {
  const t = new Date();
  return `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, '0')}-${String(t.getDate()).padStart(2, '0')}`;
}

/**
 * Parse a `YYYY-MM-DD` key into a LOCAL `Date` — the browser's calendar day.
 *
 * `new Date("2026-11-15")` parses as UTC midnight, and every negative-offset
 * zone then reads the PREVIOUS day: `/calendar?date=2026-11-15` opened
 * November 14 in New York, and the calendar wrote that wrong day back to the
 * URL and to localStorage.
 *
 * The date-only string is a CALENDAR DATE (AGENTS §9.10), so it is built
 * from its parts in local time — the same way `formatDateKey` reads it.
 */
export function parseDateKey(key: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(key.trim());
  if (!m) return null;
  const [, y, mo, d] = m;
  const date = new Date(Number(y), Number(mo) - 1, Number(d));
  if (Number.isNaN(date.getTime())) return null;
  // A key like 2026-02-31 rolls over into March; refuse it rather than
  // opening a different day than the URL names.
  if (formatDateKey(date) !== key.trim()) return null;
  return date;
}

export function formatDateKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function buildWeekDays(referenceDate: Date, events: CalendarEvent[]): WeekDay[] {
  const refMonth = referenceDate.getMonth();
  const refYear = referenceDate.getFullYear();
  return getWeekDays(referenceDate).map(d => {
    const key = formatDateKey(d);
    return {
      date: d,
      dateStr: key,
      isToday: isToday(d),
      isCurrentMonth: d.getMonth() === refMonth && d.getFullYear() === refYear,
      events: events.filter(e => e.date === key),
    };
  });
}

export const months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
// v22 Phase 7: Monday-start day labels (matches getMonthDays +
// getWeekDays, both of which now produce Monday-start grids).
export const days = ["Mon","Tue","Wed","Thu","Fri","Sat","Sun"];
export const monthsFull = ["January","February","March","April","May","June","July","August","September","October","November","December"];

export function formatDateTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
