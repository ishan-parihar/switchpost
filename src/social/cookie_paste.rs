// ─── Shared Cookie-Paste Parsing ───────────────────────────────
//
// ONE parser for every "paste your cookies here" surface. The X and Reddit
// parsers each had their own `split(';')` loop, and both of them refused
// real-world pastes.
//
// ── THE DEFECT THIS CLOSES ──────────────────────────────────────
//
// A founder copying cookies out of devtools does not paste the format the
// parsers assumed. Every one of these is a paste a human actually produces,
// and the old `split(';')` + `split_once('=')` shape failed on all of them:
//
//   1. NEWLINE-separated:      `reddit_session=abc\ntoken_v2=xyz\n...`
//   2. JSON array (devtools'  `[{"name":"reddit_session","value":"abc"},…]`
//      "Copy as JSON", extensions, cURL exports)
//   3. `Cookie:` header prefix `Cookie: token_v2=eyJ...;`
//   4. Trailing/leading `;`  `;token_v2=abc;` or `token_v2=abc;`
//   5. Quoted values         `token_v2="eyJ..."`
//   6. URL-encoded names     `token%5Fv2=eyJ...` (some proxy UIs)
//   7. Comma-separated       `token_v2=abc, csv=2`
//
// The symptom was not a parse error the founder could act on. Reddit's
// `parse_cookie_string` returns `None` when it cannot find `reddit_session`,
// so the handler answers 400 "Could not parse cookie string"; but the paste
// that DID contain `reddit_session` and arrived as a JSON array or with a
// `Cookie:` prefix got past the `None` check with a session value that was
// actually `[{` or the whole JSON blob, then failed validation with
// "Reddit response parse error: error decoding response body" and a 502 —
// which the reverse proxy rewrites into an HTML error page, so the founder
// sees "Expected JSON from /api/... but got 502 (received HTML, not JSON)"
// and has no way to tell that their paste was fine.
//
// ── THE RULE ────────────────────────────────────────────────────
//
// Parse every shape, and build the Cookie header from the names that were
// actually recognised. A paste is refused only when NEITHER of the cookies
// the provider requires is present, and then the message names the formats
// that were tried so the founder knows what to copy instead.

use std::collections::BTreeMap;

/// One parsed cookie: name → value.
///
/// A `BTreeMap` keeps the order deterministic, so the rebuilt header string
/// is stable across runs (a DB write of an unstable string would churn the
/// token blob for no reason).
pub type ParsedCookies = BTreeMap<String, String>;

/// The shapes this parser understands, in the order they are attempted.
///
/// Named rather than numbered so a caller can log which one matched — a
/// "we could not parse your paste" report that does not say whether the JSON
/// path was even reached is the reason this bug stayed open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteFormat {
    /// `name=value; name=value` — the classic Cookie header.
    Header,
    /// One `name=value` per line.
    Newline,
    /// devtools "Copy as JSON" / extension / cURL export.
    JsonArray,
}

impl PasteFormat {
    pub fn label(self) -> &'static str {
        match self {
            PasteFormat::Header => "cookie header",
            PasteFormat::Newline => "one cookie per line",
            PasteFormat::JsonArray => "JSON cookie export",
        }
    }
}

/// A paste that could be read, plus which shape matched.
#[derive(Debug, Clone)]
pub struct PasteResult {
    pub cookies: ParsedCookies,
    pub format: PasteFormat,
}

/// Strip a leading `Cookie:` / `cookie:` prefix and surrounding whitespace,
/// and drop a stray leading semicolon. Applied to the whole paste before any
/// per-segment split so a pasted header behaves like a bare list.
fn normalise(paste: &str) -> &str {
    let trimmed = paste.trim();
    let without_prefix = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .unwrap_or(trimmed)
        .trim();
    without_prefix.trim_start_matches(';').trim()
}

/// Percent-decode one cookie name or value.
///
/// Only names need this in practice (`token%5Fv2`), but decoding values is
/// wrong: a cookie value can legitimately contain `%` sequences that are not
/// escapes, and decoding one produces a value the server will reject. So
/// values are kept verbatim, and only the NAME is decoded — a name containing
/// `%` is always an encoded name.
fn decode_name(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_string();
    }
    percent_decode(raw).unwrap_or_else(|| raw.to_string())
}

/// Minimal percent-decode: `%XX` → byte. No dependency, no allocation of an
/// owned String on the no-escape path beyond the result.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() || i + 3 > bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Split a segment into (name, value), tolerating the shapes a paste carries.
fn split_segment(seg: &str) -> Option<(String, String)> {
    let seg = seg.trim();
    if seg.is_empty() {
        return None;
    }
    let (raw_name, raw_value) = seg.split_once('=')?;
    let name = decode_name(raw_name.trim());
    if name.is_empty() {
        return None;
    }
    // A quoted value is unquoted; an unquoted one is kept verbatim.
    let value = unquote(raw_value.trim());
    Some((name, value))
}

fn unquote(v: &str) -> String {
    let bytes = v.as_bytes();
    if bytes.len() >= 2 && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
        || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return v[1..v.len() - 1].to_string();
    }
    v.to_string()
}

/// Try the `name=value; name=value` shape, splitting on `;` AND `,`.
///
/// The comma matters: some UIs (and a copy from a network inspector's cookie
/// list) join entries with commas rather than semicolons, and `csv=2, x=1`
/// would otherwise leave one entry with a leading comma in its value.
fn parse_header_shape(paste: &str) -> ParsedCookies {
    let mut out = ParsedCookies::new();
    for seg in paste.split([';', ',']) {
        if let Some((name, value)) = split_segment(seg) {
            // A JSON export flattened through this path yields garbage names
            // with braces/quotes in them; skip anything that is not a
            // plausible cookie name (RFC 6265 token).
            if is_cookie_name(&name) {
                out.insert(name, value);
            }
        }
    }
    out
}

/// True for a name that could really be a cookie: no whitespace, braces,
/// quotes, or brackets in it.
fn is_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['{', '}', '[', ']', '"', '\'', ':', ' ', '\t', '\n', '\r'])
}

/// Try the one-per-line shape.
fn parse_newline_shape(paste: &str) -> ParsedCookies {
    let mut out = ParsedCookies::new();
    for line in paste.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = split_segment(line) {
            if is_cookie_name(&name) {
                out.insert(name, value);
            }
        }
    }
    out
}

/// Try the devtools "Copy as JSON" / cURL-export shape.
///
/// devtools emits `[{"name":"reddit_session","value":"abc","domain":"…"},…]`;
/// cURL emits `{"Cookie":"a=1; b=2"}` or an array of the same. Both are
/// handled: the array of objects, and the object-with-a-"Cookie"-key.
fn parse_json_shape(paste: &str) -> Option<ParsedCookies> {
    let trimmed = paste.trim();
    if !trimmed.starts_with('[') && !trimmed.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    let mut out = ParsedCookies::new();

    // cURL / browser-extension object form: {"Cookie": "a=1; b=2"}
    if let Some(cookie_header) = value.get("Cookie").and_then(|v| v.as_str()) {
        for (k, v) in parse_header_shape(cookie_header) {
            out.insert(k, v);
        }
        if !out.is_empty() {
            return Some(out);
        }
    }

    // Array-of-objects form: [{"name":…, "value":…}, …]
    let items = value.as_array()?;
    for item in items {
        let name = item.get("name").and_then(|v| v.as_str())?;
        let val = item.get("value").and_then(|v| v.as_str())?;
        if is_cookie_name(name) {
            out.insert(name.to_string(), val.to_string());
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Parse a cookie paste in whatever shape it arrived.
///
/// Returns the parsed cookies and the shape that matched.
///
/// ─── THE ORDER, AND WHY IT IS BY CONTENT NOT BY TRIES ──────────
/// The JSON shape is tried first when the paste opens with `[` or `{`:
/// running a JSON blob through the text splitters produces junk names that
/// the `is_cookie_name` filter then silently drops, and the parse returns
/// something that looks valid but carries none of the real cookies.
///
/// The header-vs-newline choice is decided by CONTENT, not by which split
/// happens to yield a cookie. A single-line paste also has exactly one
/// "line", so trying newline first reports EVERY classic header as the
/// newline shape and leaves `reddit_session=xyz; csv=2` as one unbroken
/// value. The rule: a line that still contains a `;` or `,` is a HEADER of
/// several cookies, not one cookie per line.
pub fn parse_cookie_paste(paste: &str) -> Option<PasteResult> {
    let normalised = normalise(paste);
    if normalised.is_empty() {
        return None;
    }

    if let Some(cookies) = parse_json_shape(normalised) {
        return Some(PasteResult {
            cookies,
            format: PasteFormat::JsonArray,
        });
    }

    // More than one line, and no line carries a `;` separator, is the
    // one-cookie-per-line shape. Otherwise every line is itself a header
    // and the whole paste is read with the header splitter, which handles a
    // single line AND several lines identically.
    let is_multiline = normalised.lines().filter(|l| !l.trim().is_empty()).count() > 1;
    let any_line_has_separator = normalised.lines().any(|l| l.contains(';') || l.contains(','));

    if is_multiline && !any_line_has_separator {
        let newline = parse_newline_shape(normalised);
        if !newline.is_empty() {
            return Some(PasteResult {
                cookies: newline,
                format: PasteFormat::Newline,
            });
        }
    }

    let header = parse_header_shape(normalised);
    if !header.is_empty() {
        return Some(PasteResult {
            cookies: header,
            format: PasteFormat::Header,
        });
    }

    None
}

/// Rebuild a `Cookie:` header string from the parsed cookies, restricted to
/// `wanted` when that list is non-empty.
///
/// This is the string the provider actually sends upstream. Building it from
/// the PARSED map rather than echoing the raw paste is what makes a paste
/// that carried a `Cookie:` prefix, quotes, or JSON wrapper work: the noise
/// never reaches the provider.
pub fn rebuild_cookie_header(cookies: &ParsedCookies, wanted: &[&str]) -> String {
    cookies
        .iter()
        .filter(|(name, _)| wanted.is_empty() || wanted.contains(&name.as_str()))
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(c: &'a ParsedCookies, k: &str) -> Option<&'a str> {
        c.get(k).map(|s| s.as_str())
    }

    /// The classic shape, unchanged.
    #[test]
    fn parses_a_classic_cookie_header() {
        let r = parse_cookie_paste("token_v2=abc123; csv=2; reddit_session=xyz789")
            .expect("classic header");
        assert_eq!(r.format, PasteFormat::Header);
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
        assert_eq!(get(&r.cookies, "csv"), Some("2"));
    }

    /// A pasted header carries its own prefix.
    #[test]
    fn strips_the_cookie_prefix() {
        let r = parse_cookie_paste("Cookie: token_v2=abc; reddit_session=xyz")
            .expect("prefixed header");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz"));
    }

    /// Newline-separated — devtools' cookie list copy.
    #[test]
    fn parses_newline_separated_cookies() {
        let r = parse_cookie_paste(
            "reddit_session=xyz789\ntoken_v2=abc123\ncsv=2\nedgebucket=tb0p\n",
        )
        .expect("newline list");
        assert_eq!(r.format, PasteFormat::Newline);
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
    }

    /// devtools "Copy as JSON" — the shape the old parser turned into a 502.
    #[test]
    fn parses_a_devtools_json_export() {
        let paste = r#"[{"name":"reddit_session","value":"xyz789","domain":".reddit.com"},{"name":"token_v2","value":"abc123"},{"name":"csv","value":"2"}]"#;
        let r = parse_cookie_paste(paste).expect("JSON export");
        assert_eq!(r.format, PasteFormat::JsonArray);
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
    }

    /// cURL's `-b '{"Cookie":"…"}'` object form.
    #[test]
    fn parses_a_curl_cookie_object() {
        let paste = r#"{"Cookie":"token_v2=abc123; reddit_session=xyz789"}"#;
        let r = parse_cookie_paste(paste).expect("curl object");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
    }

    /// A leading or trailing semicolon is not a cookie.
    #[test]
    fn tolerates_stray_semicolons() {
        let r = parse_cookie_paste(";token_v2=abc123;;reddit_session=xyz789;")
            .expect("stray semicolons");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
    }

    /// Quoted values are unquoted.
    #[test]
    fn unquotes_double_and_single_quoted_values() {
        let r = parse_cookie_paste(r#"reddit_session="xyz789"; token_v2='abc123'"#)
            .expect("quoted values");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
    }

    /// A proxy UI that percent-encodes the underscore in `token_v2`.
    #[test]
    fn percent_decodes_cookie_names() {
        let r = parse_cookie_paste("token%5Fv2=abc123; reddit%5Fsession=xyz789")
            .expect("encoded names");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("xyz789"));
        assert_eq!(get(&r.cookies, "token_v2"), Some("abc123"));
    }

    /// A JWT value contains `%`-free base64url, so verbatim values are safe.
    /// The percent-decode is name-only by design; this pins that a value with
    /// a literal `%` is NOT mangled.
    #[test]
    fn does_not_percent_decode_cookie_values() {
        let r = parse_cookie_paste("reddit_session=eyJhbGciOiJSUzI1%J9").expect("value");
        assert_eq!(get(&r.cookies, "reddit_session"), Some("eyJhbGciOiJSUzI1%J9"));
    }

    /// The JSON wrapper's debris must not survive as a cookie name.
    #[test]
    fn a_json_blob_does_not_yield_junk_cookie_names() {
        // If parse_json_shape were skipped and the header splitter ran on
        // this, `[{"name"` would become a name and pollute the header.
        let paste = r#"[{"name":"reddit_session","value":"abc"}]"#;
        let r = parse_cookie_paste(paste).expect("json");
        assert_eq!(r.format, PasteFormat::JsonArray);
        assert_eq!(r.cookies.len(), 1);
        assert_eq!(get(&r.cookies, "reddit_session"), Some("abc"));
    }

    /// The rebuilt header contains only the cookies asked for.
    #[test]
    fn rebuild_keeps_only_wanted_cookies() {
        let r = parse_cookie_paste("reddit_session=abc; csv=2; edgebucket=tb").expect("parse");
        let header = rebuild_cookie_header(&r.cookies, &["reddit_session", "token_v2"]);
        assert_eq!(header, "reddit_session=abc");
    }

    /// An empty wanted list keeps everything.
    #[test]
    fn rebuild_with_no_wanted_keeps_all() {
        let r = parse_cookie_paste("reddit_session=abc; csv=2").expect("parse");
        let header = rebuild_cookie_header(&r.cookies, &[]);
        assert_eq!(header, "csv=2; reddit_session=abc");
    }

    /// A paste with no `=` anywhere is not a cookie paste at all.
    #[test]
    fn refuses_a_paste_with_no_cookies() {
        assert!(parse_cookie_paste("just some words").is_none());
        assert!(parse_cookie_paste("").is_none());
        assert!(parse_cookie_paste("   ").is_none());
    }

    /// THE FOUNDER'S OWN PAYLOAD, in the shape they reported it.
    ///
    /// The paste they pasted is a full Reddit cookie header whose `g_state`
    /// and `session_tracker` values contain dots, digits and long base64 —
    /// the shape that made the old splitter fail. This is the regression
    /// guard: the parse must find `reddit_session` AND `token_v2` and must
    /// not let the long values truncate the rest of the header.
    #[test]
    fn parses_the_full_founder_reported_header() {
        let cookie_v2 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IlNIQTI1NjpzS3dsMnlsV0VtMjVmcXhwTU40cWY4MXE2OWFFdWFybnoifQ.eyJzdWIiOiJ0Ml83ZXY0MzV1eSJ9.abc";
        let session = "eyJhbGciOiJSUzI1NiIsImtpZCI6IlNIQTI1NjpsVFdYNlFVUEloWktaRG1rIn0.eyJzdWIiOiJ0Ml83ZXY0MzV1eSJ9.def";
        let g_state = "{\"i_l\":0,\"i_ll\":\"\",\"i_t\":\"\",\"i_e\":{\"enable_itp_optimization\":24}}";
        let paste = format!(
            "token_v2={cookie_v2};csv=2;session_tracker=rpofgrabgmcrmafdnq.0.1.Z0FB;g_state={g_state};pc=j5;reddit_session={session};edgebucket=tb0p;loid=00000000007ev435uy"
        );
        let r = parse_cookie_paste(&paste).expect("founder header");

        assert_eq!(get(&r.cookies, "reddit_session"), Some(session));
        assert_eq!(get(&r.cookies, "token_v2"), Some(cookie_v2));
        // The long `session_tracker` and `g_state` must not swallow the
        // cookies that follow them.
        assert_eq!(get(&r.cookies, "edgebucket"), Some("tb0p"));
        assert_eq!(get(&r.cookies, "pc"), Some("j5"));

        // The rebuilt header carries the auth cookies the provider needs.
        let header = rebuild_cookie_header(&r.cookies, &["reddit_session", "token_v2"]);
        assert_eq!(header, format!("reddit_session={session}; token_v2={cookie_v2}"));
    }
}
