# Commands

Full reference for the sksync CLI. Commands are shown as `sksync <command>`; from a clone the equivalent is `cargo run -- <command>`.

Most commands accept `--global` to operate on `~/.sksync/config.json` (user scope) instead of the project's `sksync.config.json`.

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
```

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to link the skill into. Repeatable. Required. |
| `--name <skill>` | Disambiguate repo-root discovery to a single skill by frontmatter/dir name. |
| `--global` | Add to `~/.sksync/config.json`. |

Source forms (GitHub shorthand, tree URL, skills.sh, local) and discovery rules → [Sources & Discovery](/guides/sources). Fetched skills are validated for `SKILL.md` + frontmatter `name`/`description` before install.

## `sksync attach`

Link an already dependency-managed skill into an additional agent, preserving its existing source representation, then fetch and symlink.

```sh
sksync attach <skill> --agent <agent> [--agent <agent> …]
sksync attach <skill> --agent <agent> --global
```

## `sksync bundle`

Inspect, add, remove, export, and preview sync drift for curated bundle install sets. Bundles expand into normal dependencies; they are not runtime folders.

```sh
sksync bundle inspect <source>
sksync bundle add <source> --agent <agent> [--agent <agent> …]
sksync bundle add <source> --agent <agent> --dry-run
sksync bundle remove <name> [--source <exact-source>]
sksync bundle remove <name> --dry-run
sksync bundle sync <name> [--source <exact-source>] --dry-run
sksync bundle export <name> --output <dir> [--snapshot]
sksync bundle export <name> --output <dir> --skill <skill> --dry-run
```

| Subcommand | Meaning |
|---|---|
| `inspect` | Read a bundle manifest and print normalized entry sources. Read-only. |
| `add` | Add every bundle entry to the selected agents. Aborts on any conflict. |
| `remove` | Remove local bundle provenance and delete only bundle-managed dependencies whose last provenance is removed. |
| `sync` | Preview latest manifest membership drift for an already-added bundle. Apply is not implemented yet. |
| `export` | Generate `sksync.bundle.json` from current project or global dependencies. |

| Flag | Meaning |
|---|---|
| `--agent <agent>` | Agent to link bundle entries into. Repeatable. Required for `bundle add`. |
| `--source <exact-source>` | Disambiguate duplicate bundle names during `bundle remove` or `bundle sync`. |
| `--output <dir>` | Directory that will contain the exported `sksync.bundle.json`. Required for `bundle export`. |
| `--snapshot` | Copy installed skill bodies into the bundle directory and write `./skills/<name>` sources. |
| `--skill <name>` | Export only this dependency. Repeatable. |
| `--force` | Replace an existing export output directory. |
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
```

## `sksync update`

Fetch the latest (or pinned) skill from each `dependencies` source into `skillDir` and update `sksync-lock.json`. Validates fetched skills.

```sh
sksync update
sksync update --global
```

## `sksync apply`

Run only the planner's create-symlink actions, then write `sksync-lock.json`. Fails on missing source, conflict, or drift. `--force` updates a target only when it is an existing sksync-managed link.

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

Interactive prompt flow for add / attach / detach / remove / default-agents / list+check / plan+apply. `ask` and `tui` are compatible aliases.

```sh
sksync wizard
sksync ask
sksync tui
```

## Safety rules

- Existing plain files are never overwritten.
- `add` rolls back the dependency config on failure.
- `bundle add` aborts on any conflict and rolls back config/lockfile writes on failure.
- `bundle remove` uses local provenance only and cannot delete manual dependencies solely because they once belonged to a bundle.
- `remove` deletes only sksync-managed symlinks, and installed files only when inside the configured `skillDir`.
- Git source subpaths reject absolute paths and `..`, and must stay inside the clone directory.
- Project-scope agent target directories cannot escape the project root.
- `apply` executes create-symlink actions only and fails on conflict / drift / missing source.
- `install` prefers the lockfile's resolved sources when present; `update` fetches the latest and re-locks.
- Target parent directories are created as needed.

## Related

- [Quickstart](/quickstart) — the common `init → add → plan → apply` flow.
- [Project Config](/guides/project-config) · [Agent Mappings](/guides/agent-mappings) · [Sources](/guides/sources) · [Lockfile & Sync](/guides/lockfile)
