# sksync Roadmap

`sksync` focuses on being a **safe, reproducible, lightweight Agent Skills deployment and sync tool**, not a general-purpose skill marketplace.

## Product focus

Prioritized value:

- Reproducible skill placement through config and lockfile.
- Safe synchronization between source skill bodies and agent target symlinks.
- Clear separation between project and global scopes.
- Agent target mapping inspection and refresh.
- Conservative migration from manually managed skills.

Explicitly out of scope for now:

- Marketplace or large registry operation.
- Recommendations or stack-aware skill suggestions.
- Format translation between agents.
- REST or MCP server mode.
- Mesh or messaging features.
- Automatic repair from `doctor`.

## Completed baseline

- Rust single-binary CLI.
- Project and global config.
- Bundled agent mappings, including jcode and universal Agent Skills directories.
- GitHub, `skills.sh`, and local source support.
- `skills.sh` input normalization to exact GitHub tree URLs.
- Dependency install/update/apply/check/list/outdated flows.
- Portable lockfile v4 for macOS / Linux reproduction.
- Lockfile-backed source and symlink checks.
- Add / attach / remove / detach workflows.
- Prompt wizard as a thin CLI wrapper.
- Read-only `doctor`.
- `agents list`, `agents doctor`, and `agents refresh`.
- Copy-only `import`.
- Wizard-configurable `defaultAgents` for Add skill preselection.
- macOS / Linux release assets, with Linux musl binaries for distro portability.
- Docker smoke coverage for Debian / Ubuntu containers.

## v0.1: Read-only diagnosis and agent mapping UX

Status: implemented.

Goal: improve visibility and agent mapping management without expanding the product scope unnecessarily.

### `sksync doctor`

Read-only diagnostic command. It reports problems and suggests next commands, but never mutates files.

Checks:

- config / agents.json / lockfile parse and consistency
- dependency source existence and source hash drift
- lockfile drift and stale entries
- target conflict / broken symlink / drifted symlink
- agent target mapping existence
- targetDir existence and writability

Out of scope:

- `doctor --fix`
- interactive repair
- automatic directory creation
- automatic symlink repair

### `sksync agents`

Small command group for mapping visibility and refresh.

- `sksync agents list`: show bundled/user mappings by scope.
- `sksync agents doctor`: read-only targetDir checks.
- `sksync agents refresh`: refresh `~/.sksync/agents.json` from bundled mapping.

`init --agents` can remain for compatibility, but documentation should point users toward `agents refresh` for the same operation.

## v0.2: Conservative import

Status: implemented.

Goal: give users a safe migration path from manually managed agent skill directories.

### `sksync import <path> --agent <agent>`

Import is copy-only.

- Scan an existing skill directory such as `.claude/skills` or `~/.jcode/skills`.
- Copy selected skills into `.sksync/skills` or `~/.sksync/skills`.
- Update config for the specified target agent.
- Leave the original directory untouched.
- Do not replace original files with symlinks during import.
- Require `plan` / `apply` as a separate confirmation step for target changes.

Required safety behavior:

- First-class `--dry-run` support.
- Name collision reporting.
- Clear skip/fail behavior for invalid skill directories.
- No deletion of unmanaged files.

## Current stabilization notes

- `defaultAgents` is intentionally a wizard preselection aid. CLI `sksync add <source>` still requires explicit `--agent` arguments.
- `sksync-lock.json` v4 is the current portable format; v2/v3 remain read-compatible but new writes use v4.
- `skills.sh` remains input-only; persisted config should use exact GitHub tree URLs after add-time discovery.
- Linux installer defaults to musl release assets (`x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl`) so Debian / Ubuntu users do not depend on the build runner's glibc version.
- Docker smoke tests cover `debian:bookworm`, `debian:trixie`, `ubuntu:22.04`, and `ubuntu:24.04`; Windows remains out of scope for now.

## Later only if needed

These are intentionally not part of the near-term roadmap:

- source search
- curated registries
- skill recommendation
- agent format translation
- skill bundle management
- skill test runner
- policy engine
- server / MCP integrations

They should be revisited only if the lightweight deployment model is stable and the added complexity has a clear user need.
