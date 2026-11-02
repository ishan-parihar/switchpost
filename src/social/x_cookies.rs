// ─── X/Twitter Browser Cookie Extraction ──────────────────────
// Extracts auth_token and ct0 from Chrome, Brave, Firefox, Zen browsers.
// Firefox/Zen: reads plaintext cookies from SQLite store — works.
// Chrome/Brave: cookie values are OS-encrypted (v10/v11); the value column
// is empty and this build does not decrypt them, so those browsers return
// None (B12: never fabricate a token from ciphertext).
//
// F3: the browser walk + DB reads live in browser_cookies.rs; this file is
// only the X spec and the rows→result mapping.

use super::browser_cookies::{extract_from_browsers, CookieRow, CookieSpec};

/// Result of browser cookie extraction
#[derive(Debug)]
pub struct ExtractedCookies {
    pub auth_token: String,
    pub ct0: String,
    pub cookie_string: String,
    pub source: String,
}

const X_SPEC: CookieSpec = CookieSpec {
    hosts: &["x.com", ".twitter.com", "twitter.com"],
    names: &[
        "auth_token",
        "ct0",
        "guest_id",
        "kdt",
        "twid",
        "lang",
        "personalization_id",
    ],
};

/// Try to extract X cookies from all known browsers in order
/// (brave → chrome → zen → firefox; a browser counts only if its rows
/// yield BOTH auth_token and ct0).
pub fn extract_x_cookies() -> Option<ExtractedCookies> {
    let ((auth_token, ct0, cookie_string), source) =
        extract_from_browsers(&X_SPEC, "x_cookies", map_x_rows)?;
    Some(ExtractedCookies {
        auth_token,
        ct0,
        cookie_string,
        source: source.to_string(),
    })
}

/// X mapping: prefer rows whose host is x.com (fall back to twitter.com),
/// require auth_token AND ct0 from the chosen domain bucket.
fn map_x_rows(rows: &[CookieRow]) -> Option<(String, String, String)> {
    let all_refs: Vec<&CookieRow> = rows.iter().collect();
    let x_domain: Vec<&CookieRow> = rows
        .iter()
        .filter(|(_, _, h)| h.contains("x.com"))
        .collect();
    let chosen: &[&CookieRow] = if x_domain.is_empty() {
        &all_refs
    } else {
        &x_domain
    };

    let mut auth_token = String::new();
    let mut ct0 = String::new();
    for (name, value, _) in chosen {
        match name.as_str() {
            "auth_token" => auth_token = value.clone(),
            "ct0" => ct0 = value.clone(),
            _ => {}
        }
    }
    if auth_token.is_empty() || ct0.is_empty() {
        return None; // this browser's rows are insufficient — walk continues
    }

    let cookie_string = chosen
        .iter()
        .map(|(k, v, _)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    Some((auth_token, ct0, cookie_string))
}

/// Parse a manually pasted cookie header string into
/// `(auth_token, ct0, cookie_string)` — the tuple shape every caller
/// (onboard paste flow, CLI, setup tools) destructures positionally.
pub fn parse_cookie_string(cookie_str: &str) -> Option<(String, String, String)> {
    // THE SHARED PARSER, not a second `split(';')` loop. X's cookie paste is
    // the same surface as Reddit's and had the same blind spot: a devtools
    // JSON export or a newline-separated list yielded no `auth_token`, and the
    // caller answered "missing cookie" with no hint that the paste had
    // actually arrived in a shape it could have read.
    use crate::social::cookie_paste::{parse_cookie_paste, rebuild_cookie_header};

    let paste = parse_cookie_paste(cookie_str)?;

    let auth_token = paste.cookies.get("auth_token").cloned()?;
    let ct0 = paste.cookies.get("ct0").cloned()?;

    // Rebuild from the parsed pair, not the raw paste: the upstream request
    // carries exactly the two cookies X needs, so a quoted or JSON-wrapped
    // paste does not send its wrapper to X.
    let cookie_string = rebuild_cookie_header(&paste.cookies, &["auth_token", "ct0"]);

    Some((auth_token, ct0, cookie_string))
}

/// Build the JSON token blob stored in the integration row
/// (`{auth_token, ct0[, cookie_string]}` — the shape XProvider's
/// `parse_cookie_token`/`extract_cookie_string` read back).
pub fn build_cookie_token(auth_token: &str, ct0: &str, cookie_string: Option<&str>) -> String {
    let mut m = serde_json::Map::new();
    m.insert(
        "auth_token".into(),
        serde_json::Value::String(auth_token.into()),
    );
    m.insert("ct0".into(), serde_json::Value::String(ct0.into()));
    if let Some(cs) = cookie_string {
        if !cs.is_empty() {
            m.insert("cookie_string".into(), serde_json::Value::String(cs.into()));
        }
    }
    serde_json::Value::Object(m).to_string()
}

#[cfg(test)]
mod b12_tests {
    use super::*;
    use crate::social::browser_cookies::read_chromium_cookies;

    fn chrome_cookie_db(tag: &str, rows: &[(&str, &str, Option<&[u8]>)]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("x_cookies_b12_{tag}_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE cookies (name TEXT, value TEXT, encrypted_value BLOB, host_key TEXT, path TEXT)",
            [],
        )
        .unwrap();
        for (name, value, enc) in rows {
            conn.execute(
                "INSERT INTO cookies (name, value, encrypted_value, host_key, path)
                 VALUES (?1, ?2, ?3, '.x.com', '/')",
                rusqlite::params![name, value, enc.map(<[u8]>::to_vec).unwrap_or_default()],
            )
            .unwrap();
        }
        path
    }

    /// B12 regression: a real Chromium cookie DB stores the session value
    /// encrypted with an empty `value` column. The extractor must refuse
    /// (no rows) — previously it minted from_utf8_lossy(ciphertext) as the
    /// auth_token, which "connected" and then failed every call with a
    /// misleading 401.
    #[test]
    fn b12_encrypted_only_chromium_rows_are_refused_not_fabricated() {
        let db = chrome_cookie_db(
            "refuse",
            &[
                (
                    "auth_token",
                    "",
                    Some(&[0x76, 0x31, 0x30, 0xAA, 0xBB, 0xCC]),
                ),
                ("ct0", "plain-ct0", None),
            ],
        );
        let rows = read_chromium_cookies(&db, &X_SPEC).unwrap_or_default();
        // The encrypted auth_token row must be skipped — its value must
        // never appear as a fabricated plaintext token.
        assert!(
            !rows.iter().any(|(n, _, _)| n == "auth_token"),
            "encrypted-only Chromium cookie must be skipped, never fabricated"
        );
        // And with no auth_token in the surviving rows, the X mapping
        // refuses — the walk moves on to the next browser.
        assert!(
            map_x_rows(&rows).is_none(),
            "no fabricated auth_token → mapping must refuse"
        );
        let _ = std::fs::remove_file(&db);
    }

    /// Positive control: plaintext rows read, and the X mapping accepts a
    /// complete pair / rejects an incomplete one (the retry semantics the
    /// browser walk relies on).
    #[test]
    fn b12_plaintext_chromium_rows_still_read_and_map() {
        let db = chrome_cookie_db(
            "plain",
            &[
                ("auth_token", "real-token", None),
                ("ct0", "real-ct0", None),
            ],
        );
        let rows = read_chromium_cookies(&db, &X_SPEC).expect("plaintext rows must read");
        let (at, ct0, cookie_str) = map_x_rows(&rows).expect("complete pair must map");
        assert_eq!(at, "real-token");
        assert_eq!(ct0, "real-ct0");
        assert!(cookie_str.contains("auth_token=real-token"));

        // Incomplete rows (no ct0) must be refused so the walk continues.
        let db2 = chrome_cookie_db("incomplete", &[("auth_token", "real-token", None)]);
        let rows2 = read_chromium_cookies(&db2, &X_SPEC).expect("rows read");
        assert!(
            map_x_rows(&rows2).is_none(),
            "rows without ct0 must not map — next browser must be tried"
        );

        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_file(&db2);
    }

    /// Domain preference: rows are bucketed by host — x.com rows win over
    /// twitter.com rows when both exist.
    #[test]
    fn x_rows_prefer_x_com_domain() {
        let rows: Vec<CookieRow> = vec![
            (
                "auth_token".into(),
                "from-twitter".into(),
                ".twitter.com".into(),
            ),
            ("ct0".into(), "tw-ct0".into(), ".twitter.com".into()),
            ("auth_token".into(), "from-x".into(), ".x.com".into()),
            ("ct0".into(), "x-ct0".into(), ".x.com".into()),
        ];
        let (at, ct0, cookie_str) = map_x_rows(&rows).expect("x.com bucket must win");
        assert_eq!(at, "from-x");
        assert_eq!(ct0, "x-ct0");
        assert_eq!(cookie_str, "auth_token=from-x; ct0=x-ct0");
    }
}
