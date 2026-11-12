// ─── Post sets (saved templates), from the shell ────────────────
//
// The MCP has `set_create` / `set_list` / `set_delete` in its CORE
// toolset. The CLI had no verb for them at all: the parity claim in
// AGENTS §4 was never MEASURED, and measuring it (tests/mcp_parity_test.rs)
// found the hole.
//
// The handlers call the REST handlers in-process (AGENTS §9.21): the MCP
// tools do the same, so the shell, the agent and the browser all see one
// implementation of "a saved template".

use crate::api::AppState;
use crate::cli::SetsAction;

pub async fn handle(action: SetsAction, state: &AppState) -> anyhow::Result<()> {
    let user_id = match crate::mcp::tools_posts::resolve_first_user(state).await {
        Ok(id) => id,
        Err(e) => return Err(anyhow::anyhow!("Auth error: {e}")),
    };
    let auth = crate::auth::middleware::AuthenticatedUser { user_id };

    let result: Result<serde_json::Value, String> = match action {
        SetsAction::List => {
            let sets = crate::api::sets::list_sets(axum::extract::State(state.clone()), auth)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let axum::Json(sets) = sets;
            Ok(serde_json::json!({ "data": sets }))
        }
        SetsAction::Create {
            name,
            content,
            description,
            channels,
        } => {
            let content: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Invalid --content JSON: {e}"))?;
            let channel_ids: Vec<uuid::Uuid> = channels
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    uuid::Uuid::parse_str(s)
                        .map_err(|_| anyhow::anyhow!("Invalid channel id `{s}` — expected a UUID"))
                })
                .collect::<Result<Vec<_>, anyhow::Error>>()?;
            let created = crate::api::sets::create_set(
                axum::extract::State(state.clone()),
                auth,
                axum::Json(crate::api::sets::CreateSetRequest {
                    name,
                    description,
                    content,
                    channel_ids,
                }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let axum::Json(set) = created;
            Ok(serde_json::json!({ "data": set }))
        }
        SetsAction::Delete { id } => {
            let set_id = uuid::Uuid::parse_str(&id)
                .map_err(|_| anyhow::anyhow!("Invalid set id `{id}` — expected a UUID"))?;
            let deleted = crate::api::sets::delete_set(
                axum::extract::State(state.clone()),
                auth,
                axum::extract::Path(set_id),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let axum::Json(value) = deleted;
            Ok(value)
        }
    };

    match result {
        Ok(value) => {
            crate::cli::run::output_json(&value);
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}
