# sksync Implementation Plan

## Current state analysis

This document records the original implementation plan from the design-baseline stage. At that point the repository did not yet contain the production implementation. The planned baseline files were:

- `README.md`: purpose, initial scope, and expected commands.
- `docs/DESIGN.md`: feature design, config/lockfile formats, CLI/TUI shape, and safety rules.
- `docs/ARCHITECTURE.md`: Clean Architecture, Domain Model First, error classification, and TUI principles.
- `docs/ROADMAP.md`: phased roadmap.
- `docs/RUST_TUI_PLAN.md`: Rust/TUI implementation order and recommended crates.
- `sksync.config.example.json`: example config.
- `sksync-lock.example.json`: example lockfile.

The first implementation target was the Rust CLI MVP. TUI and install/source URL integrations were intentionally deferred until the core was stable.

## Implementation direction

### 1. Build the CLI MVP first

The TUI is useful, but the core of `sksync` is to derive the desired link state from config, safely compare it with current filesystem state, and apply the resulting plan. The initial CLI surface was:

```bash
sksync init
sksync plan --dry-run
sksync apply
sksync check
sksync list
```

`install` and `tui` were deferred to later phases.

### 2. Separate core logic from UI

`cli` should only call the application layer. Filesystem, symlink, and JSON I/O details should be contained in the infrastructure layer.

Recommended initial structure:

```text
src/
  main.rs
  cli.rs
  application/
    mod.rs
    init.rs
    plan.rs
    apply.rs
    check.rs
    list.rs
    ports.rs
  domain/
    mod.rs
    agent.rs
    skill.rs
    scope.rs
    target.rs
    link_plan.rs
    lockfile.rs
    problem.rs
  infrastructure/
    mod.rs
    builtin_agents.rs
    fs.rs
    hash.rs
    json.rs
```

### 3. Follow Parse, Don't Validate

Do not pass raw JSON directly into the core. Parse it at the boundary into valid domain/application types:

- `RawConfig` → `ResolvedConfig`
- raw agent name → `AgentKind`
- raw scope → `Scope`
- raw path → `SourcePath` / `TargetPath`
- raw lockfile → version-aware `Lockfile`

### 4. Treat safety as an MVP requirement

`apply` is a potentially destructive operation, not a convenience feature. The MVP must include these protections:

- Do not overwrite regular files.
- Do not silently replace unexpected existing symlinks.
- Do not perform destructive changes without `--force`.
- Let users inspect planned operations through `plan`.
- Generate/update the lockfile.

## Phase plan

## Phase 1: Rust CLI MVP

Goal: implement the basic designed sync flow with tests.

1. Create the Cargo project, CI, lint, and formatting baseline.
2. Define config, lockfile, and domain primitives.
3. Implement built-in agent mappings.
4. Implement skill discovery and hash calculation.
5. Implement the dry-run planner.
6. Implement symlink apply and lockfile generation.
7. Implement check/list CLI commands.
8. Add execution instructions to the README.

Completion criteria:

- `cargo test` passes.
- `plan`, `check`, and `list` work with the example config.
- Safety rules for `apply` are tested in temporary directories.

## Phase 2: Prompt Wizard MVP

Goal: use the same core logic as the CLI to add, remove, inspect, and apply skills through a prompt flow.

1. Implement prompt / wizard intent selection.
2. Ask for the values needed by add / remove / remove-agent / check / apply.
3. For remove / remove-agent, choose from skill and agent lists loaded from config after scope selection.
4. Show a summary / dry-run before destructive operations.
5. After explicit confirmation, call the same application use case as the CLI.

Completion criteria:

- The wizard does not directly touch the filesystem.
- There is no persistent pane/keybinding-style UI.
- Apply / remove require confirmation.
- CLI and wizard produce consistent plan results.

## Phase 3: Portability / install workflow

Goal: broaden real-world usage.

1. Custom agent mapping.
2. Config/lockfile migration.
3. Windows symlink/junction strategy.
4. `install/update/remove` workflow.

## Priorities

1. **Highest priority**: config parsing, domain model, planner, safe apply.
2. **Next**: check/list and lockfile drift detection.
3. **Later**: TUI, install, source URL integrations, and Windows-specific behavior.

## Implementation notes

- `domain` must not depend on `clap`, `serde_json`, `std::fs`, or prompt UI crates.
- Do not treat `SourcePath` and `TargetPath` as interchangeable `PathBuf`s.
- Do not over-deduplicate `ConfigSkill` and `LockedSkill`; they have different roles.
- Use `tempfile` in tests and never touch real user directories.
- Add `insta` snapshot tests after plan/check output stabilizes.
