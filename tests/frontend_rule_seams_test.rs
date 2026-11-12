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
