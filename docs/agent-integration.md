# Agent integration

Moved out of the README to keep it short. Content is unchanged.

## Agent Integration (AXI §7)

Switchpost ships an installable AI agent skill that provides ambient context at session start — showing connected platforms, provider status, and contextual help hints.

### Install the Skill

```bash
# Via npx (recommended)
npx skills add ishan-parihar/switchpost --skill switchpost

# Or download manually (installed automatically by install.sh unless SKIP_SKILL=true)
curl -fsSL https://raw.githubusercontent.com/ishan-parihar/switchpost/master/SKILL.md \
  -o ~/.agents/skills/switchpost/SKILL.md
```

### Session Hook (Claude Code)

Add to `~/.claude/settings.json` or project `.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "switchpost"
          }
        ]
      }
    ]
  }
}
```

At session start, Switchpost prints a compact dashboard:

```
bin: /usr/local/bin/switchpost
description: Post to 29 social platforms from a single CLI

providers[3]{name,status,platforms}:
  x,connected,X/Twitter
  reddit,connected,Reddit
  bluesky,connected,Bluesky
  ...

platforms_total: 26

help[4]:
  Run `switchpost providers` to see all connected accounts
  Run `switchpost post "Hello" --platforms x,bluesky` to post
  Run `switchpost stage "Long content" --platforms x,linkedin` to stage
  Run `switchpost doctor` to check system health
```

### Session Hook (Codex)

Add to `~/.codex/hooks.json` or project `.codex/hooks.json`:

```json
{
  "SessionStart": "switchpost"
}
```

### Session Hook (OpenCode)

Create `~/.config/opencode/plugins/switchpost.ts`:

```typescript
export default {
  name: "switchpost",
  onSessionStart: async () => {
    const { execSync } = require("child_process");
    return execSync("switchpost").toString();
  },
};
```

---
