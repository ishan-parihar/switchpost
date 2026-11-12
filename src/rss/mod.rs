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
            let title = entry.title.map(|t| t.content).unwrap_or_default();
            let url = entry
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default();
            let guid = entry.id;
            let published = entry
                .published
                .and_then(|d| chrono::DateTime::from_timestamp(d.timestamp(), 0));
            let content = entry
                .content
                .and_then(|c| c.body)
                .or_else(|| entry.summary.map(|s| s.content))
                .unwrap_or_default();

            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            let content_hash = format!("{:x}", hasher.finalize());

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
