-- One-time codes standing in for a session credential in a URL.
--
-- `/setup` used to re-mint a 30-DAY session JWT and embed it in every
-- provider-card link (`?token=<jwt>`). A URL is not a safe place for a
-- credential: it lands in browser history, in the access log of every
-- proxy on the path, and in the `Referer` header the provider's redirect
-- sends back.
--
-- So the page mints a SHORT, SINGLE-USE code instead: it is useless 30
-- minutes later and cannot be replayed, which is the whole difference
-- between a leaked link and a leaked session.
--
-- `?token=<jwt>` is still accepted for the OAuth chains that were already
-- in flight; nothing breaks. New links carry the code.
CREATE TABLE auth_codes (
    code       TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER
);
