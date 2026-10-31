// ─── The Google Suite group must match the backend's credential sharing ───
//
// The operator's ask was "ensure the CLIENT-ID/SECRET integration is done
// effectively for the google-suite, and keep the other two options separate
// on the frontend too".
//
// Half of that ask is already true on the backend and was worth verifying
// rather than rebuilding:
//
//   - `Config::provider_credentials` (src/config.rs) resolves `google`,
//     `google_my_business`, `gmail`, `calendar` AND `drive` to the SAME
//     `YOUTUBE_CLIENT_ID` / `YOUTUBE_CLIENT_SECRET` in one match arm.
//   - `src/social/google_oauth.rs` is the single `exchange_code` / `refresh` /
//     `profile_picture` for all six Google-family providers.
//
// So this is not a credential change. What was missing is that the FRONTEND
// offered five separate tiles for one credential pair, which reads to the
// founder as five different logins.
//
// ── WHY YOUTUBE IS NOT IN THE GROUP ─────────────────────────────
//
// Not a preference, a platform constraint. Google's authorize endpoint
// refuses these pairs BEFORE any consent screen:
//
//   - YouTube Data API + Drive          → `Error 400: invalid_request`
//   - YouTube + `calendar.events`       → refused the same way
//
// Verified live 2026-10-30 against the deployed instance. The rule is pinned
// server-side by `GOOGLE_CONFLICTING_SCOPE_FAMILIES` in
// `src/social/scope_guard.rs` and enforced by a test in
// `tests/connect_flow_redteam_test.rs`, by FAMILY rather than exact scope so
// a later `youtube.readonly` is caught without editing the table.
//
// Putting YouTube in the group would therefore break the other three rather
// than add a fourth. That is the whole reason the group excludes it - and
// `drive` is excluded for the same measured reason.
//
// **This test runs the REAL TypeScript.** The grouping rule lives in
// `frontend/src/lib/channels/google-suite.ts`, a dependency-free module so
// this test can load it under node. A hand-written transcription would be a
// second copy of the rule, and two copies that agree with each other and
// disagree with what ships is exactly how the other cross-language rules
// drifted (AGENTS.md 9.18).
//
// Run: cargo test --test google_suite_group_test

use std::process::Command;

/// Run a JS expression against the REAL google-suite.ts under node.
fn eval_module(expr: &str) -> String {
    let script = format!(
        r#"
import {{
  GOOGLE_SUITE_GROUP, GOOGLE_SEPARATE_PROVIDERS, GOOGLE_FAMILY_PROVIDERS,
  isGoogleSuiteMember, isGoogleFamily, collapseGoogleSuite,
}} from '{module}';

const expr = process.argv[1];
const fn = new Function(
  'GOOGLE_SUITE_GROUP','GOOGLE_SEPARATE_PROVIDERS','GOOGLE_FAMILY_PROVIDERS',
  'isGoogleSuiteMember','isGoogleFamily','collapseGoogleSuite',
  'return (' + expr + ')'
);
const out = fn(
  GOOGLE_SUITE_GROUP, GOOGLE_SEPARATE_PROVIDERS, GOOGLE_FAMILY_PROVIDERS,
  isGoogleSuiteMember, isGoogleFamily, collapseGoogleSuite
);
process.stdout.write(JSON.stringify(out));
"#,
        module = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/frontend/src/lib/channels/google-suite.ts"
        )
    );

    let out = Command::new("node")
        .arg("--experimental-strip-types")
        .arg("--input-type=module")
        .arg("-e")
        .arg(&script)
        .arg(expr)
        .output()
        .expect("node must be available to run the real grouping module");

    assert!(
        out.status.success(),
        "node failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        script
    );
    String::from_utf8(out.stdout).expect("utf-8 output")
}

fn json(expr: &str) -> serde_json::Value {
    serde_json::from_str(&eval_module(expr)).expect("module returned valid JSON")
}

// ─── The group is exactly the providers that can share one consent ───

#[test]
fn the_suite_group_is_exactly_the_providers_whose_scopes_coexist() {
    // TWO, not four and not three. Both wider drafts were measured as
    // conflicting; see the mechanical test below.
    let group = json("GOOGLE_SUITE_GROUP");
    assert_eq!(
        group,
        serde_json::json!(["google", "gmail"]),
        "the group must be exactly the providers whose scope sets Google \
         grants together"
    );
}

/// THE MECHANICAL RULE, and the assertion that caught TWO wrong drafts.
///
/// Draft 1 was google+gmail+calendar+drive on the reading that those share
/// one credential pair. Refused: `google` carries YouTube scopes, `drive`
/// carries Drive scopes, and that pair is the one Google's authorize endpoint
/// names in the live 2026-10-30 `invalid_request`.
///
/// Draft 2 was google+gmail+calendar. Also refused, more subtly: the
/// `calendar` provider requests `calendar.events`, and a YouTube scope beside
/// `calendar.events` is Google's SECOND conflicting pair. Reading the scope
/// lists by eye did not catch it - running them through the preflight did.
///
/// So the group is not a list a human maintains by hand. It is checked
/// against the providers' real scope sets through the SAME preflight the
/// connect path uses. If a provider's scopes change and the group stops being
/// grantable, this test goes RED - which is the only thing that caught the
/// bug before it shipped, twice.
#[test]
fn the_group_is_a_combination_google_actually_grants() {
    // The preflight is the SERVER's own, called here verbatim. The scope sets
    use switchpost::social::scope_guard::google_scope_conflict;

    let members = json("GOOGLE_SUITE_GROUP");
    let members: Vec<String> = members
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();

    // Read each provider's scopes from the source the backend actually
    // compiles, so a scope edit is caught.
    let union: Vec<String> = members
        .iter()
        .flat_map(|p| scopes_for(p))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let result = google_scope_conflict(&union, "google-suite");
    assert_eq!(
        result,
        Ok(()),
        "the suite group's combined scope set must be one Google grants. If \
         this fails, a member's scopes now conflict - remove the offending \
         member from the group rather than shipping a consent nobody can pass"
    );
}

/// The negative direction, so the check cannot be asserting something vacuous:
/// if the group DID contain the conflicting pair, this same preflight must
/// report it. Otherwise `the_group_is_a_combination_google_actually_grants`
/// would pass on any list at all.
#[test]
fn the_preflight_does_refuse_a_known_conflicting_combination() {
    use switchpost::social::scope_guard::google_scope_conflict;

    let youtube_plus_calendar_events = [
        "https://www.googleapis.com/auth/youtube".to_string(),
        "https://www.googleapis.com/auth/calendar.events".to_string(),
    ];
    assert!(
        google_scope_conflict(&youtube_plus_calendar_events, "x").is_err(),
        "the preflight must refuse YouTube + calendar.events, so the check \
         above is measuring something"
    );

    let youtube_plus_drive = [
        "https://www.googleapis.com/auth/youtube".to_string(),
        "https://www.googleapis.com/auth/drive.file".to_string(),
    ];
    assert!(
        google_scope_conflict(&youtube_plus_drive, "x").is_err(),
        "the preflight must refuse YouTube + Drive, the live deployment's pair"
    );
}

/// The providers the suite group names, with the scope set each one requests.
///
/// Read from the provider source so a scope edit lands here rather than
/// silently invalidating the group. Kept beside the test that uses it so the
/// two cannot drift apart by file.
fn scopes_for(provider: &str) -> Vec<String> {
    let src = std::fs::read_to_string(format!(
        "{}/src/social/{}.rs",
        env!("CARGO_MANIFEST_DIR"),
        provider
    ))
    .unwrap_or_else(|e| panic!("read scopes for {provider}: {e}"));

    // The scope lists are plain string literals in a `vec![]` inside
    // `fn scopes`. Collect every googleapis scope literal in the file - the
    // providers here request only their own scopes, so this is exact.
    // NOTE the host is `www.googleapis.com`, NOT `accounts.google.com` (which
    // is the authorize endpoint and shares no host with a scope string).
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        if line.starts_with("//") || !line.contains("www.googleapis.com/auth/") {
            continue;
        }
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                out.push(line[start + 1..start + 1 + end].to_string());
            }
        }
    }
    assert!(
        !out.is_empty(),
        "found no Google scopes for provider '{provider}'; the reader is \
         stale or the provider moved"
    );
    out
}

/// The load-bearing constraint: YouTube is OUT, and for a platform reason.
///
/// If YouTube ever joins the group, the group's consent request is refused by
/// Google with `invalid_request` and ALL FOUR working providers break at once.
/// This assertion is what stops that regression, and it is the one that goes
/// RED if someone "helpfully" adds YouTube to be complete.
#[test]
fn youtube_is_never_in_the_group() {
    assert!(
        !json("GOOGLE_SUITE_GROUP")
            .as_array()
            .expect("array")
            .contains(&serde_json::json!("youtube")),
        "YouTube must NOT be in the suite group: Google's authorize endpoint \
         refuses a YouTube scope beside a Drive or a calendar.events scope, \
         which would break the other four providers rather than add a fifth"
    );
    assert!(
        json("GOOGLE_SEPARATE_PROVIDERS")
            .as_array()
            .expect("array")
            .contains(&serde_json::json!("youtube")),
        "YouTube must be listed as separate, so it still gets its own tile"
    );
}

/// `google_my_business` is Google-family but NOT suite-joinable: its scopes
/// are a different product surface and the frontend labels it "Google Biz".
/// It keeps its own tile rather than being folded into the four.
#[test]
fn google_my_business_stays_separate() {
    assert!(
        !is_google_suite_member("google_my_business"),
        "google_my_business must not be folded into the suite group"
    );
    assert!(
        is_google_family("google_my_business"),
        "but it must still be recognised as Google-family"
    );
}

// ─── The collapse is what the grid renders ───

/// The suite members must collapse to ONE tile, and everything else must
/// survive unchanged.
#[test]
fn the_grid_collapses_google_providers_into_one_suite_tile() {
    let out = json(
        "collapseGoogleSuite([
           'x','facebook','google','reddit','gmail','youtube',
           'calendar','drive','linkedin'
         ])",
    );
    let standalone: Vec<String> = out["standalone"]
        .as_array()
        .expect("standalone array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();

    // `google` stands in at its own first position, so the grid's order is
    // preserved rather than reshuffled - the founder's learned layout holds.
    // Only `gmail` collapses into the suite tile; `calendar` and `drive` keep
    // their own tiles because their scopes conflict with google's YouTube
    // grant.
    assert_eq!(
        standalone,
        vec![
            "x".to_string(),
            "facebook".to_string(),
            "google".to_string(), // the suite tile, at google's position
            "reddit".to_string(),
            "youtube".to_string(), // separate, untouched
            "calendar".to_string(), // separate: calendar.events conflicts
            "drive".to_string(),   // separate: Drive conflicts
            "linkedin".to_string(),
        ],
        "the suite members must collapse to one tile in the FIRST member's \
         position, and every non-suite provider must survive"
    );
    assert_eq!(
        out["suiteMemberCount"], 2,
        "both suite members must be counted, not just the first"
    );
}

/// The empty and no-Google cases: the collapse must be a no-op, so a
/// frontend regression that drops providers shows up here.
#[test]
fn the_collapse_is_a_noop_without_google_providers() {
    let out = json("collapseGoogleSuite(['x','linkedin','reddit'])");
    assert_eq!(
        out["standalone"],
        serde_json::json!(["x", "linkedin", "reddit"]),
        "a list with no Google providers must pass through unchanged"
    );
    assert_eq!(out["suiteMemberCount"], 0);

    let empty = json("collapseGoogleSuite([])");
    assert_eq!(empty["standalone"], serde_json::json!([]));
    assert_eq!(empty["suiteMemberCount"], 0);
}

/// Every suite member must be a provider the BACKEND registry actually
/// registers. A group entry naming a provider that does not exist would
/// render a tile whose connect always fails - the exact class of bug the
/// channels page's comment says bit it once (the list drifted and YouTube was
/// missing from the grid while the registry had it).
#[test]
fn every_suite_member_is_a_real_registered_provider() {
    let registry_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/social/registry.rs"
    ))
    .expect("read registry.rs");

    for provider in json("GOOGLE_SUITE_GROUP")
        .as_array()
        .expect("array")
        .iter()
        .chain(json("GOOGLE_SEPARATE_PROVIDERS").as_array().expect("array"))
    {
        let id = provider.as_str().expect("string");
        // The registry registers by identifier string; google_my_business is
        // spelled with an underscore there and a hyphen in the frontend's
        // PROVIDERS alias, so check the underscore form for that one.
        let needle = if id == "google_my_business" {
            "google_my_business".to_string()
        } else {
            format!("\"{id}\"")
        };
        assert!(
            registry_src.contains(&needle),
            "Google-family provider '{id}' is in the frontend group but is \
             not registered in the backend registry"
        );
    }
}

// ─── Helpers that mirror the module's own predicates ───

fn is_google_suite_member(provider: &str) -> bool {
    json(&format!("isGoogleSuiteMember({provider:?})"))
        .as_bool()
        .expect("bool")
}

fn is_google_family(provider: &str) -> bool {
    json(&format!("isGoogleFamily({provider:?})"))
        .as_bool()
        .expect("bool")
}

/// A non-Google provider must never be treated as family or suite.
#[test]
fn a_non_google_provider_is_never_family() {
    for p in ["x", "linkedin", "reddit", "youtube-typo", "", "googleish"] {
        assert!(!is_google_family(p), "'{p}' must not be Google-family");
        assert!(!is_google_suite_member(p), "'{p}' must not be a suite member");
    }
}

/// The group must be a strict partition of the family: every family member is
/// either suite or separate, never both, never neither. If the two lists
/// drifted apart, the grid would either lose a tile or render two for one
/// credential pair.
#[test]
fn suite_and_separate_are_a_partition_of_the_family() {
    let family: Vec<String> = json("GOOGLE_FAMILY_PROVIDERS")
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();
    for p in &family {
        let suite = is_google_suite_member(p);
        let separate = json("GOOGLE_SEPARATE_PROVIDERS")
            .as_array()
            .expect("array")
            .iter()
            .any(|v| v.as_str() == Some(p.as_str()));
        assert!(
            suite ^ separate,
            "'{p}' must be in exactly one of suite / separate (suite={suite}, \
             separate={separate})"
        );
    }
}
