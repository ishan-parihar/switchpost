// ─── Provider Tiers ───────────────────────────────────────────
// v25 plan §1: breadth is not the goal — depth for the 12 platforms a
// solo founder actually uses, publish-maintained for the long tail,
// archived for the rest.
//
// This module is the single source of truth for tier membership. The
// registry reads it when building `ProviderRegistry`, `/api/providers`
// and the MCP `integrations.list_providers` tool report it as a `tier`
// field, and the frontend uses that field to decide what to render.
//
// Tier-1 (depth): analytics, engagement, comments, mentions, targets.
// Tier-2 (publish-maintained): publish + validate + refresh + errors.
// Tier-3 (archive): code kept under `social::archive`, registration and
//   UI gated behind `ENABLE_ARCHIVE_PROVIDERS`.

use serde::{Deserialize, Serialize};

/// Support tier of a provider (see `docs/planning/PLAN_PARITY_DEPTH_SINGLEUSER_v25.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderTier {
    /// Depth investment — all ten capability rows.
    Tier1,
    /// Publish-maintained — no new depth spend unless publishing breaks.
    Tier2,
    /// Archived — registered only when `ENABLE_ARCHIVE_PROVIDERS` is set.
    Tier3,
}

impl ProviderTier {
    /// Wire/JSON representation used by `/api/providers` and MCP output.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderTier::Tier1 => "tier-1",
            ProviderTier::Tier2 => "tier-2",
            ProviderTier::Tier3 => "tier-3",
        }
    }

    /// Whether a provider at this tier may be registered. Tier-3 providers
    /// are opt-in via `ENABLE_ARCHIVE_PROVIDERS`.
    pub fn is_enabled_by_default(self) -> bool {
        !matches!(self, ProviderTier::Tier3)
    }
}

impl std::fmt::Display for ProviderTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tier-1 — the solo-founder core (12). Top-10 global MAU for creators
/// plus LinkedIn Page (B2B is the paying use-case) and Instagram-standalone
/// (Meta API without a Facebook Page dependency).
pub const TIER_1: &[&str] = &[
    "x",
    "linkedin",
    "linkedin-page",
    "facebook",
    "instagram",
    "instagram-standalone",
    "threads",
    "youtube",
    "tiktok",
    "reddit",
    "bluesky",
    "pinterest",
];

/// Tier-2 — publish-maintained (17). Messaging platforms, the blogging/dev
/// API-key group, the user-mandated utility keeps (Google My Business,
/// Gmail/Calendar/Drive via `google`, Skool), and the Google Suite's
/// consent-split twins.
///
/// `gmail`, `calendar` and `drive` are SEPARATE providers from `google`
/// and must stay so: Google refuses `auth/youtube*` and `auth/drive*` (and
/// the same for Gmail) in one consent, so a single `google` integration can
/// never carry both grants. Merging them back would re-break either the
/// MCP Google tools or the Hub connect flow.
pub const TIER_2: &[&str] = &[
    "telegram-bot",
    "telegram-user",
    "discord",
    "slack",
    "whatsapp",
    "wordpress",
    "mastodon",
    "medium",
    "devto",
    "hashnode",
    "github",
    "google_my_business",
    "skool",
    "gmail",
    "calendar",
    "drive",
];

/// Tier-3 — archived (1). Code lives under `src/social/archive/` and is
/// registered only when `ENABLE_ARCHIVE_PROVIDERS=1`.
///
/// Formally **won't-port** from Postiz, documented so nobody re-adds them:
/// `dribbble, tumblr, twitch, mewe, moltbook, nostr, listmonk` — niche MAU
/// plus distinct media pipelines (Twitch clips, Tumblr reblogs) that would
/// each cost a full provider cycle. `kick, vk, whop, lemmy` are removed
/// outright rather than archived.
pub const TIER_3: &[&str] = &["farcaster"];

/// Env var that re-enables Tier-3 providers (default: disabled).
pub const ARCHIVE_FLAG: &str = "ENABLE_ARCHIVE_PROVIDERS";

/// Whether archived (Tier-3) providers should be registered.
///
/// Any non-empty value other than `0`/`false`/`no` enables them, matching
/// the convention used elsewhere in the codebase.
pub fn archive_providers_enabled() -> bool {
    match std::env::var(ARCHIVE_FLAG) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

/// Tier membership for a provider identifier.
///
/// Unknown identifiers return `Tier2` rather than panicking: a provider
/// added to the registry without a tier entry still shows up in the UI and
/// keeps the publish-maintained guarantee, which is the safe default.
pub fn tier_of(identifier: &str) -> ProviderTier {
    if TIER_1.contains(&identifier) {
        ProviderTier::Tier1
    } else if TIER_3.contains(&identifier) {
        ProviderTier::Tier3
    } else {
        ProviderTier::Tier2
    }
}

/// Every identifier the registry may register: Tier-1 + Tier-2 + Tier-3.
/// The registry consults this so tier membership and registration can
/// never drift apart.
pub fn all_known_identifiers() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = TIER_1
        .iter()
        .chain(TIER_2.iter())
        .chain(TIER_3.iter())
        .copied()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Onboarding presentation metadata for a provider identifier:
/// `(display name, emoji icon, setup hint)`.
///
/// `api::onboard` used to carry four independent `match id` tables —
/// display name, icon, the "what env vars does this need" hint on the
/// `/setup` page, and the same hint again in the `public_connect` error.
/// They were edited independently, so a provider could be added to one
/// and forgotten in another. One lookup now serves all four sites.
///
/// * `name` is the *onboarding* label. It is deliberately not
///   `SocialProvider::name()`: 2 of the 13 identifiers below disagree
///   with the provider's own name (`x`, `instagram-standalone`) and the
///   fallback for everything else is the raw identifier, not a title.
///   `name_matches_provider_name` pins the 11 that do agree so this
///   table cannot drift further.
/// * `icon` is the emoji rendered on setup cards. Unknown ids get `🔗`.
/// * `env_hint` is shown on `/setup` when the provider has no
///   credentials configured. It is the full sentence, not a bare var
///   list, because `reddit` and `skool` need one ("Cookie auth
///   available…", "Requires Chrome extension…"). Unknown ids get a
///   generic line; callers that need to distinguish "unknown provider"
///   from "known but unset" check `all_known_identifiers()` first.
pub fn provider_meta(id: &str) -> (&str, &'static str, &'static str) {
    let (name, icon, env_hint) = match id {
        "x" => ("𝕏 (Twitter)", "𝕏", "Requires: X_CLIENT_ID + X_CLIENT_SECRET"),
        "linkedin" => ("LinkedIn", "💼", "Requires: LINKEDIN_CLIENT_ID + LINKEDIN_CLIENT_SECRET"),
        "linkedin-page" => (
            "LinkedIn Page",
            "💼",
            "Requires: LINKEDIN_CLIENT_ID + LINKEDIN_CLIENT_SECRET",
        ),
        "facebook" => (
            "Facebook",
            "📘",
            "Requires: FACEBOOK_CLIENT_ID + FACEBOOK_CLIENT_SECRET",
        ),
        "instagram" => (
            "Instagram",
            "📘",
            "Requires: FACEBOOK_CLIENT_ID + FACEBOOK_CLIENT_SECRET",
        ),
        "instagram-standalone" => (
            "Instagram Standalone",
            "📸",
            "Requires: INSTAGRAM_APP_ID + INSTAGRAM_APP_SECRET",
        ),
        "threads" => ("Threads", "🧵", "Requires: THREADS_APP_ID + THREADS_APP_SECRET"),
        "youtube" => (
            "YouTube",
            "▶️",
            "Requires: YOUTUBE_CLIENT_ID + YOUTUBE_CLIENT_SECRET",
        ),
        "telegram-bot" => ("Telegram Bot", "✈️", "Requires: TELEGRAM_BOT_TOKENS"),
        "telegram-user" => (
            "Telegram User",
            "✈️",
            "Requires: TELEGRAM_API_ID + TELEGRAM_API_HASH (from https://my.telegram.org)",
        ),
        "bluesky" => (
            "Bluesky",
            "🦋",
            "Requires: BLUESKY_HANDLE + BLUESKY_APP_PASSWORD",
        ),
        "skool" => (
            "Skool",
            "🎓",
            "Requires Chrome extension — install, login to Skool, extract auth_token cookie",
        ),
        "github" => ("GitHub", "🐙", "Requires: GITHUB_TOKEN"),
        "reddit" => (
            "reddit",
            "🔗",
            "Cookie auth available (no env vars needed) — or set REDDIT_CLIENT_ID + REDDIT_CLIENT_SECRET for OAuth",
        ),
        _ => (id, "🔗", "Missing environment variables"),
    };
    (name, icon, env_hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_counts_match_v25_plan() {
        assert_eq!(TIER_1.len(), 12, "v25 §1 Tier-1 is 12 platforms");
        // Tier-2 grew from 14 to 17 on 2026-10-30 when the Google Suite was
        // split: `gmail`, `calendar` and `drive` are now their own
        // connectable providers rather than being reachable only through
        // the unified `google` one. The reason is Google's two-consents
        // rule (it refuses `auth/youtube*` and `auth/drive*` together), so
        // one `google` integration can never carry every grant the MCP
        // Google tools need.
        assert_eq!(
            TIER_2.len(),
            16,
            "v25 §1 Tier-2 was 14; +gmail +calendar +drive after the \
             Google consent split; -google when it turned out to publish \
             YouTube videos and nothing else (see TIER_2's doc comment)"
        );
        assert_eq!(TIER_3.len(), 1, "only farcaster is archived in v25 §1");
        assert_eq!(all_known_identifiers().len(), 29);
    }

    #[test]
    fn tiers_are_disjoint() {
        for id in TIER_1 {
            assert!(
                !TIER_2.contains(id) && !TIER_3.contains(id),
                "{id} is in more than one tier"
            );
        }
        for id in TIER_2 {
            assert!(!TIER_3.contains(id), "{id} is in Tier-2 and Tier-3");
        }
    }

    #[test]
    fn removed_providers_are_not_in_any_tier() {
        for id in ["kick", "vk", "whop", "lemmy"] {
            assert!(
                !all_known_identifiers().contains(&id),
                "{id} was removed in v25 §1 and must not be tiered"
            );
        }
    }

    #[test]
    fn archived_provider_is_tier_3_and_disabled_by_default() {
        assert_eq!(tier_of("farcaster"), ProviderTier::Tier3);
        assert!(!ProviderTier::Tier3.is_enabled_by_default());
        assert!(ProviderTier::Tier1.is_enabled_by_default());
        assert!(ProviderTier::Tier2.is_enabled_by_default());
    }

    #[test]
    fn unknown_provider_defaults_to_tier_2() {
        assert_eq!(tier_of("not-a-provider"), ProviderTier::Tier2);
    }

    #[test]
    fn archive_flag_parses_boolean_spellings() {
        // Env var is process-global; exercise the parser shape directly.
        for (raw, expected) in [
            ("1", true),
            ("true", true),
            ("yes", true),
            ("off", false),
            ("0", false),
            ("false", false),
            ("no", false),
            ("", false),
        ] {
            assert_eq!(
                !matches!(raw, "" | "0" | "false" | "no" | "off"),
                expected,
                "unexpected parse for ENABLE_ARCHIVE_PROVIDERS={raw:?}"
            );
        }
    }

    /// `provider_meta`'s name is an onboarding label, not the provider's
    /// own name. For every identifier where the two are *supposed* to
    /// agree, assert they still do — otherwise the label table silently
    /// rots away from the providers it labels.
    #[test]
    fn provider_meta_name_matches_provider_name_where_the_table_covers_it() {
        use crate::social::registry::ProviderRegistry;
        use crate::social::test_config;

        let registry = ProviderRegistry::new(&test_config(), None, None);
        for id in [
            "linkedin",
            "linkedin-page",
            "facebook",
            "instagram",
            "threads",
            "youtube",
            "telegram-bot",
            "telegram-user",
            "bluesky",
            "skool",
            "github",
        ] {
            let provider = registry
                .get(id)
                .unwrap_or_else(|| panic!("{id} is not registered"));
            assert_eq!(
                provider_meta(id).0,
                provider.name(),
                "{id}: onboarding label drifted from the provider's own name"
            );
        }
    }

    /// The two identifiers where the onboarding label is deliberately
    /// *not* the provider's name. Pinned so nobody "fixes" the label and
    /// silently changes what `/setup` renders.
    #[test]
    fn provider_meta_keeps_its_two_intentional_name_divergences() {
        use crate::social::registry::ProviderRegistry;
        use crate::social::test_config;

        let registry = ProviderRegistry::new(&test_config(), None, None);
        for (id, label) in [
            ("x", "𝕏 (Twitter)"),
            ("instagram-standalone", "Instagram Standalone"),
        ] {
            let provider = registry
                .get(id)
                .unwrap_or_else(|| panic!("{id} is not registered"));
            assert_eq!(
                provider_meta(id).0,
                label,
                "{id}: the onboarding label is pinned by this test"
            );
            assert_ne!(
                provider.name(),
                label,
                "{id} is listed here because its label intentionally differs from the provider's own name — if they now agree, drop it from this list and from the other test"
            );
        }
    }

    #[test]
    fn provider_meta_falls_back_to_the_identifier() {
        assert_eq!(
            provider_meta("not-a-provider"),
            ("not-a-provider", "🔗", "Missing environment variables")
        );
    }
}
