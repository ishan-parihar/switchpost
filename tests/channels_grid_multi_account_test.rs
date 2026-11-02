// ─── Red-team: a connected provider keeps its tile in the grid ───────
//
// The Channels grid used to hide a provider the moment ONE integration
// existed for it. The founder managing many channels connects a second
// Google account, a third LinkedIn identity and a second X handle as a
// matter of course, and the filter made every one of those impossible:
// one account connected left one tile missing, five accounts connected
// left five tiles missing, and the only way back was to disconnect the
// one you wanted to keep alongside.
//
// The fix is a count (`connectedCounts`) instead of a filter, and an
// "N connected" badge, so the tile reads "Add another X" rather than
// disappearing.
//
// This is a source-invariant test: the grid is a browser-side Svelte
// component with no API a unit test drives, so the invariant it must hold
// is the shape of the derivation that decides which tiles render. It pins
// the absence of the hiding rule rather than the presence of a fix, which
// is the only durable assertion here - a re-implementation can use a
// different mechanism and still be correct, as long as it does not
// FILTER a connected provider out of `connectableProviders`.
//
// Run: cargo test --test channels_grid_multi_account_test

use std::path::PathBuf;

const PAGE: &str = "frontend/src/routes/channels/+page.svelte";
/// The calendar view's chrome. Its filter row is what wrapped into two and
/// then three rows when the channel rail was expanded.
const CAL_HEADER: &str = "frontend/src/lib/calendar/CalendarHeader.svelte";
/// The channel rail. Its rows hold the buttons that used to overflow the
/// channel name, so the invariants for that bug live against this file.
const RAIL: &str = "frontend/src/lib/channels/ChannelRail.svelte";

fn page_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(PAGE);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {PAGE}: {e}"))
}

/// The page's CODE, with `//` comment lines removed.
///
/// WHY THIS EXISTS: a source-shape assertion is supposed to pin what the
/// founder SEES. Chasing the phrase through the raw text lets a prose comment
/// satisfy it, which is exactly how `the_count_of_already_connected_channels_
/// is_available_to_the_grid` went mutation-blind: the tile label was deleted
/// from the template entirely and the test stayed green, because this file's
/// own comment says the tile "reads as 'Add another'". Stripping comment lines
/// is the difference between asserting behaviour and asserting documentation.
fn page_template_without_comments() -> String {
    page_source()
        .lines()
        .map(|l| l.trim_start())
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rail's CODE, comment-stripped for the same reason as the page's.
fn rail_source_without_comments() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(RAIL);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {RAIL}: {e}"))
        .lines()
        .map(|l| l.trim_start())
        .filter(|l| !l.starts_with("//"))
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The calendar header's CODE, comment-stripped.
fn cal_header_without_comments() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CAL_HEADER);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {CAL_HEADER}: {e}"))
        .lines()
        .map(|l| l.trim_start())
        .filter(|l| !l.starts_with("//"))
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A provider already present in `integrations` must still render a tile,
/// because that tile is now the only way to add a SECOND account.
#[test]
fn a_connected_provider_is_not_filtered_out_of_the_grid() {
    let src = page_source();

    // The defect was a `.filter(...)` over the provider list against the
    // set of already-connected ones. Anything that reintroduces a filter
    // on `availableProviders` reintroduces the bug.
    //
    // Take the whole statement, not one line: the filter in the original
    // defect sat on its own line inside a multi-line $derived, and a
    // line-scoped check on the opening line sails straight past it. That
    // is how this test failed to notice a mutation it was written for.
    let start = src
        .find("let connectableProviders =")
        .expect("connectableProviders is not declared");
    let stmt = &src[start..];
    let end = stmt
        .find(";\n")
        .unwrap_or_else(|| panic!("connectableProviders statement never terminates"));
    let stmt = &stmt[..end];

    assert!(
        !stmt.contains(".filter("),
        "connectableProviders filters the provider list again: `{stmt}` - a \
         provider with one integration would lose its tile, so adding a \
         second account of that provider becomes impossible"
    );

    // And the derivation must exist at all, or the grid renders nothing.
    assert!(
        src.contains("connectableProviders"),
        "connectableProviders is gone; the grid's `{{#each}}` renders nothing"
    );
}

/// The "N connected" count is what makes a kept tile readable: without it
/// a connected provider's tile is indistinguishable from a fresh one and
/// the founder clicks it expecting a first connect.
#[test]
fn the_count_of_already_connected_channels_is_available_to_the_grid() {
    let src = page_source();

    assert!(
        src.contains("connectedCounts"),
        "connectedCounts is gone - the grid cannot tell 'connected once' from \
         'not connected', and the tile's label no longer says 'Add another'"
    );

    // The label must follow the count, which is the user-visible half of
    // keeping the tile. Without it the tile reads 'Connect' for a provider
    // that is already connected, and clicking it looks like a bug.
    //
    // STRIPPED OF COMMENTS FIRST, and that is not decoration. A first draft
    // asserted `src.contains("Add another")` over the RAW source, and it was
    // mutation-BLIND: this file's own prose comment says the tile
    // \"reads as 'Add another'\", so deleting the label from the template
    // entirely left the assertion satisfied by a comment. The mutation
    // survived. A comment is not the template - strip `//` lines before
    // looking for the string the founder actually sees.
    let code = page_template_without_comments();
    assert!(
        code.contains("Add another"),
        "the tile no longer says 'Add another' when the provider is already \
         connected - the kept tile reads as a first connect. The RAW source \
         MAY mention the phrase in a comment; this assertion reads the code \
         only, so a comment cannot satisfy it"
    );
}

/// A plain `Set` of connected providers is not enough: the founder with
/// three LinkedIn profiles must see the three. A Map count works; a Set
/// does not. This pins that the counting is a count and not a boolean.
#[test]
fn the_connected_state_is_counted_not_merely_flagged() {
    let src = page_source();
    // Take the WHOLE declaration: from `let connectedCounts =` to the
    // `});` that closes the $derived.by callback. A fixed character window
    // truncates it, which is a test that passes for the wrong reason.
    let start = src
        .find("let connectedCounts =")
        .expect("connectedCounts is not declared");
    let decl = &src[start..];
    let end = decl
        .find("});")
        .unwrap_or_else(|| panic!("connectedCounts declaration never closes"));
    let decl = &decl[..end];

    assert!(
        decl.contains("new Map<"),
        "connectedCounts is not a Map, so it cannot say how many channels a \
         provider has - only whether it has any"
    );
    assert!(
        decl.contains("?? 0) + 1"),
        "connectedCounts does not accumulate, so N integrations for one \
         provider read as 1"
    );
}

// ─── Red-team: the reconnect hint must not diagnose a cause it cannot see ──
//
// PRODUCTION EVIDENCE, not a hypothetical. Deploying the feed fixes put the
// new classifier in front of the founder's real LinkedIn channels, and it
// logged, for every one of them:
//
//     "Integration ... (linkedin-page) points at a resource no longer
//      served (API error: No virtual resource found). The channel's token
//      is fine, so it stays publishable; the target is gone."
//
// ...while the SAME rows still carried `refresh_needed = 1` from pre-fix
// state, and one of them holds a token valid until 2026-12-29.
//
// So the Channels card then told the founder:
//
//     "The stored token could not be refreshed. Reconnect to resume posting."
//
// That is a diagnosis the client cannot make: `IntegrationPublic`
// (src/db/models.rs) exposes the BOOLEAN `refresh_needed` and no failure
// reason at all, so the cause is not in the payload `channelStatus()` reads.
// The founder is told the token is broken, reconnects (re-running OAuth they
// did not need), and watches the channel fail again - because the page is
// gone and the token was always fine.
//
// The fix removes the false diagnosis and keeps the correct action. This is
// the same discipline the file already documents for `token_expires_at`:
// never render a claim the API does not send.
//
// WHY A SOURCE TEST and not a behavioural one: the frontend has no unit-test
// runner (no vitest/jest in frontend/package.json; playwright-core is only
// for browser automation), and `channelStatus()` is browser-side. The
// condemned strings are the invariant - asserting their ABSENCE survives a
// rewrite of the wording, which asserting the new sentence would not.
#[test]
fn the_reconnect_hint_stops_diagnosing_the_token() {
    let src = read_channel_status_source();

    // The specific claims the client has no data for.
    for condemned in [
        "The stored token could not be refreshed",
        "could not be refreshed",
    ] {
        assert!(
            !src.contains(condemned),
            "the refresh_needed hint still asserts a cause the API does not \
             send ({condemned:?}). Production proves it is false: a channel \
             can carry a stale flag while its token is fine and the TARGET is \
             what is gone - the founder then reconnects for nothing and the \
             channel fails again."
        );
    }

    // And the survivors that make the card still useful.
    assert!(
        src.contains("Needs reconnect"),
        "the channel lost its reconnect label, so the founder has no name for \
         the state that needs attention"
    );
    assert!(
        !src.contains("unreachable!("),
        "channel-status.ts must not panic a path it cannot handle"
    );
}

/// The two files must not drift back apart: if `IntegrationPublic` ever grows
/// a real failure-reason field, this hint is where it becomes safe to say why.
///
/// WHY THIS IS A SOURCE SHAPE TEST AND NOT A STRUCTURAL ONE: the guard was
/// mutated both ways to check it actually bites. Adding a field to
/// `IntegrationPublic` WITHOUT wiring the `From` impl fails at COMPILE time
/// (E0063 missing field), and this test never runs - the reader gets an error
/// about an unrelated struct instead of the instruction below. Wiring the
/// field too makes it compile, and then this assertion fires on its own. Both
/// are a red test; only the second carries the message, so the message is
/// written for that case and for the human reading the compile error.
#[test]
fn integration_public_still_exposes_no_failure_reason() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/db/models.rs")
        .canonicalize()
        .unwrap_or_else(|e| panic!("resolve src/db/models.rs: {e}"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));

    let start = src
        .find("pub struct IntegrationPublic")
        .unwrap_or_else(|| panic!("IntegrationPublic is gone - a rename broke the payload contract"));
    let body = src[start..]
        .split('}')
        .next()
        .unwrap_or_else(|| unreachable!("a struct body always ends with }}"));

    assert!(
        !body.contains("refresh_reason")
            && !body.contains("failure_reason")
            && !body.contains("feed_error"),
        "IntegrationPublic now exposes a failure reason, so channel-status.ts \
         is free to name the cause - REMOVE the source test above that \
         forbids it and make the hint say why, using the new field"
    );
}

fn read_channel_status_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("frontend/src/lib/channels/channel-status.ts")
        .canonicalize()
        .unwrap_or_else(|e| panic!("resolve channel-status.ts: {e}"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

// ── The channel rail's row must reserve room for its action buttons ──
//
// The rail's expanded row used to render its action cluster as
// `absolute end-1 top-1/2 -translate-y-1/2`. An absolutely positioned
// element does not participate in flex sizing, so the name block's
// `min-w-0 flex-1` could not be shrunk by it: the buttons were laid out
// ON TOP of the channel name, and the row read as "buttons overflowing
// onto a second row".
//
// The fix puts the cluster IN FLOW as a flex sibling and changes the
// select button from `w-full` to `min-w-0 flex-1`, so the cluster takes
// its width out of the row and the name's `truncate` does its job.
//
// WHY SOURCE SHAPE: there is no frontend unit runner (see the header of
// this file), and the layout is CSS behaviour, so the pin is the shape
// the fix requires. The mutation that proves it reads as a red: putting
// `absolute` back on the cluster reintroduces the overlap.

/// The action cluster must be a flex SIBLING of the select button, so it
/// takes width out of the row instead of floating over the name.
#[test]
fn the_rails_action_cluster_is_in_flow_not_overlaid() {
    let src = rail_source_without_comments();
    let start = src
        .find("<nav")
        .unwrap_or_else(|| panic!("the channel rail's nav is not in ChannelRail.svelte"));
    let nav = &src[start..];
    let row_end = nav
        .find('}')
        .and_then(|c| c.checked_add(1))
        .and_then(|e| nav[e..].find("</div>").map(|o| o + e))
        .unwrap_or_else(|| panic!("the rail's first row never closes"));
    let row = &nav[..row_end];

    assert!(
        row.contains("min-w-0 flex-1"),
        "the rail's row button is not `min-w-0 flex-1` - a `w-full` button \
         fills the row and the action cluster has nowhere to sit, which is \
         how the buttons ended up overflowing: {row}"
    );

    let cluster_start = row
        .find("<div class=\"flex items-center gap-1\">")
        .unwrap_or_else(|| panic!("the rail's action cluster is not in flow"));
    let cluster = &row[cluster_start..];
    assert!(
        !cluster.contains("absolute end-1"),
        "the action cluster is absolutely positioned again: an absolute \
         element does not participate in flex sizing, so it does not shrink \
         the name block and it sits ON TOP of the channel name - the \
         overflow this test was written to pin: {cluster}"
    );
}

// ── The calendar header's min-content must fit beside an expanded rail ──
//
// Founder-reported: expanding the channel rail distorted the calendar, with
// the controls overflowing onto extra rows.
//
// MEASURED, not assumed. The calendar page is `max-w-6xl` (1040px) and holds
// the global rail (80px) plus the channel rail. Expanded, that rail is 260px,
// so the calendar gets 700px. The header's filter row is `flex-wrap`, so it
// only stays on one line while its MIN-CONTENT fits the box.
//
// Before this fix the floor was about 1040px: three `min-w-[120px]` selects
// plus two `shrink-0` action buttons. That exceeds 1040 at every viewport and
// 700 with the rail open, so the controls wrapped into two rows - and three
// when the rail was expanded. The file's own comment even recorded the two-row
// measurement as if it were a fact of the design; it is a bug.
//
// The fix makes the flexible parts actually flexible: the selects take a
// `w-[7.5rem]` basis with `min-w-0`, and the action buttons carry `min-w-0
// truncate` instead of `shrink-0`. The nav group and the view switcher stay
// `shrink-0`, because truncating "Week of October 31" or "Day/Week/Month"
// would destroy the meaning rather than the overflow. Min-content drops to
// about 524px, which fits beside an expanded rail.

/// Every control in the header's filter row must be able to give way, so the
/// row keeps one line at the calendar's real width.
#[test]
fn the_calendar_header_fits_beside_an_expanded_channel_rail() {
    let src = cal_header_without_comments();

    // The two immovable controls: the date/nav group and the view switcher.
    // They legitimately refuse to shrink, and they are the reason everything
    // else must be able to.
    assert!(
        src.contains("shrink-0"),
        "the header lost every `shrink-0`, so the date and the view switcher \
         can now be truncated instead of the controls beside them"
    );

    // The filter selects must NOT carry a min-width floor any more. A floor
    // is what made 1040px the header's minimum, which is wider than the
    // calendar ever gets.
    assert!(
        !src.contains("min-w-[120px]"),
        "a filter select carries `min-w-[120px]` again: three of them plus the \
         two action buttons set the header's floor above the calendar's width, \
         and the row wraps into two or three lines"
    );
    assert!(
        src.contains("min-w-0"),
        "the header has no `min-w-0` anywhere, so nothing can give way and the \
         row can only overflow"
    );

    // The two actions must shrink, not sit at full size on a second line.
    for label in ["Generate Posts", "+ New Post"] {
        let at = src
            .find(label)
            .unwrap_or_else(|| panic!("the header lost its {label} action"));
        // Walk back to the opening tag of the button that holds this label.
        let open = src[..at]
            .rfind("<button")
            .unwrap_or_else(|| panic!("{label} is not inside a button"));
        let tag = &src[open..at + label.len()];
        assert!(
            !tag.contains("shrink-0"),
            "the {label} button is `shrink-0` again - it cannot give way, so it \
             pushes the filter row onto a second line: {tag}"
        );
    }
}
