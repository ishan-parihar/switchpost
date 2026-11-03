# Architecture

**What this document is.** How switchpost fits together, written for someone who
wants to know whether they can run it and what happens when something fails,
rather than someone who is about to read the source.

One Rust binary. One SQLite file. Three interfaces into the same engine.

---

## The one sentence

Switchpost is a program you own that posts to your social accounts on a
schedule: you give it your credentials and the rules, it runs on your machine,
and nothing about it depends on a service you pay for.

---

## The shape

```
        three interfaces                one engine                  your accounts
        ───────────────                ─────────                   ────────────

  CLI ──────────────┐
  100+ commands     │
                    ├──▶  the engine   ──▶  the platform adapters  ──▶  26 platforms
  REST API ─────────┤     (same code,     (one per platform,           12 Tier-1
  live dashboard    │      all three)      in tier order)              14 Tier-2
                    │
  MCP server ───────┘
  389 tools
```

The three interfaces are not three programs. Anything the dashboard can do, the
CLI can do, and the MCP server can do, because all three call the same engine.
That is the single most important fact about this architecture: there is no
second implementation to drift out of sync with the first.

---

## The parts

| Part | What it does |
|---|---|
| `social` | One adapter per platform, and the tier list that decides which ones are registered. |
| `scheduler` | When a post goes out. The clock that makes this unattended software. |
| `poll` and `feed` | What comes back in: replies, mentions, the state of a post after it went out. |
| `rss` | Turning a feed into content, for the channels that publish from a source rather than a person. |
| `realtime` | The live updates the dashboard shows. |
| `db` | One SQLite file. The whole state, in a file you can copy. |
| `mcp` | The tool surface an AI agent drives. |
| `api` | The REST surface the dashboard drives. |
| `cli` | The commands a shell or a script drives. |
| `auth`, `crypto` | How credentials are stored and how they are protected at rest. |
| `config` | The settings file and its defaults. |
| `wa` | The WhatsApp path, which is a different kind of channel and is kept separate for that reason. |

---

## Why it is one binary

This is the decision the whole design rests on, so it is worth stating plainly.

The thing switchpost replaces is a collection of per-platform scripts: one per
site, each with its own credentials handling, its own failure mode, and its own
half-finished feature. That is the normal state of social automation, and it is
why nobody trusts it.

The alternative that most tools take is a service you rent. That trades the
scripts for a subscription, a rate limit you do not control, and a company that
can shut down and take your posting history with it.

One binary with one SQLite file is neither. You can read it, run it on a spare
machine, back it up by copying a file, and stop using it without losing
anything. If this project disappeared tomorrow, your posting history would still
be in a file on your disk.

---

## The three-way split between trusted and untrusted code

Three kinds of code touch this program, and they are not equivalent:

1. **The engine.** Reviewed, tested, in this repository.
2. **The platform adapters.** Reviewed, but they depend on an external API that
   changes without asking. Each one can fail on its own, and a failure here is
   the most common thing that will go wrong.
3. **Your credentials.** Not code, but the thing a mistake in either of the
   other two exposes. They are stored encrypted, and they never appear in a log
   line.

The reason the split matters: an adapter failing is routine, and the design has
to make it routine. An engine failing is a bug. Treating those as the same class
of problem is how a tool ends up either silently dropping posts or refusing to
run at all.

---

## What a failure looks like

The honest account, because this is what actually determines whether the tool is
usable:

- **A platform's API rejects a post.** The post stays queued, the failure is
  recorded against it, and the other 25 platforms are unaffected. This is the
  common case, not the exception.
- **A credential expires.** Posts to that platform fail with a message that
  names the platform and the credential. The rest keep working.
- **The process stops.** The SQLite file holds the queue. Restarting picks up
  where it left off.

What never happens silently: a post is never marked as sent when it was not, and
a failure is never recorded as a success. The record is the whole value of the
thing, so the record cannot lie.

---

## Where each interface belongs

- **The CLI** is for unattended use: a shell script, a cron line, an agent. It is
  the path that works when nobody is watching.
- **The REST API and dashboard** are for a human setting things up and checking
  on them. It is the path that works when you want to see what the queue looks
  like.
- **The MCP server** is for an AI agent driving the whole surface. 389 tools,
  grouped by provider, so an agent that already knows the domain does not have to
  learn a second vocabulary.

---

## What this is not

- **Not a service.** There is no hosted version, and there is no account to
  create. It runs where you put it.
- **Not a guarantee against a platform changing.** The adapters are code, and
  platforms change their rules. The tier list and the failure record are how that
  is made visible rather than guessed at.
- **Not finished for every platform equally.** Tier-1 platforms are the ones with
  full coverage. Tier-2 are supported and less exercised, and the tier list is
  the honest statement of which is which.
