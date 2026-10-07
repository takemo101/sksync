# Safe Persistence, JSON Output, and Selected Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Execution/delegation requires separate operator authorization; this document only plans the work.

**Goal:** Implement the approved contract in `docs/DESIGN.md` section 13: failure-safe dependency updates, four versioned JSON read commands, and strictly scoped updates.

**Architecture:** Keep domain values and application reports independent of presentation. Filesystem adapters own atomic publication, advisory guards, and opaque installation receipts; the update use case prepares all content and commits the lockfile last. Selected updates merge existing version-5 content records rather than rebuilding a smaller config or planning links.

**Tech Stack:** Rust 2021, standard-library filesystem/locking APIs, existing `serde`/`serde_json`/`thiserror`/`clap`, existing `tempfile` dev dependency, Cargo, GitButler, and local mikan Issues.

## Global Constraints

- Source of truth: `docs/DESIGN.md` sections 13.1–13.6, `docs/ARCHITECTURE.md`, `CONTEXT.md`, and `AGENTS.md`. Read them before implementation; do not infer current availability from planned syntax.
- macOS and Linux only. Standard-library advisory locking requires Rust 1.89 or later; record `rust-version = "1.89"` and verify this floor in CI.
- Do not add dependencies, a generic transaction/mock-filesystem framework, persistent journal, lock-marker registry, registry service, account service, agent runner, or `ci` command.
- Portable lockfile stays version 5: `root: "."`, content/include hashes and resolved sources only; no serialized target/owner records.
- A handled pre-commit update failure restores the whole effective selection. Rollback failure retains old copies and reports recovery paths. Post-commit cleanup failure is a warning, not rollback.
- Crash/SIGKILL/power-loss recovery and instantaneous visibility to readers are explicitly not promised.
- Keep source-namespaced paths and existing flat-path compatibility; do not migrate body layout during an update.
- Never overwrite unmanaged files/directories at agent targets. `--force` remains link-repair-only and is not added to `update`.
- `update` leaves config, dependency agents, provenance, mappings, and symlinks unchanged. Successful replacement of local edits within selected managed bodies retains current semantics.
- Tests use temporary project/home roots; child commands set `HOME`, `USERPROFILE`, and `XDG_CONFIG_HOME` explicitly. Do not mutate process-global environment or contact live remotes.
- Runtime CLI/TUI copy stays English. Issue/plan explanations may be Japanese.
- Use GitButler for version-control writes; never `git commit/reset/switch/checkout` on `gitbutler/workspace`. Preserve the existing `site/.vitepress/config.ts` changes and unrelated work.
- Code review/PR gate: `cargo fmt --check`, `cargo test --quiet`, `cargo build --release --quiet`, `cargo clippy --quiet -- -D warnings`. Export `PATH="$HOME/.cargo/bin:$PATH"` first.
- This plan authorizes neither implementation, automatic agent launches, GitHub Mirrors, PR creation, deployment, nor a release. Create its mikan Issues in `backlog`, without the automation label.

## Sequence and dependency graph

Each P-number below is a stable plan key. The mikan IDs were returned by the local board when these Issues were created; native `depends_on` stores those actual IDs. All 15 Issues start in `backlog` with no automation label or GitHub Mirror.

| key | mikan Issue | independently reviewable deliverable | prerequisites |
| --- | --- | --- | --- |
| P01 | MIK-005 | Atomic byte publication primitive | none |
| P02 | MIK-006 | All state-file writers use atomic publication | P01 |
| P03 | MIK-007 | Canonical directory writer guards and compiler floor | none |
| P04 | MIK-008 | Shared name/path safety validation | none |
| P05 | MIK-009 | Prepared installer and reversible single-body publication | P01, P04 |
| P06 | MIK-010 | Mutating entry points share writer guards | P02, P03, P05 |
| P07 | MIK-011 | Full update prepares a batch and commits lockfile last | P02, P05, P06 |
| P08 | MIK-012 | Common JSON envelope, schema, and single-render error boundary | P07 |
| P09 | MIK-013 | `list --json` and typed list failures | P08 |
| P10 | MIK-014 | `plan --json` and physical-owner DTOs | P08 |
| P11 | MIK-015 | `check --json` and unhealthy-vs-execution failure | P08 |
| P12 | MIK-016 | `outdated --json` migration and typed remote failures | P08 |
| P13 | MIK-017 | Pure selected-update validation and lockfile merge | P07, P09, P10, P11, P12 |
| P14 | MIK-018 | Public selected-update CLI connected to the batch use case | P13 |
| P15 | MIK-019 | Cross-platform contract gate and user-facing migration docs | P09, P10, P11, P12, P14 |

P08 and P13 have phase gates to preserve the approved safety → JSON → selection implementation order. P09–P12 have independent acceptance criteria but touch shared CLI/output/test files; integrate them sequentially unless an operator explicitly arranges isolated worktrees. Do not make a parent tracking Issue a prerequisite of its own children.

## Working method for every Issue

1. Add the listed regression first; run the exact targeted command and observe RED for the missing behavior, not an unrelated setup error.
2. Implement only that Issue's API and behavior. Keep the old public behavior unless the Issue explicitly changes it.
3. Run targeted tests and the four-command global gate. Commit only the Issue's files through `but`, following the installed GitButler skill.
4. Append a mikan Report with changed files, executed commands/results, deliberate exclusions, and remaining recovery paths. Mark completed only after its acceptance checks pass.

Suggested contracts below are new APIs to introduce, not claims that these symbols already exist. Keep these names/types consistent across dependent Issues; update the plan and affected Issues together if review changes an interface.

### P01: Add atomic byte publication without truncating live files

**Depends on:** none

**Files:** create `src/infrastructure/atomic_file.rs`; modify `src/infrastructure/mod.rs` and `src/application/ports.rs`; tests in the new module.

**Produces:** `CleanupWarning { path: PathBuf, message: String }` in application ports; the following infrastructure API:

```rust
pub enum WriteMode { CreateNew, Replace }
pub struct AtomicWriteOutcome { pub warnings: Vec<CleanupWarning> }
pub fn write_atomic(path: &Path, bytes: &[u8], mode: WriteMode)
    -> std::io::Result<AtomicWriteOutcome>;
pub fn resolve_write_path(path: &Path) -> std::io::Result<PathBuf>;
```

- [ ] Add the initial no-overwrite regression below; add replacement, absent-file, symlink-referent, dangling-link, non-regular-file, and Unix permission cases in the same module.

```rust
#[test]
fn atomic_create_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, b"old").unwrap();
    assert!(write_atomic(&path, b"new", WriteMode::CreateNew).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
}
```

- [ ] Run `cargo test --quiet atomic_create_does_not_overwrite` and observe RED.
- [ ] Serialize outside this helper. Resolve an existing file symlink only for I/O, exclusively create a same-parent temporary file, write all bytes, preserve permissions, and `sync_all` before publication. Replace via `std::fs::rename`; create-only via `std::fs::hard_link` followed by owned temporary-name removal. Never unlink the destination first.
- [ ] Introduce private/test-only fault injection around temporary write/sync/publication/removal. Every returned error must be pre-publication with original bytes/absence preserved. After hard-link publication, failed temporary removal returns a successful outcome with a warning. Replacement has no fallible post-rename step.
- [ ] Run `cargo test --quiet infrastructure::atomic_file`; assert no temporary collision is overwritten/followed and no foreign path is cleaned up. Commit the primitive with its tests, not caller migration.

### P02: Migrate normal and restoration state-file writes to the atomic helper

**Depends on:** P01

**Files:** `src/infrastructure/json.rs`, `src/application/init.rs`, `src/application/ports.rs`, `src/cli.rs` (`ConfigFileBackup::restore`), `src/tui/default_agents.rs`, `src/infrastructure/mod.rs`, and their existing test modules; create `src/infrastructure/init.rs` for init's existing filesystem work.

**Consumes:** P01 `write_atomic`, `WriteMode`, and `CleanupWarning`.

**Produces:** atomic config/agent-map/lockfile/backup-restoration publication; an opaque prepared lockfile seam for P07:

```rust
pub trait PreparedLockfileStore: LockfileStore {
    type Prepared;
    fn prepare_lockfile(&self, value: &Lockfile)
        -> Result<Self::Prepared, LockfileStoreError>;
    fn publish_lockfile(&self, value: &Self::Prepared)
        -> Result<Vec<CleanupWarning>, LockfileStoreError>;
}
```

`FileLockfileStore` implements this with `SerializedLockfile(Vec<u8>)` (private bytes); preparation serializes without publishing. Existing `LockfileStore::write` remains available and delegates to prepare/publish.

Keep production application code independent of infrastructure. Move init's existing filesystem operations and embedded agent-map writing into `FileInitStore` in `src/infrastructure/init.rs`. Retain pure config generation/result/error types in application init and introduce this narrow port there (not a generic filesystem interface):

```rust
pub trait InitStore {
    fn initialize(&self, config_path: &Path, skills_dir: &Path, config: &str,
        agent_mapping_path: Option<&Path>) -> Result<InitResult, InitError>;
    fn refresh_agents(&self, path: &Path) -> Result<InitAgentsResult, InitError>;
}
```

Change `init_project(root, store)`, `init_global(config_root, store)`, and `init_agents(config_root, store)` to accept `&impl InitStore`; CLI constructs `FileInitStore`. Keep filesystem regressions with the adapter and coordinator tests with the port. Do not import the atomic-file module from production application init.

- [ ] Extend current config/lockfile round-trip and default-agent preservation tests with injected failed publication and symlinked config. Capture raw original bytes and assert equality on failure, not just parseability.
- [ ] Run `cargo test --quiet lockfile` and `cargo test --quiet default_agents`; run each new regression by name and observe RED before migration.
- [ ] Replace production direct writes of JSON state, including `FileDependencyConfigStore::write_value`, init config creation, conditional initial agent-map creation, explicit agent-map refresh, `ConfigFileBackup::restore`, and default-agent preference saving. Do not replace test fixture writes or skill-package file copies.
- [ ] Use CreateNew for initial config and conditional initial mappings; a competing mapping creator is treated as already present without overwriting it. Use Replace for mutations, explicit refresh, lockfile publication, and restoration. Preserve unknown config fields and existing logical relative-path bases.
- [ ] Carry create-only cleanup warnings through `InitResult`/`InitAgentsResult` and print them at the CLI boundary. Replacement cannot silently turn a committed write into failure. Preparation of a lockfile must finish serialization before P07 changes any body.
- [ ] Audit production write call sites and rerun `cargo test --quiet init`, `cargo test --quiet lockfile`, and `cargo test --quiet default_agents`. Existing bundle/config/provenance and lockfile v2–v5 tests must pass.

### P03: Add non-blocking directory writer guards and record Rust 1.89

**Depends on:** none

**Files:** create `src/infrastructure/write_guard.rs`; modify `src/infrastructure/mod.rs` and `Cargo.toml`; tests in the new module. Do not change command entry points yet.

**Produces:**

```rust
pub struct WriteGuard {
    directories: Vec<PathBuf>,
    handles: Vec<std::fs::File>,
}
impl WriteGuard {
    pub fn acquire(directories: &[PathBuf]) -> std::io::Result<Self>;
    pub fn covers(&self, directories: &[PathBuf]) -> std::io::Result<bool>;
}
```

The fields remain private. `acquire` canonicalizes existing directories, sorts/deduplicates them (including equal Unix device/inode identities), opens directory handles, and uses `std::fs::File::try_lock`; Drop closes all handles. `covers` compares both canonical paths and current device/inode identities with the held handles, rather than trusting unchanged path strings. Reject non-directory resources and propagate WouldBlock/non-supported I/O errors without a PID-marker fallback.

- [ ] Add this independent-handle exclusion test and alias-path/order/error-release/process-exit tests.

```rust
#[test]
fn write_guard_rejects_second_writer() {
    let dir = tempfile::tempdir().unwrap();
    let first = WriteGuard::acquire(&[dir.path().to_path_buf()]).unwrap();
    assert!(WriteGuard::acquire(&[dir.path().to_path_buf()]).is_err());
    drop(first);
    assert!(WriteGuard::acquire(&[dir.path().to_path_buf()]).is_ok());
}
```

- [ ] Run `cargo test --quiet write_guard_rejects_second_writer` and observe RED.
- [ ] Implement canonical directory guards only. Acquire no guards on temporary files or live state files whose inode will be replaced. Document that directory locking is the supported Unix strategy and does not isolate readers or arbitrary external editors.
- [ ] Set package `rust-version = "1.89"`; run `cargo test --quiet write_guard`. P15 verifies native behavior on both supported OSes and the declared compiler floor.

### P04: Reject reserved skill names and unsafe managed-body destinations

**Depends on:** none

**Files:** `src/domain/skill.rs`, `src/infrastructure/install.rs`, `schemas/sksync.schema.json`, `schemas/sksync.bundle.schema.json`, `tests/schema_files.rs`, and existing config/bundle parsing tests in `src/infrastructure/json.rs`.

**Produces:** `SkillNameError::ReservedPathComponent` and this infrastructure check:

```rust
pub(crate) fn validate_managed_destination(skill_dir: &Path, destination: &Path)
    -> Result<(), SkillInstallError>;
```

- [ ] Add shared constructor regressions, retaining normalization/collision tests.

```rust
#[test]
fn rejects_reserved_skill_components() {
    assert!(SkillName::new(".").is_err());
    assert!(SkillName::new(" .. ").is_err());
    assert!(SkillName::new(".review").is_ok());
}
```

- [ ] Run `cargo test --quiet rejects_reserved_skill_components` and observe RED. Add config and bundle manifest cases for both reserved names.
- [ ] Reject normalized `.`/`..` in `SkillName::new`; align schema property-name constraints without rejecting legitimate dot-prefixed names. Validate strict body containment under canonical `skillDir`, including the nearest existing parent for a missing namespace directory. Reject root-as-body, escaping parent aliases, existing body symlinks, regular files, and special files; allow a configured store root that itself resolves through a user-authorized symlink.
- [ ] Run `cargo test --quiet reserved`, `cargo test --quiet managed_destination`, and `cargo test --quiet --test schema_files`. Do not change lexical agent-target grouping or migrate existing flat body paths.

### P05: Prepare and reversibly publish installed skill bodies

**Depends on:** P01, P04

**Files:** `src/application/ports.rs`, `src/infrastructure/install.rs`, `src/application/update.rs`, installer request construction in `src/cli.rs`, and existing installer/application test fakes.

**Consumes:** P04 validation and existing copy/filter/manifest/hash/Git helpers. Extend `SkillInstallRequest` with mandatory `managed_root: PathBuf`; every production caller supplies its resolved config `skill_dir`, not a guessed destination parent.

**Produces:** leave the existing immediate `SkillInstaller` seam available; add the prepared-install seam below for P07. This separate seam avoids rewriting old fake implementations to invent receipt behavior.

```rust
pub struct PreparedSkill<R> {
    pub name: SkillName,
    pub destination: PathBuf,
    pub installed: InstalledSkillSource,
    pub hash: Digest,
    pub files: Vec<LockedFile>,
    pub receipt: R,
}
pub struct SkillRollbackFailure {
    pub skill: SkillName,
    pub retained_backup: Option<PathBuf>,
    pub message: String,
}
pub trait PreparedSkillInstaller {
    type Receipt;
    fn prepare_skill(&self, request: &SkillInstallRequest, destination: &Path,
        skill_name: &str) -> Result<PreparedSkill<Self::Receipt>, SkillInstallError>;
    fn publish_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillInstallError>;
    fn rollback_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillRollbackFailure>;
    fn finalize_skill(&self, receipt: &mut Self::Receipt) -> Vec<CleanupWarning>;
}
```

`FileSystemSkillInstaller` implements it with a private-state receipt owning final/staging/backup paths and publication state. Error cleanup and Drop must never discard the only surviving old backup. Preparation has no live-body effect; rollback of an unpublished receipt only cleans its owned staging.

- [ ] In installer tests, create an old valid body and a new local package. Assert preparation leaves old bytes intact, then inject failure after moving the old directory but before publishing staging; rollback must restore old bytes. Add originally-absent, invalid-package, partial-publication, rollback-failure, and post-commit cleanup-failure cases.
- [ ] Run `cargo test --quiet infrastructure::install` with the new regressions and observe RED before lifecycle conversion.
- [ ] Replace timestamp-collision deletion and delete-before-rename with exclusive sibling staging/backup creation and reversible rename. Reuse `install_to_staging`, include filtering, validation, actual Git-ref resolution, and `hash_directory`; translate its files to `LockedFile` and retain the final destination in metadata.
- [ ] Implement the legacy single-install wrapper as prepare → publish → finalize, or publish failure → rollback. Retain/reveal recovery paths when rollback fails. Add `warnings: Vec<CleanupWarning>` to `InstalledSkillSource` and the existing `UpdateReport`, collect them in the immediate update helper, and adapt all constructors/callers, including add/import/bundle output. Derive Debug/Clone/PartialEq/Eq for CleanupWarning to retain existing report derives. Print warnings only at the presentation boundary. Keep legacy single-install tests and source namespacing behavior passing.
- [ ] Run `cargo test --quiet install`, `cargo test --quiet update`, and `cargo test --quiet add`. This Issue does not claim command-wide atomicity for multi-skill add/install/bundle flows.

### P06: Apply the same writer guards to mutating entry points

**Depends on:** P02, P03, P05

**Files:** `src/cli.rs`, `src/application/init.rs`, `src/tui/default_agents.rs`, `src/tui/config.rs` if needed for path resolution, `src/infrastructure/write_guard.rs`, and CLI/TUI unit tests.

**Consumes:** P01 physical write-path resolution and P03 `WriteGuard`; P05 `managed_root` and path validation. Produces one held guard set per mutation use case, including rollback lifetime.

- [ ] Inventory mutators: init/agents refresh, add/attach/import/remove, install/update/apply, bundle add/remove/sync, and default-agent preference saving. Bundle export writes only its output: guard the physical output parent, not the read-only source store. Pure inspection commands must not acquire writer guards or create directories.
- [ ] Add a busy-state-parent and busy-shared-store regression before side effects, using independent handles and temporary roots. Verify that state-file aliases select the actual referent parent and two projects sharing a store cannot interleave mutations.
- [ ] Run `cargo test --quiet writer_guard` and observe RED at the unguarded entry points. Add a guarded default-agent preservation test.
- [ ] Resolve guarded parents/store roots, create only required missing containers, acquire once in sorted canonical order, reload the snapshot, and call `covers` to reject a changed resource set. Capture config/lockfile backups only after acquisition. Do not lock again in nested stores/installers; do not hold a guard while waiting for TUI confirmation.
- [ ] Acquire guarded snapshots inside the same use-case path used by `run_with_args`, not solely in `main`. TUI preference writes use the same convention. Guards cover restoration and cleanup on every error path.
- [ ] Run `cargo test --quiet writer_guard`, `cargo test --quiet default_agents`, and existing CLI/bundle tests. No crash journal or new all-or-nothing guarantees for other commands.

### P07: Make full update a prepared batch with lockfile commit last

**Depends on:** P02, P05, P06

**Files:** `src/application/update.rs`, `src/cli.rs` (`run_update` and content-lock builder), `src/application/ports.rs` if error additions are needed; create `tests/update_cli.rs`; unit fakes in `src/application/update.rs`.

**Consumes:** `PreparedSkillInstaller`, `PreparedLockfileStore`, and an already held writer guard. Produces a commit-only report and `UpdateError::Lockfile(LockfileStoreError)`, `UpdateError::BuildLockfile { message: String }`, and `UpdateError::RollbackFailed { original: Box<UpdateError>, failures: Vec<SkillRollbackFailure> }`; retain old `update_selected_dependencies` for add isolation. Failure displays include the original cause and each retained recovery path, not only a generic rollback label.

```rust
// Reuse the existing report, extended with warnings by P05.
pub fn update_dependency_batch<I, L, F>(config: &ResolvedConfig,
    only: Option<&BTreeSet<SkillName>>, installer: &I, store: &L,
    build_lockfile: F) -> Result<UpdateReport, UpdateError>
where
    I: PreparedSkillInstaller,
    L: PreparedLockfileStore,
    F: FnOnce(&[PreparedSkill<I::Receipt>]) -> Result<Lockfile, UpdateError>;
```

- [ ] Add fakes with deterministic `prepare_skill`, nth-publication, prepared-lock serialization, lock publication, rollback, and cleanup failures. Snapshot both bodies and raw lock bytes. On prepare failure expect zero publish calls; on later failure expect reverse rollback of every affected receipt, including an originally absent body.
- [ ] Run `cargo test --quiet update_batch` and observe RED. Add a local-only CLI regression: two installed dependencies, changed first source, invalid second source; failed full update must retain both original bodies and lock bytes.
- [ ] Prepare all effective dependencies and hashes, construct the complete candidate lockfile, call `prepare_lockfile`, publish all bodies, and call `publish_lockfile` last. On any pre-commit error roll back all prepared/attempted receipts in reverse order and aggregate recovery failures without losing the original error. Only finalize after successful lock publication.
- [ ] Build content records directly from prepared metadata; retain full-update legacy entry behavior but do not make an agent link plan a prerequisite of update. Reuse existing v5 path/source serialization. Effective empty dependency selection returns a no-op without lock writes.
- [ ] Print installed/lockfile success only after the commit decision, print cleanup warnings without changing successful exit, and leave config/symlinks unchanged. Adapt add's immediate request construction without widening its fetch selection.
- [ ] Run `cargo test --quiet update_batch`, `cargo test --quiet --test update_cli`, `cargo test --quiet add`, and the full gate. Do not expose positional update names yet.

### P08: Define the JSON envelope, schema, and single-render failure boundary

**Depends on:** P07

**Files:** create `src/cli/output.rs`, `schemas/sksync-output.schema.json`, and `tests/cli_json.rs`; modify `src/cli.rs`, `src/main.rs`, and TUI error handling in `src/tui/mod.rs`/`src/tui/commands.rs` as required; extend `tests/schema_files.rs`.

**Produces:** output-only DTO/error infrastructure, preserving current command signatures. Define `JsonCommand` variants List/Plan/Check/Outdated and `OutputScope` Project/Global, serialized lowercase. Define `OutputError { code: String, message: String, hint: Option<String> }`; omit absent hint. Keep error codes centralized constants, selected from typed causes, never message matching.

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEnvelope<T> {
    pub schema_version: u32,
    pub command: JsonCommand,
    pub scope: OutputScope,
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<OutputError>,
}
pub fn write_json<W: std::io::Write, T: serde::Serialize>(writer: &mut W,
    envelope: &JsonEnvelope<T>) -> Result<(), OutputWriteError>;
```

Define `OutputWriteError::Serialize(serde_json::Error)` and `OutputWriteError::Write(std::io::Error)` as source-preserving `thiserror` variants. `write_json` uses `serde_json::to_vec` before writing, appends `b'\n'`, then calls `write_all` once through the supplied writer. No output retry after a write error.

- [ ] Add a unit test constructing a successful Outdated envelope with `data: Some(serde_json::json!({"rows": [], "problems": []}))`; serialize to `Vec<u8>`, parse it, and assert `schemaVersion == 1`, `error.is_null()`, arrays are empty, and output ends in newline. Add serialization/writer failure regressions.
- [ ] Run `cargo test --quiet json_envelope` and observe RED. Schema fixtures cover success, usable failed reports, and failure with null data for all four command data shapes specified in DESIGN 13.3.
- [ ] Introduce `RenderedFailure { exit_code: u8 }` as a private CLI control marker implementing Error, re-exported crate-locally from `cli`. Already-rendered failed handlers return this marker; `main` returns `ExitCode` without printing it. Unrendered errors still print once to stderr and exit 1. Preserve `run`/`run_with_args` Result interfaces; TUI callers must not reprint a marked error.
- [ ] Keep Clap parse errors/help/version behavior intact. Add an integration helper to `tests/cli_json.rs` using `Command::new(env!("CARGO_BIN_EXE_sksync"))`, temporary `current_dir`/HOME/USERPROFILE/XDG_CONFIG_HOME, and `serde_json::from_slice` for single-response assertions. Do not globally change environment.
- [ ] Run `cargo test --quiet json_envelope`, `cargo test --quiet --test cli_json`, and `cargo test --quiet --test schema_files`. Do not add command flags until their individual Issues.

### P09: Add list JSON while distinguishing missing and invalid state

**Depends on:** P08

**Files:** `src/cli.rs` (`ListArgs`, `run_list`), `src/cli/output.rs`, `src/application/list.rs`, `tests/cli_json.rs`; update `site/reference/commands.md` with list's flag and behavior.

**Consumes:** P08 envelope/marker/fixture helper. Produces `ListData` DTO: required `skills` and `lockfileStatus`; each skill has name/source/installSource/include/lockedHash/targets; targets have agent/nullable target/status and structured error on failure. Include tags and nullability exactly as DESIGN 13.3; do not compute current body hashes.

- [ ] Add a temporary-project case with `{"skillDir":"./.sksync/skills","dependencies":{}}`. Run `list --json` without a lockfile; assert exit 0, command/scope/list array, `lockfileStatus: "missing"`, and one JSON response. Write invalid lock JSON and assert exit 1, `INVALID_LOCKFILE`, no success envelope.
- [ ] Run `cargo test --quiet --test cli_json list_json` and observe RED.
- [ ] Add ListArgs.json and a typed report-to-DTO conversion; remove `.ok()` lockfile error swallowing. Distinguish true absence from unreadable/malformed inputs. Resolution/inspection failures retain collected rows, set ok false and a stable code, and exit 1 in human and JSON modes; observed missing/drift/conflict/source-missing states remain successful observations.
- [ ] Sort skills and targets, disable human/ANSI stdout in JSON mode, render errors at the handler boundary even when loading fails. Use configured install-source objects/include filters without fetching or hashing.
- [ ] Run `cargo test --quiet application::list` and `cargo test --quiet --test cli_json list_json`; snapshot tagged resolution/inspection failures and global scope with injected home. Document the changed failure exits in the same Issue.

### P10: Add plan JSON without flattening shared owners

**Depends on:** P08

**Files:** `src/cli.rs` (`PlanArgs`, `run_plan`), `src/cli/output.rs`, `tests/cli_json.rs`, `site/reference/commands.md`.

**Produces:** `PlanData { items, applicable }`; each item has structured owners, source, target, action and variant-specific reason/actualSource. Owner sorting is skill/agent; item sorting is target. Existing domain `LinkPlan` is reused unchanged.

- [ ] Add a local config whose universal project target already contains a regular file. Run `plan --json`: assert exit 0, ok true, applicable false, conflict action with regularFile reason; original file bytes stay unchanged.
- [ ] Run `cargo test --quiet --test cli_json plan_json` and observe RED. Add a same-physical-target pi/fx or universal/fx case with multiple owners and exactly one physical item.
- [ ] Add the flag and typed conversion for createSymlink/alreadySynced/conflict/driftedSymlink/sourceMissing. `applicable` is true only when all actions are CreateSymlink/AlreadySynced; it describes normal non-force apply, not force-repairability.
- [ ] Render loading/planning errors through P08 with null data and exit 1; completed blockers are data with exit 0. No fetching, writes, progress, or target repair.
- [ ] Run `cargo test --quiet application::plan` and `cargo test --quiet --test cli_json plan_json`; document the flag/default human behavior.

### P11: Add check JSON with complete typed problem reports

**Depends on:** P08

**Files:** `src/cli.rs` (`CheckArgs`, `run_check`), `src/cli/output.rs`, `src/application/check.rs` only for typed observations, `tests/cli_json.rs`, `site/reference/commands.md`.

**Produces:** `CheckData { healthy: bool, problems }`, preserving all eight existing problem variants including includeMismatch. JSON field/tag names follow DESIGN 13.3; problem sorting is kind/skill/agent/path.

- [ ] Install/apply a valid local skill under temporary project/home roots, then remove its managed target link. Run `check --json`: assert exit 1, ok false, CHECK_FAILED, healthy false, a targetMissing problem, and exactly one JSON object without appended human text. Restore the fixture and assert healthy exit 0 with an empty array.
- [ ] Run `cargo test --quiet --test cli_json check_json` and observe RED. Missing required lockfile and malformed config cases expect execution-error envelopes with data null, not fabricated healthy reports.
- [ ] Convert CheckProblem directly, not grouped display lines: sourceHashDrift, targetMissing, targetUnexpectedSymlink, brokenSymlink, targetConflict, inspectFailed, hashFailed, includeMismatch. Preserve expected/actual and actualSource where present.
- [ ] Distinguish completed unhealthy checks from aborted execution through error.code/data, while both fail with exit 1. Use P08's rendered-failure marker; the main/TUI boundary does not print another summary.
- [ ] Run `cargo test --quiet application::check` and `cargo test --quiet --test cli_json check_json`; retain human grouping/copy and document the JSON contract.

### P12: Migrate outdated JSON and expose structured remote failures

**Depends on:** P08

**Files:** `src/application/outdated.rs`, `src/cli.rs` (`run_outdated` and human rows), `src/cli/output.rs`, `tests/cli_json.rs`, `site/reference/commands.md`.

**Produces:** retain OutdatedRow's six successful string fields. Extend `OutdatedReport` with `problems: Vec<OutdatedProblem>`; define `OutdatedProblem { skill: String, source: String, wanted: String, error: RemoteRefError }`. Derive Clone/PartialEq/Eq for the existing String-backed `RemoteRefError::Query` to retain report derives; never format errors into latest.

- [ ] Use the existing RemoteRefResolver seam with one outdated Git source, one current source, one failing probe, and a local dependency. Assert only the outdated success is in rows, exactly one typed problem is retained, and no latest value contains error text.
- [ ] Run `cargo test --quiet application::outdated` with the new mixed-results regression and observe RED.
- [ ] Continue collecting after individual probe failures; DTO maps them to REMOTE_QUERY_FAILED. Both output modes exit 1 if any probe failed, retaining successful rows. Empty/all-success reports exit 0 even when updates are available.
- [ ] Replace the bare JSON array with envelope data.rows/data.problems, sort deterministically, and render input failures through P08. Add a local bare-Git-repository integration fixture and an unavailable local remote; no Internet dependency.
- [ ] Run `cargo test --quiet application::outdated` and `cargo test --quiet --test cli_json outdated_json`. Update command examples in the same Issue: `jq '.data.rows'`, checking `.ok`, `.error`, and `.data.problems`; no legacy-array compatibility flag. P15 records the release-note entry without publishing.

### P13: Validate explicit selection and merge locks without inspecting other skills

**Depends on:** P07, P09, P10, P11, P12

**Files:** `src/application/update.rs` and its unit tests; use existing `SkillName`, `ResolvedConfig`, `Lockfile`, and `LockedSkill` without new selection/config models.

**Produces:**

```rust
pub fn resolve_update_selection(config: &ResolvedConfig, names: &[String])
    -> Result<Option<BTreeSet<SkillName>>, UpdateError>;
pub fn validate_selected_baseline(config: &ResolvedConfig,
    selected: &BTreeSet<SkillName>, previous: Option<&Lockfile>) -> Result<(), UpdateError>;
pub fn merge_selected_lockfile(previous: &Lockfile,
    replacements: BTreeMap<SkillName, LockedSkill>) -> Lockfile;
```

None means full update. Named results are nonempty, validated, deduplicated sets of dependency keys. Merge clones previous entries and inserts only replacements; the CLI supplies current successful generation metadata after merge.

- [ ] Add tests for duplicate names, trimming/case sensitivity, unknown/legacy-only/reserved names, empty input, and local/Git dependencies. With a two-entry lock, replace one entry and assert the other LockedSkill remains equal, including source/include/hash/files and any in-memory legacy metadata.
- [ ] Run `cargo test --quiet selected_update` and observe RED. Use existing update-test config constructors, extending them with explicit lock fixtures; no FS or network is needed for the pure merge.
- [ ] Validate the whole explicit name set before fetching. Require an existing readable baseline and entries for every unselected configured skill; selected new entries may be absent. Retain stale unselected lock entries; no pruning, rehash, target inspection, or whole-config regeneration.
- [ ] Add typed UpdateError variants for UnknownSkill, NotDependency, MissingBaseline, MissingUnselectedEntry, and invalid names, with actionable English guidance to full install/update. Scope loading remains the CLI's responsibility, so there is no project/global fallback.
- [ ] Run `cargo test --quiet selected_update`; verify normal older-lock normalization still follows v5 serializer rules and drops legacy target metadata only under existing migration behavior.

### P14: Connect positional update names to the guarded batch use case

**Depends on:** P13

**Files:** `src/cli.rs` (`UpdateArgs`, `run_update`, candidate content-lock construction), `src/application/update.rs`, `tests/update_cli.rs`, `site/reference/commands.md`, `site/guides/lockfile.md`.

**Consumes:** P13 resolved selection/merge and P07 update_dependency_batch; no new transaction path.

- [ ] Extend UpdateArgs with `skills: Vec<String>` as positional names. Add local fixtures with two installed dependencies, edited sources for both, and edited/missing unselected installed content. Run `update review`: assert only review changes, qa's raw content or absence and full parsed lock entry remain unchanged, and config/target symlinks retain their snapshot.
- [ ] Run `cargo test --quiet --test update_cli selected_update` and observe RED. Use local repositories for exact Git-ref/include tests and a temporary global home for scope isolation.
- [ ] Resolve all names and baseline requirements before preparation. Acquire guards and validate the guarded snapshot, then build selected candidate entries from prepared metadata and merge into the old lock. Do not filter ResolvedConfig itself or call build_plan_from_config/build_lockfile_from_plan for selected updates.
- [ ] Route no-name full updates through the same batch. Named unknown/legacy failures fetch nothing and publish nothing. Multiple selected failures restore the entire selection and raw previous lock bytes. Effective empty full selection writes no lockfile.
- [ ] Verify bundle provenance/agent assignments/shared target ownership stay unchanged, installed layout does not move, and head observations from outdated are not represented as a frozen plan. No --force/--dry-run/--json or new wizard menu is added to update.
- [ ] Run `cargo test --quiet --test update_cli`, `cargo test --quiet add`, and the full gate. Update syntax, baseline errors, v5 merge semantics, and manual-recovery limits in the command/lockfile guides.

### P15: Add the cross-platform acceptance gate and migration guide

**Depends on:** P09, P10, P11, P12, P14

**Files:** create `.github/workflows/state-contract.yml` and `site/guides/state-safety.md`; modify `README.md`, `site/reference/commands.md`, `site/guides/lockfile.md`, `site/.vitepress/config.ts` navigation only, `docs/DESIGN.md` implementation status, `docs/RELEASE.md` release-check checklist, and relevant integration tests.

- [ ] Add native ubuntu-latest/macOS-latest jobs for stable and Rust 1.89, running the four Cargo gates. Verify directory guards, symlink/permission handling, rollback/cleanup, sorted envelopes, malformed-input exits, and unselected isolation on both OSes. Do not replace the existing release/Linux Docker workflows or add deployment steps.
- [ ] Audit every new child test command's temporary HOME/USERPROFILE/XDG_CONFIG_HOME. If reusing helpers from `tests/bundle_cli.rs`, fix only the helper isolation necessary for these regressions; do not silently use the real home or live remotes.
- [ ] Validate JSON DTO fixtures against the published schema structure with existing Rust/serde test tools; no runtime schema-validation dependency. Assert single-response parsing, newline termination, required nulls, tags, empty arrays, partial remote reports, and current lockfile portability.
- [ ] Document full-selection rollback, advisory/concurrent-reader limits, manual recovery paths, successful local-edit replacement semantics, create-only/permission/symlink behavior, selected baselines, and the deliberate outdated array → envelope and failure-exit changes. Include the concrete migration example below.

```sh
# Old client: sksync outdated --json | jq '.[]'
# New client: inspect ok/problems and read data.rows; exit 1 can include partial rows.
status=0
sksync outdated --json > outdated.json || status=$?
jq '.ok, .error, .data.problems, .data.rows' outdated.json
printf 'sksync exit status: %s\n' "$status"
```

- [ ] Add a release-check entry in `docs/RELEASE.md` requiring these breaking changes in the future release notes. Mark DESIGN section 13 implemented only when every acceptance check actually passes. Do not select a release version, publish, mirror Issues, open a PR, or modify release assets as part of this Issue.
- [ ] Run the four Cargo gates, `bun run docs:build`, JSON example parsing, and `git diff --check`. Preserve the pre-existing formatting changes in `.vitepress/config.ts`; use a targeted navigation edit rather than rewriting the file.

## Self-review and handoff

- Persistence coverage: P01–P07 own every ordinary-error and cleanup boundary; P15 verifies OS/compiler behavior. Other mutators gain atomic writes/guards/safe single-body publication but no new command-wide rollback promise.
- JSON coverage: P08 owns the envelope/schema/top-level boundary; P09–P12 independently own each report adapter and its errors/exits. P15 documents and gates the intentional compatibility change.
- Selection coverage: P04 protects names/paths; P13 owns pure selection/merge; P14 owns end-to-end CLI isolation and rollback. No target records are added to v5.
- First implementation candidates are P01, P03, or P04. The preferred sequence starts at P01. Follow actual mikan dependencies, and keep later Issues in backlog until the operator chooses execution.
- The mikan bodies reproduce their individual section and the global constraints; native dependencies point to actual Issue IDs. The plan is the cross-Issue API contract and must remain synchronized with them.
