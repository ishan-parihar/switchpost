pub mod analytics;
pub mod bluesky;
pub mod devto;
pub mod discord;
pub mod drive;
pub mod facebook;
pub mod gcal;
pub mod github;
pub mod gmail;
pub mod google;
pub mod hashnode;
pub mod instagram;
pub mod linkedin;
pub mod linkedin_page;
pub mod mastodon;
pub mod medium_blog;
pub mod notifications;
pub mod pinterest;
pub mod reddit;
pub mod sets;
pub mod skool;
pub mod slack;
pub mod tags;
pub mod telegram_bot;
pub mod telegram_user;
pub mod threads;
pub mod tiktok;
pub mod webhooks;
pub mod whatsapp;
pub mod wordpress;
pub mod x;
pub mod youtube;

use serde::Serialize;

use super::run::{render_output, truncate_json_strings};

/// Maximum chars for string values in list outputs (AXI §3: truncation).
const LIST_TRUNCATE_CHARS: usize = 500;

/// Render what emit_result would print, without printing — so a test
/// can pin that the GLOBAL --json/--pretty flags reach the platform
/// shims' output. emit_result itself hardcodes TOON once, and every
/// `switchpost x timeline`-style command silently ignored the flags.
pub(crate) fn render_emit<T: Serialize>(result: &Result<T, String>) -> String {
    match result {
        Ok(v) => {
            let val = serde_json::to_value(v).unwrap_or_default();
            // AXI §3: truncate string fields in list-like outputs
            let truncated = truncate_json_strings(&val, LIST_TRUNCATE_CHARS);
            render_output(&truncated)
        }
        Err(e) => render_output(&serde_json::json!({
            "error": e,
            "hint": "Run `switchpost doctor` to check provider health."
        })),
    }
}

/// Shared result-printing footer for every CLI platform shim.
///
/// Uses the global output format (AXI §1: TOON default, --json/--pretty).
pub fn emit_result<T: Serialize>(result: Result<T, String>) -> anyhow::Result<()> {
    match &result {
        Ok(_) => {
            println!("{}", render_emit(&result));
            Ok(())
        }
        Err(_) => {
            println!("{}", render_emit(&result));
            std::process::exit(2);
        }
    }
}
