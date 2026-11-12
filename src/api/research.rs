// ─── Research API Routes ───────────────────────────────────────
// Read-only "what should I post next" surface for a solo founder.
//
// Everything here is derived from data Switchpost already holds — own
// published-post engagement (`analytics_cache`), imported
// external/competitor posts (`external_posts` + `post_engagement`), and
// connected-feed titles (`rss_posts`). There are no external network
// calls and no writes: when a section has no stored data it comes back
// empty, and the frontend says what is missing and where to get it.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::AuthenticatedUser;
use crate::db::queries;
use crate::db::queries::{ExternalPostEngagement, OwnPostEngagement};
use crate::error::AppError;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct ResearchQuery {
    /// Window in days for every section. Default 90, clamped to 1..3650.
    #[serde(default = "default_days")]
    pub days: i32,
    /// The founder's zone as an IANA NAME ("America/New_York"). The
    /// best-time slots are bucketed in the founder's LOCAL weekday
    /// and hour (DST-correct), because "post Mon 15:00" is only
    /// actionable in the zone the founder posts from - the composer,
    /// the calendar and the analytics series all already speak the
    /// founder's zone. Unknown name: 400. Absent: UTC (legacy).
    #[serde(default)]
    pub tz: Option<String>,
    /// Items per ranked section. Default 10, clamped to 1..50.
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Words shorter than this are dropped from the content-gap section
    /// (they are almost always noise). Default 4.
    #[serde(default = "default_min_word_length")]
    pub min_word_length: usize,
}

fn default_days() -> i32 {
    90
}

fn default_limit() -> i64 {
    10
}

fn default_min_word_length() -> usize {
    4
}

pub const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Fold one epoch into a (weekday, hour) slot in the founder's zone
/// (DST-correct, per-instant) or UTC when no zone is given.
///
/// THE ONE slot rule (AGENTS.md 9.18): the research insight and the MCP
/// `posts_best_times` tool both fold through it, so an agent and the
/// dashboard cannot disagree about the same post's slot.
///
/// `weekday` is SQLite's `strftime('%w')` convention (0 = Sunday) so
/// `WEEKDAYS[weekday]` keeps indexing the way it always did.
pub fn week_slot(epoch: i64, tz: Option<chrono_tz::Tz>) -> Option<(i32, i32)> {
    use chrono::Timelike;
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0)?;
    let local = match tz {
        Some(z) => dt + chrono::Duration::seconds(crate::api::calendar::zone_offset_seconds(z, dt)),
        None => dt,
    };
    // %w: 0=Sunday..6=Saturday. date_naive has no weekday-offset trap
    // because NaiveDateTime's `weekday()` is the same convention's
    // enum - but formatting keeps the i32 shape the callers share.
    let wd = local.format("%w").to_string().parse::<i32>().ok()?;
    Some((wd, local.hour() as i32))
}

// ── Response types ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OwnPostRow {
    pub post_id: Uuid,
    pub content: String,
    pub provider: String,
    /// ISO8601, or `None` for a published post whose publish timestamp was
    /// never recorded.
    pub published_at: Option<String>,
    /// Likes + comments + shares from the cached per-post analytics.
    pub engagement: i64,
}

#[derive(Debug, Serialize)]
pub struct ExternalPostRow {
    pub post_id: Uuid,
    pub provider: String,
    pub text: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub url: Option<String>,
    pub published_at: String,
    pub engagement: i64,
}

/// One weekday×hour slot with the engagement earned there, so the founder
/// can see *when* their own best posts went out.
#[derive(Debug, Serialize)]
pub struct PostingSlot {
    pub weekday: String,
    /// 0..23, in the zone the request named (UTC when none).
    pub hour: i32,
    pub label: String,
    /// The clock range alone ("06:00–07:00"), for narrow table cells -
    /// the full label names the zone and would truncate mid-word in
    /// the 80px cell; the caption below already names the zone.
    pub hour_range: String,
    /// How many of the founder's own posts (in the window) went out in
    /// this slot — the sample the ranking rests on.
    pub posts: i64,
    pub engagement: i64,
    /// Average engagement per post in this slot, to two decimals. It is the
    /// fair comparison: a slot with one lucky post is not better than a
    /// slot with ten solid ones.
    pub engagement_per_post: f64,
}

#[derive(Debug, Serialize)]
pub struct TimingInsight {
    /// `true` when the estimate is based on too few posts to be meaningful.
    pub low_confidence: bool,
    /// How many of the founder's own posts the estimate is based on.
    pub posts_analyzed: i64,
    pub best_slots: Vec<PostingSlot>,
}

#[derive(Debug, Serialize)]
pub struct ContentGap {
    pub word: String,
    /// How many external/competitor posts (in the window) used the word.
    /// The founder has never used it in their own posts — that difference
    /// is the gap.
    pub external_posts: i64,
    /// One representative competitor post text, so the gap shows what the
    /// word was used to talk about.
    pub example: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FeedItemRow {
    pub id: Uuid,
    pub title: String,
    pub url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResearchResponse {
    /// (a) The founder's own posts, ranked by engagement.
    pub top_own_posts: Vec<OwnPostRow>,
    /// (b) Recent external/competitor posts, ranked by engagement.
    pub top_external_posts: Vec<ExternalPostRow>,
    /// (c) When the founder's own posts performed best.
    pub timing: Option<TimingInsight>,
    /// (d) Topics present in competitor posts but absent from the
    /// founder's own.
    pub content_gaps: Vec<ContentGap>,
    /// What the founder's connected RSS feeds are publishing. Feed titles
    /// are the one research signal that carries no engagement, so it is
    /// reported as a list, never ranked.
    pub connected_feed_items: Vec<FeedItemRow>,
    /// How many rows each section was computed from. A section that is
    /// empty says which source had nothing, so the frontend can name it.
    pub sources: SourceCounts,
    pub window_days: i32,
}

#[derive(Debug, Serialize)]
pub struct SourceCounts {
    pub own_posts: i64,
    pub external_posts: i64,
    pub feed_items: i64,
}

// ── Word splitting for the content gap ────────────────────────
//
// `split_words` is the only place the "what is a topic" rule lives, so the
// gap section and the tests share one definition.

/// English function words that carry no topic signal. Kept short on
/// purpose: every word listed here is a word a founder would never want
/// surfaced as a content gap, and a longer list would start dropping real
/// topics (`x` and `i` are here because they are lowercase identically in
/// post text — "x said", "i think").
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "your", "yours", "all", "any", "can", "had",
    "has", "have", "her", "him", "his", "how", "its", "let", "may", "new", "now", "off", "old",
    "one", "our", "out", "per", "put", "say", "she", "too", "use", "via", "vs", "was", "who",
    "why", "will", "with", "yet", "get", "got", "just", "like", "make", "made", "more", "most",
    "much", "must", "need", "only", "over", "some", "such", "than", "that", "them", "then", "they",
    "this", "very", "want", "were", "what", "when", "where", "which", "while", "would", "about",
    "after", "again", "also", "back", "because", "been", "before", "being", "between", "both",
    "came", "come", "could", "does", "done", "down", "each", "even", "ever", "every", "from",
    "give", "going", "good", "great", "here", "into", "keep", "know", "last", "less", "look",
    "many", "meet", "might", "most", "next", "none", "once", "onto", "other", "others", "part",
    "really", "right", "same", "seem", "should", "since", "still", "take", "there", "these",
    "thing", "things", "think", "those", "through", "time", "under", "until", "upon", "well",
    "went", "what", "when", "where", "whether", "while", "whose", "within", "without", "work",
    "year", "years", "your",
];

fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.contains(&word)
}

/// Split post text into lowercase word tokens.
///
/// Handles Unicode punctuation, URLs, hashtags and at-handles: everything
/// that is not a letter or digit is a separator, so "ai-first" and
/// "ai_first" and "#AI" all yield `ai`. Words that are pure URLs or pure
/// `@handle` tokens are dropped — a competitor's handle is a person, not a
/// topic.
fn split_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        // Drop whole tokens that are URLs or handles before splitting on
        // punctuation. Splitting first scatters a URL into "t", "co", "abc"
        // and an @handle into "niche_author", which then read as topics.
        .filter(|tok| {
            // Classify on the RAW token: trimming the leading `@` first
            // would make the handle test below never fire.
            !tok.starts_with("http") && !tok.starts_with("www") && !tok.starts_with('@')
        })
        .flat_map(|tok| {
            tok.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|w| !is_stop_word(w))
        .collect()
}

/// A repeated word in competitor posts that the founder has never used.
///
/// Both sides are tokenized with the same [`split_words`] rule, so a gap is
/// a difference in vocabulary and not a difference in punctuation.
// `min_count` requires the word to appear in at least that many competitor
// posts, which is what separates "a topic the niche keeps returning to"
// from one competitor's one-off phrasing.
fn content_gaps(
    competitor_texts: &[&str],
    own_texts: &[String],
    min_word_length: usize,
    min_count: usize,
    limit: usize,
) -> Vec<ContentGap> {
    use std::collections::HashMap;

    let own_words: std::collections::HashSet<String> = own_texts
        .iter()
        .flat_map(|t| split_words(t))
        .filter(|w| w.chars().count() >= min_word_length)
        .collect();

    // word -> (posts that used it, first example text)
    let mut counts: HashMap<String, (i64, Option<String>)> = HashMap::new();
    for text in competitor_texts {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for word in split_words(text) {
            if word.chars().count() < min_word_length {
                continue;
            }
            if own_words.contains(&word) {
                continue;
            }
            if seen.insert(word.clone()) {
                let entry = counts.entry(word).or_insert((0, None));
                entry.0 += 1;
                if entry.1.is_none() {
                    entry.1 = Some((*text).to_string());
                }
            }
        }
    }

    let mut gaps: Vec<ContentGap> = counts
        .into_iter()
        .filter(|(_, (posts, _))| *posts >= min_count as i64)
        .map(|(word, (posts, example))| ContentGap {
            word,
            external_posts: posts,
            example,
        })
        .collect();

    gaps.sort_by(|a, b| {
        b.external_posts
            .cmp(&a.external_posts)
            .then_with(|| a.word.cmp(&b.word))
    });
    gaps.truncate(limit);
    gaps
}

// ── Handler ────────────────────────────────────────────────────

/// GET /api/research?days=90&limit=10&min_word_length=4
///
/// Aggregated, read-only. Returns four sections the founder can act on, all
/// computed from stored data: their own best-performing posts, recent
/// competitor posts ranked by engagement, when their own posts performed
/// best, and topics the niche uses that they have not. Plus the titles of
/// their connected RSS feeds.
///
/// A section with no stored data comes back empty rather than filled with a
/// guess — the frontend renders a real empty state per section that names
/// what is missing.
pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<ResearchQuery>,
) -> Result<Json<ResearchResponse>, AppError> {
    // The window is capped at TWO YEARS.
    //
    // It used to accept 3650 (ten years), and the content-gaps section
    // reads every external post in the window into memory — so a ten-year
    // window on a well-fed feed loaded the whole corpus, on the single
    // writer, to compute word frequencies. Two years is more than the
    // feature needs (a topic the niche returns to across years is visible
    // in the ranked sections too).
    const MAX_RESEARCH_WINDOW_DAYS: i32 = 730;
    let days = i64::from(query.days.clamp(1, MAX_RESEARCH_WINDOW_DAYS));
    let limit = query.limit.clamp(1, 50);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);

    // (a) Own posts ranked by engagement.
    let own_rows: Vec<OwnPostEngagement> =
        queries::top_own_posts_by_engagement(&state.db, auth.user_id, cutoff, limit).await?;
    let own_count = own_rows.len() as i64;
    let top_own_posts: Vec<OwnPostRow> = own_rows
        .iter()
        .map(|p| OwnPostRow {
            post_id: p.post_id,
            content: p.content.clone(),
            provider: p.provider.clone(),
            published_at: p.published_at.map(|d| d.to_rfc3339()),
            engagement: p.engagement,
        })
        .collect();

    // (b) External/competitor posts ranked by engagement.
    let external_rows: Vec<ExternalPostEngagement> =
        queries::top_external_posts_by_engagement(&state.db, auth.user_id, cutoff, limit).await?;
    let external_count = external_rows.len() as i64;
    let top_external_posts: Vec<ExternalPostRow> = external_rows
        .iter()
        .map(|p| ExternalPostRow {
            post_id: p.post_id,
            provider: p.provider.clone(),
            text: p.text.clone(),
            author_name: p.author_name.clone(),
            author_handle: p.author_handle.clone(),
            url: p.url.clone(),
            published_at: p.created_at.to_rfc3339(),
            engagement: p.engagement,
        })
        .collect();

    // (c) When the founder's own posts performed best.
    //
    // Reuses the same histogram `handle_posts_best_times` backs, so the
    // dashboard and the MCP tool cannot disagree - the fold is THE
    // SAME `week_slot` rule (the histogram returns per-epoch rows and
    // each surface folds them itself, in its own zone). Unlike the MCP
    // tool this also carries engagement_per_post, which is the number
    // a founder needs to compare slots of different volumes.
    let timing: Option<TimingInsight> = {
        // The zone is parsed ONCE here and the slot label names it: a
        // "post Mon 15:00" hint is only actionable in the zone the
        // founder posts from, and it used to be labelled UTC while
        // the composer and the calendar beside it spoke local time.
        let zone = query
            .tz
            .as_deref()
            .map(crate::api::calendar::parse_zone)
            .transpose()
            .map_err(AppError::BadRequest)?;
        let zone_label = zone
            .map(|z| z.name().to_string())
            .unwrap_or_else(|| "UTC".into());
        // Windowed to the same `days` as the rest of the response, so the
        // timing insight describes the same period the rankings do.
        let rows =
            queries::engagement_histogram_by_slot(&state.db, auth.user_id, None, Some(cutoff))
                .await
                .unwrap_or_default();
        // Fold per-epoch rows into (weekday, hour) slots through the ONE
        // rule, summing posts and engagement per slot.
        let mut slot_map: std::collections::HashMap<(i32, i32), (i64, i64)> =
            std::collections::HashMap::new();
        for (_, epoch, posts, engagement) in rows {
            if let Some(slot) = week_slot(epoch, zone) {
                let e = slot_map.entry(slot).or_insert((0, 0));
                e.0 += posts;
                e.1 += engagement;
            }
        }
        let posts_analyzed: i64 = slot_map.values().map(|(p, _)| p).sum();
        (!slot_map.is_empty()).then(|| {
            let mut ranked: Vec<PostingSlot> = slot_map
                .into_iter()
                .map(|((weekday, hour), (posts, engagement))| {
                    let wd = WEEKDAYS[weekday as usize];
                    let hour_range = format!("{hour:02}:00–{:02}:00", (hour + 1) % 24);
                    let label = format!("{wd} {hour_range} {zone_label}");
                    PostingSlot {
                        weekday: wd.to_string(),
                        hour,
                        label,
                        hour_range,
                        posts,
                        engagement,
                        engagement_per_post: engagement as f64 / posts as f64,
                    }
                })
                .collect();
            ranked.sort_by(|a, b| {
                b.engagement_per_post
                    .partial_cmp(&a.engagement_per_post)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.engagement.cmp(&a.engagement))
            });
            ranked.truncate(3);
            // Fewer than three posts is too thin to call a pattern; the
            // frontend still shows the slots, flagged as low confidence.
            TimingInsight {
                low_confidence: posts_analyzed < 3,
                posts_analyzed,
                best_slots: ranked,
            }
        })
    };

    // (d) Content gaps — topics the niche uses that the founder has not.
    //
    // Both sides are deliberately wider than the ranked sections above.
    //
    // The competitor side reads every external post in the window, not just
    // the top-`limit` ones: a topic the niche returns to across many
    // mid-performing posts is a stronger signal than a topic one viral post
    // mentioned once, and reading only the top slice would hide it.
    //
    // The founder's side reads every post they have ever written, not just
    // the window: a topic they covered a year ago is still covered, and
    // windowing it would resurrect that topic as a false gap.
    // The corpus is CAPPED. Word frequencies over the most recent rows
    // answer the question ("what does the niche talk about that the
    // founder does not?") — loading every row the founder has ever
    // written only made the request slower and the memory larger.
    const MAX_CORPUS_ROWS: i64 = 5_000;
    let competitor_texts: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT text FROM external_posts
          WHERE user_id = ? AND hidden_at IS NULL AND created_at >= ?
          ORDER BY created_at DESC LIMIT ?",
    )
    .bind(auth.user_id)
    .bind(crate::db::types::EpochUtc::from(cutoff))
    .bind(MAX_CORPUS_ROWS)
    .fetch_all(&state.db)
    .await?;
    let own_texts: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT content FROM posts WHERE user_id = ? AND deleted_at IS NULL
          ORDER BY created_at DESC LIMIT ?",
    )
    .bind(auth.user_id)
    .bind(MAX_CORPUS_ROWS)
    .fetch_all(&state.db)
    .await?;
    let competitor_refs: Vec<&str> = competitor_texts.iter().map(String::as_str).collect();
    let content_gaps = content_gaps(
        &competitor_refs,
        &own_texts,
        query.min_word_length.max(2),
        2,
        20,
    );

    // Connected-feed items. Titles only — rss_posts carries no engagement,
    // so there is nothing honest to rank it by.
    let feed_items: Vec<FeedItemRow> = sqlx::query_as::<_, (Uuid, String, String, Option<i64>)>(
        r#"SELECT rp.id, rp.title, rp.url,
                  CAST(strftime('%s', rp.published_at) AS INTEGER) AS published_epoch
             FROM rss_posts rp
             JOIN rss_feeds rf ON rp.feed_id = rf.id
            WHERE rf.user_id = ? AND rp.is_imported = 0
            ORDER BY rp.published_at DESC NULLS LAST
            LIMIT 20"#,
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(id, title, url, published_epoch)| FeedItemRow {
        id,
        title,
        url,
        published_at: published_epoch.map(|e| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(e, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default()
        }),
    })
    .collect();

    Ok(Json(ResearchResponse {
        sources: SourceCounts {
            own_posts: own_count,
            external_posts: external_count,
            feed_items: feed_items.len() as i64,
        },
        top_own_posts,
        top_external_posts,
        timing,
        content_gaps,
        connected_feed_items: feed_items,
        window_days: query.days.clamp(1, 3650),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot fold is zone-aware: a post at 2026-10-25T02:00:00Z is
    /// Saturday 22:00 in New York but Sunday 02:00 in UTC. The research
    /// page sends the zone name, so the founder reads the slot in the
    /// zone they post from - and the weekday and the hour come from ONE
    /// fold, so they cannot disagree at the midnight boundary.
    #[test]
    fn week_slot_folds_the_weekday_and_hour_in_the_founders_zone() {
        let ny = crate::api::calendar::parse_zone("America/New_York").unwrap();
        let epoch = chrono::DateTime::parse_from_rfc3339("2026-10-25T02:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
            .timestamp();
        // UTC fold: Sunday (%w=0) 02:00.
        assert_eq!(week_slot(epoch, None), Some((0, 2)));
        // New York fold: Saturday (%w=6) 22:00, EDT.
        assert_eq!(week_slot(epoch, Some(ny)), Some((6, 22)));
    }

    #[test]
    fn split_words_drops_punctuation_urls_and_handles() {
        let words = split_words("AI-first workflows: #robotics https://t.co/abc via @niche_author");
        assert!(words.contains(&"ai".to_string()));
        assert!(words.contains(&"first".to_string()));
        assert!(words.contains(&"workflows".to_string()));
        assert!(words.contains(&"robotics".to_string()));
        assert!(
            words
                .iter()
                .all(|w| !w.contains("t.co") && !w.starts_with("niche")),
            "URLs and handles must not surface as topics"
        );
    }

    #[test]
    fn split_words_lowercases_and_keeps_digits() {
        assert_eq!(
            split_words("Rust vs. RustLang2026"),
            vec!["rust", "rustlang2026"]
        );
    }

    #[test]
    fn gap_needs_the_word_in_at_least_two_competitor_posts() {
        let competitor = [
            "ai robotics is here",
            "robotics again",
            "something else entirely",
        ];
        // `robotics` is in two posts, `something` in one.
        let gaps = content_gaps(&competitor, &[], 4, 2, 20);
        assert!(gaps.iter().any(|g| g.word == "robotics"));
        assert!(!gaps.iter().any(|g| g.word == "something"));
    }

    #[test]
    fn word_the_founder_already_used_is_not_a_gap() {
        let competitor = ["ai robotics is here", "robotics again"];
        let gaps = content_gaps(
            &competitor,
            &["we shipped a robotics post".into()],
            4,
            2,
            20,
        );
        assert!(!gaps.iter().any(|g| g.word == "robotics"));
    }

    #[test]
    fn short_words_are_filtered_by_min_length() {
        let competitor = ["ai this", "ai that", "ai again"];
        let gaps = content_gaps(&competitor, &[], 4, 2, 20);
        assert!(!gaps.iter().any(|g| g.word == "ai"));
    }

    #[test]
    fn gaps_carry_a_representative_competitor_example() {
        let competitor = ["ai robotics is here", "robotics again"];
        let gaps = content_gaps(&competitor, &[], 4, 2, 20);
        let robotics = gaps
            .iter()
            .find(|g| g.word == "robotics")
            .expect("robotics is a gap");
        assert_eq!(robotics.example.as_deref(), Some("ai robotics is here"));
        assert_eq!(robotics.external_posts, 2);
    }

    #[test]
    fn empty_competitors_yields_no_gaps() {
        assert!(content_gaps(&[], &["anything".into()], 4, 2, 20).is_empty());
    }
}
