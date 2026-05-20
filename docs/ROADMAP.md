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

## Phase 2: Prompt-style TUI MVP

- Add `sksync tui` as a SkillKit-like question flow
- Ask for add/remove/remove-agent intents and required source / skill / agent values
- Show dry-run summary before mutation
- Run add/remove/check/apply usecases from TUI
- Keep dashboard-style screens out of the MVP unless a separate mode is needed

## Phase 3: npm-like dependency workflow

- Stabilize `add` / `install` / `update` semantics
- Implement `remove <skill>` for dependency, installed skill, lockfile entry, and managed symlink removal
- Add `remove <skill> --agent <agent>` for agent-scoped symlink / target removal
- Implement `outdated` for Git drift reporting and registry provider placeholder reporting
- Do not add a `ci` command for now; lockfile-first `install` covers the current reproducibility need

## Phase 4: Portability and install workflow

- Add lockfile migration support
- Split portable lockfile data from machine-local target path state
- Add custom agent mappings
- Add cross-platform symlink/junction behavior
- Preserve lockfile reproducibility across machines

## Phase 5: Registry or source integrations

- Implement `registry:skills.sh/<package>` provider
- Support additional `registry:<host>/<package>#version` providers
- Explore GitLab / gist sources after GitHub/local/registry are stable
