# Project Config (`sksync.config.json`)

`sksync.config.json` is the manifest sksync reads to decide which skills to fetch and which agents each skill is symlinked into. It is the file you commit and share; the skill bodies under `.sksync/skills/` and the lockfile are generated.

## Initialize

```sh
sksync init                 # project: sksync.config.json + .sksync/skills/
sksync init --global        # global:  ~/.sksync/config.json + agents.json + skills/
sksync init --agents        # only force-refresh ~/.sksync/agents.json
```

`init` never overwrites an existing config — it fails instead. In global mode it leaves an existing `agents.json` untouched.

## Shape

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
  "skillDir": "./.sksync/skills",
  "defaultAgents": ["universal"],
  "dependencies": {
    "example-skill": {
      "source": "github:owner/repo/path/to/skills/example-skill#main",
      "agents": ["pi", "claude-code", "codex", "gemini", "opencode"]
    },
    "local-example": {
      "source": "./vendor/local-example",
      "agents": ["pi"]
    }
  }
}
```

| Field | Meaning |
|---|---|
| `$schema` | Optional. Points at the published JSON Schema for editor validation. |
| `skillDir` | Directory where fetched skill bodies are stored. Defaults to `./.sksync/skills` (project) or `~/.sksync/skills` (global). |
| `defaultAgents` | Agents pre-selected in the wizard's *Add skill* step. Does **not** change CLI behavior. |
| `dependencies.<name>` | One managed skill: a `source` and the list of `agents` it links into. |
| `dependencies.<name>.bundles` | Optional local provenance for bundles that installed or adopted the dependency. |
| `dependencies.<name>.managedByBundles` | Optional boolean. Defaults to `false`; `true` means bundle removal can delete the dependency when its last bundle provenance is removed. |

## Sources

A dependency `source` is either a string (the common case) or a structured object. String forms cover GitHub shorthand, GitHub tree URLs, `skills.sh`, and local paths — see [Sources & Discovery](/guides/sources) for the full table.

Use the structured form when you need an explicit provider, for example to clone a private repo over SSH:

```json
{
  "dependencies": {
    "my-skill": {
      "source": {
        "provider": "git",
        "url": "git@github.com:org/private-skills.git",
        "path": "skills/my-skill",
        "ref": "main"
      },
      "agents": ["pi"]
    }
  }
}
```

::: info
sksync has no `--token` / `--github-token` option and never stores credentials in config. Private repositories work whenever `git clone <repo>` already works in your environment — auth is delegated to your Git credential helper, GitHub CLI, or PAT. See [Sources → Private repositories](/guides/sources#private-repositories).
:::

## Bundle provenance

`bundle add` records provenance on dependencies instead of creating bundle folders:

```json
{
  "dependencies": {
    "review": {
      "source": "https://github.com/org/repo/tree/main/skills/review",
      "agents": ["pi"],
      "bundles": [
        { "name": "review-workflow", "source": "./bundles/review-workflow" }
      ],
      "managedByBundles": true
    }
  }
}
```

Manual dependencies adopted by a bundle keep `managedByBundles: false`, so `bundle remove` detaches provenance but keeps the dependency.

## Default wizard agents

`defaultAgents` seeds the agent selection in `sksync wizard` → *Add skill*. The plain `sksync add` CLI still requires explicit `--agent` flags.

```json
{ "defaultAgents": ["universal", "pi"] }
```

You can also set this from the wizard's *Configure default agents* step.

## Project vs global scope

The same config shape works in two scopes, and most commands accept `--global` to switch:

- **Project** — `sksync.config.json` at the repo. Targets resolve relative to the project root and cannot escape it.
- **Global** — `~/.sksync/config.json`. Targets resolve under your home directory.

Which agent target directory a skill links into is resolved from `agents.json` (the `project` map for project scope, the `global` map for `--global`), unless an inline `agents` override is present in the config. See [Agent Mappings](/guides/agent-mappings).

## Generated files and `.gitignore`

Project-local generated artifacts should be git-ignored:

```sh
.sksync/           # downloaded/copied skill bodies (.sksync/skills/<skill>)
skills/            # legacy generated skill store from older defaults
sksync-lock.json   # portable lockfile v4 (local state until sharing policy is final)
```

The file you share is `sksync.config.json`. The lockfile is portable and *can* be shared to reproduce installs across machines, but is currently treated as local state — see [Lockfile & Sync](/guides/lockfile).

## Examples & schema

- [`sksync.config.example.json`](https://github.com/takemo101/sksync/blob/main/sksync.config.example.json)
- [`schemas/sksync.schema.json`](https://github.com/takemo101/sksync/blob/main/schemas/sksync.schema.json)

## Related

- [Agent Mappings](/guides/agent-mappings) — `agents.json` and the bundled target directories.
- [Sources & Discovery](/guides/sources) — every `source` form.
- [Bundles](/guides/bundles) — curated install sets and provenance.
- [Commands](/reference/commands) — `init`, `add`, `bundle`, `plan`, `apply`.
