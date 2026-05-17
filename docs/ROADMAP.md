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

## Phase 3: Portability and install workflow

- Add install/update/remove workflows
- Add lockfile migration support
- Add custom agent mappings
- Add cross-platform symlink/junction behavior

## Phase 4: Registry or source integrations

- Explore GitHub/local path based skill installation
- Consider registry-like index only if needed
- Preserve lockfile reproducibility across machines
