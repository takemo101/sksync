# sksync Architecture

This document summarizes the architecture principles used for `sksync`.

Primary influences:

- Clean Architecture
- Package Design / Package Refactoring
- Domain Model First
- Parse, Don't Validate
- Domain Primitives / Always-Valid Domain Model
- Tell, Don't Ask
- Law of Demeter
- Intent-based Deduplication
- Error Handling / Error Classification
- Backward Compatibility Governance

## 1. Architecture direction

`sksync` is a Rust CLI/TUI application, but its center is the domain logic for skill synchronization. CLI, TUI, and filesystem operations are entry/exit adapters. Decisions about what should be linked where belong in the core layers.

```text
┌─────────────────────────────────────────────┐
│ Interface Layer                              │
│  - CLI: clap                                 │
│  - TUI: prompt/wizard adapter                │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Application Layer                            │
│  - init                                      │
│  - add / attach / remove                     │
│  - install / update / outdated               │
│  - plan / dry-run / apply                    │
│  - check / list / doctor                     │
│  - agents / import use cases                 │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Domain Layer                                 │
│  - Skill                                     │
│  - Agent                                     │
│  - Scope                                     │
│  - TargetPath                                │
│  - LinkPlan                                  │
│  - Lockfile model                            │
│  - Drift / Conflict / BrokenLink             │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Infrastructure Layer                         │
│  - filesystem                                │
│  - symlink                                   │
│  - hash                                      │
│  - config/lockfile JSON I/O                  │
└─────────────────────────────────────────────┘
```

## 2. Dependency direction

Follow Clean Architecture dependency rules:

- `domain` does not depend on other layers.
- `application` depends on `domain`.
- `cli` / `tui` only call `application`.
- `infrastructure` implements ports/traits defined by `application`.
- TUI does not directly create symlinks or write lockfiles.
- If the TUI updates config preferences such as `defaultAgents`, that update must stay a thin adapter that preserves existing config JSON fields.

Dependency direction:

```text
cli/tui ──▶ application ──▶ domain
              ▲
              │ trait
              │
infrastructure ┘
```

## 3. Rust module structure

```text
src/
  main.rs
  cli.rs
  tui/
    mod.rs
    app.rs
    ui.rs
    events.rs
  application/
    mod.rs
    init.rs
    add.rs
    install.rs
    update.rs
    remove.rs
    outdated.rs
    plan.rs
    apply.rs
    check.rs
    list.rs
    lockfile_build.rs
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
    fs.rs
    json/
      config.rs
      dependency_config.rs
      lockfile.rs
      agents.rs
    hash.rs
    builtin_agents.rs
```

The current implementation may use fewer files where responsibilities are still small. Split modules by responsibility, not by technical fashion.

## 4. Package design principles

### Avoid purely technical buckets

Avoid names such as `models`, `utils`, `helpers`, or `services` when they hide responsibilities. Prefer names that describe the reason the module changes.

Good boundaries:

- `domain::skill`
- `domain::agent`
- `application::plan`
- `application::apply`
- `infrastructure::builtin_agents`

Avoid:

- `utils`
- `types`
- `services`
- `managers`

### Keep public interfaces small

Each module should expose only the types/functions it wants other modules to rely on. Prefer private or `pub(crate)` implementation details.

## 5. Domain Model First

Implementation should proceed from the domain outward:

1. Define domain types.
2. Test domain invariants.
3. Build the dry-run `LinkPlan`.
4. Build application use cases.
5. Connect CLI/TUI.
6. Connect filesystem infrastructure.

Do not start from TUI or filesystem operations. The core question is: what should be linked where, and how does current state differ from desired state?

## 6. Parse, Don't Validate

External input should be parsed once at the boundary. Avoid repeatedly checking raw strings throughout the core.

Examples:

- raw agent name → `AgentKind`
- raw scope → `Scope`
- raw path → `SourcePath` / `TargetPath`
- JSON config → `RawConfig` → `ResolvedConfig`

```text
JSON / CLI args / filesystem
  ↓ parse
valid domain/application types
  ↓ use
planner / apply / check
```

## 7. Domain primitives / always-valid domain model

Use Rust newtypes for important concepts instead of passing primitive `String` / `PathBuf` values everywhere.

Candidates:

```rust
struct SkillName(String);
struct AgentName(String);
enum AgentKind { Pi, ClaudeCode, Codex, Gemini, OpenCode, Custom(String) }
enum Scope { User, Project }
struct SourcePath(PathBuf);
struct TargetPath(PathBuf);
struct Sha256Digest(String);
```

Example invariants:

- `SkillName` is not empty.
- `SkillName` cannot contain path separators.
- `TargetPath` has been resolved from agent mapping.
- `ResolvedConfig` contains no references to unknown agents.
- `LinkPlan` does not include operations where source and target are the same path.

## 8. Tell, Don't Ask

Do not pull state out of objects and make decisions externally when the object can expose a behavior.

Avoid:

```rust
if entry.is_symlink && entry.target == expected {
    // ok
}
```

Prefer:

```rust
match current_link.compare_with(expected_link) {
    LinkStatus::Synced => ...,
    LinkStatus::Drifted(problem) => ...,
}
```

## 9. Law of Demeter

Avoid deep structural navigation, especially from CLI/TUI into domain internals.

Avoid:

```rust
app.config.skills[skill].agents[agent].target.path
```

Prefer:

```rust
let rows = app.view_model().skill_rows();
```

TUI should receive view models and should not need to know internal domain structure.

## 10. Intent-based deduplication

Do not merge code only because it looks similar. Merge it when it represents the same intent.

Examples:

- `SourcePath` and `TargetPath` both wrap `PathBuf`, but their intent differs, so they should be separate types.
- `ConfigSkill` and `LockedSkill` may look similar, but their roles differ.
- CLI display rows and TUI display rows may change for different reasons; keep them separate unless their intent is truly shared.

## 11. Error handling / classification

Classify errors by how users and the program can respond.

| Kind | Examples | Response |
| --- | --- | --- |
| UserFixable | invalid config, missing source, target conflict | clear message and suggested command |
| Environment | symlink permission denied, missing home directory | OS-specific guidance |
| Drift | lockfile hash mismatch, broken symlink | report through `check` / TUI with repair candidates |
| Bug | unreachable state, violated invariant after parsing | internal error |

Rust implementation guidance:

- library/domain errors: `thiserror`
- CLI/TUI entrypoints: `anyhow`
- user display: short messages with context

## 12. Backward compatibility governance

`sksync.config.json` and `sksync-lock.json` are public APIs. Lockfile v4 stores only portable source/hash/resolved install-source data and recomputes machine-local target paths from current config. v2/v3 remain read-compatible; new writes use v4.

Rules:

- Lockfiles must include `lockfileVersion`.
- Consider introducing an explicit config schema version if needed.
- Old lockfiles must remain migratable/readable.
- v2 `targets` are read-compatible only and are not written by new lockfiles.
- Compatibility logic belongs in `infrastructure::json` or a dedicated migration module.
- Do not leak legacy format concerns into the domain model.

## 13. Repository / port placement

Persistence and filesystem traits belong in the application layer. Domain must not know about persistence.

Example:

```rust
trait ConfigStore {
    fn load_config(&self) -> Result<RawConfig>;
    fn save_config(&self, config: &RawConfig) -> Result<()>;
}

trait LinkStore {
    fn inspect_target(&self, target: &TargetPath) -> Result<TargetState>;
    fn create_symlink(&self, source: &SourcePath, target: &TargetPath) -> Result<()>;
}
```

## 14. TUI design principles

The TUI is a thin adapter around application use cases. It is a prompt-style wizard, not a persistent application UI.

- TUI calls `AddUseCase`, `AttachUseCase`, `RemoveUseCase`, `PlanUseCase`, `ApplyUseCase`, and `CheckUseCase`.
- TUI does not directly perform symlink, source install, or lockfile filesystem operations.
- Wizard preference config updates preserve existing JSON fields and stay adapter-level.
- TUI state contains only in-progress prompt answers, current selections, and confirmation state.
- Add / remove / agent changes / default agents configuration collect required values through prompt flows.
- Destructive operations show a dry-run summary and require explicit confirmation.
- No persistent list screen; status should be shown as `list` / `check` summaries.

## 15. Design review checklist

When reviewing implementation, check:

- [ ] `domain` does not depend on CLI/TUI/filesystem crates.
- [ ] Config is parsed at the boundary and core uses valid types.
- [ ] Important domain concepts are not left as raw `String` / `PathBuf`.
- [ ] No miscellaneous `utils` module has become a dumping ground.
- [ ] TUI does not directly perform symlink or lockfile operations.
- [ ] Config/lockfile compatibility rules are preserved.
- [ ] Existing files are not overwritten without `--force`.
- [ ] Errors are classified as user-fixable, environment, drift, or internal bug where appropriate.
