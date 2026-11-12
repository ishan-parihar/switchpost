// ─── Webhooks API Routes ───────────────────────────────────────
// CRUD for outgoing webhooks with test delivery support.

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::middleware::AuthenticatedUser;

use super::AppState;

// ── Database Row Types ──────────────────────────────────────

/// A `webhooks` row. The event-type list is NOT a column on the table any
/// more — it lives in `webhook_event_types(webhook_id, event_type)`, so it is
/// fetched separately and stitched on by [`load_event_types`].
#[derive(Debug, FromRow)]
struct WebhookBaseRow {
    id: Uuid,
    user_id: Uuid,
    name: String,
    url: String,
    secret: Option<String>,
    is_active: bool,
    last_triggered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// A webhook row plus its event types, in the shape the API serialises.
#[derive(Debug)]
struct WebhookWithEvents {
    base: WebhookBaseRow,
    event_types: Vec<String>,
}

impl WebhookBaseRow {
    /// Columns shared by every webhook SELECT/RETURNING so the call sites
    /// can't drift apart.
    const SELECT_COLS: &'static str = "id, user_id, name, url, secret, is_active, \
         last_triggered_at, created_at, updated_at";
}

#[derive(Debug, FromRow, Serialize)]
pub struct WebhookDeliveryRow {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_type: String,
    pub status: String,
    pub status_code: Option<i32>,
    pub response_body: Option<String>,
    pub attempted_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

// ── Request Types ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WebhookCreateRequest {
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookUpdateRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

// ── Response Types ────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_types: Vec<String>,
    pub is_active: bool,
    pub last_triggered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Public response used for list/get — omits the HMAC secret.
/// Only the `create` handler returns the secret (once).
#[derive(Debug, Serialize)]
pub struct WebhookResponsePublic {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub event_types: Vec<String>,
    pub is_active: bool,
    pub last_triggered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct WebhookDeliveryResponse {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_type: String,
    pub status: String,
    pub status_code: Option<i32>,
    pub response_body: Option<String>,
    pub attempted_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WebhookTestResult {
    pub status_code: u16,
    pub response_body: String,
    pub delivery: WebhookDeliveryResponse,
}

// ── Helpers ─────────────────────────────────────────────────

fn webhook_to_response(w: WebhookWithEvents) -> WebhookResponse {
    let WebhookWithEvents {
        base: w,
        event_types,
    } = w;
    WebhookResponse {
        id: w.id,
        user_id: w.user_id,
        name: w.name,
        url: w.url,
        secret: w.secret,
        event_types,
        is_active: w.is_active,
        last_triggered_at: w.last_triggered_at.map(|dt| dt.to_rfc3339()),
        created_at: w.created_at.to_rfc3339(),
        updated_at: w.updated_at.to_rfc3339(),
    }
}

fn webhook_to_response_public(w: WebhookWithEvents) -> WebhookResponsePublic {
    let WebhookWithEvents {
        base: w,
        event_types,
    } = w;
    WebhookResponsePublic {
        id: w.id,
        name: w.name,
        url: w.url,
        event_types,
        is_active: w.is_active,
        last_triggered_at: w.last_triggered_at.map(|dt| dt.to_rfc3339()),
        created_at: w.created_at.to_rfc3339(),
        updated_at: w.updated_at.to_rfc3339(),
    }
}

fn delivery_to_response(d: WebhookDeliveryRow) -> WebhookDeliveryResponse {
    WebhookDeliveryResponse {
        id: d.id,
        webhook_id: d.webhook_id,
        event_type: d.event_type,
        status: d.status,
        status_code: d.status_code,
        response_body: d.response_body,
        attempted_at: d.attempted_at.to_rfc3339(),
        delivered_at: d.delivered_at.map(|dt| dt.to_rfc3339()),
    }
}

/// Read one webhook's event types from `webhook_event_types`.
async fn load_event_types(
    db: &crate::db::SqlitePool,
    webhook_id: Uuid,
) -> Result<Vec<String>, crate::error::AppError> {
    let types: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM webhook_event_types
         WHERE webhook_id = ? ORDER BY event_type",
    )
    .bind(webhook_id)
    .fetch_all(db)
    .await?;
    Ok(types)
}

/// Fetch every webhook owned by `user_id`, newest first, with each row's
/// event types stitched on.
async fn list_webhooks(
    db: &crate::db::SqlitePool,
    user_id: Uuid,
) -> Result<Vec<WebhookWithEvents>, crate::error::AppError> {
    let sql = format!(
        "SELECT {} FROM webhooks WHERE user_id = ? ORDER BY created_at DESC",
        WebhookBaseRow::SELECT_COLS
    );
    let rows: Vec<WebhookBaseRow> = sqlx::query_as(&sql).bind(user_id).fetch_all(db).await?;
    let mut out = Vec::with_capacity(rows.len());
    for base in rows {
        let event_types = load_event_types(db, base.id).await?;
        out.push(WebhookWithEvents { base, event_types });
    }
    Ok(out)
}

/// Fetch a single webhook by id scoped to `user_id`; `None` when absent.
async fn fetch_webhook(
    db: &crate::db::SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<WebhookWithEvents>, crate::error::AppError> {
    let sql = format!(
        "SELECT {} FROM webhooks WHERE id = ? AND user_id = ?",
        WebhookBaseRow::SELECT_COLS
    );
    let base: Option<WebhookBaseRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    match base {
        Some(base) => {
            let event_types = load_event_types(db, id).await?;
            Ok(Some(WebhookWithEvents { base, event_types }))
        }
        None => Ok(None),
    }
}

/// Replace a webhook's event types wholesale, inside the caller's transaction.
async fn replace_event_types(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    webhook_id: Uuid,
    event_types: &[String],
) -> Result<(), crate::error::AppError> {
    sqlx::query("DELETE FROM webhook_event_types WHERE webhook_id = ?")
        .bind(webhook_id)
        .execute(&mut **tx)
        .await?;
    for event_type in event_types {
        sqlx::query("INSERT INTO webhook_event_types (webhook_id, event_type) VALUES (?, ?)")
            .bind(webhook_id)
            .bind(event_type)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────

/// POST /api/webhooks
pub async fn create(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(body): Json<WebhookCreateRequest>,
) -> Result<Json<WebhookResponse>, crate::error::AppError> {
    if body.name.trim().is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "Webhook name is required".into(),
        ));
    }
    if body.url.trim().is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "Webhook URL is required".into(),
        ));
    }
    // The app FETCHES this URL on the founder's behalf, so it is an SSRF
    // target: a webhook pointed at the cloud metadata service would read
    // the instance credentials and hand them back through the test
    // endpoint. The guard checks the scheme, the host literal and every
    // DNS answer; `ALLOW_PRIVATE_OUTBOUND_URLS=1` is the operator's opt-in
    // for a receiver on their own machine.
    crate::net_guard::validate_outbound_url(&body.url)
        .await
        .map_err(|e| crate::error::AppError::BadRequest(format!("Webhook URL rejected: {e}")))?;

    // The event types are a second table now, so the row and its
    // `webhook_event_types` lines go in one transaction — a partial write
    // would leave a webhook that never fires.
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to begin tx: {e}")))?;

    let sql = format!(
        "INSERT INTO webhooks (id, user_id, name, url, secret)
         VALUES (?, ?, ?, ?, ?)
         RETURNING {}",
        WebhookBaseRow::SELECT_COLS
    );
    // `webhooks.id` is TEXT with no DB-side default — SQLite has no
    // gen_random_uuid(), so Rust generates the value.
    let base: WebhookBaseRow = sqlx::query_as(&sql)
        .bind(Uuid::new_v4())
        .bind(auth.user_id)
        .bind(body.name.trim())
        .bind(body.url.trim())
        .bind(body.secret)
        .fetch_one(&mut *tx)
        .await?;

    replace_event_types(&mut tx, base.id, &body.event_types).await?;

    tx.commit()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to commit webhook: {e}")))?;

    let event_types = load_event_types(&state.db, base.id).await?;
    Ok(Json(webhook_to_response(WebhookWithEvents {
        base,
        event_types,
    })))
}

/// GET /api/webhooks
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<Vec<WebhookResponsePublic>>, crate::error::AppError> {
    let rows = list_webhooks(&state.db, auth.user_id).await?;
    Ok(Json(
        rows.into_iter().map(webhook_to_response_public).collect(),
    ))
}

/// GET /api/webhooks/:id
pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<WebhookResponsePublic>, crate::error::AppError> {
    let row = fetch_webhook(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Webhook not found".into()))?;

    Ok(Json(webhook_to_response_public(row)))
}

/// PUT /api/webhooks/:id
pub async fn update(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<WebhookUpdateRequest>,
) -> Result<Json<WebhookResponsePublic>, crate::error::AppError> {
    // Fetch current row first to merge with updates
    let current = fetch_webhook(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Webhook not found".into()))?;
    let WebhookWithEvents {
        base: current,
        event_types: current_event_types,
    } = current;

    let new_name = body.name.unwrap_or(current.name);
    let new_url = body.url.unwrap_or(current.url);
    let new_secret = body.secret.or(current.secret);
    let new_event_types = body.event_types.unwrap_or(current_event_types);
    let new_is_active = body.is_active.unwrap_or(current.is_active);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to begin tx: {e}")))?;

    // `event_types` is no longer a column on `webhooks`, so the base UPDATE
    // drops it and the list is rewritten separately in the same tx.
    let sql = format!(
        "UPDATE webhooks
         SET name = ?, url = ?, secret = ?, is_active = ?,
             updated_at = unixepoch()
         WHERE id = ? AND user_id = ?
         RETURNING {}",
        WebhookBaseRow::SELECT_COLS
    );
    let base: WebhookBaseRow = sqlx::query_as(&sql)
        .bind(new_name.trim())
        .bind(new_url.trim())
        .bind(new_secret)
        .bind(new_is_active)
        .bind(id)
        .bind(auth.user_id)
        .fetch_one(&mut *tx)
        .await?;

    replace_event_types(&mut tx, id, &new_event_types).await?;

    tx.commit()
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to commit webhook: {e}")))?;

    Ok(Json(webhook_to_response_public(WebhookWithEvents {
        base,
        event_types: new_event_types,
    })))
}

/// DELETE /api/webhooks/:id
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let result = sqlx::query("DELETE FROM webhooks WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(crate::error::AppError::NotFound("Webhook not found".into()));
    }

    Ok(Json(serde_json::json!({"deleted": true})))
}

/// GET /api/webhooks/:id/deliveries
/// Lists recent deliveries for a webhook.
pub async fn deliveries(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<WebhookDeliveryResponse>>, crate::error::AppError> {
    // Verify webhook ownership and list deliveries in one join
    let rows: Vec<WebhookDeliveryRow> = sqlx::query_as(
        r#"
        SELECT d.id, d.webhook_id, d.event_type, d.status, d.status_code,
               d.response_body, d.attempted_at, d.delivered_at
        FROM webhook_deliveries d
        JOIN webhooks w ON w.id = d.webhook_id
        WHERE d.webhook_id = ? AND w.user_id = ?
        ORDER BY d.attempted_at DESC
        LIMIT 50
        "#,
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(delivery_to_response).collect()))
}

/// POST /api/webhooks/:id/test
/// Sends a test event to the webhook URL and records the delivery result.
pub async fn test(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<WebhookTestResult>, crate::error::AppError> {
    // Fetch the webhook
    let row = fetch_webhook(&state.db, id, auth.user_id)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("Webhook not found".into()))?;

    // Build test payload
    let payload = serde_json::json!({
        "event_type": "test",
        "timestamp": Utc::now().to_rfc3339(),
        "data": {
            "message": "This is a test webhook event from Switchpost"
        }
    });

    // Send the test via the dispatcher
    let result = crate::services::webhook_dispatcher::send_webhook(
        &row.base.url,
        row.base.secret.as_deref(),
        "test",
        &payload,
    )
    .await
    .map_err(|e| crate::error::AppError::BadRequest(format!("HTTP request failed: {e}")))?;

    let (status_code, response_body) = result;

    // Record delivery — only set delivered_at on success
    let delivered_dt = if (200..300).contains(&status_code) {
        Some(Utc::now())
    } else {
        None
    };
    let delivery_row: WebhookDeliveryRow = sqlx::query_as(
        r#"
        INSERT INTO webhook_deliveries (id, webhook_id, event_type, payload, status, status_code, response_body, delivered_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id, webhook_id, event_type, status, status_code, response_body, attempted_at, delivered_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind("test")
    .bind(&payload)
    .bind(if status_code == 200 || status_code == 201 { "delivered" } else { "failed" })
    .bind(status_code as i32)
    .bind(&response_body)
    .bind(delivered_dt)
    .fetch_one(&state.db)
    .await?;

    // Update last_triggered_at
    if let Err(e) = sqlx::query("UPDATE webhooks SET last_triggered_at = unixepoch() WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        tracing::warn!(%id, error = %e, "Failed to update webhook last_triggered_at");
    }

    Ok(Json(WebhookTestResult {
        status_code,
        response_body,
        delivery: delivery_to_response(delivery_row),
    }))
}
