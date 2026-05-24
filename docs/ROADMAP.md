# sksync Roadmap

`sksync` は総合的な skill marketplace ではなく、**安全・再現可能・軽量な Agent Skills deployment / sync tool** に集中する。

## Product focus

優先する価値:

- config / lockfile による再現可能な skill 配置
- source body と agent target symlink の安全な同期
- project / global scope の明確な分離
- agent target mapping の確認・更新
- 既存手管理 skill からの保守的な移行

当面やらないこと:

- marketplace / large registry
- recommendation / stack-aware suggestion
- agent 間 format translation
- REST / MCP server
- mesh / messaging
- `doctor` による自動修復

## Completed baseline

- Rust single-binary CLI
- project / global config
- bundled agent mappings, including jcode and universal Agent Skills directories
- GitHub / skills.sh / local source support
- `skills.sh` input normalization to exact GitHub tree URLs
- dependency install/update/apply/check/list/outdated flows
- portable lockfile v4 for macOS / Linux reproduction
- lockfile-backed source and symlink checks
- add / attach / remove / detach workflows
- prompt wizard as a thin CLI wrapper
- read-only `doctor`
- `agents list` / `agents doctor` / `agents refresh`
- copy-only `import`
- wizard-configurable `defaultAgents` for Add skill preselection

## v0.1: Read-only diagnosis and agent mapping UX

Status: implemented.

Goal: 既存機能を増やしすぎず、状態把握と agent mapping 管理を分かりやすくする。

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

- `sksync agents list`: show bundled/user mappings by scope
- `sksync agents doctor`: read-only targetDir checks
- `sksync agents refresh`: refresh `~/.sksync/agents.json` from bundled mapping

`init --agents` can remain for compatibility, but should point users toward `agents refresh` once the command exists.

## v0.2: Conservative import

Status: implemented.

Goal: give users a safe migration path from manually managed agent skill directories.

### `sksync import <path> --agent <agent>`

Import is copy-only.

- scan an existing skill directory such as `.claude/skills` or `~/.jcode/skills`
- copy selected skills into `.sksync/skills` or `~/.sksync/skills`
- update config for the specified target agent
- leave the original directory untouched
- do not replace original files with symlinks during import
- require `plan` / `apply` as a separate confirmation step for target changes

Required safety behavior:

- `--dry-run` first-class support
- name collision reporting
- skip or fail clearly on invalid skill directories
- no deletion of unmanaged files

## Current stabilization notes

- `defaultAgents` is intentionally a wizard preselection aid. CLI `sksync add <source>` still requires explicit `--agent` arguments.
- `sksync-lock.json` v4 is the current portable format; v2/v3 remain read-compatible but new writes use v4.
- `skills.sh` remains input-only; persisted config should use exact GitHub tree URLs after add-time discovery.

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
