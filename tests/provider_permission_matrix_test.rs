//! The permission matrix: a provider that can DELETE must ask for the
//! scope that gates the delete.
//!
//! A permission gap has two shapes, and this test catches both:
//!
//!   1. the provider CAN delete (it implements `delete_published`) but the
//!      consent never asks for the scope that gates the delete — so the
//!      delete is refused by the platform at runtime;
//!   2. the consent asks for a scope NOTHING reads — a gate nobody can
//!      pass, and a request Meta's App Review has to approve for nothing.
//!
//! The table is the platforms' own documentation of which scope gates
//! which operation. It is a table, not a comment, so a provider that
//! grows a delete without growing its scopes fails the build.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// (provider file stem, the scope that gates its DELETE, why).
const DELETE_SCOPES: &[(&str, &str, &str)] = &[
    (
        "reddit",
        "edit",
        "Reddit gates /api/del on the `edit` scope",
    ),
    (
        "threads",
        "threads_delete",
        "Threads gates DELETE /{thread-id} on `threads_delete`",
    ),
    (
        "x",
        "tweet.write",
        "X gates DELETE /2/tweets/:id on `tweet.write`",
    ),
    (
        "facebook",
        "pages_manage_posts",
        "Facebook gates a page-post delete on `pages_manage_posts`",
    ),
    (
        "instagram",
        "instagram_content_publish",
        "Instagram gates a media delete on `instagram_content_publish`",
    ),
    (
        "linkedin",
        "w_member_social",
        "LinkedIn gates DELETE /ugcPosts on `w_member_social`",
    ),
    (
        "linkedin_page",
        "w_organization_social",
        "LinkedIn gates an organization-post delete on `w_organization_social`",
    ),
    (
        "youtube",
        "https://www.googleapis.com/auth/youtube",
        "YouTube gates DELETE /youtube/v3/videos on the `youtube` scope",
    ),
    (
        "tiktok",
        "video.delete",
        "TikTok gates /v2/video/delete on `video.delete`",
    ),
    (
        "mastodon",
        "write",
        "Mastodon gates DELETE /api/v1/statuses/:id on `write:statuses`",
    ),
    (
        "pinterest",
        "pins:write",
        "Pinterest gates DELETE /v5/pins/:id on `pins:write`",
    ),
];

/// The scope that gates the media UPLOAD, where the provider uploads media.
const MEDIA_SCOPES: &[(&str, &str, &str)] = &[(
    "x",
    "media.write",
    "X gates the media upload on `media.write`",
)];

fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The scope strings a provider's `scopes()` returns.
fn scopes_of(stem: &str) -> Vec<String> {
    let src = std::fs::read_to_string(repo_path(&format!("src/social/{stem}.rs")))
        .unwrap_or_else(|e| panic!("src/social/{stem}.rs must be readable: {e}"));
    let start = src
        .find("fn scopes(&self)")
        .unwrap_or_else(|| panic!("{stem} has no scopes() — it asks for no permission at all"));
    let body = &src[start..];
    let end = body.find("\n    }").unwrap_or(body.len());
    // Skip DOCS, then take the quoted strings: a scope list is a list of
    // string literals, and the prose around it names scopes in backticks
    // that are NOT what the function asks for.
    let code = body[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<&str>>()
        .join("\n");
    code.split('"')
        .skip(1)
        .step_by(2)
        .map(|s| s.to_string())
        .collect()
}

fn implements_delete(stem: &str) -> bool {
    let src = std::fs::read_to_string(repo_path(&format!("src/social/{stem}.rs"))).unwrap();
    src.contains("async fn delete_published")
}

#[test]
fn a_provider_that_can_delete_asks_for_the_scope_that_gates_the_delete() {
    for (stem, scope, why) in DELETE_SCOPES {
        let src = std::fs::read_to_string(repo_path(&format!("src/social/{stem}.rs"))).unwrap();
        assert!(
            implements_delete(stem),
            "{stem} is in the delete matrix but does not implement delete_published — {why}"
        );
        let scopes = scopes_of(stem);
        assert!(
            scopes.iter().any(|s| s == scope),
            "{stem} can delete ({why}) but its consent does not ask for `{scope}`. \
             The scopes it asks for: {scopes:?}. The delete is refused at runtime \
             without it."
        );
    }
}

#[test]
fn a_provider_that_uploads_media_asks_for_the_media_scope() {
    for (stem, scope, why) in MEDIA_SCOPES {
        let scopes = scopes_of(stem);
        assert!(
            scopes.iter().any(|s| s == scope),
            "{stem} uploads media ({why}) but its consent does not ask for `{scope}`. \
             Its scopes: {scopes:?}"
        );
    }
}

#[test]
fn no_provider_asks_for_a_scope_nothing_reads() {
    // The scopes the code actually READS out of a response or a request —
    // the list of things a permission can be FOR. A scope outside this list
    // is a gate nobody can pass.
    let read_or_used: HashMap<&str, Vec<&str>> = HashMap::from([
        (
            "reddit",
            vec![
                "identity",
                "read",
                "submit",
                "edit",
                "vote",
                "flair",
                "history",
                "mysubreddits",
                "privatemessages",
                "subscribe",
            ],
        ),
        (
            "threads",
            vec![
                "threads_basic",
                "threads_content_publish",
                "threads_manage_replies",
                "threads_read_replies",
                "threads_manage_insights",
                "threads_delete",
            ],
        ),
        (
            "x",
            vec![
                "tweet.read",
                "tweet.write",
                "users.read",
                "offline.access",
                "media.write",
                "like.read",
                "like.write",
                "bookmark.read",
                "bookmark.write",
                "follows.read",
                "follows.write",
                "list.read",
                "dm.read",
                "dm.write",
            ],
        ),
        (
            "facebook",
            vec![
                "pages_show_list",
                "pages_read_engagement",
                "pages_manage_posts",
                "business_management",
                "pages_manage_engagement",
                "pages_manage_metadata",
                "pages_read_user_content",
                "public_profile",
                "read_insights",
                "pages_messaging",
            ],
        ),
        (
            "instagram",
            vec![
                "instagram_basic",
                "instagram_content_publish",
                "instagram_manage_comments",
                "instagram_manage_insights",
                "instagram_manage_messages",
                // The page scopes: an Instagram Business account is reached
                // THROUGH a Facebook page, so the page-listing scopes are
                // part of what this provider reads.
                "pages_show_list",
                "pages_read_engagement",
                "business_management",
                "pages_manage_engagement",
                "pages_manage_metadata",
                "pages_read_user_content",
                "read_insights",
                "pages_messaging",
            ],
        ),
        (
            "linkedin",
            vec!["openid", "profile", "email", "w_member_social"],
        ),
        (
            "linkedin_page",
            vec![
                "openid",
                "profile",
                "email",
                "w_member_social",
                "rw_organization_admin",
                "w_organization_social",
                "r_organization_social",
            ],
        ),
        (
            "youtube",
            vec![
                "https://www.googleapis.com/auth/youtube",
                "https://www.googleapis.com/auth/youtube.upload",
                "https://www.googleapis.com/auth/youtube.force-ssl",
            ],
        ),
        (
            "tiktok",
            vec![
                "user.info.basic",
                "video.publish",
                "video.upload",
                "video.list",
                "video.delete",
            ],
        ),
        ("mastodon", vec!["read", "write", "follow", "push"]),
        (
            "pinterest",
            vec![
                "boards:read",
                "boards:write",
                "pins:read",
                "pins:write",
                "user_accounts:read",
            ],
        ),
        ("discord", vec!["identify", "guilds"]),
        (
            "slack",
            vec![
                "channels:history",
                "channels:read",
                "channels:manage",
                "chat:write",
                "chat:write.customize",
                "users:read",
                "users:read.email",
                "team:read",
            ],
        ),
        ("hashnode", vec!["write:posts"]),
    ]);

    for (stem, allowed) in &read_or_used {
        let scopes = scopes_of(stem);
        for scope in scopes {
            // A scope string that is not in the allowed list is either a
            // scope this table has not learned about yet or a dead one.
            // The table is the contract: add the scope AND its reader, or
            // drop the scope.
            assert!(
                allowed.contains(&scope.as_str())
                    || scope.starts_with("https://www.googleapis.com/auth/")
                    || scope.starts_with("http"),
                "{stem} asks for `{scope}`, which nothing in the provider reads. \
                 Either the provider has a reader for it (add it to this table) \
                 or the scope is a gate nobody can pass — drop it."
            );
        }
    }
}
