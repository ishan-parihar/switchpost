// ─── Webhook Dispatcher Service ─────────────────────────────────
// Sends outbound webhook events asynchronously with HMAC signing.
// Records delivery results in the webhook_deliveries table.

use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::AppState;

/// Compute HMAC-SHA256 manually using sha2 crate.
/// Uses the standard HMAC construction: HMAC(K, m) = H((K' ⊕ opad) || H((K' ⊕ ipad) || m))
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;

    let key = if key.len() > BLOCK_SIZE {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.finalize().to_vec()
    } else {
        key.to_vec()
    };

    // Pad key to block size
    let mut key_padded = key.clone();
    key_padded.resize(BLOCK_SIZE, 0);

    // Inner hash: H((K ⊕ ipad) || message)
    let mut inner = Vec::with_capacity(BLOCK_SIZE + message.len());
    for &k in &key_padded {
        inner.push(k ^ 0x36);
    }
    inner.extend_from_slice(message);
    let inner_hash = Sha256::digest(&inner);

    // Outer hash: H((K ⊕ opad) || inner_hash)
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 32);
    for &k in &key_padded {
        outer.push(k ^ 0x5c);
    }
    outer.extend_from_slice(&inner_hash);
    Sha256::digest(&outer).to_vec()
}

/// Send a webhook HTTP POST with optional HMAC-SHA256 signature.
/// Returns (status_code, response_body).
pub async fn send_webhook(
    url: &str,
    secret: Option<&str>,
    event_type: &str,
    payload: &Value,
) -> Result<(u16, String), String> {
    send_webhook_with(
        url,
        secret,
        event_type,
        payload,
        crate::net_guard::private_targets_allowed(),
    )
    .await
}

/// The same send with the private-target decision passed in.
///
/// The test receiver is on 127.0.0.1, so the test asks for it explicitly
/// instead of flipping an environment variable that every other test in
/// the process would see.
#[cfg_attr(not(test), allow(dead_code))]
async fn send_webhook_with(
    url: &str,
    secret: Option<&str>,
    event_type: &str,
    payload: &Value,
    allow_private: bool,
) -> Result<(u16, String), String> {
    // Redirects are followed BY HAND in `net_guard::send_guarded`, so
    // every hop can be validated. A client that follows them itself would
    // skip the check on every hop but the first — which is how a public
    // URL reached 127.0.0.1 before the guard existed.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let body =
        serde_json::to_string(payload).map_err(|e| format!("Failed to serialize payload: {e}"))?;

    let mut headers: Vec<(String, String)> = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("X-Webhook-Event".to_string(), event_type.to_string()),
    ];
    if let Some(secret_key) = secret {
        let signature = hmac_sha256(secret_key.as_bytes(), body.as_bytes());
        let sig_hex = hex::encode(signature);
        headers.push((
            "X-Webhook-Signature".to_string(),
            format!("sha256={sig_hex}"),
        ));
    }
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let response = crate::net_guard::send_guarded_with(
        &client,
        reqwest::Method::POST,
        url,
        &header_refs,
        Some(body),
        MAX_RESPONSE_BYTES,
        allow_private,
    )
    .await?;

    let status_code = response.status;
    let response_body = response.body;

    Ok((status_code, response_body))
}

/// Keep at most this much of a receiver's response. It is stored in the
/// delivery log, so an unbounded body would bloat the database.
const MAX_RESPONSE_BYTES: usize = 4096;

/// Attempts per webhook before a delivery is recorded as failed.
const WEBHOOK_ATTEMPTS: u32 = 3;

/// Dispatch an event to all active webhooks matching the event type for a user.
/// Runs synchronously (caller should spawn in background task if needed).
pub async fn dispatch_event(state: &AppState, event_type: &str, payload: &Value, user_id: Uuid) {
    let webhooks = match sqlx::query_as::<_, WebhookDispatchRow>(
        r#"
        SELECT w.id, w.url, w.secret
        FROM webhooks w
        WHERE w.user_id = ?
          AND w.is_active = 1
          AND EXISTS (
            SELECT 1 FROM webhook_event_types t
            WHERE t.webhook_id = w.id AND t.event_type = ?
          )
        "#,
    )
    .bind(user_id)
    .bind(event_type)
    .fetch_all(&state.db)
    .await
    {
        Ok(wh) => wh,
        Err(e) => {
            tracing::error!("Failed to fetch webhooks for dispatch: {e}");
            return;
        }
    };

    if webhooks.is_empty() {
        return;
    }

    let payload_json = serde_json::to_value(payload).unwrap_or_else(|_| serde_json::json!({}));

    for wh in webhooks {
        let url = wh.url.clone();
        let secret = wh.secret.clone();
        let webhook_id = wh.id;

        // Retry only failures that may pass: a network error, 429 or 5xx.
        // Other 4xx mean the receiver rejected the event, so retrying cannot help.
        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            let result = send_webhook(&url, secret.as_deref(), event_type, &payload_json).await;
            let retryable = match &result {
                Ok((code, _)) => *code == 429 || *code >= 500,
                Err(_) => true,
            };
            if !retryable || attempt >= WEBHOOK_ATTEMPTS {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
        };

        let (status, status_code, response_body) = match &result {
            Ok((code, body)) => {
                if (200..300).contains(code) {
                    ("delivered", Some(*code as i32), Some(body.clone()))
                } else {
                    ("failed", Some(*code as i32), Some(body.clone()))
                }
            }
            Err(e) => ("failed", None, Some(e.clone())),
        };

        let delivered_at = if status == "delivered" {
            Some(Utc::now())
        } else {
            None
        };

        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO webhook_deliveries (id, webhook_id, event_type, payload, status, status_code, response_body, delivered_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(webhook_id)
        .bind(event_type)
        .bind(&payload_json)
        .bind(status)
        .bind(status_code)
        .bind(&response_body)
        .bind(delivered_at)
        .execute(&state.db)
        .await
        {
            tracing::error!("Failed to record webhook delivery: {e}");
        }

        if let Err(e) =
            sqlx::query("UPDATE webhooks SET last_triggered_at = unixepoch() WHERE id = ?")
                .bind(webhook_id)
                .execute(&state.db)
                .await
        {
            tracing::error!("Failed to update webhook last_triggered_at: {e}");
        }
    }
}

// ── Internal Row Type ────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct WebhookDispatchRow {
    id: Uuid,
    url: String,
    secret: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn response_body_is_capped_at_the_stored_limit() {
        // A receiver that answers 200 with a body far larger than the cap.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = [0u8; 4096];
            let _ = sock.read(&mut req).await;
            let body = "x".repeat(10_000);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });

        let (code, body) = send_webhook_with(
            &format!("http://{addr}/hook"),
            None,
            "post_published",
            &serde_json::json!({}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        assert_eq!(body.len(), MAX_RESPONSE_BYTES, "stored body must be capped");
    }
}
