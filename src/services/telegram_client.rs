use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use grammers_client::types::{LoginToken, PasswordToken};
use grammers_client::{Client, Config, InitParams, SignInError};
use grammers_session::Session;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::services::telegram_rate::{Category, SendJob, WriteQueue, TELEGRAM_RATE_GUARD};

/// The single user-session account key for the rate guard.
const ACCOUNT: &str = "telegram-user";

struct InnerState {
    client: Option<Client>,
    pending_token: Option<LoginToken>,
    pending_password_token: Option<PasswordToken>,
}

pub struct TelegramClientManager {
    inner: Mutex<InnerState>,
    api_id: i32,
    api_hash: String,
    session_path: PathBuf,
    queue: Mutex<WriteQueue>,
    worker_running: AtomicBool,
}

impl TelegramClientManager {
    pub fn new(api_id: i32, api_hash: String, session_dir: PathBuf) -> Self {
        let session_path = session_dir.join("telegram-user.session");
        Self {
            inner: Mutex::new(InnerState {
                client: None,
                pending_token: None,
                pending_password_token: None,
            }),
            api_id,
            api_hash,
            session_path,
            queue: Mutex::new(WriteQueue::default()),
            worker_running: AtomicBool::new(false),
        }
    }

    pub async fn is_authenticated(&self) -> Result<bool, String> {
        let mut guard = self.inner.lock().await;
        Self::ensure_client(&mut guard, self.api_id, &self.api_hash, &self.session_path).await?;
        match guard.client.as_ref() {
            Some(client) => client
                .is_authorized()
                .await
                .map_err(|e| format!("Auth check failed: {e}")),
            None => Ok(false),
        }
    }

    pub async fn request_login_code(&self, phone: &str) -> Result<Value, String> {
        let mut guard = self.inner.lock().await;
        Self::ensure_client(&mut guard, self.api_id, &self.api_hash, &self.session_path).await?;

        let client = guard
            .client
            .as_ref()
            .ok_or("Telegram client not initialised")?;
        match client.request_login_code(phone).await {
            Ok(token) => {
                guard.pending_token = Some(token);
                Ok(serde_json::json!({ "status": "code_sent" }))
            }
            Err(e) => {
                let err_str = format!("{e}");
                if err_str.contains("AUTH_RESTART") {
                    // Session corrupted — clear and retry with fresh session
                    tracing::warn!("AUTH_RESTART: clearing Telegram session and retrying");
                    guard.client = None;
                    guard.pending_token = None;
                    let _ = std::fs::remove_file(&self.session_path);
                    Self::ensure_client(
                        &mut guard,
                        self.api_id,
                        &self.api_hash,
                        &self.session_path,
                    )
                    .await?;
                    let client = guard
                        .client
                        .as_ref()
                        .ok_or("Telegram client not initialised")?;
                    let token = client.request_login_code(phone).await.map_err(|e| {
                        format!("Request login code failed after session reset: {e}")
                    })?;
                    guard.pending_token = Some(token);
                    Ok(serde_json::json!({ "status": "code_sent", "note": "session was reset" }))
                } else {
                    Err(format!("Request login code failed: {e}"))
                }
            }
        }
    }

    pub async fn sign_in(&self, _phone: &str, code: &str) -> Result<Value, String> {
        let mut guard = self.inner.lock().await;
        Self::ensure_client(&mut guard, self.api_id, &self.api_hash, &self.session_path).await?;

        let token = guard
            .pending_token
            .take()
            .ok_or_else(|| "No pending login token. Call request_login_code first.".to_string())?;
        let client = guard
            .client
            .as_ref()
            .ok_or("Telegram client not initialised")?;

        match client.sign_in(&token, code).await {
            Ok(user) => {
                let _ = client.session().save_to_file(&self.session_path);
                Ok(serde_json::json!({
                    "id": user.id(),
                    "first_name": user.first_name(),
                    "username": user.username(),
                }))
            }
            Err(SignInError::PasswordRequired(pw_token)) => {
                guard.pending_password_token = Some(pw_token);
                Err("2FA required. Use check_password to provide account password.".to_string())
            }
            Err(e) => Err(format!("Sign in failed: {e}")),
        }
    }

    pub async fn check_password(&self, password: &str) -> Result<Value, String> {
        let mut guard = self.inner.lock().await;
        let pw_token = guard
            .pending_password_token
            .take()
            .ok_or_else(|| "No pending 2FA.".to_string())?;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Client not connected.".to_string())?;

        let user = client
            .check_password(pw_token, password)
            .await
            .map_err(|e| format!("Check password failed: {e}"))?;

        let _ = client.session().save_to_file(&self.session_path);
        Ok(serde_json::json!({
            "id": user.id(),
            "first_name": user.first_name(),
            "username": user.username(),
        }))
    }

    pub async fn send_message(&self, peer: &str, text: &str) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let chat = client
            .resolve_username(peer)
            .await
            .map_err(|e| format!("Resolve username failed: {e}"))?
            .ok_or_else(|| format!("Could not resolve username: {peer}"))?;

        let msg = client
            .send_message(chat, text)
            .await
            .map_err(|e| format!("Send message failed: {e}"))?;

        Ok(serde_json::json!({
            "id": msg.id(),
            "date": msg.date().to_string(),
        }))
    }

    pub async fn list_dialogs(&self) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let mut dialogs: Vec<Value> = Vec::new();
        let mut iter = client.iter_dialogs();
        while let Some(dialog) = iter
            .next()
            .await
            .map_err(|e| format!("Dialog error: {e}"))?
        {
            let chat = dialog.chat();
            dialogs.push(serde_json::json!({
                "id": chat.id(),
                "name": chat.name(),
            }));
        }

        Ok(serde_json::json!({ "dialogs": dialogs }))
    }

    pub async fn list_dialogs_detailed(&self) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let mut dialogs: Vec<Value> = Vec::new();
        let mut iter = client.iter_dialogs();
        while let Some(dialog) = iter
            .next()
            .await
            .map_err(|e| format!("Dialog error: {e}"))?
        {
            let chat = dialog.chat();
            let chat_id = chat.id();
            // Telegram ID conventions: positive = user/peer,
            // negative with -100 prefix = channel/supergroup,
            // negative without prefix = group
            let target_type = if chat_id < -1_000_000_000_000i64 {
                "channel"
            } else if chat_id < 0 {
                "group"
            } else {
                "peer"
            };
            dialogs.push(serde_json::json!({
                "id": chat_id,
                "name": chat.name(),
                "type": target_type,
            }));
        }

        Ok(serde_json::json!({ "dialogs": dialogs }))
    }

    pub async fn list_contacts(&self) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let mut contacts: Vec<Value> = Vec::new();
        let mut iter = client.iter_dialogs();
        while let Some(dialog) = iter
            .next()
            .await
            .map_err(|e| format!("Dialog error: {e}"))?
        {
            let chat = dialog.chat();
            contacts.push(serde_json::json!({
                "id": chat.id(),
                "name": chat.name(),
            }));
        }

        Ok(serde_json::json!({ "contacts": contacts }))
    }

    pub async fn search(&self, query: &str) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let mut results: Vec<Value> = Vec::new();
        let q = query.to_lowercase();
        let mut iter = client.iter_dialogs();
        while let Some(dialog) = iter
            .next()
            .await
            .map_err(|e| format!("Dialog error: {e}"))?
        {
            let chat = dialog.chat();
            if chat.name().to_lowercase().contains(&q) {
                results.push(serde_json::json!({
                    "id": chat.id(),
                    "name": chat.name(),
                    "type": "dialog",
                }));
            }
        }

        Ok(serde_json::json!({ "results": results }))
    }

    // ── P6.4: read-side (guarded) ──────────────────────────────

    /// Gate a guarded call; Err carries the retry-after for the caller.
    fn check_gate(category: Category, peer: Option<&str>) -> Result<(), String> {
        let wait = TELEGRAM_RATE_GUARD.before_call(ACCOUNT, category, peer);
        if !wait.is_zero() {
            return Err(format!(
                "Telegram rate limit: retry in {:.0}s (category {category:?})",
                wait.as_secs_f64()
            ));
        }
        Ok(())
    }

    /// Resolve a peer string to a chat. `me` is the session's own account
    /// (Saved Messages), so a live roundtrip needs no public @username.
    /// Grammers 0.7 otherwise resolves usernames only — there is no raw-id
    /// → chat path — so "user:<id>" strings are the gate's peer keys, not
    /// resolvable targets (clear error, not a silent wrong chat).
    async fn resolve_chat(
        &self,
        client: &Client,
        peer: &str,
    ) -> Result<grammers_client::types::Chat, String> {
        if peer.eq_ignore_ascii_case("me") {
            let me = client
                .get_me()
                .await
                .map_err(|e| format!("get_me failed: {e}"))?;
            return Ok(grammers_client::types::Chat::User(me));
        }
        client
            .resolve_username(peer.trim_start_matches('@'))
            .await
            .map_err(|e| format!("Resolve username failed: {e}"))?
            .ok_or_else(|| format!("Could not resolve username: {peer}"))
    }

    /// Fetch a chat's message history, chronologically (oldest first).
    /// `offset_id` pages against iter_messages' newest-first order: pass
    /// the OLDEST message id of the previous page to continue back in time.
    pub async fn fetch_history(
        &self,
        peer: &str,
        limit: u32,
        offset_id: Option<i32>,
    ) -> Result<Value, String> {
        Self::check_gate(Category::History, None)?;
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let chat = self.resolve_chat(client, peer).await?;
        let mut messages: Vec<Value> = Vec::new();
        let mut iter = client.iter_messages(&chat).limit(limit as usize);
        if let Some(offset) = offset_id {
            iter = iter.offset_id(offset);
        }
        while let Some(msg) = iter
            .next()
            .await
            .map_err(|e| format!("History error: {e}"))?
        {
            messages.push(serde_json::json!({
                "id": msg.id(),
                "date": msg.date().to_string(),
                "text": msg.text(),
            }));
        }
        TELEGRAM_RATE_GUARD.record_success(ACCOUNT, Category::History);
        // grammers yields newest-first; return chronological.
        messages.reverse();
        let oldest = messages.first().map(|m| m["id"].clone());
        Ok(serde_json::json!({
            "peer": peer,
            "count": messages.len(),
            "messages": messages,
            "oldest_id": oldest,
        }))
    }

    /// Tail: latest messages, optionally only those NEWER than `after_id`
    /// (the watch cursor). Returns the new newest_id for the next call.
    pub async fn tail_messages(
        &self,
        peer: &str,
        limit: u32,
        after_id: Option<i32>,
    ) -> Result<Value, String> {
        Self::check_gate(Category::History, None)?;
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let chat = self.resolve_chat(client, peer).await?;
        // Fetch newest-first, filter for ids above the cursor, then flip to
        // chronological. ponytail: over-fetch by 2x the limit — the id
        // filter is applied client-side; raise if tails routinely drop.
        let mut messages: Vec<Value> = Vec::new();
        let mut iter = client.iter_messages(&chat).limit((limit * 2) as usize);
        while let Some(msg) = iter.next().await.map_err(|e| format!("Tail error: {e}"))? {
            if let Some(after) = after_id {
                if msg.id() <= after {
                    continue;
                }
            }
            messages.push(serde_json::json!({
                "id": msg.id(),
                "date": msg.date().to_string(),
                "text": msg.text(),
            }));
            if messages.len() >= limit as usize {
                break;
            }
        }
        TELEGRAM_RATE_GUARD.record_success(ACCOUNT, Category::History);
        messages.reverse();
        let newest = messages.last().map(|m| m["id"].clone());
        Ok(serde_json::json!({
            "peer": peer,
            "count": messages.len(),
            "messages": messages,
            "newest_id": newest,
        }))
    }

    /// Edit a message you sent.
    pub async fn edit_message(
        &self,
        peer: &str,
        message_id: i32,
        text: &str,
    ) -> Result<Value, String> {
        Self::check_gate(Category::Edit, None)?;
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let chat = self.resolve_chat(client, peer).await?;
        client
            .edit_message(&chat, message_id, text)
            .await
            .map_err(|e| {
                let estr = format!("{e}");
                if crate::services::telegram_rate::parse_flood_wait(&estr).is_some() {
                    TELEGRAM_RATE_GUARD.record_flood(
                        ACCOUNT,
                        Category::Edit,
                        std::time::Duration::from_secs(5),
                    );
                }
                format!("Edit failed: {e}")
            })?;
        TELEGRAM_RATE_GUARD.record_success(ACCOUNT, Category::Edit);
        Ok(serde_json::json!({ "edited": true, "id": message_id }))
    }

    /// Delete a message you sent (revoke for everyone where allowed).
    pub async fn delete_message(&self, peer: &str, message_id: i32) -> Result<Value, String> {
        Self::check_gate(Category::Delete, None)?;
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let chat = self.resolve_chat(client, peer).await?;
        let deleted = client
            .delete_messages(&chat, &[message_id])
            .await
            .map_err(|e| format!("Delete failed: {e}"))?;
        TELEGRAM_RATE_GUARD.record_success(ACCOUNT, Category::Delete);
        Ok(serde_json::json!({ "deleted": deleted, "id": message_id }))
    }

    // ── P6.4: write queue ─────────────────────────────────────

    /// Enqueue a send through the rate-limited write queue. The worker
    /// drains jobs as the limiter allows; send_message stays available for
    /// direct calls that want an immediate result.
    ///
    /// # The ceiling this queue has (R1, measured not assumed)
    ///
    /// `WriteQueue` is an in-memory `VecDeque` behind a `Mutex`, so an
    /// undelivered job exists only in this process's heap. If the process
    /// exits - a restart, a panic, a `kill`, a crash - every job still in the
    /// queue is gone, and the caller's "queued: true" response has already
    /// told them it was accepted. The message is neither sent nor reported
    /// failed: it is silently lost.
    ///
    /// The window is narrow but real. A job is at risk only while it waits in
    /// the queue, so the loss is bounded by whatever the rate gate is holding,
    /// which is the limiter's decision, not the founder's message. Once the
    /// worker moves a job to the head and the send returns, the job is out of
    /// the queue and into the outcome path (`Ok` pops it, a transient error
    /// keeps it for a retry with `MAX_HEAD_RETRIES`, a permanent error drops
    /// it and logs at ERROR). The risk is not a failed send - those are
    /// reported - it is the queue never reaching the send.
    ///
    /// # What was chosen, and what was not
    ///
    /// The register offers two shapes (R1): document this ceiling (S), or
    /// persist a `telegram_queue` table (M). Persisting is deliberately NOT
    /// done, on the register's own condition - persist only if V1b (a live
    /// `live_telegram_test` roundtrip) shows real payloads being lost. V1b is
    /// DEFERRED by the user pending credentials, so the evidence for the M
    /// shape does not exist. An M-shaped fix built before that evidence is a
    /// table whose only reader is a path nobody has observed failing.
    ///
    /// # ponytail: the deliberate shortcut
    ///
    /// The queue is not persisted, so an undelivered job dies with the process
    /// and its "queued: true" reply is the last anyone hears of it. Revisit
    /// with a `telegram_queue` table when V1b produces a real loss, and until
    /// then treat `queued` as "accepted for this process's lifetime", which
    /// is what it means.
    pub async fn queue_send(self: &Arc<Self>, peer: &str, text: &str) -> Result<Value, String> {
        // Fail fast when unauthenticated — the queue is not an error sink.
        if !self.is_authenticated().await? {
            return Err("Telegram user session not authenticated.".to_string());
        }
        let position = {
            let mut q = self.queue.lock().await;
            q.push(SendJob {
                peer: peer.to_string(),
                text: text.to_string(),
            })
        };
        self.ensure_worker();
        Ok(serde_json::json!({
            "queued": true,
            "position": position,
            "queue_len": position,
        }))
    }

    pub async fn queue_len(&self) -> usize {
        self.queue.lock().await.len()
    }

    fn ensure_worker(self: &Arc<Self>) {
        if self.worker_running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let mut q = mgr.queue.lock().await;
                q.drain_with(&TELEGRAM_RATE_GUARD, ACCOUNT, |job| {
                    // The future must own its data — it cannot borrow the
                    // &SendJob (its lifetime ends with the closure call).
                    let peer = job.peer.clone();
                    let text = job.text.clone();
                    let mgr = Arc::clone(&mgr);
                    async move { mgr.send_message(&peer, &text).await.map(|_| ()) }
                })
                .await;
            }
        });
    }

    pub async fn user_info(&self) -> Result<Value, String> {
        let guard = self.inner.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| "Telegram client not connected.".to_string())?;

        let me = client
            .get_me()
            .await
            .map_err(|e| format!("Get me failed: {e}"))?;

        Ok(serde_json::json!({
            "id": me.id(),
            "first_name": me.first_name(),
            "username": me.username(),
        }))
    }

    async fn ensure_client(
        guard: &mut InnerState,
        api_id: i32,
        api_hash: &str,
        session_path: &PathBuf,
    ) -> Result<(), String> {
        if guard.client.is_some() {
            return Ok(());
        }

        let parent = session_path.parent().ok_or_else(|| {
            format!(
                "Telegram session path has no parent directory: {}",
                session_path.display()
            )
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create session dir: {e}"))?;

        let session = Session::load_file_or_create(session_path)
            .map_err(|e| format!("Failed to load or create session: {e}"))?;

        let config = Config {
            session,
            api_id,
            api_hash: api_hash.to_string(),
            params: InitParams::default(),
        };

        let client = Client::connect(config)
            .await
            .map_err(|e| format!("Failed to connect to Telegram: {e}"))?;

        guard.client = Some(client);
        Ok(())
    }
}

pub type OptionalTelegramClient = Option<Arc<TelegramClientManager>>;
