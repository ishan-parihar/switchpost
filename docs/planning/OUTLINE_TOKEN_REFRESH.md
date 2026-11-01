# OUTLINE — Token auto-refresh gaps (red-team + ponytail pass)

Generated 2026-10-31 from a red-team audit of `proactive_token_refresh`
(`src/scheduler/mod.rs:443`) and of every provider that opts in via
`needs_cron_refresh() -> true`.

This is the answer to "LinkedIn shows *needs reconnect* but its refresh is
probably flawed" plus the sweep the user asked for across all providers.

Nothing in this document is applied. It is the queue.

---

## The mechanism, in one paragraph

A scheduler tick calls `proactive_token_refresh` (`scheduler/mod.rs:443`).
It asks the registry for every provider whose `needs_cron_refresh()` is
`true` (12 of them; the trait default is `false`), then for each asks
`get_integrations_needing_refresh` for rows inside a 24h window
(`queries.rs`, predicate below), skips anything already flagged
`refresh_needed`, decrypts the refresh token, calls
`provider.refresh_token()`, and on `Ok` writes through the ONE encrypting
writer `update_integration_token`. On `Err` it calls
`mark_integration_refresh_needed`, which is what the channel UI renders as
"needs reconnect". The window predicate is:

```sql
WHERE provider_identifier = ? AND disabled = false
  AND refresh_needed = false
  AND refresh_token IS NOT NULL
  AND token_expires_at IS NOT NULL      -- ★ the silent gate
  AND token_expires_at <= now + 24h
```

---

## G-01 — `linkedin`: a cosmetic profile fetch discards a GOOD token ★ the reported bug

**Where.** `src/social/linkedin.rs:473`, inside `refresh_token`.

**Defect.** The refresh itself succeeds — LinkedIn returns a valid
`access_token`, and `expires_in` is parsed. The function then fetches
`https://api.linkedin.com/v2/userinfo` to keep the profile fresh, and
propagates its failure with `?`:

```rust
.json::<serde_json::Value>()
.await?;          // ← a 5xx here throws away the token above
```

**Consequence chain, live:**

1. `provider.refresh_token()` returns `Ok(valid_token)`.
2. The `userinfo` call fails — a transient LinkedIn 5xx, a rate limit, or a
   page-scoped token that was never granted `openid`/`profile`.
3. `?` converts that into `ProviderError::Http`, so the whole call reads as
   a failure.
4. `proactive_token_refresh` takes the `Err` arm and sets
   `refresh_needed = true`.
5. The channel row renders **"needs reconnect"**.
6. The good access token is never written, so the token stays expired and
   the founder re-authorizes by hand.

So the operator's report is exactly this, and "their auto-refresh mechanism
is flawed" is right for the wrong reason — the *refresh* works; the
*profile bookkeeping attached to it* is what fails.

**The fix already exists in the tree.** `linkedin_page.rs` — the sibling
provider for the same platform — does this correctly:

```rust
Ok(r) => r.json::<serde_json::Value>().await.ok(),   // profile is OPTIONAL
```

Its comment says why: *"page-scoped tokens may not have openid/profile
scope."* That is a deliberate design decision recorded on one file and
missing on the other. `linkedin` should do the same: a failed profile read
degrades the profile fields, never the refresh.

**Size.** ~10 lines changed, no new abstraction.

**Test.** The user's standing rule: a test that fails on pre-fix code.
`token_url()` is a private `&self -> &'static str`, so it is not a seam; the
providers are constructed from `Config`, so a test has to point an LD
mock at LinkedIn's HTTP. Cheapest honest option is to extract the profile
fetch into a small `profile_from(...) -> Option<...>` helper with the
`match` shape, and pin the *helper* rather than the HTTP — which is also the
ponytail answer to G-04 below.

---

## G-02 — the same defect at two more sites

The audit found the identical `?`-on-profile pattern at:

| Provider | Site | Note |
|---|---|---|
| `instagram_standalone` | `refresh_token`, fn at `:192` | same Meta-family shape as threads |
| `threads` | `refresh_token`, fn at `:186` | same shape, same consequence |

Each discards a valid token when the `/me` profile read fails. Both are
fixed by the same change as G-01.

`instagram_standalone` and `threads` are **consumer-facing** for the
founder (Instagram Reels and Threads are scheduled channels), so these are
not theoretical: an Instagram channel goes "needs reconnect" whenever
`me?fields=...` hiccups.

---

## G-03 — `refresh_needed` is sticky, and reclaim is manual

Once `mark_integration_refresh_needed` sets the flag, the 24h window query
excludes the row forever (`AND refresh_needed = false`). The only ways back
are the manual `POST /api/integrations/{id}/refresh` endpoint (which clears
the flag on success, `integrations.rs:500`) or a full reconnect.

**Consequence.** A single transient failure during a 24h window permanently
stops auto-refresh for that channel. The founder sees "needs reconnect" and
must act, even though the token may still have days left on it.

**Ponytail read.** This is a *policy* question, not a code defect: a
flag whose only writer marks it "give up" is a reasonable design if the
alternative is retrying a revoked token forever. The honest options are
(1) leave it, and make G-01/G-02 fix the cause; (2) add a bounded retry —
clear the flag on a *transport* error but keep it on an `Auth` error, which
is the distinction `PublishError`/`ProviderError` already make (`ProviderError::Auth`
vs `::Network`); (3) show the founder the *reason*, which the flag cannot.

Recommended: land G-01+G-02 first, then decide, because with the cause
fixed the flag is set far less often and the question changes.

**Do not** add a retry loop without the Auth/Network distinction: it turns
a revoked-token reconnect prompt into a silent retry-forever.

---

## G-04 — fix the cause in ONE place, not in three

Ponytail. The three sites in G-01/G-02 are the same rule in three files, and
there are now two correct idioms for it (`linkedin_page`'s `match`, and
whatever G-01 writes). One rule, one implementation:

```rust
/// A provider's profile read is BOOKEEPING. A failure here must never
/// fail the token refresh that carries it.
async fn profile_optional(
    http: &reqwest::Client,
    url: &str,
    bearer: &str,
) -> Option<serde_json::Value> { … }
```

Every Facebook-family / LinkedIn-family refresh then calls it, and the
`linkedin_page` `match` becomes the same call. Net: one implementation, three
call sites, and the next provider to refresh inherits the right behaviour
instead of copying the wrong one.

This is the same shape as the `google_oauth` unification (`a1ac73f`), and it
belongs next to it for that reason.

---

## G-05 — `resolve_token` (pre-publish) has the same blast radius, worse

`src/scheduler/mod.rs:1824`. When a post is about to publish and its token
is within `TOKEN_REFRESH_BUFFER_SECS` of expiry, `resolve_token` refreshes
and — on `Err` — **propagates**, which fails the publish. There is no
`refresh_needed` write here and no "try the old token anyway" path.

**Consequence.** A post that was perfectly publishable with the existing
token is failed because a *profile* call inside the refresh errored. That is
a customer-visible dropped post, worse than the channel row in G-01.

**Fix.** G-01's graceful degrade covers this too, because the failure stops
being an `Err`. Worth a test at this layer: a post in the buffer window
with a good access token and a failing profile read must still publish.

---

## G-06 — the passive paths that nobody has audited for `needs_cron_refresh`

The 12 cron providers are `calendar, drive, gmail, google,
instagram_standalone, linkedin_page, linkedin, reddit, threads, tiktok, x,
youtube`. Google-family is now unified through `google_oauth` (a1ac73f), so
their refresh is one implementation — already good.

Not audited for this pass, and not claimed as clean:

- **`facebook`, `pinterest`, `mastodon`, `bluesky`, `slack`, `wordpress`,
  `medium`, `devto`, `hashnode`, `github`, `skool`, `discord`,
  `whatsapp`, `telegram_*`** — providers outside the cron set, i.e. they
  refresh only on the pre-publish `resolve_token` path. A `?`-on-profile
  there fails a publish instead of a channel row.
- **Cookie-auth channels** (`x` cookie, `reddit` cookies) have no refresh
  token; `get_integrations_needing_refresh` excludes them by
  `refresh_token IS NOT NULL`, correctly.

That is a real gap in *coverage*, not a claim of safety. It is listed so the
next pass knows what is still unexamined.

---

## G-07 — two false alarms, recorded so nobody re-raises them

Both came up during the audit and were **disproved**. They are written down
because both looked real and both would cost a future reader the same hour.

**"8 of 12 providers never set `expires_in`, so their token never
refreshes again."** False. The check was a grep for `expires_in` in
`src/social/<provider>.rs`, and the Google-family providers keep it in the
shared `google_oauth::refresh` they delegate to. Verified per provider by
reading the returned `AuthToken`: all 12 propagate `expires_in` from their
response JSON. The `None` consequence (a written `token_expires_at = NULL`,
after which the `IS NOT NULL` gate makes the row invisible forever) is real
*in principle* and no provider triggers it today. If one ever does, that is
the shape to look for.

**"The scheduler's decrypt fallback silently sends ciphertext to the
provider."** False. `maybe_decrypt_token` (`crypto.rs`) has the *identical*
`unwrap_or_else(|_| token.to_string())`. The scheduler's version is a
duplicate of a correct helper, not a divergence. Worth folding into the
helper (ponytail), not worth treating as a security bug.

---

## Recommended order

| # | Item | Size | Why now |
|---|---|---|---|
| 1 | G-01 + G-02 — graceful profile read at 3 sites | ~10 lines each | The operator's reported symptom. Live, customer-visible. |
| 2 | G-04 — extract `profile_optional`, route G-01/G-02 and `linkedin_page` through it | net delete | Stops the 4th copy being written wrong. |
| 3 | Test at the `resolve_token` layer (G-05) | 1 test | Proves a dropped post cannot happen, not just a channel row. |
| 4 | G-03 policy decision | product call | Only meaningful once 1-2 land. |
| 5 | G-06 sweep of the non-cron providers | audit | Coverage gap, no known defect. |

**Do not** land 3 or 4 before 1: a retry policy on top of a defect that
discards good tokens retries the wrong thing.

---

## G-08 — the feed's refresh+fallback block is written twice (recorded, not fixed)

`src/feed/mod.rs` contains TWO byte-identical copies of the same 29-line
block: resolve token -> `get_recent_posts` -> on `TokenExpired` call
`try_refresh_and_fetch` -> on any other error call
`handle_feed_failure_inner` -> `continue`.

- `refresh_user_posts` — `mod.rs:382-410`
- `refresh_all_posts` — `mod.rs:488-514`

Both are live, not dead code: `refresh_all_posts` runs on the background
timer (`start_feed_refresher`, `mod.rs:64`) and `refresh_user_posts` is
called by **both** the REST endpoint (`api/feed.rs:191`) and the MCP tool
(`mcp/tools_feed.rs:128`), so a change on one path and not the other is a
two-interface divergence.

**Measured, not assumed:** the two blocks were `diff`ed and are identical
(only my extraction line offsets differed). No behavioural difference exists
today, so this is NOT a defect and it is not mutation-provable — a mutation
on one copy does not change behaviour, because the other copy is not running.
It violates AGENTS.md 9.18 (one implementation per rule) and is a *latent*
surface, which is exactly the shape that produced defect B: the fix in 5c09a5f
lives in the shared `try_refresh_and_fetch`, so both copies inherit it, but a
*next* fix aimed at the block would have to be made twice.

Recorded, deliberately NOT acted on in this pass: the pass is a red-team cycle
where every change must be proven by a mutation that turns a test RED, and an
identical-duplication fold produces no such mutation. If it is folded later,
that is a separate refactor pass with its own evidence, not something to slip
into a defect fix. (The same reasoning G-07 applies to
`maybe_decrypt_token`: "worth folding... not worth treating as a security bug".)

A THIRD copy was checked for and does not exist: `refresh_all_engagement`
(`mod.rs:576`) never calls `get_recent_posts` and has no `TokenExpired` arm.

---

## G-09 — three more false alarms, recorded so nobody re-raises them

All three came up while chasing defect D, and all three were **disproved by
reading the call graph**, not by assumption. Each looked real, and each would
cost a future reader the same hour.

**"A gone resource that also mentions `scope` becomes NeedsReconnect, which is
defect A again."** Read as: `classify_feed_failure` checks `auth_lost` BEFORE
`resource_gone`, so a message containing both wins NeedsReconnect. TRUE about
the ordering, but no live message has both. The message that looked like the
counter-example is `map_error`'s Forbidden arm (`social/linkedin.rs:774-778`:
"...missing the scope the endpoint needs (r_member_social,
r_organization_social) or the resource does not belong to this member") -
which does say `scope` on a target that may be gone. It is NOT reachable from
the classifier: `map_error` is called from exactly two sites, both publish /
analytics (`linkedin.rs:885` and `:987`), and `classify_feed_failure` is called
only from `handle_feed_failure_inner`, which is called only from the two feed
read blocks. Different call graph. The ordering is real and untested, so it is
a latent surface, not a defect - and it is exactly what defect D was for the
other arm.

**"`friendly_error` appends the raw response body, which leaks the token."**
False. `friendly_error` (`social/mod.rs:1329`) formats `{friendly} (raw: {raw})`,
and `raw` is built from `json["message"]` / `json["error_description"]` only
(`linkedin.rs:874`, `:983`) - the PROVIDER's error fields, never a header and
never the request. The one place LinkedIn can echo a credential is guarded
separately and correctly: `get_posts`' 401 branch logs status only, deliberately
(`linkedin.rs:176-179`, "LinkedIn error responses can echo the requested Bearer
token or URN"). That guard is on the LOG, and it is the only log of that body.

**"`refresh_all_posts` and `refresh_user_posts` drift, so one fixes and the
other does not."** Disproved by `diff`ing the two blocks (see G-08): they are
byte-identical today. What the duplication DOES cost is the next fix - it would
have to be made twice - which is G-08's point.
