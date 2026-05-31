# Commands

Full reference for the sksync CLI. Commands are shown as `sksync <command>`; from a clone the equivalent is `cargo run -- <command>`.

Most commands accept `--global` to operate on `~/.sksync/config.json` (user scope) instead of the project's `sksync.config.json`. Long-running operations print short progress phase messages to stderr; command results and tables remain on stdout.

[[toc]]

## `sksync init`

Scaffold config for a new project or globally.

```sh
sksync init                 # project: sksync.config.json + .sksync/skills/
sksync init --global        # global:  ~/.sksync/config.json + agents.json + skills/
sksync init --agents        # force-refresh only ~/.sksync/agents.json
```

- Project mode creates `sksync.config.json` and `.sksync/skills/`.
- Global mode (`--global`) creates `~/.sksync/config.json`, `~/.sksync/agents.json`, and `~/.sksync/skills/`.
- Fails rather than overwriting an existing config. In global mode, an existing `agents.json` is left untouched.
- `--agents` touches neither config nor skill directories — it force-rewrites only `agents.json` from bundled defaults (same effect as [`agents refresh`](#sksync-agents)).

## `sksync add`

Add an Agent Skills source as a dependency: append to the config, fetch the skill, and create the symlinks. Rolls the config back on any failure during install / plan / apply.

```sh
sksync add <source> --agent <agent> [--agent <agent> …]
sksync add <source> --name <skill> --agent <agent>
sksync add <source> --agent <agent> --global
sksync add <source> --agent <agent> --force
```

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to link the skill into. Repeatable. Required. |
| `--name <skill>` | Disambiguate repo-root discovery to a single skill by frontmatter/dir name. |
| `--force` | During the final link apply step, replace drifted or broken target symlinks only. Never replaces files or directories. |
| `--global` | Add to `~/.sksync/config.json`. |

Source forms (GitHub shorthand, tree URL, skills.sh, local) and discovery rules → [Sources & Discovery](/guides/sources). Fetched skills are validated for `SKILL.md` + frontmatter `name`/`description` before install.

## `sksync attach`

Link an already dependency-managed skill into an additional agent, preserving its existing source representation, then fetch and symlink.

```sh
sksync attach <skill> --agent <agent> [--agent <agent> …]
sksync attach <skill> --agent <agent> --global
sksync attach <skill> --agent <agent> --force
```

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to link the skill into. Repeatable. Required. |
| `--force` | During the final link apply step, replace drifted or broken target symlinks only. Never replaces files or directories. |
| `--global` | Use `~/.sksync/config.json`. |

## `sksync bundle`

Inspect, add, remove, export, and synchronize curated bundle install sets. Bundles expand into normal dependencies; they are not runtime folders.

```sh
sksync bundle inspect <source>
sksync bundle add <source> --agent <agent> [--agent <agent> …]
sksync bundle add <source> --agent <agent> --dry-run
sksync bundle add <source> --agent <agent> --force
sksync bundle remove <name> [--source <exact-source>]
sksync bundle remove <name> --dry-run
sksync bundle sync <name> [--source <exact-source>] [--agent <agent> …] [--dry-run]
sksync bundle sync <name> --force
sksync bundle export <name> --output <dir> [--snapshot]
sksync bundle export <name> --output <dir> --skill <skill> --dry-run
```

| Subcommand | Meaning |
|---|---|
| `inspect` | Read a bundle manifest and print normalized entry sources. Read-only. |
| `add` | Add every bundle entry to the selected agents. Aborts on any conflict. |
| `remove` | Remove local bundle provenance and delete only bundle-managed dependencies whose last provenance is removed. |
| `sync` | Preview and apply manifest membership drift for an already-added bundle. Source changes and missing agents block writes. |
| `export` | Generate `sksync.bundle.json` from current project or global dependencies. |

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to link bundle entries into. Repeatable. Required for `bundle add`; fallback-only for `bundle sync` when dependency agents cannot be inferred. |
| `--source <exact-source>` | Disambiguate duplicate bundle names during `bundle remove` or `bundle sync`. |
| `--output <dir>` | Directory that will contain the exported `sksync.bundle.json`. Required for `bundle export`. |
| `--snapshot` | Copy installed skill bodies into the bundle directory and write `./skills/<name>` sources. |
| `--skill <name>` | Export only this dependency. Repeatable. |
| `--force` | For `bundle add` / `bundle sync`, replace drifted or broken target symlinks during the final link apply step only. For `bundle export`, replace an existing generated output directory. |
| `--dry-run` | Show planned add/remove/export/sync work without writing. |
| `--global` | Operate on the global config. Supported by `bundle add`, `bundle remove`, `bundle sync`, and `bundle export`; `bundle inspect` is manifest-only. |

Example `bundle add --dry-run` output:

```text
Bundle add plan (2)
create review <- ./bundles/review-workflow/skills/review
merge qa <- https://github.com/org/qa-skills/tree/main/skills/qa
```

Example `bundle remove --dry-run` output:

```text
Bundle remove plan (2)
remove review (*)
detach-provenance qa (*)
```

Example `bundle sync --dry-run` output:

```text
Bundle sync plan (1)
Bundle: review-workflow
Source: ./bundles/review-workflow
keep: 2
add lint <- ./bundles/review-workflow/skills/lint
  agents: pi, claude-code
```

See [Bundles](/guides/bundles) for manifest authoring, provenance, migration, and troubleshooting guidance.

## `sksync agents`

Inspect and update agent target mappings (`~/.sksync/agents.json`).

```sh
sksync agents list       # print resolved mappings
sksync agents doctor     # read-only: check targetDir existence and writability
sksync agents refresh    # rewrite agents.json from bundled defaults
```

See [Agent Mappings](/guides/agent-mappings).

## `sksync doctor`

Read-only, comprehensive diagnosis of config / lockfile / sources / targets / agent mapping. Prints the next command to try and exits non-zero on problems. Performs **no** auto-repair and creates no directories.

```sh
sksync doctor
sksync doctor --global
```

## `sksync import`

Copy skills from an existing agent skills directory into `.sksync/skills` (or `~/.sksync/skills`) and register them as dependencies of the given agent(s). Copy-only: the source directory is never modified, deleted, or replaced with a symlink. Reflect symlinks afterward with [`plan`](#sksync-plan) / [`apply`](#sksync-apply).

```sh
sksync import ~/.claude/skills --agent claude-code --dry-run
sksync import ~/.claude/skills --agent claude-code
sksync import ~/.agents/skills --agent universal --agent pi
sksync import ~/.jcode/skills --agent jcode --global
```

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to register the imported skills under. Repeatable. |
| `--dry-run` | Show what would be imported without writing. |
| `--global` | Import into `~/.sksync/skills` and global config. |

## `sksync remove`

Remove skills from dependency config, the installed skill directory, managed symlinks, and the lockfile. Installed bodies are deleted **only** when inside the configured `skillDir`; local / legacy unmanaged source directories are left alone.

```sh
sksync remove <skill> [<skill> …]
sksync remove <skill> --global
sksync remove <skill> --keep-files
sksync remove <skill> --config-only
```

Per-agent removal — unlink only the given agent(s), keeping other links and the `.sksync/skills/<skill>` body:

```sh
sksync remove <skill> --agent pi
sksync remove <skill> --agent pi --agent claude-code
```

Removing the last agent is equivalent to removing the whole skill.

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Unlink only this agent. Repeatable. |
| `--keep-files` | Remove config/links but keep the installed body. |
| `--config-only` | Only edit config; leave files and links in place. |
| `--global` | Operate on the global config. |

## `sksync outdated`

Compare the lockfile against upstream and list updatable skills. Git sources compare the remote ref HEAD against the lockfile's resolved commit.

```sh
sksync outdated
sksync outdated --global
sksync outdated --json
```

## `sksync plan`

Read the config, inspect current target state, and report create / in-sync / conflict / drift. Read-only with `--dry-run`.

```sh
sksync plan --dry-run
sksync plan --global
```

## `sksync install`

Reconstruct skills, preferring the lockfile's resolved sources when `sksync-lock.json` exists; otherwise fetch from config and create the lockfile. Creates symlinks.

```sh
sksync install
sksync install --global
sksync install --force
```

`--force` applies only after skills are reconstructed: it may replace drifted or broken target symlinks, but never regular files or directories.

## `sksync update`

Fetch the latest (or pinned) skill from each `dependencies` source into `skillDir` and update `sksync-lock.json`. Validates fetched skills.

```sh
sksync update
sksync update --global
```

## `sksync apply`

Run only the planner's create-symlink actions, then write `sksync-lock.json`. Without `--force`, apply fails on missing source, conflict, or drift. With `--force`, apply may unlink and recreate drifted or broken target symlinks; regular files and directories remain blocking conflicts.

```sh
sksync apply
sksync apply --force
sksync apply --global
```

## `sksync check`

Compare the lockfile against current state — source hash drift, missing targets, broken symlinks. Exits non-zero on any problem.

```sh
sksync check
sksync check --global
```

## `sksync list`

List configured skills with each agent's target path and status. Shows locked hashes when `sksync-lock.json` exists.

```sh
sksync list
sksync list --global
```

## `sksync wizard`

Interactive prompt flow for add / attach / detach / remove skill operations, Add bundle, Remove bundle, default-agents, list+check, and plan+apply. `ask` and `tui` are compatible aliases.

```sh
sksync wizard
sksync ask
sksync tui
```

## Safety rules

- Existing plain files and directories are never overwritten, even with `--force`.
- `add` rolls back the dependency config on failure.
- `bundle add` aborts on any config/provenance conflict and rolls back config/lockfile writes on failure.
- `bundle remove` uses local provenance only and cannot delete manual dependencies solely because they once belonged to a bundle.
- `remove` deletes only sksync-managed symlinks, and installed files only when inside the configured `skillDir`.
- Git source subpaths reject absolute paths and `..`, and must stay inside the clone directory.
- Project-scope agent target directories cannot escape the project root.
- Link-applying commands support `--force` only for drifted or broken target symlinks; missing sources, regular files, and directories are never force-replaced. Commands that skip blocked targets leave those targets unchanged and report them in the printed plan.
- `install` prefers the lockfile's resolved sources when present; `update` fetches the latest and re-locks.
- Target parent directories are created as needed.

## Related

- [Quickstart](/quickstart) — the common `init → add → plan → apply` flow.
- [Project Config](/guides/project-config) · [Agent Mappings](/guides/agent-mappings) · [Sources](/guides/sources) · [Lockfile & Sync](/guides/lockfile)
