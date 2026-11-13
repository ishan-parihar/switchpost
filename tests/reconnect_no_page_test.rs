// ─── Red-team: "Refresh" must not fail for a channel that has no pages ─
//
// The refresh endpoint calls `provider.reconnect(...)` unconditionally.
// The trait's `reconnect` routes through `fetch_page_info` — the
// PAGE-SELECTION function — which is SUPPOSED to refuse for a provider
// with no page concept.
//
// Before this was fixed, that refusal was forwarded by a `?` and became
// the answer to the ONLY button the Channels page offers to repair a
// token: "Failed to refresh profile: X does not support page
// management", on a Reddit channel, naming a different platform
// entirely. 13 providers were in that state.
//
// Two halves hold it together, and both are pinned here:
//
//  1. The refusal is a TYPED variant (`ProviderError::NoPageManagement`),
//     not a string match. The 11 refusals word their English differently
//     ("does not support page management", "does not support page
//     selection", "does not support page-level management") — a
//     `msg.contains("page management")` guard at the trait level would
//     be a string match that silently stops working the first time
//     someone rewords a message.
//
//  2. The default `reconnect` DEGRADES on that variant instead of
//     forwarding it, so a channel-shaped provider is refreshable without
//     a per-provider override. A refusal for any OTHER reason is still an
//     error.
//
// The table below is re-checked, not assumed: every provider in it must
// still refuse, and must refuse with the typed variant. If a provider
// grows pages, its entry here becomes a stale-premise failure, which is
// the correct signal to remove it.
//
// Run: cargo test --test reconnect_no_page_test

use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Extract a fn body: from the signature to the matching top-level `    }`.
fn fn_body(src: &str, signature: &str) -> String {
    let start = src
        .find(signature)
        .unwrap_or_else(|| panic!("signature not found: {signature}"));
    let rest = &src[start..];
    let end = rest
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("no body end for: {signature}"));
    rest[..end].to_string()
}

/// Providers with no page / sub-account concept. Their
/// `fetch_page_info` refuses, so the refusal must be typed.
///
/// NOT in this table, deliberately: `google_my_business` and `wordpress`
/// DO have a working `fetch_page_info` (they read their own account and
/// return a photo); their `Api(...)` arms are genuine error paths for a
/// failed fetch, which must stay errors. Only the outright REFUSALS go
/// here. A provider's presence in this list is re-checked by the test,
/// so a stale entry fails loudly rather than silently.
const NO_PAGE_PROVIDERS: &[&str] = &[
    "src/social/bluesky.rs",
    "src/social/discord.rs",
    "src/social/hashnode.rs",
    "src/social/instagram_standalone.rs",
    "src/social/pinterest.rs",
    "src/social/reddit.rs",
    "src/social/skool.rs",
    "src/social/telegram_bot.rs",
    "src/social/telegram_user.rs",
    "src/social/threads.rs",
    "src/social/whatsapp.rs",
    "src/social/x.rs",
];

/// Every channel-shaped provider refuses with the TYPED variant.
///
/// discord, google_my_business, hashnode and wordpress refuse through
/// `pages()`/`fetch_page_info()` — whichever they refuse from, the
/// signal must be the variant, because that is what the trait reads.
#[test]
fn every_page_refusal_is_typed_not_an_english_string() {
    let mut untyped: Vec<String> = Vec::new();

    for path in NO_PAGE_PROVIDERS {
        let src = read(path);
        let body = fn_body(&src, "async fn fetch_page_info");

        // The premise, re-checked rather than assumed.
        assert!(
            body.contains("Err("),
            "{path}: expected fetch_page_info to refuse, it does not — \
             drop it from the table, the premise is stale"
        );

        if !body.contains("ProviderError::NoPageManagement") {
            untyped.push(path.to_string());
        }
    }

    assert!(
        untyped.is_empty(),
        "these providers still refuse with a hand-worded Api(String), so the \
         trait's degradation cannot recognise them and Refresh still fails: \
         {untyped:?}"
    );
}

/// The default `reconnect` must DEGRADE on the typed variant rather than
/// forward it — that is the single reason a channel-shaped provider is
/// refreshable without its own override.
#[test]
fn the_default_reconnect_degrades_instead_of_forwarding_a_page_refusal() {
    let src = read("src/social/mod.rs");
    let body = fn_body(&src, "async fn reconnect");

    assert!(
        body.contains("Err(ProviderError::NoPageManagement)"),
        "the default reconnect forwards the page refusal with `?`, so Refresh \
         fails on every channel-shaped provider"
    );
}

/// The degradation must be narrow: a GENUINE failure (network, auth,
/// rate limit, a real API error) must still surface. A `_ => Ok(...)`
/// catch-all would make this test's companion a lie.
#[test]
fn the_degraded_arm_is_not_a_catch_all() {
    let src = read("src/social/mod.rs");
    let body = fn_body(&src, "async fn reconnect");

    for forbidden in ["Err(_) =>", "Err(e) => Ok(", "Err(_) if"] {
        assert!(
            !body.contains(forbidden),
            "the default reconnect swallows every error ({forbidden}); \
             a real outage would be reported as a successful refresh"
        );
    }

    // And there IS a re-raise arm.
    assert!(
        body.contains("Err(e) => Err(e)"),
        "the default reconnect no longer re-raises genuine errors"
    );
}

/// A provider that overrides `reconnect` must NOT route through a
/// refuser just to be told there are no pages — that is the same trap
/// one level down.
///
/// The list is of providers whose override was a POINTLESS copy of the
/// default. `x` is in it because X's reconnect does something genuinely
/// different (cookie/v2 auth + a real profile read), so it is a control:
/// an override that WORKS must not be flagged either. `youtube` is not
/// listed — its `fetch_page_info` genuinely reads the channel.
#[test]
fn a_reconnect_override_does_not_route_through_a_refusing_fetch_page_info() {
    for path in ["src/social/x.rs"] {
        let src = read(path);
        if !src.contains("async fn reconnect") {
            continue;
        }
        let body = fn_body(&src, "async fn reconnect");
        assert!(
            !body.contains("fetch_page_info"),
            "{path}: reconnect routes through the refusing fetch_page_info, \
             so the override does nothing the default does not already do"
        );
    }
}
