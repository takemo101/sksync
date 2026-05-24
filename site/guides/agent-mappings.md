# Agent Mappings (`agents.json`)

`~/.sksync/agents.json` maps each agent name to the directory where its Agent Skills live. sksync uses these target directories to decide where a skill's symlink is created. The file holds two maps — `global` and `project` — and ships with bundled defaults for 40+ agents.

## How a target is resolved

For a dependency that lists `"agents": ["claude-code", "pi"]`, sksync resolves a target directory per agent, in this order of precedence:

1. **Inline `agents` override** in the config (highest priority).
2. **`agents.json` map for the active scope** — the `project` map for project config, the `global` map for `--global`.

Project scope uses the `project` map (e.g. `.claude/skills`); global scope uses the `global` map (e.g. `~/.claude/skills`). Project-scope targets cannot resolve outside the project root.

## Bundled defaults (selection)

`sksync init --global` writes the full bundled mapping. A representative subset:

| Agent | Global `targetDir` | Project `targetDir` |
|---|---|---|
| `pi` | `~/.pi/agent/skills` | `.pi/agent/skills` |
| `claude-code` | `~/.claude/skills` | `.claude/skills` |
| `codex` | `~/.codex/skills` | `.codex/skills` |
| `jcode` | `~/.jcode/skills` | `.jcode/skills` |
| `gemini` / `gemini-cli` | `~/.gemini/skills` | `.gemini/skills` |
| `opencode` | `~/.config/opencode/skills` | `.opencode/skills` |
| `antigravity` | `~/.gemini/antigravity/skills` | `.agents/skills` |
| `cursor` | `~/.cursor/skills` | `.cursor/skills` |
| `windsurf` | `~/.codeium/windsurf/skills` | `.windsurf/skills` |
| `universal` | `~/.agents/skills` | `.agents/skills` |

The full bundled set also includes `aider`, `amazon-q`, `amp`, `augment-code`, `bolt`, `clawdbot`, `cline`, `codebuddy`, `codegpt`, `commandcode`, `continue`, `crush`, `devin`, `droid`/`factory`, `github-copilot`, `goose`, `hermes`, `kilo`, `kiro-cli`, `lovable`, `mcpjam`, `mux`, `neovate`, `openclaw`, `openhands`, `playcode-agent`, `qoder`, `qwen`, `replit-agent`, `roo`, `sourcegraph-cody`, `tabby`, `tabnine`, `trae`, `vercel`, and `zencoder`. See the complete file in [`sksync.agents.example.json`](https://github.com/takemo101/sksync/blob/main/sksync.agents.example.json).

::: info
**`universal`** is the canonical directory of the Agent Skills ecosystem: `~/.agents/skills` (global) and `.agents/skills` (project). Linking a skill into `universal` makes it visible to any tool that reads the shared directory.

**Antigravity** follows its official spec and uses the workspace default `.agents/skills` for project scope. The legacy `.agent/skills` is still honored by Antigravity for backward compatibility, but the sksync bundled default is `.agents/skills`.
:::

## Shape

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json",
  "global": {
    "claude-code": { "targetDir": "~/.claude/skills" },
    "pi": { "targetDir": "~/.pi/agent/skills" }
  },
  "project": {
    "claude-code": { "targetDir": ".claude/skills" },
    "pi": { "targetDir": ".pi/agent/skills" }
  }
}
```

## Inspecting and updating mappings

```sh
sksync agents list       # print the resolved mappings
sksync agents doctor     # read-only: check targetDir existence and writability
sksync agents refresh    # rewrite ~/.sksync/agents.json from bundled defaults
```

`sksync init --agents` does the same overwrite as `agents refresh`: it leaves config and skill directories untouched and force-rewrites only `~/.sksync/agents.json`. Use either when you want to pull in newly bundled agent entries.

::: warning
`agents refresh` / `init --agents` overwrite `agents.json` with the bundled defaults. If you customized `targetDir` values or added entries, re-apply those changes afterward.
:::

## Adding a custom agent or directory

To support an agent that is not bundled, or to point an existing agent at a non-default directory, edit `~/.sksync/agents.json` directly and add it under both `global` and `project`:

```json
{
  "project": {
    "my-agent": { "targetDir": ".myagent/skills" }
  },
  "global": {
    "my-agent": { "targetDir": "~/.myagent/skills" }
  }
}
```

Then reference it like any other agent: `sksync add <source> --agent my-agent`.

## Examples & schema

- [`sksync.agents.example.json`](https://github.com/takemo101/sksync/blob/main/sksync.agents.example.json)
- [`schemas/sksync.agents.schema.json`](https://github.com/takemo101/sksync/blob/main/schemas/sksync.agents.schema.json)

## Related

- [Project Config](/guides/project-config) — inline `agents` override precedence.
- [Commands → agents](/reference/commands#sksync-agents) — `list` / `doctor` / `refresh`.
