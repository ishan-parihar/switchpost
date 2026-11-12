// ─── Shared Browser Cookie Utilities ────────────────────────────
// Common helpers for browser cookie extraction (Chrome, Brave, Firefox, Zen).
// F3: ONE extractor lives here, parameterised by a CookieSpec; the
// platform files (x_cookies.rs, reddit_cookies.rs) shrink to their spec
// plus a platform-specific rows→result mapping.

use std::path::{Path, PathBuf};

/// Get the user's home directory
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Build the default Chrome/Brave profile path
pub fn chrome_default_profile(home: &Path, config_dir: &str) -> PathBuf {
    home.join(".config").join(config_dir).join("Default")
}

// ── The shared extractor (F3) ───────────────────────────────────

/// What cookies to pull, and from which host suffixes.
pub struct CookieSpec {
    /// Host suffixes, each matched with `LIKE '%<host>'`; a row matching
    /// ANY is kept.
    pub hosts: &'static [&'static str],
    /// Cookie names to pull (the SQL IN list).
    pub names: &'static [&'static str],
}

/// One cookie row as read from a browser store: (name, value, host).
pub type CookieRow = (String, String, String);

fn host_clause(column: &str, hosts: &[&str]) -> String {
    hosts
        .iter()
        .map(|h| format!("{column} LIKE '%{h}'"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn in_clause(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Read a Chromium-family cookie DB copy. Plaintext rows only (B12):
/// real Chromium stores the value OS-encrypted with an empty `value`
/// column — those rows are SKIPPED, never fabricated, so an encrypted-only
/// DB yields `None` and the walk falls through to Firefox/Zen.
/// ponytail: implement v10/v11 PBKDF2-AES-128-CBC decrypt if Chromium
/// import ever becomes a real need (needs pbkdf2+sha1+cbc crates).
pub fn read_chromium_cookies(db_path: &Path, spec: &CookieSpec) -> Option<Vec<CookieRow>> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name, value, host_key FROM cookies WHERE ({}) AND name IN ({})",
            host_clause("host_key", spec.hosts),
            in_clause(spec.names),
        ))
        .ok()?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        if !row.1.is_empty() {
            out.push(row);
        }
    }
    // An all-encrypted DB yields nothing usable — None, not an empty list,
    // so callers (and the B12 regression) treat it as "no cookies here".
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Read a Firefox/Zen `moz_cookies` copy — plaintext values.
pub fn read_mozilla_cookies(db_path: &Path, spec: &CookieSpec) -> Option<Vec<CookieRow>> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name, value, host FROM moz_cookies WHERE ({}) AND name IN ({})",
            host_clause("host", spec.hosts),
            in_clause(spec.names),
        ))
        .ok()?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        if !row.1.is_empty() {
            out.push(row);
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Mozilla-family profile discovery: `profiles.ini` `Path=` entries with a
/// plain directory-scan fallback (covers Zen, which has no .mozilla root).
fn mozilla_profiles(root: &Path) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    let ini = root.join("profiles.ini");
    if let Ok(content) = std::fs::read_to_string(&ini) {
        for line in content.lines() {
            if let Some(path) = line.strip_prefix("Path=") {
                let p = root.join(path);
                if p.join("cookies.sqlite").exists() {
                    profiles.push(p);
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("cookies.sqlite").exists() && !profiles.contains(&path) {
                profiles.push(path);
            }
        }
    }
    profiles
}

/// Walk installed browsers in priority order (brave → chrome → zen →
/// firefox) and return the first source whose rows satisfy the
/// platform-specific `map`. `map` returning `None` moves on to the next
/// browser/profile — e.g. X requires auth_token AND ct0, so a browser
/// holding only `guest_id` is skipped, not accepted.
///
/// Browsers are located in this order:
/// - Brave (three known Linux profile-dir spellings), then Chrome —
///   Chromium family, `read_chromium_cookies`.
/// - Zen (`~/.zen`), then Firefox (`~/.mozilla/firefox`) — Mozilla
///   family, `read_mozilla_cookies`.
pub fn extract_from_browsers<T>(
    spec: &CookieSpec,
    tmp_prefix: &str,
    map: impl Fn(&[CookieRow]) -> Option<T>,
) -> Option<(T, &'static str)> {
    let home = home_dir();

    let mut chromium: Vec<(&'static str, PathBuf)> = vec![
        (
            "brave",
            chrome_default_profile(&home, "BraveSoftware/Brave-Browser"),
        ),
        (
            "brave",
            chrome_default_profile(&home, "BraveSoftware/Brave-Origin"),
        ),
        (
            "brave",
            chrome_default_profile(&home, "BraveSoftware/Brave-Origin-Beta"),
        ),
        ("chrome", chrome_default_profile(&home, "google-chrome")),
    ];

    for (source, profile) in chromium.drain(..) {
        let db = profile.join("Cookies");
        if !db.exists() {
            continue;
        }
        let tmp = std::env::temp_dir().join(format!("{tmp_prefix}_{source}.db"));
        let _ = std::fs::remove_file(&tmp);
        if std::fs::copy(&db, &tmp).is_err() {
            continue;
        }
        let mapped = read_chromium_cookies(&tmp, spec).and_then(|rows| map(&rows));
        let _ = std::fs::remove_file(&tmp);
        if let Some(result) = mapped {
            return Some((result, source));
        }
    }

    let zen_root = home.join(".zen");
    let firefox_root = home.join(".mozilla").join("firefox");
    for (source, root) in [("zen", &zen_root), ("firefox", &firefox_root)] {
        if !root.exists() {
            continue;
        }
        for profile in mozilla_profiles(root) {
            let db = profile.join("cookies.sqlite");
            if !db.exists() {
                continue;
            }
            let tmp = std::env::temp_dir().join(format!("{tmp_prefix}_{source}.db"));
            let _ = std::fs::remove_file(&tmp);
            if std::fs::copy(&db, &tmp).is_err() {
                continue;
            }
            let mapped = read_mozilla_cookies(&tmp, spec).and_then(|rows| map(&rows));
            let _ = std::fs::remove_file(&tmp);
            if let Some(result) = mapped {
                return Some((result, source));
            }
        }
    }

    None
}
