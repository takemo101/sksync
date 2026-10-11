# Wizard update and outdated Implementation Plan

> **For agentic workers:** Execute inline with test-driven development; delegation requires separate operator authorization. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full/selected update and read-only outdated operations to the wizard.

**Architecture:** Extend the existing intent menu and operations module. Build the existing CLI arguments in `tui/commands.rs`, reuse scope/skill prompts and `confirm_and_run`, and route through `run_sksync`; do not duplicate update/outdated application logic.

**Tech Stack:** Rust 1.89+, inquire, clap; no new dependencies.

## Global Constraints

- English UI; macOS/Linux. Global/project selection must map exactly to the existing CLI scope.
- Full update passes no names, preserving its ability to establish a lockfile baseline. Selected mode requires at least one chosen config dependency key and retains CLI baseline checks.
- Update confirmation defaults to no and warns that selected managed-body local edits are replaced. Decline/Esc must not fetch or write.
- Outdated uses human output, performs no update/apply/lockfile write, and preserves existing remote-probe failure/partial-result behavior.
- From first Cargo, use `python3 /tmp/sksync-wiz.IPnS9G/safe-cargo.py <fresh-two-character-label> ...`; tests/PTY fixtures use owned temporary HOME/USERPROFILE/XDG_CONFIG_HOME/TMPDIR and child CWD only.
- No environment dumps, credentials/auth/permissions changes, dependencies, new transaction abstraction, release, PR, version/site changes or deployment.

## Task 1: Expose menu actions (RED → GREEN)

**Files:** `src/tui/mod.rs`.

**Interfaces:** `wizard_intents() -> Vec<Intent>` and `Intent::Display`; dispatch to `operations::run_update` / `operations::run_outdated`.

- [x] Add behavioral tests using existing symbols:

```rust
assert!(wizard_intents().iter().any(|intent| intent.to_string() == "Update skills"));
assert!(wizard_intents().iter().any(|intent| intent.to_string() == "Show outdated skills"));
```

- [x] Run isolated `test --quiet wizard_intents_include`; expect the two new assertions to fail on the existing menu, without compiler/setup failure.
- [x] Add `Update` / `Outdated` intent variants, labels, vector entries and dispatcher branches only after RED.

## Task 2: Reuse the CLI command boundary

**Files:** `src/tui/commands.rs`, `src/tui/operations.rs`.

**Interfaces:** `update_skill_args(skills: &[String], scope: ConfigScope) -> Vec<String>` and `outdated_args(scope: ConfigScope) -> Vec<String>`.

- [x] Test full project/global, selected project/global (including a dash-prefixed valid dependency key), and outdated project/global arguments. Full update must not acquire selected-update semantics; outdated must not add JSON/force/update flags.
- [x] Observe meaningful argument assertion RED after establishing the callable API; distinguish compiler scaffolding failures from behavioral RED.
- [x] Implement the minimal argument builders:

```rust
let mut args = vec!["update".to_owned()];
if !skills.is_empty() {
    args.push("--".to_owned());
    args.extend_from_slice(skills);
}
// --global must precede -- so dash-prefixed config keys stay positional.
```

- [x] `run_update`: select scope; select `All skills` or `Selected skills`; only selected mode loads config and invokes `prompt_skills_from_config`; build arguments; call `confirm_and_run` with the local-edit warning. No force/JSON/dry-run addition.
- [x] `run_outdated`: select scope, then `run_sksync(project_root, &outdated_args(scope))` directly. No confirmation is required for this explicitly chosen read operation.
- [x] Verify the menu/argument tests, then an owned-fixture PTY check of real wizard routing, full/selected/global behavior, declined update and read-only outdated. Never call CWD-changing `run_sksync` directly in parallel unit tests.

## Task 3: Documentation and final validation

**Files:** `README.md`, `docs/UPDATES.md`, `docs/CLI_JSON.md`.

- [x] Describe source-only/unreleased wizard additions separately from the unchanged v0.0.16 binary/manual. Remove the source claim that wizard selection is unavailable; keep CLI update flag limitations and selected-baseline/crash-recovery limits.
- [x] Run isolated fmt, full tests, release build, clippy `-D warnings`, and relevant complete CLI/update regressions; inspect changed-path diagnostics and diff.
- [x] Assert protected site/version/workflows and all unrelated paths match the baseline, no staged changes, and report what was and was not exercised.
- [x] Stop with the verified local feature; commit/PR/merge/release requires a new request.

## Verification

- Genuine menu RED (`r0`, two missing-label assertions) and argument/menu RED (`r1`, seven assertions) preceded implementation; wizard GREEN (`g0`) passed all 10 matching tests.
- Final immutable-driver gates `f1` / `t1` / `b1` / `c1` / `j1` / `u1` passed: fmt, full locked tests, release build, warnings-denied clippy, and complete CLI JSON/update nocapture regressions. macOS harness total: 601; two existing non-UTF-8 CLI fixture bodies returned with explicit EILSEQ omission markers. Repeated targeted suites are not additional unique tests. Linux was not exercised for this local change.
- Eleven real owned-fixture PTY cases passed: all/selected updates in both scopes; dash-prefixed dependency key; default-no decline; Esc; empty selection rejection without falling back to all; read-only outdated in both scopes; read-only remote-query failure. Selected body/hash changed while an edited unselected body and its lock record were preserved. Cancellation and outdated cases preserved whole fixture snapshots. No live network was used.
- Two fixture infrastructure failures were retained separately: waiting for process exit without pumping terminal cursor-position responses, and a global fixture incorrectly using agent scope `global` instead of `user`. Only the temporary check harness was repaired; neither was represented as a product RED or repaired in production code.
- Changed-path LSP/Markdown diagnostics and diff checks were clean. All unrelated content/modes, site/version/workflows and the empty staged index matched the baseline. No dependency, CLI/application implementation, shared CWD helper, publication, or independent delegated-review change.
- Local evidence is under `/tmp/sksync-wiz.IPnS9G/`; source docs mark the wizard additions unreleased and distinguish the unchanged v0.0.16 distribution/manual.
