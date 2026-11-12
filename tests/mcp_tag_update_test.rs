// ─── MCP tag update ────────────────────────────────────────────
// The `tags_update` MCP tool re-implemented the UPDATE instead of calling the
// REST handler, and bound the parameters in the WRONG ORDER:
//
//     UPDATE tags SET name = COALESCE(?, name), color = COALESCE(?, color) …
//       WHERE id = ? AND user_id = ?
//
// with the binds `tag_id, user_id, new_name, new_color`. So the name was set
// to the tag's UUID, the colour to the user's UUID, and the WHERE clause
// looked for a tag whose id was the NEW NAME. The tool could never rename a
// tag.
//
// The tool now routes through `api::tags::update`, so the SQL and the
// ownership check live in one place.
//
// Run: cargo test --test mcp_tag_update_test
// Uses the isolated test DB from `load_dotenv_for_tests` (never data/switchpost.db).

use rmcp::Json;

use switchpost::api::AppState;
use switchpost::config::Config;
use switchpost::db;
use switchpost::mcp::tools_tags::{
    handle_tag_create, handle_tag_update, TagCreateInput, TagUpdateInput,
};
use switchpost::realtime::Broadcaster;
use switchpost::social::registry::ProviderRegistry;

async fn state() -> AppState {
    switchpost::config::load_dotenv_for_tests();
    let config = Config::from_env().expect("config");
    let db = db::create_pool(&config.database_url).await.expect("pool");
    db::ensure_local_user(&db).await.expect("local user");
    let registry = ProviderRegistry::new(&config, None, None);
    AppState {
        db,
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

#[tokio::test]
async fn the_mcp_tool_renames_a_tag() {
    let state = state().await;
    let name = format!("tag-{}", uuid::Uuid::new_v4());

    let Json(created) = handle_tag_create(
        &state,
        &TagCreateInput {
            name: name.clone(),
            color: Some("#6366f1".into()),
        },
    )
    .await
    .expect("create the tag");
    let tag_id = created["data"]["id"].as_str().expect("id").to_string();

    let new_name = format!("renamed-{}", uuid::Uuid::new_v4());
    let Json(updated) = handle_tag_update(
        &state,
        &TagUpdateInput {
            tag_id: tag_id.clone(),
            name: Some(new_name.clone()),
            color: Some("#ff0000".into()),
        },
    )
    .await
    .expect("rename the tag");

    assert_eq!(
        updated["data"]["name"].as_str(),
        Some(new_name.as_str()),
        "the tag must carry the new name, not the tag's UUID"
    );
    assert_eq!(updated["data"]["color"].as_str(), Some("#ff0000"));

    /**/
    let name_in_db: String = sqlx::query_scalar("SELECT name FROM tags WHERE id = ?")
        .bind(uuid::Uuid::parse_str(&tag_id).expect("uuid"))
        .fetch_one(&state.db)
        .await
        .expect("read the row");
    assert_eq!(name_in_db, new_name);
}

#[tokio::test]
async fn the_mcp_tool_leaves_the_color_alone_when_it_is_not_given() {
    let state = state().await;
    let name = format!("tag-{}", uuid::Uuid::new_v4());

    let Json(created) = handle_tag_create(
        &state,
        &TagCreateInput {
            name,
            color: Some("#00ff00".into()),
        },
    )
    .await
    .expect("create the tag");
    let tag_id = created["data"]["id"].as_str().expect("id").to_string();

    let renamed = format!("renamed-only-{}", uuid::Uuid::new_v4());
    let Json(updated) = handle_tag_update(
        &state,
        &TagUpdateInput {
            tag_id,
            name: Some(renamed.clone()),
            color: None,
        },
    )
    .await
    .expect("rename the tag");

    assert_eq!(updated["data"]["name"].as_str(), Some(renamed.as_str()));
    assert_eq!(
        updated["data"]["color"].as_str(),
        Some("#00ff00"),
        "a colour that was not sent must be left alone"
    );
}
