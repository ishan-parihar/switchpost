//! Outbound-URL guard — ONE rule for every URL the app fetches on the
//! founder's behalf (webhook receivers, RSS feeds).
//!
//! ## Why this exists
//!
//! A webhook URL and an RSS feed URL are both attacker-influenced strings
//! the SERVER fetches. Before this module:
//!
//! - `POST /api/webhooks` accepted any URL (a non-empty string was the
//!   only check), and `POST /api/webhooks/{id}/test` sent to it and
//!   returned the receiver's response body to the caller. That is an
//!   authenticated SSRF with a READ-BACK: `http://169.254.169.254/…`
//!   answered the cloud metadata request and handed the body back.
//! - `POST /api/rss` stored any feed URL, and the poller fetched it with
//!   `reqwest::get` — default redirect policy, no host check — and showed
//!   the parsed title/link/content back in the feed-items list.
//!
//! A redirect made both worse: a public-looking URL that answers `302
//! Location: http://127.0.0.1:6543/` reached the app's own API.
//!
//! ## The rule
//!
//! An outbound URL is allowed when:
//!
//! 1. its scheme is `http` or `https`;
//! 2. its host is not a loopback / private / link-local / CGNAT address —
//!    checked on the LITERAL and on every DNS answer, so a name that
//!    resolves to 10.0.0.5 is refused the same way the literal is;
//! 3. every redirect hop is checked again, because a hop is a new URL.
//!
//! The founder who genuinely wants a webhook on their own machine (an n8n
//! on `localhost`, a receiver on the LAN) sets
//! `ALLOW_PRIVATE_OUTBOUND_URLS=1`. That is the operator's decision to
//! make, not a default.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::Url;

/// Environment variable that opts back into private/loopback targets.
pub const ALLOW_PRIVATE_ENV: &str = "ALLOW_PRIVATE_OUTBOUND_URLS";

/// How many redirects are followed. Each hop is validated.
const MAX_REDIRECTS: usize = 5;

/// Are private targets allowed right now?
///
/// Public so the callers that need the same answer (the webhook
/// dispatcher, the RSS poller) do not each re-implement the rule.
pub fn private_targets_allowed() -> bool {
    private_allowed()
}

/// Are private targets allowed right now?
///
/// Read per call, not cached: the operator can restart the service after
/// changing the variable, and a cached answer would outlive the change.
fn private_allowed() -> bool {
    std::env::var(ALLOW_PRIVATE_ENV)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

/// Is this IP address one the app must not reach on the founder's behalf?
///
/// Covers the ranges that matter for an SSRF:
///
/// - `127.0.0.0/8` loopback, `0.0.0.0/8` "this host";
/// - `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` private LAN;
/// - `169.254.0.0/16` link-local — which is where the cloud metadata
///   service lives (`169.254.169.254`);
/// - `100.64.0.0/10` carrier-grade NAT;
/// - IPv6 `::1`, `fc00::/7` unique-local, `fe80::/10` link-local;
/// - IPv4-mapped IPv6 (`::ffff:127.0.0.1`), which is the same host.
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(v4: &Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        // 100.64.0.0/10 — carrier-grade NAT (RFC 6598)
        || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
}

/// Is this IPv6 address private / local?
fn is_private_v6(v6: &Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    // IPv4-mapped / IPv4-compatible: ::ffff:127.0.0.1 is 127.0.0.1.
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_private_v4(&v4);
    }
    let seg = v6.segments();
    // fc00::/7 — unique local
    if (seg[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 — link local
    if (seg[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}

/// Is this HOST NAME one the app must not reach?
///
/// Names, not just addresses: `localhost`, `*.localhost`, `*.local`
/// (mDNS), `*.internal` (cluster DNS), and the two cloud metadata names.
pub fn is_private_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    if h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h == "metadata.google.internal"
        || h == "instance-data"
    {
        return true;
    }
    // A literal address is judged by the address rules.
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_private_ip(&ip);
    }
    false
}

/// Validate an outbound URL and return it parsed.
///
/// Checks the scheme, the host LITERAL, and every address the host
/// resolves to. A DNS failure is NOT a pass: the fetch would fail anyway,
/// and a name that cannot be resolved is not a name to hand to the
/// resolver twice.
pub async fn validate_outbound_url(raw: &str) -> Result<Url, String> {
    validate_outbound_url_with(raw, private_allowed()).await
}

/// The same check with the private-target decision passed in.
///
/// The explicit form is what the tests use: a test that sets an
/// environment variable would race every other test in the process.
pub async fn validate_outbound_url_with(raw: &str, allow_private: bool) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL is empty".into());
    }
    let parsed = Url::parse(trimmed).map_err(|e| format!("not a valid URL: {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "scheme `{}` is not allowed — use http or https",
            parsed.scheme()
        ));
    }
    let host = parsed.host_str().unwrap_or("").to_string();
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    if allow_private {
        return Ok(parsed);
    }
    if is_private_host(&host) {
        return Err(format!(
            "host `{host}` is a local or private address; outbound requests to it are refused. \
             Set {ALLOW_PRIVATE_ENV}=1 if you mean it."
        ));
    }
    // Resolve and check every answer: a public name can point at 10.0.0.5.
    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::net::lookup_host((host.clone(), port)).await {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            if addrs.is_empty() {
                return Err(format!("host `{host}` did not resolve to any address"));
            }
            for addr in &addrs {
                if is_private_ip(&addr.ip()) {
                    return Err(format!(
                        "host `{host}` resolves to the private address {}; outbound requests to it \
                         are refused. Set {ALLOW_PRIVATE_ENV}=1 if you mean it.",
                        addr.ip()
                    ));
                }
            }
            Ok(parsed)
        }
        Err(e) => Err(format!("host `{host}` could not be resolved: {e}")),
    }
}

/// The response of a guarded outbound request.
pub struct GuardedResponse {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Send an outbound request, validating the URL and every redirect hop.
///
/// The CLIENT MUST be built with `redirect(Policy::none())` — the hops are
/// followed here, by hand, so each one can be validated. A client that
/// follows redirects on its own would skip the check on every hop but the
/// first.
///
/// `max_body_bytes` caps the body that is read into memory.
pub async fn send_guarded(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
    max_body_bytes: usize,
) -> Result<GuardedResponse, String> {
    send_guarded_with(
        client,
        method,
        url,
        headers,
        body,
        max_body_bytes,
        private_allowed(),
    )
    .await
}

/// The same send with the private-target decision passed in.
pub async fn send_guarded_with(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
    max_body_bytes: usize,
    allow_private: bool,
) -> Result<GuardedResponse, String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let parsed = validate_outbound_url_with(&current, allow_private).await?;
        let mut req = client.request(method.clone(), parsed.clone());
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        if let Some(b) = &body {
            req = req.body(b.clone());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let status = resp.status().as_u16();
        let resp_headers = resp.headers().clone();
        if is_redirect(status) {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("redirect ({status}) without a Location header"))?;
            let next = parsed
                .join(location)
                .map_err(|e| format!("redirect to an unusable Location: {e}"))?;
            current = next.to_string();
            continue;
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read response body: {e}"))?;
        let body = if bytes.len() > max_body_bytes {
            String::from_utf8_lossy(&bytes[..max_body_bytes]).into_owned()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        return Ok(GuardedResponse {
            status,
            headers: resp_headers,
            body,
        });
    }
    Err(format!("too many redirects (>{MAX_REDIRECTS})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.10",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                is_private_host(ip),
                "{ip} must be refused as an outbound target"
            );
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",
            "172.15.0.1",
            "93.184.216.34",
        ] {
            assert!(!is_private_host(ip), "{ip} must be allowed");
        }
    }

    #[test]
    fn local_names_are_refused() {
        for host in [
            "localhost",
            "app.localhost",
            "printer.local",
            "db.internal",
            "metadata.google.internal",
            "instance-data",
        ] {
            assert!(is_private_host(host), "{host} must be refused");
        }
    }

    #[test]
    fn public_names_are_allowed_by_name() {
        // The NAME is not private; the DNS answers are checked separately.
        for host in ["example.com", "hooks.example.org", "8.8.8.8"] {
            assert!(!is_private_host(host), "{host} must be allowed by name");
        }
    }

    #[tokio::test]
    async fn a_non_http_scheme_is_refused() {
        let err = validate_outbound_url_with("file:///etc/passwd", false)
            .await
            .expect_err("file:// must be refused");
        assert!(err.contains("scheme"), "got: {err}");
    }

    #[tokio::test]
    async fn a_localhost_url_is_refused() {
        let err = validate_outbound_url_with("http://127.0.0.1:6543/api/posts", false)
            .await
            .expect_err("loopback must be refused");
        assert!(err.contains("private"), "got: {err}");
    }

    #[tokio::test]
    async fn an_empty_url_is_refused() {
        assert!(validate_outbound_url_with("   ", false).await.is_err());
    }
}
