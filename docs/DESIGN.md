# sksync Design

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for architectural principles.

## 1. Background

Agent Skills are stored in different locations depending on the agent.

Examples:

- Claude Code skills
- Codex instructions / skills
- Gemini CLI context / extensions
- OpenCode command / agent config
- Pi `.pi/agent/skills` or user-configured paths

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

Source strings should stay compact. `sksync add <source> --agent <agent>` updates `dependencies` and then runs install/apply behavior. With `--global`, it updates `~/.sksync/config.json`.

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
| pi | `~/.pi/agent/skills` | `.pi/agent/skills` | Matches existing Pi skill format. |
| claude-code | `~/.claude/skills` | `.claude/skills` | Claude Code skill directory. |
| codex | `~/.codex/skills` | `.codex/skills` | May need instruction conversion later. |
| gemini | `~/.gemini/skills` | `.gemini/skills` | Aligned with Gemini CLI. |
| jcode | `~/.jcode/skills` | `.jcode/skills` | jcode skill directory. |
| opencode | `~/.config/opencode/skills` | `.opencode/skills` | Watch for OS-specific differences. |
| antigravity | `~/.gemini/antigravity/skills` | `.agents/skills` | Workspace default is `.agents/skills`. |
| universal | `~/.agents/skills` | `.agents/skills` | Canonical Agent Skills directory. |

## 6. Lockfile

File name: `sksync-lock.json`

Schema: [`schemas/sksync-lock.schema.json`](../schemas/sksync-lock.schema.json)

Like `package-lock.json`, the lockfile stores the information needed for `sksync install` to reconstruct the same skill bodies on another environment. Supported OS targets are macOS and Linux for now; Windows-specific path/symlink differences are out of scope. Linux distribution assets use musl to avoid glibc-version coupling.

### Portable lockfile v4

Lockfile v4 avoids machine-local absolute paths.

- `root` is always `"."`.
- `skills.<name>.source` is relative to the lockfile directory.
- `installSource` stores an exact source that can be fetched again.
  - Git sources store `url`, resolved commit `ref`, and repo `path`.
  - `skills.sh` input is normalized to an exact GitHub tree URL at add time and locked as a Git source.
  - Local sources under the project/global root can be stored as relative paths.
  - Absolute local sources outside the project/global root are non-portable.
- `files[].path` is relative to the skill directory.
- Agent target paths and symlink state are not stored.

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync-lock.schema.json",
  "lockfileVersion": 4,
  "generatedBy": "sksync@0.0.7",
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

Existing v3 lockfiles remain readable. If v3 contains absolute `root` / `source`, it is treated as legacy and rewritten to v4 relative form the next time `install`, `update`, or `apply` writes a lockfile.

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
- `add`: accept GitHub / `skills.sh` / local sources, add a dependency, support multiple `--agent`, and run install/apply behavior; `--force` passes through to the final link apply step.
- `install`: prefer lockfile `installSource`; otherwise fetch from config and create a lockfile; then apply managed symlinks; `--force` repairs drifted or broken target symlinks during the apply step.
- `update`: fetch dependencies, resolve Git sources to exact commits, and refresh the lockfile. It does not apply links and therefore has no `--force`.
- `attach`: add agents to an existing dependency-managed skill while preserving source representation; `--force` passes through to the final link apply step.
- `remove <skills...>`: remove config entries, installed files, managed symlinks, and lockfile entries; support `--keep-files`, `--config-only`, and `--global`. It does not support generic `--force`.
- `remove <skill> --agent <agent>`: remove selected agent symlinks and targets only; full removal if the last agent is removed.
- `outdated`: compare Git lockfile commits with remote ref HEAD; support `--global` and `--json`.
- `bundle`: inspect/add/remove/export curated install sets; `bundle add --force` and `bundle sync --force` pass through to their final link apply step; `bundle export --force` replaces an existing generated output directory.
- `apply`: resolve targets, detect conflicts, create/update symlinks, and write lockfile; `--force` only allows symlink repair/replacement, never regular-file or directory replacement.
- `check`: compare config, lockfile, hashes, sources, and symlink health.
- `doctor`: read-only comprehensive diagnosis with suggested next commands, never automatic repair.
- `agents`: list effective mappings, diagnose target directories, and refresh bundled mappings.
- `import`: copy-only migration from existing skill directories; no original files are mutated.
- `wizard`: prompt-based wrapper around CLI/application use cases.

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
  create symlink: .pi/agent/skills/cuekit-dogfood
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
- `Add bundle` loads the manifest after source entry, shows the bundle name, description, and entries, and asks the user to continue before agent selection.
- `Remove bundle` lists exact bundle provenance choices as `name — source` so same-name bundles from different sources are never ambiguous in the wizard.
- Bundle wizard flows stop at add/remove for the first bundle UX iteration; `bundle sync` starts as a CLI flow with dry-run preview.
- Status uses `list` / `check` summaries; there is no persistent screen UI.

## 9. Safety rules

- Do not overwrite existing regular files or directories, even with `--force`.
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
    mod.rs         # prompt / wizard entry
```

### Architecture direction

- Planner builds the diff between desired state and current state.
- Apply executes only planner results.
- CLI and TUI share planner / apply / check logic.
- TUI contains no core logic.
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
