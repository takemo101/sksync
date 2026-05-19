# sksync Roadmap

## Phase 0: Design baseline

- Define the problem space for cross-agent skill synchronization
- Define config and lockfile examples
- Define Rust CLI/TUI architecture
- Define safety rules for symlink operations
- Define built-in target agents: `claude-code`, `codex`, `gemini`, `opencode`, `pi`

## Phase 1: Rust CLI MVP

- Initialize Cargo project
- Implement config and lockfile models
- Implement built-in agent mapping
- Implement dry-run planner
- Implement `check`
- Implement safe symlink `apply`

## Phase 2: TUI MVP

- Add `sksync tui`
- Show agents, skills, and plan results
- Run dry-run/check/apply from TUI
- Add confirmation modal before apply

## Phase 3: npm-like dependency workflow

- Stabilize `add` / `install` / `update` semantics
- Add `remove <skill>` for dependency, installed skill, lockfile entry, and managed symlink removal
- Add `outdated` for Git / registry version drift reporting
- Do not add a `ci` command for now; lockfile-first `install` covers the current reproducibility need

## Phase 4: Portability and install workflow

- Add lockfile migration support
- Add custom agent mappings
- Add cross-platform symlink/junction behavior
- Preserve lockfile reproducibility across machines

## Phase 5: Registry or source integrations

- Implement `registry:skills.sh/<package>` provider
- Support additional `registry:<host>/<package>#version` providers
- Explore GitLab / gist sources after GitHub/local/registry are stable
