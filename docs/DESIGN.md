# sksync Design

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for architectural principles.

[Section 13](#approved-next-iteration) records the approved next-iteration contract. Those changes are not implemented in v0.0.15; current command availability remains as described in section 7 until implementation.

## 1. Background

Agent Skills are stored in different locations depending on the agent.

Examples:

- Claude Code skills
- Codex instructions / skills
- Gemini CLI context / extensions
- OpenCode command / agent config
- Pi `~/.pi/agent/skills` globally or `.pi/skills` per project

Managing those locations manually causes problems:

- The same skill is copied into multiple agents and drifts over time.
- It is hard to reproduce a setup on another machine.
- Project-local and user-global skills get mixed.
- Adding a new agent requires repetitive manual setup.

`sksync` stores skill bodies in one place and creates symlinks into each agent's expected directory based on configuration.

## 2. Concept

```text
shared skill store
  └─ .sksync/skills/
      ├─ foo/SKILL.md
      └─ bar/SKILL.md

sksync.config.json / ~/.sksync/config.json
  └─ dependencies: GitHub/local source + target agents

~/.sksync/agents.json
  └─ global and project target directories per agent

sksync update
  └─ GitHub/local source -> <project>/.sksync/skills/foo

sksync apply
  ├─ ~/.pi/agent/skills/foo -> <project>/.sksync/skills/foo
  ├─ ~/.claude/skills/foo -> <project>/.sksync/skills/foo
  └─ ...
```

### Product boundary

`sksync` is not a general skill marketplace or package platform. It is designed as a **safe, reproducible, lightweight Agent Skills deployment and sync tool**.

Core value:

- Reproducible skill placement through config and lockfile.
- Safe synchronization between source bodies and agent target symlinks.
- Clear project/global scope separation.
- Agent target mapping inspection and refresh.
- Conservative migration from manually managed skills.

Intentionally out of scope:

- marketplace / large registry operation
- recommendation / stack-aware skill suggestions
- format translation between agents
- REST / MCP server mode
- mesh / messaging
- automatic repair by `doctor`

If those capabilities become necessary, they should be external integrations or separate tools unless there is a strong reason to include them in core.

## 3. Terms

| Term | Meaning |
| --- | --- |
| skill | Reusable instructions, tool descriptions, or templates loaded by an agent. |
| source | Install source such as GitHub / `skills.sh` / local directory, or the concrete skill body directory. |
| dependency | Config entry describing where a skill comes from and which agents receive links. |
| bundle | Curated install set whose entries expand into normal dependencies. Bundles are not runtime folders. |
| bundle manifest | Shareable document that defines a bundle by naming its bundle entries. |
| bundle entry | Skill reference inside a bundle. The key is the resulting skill name; the entry source points to the skill body. |
| bundle provenance | Local dependency metadata recording which bundle(s) installed or adopted a dependency. |
| target | Directory where an agent reads skills. |
| mapping | Agent-to-target-directory configuration. |
| lockfile | File that pins synchronized skill content, source version, and hashes. |

## 4. Configuration files

`sksync` uses two configuration files.

1. **Install dependency config**: where skills come from and which agents they should be linked into. Available for project (`sksync.config.json`) and global (`~/.sksync/config.json`) scopes.
2. **Agent target mapping**: where each agent expects skills. Stored globally in `~/.sksync/agents.json`, with separate global/user and project mappings.

### Install dependency config

Schema: [`schemas/sksync.schema.json`](../schemas/sksync.schema.json)

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
  "skillDir": "./.sksync/skills",
  "defaultAgents": ["universal"],
  "dependencies": {
    "reviewer": {
      "source": "github:owner/repo/skills/reviewer#main",
      "include": ["SKILL.md", "references"],
      "agents": ["pi", "claude-code", "codex"]
    },
    "browser": {
      "source": "https://github.com/owner/repo/tree/main/skills/browser",
      "agents": ["pi", "gemini", "opencode"]
    },
    "local-helper": {
      "source": "./vendor/local-helper",
      "agents": ["pi"]
    }
  }
}
```

Source strings should stay compact. `sksync add <source> --agent <agent>` updates `dependencies` and then runs install/apply behavior. With `--global`, it updates `~/.sksync/config.json`. Optional `include` filters are separate from the source identity; they select which files/directories to copy from the resolved package root. Missing `include` means copy the full package. `sksync add --manifest-only` stores `include: ["SKILL.md"]`, while repeated `--include <pattern>` stores custom filters.

`defaultAgents` is used only by the wizard to preselect agents in `Add skill` and `Add bundle` flows. CLI `add` and `bundle add` still require explicit `--agent` arguments for compatibility and clarity.

Bundle-added dependencies may include `bundles` provenance and `managedByBundles`. Missing `managedByBundles` means `false`. Manual dependencies keep `managedByBundles: false` when a bundle with the same source adopts them, so `sksync bundle remove` only detaches provenance instead of deleting the dependency.

#### Source formats

```text
github:owner/repo/path/to/skill#ref
owner/repo/path/to/skill#ref
owner/repo#ref
https://github.com/owner/repo/tree/ref/path/to/skill
skills.sh/owner/repo/skill-name#ref
skills.sh/owner/repo#ref
https://www.skills.sh/owner/repo/skill-name#ref
./local-skill
```

`registry:<host>/<package>` and `--provider` are not supported. Source URL transformers are inferred from the source string.

#### Add-time discovery

If the source points to a repo root or parent directory, sksync searches for `SKILL.md` up to depth 5.

- If the direct source contains `SKILL.md`, use that source.
- If one skill is found, select it automatically.
- If multiple skills are found in an interactive terminal, prompt for multiple selections.
- If multiple skills are found in a non-interactive environment, fail with guidance.
- With `--name`, automatically select exactly one discovered skill whose frontmatter `name` or directory name matches.
- Exclude `.git`, `node_modules`, and `.sksync` from discovery.
- Display skill names in bold cyan in prompts.

The selected skill is saved back to config with the real discovered subpath. For example, selecting `skills/foo` from `owner/repo` saves `owner/repo/skills/foo`.

#### `skills.sh` transformer

`sksync` treats `skills.sh` as a URL transformer to GitHub, not as a registry. `skills.sh` URLs/shorthands are valid input, but config stores the selected source as an exact GitHub tree URL.

```text
https://www.skills.sh/vercel-labs/skills/find-skills
→ https://github.com/vercel-labs/skills.git
→ skills/find-skills
→ saved source: https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills
```

If the direct `skills.sh` URL slug does not match the actual GitHub repo path, repo-root discovery finds the matching skill and saves the exact tree URL.

```text
https://www.skills.sh/gitbutlerapp/gitbutler/but
→ discovers crates/but/skill
→ saved source: https://github.com/gitbutlerapp/gitbutler/tree/HEAD/crates/but/skill
```

#### Install validation

`sksync add`, `update`, and `install` validate fetched skills before replacing the destination:

- `SKILL.md` exists.
- `SKILL.md` is a file.
- YAML frontmatter exists.
- Frontmatter contains non-empty string `name` and `description` fields.

If validation fails, sksync does not replace the destination and removes the staging directory.

Internally, source URL transformers run in order. `sksync update` fetches from dependencies and updates the lockfile. `sksync install` prefers lockfile sources when a lockfile exists.

### Bundles

A bundle is a curated install set stored as `sksync.bundle.json` at the root of a bundle source. It contains a name, description, and entries keyed by the final skill name.

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json",
  "name": "review-workflow",
  "description": "Skills for review and QA workflows.",
  "entries": {
    "review": { "source": "./skills/review" },
    "qa": { "source": "github:org/qa-skills/skills/qa#main" }
  }
}
```

Bundle manifests do not contain agents. `sksync bundle add <source> --agent ...` expands entries into normal dependencies, union-merges agents when an existing dependency has the same source, and conflicts when the same skill name already points at a different source. `sksync bundle remove <name>` uses local config provenance only; it does not refetch the remote manifest.

Bundle add planning reports `create`, `merge`, `conflict`, and `skipped`. Any conflict aborts the whole add before writes. Bundle remove planning reports `remove`, `detach-provenance`, `ambiguous`, and `not-found`. Dependencies created by bundles use `managedByBundles: true`; manual dependencies adopted by matching source keep `managedByBundles: false`, so removing the bundle only detaches provenance.

Agent symlink targets stay flat. Agents never see bundle folders, and the lockfile does not store bundle provenance. Bundle provenance is local UX/config metadata, not content reproducibility state.

#### Bundle manifest discovery

`sksync bundle add <source>` and `sksync bundle inspect <source>` resolve one bundle manifest before loading entries. TUI bundle add uses the same resolver and passes the resolved source into dry-run/apply commands so users do not answer the same selection twice. `bundle sync` and `bundle remove` do not discover manifests; they use the exact bundle provenance source already stored in config.

Resolution is exact-first:

- If `<source>` points directly at `sksync.bundle.json`, use its parent directory as the bundle source.
- Otherwise, if `<source>/sksync.bundle.json` exists, use `<source>` directly.
- Otherwise, search under `<source>` up to depth 5 for `sksync.bundle.json`.
- Exclude `.git`, `node_modules`, and `.sksync` from discovery.

If discovery finds one manifest, select it automatically. If it finds multiple manifests in an interactive terminal, prompt for one selection. The prompt and filtering use the manifest's relative path, manifest `name`, and `description`. In a non-interactive environment, multiple manifests are an error; print the candidate paths and names and ask the user to pass a more specific source or `--name <bundle>`.

`bundle add` and `bundle inspect` support `--name <bundle>` as an explicit selector. The selector must match exactly one candidate by manifest `name` or manifest parent directory name. Zero matches, multiple matches, or a direct manifest whose name/path does not match `--name` are errors.

The selected manifest's parent directory becomes the resolved bundle source. That resolved source is printed by `bundle inspect` and stored as bundle provenance by `bundle add`, even if the user originally supplied a wider repo root. This keeps later `bundle sync` deterministic.

GitHub `/blob/<ref>/.../sksync.bundle.json` URLs are accepted as direct manifest sources and normalize to the corresponding parent `/tree/<ref>/...` source. Tests for discovery should use temporary local directories and pure source-normalization functions rather than real network clones.

#### Bundle sync

```sh
sksync bundle sync <name> [--source <exact-source>] [--global] [--agent <agent>...] [--dry-run]
```

`bundle sync` follows changes in bundle manifest membership for one named bundle at a time. It compares the latest manifest for a bundle source with local dependencies that record the same bundle name and exact source. If no matching local bundle provenance exists, sync fails as not found. If the bundle name exists locally for multiple sources, the user must pass `--source <exact-source>` to disambiguate. If the reloaded manifest declares a different bundle name than the requested local provenance, sync aborts before writes rather than silently renaming local provenance. Sync does not refresh content for existing kept entries; `sksync update` remains responsible for dependency content updates.

The sync plan reports changed or blocking items first: `add`, `adopt`, `remove`, `detach-provenance`, `source-changed`, and `missing-agents`. Unchanged manifest entries are counted as `keep` in the summary rather than printed as noisy per-entry rows by default. Any blocking status aborts apply before writes.

When sync discovers a new manifest entry, the new dependency uses the deduplicated union of dependency agents from other local dependencies with the same bundle name and exact source. If a same-name dependency already exists with the same or equivalent source, sync adopts it by adding bundle provenance, union-merging the inferred dependency agents, and leaving it manual rather than bundle-managed. If no agents can be inferred for a new dependency, the sync plan reports a blocking `missing-agents` item until the user supplies agents explicitly. CLI `--agent` values are a fallback for this inference failure, not an override for a bundle installation that already has inferable agents. `--dry-run` should preview all manifest drift and blockers without mutating config, lockfile, installed skill bodies, or symlinks.

When sync discovers a local dependency whose matching bundle entry disappeared from the latest manifest, it applies the same safety rule as bundle removal: bundle-managed dependencies may be removed when no other bundle provenance remains, while manual or adopted dependencies only lose the matching bundle provenance. Sync does not silently apply source changes. If a manifest entry points to a different source than the local dependency with the same skill name, sync reports a blocking `source-changed` status with the local and manifest sources, then aborts before writes.

#### Bundle export

Bundle consumption has a matching creation workflow:

```sh
sksync bundle export <name> --output <dir> [--global] [--snapshot] [--skill <name>...] [--dry-run] [--force]
```

Default export is manifest-only: it reads current project/global dependencies and writes `sksync.bundle.json` with the existing dependency source strings. Snapshot export copies the currently installed skill bodies into `<dir>/skills/<name>` and writes manifest-relative sources such as `./skills/review`.

Export never writes agents, existing bundle provenance, or `managedByBundles` into the bundle manifest. It is read-only with respect to the source config, lockfile, and installed skill store. Existing output is not overwritten unless `--force` is passed.

### Agent target mapping

Schema: [`schemas/sksync.agents.schema.json`](../schemas/sksync.agents.schema.json)

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json",
  "global": {
    "claude-code": { "targetDir": "~/.claude/skills" },
    "cursor": { "targetDir": "~/.cursor/skills" }
  },
  "project": {
    "claude-code": { "targetDir": ".claude/skills" },
    "cursor": { "targetDir": ".cursor/skills" }
  }
}
```

`sksync.agents.example.json` contains major coding-agent keys and is generated as `~/.sksync/agents.json` by `sksync init --global`. `global` is used for global/user scope; `project` is shared across projects for project scope.

### Configuration policy

- `skillDir` may be relative.
- Skills with `dependencies.*.source` are installed into `skillDir/<skillName>` by `sksync update` / `sksync install`.
- Project config resolves agent targets in project scope; global config (`--global`) resolves targets in user scope.
- `sksync plan/apply/check/list/install/update` support `--global`.
- `skills.*.source` remains supported as a legacy/local-only skill format.
- Actual agent target paths come from built-in mappings or `~/.sksync/agents.json`.
- For project config, the shared `project` mapping takes precedence over `global` mapping.

## 5. Built-in agent mapping

Defaults are overrideable through config.

| agent | user scope default | project scope default | notes |
| --- | --- | --- | --- |
| pi | `~/.pi/agent/skills` | `.pi/skills` | Matches Pi's documented global and project skill formats. |
| grok | `~/.grok/skills` | `.grok/skills` | Grok CLI skill directories. |
| zero | `~/.local/share/zero/skills` | — | Zero supports a user-level data directory only; `ZERO_SKILLS_DIR` can override it. |
| claude-code | `~/.claude/skills` | `.claude/skills` | Claude Code skill directory. |
| codex | `~/.codex/skills` | `.codex/skills` | May need instruction conversion later. |
| gemini | `~/.gemini/skills` | `.gemini/skills` | Aligned with Gemini CLI. |
| jcode | `~/.jcode/skills` | `.jcode/skills` | jcode skill directory. |
| opencode | `~/.config/opencode/skills` | `.opencode/skills` | Watch for OS-specific differences. |
| antigravity | `~/.gemini/antigravity/skills` | `.agents/skills` | Workspace default is `.agents/skills`. |
| kimi-code | `~/.kimi-code/skills` | `.kimi-code/skills` | Kimi Code CLI specific skill directory. |
| universal | `~/.agents/skills` | `.agents/skills` | Canonical Agent Skills directory. |

## 6. Lockfile

File name: `sksync-lock.json`

Schema: [`schemas/sksync-lock.schema.json`](../schemas/sksync-lock.schema.json)

Like `package-lock.json`, the lockfile stores the information needed for `sksync install` to reconstruct the same skill bodies on another environment. Supported OS targets are macOS and Linux for now; Windows-specific path/symlink differences are out of scope. Linux distribution assets use musl to avoid glibc-version coupling.

### Portable lockfile v5

Lockfile v5 avoids machine-local absolute paths and records effective include filters.

- `root` is always `"."`.
- `skills.<name>.source` is relative to the lockfile directory.
- `installSource` stores an exact source that can be fetched again.
  - Git sources store `url`, resolved commit `ref`, and repo `path`.
  - `skills.sh` input is normalized to an exact GitHub tree URL at add time and locked as a Git source.
  - Local sources under the project/global root can be stored as relative paths.
  - Absolute local sources outside the project/global root are non-portable.
- `files[].path` is relative to the skill directory.
- Optional `include` records the effective package filter. Missing means full-package install.
- Agent target paths and symlink state are not stored.

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync-lock.schema.json",
  "lockfileVersion": 5,
  "generatedBy": "sksync@0.0.15",
  "generatedAt": "2026-05-17T00:00:00.000Z",
  "root": ".",
  "skills": {
    "but": {
      "source": ".sksync/skills/gitbutlerapp/gitbutler/crates/but/but",
      "installSource": {
        "type": "git",
        "url": "https://github.com/gitbutlerapp/gitbutler.git",
        "ref": "abc123resolvedcommit",
        "path": "crates/but/skill"
      },
      "include": ["SKILL.md"],
      "hash": "sha256-...",
      "files": [
        { "path": "SKILL.md", "hash": "sha256-..." }
      ]
    }
  }
}
```

### Project / global base roots

Paths resolve relative to the lockfile directory.

| scope | lockfile path | relative base | source example |
| --- | --- | --- | --- |
| project | `./sksync-lock.json` | project root | `.sksync/skills/...` |
| global | `~/.sksync/sksync-lock.json` | `~/.sksync` | `skills/...` |

This lets a project move from `/Users/alice/work/app` to `/home/bob/work/app` while keeping project lockfile `source` paths as `.sksync/skills/...`. Global lockfiles likewise avoid storing `/Users/alice/.sksync` vs `/home/bob/.sksync`.

### `sksync install` reproduction model

When a lockfile exists, `install` prefers the lockfile `installSource`.

1. Read current environment config.
2. Fetch the exact source from lockfile `installSource`.
3. Place skill bodies under the current environment's `skillDir`.
4. Verify lockfile hash and file hashes.
5. Recompute target paths from current agent mappings and create symlinks.

Lockfile `source` means "where the skill body should live in the current environment", not an absolute path from the machine that generated the lockfile.

### Backward compatibility

Existing v3/v4 lockfiles remain readable. If legacy lockfiles contain absolute `root` / `source`, they are rewritten to the current v5 relative form the next time `install`, `update`, or `apply` writes a lockfile.

### Non-portable local source

A local source outside the project/global root is not portable across macOS / Linux machines.

```json
{
  "installSource": {
    "type": "local",
    "path": "/Users/alice/manual-skills/review"
  }
}
```

`doctor` should warn about this case. `install` should fail clearly if the path does not exist in the current environment. It must not guess alternate paths.

### Lockfile contents

- skill name
- source path relative to lockfile directory
- whole skill directory hash
- per-file hashes
- resolved install source ref/version
- sksync version

Target paths and symlink state are not written to new lockfiles.

### Linux release and Docker smoke coverage

Release workflow builds macOS targets plus Linux musl targets.

| target | purpose |
| --- | --- |
| `x86_64-unknown-linux-musl` | Debian / Ubuntu x86_64 default installer asset. |
| `aarch64-unknown-linux-musl` | Linux arm64 / aarch64 default installer asset. |

The Linux installer chooses assets from `uname -s` / `uname -m`. Docker smoke workflow runs the x86_64 musl binary in Debian / Ubuntu containers and verifies local-source `init`, `add`, `plan`, `apply`, `check`, `list`, and `remove`.

Initial smoke coverage:

- `debian:bookworm`
- `debian:trixie`
- `ubuntu:22.04`
- `ubuntu:24.04`

Windows remains out of scope. Alpine will likely work with musl binaries, but is not part of formal smoke coverage yet.

## 7. CLI / TUI commands

`sksync` is a single Rust binary. CLI commands are for automation/scripting; the wizard is for prompt-driven local use. The command model follows npm-like dependency management. There is intentionally no dedicated `ci` command.

The selected-update syntax and expanded JSON flags in [section 13](#approved-next-iteration) are planned, not currently available. In particular, the existing `outdated --json` array remains the v0.0.15 format until the announced migration.

### npm-like command model

| command | npm analog | role |
| --- | --- | --- |
| `sksync add <source> --agent <agent>` | `npm install <pkg>` / `npm add` | Add dependency config, fetch, and link. |
| `sksync install` | `npm install` | Reconstruct from lockfile if present, otherwise config. |
| `sksync update` | `npm update` | Fetch latest/specified dependency versions and update lockfile. |
| `sksync attach <skill> --agent <agent>` | optional dependency add | Attach an existing dependency-managed skill to more agents. |
| `sksync remove <skills...>` | `npm uninstall` | Remove one or more dependencies, installed files, lockfile entries, and managed symlinks. |
| `sksync remove <skill> --agent <agent>` | optional dependency removal | Remove only selected agent targets/symlinks. |
| `sksync outdated` | `npm outdated` | Compare lockfile resolved sources with upstream/latest. |
| `sksync bundle inspect/add/remove/export/sync` | npm workspace/package-set operations | Manage curated bundle install sets and follow bundle manifest membership drift. |
| `sksync apply` | sksync-specific | Reflect installed skills into agent targets. |
| `sksync check` | `npm ls` / health check | Check lockfile hash, source, and target symlink drift. |
| `sksync doctor` | health check | Read-only diagnosis of config, lockfile, source, target, and mapping problems. |
| `sksync agents <subcommand>` | config management | List, diagnose, and refresh agent target mappings. |
| `sksync import <path> --agent <agent>` | migration | Copy existing skill directories into `.sksync/skills`. |
| `sksync list` | `npm ls` | List managed skills and link states per agent. |
| `sksync wizard` | n/a | Prompt wizard for status and operations. |

### Command behavior summary

- `init`: create project/global config and skill directories without overwriting existing config; `--agents` refreshes only `~/.sksync/agents.json`.
- `add`: accept GitHub / `skills.sh` / local sources, add new dependencies, support multiple `--agent`, and install/apply only the newly added dependencies; it must not update, refetch, or rewrite existing dependencies. `--force` passes through to the final link apply step.
- `install`: prefer lockfile `installSource`; otherwise fetch from config and create a lockfile; then apply managed symlinks; `--force` repairs drifted or broken target symlinks during the apply step.
- `update`: fetch dependencies, resolve Git sources to exact commits, and refresh the lockfile. It does not apply links and therefore has no `--force`.
- `attach`: add agents to an existing dependency-managed skill while preserving source representation; `--force` passes through to the final link apply step.
- `remove <skills...>`: remove config entries, installed files, managed symlinks, and lockfile entries; support `--keep-files`, `--config-only`, and `--global`. It does not support generic `--force`.
- `remove <skill> --agent <agent>`: remove selected agent symlinks and targets only; full removal if the last agent is removed.
- `outdated`: compare Git lockfile commits with remote ref HEAD; support `--global` and `--json`.
- `bundle`: inspect/add/remove/export curated install sets; `bundle add --force` and `bundle sync --force` pass through to their final link apply step; `bundle export --force` replaces an existing generated output directory.
- `apply`: resolve targets, detect conflicts, create/update symlinks, and write lockfile; `--force` only allows symlink repair/replacement, never regular-file or directory replacement.
- `check`: compare config, lockfile, hashes, sources, and symlink health.
- `doctor`: read-only comprehensive diagnosis with suggested next commands, never automatic repair. By default it is local-only and performs no remote git operations. `doctor --remote` additionally probes each dependency's current config source and reports remote Git source paths that no longer exist at the configured ref; `doctor --remote --global` checks the global config instead of the project config. Remote checks never mutate config, lockfile, installed bodies, or symlinks. Output is grouped by problem type with one fix hint per group (target conflicts are summarized by agent and parent target directory), agent-mapping diagnostics are kept in their own sections, and when `--remote` is used the remote source problems are printed first so they are not buried under local/link findings.
- `agents`: list effective mappings, diagnose target directories, and refresh bundled mappings.
- `import`: copy-only migration from existing skill directories; no original files are mutated.
- `wizard`: prompt-based wrapper around CLI/application use cases.

### Existing dependencies during `add`

`add` is a dependency-creation command, not an update or attach shortcut. When source discovery returns multiple skill candidates, a candidate whose skill name already exists in config is an existing dependency:

- interactive flows show existing dependencies as `already installed` but do not allow selecting them;
- non-interactive `add` fails clearly when the requested skill name is already installed, with guidance to use `attach`, `update`, or `remove` as appropriate;
- if every discovered candidate is already installed, the flow explains that there are no new skills to add and exits without changes;
- existing dependencies are not reinstalled, refetched, updated, or rewritten as part of `add`, even when their source is stale or their upstream `HEAD` moved.

### `--force` link replacement semantics

`--force` is supported only on commands that perform link application: `apply`, `install`, `attach`, `add`, `bundle add`, and `bundle sync`. It affects only the final target-link reconciliation step.

With `--force`, sksync may replace these existing targets:

- a symlink at the desired target path that points to a different source than the resolved skill body;
- a broken symlink at the desired target path.

Replacement means unlinking the existing symlink and creating a new symlink to the resolved skill body. `--force` must not replace or delete regular files, directories, missing sources, or targets outside the configured agent target path. Those remain blocking conflicts. Read-only commands (`plan`, `check`, `doctor`, `list`, `outdated`, `bundle inspect`) do not accept `--force`; `update` does not accept `--force` because it updates installed skill bodies and the lockfile but does not apply target links; `remove` intentionally has no generic `--force` because removal is limited to managed links and installed bodies inside `skillDir`.

## 8. Wizard design

`sksync wizard` is an interactive prompt wizard. Its purpose is to safely add/remove skills and bundles without requiring users to remember command-line flags. `sksync ask` and `sksync tui` remain aliases.

Example flow:

```text
? What would you like to do?
  > Add skill
    Attach skill to agent
    Remove skill
    Detach skill from agent
    Add bundle
    Remove bundle
    Configure default agents
    Show status
    Apply links

? Skill source
  github:owner/repo/path/to/skill#main

? Select agent(s)
  [x] pi
  [x] claude-code
  [ ] codex
  [ ] gemini

Planned changes:
  add dependency: cuekit-dogfood
  install source -> .sksync/skills/cuekit-dogfood
  create symlink: .pi/skills/cuekit-dogfood
  create symlink: .claude/skills/cuekit-dogfood

? Apply these changes? (y/N)
```

### TUI operation model

| intent | prompts | use case |
| --- | --- | --- |
| add skill | source, name override, agent, global scope | `add` |
| attach to agent | project/global scope, configured skill list, available agent list | `attach` |
| remove skill | project/global scope, configured skill list, remove mode | `remove` |
| detach from agent | project/global scope, configured skill list, configured agent list | `remove --agent` |
| add bundle | bundle source, manifest preview, project/global scope, agent list, plan confirmation | `bundle add` |
| remove bundle | project/global scope, exact bundle provenance selection, plan confirmation | `bundle remove` |
| default agents | project/global scope, agent list | config update |
| status | global scope, output detail | `list` / `check` |
| apply | global scope, force, confirmation | `plan` -> `apply` |

### TUI principles

- Runtime TUI copy is English for international users.
- TUI only asks questions and confirms actions; it does not contain core logic.
- Default-agents preference updates preserve existing config fields.
- Remove mode is a single-select choice with explicit normal/keep-files/config-only semantics.
- Each flow calls the same application use case as the CLI.
- Destructive operations require plan/summary and explicit confirmation.
- TUI state is temporary prompt state only.
- Persistent state lives only in config, lockfile, or local state.
- `Add skill` and `Add bundle` use config `defaultAgents` as initial selection, but the user can change it each time.
- `Add skill` shows already installed skill candidates as disabled `already installed` rows when adding from a multi-skill source.
- `Add bundle` loads the manifest after source entry, shows the bundle name, description, and entries, and asks the user to continue before agent selection.
- `Remove bundle` lists exact bundle provenance choices as `name — source` so same-name bundles from different sources are never ambiguous in the wizard.
- Bundle wizard flows stop at add/remove for the first bundle UX iteration; `bundle sync` starts as a CLI flow with dry-run preview.
- Status uses `list` / `check` summaries; there is no persistent screen UI.

## 9. Safety rules

- At agent target paths, do not overwrite existing regular files or directories, even with `--force`. Replacement of dependency-managed skill bodies inside `skillDir` is a separate operation governed by section 13.
- Update/delete only targets represented by sksync config/lockfile plans.
- Warn if an existing symlink points somewhere unexpected.
- Without `--force`, drifted and broken target symlinks block apply.
- With `--force`, only drifted or broken target symlinks may be unlinked and recreated.
- Provide dry-run planning.
- Do not delete links that are not represented by the lockfile/config.

## 10. Implementation direction

Rust is the implementation language.

### Crate candidates

| Purpose | Crate |
| --- | --- |
| CLI parser | `clap` |
| config / lockfile serialization | `serde`, `serde_json` |
| path / home resolution | `dirs`, `shellexpand` |
| hash | `sha2`, `hex` |
| walking | `walkdir`, `ignore` |
| error handling | `anyhow`, `thiserror` |
| prompt wizard | `inquire` |
| snapshot / temp tests | `insta`, `tempfile` |

### Module shape

```text
src/
  main.rs          # clap entrypoint
  cli.rs           # command definitions
  config.rs        # sksync.config.json model / loader
  lockfile.rs      # sksync-lock.json model / writer
  agent.rs         # built-in agent mapping
  skill.rs         # skill discovery / hashing
  planner.rs       # desired link plan / dry-run result
  apply.rs         # symlink create/update
  check.rs         # drift / broken link detection
  tui/
    mod.rs             # prompt wizard coordinator and shared helpers
    add_skill.rs       # Add skill flow and include-filter prompts
    bundle.rs          # Add/Remove bundle flows
    commands.rs        # CLI argument builders used by wizard flows
    config.rs          # config scope/path/load helpers
    default_agents.rs  # Configure default agents flow
    operations.rs      # status/list+check and plan+apply flows
    skill.rs           # attach/remove/detach skill flows
```

### Architecture direction

- Planner builds the diff between desired state and current state.
- Apply executes only planner results.
- CLI and TUI share planner / apply / check logic.
- TUI contains no core logic.
- TUI flows stay split by user intent, with command argument construction isolated from prompt collection.
- OS differences are contained in agent resolution and apply/filesystem layers.

## 11. Minimum MVP

1. Read `sksync.config.json`.
2. Resolve target paths from built-in agent mappings.
3. Symlink `.sksync/skills/*` into target agent directories.
4. Generate `sksync-lock.json`.
5. Detect drift through `sksync check`.

## 12. Open questions

- Official skill directory specifications for every agent.
- Whether to add a conversion layer for agents with different skill formats.
- How far to take source URL transformers and package-manager-like install behavior.
- Exact precedence between project and user scopes.
- Windows symlink permissions and junction support. Windows remains out of scope for now; macOS / Linux are prioritized.
- Whether TUI belongs in the initial MVP or after CLI MVP.

<a name="approved-next-iteration"></a>

## 13. Approved next iteration: safe persistence, JSON output, and selected updates

**Status: approved design; implementation pending.** The two product decisions are:

1. A handled update failure restores **all selected skill bodies and the previous lockfile**, not just the last failing skill. Automatic recovery after process termination or power loss is out of scope.
2. `list`, `plan`, `check`, and `outdated` use **one versioned JSON envelope**. Changing the existing `outdated --json` array is an intentional breaking change and requires migration guidance.

These improvements strengthen sksync's existing declarative install/link model. They do not introduce a registry, account service, agent runner, generic transaction framework, or persistent update journal. Implementation order is persistence safety, JSON output, then selected updates.

### 13.1 Safe file persistence

Configuration and lockfile writes must not truncate the live file before a complete replacement is ready. Use one filesystem helper for serialized bytes, shared by config mutations, agent-map refreshes, lockfile writes, and `ConfigFileBackup::restore`; rollback writes must not retain the unsafe direct-write path.

The helper contract is:

1. Serialize completely before modifying the destination. Create a uniquely named temporary regular file in the **destination's actual parent directory**, exclusively, so no existing temporary path is overwritten or followed.
2. Write all bytes, preserve an existing regular file's permission bits, and synchronize the temporary file before publication. New files respect the normal creation permissions and umask.
3. For replacement, rename the prepared file over the existing regular file. Never remove or truncate the old file first. All returned write errors occur before publication and leave the previous bytes intact.
4. For create-only operations such as initial config creation, publish without overwriting a concurrently created destination. On supported local filesystems, a same-directory hard-link publication followed by temporary-name removal provides this with the standard library. Unsupported publication must fail safely, not fall back to an overwriting rename.
5. Preserve an existing config/lockfile symlink: resolve its existing regular-file referent and replace that file in its own parent directory, not the symlink. Reject dangling links and non-regular referents. Resolve the referent for I/O only; config-relative source paths and portable lockfile paths keep their existing logical scope base. This preserves the current ability to keep config in a dotfiles repository.
6. Remove only temporary paths owned by this operation. A failure to clean up an extra temporary name **after publication** is a warning, not an error claiming the write did not happen.

This guarantees complete-file publication under handled I/O failures, not power-loss durability. Filesystem-specific rename behavior must be tested on macOS and Linux; Windows remains out of scope.

### 13.2 Update boundary and rollback

The update unit is the effective dependency selection, its installed bodies, and the lockfile. `update` must not modify dependency config, bundle provenance, default agents, agent mappings, or target symlinks. The same guarantee applies to full and selected updates.

#### Writer exclusion

Before preparing an update, acquire non-blocking, OS-released advisory guards for the canonical physical state-parent directories and managed skill-store directory. Resolve config/lockfile symlink referents before choosing guard locations. Deduplicate and acquire directory guards in deterministic path order; a busy resource fails before fetching or replacement. Directory-handle locking avoids new lock-marker files and a global lock registry.

Other sksync mutators of these resources must use the same guard convention. Acquire guards once at the use-case boundary, not again inside nested installers or atomic-write helpers. After acquisition, reload/validate the config and lockfile snapshot; if the resource paths changed during acquisition, stop rather than proceeding under the wrong guards. Hold guards through rollback or post-commit cleanup. Standard-library advisory locking requires Rust 1.89 or later; record that compiler floor when implementing it.

These are cooperative writer guards, not locks on agent readers. Non-cooperating external edits during an update are outside the guarantee; detected unexpected destination changes must block replacement, never justify deleting the unexpected path.

#### Prepare, publish, commit

| phase | behavior | live-state effect |
| --- | --- | --- |
| Validate | Resolve scope and names, inspect the existing lockfile and managed destinations, acquire guards, and validate the guarded snapshot. Reject regular files, symlinks, or special files at managed-body destinations and parent paths escaping the canonical `skillDir`. | No body/lockfile replacement. |
| Prepare | Fetch/copy **every selected dependency** into private sibling staging directories. Apply `include`, validate `SKILL.md`, resolve actual Git commits, and calculate existing-format directory/per-file hashes from staged content. Build and serialize the candidate lockfile. | Old bodies and lockfile remain usable. |
| Publish bodies | For each selected destination, rename the old directory to a unique private sibling backup, then rename staging to the final destination. Record whether the destination originally existed. | Selected bodies change; backups remain available. |
| Commit lockfile | Atomically publish the candidate lockfile after every body publication succeeds. | Successful lockfile publication is the commit point. |
| Finalize | Remove this operation's backups and remaining staging directories. | Cleanup only; cleanup failures are warnings. |

Use sibling staging/backup directories on each destination filesystem, including a configured `skillDir` outside the project. Use the existing resolved dependency body path, including source-namespaced storage and legacy flat-path compatibility; updating must not silently migrate its layout. Never replace a managed-body destination by first deleting its old directory. Hashes and lockfile `source` paths describe the final installed body, not a private staging path.

On any handled failure **before the lockfile commit point**, restore published bodies in reverse order. Restore old directories from backups; if a selected body was absent originally, remove only the new directory owned by this operation. A failed atomic lockfile write leaves its original bytes, or its original absence, intact. No selected success is retained as a partial update.

If rollback itself fails, report both the original failure and each recovery failure, identify the affected skills and retained backup paths, and exit nonzero. Do not delete the only surviving old copy, hide the failure behind the original error, or continue to another update. Recovery is manual in this iteration.

After the commit point, backup-cleanup failure must not trigger rollback: some old backups may already have been removed. Report a warning and retained paths while treating the update as committed. Successful updates intentionally replace selected managed-body content, including local edits, as today; retained failure backups are not an undo-history feature.

#### Limits of the guarantee

- The guarantee is the final state after a handled failure with successful rollback, **not** instantaneous visibility of a multi-directory transaction to readers. An agent may briefly observe mixed versions or a missing logical destination during directory cutover.
- SIGKILL, crashes, power loss, and permanent filesystem failure can leave staging/backup directories or mixed live state. Do not auto-delete unknown leftovers or advertise crash recovery; a persistent journal and recovery command require a separate design.
- This command-wide guarantee applies to `update`. Other installer callers receive safer per-directory replacement and atomic file writes, but `add`, `install`, `apply`, and bundle operations do not thereby acquire a new all-or-nothing command guarantee.

### 13.3 Common JSON output contract

Planned supported forms:

```sh
sksync list --json [--global]
sksync plan --json [--global]
sksync check --json [--global]
sksync outdated --json [--global]
```

JSON is an opt-in output adapter for the same application reports as human output. It must not add network access to `list`, `plan`, or `check`, perform repairs, or alter selection/scope rules. Other commands do not gain `--json` in this iteration.

Every successfully written JSON response is one UTF-8 object followed by a newline, with these required fields:

| field | contract |
| --- | --- |
| `schemaVersion` | Integer `1`; independent of the package version and lockfile version. |
| `command` | `list`, `plan`, `check`, or `outdated`. |
| `scope` | `project` or `global`; invocation scope, not an agent's placement scope. |
| `ok` | Whether the command's result satisfies its success condition. |
| `data` | Command-specific object; `null` if no usable report was produced. |
| `error` | `null` on success, otherwise `{ "code": "...", "message": "..." }`, with optional human-readable `hint`. |

`error.code` is a stable machine identifier; `message` and `hint` are explanatory text, not parsing contracts. Serialize typed reports through CLI DTOs rather than exposing Rust debug output, scraping tables, or adding presentation fields to domain types. Prepare the full serialized response before writing stdout. Disable ANSI, progress, prompts, and human summaries on stdout; any optional diagnostics go to stderr. A stdout write failure may prevent delivery of JSON and is reported as an I/O failure, not followed by a second response.

Examples:

```json
{
  "schemaVersion": 1,
  "command": "outdated",
  "scope": "project",
  "ok": true,
  "data": { "rows": [], "problems": [] },
  "error": null
}
```

```json
{
  "schemaVersion": 1,
  "command": "check",
  "scope": "project",
  "ok": false,
  "data": {
    "healthy": false,
    "problems": [
      {
        "kind": "targetMissing",
        "skill": "review",
        "agent": "pi",
        "path": "/work/project/.pi/skills/review"
      }
    ]
  },
  "error": {
    "code": "CHECK_FAILED",
    "message": "Installed state does not match the lockfile."
  }
}
```

Publish the version-1 contract as `schemas/sksync-output.schema.json` when implementing the feature, with command-specific data variants and shared error definitions. Keep fixtures aligned with the DTOs; a new runtime schema-validation dependency is unnecessary.

#### Command data and exit behavior

| command | `data` contract | success and exit behavior |
| --- | --- | --- |
| `list` | `skills`: rows with `name`, `source`, nullable `installSource`/`include`/`lockedHash`, and `targets`. `installSource` uses the existing tagged local/Git source shape; `include` is a string array or null. Each target has `agent`, nullable resolved `target`, tagged `status`, and a structured `error` on resolution/inspection failure. `lockfileStatus` distinguishes `available` and `missing`. | Missing lockfile is allowed and gives null locked hashes. Link drift/conflicts are valid observations, not command failures. Invalid existing lockfiles and inspection/resolution failures are surfaced, not silently discarded. Listing does not add directory hashing. |
| `plan` | `items`: one row per physical target with structured `owners: [{ "skill": "...", "agent": "..." }]`, `source`, `target`, tagged `action`, and action-specific details. `applicable` states whether normal, non-force apply is unblocked. | A completed plan exits `0`, even when `applicable` is false. A conflict is plan data, not a failed planning operation. |
| `check` | `healthy` and `problems`. Each problem has a stable `kind`, skill/agent/path fields relevant to that variant, and expected/actual values where available. | Healthy: `ok: true`, exit `0`. Completed unhealthy check: `ok: false`, exit `1`, `CHECK_FAILED`, with the full report in `data`. Unreadable/invalid required inputs: execution error, normally `data: null`. |
| `outdated` | `rows`: successful outdated results retaining `skill`, `current`, `wanted`, `latest`, `source`, `status`. `problems`: failed remote queries with `skill`, `source`, `wanted`, and structured `error`. Up-to-date Git sources and non-Git dependencies are omitted as today. | All probes succeeded: exit `0`, including when updates exist. Any probe failed: `ok: false`, exit `1`, `REMOTE_QUERY_FAILED`, retaining successful rows and problems in `data`. Never put an `error: ...` string in `latest`. |

List target-status tags are `synced`, `missing`, `drifted`, `conflict`, `brokenSymlink`, `sourceMissing`, `resolveFailed`, and `inspectFailed`. Plan action tags are `createSymlink`, `alreadySynced`, `conflict`, `driftedSymlink`, and `sourceMissing`; conflict reasons are `regularFile`, `directory`, and `brokenSymlink`. Check problem kinds are `sourceHashDrift`, `targetMissing`, `targetUnexpectedSymlink`, `brokenSymlink`, `targetConflict`, `inspectFailed`, `hashFailed`, and `includeMismatch`. Fields follow the corresponding existing report variant, using camelCase (`actualSource`, for example); do not encode shared target owners as comma-joined labels.

Sort skills and remote-query problems by skill name, target lists by agent/target, plan rows by target with sorted owners, and check problems by kind/skill/agent/path. Preserve a stable documented order, not filesystem traversal order. Arrays are empty rather than null; absent variant-specific fields are omitted, while explicitly nullable row fields remain present.

Application-level errors use typed categories such as `CONFIG_NOT_FOUND`, `INVALID_CONFIG`, `LOCKFILE_NOT_FOUND`, `INVALID_LOCKFILE`, `TARGET_RESOLUTION_FAILED`, `INSPECTION_FAILED`, `REMOTE_QUERY_FAILED`, `IO_ERROR`, and `SERIALIZATION_FAILED`. Use `INTERNAL_ERROR` only for uncategorized failures. Never derive codes by parsing an error message. Failed list inspection may retain collected rows in `data`; check findings and outdated query failures always retain their report. Exit `1` covers failed checks and execution errors; `error.code` and `data` distinguish them. Human and JSON adapters share exit semantics.

Clap syntax errors remain stderr text with exit `2`, because no valid command invocation exists to render. `--help` and `--version` retain normal behavior. The envelope guarantee applies to successfully parsed JSON-enabled invocations and application-level failures, not to malformed CLI syntax. The top-level error path must not print a second JSON object or duplicate human summary after a handler has rendered its response.

#### Compatibility and migration

This intentionally replaces the published bare `outdated --json` array with the envelope; no legacy-array flag is planned. Clients that previously iterated the root array must use `.data.rows` and inspect `.ok`, `.error`, and `.data.problems`. Remote-query failure also becomes a nonzero exit in human and JSON modes rather than an apparently successful row containing error text. `list` likewise stops silently ignoring malformed existing lockfiles, and resolution/inspection failures become nonzero exits in both output modes; a missing optional lockfile remains valid.

Before releasing the implementation, announce these changes in release notes, update the manual/examples, and add regression fixtures for all four envelopes. Do not change the advertised v0.0.15 format before implementation. Compatible additions may add optional fields; clients must ignore unknown fields. Renaming/removing fields, changing their types, or changing existing tag meanings requires a new `schemaVersion` and a documented migration. A future new version needs explicit negotiation; do not silently make `--json` emit an incompatible schema for existing clients.

### 13.4 Selected updates

Planned CLI syntax:

```sh
sksync update [skills...] [--global]
# Examples:
sksync update review
sksync update review browser --global
```

- No names means all dependency-managed skills, preserving the existing command intent. Legacy `skills` entries without an install source are not fetched. An empty effective dependency selection is a successful no-op without body/lockfile writes.
- Explicit names are config dependency keys, not frontmatter names, paths, bundle names, or patterns. Validate through `SkillName`, including rejecting the reserved path-component names `.` and `..` in its shared constructor, match case-sensitively, deduplicate, and process in stable name order. Unknown names or explicit legacy-only skills abort the whole command before fetching or replacing anything; never silently ignore a typo.
- Use only the chosen project/global config; no fallback lookup in the other scope. Membership in a bundle does not expand the selection. Update local and Git install sources using the configured `include` filters, while retaining the configured source string/ref and bundle provenance.
- Explicitly selected updates require a readable existing lockfile and entries for every unselected configured skill. Missing baseline entries or a missing/invalid lockfile fail before fetch, with guidance to establish the baseline using full `install`/`update`. Selected entries may be newly added and initially absent; rollback must restore their original body absence.
- Construct a selected-update lockfile by **merging into the existing lockfile**, replacing only selected entries with their newly resolved source, include filter, hashes, and files. Preserve all unselected entries, including stale entries whose config dependency has disappeared; pruning belongs to removal/full reconciliation, not selective update. Preserve supported-version content semantics when normalizing an older lockfile to the current format. Version 5 still omits agent targets; legacy target metadata is dropped only under the existing migration rules, not interpreted as permission to change links.
- Do not refetch, hash, rewrite, repair, or require healthy bodies/targets for unselected skills. In particular, a missing unselected body or locally modified unselected content must not block the selected update or get silently blessed by a regenerated hash. Keep the full config available; selection is not a smaller replacement config.
- Do not require a link plan to construct the selected version-5 lockfile: it records content, not target placement. `update` neither inspects nor applies target symlinks; shared-target ownership remains represented by unchanged config and normal link planning. Target diagnosis/repair belongs to `plan`, `check`, and `apply`.
- Keep the top-level lockfile version/root compatibility rules. `generatedAt` and `generatedBy` may change after a successful update; this does not authorize regenerating unselected entries. Full update retains its complete-lockfile construction behavior.
- A later `outdated`/`check` may continue to report drift in unselected skills. That is expected, not a reason for selective update to widen its scope.
- `outdated` is an observation, not a frozen approval plan: a moving remote ref may change before `update`. The lockfile records the exact commit actually prepared. No new `--force`, `--dry-run`, `--json`, interactive selection menu, or per-file diff UI is added to `update` in this iteration.

The implementation may reuse the name-filtering logic in `update_selected_dependencies`, already used by `add`. It **must not** reuse the current immediate-replacement loop as the batch transaction: selection and preparation happen before any live publication. Existing `add` behavior remains limited to newly added dependencies; it must not inherit a whole-config fetch from the refactor.

### 13.5 Module responsibilities and implementation sequence

Follow the layered architecture in `ARCHITECTURE.md`; the earlier conceptual module sketch is not a reason to introduce parallel models.

| location | responsibility |
| --- | --- |
| `src/application/update.rs` | Validate selection, coordinate prepare/publish/rollback, merge selected lock entries, and return reports only after the commit decision. Keep full config and selected names separate. |
| `src/application/ports.rs` | Extend the existing installer/store seams with prepared-install metadata and opaque publication/rollback receipts. Preserve testability without a generic transaction or mock-filesystem framework. |
| `src/infrastructure/install.rs` | Own private sibling staging/backup paths and per-directory prepare, publish, restore, and finalize operations. Retain the safe single-install wrapper for other callers. |
| `src/infrastructure/atomic_file.rs` (new) | Complete-file atomic/create-only publication and owned temporary cleanup; shared by normal and restoration writes. |
| `src/infrastructure/write_guard.rs` (new) | Non-blocking canonical directory guards and deterministic acquisition/release; no PID-based stale-lock heuristic. |
| `src/infrastructure/json.rs` and existing config writers | Serialize domain/config values, preserve config fields, and delegate byte publication to the shared helper. |
| `src/cli.rs` and `src/cli/output.rs` (new) | Parse flags/names, adapt existing reports to explicit JSON DTOs, classify errors, and render exactly once. No filesystem recovery rules in the renderer. |
| `src/application/list.rs`, `check.rs`, `outdated.rs` | Expose typed observations/errors needed by both output adapters; no table parsing or string-encoded remote failures. |

Sequence:

1. Add failure-path tests, atomic file persistence, guarded writer entry points, and backup-preserving directory replacement. Make full `update` a prepared batch with the lockfile as commit point. Adapt all callers of the shared write/install seams without broadening their command guarantees.
2. Add the common JSON adapter for the four read commands, typed remote-query failures, exit handling, schema fixtures, and migration documentation. Human output remains the default.
3. Expose selected names on CLI `update`, implement lockfile merge without unselected inspection, and verify mixed selected/unselected state. No additional TUI workflow is required.

### 13.6 Acceptance checks

Use temporary directories and injected home/config roots only; never touch the real home directory or require live network services. Use existing port fakes plus targeted failure injection at publication boundaries, not a new general filesystem abstraction.

Persistence/update checks:

- Serialization, temporary write/sync, or publication failure leaves the original file byte-for-byte intact; absent originals stay absent. Create-only publication refuses an existing destination. Config symlinks remain symlinks with complete updated referents and preserved permissions.
- Failure preparing the second skill leaves every old body and lockfile unchanged. Failure publishing any body or writing the final lockfile restores all earlier publications, including bodies that were originally absent.
- A rollback failure retains the only old copy, reports both failures and recovery paths, and never emits a success report. Post-commit cleanup failure leaves new bodies/new lockfile committed and reports a warning.
- Unexpected regular files/symlinks at managed-body destinations are not deleted. Existing target-file/directory and force-only-symlink-repair tests continue to pass.
- Two writers sharing either a physical state parent or skill store cannot publish concurrently; guards release on error/process exit. Verify directory locking and complete-file replacement on macOS and Linux, including alias paths. Readers need not acquire a writer guard.

JSON checks:

- All four commands emit exactly one parseable envelope without human text/ANSI on stdout, for success and application errors. Validate required fields, nullable fields, tags, sorted arrays, and stable codes with fixtures.
- A blocked plan succeeds with `applicable: false`; unhealthy check fails with report data; empty outdated results succeed; failed remote queries fail with typed problems and preserve successful rows.
- Missing optional list lockfile differs from malformed existing lockfile. Valid JSON invocations with missing config/required lockfile produce envelopes; malformed flags retain Clap exit `2`.
- Human and JSON modes use the same reports/exit decisions. JSON mode does not add fetching, writes, prompts, or symlink repair. Test the documented migration from a root array to `.data.rows`.

Selection checks:

- Updating one Git skill leaves every other body, include filter, resolved commit, and file/hash record unchanged. Keep dependency agents and actual target links unchanged; no target records are added to version-5 lockfiles. Cover both a missing and a locally edited unselected body; neither is hashed or fetched.
- Cover multiple names, duplicates, unknown names, reserved `.`/`..` names, legacy-only names, empty effective selection, local sources, global/project isolation, missing/invalid baselines, and bundle-owned/shared-target dependencies. Keep config/bundle JSON-schema name constraints aligned with the shared validation.
- Any selected failure restores the entire selection and old lockfile. Config/provenance and existing symlinks remain unchanged on success and failure. Existing `add` isolation and full-update/install behavior retain regression coverage.

### References

The adoption is limited to local persistence patterns and machine-readable contracts, reimplemented through sksync's existing seams rather than importing skilld's framework. Reference snapshot: skilld commit `3ef9256ff19a8d6403c6d55286c6131212bc7896`.

- [skilld local-store staging, backups, and lock publication](https://github.com/skilld-dev/skilld/blob/3ef9256ff19a8d6403c6d55286c6131212bc7896/crates/skilld-command/src/local_store.rs)
- [skilld versioned output envelopes](https://github.com/skilld-dev/skilld/blob/3ef9256ff19a8d6403c6d55286c6131212bc7896/crates/skilld-command/src/output.rs)
- [skilld selective-update UI](https://github.com/skilld-dev/skilld/blob/3ef9256ff19a8d6403c6d55286c6131212bc7896/crates/skilld-native/src/update_ui.rs)
- [Standard-library advisory locking and compiler floor](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
