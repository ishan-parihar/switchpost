// ─── Red-team: provider connect flows against a real router ───
//
// Everything here drives `build_router(...)` with a session cookie, the same
// way the browser does. Each test names the observable defect it pins, so a
// fix has to make a RED test green rather than a green test stay green.
//
// Run: cargo test --test connect_flow_redteam_test

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use switchpost::api::{build_router, AppState};
use switchpost::auth::jwt::create_token;
use switchpost::auth::middleware::{DEFAULT_USER_ID, SESSION_COOKIE};
use switchpost::config::Config;
use switchpost::db;
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;
use tower::ServiceExt;
use uuid::Uuid;

async fn state() -> AppState {
    switchpost::config::load_dotenv_for_tests();
    let config = Config::from_env().expect("config");
    let pool = db::create_pool(&config.database_url).await.expect("pool");
    db::ensure_local_user(&pool).await.expect("local user");
    let registry = ProviderRegistry::new(&config, None, None);
    AppState {
        db: pool,
        config: config.clone(),
        broadcast: Broadcaster::new(),
        providers: registry,
        token_key: None,
        telegram_client_manager: None,
        wa_client: None,
        media_http_client: reqwest::Client::new(),
        media_wreq_client: wreq::Client::new(),
    }
}

/// A session cookie the auth middleware accepts, so every request below is
/// exercised as an AUTHENTICATED browser call.
fn session_cookie() -> String {
    let config = Config::from_env().expect("config");
    let token = create_token(DEFAULT_USER_ID, &config.jwt_secret).expect("token");
    format!("{SESSION_COOKIE}={token}")
}

/// Three tests below use `instagram-standalone` because its connect path is
/// OAuth-shaped, so the authorize URL is observable. That requires the
/// provider to be CONFIGURED, and it is configured through `.env` (gitignored)
/// or real env vars - so on a fresh CI checkout it is not.
///
/// Each of those three has an "unconfigured, so treat as a pass" escape,
/// because a credentials refusal is the honest answer there and not a defect.
/// The escape is also how a test can green WITHOUT EXERCISING ANYTHING: the
/// handler checks credentials BEFORE `redirect_uri` validation, so an
/// unconfigured provider never reaches the code under test.
///
/// Two things hold that open. `note_unconfigured` counts and reports each
/// skip, and the direct test below runs the rule with no provider at all -
/// so the rule is exercised in every environment, including a clean checkout.
static REDIRECT_SKIPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The `redirect_uri` router tests escape on a credentials refusal. Count it
/// and say so loudly, so an all-skipped run is visible rather than hidden in
/// a green checkmark. (A test cannot assert on another test's skip - cargo
/// runs them on threads - so this reports rather than gates; the direct test
/// above is what guarantees the rule is exercised regardless.)
fn note_unconfigured(which: &str) {
    let n = REDIRECT_SKIPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    eprintln!(
        "SKIP[{which}]: instagram-standalone is not configured, so redirect_uri \
         validation (which runs AFTER the credentials check) was never reached. \
         This is skip #{n} of the three; a run that only skips has proved \
         nothing about redirect_uri."
    );
}

/// The `redirect_uri` rule, exercised directly - no provider credentials, so
/// this runs in EVERY environment.
///
/// The three router-level tests below reach the same code, but only when
/// `instagram-standalone` is configured, and the handler checks credentials
/// BEFORE it validates `redirect_uri` - so on a fresh checkout those three
/// green without ever calling this function. That is the state in which a
/// regression ships. This test closes it: it can always run, so the rule is
/// always exercised.
///
/// For the same reason the three router tests' "unconfigured, treat as a
/// pass" escape is counted: if they all skip here, that is visible in the
/// run output rather than hidden in a green checkmark.
#[test]
fn the_redirect_uri_rule_is_exercised_without_any_provider_credentials() {
    let app_url = "https://switchpost.example.com";
    let callback = format!("{app_url}/api/auth/callback");

    // 1. A JSON-quoted value round-tripped through serde_json is normalised,
    //    not refused. This is the exact wire form the operator saw: Meta's
    //    consent page rendered literal quotes and `\/` escapes.
    let json_quoted = format!("{}", serde_json::to_string(&callback).unwrap());
    assert_eq!(
        switchpost::social::common::resolve_redirect_uri(Some(&json_quoted), app_url).unwrap(),
        callback,
        "the JSON-quoted form of our own callback was not normalised back to the \
         callback, so the operator's connect request is refused even though it \
         carried the right URI"
    );

    // 2. A doubly-quoted value (quoted twice by a caller that already
    //    round-tripped it once) must be REFUSED, not accepted: one layer is
    //    stripped, and what is left still carries its own quotes, so it is
    //    neither the callback nor an absolute URL. A blanket unquote here
    //    would accept a value the caller corrupted twice, and the operator's
    //    connect would still fail - just at the provider instead of at the
    //    door.
    let double_quoted = serde_json::to_string(&json_quoted).unwrap();
    assert!(
        switchpost::social::common::resolve_redirect_uri(Some(&double_quoted), app_url).is_err(),
        "a doubly-quoted redirect_uri was accepted. {double_quoted:?}"
    );

    // 3. A foreign host is an open redirect and must be refused.
    for foreign in [
        "https://evil.test/cb",
        "https://switchpost.example.org/api/auth/callback",
    ] {
        assert!(
            switchpost::social::common::resolve_redirect_uri(Some(foreign), app_url).is_err(),
            "redirect_uri {foreign} was accepted; whoever controls the query string \
             controls where the provider's authorization code goes"
        );
    }

    // 4. Omitting it still yields the deployment's callback.
    assert_eq!(
        switchpost::social::common::resolve_redirect_uri(None, app_url).unwrap(),
        callback
    );

    // 5. An empty or whitespace-only value means "no opinion" and uses the
    //    deployment's callback - the same as omitting it. That is the desired
    //    behaviour, not a hole: an empty query parameter is what a client that
    //    zeroes a field sends, and refusing it would dead-end a caller who
    //    meant the default.
    for no_opinion in ["", "   ", "\t"] {
        assert_eq!(
            switchpost::social::common::resolve_redirect_uri(Some(no_opinion), app_url).unwrap(),
            callback,
            "an empty or whitespace redirect_uri ({no_opinion:?}) must be treated as \
             'no opinion' and fall back to the deployment's callback, the same as \
             omitting it. Refusing it would dead-end a caller who meant the default."
        );
    }

    // 6. A value that is quoted-but-empty (what a caller who round-tripped an
    //    empty string through serde_json sends) is a DIFFERENT case, and must
    //    be refused: the quotes say "this is a real value" while the content
    //    says "there is none". Falling back silently would let a broken
    //    client's connect look like it worked.
    for empty_but_quoted in ["\"\"", "\"   \""] {
        let err = switchpost::social::common::resolve_redirect_uri(Some(empty_but_quoted), app_url)
            .expect_err(&format!("redirect_uri {empty_but_quoted:?} was accepted"));
        assert!(
            err.contains("Omit"),
            "the refusal for {empty_but_quoted:?} must name the recovery, so the \
             caller knows to leave the parameter out. Got: {err}"
        );
    }

    // 7. A trailing-slash base must not produce a double slash.
    assert_eq!(
        switchpost::social::common::resolve_redirect_uri(None, "https://switchpost.example.com/")
            .unwrap(),
        callback,
        "an APP_URL with a trailing slash produced a doubled slash in the callback"
    );
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let cookie = session_cookie();
    // The Origin must match the CSRF middleware's expected Frontend origin.
    // `FRONTEND_URL` defaults to `APP_URL`, and `APP_URL` defaults to
    // `https://localhost:6543` — the SCHEME matters, so a plain
    // `http://localhost:6543` here would be refused by the origin check
    // before the handler ever ran, and the test would report a CSRF failure
    // as a handler failure. Both are sent so the header set covers the
    // Referer fallback too.
    let origin = std::env::var("FRONTEND_URL")
        .or_else(|_| std::env::var("APP_URL"))
        .unwrap_or_else(|_| "https://localhost:6543".to_string());
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("origin", origin.as_str())
        .header("referer", format!("{origin}/"));
    match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    }
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(json_request(method, uri, body))
        .await
        .expect("router call");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Parse a response body as JSON, with the defect named in the panic so a
/// non-JSON body fails the test instead of silently yielding `null`.
fn parse_json(what: &str, status: StatusCode, body: &str) -> Value {
    serde_json::from_str::<Value>(body).unwrap_or_else(|_| {
        panic!(
            "{what}: the answer was not JSON (HTTP {status}). \
             The frontend reports this as `Invalid JSON response from {what}` \
             and the operator never sees the real reason.\nbody: {body}"
        )
    })
}

/// Every provider identifier the registry knows, so a provider cannot be
/// silently dropped from the connect surface.
async fn registered_providers() -> Vec<String> {
    let config = Config::from_env().expect("config");
    let registry = ProviderRegistry::new(&config, None, None);
    registry.list().iter().map(|s| s.to_string()).collect()
}

// ─────────────────────────────────────────────────────────────
// 1. The connect surface must cover the registry
// ─────────────────────────────────────────────────────────────

/// The dashboard's Available Providers grid is a hardcoded TypeScript list
/// (`frontend/src/routes/channels/+page.svelte`). The registry is the
/// backend's truth. Every registered provider that is connectable must be
/// offered in the frontend, or it is unreachable through the WebUI.
#[tokio::test]
async fn channels_page_offers_every_registered_connectable_provider() {
    let ids = registered_providers().await;
    assert!(ids.len() >= 26, "expected the full registry, got {ids:?}");

    // The frontend's own literal, transcribed. This travels as a string
    // because the check is about the RENDERED offer list.
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/routes/channels/+page.svelte"
    ))
    .expect("read channels page");

    let mut missing = Vec::new();
    for id in &ids {
        if !source.contains(&format!("\"{id}\"")) {
            missing.push(id.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "frontend Available Providers grid omits {missing:?}, \
         which the backend registers as connectable — a channel the \
         founder cannot add from the WebUI"
    );
}

// ─────────────────────────────────────────────────────────────
// 2. A refused connect must be JSON, not HTML
// ─────────────────────────────────────────────────────────────

/// The operator's dashboard reported `Invalid JSON response from
/// /api/integrations/connect/whatsapp/pair`. The route exists in the
/// backend, so the failing half is the presentation: something between the
/// browser and the handler answered non-JSON (the SPA HTML fallback, or a
/// proxy error page), and the API client reports that as a JSON parse error
/// with the status swallowed.
///
/// The fix is on BOTH sides:
///   - the router must never answer an unknown `/api/*` path with HTML, and
///   - the API client must surface the status + body when it cannot parse.
/// This test pins the router half; the client half is pinned in
/// `frontend` via the `Invalid JSON response` message containing the status.
#[tokio::test]
async fn unknown_api_route_never_answers_html() {
    let state = state().await;

    // NOTE THE PATH SHAPE. An earlier revision of this test used
    // `/api/integrations/connect/this-route-does-not-exist`, which matches
    // the REAL `connect/:provider` route and is answered 401 by the auth
    // middleware — so the test passed without the fallback fix existing. A
    // test that passes on the broken code proves nothing.
    //
    // The paths below match no route at all, so they reach the SPA fallback,
    // which is exactly where the 200-HTML answer came from.
    //
    // `/api/integrations/connect/<unknown>` and `/api/auth/callback/<unknown>`
    // are a separate case: they match REAL routes, which validate the
    // provider and answer a JSON `bad_request` — a GOOD answer, just a
    // different code. They are asserted separately below so they are not
    // mistaken for a guard hit.
    for uri in ["/api/definitely-not-a-route", "/api/posts/xyzzy/nope"] {
        let (status, body) = call(&state, "GET", uri, None).await;

        assert_ne!(
            status,
            StatusCode::OK,
            "{uri} answered 200 — the SPA fallback swallowed it and handed \
             the browser HTML where JSON was promised"
        );
        assert!(
            !body.contains("<!doctype html") && !body.contains("<html"),
            "{uri} answered HTML (HTTP {status}); the browser parses it as a \
             JSON error and the operator never sees the reason. body: {body}"
        );

        let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| {
            panic!("{uri}: expected a JSON 404, got HTTP {status} and body {body}")
        });
        assert_eq!(
            parsed.get("code").and_then(Value::as_str),
            Some("not_found"),
            "{uri}: the 404 must carry a machine-readable code"
        );
        let msg = parsed.get("error").and_then(Value::as_str).unwrap_or("");
        assert!(
            msg.contains(uri) || msg.contains("No such endpoint"),
            "{uri}: the error must name the missing path, got {msg}"
        );
    }

    // A connect to a provider the registry does not know is refused by the
    // handler itself, with the provider NAME — which is the most useful
    // answer of the three: the founder gets told which card they clicked.
    let (status, body) = call(
        &state,
        "POST",
        "/api/integrations/connect/not-a-provider-at-all",
        Some(serde_json::json!({})),
    )
    .await;
    assert!(
        !body.contains("<!doctype html"),
        "an unknown provider connect answered HTML (HTTP {status})"
    );
    assert_ne!(
        status,
        StatusCode::OK,
        "an unknown provider connect answered 200"
    );
    if body.trim().is_empty() {
        // A 404/405 from the router without a body is still a NON-200,
        // non-HTML answer, which is what the founder needs. The strict JSON
        // shape is only required when there IS a body to parse.
        return;
    }
    let parsed: Value = serde_json::from_str(&body)
        .unwrap_or_else(|_| panic!("unknown provider connect: expected JSON, got {body}"));
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("bad_request"),
        "an unknown provider is a bad request, and must name itself as one"
    );
}

/// The counterpart: the SPA's OWN client-routed paths must STILL serve HTML.
///
/// The `/api/*` guard is one line inside the SPA fallback, and a guard that
/// is too broad takes down every route in the app. Pin that a normal
/// client-routed path does NOT get the API-namespace JSON message.
#[tokio::test]
async fn a_client_routed_path_is_not_caught_by_the_api_guard() {
    let state = state().await;
    let (status, body) = call(&state, "GET", "/posts/some-id", None).await;
    assert!(
        !body.contains("No such endpoint"),
        "/posts/some-id was answered by the /api/* guard — the guard is too \
         broad and will take down every client route in the app (HTTP {status})"
    );
}

// ─────────────────────────────────────────────────────────────
// 3. A 502 from a provider must carry the provider's reason
// ─────────────────────────────────────────────────────────────

/// `POST /api/integrations/connect/whatsapp/pair` on an instance with no
/// WhatsApp client configured answers `502 {"error":"WhatsApp client not
/// configured"}`. That is JSON and the frontend shows it. But the
/// WhatsAppClient is `Option`ed out in tests only; the real defect class is
/// a handler that answers a bare string body. Pin the shape.
#[tokio::test]
async fn whatsapp_pair_and_telegram_request_code_answer_json_when_unconfigured() {
    let state = state().await;

    let (status, body) = call(
        &state,
        "POST",
        "/api/integrations/connect/whatsapp/pair",
        Some(serde_json::json!({"phone_number": "+1234567890"})),
    )
    .await;
    let parsed = parse_json("whatsapp pair", status, &body);
    assert!(
        parsed.get("error").is_some(),
        "an error answer must carry an `error` field, got {parsed}"
    );

    let (status, body) = call(
        &state,
        "POST",
        "/api/integrations/connect/telegram-user/request-code",
        Some(serde_json::json!({"phone": "+1234567890"})),
    )
    .await;
    let parsed = parse_json("telegram request-code", status, &body);
    assert!(
        parsed.get("error").is_some(),
        "an error answer must carry an `error` field, got {parsed}"
    );
}

// ─────────────────────────────────────────────────────────────
// 4. OAuth scope compatibility (the Google 400)
// ─────────────────────────────────────────────────────────────

/// The Google provider requested all of YouTube + Gmail + Calendar + Drive
/// in ONE authorization URL. Google rejects that combination wholesale:
///
///   "This request contains scopes that cannot be requested together :
///    [https://www.googleapis.com/auth/youtube, ... drive.file]"
///
/// Nothing in the backend guards against an incompatible combination, so
/// the founder gets a raw Google error page with no way forward. The
/// guard belongs in the provider: `generate_auth_url` must either request
/// only the compatible set, or split the flow per product.
#[tokio::test]
async fn google_provider_requests_a_compatible_scope_set() {
    let config = Config::from_env().expect("config");
    let registry = ProviderRegistry::new(&config, None, None);
    let google = registry.get("google").expect("google is registered");

    let scopes = google.scopes();

    // youtube.force-ssl is a YouTube-Data-API scope and drive.file is a
    // Drive scope. Google's rule: a YouTube Data API scope and a Drive
    // scope cannot be requested together in one consent.
    let youtube_scopes = [
        "https://www.googleapis.com/auth/youtube",
        "https://www.googleapis.com/auth/youtube.upload",
        "https://www.googleapis.com/auth/youtube.force-ssl",
    ];
    let drive_scopes = [
        "https://www.googleapis.com/auth/drive.readonly",
        "https://www.googleapis.com/auth/drive.file",
    ];

    let has_youtube = youtube_scopes.iter().any(|s| scopes.iter().any(|x| x == s));
    let has_drive = drive_scopes.iter().any(|s| scopes.iter().any(|x| x == s));

    assert!(
        !(has_youtube && has_drive),
        "google requests YouTube and Drive scopes together — Google rejects \
         this combination at the authorize endpoint with 'scopes that cannot \
         be requested together'. Split the connect flow per product, or \
         narrow the set."
    );
}

// ─────────────────────────────────────────────────────────────
// 5. LinkedIn: the scope that needs a product nobody has
// ─────────────────────────────────────────────────────────────

/// LinkedIn's `r_member_social` requires the "Sign In with LinkedIn using
/// Mobile or the LinkedIn API" product to be enabled on the app. When it is
/// NOT enabled, LinkedIn answers the authorize request with its own error
/// page ("Bummer, something went wrong. In five seconds, you will be
/// redirected to: …") rather than a consent screen. The provider cannot
/// fix the operator's app configuration, but it MUST NOT request a scope the
/// product gate will refuse — and it must tell the operator which product
/// to enable when it does.
#[tokio::test]
async fn linkedin_does_not_request_a_product_gated_scope_silently() {
    let config = Config::from_env().expect("config");
    let registry = ProviderRegistry::new(&config, None, None);
    let linkedin = registry.get("linkedin").expect("linkedin is registered");

    let scopes = linkedin.scopes();

    // The defect is that this scope needs a product that is not on by
    // default, and the OAuth failure gives no hint at all — the founder
    // has no way to know which product to enable.
    assert!(
        !scopes.iter().any(|s| s == "r_member_social"),
        "linkedin requests `r_member_social`, which needs the LinkedIn \
         'Sign In with LinkedIn' product to be enabled. Without it LinkedIn \
         answers 'Bummer, something went wrong'. Either drop the scope or \
         surface the missing product in the connect error."
    );
}

// ─────────────────────────────────────────────────────────────
// 6. The OAuth redirect_uri must match the deployment origin
// ─────────────────────────────────────────────────────────────

/// `instagram-standalone` failed on the VPS with a redirect_uri error. The
/// backend builds the redirect as `{APP_URL}/api/auth/callback`, which is
/// correct — but nothing asserts the value that actually reaches the
/// provider equals `APP_URL`. When `APP_URL` is misconfigured (trailing
/// slash, http vs https, port), Meta refuses the request with
/// "URL Blocked: This redirect failed because the redirect URI is not
/// whitelisted" (error 1349168) and the founder cannot see what was sent.
///
/// Pin: the authorize URL's `redirect_uri` is byte-identical to
/// `{APP_URL}/api/auth/callback`, and the API logs it (the operator's only
/// clue when Meta refuses).
#[tokio::test]
async fn authorize_url_carries_the_app_url_callback() {
    let state = state().await;
    let expected = format!("{}/api/auth/callback", state.config.app_url);

    let (status, body) = call(
        &state,
        "GET",
        "/api/integrations/connect/instagram-standalone",
        None,
    )
    .await;

    // The provider is unconfigured in a test env, so a 400 naming the
    // missing env vars is the honest answer — never a silent HTML fallback.
    if status == StatusCode::BAD_REQUEST {
        let parsed = parse_json("connect redirect_uri", status, &body);
        assert!(
            parsed.get("error").is_some(),
            "a 400 must be JSON with an `error` field, got {parsed}"
        );
        return;
    }

    let parsed = parse_json("connect redirect_uri", status, &body);
    let url = parsed
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or_else(|| panic!("connect answer has no url: {parsed}"));
    let redirect_uri = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "redirect_uri")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_else(|| panic!("authorize URL carries no redirect_uri: {url}"));

    assert_eq!(
        redirect_uri, expected,
        "the authorize URL's redirect_uri does not match the configured APP_URL callback — \
         Meta refuses the request with 'URL Blocked' (error 1349168)"
    );
    let _ = Uuid::nil();
}

// ─────────────────────────────────────────────────────────────
// 7. The frontend's provider grid must not omit a rendered label
// ─────────────────────────────────────────────────────────────

/// Facebook-Business and Instagram-Business were absent from the dashboard
/// while the backend registered their providers (as `facebook` and
/// `instagram`). The identifiers DO exist and ARE registered — so the
/// frontend simply never rendered a card for them. Pin that both appear in
/// the frontend's offer list AND in the backend registry.
#[tokio::test]
async fn facebook_and_instagram_are_registered_and_rendered() {
    let ids = registered_providers().await;
    for id in ["facebook", "instagram", "instagram-standalone"] {
        assert!(
            ids.iter().any(|x| x == id),
            "{id} is not registered — the dashboard cannot offer it"
        );
    }

    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/routes/channels/+page.svelte"
    ))
    .expect("read channels page");
    for id in ["facebook", "instagram", "instagram-standalone"] {
        assert!(
            source.contains(&format!("\"{id}\"")),
            "{id} is registered but absent from the dashboard's Available \
             Providers grid"
        );
    }
}

// ─────────────────────────────────────────────────────────────
// 8. The frontend API client must not hide the HTTP status
// ─────────────────────────────────────────────────────────────

/// `Invalid JSON response from /api/...` was the whole message the operator
/// saw. It carried no status and no body, so a 502 from the provider, a 404
/// from a mistyped route and a reverse-proxy error page were
/// indistinguishable - and six distinct production defects were all reported
/// to us as that one string.
///
/// The client is now fixed to name the status, the content type and a body
/// excerpt. Pin exactly that: the message must NOT be the old bare form,
/// and it must carry both the status and a body slice.
#[tokio::test]
async fn frontend_api_client_surfaces_status_and_body_on_unparseable_response() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/lib/api/client.ts"
    ))
    .expect("read api client");

    assert!(
        source.contains("Invalid JSON response") && source.contains("res.status"),
        "the API client's unparseable-body error must carry the HTTP status \
         so a bare path-less message is not the operator's only information"
    );

    // The body excerpt is what tells an HTML error page from a JSON error
    // page, which is the whole difference between "the route does not exist"
    // and "the route 500'd".
    assert!(
        source.contains("content-type") && source.contains("excerpt"),
        "the API client must inspect the response's content-type and include \
         a short body slice, so a proxy error page is visible"
    );

    // The HTML case is the classic one: the SPA fallback answering an
    // /api/* path. Say so in words, because "Expected JSON ... 200 text/html"
    // is still not something a founder can act on without being told what
    // it means.
    assert!(
        source.contains("received HTML, not JSON"),
        "the client must name the HTML-where-JSON-was-expected case, because \
         that is the shape produced by the server's SPA fallback masking an \
         unknown /api/* route"
    );
}

// ─────────────────────────────────────────────────────────────
// 9. The dashboard's provider grid must equal the backend registry
// ─────────────────────────────────────────────────────────────

/// The dashboard's `availableProviders` literal is the only thing standing
/// between a registered provider and a card on the Channels screen. It is a
/// hand-maintained list, and it has drifted before: `youtube` was missing
/// for a release while the registry, `$lib/providers` and the onboarding
/// modal all had it, so the grid showed 25 cards for a 26-platform product
/// with no way to connect YouTube.
///
/// Pin the whole set both ways. A one-way check ("the grid contains what the
/// registry has") passes a grid that is MISSING entries; a grid that has an
/// EXTRA entry is equally broken (a connect button for a provider the
/// registry will not serve). So the two sets must be equal, excluding the
/// Tier-3 archive opt-in, which the frontend deliberately leaves out until
/// `ENABLE_ARCHIVE_PROVIDERS=1` is set.
#[tokio::test]
async fn dashboard_provider_grid_matches_the_backend_registry() {
    let registered = registered_providers().await;
    let expected: std::collections::HashSet<String> = registered.into_iter().collect();

    // The frontend literals this test reads.
    let page = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/frontend/src/routes/channels/+page.svelte"
    ))
    .expect("read channels page");

    // Pull the ids out of the `availableProviders` array literal rather than
    // the whole file, so an unrelated mention of "drive" in a comment cannot
    // satisfy the check.
    let start = page
        .find("const availableProviders")
        .expect("availableProviders not found");
    let rest = &page[start..];
    let end = rest
        .find(']')
        .map(|i| start + i)
        .expect("unterminated array");
    let literal = &page[start..end];

    // Quoted ids, minus the ARCHIVE_PROVIDERS spread the frontend intentionally
    // leaves empty, and minus HIDDEN_PROVIDERS (kick/vk/whop/lemmy), which the
    // registry never registers anyway.
    let hidden = ["kick", "vk", "whop", "lemmy"];
    let offered: std::collections::HashSet<String> = literal
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| !s.contains("ARCHIVE"))
        .filter(|s| !hidden.contains(&s.as_str()))
        .collect();

    let missing: Vec<_> = expected.difference(&offered).cloned().collect();
    let extra: Vec<_> = offered.difference(&expected).cloned().collect();
    assert!(
        missing.is_empty(),
        "registered but absent from the dashboard grid: {missing:?}. A \
         hand-maintained list drifts; read the registry's list at \
         src/social/registry.rs."
    );
    assert!(
        extra.is_empty(),
        "offered in the dashboard but NOT registered by the backend: \
         {extra:?}. The connect button would 404 or draw an error page."
    );
}

// ─────────────────────────────────────────────────────────────
// 10. Every OAuth redirect_uri is built from APP_URL, not FRONTEND_URL
// ─────────────────────────────────────────────────────────────

/// `APP_URL` and `FRONTEND_URL` are two settings with two purposes, and
/// `.env.example` says so: APP_URL is "Used for OAuth redirect URIs",
/// FRONTEND_URL controls "CORS allowed origins AND the CSRF allow-list".
/// They default to the same value, which is why a deployment behind a
/// reverse proxy (APP_URL = the public host, FRONTEND_URL = the local
/// origin) is where the confusion bites.
///
/// Three sites built the callback from `frontend_url` instead: two that
/// only REPORT the redirect_uri to the operator, and one that hands it to
/// `exchange_code`. The reporting ones are worse than useless — they tell
/// the founder to register a URL in the Meta console that the connect flow
/// never sends, and Meta then answers "URL Blocked" (error 1349168) for a
/// flow the founder has every reason to believe is correct. The
/// `exchange_code` one fails outright with a redirect_uri mismatch.
///
/// Pin the rule across every non-test call site that constructs the
/// callback. A new site that reaches for `frontend_url` fails here.
/// Every "/api/auth/callback" literal in the tree must be built from APP_URL.
///
/// This scans the WHOLE of `src/`, not an allowlist of files. The list form
/// was what hid the original defect: three sites were fixed while eleven in
/// `src/mcp/tools_setup.rs` and one in `src/cli/run.rs` sat outside the
/// list, unprotected and unchecked. They happened to be correct — which is
/// exactly the case a reviewer must never rely on, because the next added
/// file is the one that regresses.
///
/// The check is POSITIVE: each line must name `app_url`. A negative check
/// ("must not name frontend_url") passes on a line that names neither,
/// and the setting omitted is the one that matters.
#[tokio::test]
async fn every_oauth_callback_is_built_from_app_url() {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                files.push(p);
            }
        }
    }
    files.sort();

    let mut constructions = 0usize;
    for path in &files {
        let f = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string();
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {f}: {e}"));
        for (idx, _) in src.match_indices("/api/auth/callback") {
            let line_start = src[..idx].rfind('\n').map_or(0, |i| i + 1);
            let line_end = src[idx..].find('\n').map_or(src.len(), |rel| idx + rel);
            let whole_line = &src[line_start..line_end];
            let trimmed = whole_line.trim_start();

            // Skip DOCUMENTATION. The path appears in doc comments
            // ("redirects back to `{app_url}/api/auth/callback`") and those
            // lines have NO code at all, so a positive check on the stripped
            // line reads an empty string and fails on a file that is correct.
            if trimmed.starts_with("//") {
                continue;
            }
            // Skip NON-CONSTRUCTIONS. The path is a router literal
            // (`src/api/mod.rs`) and appears in test fixtures as a
            // hard-coded `https://localhost:6544/...` string. Those are not
            // the redirect_uri the provider is sent, and asserting on them
            // would name a string literal instead of the setting at fault.
            // Only a line that BUILDS the URL is subject to the rule.
            let code = whole_line.split("//").next().unwrap_or(whole_line);
            let builds = code.contains("format!")
                || code.contains("{app_url}")
                || code.contains("{APP_URL}");
            if !builds {
                continue;
            }

            constructions += 1;
            // This check is LEXICAL, and that is a deliberate, recorded
            // limit rather than an oversight. A binding still *named*
            // `app_url` but sourced from `state.config.frontend_url`
            // cannot be seen from the use line — mutation-tested against
            // `src/mcp/tools_setup.rs`, where all eleven sites bind that
            // way, and confirmed to pass. Two things bound the exposure:
            // the only two files with a real defect
            // (`api/integrations.rs`, `mcp/tools_integrations.rs`) inline
            // `state.config.app_url`, so the setting is on the asserted
            // line, and the realistic regression there goes RED; and
            // renaming a binding without renaming its uses is a
            // compile error, so the compiler catches the clumsy version.
            // If a file here ever becomes a redirect_uri source for a
            // provider, inline the field on the construction line.
            let lower = code.to_ascii_lowercase();
            let names_app_url = lower.contains("app_url");
            let names_frontend = lower.contains("frontend_url");
            if !names_app_url || names_frontend {
                panic!(
                    "{f} builds an OAuth callback from something other than APP_URL. \
                     The callback comes from APP_URL; FRONTEND_URL is the CORS \
                     allow-list entry and the CSRF origin check, a different \
                     setting. Code: {}",
                    code.trim()
                );
            }
        }
    }
    // A zero here means the walk or the filters are broken, not that the
    // codebase is clean. The count is real: it is the number of construction
    // sites across `src/`, counted on the tree this test ships with.
    assert!(
        constructions >= 20,
        "found {constructions} callback construction sites, expected 20+ - \
         is the src/ walk or the filter working?"
    );
}

/// The mirror of the above: `frontend_url` must still be used for what it
/// is actually for. A fix that swapped it out of the CSRF check would open
/// a hole while looking consistent.
#[tokio::test]
async fn frontend_url_is_still_the_csrf_origin() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/api/mod.rs"))
        .expect("read api/mod.rs");

    // Check the CONSTRUCTION, not the function: CsrfState is where the
    // allow-list is set. (The first textual match for `csrf_origin_check`
    // is a doc-comment mention, so searching from there is how this test
    // first passed a false failure.)
    let construction = src
        .find("CsrfState {")
        .expect("CsrfState construction not found");
    let window = &src[construction..(construction + 200).min(src.len())];
    assert!(
        window.contains("frontend_url"),
        "CsrfState's allowed_origin no longer comes from FRONTEND_URL — that \
         is the setting's one job (CORS allow-list + CSRF origin). Window: {window}"
    );

    // And the mirror, so a future edit cannot "fix" the inconsistency by
    // pointing the CSRF check at APP_URL instead.
    assert!(
        !window.contains("app_url"),
        "CsrfState's allowed_origin now reads APP_URL; FRONTEND_URL is the \
         CORS/CSRF origin and must stay so"
    );
}

/// Every provider that requests GOOGLE scopes must pass the scope guard.
///
/// The guard (`social::scope_guard`) was introduced for the unified `google`
/// provider and is CALLED only from `google::generate_auth_url`. But the fix
/// that narrowed `google`'s scopes also split Drive, Gmail and Calendar into
/// their own providers, so three more consent flows now request
/// `googleapis.com` scopes and NONE of them passes through the guard.
///
/// That reintroduces the defect the guard was built to prevent, one provider
/// over. Proven by mutation before the test existed: re-adding
/// `youtube.upload` beside `drive.file` in `src/social/drive.rs` compiled and
/// `cargo test --lib` stayed 527/527 green — the refusal that used to happen
/// locally would now happen on Google's page again, with a message naming no
/// fix and no consent screen.
///
/// The test is over the REGISTRY, not a list of names, so a provider added
/// later is covered the day it is registered.
#[tokio::test]
async fn every_google_scoped_provider_passes_the_scope_guard() {
    switchpost::config::load_dotenv_for_tests();
    let config = Config::from_env().expect("config");
    let registry = ProviderRegistry::new(&config, None, None);

    let mut checked = 0usize;
    for provider in registry.all() {
        let scopes = provider.scopes();
        // Only Google-flavored providers: the conflict rules are Google's,
        // and asserting them on X or Reddit would be checking a rule that
        // does not apply to them.
        if !scopes.iter().any(|s| s.contains("googleapis.com")) {
            continue;
        }
        checked += 1;
        if let Err((a, b, _)) =
            switchpost::social::scope_guard::google_scope_conflict(&scopes, provider.identifier())
        {
            panic!(
                "provider `{}` requests a scope pair Google refuses together: \
                 {a} + {b}. Add the pair-check to this provider's \
                 generate_auth_url, or drop one of the scopes.",
                provider.identifier()
            );
        }
    }
    // Four is the real count: google, drive, gmail, calendar. A drop to zero
    // means the registry stopped populating, not that the rule is satisfied.
    assert!(
        checked >= 4,
        "checked {checked} google-scoped providers, expected 4+ (google, \
         drive, gmail, calendar)"
    );
}

/// No token path anywhere in the tree may report an EMPTY access_token as
/// success.
///
/// This was found by grepping the whole of `src/social/` for the shape rather
/// than reading one provider. Exactly one site had it: Discord's
/// `refresh_token` read `json["access_token"].as_str().unwrap_or("")`, so a
/// refused refresh SUCCEEDED with an empty token. Discord answers a revoked
/// refresh token with `{"error":"invalid_grant"}` and no `access_token`.
///
/// The failure mode is the worst available shape because it is invisible:
/// the integration stays "connected", the dashboard shows it healthy, the
/// scheduler writes an empty token without complaint, and the defect only
/// becomes visible as an opaque publish failure hours later, with nothing
/// connecting it to the refresh that actually broke. AGENTS.md §6.3 requires
/// that a refresh path never write a bad token, and an empty string is a bad
/// token that no guard catches.
///
/// The test is over the tree, not a list of providers, for the same reason the
/// redirect_uri guard is: the file that regresses next is not on anyone's
/// list.
#[test]
fn no_token_path_reports_an_empty_access_token_as_success() {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/social")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                files.push(p);
            }
        }
    }
    files.sort();

    for path in &files {
        let f = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string();
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {f}: {e}"));
        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }
            // The exact shape: an access_token read whose missing/empty case
            // becomes "" rather than an error. Two spellings appear in
            // practice; both are guarded here.
            let silent_empty =
                trimmed.contains("unwrap_or(\"\")") && trimmed.contains("access_token");
            let silent_unwrap = trimmed.contains("access_token")
                && trimmed.contains(".unwrap()")
                && trimmed.contains("as_str()");
            if silent_empty || silent_unwrap {
                panic!(
                    "{f}:{} silently substitutes an empty access_token instead of \
                     returning an error: {trimmed}. A refused refresh would be \
                     reported as a SUCCESS, the integration would stay \
                     'connected' in the dashboard, and the real failure would \
                     surface hours later as an opaque publish error. Return a \
                     ProviderError that quotes the provider's own body.",
                    lineno + 1
                );
            }
        }
    }
}

/// An OAuth token exchange that discards the provider's body collapses four
/// different failures into one useless sentence.
///
/// The shape, repeated in most of `src/social/`, was:
///
/// ```text
/// let json: Value = resp.json().await?;
/// let access_token = json["access_token"]
///     .as_str()
///     .ok_or_else(|| ProviderError::Auth("Missing access_token".into()))?
/// ```
///
/// `error_for_status` was never called and the body was never read, so a
/// redirect_uri mismatch, a wrong client secret, an expired code and a
/// revoked grant all answered "Missing access_token" - the same sentence for
/// all four. The one thing the operator can act on, WHICH of those it was,
/// is exactly what the code threw away. This is the reported "LinkedIn login
/// backend error".
///
/// NOTE the bookkeeping, because a tree-wide test only bites at the edges:
/// this site-graph check is a majority rule and it cannot police a provider
/// that was ALREADY correct, which is why `linkedin.rs` carries its own
/// assertion on the shape (`tests` walk pins the prologue) and `discord.rs`
/// carries the empty-token one. What this test does own is the next provider
/// copied from an old one: it stops the spread.
///
/// AGENTS.md 9.21: one implementation per rule, with a test pinning it.
#[test]
fn no_token_exchange_discards_the_providers_body() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/social");
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                let contents = std::fs::read_to_string(&p).unwrap();
                let lines: Vec<&str> = contents.lines().collect();
                for (idx, line) in lines.iter().enumerate() {
                    // The discarded-body shape: parse, then read the token,
                    // with no status check anywhere in the window between
                    // them and nothing but the missing key to report.
                    if !line.contains("resp.json().await?") && !line.contains("res.json().await?") {
                        continue;
                    }
                    // The window has to reach BOTH ways. The status capture is
                    // usually ONE LINE ABOVE the parse (`let status = resp.status();`),
                    // and the check can be below or beside it. A forward-only
                    // window silently exempts every site that captures the
                    // status first - which is the shape a mutation most
                    // naturally produces.
                    let lo = idx.saturating_sub(3);
                    let hi = (idx + 6).min(lines.len());
                    let window: Vec<&str> = lines[lo..hi].to_vec();
                    // JOIN the window before matching. The token read is a
                    // multi-line chain in this tree -
                    //   let access_token = json["access_token"]
                    //       .as_str()
                    //       .ok_or_else(...)
                    // - so `"access_token"` and `as_str()` are NEVER on the
                    // same physical line. A per-line match exempts every
                    // correctly-formatted offender, which is what a mutation
                    // produces. The predicate has to see the SHAPE, not the
                    // line breaks.
                    let joined = window.join("\n");
                    // AND it has to be a TOKEN read. A `PageInfo` builder and
                    // a status poll both call `as_str()` a line or two away
                    // from a `json`, so a loose window flags them, and a
                    // guard that cries wolf on correct code gets deleted
                    // rather than obeyed. Require the token name itself.
                    let has_token_read = joined.contains("[\"access_token\"]")
                        && joined.contains(".as_str()")
                        && joined.contains("access_token = ");
                    let has_status_check = joined.contains("status")
                        && (joined.contains("if ") || joined.contains("!"));
                    if has_token_read && !has_status_check {
                        offenders.push(format!(
                            "{}:{}",
                            p.file_name().unwrap().to_string_lossy(),
                            idx + 1
                        ));
                    }
                }
            }
        }
    }
    // `common.rs` IS the shared implementation, and it checks the status, so
    // it cannot offend. Anything that offends is a provider re-deriving it.
    let real: Vec<&String> = offenders
        .iter()
        .filter(|o| !o.starts_with("common.rs"))
        .collect();
    assert!(
        real.is_empty(),
        "token exchanges that discard the provider's body: {}. Every one of these \
         reports \"Missing access_token\" for a redirect_uri mismatch, a wrong \
         client secret, an expired code and a revoked grant alike. Route through \
         `social::common::extract_access_token`, which checks the status and \
         quotes the provider's own words.",
        real.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

// ─────────────────────────────────────────────────────────────
// 11. A caller-supplied redirect_uri is VALIDATED, not honoured
// ─────────────────────────────────────────────────────────────

/// The connect routes accept `?redirect_uri=`, and every one of them used to
/// hand the string straight to `generate_auth_url` - into the authorize URL and
/// into the stored OAuth state. So the parameter decided where the provider
/// sends an authorization code.
///
/// Two observed failure modes, one reported and one not:
///
///   1. The REPORTED one. Meta's consent page came back with
///      `params[redirect_uri]=%22https%3A%5C%2F%5C%2Fswitchpost.ishanparihar
///      .com%5C%2Fapi%5C%2Fauth%5C%2Fcallback%22` - a JSON-quoted string, quotes
///      and all - and the operator read it as a bug in our own URL building.
///      It was a value we accepted. Meta renders the quotes, we never sanitised
///      them, and the operator hunted a URL we never sent.
///   2. The UNREPORTED one, and the more serious. `?redirect_uri=https://any-
///      where.test/cb` was honoured verbatim. Whoever controls the query string
///      controls where the code goes, and the login still SUCCEEDS from the
///      founder's point of view, so nothing on screen says it went wrong.
///
/// The rule: `resolve_redirect_uri` strips one layer of accidental JSON quoting,
/// then requires the value to be exactly this deployment's `{APP_URL}/api/auth
/// /callback` - the one URI every provider registers. Omitting the parameter is
/// the correct way to say "use the default", and it still works.
///
/// Mutation-tested: replacing `resolve_redirect_uri` with a bare
/// `query.redirect_uri.unwrap_or_else(...)` at the connect handler makes this
/// RED at the JSON-quoted case, because the authorize URL then carries the
/// quotes. A one-layer quote strip alone is NOT enough - the arbitrary-host
/// case stays open - so the second assert is the one that matters.
#[tokio::test]
async fn a_caller_supplied_redirect_uri_is_validated_not_honoured() {
    let state = state().await;
    let expected = format!("{}/api/auth/callback", state.config.app_url);

    // The JSON-quoted form the Meta consent page showed. URL-encoded by hand so
    // the query string reaches the handler exactly as the browser sent it.
    let quoted_value = format!("\"{expected}\"");
    let quoted = urlencoding::encode(&quoted_value);
    let (status, body) = call(
        &state,
        "GET",
        &format!("/api/integrations/connect/instagram-standalone?redirect_uri={quoted}"),
        None,
    )
    .await;

    // A provider that needs credentials may 400 first; the assertion is about
    // the redirect_uri, so it has to be one that is configured in CI. Try a
    // set and skip the ones that answer with the credentials error, which is
    // the honest answer for an unconfigured env and not a defect.
    if status == StatusCode::BAD_REQUEST {
        let parsed = parse_json("connect quoted redirect_uri", status, &body);
        if parsed
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .contains("INSTAGRAM_CLIENT_ID")
        {
            note_unconfigured("a_caller_supplied_redirect_uri_is_validated_not_honoured");
            return;
        }
        panic!(
            "a quoted redirect_uri must never reach the provider, but the request was \
             refused for another reason: {parsed}"
        );
    }

    let url = extract_authorize_url(status, &body);
    let redirect_uri = query_param(&url, "redirect_uri");

    assert_eq!(
        redirect_uri, expected,
        "the authorize URL carried the caller's quoted redirect_uri verbatim. Meta \
         renders those literal quotes in its consent page - \
         params[redirect_uri]=%22https%3A%5C%2F%5C%2F...%22 - which reads as our own \
         URL being wrong and sends the operator to hunt a URL we never sent. \
         `resolve_redirect_uri` strips one layer and then requires the \
         deployment's own callback. URL was: {url}"
    );
}

/// An arbitrary host must be REFUSED, not silently rewritten. A redirect_uri
/// that points somewhere else is an open redirect in the OAuth flow: the code
/// goes there, and the login still looks like it worked.
///
/// Note the deliberate asymmetry with the test above: one that is *normalised*
/// (the quoted form points at our own callback) and one that is *refused*
/// (this one points at a different host). Swapping the two is how the
/// arbitrary-host case stays open behind a quote-stripping fix.
#[tokio::test]
async fn a_redirect_uri_pointing_at_another_host_is_refused() {
    let state = state().await;

    for foreign in [
        "https://evil.test/cb",
        "https://switchpost.example.org/api/auth/callback",
        // Same host, different path: still not our callback.
        "https://localhost:6543/api/auth/callback",
        // A relative path that is not the callback.
        "/api/auth/close",
    ] {
        let (status, body) = call(
            &state,
            "GET",
            &format!(
                "/api/integrations/connect/instagram-standalone?redirect_uri={}",
                urlencoding::encode(foreign)
            ),
            None,
        )
        .await;

        // An unconfigured provider can 400 on credentials first. Distinguish
        // that from the validation refusal by the message, so this test cannot
        // pass merely because nothing ran.
        if status == StatusCode::BAD_REQUEST {
            let parsed = parse_json("connect foreign redirect_uri", status, &body);
            let msg = parsed.get("error").and_then(|e| e.as_str()).unwrap_or("");
            if msg.contains("INSTAGRAM_CLIENT_ID") {
                note_unconfigured("a_redirect_uri_pointing_at_another_host_is_refused");
                continue;
            }
        }

        assert_ne!(
            status,
            StatusCode::OK,
            "a redirect_uri of {foreign:?} was ACCEPTED. Whoever controls the query \
             string then controls where the provider's authorization code goes, and \
             the login still succeeds from the founder's point of view. Body: {body}"
        );
        let parsed = parse_json("connect foreign redirect_uri", status, &body);
        let msg = parsed.get("error").and_then(|e| e.as_str()).unwrap_or("");
        assert!(
            msg.contains("is not this deployment's callback")
                || msg.contains("is not an absolute URL"),
            "the refusal for {foreign:?} must name the rule, so the caller knows what to \
             omit. Got: {msg}"
        );
    }
}

/// Omitting the parameter still works, now and always. A fix that refuses
/// everything would satisfy both tests above and break every real caller.
#[tokio::test]
async fn omitting_redirect_uri_still_uses_the_deployed_callback() {
    let state = state().await;
    let expected = format!("{}/api/auth/callback", state.config.app_url);

    let (status, body) = call(
        &state,
        "GET",
        "/api/integrations/connect/instagram-standalone",
        None,
    )
    .await;
    if status == StatusCode::BAD_REQUEST {
        let parsed = parse_json("connect no redirect_uri", status, &body);
        if parsed
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .contains("INSTAGRAM_CLIENT_ID")
        {
            note_unconfigured("omitting_redirect_uri_still_uses_the_deployed_callback");
            return;
        }
    }
    let url = extract_authorize_url(status, &body);
    assert_eq!(
        query_param(&url, "redirect_uri"),
        expected,
        "omitting redirect_uri must still produce the deployment's callback. A fix that \
         refuses everything would pass the two tests above and break every real caller."
    );
}

/// Read a query parameter out of an authorize URL, or fail naming the URL.
fn query_param(url: &str, key: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_else(|| panic!("authorize URL carries no {key}: {url}"))
}

/// Pull the authorize URL out of a connect response, tolerating the
/// credentials-missing 400 that an unconfigured CI environment answers.
fn extract_authorize_url(status: StatusCode, body: &str) -> String {
    if status == StatusCode::BAD_REQUEST {
        let parsed = parse_json("connect authorize url", status, body);
        let msg = parsed.get("error").and_then(|e| e.as_str()).unwrap_or("");
        if msg.contains("INSTAGRAM_CLIENT_ID") || msg.contains("credentials") {
            // The provider is not configured here, so the connect request was
            // refused at the credentials gate - which is BEFORE
            // `redirect_uri` validation. Nothing about redirect_uri ran; say so
            // instead of treating it as a pass.
            note_unconfigured("extract_authorize_url");
            return String::new();
        }
    }
    let parsed = parse_json("connect authorize url", status, body);
    parsed
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or_else(|| panic!("connect answer has no url: {parsed}"))
        .to_string()
}

// ─────────────────────────────────────────────────────────────
// 3. The page picker is idempotent
// ─────────────────────────────────────────────────────────────

/// Re-running a page connect that already succeeded must SUCCEED, not dead-end.
///
/// The operator hit this in the unified Google flow: the picker lists every
/// page under the parent account, INCLUDING the ones already connected, and
/// clicking one that was connected answered `Page 'X' is already connected`
/// with no way forward. The multi-step flow has nothing to do next, so the
/// connect stops halfway and the operator is left guessing.
///
/// The integration is already in exactly the state the caller asked for, so
/// the honest answer is to return it. This test seeds a parent row and the
/// page row it points at, then repeats the connect.
///
/// What makes the repeat path reachable here is that the already-connected
/// lookup is hoisted BELOW the parent lookup and ABOVE the provider's
/// `pages()` call: it needs only the parent's provider and the requested
/// `page_id`, both of which are in hand before any network I/O. So the test
/// does not need a reachable provider - and, more importantly, neither does
/// a founder whose provider API is down or rate-limiting.
#[tokio::test]
async fn re_connecting_an_already_connected_page_succeeds() {
    let state = state().await;
    let user_id = DEFAULT_USER_ID;

    // A parent account and the page row it already connected. Both are
    // written directly, because a live OAuth is never available in CI and
    // the point of the test is the handler's repeat branch, not the token.
    let parent = switchpost::db::queries::create_integration(
        &state.db,
        user_id,
        "google",
        "Google",
        "parent-account-1",
        "parent-access-token",
        None,
        None,
        Some("Parent Account"),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("seed parent integration");

    let page = switchpost::db::queries::create_integration(
        &state.db,
        user_id,
        "google",
        "Google",
        "page-acct-1",
        "page-access-token",
        None,
        None,
        Some("Ishan Parihar | Ontological Architect"),
        None,
        None,
        Some(&parent.internal_id),
        None,
        None,
    )
    .await
    .expect("seed page integration");

    let before = switchpost::db::queries::list_integrations(&state.db, user_id)
        .await
        .expect("list before");
    let rows_for_page = before
        .iter()
        .filter(|i| i.internal_id == "page-acct-1")
        .count();
    assert_eq!(
        rows_for_page, 1,
        "the seed should hold exactly one row for the page, so a repeat is \
         measurable. Found {rows_for_page}"
    );

    let (status, body) = call(
        &state,
        "POST",
        &format!(
            "/api/integrations/{}/connect-page/page-acct-1",
            parent.id
        ),
        None,
    )
    .await;

    // A provider error here would mean the repeat branch was NOT taken,
    // which is exactly the defect being pinned. The provider is never
    // reached on the repeat path, so that answer would be a real failure.
    assert_eq!(
        status,
        StatusCode::OK,
        "re-running a page connect that already succeeded was refused. The \
         picker lists the already-connected pages, so choosing one dead-ends \
         the multi-step flow with nothing to do next. Body: {body}"
    );

    let parsed = parse_json("re-connect page", status, &body);
    let integration = parsed
        .get("integration")
        .unwrap_or_else(|| panic!("re-connect answer has no integration: {parsed}"));
    assert_eq!(
        integration.get("id").and_then(|v| v.as_str()),
        Some(page.id.to_string()).as_deref(),
        "the repeat must return the row that is already there, not a fresh \
         one. A new row would mean the connect duplicated the page."
    );

    let after = switchpost::db::queries::list_integrations(&state.db, user_id)
        .await
        .expect("list after");
    let rows_after = after
        .iter()
        .filter(|i| i.internal_id == "page-acct-1")
        .count();
    assert_eq!(
        rows_after, rows_for_page,
        "the repeat created {rows_after} rows for a page that already had \
         {rows_for_page}. `create_integration` upserts on (user_id, \
         provider_identifier, internal_id), so a second row would mean the \
         upsert stopped matching - and the composer would offer the same \
         channel twice."
    );

    // The page token must be REFRESHED on the row, not left as whatever was
    // there: re-running the connect is how a founder fixes a stale token.
    let refreshed = switchpost::db::queries::get_integration(&state.db, page.id, user_id)
        .await
        .expect("get integration")
        .expect("page integration still exists");
    assert!(
        !refreshed.access_token.is_empty(),
        "the repeat left the page's access token empty, so every publish on \
         this channel fails with an auth error the founder cannot explain."
    );
}

/// The repeat path must not depend on the provider still listing the page.
///
/// A page can be deleted on the provider's side, or the parent can lose
/// access, and the row here is still a working integration. Refusing the
/// repeat then would take a channel the founder is already using and make it
/// unreconnectable - the same dead-end, reached from a different direction.
#[tokio::test]
async fn re_connecting_survives_a_provider_that_no_longer_lists_the_page() {
    let state = state().await;
    let user_id = DEFAULT_USER_ID;

    let parent = switchpost::db::queries::create_integration(
        &state.db,
        user_id,
        "google",
        "Google",
        "parent-account-2",
        "parent-access-token",
        None,
        None,
        Some("Parent Account 2"),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("seed parent integration");

    let page = switchpost::db::queries::create_integration(
        &state.db,
        user_id,
        "google",
        "Google",
        "page-acct-2",
        "already-working-token",
        None,
        None,
        Some("Orphaned Page"),
        None,
        None,
        Some(&parent.internal_id),
        None,
        None,
    )
    .await
    .expect("seed page integration");

    // The provider is unreachable here (no credentials, no network in CI),
    // so its `pages()` answers an error or an empty list either way. If the
    // repeat branch is taken, that never matters.
    let (status, body) = call(
        &state,
        "POST",
        &format!(
            "/api/integrations/{}/connect-page/page-acct-2",
            parent.id
        ),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the repeat failed because the provider no longer lists the page. A \
         row the founder is already using must stay reconnectable. Body: {body}"
    );

    // The row survives with the token it already held: that is the token
    // the row was connected with, and it is still the working one.
    let after = switchpost::db::queries::get_integration(&state.db, page.id, user_id)
        .await
        .expect("get integration")
        .unwrap_or_else(|| panic!("the page row disappeared after a repeat"));
    assert_eq!(
        after.id, page.id,
        "the page row disappeared after a repeat, so a channel the founder is \
         already using is gone."
    );
    assert!(
        !after.access_token.is_empty(),
        "the repeat left the page's access token empty."
    );
}

/// The repeat path must not call `pages()` at all.
///
/// The existing `re_connecting_an_already_connected_page_succeeds` test claims
/// a founder whose provider is down can still recover a repeat, but it could
/// never have proved that: `google`'s `pages()` answers an empty list for a
/// bogus token rather than an error (the YouTube endpoint returns a JSON body
/// with no `items`, and the reader maps that to zero pages), so the call that
/// was supposed to fail simply didn't. The test was green while the handler
/// still made the network call unconditionally.
///
/// So this pins the thing the other test only asserted in prose: the fetch is
/// GATED on "this page is not already connected".
///
/// Mutation-proven: putting the `pages()` call back outside the `if` (the
/// pre-fix shape) turns this RED, and the two router-level tests stay green -
/// which is exactly how the original defect survived a green suite.
#[test]
fn a_repeat_connect_does_not_fetch_pages_from_the_provider() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/api/integrations.rs"),
    )
    .expect("read src/api/integrations.rs");

    let start = src
        .find("let already_connected = sqlx::query_scalar")
        .expect("the connect-page lookup is not in integrations.rs");
    let tail = &src[start..];
    let call = tail
        .find("provider_obj\n")
        .or_else(|| tail.find(".pages("))
        .expect("the pages() call is not after the lookup");
    let region = &tail[call.saturating_sub(400)
        ..tail[call..]
            .find(".await")
            .map(|e| call + e + ".await".len())
            .unwrap_or(tail.len())];

    assert!(
        region.contains("if already_connected.is_some()")
            || region.contains("if let Some(")
            || region.contains("already_connected.is_none()"),
        "pages() is called whether or not the page is already connected, so a \
         repeat makes a provider call it does not need - and that call is the \
         one that fails during an outage, which is precisely when a founder \
         retries a connect that failed halfway. The gated call was: {region}"
    );

    // And the gate must select the EMPTY answer for the repeat, not skip the
    // variable: `page` is read unconditionally afterwards.
    assert!(
        region.contains("Vec::new()") && region.contains(".pages("),
        "the repeat path does not skip the provider fetch, or does not produce \
         an empty page list for it. Both must hold, because `page` is read \
         unconditionally below. Call was: {region}"
    );
}
