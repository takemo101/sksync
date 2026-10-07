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
| `pi` | `~/.pi/agent/skills` | `.pi/skills` |
| `oh-my-pi` | `~/.omp/agent/skills` | `.omp/skills` |
| `empryo` | `~/.empryo/skills` | `.empryo/skills` |
| `phi` | `~/.phi/skills` | — |
| `pig` | `~/.pig/agent/skills` | `.pig/skills` |
| `vtcode` | `~/.agents/skills` | `.agents/skills` |
| `grok` | `~/.grok/skills` | `.grok/skills` |
| `zero` | `~/.local/share/zero/skills` | — |
| `claude-code` | `~/.claude/skills` | `.claude/skills` |
| `codewhale` | `~/.codewhale/skills` | `.codewhale/skills` |
| `codex` | `~/.codex/skills` | `.codex/skills` |
| `fx` | `~/.fx/skills` | `.agents/skills` |
| `jcode` | `~/.jcode/skills` | `.jcode/skills` |
| `gemini` / `gemini-cli` | `~/.gemini/skills` | `.gemini/skills` |
| `opencode` | `~/.config/opencode/skills` | `.opencode/skills` |
| `antigravity` | `~/.gemini/antigravity/skills` | `.agents/skills` |
| `kimi-code` | `~/.kimi-code/skills` | `.kimi-code/skills` |
| `cursor` | `~/.cursor/skills` | `.cursor/skills` |
| `windsurf` | `~/.codeium/windsurf/skills` | `.windsurf/skills` |
| `universal` | `~/.agents/skills` | `.agents/skills` |

The full bundled set also includes `aider`, `amazon-q`, `amp`, `augment-code`, `bolt`, `clawdbot`, `cline`, `codebuddy`, `codegpt`, `commandcode`, `continue`, `crush`, `devin`, `droid`/`factory`, `fx`, `github-copilot`, `goose`, `hermes`, `kilo`, `kiro-cli`, `lovable`, `mcpjam`, `mux`, `neovate`, `openclaw`, `openhands`, `playcode-agent`, `qoder`, `qwen`, `replit-agent`, `roo`, `sourcegraph-cody`, `tabby`, `tabnine`, `trae`, `vercel`, and `zencoder`. Kimi Code CLI is available as `kimi-code`. See the complete file in [`sksync.agents.example.json`](https://github.com/takemo101/sksync/blob/main/sksync.agents.example.json).

::: info
**`universal`** is the canonical directory of the Agent Skills ecosystem: `~/.agents/skills` (global) and `.agents/skills` (project). Linking a skill into `universal` makes it visible to any tool that reads the shared directory.

**Oh My Pi** uses its native [OMP skill directories](https://github.com/can1357/oh-my-pi/blob/main/docs/skills.md). The global mapping targets the default profile; if you use a named profile or an overridden agent directory, customize `targetDir` accordingly.

**Empryo** scans its native [global and project skill directories](https://empryo.com/docs/tools/skills), alongside shared Agent Skills and Claude Code directories.

**Phi** loads skills from [`~/.phi/skills`](https://github.com/pulseaiclub/phi#skills) by default. It has no automatic project-local skill discovery, so only the global mapping is bundled. To use a project directory, configure Phi's `skill_path` or `PHI_SKILL_PATH` and add a matching project `targetDir` yourself.

**PiG** uses [`~/.pig/agent/skills` and `.pig/skills`](https://github.com/MichaelKinsy/PiG/blob/main/internal/pigdocs/content/config.md) by default. Customize `targetDir` if you override its configuration root/agent directory or opt into Pi's directories with `PIG_USE_PI_DIRS=1`.

**VT Code** scans the shared [Agent Skills directories](https://github.com/vinhnx/VTCode/blob/main/crates/codegen/vtcode-core/src/skills/discovery.rs): `~/.agents/skills` and `.agents/skills`. The bundled mapping uses these portable roots rather than its platform-dependent user data directory, so `vtcode` and `universal` intentionally share the same physical sksync links.

**fx** uses `~/.fx/skills` for managed user-level skills. In project scope it reads the shared `.agents/skills` directory, so `fx`, `universal`, and other compatible agents can intentionally share one physical sksync link.

**Antigravity** follows its official spec and uses the workspace default `.agents/skills` for project scope. The legacy `.agent/skills` is still honored by Antigravity for backward compatibility, but the sksync bundled default is `.agents/skills`.

**Kimi Code CLI** uses its Kimi-specific official scan directories: `~/.kimi-code/skills` globally and `.kimi-code/skills` per project. Kimi also scans the shared Agent Skills directories, which remain available through the separate `universal` mapping.

**CodeWhale** uses `~/.codewhale/skills` globally and `.codewhale/skills` per project. These native paths remain available when CodeWhale is configured to scan only CodeWhale-owned roots.

**Open Interpreter** reads the shared Agent Skills directories, `~/.agents/skills` and `.agents/skills`. Use `universal` for Open Interpreter; sksync does not add a separate `interpreter` alias.

**Zero** discovers user-level skills at `$XDG_DATA_HOME/zero/skills` (defaulting to `~/.local/share/zero/skills`). It does not have a project-local skill directory unless `ZERO_SKILLS_DIR` is explicitly overridden.
:::

## Shape

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json",
  "global": {
    "claude-code": { "targetDir": "~/.claude/skills" },
    "grok": { "targetDir": "~/.grok/skills" },
    "pi": { "targetDir": "~/.pi/agent/skills" },
    "zero": { "targetDir": "~/.local/share/zero/skills" }
  },
  "project": {
    "claude-code": { "targetDir": ".claude/skills" },
    "grok": { "targetDir": ".grok/skills" },
    "pi": { "targetDir": ".pi/skills" }
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
