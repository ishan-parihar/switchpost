// ─── The Google-family glyphs must be DISTINCT ─────────────────────
//
// THE DEFECT THIS PINS. `ChannelRail` drew its avatar glyph as
// `initials(provider_name)` - the first two characters of the DISPLAY name.
// Every Google-family provider's display name starts with "Google", so one
// login that produced a connected account per service rendered
//
//     Google Suite    -> GO
//     Gmail           -> GM
//     Google Calendar -> GO
//     Google Drive    -> GO
//     YouTube         -> YO
//
// THREE indistinguishable "GO" glyphs. The founder reported this as "the
// segregation of the tags for connected accounts on google-suite/youtube is
// conflictingly integrated", and the visual probe confirmed it on /calendar
// and /search with literal "GO" overlapping "GO" findings.
//
// The rail now reads the curated 2-letter code from `providers.ts`
// (`providerIcon`), which is distinct per provider id. This test runs the
// REAL TypeScript module - a hand-written transcription would be a second
// copy of the rule (AGENTS.md 9.18).
//
// WHY `google` HAS AN ENTRY AT ALL. The curated table had `gmail`, `calendar`,
// `drive` and `google-my-business` but NO `google`, so the Suite provider
// itself fell through to the `•` fallback - a bullet that carries no
// identity and that the probe's tiny-text gate rejects. The entry is added
// here as a rule, not as data: a curated glyph must exist for every provider
// the family offers.
//
// Run: cargo test --test google_family_glyph_test

use std::process::Command;

/// The Google family, exactly as `GOOGLE_FAMILY_PROVIDERS` lists it.
/// The ids are the ones the BACKEND registry registers (underscore form),
/// which is what `Integration.provider_identifier` carries.
const GOOGLE_FAMILY: &[&str] = &[
    "google",
    "gmail",
    "calendar",
    "drive",
    "youtube",
    "google_my_business",
];

fn eval(expr: &str) -> serde_json::Value {
    // `providers.ts` imports `$lib/stores/theme.svelte`, a Vite alias node cannot
    // resolve and a `.svelte` file node cannot compile. The
    // `lib_alias_resolver` loader maps both, so the REAL module is loaded
    // rather than a copy of it. Without it the test would fail at import and
    // measure nothing.
    let script = format!(
        r#"
import {{ providerMeta, providerIcon }} from '{module}';
const fn = new Function(
  'providerMeta','providerIcon',
  'return (' + process.argv[1] + ')'
);
process.stdout.write(JSON.stringify(fn(providerMeta, providerIcon)));
"#,
        module = concat!(env!("CARGO_MANIFEST_DIR"), "/frontend/src/lib/providers.ts")
    );

    let out = Command::new("node")
        .arg("--experimental-strip-types")
        .arg("--import")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/lib_alias_resolver.mjs"
        ))
        .arg("--input-type=module")
        .arg("-e")
        .arg(&script)
        .arg(expr)
        .output()
        .expect("node must be available to run the real providers module");

    assert!(
        out.status.success(),
        "node failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        script
    );
    serde_json::from_slice(&out.stdout).expect("module returned valid JSON")
}

fn icon_of(provider: &str) -> String {
    // Quoted so the expression is a plain string literal, whatever the id.
    eval(&format!("providerIcon({provider:?})"))
        .as_str()
        .expect("icon is a string")
        .trim()
        .to_string()
}

/// THE RULE: every Google-family provider must have a DISTINCT glyph.
///
/// This is the assertion that fails if someone adds a provider whose name
/// shares a prefix, or removes a curated code. It is also the reason `google`
/// got an entry: without one, two providers could collide on the fallback.
#[test]
fn every_google_family_provider_has_a_distinct_glyph() {
    let mut seen: Vec<(String, String)> = Vec::new();
    for id in GOOGLE_FAMILY {
        seen.push((id.to_string(), icon_of(id)));
    }

    for (id, icon) in &seen {
        assert!(
            !icon.is_empty(),
            "'{id}' has no glyph at all - the channel rail would render an \
             empty avatar for a connected account"
        );
    }

    // Distinctness, with the collision NAMED when it fails.
    let codes: Vec<&str> = seen.iter().map(|(_, i)| i.as_str()).collect();
    for (i, code) in codes.iter().enumerate() {
        for (j, other) in codes.iter().enumerate() {
            if i != j && code == other {
                let (a, _) = &seen[i];
                let (b, _) = &seen[j];
                panic!(
                    "'{a}' and '{b}' share the glyph '{code}': the rail cannot \
                     tell two connected Google accounts apart. Give one a \
                     distinct curated code in frontend/src/lib/providers.ts"
                );
            }
        }
    }
}

/// The specific collision the founder reported, pinned by name so the test
/// cannot pass on a different-but-equivalent fix.
#[test]
fn the_reported_google_collision_is_gone() {
    // Before the fix these three were all "GO".
    let suite = icon_of("google");
    let cal = icon_of("calendar");
    let drive = icon_of("drive");

    assert_eq!(suite, "GS", "the Suite provider's own glyph");
    assert_eq!(cal, "CA", "Google Calendar must not read 'GO'");
    assert_eq!(drive, "DR", "Google Drive must not read 'GO'");
}

/// The underscore/hyphen bridge: the BACKEND registers `google_my_business`
/// with an underscore and the frontend table spells the same provider with a
/// hyphen. Both must resolve to the same curated metadata, or the GMB channel
/// silently renders the indigo fallback and a `•` glyph.
#[test]
fn the_underscore_and_hyphen_spellings_resolve_the_same() {
    let underscored = eval("providerMeta('google_my_business')");
    let hyphenated = eval("providerMeta('google-my-business')");

    assert_eq!(
        underscored["icon"], hyphenated["icon"],
        "the two spellings of the SAME provider must resolve to one glyph"
    );
    assert_eq!(
        underscored["label"], hyphenated["label"],
        "and one label, so the rail never shows 'Google_my_business'"
    );
}

/// A provider with no curated entry must still resolve to SOMETHING - the
/// fallback path must not panic, and its label must read as a name rather
/// than as a raw identifier with an underscore in it.
#[test]
fn an_unknown_provider_still_resolves_to_a_label() {
    let meta = eval("providerMeta('some-future-provider')");
    assert!(
        meta["label"]
            .as_str()
            .is_some_and(|l| !l.is_empty() && !l.contains('_')),
        "an unknown provider must get a readable label, not a raw id: {meta}"
    );
}

/// Every Google-family provider the BACKEND registry registers must have a
/// curated glyph - falling through to the fallback is silent (a channel that
/// looks connected but renders an indigo bullet), and that is what happened
/// to `google` before its entry existed.
///
/// Scoped to the GOOGLE FAMILY on purpose: the wider curated table
/// deliberately carries one-character codes for short brand names (`x` ->
/// 'X', `facebook` -> 'f'), so a "2+ characters everywhere" rule would be
/// wrong. The rule that matters is that the five providers a single Google
/// login can produce are all distinguishable.
#[test]
fn every_google_family_registered_provider_has_a_curated_glyph() {
    let registry = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/social/registry.rs"
    ))
    .expect("read registry.rs");

    // The registry registers by identifier string. Each family member must
    // appear in it, or the frontend is curating a glyph for a provider the
    // backend never connects.
    for id in GOOGLE_FAMILY {
        assert!(
            registry.contains(&format!("\"{id}\"")),
            "Google-family provider '{id}' is not registered in the backend \
             registry, so it can never appear on the channel rail"
        );
        let icon = icon_of(id);
        assert!(
            !icon.is_empty() && icon != "\u{2022}",
            "'{id}' resolved to the '{icon}' fallback, not a curated code. A \
             bullet is DISTINCT from every other glyph, so the distinctness \
             test above stays green - the failure is silent, which is why \
             `google` was missing until the collision report surfaced it."
        );
    }
}

/// No ROUTE may hand-maintain its own provider glyph table again.
///
/// THE DEFECT THIS PINS. Three routes carried a private inline copy of the
/// glyph rule - `dms/+page.svelte` (7 entries), `comments/+page.svelte`
/// (5 entries), and the rail's own `initials()` before this cycle. The
/// copies drifted in different DIRECTIONS: the DM copy fell back to a `•`,
/// the comments copy fell back to the first two letters of the identifier,
/// and the rail's `initials()` produced the reported "GO"/"GO"/"GO"
/// collision. One login therefore rendered three DIFFERENT glyphs for the
/// same account depending on which page you looked at.
///
/// The curated table in `providers.ts` is now the only source, and a route
/// that wants a glyph calls `providerIcon`. This test makes the rule
/// enforceable, because a copy of a rule is silent until it drifts
/// (AGENTS.md 9.18).
#[test]
fn no_route_hand_maintains_a_glyph_table() {
    // Walk the frontend route + component tree. `frontend/src` is the whole
    // client surface under Svelte/TS.
    for entry in walk_frontend_source() {
        // The signature is a Record literal that maps PROVIDER IDENTIFIERS to
        // short codes. `Icon.svelte` also declares `const icons`, but for SVG
        // paths keyed by icon NAME, so the needle carries the type inside the
        // braces' KEYS - `x:`, `reddit:`, `linkedin:` - rather than matching
        // on the declaration, which would flag every icon set in the app.
        //
        // This is the shape that appeared THREE times and drifted.
        let has = |needle: &str| entry.1.contains(needle);
        let is_glyph_table = has("x: \"X\"") || has("x: 'X'") || has("reddit: \"R\"");

        assert!(
            !is_glyph_table,
            "{} re-introduces an inline provider glyph table. Use \
             `providerIcon(provider_identifier)` from $lib/providers - the \
             curated table is the ONE source (AGENTS.md 9.18).",
            entry.0
        );
    }
}

/// Every `.svelte` / `.ts` file under frontend/src, as (relative path, text).
fn walk_frontend_source() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/frontend/src");
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "svelte" || e == "ts")
            {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let rel = path
                        .strip_prefix(concat!(env!("CARGO_MANIFEST_DIR"), "/"))
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| path.display().to_string());
                    out.push((rel, text));
                }
            }
        }
    }
    assert!(
        !out.is_empty(),
        "walked frontend/src and found no files - the guard measures nothing"
    );
    out
}

/// `ProviderIcon`'s letter fallback must ALSO read the curated table.
///
/// THE DEFECT THIS PINS. The rail's `initials()` collision was one copy of
/// the rule; `ProviderIcon.svelte` carries a second, INDEPENDENT one - its
/// `fallbackText` was `provider.slice(0, 2).toUpperCase()`, the first two
/// letters of the IDENTIFIER. That still collides for `google` and
/// `google-my-business`, which both render "GO" on a brand-colored circle,
/// and the probe reported the resulting `"GO" overlaps "GO"` on /calendar
/// and /search. The rail fix alone would NOT have removed that finding.
#[test]
fn provider_icon_letter_fallback_uses_the_curated_glyph() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/lib/channels/ProviderIcon.svelte"
    ))
    .expect("read ProviderIcon.svelte");

    // The old shape was a bare slice of the identifier.
    assert!(
        !source.contains("const fallbackText = $derived(provider.slice(0, 2).toUpperCase())"),
        "ProviderIcon derives its fallback letters from the IDENTIFIER again. \
         `google` and `google-my-business` both render \"GO\" from that rule, \
         and the probe reports \"GO\" overlaps \"GO\". Read the curated glyph \
         from $lib/providers instead (AGENTS.md 9.18)."
    );

    // It must consult the curated table.
    assert!(
        source.contains("curatedGlyph(provider)"),
        "ProviderIcon does not read the curated glyph table, so its letter \
         fallback is a second copy of the rule that drifted last time"
    );

    // And the two sources must AGREE for the family that reported the bug.
    for id in GOOGLE_FAMILY {
        assert_eq!(
            icon_of(id),
            icon_via_provider_icon(id),
            "'{id}' renders one glyph in the curated table and a different \
             one in ProviderIcon's fallback - the two copies have drifted"
        );
    }
}

/// The letter `ProviderIcon` shows for a provider, read by replicating the
/// component's `logos` lookup and `fallbackText` rule over the REAL curated
/// table, so the two can be compared per id.
fn icon_via_provider_icon(id: &str) -> String {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/lib/channels/ProviderIcon.svelte"
    ))
    .expect("read ProviderIcon.svelte");

    // A provider with an SVG path renders the LOGO, not a letter, so there is
    // no letter to compare. Only the fallback providers are in scope here.
    let has_logo = source
        .lines()
        .any(|line| line.trim_start().starts_with(&format!("\"{id}\":"))
            && line.contains("'<path"));
    if has_logo {
        return String::new();
    }

    let script = r#"
import { providerIcon } from '/home/ishanp/Documents/github/my-projects/agentic-utility/interaction-surfaces/switchpost/frontend/src/lib/providers.ts';
const id = process.argv[1];
const code = providerIcon(id);
const text = code === '•' ? id.slice(0, 2).toUpperCase() : code;
process.stdout.write(JSON.stringify(text));
"#;
    let out = Command::new("node")
        .arg("--experimental-strip-types")
        .arg("--import")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/lib_alias_resolver.mjs"
        ))
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .arg(id)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "node could not evaluate the fallback rule: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice::<String>(&out.stdout).expect("fallback text")
}
