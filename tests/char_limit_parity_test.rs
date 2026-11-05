//! The frontend's static limit table and the Rust providers must agree.
//!
//! The bug this pins: `frontend/src/lib/providers.ts` keeps a static
//! `charLimit` per provider as a FALLBACK for the moment before
//! `GET /api/providers` answers. That fallback had drifted from the
//! provider implementations in 13 places, and the worst was X: the
//! composer claimed 280 while the server enforced 4000, so a 280..4000
//! character post was blocked and the Post button disabled — the exact
//! defect AGENTS.md §9.20 records having fixed once, back in place
//! because the fallback table was never pinned to the authority.
//!
//! The exposure is real, not theoretical: the API client RETURNS
//! `{ error }` on a failed request instead of throwing, so the
//! composer's `catch` never fires on an HTTP error, `provRes.data` is
//! undefined, and the static table stays in place silently. Any 500,
//! any network failure, any offline open of the app leaves the composer
//! enforcing numbers the server does not.
//!
//! The direction matters: `Provider::max_content_length()` is the OWNER.
//! The static table must match it. Where they disagree, the Rust value
//! wins and this test fails.

use std::path::Path;

/// `Provider::max_content_length()` from every `src/social/*.rs`.
///
/// Returns `None` for a body that is not a plain integer literal
/// (`usize::MAX`, a comment on the first line), because those are not
/// comparable to a number without evaluating the expression — and a
/// test that guessed would be a test that lied.
fn rust_limits() -> Vec<(String, Option<usize>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/social");
    let mut out: Vec<(String, Option<usize>)> = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("src/social is readable") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("provider file is readable");
        let Some(m) = src.find("fn max_content_length(&self) -> usize {") else {
            continue;
        };
        let body = &src[m..];
        // The digits on the signature line: `-> usize {\n    <digits>`.
        let after = body.splitn(2, '{').nth(1).unwrap_or("");
        let first_meaningful = after
            .lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap_or("");
        // The leading run of DIGITS, not the whole line: a provider can
        // carry a trailing comment (`0 // GitHub does not support
        // publishing content like social posts`), and `parse::<usize>()`
        // on the whole string fails - which silently dropped github,
        // devto and wordpress out of the Rust map, so the gate never
        // compared them and a mutation on github stayed GREEN.
        // `usize::MAX` yields no digits, so it stays unparseable, which
        // is the honest answer for a limit that is not a number.
        let digits: String = first_meaningful
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let value = (!digits.is_empty()).then(|| digits.parse::<usize>().unwrap());
        out.push((path.file_stem().unwrap().to_str().unwrap().to_string(), value));
    }
    out.sort();
    out
}

/// The static table in `frontend/src/lib/providers.ts`.
///
/// Skips comment lines, because the table documents the old values in
/// prose and prose is not a value.
fn frontend_limits() -> Vec<(String, usize)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("frontend/src/lib/providers.ts");
    let src = std::fs::read_to_string(&path).expect("frontend/src/lib/providers.ts is readable");
    let mut out: Vec<(String, usize)> = Vec::new();

    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with("*") || t.starts_with("/*") {
            continue;
        }
        // `  'x':                { label: 'X', ..., charLimit: 280 },`
        //
        // A TABLE ENTRY is required, not a bare `charLimit:` mention:
        // the file also declares the `ProviderMeta` interface
        // (`charLimit: number;`), and parsing that as a table row
        // gives key "charLimit" and no number - which is how the first
        // version of this gate found zero entries and failed its own
        // vacuity check. The interface is not a value.
        if !t.contains("charLimit:") || !t.contains("label:") {
            continue;
        }
        let key = t
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(|c| c == '\'' || c == '"');
        // The value sits in `charLimit: 4000 },` - the digits come first,
        // then the closing brace and comma. Splitting on a comma leaves
        // "4000 }" behind, which does not parse, so the digits are taken
        // as the leading run of them.
        // `rsplit` yields from the RIGHT, so the text AFTER the marker is
        // the FIRST segment. Taking `.nth(1)` returns the text BEFORE it -
        // which is how the first version of this gate parsed nothing.
        let limit = t
            .rsplit("charLimit:")
            .nth(0)
            .and_then(|rest| {
                let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<usize>().ok()
            });
        if !key.is_empty() {
            if let Some(l) = limit {
                out.push((key.to_string(), l));
            }
        }
    }
    out.sort();
    out
}

/// The registry's key for a provider file stem or frontend key.
///
/// The frontend writes `linkedin-page` and the Rust file is
/// `linkedin_page.rs`; the registry registers the underscored form.
fn normalise(key: &str) -> String {
    key.replace('-', "_")
}

#[test]
fn the_frontend_fallback_table_matches_the_rust_providers() {
    let rust = rust_limits();
    assert!(
        !rust.is_empty(),
        "no Rust providers were found, so this test would pass VACUOUSLY \
         (AGENTS.md 9.38/9.45). Check the path glob."
    );

    let front = frontend_limits();
    assert!(
        !front.is_empty(),
        "no frontend table entries were found, so this test would pass \
         VACUOUSLY. Check the parsing against frontend/src/lib/providers.ts."
    );

    let rust_by_key: std::collections::HashMap<String, usize> = rust
        .iter()
        .filter_map(|(k, v)| v.map(|v| (normalise(k), v)))
        .collect();

    let mut drift: Vec<String> = Vec::new();
    for (key, fe_value) in &front {
        let rk = normalise(key);
        let Some(rust_value) = rust_by_key.get(&rk) else {
            // Not a Rust provider (or its limit is not a literal). The
            // fallback for an unknown provider is not the static table's
            // job to police.
            continue;
        };
        if fe_value != rust_value {
            drift.push(format!(
                "{key}: frontend fallback says {fe_value}, Provider::max_content_length() says {rust_value}"
            ));
        }
    }

    assert!(
        drift.is_empty(),
        "The static fallback table in frontend/src/lib/providers.ts has drifted from the \
         provider implementations. The Rust value is the OWNER and the server publishes it; \
         the static table is what the composer uses when the fetch fails, and the API client \
         RETURNS an error instead of throwing, so the failure is SILENT. Fix the table to \
         match the provider:\n  {}",
        drift.join("\n  ")
    );
}

#[test]
fn a_limit_of_zero_means_no_publish_surface_and_the_table_says_so() {
    // github / gmail / calendar / drive have `max_content_length() == 0`,
    // which the composer should read as "this channel cannot carry a post".
    // The static table claiming 65536 for them would offer a channel whose
    // posts the server refuses (AGENTS.md 9.20).
    let rust = rust_limits();
    let zero: Vec<String> = rust
        .iter()
        .filter(|(_, v)| *v == Some(0))
        .map(|(k, _)| normalise(k))
        .collect();
    assert!(
        !zero.is_empty(),
        "expected the no-surface providers (github, gmail, calendar, drive) to report 0; \
         found none, so this test is vacuous."
    );

    let front = frontend_limits();
    let mut wrong: Vec<String> = Vec::new();
    for (key, value) in &front {
        if zero.contains(&normalise(key)) && *value != 0 {
            wrong.push(format!("{key}: static table says {value}, provider says 0 (no publish surface)"));
        }
    }
    assert!(
        wrong.is_empty(),
        "these providers have no publish surface, so a non-zero static fallback offers a \
         channel the server refuses:\n  {}",
        wrong.join("\n  ")
    );
}
