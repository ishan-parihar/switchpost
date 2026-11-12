// ─── Calendar API Routes ──────────────────────────────────────
// Date-range queries for the content calendar.
// Returns posts grouped by date for calendar grid rendering.

use super::AppState;
use crate::api::tags::TagResponse;
use crate::auth::middleware::AuthenticatedUser;
use crate::db::queries;
use crate::error::AppError;
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    pub start: String, // ISO8601 date or datetime
    pub end: String,
    // v23-4: optional filters. Comma-separated integration_ids + single
    // campaign_id. When provided, the query narrows to matching posts.
    pub integration_ids: Option<String>, // comma-separated UUIDs
    pub campaign_id: Option<Uuid>,
    /// The founder's zone as an IANA NAME ("America/New_York").
    ///
    /// Preferred over `tz_offset` because a CONSTANT offset is wrong
    /// across a DST transition: a week window spanning 2026-11-01 in
    /// New York contained two offsets, and the posts after the
    /// transition were bucketed an hour late - a post within an hour
    /// of local midnight landed on the wrong day. A NAME resolves the
    /// offset per-instant (the server embeds the IANA database via
    /// chrono-tz), which is what the frontend's own `zoneOffsetMs`
    /// does. A name takes precedence when both are sent.
    ///
    /// An unknown name is REFUSED with a 400 naming it - a garbled
    /// zone must not silently bucket every post in UTC.
    pub tz: Option<String>,
    /// The zone's UTC offset in SECONDS. Legacy form: correct within a
    /// window that has no DST transition, wrong across one. Kept for
    /// the MCP tool and any API caller that only knows an offset.
    /// Validated on EVERY request (see the handler) - a valid `tz`
    /// shadows the constant's USE but not its validation.
    pub tz_offset: Option<i64>,
}

// ── zone resolution ───────────────────────────────────────────

/// Parse an IANA zone name ONCE, up front. An unknown name is a
/// caller error and must be refused - NOT silently treated as UTC:
/// a garbled zone would misplace every post in the window with a 200
/// and plausible-looking day keys, the worst kind of wrong (the same
/// reason `tz_offset` beyond a day is a 400, not a clamp).
pub fn parse_zone(name: &str) -> Result<chrono_tz::Tz, String> {
    use std::str::FromStr;
    chrono_tz::Tz::from_str(name).map_err(|_| format!("tz is not a known IANA zone name: {name}"))
}

/// The offset of a PARSED zone (see `parse_zone`) at one instant.
/// Per-instant, so a window spanning a DST transition buckets every
/// post into the day the founder sees - the same as the frontend's
/// `zoneOffsetMs`.
pub fn zone_offset_seconds(zone: chrono_tz::Tz, at: chrono::DateTime<chrono::Utc>) -> i64 {
    use chrono::Offset;
    at.with_timezone(&zone).offset().fix().local_minus_utc() as i64
}

/// The zone's UTC offset at an instant, from the zone's NAME.
///
/// The name-taking form callers outside the crate (the parity test,
/// which compares this rule against the browser's `Intl`) and any
/// in-crate caller that only has a name use. Parses through THE ONE
/// `parse_zone`, so an unknown name is an error, never a silent UTC.
pub fn zone_offset_by_name(zone: &str, at: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    parse_zone(zone).ok().map(|tz| zone_offset_seconds(tz, at))
}

/// The LOCAL day an instant belongs to, given the zone's UTC offset.
///
/// THE ONE IMPLEMENTATION of the day-key rule. The REST calendar and
/// the MCP `calendar_get` tool both call it, because the two surfaces
/// must not disagree: an agent asking "what is on Oct 25?" through the
/// tool and a founder looking at the calendar have to get the same
/// answer, and before this existed the tool bucketed in UTC while the
/// REST endpoint bucketed in the founder's zone.
///
/// The offset is applied to the instant and THEN the date is taken,
/// which is the only order that is right: taking the UTC date first
/// and shifting it afterwards would move a post across midnight the
/// wrong way (a 02:00 UTC post in New York is the PREVIOUS day, and a
/// post at 23:00 UTC in Auckland is the NEXT day).
pub fn local_day_key(dt: chrono::DateTime<chrono::Utc>, tz_offset: Option<i64>) -> String {
    match tz_offset {
        // The CALLERS validate the offset (both the REST handler and
        // the MCP tool refuse |offset| > 86400 before calling); this
        // bound is a BACKSTOP, not the validation, because
        // `Duration::seconds` PANICS past i64::MAX/1000 and this
        // function is shared - a future caller that forgets the check
        // falls back to UTC instead of panicking.
        Some(secs) if secs.unsigned_abs() <= 86_400 => (dt + chrono::Duration::seconds(secs))
            .format("%Y-%m-%d")
            .to_string(),
        _ => dt.format("%Y-%m-%d").to_string(),
    }
}

#[derive(Debug, Serialize)]
pub struct CalendarPost {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub integration_name: String,
    pub state: String,
    pub content: String,
    pub title: Option<String>,
    pub media: serde_json::Value,
    pub scheduled_at: Option<String>,
    pub published_at: Option<String>,
    pub platform_post_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub tags: Vec<TagResponse>,
    pub repeat_interval_days: Option<i32>,
    pub repeat_end_date: Option<String>,
    pub group_id: Option<Uuid>,
    pub first_comment: Option<String>,
    pub sequence: i32,
    // v23-4: campaign_id for chip coloring + filtering.
    pub campaign_id: Option<Uuid>,
    // Engagement metrics (optional — only populated when analytics_cache has data)
    pub likes: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub impressions: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CalendarDay {
    pub date: String,
    pub posts: Vec<CalendarPost>,
    pub post_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CalendarResponse {
    pub days: Vec<CalendarDay>,
    pub total: usize,
}

/// GET /api/calendar?start=2026-05-01&end=2026-05-31
/// v23-4: optional filters via integration_ids (comma-separated) + campaign_id.
pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<CalendarQuery>,
) -> Result<Json<CalendarResponse>, AppError> {
    // Parse date range
    let start = parse_date_or_datetime(&query.start).ok_or_else(|| {
        AppError::BadRequest("Invalid start date. Use ISO8601 (YYYY-MM-DD or full datetime)".into())
    })?;
    // The `end` parameter is an INCLUSIVE DATE: "end=2026-10-31" means
    // "through the end of the 31st", not "at 00:00:00 on the 31st".
    // Parsing it to midnight excluded every post on the last day of the
    // range, so the 31st cell of a month view and the Sunday column of a
    // week view were always empty. A caller that sends a full datetime
    // gets that exact instant (the UI sends date-only).
    let end = parse_end_date_or_datetime(&query.end)
        .ok_or_else(|| AppError::BadRequest("Invalid end date".into()))?;

    // CAP THE RANGE.
    //
    // The calendar is a VIEW: a founder opens a month, a week or a day. A
    // caller that asks for ten years gets every post in the database
    // loaded, bucketed and serialised — on the single writer, while the
    // scheduler is publishing. The cap is a YEAR, which is more than the
    // UI ever asks for and is enough for a yearly view.
    const MAX_CALENDAR_RANGE_DAYS: i64 = 366;
    let range_days = (end - start).num_days();
    if range_days > MAX_CALENDAR_RANGE_DAYS {
        return Err(AppError::BadRequest(format!(
            "The calendar range is {range_days} days; the maximum is {MAX_CALENDAR_RANGE_DAYS}. \
             Ask for a narrower window."
        )));
    }

    // A NAME beats a constant: it resolves the offset PER-INSTANT
    // (DST-correct), so a window spanning a transition buckets every
    // post into the day the founder sees. The constant stays for
    // callers that only know an offset (the MCP tool's legacy input).
    //
    // An unknown NAME is refused with a 400, exactly like an out-of-
    // bound constant: a garbled zone must be loud, not silently UTC
    // (which would misplace every post in the window with a 200).
    // `tz_offset` is validated ALWAYS, not only on the no-`tz` arm:
    // `chrono::Duration::seconds` PANICS when the magnitude is beyond
    // i64::MAX/1000, there is no CatchPanic layer in the router, and
    // a caller that sends BOTH a name and a garbage offset must not
    // reach the bucketing just because the name wins.
    if let Some(secs) = query.tz_offset {
        if secs.unsigned_abs() > 86_400 {
            return Err(AppError::BadRequest(
                "tz_offset must be within plus or minus one day (in seconds)".into(),
            ));
        }
    }
    let tz = match query.tz.as_deref() {
        Some(name) => Some(parse_zone(name).map_err(AppError::BadRequest)?),
        None => None,
    };
    // The constant is only USED when no name was given (the name
    // resolves per-instant), but it was validated above either way.
    let const_offset = if tz.is_some() { None } else { query.tz_offset };
    // One closure for all three cases: name (per-instant), constant
    // offset, UTC (absent).
    let local = |dt: chrono::DateTime<chrono::Utc>| -> String {
        let secs = match tz {
            Some(zone) => zone_offset_seconds(zone, dt),
            None => const_offset.unwrap_or(0),
        };
        local_day_key(dt, Some(secs))
    };

    // v23-4: parse optional integration_ids filter (comma-separated UUIDs).
    let integration_ids: Option<Vec<Uuid>> = query
        .integration_ids
        .as_deref()
        .map(|s| {
            s.split(',')
                .filter_map(|id| Uuid::parse_str(id.trim()).ok())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    let integration_ids_ref = integration_ids.as_deref();

    let posts = queries::get_calendar_posts_with_metrics(
        &state.db,
        auth.user_id,
        start,
        end,
        integration_ids_ref,
        query.campaign_id,
    )
    .await?;

    // The tags of every post in the range, in ONE query.
    //
    // It used to be one `get_tags_for_post` per post: a year-wide window
    // opened 300 posts and ran 301 sequential queries against the single
    // writer, which is the same writer the scheduler publishes on.
    let post_ids: Vec<Uuid> = posts.iter().map(|p| p.id).collect();
    let tags_by_post = queries::get_tags_for_posts(&state.db, &post_ids, auth.user_id)
        .await
        .unwrap_or_default();

    let mut day_map: std::collections::BTreeMap<String, Vec<CalendarPost>> =
        std::collections::BTreeMap::new();
    for p in posts {
        // Use published_at for published posts, scheduled_at for queued/draft posts
        //
        // The day key is a LOCAL day: the post belongs to the day the
        // founder sees, in THEIR zone.
        //
        // The offset is applied to the instant and THEN the date is
        // taken, which is the only order that is right: taking the UTC
        // date first and shifting it afterwards would move a post
        // across midnight the wrong way (a 02:00 UTC post in New York
        // is the PREVIOUS day, and a post at 23:00 UTC in Auckland is
        // the NEXT day).
        //
        // `local` is the shared closure built above: per-instant for a
        // zone name, constant for a legacy offset, UTC when absent.
        let date_key = if p.state == "published" && p.published_at.is_some() {
            p.published_at.map(&local)
        } else {
            p.scheduled_at.map(&local)
        }
        .unwrap_or_else(|| "unscheduled".into());

        let integration_name = p
            .integration_name
            .unwrap_or_else(|| "Removed channel".into());

        let tags = tags_by_post
            .get(&p.id)
            .map(|rows| {
                rows.iter()
                    .map(|row| TagResponse {
                        id: row.id,
                        name: row.name.clone(),
                        color: row.color.clone(),
                        created_at: row.created_at.to_rfc3339(),
                        updated_at: row.updated_at.to_rfc3339(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        day_map.entry(date_key).or_default().push(CalendarPost {
            id: p.id,
            integration_id: p.integration_id,
            integration_name,
            state: p.state,
            content: p.content,
            title: p.title,
            media: crate::db::models::media_as_array(&p.media),
            scheduled_at: p.scheduled_at.map(|d| d.to_rfc3339()),
            published_at: p.published_at.map(|d| d.to_rfc3339()),
            platform_post_url: p.platform_post_url,
            error_message: p.error_message,
            created_at: p.created_at.to_rfc3339(),
            tags,
            repeat_interval_days: p.repeat_interval_days,
            repeat_end_date: p.repeat_end_date.map(|d| d.to_rfc3339()),
            group_id: p.group_id,
            first_comment: p.first_comment,
            sequence: p.sequence,
            campaign_id: p.campaign_id,
            likes: p.likes,
            comments: p.comments,
            shares: p.shares,
            impressions: p.impressions,
        });
    }

    let days: Vec<CalendarDay> = day_map
        .into_iter()
        .map(|(date, posts)| {
            let post_count = posts.len();
            CalendarDay {
                date,
                posts,
                post_count,
            }
        })
        .collect();

    let total = days.iter().map(|d| d.posts.len()).sum();
    Ok(Json(CalendarResponse { days, total }))
}

fn parse_date_or_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try full ISO8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try date-only
    if let Ok(naive) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive_dt = naive
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(Utc)
            .single()?;
        return Some(naive_dt);
    }
    None
}

/// Parse the `end` of an inclusive date range.
///
/// A date-only `end` covers the WHOLE day, so it resolves to the last
/// representable instant of that day. A full datetime is taken as given,
/// because a caller that sends one is expressing an exact bound.
fn parse_end_date_or_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        // 23:59:59.999999999 - the last instant of the day. The SQL uses
        // `<=`, so this includes every post on the end date.
        let naive_dt = naive
            .and_hms_nano_opt(23, 59, 59, 999_999_999)?
            .and_local_timezone(Utc)
            .single()?;
        return Some(naive_dt);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{parse_zone, zone_offset_seconds};

    /// The shared resolver resolves the offset PER-INSTANT, which is
    /// the whole point of taking a NAME instead of a constant: New
    /// York is -4h before the 2026-11-01 06:00 UTC fall-back and -5h
    /// after. Pinned here, where the rule lives (AGENTS.md 9.18).
    #[test]
    fn zone_offset_seconds_resolves_the_transition_per_instant() {
        let tz = parse_zone("America/New_York").expect("known zone");
        let before = chrono::DateTime::parse_from_rfc3339("2026-11-01T05:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let after = chrono::DateTime::parse_from_rfc3339("2026-11-01T07:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(zone_offset_seconds(tz, before), -4 * 3600);
        assert_eq!(zone_offset_seconds(tz, after), -5 * 3600);
    }

    /// An unknown name is REFUSED, not treated as UTC: a garbled
    /// zone must never silently misplace every post in a window.
    #[test]
    fn an_unknown_zone_name_is_refused_not_treated_as_utc() {
        assert!(parse_zone("Mars/Olympus_Mons").is_err());
        assert!(
            parse_zone("America%2FNew_York").is_err(),
            "a doubly-encoded name (the classic client bug) must be loud, not UTC"
        );
        assert!(parse_zone("").is_err());
    }
}
