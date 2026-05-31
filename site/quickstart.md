# Quickstart

Get a shared skill symlinked into multiple coding agents in a few minutes. This page assumes you have [installed sksync](/install) and that `git` is on your `PATH`.

## 1. Initialize a project

In the repo where you want to manage skills:

```sh
cd /path/to/your/repo
sksync init
```

This creates:

- `sksync.config.json` — your dependency manifest
- `.sksync/skills/` — where skill bodies are stored

::: tip
Manage skills for your whole machine instead of one repo with `sksync init --global`, which writes `~/.sksync/config.json`, `~/.sksync/agents.json`, and `~/.sksync/skills/`. Commands that read or write config accept `--global`; manifest-only commands such as `sksync bundle inspect` do not need it.
:::

## 2. Add a skill for one or more agents

Point `sksync add` at a source and list the agents that should see the skill:

```sh
sksync add owner/repo/path/to/skill --agent claude-code --agent pi
```

sksync fetches the skill into `.sksync/skills/<skill>`, records it in `sksync.config.json`, and creates the symlinks. If anything fails along the way, the config is rolled back to its previous state.

Sources can be GitHub shorthand, a GitHub tree URL, a `skills.sh` URL, or a local directory — see [Sources & Discovery](/guides/sources).

```sh
# Discover SKILL.md under a repo root and pick interactively
sksync add owner/repo --agent claude-code

# From skills.sh
sksync add https://www.skills.sh/owner/repo/skill-name --agent pi

# From a local directory
sksync add ./local-skill --agent claude-code --agent gemini
```

## Optional: add a team bundle

If your team publishes a bundle, inspect it first, then dry-run the add:

```sh
sksync bundle inspect ./bundles/review-workflow
sksync bundle add ./bundles/review-workflow --agent claude-code --agent pi --dry-run
```

When the plan looks right, install every bundle entry into the selected agents:

```sh
sksync bundle add ./bundles/review-workflow --agent claude-code --agent pi
```

Bundles expand into normal dependencies. They do not create runtime bundle folders, and agents still see flat skills. See [Bundles](/guides/bundles) for manifest authoring, provenance, and removal behavior.

## 3. Preview the plan

See what sksync would create, what is already in sync, and any conflicts or drift — without touching the filesystem:

```sh
sksync plan --dry-run
```

## 4. Apply

Create the symlinks the planner proposed and write the lockfile:

```sh
sksync apply
```

`apply` creates missing symlinks and fails if a source is missing, a target conflicts with an unmanaged file, or drift is detected. Use `--force` to repair drifted or broken target symlinks; regular files and directories are never replaced.

## 5. List and check

Show configured skills with each agent's target path and status:

```sh
sksync list
```

Verify the installed state against the lockfile — detecting hash drift, missing targets, and broken symlinks:

```sh
sksync check
```

`check` exits non-zero when something is wrong, so it works well in scripts.

## What just happened

- `sksync add` resolved the source, validated `SKILL.md`, stored the body under `.sksync/skills/<skill>`, and recorded the dependency.
- `sksync apply` symlinked that single body into each agent's skills directory and wrote `sksync-lock.json`.
- sksync never overwrote a plain file and only created links it can later manage and remove.

## Keep it in sync

| You want to… | Command |
|---|---|
| See which skills have upstream updates | `sksync outdated` |
| Pull latest from sources and update the lockfile | `sksync update` |
| Reconstruct skills from the lockfile (e.g. fresh clone) | `sksync install` |
| Re-run only the symlink creation | `sksync apply` |
| Diagnose config / targets / mappings (read-only) | `sksync doctor` |

## Next steps

- **Understand the manifest** → [Project Config](/guides/project-config)
- **See where each agent's skills live** → [Agent Mappings](/guides/agent-mappings)
- **All source formats and discovery rules** → [Sources & Discovery](/guides/sources)
- **Team bundle manifests and provenance** → [Bundles](/guides/bundles)
- **Lockfile, reproducible installs, and updates** → [Lockfile & Sync](/guides/lockfile)
- **Full command reference** → [Commands](/reference/commands)
