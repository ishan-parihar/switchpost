// ─── Provider Registry ────────────────────────────────────────
// Central registry of all available social media providers.
// Used by both the API layer and the MCP layer to route requests.
//
// Also holds a per-provider concurrency limiter (`Semaphore`) so the
// scheduler can't fire 30 simultaneous posts at the same X account
// and trip per-account rate limits. The limit defaults to 1 for
// strict-serial platforms (X, Threads, Instagram — all have aggressive
// per-account rate windows) and 3 for platforms with more headroom
// (Reddit, Discord, Slack, Telegram-Bot, etc.). Override via the
// `PROVIDER_CONCURRENCY_{IDENTIFIER}` env var (uppercased, hyphens →
// underscores, e.g. `PROVIDER_CONCURRENCY_LINKEDIN_PAGE=2`).

use std::collections::HashMap;
use std::sync::Arc;

use super::archive::farcaster;
use super::mastodon;
use super::slack;
use super::tier::{self, ProviderTier};
use super::*;
use crate::config::Config;
use crate::services::telegram_client::TelegramClientManager;
use crate::wa::WhaClient;

/// Default per-provider concurrent publish budget. Conservative to
/// avoid tripping per-account rate limits on the strictest platforms
/// (X free tier ≈ 17 posts / 24h; we don't want to blow through
/// that in a single scheduler tick).
const DEFAULT_PROVIDER_CONCURRENCY: usize = 1;

/// Providers that can safely handle more concurrent calls (their rate
/// limits are per-IP or per-token-with-high-ceiling, not per-account).
const HIGH_CONCURRENCY_PROVIDERS: &[&str] = &[
    "reddit",
    "discord",
    "slack",
    "telegram-bot",
    "telegram-user",
    "whatsapp",
    "github",
    "wordpress",
    "medium",
    "devto",
    "hashnode",
    "skool",
];

/// Thread-safe provider registry
#[derive(Clone)]
pub struct ProviderRegistry {
    providers: Arc<HashMap<&'static str, Arc<dyn SocialProvider>>>,
    /// Per-provider concurrency limiter. Each entry is an
    /// `Arc<Semaphore>` sized to the platform's concurrent-post budget.
    /// The scheduler acquires a permit before calling `provider.publish()`
    /// and releases it when the publish completes (success or failure).
    concurrency: Arc<HashMap<&'static str, Arc<tokio::sync::Semaphore>>>,
    /// Boot-resolved per-provider permit budget — sizing source for each
    /// account's semaphore on first use (C1: breaker + semaphore are per
    /// ACCOUNT; the provider-level maps remain the fallback for env-token
    /// calls that have no integration row).
    concurrency_limits: Arc<HashMap<&'static str, usize>>,
    /// Insert-on-demand per-account semaphores, key "{provider}:{integration_id}".
    account_concurrency: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
    /// Per-provider circuit breaker. When a provider has N consecutive
    /// failures (e.g. 5xx from the platform API), the circuit opens and
    /// subsequent publish attempts are skipped for a cooldown period
    /// (default 60s) instead of burning through all queued posts.
    /// After the cooldown, the circuit goes half-open: one request is
    /// allowed through; if it succeeds, the circuit closes; if it fails,
    /// the cooldown restarts.
    circuit_breakers: Arc<HashMap<&'static str, Arc<CircuitBreaker>>>,
    /// Boot-resolved (threshold, cooldown) per provider — sizing source
    /// for per-account breakers.
    cb_config: Arc<HashMap<&'static str, (u32, i64)>>,
    /// Insert-on-demand per-account breakers, key "{provider}:{integration_id}".
    account_breakers: Arc<std::sync::Mutex<HashMap<String, Arc<CircuitBreaker>>>>,
}

/// Per-provider circuit breaker with three states: closed, open, half-open.
///
/// - **Closed**: all requests pass through. Failure count tracked.
/// - **Open**: all requests rejected immediately for `cooldown_secs`.
///   After cooldown, transitions to half-open.
/// - **Half-open**: one request allowed through. If it succeeds → closed.
///   If it fails → back to open with full cooldown.
///
/// This prevents a platform outage (e.g. X 5xx for 10 minutes) from
/// cascading to every queued post. Without it, 50 queued X posts would
/// each independently fail their 3 retries = 150 doomed API calls.
pub struct CircuitBreaker {
    state: std::sync::atomic::AtomicU8, // 0=closed, 1=open, 2=half-open
    failure_count: std::sync::atomic::AtomicU32,
    opened_at: std::sync::atomic::AtomicI64, // unix timestamp
    /// Number of consecutive failures before opening.
    failure_threshold: u32,
    /// Seconds to wait before transitioning from open to half-open.
    cooldown_secs: i64,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown_secs: i64) -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(0), // closed
            failure_count: std::sync::atomic::AtomicU32::new(0),
            opened_at: std::sync::atomic::AtomicI64::new(0),
            failure_threshold,
            cooldown_secs,
        }
    }

    /// Returns `true` if the request should be allowed through,
    /// `false` if the circuit is open (request should be skipped).
    ///
    /// If the circuit is open but the cooldown has elapsed, this
    /// transitions to half-open and allows one request through.
    ///
    /// v22 Phase 2 (BUG #3): Half-open now admits exactly ONE request.
    /// Previously `allow_request()` returned `true` unconditionally for
    /// half-open, so if 5 posts for the same provider were claimed in
    /// one tick and the circuit just transitioned to half-open, all 5
    /// were spawned, all 5 hit the platform, and 5 failures re-opened
    /// the circuit — defeating the purpose of half-open. Now we
    /// atomically transition half-open → open on first admission using
    /// `compare_exchange`, so subsequent callers in the same tick see
    /// `open` and are rejected.
    pub fn allow_request(&self) -> bool {
        use std::sync::atomic::Ordering;
        let state = self.state.load(Ordering::SeqCst);
        match state {
            0 => true, // closed — all requests pass
            1 => {
                // open — check if cooldown elapsed
                let now = chrono::Utc::now().timestamp();
                let opened = self.opened_at.load(Ordering::SeqCst);
                if now - opened >= self.cooldown_secs {
                    // Attempt to transition open → half-open atomically.
                    // If another thread beat us to it, we'll see the
                    // updated state and fall through to the half-open
                    // branch below.
                    let _ = self
                        .state
                        .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst);
                    tracing::info!("Circuit breaker → half-open (cooldown elapsed)");
                    // Fall through to half-open handling: admit exactly one.
                    self.admit_one_from_half_open()
                } else {
                    false
                }
            }
            2 => {
                // half-open — admit exactly one request, then flip back
                // to open so concurrent callers in the same tick are
                // rejected. The single admitted request will either
                // record_success (→ closed) or record_failure (→ open
                // with full cooldown).
                self.admit_one_from_half_open()
            }
            _ => true,
        }
    }

    /// Atomically admit one request from the half-open state by
    /// transitioning half-open → open. The admitted request's outcome
    /// (record_success / record_failure) will then determine the next
    /// transition. Returns `true` if this caller won the admission
    /// race, `false` if another caller already won it.
    fn admit_one_from_half_open(&self) -> bool {
        use std::sync::atomic::Ordering;
        // Try to flip 2 (half-open) → 1 (open). If we succeed, we're
        // the one admitted request. If we fail, someone else already
        // claimed the slot — reject.
        match self
            .state
            .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => {
                // We won the admission race. Set opened_at so the
                // cooldown for the NEXT half-open attempt starts now
                // (in case record_success/record_failure never fires,
                // e.g. the task panics).
                self.opened_at
                    .store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
                true
            }
            Err(_) => false, // someone else was admitted; reject
        }
    }

    /// Record a successful request. Resets failure count and closes
    /// the circuit (if it was open or half-open).
    pub fn record_success(&self) {
        use std::sync::atomic::Ordering;
        let prev = self.state.swap(0, Ordering::SeqCst); // closed
        self.failure_count.store(0, Ordering::SeqCst);
        if prev != 0 {
            tracing::info!("Circuit breaker → closed (success recorded)");
        }
    }

    /// Give back a half-open admission whose request never reached the
    /// platform (for example a thread part whose predecessor is unpublished).
    /// Without this the breaker stays open for another full cooldown although
    /// nothing was learned about the platform.
    pub fn release_probe(&self) {
        use std::sync::atomic::Ordering;
        let _ = self
            .state
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Record a failed request. Increments failure count; if it
    /// reaches the threshold, opens the circuit.
    pub fn record_failure(&self) {
        use std::sync::atomic::Ordering;
        let count = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
        if count >= self.failure_threshold {
            let prev = self.state.swap(1, Ordering::SeqCst); // open
            self.opened_at
                .store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
            if prev != 1 {
                tracing::warn!("Circuit breaker → open ({} consecutive failures)", count);
            }
        }
    }
}

impl ProviderRegistry {
    /// Build registry with all providers, given app config for credentials
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &Config,
        telegram_client_manager: Option<Arc<TelegramClientManager>>,
        wa_client: Option<Arc<tokio::sync::Mutex<WhaClient>>>,
    ) -> Self {
        let mut providers: HashMap<&'static str, Arc<dyn SocialProvider>> = HashMap::new();

        // B10 startup self-check: warn loudly when the pinned LinkedIn
        // version approaches its ~12-month sunset, so the bump never comes
        // as a surprise outage.
        if let Some((years, months)) = crate::social::linkedin::linkedin_version_age_years_months(
            crate::social::linkedin::LINKEDIN_VERSION,
            chrono::Utc::now(),
        ) {
            let age_months = years * 12 + months;
            if age_months >= 11 {
                tracing::warn!(
                    "LinkedIn-Version {} is {}y{}m old — LinkedIn sunsets versions ~12 months after release. Bump LINKEDIN_VERSION.",
                    crate::social::linkedin::LINKEDIN_VERSION,
                    years,
                    months
                );
            }
        }

        // Every Tier-1 / Tier-2 provider registers UNCONDITIONALLY.
        //
        // Registration is a catalogue decision, not a credentials decision.
        // Gating it on `config.*_client_id.is_some()` made `/api/providers`
        // advertise 19 of 26 on a clean environment while `/channels` rendered
        // all 26 cards and the MCP layer advertised every tool — three
        // surfaces, three different answers to "what does this build support?".
        //
        // Every provider's `new(config)` is infallible: missing credentials
        // land as empty strings, a default instance URL, or a cloned config,
        // and the call that needs a credential fails with the ordinary
        // not-connected / auth error. That is the same shape reddit, pinterest,
        // wordpress and skool have always had, and it is what the UI's
        // "Connect" button needs to exist to offer in the first place.
        //
        // Tier-3 (archive) stays gated — see `tier::archive_providers_enabled`.

        // Current providers
        providers.insert("x", Arc::new(x::XProvider::new(config)));
        providers.insert(
            "linkedin",
            Arc::new(linkedin::LinkedInProvider::new(config)),
        );
        providers.insert("bluesky", Arc::new(bluesky::BlueskyProvider::new(config)));
        providers.insert(
            "facebook",
            Arc::new(facebook::FacebookProvider::new(config)),
        );
        providers.insert(
            "instagram",
            Arc::new(instagram::InstagramProvider::new(config)),
        );

        // LinkedIn Page — shares LinkedIn's OAuth app credentials, so it is
        // constructible either way.
        providers.insert(
            "linkedin-page",
            Arc::new(linkedin_page::LinkedInPageProvider::new(config)),
        );

        providers.insert(
            "instagram-standalone",
            Arc::new(instagram_standalone::InstagramStandaloneProvider::new(
                config,
            )),
        );

        providers.insert("threads", Arc::new(threads::ThreadsProvider::new(config)));

        providers.insert("reddit", Arc::new(reddit::RedditProvider::new(config)));

        providers.insert("discord", Arc::new(discord::DiscordProvider::new(config)));

        // Telegram Bot — token-based accounts (comma-separated TELEGRAM_BOT_TOKENS)
        providers.insert(
            "telegram-bot",
            Arc::new(telegram_bot::TelegramBotProvider::new(config)),
        );

        // Telegram User — Gramers-based MTProto client
        providers.insert(
            "telegram-user",
            Arc::new(telegram_user::TelegramUserProvider::new(
                config,
                telegram_client_manager.clone(),
            )),
        );

        providers.insert(
            "pinterest",
            Arc::new(pinterest::PinterestProvider::new(config)),
        );

        // WhatsApp — native wa-rs client with wacli fallback
        providers.insert(
            "whatsapp",
            Arc::new(whatsapp::WhatsAppProvider::new(config, wa_client.clone())),
        );

        // TikTok — OAuth-based video platform.
        // v25 §1: no separate `tiktok-business` provider. Business and
        // creator accounts both publish through the same Content Posting
        // API with the same scopes; only the account type differs, so the
        // "merge" is structural — there is nothing to gate.
        providers.insert("tiktok", Arc::new(tiktok::TikTokProvider::new(config)));

        // Google My Business — uses same Google OAuth credentials
        providers.insert(
            "google_my_business",
            Arc::new(google_my_business::GoogleMyBusinessProvider::new(config)),
        );

        // Mastodon — OAuth-based microblogging (with app registration).
        // v25 §1: no separate `mastodon-custom` provider. Self-hosted
        // instances are reached through `MASTODON_INSTANCE_URL` /
        // `instance_url`, so the custom-instance path is already part of
        // this provider rather than a gated sibling. `new` falls back to
        // mastodon.social when the instance env var is unset.
        providers.insert(
            "mastodon",
            Arc::new(mastodon::MastodonProvider::new(config)),
        );

        // Medium — API key-based publishing
        providers.insert("medium", Arc::new(medium::MediumProvider::new(config)));

        // Dev.to — API key-based publishing
        providers.insert("devto", Arc::new(devto::DevtoProvider::new(config)));

        // Hashnode — API key-based blogging
        providers.insert(
            "hashnode",
            Arc::new(hashnode::HashnodeProvider::new(config)),
        );

        // GitHub — PAT-based
        providers.insert("github", Arc::new(github::GithubProvider::new(config)));

        // YouTube — dedicated provider for importing recent videos
        // Uses YOUTUBE_CLIENT_ID / YOUTUBE_CLIENT_SECRET
        providers.insert("youtube", Arc::new(youtube::YoutubeProvider::new(config)));

        // Google Suite — unified provider for Gmail, Calendar, Drive
        // Uses YOUTUBE_CLIENT_ID / YOUTUBE_CLIENT_SECRET for all Google OAuth scopes
        providers.insert("google", Arc::new(google::GoogleProvider::new(config)));

        // Drive — connectable separately from the unified `google` provider.
        //
        // WHY A SECOND GOOGLE PROVIDER EXISTS
        //
        // Google refuses `auth/youtube*` and `auth/drive*` in ONE consent
        // ("This request contains scopes that cannot be requested together"),
        // so the unified `google` provider can ask for YouTube+Gmail+Calendar
        // OR for Drive, never both. Drive's MCP tools (list/get/search/export)
        // need a Drive-scoped token, and the Hub tools need a
        // YouTube+Gmail one. They cannot share one grant.
        //
        // So `drive` is its own connectable provider: the founder connects
        // it separately, and `find_goog_token` prefers whichever integration
        // actually carries a Drive grant rather than guessing from the
        // `google` one. Registering it here (not leaving it reachable only
        // through `google`) is what keeps the Drive tools working when the
        // Hub scopes were narrowed to fix the live `invalid_request`.
        providers.insert("drive", Arc::new(drive::DriveProvider::new(config)));

        // Gmail — connectable in its own right.
        //
        // The dashboard's Available Providers grid has offered a Gmail
        // connect card since the grid was written, but the registry never
        // registered a `gmail` provider: the card 404'd, and (because the
        // SPA fallback used to answer unknown /api/* paths with 200 HTML)
        // it failed as "Invalid JSON response from
        // /api/integrations/connect/gmail", which reads as a server bug
        // rather than a missing route.
        //
        // Same two-consent rule as Drive: Google refuses `auth/youtube*`
        // beside `auth/drive*`, so the unified `google` integration and
        // the Gmail one can not share a grant either.
        providers.insert("gmail", Arc::new(gmail::GmailProvider::new(config)));

        // Calendar — the third Google Suite surface. Same reasoning.
        providers.insert(
            "calendar",
            Arc::new(calendar::CalendarProvider::new(config)),
        );

        // Chrome extension-based provider (no OAuth credentials needed)
        providers.insert("skool", Arc::new(skool::SkoolProvider::new()));

        // WordPress — REST API + Application Password (no global credentials)
        providers.insert(
            "wordpress",
            Arc::new(wordpress::WordPressProvider::new(config)),
        );

        // Slack — OAuth-based messaging workspace provider
        providers.insert("slack", Arc::new(slack::SlackProvider::new(config)));

        // Farcaster — Tier-3 archive (v25 §1). Web3/Neynar, no OAuth.
        // Compiles from `social::archive` but registers only when the
        // operator opts back in via ENABLE_ARCHIVE_PROVIDERS.
        if tier::archive_providers_enabled() {
            providers.insert(
                "farcaster",
                Arc::new(farcaster::FarcasterProvider::new(config)),
            );
        }

        tracing::info!(
            "Provider registry initialized with: {}",
            providers.keys().cloned().collect::<Vec<_>>().join(", ")
        );

        // Build per-provider concurrency semaphores. Each provider
        // gets a budget based on its platform's rate-limit profile.
        let mut concurrency: HashMap<&'static str, Arc<tokio::sync::Semaphore>> = HashMap::new();
        let mut concurrency_limits: HashMap<&'static str, usize> = HashMap::new();
        let mut circuit_breakers: HashMap<&'static str, Arc<CircuitBreaker>> = HashMap::new();
        let mut cb_config: HashMap<&'static str, (u32, i64)> = HashMap::new();
        for &id in providers.keys() {
            let default = if HIGH_CONCURRENCY_PROVIDERS.contains(&id) {
                3
            } else {
                DEFAULT_PROVIDER_CONCURRENCY
            };
            // Allow env override: PROVIDER_CONCURRENCY_X=2 etc.
            let env_key = format!(
                "PROVIDER_CONCURRENCY_{}",
                id.to_uppercase().replace('-', "_")
            );
            let limit = std::env::var(&env_key)
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &usize| *v >= 1 && *v <= 20)
                .unwrap_or(default);
            concurrency.insert(id, Arc::new(tokio::sync::Semaphore::new(limit)));
            concurrency_limits.insert(id, limit);

            // Circuit breaker: 5 consecutive failures → open for 60s.
            // Configurable via PROVIDER_CB_THRESHOLD_{ID} and
            // PROVIDER_CB_COOLDOWN_{ID} env vars.
            let threshold: u32 = std::env::var(format!(
                "PROVIDER_CB_THRESHOLD_{}",
                id.to_uppercase().replace('-', "_")
            ))
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v >= 1 && *v <= 50)
            .unwrap_or(5);
            let cooldown: i64 = std::env::var(format!(
                "PROVIDER_CB_COOLDOWN_{}",
                id.to_uppercase().replace('-', "_")
            ))
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v >= 10 && *v <= 3600)
            .unwrap_or(60);
            circuit_breakers.insert(id, Arc::new(CircuitBreaker::new(threshold, cooldown)));
            cb_config.insert(id, (threshold, cooldown));
        }

        // Tier drift guard: every registered provider must be a known tier
        // member. Since registration stopped being credential-conditional, the
        // useful assertion is the other direction too — every non-archive tier
        // member must be registered — so a provider dropped from this function
        // is caught here instead of surfacing as a connect button that 404s.
        let registered: Vec<&str> = providers.keys().copied().collect();
        let unknown: Vec<&&str> = registered
            .iter()
            .filter(|id| !tier::all_known_identifiers().contains(*id))
            .collect();
        if !unknown.is_empty() {
            tracing::warn!(
                "providers registered but missing from src/social/tier.rs: {:?}",
                unknown
            );
        }
        let missing: Vec<&&str> = tier::TIER_1
            .iter()
            .chain(tier::TIER_2.iter())
            .filter(|id| !registered.contains(id))
            .collect();
        if !missing.is_empty() {
            tracing::warn!(
                "tier-1/tier-2 providers not registered in the registry: {:?}",
                missing
            );
        }

        Self {
            providers: Arc::new(providers),
            concurrency: Arc::new(concurrency),
            concurrency_limits: Arc::new(concurrency_limits),
            account_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
            circuit_breakers: Arc::new(circuit_breakers),
            cb_config: Arc::new(cb_config),
            account_breakers: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Build a registry holding only the given providers.
    ///
    /// `new()` always registers the 26 real providers, so a service-level
    /// test that routes through it would hit a live platform API. This
    /// injects a fake `SocialProvider` instead, letting `PostService`
    /// be exercised offline. The limiter and breaker are still populated
    /// so a test touching them behaves like production.
    #[cfg(test)]
    pub fn with_providers(providers: Vec<(&'static str, Arc<dyn SocialProvider>)>) -> Self {
        let mut map: HashMap<&'static str, Arc<dyn SocialProvider>> = HashMap::new();
        for (id, provider) in providers {
            map.insert(id, provider);
        }
        let concurrency = map
            .keys()
            .map(|id| (*id, Arc::new(tokio::sync::Semaphore::new(1))))
            .collect();
        let concurrency_limits = map.keys().map(|id| (*id, 1usize)).collect();
        let circuit_breakers = map
            .keys()
            .map(|id| (*id, Arc::new(CircuitBreaker::new(5, 60))))
            .collect();
        let cb_config = map.keys().map(|id| (*id, (5u32, 60i64))).collect();
        Self {
            providers: Arc::new(map),
            concurrency: Arc::new(concurrency),
            concurrency_limits: Arc::new(concurrency_limits),
            account_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
            circuit_breakers: Arc::new(circuit_breakers),
            cb_config: Arc::new(cb_config),
            account_breakers: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Get a provider by identifier
    pub fn get(&self, identifier: &str) -> Option<Arc<dyn SocialProvider>> {
        self.providers.get(identifier).cloned()
    }

    /// Get the per-provider concurrency semaphore for the given
    /// provider. Returns `None` if the provider isn't registered.
    /// The caller should `.acquire()` before making the platform API
    /// call and hold the permit until the call completes.
    pub fn concurrency(&self, identifier: &str) -> Option<Arc<tokio::sync::Semaphore>> {
        self.concurrency.get(identifier).cloned()
    }

    /// Get the per-provider circuit breaker. The scheduler calls
    /// `allow_request()` before publishing — if it returns `false`,
    /// the post is left in `queued` state (not marked as error) and
    /// retried on the next tick after the cooldown elapses.
    pub fn circuit_breaker(&self, identifier: &str) -> Option<Arc<CircuitBreaker>> {
        self.circuit_breakers.get(identifier).cloned()
    }

    /// C1: the concurrency limiter for ONE account. `integration = None`
    /// (env-token calls with no row) falls back to the provider-level
    /// semaphore. Per-account entries are created on first use with the
    /// provider's permit budget — two X accounts each get their own gate,
    /// so one slow publish on @a does not block @b.
    pub fn concurrency_for(
        &self,
        identifier: &str,
        integration: Option<uuid::Uuid>,
    ) -> Option<Arc<tokio::sync::Semaphore>> {
        let integration = integration?;
        let limit = self.concurrency_limits.get(identifier).copied()?;
        let key = format!("{identifier}:{integration}");
        let mut accounts = self
            .account_concurrency
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Some(
            accounts
                .entry(key)
                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(limit)))
                .clone(),
        )
    }

    /// C1: the circuit breaker for ONE account. `integration = None` falls
    /// back to the provider-level breaker. A revoked token opens the breaker
    /// for THAT account only — the other accounts of the same platform keep
    /// publishing.
    pub fn circuit_breaker_for(
        &self,
        identifier: &str,
        integration: Option<uuid::Uuid>,
    ) -> Option<Arc<CircuitBreaker>> {
        let integration = integration?;
        let (threshold, cooldown) = self.cb_config.get(identifier).copied()?;
        let key = format!("{identifier}:{integration}");
        let mut accounts = self
            .account_breakers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Some(
            accounts
                .entry(key)
                .or_insert_with(|| Arc::new(CircuitBreaker::new(threshold, cooldown)))
                .clone(),
        )
    }

    /// List all registered provider identifiers
    pub fn list(&self) -> Vec<&'static str> {
        self.providers.keys().copied().collect()
    }

    /// Support tier for a provider identifier (v25 §1).
    ///
    /// Sourced from `tier::tier_of`, so `/api/providers` and the MCP
    /// `integrations.list_providers` tool can report it without the
    /// registry holding a second copy of the tier tables.
    pub fn tier(&self, identifier: &str) -> ProviderTier {
        tier::tier_of(identifier)
    }

    /// Get all providers
    pub fn all(&self) -> Vec<Arc<dyn SocialProvider>> {
        self.providers.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::test_config;

    /// C1: breakers are per ACCOUNT. Five failures on account A open ITS
    /// circuit; account B of the same provider keeps publishing, and the
    /// provider-level breaker (env-token fallback) is untouched.
    #[test]
    fn breaker_open_on_one_account_does_not_block_the_other() {
        let registry = ProviderRegistry::new(&test_config(), None, None);
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();

        let breaker_a = registry.circuit_breaker_for("x", Some(a)).unwrap();
        let breaker_b = registry.circuit_breaker_for("x", Some(b)).unwrap();
        let provider_level = registry.circuit_breaker("x").unwrap();

        // Open A: 5 consecutive failures (the test threshold).
        for _ in 0..5 {
            breaker_a.record_failure();
        }
        assert!(!breaker_a.allow_request(), "account A must be open");
        assert!(breaker_b.allow_request(), "account B must stay closed");
        assert!(
            provider_level.allow_request(),
            "provider-level fallback must stay closed"
        );

        // Distinct instances per account.
        let breaker_a2 = registry.circuit_breaker_for("x", Some(a)).unwrap();
        assert!(
            !breaker_a2.allow_request(),
            "same key must return the SAME breaker"
        );
    }

    /// C1: `None` (env-token call, no integration row) falls back to the
    /// provider-level maps — the old single-gate behavior.
    #[test]
    fn concurrency_for_none_falls_back_to_provider_level() {
        let registry = ProviderRegistry::new(&test_config(), None, None);
        let a = uuid::Uuid::new_v4();
        assert!(
            registry.concurrency_for("x", None).is_none(),
            "None → callers fall back to concurrency()"
        );
        assert!(registry.concurrency_for("x", Some(a)).is_some());
        assert!(registry.circuit_breaker_for("x", None).is_none());
    }

    /// Registration must not depend on credentials.
    ///
    /// This is the regression the credential gating caused: on a clean
    /// environment `/api/providers` advertised 19 of 26 while `/channels`
    /// rendered all 26 cards and MCP advertised every tool. `test_config()`
    /// carries no provider credentials, so a credential-conditional insert
    /// fails this assertion.
    #[test]
    fn registry_lists_every_default_tier_member_without_credentials() {
        let registry = ProviderRegistry::new(&test_config(), None, None);
        let mut ids = registry.list();
        ids.sort_unstable();

        let mut expected: Vec<&'static str> = tier::TIER_1
            .iter()
            .chain(tier::TIER_2.iter())
            .copied()
            .collect();
        expected.sort_unstable();
        expected.dedup();

        assert_eq!(
            ids,
            expected,
            "a clean environment must register all {} default providers",
            expected.len()
        );
        assert_eq!(
            ids.len(),
            29,
            "v25 §1 was 12 Tier-1 + 14 Tier-2 = 26; +gmail +calendar +drive \\
             after the Google consent split (see TIER_2's doc comment) makes it 29"
        );
    }

    /// Every registered identifier must be a known tier member (drift guard).
    #[test]
    fn registry_contains_no_identifier_outside_the_tier_tables() {
        let registry = ProviderRegistry::new(&test_config(), None, None);
        for id in registry.list() {
            assert!(
                tier::all_known_identifiers().contains(&id),
                "{id} is registered but absent from src/social/tier.rs"
            );
        }
    }

    /// Every registered provider gets a concurrency limiter and a circuit
    /// breaker, or the scheduler has nothing to acquire and outages cascade.
    #[test]
    fn every_registered_provider_has_a_limiter_and_breaker() {
        let registry = ProviderRegistry::new(&test_config(), None, None);
        for id in registry.list() {
            assert!(
                registry.concurrency(id).is_some(),
                "{id} has no concurrency limiter"
            );
            assert!(
                registry.circuit_breaker(id).is_some(),
                "{id} has no circuit breaker"
            );
        }
    }
}

#[cfg(test)]
mod circuit_breaker_tests {
    use super::CircuitBreaker;

    #[test]
    fn a_closed_breaker_admits_every_request() {
        let cb = CircuitBreaker::new(5, 60);
        for _ in 0..10 {
            assert!(cb.allow_request(), "a closed circuit admits the request");
        }
    }

    #[test]
    fn the_threshold_failures_open_the_circuit() {
        let cb = CircuitBreaker::new(3, 60);
        cb.record_failure();
        cb.record_failure();
        assert!(
            cb.allow_request(),
            "two failures is under a threshold of three"
        );
        cb.record_failure();
        assert!(
            !cb.allow_request(),
            "the third failure opens the circuit and the next request is refused"
        );
    }

    #[test]
    fn a_success_closes_the_circuit_again() {
        let cb = CircuitBreaker::new(1, 60);
        cb.record_failure();
        assert!(!cb.allow_request(), "the circuit is open");
        cb.record_success();
        assert!(cb.allow_request(), "a success closes it");
    }

    /// The breaker's state: 0 closed, 1 open, 2 half-open.
    ///
    /// The test module is inside the breaker's own file, so it can read
    /// the state the transitions write — which is how a transition is
    /// observed without waiting out a cooldown.
    fn state_of(cb: &CircuitBreaker) -> u8 {
        cb.state.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn an_open_circuit_refuses_until_the_cooldown_elapses() {
        let cb = CircuitBreaker::new(1, 60);
        cb.record_failure();
        assert_eq!(state_of(&cb), 1, "the circuit is open after the failure");
        assert!(
            !cb.allow_request(),
            "an open circuit refuses the request while the cooldown runs"
        );
        assert!(!cb.allow_request(), "and keeps refusing it");
    }

    #[test]
    fn the_first_caller_after_the_cooldown_is_the_probe() {
        // A cooldown of zero makes the transition immediate.
        let cb = CircuitBreaker::new(1, 0);
        cb.record_failure();
        assert!(
            cb.allow_request(),
            "the first caller after the cooldown is the one probe"
        );
    }

    #[test]
    fn half_open_admits_exactly_one_request() {
        // The whole point of half-open: ONE probe reaches the platform, so
        // a recovering platform is not hit by the whole backlog at once.
        let cb = CircuitBreaker::new(1, 60);
        cb.record_failure();
        // The breaker is in half-open (the state a probe is admitted from).
        cb.release_probe();
        assert_eq!(state_of(&cb), 2, "the breaker is half-open");

        assert!(
            cb.allow_request(),
            "the first caller in half-open is the one probe"
        );
        // The probe flips the state back to OPEN, so the concurrent caller
        // in the same tick is refused. It used to admit them all.
        assert!(
            !cb.allow_request(),
            "a second caller in the same tick must NOT be admitted"
        );
        assert!(
            !cb.allow_request(),
            "nobody else is admitted in this tick, however many callers there are"
        );
    }

    #[test]
    fn the_probe_failure_re_opens_for_a_full_cooldown() {
        let cb = CircuitBreaker::new(1, 60);
        cb.record_failure();
        assert_eq!(state_of(&cb), 1, "the circuit is open");
        let opened_at = cb.opened_at.load(std::sync::atomic::Ordering::SeqCst);

        // Simulate the probe: it is admitted, and it fails.
        cb.release_probe();
        cb.record_failure();
        assert_eq!(
            state_of(&cb),
            1,
            "the failed probe leaves the circuit OPEN, not half-open"
        );
        assert!(
            cb.opened_at.load(std::sync::atomic::Ordering::SeqCst) >= opened_at,
            "the cooldown starts again when the probe fails"
        );
        assert!(
            !cb.allow_request(),
            "and the next request is refused until the new cooldown elapses"
        );
    }

    #[test]
    fn the_probe_success_closes_the_circuit() {
        let cb = CircuitBreaker::new(1, 0);
        cb.record_failure();
        assert!(cb.allow_request(), "the probe is admitted");
        cb.record_success();
        assert!(cb.allow_request(), "the probe's success closes the circuit");
        assert!(cb.allow_request(), "and it stays closed");
    }

    #[test]
    fn a_released_probe_is_not_a_failure() {
        // A probe whose request never reached the platform (a thread part
        // whose predecessor is unpublished) learned nothing. It must not
        // count as a failure.
        let cb = CircuitBreaker::new(1, 0);
        cb.record_failure();
        assert!(cb.allow_request(), "the probe is admitted");
        cb.release_probe();
        // The state is back to half-open, so the next caller gets the probe
        // rather than being refused.
        assert!(
            cb.allow_request(),
            "releasing the probe leaves the circuit able to probe again"
        );
    }
}
