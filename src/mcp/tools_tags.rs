// ─── MCP Tags Tools ─────────────────────────────────────────────
// CRUD tools for user-defined tags.

use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::types::EpochUtc;

use crate::api::AppState;

// ── Input Types ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TagCreateInput {
    /// Tag name (required)
    pub name: String,
    /// Hex color code (e.g. "#6366f1", defaults to indigo)
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TagListInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TagGetInput {
    /// Tag ID (UUID)
    pub tag_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TagUpdateInput {
    /// Tag ID (UUID)
    pub tag_id: String,
    /// New tag name (optional)
    pub name: Option<String>,
    /// New hex color code (optional)
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TagDeleteInput {
    /// Tag ID (UUID)
    pub tag_id: String,
}

// ── Handlers ─────────────────────────────────────────────────

/// Create a new tag
pub async fn handle_tag_create(
    state: &AppState,
    input: &TagCreateInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("Tag name cannot be empty".into());
    }
    let color = input.color.as_deref().unwrap_or("#6366f1").to_string();
    let id = Uuid::new_v4();

    let tag = sqlx::query!(
        r#"INSERT INTO tags (id, user_id, name, color)
           VALUES (?, ?, ?, ?)
           RETURNING id as "id!: Uuid", name, color,
                     created_at as "created_at: EpochUtc",
                     updated_at as "updated_at: EpochUtc""#,
        id,
        user_id,
        name,
        color,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Failed to create tag: {e}"))?;

    Ok(Json(serde_json::json!({
        "data": {
            "id": tag.id.to_string(),
            "name": tag.name,
            "color": tag.color,
            "created_at": tag.created_at.0.to_rfc3339(),
            "updated_at": tag.updated_at.0.to_rfc3339(),
        }
    })))
}

/// List all tags for the user
pub async fn handle_tag_list(
    state: &AppState,
    _input: &TagListInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;

    let tags = sqlx::query!(
        r#"SELECT id as "id!: Uuid", name, color,
                  created_at as "created_at: EpochUtc",
                  updated_at as "updated_at: EpochUtc"
           FROM tags WHERE user_id = ? ORDER BY name ASC"#,
        user_id,
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| format!("Failed to list tags: {e}"))?;

    let data: Vec<serde_json::Value> = tags
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "name": t.name,
                "color": t.color,
                "created_at": t.created_at.0.to_rfc3339(),
                "updated_at": t.updated_at.0.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "data": data })))
}

/// Get a single tag by ID
pub async fn handle_tag_get(
    state: &AppState,
    input: &TagGetInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let tag_id =
        Uuid::parse_str(&input.tag_id).map_err(|_| format!("Invalid tag ID: {}", input.tag_id))?;

    let tag = sqlx::query!(
        r#"SELECT id as "id!: Uuid", name, color,
                  created_at as "created_at: EpochUtc",
                  updated_at as "updated_at: EpochUtc"
           FROM tags WHERE id = ? AND user_id = ?"#,
        tag_id,
        user_id,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Failed to get tag: {e}"))?
    .ok_or_else(|| "Tag not found".to_string())?;

    Ok(Json(serde_json::json!({
        "data": {
            "id": tag.id.to_string(),
            "name": tag.name,
            "color": tag.color,
            "created_at": tag.created_at.0.to_rfc3339(),
            "updated_at": tag.updated_at.0.to_rfc3339(),
        }
    })))
}

/// Update an existing tag.
///
/// ROUTES THROUGH THE REST HANDLER (AGENTS §9.21): the tool layer converts
/// types, and the validation and the SQL live in `api::tags::update`.
///
/// This used to re-implement the UPDATE here — and bound the parameters in
/// the wrong order. The SQL reads `name = COALESCE(?, name), color =
/// COALESCE(?, color) … WHERE id = ? AND user_id = ?`, and the binds were
/// `tag_id, user_id, new_name, new_color`. So the name was set to the tag's
/// UUID, the colour to the user's UUID, and the WHERE clause looked for a
/// tag whose id was the NEW NAME. The tool could never rename a tag: it
/// answered "Tag not found" for every request, or wrote a UUID into the
/// name if the tag id happened to match one of them.
pub async fn handle_tag_update(
    state: &AppState,
    input: &TagUpdateInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let tag_id = super::rest_bridge::parse_id("tag_id", &input.tag_id)?;

    let updated = crate::mcp::rest_bridge::rest_result(
        crate::api::tags::update(
            axum::extract::State(state.clone()),
            crate::auth::middleware::AuthenticatedUser { user_id },
            axum::extract::Path(tag_id),
            axum::Json(crate::api::tags::TagUpdateRequest {
                name: input.name.clone(),
                color: input.color.clone(),
            }),
        )
        .await,
    )?;

    let axum::Json(body) = axum::Json(updated.0.clone());
    Ok(Json(serde_json::json!({ "data": body })))
}

/// Delete a tag
pub async fn handle_tag_delete(
    state: &AppState,
    input: &TagDeleteInput,
) -> Result<Json<serde_json::Value>, String> {
    let user_id = super::tools_posts::resolve_first_user(state).await?;
    let tag_id =
        Uuid::parse_str(&input.tag_id).map_err(|_| format!("Invalid tag ID: {}", input.tag_id))?;

    let result = sqlx::query!(
        "DELETE FROM tags WHERE id = ? AND user_id = ?",
        tag_id,
        user_id,
    )
    .execute(&state.db)
    .await
    .map_err(|e| format!("Failed to delete tag: {e}"))?;

    if result.rows_affected() == 0 {
        return Err("Tag not found".to_string());
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}
