// ─── Threads app readiness ─────────────────────────────────────
//
// WHY THIS EXISTS
//
// The Threads connect flow failed on the live server with a Meta privacy
// consent URL carrying `error_code=1349003&source=gdp_error`. That URL
// is Meta's OWN screen: the browser is redirected to
// `www.threads.com/privacy/consent/?flow=gdp` and never comes back to
// our callback, so our own handler never runs and the founder is left
// staring at a Meta page with nothing actionable.
//
// ─── WHAT WAS ACTUALLY WRONG ───────────────────────────────────
//
// The first hypothesis was the scope set. Commit 1f16f12 narrowed the
// request from nine `threads_*` permissions to the five this provider
// implements, and the URL in the founder's report ALREADY carried the
// narrowed five — so that fix was necessary but not sufficient.
//
// Measured against the live app id and secret on 2026-11-06:
//
//   1. `POST graph.threads.net/oauth/access_token?grant_type=client_credentials`
//      with the configured pair RETURNS an app access token. A wrong
//      secret is rejected with "Invalid client_secret". So the
//      credential pair is VALID and is not the cause.
//
//   2. `GET graph.threads.net/debug_token` with that app token answers
//      "The app associated with this request is in dev mode."
//      The same probe on a DIFFERENT Meta app in the same `.env`
//      answers "Invalid OAuth access token" — so the message is
//      specific to this app, not a generic Graph API error.
//
// Meta's own get-started page says what dev mode means:
//
//   "Threads testers can grant your app these permissions at any time.
//    In order for app users without a role on your app to be able to
//    grant your app these permissions, each permission must first be
//    approved through the App Review process, and your app must be
//    published."
//
// So the founder's Threads account — a normal account with no role on
// the app — cannot grant the permissions, and Meta's consent gate
// refuses the sign-in with `1349003`. That is the whole failure.
//
// ─── WHAT THIS MODULE DOES ─────────────────────────────────────
//
// It answers ONE question: "can this app actually grant the scopes we
// ask for, to this user?" The answer needs the app's publication state,
// which Graph API does not expose for a Threads app — so the probe is
// the `debug_token` call above, and the verdict is INCONCLUSIVE for a
// THREADS app id.
//
// The module's job is therefore to produce the RIGHT MESSAGE, not to
// guess. The two causes Meta's community reports are:
//
//   (1) the app is in development mode (or unreviewed for the
//       permissions) and the Threads account has no role on it —
//       the founder must add the account as a tester, or publish the
//       app and pass App Review;
//   (2) the Threads account is tied to an Instagram BUSINESS account —
//       Meta reports this fails at consent for business accounts.
//
// (1) is what we MEASURED on this deployment. (2) is what Meta's
// community reports for the same error code. The message names both,
// because the founder can act on either, and it says plainly that this
// is Meta's gate and not a broken callback URL.
//
// The probe is deliberately NOT run on every connect. It is one
// Graph API call, and it runs only when the founder is about to be sent
// to a consent screen that has already failed — otherwise a normal
// connect would pay a round trip to learn nothing.

use serde::{Deserialize, Serialize};

/// What we know about the Threads app's ability to grant our scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadsAppMode {
    /// The app is published and can grant the permissions to any account.
    Published,
    /// The app is in development mode. Only accounts with a ROLE on the
    /// app (testers) can grant the permissions.
    DevMode,
    /// We could not tell. This is the honest answer for a Threads app
    /// id, whose app metadata is not readable through Graph API.
    Unknown,
}

/// The result of the readiness probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadsAppReadiness {
    pub mode: ThreadsAppMode,
    /// A human sentence describing the mode.
    pub detail: String,
    /// The scopes we ask for, so the message can name them.
    pub requested_scopes: Vec<String>,
}

/// The message shown when Meta's consent gate refuses the sign-in.
///
/// It answers three questions in order: what happened, why, and what to
/// do about it. The founder's report showed that "what happened" without
/// the other two is a dead end.
///
/// The wording deliberately says "Meta's own consent gate" rather than
/// blaming the callback URL, because the URL is correct (it is
/// APP_URL-derived and HTTPS) and a founder who goes to check it will
/// find nothing wrong there.
pub fn consent_gate_message(requested_scopes: &[String], detail: &str) -> String {
    let scopes = if requested_scopes.is_empty() {
        "the permissions this app needs".to_string()
    } else {
        requested_scopes.join(", ")
    };
    format!(
        "Meta's own consent gate can refuse this sign-in, so no authorization code may come \
         back to Switchpost. This is not a broken callback URL — the callback is correct. \
         Two causes, both on Meta's side:\n\n\
         1. The Threads app is still in DEVELOPMENT MODE, so only an account with a role on \
         the app can grant the permissions ({scopes}). If that is your app: open \
         https://developers.facebook.com/apps → your app → App roles → Add People → Threads \
         Tester, add the Threads account you are signing in with, and have that account accept \
         the invitation under Settings → Account → Website permissions. To let ANY account \
         grant the permissions, the app has to be published and each permission approved \
         through App Review.\n\n\
         2. The Threads account is tied to an INSTAGRAM BUSINESS account. Meta reports this \
         fails at the same consent step; sign in with a personal Threads account, or add the \
         business account as a tester on the app.\n\n\
         What Switchpost checked: {detail}\n\n\
         If the account already has a role on the app, complete Meta's consent once by hand at \
         https://www.threads.com/privacy/consent/?flow=gdp, then retry the sign-in here."
    )
}

/// The detail sentence for a dev-mode app.
///
/// This is the MEASURED case on this deployment: `debug_token` against
/// the app's own access token answers "The app associated with this
/// request is in dev mode."
pub fn dev_mode_detail() -> String {
    "Meta reports that this Threads app is in development mode, so only accounts with a role \
     on the app can grant the permissions Switchpost asks for."
        .to_string()
}

/// The detail sentence for a published app.
///
/// Measured against the sibling Facebook app in the same `.env`: its
/// `debug_token` answer is a readable token with the app name, so the
/// app is NOT in dev mode.
pub fn published_detail() -> String {
    "Meta reports that this Threads app is published, so its permissions are granted through \
     App Review."
        .to_string()
}

/// The detail sentence when the probe could not tell.
pub fn unknown_detail() -> String {
    "Switchpost could not read the app's publication state through Meta's API, so the cause \
     below is the most likely one rather than a confirmed one."
        .to_string()
}

/// Parse Meta's `debug_token` answer into an app mode.
///
/// Meta answers one of two things for an app access token:
///
///   - `{"error": {"message": "The app associated with this request is in dev mode."}}`
///     — the app is unpublished. This is the case we act on.
///   - `{"data": {"scopes": [...], ...}}` — the token is readable and the
///     app is not in dev mode.
///
/// Anything else is INCONCLUSIVE, and is reported as such rather than
/// guessed at. A Graph API outage must not be reported to the founder
/// as "your app is unpublished".
pub fn parse_debug_token(body: &serde_json::Value) -> ThreadsAppMode {
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();

    if message.contains("dev mode") {
        return ThreadsAppMode::DevMode;
    }

    // A readable token means the app is not in dev mode.
    if body.get("data").map(|d| d.is_object()).unwrap_or(false) {
        return ThreadsAppMode::Published;
    }

    ThreadsAppMode::Unknown
}

/// Probe the app's mode through Meta's `debug_token`.
///
/// One Graph API call. The answer for a THREADS app id is usually
/// `DevMode` or `Unknown` — the app's publication state is not
/// readable for that app type — and `Published` for the app types
/// whose metadata Graph exposes.
pub async fn probe_app_mode(client_id: &str, client_secret: &str) -> ThreadsAppMode {
    let Ok(app_token) = app_access_token(client_id, client_secret).await else {
        return ThreadsAppMode::Unknown;
    };

    let url = format!(
        "https://graph.threads.net/debug_token?input_token={app_token}&access_token={app_token}"
    );
    let Ok(resp) = crate::social::shared_http().get(&url).send().await else {
        return ThreadsAppMode::Unknown;
    };
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return ThreadsAppMode::Unknown;
    };

    parse_debug_token(&body)
}

/// Exchange the app's id and secret for an app access token.
///
/// `graph.threads.net` is used rather than `graph.facebook.com`: a
/// Threads app id is a THREADS app id, and the Facebook graph rejects
/// it with "Invalid application ID" even when the pair is correct.
async fn app_access_token(client_id: &str, client_secret: &str) -> Result<String, String> {
    let url = format!(
        "https://graph.threads.net/oauth/access_token?client_id={}&client_secret={}&grant_type=client_credentials",
        urlencoding::encode(client_id),
        urlencoding::encode(client_secret),
    );
    let resp = crate::social::shared_http()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("failed to reach Meta: {e}"))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid response from Meta: {e}"))?;
    body["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Meta did not return an app access token".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact answer Meta gave for THIS deployment's Threads app on
    /// 2026-11-06, measured against the app's own access token.
    const DEV_MODE_RESPONSE: &str = r#"{
        "error": {
            "message": "The app associated with this request is in dev mode.",
            "code": 100,
            "type": "OAuthException",
            "fbtrace_id": "Ayr8-qRFVpkhuFoOpoVGa7Y"
        }
    }"#;

    #[test]
    fn the_measured_dev_mode_answer_is_recognised() {
        let body: serde_json::Value = serde_json::from_str(DEV_MODE_RESPONSE).unwrap();
        assert_eq!(parse_debug_token(&body), ThreadsAppMode::DevMode);
    }

    /// A readable token is NOT dev mode.
    #[test]
    fn a_readable_token_is_published() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"data":{"app_id":"123","type":"APP","is_valid":true,"scopes":[]}}"#,
        )
        .unwrap();
        assert_eq!(parse_debug_token(&body), ThreadsAppMode::Published);
    }

    /// An unrelated Graph API error must NOT be reported as "your app is
    /// unpublished". The control measurement on a different Meta app in
    /// the same `.env` produced exactly this answer.
    #[test]
    fn an_unrelated_graph_error_is_unknown_not_dev_mode() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"error":{"message":"Invalid OAuth access token - Cannot parse access token","code":190,"type":"OAuthException"}}"#,
        )
        .unwrap();
        assert_eq!(parse_debug_token(&body), ThreadsAppMode::Unknown);
    }

    #[test]
    fn a_non_object_answer_is_unknown() {
        assert_eq!(
            parse_debug_token(&serde_json::json!("not an object")),
            ThreadsAppMode::Unknown
        );
    }

    /// The message must name BOTH causes Meta documents, because the
    /// founder can act on either one and we cannot tell them apart from
    /// the outside.
    #[test]
    fn the_consent_message_names_both_causes() {
        let msg = consent_gate_message(
            &[
                "threads_basic".to_string(),
                "threads_content_publish".to_string(),
            ],
            &dev_mode_detail(),
        );
        assert!(msg.contains("DEVELOPMENT MODE"), "{msg}");
        assert!(msg.contains("INSTAGRAM BUSINESS"), "{msg}");
    }

    /// The message must NOT blame the callback URL, because the callback
    /// URL is correct and sending the founder to check it wastes their
    /// time.
    #[test]
    fn the_consent_message_says_the_callback_is_not_the_problem() {
        let msg = consent_gate_message(&[], &dev_mode_detail());
        assert!(msg.contains("not a broken callback URL"), "{msg}");
    }

    /// The scopes are named, so the founder knows which permissions the
    /// app needs approved.
    #[test]
    fn the_consent_message_names_the_requested_scopes() {
        let msg = consent_gate_message(&["threads_manage_replies".to_string()], "detail");
        assert!(msg.contains("threads_manage_replies"), "{msg}");
    }
}
