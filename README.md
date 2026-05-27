# sksync

`sksync` is a CLI tool that syncs Agent Skills target directories for multiple coding agents from a single configuration file.

## Purpose

- Create symlinks from one shared skill body into each agent's expected skills directory.
- Keep source skill bodies under `.sksync/skills/` and safely link them into agent directories.
- Support bundled target mappings for major agents such as Claude Code, Codex, Gemini, jcode, OpenCode, Pi, and Antigravity.
- Add skills from GitHub, local directories, or `skills.sh` URLs and make them reproducible with a lockfile.

## CLI

### Install on macOS / Linux

Install a prebuilt binary from GitHub Releases to `~/.local/bin/sksync`. macOS uses Apple Silicon / Intel assets; Linux uses x86_64 / aarch64 musl assets.

```bash
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | sh
```

Choose a different install directory:

```bash
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | INSTALL_DIR=/usr/local/bin sh
```

The installer automatically selects one of these release assets:

| OS | Architecture | Asset target |
| --- | --- | --- |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| macOS | Intel | `x86_64-apple-darwin` |
| Linux | x86_64 / amd64 | `x86_64-unknown-linux-musl` |
| Linux | arm64 / aarch64 | `aarch64-unknown-linux-musl` |

Linux uses musl assets so the same binary works across multiple distributions such as Debian and Ubuntu. Linux assets are available from releases that include this support. If `latest` predates them, build from source or set `VERSION=v...` to a newer tag. Windows is not supported yet.

### Uninstall

If you installed with `install.sh`, remove the installed binary:

```bash
rm -f ~/.local/bin/sksync
```

If you used a custom `INSTALL_DIR`, delete the binary from that directory instead:

```bash
rm -f /usr/local/bin/sksync
```

If you installed from a repository clone with `just install`, use the same `INSTALL_DIR` with `just uninstall`:

```bash
just uninstall
# or
INSTALL_DIR=/usr/local/bin just uninstall
```

To fully reset global config, agent mappings, and installed global skills, remove `~/.sksync` as well:

```bash
rm -f ~/.local/bin/sksync
rm -rf ~/.sksync
```

### Build

```bash
cargo build
cargo test
cargo run -- --help
```

Run commands locally through `cargo run --`:

```bash
cargo run -- init
cargo run -- init --global
cargo run -- init --agents
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- attach skill-name --agent gemini
cargo run -- agents list
cargo run -- agents doctor
cargo run -- agents refresh
cargo run -- doctor
cargo run -- import ~/.claude/skills --agent claude-code --dry-run
cargo run -- import ~/.agents/skills --agent universal --agent pi
cargo run -- bundle inspect ./bundle-dir
cargo run -- bundle add ./bundle-dir --agent pi --dry-run
cargo run -- bundle remove bundle-name --dry-run
cargo run -- remove skill-name
cargo run -- remove skill-a skill-b
cargo run -- outdated
cargo run -- install
cargo run -- update
cargo run -- plan --dry-run
cargo run -- apply
cargo run -- check
cargo run -- list
cargo run -- wizard
```

After `cargo build`, you can also run `./target/debug/sksync ...`.

### `sksync init`

Create a starter config for a new project.

```bash
cargo run -- init
# or initialize global config
cargo run -- init --global
# or force-refresh only ~/.sksync/agents.json
cargo run -- init --agents
```

Project mode creates:

- `sksync.config.json`
- `.sksync/skills/`

Global mode (`--global`) creates:

- `~/.sksync/config.json`
- `~/.sksync/agents.json`
- `~/.sksync/skills/`

If the target config already exists, `init` fails instead of overwriting it. In global mode, an existing `agents.json` is also left untouched.

`init --agents` does not touch config files or skills directories. It only overwrites `~/.sksync/agents.json` with the bundled default mappings. Use it to pick up new agent mappings. `sksync agents refresh` provides the same refresh behavior.

#### Agent target mappings

`~/.sksync/agents.json` stores both global and project agent target directory mappings. Project config uses `project` mappings, while global config (`--global`) uses `global` mappings. Inline `agents` overrides in `sksync.config.json` take highest priority.

The bundled mappings include entries for major Agent Skills-compatible agents. Examples:

| Agent | Global targetDir | Project targetDir |
| --- | --- | --- |
| `pi` | `~/.pi/agent/skills` | `.pi/agent/skills` |
| `claude-code` | `~/.claude/skills` | `.claude/skills` |
| `codex` | `~/.codex/skills` | `.codex/skills` |
| `jcode` | `~/.jcode/skills` | `.jcode/skills` |
| `gemini` / `gemini-cli` | `~/.gemini/skills` | `.gemini/skills` |
| `opencode` | `~/.config/opencode/skills` | `.opencode/skills` |
| `antigravity` | `~/.gemini/antigravity/skills` | `.agents/skills` |
| `universal` | `~/.agents/skills` | `.agents/skills` |

Antigravity uses the official workspace default `.agents/skills`. Antigravity treats `.agent/skills` as a backward-compatible directory, but sksync's bundled default is `.agents/skills`.

`universal` is the canonical Agent Skills ecosystem directory. It maps to `~/.agents/skills` globally and `.agents/skills` in projects.

#### Default wizard agents

Set `defaultAgents` in config to preselect agents in the wizard's `Add skill` flow. CLI `sksync add` still requires explicit `--agent` arguments.

```json
{
  "defaultAgents": ["universal", "pi"]
}
```

You can also set `defaultAgents` from the wizard's `Configure default agents` flow.

### `sksync add`

Add an Agent Skills source as a dependency. Given a source and one or more agents, sksync updates dependency config, installs the skill, and creates symlinks. If install / plan / apply fails, the config is rolled back.

```bash
cargo run -- add <source> --agent pi [--agent claude-code]
```

Common examples:

```bash
# GitHub shorthand / prefix / tree URL
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- add github:owner/repo/path/to/skill#main --agent pi
cargo run -- add https://github.com/owner/repo/tree/main/path/to/skill --agent pi

# Discover and select SKILL.md files from a repo root
cargo run -- add owner/repo --agent pi
cargo run -- add owner/repo --name skill-name --agent pi

# skills.sh URL / shorthand
cargo run -- add skills.sh/owner/repo --agent pi
cargo run -- add https://www.skills.sh/owner/repo --agent pi
cargo run -- add https://www.skills.sh/owner/repo/skill-name --agent pi

# local directory
cargo run -- add ./local-skill --agent pi --agent gemini
```

#### Source formats

| Format | Meaning |
| --- | --- |
| `owner/repo/path/to/skill#ref` | GitHub shorthand. Clones `owner/repo` and uses `path/to/skill` as the skill directory. |
| `github:owner/repo/path/to/skill#ref` | Explicit GitHub shorthand. |
| `https://github.com/owner/repo/tree/ref/path/to/skill` | GitHub tree URL. Uses the given `ref` and path as-is. |
| `owner/repo#ref` | Treats the source as a repo root / parent directory and discovers `SKILL.md` files underneath it. |
| `skills.sh/owner/repo[/skill-or-path]#ref` | `skills.sh` source. Internally transformed to a GitHub repo source. |
| `https://www.skills.sh/owner/repo[/skill-or-path]#ref` | `skills.sh` URL. If the guessed direct path is wrong, sksync falls back to repo-root discovery. |
| `./local-skill`, `../skills/foo`, `/abs/path` | Local directory. Relative paths resolve from the config file directory. |

`registry:<host>/<package>` and `--provider` are not supported. Source URL transformers are inferred from the source string.

#### Private repositories

Private Git repositories use your local `git` authentication. sksync does not manage tokens. If `git clone <repo>` works in the current environment, sksync can use the same source.

- GitHub shorthand (`owner/repo/path#ref`) is converted to `https://github.com/owner/repo.git`. For private repositories, configure HTTPS auth through a Git credential helper, GitHub CLI, PAT, or equivalent.
- Use structured sources for SSH URLs.

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

sksync has no `--token` or `--github-token` option and never stores credentials in config. Auth failures are reported as underlying `git` command errors. `skills.sh` URLs are generally expected to point to public sources.

#### Discovery behavior

When a source points to a repo root or parent directory without a direct `SKILL.md`, sksync searches for `SKILL.md` up to depth 5.

- One match: automatically selected.
- Multiple matches in an interactive terminal: prompt for one or more selections.
- Multiple matches in a non-interactive environment: error with guidance to pass `--name <skill>` or a more specific source.
- With `--name`: automatically select exactly one discovered skill whose frontmatter `name` or directory name matches.
- `.git`, `node_modules`, and `.sksync` are excluded from discovery.

The multi-select prompt displays skill names in bold cyan.

#### skills.sh mapping

`sksync` treats `skills.sh` as a URL transformer to GitHub sources, not as a registry. You can pass `skills.sh` URLs or shorthands, but config stores the selected skill as an exact GitHub tree URL: `https://github.com/<owner>/<repo>/tree/<ref>/<path>`.

```text
https://www.skills.sh/vercel-labs/skills/find-skills
→ https://github.com/vercel-labs/skills.git
→ skills/find-skills
→ source saved as https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills
```

If the `skills.sh` URL slug does not match the actual path inside the GitHub repo, sksync uses repo-root discovery to find the real path and saves the exact GitHub tree URL.

```text
https://www.skills.sh/gitbutlerapp/gitbutler/but
→ discovers crates/but/skill
→ source saved as https://github.com/gitbutlerapp/gitbutler/tree/HEAD/crates/but/skill
```

#### Skill validation

Fetched skills are validated before install:

- `SKILL.md` exists.
- `SKILL.md` is a file.
- YAML frontmatter exists.
- Frontmatter contains non-empty string `name` and `description` fields.

If validation fails, sksync does not replace the destination and deletes the staging directory.

Pass `--global` to add the dependency to `~/.sksync/config.json` as a global dependency.

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --global
```

### `sksync bundle`

Bundles are curated install sets described by `sksync.bundle.json`. A bundle is not an installed runtime folder: `bundle add` expands entries into normal dependencies, using agents you choose at add time, and records local provenance so `bundle remove` can later detach or remove those dependencies safely.

```bash
cargo run -- bundle inspect <source>
cargo run -- bundle add <source> --agent pi [--agent claude-code] [--dry-run]
cargo run -- bundle remove <name> [--source <exact-source>] [--dry-run]
cargo run -- bundle sync <name> [--source <exact-source>] [--dry-run]
```

Example manifest:

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

Bundle entry keys are final skill names. Entry sources may be local, GitHub, `skills.sh`, or manifest-relative paths. Bundle manifests never choose target agents.

Typical team flow:

```bash
# See manifest metadata and normalized entry sources.
cargo run -- bundle inspect ./bundles/review-workflow

# Preview create / merge / conflict / skipped statuses.
cargo run -- bundle add ./bundles/review-workflow --agent pi --agent claude-code --dry-run

# Install every entry into the selected agents.
cargo run -- bundle add ./bundles/review-workflow --agent pi --agent claude-code

# Later, preview bundle manifest membership drift.
cargo run -- bundle sync review-workflow --dry-run

# Later, remove local provenance and bundle-managed dependencies.
cargo run -- bundle remove review-workflow --dry-run
cargo run -- bundle remove review-workflow
```

`bundle add` is all-or-nothing at the config/lockfile level. Existing dependencies with the same normalized source are adopted and keep `managedByBundles: false`, so `bundle remove` later detaches provenance without deleting manually managed dependencies. Existing dependencies with the same skill name but a different source are reported as conflicts and nothing is written.

`bundle sync --dry-run` reloads the latest manifest for an already-added bundle and previews membership drift such as new entries, removed entries, source changes, and missing dependency agents. The current implementation is preview-only; applying sync changes is planned separately. Existing skill content updates remain the responsibility of `sksync update`.

Authoring tips:

- keep bundle names and entry keys stable,
- prefer explicit refs for remote entries; use tags or commits when strict reproducibility matters,
- use manifest-relative sources when bundle skills live beside the manifest,
- keep bundles focused, such as `review-workflow`, `rust-baseline`, or `team-onboarding`.

Export workflow:

```bash
# Preview writing only sksync.bundle.json from current dependencies.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline --dry-run

# Create only sksync.bundle.json from current dependencies.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline

# Copy installed skill bodies into the bundle directory.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline --snapshot

# Export only selected dependencies.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline --skill review --skill qa
```

Default `bundle export` is lightweight: it preserves dependency source references and writes only `sksync.bundle.json`. `--snapshot` creates a self-contained bundle directory from installed skill bodies and rewrites entries to `./skills/<name>` sources. Export never writes agents, local bundle provenance, or `managedByBundles` into the bundle manifest.

### `sksync attach`

Attach an existing dependency-managed skill to additional agents. sksync preserves the existing source representation, installs the skill, and creates symlinks.

```bash
cargo run -- attach cuekit-dogfood --agent claude-code
cargo run -- attach cuekit-dogfood --agent pi --agent gemini --global
```

### `sksync agents`

Inspect and update agent target mappings. `doctor` is read-only and checks whether target directories exist and are writable.

```bash
cargo run -- agents list
cargo run -- agents doctor
cargo run -- agents refresh
```

### `sksync doctor`

Run a read-only diagnosis across config, lockfile, sources, targets, and agent mappings. If problems are found, sksync prints suggested next commands and exits non-zero. It does not auto-fix problems or create directories.

```bash
cargo run -- doctor
cargo run -- doctor --global
```

### `sksync import`

Copy existing agent skill directories into `.sksync/skills` or `~/.sksync/skills` and register them as dependencies for the specified agents. `--agent` can be passed multiple times. The original directory is never changed, deleted, or replaced with symlinks. Review target symlink changes separately with `plan` / `apply`.

```bash
cargo run -- import ~/.claude/skills --agent claude-code --dry-run
cargo run -- import ~/.claude/skills --agent claude-code
cargo run -- import ~/.agents/skills --agent universal --agent pi
cargo run -- import ~/.jcode/skills --agent jcode --global
```

### `sksync remove`

Remove one or more skills from dependency config, installed skill directories, managed symlinks, and lockfile entries. Installed skill directories are deleted only when they are sksync-managed directories under the configured `skillDir`; unmanaged local or legacy source directories are left untouched.

```bash
cargo run -- remove cuekit-dogfood
cargo run -- remove cuekit-dogfood qa-skill review-helper
cargo run -- remove cuekit-dogfood --global
cargo run -- remove cuekit-dogfood --keep-files
cargo run -- remove cuekit-dogfood --config-only
```

Pass `--agent` to detach only the selected agent links.

```bash
cargo run -- remove cuekit-dogfood --agent pi
cargo run -- remove cuekit-dogfood --agent pi --agent claude-code
```

In agent-specific removal, sksync removes the selected agents from `dependencies.<skill>.agents` and lockfile targets while keeping the skill body and other agent links. If the last agent is removed, sksync falls back to full skill removal.

### `sksync outdated`

Compare the lockfile with upstream and show skills that can be updated. Git sources compare the remote ref HEAD with the lockfile's resolved commit.

```bash
cargo run -- outdated
cargo run -- outdated --global
cargo run -- outdated --json
```

### `sksync plan --dry-run`

Read `sksync.config.json`, inspect current target state, and show planned creates, already-synced links, conflicts, drift, and other states.

```bash
cargo run -- plan --dry-run
cargo run -- plan --global
```

### `sksync install`

If `sksync-lock.json` exists, reconstruct skills from lockfile sources first, then create symlinks. Without a lockfile, install fetches from config and creates one.

```bash
cargo run -- install
cargo run -- install --global
```

### `sksync update`

Download or copy the latest or pinned skills from `dependencies` sources into `skillDir`, then update `sksync-lock.json`. Fetched skills are validated with `SKILL.md` and YAML frontmatter `name` / `description` checks.

```bash
cargo run -- update
cargo run -- update --global
```

Supported sources are the same as `sksync add`. Repo-root / parent-directory discovery happens during `add` and is saved as a selected path, so `update` / `install` re-fetch the saved source.

### `sksync apply`

Run only the planner's create-symlink actions, then write `sksync-lock.json`. `apply` fails on missing sources, conflicts, or drift. `--force` only allows replacement when the existing target is a sksync-managed link that is safe to update.

```bash
cargo run -- apply
cargo run -- apply --force
cargo run -- apply --global
```

### `sksync check`

Compare `sksync-lock.json` with the current state and detect source hash drift, missing targets, broken symlinks, and related problems. Source hashes come from the lockfile; target health is recalculated from current config and agent mappings. Problems cause a non-zero exit.

```bash
cargo run -- check
cargo run -- check --global
```

### `sksync list`

List configured skills and each agent's target path / state. If `sksync-lock.json` exists, locked hashes are shown too.

```bash
cargo run -- list
cargo run -- list --global
```

### `sksync wizard`

Launch an interactive prompt wizard for add / attach / detach / remove / default agents configuration / list+check / plan+apply flows. `ask` and `tui` are compatible aliases.

```bash
cargo run -- wizard
cargo run -- ask
cargo run -- tui
```

### Safety rules

- Never overwrite existing regular files.
- `add` rolls back dependency config on failure.
- `remove` deletes only symlinks managed by sksync.
- `remove` deletes installed files only when they are under the configured `skillDir`.
- Git source subpaths reject absolute paths and `..`, and must stay inside the clone directory.
- Project-scope agent `targetDir` paths cannot escape the project root.
- `outdated` compares Git remote refs with lockfile commits.
- `install` prefers lockfile resolved sources when a lockfile exists.
- `update` fetches from dependencies and refreshes the lockfile.
- `apply` runs create-symlink actions only.
- Project config resolves targets as project scope; `--global` config resolves targets as user scope.
- `apply` fails on conflict, drift, or missing source states.
- Parent directories for target paths are created as needed.
- Use temporary directories for tests and examples when possible.

### Generated files and gitignore

Project-local generated files are git-ignored:

- `.sksync/` - downloaded/copied skill bodies (`.sksync/skills/<skill>`)
- `skills/` - legacy generated skill store from older defaults
- `sksync-lock.json` - portable lockfile v4. It is project-local generated state, but if shared, it can reproduce skills across macOS / Linux with `sksync install`.

The primary file to share is `sksync.config.json`.

### Config / lockfile examples

- [`sksync.config.example.json`](sksync.config.example.json) - project/global install dependencies
- [`sksync.agents.example.json`](sksync.agents.example.json) - global and project agent target mappings (`~/.sksync/agents.json`) with bundled Agent Skills entries
- [`sksync-lock.example.json`](sksync-lock.example.json) - current portable lockfile v4 example
- [`schemas/sksync.schema.json`](schemas/sksync.schema.json) - JSON Schema for `config.json` / `sksync.config.json`
- [`schemas/sksync.agents.schema.json`](schemas/sksync.agents.schema.json) - JSON Schema for `agents.json`
- [`schemas/sksync-lock.schema.json`](schemas/sksync-lock.schema.json) - JSON Schema for current portable `sksync-lock.json` v4

## Linux / Docker compatibility

Linux release assets are available for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. CI runs the x86_64 musl binary inside Debian / Ubuntu Docker images and smoke-tests a local source through `init` / `add` / `plan` / `apply` / `check` / `list` / `remove`.

Current smoke coverage:

- `debian:bookworm`
- `debian:trixie`
- `ubuntu:22.04`
- `ubuntu:24.04`

Windows is out of scope for now. sksync prioritizes stable symlink behavior on macOS and Linux.

## Roadmap

The command model follows npm-like dependency management while source integrations and portability mature.

- Additional source URL transformers beyond `skills.sh`
- GitLab / gist support
- Finalize the lockfile sharing policy
- Improve cross-platform symlink / junction behavior

There is intentionally no dedicated `ci` command. Reproducible reconstruction is consolidated into `sksync install`.

For more detail, see:

- [`docs/DESIGN.md`](docs/DESIGN.md) - feature design
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - architecture principles
- [`docs/ROADMAP.md`](docs/ROADMAP.md) - development roadmap
