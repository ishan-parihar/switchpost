// ─── Reddit Browser Cookie Extraction ─────────────────────────
// Extracts reddit_session and token_v2 from Chrome, Brave, Firefox, Zen browsers.
//
// F3: the browser walk + DB reads live in browser_cookies.rs; this file is
// only the Reddit spec, the rows→result mapping, and the DB token-blob API.

use super::browser_cookies::{extract_from_browsers, CookieRow, CookieSpec};

/// Result of browser cookie extraction
#[derive(Debug)]
pub struct ExtractedRedditCookies {
    pub reddit_session: String,
    pub token_v2: Option<String>,
    pub cookie_string: String,
    pub source: String,
}

const REDDIT_SPEC: CookieSpec = CookieSpec {
    hosts: &["reddit.com"],
    names: &[
        "reddit_session",
        "token_v2",
        "csrf_token",
        "loid",
        "session_tracker",
        "edgebucket",
    ],
};

/// Try to extract Reddit cookies from all known browsers in order
/// (brave → chrome → zen → firefox; a browser counts only if its rows
/// yield reddit_session OR token_v2).
pub fn extract_reddit_cookies() -> Option<ExtractedRedditCookies> {
    let ((session, token_v2, cookie_string), source) =
        extract_from_browsers(&REDDIT_SPEC, "reddit_cookies", map_reddit_rows)?;
    Some(ExtractedRedditCookies {
        reddit_session: session,
        token_v2,
        cookie_string,
        source: source.to_string(),
    })
}

/// Reddit mapping: keep every wanted cookie in the header string; require
/// reddit_session OR token_v2.
fn map_reddit_rows(rows: &[CookieRow]) -> Option<(String, Option<String>, String)> {
    let mut reddit_session = String::new();
    let mut token_v2 = String::new();
    for (name, value, _) in rows {
        match name.as_str() {
            "reddit_session" => reddit_session = value.clone(),
            "token_v2" => token_v2 = value.clone(),
            _ => {}
        }
    }
    if reddit_session.is_empty() && token_v2.is_empty() {
        return None; // insufficient rows — walk continues
    }

    let cookie_string = rows
        .iter()
        .map(|(k, v, _)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    Some((
        reddit_session,
        if token_v2.is_empty() {
            None
        } else {
            Some(token_v2)
        },
        cookie_string,
    ))
}

/// Build the JSON token blob for storage in DB
pub fn build_cookie_token(
    reddit_session: &str,
    token_v2: Option<&str>,
    cookie_string: Option<&str>,
) -> String {
    let mut m = serde_json::Map::new();
    m.insert(
        "reddit_session".into(),
        serde_json::Value::String(reddit_session.into()),
    );
    if let Some(t) = token_v2 {
        m.insert("token_v2".into(), serde_json::Value::String(t.into()));
    }
    if let Some(cs) = cookie_string {
        if !cs.is_empty() {
            m.insert("cookie_string".into(), serde_json::Value::String(cs.into()));
        }
    }
    serde_json::Value::Object(m).to_string()
}

/// Parse a cookie token JSON blob back into components
pub fn parse_cookie_token(token: &str) -> Option<(String, Option<String>, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(token).ok()?;
    let session = v.get("reddit_session")?.as_str()?.to_string();
    if session.is_empty() {
        return None;
    }
    let token_v2 = v.get("token_v2").and_then(|s| s.as_str()).map(String::from);
    let cookie_string = v
        .get("cookie_string")
        .and_then(|s| s.as_str())
        .map(String::from);
    Some((session, token_v2, cookie_string))
}

/// Check if a token string is a Reddit cookie auth JSON blob
pub fn is_cookie_auth(token: &str) -> bool {
    parse_cookie_token(token).is_some()
}

/// Parse a raw cookie paste into Reddit's components.
///
/// THE PARSE GOES THROUGH THE SHARED `cookie_paste` PARSER (§9: one
/// implementation per rule, on both sides of the wire). The old body was a
/// bare `split(';')` + `split_once('=')` loop, which failed on every paste
/// shape a browser's devtools actually emits — newline lists, the JSON
/// "Copy as JSON" export, a pasted `Cookie:` header, quoted values and
/// percent-encoded names. The failure was not a clear refusal: a JSON blob
/// that CONTAINED `reddit_session` passed the old `reddit_session?` guard
/// with a session value that was really `[{`, and the handler then answered
/// a 502 that a reverse proxy rewrites into an HTML error page.
///
/// The validation header is REBUILT from the parsed names, so the upstream
/// request carries exactly the cookies Reddit needs and nothing else — a
/// paste that arrives wrapped in JSON never sends JSON upstream.
pub fn parse_cookie_string(cookie_str: &str) -> Option<ExtractedRedditCookies> {
    use crate::social::cookie_paste::{parse_cookie_paste, rebuild_cookie_header};

    let paste = parse_cookie_paste(cookie_str)?;

    let reddit_session = paste
        .cookies
        .get("reddit_session")
        .filter(|v| !v.is_empty())
        .cloned()
        .or_else(|| {
            // `token_v2` alone is a valid Reddit auth: the browser walk
            // accepted it (`map_reddit_rows`), so a manual paste must too.
            paste.cookies.get("token_v2").filter(|v| !v.is_empty()).cloned()
        })?;

    let token_v2 = paste.cookies.get("token_v2").cloned();

    // Rebuild the header from what we actually parsed, keeping every cookie
    // Reddit's own state may read (the browser walk's `REDDIT_SPEC` names
    // plus the two auth cookies) rather than echoing the raw paste.
    let wanted = &[
        "reddit_session",
        "token_v2",
        "csrf_token",
        "loid",
        "session_tracker",
        "edgebucket",
    ];
    let cookie_string = rebuild_cookie_header(&paste.cookies, wanted);

    Some(ExtractedRedditCookies {
        reddit_session,
        token_v2,
        cookie_string,
        source: format!("manual ({})", paste.format.label()),
    })
}

#[cfg(test)]
mod f3_tests {
    use super::*;

    /// Reddit mapping: requires session OR token_v2; keeps every row in
    /// the header string; incomplete rows are refused so the walk moves on.
    #[test]
    fn reddit_mapping_requires_session_or_token() {
        let rows: Vec<CookieRow> = vec![
            ("reddit_session".into(), "s1".into(), ".reddit.com".into()),
            ("loid".into(), "loid-1".into(), ".reddit.com".into()),
        ];
        let (session, token_v2, cookie_string) =
            map_reddit_rows(&rows).expect("session alone suffices");
        assert_eq!(session, "s1");
        assert_eq!(token_v2, None);
        assert_eq!(cookie_string, "reddit_session=s1; loid=loid-1");

        let rows2: Vec<CookieRow> = vec![("loid".into(), "loid-1".into(), ".reddit.com".into())];
        assert!(
            map_reddit_rows(&rows2).is_none(),
            "rows with neither session nor token_v2 must be refused"
        );
    }
}
