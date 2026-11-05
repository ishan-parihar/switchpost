//! Every live suite must resolve its account from `LIVE_TEST_EMAIL`.
//!
//! The bug this pins: the live suites look up a FIXTURE account by default.
//! That fixture belongs to no real installation, so on a real operator's
//! machine the suite finds no user and skips - and the skip message names
//! a MISSING CHANNEL ("no youtube integration in DB"), which sends the
//! founder to connect a channel they already connected. The real cause is
//! that the suite looked up the wrong ACCOUNT.
//!
//! So two properties are pinned, and they are different properties:
//!
//!  1. No live suite hard-codes an account address in a query or lookup.
//!  2. Every live suite that resolves an account honours `LIVE_TEST_EMAIL`.
//!
//! This is a SOURCE-level check, and that is the honest level for it: the
//! property is "the suite does not bake an account in", which is a fact
//! about the source. The behavioural half - that the override actually
//! selects a different account and the skip message then names the real
//! cause - is verified by running the suites with and without the
//! override; that run is recorded in the worklog, not re-asserted here,
//! because a test that shells out to `cargo test` is a test that cannot
//! tell a real skip from a compile failure.

use std::path::{Path, PathBuf};

/// Every live test suite in `tests/`.
fn live_suites() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/ is readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // This checker is itself named live_*.rs, and its own source
            // quotes the patterns it forbids - so it would match itself.
            // A gate that fails on its own text is a gate nobody can use.
            name.starts_with("live_")
                && name.ends_with(".rs")
                && name != "live_account_selection_test.rs"
        })
        .collect();
    out.sort();
    out
}

#[test]
fn no_live_suite_hard_codes_an_account_address() {
    let mut offenders: Vec<String> = Vec::new();

    for path in live_suites() {
        let src = std::fs::read_to_string(&path).expect("suite is readable");
        for (n, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            // A COMMENT is not a lookup. The suites document the old shape
            // in prose, and prose must not fail a code gate (AGENTS.md 9.55).
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("/*") {
                continue;
            }
            // An address baked into a SQL literal or a lookup call.
            let looks_baked = (trimmed.contains("email = '")
                || trimmed.contains("email = \"")
                || trimmed.contains("get_user_by_email(pool, \"")
                || trimmed.contains("get_user_by_email(&pool, \""))
                && !trimmed.contains("email = ?")
                && !trimmed.contains("email = $1");
            if looks_baked {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.file_name().unwrap().to_str().unwrap(),
                    n + 1,
                    trimmed
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "live suites must resolve the account from LIVE_TEST_EMAIL, not bake one in.\n\
         A baked fixture address means the suite looks up an account that belongs to nobody, \
         finds nothing, and reports a MISSING CHANNEL instead of the missing ACCOUNT.\n\
         offenders:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn every_live_suite_that_resolves_an_account_honours_the_override() {
    let mut offenders: Vec<String> = Vec::new();

    for path in live_suites() {
        let src = std::fs::read_to_string(&path).expect("suite is readable");
        let code: String = src
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.starts_with("//") && !t.starts_with("*") && !t.starts_with("/*")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let resolves_an_account = code.contains("get_user_by_email")
            || code.contains("FROM users")
            || code.contains("JOIN users");
        if !resolves_an_account {
            continue;
        }
        // The presence of the STRING is not the property - a comment
        // mentions it, and a constant is declared, without the env var
        // ever being READ. The property is that the suite resolves its
        // account through the env var, so that is what is asserted: a
        // call to `std::env::var` (or `env::var`) naming the override.
        //
        // This is the difference my first mutation run found: replacing
        // the function BODY with a hard-coded address left the string in
        // place and the gate stayed green. A gate satisfied by prose is a
        // gate that measures nothing (§9.33, §9.55).
        let reads_the_override = code
            .lines()
            .filter(|l| l.contains("env::var"))
            .any(|l| l.contains("LIVE_TEST_EMAIL"));
        if !reads_the_override {
            offenders.push(path.file_name().unwrap().to_str().unwrap().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these live suites resolve an account but ignore LIVE_TEST_EMAIL, so they can only \
         ever run against the default fixture address:\n  {}",
        offenders.join("\n  ")
    );
}

/// The live suites that must exist and must be checked.
///
/// The list is PINNED, not counted. A count is satisfied while a suite is
/// renamed out of the gate's reach - my first mutation run did exactly
/// that (`live_reddit_test` -> `zz_reddit_live_test`) and the gate stayed
/// green, because nine other suites were still there to satisfy "at least
/// five". A renamed suite is a suite nobody checks, and it looks exactly
/// like a suite that was deliberately excluded.
///
/// So: adding a live suite is a change to this list, and a suite that
/// disappears is a FAILURE with its name, not a smaller count.
const EXPECTED_SUITES: &[&str] = &[
    "live_discord_test.rs",
    "live_failure_classification_test.rs",
    "live_github_google_test.rs",
    "live_linkedin_youtube_test.rs",
    "live_reddit_test.rs",
    "live_telegram_test.rs",
    "live_threads_ias_test.rs",
    "live_whatsapp_cloud_test.rs",
    "live_x_test.rs",
    "live_youtube_test.rs",
];

#[test]
fn the_live_suite_list_is_pinned() {
    let found: Vec<String> = live_suites()
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .filter(|n| n != "live_account_selection_test.rs")
        .collect();

    let missing: Vec<&&str> = EXPECTED_SUITES
        .iter()
        .filter(|e| !found.iter().any(|f| f == *e))
        .collect();
    assert!(
        missing.is_empty(),
        "these live suites are missing, so the checks above cannot see them. \
         A suite that was RENAMED escapes the gate exactly as silently as one \
         that was deleted:\n  {}\nfound: {:?}",
        missing
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n  "),
        found
    );

    let unexpected: Vec<&String> = found
        .iter()
        .filter(|f| !EXPECTED_SUITES.contains(&f.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "these live suites are not in the pinned list, so they are NOT checked by \
         the two tests above. Add them here when you add a suite:\n  {}",
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
