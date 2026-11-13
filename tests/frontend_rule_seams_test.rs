// ─── The frontend rule seams ─────────────────────────────────────
//
// The composer, the calendar and the preview panes each had their own copy
// of a rule the server owns. The copies drifted, and the drift was silent:
// the founder saw a number, or a day, that was not the one the server
// would use.
//
//   - the four preview panes measured with `plainText.length` (UTF-16 code
//     units) and cropped the ESCAPED string, so a 600-emoji caption read
//     "600/2200" while the server counted 2400 bytes and rejected it, and
//     a caption of `&`s had its crop marker ~400 characters early.
//   - the calendar deep link parsed `?date=2026-11-15` with `new Date()`,
//     which is UTC midnight: the calendar opened November 14 in every
//     negative-offset zone, then wrote that wrong day back to the URL.
//
// These tests pin the SEAMS: the rule lives in one place, and the callers
// use it. They read the sources, because the frontend has no test runner
// and the rule is a source-level contract (the same way
// `tests/wall_clock_parity_test.rs` pins the TS conversion).
//
// Run: cargo test --test frontend_rule_seams_test

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"))
}

/// The source with its COMMENTS removed.
///
/// A rule's explanation often names the old broken call (`new Date(urlDate)`),
/// and a test that greps the raw text would trip on the explanation rather
/// than on the code.
fn read_without_comments(relative: &str) -> String {
    let src = read(relative);
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        // A trailing comment on a code line.
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

const PREVIEWS: [&str; 4] = [
    "frontend/src/lib/composer/previews/GeneralPreview.svelte",
    "frontend/src/lib/composer/previews/FacebookPreview.svelte",
    "frontend/src/lib/composer/previews/InstagramPreview.svelte",
    "frontend/src/lib/composer/previews/LinkedInPreview.svelte",
];

#[test]
fn every_preview_uses_the_shared_crop() {
    for path in PREVIEWS {
        let src = read(path);
        assert!(
            src.contains("cropForPreview"),
            "{path} must use the shared crop helper — the composer shows a number the server \
             enforces, and the number has to come from the same rule"
        );
        assert!(
            !src.contains("plainText.length"),
            "{path} must not measure with `plainText.length`: that is UTF-16 code units, and \
             the server counts UTF-8 bytes (X counts weighted units)"
        );
        assert!(
            !src.contains("escaped.slice("),
            "{path} must not slice the ESCAPED string: `&` becomes five characters, so the crop \
             marker lands early — and a slice inside an injected mention tag breaks the markup"
        );
    }
}

#[test]
fn the_crop_helper_measures_with_the_server_rule() {
    let src = read("frontend/src/lib/composer/previews/crop.ts");
    assert!(
        src.contains("countFor("),
        "the crop must count with the provider's own rule (`countFor`)"
    );
    assert!(
        src.contains("charLimitFor("),
        "the crop must take the limit from the published provider limit (`charLimitFor`)"
    );
    assert!(
        src.contains("text.slice(0, limit)"),
        "the crop must slice the PLAIN text"
    );
    // The crop is applied to the plain text, and the two halves are escaped
    // SEPARATELY. Escaping first and slicing the escaped string is what put
    // the marker in the wrong place.
    let crop_body = src
        .split("export function cropForPreview")
        .nth(1)
        .expect("the crop body");
    assert!(
        crop_body.contains("text.slice(0, limit)"),
        "the crop must slice the plain text"
    );
    assert!(
        crop_body.contains("highlightPreviewText(before, options)"),
        "the first half must be escaped on its own"
    );
    assert!(
        crop_body.contains("highlightPreviewText(after, options)"),
        "the second half must be escaped on its own — a slice inside an injected mention tag \
         would break the markup"
    );
}

#[test]
fn the_calendar_parses_a_date_only_url_parameter_as_a_calendar_date() {
    let src = read_without_comments("frontend/src/routes/calendar/+page.svelte");
    assert!(
        src.contains("parseDateKey("),
        "the calendar must parse `?date=` with `parseDateKey`"
    );
    assert!(
        !src.contains("new Date(urlDate)"),
        "`new Date(urlDate)` parses YYYY-MM-DD as UTC midnight: the calendar opened the PREVIOUS \
         day in every negative-offset zone"
    );
}

#[test]
fn the_date_key_parser_builds_a_local_day() {
    let src = read("frontend/src/lib/calendar/utils.ts");
    assert!(
        src.contains("export function parseDateKey"),
        "the parser must exist in the calendar utils"
    );
    let body = src
        .split("export function parseDateKey")
        .nth(1)
        .expect("the parser body");
    assert!(
        body.contains("new Date(Number(y), Number(mo) - 1, Number(d))"),
        "the date must be built from its parts in LOCAL time"
    );
    assert!(
        !body.contains("new Date(key)"),
        "the parser must not hand the raw key to the Date constructor"
    );
}

// ─── The provider-specific composer fields ─────────────────────────
//
// The founder asked for the platform-specific fields postiz has. The
// providers ALREADY read a set of keys out of the post's settings -
// Pinterest's `link`, Reddit's flair, Mastodon's content warning and
// sensitive flag, TikTok's three interaction switches, WordPress's
// featured media, Hashnode's publication, Skool's group - and NOTHING in
// the composer collected them.
//
// AGENTS.md 9.28: a control in the composer is a PROMISE. A control for a
// key no provider reads is a promise nothing keeps, and a provider that
// reads a key the composer never collects is a setting the founder cannot
// reach. So the inventory is closed in BOTH directions, and these two
// tests hold it there:
//
//   1. every key the providers read has a control - a deleted field fails
//      the build
//   2. every control the composer has has a READER in
//      `src/social/<provider>.rs` - a control nobody reads fails too

const COMPOSER: &str = "frontend/src/lib/composer/ComposerModal.svelte";

/// The settings keys the providers already read out of the post's
/// settings. One entry per (provider, key).
///
/// This is the INVENTORY, and it is closed: a key belongs here only
/// because a provider reads it, and a provider's read of a key is what
/// puts it here.
const PROVIDER_FIELD_INVENTORY: [(&str, &str); 12] = [
    ("pinterest", "link"),
    ("reddit", "flair_id"),
    ("reddit", "flair_text"),
    ("mastodon", "sensitive"),
    ("mastodon", "spoiler_text"),
    ("mastodon", "language"),
    ("tiktok", "disable_duet"),
    ("tiktok", "disable_comment"),
    ("tiktok", "disable_stitch"),
    ("wordpress", "featured_media"),
    ("hashnode", "publication_id"),
    ("skool", "groupId"),
];

/// The source file a provider's implementation lives in.
///
/// The provider identifier is the file name for every provider in the
/// inventory, and the dash form is the underscore form for the ones
/// whose identifier is dashed (`linkedin-page` -> `linkedin_page.rs`).
fn provider_source_path(provider: &str) -> String {
    let direct = format!("src/social/{provider}.rs");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if root.join(&direct).exists() {
        return direct;
    }
    format!("src/social/{}.rs", provider.replace('-', "_"))
}

/// The (provider, key) pairs the composer's `PROVIDER_FIELDS` table
/// promises, parsed out of the source.
///
/// The table is read as TEXT: the frontend has no test runner, and the
/// promise is a source-level contract (the same way
/// `tests/wall_clock_parity_test.rs` pins the TS conversion).
fn provider_fields_promises(src: &str) -> Vec<(String, String)> {
    let start = src
        .find("const PROVIDER_FIELDS")
        .expect("the composer must declare a PROVIDER_FIELDS table");
    let body = &src[start..];
    let end = body
        .find("\n};")
        .expect("the end of the PROVIDER_FIELDS table");
    let body = &body[..end];

    let mut out: Vec<(String, String)> = Vec::new();
    let mut provider = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        // A provider line: `pinterest: [{ ... }]` or `reddit: [`.
        //
        // The name must be the WHOLE prefix of the line, so a field line
        // (`{ key: 'link', ... }`) can never be mistaken for one.
        if let Some(colon) = trimmed.find(": [") {
            let name = trimmed[..colon].trim();
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if valid {
                provider = name.to_string();
            }
        }
        // The keys a control writes.
        //
        // `key:` is lowercase, so `altKey:` - the OTHER key the same
        // control can write, for a provider that reads either - is not
        // matched by it and needs its own marker.
        for marker in ["key: '", "altKey: '"] {
            let mut rest = line;
            while let Some(i) = rest.find(marker) {
                let after = &rest[i + marker.len()..];
                let close = after.find('\'').expect("a closing quote on the key");
                if !provider.is_empty() {
                    out.push((provider.clone(), after[..close].to_string()));
                }
                rest = &after[close + 1..];
            }
        }
    }
    out
}

#[test]
fn the_composer_collects_every_provider_setting_the_providers_read() {
    let src = read_without_comments(COMPOSER);
    let promises = provider_fields_promises(&src);
    assert!(
        !promises.is_empty(),
        "the composer's PROVIDER_FIELDS table could not be read - the inventory check would pass \
         on an empty table"
    );
    for (provider, key) in PROVIDER_FIELD_INVENTORY {
        assert!(
            src.contains(&format!("'{key}'")),
            "the composer must collect `{key}` for {provider}: the provider already reads it out of \
             the post's settings, and a control that does not exist means the founder cannot set it \
             (AGENTS.md 9.28)"
        );
        assert!(
            promises
                .iter()
                .any(|(p, k)| p == provider && k == key),
            "the composer's PROVIDER_FIELDS table must carry `{key}` for {provider} - the key is \
             read by {provider}'s provider, so it belongs to that provider's card"
        );
    }
}

#[test]
fn every_composer_provider_field_has_a_reader_in_the_provider() {
    let src = read_without_comments(COMPOSER);
    let promises = provider_fields_promises(&src);
    assert!(
        !promises.is_empty(),
        "the composer's PROVIDER_FIELDS table could not be read - the promise-and-reader check \
         would pass on an empty table"
    );
    for (provider, key) in promises {
        let path = provider_source_path(&provider);
        let provider_src = read_without_comments(&path);
        assert!(
            provider_src.contains(&format!("\"{key}\"")),
            "the composer collects `{key}` for {provider}, but {path} never reads it: a control \
             nobody reads is a promise nothing keeps (AGENTS.md 9.28)"
        );
    }
}
