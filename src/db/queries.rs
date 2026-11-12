// ─── Database Queries ─────────────────────────────────────────
// Typed SQL query functions using sqlx.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::models::*;
use super::types::EpochUtc;
use super::SqlitePool;

// ══════════════════════════════════════════════════════════════
// USERS
// ══════════════════════════════════════════════════════════════

pub async fn create_user(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
    name: &str,
) -> Result<User, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query_as!(
        User,
        r#"INSERT INTO users (id, email, password, name) VALUES (?, ?, ?, ?)
         RETURNING id as "id!: Uuid", email, password, name,
                   timezone as "timezone!: i32",
                   created_at as "created_at!: DateTime<Utc>",
                   updated_at as "updated_at!: DateTime<Utc>""#,
        id,
        email,
        password_hash,
        name,
    )
    .fetch_one(pool)
    .await
}

pub async fn create_user_with_id(
    pool: &SqlitePool,
    id: Uuid,
    email: &str,
    password_hash: &str,
    name: &str,
) -> Result<User, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"INSERT INTO users (id, email, password, name) VALUES (?, ?, ?, ?)
         ON CONFLICT (email) DO UPDATE SET id = EXCLUDED.id, password = EXCLUDED.password, name = EXCLUDED.name
         RETURNING id as "id!: Uuid", email, password, name,
                   timezone as "timezone!: i32",
                   created_at as "created_at!: DateTime<Utc>",
                   updated_at as "updated_at!: DateTime<Utc>""#,
        id,
        email,
        password_hash,
        name,
    )
    .fetch_one(pool)
    .await
}

pub async fn get_user_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id as "id!: Uuid", email, password, name,
                  timezone as "timezone!: i32",
                  created_at as "created_at!: DateTime<Utc>",
                  updated_at as "updated_at!: DateTime<Utc>"
         FROM users WHERE email = ?"#,
        email,
    )
    .fetch_optional(pool)
    .await
}

pub async fn get_user_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id as "id!: Uuid", email, password, name,
                  timezone as "timezone!: i32",
                  created_at as "created_at!: DateTime<Utc>",
                  updated_at as "updated_at!: DateTime<Utc>"
         FROM users WHERE id = ?"#,
        id,
    )
    .fetch_optional(pool)
    .await
}

// ══════════════════════════════════════════════════════════════
// INTEGRATIONS
// ══════════════════════════════════════════════════════════════

#[expect(
    clippy::too_many_arguments,
    reason = "OAuth provider fields: all persisted and required for upsert identity"
)]
pub async fn create_integration(
    pool: &SqlitePool,
    user_id: Uuid,
    provider_identifier: &str,
    provider_name: &str,
    internal_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    token_expires_at: Option<DateTime<Utc>>,
    profile_name: Option<&str>,
    profile_picture: Option<&str>,
    profile_url: Option<&str>,
    root_internal_id: Option<&str>,
    auth_method: Option<&str>,
    token_key: Option<[u8; 32]>,
) -> Result<Integration, TokenUpdateError> {
    // ── THE ONE ENCRYPTING WRITER FOR NEW INTEGRATIONS ───────
    //
    // Before this change, only the OAuth callback encrypted: every other
    // connect path (direct connect, public onboarding, the X cookie store,
    // the Telegram bot token, the page picker) handed a PLAINTEXT token to
    // this function, which wrote it straight into the row. With
    // TOKEN_ENCRYPTION_KEY set, a founder who connected a channel through
    // the browser had a working credential sitting in the clear in the
    // database file — and one `sqlite3 .dump` leaked every account.
    //
    // So the encryption lives HERE, in the writer, exactly like
    // `update_integration_token` does for refreshes. Callers pass
    // PLAINTEXT; this function encrypts; an encryption failure returns
    // Err and NEVER falls back to plaintext (AGENTS §6.3).
    let method = auth_method.unwrap_or("oauth");
    let id = Uuid::new_v4();
    let store_access = match token_key {
        Some(k) => crate::crypto::encrypt_string(access_token, &k).map_err(|reason| {
            TokenUpdateError::Encryption {
                integration_id: id,
                reason,
            }
        })?,
        None => access_token.to_string(),
    };
    let store_refresh = match (token_key, refresh_token) {
        (Some(k), Some(rt)) => Some(crate::crypto::encrypt_string(rt, &k).map_err(|reason| {
            TokenUpdateError::Encryption {
                integration_id: id,
                reason,
            }
        })?),
        (_, rt) => rt.map(str::to_string),
    };
    sqlx::query_as!(
        Integration,
        r#"INSERT INTO integrations
           (id, user_id, provider_identifier, provider_name, internal_id,
             access_token, refresh_token, token_expires_at,
             profile_name, profile_picture, profile_url,
             root_internal_id, auth_method)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT (user_id, provider_identifier, internal_id)
           DO UPDATE SET access_token = ?, refresh_token = ?,
             token_expires_at = ?, profile_name = ?,
             profile_picture = COALESCE(?, integrations.profile_picture),
             profile_url = ?,
             root_internal_id = COALESCE(?, integrations.root_internal_id),
             auth_method = ?,
             refresh_needed = false, disabled = false,
             updated_at = unixepoch()
           RETURNING id as "id!: Uuid", user_id as "user_id!: Uuid",
             provider_identifier, provider_name, internal_id,
             access_token, refresh_token,
             token_expires_at as "token_expires_at?: DateTime<Utc>",
             profile_name, profile_picture, profile_url,
             disabled as "disabled!: bool",
             refresh_needed as "refresh_needed!: bool",
             root_internal_id, posting_times, auth_method,
             created_at as "created_at!: DateTime<Utc>",
             updated_at as "updated_at!: DateTime<Utc>""#,
        id,
        user_id,
        provider_identifier,
        provider_name,
        internal_id,
        store_access,
        store_refresh,
        token_expires_at,
        profile_name,
        profile_picture,
        profile_url,
        root_internal_id,
        method,
        // SQLite has no numbered params: the ON CONFLICT DO UPDATE arm
        // repeats the tail binds positionally (13..20).
        store_access,
        store_refresh,
        token_expires_at,
        profile_name,
        profile_picture,
        profile_url,
        root_internal_id,
        method,
    )
    .fetch_one(pool)
    .await
    .map_err(TokenUpdateError::from)
}

/// Re-encrypt integration tokens that were stored in PLAINTEXT.
///
/// The encrypting writer landed in `create_integration`; rows written
/// BEFORE it existed are still plaintext. `maybe_decrypt_token` falls
/// back to the raw value for those, so the app keeps working — but the
/// file on disk is still the leak this closes.
///
/// The pass is conservative on purpose:
///
/// - a value that decrypts with THIS key is already encrypted → skipped;
/// - a value that does not decrypt but LOOKS encrypted (hex, long enough)
///   was written with a different key → left alone, because encrypting it
///   again would destroy it;
/// - anything else is plaintext → encrypted now.
///
/// Idempotent: a second run finds nothing to do.
pub async fn reencrypt_plaintext_tokens(
    pool: &SqlitePool,
    token_key: [u8; 32],
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        "SELECT id, access_token, refresh_token FROM integrations",
    )
    .fetch_all(pool)
    .await?;

    let mut changed = 0usize;
    for (id, access, refresh) in rows {
        let access_is_plain = !crate::crypto::is_encrypted_with(&access, &token_key)
            && !crate::crypto::looks_encrypted(&access);
        let refresh_is_plain = match &refresh {
            Some(rt) => {
                !crate::crypto::is_encrypted_with(rt, &token_key)
                    && !crate::crypto::looks_encrypted(rt)
            }
            None => false,
        };
        if !access_is_plain && !refresh_is_plain {
            continue;
        }
        let new_access = if access_is_plain {
            crate::crypto::encrypt_string(&access, &token_key)
                .map_err(|e| sqlx::Error::Protocol(format!("token re-encryption failed: {e}")))?
        } else {
            access.clone()
        };
        let new_refresh = match (&refresh, refresh_is_plain) {
            (Some(rt), true) => {
                Some(crate::crypto::encrypt_string(rt, &token_key).map_err(|e| {
                    sqlx::Error::Protocol(format!("token re-encryption failed: {e}"))
                })?)
            }
            (other, _) => other.clone(),
        };
        sqlx::query("UPDATE integrations SET access_token = ?, refresh_token = ? WHERE id = ?")
            .bind(&new_access)
            .bind(&new_refresh)
            .bind(id)
            .execute(pool)
            .await?;
        changed += 1;
    }
    Ok(changed)
}

// ══════════════════════════════════════════════════════════════
// X QUERY IDS (F4)
// ══════════════════════════════════════════════════════════════

/// All discovered X GraphQL queryIds with their fetch time (epoch seconds).
pub async fn list_x_query_ids(
    pool: &SqlitePool,
) -> Result<Vec<(String, String, i64)>, sqlx::Error> {
    let rows = sqlx::query!(r#"SELECT op_name, query_id, fetched_at FROM x_query_ids"#)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.op_name, r.query_id, r.fetched_at))
        .collect())
}

/// Upsert one discovered queryId. `fetched_at` is epoch seconds.
pub async fn upsert_x_query_id(
    pool: &SqlitePool,
    op_name: &str,
    query_id: &str,
    fetched_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO x_query_ids (op_name, query_id, fetched_at) VALUES (?, ?, ?)
         ON CONFLICT(op_name) DO UPDATE SET
           query_id = excluded.query_id,
           fetched_at = excluded.fetched_at"#,
        op_name,
        query_id,
        fetched_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Enable or disable one integration. Returns `false` when no row matched.
pub async fn set_integration_disabled(
    pool: &SqlitePool,
    user_id: Uuid,
    id: Uuid,
    disabled: bool,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE integrations SET disabled = ?, updated_at = unixepoch() WHERE id = ? AND user_id = ?",
    )
    .bind(disabled)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn list_integrations(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<Vec<Integration>, sqlx::Error> {
    sqlx::query_as!(
        Integration,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                provider_identifier, provider_name, internal_id,
                access_token, refresh_token,
                token_expires_at as "token_expires_at?: DateTime<Utc>",
                profile_name, profile_picture, profile_url,
                disabled as "disabled!: bool",
                refresh_needed as "refresh_needed!: bool",
                root_internal_id, posting_times, auth_method,
                created_at as "created_at!: DateTime<Utc>",
                updated_at as "updated_at!: DateTime<Utc>"
         FROM integrations WHERE user_id = ? ORDER BY created_at DESC"#,
        user_id,
    )
    .fetch_all(pool)
    .await
}

/// List all non-disabled integrations across all users
pub async fn list_all_integrations_across_users(
    pool: &SqlitePool,
) -> Result<Vec<Integration>, sqlx::Error> {
    sqlx::query_as!(
        Integration,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                provider_identifier, provider_name, internal_id,
                access_token, refresh_token,
                token_expires_at as "token_expires_at?: DateTime<Utc>",
                profile_name, profile_picture, profile_url,
                disabled as "disabled!: bool",
                refresh_needed as "refresh_needed!: bool",
                root_internal_id, posting_times, auth_method,
                created_at as "created_at!: DateTime<Utc>",
                updated_at as "updated_at!: DateTime<Utc>"
         FROM integrations WHERE disabled = false ORDER BY user_id, provider_identifier"#,
    )
    .fetch_all(pool)
    .await
}

pub async fn get_integration(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<Integration>, sqlx::Error> {
    sqlx::query_as!(
        Integration,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                provider_identifier, provider_name, internal_id,
                access_token, refresh_token,
                token_expires_at as "token_expires_at?: DateTime<Utc>",
                profile_name, profile_picture, profile_url,
                disabled as "disabled!: bool",
                refresh_needed as "refresh_needed!: bool",
                root_internal_id, posting_times, auth_method,
                created_at as "created_at!: DateTime<Utc>",
                updated_at as "updated_at!: DateTime<Utc>"
         FROM integrations WHERE id = ? AND user_id = ?"#,
        id,
        user_id,
    )
    .fetch_optional(pool)
    .await
}

/// Error surfaced by [`update_integration_token`]: a failed DB write, or a
/// failed encryption — which MUST abort the store instead of downgrading
/// to plaintext (audit B8).
#[derive(Debug, thiserror::Error)]
pub enum TokenUpdateError {
    #[error("token encryption failed for integration {integration_id}: {reason}")]
    Encryption {
        integration_id: Uuid,
        reason: String,
    },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// B3: the `in_reply_to` predecessor for a thread part — the latest
///   published part with `sequence - 1` in the SAME group AND the SAME
///   integration.
///
/// `group_id` is shared across every integration of a multi-platform
/// thread (`create_thread_posts`), so scoping to `integration_id` is
/// what stops part 2 on X from replying to part 1's LinkedIn URN.
/// Runtime form: inline in two call sites pre-extraction, and the
/// `?`-bound UUIDs keep it cache-free.
pub async fn find_thread_predecessor(
    pool: &SqlitePool,
    group_id: Uuid,
    integration_id: Uuid,
    sequence: i32,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"SELECT platform_post_id FROM posts
           WHERE group_id = ? AND integration_id = ? AND sequence = ?
             AND state = 'published' AND platform_post_id IS NOT NULL
           LIMIT 1"#,
    )
    .bind(group_id)
    .bind(integration_id)
    .bind(sequence)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(pid,)| pid))
}

/// The recently published posts of an integration, with their platform
/// post ids.
///
/// The analytics refresher needs this to refresh the PER-POST cache
/// rows. The per-post rows are the ones the analytics page, the
/// research page and the campaign engagement read - and they were
/// only ever written when a human opened the Post Stats modal. The
/// refresher wrote the ACCOUNT-level rows, and the cleaner deleted
/// the expired per-post rows every 30 minutes. So a founder's
/// "Likes 42" silently became "Likes 0" an hour later, with no
/// publish, no error and no refresh.
///
/// The window is the RECENT posts, because a per-post fetch is one
/// API call per post and a founder's whole archive is not free.
pub async fn recent_published_posts(
    pool: &SqlitePool,
    integration_id: Uuid,
    limit: i64,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        r#"SELECT id, platform_post_id FROM posts
           WHERE integration_id = ?
             AND state = 'published'
             AND platform_post_id IS NOT NULL
             AND deleted_at IS NULL
           ORDER BY published_at DESC
           LIMIT ?"#,
    )
    .bind(integration_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The error message of a thread predecessor that ended in `error`, if any.
/// A part whose predecessor failed can never be published, so the scheduler
/// fails it instead of requeuing it forever.
pub async fn find_failed_thread_predecessor(
    pool: &SqlitePool,
    group_id: Uuid,
    integration_id: Uuid,
    sequence: i32,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"SELECT error_message FROM posts
           WHERE group_id = ? AND integration_id = ? AND sequence = ?
             AND state = 'error' AND deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(group_id)
    .bind(integration_id)
    .bind(sequence)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(msg,)| msg.unwrap_or_else(|| "unknown error".into())))
}

/// Persist a (possibly rotated) OAuth token pair for an integration.
///
/// Single encrypting writer (audit B8): callers pass PLAINTEXT tokens.
/// - When `token_key` is set, the access token and any rotated refresh
///   token are AES-256-GCM encrypted HERE; an encryption failure returns
///   `Err` — never a silent plaintext downgrade.
/// - `refresh_token = COALESCE(?, refresh_token)`: providers that do not
///   rotate refresh tokens (Reddit's are permanent) pass `None` and the
///   previously stored token survives instead of being wiped.
pub async fn update_integration_token(
    pool: &SqlitePool,
    id: Uuid,
    access_token: &str,
    refresh_token: Option<&str>,
    token_expires_at: Option<DateTime<Utc>>,
    token_key: Option<[u8; 32]>,
) -> Result<(), TokenUpdateError> {
    let store_access = match token_key {
        Some(k) => crate::crypto::encrypt_string(access_token, &k).map_err(|reason| {
            TokenUpdateError::Encryption {
                integration_id: id,
                reason,
            }
        })?,
        None => access_token.to_string(),
    };
    let store_refresh = match (token_key, refresh_token) {
        (Some(k), Some(rt)) => Some(crate::crypto::encrypt_string(rt, &k).map_err(|reason| {
            TokenUpdateError::Encryption {
                integration_id: id,
                reason,
            }
        })?),
        (_, rt) => rt.map(str::to_string),
    };
    sqlx::query!(
        "UPDATE integrations SET access_token = ?, refresh_token = COALESCE(?, refresh_token),
         token_expires_at = ?, refresh_needed = false, updated_at = unixepoch()
         WHERE id = ?",
        store_access,
        store_refresh,
        token_expires_at,
        id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark an integration as needing reconnection (e.g. scope mismatch, revoked token).
pub async fn mark_integration_refresh_needed(
    pool: &SqlitePool,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE integrations SET refresh_needed = true, updated_at = unixepoch() WHERE id = ?",
        id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear the `refresh_needed` flag.
///
/// WHY THIS EXISTS
///
/// `mark_integration_refresh_needed` had no counterpart, so the flag was a
/// ONE-WAY door. Once a channel was marked, nothing in the codebase could
/// ever unmark it — not the successful refresh that proved the token works,
/// not a reconnect that installed a brand-new token, not the operator who
/// fixed the actual problem by hand. The 24h proactive refresh
/// (`get_integrations_needing_refresh`, below) filters `AND refresh_needed =
/// false`, so the channel was cut out of that too.
///
/// `update_integration_token` already clears it on a token write, and
/// `reconnect` goes through that — so a reconnect DOES recover a
/// channel. What was missing was the other direction: proof of health.
/// A refresh that succeeds and then fetches posts successfully is proof the
/// channel is fine, and it should clear the flag so the channel returns to
/// the normal lifecycle.
///
/// Idempotent and safe to call on a row whose flag is already false.
///
/// Runtime `sqlx::query` rather than the `query!` macro: this is a NEW
/// query and regenerating `.sqlx/` needs a live SQLite file plus
/// `cargo sqlx prepare` (AGENTS.md §5.1-5.3).
pub async fn clear_integration_refresh_needed(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE integrations SET refresh_needed = false, updated_at = unixepoch() WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get integrations that need proactive token refresh (tokens expiring within `window_hours`)
pub async fn get_integrations_needing_refresh(
    pool: &SqlitePool,
    provider_identifier: &str,
    window_hours: i64,
) -> Result<Vec<Integration>, sqlx::Error> {
    let cutoff_time = chrono::Utc::now() + chrono::Duration::hours(window_hours);
    sqlx::query_as!(
        Integration,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                provider_identifier, provider_name, internal_id,
                access_token, refresh_token,
                token_expires_at as "token_expires_at?: DateTime<Utc>",
                profile_name, profile_picture, profile_url,
                disabled as "disabled!: bool",
                refresh_needed as "refresh_needed!: bool",
                root_internal_id, posting_times, auth_method,
                created_at as "created_at!: DateTime<Utc>",
                updated_at as "updated_at!: DateTime<Utc>"
         FROM integrations
         WHERE provider_identifier = ?
           AND disabled = false
           AND refresh_needed = false
           AND refresh_token IS NOT NULL
           AND token_expires_at IS NOT NULL
           AND token_expires_at <= ?"#,
        provider_identifier,
        cutoff_time,
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_integration(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "DELETE FROM integrations WHERE id = ? AND user_id = ?",
        id,
        user_id,
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Delete a provider's SEED rows (the seed_demo.py fixtures: `seed-token`
/// access tokens or X's `{"auth_token":"seed"}` cookie blob) for a user.
/// Called when a REAL credential for the same provider lands, so the
/// channel list shows the real account and not the fictional seed one
/// beside it. Returns the number of seed rows removed.
pub async fn delete_seed_integrations(
    pool: &SqlitePool,
    user_id: Uuid,
    provider_identifier: &str,
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query!(
        "DELETE FROM integrations WHERE user_id = ? AND provider_identifier = ?\n         AND (access_token = 'seed-token' OR access_token LIKE '%\"seed\"%')",
        user_id,
        provider_identifier,
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

// ══════════════════════════════════════════════════════════════
// POSTS
// ══════════════════════════════════════════════════════════════

/// Batch-create posts for multiple integrations in a single transaction.
#[expect(
    clippy::too_many_arguments,
    reason = "Post composer fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn create_posts_for_integrations(
    pool: &SqlitePool,
    user_id: Uuid,
    integration_ids: &[Uuid],
    content: &str,
    title: Option<&str>,
    media: &serde_json::Value,
    settings: &serde_json::Value,
    scheduled_at: Option<DateTime<Utc>>,
    state: Option<PostState>,
    first_comment: Option<&str>,
    sequence: i32,
    campaign_id: Option<Uuid>,
) -> Result<Vec<Post>, sqlx::Error> {
    let st = state.unwrap_or(PostState::Draft);
    let mut tx = pool.begin().await?;
    let mut posts = Vec::with_capacity(integration_ids.len());

    for &integration_id in integration_ids {
        let post = sqlx::query_as::<_, Post>(
            r#"INSERT INTO posts
               (id, user_id, integration_id, content, title, media, settings, scheduled_at, state, first_comment, sequence, idempotency_key, campaign_id)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING id, user_id, integration_id, state,
                 content, title, media, settings, scheduled_at, published_at,
                 platform_post_id, platform_post_url, error_message,
                 created_at, updated_at,
                 repeat_interval_days, repeat_end_date, group_id,
                 first_comment, sequence, idempotency_key,
                 campaign_id, kanban_substate, due_date"#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(integration_id)
        .bind(content)
        .bind(title)
        .bind(media)
        .bind(settings)
        .bind(scheduled_at.map(EpochUtc::from))
        .bind(&st)
        .bind(first_comment)
        .bind(sequence)
        .bind(Uuid::new_v4())
        .bind(campaign_id)
        .fetch_one(&mut *tx)
        .await?;
        posts.push(post);
    }

    tx.commit().await?;
    Ok(posts)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Post composer fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn create_post(
    pool: &SqlitePool,
    user_id: Uuid,
    integration_id: Uuid,
    content: &str,
    title: Option<&str>,
    media: &serde_json::Value,
    settings: &serde_json::Value,
    scheduled_at: Option<DateTime<Utc>>,
    state: Option<PostState>,
    first_comment: Option<&str>,
    sequence: i32,
    campaign_id: Option<Uuid>,
) -> Result<Post, sqlx::Error> {
    let st = state.unwrap_or(PostState::Draft);
    // Runtime query (Phase v22 — idempotency_key column added, can't
    // regenerate .sqlx offline cache without a live Postgres).
    sqlx::query_as::<_, Post>(
        r#"INSERT INTO posts
           (id, user_id, integration_id, content, title, media, settings, scheduled_at, state, first_comment, sequence, idempotency_key, campaign_id)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           RETURNING id, user_id, integration_id, state,
              content, title, media, settings, scheduled_at, published_at,
              platform_post_id, platform_post_url, error_message,
              created_at, updated_at,
              repeat_interval_days, repeat_end_date, group_id,
              first_comment, sequence, idempotency_key,
              campaign_id, kanban_substate, due_date"#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(integration_id)
    .bind(content)
    .bind(title)
    .bind(media)
    .bind(settings)
    .bind(scheduled_at.map(EpochUtc::from))
    .bind(st)
    .bind(first_comment)
    .bind(sequence)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .fetch_one(pool)
    .await
}

/// One part of a thread/stage insert — everything the row needs, so the
/// thread path and the staging path share ONE insert statement (B4).
#[derive(Debug, Clone)]
pub struct ThreadPart {
    pub content: String,
    pub media: serde_json::Value,
    pub settings: serde_json::Value,
    pub first_comment: Option<String>,
}

impl ThreadPart {
    /// Plain text part (legacy create_thread_posts shape): no media, no
    /// first comment, settings carry only the delay marker.
    fn text(content: &str, delay_minutes: Option<i64>) -> Self {
        Self {
            content: content.to_string(),
            media: serde_json::Value::Null,
            settings: match delay_minutes {
                Some(d) => serde_json::json!({"delay_minutes": d}),
                None => serde_json::json!({}),
            },
            first_comment: None,
        }
    }
}

/// Insert thread parts for every integration inside ONE transaction (B4:
/// a mid-loop failure can no longer leave half the platforms staged).
/// Shared by create_thread_posts and the staging service.
#[expect(
    clippy::too_many_arguments,
    reason = "Thread insert fields: all bound to DB columns, no nesting grouping available"
)]
async fn insert_thread_posts_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: Uuid,
    integration_ids: &[Uuid],
    parts: &[ThreadPart],
    scheduled_at: Option<DateTime<Utc>>,
    state: PostState,
    group_id: Uuid,
    delay_minutes: Option<i64>,
) -> Result<Vec<Post>, sqlx::Error> {
    let mut posts = Vec::new();
    for (seq, part) in (1i32..).zip(parts.iter()) {
        // Apply per-part delay: each part's scheduled_at is offset by
        // (seq - 1) * delay_minutes. Part 1 publishes at scheduled_at,
        // part 2 at scheduled_at + delay, part 3 at scheduled_at + 2*delay,
        // etc. (B2: the delay lives ONLY here — the scheduler no longer
        // sleeps it a second time.)
        let part_scheduled_at = scheduled_at.map(|base| {
            if let Some(delay) = delay_minutes {
                if delay > 0 && seq > 1 {
                    base + chrono::Duration::minutes(delay * (seq as i64 - 1))
                } else {
                    base
                }
            } else {
                base
            }
        });

        for &integration_id in integration_ids {
            let post = sqlx::query_as::<_, Post>(
                r#"INSERT INTO posts
                   (id, user_id, integration_id, content, title, media, settings, scheduled_at, state, first_comment, sequence, group_id, idempotency_key)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   RETURNING id, user_id, integration_id, state,
                     content, title, media, settings, scheduled_at, published_at,
                     platform_post_id, platform_post_url, error_message,
                     created_at, updated_at,
                     repeat_interval_days, repeat_end_date, group_id,
                     first_comment, sequence, idempotency_key,
                     campaign_id, kanban_substate, due_date"#,
            )
            .bind(Uuid::new_v4())
            .bind(user_id)
            .bind(integration_id)
            .bind(&part.content)
            .bind(None::<&str>)
            .bind(&part.media)
            .bind(&part.settings)
            .bind(part_scheduled_at.map(EpochUtc::from))
            .bind(&state)
            .bind(part.first_comment.as_deref())
            .bind(seq)
            .bind(group_id)
            .bind(Uuid::new_v4())
            .fetch_one(&mut **tx)
            .await?;
            posts.push(post);
        }
    }
    Ok(posts)
}

/// Create thread posts (multiple content parts sharing a group_id)
#[expect(
    clippy::too_many_arguments,
    reason = "Thread post fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn create_thread_posts(
    pool: &SqlitePool,
    user_id: Uuid,
    integration_ids: &[Uuid],
    content_parts: &[String],
    scheduled_at: Option<DateTime<Utc>>,
    state: Option<PostState>,
    group_id: Uuid,
    delay_minutes: Option<i64>,
) -> Result<Vec<Post>, sqlx::Error> {
    let st = state.unwrap_or(PostState::Draft);
    let parts: Vec<ThreadPart> = content_parts
        .iter()
        .map(|p| ThreadPart::text(p, delay_minutes))
        .collect();
    let mut tx = pool.begin().await?;
    let posts = insert_thread_posts_tx(
        &mut tx,
        user_id,
        integration_ids,
        &parts,
        scheduled_at,
        st,
        group_id,
        delay_minutes,
    )
    .await?;
    tx.commit().await?;
    // §6.2: thread parts may be due sooner than the current sleep.
    crate::scheduler::wake();
    Ok(posts)
}

/// B4: staging service entry — per-integration content/settings/media inside
/// ONE transaction, all rows sharing the stage's campaign `group_id`.
pub async fn insert_stage_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: Uuid,
    integration_id: Uuid,
    parts: &[ThreadPart],
    scheduled_at: Option<DateTime<Utc>>,
    state: PostState,
    group_id: Uuid,
) -> Result<Vec<Post>, sqlx::Error> {
    // Stage splits carry no inter-part delay: the parts become due together
    // and the B3 predecessor gate publishes them in sequence order.
    insert_thread_posts_tx(
        tx,
        user_id,
        &[integration_id],
        parts,
        scheduled_at,
        state,
        group_id,
        None,
    )
    .await
}

/// Get all posts sharing a group_id (for thread display)
pub async fn get_posts_by_group_id(
    pool: &SqlitePool,
    user_id: Uuid,
    group_id: Uuid,
) -> Result<Vec<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"SELECT id, user_id, integration_id, state,
           content, title, media, settings, scheduled_at, published_at,
           platform_post_id, platform_post_url, error_message,
           created_at, updated_at,
           repeat_interval_days, repeat_end_date, group_id,
           first_comment, sequence, idempotency_key,
           campaign_id, kanban_substate, due_date
         FROM posts WHERE user_id = ? AND group_id = ?
         ORDER BY sequence ASC, created_at ASC"#,
    )
    .bind(user_id)
    .bind(group_id)
    .fetch_all(pool)
    .await
}

pub async fn list_posts(
    pool: &SqlitePool,
    user_id: Uuid,
    state_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Post>, sqlx::Error> {
    if let Some(st) = state_filter {
        let ps: PostState = match st {
            "idea" => PostState::Idea,
            "draft" => PostState::Draft,
            "queued" => PostState::Queued,
            "published" => PostState::Published,
            "error" | "errored" => PostState::Error,
            _ => return list_posts_all(pool, user_id, limit, offset).await,
        };
        // Runtime query (not query_as!) so we don't need to regenerate the
        // sqlx offline cache for this minor WHERE-clause change. The query
        // is identical to the cached version except for the added
        // `AND deleted_at IS NULL` filter.
        sqlx::query_as::<_, Post>(
            r#"SELECT id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id,
               kanban_sort_order, kanban_substate, due_date, priority
             FROM posts WHERE user_id = ? AND state = ? AND deleted_at IS NULL
             ORDER BY scheduled_at DESC NULLS LAST, created_at DESC
             LIMIT ? OFFSET ?"#,
        )
        .bind(user_id)
        .bind(ps)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        list_posts_all(pool, user_id, limit, offset).await
    }
}

/// Count posts for a user, optionally filtered by state
pub async fn count_posts_by_user(
    pool: &SqlitePool,
    user_id: Uuid,
    state_filter: Option<&str>,
) -> Result<i64, sqlx::Error> {
    if let Some(st) = state_filter {
        let ps: PostState = match st {
            "idea" => PostState::Idea,
            "draft" => PostState::Draft,
            "queued" => PostState::Queued,
            "published" => PostState::Published,
            "error" | "errored" => PostState::Error,
            _ => {
                let row: (Option<i64>,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM posts WHERE user_id = ? AND deleted_at IS NULL",
                )
                .bind(user_id)
                .fetch_one(pool)
                .await?;
                return Ok(row.0.unwrap_or(0));
            }
        };
        let row: (Option<i64>,) = sqlx::query_as(
            "SELECT COUNT(*) FROM posts WHERE user_id = ? AND state = ? AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(ps)
        .fetch_one(pool)
        .await?;
        Ok(row.0.unwrap_or(0))
    } else {
        let row: (Option<i64>,) =
            sqlx::query_as("SELECT COUNT(*) FROM posts WHERE user_id = ? AND deleted_at IS NULL")
                .bind(user_id)
                .fetch_one(pool)
                .await?;
        Ok(row.0.unwrap_or(0))
    }
}

async fn list_posts_all(
    pool: &SqlitePool,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<Post>, sqlx::Error> {
    // Runtime query (see note in list_posts above).
    sqlx::query_as::<_, Post>(
        r#"SELECT id, user_id, integration_id, state,
            content, title, media, settings, scheduled_at, published_at,
            platform_post_id, platform_post_url, error_message,
            created_at, updated_at,
            repeat_interval_days, repeat_end_date, group_id,
            first_comment, sequence, idempotency_key,
            campaign_id,
            kanban_sort_order, kanban_substate, due_date, priority
           FROM posts WHERE user_id = ? AND deleted_at IS NULL
           ORDER BY scheduled_at DESC NULLS LAST, created_at DESC
           LIMIT ? OFFSET ?"#,
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

// ── Phase 5: search + filter + sort variants ───────────────────
// Runtime queries (not sqlx::query! macros) per AGENTS.md §0 rule 4.
// These support the new ListPostsQuery params: q, integration_ids,
// tag_ids, sort. The original list_posts / list_posts_all are kept
// for backward compatibility (MCP/CLI use them).

/// SQLite has no array type, so `IN (?, ?, ?)` id lists are bound as a
/// JSON array of strings and expanded with `json_each(?)`. `None` binds
/// SQL NULL, which the `? IS NULL` guard in the query short-circuits on.
///
/// The expansion must go through `unhex(replace(value, '-', ''))`, not the
/// raw `json_each` value: uuid columns hold 16-byte BLOBs, and a TEXT
/// value never equals one. See the note in `crate::db::types`.
fn ids_json(ids: Option<&[Uuid]>) -> Option<String> {
    ids.map(|v| {
        serde_json::to_string(&v.iter().map(|u| u.to_string()).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_string())
    })
}

/// Build the ORDER BY clause from a sort string.
/// Supported: "scheduled_date" (default), "created_date", "engagement".
/// Prefix with "-" for descending (descending is the default for all).
fn sort_to_order_by(sort: &str) -> &'static str {
    let ascending = !sort.starts_with('-');
    let field = sort.trim_start_matches('-');
    // We only support descending for now (ascending would need index work);
    // ignore the ascending flag and always return DESC.
    let _ = ascending;
    match field {
        "created_date" => "created_at DESC NULLS LAST",
        "engagement" => {
            // The engagement sort is handled by its own query branch (it
            // needs a subquery over `analytics_cache`). This arm is only
            // reached if a caller reaches the plain branch with
            // sort="engagement" - and the `posts` table has no column
            // named `engagement`, so the ORDER BY used to fail with
            // "no such column: engagement" and 500 the whole list. Fall
            // back to the default order; the dedicated branch is the one
            // that actually sorts by engagement.
            "scheduled_at DESC NULLS LAST, created_at DESC"
        }
        _ => "scheduled_at DESC NULLS LAST, created_at DESC",
    }
}

/// Search + filter + sort posts for a user. Used by the Phase 5 posts
/// list endpoint. All params are optional; passing None for a filter
/// means "no filter on this field".
#[expect(
    clippy::too_many_arguments,
    reason = "Search/filter parameters: each filter is a distinct DB column, no grouping available"
)]
pub async fn list_posts_search(
    pool: &SqlitePool,
    user_id: Uuid,
    state_filter: Option<&str>,
    q: Option<&str>,
    integration_ids: Option<&[Uuid]>,
    tag_ids: Option<&[Uuid]>,
    campaign_id: Option<Uuid>,
    sort: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Post>, sqlx::Error> {
    let order_by = sort_to_order_by(sort);

    // Build a dynamic query. We use string interpolation for the ORDER BY
    // (safe because sort_to_order_by returns a fixed set of literals) and
    // bind all user-provided values as parameters.
    let q_pattern = q.map(|s| {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    });

    let sql: String = if sort == "engagement" {
        // Engagement sort.
        //
        // The engagement of the user's OWN posts lives in `analytics_cache`,
        // keyed by (user_id, platform_post_id) - the payload is a list of
        // {label, data:[{total}]} CUMULATIVE series, so each metric's current
        // value is its LAST point (v26: this used to SUM the points, which
        // inflated the sort score ~10x while /analytics showed the true value
        // - the same post ranked differently on the two pages).
        //
        // The metric expressions come from `metric_value_expr` - the ONE
        // implementation - so an X post's "Retweets" counts and no reader
        // can disagree with another.
        let engagement_score = format!(
            "COALESCE({likes}, 0) + COALESCE({comments}, 0) + COALESCE({shares}, 0)",
            likes = metric_value_expr(METRIC_LIKES, "p"),
            comments = metric_value_expr(METRIC_COMMENTS, "p"),
            shares = metric_value_expr(METRIC_SHARES, "p"),
        );
        // SQLite has no uuid[] arrays: the id lists are bound as JSON
        // text and expanded with json_each(...). `? IS NULL` is how the
        // optional filters are skipped, and ILIKE becomes LIKE with an
        // explicit ESCAPE so the Rust-side backslash escaping still works
        // (SQLite LIKE is case-insensitive for ASCII by default).
        format!(
            r#"SELECT p.id, p.user_id, p.integration_id, p.state,
                  p.content, p.title, p.media, p.settings, p.scheduled_at, p.published_at,
                  p.platform_post_id, p.platform_post_url, p.error_message,
                  p.created_at, p.updated_at,
                  p.repeat_interval_days, p.repeat_end_date, p.group_id,
                  p.first_comment, p.sequence, p.idempotency_key,
                  p.campaign_id,
                  p.kanban_sort_order, p.kanban_substate, p.due_date, p.priority
           FROM posts p
           WHERE p.user_id = ?
             AND p.deleted_at IS NULL
             AND (? IS NULL OR p.state = ?)
             AND (? IS NULL OR p.content LIKE ? ESCAPE '\' OR p.title LIKE ? ESCAPE '\')
             AND (? IS NULL OR p.integration_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?)))
             AND (? IS NULL OR p.id IN (
               SELECT post_id FROM post_tags WHERE tag_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?))
             ))
             AND (? IS NULL OR p.campaign_id = ?)
           ORDER BY ({engagement_score}) DESC
           LIMIT ? OFFSET ?"#,
        )
    } else {
        format!(
            r#"SELECT id, user_id, integration_id, state,
                  content, title, media, settings, scheduled_at, published_at,
                  platform_post_id, platform_post_url, error_message,
                  created_at, updated_at,
                  repeat_interval_days, repeat_end_date, group_id,
                  first_comment, sequence, idempotency_key,
                  campaign_id,
                  kanban_sort_order, kanban_substate, due_date, priority
           FROM posts
           WHERE user_id = ?
             AND deleted_at IS NULL
             AND (? IS NULL OR state = ?)
             AND (? IS NULL OR content LIKE ? ESCAPE '\' OR title LIKE ? ESCAPE '\')
             AND (? IS NULL OR integration_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?)))
             AND (? IS NULL OR id IN (
               SELECT post_id FROM post_tags WHERE tag_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?))
             ))
             AND (? IS NULL OR campaign_id = ?)
            ORDER BY {} LIMIT ? OFFSET ?"#,
            order_by
        )
    };

    let mut q_builder = sqlx::query_as::<_, Post>(&sql)
        .bind(user_id)
        .bind(state_filter)
        .bind(state_filter)
        .bind(&q_pattern)
        .bind(&q_pattern)
        .bind(&q_pattern);
    // Bind integration_ids / tag_ids as JSON text; json_each expands them
    // back into a row set. `None` binds SQL NULL, which the `? IS NULL`
    // guard above short-circuits on.
    q_builder = q_builder
        .bind(ids_json(integration_ids))
        .bind(ids_json(integration_ids));
    q_builder = q_builder.bind(ids_json(tag_ids)).bind(ids_json(tag_ids));
    // The campaign filter: one placeholder, bound twice (`? IS NULL OR
    // campaign_id = ?`).
    q_builder = q_builder.bind(campaign_id).bind(campaign_id);
    q_builder = q_builder.bind(limit).bind(offset);
    q_builder.fetch_all(pool).await
}

/// Count posts matching the search + filter criteria (for pagination total).
pub async fn count_posts_search(
    pool: &SqlitePool,
    user_id: Uuid,
    state_filter: Option<&str>,
    q: Option<&str>,
    integration_ids: Option<&[Uuid]>,
    tag_ids: Option<&[Uuid]>,
    campaign_id: Option<Uuid>,
) -> Result<i64, sqlx::Error> {
    let q_pattern = q.map(|s| {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    });

    let row: (Option<i64>,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM posts
           WHERE user_id = ?
             AND (? IS NULL OR state = ?)
             AND (? IS NULL OR content LIKE ? ESCAPE '\' OR title LIKE ? ESCAPE '\')
             AND (? IS NULL OR integration_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?)))
              AND (? IS NULL OR id IN (
                SELECT post_id FROM post_tags WHERE tag_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?))
              ))
              AND (? IS NULL OR campaign_id = ?)"#,
    )
    .bind(user_id)
    .bind(state_filter)
    .bind(state_filter)
    .bind(&q_pattern)
    .bind(&q_pattern)
    .bind(&q_pattern)
    .bind(ids_json(integration_ids))
    .bind(ids_json(integration_ids))
    .bind(ids_json(tag_ids))
    .bind(ids_json(tag_ids))
    .bind(campaign_id)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0))
}

pub async fn get_post(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<Post>, sqlx::Error> {
    // Runtime query (see note in list_posts above).
    sqlx::query_as::<_, Post>(
        r#"SELECT id, user_id, integration_id, state,
            content, title, media, settings, scheduled_at, published_at,
            platform_post_id, platform_post_url, error_message,
            created_at, updated_at,
            repeat_interval_days, repeat_end_date, group_id,
            first_comment, sequence, idempotency_key,
            campaign_id,
            kanban_sort_order, kanban_substate, due_date, priority
          FROM posts WHERE id = ? AND user_id = ? AND deleted_at IS NULL"#,
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn update_post_content(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    content: &str,
    title: Option<&str>,
    media: &serde_json::Value,
    settings: &serde_json::Value,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET content = ?, title = ?, media = ?, settings = ?,
            updated_at = unixepoch()
            WHERE id = ? AND user_id = ?
            RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(content)
    .bind(title)
    .bind(media)
    .bind(settings)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// v23-5: update post content + first_comment in one UPDATE.
/// The original update_post_content only updates content/title/media/
/// settings — first_comment was silently dropped on edit. This variant
/// adds first_comment to the UPDATE so the composer's edit-mode can
/// persist first-comment changes.
#[expect(
    clippy::too_many_arguments,
    reason = "Post edit fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn update_post_full(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    content: &str,
    title: Option<&str>,
    media: &serde_json::Value,
    settings: &serde_json::Value,
    first_comment: Option<&str>,
    campaign_id: Option<Option<Uuid>>,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET
             content = ?, title = ?, media = ?, settings = ?,
             first_comment = ?,
             campaign_id = CASE WHEN ? THEN ? ELSE campaign_id END,
             updated_at = unixepoch()
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(content)
    .bind(title)
    .bind(media)
    .bind(settings)
    .bind(first_comment)
    // The three-state pattern: absent = keep the stored campaign,
    // `Some(None)` = clear it, `Some(Some(id))` = set it.
    //
    // The campaign column existed and the composer had a dropdown for
    // it, but the UPDATE never touched it - so filing a post under a
    // campaign on an EDIT silently did nothing. The column was
    // read-only after creation.
    .bind(campaign_id.is_some())
    .bind(campaign_id.flatten())
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn schedule_post(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    scheduled_at: DateTime<Utc>,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET scheduled_at = ?, state = 'queued',
            updated_at = unixepoch()
            WHERE id = ? AND user_id = ?
              AND state IN ('idea', 'draft', 'queued', 'error')
              AND deleted_at IS NULL
            RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(EpochUtc::from(scheduled_at))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    // §6.2: a fresh scheduled_at — shorten the scheduler's sleep so the
    // post goes out on time (not up to 30s late).
    .inspect(|_| crate::scheduler::wake())
}

/// v24-1: Unschedule a post — transition it back to draft state.
/// Sets scheduled_at = NULL and state = 'draft'. Only works on posts
/// that are currently 'queued' or 'draft' (not published/error/publishing).
/// Closes the ComposerModal TODO at line 551 which used the "100 years
/// in the future" hack to effectively unschedule.
pub async fn unschedule_post(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET scheduled_at = NULL, state = 'draft',
            error_message = NULL,
            updated_at = unixepoch()
            WHERE id = ? AND user_id = ?
              AND state IN ('queued', 'draft', 'error')
            RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Phase v21: reset a published post for re-publishing.
///
/// Sets `state = 'queued'`, updates `scheduled_at` to the new time,
/// and CLEARS `platform_post_id`, `platform_post_url`, `published_at`,
/// `error_message`, `retry_count`, `next_retry_at`. The scheduler will
/// then pick it up at the new time and publish a NEW post to the
/// platform (creating a fresh platform_post_id).
///
/// This is the "Reschedule the post" path of the postiz-inspired
/// drag-published-post modal.
///
/// Uses a runtime query (not query_as!) because we don't have a live
/// Postgres to regenerate the .sqlx offline cache with this new SQL.
pub async fn reset_post_for_republish(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    scheduled_at: DateTime<Utc>,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET
              scheduled_at = ?,
              state = 'queued',
              platform_post_id = NULL,
              platform_post_url = NULL,
              published_at = NULL,
              error_message = NULL,
              retry_count = 0,
              next_retry_at = NULL,
              -- Phase v22: generate a NEW idempotency key on re-publish
              -- so the provider treats it as a fresh post (not a retry
              -- of the original publish). Without this, a re-publish
              -- would be deduplicated by the provider and no new post
              -- would be created. SQLite has no gen_random_uuid(), so the
              -- key is generated in Rust and bound as a TEXT param.
              idempotency_key = ?,
              updated_at = unixepoch()
             WHERE id = ? AND user_id = ?
             RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(EpochUtc::from(scheduled_at))
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    // §6.2: re-publish just got a new due time.
    .inspect(|_| crate::scheduler::wake())
}

/// Phase v21: change a post's scheduled_at WITHOUT touching its state
/// or release fields. Used for the "Just update the post details" path
/// of the postiz-inspired drag-published-post modal — the user wants
/// to re-date a published post for archival purposes without triggering
/// a re-publish.
///
/// Leaves `state`, `platform_post_id`, `platform_post_url`,
/// `published_at` untouched. Only `scheduled_at` and `updated_at` change.
pub async fn update_post_date_only(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    scheduled_at: DateTime<Utc>,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET scheduled_at = ?, updated_at = unixepoch()
             WHERE id = ? AND user_id = ?
             RETURNING id, user_id, integration_id, state,
               content, title, media, settings, scheduled_at, published_at,
               platform_post_id, platform_post_url, error_message,
               created_at, updated_at,
               repeat_interval_days, repeat_end_date, group_id,
               first_comment, sequence, idempotency_key,
               campaign_id, kanban_substate, due_date"#,
    )
    .bind(EpochUtc::from(scheduled_at))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Atomically move a post into `publishing` before a manual publish. The
/// scheduler claims due posts the same way, so the two cannot both publish
/// one post. Returns false when the post is no longer publishable.
///
/// `draft` is claimable because a manual publish accepts an unscheduled
/// post: the composer's "Post Now" creates it without a schedule and then
/// publishes it. The scheduler only ever claims `queued` posts, so this
/// does not let the scheduler publish a draft.
pub async fn claim_post_for_publish(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE posts SET state = 'publishing', updated_at = unixepoch()
         WHERE id = ? AND user_id = ? AND state IN ('queued', 'error', 'draft') AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn update_post_state(
    pool: &SqlitePool,
    id: Uuid,
    state: PostState,
    platform_post_id: Option<&str>,
    platform_post_url: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = if state == PostState::Published {
        Some(Utc::now())
    } else {
        None
    };
    // SQLite stores timestamps as INTEGER epoch, so the bind is EpochUtc.
    let published_at = now.map(EpochUtc::from);
    // `state` is already a PostState; the old `state as PostState` cast was a
    // PG-shaped no-op that created a temporary the macro could not outlive.
    //
    // The caller's `error_message` is NULL when it has nothing to say, and a
    // bare write then ERASES the stored reason - so a post that failed keeps
    // saying `error` and says nothing about why, and the founder is left with
    // a failure they cannot act on.
    //
    // That is not hypothetical: the manual publish path releases a claim with
    // `state = post.state.clone()` and `error_message = None`, and
    // `PostState::Error` is an accepted state for a manual publish. So a post
    // that failed, then had its thread publish skipped, lost its reason on the
    // release.
    //
    // Four of this statement's five value columns were already COALESCE-guarded
    // (published_at, platform_post_id, platform_post_url) and this one was not.
    // Same shape as the avatar and profile_name bugs, one column later.
    //
    // It also makes the parameter dead as written: all three call sites pass
    // None, so it only ever bound NULL. Guarded, those sites still work AND a
    // future caller that passes Some can actually set a reason.
    //
    // The DELIBERATE clear lives elsewhere and still writes outright:
    // `reset_post_for_republish` and `mark_post_error` mean it.
    //
    // Runtime query, not `query!`: the COALESCE arm's column list changed and
    // regenerating the offline cache needs a live database this environment
    // does not have (AGENTS.md §5.1-5.3).
    sqlx::query(
        "UPDATE posts SET state = ?, published_at = COALESCE(?, published_at), \
         platform_post_id = COALESCE(?, platform_post_id), \
         platform_post_url = COALESCE(?, platform_post_url), \
         error_message = COALESCE(?, error_message), updated_at = unixepoch() \
         WHERE id = ?",
    )
    .bind(state)
    .bind(published_at)
    .bind(platform_post_id)
    .bind(platform_post_url)
    .bind(error_message)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_post(pool: &SqlitePool, id: Uuid, user_id: Uuid) -> Result<bool, sqlx::Error> {
    // Soft-delete: set deleted_at = NOW() on the post AND on every post
    // sharing the same group_id (if any). This makes the delete reversible
    // and keeps the calendar / posts-list queries clean (they filter
    // `WHERE deleted_at IS NULL`).
    //
    // If the post has no group_id, only the single row is soft-deleted.
    // Runtime query (see note in list_posts above).
    let r = sqlx::query(
        r#"UPDATE posts SET deleted_at = unixepoch()
           WHERE user_id = ? AND deleted_at IS NULL AND (
             id = ?
             OR (group_id IS NOT NULL AND group_id = (SELECT group_id FROM posts WHERE id = ? AND user_id = ?))
           )"#,
    )
    .bind(user_id)
    .bind(id)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Hard-undelete a post (and its group). Useful for a future "Trash" UI.
pub async fn undelete_post(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    // Runtime query (see note in list_posts above).
    let r = sqlx::query(
        r#"UPDATE posts SET deleted_at = NULL
           WHERE user_id = ? AND deleted_at IS NOT NULL AND (
             id = ?
             OR (group_id IS NOT NULL AND group_id = (SELECT group_id FROM posts WHERE id = ? AND user_id = ?))
           )"#,
    )
    .bind(user_id)
    .bind(id)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// List all posts sharing a group_id (for thread/group editing in the composer).
/// Returns empty vec if group_id is None or no posts match.
/// Excludes soft-deleted posts.
pub async fn list_posts_by_group(
    pool: &SqlitePool,
    user_id: Uuid,
    group_id: Uuid,
) -> Result<Vec<Post>, sqlx::Error> {
    // Runtime query (see note in list_posts above).
    sqlx::query_as::<_, Post>(
        r#"SELECT id, user_id, integration_id, state,
           content, title, media, settings, scheduled_at, published_at,
           platform_post_id, platform_post_url, error_message,
           created_at, updated_at,
           repeat_interval_days, repeat_end_date, group_id,
           first_comment, sequence, idempotency_key,
           campaign_id, kanban_substate, due_date
         FROM posts
         WHERE user_id = ? AND group_id = ? AND deleted_at IS NULL
         ORDER BY sequence ASC, created_at ASC"#,
    )
    .bind(user_id)
    .bind(group_id)
    .fetch_all(pool)
    .await
}

/// Get a single post with its integration details (used for retry/publish now)
pub async fn get_post_with_integration(
    pool: &SqlitePool,
    post_id: Uuid,
    user_id: Uuid,
) -> Result<Option<PostWithIntegration>, sqlx::Error> {
    sqlx::query_as::<_, PostWithIntegration>(
        r#"SELECT p.id, p.user_id, p.integration_id,
             p.state,
             p.content, p.title, p.media, p.settings,
             p.scheduled_at, p.published_at,
             p.platform_post_id, p.platform_post_url, p.error_message,
             p.created_at, p.updated_at,
             p.repeat_interval_days, p.repeat_end_date, p.group_id,
             p.first_comment, p.sequence, p.idempotency_key,
             i.provider_identifier, i.access_token,
             i.refresh_token, i.token_expires_at,
             i.disabled as "integration_disabled",
             i.refresh_needed as "integration_refresh_needed"
           FROM posts p
           JOIN integrations i ON p.integration_id = i.id
           WHERE p.id = ? AND p.user_id = ?"#,
    )
    .bind(post_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// `posts.scheduled_at` compared as epoch seconds, whichever storage class
/// the row happens to use.
///
/// G-02: the column is `INTEGER` epoch, but sqlx's `DateTime<Utc>`
/// *encoder* emits RFC3339 TEXT, so older rows hold a TEXT value. SQLite
/// orders INTEGER before TEXT, so a plain `scheduled_at <= unixepoch()`
/// is **always false** for those rows and the scheduler never claimed them
/// (symptom: a `queued` post scheduled long ago that simply never
/// publishes). Writes now bind [`EpochUtc`], and migration 0002 rewrites
/// the existing TEXT rows — this expression keeps the *read* correct even
/// for a row that predates both.
///
/// `typeof` has to be checked explicitly: `strftime('%s', <integer>)` is
/// NULL (SQLite reads a bare number as a Julian day), and a plain
/// `CAST(<text> AS INTEGER)` yields the leading numeric prefix, i.e. the
/// year `2030`, not the epoch — which would mark a 2030 post as
/// long-past-due. Only TEXT is parsed here.
///
/// ponytail: wrapping the column defeats `idx_posts_scheduled`. Fine on a
/// single-user `posts` table; revert to a bare `p.scheduled_at` if that
/// index ever needs to be a covering scan.
const SCHEDULED_AT_EPOCH: &str = "CASE WHEN typeof(p.scheduled_at) = 'integer' \
        THEN p.scheduled_at \
        ELSE CAST(strftime('%s', p.scheduled_at) AS INTEGER) END";

/// Get posts due for publishing.
///
/// Uses `FOR UPDATE SKIP LOCKED` to claim rows atomically so multiple
/// `switchpost serve` instances running in parallel do not double-publish
/// the same post. Rows are still returned in `state = 'queued'`; the caller
/// is expected to commit/rollback to release the lock.
/// §6.2: the earliest `scheduled_at` (unix epoch) of any publishable
/// queued post — the scheduler sleeps until min(this, poll period) so a
/// "publish in 1 minute" isn't held hostage by the 30s poll. Runtime
/// form: like get_due_posts, it claims nothing and needs no cache entry.
/// (Mixed column types: the same CASE expression get_due_posts uses.)
pub async fn next_queued_epoch(pool: &SqlitePool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT MIN(CASE WHEN typeof(p.scheduled_at) = 'integer'
                  THEN p.scheduled_at
                  ELSE CAST(strftime('%s', p.scheduled_at) AS INTEGER) END)
           FROM posts p
           JOIN integrations i ON p.integration_id = i.id
           WHERE p.state = 'queued' AND i.disabled = false
             AND p.scheduled_at IS NOT NULL"#,
    )
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

/// Flip the given posts from `queued` to `publishing`, and return the
/// ids THIS statement actually flipped.
///
/// This is the claim, and it is a compare-and-set: the UPDATE only
/// touches rows that are STILL `queued`. Two claims that read the same
/// id list both try to flip it; only the one that wins the write lock
/// changes the row, and `RETURNING id` tells it exactly which rows
/// those were.
///
/// `RETURNING id` is what makes the claim EXCLUSIVE. Reading the rows
/// back by `state = 'publishing'` would not: a row another claim
/// flipped is also 'publishing', and the loser would publish it too.
/// A post published twice is unrecoverable on a real audience.
///
/// The SQLite transaction is a DEFERRED `BEGIN` - it takes the write
/// lock at the first WRITE, not at BEGIN - so the SELECT of the due
/// ids runs with no lock held and two claims CAN read the same list.
/// This guard is what makes that safe.
pub async fn claim_posts_for_publish<'e, E>(
    executor: E,
    ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let holes = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "UPDATE posts SET state = 'publishing', updated_at = unixepoch()
         WHERE id IN ({holes}) AND state = 'queued'
         RETURNING id"
    );
    let mut claim = sqlx::query_as::<_, (Uuid,)>(&sql);
    for id in ids {
        claim = claim.bind(id);
    }
    let claimed: Vec<(Uuid,)> = claim.fetch_all(executor).await?;
    Ok(claimed.into_iter().map(|(id,)| id).collect())
}

pub async fn get_due_posts(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<PostWithIntegration>, sqlx::Error> {
    // Atomically claim due posts by transitioning them
    // `queued -> publishing` in the same transaction that selects them.
    // This closes the dual-instance double-publish hole: a second
    // `switchpost serve` instance running concurrently will not see
    // rows already claimed by the first, because the WHERE clause
    // filters on `state = 'queued'` and we just flipped them to
    // `'publishing'`.
    //
    // Previously the transaction committed immediately after SELECT,
    // releasing the row lock before publish_post ran -- so two
    // instances could both pull the same queued posts.
    //
    // Uses runtime `sqlx::query_as` (not the `query_as!` macro) so
    // the build doesn't require a live DB or the .sqlx offline cache.
    // The trade-off is no compile-time column type checking — but
    // `PostWithIntegration` derives `FromRow` which handles the
    // runtime deserialization.
    let mut tx = pool.begin().await?;

    // SQLite has neither data-modifying CTEs (`WITH x AS (UPDATE ...)`)
    // nor `FOR UPDATE SKIP LOCKED`.
    //
    // The claim is made atomic by the GUARD on the UPDATE, not by the
    // transaction: the UPDATE only flips rows that are STILL 'queued'.
    // Two claims that read the same id list both try to flip it, and
    // only the one that wins the write lock actually changes the row -
    // the other matches nothing and returns no rows.
    //
    // The comment here used to claim "the pool opens transactions with
    // BEGIN IMMEDIATE (see db::create_pool)". It does not: sqlx issues
    // a deferred `BEGIN`, which takes the write lock at the FIRST
    // WRITE, not at BEGIN. So the SELECT ran with no lock held, and
    // two claims could read the same ids. The UPDATE's missing
    // `state = 'queued'` guard was the other half: without it, the
    // second claim flipped rows the first had already moved to
    // 'publishing' and returned them - and the post was PUBLISHED
    // TWICE.
    let sql = format!(
        r#"SELECT p.id
           FROM posts p
           JOIN integrations i ON p.integration_id = i.id
           WHERE p.state = 'queued'
             AND p.deleted_at IS NULL
             AND {SCHEDULED_AT_EPOCH} <= unixepoch()
             AND i.disabled = false
           ORDER BY {SCHEDULED_AT_EPOCH} ASC, p.sequence ASC
           LIMIT ?"#
    );

    let ids: Vec<(Uuid,)> = sqlx::query_as(&sql).bind(limit).fetch_all(&mut *tx).await?;

    if ids.is_empty() {
        tx.commit().await?;
        return Ok(Vec::new());
    }

    let post_ids: Vec<Uuid> = ids.iter().map(|(id,)| *id).collect();
    let claimed: Vec<Uuid> = claim_posts_for_publish(&mut *tx, &post_ids).await?;

    if claimed.is_empty() {
        // Another claim took every post this one selected. Publish
        // nothing rather than publishing a post twice.
        tx.commit().await?;
        return Ok(Vec::new());
    }

    let claimed_holes = std::iter::repeat_n("?", claimed.len())
        .collect::<Vec<_>>()
        .join(", ");
    let rows_sql = format!(
        r#"SELECT p.id, p.user_id, p.integration_id,
        p.state,
        p.content, p.title, p.media, p.settings,
        p.scheduled_at, p.published_at,
        p.platform_post_id, p.platform_post_url, p.error_message,
        p.created_at, p.updated_at,
        p.repeat_interval_days, p.repeat_end_date, p.group_id,
        p.first_comment, p.sequence, p.idempotency_key,
        i.provider_identifier, i.access_token,
        i.refresh_token, i.token_expires_at,
        i.disabled as integration_disabled,
        i.refresh_needed as integration_refresh_needed
      FROM posts p
      JOIN integrations i ON p.integration_id = i.id
      WHERE p.id IN ({claimed_holes})"#
    );
    let mut rows_query = sqlx::query_as::<_, PostWithIntegration>(&rows_sql);
    for id in &claimed {
        rows_query = rows_query.bind(id);
    }
    let rows: Vec<PostWithIntegration> = rows_query.fetch_all(&mut *tx).await?;

    tx.commit().await?;
    Ok(rows)
}

/// Reclaim posts stuck in `publishing` state for longer than the
/// given threshold (e.g. 5 minutes). Called on scheduler startup to
/// recover from a crash that left posts mid-flight. The operator
/// should manually review reclaimed posts before re-queuing, since
/// the platform API may have actually accepted the publish.
pub async fn reclaim_stuck_publishing(
    pool: &SqlitePool,
    stuck_after_secs: i64,
) -> Result<u64, sqlx::Error> {
    // SQLite has no make_interval() and stores timestamps as INTEGER
    // epoch, so the cutoff is computed in Rust and bound as EpochUtc.
    let cutoff = EpochUtc::from(Utc::now() - chrono::Duration::seconds(stuck_after_secs));
    let message =
        format!("Post was stuck in publishing state for > {stuck_after_secs} seconds -- manual review required");
    let result = sqlx::query(
        "UPDATE posts
         SET state = 'error',
             error_message = ?,
             updated_at = unixepoch()
         WHERE state = 'publishing'
           AND updated_at < ?",
    )
    .bind(message)
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// A post the scheduler failed to publish on time (the "posting gap"
/// detector's row). Deliberately narrower than [`PostWithIntegration`]
/// — the alert path only needs enough to identify the post and say how
/// late it is.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StuckPost {
    pub id: Uuid,
    pub user_id: Uuid,
    /// `state` cast to text in SQL: the alert path formats it into a log
    /// line, and a plain `String` avoids decoding the `post_state` enum.
    pub state: String,
    /// First 120 chars of the post body, for the notification body.
    pub content_preview: String,
    pub scheduled_at: DateTime<Utc>,
}

/// Find posts still sitting in `queued`/`publishing` more than
/// `grace_secs` past their `scheduled_at`.
///
/// Read-only: this reports a gap, it does not touch the state machine.
/// Soft-deleted posts (migration 027) are excluded — the operator
/// cancelled those, so a late slot is not a gap. Posts that already have
/// an *unread* `post_publishing_gap` notification are also excluded, so a
/// post that stays stuck surfaces once (and again after the operator
/// reads the alert) instead of on every scheduler tick.
/// Runtime `sqlx::query_as` (not the `query_as!` macro) so the build
/// needs no live DB or `.sqlx` cache entry.
pub async fn find_stuck_posts(
    pool: &SqlitePool,
    grace_secs: i64,
    limit: i64,
) -> Result<Vec<StuckPost>, sqlx::Error> {
    // SQLite has no make_interval(); the cutoff is computed in Rust and
    // bound as EpochUtc (INTEGER epoch under the db::types seam).
    let cutoff = EpochUtc::from(Utc::now() - chrono::Duration::seconds(grace_secs));
    sqlx::query_as::<_, StuckPost>(
        r#"SELECT p.id, p.user_id,
                  p.state AS state,
                  substr(p.content, 1, 120) AS content_preview,
                  p.scheduled_at
           FROM posts p
           WHERE p.state IN ('queued', 'publishing')
             AND p.deleted_at IS NULL
             AND p.scheduled_at IS NOT NULL
             AND p.scheduled_at < ?
             AND NOT EXISTS (
                 SELECT 1 FROM notifications n
                 WHERE n.user_id = p.user_id
                   AND n.notification_type = 'post_publishing_gap'
                   AND n.reference_id = p.id
                   AND n.is_read = false
             )
           ORDER BY p.scheduled_at ASC
           LIMIT ?"#,
    )
    .bind(cutoff)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Record a single publish attempt in the `publish_attempts` audit
/// table. Called by the scheduler on every publish call (success or
/// failure) so the operator has a full history.
pub async fn record_publish_attempt(
    pool: &SqlitePool,
    post_id: Uuid,
    attempt_number: i32,
    status: &str,
    error_message: Option<&str>,
    started_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO publish_attempts
         (id, post_id, attempt_number, status, error_message, started_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, unixepoch())",
    )
    .bind(Uuid::new_v4())
    .bind(post_id)
    .bind(attempt_number)
    .bind(status)
    .bind(error_message)
    .bind(EpochUtc::from(started_at))
    .execute(pool)
    .await?;
    Ok(())
}

// ══════════════════════════════════════════════════════════════
// TAGS
// ══════════════════════════════════════════════════════════════

#[derive(sqlx::FromRow)]
pub struct PostTagRow {
    pub id: Uuid,
    pub name: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_tags_for_post(
    pool: &SqlitePool,
    post_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<PostTagRow>, sqlx::Error> {
    sqlx::query_as!(
        PostTagRow,
        r#"SELECT t.id as "id!: Uuid", t.name, t.color,
                  t.created_at as "created_at!: DateTime<Utc>",
                  t.updated_at as "updated_at!: DateTime<Utc>"
           FROM post_tags pt
           JOIN tags t ON pt.tag_id = t.id
           WHERE pt.post_id = ? AND t.user_id = ?
           ORDER BY t.name"#,
        post_id,
        user_id,
    )
    .fetch_all(pool)
    .await
}

pub async fn set_post_tags(
    pool: &SqlitePool,
    post_id: Uuid,
    tag_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Delete existing post_tags for this post
    sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(post_id)
        .execute(&mut *tx)
        .await?;

    // Insert new post_tags (only if tag_ids is non-empty)
    for &tag_id in tag_ids {
        sqlx::query("INSERT INTO post_tags (post_id, tag_id) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .bind(post_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Get posts for a date range (calendar view)
pub async fn get_posts_by_date_range(
    pool: &SqlitePool,
    user_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"SELECT id, user_id, integration_id, state,
            content, title, media, settings, scheduled_at, published_at,
            platform_post_id, platform_post_url, error_message,
            created_at, updated_at,
            repeat_interval_days, repeat_end_date, group_id,
            first_comment, sequence, idempotency_key,
            campaign_id, kanban_substate, due_date
          FROM posts
           WHERE user_id = ?
             AND scheduled_at IS NOT NULL
             AND scheduled_at >= ?
             AND scheduled_at <= ?
           ORDER BY scheduled_at ASC"#,
    )
    .bind(user_id)
    .bind(EpochUtc::from(start))
    .bind(EpochUtc::from(end))
    .fetch_all(pool)
    .await
}

/// Get posts for a date range with engagement metrics from analytics_cache
pub async fn get_calendar_posts_with_metrics(
    pool: &SqlitePool,
    user_id: Uuid,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    // v23-4: optional filters. When Some, the query adds a WHERE clause.
    integration_ids: Option<&[Uuid]>,
    campaign_id: Option<Uuid>,
) -> Result<Vec<CalendarPostWithMetrics>, sqlx::Error> {
    // v23-4: build the query dynamically based on filters. We use
    // runtime query_as (not the query! macro) so we can conditionally
    // add WHERE clauses without regenerating the .sqlx cache.
    //
    // The filter parameters are appended after the existing $1/$2/$3
    // (user_id, start_date, end_date). We build a parameter list and
    // bind them in order.
    let mut where_clauses: Vec<String> = vec![
        "p.user_id = ?".into(),
        "p.deleted_at IS NULL".into(),
        // The OR is parenthesised: AND-joining it bare let the second branch
        // escape the user_id / deleted_at filters above.
        "((p.state != 'published' AND p.scheduled_at >= ? AND p.scheduled_at <= ?) \
         OR (p.state = 'published' AND p.published_at >= ? AND p.published_at <= ?))"
            .into(),
    ];
    // SQLite has no array type: the integration_id list is bound as JSON
    // text and expanded by json_each (? IS NULL disables the filter).
    // The expansion goes through `unhex` because uuid columns hold
    // 16-byte BLOBs — a TEXT value never equals one.
    let mut has_ids = false;
    if let Some(ids) = &integration_ids {
        if !ids.is_empty() {
            where_clauses.push(
                "p.integration_id IN (SELECT unhex(replace(value, '-', '')) FROM json_each(?))"
                    .into(),
            );
            has_ids = true;
        }
    }
    if let Some(_cid) = campaign_id {
        where_clauses.push("p.campaign_id = ?".into());
    }

    let where_sql = where_clauses.join(" AND ");

    let sql = format!(
        r#"SELECT
           p.id, p.user_id, p.integration_id,
           p.state as state,
           p.content, p.title, p.media,
           p.scheduled_at, p.published_at,
           p.platform_post_id, p.platform_post_url,
           p.error_message,
           p.created_at,
           p.repeat_interval_days, p.repeat_end_date,
           p.group_id, p.first_comment, p.sequence,
           p.campaign_id,
           i.provider_name as integration_name,
           -- Engagement for the user's OWN posts lives in `analytics_cache`,
           -- keyed by (user_id, platform_post_id) - NOT in `post_engagement`,
           -- whose `post_id` FK targets `external_posts` and so can never
           -- hold a row for a `posts` row. These four columns used to be
           -- hard-coded NULL, which made every engagement badge on the
           -- calendar permanently absent. The payload is a list of
           -- {{label, data:[{{total}}]}} series; the provider series are
           -- CUMULATIVE, so the current value is the LAST point of the
           -- matching series (v26: this used to SUM the points, which
           -- multiplied a running total by the sample count - a post with
           -- 70 likes read as 520 on the badge while /analytics showed 70).
           -- A post with no cached analytics keeps NULL (unknown), not 0.
           {likes} AS likes,
           {comments} AS comments,
           {shares} AS shares,
           {impressions} AS impressions
         FROM posts p
         LEFT JOIN integrations i ON p.integration_id = i.id
         WHERE {where_sql}
         ORDER BY
           CASE WHEN p.state = 'published' THEN p.published_at ELSE p.scheduled_at END ASC"#,
        likes = metric_value_expr(METRIC_LIKES, "p"),
        comments = metric_value_expr(METRIC_COMMENTS, "p"),
        shares = metric_value_expr(METRIC_SHARES, "p"),
        impressions = metric_value_expr(METRIC_IMPRESSIONS, "p"),
    );

    // Epoch INTEGER columns: a bare DateTime binds as TEXT, which SQLite
    // orders after every INTEGER, so no range comparison could ever match.
    let (start_epoch, end_epoch) = (EpochUtc::from(start_date), EpochUtc::from(end_date));
    let mut q = sqlx::query_as::<_, CalendarPostWithMetrics>(&sql)
        .bind(user_id)
        .bind(start_epoch)
        .bind(end_epoch)
        .bind(start_epoch)
        .bind(end_epoch);

    if has_ids {
        q = q.bind(ids_json(integration_ids));
    }
    if let Some(cid) = campaign_id {
        q = q.bind(cid);
    }

    q.fetch_all(pool).await
}

/// Find next free time slot for scheduling
pub async fn find_next_free_slot(
    pool: &SqlitePool,
    user_id: Uuid,
    integration_id: Option<Uuid>,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let now = Utc::now();

    // Get posting_times from integration if provided
    let posting_times: Vec<i64> = if let Some(iid) = integration_id {
        let integration = sqlx::query_scalar!(
            r#"SELECT posting_times as "posting_times: serde_json::Value" FROM integrations WHERE id = ? AND user_id = ?"#,
            iid,
            user_id
        )
        .fetch_optional(pool)
        .await?;

        integration
            .and_then(|v| {
                crate::db::models::decode_posting_times(v)
                    .as_array()
                    .cloned()
            })
            .map(|arr| {
                arr.iter()
                    .filter_map(|obj| obj.get("time").and_then(|t| t.as_i64()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };

    // Fallback: if no posting_times configured, use old logic (last + 2h)
    if posting_times.is_empty() {
        let last: Option<DateTime<Utc>> = if let Some(iid) = integration_id {
            sqlx::query_scalar!(
                r#"SELECT MAX(scheduled_at) as "max: Option<EpochUtc>" FROM posts
                  WHERE user_id = ? AND integration_id = ? AND state != 'error'"#,
                user_id,
                iid
            )
            .fetch_one(pool)
            .await?
            .flatten()
            .map(|e| e.0)
        } else {
            sqlx::query_scalar!(
                r#"SELECT MAX(scheduled_at) as "max: Option<EpochUtc>" FROM posts
                  WHERE user_id = ? AND state != 'error'"#,
                user_id,
            )
            .fetch_one(pool)
            .await?
            .flatten()
            .map(|e| e.0)
        };
        // Never before `now`: a last post from days ago would otherwise yield a
        // slot in the past that the scheduler publishes immediately.
        let base = last.map_or(now, |l| l.max(now));
        return Ok(Some(base + chrono::Duration::hours(2)));
    }

    // Get scheduled posts for the next 14 days
    let end = now + chrono::Duration::days(14);
    // Bind to a `let` first: `EpochUtc::from(now)` as an inline argument is a
    // temporary, and the macro holds a reference to it across the `.await`.
    let now_epoch = EpochUtc::from(now);
    let end_epoch = EpochUtc::from(end);
    let scheduled: Vec<DateTime<Utc>> = if let Some(iid) = integration_id {
        sqlx::query_scalar!(
            r#"SELECT scheduled_at FROM posts
               WHERE user_id = ? AND integration_id = ? AND state != 'error'
               AND scheduled_at >= ? AND scheduled_at <= ?"#,
            user_id,
            iid,
            now_epoch,
            end_epoch
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .flatten()
        .filter_map(|secs| DateTime::from_timestamp(secs, 0))
        .collect()
    } else {
        sqlx::query_scalar!(
            r#"SELECT scheduled_at FROM posts
               WHERE user_id = ? AND state != 'error'
               AND scheduled_at >= ? AND scheduled_at <= ?"#,
            user_id,
            now_epoch,
            end_epoch
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .flatten()
        .filter_map(|secs| DateTime::from_timestamp(secs, 0))
        .collect()
    };

    // Walk forward day by day, checking each posting_time slot
    let today = now.date_naive();
    for day_offset in 0..14i64 {
        let date = today + chrono::Duration::days(day_offset);
        for &minutes in &posting_times {
            // v22 Phase 1 (BUG #22): guard against malformed posting_times
            // JSON. `and_hms_opt` returns None when hours > 23 or minutes
            // > 59 — previously this `.unwrap()`ed and panicked the request
            // thread (returning HTTP 500). Now we skip the invalid slot
            // and log a warning so the operator can fix the integration's
            // posting_times config.
            let h = (minutes / 60) as u32;
            let m = (minutes % 60) as u32;
            if h > 23 || m > 59 || minutes < 0 {
                tracing::warn!(
                    "Invalid posting_time minutes value {minutes} (h={h}, m={m}) — skipping slot"
                );
                continue;
            }
            let Some(slot_naive) = date.and_hms_opt(h, m, 0) else {
                tracing::warn!(
                    "and_hms_opt returned None for minutes={minutes} (h={h}, m={m}) — skipping slot"
                );
                continue;
            };
            let slot = slot_naive.and_utc();
            // Skip slots in the past
            if slot <= now {
                continue;
            }
            // Skip slots that already have a post (within 1 minute tolerance)
            let occupied = scheduled
                .iter()
                .any(|s| (*s - slot).num_minutes().abs() < 1);
            if !occupied {
                return Ok(Some(slot));
            }
        }
    }

    // All slots full in 14 days — fallback
    Ok(Some(now + chrono::Duration::hours(2)))
}

// ══════════════════════════════════════════════════════════════
// MEDIA
// ══════════════════════════════════════════════════════════════

#[expect(
    clippy::too_many_arguments,
    reason = "Media metadata fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn create_media(
    pool: &SqlitePool,
    user_id: Uuid,
    original_name: &str,
    storage_path: &str,
    mime_type: &str,
    file_size: i64,
    width: Option<i32>,
    height: Option<i32>,
    alt_text: Option<&str>,
) -> Result<MediaEntry, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query_as!(
        MediaEntry,
        r#"INSERT INTO media (id, user_id, original_name, storage_path, mime_type, file_size, width, height, alt_text)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id as "id!: Uuid", user_id as "user_id!: Uuid",
           original_name, storage_path, mime_type, file_size,
           width as "width?: i32", height as "height?: i32", alt_text,
           created_at as "created_at!: DateTime<Utc>""#,
        id,
        user_id,
        original_name,
        storage_path,
        mime_type,
        file_size,
        width,
        height,
        alt_text,
    )
    .fetch_one(pool)
    .await
}

pub async fn list_media(
    pool: &SqlitePool,
    user_id: Uuid,
    limit: i64,
    offset: i64,
    search: Option<&str>,
) -> Result<Vec<MediaEntry>, sqlx::Error> {
    match search {
        Some(query) => {
            // SQLite's LIKE is case-insensitive for ASCII, which is what ILIKE
            // gave us. LIKE metacharacters are escaped so the user's search
            // text stays literal. Bound to a `let` first: `format!` as an
            // inline macro argument is a temporary, and the macro holds a
            // reference to it across the `.await`.
            let pattern = format!(
                "%{}%",
                query
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            sqlx::query_as!(
                MediaEntry,
                r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                        original_name, storage_path, mime_type, file_size,
                        width as "width?: i32", height as "height?: i32", alt_text,
                        created_at as "created_at!: DateTime<Utc>"
                 FROM media WHERE user_id = ? AND original_name LIKE ? ESCAPE '\'
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?"#,
                user_id,
                // The bind order must match the placeholder order in the
                // SQL: `user_id, pattern, limit, offset`. It was
                // `user_id, limit, offset, pattern`, so the LIKE
                // pattern received the integer LIMIT - and the search
                // matched NOTHING. Past the first page, OFFSET received
                // the pattern string and SQLite raised a datatype
                // mismatch, which the error handler turned into a 500.
                pattern,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as!(
                MediaEntry,
                r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                        original_name, storage_path, mime_type, file_size,
                        width as "width?: i32", height as "height?: i32", alt_text,
                        created_at as "created_at!: DateTime<Utc>"
                 FROM media WHERE user_id = ?
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?"#,
                user_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await
        }
    }
}

/// Update a media item's alt text.
///
/// The alt text is a real column (`media.alt_text`, migration 0004),
/// but nothing wrote it from the dashboard: the upload hard-coded
/// `None` and there was no endpoint to set it, so the founder's alt
/// text lived in the browser tab and was destroyed by the first save.
pub async fn set_media_alt(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    alt: Option<&str>,
) -> Result<Option<MediaEntry>, sqlx::Error> {
    sqlx::query_as::<_, MediaEntry>(
        r#"UPDATE media SET alt_text = ?
           WHERE id = ? AND user_id = ?
           RETURNING id as "id!: Uuid", user_id as "user_id!: Uuid",
                     original_name, storage_path, mime_type, file_size,
                     width as "width?: i32", height as "height?: i32", alt_text,
                     created_at as "created_at!: DateTime<Utc>""#,
    )
    .bind(alt)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn delete_media(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<MediaEntry>, sqlx::Error> {
    sqlx::query_as!(
        MediaEntry,
        r#"DELETE FROM media WHERE id = ? AND user_id = ?
         RETURNING id as "id!: Uuid", user_id as "user_id!: Uuid",
           original_name, storage_path, mime_type, file_size,
           width as "width?: i32", height as "height?: i32", alt_text,
           created_at as "created_at!: DateTime<Utc>""#,
        id,
        user_id,
    )
    .fetch_optional(pool)
    .await
}

pub async fn get_media(pool: &SqlitePool, id: Uuid) -> Result<Option<MediaEntry>, sqlx::Error> {
    sqlx::query_as!(
        MediaEntry,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                original_name, storage_path, mime_type, file_size,
                width as "width?: i32", height as "height?: i32", alt_text,
                created_at as "created_at!: DateTime<Utc>"
         FROM media WHERE id = ?"#,
        id,
    )
    .fetch_optional(pool)
    .await
}

pub async fn get_media_user(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<MediaEntry>, sqlx::Error> {
    sqlx::query_as!(
        MediaEntry,
        r#"SELECT id as "id!: Uuid", user_id as "user_id!: Uuid",
                original_name, storage_path, mime_type, file_size,
                width as "width?: i32", height as "height?: i32", alt_text,
                created_at as "created_at!: DateTime<Utc>"
         FROM media WHERE id = ? AND user_id = ?"#,
        id,
        user_id,
    )
    .fetch_optional(pool)
    .await
}

// ══════════════════════════════════════════════════════════════
// OAUTH STATE
// ══════════════════════════════════════════════════════════════

pub async fn save_oauth_state(
    pool: &SqlitePool,
    state: &str,
    provider: &str,
    code_verifier: &str,
    redirect_uri: Option<&str>,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO oauth_states (id, state, provider, code_verifier, redirect_uri)
         VALUES (?, ?, ?, ?, ?)",
        id,
        state,
        provider,
        code_verifier,
        redirect_uri,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_oauth_state(
    pool: &SqlitePool,
    state: &str,
) -> Result<Option<OAuthState>, sqlx::Error> {
    sqlx::query_as!(
        OAuthState,
        r#"SELECT id as "id!: Uuid", state, provider, code_verifier, redirect_uri,
                  created_at as "created_at!: DateTime<Utc>",
                  expires_at as "expires_at!: DateTime<Utc>"
         FROM oauth_states WHERE state = ? AND expires_at > unixepoch()"#,
        state,
    )
    .fetch_optional(pool)
    .await
}

pub async fn delete_oauth_state(pool: &SqlitePool, state: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM oauth_states WHERE state = ?", state)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn cleanup_expired_oauth_states(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query!("DELETE FROM oauth_states WHERE expires_at < unixepoch()")
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

// ══════════════════════════════════════════════════════════════
// NOTIFICATIONS
// ══════════════════════════════════════════════════════════════

pub async fn create_notification(
    pool: &SqlitePool,
    user_id: Uuid,
    title: &str,
    body: &str,
    notification_type: &str,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
) -> Result<Notification, sqlx::Error> {
    sqlx::query_as::<_, Notification>(
        r#"INSERT INTO notifications (id, user_id, title, body, notification_type, reference_type, reference_id)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           RETURNING id, user_id, title, body, notification_type, reference_type, reference_id, is_read, created_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(title)
    .bind(body)
    .bind(notification_type)
    .bind(reference_type)
    .bind(reference_id)
    .fetch_one(pool)
    .await
}

pub async fn list_notifications(
    pool: &SqlitePool,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query_as::<_, Notification>(
        r#"SELECT id, user_id, title, body, notification_type, reference_type, reference_id, is_read, created_at
           FROM notifications WHERE user_id = ?
           ORDER BY created_at DESC
           LIMIT ? OFFSET ?"#,
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn count_unread_notifications(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<i64, sqlx::Error> {
    let row: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM notifications WHERE user_id = ? AND is_read = false"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row.unwrap_or(0))
}

pub async fn mark_notification_read(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<Notification>, sqlx::Error> {
    sqlx::query_as::<_, Notification>(
        r#"UPDATE notifications SET is_read = true
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, title, body, notification_type, reference_type, reference_id, is_read, created_at"#,
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn mark_all_notifications_read(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        r#"UPDATE notifications SET is_read = true WHERE user_id = ? AND is_read = false"#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

// ══════════════════════════════════════════════════════════════
// RECURRING POSTS
// ══════════════════════════════════════════════════════════════

// Copies of a draft stay drafts. Any other source yields a `queued` copy, so a
// copy never inherits `published`, `publishing` or `error` (a source can move
// between the handler's state check and this insert). Such a copy would never
// publish on its date.
pub async fn create_repeated_post(
    pool: &SqlitePool,
    user_id: Uuid,
    original_id: Uuid,
    scheduled_at: &DateTime<Utc>,
    group_id: Uuid,
) -> Result<Post, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"INSERT INTO posts (id, user_id, integration_id, title, content, media, settings, scheduled_at, state, repeat_interval_days, repeat_end_date, group_id, idempotency_key)
           SELECT ?, p.user_id, p.integration_id, p.title, p.content, p.media, p.settings, ?, CASE WHEN p.state = 'draft' THEN 'draft' ELSE 'queued' END, NULL, NULL, ?, ?
           FROM posts p WHERE p.id = ? AND p.user_id = ?
           RETURNING id, user_id, integration_id, state,
             content, title, media, settings, scheduled_at, published_at,
             platform_post_id, platform_post_url, error_message,
             created_at, updated_at,
             repeat_interval_days, repeat_end_date, group_id,
             first_comment, sequence, idempotency_key,
             campaign_id, kanban_substate, due_date"#,
    )
        .bind(Uuid::new_v4())
        .bind(EpochUtc::from(*scheduled_at))
        .bind(group_id)
        .bind(Uuid::new_v4())
        .bind(original_id)
        .bind(user_id)
    .fetch_one(pool)
    .await
}

pub async fn set_post_recurring(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    interval_days: i32,
    end_date: &DateTime<Utc>,
    group_id: Uuid,
) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET repeat_interval_days = ?, repeat_end_date = ?, group_id = ?,
            updated_at = unixepoch()
            WHERE id = ? AND user_id = ?
            RETURNING id, user_id, integration_id, state,
              content, title, media, settings, scheduled_at, published_at,
              platform_post_id, platform_post_url, error_message,
              created_at, updated_at,
              repeat_interval_days, repeat_end_date, group_id,
              first_comment, sequence, idempotency_key,
              campaign_id, kanban_substate, due_date"#,
    )
    .bind(interval_days)
    .bind(EpochUtc::from(*end_date))
    .bind(group_id)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn set_post_recurring_with_copies(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    interval_days: i32,
    end_date: &DateTime<Utc>,
    group_id: Uuid,
    original_scheduled: &DateTime<Utc>,
) -> Result<(Vec<Uuid>, Vec<String>), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query_as::<_, Post>(
        r#"UPDATE posts SET repeat_interval_days = ?, repeat_end_date = ?, group_id = ?,
           updated_at = unixepoch()
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, integration_id, state,
              content, title, media, settings, scheduled_at, published_at,
              platform_post_id, platform_post_url, error_message,
              created_at, updated_at,
              repeat_interval_days, repeat_end_date, group_id,
              first_comment, sequence, idempotency_key,
              campaign_id, kanban_substate, due_date"#,
    )
    .bind(interval_days)
    .bind(EpochUtc::from(*end_date))
    .bind(group_id)
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let interval = chrono::Duration::days(interval_days as i64);
    let mut current = *original_scheduled + interval;
    let mut post_ids = Vec::new();
    let mut scheduled_dates = Vec::new();

    while current <= *end_date {
        // Runtime query (Phase v22 — idempotency_key column added).
        let copy = sqlx::query_as::<_, Post>(
            r#"INSERT INTO posts (id, user_id, integration_id, title, content, media, settings, scheduled_at, state, repeat_interval_days, repeat_end_date, group_id, idempotency_key)
               SELECT ?, p.user_id, p.integration_id, p.title, p.content, p.media, p.settings, ?, CASE WHEN p.state = 'draft' THEN 'draft' ELSE 'queued' END, NULL, NULL, ?, ?
               FROM posts p WHERE p.id = ? AND p.user_id = ?
               RETURNING id, user_id, integration_id, state,
                 content, title, media, settings, scheduled_at, published_at,
                 platform_post_id, platform_post_url, error_message,
                 created_at, updated_at,
                 repeat_interval_days, repeat_end_date, group_id,
                 first_comment, sequence, idempotency_key,
                 campaign_id, kanban_substate, due_date"#,
        )
        .bind(Uuid::new_v4())
        .bind(EpochUtc::from(current))
        .bind(group_id)
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        post_ids.push(copy.id);
        scheduled_dates.push(current.to_rfc3339());
        current += interval;
    }

    tx.commit().await?;
    // §6.2: the copies carry due times starting at the base schedule.
    crate::scheduler::wake();
    Ok((post_ids, scheduled_dates))
}

// ══════════════════════════════════════════════════════════════
// RSS FEEDS
// ══════════════════════════════════════════════════════════════

pub async fn create_rss_feed(
    pool: &SqlitePool,
    user_id: Uuid,
    feed_url: &str,
    integration_id: Uuid,
    title: &str,
    use_ai_summary: bool,
    enabled: bool,
) -> Result<RssFeed, sqlx::Error> {
    sqlx::query_as::<_, RssFeed>(
        r#"INSERT INTO rss_feeds (id, user_id, feed_url, integration_id, title, use_ai_summary, enabled)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           RETURNING id, user_id, feed_url, integration_id, title,
             last_polled_at, poll_interval_min, enabled, use_ai_summary,
             created_at, updated_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(feed_url)
    .bind(integration_id)
    .bind(title)
    .bind(use_ai_summary)
    .bind(enabled)
    .fetch_one(pool)
    .await
}

pub async fn list_rss_feeds(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<RssFeed>, sqlx::Error> {
    sqlx::query_as::<_, RssFeed>(
        r#"SELECT id, user_id, feed_url, integration_id, title,
           last_polled_at, poll_interval_min, enabled, use_ai_summary,
           created_at, updated_at
           FROM rss_feeds WHERE user_id = ?
           ORDER BY created_at DESC"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn get_rss_feed(
    pool: &SqlitePool,
    feed_id: Uuid,
    user_id: Uuid,
) -> Result<Option<RssFeed>, sqlx::Error> {
    sqlx::query_as::<_, RssFeed>(
        r#"SELECT id, user_id, feed_url, integration_id, title,
           last_polled_at, poll_interval_min, enabled, use_ai_summary,
           created_at, updated_at
           FROM rss_feeds WHERE id = ? AND user_id = ?"#,
    )
    .bind(feed_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn delete_rss_feed(
    pool: &SqlitePool,
    feed_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM rss_feeds WHERE id = ? AND user_id = ?")
        .bind(feed_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

pub async fn toggle_rss_feed(
    pool: &SqlitePool,
    feed_id: Uuid,
    user_id: Uuid,
) -> Result<Option<RssFeed>, sqlx::Error> {
    sqlx::query_as::<_, RssFeed>(
        r#"UPDATE rss_feeds SET enabled = NOT enabled, updated_at = unixepoch()
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, feed_url, integration_id, title,
             last_polled_at, poll_interval_min, enabled, use_ai_summary,
             created_at, updated_at"#,
    )
    .bind(feed_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_feeds_due_for_polling(pool: &SqlitePool) -> Result<Vec<RssFeed>, sqlx::Error> {
    // SQLite has no interval arithmetic, so the Postgres form
    // `last_polled_at + (poll_interval_min::text || ' minutes')::interval < NOW()`
    // cannot be expressed in SQL. The filter is equivalent to
    // `last_polled_at + poll_interval_min * 60 < now`, which SQLite can do
    // with plain integer seconds.
    sqlx::query_as::<_, RssFeed>(
        "SELECT id, user_id, feed_url, integration_id, title, last_polled_at, poll_interval_min, enabled, use_ai_summary, created_at, updated_at FROM rss_feeds WHERE enabled = true AND (last_polled_at IS NULL OR last_polled_at + poll_interval_min * 60 < unixepoch())"
    )
    .fetch_all(pool)
    .await
}

pub async fn update_feed_last_polled(pool: &SqlitePool, feed_id: Uuid) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE rss_feeds SET last_polled_at = unixepoch(), updated_at = unixepoch() WHERE id = ?",
    )
    .bind(feed_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

pub async fn insert_rss_post(
    pool: &SqlitePool,
    feed_id: Uuid,
    guid: &str,
    title: &str,
    url: &str,
    published_at: Option<DateTime<Utc>>,
    content_hash: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    // Claim by (feed_id, guid). `None` means this entry was already imported.
    let id = Uuid::new_v4();
    let r = sqlx::query(
        r#"INSERT INTO rss_posts (id, feed_id, guid, title, url, published_at, content_hash)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT (feed_id, guid) DO NOTHING"#,
    )
    .bind(id)
    .bind(feed_id)
    .bind(guid)
    .bind(title)
    .bind(url)
    .bind(published_at)
    .bind(content_hash)
    .execute(pool)
    .await?;
    Ok((r.rows_affected() == 1).then_some(id))
}

/// Undo a claim from `insert_rss_post` when the post for it was not created,
/// so the next poll retries the entry.
pub async fn delete_rss_post(pool: &SqlitePool, rss_post_id: Uuid) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM rss_posts WHERE id = ?")
        .bind(rss_post_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

pub async fn list_rss_feed_items(
    pool: &SqlitePool,
    feed_id: Uuid,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<RssPost>, sqlx::Error> {
    sqlx::query_as::<_, RssPost>(
        r#"SELECT rp.id, rp.feed_id, rp.post_id, rp.guid, rp.title, rp.url,
           rp.published_at, rp.content_hash, rp.is_imported, rp.created_at
           FROM rss_posts rp
           JOIN rss_feeds rf ON rp.feed_id = rf.id
           WHERE rf.id = ? AND rf.user_id = ?
           ORDER BY rp.created_at DESC
           LIMIT ? OFFSET ?"#,
    )
    .bind(feed_id)
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn update_rss_post_post_id(
    pool: &SqlitePool,
    rss_post_id: Uuid,
    post_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE rss_posts SET post_id = ?, is_imported = true WHERE id = ?")
        .bind(post_id)
        .bind(rss_post_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

pub async fn delete_notification(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(r#"DELETE FROM notifications WHERE id = ? AND user_id = ?"#)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() > 0)
}

// ══════════════════════════════════════════════════════════════
// POST ENGAGEMENT
// ══════════════════════════════════════════════════════════════

/// Upsert post_engagement for an external post.
/// Inserts a new row or updates the existing one on conflict (post_id).
pub async fn upsert_post_engagement(
    pool: &SqlitePool,
    post_id: Uuid,
    data: &crate::social::EngagementRow,
) -> Result<PostEngagement, sqlx::Error> {
    sqlx::query_as::<_, PostEngagement>(
        r#"INSERT INTO post_engagement
           (id, post_id, likes, comments, shares, views, saves, quotes, reposts, replies,
            reactions, upvotes, downvotes, upvote_ratio, awards, raw, fetched_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, unixepoch())
           ON CONFLICT (post_id) DO UPDATE SET
             likes = EXCLUDED.likes,
             comments = EXCLUDED.comments,
             shares = EXCLUDED.shares,
             views = EXCLUDED.views,
             saves = EXCLUDED.saves,
             quotes = EXCLUDED.quotes,
             reposts = EXCLUDED.reposts,
             replies = EXCLUDED.replies,
             reactions = EXCLUDED.reactions,
             upvotes = EXCLUDED.upvotes,
             downvotes = EXCLUDED.downvotes,
             upvote_ratio = EXCLUDED.upvote_ratio,
             awards = EXCLUDED.awards,
             raw = EXCLUDED.raw,
              fetched_at = unixepoch(),
              updated_at = unixepoch()
           RETURNING id, post_id, likes, comments, shares, views, saves, quotes, reposts, replies,
             reactions, upvotes, downvotes, upvote_ratio, awards, raw, fetched_at, created_at, updated_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(post_id)
    .bind(data.likes)
    .bind(data.comments)
    .bind(data.shares)
    .bind(data.views)
    .bind(data.saves)
    .bind(data.quotes)
    .bind(data.reposts)
    .bind(data.replies)
    .bind(&data.reactions)
    .bind(data.upvotes)
    .bind(data.downvotes)
    .bind(data.upvote_ratio)
    .bind(data.awards)
    .bind(&data.raw)
    .fetch_one(pool)
    .await
}

/// Get engagement data for a specific post.
pub async fn get_post_engagement_by_post_id(
    pool: &SqlitePool,
    post_id: Uuid,
) -> Result<Option<PostEngagement>, sqlx::Error> {
    sqlx::query_as::<_, PostEngagement>(
        r#"SELECT id, post_id, likes, comments, shares, views, saves, quotes, reposts, replies,
           reactions, upvotes, downvotes, upvote_ratio, awards, raw, fetched_at, created_at, updated_at
         FROM post_engagement WHERE post_id = ?"#
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await
}

/// E5: per-(provider, weekday, hour) engagement histogram over imported
/// posts with synced engagement — the input for best-time-to-post
/// suggestions. The engagement history for the user's own posts lives on
/// external_posts (the `switchpost import` flow + the feed refresher's
/// engagement sync); weekday is SQLite's %w (0=Sunday..6=Saturday),
/// hour 0-23, both UTC over the platform post timestamp.
pub async fn engagement_histogram_by_slot(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
    // Optional window floor. `None` keeps the pre-cutoff behaviour the
    // MCP `posts_best_times` tool relies on (all history).
    cutoff: Option<DateTime<Utc>>,
) -> Result<Vec<(String, i64, i64, i64)>, sqlx::Error> {
    // One row per (provider, epoch): the WEEKDAY×HOUR fold is the
    // CALLER's job, because a zone-aware fold cannot be expressed in
    // SQLite (`strftime('%w'/'%H')` is UTC-only; `localtime` is the
    // OS zone, not the founder's). Grouping the raw epoch here keeps
    // the SQL a pure aggregation and lets the caller bucket in the
    // founder's zone - the same shape as the analytics day series.
    sqlx::query_as(
        r#"SELECT ep.provider,
           CAST(strftime('%s', datetime(ep.created_at, 'unixepoch')) AS INTEGER),
           COUNT(*),
           SUM(COALESCE(pe.likes, 0) + COALESCE(pe.comments, 0) + COALESCE(pe.shares, 0))
          FROM external_posts ep
          INNER JOIN post_engagement pe ON pe.post_id = ep.id
         WHERE ep.user_id = ?
           AND ep.hidden_at IS NULL
           AND (? IS NULL OR ep.provider = ?)
           AND (? IS NULL OR ep.created_at >= ?)
           AND (COALESCE(pe.likes, 0) + COALESCE(pe.comments, 0) + COALESCE(pe.shares, 0)) > 0
         GROUP BY 1, 2"#,
    )
    .bind(user_id)
    .bind(provider)
    .bind(provider)
    .bind(cutoff.map(EpochUtc::from))
    .bind(cutoff.map(EpochUtc::from))
    .fetch_all(pool)
    .await
}

/// E3: the most recent platform_post_ids of an integration's published
/// posts — the inbox fans out get_post_comments over these.
pub async fn recent_platform_post_ids(
    pool: &SqlitePool,
    user_id: Uuid,
    integration_id: Uuid,
    limit: i64,
) -> Result<Vec<(String,)>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT platform_post_id FROM posts
            WHERE user_id = ? AND integration_id = ?
              AND state = 'published'
              AND platform_post_id IS NOT NULL
              AND deleted_at IS NULL
         ORDER BY published_at DESC LIMIT ?"#,
    )
    .bind(user_id)
    .bind(integration_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// List external posts with their engagement data LEFT JOINed.
/// Returns posts with engagement_* prefixed fields.
pub async fn list_all_external_posts_with_engagement(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
    author_handle: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<ExternalPostWithEngagement>, sqlx::Error> {
    if let Some(provider) = provider {
        sqlx::query_as::<_, ExternalPostWithEngagement>(
            r#"SELECT ep.id, ep.user_id, ep.provider, ep.platform_post_id,
               ep.text, ep.author_name, ep.author_handle, ep.author_avatar,
               ep.created_at, ep.url, ep.media, ep.metadata, ep.imported_at,
               pe.likes AS engagement_likes,
               pe.comments AS engagement_comments,
               pe.shares AS engagement_shares,
               pe.views AS engagement_views,
               pe.saves AS engagement_saves,
               pe.quotes AS engagement_quotes,
               pe.reposts AS engagement_reposts,
               pe.replies AS engagement_replies,
               pe.reactions AS engagement_reactions,
               pe.upvotes AS engagement_upvotes,
               pe.downvotes AS engagement_downvotes,
               pe.upvote_ratio AS engagement_upvote_ratio,
               pe.awards AS engagement_awards,
               pe.raw AS engagement_raw,
               pe.fetched_at AS engagement_fetched_at
             FROM external_posts ep
             LEFT JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ? AND ep.provider = ?
                AND ep.hidden_at IS NULL
                AND (? IS NULL OR ep.created_at < ?)
                AND (? IS NULL OR ep.author_handle = ?)
              ORDER BY ep.created_at DESC
              LIMIT ?"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(cursor)
        .bind(cursor)
        .bind(author_handle)
        .bind(author_handle)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ExternalPostWithEngagement>(
            r#"SELECT ep.id, ep.user_id, ep.provider, ep.platform_post_id,
               ep.text, ep.author_name, ep.author_handle, ep.author_avatar,
               ep.created_at, ep.url, ep.media, ep.metadata, ep.imported_at,
               pe.likes AS engagement_likes,
               pe.comments AS engagement_comments,
               pe.shares AS engagement_shares,
               pe.views AS engagement_views,
               pe.saves AS engagement_saves,
               pe.quotes AS engagement_quotes,
               pe.reposts AS engagement_reposts,
               pe.replies AS engagement_replies,
               pe.reactions AS engagement_reactions,
               pe.upvotes AS engagement_upvotes,
               pe.downvotes AS engagement_downvotes,
               pe.upvote_ratio AS engagement_upvote_ratio,
               pe.awards AS engagement_awards,
               pe.raw AS engagement_raw,
               pe.fetched_at AS engagement_fetched_at
             FROM external_posts ep
             LEFT JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ?
                AND ep.hidden_at IS NULL
                AND (? IS NULL OR ep.created_at < ?)
                AND (? IS NULL OR ep.author_handle = ?)
              ORDER BY ep.created_at DESC
              LIMIT ?"#,
        )
        .bind(user_id)
        .bind(cursor)
        .bind(cursor)
        .bind(author_handle)
        .bind(author_handle)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

/// Get total engagement summary across all posts for a user (for analytics dashboard).
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct EngagementSummary {
    pub total_likes: Option<i64>,
    pub total_comments: Option<i64>,
    pub total_shares: Option<i64>,
    pub total_views: Option<i64>,
    pub total_reposts: Option<i64>,
    pub total_replies: Option<i64>,
    pub total_upvotes: Option<i64>,
    pub total_awards: Option<i64>,
    pub posts_with_engagement: Option<i64>,
}

pub async fn get_engagement_summary(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
    cutoff: Option<DateTime<Utc>>,
) -> Result<EngagementSummary, sqlx::Error> {
    // Phase 2: add cutoff filter for date-range analytics.
    //
    // v26: the feed import pulls the CONNECTED account's own posts, so a post
    // published through Forge lands in BOTH `posts` (scored from
    // analytics_cache by /api/analytics/engagement) and `external_posts`
    // (scored here). The headline cards ADD the two sums, so the same post
    // counted twice - the "hallucinated metrics" report. Exclude imported
    // rows whose platform_post_id matches one of the user's own published
    // posts: the own-post side already scores them.
    let exclude_own = r#"AND ep.platform_post_id NOT IN (
               SELECT p.platform_post_id FROM posts p
               WHERE p.user_id = ep.user_id
                 AND p.state = 'published'
                 AND p.platform_post_id != '')"#;
    if let Some(provider) = provider {
        sqlx::query_as::<_, EngagementSummary>(
            &format!(
                r#"SELECT
               SUM(pe.likes) AS total_likes,
               SUM(pe.comments) AS total_comments,
               SUM(pe.shares) AS total_shares,
               SUM(pe.views) AS total_views,
               SUM(pe.reposts) AS total_reposts,
               SUM(pe.replies) AS total_replies,
               SUM(pe.upvotes) AS total_upvotes,
               SUM(pe.awards) AS total_awards,
               COUNT(pe.id) AS posts_with_engagement
             FROM external_posts ep
             INNER JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ? AND ep.provider = ?
                AND (? IS NULL OR ep.created_at >= ?)
                {exclude_own}"#
            ),
        )
        .bind(user_id)
        .bind(provider)
        .bind(cutoff.map(EpochUtc::from))
        .bind(cutoff.map(EpochUtc::from))
        .fetch_one(pool)
        .await
    } else {
        sqlx::query_as::<_, EngagementSummary>(
            &format!(
                r#"SELECT
               SUM(pe.likes) AS total_likes,
               SUM(pe.comments) AS total_comments,
               SUM(pe.shares) AS total_shares,
               SUM(pe.views) AS total_views,
               SUM(pe.reposts) AS total_reposts,
               SUM(pe.replies) AS total_replies,
               SUM(pe.upvotes) AS total_upvotes,
               SUM(pe.awards) AS total_awards,
               COUNT(pe.id) AS posts_with_engagement
             FROM external_posts ep
             INNER JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ?
                AND (? IS NULL OR ep.created_at >= ?)
                {exclude_own}"#
            ),
        )
        .bind(user_id)
        .bind(cutoff.map(EpochUtc::from))
        .bind(cutoff.map(EpochUtc::from))
        .fetch_one(pool)
        .await
    }
}

// ══════════════════════════════════════════════════════════════
// SIGNATURES
// ══════════════════════════════════════════════════════════════

pub async fn list_signatures(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<Vec<Signature>, sqlx::Error> {
    // Runtime query (Phase v21/v22 — is_default column added, can't
    // regenerate .sqlx offline cache without a live Postgres).
    sqlx::query_as::<_, Signature>(
        r#"SELECT id, user_id, name, content, provider, is_default,
           created_at, updated_at
           FROM signatures WHERE user_id = ?
           ORDER BY is_default DESC, created_at DESC"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn create_signature(
    pool: &SqlitePool,
    user_id: Uuid,
    name: &str,
    content: &str,
    provider: Option<&str>,
) -> Result<Signature, sqlx::Error> {
    sqlx::query_as::<_, Signature>(
        r#"INSERT INTO signatures (id, user_id, name, content, provider, is_default)
           VALUES (?, ?, ?, ?, ?, FALSE)
           RETURNING id, user_id, name, content, provider, is_default,
             created_at, updated_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(name)
    .bind(content)
    .bind(provider)
    .fetch_one(pool)
    .await
}

pub async fn update_signature(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
    name: Option<&str>,
    content: Option<&str>,
    provider: Option<&str>,
) -> Result<Option<Signature>, sqlx::Error> {
    sqlx::query_as::<_, Signature>(
        r#"UPDATE signatures SET
           name = COALESCE(?, name),
           content = COALESCE(?, content),
           provider = COALESCE(?, provider),
           updated_at = unixepoch()
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, name, content, provider, is_default,
             created_at, updated_at"#,
    )
    .bind(name)
    .bind(content)
    .bind(provider)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Phase v21/v22: set a signature as the default for its provider.
/// Atomically clears is_default on all other signatures for the same
/// (user_id, provider) and sets it on the target. Uses a transaction
/// so the swap is all-or-nothing.
///
/// `provider` is the target signature's provider (NULL = global).
/// The partial unique index idx_signatures_default_per_provider enforces
/// at-most-one-default per (user_id, provider) at the DB level — this
/// function clears the old default first to avoid a constraint violation.
pub async fn set_default_signature(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Option<Signature>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // 1. Fetch the target signature to get its provider.
    let target: Option<(Option<String>,)> =
        sqlx::query_as("SELECT provider FROM signatures WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;

    let target = match target {
        Some(t) => t.0,
        None => {
            tx.rollback().await?;
            return Ok(None);
        }
    };

    // 2. Clear is_default on all other signatures with the same (user_id, provider).
    sqlx::query(
        r#"UPDATE signatures SET is_default = FALSE
           WHERE user_id = ? AND is_default = TRUE
             AND (provider IS ?)"#,
    )
    .bind(user_id)
    .bind(&target)
    .execute(&mut *tx)
    .await?;

    // 3. Set is_default = TRUE on the target.
    let updated = sqlx::query_as::<_, Signature>(
        r#"UPDATE signatures SET is_default = TRUE, updated_at = unixepoch()
           WHERE id = ? AND user_id = ?
           RETURNING id, user_id, name, content, provider, is_default,
             created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Phase v21/v22: get the default signature for a given provider.
/// Falls back to the global default (provider IS NULL) if no
/// provider-specific default exists. Returns None if neither exists.
///
/// Used by the composer's auto-append flow: when creating a new post,
/// the frontend calls this to get the signature to append.
pub async fn get_default_signature(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
) -> Result<Option<Signature>, sqlx::Error> {
    // Try provider-specific default first, then global.
    if let Some(p) = provider {
        let provider_specific = sqlx::query_as::<_, Signature>(
            r#"SELECT id, user_id, name, content, provider, is_default,
               created_at, updated_at
               FROM signatures
               WHERE user_id = ? AND provider = ? AND is_default = TRUE"#,
        )
        .bind(user_id)
        .bind(p)
        .fetch_optional(pool)
        .await?;
        if provider_specific.is_some() {
            return Ok(provider_specific);
        }
    }
    // Fall back to global default.
    sqlx::query_as::<_, Signature>(
        r#"SELECT id, user_id, name, content, provider, is_default,
           created_at, updated_at
           FROM signatures
           WHERE user_id = ? AND provider IS NULL AND is_default = TRUE"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn delete_signature(
    pool: &SqlitePool,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "DELETE FROM signatures WHERE id = ? AND user_id = ?",
        id,
        user_id,
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

// ══════════════════════════════════════════════════════════════
// ANALYTICS CACHE
// ══════════════════════════════════════════════════════════════

/// Get all users (for background cache refresh)
pub async fn list_all_users(pool: &SqlitePool) -> Result<Vec<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id as "id!: Uuid", email, password, name,
                  timezone as "timezone!: i32",
                  created_at as "created_at!: DateTime<Utc>",
                  updated_at as "updated_at!: DateTime<Utc>"
           FROM users"#
    )
    .fetch_all(pool)
    .await
}

/// Upsert analytics cache: deletes existing entry for (user_id, provider, platform_post_id)
/// then inserts a fresh one, all in a transaction.
pub async fn upsert_analytics_cache(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: &str,
    platform_post_id: Option<&str>,
    data: &serde_json::Value,
) -> Result<AnalyticsCache, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM analytics_cache WHERE user_id = ? AND provider = ? \
         AND (platform_post_id = ? OR (? IS NULL AND platform_post_id IS NULL))",
    )
    .bind(user_id)
    .bind(provider)
    .bind(platform_post_id)
    .bind(platform_post_id)
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query_as::<_, AnalyticsCache>(
        "INSERT INTO analytics_cache (id, user_id, provider, platform_post_id, data) \
         VALUES (?, ?, ?, ?, ?) \
         RETURNING id, user_id, provider, platform_post_id, data, cached_at, expires_at",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(provider)
    .bind(platform_post_id)
    .bind(data)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(result)
}

/// Get non-expired account-level analytics for (user_id, provider)
pub async fn get_cached_analytics(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: &str,
    now: DateTime<Utc>,
) -> Result<Vec<AnalyticsCache>, sqlx::Error> {
    sqlx::query_as::<_, AnalyticsCache>(
        "SELECT id, user_id, provider, platform_post_id, data, cached_at, expires_at \
         FROM analytics_cache \
         WHERE user_id = ? AND provider = ? AND platform_post_id IS NULL AND expires_at > ? \
         ORDER BY provider",
    )
    .bind(user_id)
    .bind(provider)
    .bind(EpochUtc::from(now))
    .fetch_all(pool)
    .await
}

/// Get a specific cached analytics entry for a post
pub async fn get_single_cached_analytics(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: &str,
    platform_post_id: &str,
) -> Result<Option<AnalyticsCache>, sqlx::Error> {
    sqlx::query_as::<_, AnalyticsCache>(
        "SELECT id, user_id, provider, platform_post_id, data, cached_at, expires_at \
         FROM analytics_cache \
         WHERE user_id = ? AND provider = ? AND platform_post_id = ? AND expires_at > unixepoch()",
    )
    .bind(user_id)
    .bind(provider)
    .bind(platform_post_id)
    .fetch_optional(pool)
    .await
}

/// Delete all expired analytics cache entries. Returns count of deleted rows.
pub async fn delete_expired_analytics_cache(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM analytics_cache WHERE expires_at < unixepoch()")
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

// ── External Posts ──────────────────────────────────────────

/// Insert a new external post. Returns the created record.
/// Insert an external post, updating on conflict (provider + platform_post_id).
/// Returns `Some(post)` always — either the newly inserted or the updated record.
#[expect(
    clippy::too_many_arguments,
    reason = "External post fields: all bound to DB columns, no nesting grouping available"
)]
pub async fn insert_external_post(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: &str,
    platform_post_id: &str,
    text: &str,
    author_name: Option<&str>,
    author_handle: Option<&str>,
    author_avatar: Option<&str>,
    created_at: DateTime<Utc>,
    url: Option<&str>,
    media: &serde_json::Value,
    metadata: &serde_json::Value,
) -> Result<Option<ExternalPost>, sqlx::Error> {
    tracing::info!(
        "insert_external_post: provider={} post_id={} text_len={} name={:?} handle={:?} avatar_present={} url_present={}",
        provider, platform_post_id, text.len(), author_name, author_handle, author_avatar.is_some(), url.is_some(),
    );
    let result = sqlx::query_as::<_, ExternalPost>(
        "INSERT INTO external_posts \
         (id, user_id, provider, platform_post_id, text, author_name, author_handle, author_avatar, created_at, url, media, metadata) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (provider, platform_post_id) DO UPDATE SET \
           text = EXCLUDED.text, \
           author_name = COALESCE(EXCLUDED.author_name, external_posts.author_name), \
           author_handle = COALESCE(EXCLUDED.author_handle, external_posts.author_handle), \
           author_avatar = COALESCE(EXCLUDED.author_avatar, external_posts.author_avatar), \
           created_at = EXCLUDED.created_at, \
           url = EXCLUDED.url, \
           media = EXCLUDED.media, \
           metadata = EXCLUDED.metadata, \
           imported_at = unixepoch() \
         RETURNING id, user_id, provider, platform_post_id, text,
           author_name, author_handle, author_avatar,
           created_at, url, media, metadata, imported_at",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(provider)
    .bind(platform_post_id)
    .bind(text)
    .bind(author_name)
    .bind(author_handle)
    .bind(author_avatar)
    // The column is INTEGER epoch and every reader compares it against
    // an INTEGER cutoff. Binding a `DateTime<Utc>` here wrote RFC 3339
    // TEXT, and `created_at >= ?` was then ALWAYS TRUE - the 90-day
    // research window, the feed summary and the posting-time histogram
    // all let through posts of any age.
    .bind(EpochUtc::from(created_at))
    .bind(url)
    .bind(media)
    .bind(metadata)
    .fetch_optional(pool)
    .await?;
    if let Some(ref post) = result {
        tracing::info!(
            "insert_external_post RETURNED: id={} name={:?} handle={:?} avatar_present={}",
            post.id,
            post.author_name,
            post.author_handle,
            post.author_avatar.is_some(),
        );
    } else {
        tracing::info!("insert_external_post RETURNED: None");
    }
    Ok(result)
}

/// Update the metadata JSON of an external post (used for engagement updates).
pub async fn update_external_post_metadata(
    pool: &SqlitePool,
    id: Uuid,
    metadata: &serde_json::Value,
) -> Result<ExternalPost, sqlx::Error> {
    sqlx::query_as::<_, ExternalPost>(
        "UPDATE external_posts SET metadata = ? WHERE id = ? \
         RETURNING id, user_id, provider, platform_post_id, text,\
           author_name, author_handle, author_avatar,\
           created_at, url, media, metadata, imported_at",
    )
    .bind(metadata)
    .bind(id)
    .fetch_one(pool)
    .await
}

/// Get a single external post by ID and user_id.
pub async fn get_external_post_by_id(
    pool: &SqlitePool,
    user_id: Uuid,
    post_id: Uuid,
) -> Result<Option<ExternalPost>, sqlx::Error> {
    sqlx::query_as::<_, ExternalPost>(
        "SELECT id, user_id, provider, platform_post_id, text,\
           author_name, author_handle, author_avatar,\
           created_at, url, media, metadata, imported_at \
         FROM external_posts WHERE id = ? AND user_id = ?",
    )
    .bind(post_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// List external posts for a user + provider, newest first.
pub async fn list_external_posts(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: &str,
    limit: i64,
) -> Result<Vec<ExternalPost>, sqlx::Error> {
    sqlx::query_as::<_, ExternalPost>(
        "SELECT id, user_id, provider, platform_post_id, text,\
           author_name, author_handle, author_avatar,\
           created_at, url, media, metadata, imported_at \
         FROM external_posts WHERE user_id = ? AND provider = ? \
           ORDER BY created_at DESC LIMIT ?",
    )
    .bind(user_id)
    .bind(provider)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// List all external posts for a user across all providers, cursor-paginated by created_at DESC.
/// Pass cursor = None for the first page, then use the last post's created_at as the next cursor.
pub async fn list_all_external_posts(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<ExternalPost>, sqlx::Error> {
    if let Some(provider) = provider {
        sqlx::query_as::<_, ExternalPost>(
            "SELECT id, user_id, provider, platform_post_id, text,\
               author_name, author_handle, author_avatar,\
               created_at, url, media, metadata, imported_at \
             FROM external_posts \
              WHERE user_id = ? AND provider = ? \
                AND (? IS NULL OR created_at < ?) \
              ORDER BY created_at DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(provider)
        .bind(cursor)
        .bind(cursor)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ExternalPost>(
            "SELECT id, user_id, provider, platform_post_id, text,\
               author_name, author_handle, author_avatar,\
               created_at, url, media, metadata, imported_at \
             FROM external_posts \
              WHERE user_id = ? \
                AND (? IS NULL OR created_at < ?) \
              ORDER BY created_at DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(cursor)
        .bind(cursor)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

/// Search external posts with engagement data LEFT JOINed, mirroring
/// `list_all_external_posts_with_engagement`. Used by the /api/feed?q= endpoint
/// so search results include engagement metrics.
///
/// Case-insensitive ILIKE on `text`, `author_name`, or `author_handle`.
/// `q` is the raw user query — caller should pass it non-empty.
/// LIKE metacharacters (%, _, \) in the query are escaped so user input
/// is treated literally.
pub async fn search_all_external_posts_with_engagement(
    pool: &SqlitePool,
    user_id: Uuid,
    q: &str,
    provider: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<ExternalPostWithEngagement>, sqlx::Error> {
    let escaped = q
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");

    if let Some(provider) = provider {
        sqlx::query_as::<_, ExternalPostWithEngagement>(
            r#"SELECT ep.id, ep.user_id, ep.provider, ep.platform_post_id,
               ep.text, ep.author_name, ep.author_handle, ep.author_avatar,
               ep.created_at, ep.url, ep.media, ep.metadata, ep.imported_at,
               pe.likes AS engagement_likes,
               pe.comments AS engagement_comments,
               pe.shares AS engagement_shares,
               pe.views AS engagement_views,
               pe.saves AS engagement_saves,
               pe.quotes AS engagement_quotes,
               pe.reposts AS engagement_reposts,
               pe.replies AS engagement_replies,
               pe.reactions AS engagement_reactions,
               pe.upvotes AS engagement_upvotes,
               pe.downvotes AS engagement_downvotes,
               pe.upvote_ratio AS engagement_upvote_ratio,
               pe.awards AS engagement_awards,
               pe.raw AS engagement_raw,
               pe.fetched_at AS engagement_fetched_at
             FROM external_posts ep
             LEFT JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ? AND ep.provider = ?
                AND (? IS NULL OR ep.created_at < ?)
                AND (ep.text LIKE ? ESCAPE '\' OR ep.author_name LIKE ? ESCAPE '\' OR ep.author_handle LIKE ? ESCAPE '\')
              ORDER BY ep.created_at DESC
              LIMIT ?"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(cursor)
        .bind(cursor)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ExternalPostWithEngagement>(
            r#"SELECT ep.id, ep.user_id, ep.provider, ep.platform_post_id,
               ep.text, ep.author_name, ep.author_handle, ep.author_avatar,
               ep.created_at, ep.url, ep.media, ep.metadata, ep.imported_at,
               pe.likes AS engagement_likes,
               pe.comments AS engagement_comments,
               pe.shares AS engagement_shares,
               pe.views AS engagement_views,
               pe.saves AS engagement_saves,
               pe.quotes AS engagement_quotes,
               pe.reposts AS engagement_reposts,
               pe.replies AS engagement_replies,
               pe.reactions AS engagement_reactions,
               pe.upvotes AS engagement_upvotes,
               pe.downvotes AS engagement_downvotes,
               pe.upvote_ratio AS engagement_upvote_ratio,
               pe.awards AS engagement_awards,
               pe.raw AS engagement_raw,
               pe.fetched_at AS engagement_fetched_at
             FROM external_posts ep
             LEFT JOIN post_engagement pe ON pe.post_id = ep.id
              WHERE ep.user_id = ?
                AND (? IS NULL OR ep.created_at < ?)
                AND (ep.text LIKE ? ESCAPE '\' OR ep.author_name LIKE ? ESCAPE '\' OR ep.author_handle LIKE ? ESCAPE '\')
              ORDER BY ep.created_at DESC
              LIMIT ?"#,
        )
        .bind(user_id)
        .bind(cursor)
        .bind(cursor)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

// ── Resolved comments ──────────────────────────────────────────
// Persists which platform comments the user has marked "resolved"
// in the UI. Comments themselves are fetched live from provider APIs;
// this lightweight table is the only place the resolved flag can live.

/// Mark a comment as resolved for this user. Idempotent — if already
/// resolved, the row is touched (resolved_at refreshed) but no error
/// is returned.
pub async fn resolve_comment(
    pool: &SqlitePool,
    user_id: Uuid,
    comment_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO resolved_comments (user_id, comment_id, resolved_at) \
         VALUES (?, ?, unixepoch()) \
         ON CONFLICT (user_id, comment_id) DO UPDATE SET resolved_at = unixepoch()",
    )
    .bind(user_id)
    .bind(comment_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Unmark a comment as resolved (re-open it). Idempotent.
pub async fn unresolve_comment(
    pool: &SqlitePool,
    user_id: Uuid,
    comment_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM resolved_comments WHERE user_id = ? AND comment_id = ?")
        .bind(user_id)
        .bind(comment_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Return the set of resolved comment IDs for this user.
/// Used by the comments list endpoint to flag each CommentItem.status
/// as "resolved" instead of always "new".
pub async fn list_resolved_comment_ids(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT comment_id FROM resolved_comments WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

// ── Cached comments (B-3) ───────────────────────────────────────
// Cache layer for platform comments. The comments list endpoint reads
// from this table instead of doing 50 sequential provider API calls
// per page load. The background feed refresher writes here.

/// A cached comment row — shape matches the `cached_comments` table.
///
/// `post_text` is joined in so the comments list can show what each
/// comment is replying to without a second query.
///
/// The join was to `external_posts` alone, and `cached_comments.post_id`
/// points at `posts.id` - a comment on one of the FOUNDER'S OWN posts.
/// So the join matched nothing, `post_text` was always NULL, and the
/// comments list fell back to printing the raw internal UUID where the
/// post's text should be. A founder cannot act on a UUID.
///
/// Both are joined now, and the founder's own post wins: it is the one
/// the comment is actually on.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct CachedComment {
    pub id: i64,
    pub user_id: Uuid,
    pub comment_id: String,
    pub post_id: Uuid,
    pub provider: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub author_avatar: Option<String>,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    // Joined from external_posts — the text of the post this comment
    // is replying to. Optional because the post may have been deleted
    // (ON DELETE CASCADE on external_posts.id will remove the cached
    // comment too, but defensively handle NULL here).
    pub post_text: Option<String>,
}

/// Upsert a batch of comments for a single (user_id, post_id).
/// Called by the background feed refresher after it pulls comments
/// from a provider. Existing rows are touched (fetched_at updated)
/// so we know the cache is fresh; new rows are inserted.
///
/// `fetched_at` is set to NOW() for all rows in this batch.
/// A cached comment row upsert: (comment_id, text, author_name, author_handle,
/// author_avatar, created_at).
type CachedCommentRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
);

pub async fn upsert_cached_comments(
    pool: &SqlitePool,
    user_id: Uuid,
    post_id: Uuid,
    provider: &str,
    comments: &[CachedCommentRow],
) -> Result<(), sqlx::Error> {
    // (comment_id, text, author_name, author_handle, author_avatar, created_at)
    if comments.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    for (comment_id, text, author_name, author_handle, author_avatar, created_at) in comments {
        sqlx::query(
            r#"INSERT INTO cached_comments
                 (user_id, comment_id, post_id, provider,
                  author_name, author_handle, author_avatar,
                  text, created_at, fetched_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, unixepoch())
               ON CONFLICT (user_id, comment_id) DO UPDATE SET
                 post_id = EXCLUDED.post_id,
                 provider = EXCLUDED.provider,
                 author_name = EXCLUDED.author_name,
                 author_handle = EXCLUDED.author_handle,
                 author_avatar = EXCLUDED.author_avatar,
                 text = EXCLUDED.text,
                 created_at = EXCLUDED.created_at,
                  fetched_at = unixepoch()"#,
        )
        .bind(user_id)
        .bind(comment_id)
        .bind(post_id)
        .bind(provider)
        .bind(author_name)
        .bind(author_handle)
        .bind(author_avatar)
        .bind(text)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// List cached comments for a user, newest first, with the post text
/// joined in. Optional provider filter. Limited to `limit` rows.
pub async fn list_cached_comments(
    pool: &SqlitePool,
    user_id: Uuid,
    provider: Option<&str>,
    limit: i64,
) -> Result<Vec<CachedComment>, sqlx::Error> {
    if let Some(provider) = provider {
        sqlx::query_as::<_, CachedComment>(
            r#"SELECT cc.id, cc.user_id, cc.comment_id, cc.post_id, cc.provider,
                      cc.author_name, cc.author_handle, cc.author_avatar,
                      cc.text, cc.created_at, cc.fetched_at,
                      COALESCE(p.content, ep.text) AS post_text
               FROM cached_comments cc
               LEFT JOIN posts p ON p.id = cc.post_id
               LEFT JOIN external_posts ep ON ep.id = cc.post_id
               WHERE cc.user_id = ? AND cc.provider = ?
               ORDER BY cc.created_at DESC
               LIMIT ? "#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, CachedComment>(
            r#"SELECT cc.id, cc.user_id, cc.comment_id, cc.post_id, cc.provider,
                      cc.author_name, cc.author_handle, cc.author_avatar,
                      cc.text, cc.created_at, cc.fetched_at,
                      COALESCE(p.content, ep.text) AS post_text
               FROM cached_comments cc
               LEFT JOIN posts p ON p.id = cc.post_id
               LEFT JOIN external_posts ep ON ep.id = cc.post_id
               WHERE cc.user_id = ?
               ORDER BY cc.created_at DESC
               LIMIT ? "#,
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

/// Phase v21: list cached comments for a specific post (used by
/// GET /api/feed/{post_id}/comments). Falls back to live-fetch on cache
/// miss (handled by the caller). Returns newest-first, no limit (the
/// background refresher caps the cache size per post).
pub async fn list_cached_comments_for_post(
    pool: &SqlitePool,
    user_id: Uuid,
    post_id: Uuid,
) -> Result<Vec<CachedComment>, sqlx::Error> {
    sqlx::query_as::<_, CachedComment>(
        r#"SELECT cc.id, cc.user_id, cc.comment_id, cc.post_id, cc.provider,
                  cc.author_name, cc.author_handle, cc.author_avatar,
                  cc.text, cc.created_at, cc.fetched_at,
                  ep.text AS post_text
           FROM cached_comments cc
           LEFT JOIN external_posts ep ON ep.id = cc.post_id
           WHERE cc.user_id = ? AND cc.post_id = ?
           ORDER BY cc.created_at ASC"#,
    )
    .bind(user_id)
    .bind(post_id)
    .fetch_all(pool)
    .await
}
#[cfg(test)]
mod g02_tests {
    //! Regression coverage for G-02: `posts.scheduled_at` is an INTEGER
    //! epoch column, but sqlx's `DateTime<Utc>` encoder emits RFC3339 TEXT.
    //! SQLite sorts TEXT after INTEGER, so `scheduled_at <= unixepoch()` was
    //! permanently false for those rows and the scheduler never claimed them.
    use super::*;

    async fn fixture() -> (SqlitePool, Uuid) {
        let pool = crate::db::create_pool("sqlite::memory:").await.unwrap();
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        crate::db::ensure_local_user(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO integrations
                (id, user_id, provider_identifier, provider_name, internal_id,
                 access_token, disabled, refresh_needed, posting_times, auth_method)
             VALUES (?, ?, 'x', 'X', 'x', 'tok', 0, 0, '[]', 'oauth')",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
        (pool, user_id)
    }

    async fn insert(pool: &SqlitePool, at: Option<DateTime<Utc>>) -> Uuid {
        let post = create_post(
            pool,
            crate::auth::middleware::DEFAULT_USER_ID,
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM integrations")
                .fetch_one(pool)
                .await
                .unwrap(),
            "g02",
            None,
            &serde_json::json!([]),
            &serde_json::json!({}),
            at,
            Some(PostState::Queued),
            None,
            0,
            None,
        )
        .await
        .unwrap();
        post.id
    }

    #[tokio::test]
    async fn a_scheduled_post_is_stored_as_epoch_integer_not_rfc3339_text() {
        let (pool, _) = fixture().await;
        let at = Utc::now() - chrono::Duration::days(1);
        let id = insert(&pool, Some(at)).await;

        let ty: String = sqlx::query_scalar("SELECT typeof(scheduled_at) FROM posts WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ty, "integer", "scheduled_at must land as INTEGER epoch");

        // And it round-trips back to the same instant through `Post`.
        let stored: Post = sqlx::query_as("SELECT * FROM posts WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored.scheduled_at.unwrap().timestamp(), at.timestamp());
    }

    #[tokio::test]
    async fn rss_entry_is_claimed_once_by_guid_even_if_its_body_changes() {
        let (pool, user_id) = fixture().await;
        let integration_id: Uuid = sqlx::query_scalar("SELECT id FROM integrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        let feed = create_rss_feed(
            &pool,
            user_id,
            "https://example.com/feed.xml",
            integration_id,
            "F",
            false,
            true,
        )
        .await
        .unwrap();

        let first = insert_rss_post(&pool, feed.id, "guid-1", "T", "u", None, "hash-a")
            .await
            .unwrap();
        assert!(first.is_some(), "a new guid is claimed");

        // Same guid with an edited body: not imported again.
        let edited = insert_rss_post(&pool, feed.id, "guid-1", "T", "u", None, "hash-b")
            .await
            .unwrap();
        assert!(edited.is_none(), "an edited entry must not be re-imported");

        // Releasing the claim lets a later poll claim it again.
        delete_rss_post(&pool, first.unwrap()).await.unwrap();
        let retried = insert_rss_post(&pool, feed.id, "guid-1", "T", "u", None, "hash-b")
            .await
            .unwrap();
        assert!(retried.is_some(), "a released claim is retried");
    }

    #[tokio::test]
    async fn media_alt_text_round_trips() {
        let (pool, user_id) = fixture().await;
        let stored = create_media(
            &pool,
            user_id,
            "bike.png",
            "bike.png",
            "image/png",
            10,
            None,
            None,
            Some("a red bike"),
        )
        .await
        .unwrap();
        assert_eq!(stored.alt_text.as_deref(), Some("a red bike"));

        let read = get_media(&pool, stored.id)
            .await
            .unwrap()
            .expect("media row");
        assert_eq!(read.alt_text.as_deref(), Some("a red bike"));
    }

    #[tokio::test]
    async fn next_free_slot_fallback_is_never_in_the_past() {
        // No posting_times: the fallback used to return last + 2h, so a last post
        // from three days ago gave a slot three days in the past.
        let (pool, user_id) = fixture().await;
        let integration_id: Uuid = sqlx::query_scalar("SELECT id FROM integrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        insert(&pool, Some(Utc::now() - chrono::Duration::days(3))).await;

        let slot = find_next_free_slot(&pool, user_id, Some(integration_id))
            .await
            .unwrap()
            .expect("fallback always yields a slot");
        assert!(slot > Utc::now(), "slot {slot} is in the past");
    }

    #[tokio::test]
    async fn the_scheduler_claims_past_due_and_ignores_the_future() {
        let (pool, _) = fixture().await;
        let past = insert(&pool, Some(Utc::now() - chrono::Duration::days(1))).await;
        let _future = insert(&pool, Some(Utc::now() + chrono::Duration::days(1))).await;

        let due = get_due_posts(&pool, 10).await.unwrap();
        assert_eq!(due.len(), 1, "only the past-due post is claimed");
        assert_eq!(due[0].id, past);
        // The claim is what actually drives publishing — not just a read.
        let state: String = sqlx::query_scalar("SELECT state FROM posts WHERE id = ?")
            .bind(past)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(state, "publishing");
    }

    #[tokio::test]
    async fn a_legacy_text_row_is_still_claimed() {
        // Defence in depth for rows written before migration 0002: the
        // predicate parses TEXT instead of relying on the migration alone.
        let (pool, user_id) = fixture().await;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO posts (id, user_id, integration_id, state, content, media,
                                settings, scheduled_at, idempotency_key)
             VALUES (?, ?, (SELECT id FROM integrations), 'queued', 'legacy text row',
                     '[]', '{}', ?, ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(
            (Utc::now() - chrono::Duration::days(2))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();

        let due = get_due_posts(&pool, 10).await.unwrap();
        assert_eq!(
            due.len(),
            1,
            "the TEXT row is claimed, not silently starved"
        );
        assert_eq!(due[0].id, id);
    }
}
/// Phase-1 regression coverage (audit 2026-10-14):
/// - B3: thread predecessor lookup is scoped to the integration.
/// - B8: update_integration_token is the single encrypting writer —
///   ciphertext at rest, rotated refresh stored, and a provider
///   that passes no rotated refresh token never wipes the stored one.
#[cfg(test)]
mod b03_b08_tests {
    use super::*;

    async fn fixture() -> (SqlitePool, Uuid) {
        let pool = crate::db::create_pool("sqlite::memory:").await.unwrap();
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        crate::db::ensure_local_user(&pool).await.unwrap();
        (pool, user_id)
    }

    async fn integration(pool: &SqlitePool, user_id: Uuid, internal_id: &str) -> Uuid {
        create_integration(
            pool,
            user_id,
            "x",
            "X",
            internal_id,
            "tok",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap()
        .id
    }

    async fn published_part(
        pool: &SqlitePool,
        user_id: Uuid,
        integration_id: Uuid,
        group_id: Uuid,
        sequence: i32,
        platform_post_id: &str,
    ) -> Uuid {
        let post = create_post(
            pool,
            user_id,
            integration_id,
            "part",
            None,
            &serde_json::json!([]),
            &serde_json::json!({}),
            None,
            Some(PostState::Published),
            None,
            sequence,
            None,
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE posts SET group_id = ?, platform_post_id = ?,
             platform_post_url = ?, published_at = unixepoch() WHERE id = ?",
        )
        .bind(group_id)
        .bind(platform_post_id)
        .bind(format!("https://example.com/{platform_post_id}"))
        .bind(post.id)
        .execute(pool)
        .await
        .unwrap();
        post.id
    }

    /// B3: one `group_id` is shared across both integrations (exactly what
    /// `create_thread_posts` produces for a multi-platform thread). Each
    /// integration's part 2 must resolve to ITS OWN part 1 — without the
    /// `integration_id` filter the lookup could hand part 2 on account A
    /// part 1's platform id from account B.
    #[tokio::test]
    async fn thread_predecessor_is_scoped_to_the_integration() {
        let (pool, user_id) = fixture().await;
        let a = integration(&pool, user_id, "acct-a").await;
        let b = integration(&pool, user_id, "acct-b").await;
        let group = Uuid::new_v4();

        published_part(&pool, user_id, a, group, 1, "pid-a-1").await;
        published_part(&pool, user_id, b, group, 1, "pid-b-1").await;

        let for_a = find_thread_predecessor(&pool, group, a, 1)
            .await
            .expect("lookup");
        let for_b = find_thread_predecessor(&pool, group, b, 1)
            .await
            .expect("lookup");

        assert_eq!(
            for_a.as_deref(),
            Some("pid-a-1"),
            "account A must reply to its own part 1"
        );
        assert_eq!(
            for_b.as_deref(),
            Some("pid-b-1"),
            "account B must reply to its own part 1"
        );

        // The other integration's part 1 is NOT a valid predecessor.
        assert_eq!(
            find_thread_predecessor(&pool, group, a, 2).await.unwrap(),
            None,
            "no seq-2 predecessor exists yet — must be None, not pid-b-1"
        );
    }

    /// B3: an unpublished predecessor yields None (the scheduler leaves the
    /// part queued instead of posting standalone).
    #[tokio::test]
    async fn thread_predecessor_none_until_predecessor_publishes() {
        let (pool, user_id) = fixture().await;
        let a = integration(&pool, user_id, "acct-a").await;
        let group = Uuid::new_v4();

        // Part 1 still queued — no platform_post_id yet.
        create_post(
            &pool,
            user_id,
            a,
            "part 1",
            None,
            &serde_json::json!([]),
            &serde_json::json!({}),
            None,
            Some(PostState::Queued),
            None,
            1,
            None,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE posts SET group_id = ? WHERE sequence = 1")
            .bind(group)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            find_thread_predecessor(&pool, group, a, 1).await.unwrap(),
            None,
            "a queued predecessor must not be linkable"
        );
    }

    /// B8: with a key configured, the stored access token is ciphertext
    /// (decrypts to the plaintext we passed), and a rotated refresh token
    /// is stored encrypted.
    #[tokio::test]
    async fn token_update_encrypts_at_rest_and_stores_rotated_refresh() {
        let (pool, user_id) = fixture().await;
        let id = integration(&pool, user_id, "acct-a").await;
        let key = [7u8; 32];

        update_integration_token(
            &pool,
            id,
            "new-access",
            Some("rotated-refresh"),
            None,
            Some(key),
        )
        .await
        .expect("token update");

        let (raw_access, raw_refresh): (String, String) =
            sqlx::query_as("SELECT access_token, refresh_token FROM integrations WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_ne!(
            raw_access, "new-access",
            "access token must not be plaintext at rest"
        );
        assert_eq!(
            crate::crypto::decrypt_string(&raw_access, &key).unwrap(),
            "new-access",
            "ciphertext must decrypt back to the token we passed"
        );
        assert_ne!(
            raw_refresh, "rotated-refresh",
            "rotated refresh token must not be plaintext at rest"
        );
        assert_eq!(
            crate::crypto::decrypt_string(&raw_refresh, &key).unwrap(),
            "rotated-refresh"
        );
    }

    /// B8: providers that do not rotate refresh tokens (Reddit's are
    /// permanent) pass `None` — the previously stored token must survive
    /// (the old write set `refresh_token = NULL` and wiped it).
    #[tokio::test]
    async fn token_update_without_rotation_keeps_the_stored_refresh_token() {
        let (pool, user_id) = fixture().await;
        let id = integration(&pool, user_id, "acct-a").await;
        let key = [7u8; 32];

        // Seed a stored refresh token first (rotated write).
        update_integration_token(
            &pool,
            id,
            "access-1",
            Some("stored-refresh"),
            None,
            Some(key),
        )
        .await
        .expect("seed");

        // A later refresh whose provider returned no rotated token.
        update_integration_token(&pool, id, "access-2", None, None, Some(key))
            .await
            .expect("token update");

        let raw_refresh: String =
            sqlx::query_scalar("SELECT refresh_token FROM integrations WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let refresh = crate::crypto::decrypt_string(&raw_refresh, &key)
            .unwrap_or_else(|_| raw_refresh.clone());
        assert_eq!(
            refresh, "stored-refresh",
            "a non-rotating provider must not wipe the stored refresh token"
        );
    }

    /// B8: without a key configured the tokens are stored as-is (the
    /// single writer must not break unencrypted single-user installs).
    #[tokio::test]
    async fn token_update_without_key_stores_plaintext() {
        let (pool, user_id) = fixture().await;
        let id = integration(&pool, user_id, "acct-a").await;

        update_integration_token(&pool, id, "plain-access", Some("plain-refresh"), None, None)
            .await
            .expect("token update");

        let (access, refresh): (String, String) =
            sqlx::query_as("SELECT access_token, refresh_token FROM integrations WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(access, "plain-access");
        assert_eq!(refresh, "plain-refresh");
    }

    /// B13: `set_post_recurring_with_copies` — the shared path BOTH REST
    /// `/api/posts/{id}/repeat` and the MCP `posts_repeat` tool call — must
    /// create real queued copies at spaced dates and stamp the series on the
    /// original. The pre-fix MCP tool only wrote `repeat_interval_days`,
    /// which nothing read: zero copies, ever.
    #[tokio::test]
    async fn repeat_creates_spaced_copies_and_stamps_the_series() {
        let (pool, user_id) = fixture().await;
        let integration_id = integration(&pool, user_id, "acct-a").await;

        let base = Utc::now();
        let post = create_post(
            &pool,
            user_id,
            integration_id,
            "evergreen",
            None,
            &serde_json::json!([]),
            &serde_json::json!({}),
            Some(base),
            Some(PostState::Queued),
            None,
            0,
            None,
        )
        .await
        .unwrap();

        // Every 2 days for 5 days → copies at +2d and +4d.
        let end = base + chrono::Duration::days(5);
        let group = Uuid::new_v4();
        let (ids, dates) =
            set_post_recurring_with_copies(&pool, post.id, user_id, 2, &end, group, &base)
                .await
                .unwrap();

        assert_eq!(ids.len(), 2, "copies at +2d/+4d, got {ids:?}");
        assert_eq!(dates.len(), 2);

        let (state, copy_days, copy_group, scheduled): (
            String,
            Option<i64>,
            Option<Uuid>,
            Option<i64>,
        ) = sqlx::query_as(
            "SELECT state, repeat_interval_days, group_id, scheduled_at FROM posts WHERE id = ?",
        )
        .bind(ids[0])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "queued",
            "copies must go out on schedule, not sit as drafts"
        );
        assert_eq!(
            copy_days, None,
            "copies must not themselves recur (runaway guard)"
        );
        assert_eq!(copy_group, Some(group), "copies join the series group");
        let delta = (scheduled.unwrap() - base.timestamp()).abs();
        assert!(
            (delta - 2 * 86400).abs() <= 1,
            "first copy due ~2 days after base, got {delta}s"
        );

        // The original carries the repeat contract + the same group.
        let (orig_days, orig_group): (Option<i64>, Option<Uuid>) =
            sqlx::query_as("SELECT repeat_interval_days, group_id FROM posts WHERE id = ?")
                .bind(post.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orig_days, Some(2));
        assert_eq!(orig_group, Some(group));
    }

    /// E5: the best-times histogram buckets imported posts with synced
    /// engagement by the platform post's UTC weekday×hour.
    #[tokio::test]
    async fn engagement_histogram_buckets_by_weekday_and_hour() {
        let (pool, user_id) = fixture().await;

        // Imported (external) posts — the engagement history source.
        // 2026-10-14 is a Wednesday (%w = 3); 2026-10-11 a Sunday (%w = 0).
        let wed_14 = chrono::DateTime::parse_from_rfc3339("2026-10-14T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let sun_09 = chrono::DateTime::parse_from_rfc3339("2026-10-11T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut ids = Vec::new();
        for (i, at) in [wed_14, sun_09].into_iter().enumerate() {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO external_posts (id, user_id, provider, platform_post_id, text, created_at)
                 VALUES (?, ?, 'x', ?, 'imported', ?)",
            )
            .bind(id)
            .bind(user_id)
            .bind(format!("plat-{i}"))
            .bind(at.timestamp())
            .execute(&pool)
            .await
            .unwrap();
            ids.push(id);
        }

        // Engagement on the Wednesday post only: 10 + 4 + 1 = 15.
        upsert_post_engagement(
            &pool,
            ids[0],
            &crate::social::EngagementRow {
                likes: 10,
                comments: 4,
                shares: 1,
                views: 0,
                saves: 0,
                quotes: 0,
                reposts: 0,
                replies: 0,
                reactions: serde_json::json!({}),
                upvotes: 0,
                downvotes: 0,
                upvote_ratio: None,
                awards: 0,
                raw: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        // The Sunday post has zero engagement everywhere — it must NOT
        // appear (INNER JOIN, > 0 filter).
        upsert_post_engagement(
            &pool,
            ids[1],
            &crate::social::EngagementRow {
                likes: 0,
                comments: 0,
                shares: 0,
                views: 0,
                saves: 0,
                quotes: 0,
                reposts: 0,
                replies: 0,
                reactions: serde_json::json!({}),
                upvotes: 0,
                downvotes: 0,
                upvote_ratio: None,
                awards: 0,
                raw: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

        let rows = engagement_histogram_by_slot(&pool, user_id, None, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "only the engaged post buckets: {rows:?}");
        let (provider, epoch, posts, engagement) = &rows[0];
        assert_eq!(provider, "x");
        // The caller folds the epoch into a weekday/hour slot in UTC
        // when no zone is given.
        use chrono::Timelike;
        let dt = DateTime::<Utc>::from_timestamp(*epoch, 0).unwrap();
        let (wd, h) = (
            dt.format("%w").to_string().parse::<i32>().unwrap(),
            dt.hour() as i32,
        );
        assert_eq!((wd, h), (3, 14), "Wednesday 14:00 UTC bucket");
        assert_eq!(*posts, 1);
        assert_eq!(*engagement, 15, "likes+comments+shares summed");

        // The provider filter narrows correctly.
        let filtered = engagement_histogram_by_slot(&pool, user_id, Some("reddit"), None)
            .await
            .unwrap();
        assert!(filtered.is_empty(), "filter must exclude other providers");
    }
}

/// Engagement for the user's OWN published posts, over a window.
///
/// Own-post engagement lives in `analytics_cache`, keyed by
/// `(user_id, platform_post_id)`: `/api/analytics/post/{id}` writes it
/// there after a live provider fetch. `post_engagement` is NOT
/// consulted - its `post_id` FK targets `external_posts`, so it can
/// never hold a row for a `posts` row.
///
/// The payload is provider-shaped JSON: a list of {label, data:[{total}]}
/// series with per-provider metric labels ("Likes", "Comments",
/// "Shares", "Impressions"). Each metric is the LAST point of its
/// series (the "latest total" shape X, YouTube and TikTok all emit).
///
/// Returns one row per metric, so the caller can total across metrics
/// without re-parsing the JSON.
pub async fn own_post_engagement_totals(
    pool: &SqlitePool,
    user_id: Uuid,
    cutoff: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<OwnPostEngagementTotals, sqlx::Error> {
    // The metric expressions come from `metric_value_expr` - the ONE
    // implementation every own-post engagement reader shares: the last
    // point of a cumulative series, labels mapped by
    // `metric_label_matches`. It used to match only
    // 'likes'/'comments'/'shares', which never matched X ("Retweets",
    // "Replies") or Bluesky ("Reposts", "Replies") - so those users read
    // 0 comments and 0 shares forever.
    let sql = format!(
        r#"SELECT
             COALESCE(SUM(likes), 0)    AS likes,
             COALESCE(SUM(comments), 0) AS comments,
             COALESCE(SUM(shares), 0)   AS shares,
             COALESCE(SUM(impressions), 0) AS impressions
           FROM (
             SELECT p.id AS post_id,
                    {likes} AS likes,
                    {comments} AS comments,
                    {shares} AS shares,
                    {impressions} AS impressions
               FROM posts p
              WHERE p.user_id = ?
                AND p.deleted_at IS NULL
                AND p.state = 'published'
                AND p.published_at >= ?
                AND p.published_at < ?)"#,
        likes = metric_value_expr(METRIC_LIKES, "p"),
        comments = metric_value_expr(METRIC_COMMENTS, "p"),
        shares = metric_value_expr(METRIC_SHARES, "p"),
        impressions = metric_value_expr(METRIC_IMPRESSIONS, "p"),
    );
    sqlx::query_as::<_, OwnPostEngagementTotals>(&sql)
        .bind(user_id)
        .bind(EpochUtc::from(cutoff))
        .bind(EpochUtc::from(until))
        .fetch_one(pool)
        .await
}

/// The canonical metric each provider's series label belongs to.
///
/// The providers do NOT share a vocabulary: X reports "Retweets" and
/// "Replies", Bluesky reports "Reposts" and "Replies", LinkedIn reports
/// lowercase "likes"/"comments"/"shares", TikTok reports "Comments" and
/// "Shares". The readers used to match `lower(label) IN ('likes',
/// 'comments', 'shares')`, which only ever matched LinkedIn and TikTok -
/// so for an X or Bluesky user the "Comments" and "Shares" headline
/// cards read 0 forever, and an X post with 500 retweets scored ZERO
/// engagement and was excluded from the ranking entirely by
/// `WHERE engagement > 0`.
///
/// This is the ONE place the vocabulary is mapped. Every reader uses it.
pub const METRIC_LIKES: &str = "likes";
pub const METRIC_COMMENTS: &str = "comments";
pub const METRIC_SHARES: &str = "shares";
pub const METRIC_IMPRESSIONS: &str = "impressions";

/// The SQL predicate matching a series label to a canonical metric.
///
/// `lower(label) = 'likes'` AND the synonyms for that metric.
pub fn metric_label_matches(metric: &str) -> String {
    match metric {
        METRIC_LIKES => {
            "lower(json_extract(series.value, '$.label')) IN ('likes', 'like', 'reactions', 'favorites')"
                .to_string()
        }
        METRIC_COMMENTS => {
            "lower(json_extract(series.value, '$.label')) IN ('comments', 'comment', 'replies', 'reply')"
                .to_string()
        }
        METRIC_SHARES => {
            "lower(json_extract(series.value, '$.label')) IN ('shares', 'share', 'retweets', 'reposts', 'repost', 'quotes')"
                .to_string()
        }
        METRIC_IMPRESSIONS => {
            "lower(json_extract(series.value, '$.label')) IN ('impressions', 'impression', 'views', 'view')"
                .to_string()
        }
        _ => "lower(json_extract(series.value, '$.label')) = '".to_string() + metric + "'",
    }
}

/// The SQL expression reading the CURRENT value of the series that
/// belongs to `metric` for one post: the LAST point of the series.
///
/// The provider series (X, YouTube, TikTok) are CUMULATIVE running
/// totals, so the current value is the last point - NOT the SUM of the
/// points, which multiplies a running total by the number of samples
/// (a post with 70 likes read as 520 across 10 points). This is the
/// ONE implementation every own-post engagement reader uses, so they
/// cannot disagree about which labels mean what or which point is
/// "current". It returns NULL when the post has no cached series -
/// "not measured" must stay NULL, not 0.
pub fn metric_value_expr(metric: &str, post_alias: &str) -> String {
    format!(
        "(SELECT json_extract(point.value, '$.total')
           FROM analytics_cache ac, json_each(ac.data) AS series,
                json_each(json_extract(series.value, '$.data')) AS point
          WHERE ac.user_id = {post_alias}.user_id
            AND ac.platform_post_id = {post_alias}.platform_post_id
            AND {}
          ORDER BY point.key DESC LIMIT 1)",
        metric_label_matches(metric)
    )
}

/// The per-metric totals of the user's own published posts.
#[derive(Debug, sqlx::FromRow)]
pub struct OwnPostEngagementTotals {
    pub likes: i64,
    pub comments: i64,
    pub shares: i64,
    pub impressions: i64,
}

/// The engagement of ONE of the user's own posts.
#[derive(Debug, sqlx::FromRow)]
pub struct PostEngagementRow {
    pub post_id: Uuid,
    pub likes: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub impressions: Option<i64>,
}

/// The engagement of each of the given posts, keyed by post id.
///
/// The same source as [`own_post_engagement_totals`]: `analytics_cache`,
/// keyed by `(user_id, platform_post_id)`. A post with no cached
/// analytics has NO row here - "not measured" is not "zero", and a list
/// that renders a fabricated zero is worse than one that renders
/// nothing.
///
/// Each metric is the LAST point of its series, which is the provider's
/// running total - the current value.
pub async fn post_engagement_by_post(
    pool: &SqlitePool,
    user_id: Uuid,
    post_ids: &[Uuid],
) -> Result<Vec<PostEngagementRow>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }
    let holes = std::iter::repeat_n("?", post_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r#"SELECT p.id AS post_id,
                  {likes} AS likes,
                  {comments} AS comments,
                  {shares} AS shares,
                  {impressions} AS impressions
             FROM posts p
            WHERE p.user_id = ?
              AND p.deleted_at IS NULL
              AND p.id IN ({holes})"#,
        holes = holes,
        likes = metric_value_expr(METRIC_LIKES, "p"),
        comments = metric_value_expr(METRIC_COMMENTS, "p"),
        shares = metric_value_expr(METRIC_SHARES, "p"),
        impressions = metric_value_expr(METRIC_IMPRESSIONS, "p"),
    );
    let mut q = sqlx::query_as::<_, PostEngagementRow>(&sql);
    q = q.bind(user_id);
    for id in post_ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await
}

/// Research: the user's own published posts ranked by engagement.
///
/// Engagement for the user's own posts lives in `analytics_cache`, keyed by
/// `(user_id, platform_post_id)`: the cache-first `/api/analytics/post/{id}`
/// endpoint writes it there after a live provider fetch. `post_engagement`
/// is not consulted — its `post_id` FK targets `external_posts`, so it can
/// never hold a row for a `posts` row.
///
/// The payload is provider-shaped JSON: a list of {label, data:[{total}]}
/// series with per-provider metric labels ("Likes", "Comments", "Shares").
/// Rather than map every provider's labels, match the three metric names
/// case-insensitively and sum the points of each matching series. Labels that
/// match nothing ("Impressions", "Views") are deliberately excluded: reach
/// is not engagement, and counting it here would rank reach-heavy posts above
/// posts people actually interacted with.
pub async fn top_own_posts_by_engagement(
    pool: &SqlitePool,
    user_id: Uuid,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<OwnPostEngagement>, sqlx::Error> {
    // The analytics_cache payload is provider-shaped JSON: a list of
    // {label, data:[{total}]} series with per-provider metric labels
    // ("Likes", "Comments", "Shares"). Rather than map every provider's
    // labels, match the three metric names case-insensitively and read the
    // last point of each series (the same "latest total" shape X, YouTube
    // and TikTok all emit). Series that match nothing contribute 0.
    // The subquery exists so `engagement > 0` can filter the computed
    // column: SQLite has no HAVING without GROUP BY, and a "top posts"
    // ranking must not list a post whose engagement was never measured —
    // an unmeasured post is unknown, not zero, and ranking it last would
    // pass it off as evidence of poor performance.
    // The label set comes from `metric_label_matches` - the one place
    // the provider vocabulary is mapped. Matching only
    // 'likes'/'comments'/'shares' meant an X post with 500 retweets and
    // 2 replies scored 2, and a Bluesky post with 300 reposts scored 0 -
    // so `WHERE engagement > 0` EXCLUDED them from the ranking
    // entirely.
    let sql = format!(
        r#"SELECT post_id, content, provider, published_at, engagement
          FROM (SELECT p.id AS post_id,
                       p.content,
                       i.provider_identifier AS provider,
                       p.published_at,
                       COALESCE({likes} + {comments} + {shares}, 0) AS engagement
                  FROM posts p
                  JOIN integrations i ON p.integration_id = i.id
                 WHERE p.user_id = ?
                   AND p.deleted_at IS NULL
                   AND p.state = 'published'
                   AND p.published_at >= ?)
         WHERE engagement > 0
         ORDER BY engagement DESC, published_at DESC
         LIMIT ?"#,
        likes = metric_value_expr(METRIC_LIKES, "p"),
        comments = metric_value_expr(METRIC_COMMENTS, "p"),
        shares = metric_value_expr(METRIC_SHARES, "p"),
    );
    sqlx::query_as::<_, OwnPostEngagement>(&sql)
        .bind(user_id)
        .bind(EpochUtc::from(cutoff))
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Research: recent imported external posts ranked by engagement.
///
/// "Recent" is the platform post's own timestamp, and hidden posts are
/// excluded so a soft-hidden competitor post stops influencing the ranking.
pub async fn top_external_posts_by_engagement(
    pool: &SqlitePool,
    user_id: Uuid,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ExternalPostEngagement>, sqlx::Error> {
    sqlx::query_as::<_, ExternalPostEngagement>(
        r#"SELECT ep.id AS post_id,
               ep.provider,
               ep.text,
               ep.author_name,
               ep.author_handle,
               ep.url,
               ep.created_at,
               COALESCE(SUM(COALESCE(pe.likes, 0) + COALESCE(pe.comments, 0)
                            + COALESCE(pe.shares, 0) + COALESCE(pe.replies, 0)
                            + COALESCE(pe.reposts, 0) + COALESCE(pe.quotes, 0)
                            + COALESCE(pe.saves, 0) + COALESCE(pe.upvotes, 0)), 0) AS engagement
          FROM external_posts ep
          JOIN post_engagement pe ON pe.post_id = ep.id
         WHERE ep.user_id = ?
           AND ep.hidden_at IS NULL
           AND ep.created_at >= ?
         GROUP BY ep.id
        HAVING engagement > 0
         ORDER BY engagement DESC, ep.created_at DESC
         LIMIT ?"#,
    )
    .bind(user_id)
    .bind(EpochUtc::from(cutoff))
    .bind(limit)
    .fetch_all(pool)
    .await
}
// ── Research aggregation result types ──────────────────────────

/// One of the user's own posts with its engagement total, for the
/// research surface's "top performing posts" section.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct OwnPostEngagement {
    pub post_id: Uuid,
    pub content: String,
    /// Provider the post published through (from its integration), since
    /// `posts` has no provider column of its own.
    pub provider: String,
    pub published_at: Option<DateTime<Utc>>,
    pub engagement: i64,
}

/// One imported external post with its engagement total, for the research
/// surface's "recent competitor posts" section.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ExternalPostEngagement {
    pub post_id: Uuid,
    pub provider: String,
    pub text: String,
    pub author_name: Option<String>,
    pub author_handle: Option<String>,
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub engagement: i64,
}

/// A word that appears in external/competitor posts but not in any of the
/// user's own posts. The count is how many external posts (in the window)
/// used it, so the caller can rank and drop one-off noise.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ExternalWordGap {
    pub word: String,
    pub external_posts: i64,
}

// ══════════════════════════════════════════════════════════════
// ONE-TIME AUTH CODES (v12)
// ══════════════════════════════════════════════════════════════

/// How long a one-time setup code lives. Long enough for a founder to click
/// a link; short enough that a leaked link is worthless quickly.
const AUTH_CODE_TTL_SECS: i64 = 30 * 60;

/// Mint a single-use code for `user_id`.
pub async fn create_auth_code(pool: &SqlitePool, user_id: Uuid) -> Result<String, sqlx::Error> {
    let code = format!("ac_{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO auth_codes (code, user_id, expires_at) VALUES (?, ?, unixepoch() + ?)",
    )
    .bind(&code)
    .bind(user_id)
    .bind(AUTH_CODE_TTL_SECS)
    .execute(pool)
    .await?;
    Ok(code)
}

/// Resolve a code for the user it belongs to.
///
/// A code is NOT single-use: the setup page renders a form and then the
/// user POSTs it, so the same code is presented twice inside one 30-minute
/// window. Consuming it on the first read would break that flow, and the
/// security win does not come from single-use — it comes from the code
/// being worth something for half an hour instead of for a month.
pub async fn exchange_auth_code(pool: &SqlitePool, code: &str) -> Result<Uuid, sqlx::Error> {
    let row: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT user_id FROM auth_codes
           WHERE code = ? AND expires_at > unixepoch()"#,
    )
    .bind(code.trim())
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| sqlx::Error::RowNotFound)
}

/// Drop codes that are spent or stale. Called by the janitor.
pub async fn prune_auth_codes(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "DELETE FROM auth_codes WHERE used_at IS NOT NULL OR expires_at <= unixepoch()",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// v12: the ONE encrypting writer for integration tokens.
///
/// `create_integration` used to store whatever it was handed. Only the
/// OAuth callback encrypted, so every other connect path (direct connect,
/// public onboarding, the X cookie store, the Telegram bot token, the page
/// picker) left a PLAINTEXT working credential in the row. These tests
/// pin the new contract: the writer encrypts, and the startup pass that
/// re-encrypts older rows is idempotent and never double-encrypts.
#[cfg(test)]
mod v12_auth_code_tests {
    use super::*;

    async fn fixture() -> (SqlitePool, Uuid) {
        let pool = crate::db::create_pool("sqlite::memory:").await.unwrap();
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        crate::db::ensure_local_user(&pool).await.unwrap();
        (pool, user_id)
    }

    #[tokio::test]
    async fn a_mintcode_resolves_to_the_user_it_was_minted_for() {
        let (pool, user_id) = fixture().await;
        let code = create_auth_code(&pool, user_id).await.expect("mint");
        assert!(code.starts_with("ac_"), "got {code}");
        assert_eq!(
            exchange_auth_code(&pool, &code).await.expect("resolve"),
            user_id
        );
    }

    #[tokio::test]
    async fn the_same_code_resolves_more_than_once_inside_its_window() {
        // The setup form posts the same credential back inside the same
        // 30-minute window, so a single-use code would break it.
        let (pool, user_id) = fixture().await;
        let code = create_auth_code(&pool, user_id).await.expect("mint");
        assert_eq!(
            exchange_auth_code(&pool, &code).await.expect("first"),
            user_id
        );
        assert_eq!(
            exchange_auth_code(&pool, &code).await.expect("second"),
            user_id
        );
    }

    #[tokio::test]
    async fn an_expired_or_unknown_code_does_not_resolve() {
        let (pool, user_id) = fixture().await;
        let code = create_auth_code(&pool, user_id).await.expect("mint");
        sqlx::query("UPDATE auth_codes SET expires_at = unixepoch() - 1")
            .execute(&pool)
            .await
            .expect("expire");
        assert!(exchange_auth_code(&pool, &code).await.is_err());
        assert!(exchange_auth_code(&pool, "ac_nope").await.is_err());
    }

    #[tokio::test]
    async fn pruning_removes_only_the_stale_codes() {
        let (pool, user_id) = fixture().await;
        let live = create_auth_code(&pool, user_id).await.expect("live");
        let stale = create_auth_code(&pool, user_id).await.expect("stale");
        sqlx::query("UPDATE auth_codes SET expires_at = unixepoch() - 1 WHERE code = ?")
            .bind(&stale)
            .execute(&pool)
            .await
            .expect("expire one");
        let pruned = prune_auth_codes(&pool).await.expect("prune");
        assert_eq!(pruned, 1);
        assert_eq!(
            exchange_auth_code(&pool, &live).await.expect("still live"),
            user_id
        );
        assert!(exchange_auth_code(&pool, &stale).await.is_err());
    }
}

#[cfg(test)]
mod v12_token_encryption_tests {
    use super::*;

    fn key_a() -> [u8; 32] {
        let mut k = [0u8; 32];
        k.copy_from_slice(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        k
    }

    fn key_b() -> [u8; 32] {
        let mut k = [0u8; 32];
        k.copy_from_slice(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        k
    }

    async fn fixture() -> (SqlitePool, Uuid) {
        let pool = crate::db::create_pool("sqlite::memory:").await.unwrap();
        let user_id = crate::auth::middleware::DEFAULT_USER_ID;
        crate::db::ensure_local_user(&pool).await.unwrap();
        (pool, user_id)
    }

    async fn create(
        pool: &SqlitePool,
        user_id: Uuid,
        internal_id: &str,
        token: &str,
        token_key: Option<[u8; 32]>,
    ) -> Uuid {
        create_integration(
            pool,
            user_id,
            "facebook",
            "Facebook",
            internal_id,
            token,
            Some("refresh-token"),
            None,
            None,
            None,
            None,
            None,
            None,
            token_key,
        )
        .await
        .unwrap()
        .id
    }

    async fn stored(pool: &SqlitePool, id: Uuid) -> (String, Option<String>) {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT access_token, refresh_token FROM integrations WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_integration_encrypts_the_token_when_a_key_is_set() {
        let (pool, user_id) = fixture().await;
        let id = create(&pool, user_id, "page-1", "super-secret", Some(key_a())).await;
        let (access, refresh) = stored(&pool, id).await;

        assert_ne!(
            access, "super-secret",
            "the token must not be stored in the clear"
        );
        assert_eq!(
            crate::crypto::decrypt_string(&access, &key_a()).unwrap(),
            "super-secret"
        );
        assert_eq!(
            crate::crypto::decrypt_string(refresh.as_deref().unwrap(), &key_a()).unwrap(),
            "refresh-token"
        );
    }

    #[tokio::test]
    async fn create_integration_stores_plaintext_when_no_key_is_set() {
        let (pool, user_id) = fixture().await;
        let id = create(&pool, user_id, "page-2", "plain-token", None).await;
        let (access, _) = stored(&pool, id).await;
        assert_eq!(access, "plain-token");
    }

    #[tokio::test]
    async fn reencrypt_turns_a_plaintext_row_into_ciphertext() {
        let (pool, user_id) = fixture().await;
        let id = create(&pool, user_id, "page-3", "legacy-plain", None).await;

        let changed = reencrypt_plaintext_tokens(&pool, key_a()).await.unwrap();
        assert_eq!(changed, 1, "exactly the plaintext row should be rewritten");

        let (access, refresh) = stored(&pool, id).await;
        assert_eq!(
            crate::crypto::decrypt_string(&access, &key_a()).unwrap(),
            "legacy-plain"
        );
        assert_eq!(
            crate::crypto::decrypt_string(refresh.as_deref().unwrap(), &key_a()).unwrap(),
            "refresh-token"
        );
    }

    #[tokio::test]
    async fn reencrypt_is_idempotent() {
        let (pool, user_id) = fixture().await;
        let id = create(&pool, user_id, "page-4", "already-encrypted", Some(key_a())).await;
        let before = stored(&pool, id).await;

        let changed = reencrypt_plaintext_tokens(&pool, key_a()).await.unwrap();
        assert_eq!(changed, 0, "an encrypted row must be left alone");
        assert_eq!(stored(&pool, id).await, before);
    }

    #[tokio::test]
    async fn reencrypt_never_double_encrypts_a_row_written_with_another_key() {
        let (pool, user_id) = fixture().await;
        let id = create(&pool, user_id, "page-5", "other-key", Some(key_a())).await;
        let before = stored(&pool, id).await;

        let changed = reencrypt_plaintext_tokens(&pool, key_b()).await.unwrap();
        assert_eq!(
            changed, 0,
            "a row that looks encrypted must not be rewritten"
        );
        assert_eq!(
            stored(&pool, id).await,
            before,
            "the ciphertext must survive a pass with the wrong key"
        );
        assert_eq!(
            crate::crypto::decrypt_string(&before.0, &key_a()).unwrap(),
            "other-key"
        );
    }
}
