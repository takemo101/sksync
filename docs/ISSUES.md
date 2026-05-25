# sksync Implementation Issues

This file breaks the original implementation plan into small tasks that coding agents can pick up independently. Each issue should be small enough to complete in one focused session, have clear acceptance criteria, and touch a limited set of files.

## Milestone 1: Rust CLI MVP foundation

### Issue 1: Create the Cargo project and base crate structure

**Goal**
Create the smallest Rust CLI project that builds and tests.

**Work**

- Create `Cargo.toml`.
- Create `src/main.rs`.
- Create the initial module tree:
  - `src/cli.rs`
  - `src/application/mod.rs`
  - `src/domain/mod.rs`
  - `src/infrastructure/mod.rs`
- Add dependencies:
  - runtime: `clap`, `serde`, `serde_json`, `anyhow`, `thiserror`, `dirs`, `shellexpand`, `sha2`, `hex`, `walkdir`
  - dev: `tempfile`
- Ensure `cargo test` passes.

**Acceptance criteria**

- `cargo build` succeeds.
- `cargo test` succeeds.
- `sksync --help` can be displayed.

**Depends on**: none

---

### Issue 2: Implement CLI command definitions

**Goal**
Define the expected subcommands with `clap`, even if some handlers are placeholders at first.

**Work**

- Define the command enum in `src/cli.rs`.
- Add subcommands:
  - `init`
  - `plan` / `--dry-run`
  - `apply` / `--force`
  - `check`
  - `list`
  - placeholder `tui`
- Dispatch commands from `main.rs`.
- Placeholder commands must return clear errors instead of panicking.

**Acceptance criteria**

- `sksync init --help` works.
- `sksync plan --help` works.
- Unimplemented placeholders do not panic.

**Depends on**: Issue 1

---

## Milestone 2: Domain model and config parsing

### Issue 3: Define domain primitives

**Goal**
Avoid passing important concepts as raw `String` or `PathBuf` values.

**Work**

- Add `src/domain/skill.rs`:
  - `SkillName`
  - `SourcePath`
- Add `src/domain/agent.rs`:
  - `AgentKind`
  - custom agent representation / `AgentName`
- Add `src/domain/scope.rs`:
  - `Scope::{User, Project}`
- Add `src/domain/target.rs`:
  - `TargetPath`
- Validate invariants in constructors:
  - skill names are not empty
  - skill names cannot contain path separators
  - scope is only `user` or `project`

**Acceptance criteria**

- Unit tests cover valid and invalid cases.
- Invalid skill names cannot become domain types.
- `domain` does not depend on CLI/TUI/filesystem layers.

**Depends on**: Issue 1

---

### Issue 4: Implement config JSON model and loader

**Goal**
Load `sksync.config.json` and convert it into a configuration the domain/application layers can use.

**Work**

- Define raw config models in `src/infrastructure/json.rs`.
- Define `ConfigStore` in `src/application/ports.rs`.
- Implement file-backed `ConfigStore`.
- Implement `RawConfig` → `ResolvedConfig` conversion.
- If `skills.*.source` is omitted, default it to `skillDir/<skillName>`.
- Use the example config as a fixture in tests.

**Acceptance criteria**

- `sksync.config.example.json` parses.
- Unknown agent references fail clearly.
- Source defaulting is tested.

**Depends on**: Issue 3

---

### Issue 5: Implement lockfile model and JSON reader/writer

**Goal**
Record sync results in a reproducible lockfile format.

**Work**

- Define lockfile domain types in `src/domain/lockfile.rs`.
- Add JSON reader/writer in `src/infrastructure/json.rs`.
- Include `lockfileVersion`, `generatedBy`, `generatedAt`, `root`, and `skills`.
- Return a clear error for unknown versions.

**Acceptance criteria**

- `sksync-lock.example.json` parses.
- A lockfile can be written and read back.
- Unsupported version behavior is tested.

**Depends on**: Issue 3

---

## Milestone 3: Agent mapping and skill discovery

### Issue 6: Implement built-in agent mapping

**Goal**
Resolve a target directory from agent + scope.

**Work**

- Implement `src/infrastructure/builtin_agents.rs`.
- Add default mappings for:
  - `pi`: user `~/.pi/agent/skills`, project `.pi/agent/skills`
  - `claude-code`: user `~/.claude/skills`, project `.claude/skills`
  - `codex`: user `~/.codex/skills`, project `.codex/skills`
  - `gemini`: user `~/.gemini/skills`, project `.gemini/skills`
  - `opencode`: user `~/.config/opencode/skills`, project `.opencode/skills`
- Design the API to allow `agents.*.targetDir` overrides.
- Expand `~` and resolve project-relative paths against the project root.

**Acceptance criteria**

- Tests cover each agent/scope target path.
- User-scope `~` expansion works.
- Project-scope paths resolve relative to project root.

**Depends on**: Issue 4

---

### Issue 7: Implement skill discovery and SHA-256 hashing

**Goal**
Record source directory contents in the lockfile.

**Work**

- Implement `src/infrastructure/hash.rs`.
- Collect files under the source directory in stable order.
- Calculate SHA-256 for each file.
- Calculate a directory-level hash.
- Exclude `.git`, target artifacts, and other ignored directories as needed.

**Acceptance criteria**

- Same content produces the same hash.
- File content changes alter the hash.
- File ordering does not affect the hash.

**Depends on**: Issue 4

---

## Milestone 4: Planner and apply

### Issue 8: Inspect current target state

**Goal**
Safely classify the current state of each target path.

**Work**

- Define `LinkStore` in `src/application/ports.rs`.
- Define `TargetState` variants:
  - missing
  - symlink to expected source
  - symlink to unexpected source
  - regular file/directory conflict
  - broken symlink
- Implement filesystem inspection in `src/infrastructure/fs.rs`.
- Read symlink metadata correctly.

**Acceptance criteria**

- Tempdir tests cover each target state.
- Regular files are not treated as symlinks.
- Broken symlinks are detected.

**Depends on**: Issue 6

---

### Issue 9: Implement dry-run planner

**Goal**
Build a safe operation plan from config and current state.

**Work**

- Define `src/domain/link_plan.rs`.
- Define plan actions:
  - create symlink
  - already synced
  - conflict
  - drifted symlink
  - source missing
- Implement `src/application/plan.rs`.
- Convert plan results into CLI-friendly output.

**Acceptance criteria**

- Missing target produces a create action.
- Synced target produces no-op / already-synced output.
- Existing regular file produces conflict.
- Unexpected symlink produces drift.
- `sksync plan --dry-run` displays the operation list.

**Depends on**: Issue 8

---

### Issue 10: Implement safe symlink apply

**Goal**
Apply only safe planner actions.

**Work**

- Implement `src/application/apply.rs`.
- Create parent directories as needed.
- Create symlinks for create actions.
- Refuse conflict/drift/source-missing actions.
- Support `--force` only for explicitly safe replacement cases.
- Write/update the lockfile after successful apply.

**Acceptance criteria**

- Apply creates expected symlinks in a tempdir.
- Apply refuses regular-file conflicts.
- Apply refuses unexpected symlink drift without force.
- Lockfile is written after successful apply.

**Depends on**: Issue 9

---

## Milestone 5: Check and list

### Issue 11: Implement `sksync check`

**Goal**
Detect drift between config, lockfile, source hashes, and target symlinks.

**Work**

- Load config and lockfile.
- Recalculate source hashes.
- Inspect target states.
- Report missing sources, hash drift, broken symlinks, conflicts, and unexpected symlinks.
- Exit non-zero when problems are found.

**Acceptance criteria**

- Clean state exits zero.
- Hash drift exits non-zero.
- Broken symlink exits non-zero.
- Output includes actionable problem descriptions.

**Depends on**: Issues 7, 9, 10

---

### Issue 12: Implement `sksync list`

**Goal**
Show configured skills and per-agent target status.

**Work**

- Load config and optional lockfile.
- Build rows for each skill/agent pair.
- Show source path, target path, status, and locked hash when available.
- Keep output concise and stable.

**Acceptance criteria**

- Configured skills are listed.
- Per-agent target status is displayed.
- Missing lockfile is handled gracefully.

**Depends on**: Issue 9

---

## Milestone 6: Install/update/source workflows

### Issue 13: Implement dependency install/update

**Goal**
Fetch/copy dependency sources into `skillDir` and update lockfile state.

**Work**

- Parse GitHub shorthand, GitHub tree URLs, `skills.sh` input, and local paths.
- Fetch Git sources with the local `git` command.
- Copy local sources conservatively.
- Validate `SKILL.md` frontmatter.
- Store dependency-managed skill bodies under `skillDir`.
- Update lockfile source/hash/installSource data.

**Acceptance criteria**

- GitHub source installs into `skillDir`.
- Local source copies into `skillDir` without mutating the original.
- Invalid skills fail before replacing destinations.
- Lockfile records resolved source information.

**Depends on**: Issues 4, 5, 7

---

### Issue 14: Implement add/attach/remove workflows

**Goal**
Provide package-manager-like dependency management commands.

**Work**

- `add`: update config, install, plan, and apply; roll back config on failure.
- `attach`: add agents to an existing dependency-managed skill while preserving source representation.
- `remove`: remove one or more skills from config, lockfile, installed files, and managed symlinks.
- `remove --agent`: detach only selected agents.
- Keep removal conservative and do not delete unmanaged files.

**Acceptance criteria**

- `add` rolls back config on failure.
- `attach` preserves structured/string source representation.
- `remove` accepts multiple skill names.
- `remove --agent` keeps other agents and the skill body unless the last agent is removed.

**Depends on**: Issues 10, 13

---

## Milestone 7: TUI / wizard

### Issue 15: Implement prompt wizard

**Goal**
Let users perform common actions without remembering CLI flags.

**Work**

- Add `wizard` plus `ask` / `tui` aliases.
- Use `inquire` for prompts.
- Implement add, attach, remove, detach, status, apply, and default agents flows.
- Use CLI/application use cases instead of duplicating core logic.
- Show confirmations before destructive actions.

**Acceptance criteria**

- Wizard can add and remove a skill.
- Wizard can attach/detach agents.
- Wizard can configure `defaultAgents`.
- Empty detach/remove selections do not accidentally remove skills.

**Depends on**: Issue 14

---

## Milestone 8: Diagnostics, mappings, and import

### Issue 16: Implement `doctor` and `agents`

**Goal**
Improve visibility into health and target mappings.

**Work**

- `doctor`: read-only comprehensive diagnosis of config, lockfile, sources, targets, and mappings.
- `agents list`: show effective mappings.
- `agents doctor`: read-only targetDir diagnostics.
- `agents refresh`: refresh bundled mappings into `~/.sksync/agents.json`.

**Acceptance criteria**

- `doctor` never mutates files.
- Missing target directories are warnings, not fatal errors.
- Suggested next commands are shown for actionable problems.
- Agent mappings can be refreshed without touching dependency config.

**Depends on**: Issues 6, 11

---

### Issue 17: Implement copy-only import

**Goal**
Provide a safe migration path from existing manually managed skill directories.

**Work**

- Scan an input directory for valid skill directories.
- Reject path traversal and invalid skill names.
- Copy valid skills into `skillDir`.
- Register dependencies for one or more agents.
- Support `--dry-run`.
- Roll back partial copies on copy failure.

**Acceptance criteria**

- Import never mutates, deletes, or symlink-replaces originals.
- Name conflicts are reported clearly.
- Invalid directories are skipped or fail clearly.
- Partial copy failures clean the destination.

**Depends on**: Issues 13, 14

---

## Milestone 9: Release and documentation

### Issue 18: Add schemas, docs, and examples

**Goal**
Keep user-facing files aligned with implemented behavior.

**Work**

- Add/update JSON Schemas for config, agents, and lockfile.
- Keep example config, agents, and lockfile files valid.
- Document CLI commands, source formats, safety rules, and portability.
- Keep manual/site docs in English.

**Acceptance criteria**

- Schema files are valid JSON.
- Examples reference the correct schema IDs.
- Documentation matches current CLI behavior.
- `bun run docs:build` succeeds when site docs change.

**Depends on**: all user-facing behavior issues

---

### Issue 19: Add Linux release support

**Goal**
Make releases usable on Debian/Ubuntu-style Linux environments.

**Work**

- Build Linux musl release assets for x86_64 and aarch64.
- Add Docker smoke tests for Debian and Ubuntu containers.
- Update `install.sh` to select Linux assets and verify checksums with `sha256sum` or `shasum`.
- Publish Linux assets without waiting for queued macOS runners.

**Acceptance criteria**

- Linux smoke tests pass on Debian/Ubuntu matrix.
- `install.sh` selects the right asset for Linux/macOS.
- Release workflow publishes Linux assets as soon as Linux builds complete.
- macOS assets can be uploaded later and checksums refreshed.

**Depends on**: stable release workflow
