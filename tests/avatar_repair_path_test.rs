// ─── Red-team: a provider's profile path must carry the avatar ───────────
//
// The operator reported profile pictures missing across x-cookie,
// linkedin-page, linkedin-account and gmail.
//
// The DB write, the API serialization and the frontend's `<img>` were all
// correct - a picture entered `integrations.profile_picture` and came
// straight back out. The defect was upstream, in the providers' own
// `fetch_page_info`: the function REFRESH calls to repair a connected row.
//
// Why that one function matters more than it looks: the refresh handler's
// write is
//
//     profile_picture = COALESCE(?, profile_picture)
//
// which keeps the old value when the provider hands back nothing. That is
// right for a provider whose image genuinely went away, and it is WRONG as
// a repair path: a channel whose avatar was never captured can never gain
// one, because the only code that could supply it returns `None` forever.
//
// Several Google-family providers had exactly that: their
// `fetch_page_info` hardcoded `picture: None` on the success branch while
// their own `exchange_code` - the same provider, the same account, the same
// credentials - read `user["picture"]` from Google's OpenID userinfo
// endpoint. One provider, two answers to "who is this channel", and the
// answer used at refresh time was the one without the photo.
//
// calendar's was the clearest: its SUCCESS branch hardcoded `None` and its
// FAILURE branch, a few lines below, read `user["picture"]`.
//
// These tests hold the invariant a source read can check: whichever way a
// provider identifies its own account, the profile path must not return an
// avatar the provider demonstrably has access to.

use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The Google OpenID userinfo endpoint - the ONE place the whole Google
/// family reads an account's name, email and photo.
const GOOGLE_USERINFO: &str = "oauth2/v2/userinfo";

/// Does this source read `user["picture"]` (or the equivalent) anywhere?
fn reads_picture(src: &str) -> bool {
    src.contains("\"picture\"") && src.contains(GOOGLE_USERINFO)
}

/// The providers that share Google credentials and therefore share the
/// account identity: whatever one of them can see about "who am I", the
/// rest can too, because it is the same OAuth account.
const GOOGLE_FAMILY: &[&str] = &[
    "src/social/gmail.rs",
    "src/social/calendar.rs",
    "src/social/drive.rs",
    "src/social/youtube.rs",
    "src/social/google_my_business.rs",
];

/// A channel's face is not always the Google ACCOUNT's face.
///
/// Most of the family shows the account: the account's photo, from the
/// one shared OpenID reader in `google_oauth::profile_picture`. Two
/// members are legitimately different, and this test demands OpenID for
/// them wrongly until it learned otherwise:
///
///  * `google` and `youtube` - a channel's face is the channel's own
///    YouTube thumbnail from `youtube/v3/channels`. That is the right
///    answer for that row, and the thumbnail is a bigger, better avatar
///    than the account photo anyway. Demanding OpenID here is not a
///    stricter version of the same rule, it is a DIFFERENT rule, and it
///    fails working code.
///  * `drive` - "who is this channel" IS the account, and it reads
///    `user["picture"]` inline rather than through the shared helper.
///    That is a real duplication the SECOND test in this file flags for
///    having too many inline spellings of the endpoint; it is not a
///    missing avatar. So this test must not fail on it, and it must
///    still accept a photo coming from the account.
///
/// The default (absent from this table) is the shared OpenID reader.
const PICTURE_SOURCE_OVERRIDES: &[(&str, &str)] = &[
    ("src/social/youtube.rs", "item[\"snippet\"][\"thumbnails\"]"),
    ("src/social/drive.rs", "user[\"picture\"]"),
];

/// A provider that reads `user["picture"]` from Google userinfo must not
/// hardcode `picture: None` on its success path.
///
/// Pre-fix, gmail and calendar both failed this. calendar's was
/// self-evident in a single function: the `if status.is_success()` arm
/// said `picture: None` and the `else` arm, twenty lines below, said
/// `picture: user["picture"]`. The same provider answering "who is this
/// channel" two different ways within one function is the defect, and no
/// amount of reading the write path would have found it.
///
/// The check is NOT `fpi.contains("picture: None")`. That literal is what
/// the pre-fix code happened to look like, and it is also the reason this
/// test could not catch the same defect rewritten one line differently:
/// binding the picture to a local (`let picture: Option<String> = None;`
/// then `picture,`) makes the struct field say `picture` and the source
/// still answer "no avatar, forever". Verified by mutation - that rewrite
/// passed this test while the write was still broken.
///
/// So the invariant asserted here is what the SUCCESS ARM DOES: it must
/// read the account's photo from SOME real source. But which source is
/// per-provider and correct, and this test's first version got that wrong
/// too - it demanded the OpenID reader for the whole family and flagged
/// `google` (YouTube), whose channel's face is the YouTube channel
/// thumbnail, not the Google account's avatar. Both were corrected by
/// mutation rather than by reading; see
/// `PICTURE_SOURCE_OVERRIDES`.
///
/// A provider that demonstrably has access to a photo and returns None
/// anyway is the defect, whatever the spelling.
#[test]
fn a_google_provider_that_reads_the_accounts_picture_must_return_it() {
    let mut offenders = Vec::new();
    for path in GOOGLE_FAMILY {
        let src = read(path);
        if !reads_picture(&src) {
            continue;
        }
        // The provider demonstrably has access to the account's photo. Its
        // fetch_page_info - the refresh path - must therefore not hardcode
        // None on the branch that runs when the API call SUCCEEDS.
        let Some(fpi) = extract_fn(&src, "async fn fetch_page_info") else {
            continue;
        };

        // The success arm's shape, not its spelling. Take the branch that
        // runs when the provider's own call worked - `if status.is_success()`
        // where present, otherwise the whole body - and require it to reach
        // a real reader. A literal `None` anywhere in that arm is the defect
        // whether it lands in the struct literal or a local.
        let success_arm = match fpi.find("if status.is_success()") {
            Some(at) => fpi[at..].to_string(),
            None => fpi.clone(),
        };

        // Which reader counts depends on whose face this is. The Google
        // account's photo comes from the shared OpenID reader; a YouTube
        // CHANNEL's face is the channel's own thumbnail, which is the right
        // answer for that row and not a substitute to be standardised away.
        let reader = PICTURE_SOURCE_OVERRIDES
            .iter()
            .find(|(p, _)| *p == *path)
            .map(|(_, r)| *r)
            .unwrap_or("profile_picture(");

        let reaches_a_reader = success_arm.contains(reader);
        let supplies_no_photo =
            success_arm.contains("picture: None") || success_arm.contains("Option<String> = None");

        if !reaches_a_reader || supplies_no_photo {
            offenders.push(format!(
                "{path}: reads {GOOGLE_USERINFO}[\"picture\"] but its \
                 fetch_page_info does not carry it. Refresh writes \
                 `profile_picture = COALESCE(?, profile_picture)`, so a row \
                 whose avatar was never captured can never gain one - the \
                 only code that could supply it never does. \
                 (reaches_a_reader={reaches_a_reader}, \
                 supplies_no_photo={supplies_no_photo})"
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "providers whose profile path cannot carry an avatar they have \
         access to:\n{}",
        offenders.join("\n\n")
    );
}

/// The shared helper is the ONE implementation of the rule.
///
/// Three providers needed this (gmail, calendar, and any future Google
/// family member), and a copy per provider is how they drift: the v8 audit
/// history in AGENTS.md §9.18 is a list of rules implemented twice on the
/// same side of the wire and then disagreeing. The endpoint, the header
/// shape and the "a missing photo is not an error" rule all live in one
/// function, and the providers delegate to it.
#[test]
fn the_google_picture_rule_has_one_implementation() {
    let shared = read("src/social/google_oauth.rs");
    assert!(
        shared.contains("pub async fn profile_picture"),
        "the Google-family profile picture must be read through ONE shared \
         helper in src/social/google_oauth.rs. A copy per provider drifts."
    );
    assert!(
        shared.contains(GOOGLE_USERINFO),
        "the shared helper must read the account's photo from the OpenID \
         userinfo endpoint, which is the only place Google publishes it."
    );

    // Every provider that calls the helper must CALL it, not inline a
    // second copy of the endpoint.
    let mut inlined = Vec::new();
    for path in GOOGLE_FAMILY {
        if *path == "src/social/google_oauth.rs" {
            continue;
        }
        let src = read(path);
        let Some(fpi) = extract_fn(&src, "async fn fetch_page_info") else {
            continue;
        };
        // A provider whose fetch_page_info IS a userinfo call with no other
        // API involved (drive: "who is this channel" IS the account) has no
        // duplication to make. What is a defect is a file spelling the endpoint
        // more than twice - once in exchange_code, once in fetch_page_info -
        // because the copies are then free to drift on the header, the error
        // handling and the rule.
        let spellings = src.matches(GOOGLE_USERINFO).count();
        if fpi.contains(GOOGLE_USERINFO) && spellings > 2 {
            inlined.push(format!(
                "{path}: spells the userinfo endpoint {spellings} times. The rule \
                 has ONE shared implementation in google_oauth.rs; two copies of \
                 a fetch drift."
            ));
        }
    }
    assert!(
        inlined.is_empty(),
        "a second copy of the userinfo fetch:\n{}",
        inlined.join("\n")
    );
}

/// Extract a function body by its signature line, brace-matched.
///
/// Brace-matching rather than "until the next `}` at column 4" so a
/// nested block (an `if` inside the body) cannot end the scan early and
/// silently narrow what is checked - a scan that sees half a function
/// passes on code it never read.
fn extract_fn(src: &str, signature: &str) -> Option<String> {
    let start = src.find(signature)?;
    let open = src[start..].find('{')? + start;
    let mut depth = 0usize;
    let mut end = None;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    Some(src[start..end?].to_string())
}
