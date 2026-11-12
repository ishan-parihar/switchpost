// ─── The env template must cover the env the code reads ────────
//
// `.env.example` is the template the founder copies. A variable the code
// reads but the template does not mention is a variable the founder never
// sets — and the feature it belongs to fails with a message that names a
// variable they have never heard of.
//
// The audit found 21 such variables (MEDIA_S3_*, the WhatsApp Cloud set,
// the AI/LLM pair, NEYNAR, Dub.co, the feed interval, the X link rule, the
// MCP toolset, the event-log retention, the Reddit cookie token and the
// Deckmill binary). They are documented now, and this test keeps the
// template honest: every variable the code reads has to be in it.
//
// The scan covers both ways the code reads a variable:
//   - `config.rs`'s `opt("NAME")` (the typed config), and
//   - `std::env::var("NAME")` (the reads that happen at the point of use).
//
// Run: cargo test --test env_template_test

use std::collections::BTreeSet;
use std::path::Path;

/// The variables the code reads that are NOT the founder's to configure in
/// `.env`: the process's own environment, and the database URL (which the
/// template documents in its own section).
const NOT_TEMPLATE_VARS: [&str; 4] = ["HOME", "USERPROFILE", "PATH", "TMPDIR"];

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"))
}

/// Every variable name the code reads.
fn vars_read_by_the_code() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut files: Vec<String> = Vec::new();
    collect_rust_files("src", &mut files);
    for file in files {
        let src = read(&file);
        // `opt("NAME")` — config.rs's typed reads.
        const OPT: &str = "opt(\"";
        for (offset, _) in src.match_indices(OPT) {
            let rest = &src[offset + OPT.len()..];
            if let Some(end) = rest.find('"') {
                names.insert(rest[..end].to_string());
            }
        }
        // `std::env::var("NAME")` — the reads that happen at the point of use.
        const VAR: &str = "env::var(\"";
        for (offset, _) in src.match_indices(VAR) {
            let rest = &src[offset + VAR.len()..];
            if let Some(end) = rest.find('"') {
                names.insert(rest[..end].to_string());
            }
        }
    }
    names
}

fn collect_rust_files(dir: &str, out: &mut Vec<String>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(
                path.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap()
                    .to_str()
                    .unwrap(),
                out,
            );
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(
                path.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
        }
    }
}

/// The variables the template documents.
fn vars_documented() -> BTreeSet<String> {
    let src = read(".env.example");
    let mut names = BTreeSet::new();
    for line in src.lines() {
        let line = line.trim();
        // A documented variable is either set (`NAME=value`) or commented
        // out (`# NAME=value`) — the template ships everything commented.
        let line = line.strip_prefix('#').unwrap_or(line).trim();
        if let Some((name, _)) = line.split_once('=') {
            let name = name.trim();
            if name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                && !name.is_empty()
            {
                names.insert(name.to_string());
            }
        }
    }
    names
}

#[test]
fn every_variable_the_code_reads_is_in_the_template() {
    let read = vars_read_by_the_code();
    let documented = vars_documented();
    let mut missing: Vec<&String> = read
        .iter()
        .filter(|name| !documented.contains(*name))
        .filter(|name| !NOT_TEMPLATE_VARS.contains(&name.as_str()))
        .filter(|name| name.as_str() != "DATABASE_URL")
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "these variables are read by the code but are not in .env.example: {missing:?}"
    );
}

#[test]
fn the_template_documents_the_variables_the_audit_found_missing() {
    let documented = vars_documented();
    for name in [
        "MEDIA_S3_BUCKET",
        "MEDIA_S3_ENDPOINT",
        "MEDIA_S3_KEY",
        "MEDIA_S3_SECRET",
        "WHATSAPP_CLOUD_TOKEN",
        "WHATSAPP_CLOUD_PHONE_NUMBER_ID",
        "WHATSAPP_CLOUD_WABA_ID",
        "WHATSAPP_CLOUD_VERIFY_TOKEN",
        "WHATSAPP_CLOUD_APP_SECRET",
        "LLM_ENDPOINT",
        "LLM_MODEL",
        "NEYNAR_API_KEY",
        "DUB_CO_API_KEY",
        "DUB_CO_WORKSPACE",
        "FEED_REFRESH_INTERVAL_SECONDS",
        "STRIP_LINKS_FROM_X",
        "SWITCHPOST_TOOLSETS",
        "EVENTS_LOG_RETENTION_DAYS",
        "REDDIT_COOKIE_TOKEN",
        "DECKMILL_BIN",
    ] {
        assert!(
            documented.contains(name),
            "{name} is read by the code and must be in .env.example"
        );
    }
}

#[test]
fn the_scan_actually_found_the_variables() {
    // A scan that finds nothing would pass the first test for the wrong
    // reason.
    let read = vars_read_by_the_code();
    assert!(
        read.len() > 40,
        "the scan found only {} variables — it is not reading the code",
        read.len()
    );
    assert!(read.contains("TOKEN_ENCRYPTION_KEY"));
    assert!(read.contains("APP_PASSWORD"));
}
