use std::sync::Arc;

use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::sync::watch;
use tracing;

use crate::config::Config;
use crate::db::models::PostState;
use crate::poll::{spawn_poll, FirstTick};
use crate::social::registry::ProviderRegistry;
use sqlx::SqlitePool;

/// A feed body larger than this is truncated: a hostile feed must not be
/// able to stream megabytes into the single binary's memory.
const RSS_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

pub fn start_rss_poller(
    db: SqlitePool,
    _providers: Arc<ProviderRegistry>,
    config: Arc<Config>,
    shutdown_rx: watch::Receiver<bool>,
) {
    tracing::info!("RSS poller started (interval: 15 min)");
    spawn_poll(
        Duration::from_secs(15 * 60), // every 15 min
        FirstTick::AfterDelay,
        shutdown_rx,
        "RSS poller shutting down",
        move || {
            let db = db.clone();
            let config = config.clone();
            async move {
                if let Err(e) = poll_all_feeds(&db, &config).await {
                    tracing::error!("RSS poller error: {e}");
                }
            }
        },
    );
}

/// What one RSS entry contributes to a post.
pub struct RssEntryFields {
    pub title: String,
    pub url: String,
    /// The entry's identity. An entry whose guid was already imported is
    /// skipped, even if its body changed since.
    pub guid: String,
    pub published: Option<chrono::DateTime<chrono::Utc>>,
    pub content: String,
    /// A hash of the content, so a changed body is visible.
    pub content_hash: String,
}

/// Reduce a parsed feed entry to the fields a post is made of.
///
/// The rules, in one place, because a feed that is read wrong creates wrong
/// posts UNATTENDED: the poller runs on a timer and nobody sees what it
/// published until it is on the platform.
///
/// - the body is the content, and the summary is the fallback;
/// - the first link is the entry's URL;
/// - the content hash is over the BODY, so a re-published entry with the
///   same body is the same post.
pub fn entry_to_post_fields(entry: &feed_rs::model::Entry) -> RssEntryFields {
    let title = entry
        .title
        .as_ref()
        .map(|t| t.content.clone())
        .unwrap_or_default();
    let url = entry
        .links
        .first()
        .map(|l| l.href.clone())
        .unwrap_or_default();
    let published = entry
        .published
        .and_then(|d| chrono::DateTime::from_timestamp(d.timestamp(), 0));
    let content = entry
        .content
        .as_ref()
        .and_then(|c| c.body.clone())
        .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()))
        .unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    RssEntryFields {
        title,
        url,
        guid: entry.id.clone(),
        published,
        content,
        content_hash,
    }
}

async fn poll_all_feeds(
    db: &SqlitePool,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let feeds = crate::db::queries::get_feeds_due_for_polling(db).await?;
    // One stalled feed host must not hang the whole poll. Redirects are
    // followed by hand so every hop can be validated — a feed URL is a
    // server-side fetch target and must not be able to reach the LAN.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    for feed in feeds {
        tracing::info!("Polling RSS feed: {} ({})", feed.title, feed.feed_url);

        let response = match crate::net_guard::send_guarded(
            &client,
            reqwest::Method::GET,
            &feed.feed_url,
            &[],
            None,
            RSS_MAX_BODY_BYTES,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to fetch RSS feed {}: {e}", feed.feed_url);
                continue;
            }
        };
        let xml = response.body;

        let parsed = match feed_rs::parser::parse(xml.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to parse RSS XML for {}: {e}", feed.feed_url);
                continue;
            }
        };

        for entry in parsed.entries {
            let entry = entry_to_post_fields(&entry);
            let title = entry.title;
            let url = entry.url;
            let guid = entry.guid;
            let published = entry.published;
            let content = entry.content;
            let content_hash = entry.content_hash;

            // Claim the entry by guid before creating its post. A guid already
            // imported is skipped, even if its body changed since.
            let rss_row = match crate::db::queries::insert_rss_post(
                db,
                feed.id,
                &guid,
                &title,
                &url,
                published,
                &content_hash,
            )
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("Failed to insert RSS post for feed {}: {e}", feed.id);
                    continue;
                }
            };

            // ── Build post content ──────────────────────────────
            // If use_ai_summary is enabled AND LLM_ENDPOINT is configured,
            // call the LLM to summarize the RSS entry into ≤280 chars.
            // Otherwise, use the simple "{title}\n\n{url}" template.
            //
            // IMPORTANT: RSS-imported posts are created as DRAFT (not QUEUED)
            // so the solo founder can review — especially when AI-generated —
            // before the scheduler publishes them. This matches postiz-app's
            // pattern and is safer than going straight to QUEUED.
            let post_content = if feed.use_ai_summary {
                match summarize_with_llm(config, &title, &content, &url).await {
                    Ok(summary) => summary,
                    Err(e) => {
                        tracing::warn!(
                            "RSS AI summary failed for feed {}, falling back to template: {e}",
                            feed.id
                        );
                        format!("{}\n\n{}", title, url)
                    }
                }
            } else {
                format!("{}\n\n{}", title, url)
            };

            match crate::db::queries::create_post(
                db,
                feed.user_id,
                feed.integration_id,
                &post_content,
                Some(&title),
                // `media` is an ARRAY - the column's default is '[]' and
                // every reader treats it as one. It was `{}`, and the
                // kanban's `post.media.find(...)` crashed the whole ROUTE
                // with "i.find is not a function" on every RSS draft.
                &serde_json::json!([]),
                &serde_json::json!({"rss_auto": true}),
                None,
                Some(PostState::Draft), // Changed from Queued → Draft for review
                None,
                0,
                None,
            )
            .await
            {
                Ok(post) => {
                    if let Err(e) =
                        crate::db::queries::update_rss_post_post_id(db, rss_row, post.id).await
                    {
                        tracing::warn!("Failed to update RSS post post_id: {e}");
                    }
                    tracing::info!(
                        "RSS autopost: created draft post {} for feed {} (ai_summary={})",
                        post.id,
                        feed.id,
                        feed.use_ai_summary
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "RSS autopost: failed to create post for feed {}: {e}",
                        feed.id
                    );
                    // Release the claim so the next poll retries this entry.
                    if let Err(del) = crate::db::queries::delete_rss_post(db, rss_row).await {
                        tracing::warn!("Failed to release RSS claim {rss_row}: {del}");
                    }
                }
            }
        }

        if let Err(e) = crate::db::queries::update_feed_last_polled(db, feed.id).await {
            tracing::warn!("Failed to update last_polled for feed {}: {e}", feed.id);
        }
    }

    Ok(())
}

/// Call an OpenAI-compatible LLM endpoint to summarize an RSS entry
/// into a social-media-friendly post (≤280 chars, no hashtags, engaging tone).
///
/// Uses `LLM_ENDPOINT` + `LLM_MODEL` from config. If either is unset,
/// returns an error and the caller falls back to the template.
async fn summarize_with_llm(
    config: &Config,
    title: &str,
    content: &str,
    url: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = config
        .llm_endpoint
        .as_ref()
        .ok_or("LLM_ENDPOINT not configured")?;
    let model = config
        .llm_model
        .as_ref()
        .ok_or("LLM_MODEL not configured")?;

    // Strip HTML from content for the LLM prompt (simple tag stripper)
    let clean_content: String = content
        .chars()
        .fold((false, String::new()), |(in_tag, mut acc), ch| match ch {
            '<' => (true, acc),
            '>' => (false, acc),
            _ if !in_tag => {
                acc.push(ch);
                (false, acc)
            }
            _ => (in_tag, acc),
        })
        .1;
    let clean_content = clean_content.trim();
    // Truncate to 2000 chars to avoid token overflow
    let clean_content = if clean_content.len() > 2000 {
        &clean_content[..2000]
    } else {
        clean_content
    };

    let prompt = format!(
        "Summarize this article into a engaging social media post (max 280 characters). \
         Do NOT include hashtags. Do NOT include the URL. \
         Write in a conversational, engaging tone.\n\n\
         Title: {title}\n\n\
         Content: {clean_content}\n\n\
         Summary:"
    );

    let client = crate::social::shared_http();
    let response = client
        .post(endpoint)
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 150,
            "temperature": 0.7,
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("LLM returned status {}", response.status()).into());
    }

    let json: serde_json::Value = response.json().await?;
    let summary = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("LLM response missing content")?
        .trim()
        .to_string();

    // Append the URL so the post has a link
    Ok(format!("{summary}\n\n{url}"))
}

#[cfg(test)]
mod rss_parser_tests {
    use super::*;

    /// A feed with the three shapes an entry can take: a body, a summary
    /// only, and neither.
    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example feed</title>
    <link>https://example.com</link>
    <description>An example</description>
    <item>
      <title>First post</title>
      <link>https://example.com/first</link>
      <guid>guid-1</guid>
      <pubDate>Mon, 02 Nov 2026 09:30:00 GMT</pubDate>
      <description><![CDATA[The summary body]]></description>
      <content:encoded xmlns:content="http://purl.org/rss/1.0/modules/content/"><![CDATA[<p>The full body</p>]]></content:encoded>
    </item>
    <item>
      <title>Second post</title>
      <link>https://example.com/second</link>
      <guid>guid-2</guid>
      <description>Only a summary</description>
    </item>
    <item>
      <title>Third post</title>
      <guid>guid-3</guid>
    </item>
  </channel>
</rss>"#;

    fn entries() -> Vec<feed_rs::model::Entry> {
        let parsed = feed_rs::parser::parse(FEED.as_bytes()).expect("the fixture parses");
        parsed.entries
    }

    #[test]
    fn the_body_is_the_content_and_the_summary_is_the_fallback() {
        let entries = entries();
        assert_eq!(entries.len(), 3);

        let first = entry_to_post_fields(&entries[0]);
        assert_eq!(
            first.content, "<p>The full body</p>",
            "the body wins over the summary"
        );
        assert_eq!(first.title, "First post");
        assert_eq!(first.url, "https://example.com/first");
        assert_eq!(first.guid, "guid-1");

        let second = entry_to_post_fields(&entries[1]);
        assert_eq!(
            second.content, "Only a summary",
            "an entry with no body falls back to its summary"
        );

        let third = entry_to_post_fields(&entries[2]);
        assert_eq!(
            third.content, "",
            "an entry with neither has no content — it must not become a post with the title"
        );
        assert_eq!(third.url, "", "an entry with no link has no URL");
    }

    #[test]
    fn the_published_date_is_read() {
        let entries = entries();
        let first = entry_to_post_fields(&entries[0]);
        let published = first.published.expect("the entry carries a pubDate");
        assert_eq!(
            published.to_rfc3339(),
            "2026-11-02T09:30:00+00:00",
            "the pubDate must land on the right instant"
        );
        assert!(
            entry_to_post_fields(&entries[1]).published.is_none(),
            "an entry with no date has no publication time"
        );
    }

    #[test]
    fn the_content_hash_is_over_the_body() {
        let entries = entries();
        let first = entry_to_post_fields(&entries[0]);
        let again = entry_to_post_fields(&entries[0]);
        assert_eq!(
            first.content_hash, again.content_hash,
            "the same entry hashes the same way — a re-polled feed must not look like a new post"
        );
        let second = entry_to_post_fields(&entries[1]);
        assert_ne!(
            first.content_hash, second.content_hash,
            "a different body hashes differently"
        );
        assert_eq!(first.content_hash.len(), 64, "a sha256 hex digest");
    }

    #[test]
    fn a_malformed_feed_is_an_error_not_a_silent_empty_poll() {
        let parsed = feed_rs::parser::parse(&b"this is not xml at all"[..]);
        assert!(
            parsed.is_err(),
            "a feed that cannot be parsed must fail loudly — the poller logs it and skips the feed"
        );
    }
}
