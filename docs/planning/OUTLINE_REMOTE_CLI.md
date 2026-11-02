# Remote CLI plan: run the local CLI against the VPS by API key

Status: **plan only**. Nothing here is implemented. Written 2026-11-02 from a
measurement of the tree and of the postiz reference, not from an assumption
about either.

---

## 1. What the measurement found, so the plan is not built on a guess

### 1.1 The server side already exists. This is a client gap, not a server one.

`src/auth/middleware.rs::auth_middleware` has **three** credential paths:

1. `Authorization: Bearer sf_…` → `verify_api_key()` against the `api_keys`
   table.
2. `sf_session` cookie → JWT, the WebUI path.
3. A present-but-rejected `Authorization` header short-circuits to
   `401 invalid_api_key` rather than falling through to the cookie path —
   the deliberate choice, so a bad key is never silently downgraded to a
   session.

`src/api/developer.rs` issues, lists, revokes and regenerates those keys
(`POST /api/developer/api-keys`), and the routes are registered in
`src/api/mod.rs` at lines 506–511.

**Verified against production, not just locally:**

```
curl -sk https://127.0.0.1:6543/api/developer/api-keys   ->  401
```

`401`, not `404`. The route is live on the VPS and the gate is armed. A
reader could reasonably assume this was the missing half. It is not.

### 1.2 Both clients are local-only, and for one precise reason each.

| Client | Transport | Coupling | Why it cannot reach a remote host |
|---|---|---|---|
| `switchpost` CLI (`src/cli/run.rs`) | none | `init_state()` at **run.rs:257** opens the SQLite pool | every command takes `&AppState`, and the state is built from `config.database_url` |
| MCP (`src/mcp/mod.rs`) | **stdio AND HTTP** | shares the same `AppState`, but the HTTP mount builds it on the server | see §1.4 — the HTTP mount is **already usable remotely** |

The CLI is 4,059 LOC built on 19 `services::` calls and one `init_state()`.
That is the whole distance between what exists and what is being asked for:
**the services layer is already the right seam**, which is why the shape below
adds a transport rather than rewriting commands.

### 1.3 The plan's smallest honest step is smaller than "a remote CLI".

`init_state()` is the *only* thing that needs to change for the CLI to work
against a remote host, because everything downstream already goes through
`services::`. So the work is:

1. A transport that can exercise those services over HTTP.
2. A mode that resolves `AppState` to something backed by that transport
   instead of a local pool.

Everything else — the 389 MCP tools, the CLI's command surface, the platform
shims — is untouched, because they all already sit behind the seam.

### 1.4 The remote-agent gap does not exist, and a plan written without
### checking would have built it

Writing §1.2 from the file alone, I recorded MCP as "stdio only" and made
Phase 3 "add an HTTP transport for MCP". That is wrong, and it would have
been a phase of work for something that already ships.

`src/api/mod.rs` mounts a `StreamableHttpService` at **`POST /mcp`**
(mod.rs:615–645, route at :622), and the comment at :615 states the intent:
agents connect by URL with zero processes. That service is created inside
`protected_routes` (mod.rs:218+), so it sits under the same `auth_middleware`
— the one that accepts a bearer key.

**Verified against production:**

```
POST https://127.0.0.1:6543/mcp            -> 401   (no credential)
POST ... with Authorization: Bearer sf_bogus -> 401   (rejected key)
```

So an AI agent on a laptop can already drive the VPS with an API key. The
missing half is **only** the shell CLI.

The habit this cost me: I wrote a table row about a transport from one file
(`src/mcp/mod.rs`) without opening the file that mounts it. The same trap as
the `git checkout` that deleted a test earlier in this cycle — the fact was
in the tree, not in the file I happened to be reading.

Phase 3 is therefore **deleted** from the plan, and replaced with "decide
whether the CLI should be able to shell out to MCP, or keep REST parity".
---

## 2. What postiz actually does (measured, and it is not what you'd guess)

The brief asked how postiz implements "CLI/MCP utilization for remote
instances". **It has neither.** Measured in `../external-repos/postiz-app`:

- No `apps/cli`. The `bin` fields are absent from `package.json`.
- No MCP server. The only match in the whole repo is a `.svg` icon.
- `apps/commands` is `postiz-command`, an internal Nest worker (`nest start
  --entryFile`), not a user-facing CLI.

What postiz *does* have is a **typed SDK**, `@postiz/node`, and a **public
REST surface** at `/public/v1/*`. That is the pattern worth copying, and it
is a different shape from "a CLI that talks to a remote box".

**The auth contract, from `apps/sdk/src/index.ts`:**

```ts
constructor(private _apiKey: string, private _path = 'https://api.postiz.com')

fetch(`${this._path}/public/v1/posts`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json', Authorization: this._apiKey },
  body: JSON.stringify(posts),
})
```

**The server side, from `apps/backend/src/services/auth/public.auth.middleware.ts`:**

- Reads `req.headers.authorization` (or `Authorization`).
- `pos_` prefix → OAuth-app token; otherwise a personal API key.
- Missing key → `401 No API Key found`. Unknown key → `401 Invalid API key`.
- Sets an org onto the request, exactly like the session path would.

Two design points to take, and one not to:

| Postiz | Take it? | Why |
|---|---|---|
| Key sent as the raw `Authorization` value, no `Bearer` prefix | **No** | switchpost's gate already parses `Bearer` (`extract_bearer`). Adopting postiz's raw form would mean writing a second parser in the gate, for compatibility with a repo we are not. |
| A distinct URL namespace, `/public/v1/*` | Optional, cheap, and worth deciding explicitly | switchpost currently applies the SAME `auth_middleware` to every route, so an API key already authorises the *whole* surface, including `/api/developer/api-keys` — a key can mint more keys. See §5. |
| A hosted default with a self-hosted override (`_path` default `https://api.postiz.com`) | **Yes** | This is the shape `switchpost --url` should take: env var, flag, default to the local config. |
| `X-Postiz-Org` / `x-postiz-include-deleted` extra headers | No | Those exist because postiz is **multi-tenant**. switchpost has one user (§0.5.1) — every one of those headers is a violation waiting to be written. |

The tenant question is the one place this repo's preferences override the
reference, and it should be recorded rather than discovered later: **anything
copied from postiz that assumes an organization, a role, or a
can-be-impersonated principal is out of scope.**

---

## 3. The plan

Deliberately in `/ponytail` terms — the smallest change that still solves the
actual problem, phased so each phase is independently shippable and each has a
gate that can fail.

### Phase 1 — Make the seam a transport, for the CLI only

**Change.** Add `SWITCHPOST_URL` (or `--url` / `-u`) and
`SWITCHPOST_API_KEY` (or `SWITCHPOST_API_KEY_FILE`) to the CLI's argument
parsing and `src/config.rs`.

**Behaviour.** When `SWITCHPOST_URL` is set and points at an `http(s)` URL
that is **not** the local bind, `init_state()` must **not** open a SQLite
pool. It returns a state whose data access is an HTTP client pointed at that
base URL, sending `Authorization: Bearer $SWITCHPOST_API_KEY` on every call.

When unset, behaviour is byte-for-byte what it is today. A founder on the box
is on the documented local path, with no new dependency and no new failure
mode.

**This is the whole design.** It works because the CLI's data access is
already behind `services::`; Phase 1 *replaces one implementation of that
seam* rather than adding a parallel code path. The 4,059 LOC of command
surface does not learn that a network exists.

**Gate.** A command that today works locally must answer the *same* result
remotely and locally for the same fixture:

```
SWITCHPOST_URL=https://127.0.0.1:6544 SWITCHPOST_API_KEY=sf_test… switchpost posts list
```

and — the half that matters — the negative case: a wrong key must produce a
diagnostic that names the credential and the URL, not a pool error,
because "unable to open database" on a remote run is the failure a
founder cannot act on.

### Phase 2 — Close the key-scope hole the plan exposes

**Problem, found while writing Phase 1.** A key authorises every route guarded
by `auth_middleware`, which includes `POST /api/developer/api-keys`. So the
first thing a granted key can do is mint more keys, and revoke them.

**Change.** Mark the developer/key-management routes as requiring the
*local* credential — the session cookie — rather than accepting a bearer key.
The gate already distinguishes two credential paths, so this is one more
predicate, not a new subsystem.

**Gate.** A unit test that a bearer key on `POST /api/developer/api-keys`
gets `403` and does not create a row. This must be a real test against
`build_router` (AGENTS.md §6.5: set the `Origin` header, or the CSRF gate
answers 403 before the handler and the test proves nothing).

### Phase 3 — DELETED. MCP over HTTP already ships.

An earlier draft of this plan had "add a Streamable-HTTP transport for MCP"
here. §1.4 is why it was removed: `POST /mcp` already exists, already sits
under the key-accepting auth gate, and is live on the VPS. An agent on a
laptop needs nothing new.

The open question, if any, is narrow: **should the CLI have a mode that talks
to `/mcp` instead of REST?** The argument for is one protocol and one
authorisation path instead of two. The argument against is §0.5.4 — the three
interfaces already have parity, and the CLI mirroring MCP rather than the
services layer would move it further from the code it calls. Not worth doing
without a reason; record the decision if it ever gets one.

### Phase 4 — Documentation and key hygiene

- `docs/deployment.md`: the env vars, and one line saying the key is a
  bearer token for the same single-user owner, not a multi-user credential.
- `.env.example`: the two new vars, commented as remote-only.
- The key is shown **once** at issuance (`POST /api/developer/api-keys`
  returns it once, and only the hash is stored — `hash_api_key`, verified in
  `middleware.rs:63`). That contract must not be weakened by a "show me my
  key" feature.

---

## 4. Scope discipline (what this plan must not become)

Every item is a design preference this repo has already declared
(AGENTS.md §0.5), and the postiz reference offers a version of each:

| Tempted by postiz | Decision |
|---|---|
| Multi-tenant orgs, `X-Postiz-Org` role overrides, `SUPERADMIN` | **No.** Single user, `DEFAULT_USER_ID`. |
| A subscription/paywall gate on the API | **No.** Runs locally, no billing. |
| Scoped keys ("this key may only post") | Defer. Needs a `scopes` column and per-route checks. It is a real improvement; it is not needed to solve the asked-for problem, so it should not ride along. |
| Making the local CLI a thin HTTP wrapper *always* | **No.** Breaks the local path to serve the remote one. The flag inverts it. |

---

## 5. The overflow fix, which is in this changeset and is done

Separate from the plan, because it is a defect rather than a feature.

**What was wrong.** `frontend/src/lib/channels/ChannelRail.svelte` rendered
its expanded row's action cluster as
`absolute end-1 top-1/2 -translate-y-1/2`, while the name block next to it was
`min-w-0 flex-1` with `truncate`.

An absolutely positioned element does not participate in flex sizing, so the
name block could never be shrunk by it. The two 44px buttons were drawn on
top of the channel name, which is what the founder reported as "the buttons
overflow in two two rows distorting the calendar preview".

**The fix.** The cluster is now an in-flow flex sibling, and the select went
from `w-full` to `min-w-0 flex-1`. The cluster's 92px is now taken out of the
row by layout, and the name's `truncate` does the job it was already written
to do. The bottom-sheet variant already had this shape, which is why only the
desktop rail was wrong.

**Proof, both directions** (tests/channels_grid_multi_account_test.rs,
`the_rails_action_cluster_is_in_flow_not_overlaid`):

- With the `absolute …` restored: **FAILED**, with the mechanism named.
- With the fix: **6/6 green**.

Note the assertion is over comment-stripped source — the same trap that made
the `Add another` guard mutation-blind earlier in this cycle. A comment
repeating the phrase would satisfy a naive `src.contains`.

**Siblings checked, not changed:** the calendar grids are already guarded
(`WeekView`'s root carries `overflow-x-auto` + a 640px floor, and
`MonthView`/`ListView` carry it too — v25 F5); the other `w-full` uses in
`frontend/src/lib/channels/` are in full-width form contexts, not flex rows
with siblings, so they cannot overflow. `MediaCarousel`'s centred arrows are
nav chrome, not text overlap.

---

## 6. Order, and why

1. Phase 2 (the key-scope hole) — **before** Phase 1 ships a reason to use a
   key. Shipping Phase 1 first hands out a credential that can mint
   credentials.
2. Phase 1 (the transport).
3. Phase 4 (docs) with Phase 1, same changeset.
4. Phase 3 is not work, it is a decision, and it should not be made until
   someone actually wants a CLI mode that speaks MCP.

Nothing here is started. Phase 2 is a change to the auth gate, and Phase 1
touches `init_state`, which every CLI command depends on — both want a
deliberate start rather than being squeezed in beside a bug fix.
