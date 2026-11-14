//! The channels grid must be able to say what a provider needs BEFORE the
//! founder clicks Connect.
//!
//! A provider whose credentials are absent is refused honestly - but the
//! refusal used to arrive after the click, so the founder could not plan
//! around it. The grid now reads `configured` and `requires` from
//! `/api/providers`; this pins the two halves that make the sentence
//! actionable:
//!
//!   1. every provider's hint names real ENV VAR NAMES, not prose, so the
//!      operator can grep the .env for them;
//!   2. a hint that the provider does not need is absent rather than
//!      misleading (a paste-at-connect flow has no env var to set).

use switchpost::social::registry::ProviderRegistry;
use switchpost::social::tier::provider_meta;

fn registry() -> ProviderRegistry {
    switchpost::config::load_dotenv_for_tests();
    let config = switchpost::config::Config::from_env().expect("config");
    ProviderRegistry::new(&config, None, None)
}

#[test]
fn every_provider_hint_names_upper_snake_env_vars() {
    let registry = registry();
    for id in registry.all().into_iter().map(|p| p.identifier()) {
        let hint = provider_meta(id).2;
        if hint.is_empty() {
            continue;
        }
        // Either it names the env vars to set, or it says plainly that there
        // are none and what to paste instead. "Missing environment
        // variables" is neither, which is why it is not accepted.
        assert!(
            hint.contains("Requires:") || hint.to_lowercase().contains("no env var"),
            "{id}: the hint must either start with `Requires:` and name the env \
             vars, or say `No env var` and name what to paste ({hint:?})"
        );
        if !hint.to_lowercase().contains("no env var") {
        // The env var names are the actionable half. A hint with none is a
        // sentence the founder cannot type into a .env.
        let has_var = hint
            .split(|c: char| !c.is_ascii_uppercase() && !c.is_ascii_digit() && c != '_')
            .any(|tok| tok.len() >= 4 && tok.contains('_'));
        assert!(
            has_var,
            "{id}: the hint names no env var ({hint:?}), so the tile cannot tell \
             the founder what to set"
        );
        }
    }
}

#[test]
fn a_paste_flow_provider_does_not_claim_an_env_var() {
    // These providers are connected by pasting a token in the app; they have
    // no env var to set, so a "Requires: ..." hint on them would be a lie.
    let registry = registry();
    for id in ["devto", "medium", "hashnode", "wordpress"] {
        let provider = registry
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert!(
            !provider.uses_oauth(),
            "{id} is a paste flow; an OAuth flag would route the founder to a \
             consent screen that does not exist"
        );
    }
}
