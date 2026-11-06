use rmcp::{schemars::JsonSchema, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuAuthStatusOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSendMessageInput {
    pub peer: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSendMessageOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuListDialogsInput {
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuListDialogsOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuListContactsInput {
    pub query: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuListContactsOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSearchInput {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSearchOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuRequestCodeInput {
    pub phone: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuRequestCodeOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSignInInput {
    pub phone: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuSignInOutput {
    pub data: serde_json::Value,
}

/// The Telegram user client, or a message that says what is missing.
///
/// The manager always exists now (it can be UNCONFIGURED rather than
/// absent), so the check is "does it have credentials?" rather than "is
/// it there?". The message names the two variables and the page they come
/// from, because an agent driving this tool can act on it.
fn client_manager(
    state: &AppState,
) -> Result<&std::sync::Arc<crate::services::telegram_client::TelegramClientManager>, String> {
    let mgr = state.telegram_client_manager.as_ref().ok_or_else(|| {
        "Telegram user client not available in this process. Set TELEGRAM_API_ID and \
         TELEGRAM_API_HASH."
            .to_string()
    })?;
    if mgr.credentials().is_none() {
        return Err(
            "Telegram user client has no API credentials. Set TELEGRAM_API_ID and \
             TELEGRAM_API_HASH (from https://my.telegram.org) in ~/.switchpost/.env, or POST them \
             to /api/integrations/connect/telegram-user/credentials."
                .to_string(),
        );
    }
    Ok(mgr)
}

pub async fn handle_tu_auth_status(state: &AppState) -> Result<Json<TuAuthStatusOutput>, String> {
    let mgr = client_manager(state)?;
    let is_auth = mgr.is_authenticated().await?;
    let user = if is_auth {
        mgr.user_info().await.ok()
    } else {
        None
    };
    Ok(Json(TuAuthStatusOutput {
        data: serde_json::json!({
            "authenticated": is_auth,
            "user": user,
        }),
    }))
}

pub async fn handle_tu_send_message(
    state: &AppState,
    input: &TuSendMessageInput,
) -> Result<Json<TuSendMessageOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.send_message(&input.peer, &input.text).await?;
    Ok(Json(TuSendMessageOutput { data: result }))
}

pub async fn handle_tu_list_dialogs(
    state: &AppState,
    _input: &TuListDialogsInput,
) -> Result<Json<TuListDialogsOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.list_dialogs().await?;
    Ok(Json(TuListDialogsOutput { data: result }))
}

pub async fn handle_tu_list_contacts(
    state: &AppState,
    _input: &TuListContactsInput,
) -> Result<Json<TuListContactsOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.list_contacts().await?;
    Ok(Json(TuListContactsOutput { data: result }))
}

pub async fn handle_tu_search(
    state: &AppState,
    input: &TuSearchInput,
) -> Result<Json<TuSearchOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.search(&input.query).await?;
    Ok(Json(TuSearchOutput { data: result }))
}

pub async fn handle_tu_request_code(
    state: &AppState,
    input: &TuRequestCodeInput,
) -> Result<Json<TuRequestCodeOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.request_login_code(&input.phone).await?;
    Ok(Json(TuRequestCodeOutput { data: result }))
}

pub async fn handle_tu_sign_in(
    state: &AppState,
    input: &TuSignInInput,
) -> Result<Json<TuSignInOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.sign_in(&input.phone, &input.code).await?;
    Ok(Json(TuSignInOutput { data: result }))
}

// ── P6.4: user-session parity (tg-cli port) ──────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuHistoryInput {
    /// Username or "user:<id>"
    pub peer: String,
    pub limit: Option<u32>,
    /// Pagination cursor: the oldest_id of the previous page (continue
    /// back in time).
    pub offset_id: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuHistoryOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuTailInput {
    /// Username or "user:<id>"
    pub peer: String,
    pub limit: Option<u32>,
    /// Only messages newer than this id (the previous call's newest_id).
    pub after_id: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuTailOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuEditMessageInput {
    /// Username or "user:<id>"
    pub peer: String,
    pub message_id: i32,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuEditMessageOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuDeleteMessageInput {
    /// Username or "user:<id>"
    pub peer: String,
    pub message_id: i32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuDeleteMessageOutput {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuQueueSendInput {
    /// Username or "user:<id>"
    pub peer: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TuQueueSendOutput {
    pub data: serde_json::Value,
}

pub async fn handle_tu_history(
    state: &AppState,
    input: &TuHistoryInput,
) -> Result<Json<TuHistoryOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr
        .fetch_history(&input.peer, input.limit.unwrap_or(50), input.offset_id)
        .await?;
    Ok(Json(TuHistoryOutput { data: result }))
}

pub async fn handle_tu_tail(
    state: &AppState,
    input: &TuTailInput,
) -> Result<Json<TuTailOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr
        .tail_messages(&input.peer, input.limit.unwrap_or(25), input.after_id)
        .await?;
    Ok(Json(TuTailOutput { data: result }))
}

pub async fn handle_tu_edit_message(
    state: &AppState,
    input: &TuEditMessageInput,
) -> Result<Json<TuEditMessageOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr
        .edit_message(&input.peer, input.message_id, &input.text)
        .await?;
    Ok(Json(TuEditMessageOutput { data: result }))
}

pub async fn handle_tu_delete_message(
    state: &AppState,
    input: &TuDeleteMessageInput,
) -> Result<Json<TuDeleteMessageOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.delete_message(&input.peer, input.message_id).await?;
    Ok(Json(TuDeleteMessageOutput { data: result }))
}

pub async fn handle_tu_queue_send(
    state: &AppState,
    input: &TuQueueSendInput,
) -> Result<Json<TuQueueSendOutput>, String> {
    let mgr = client_manager(state)?;
    let result = mgr.queue_send(&input.peer, &input.text).await?;
    Ok(Json(TuQueueSendOutput { data: result }))
}

pub async fn handle_tu_queue_status(state: &AppState) -> Result<Json<TuHistoryOutput>, String> {
    let mgr = client_manager(state)?;
    let len = mgr.queue_len().await;
    Ok(Json(TuHistoryOutput {
        data: serde_json::json!({ "queue_len": len }),
    }))
}
