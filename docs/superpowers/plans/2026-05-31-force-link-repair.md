# Force Link Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the documented `--force` behavior so link-applying commands can repair drifted or broken target symlinks without ever replacing regular files or directories.

**Architecture:** Keep target-state decisions in the planner, make the application layer decide which planned actions are force-repairable, and keep filesystem safety checks in the `LinkApplier` implementation. CLI commands that already run link application should pass the same force semantics through to `apply_link_plan`.

**Tech Stack:** Rust 2021, clap, thiserror, std Unix symlinks, existing unit tests and integration-style CLI parser tests.

---

## File map

- Modify `src/application/ports.rs`: extend `LinkApplier` with a symlink-only replacement operation and add filesystem error variants for safe unlink failures.
- Modify `src/infrastructure/fs.rs`: implement symlink-only replacement for real filesystem targets, refusing regular files and directories even if the planner was stale.
- Modify `src/application/apply.rs`: make `ApplyOptions.force` meaningful for `DriftedSymlink` and `ConflictReason::BrokenSymlink`; keep regular file, directory, and missing source failures unchanged.
- Modify `src/application/add.rs`: pass force through the add workflow's final apply step.
- Modify `src/cli.rs`: add `--force` to `add`, `attach`, `install`, `bundle add`, and `bundle sync`; pass force into the existing apply calls.
- Tests in `src/application/apply.rs`: cover force replacement behavior and non-force safety.
- Tests in `src/infrastructure/fs.rs`: cover real symlink replacement and non-symlink refusal.
- Tests in `src/cli.rs`: cover CLI parsing for the new flags.

---

### Task 1: Add a symlink-only replacement port

**Files:**
- Modify: `src/application/ports.rs`

- [ ] **Step 1: Extend `LinkApplyError`**

In `src/application/ports.rs`, change `LinkApplyError` to include explicit replacement failure variants:

```rust
#[derive(Debug, Error)]
pub enum LinkApplyError {
    #[error("failed to create parent directory {path}: {source}")]
    CreateParent {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("target already exists at {path}")]
    TargetExists { path: String },
    #[error("refusing to replace non-symlink target at {path}")]
    TargetNotSymlink { path: String },
    #[error("failed to remove existing symlink {path}: {source}")]
    RemoveSymlink {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create symlink {target} -> {source}: {error}")]
    CreateSymlink {
        source: String,
        target: String,
        #[source]
        error: std::io::Error,
    },
}
```

- [ ] **Step 2: Extend `LinkApplier`**

In the same file, update the trait:

```rust
pub trait LinkApplier {
    fn create_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError>;

    fn replace_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError>;
}
```

- [ ] **Step 3: Run the focused compile check**

Run:

```bash
cargo test --quiet application::apply
```

Expected: compile fails because `FileSystemLinkStore` and fake `LinkApplier` test doubles do not implement `replace_symlink` yet.

- [ ] **Step 4: Commit**

```bash
but status -fv
but commit force-link-repair -c -m "Add link replacement port" --changes <ports-file-id>
```

---

### Task 2: Implement filesystem symlink replacement safely

**Files:**
- Modify: `src/infrastructure/fs.rs`
- Test: `src/infrastructure/fs.rs`

- [ ] **Step 1: Add failing filesystem tests**

Append these tests inside `#[cfg(test)] mod tests` in `src/infrastructure/fs.rs`:

```rust
#[test]
fn replace_symlink_replaces_unexpected_symlink() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let expected = temp_dir.path().join("expected");
    let actual = temp_dir.path().join("actual");
    let target_link = temp_dir.path().join("target");
    fs::create_dir(&expected).expect("create expected");
    fs::create_dir(&actual).expect("create actual");
    symlink(&actual, &target_link).expect("create existing symlink");

    FileSystemLinkStore
        .replace_symlink(&source_path(&expected), &target_path(&target_link))
        .expect("replace symlink");

    let replaced = fs::read_link(&target_link).expect("read replaced link");
    assert_eq!(replaced, expected.canonicalize().expect("canonical expected"));
}

#[test]
fn replace_symlink_replaces_broken_symlink() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let expected = temp_dir.path().join("expected");
    let target_link = temp_dir.path().join("target");
    fs::create_dir(&expected).expect("create expected");
    symlink(temp_dir.path().join("missing"), &target_link).expect("create broken symlink");

    FileSystemLinkStore
        .replace_symlink(&source_path(&expected), &target_path(&target_link))
        .expect("replace broken symlink");

    let replaced = fs::read_link(&target_link).expect("read replaced link");
    assert_eq!(replaced, expected.canonicalize().expect("canonical expected"));
}

#[test]
fn replace_symlink_refuses_regular_file() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let expected = temp_dir.path().join("expected");
    let target = temp_dir.path().join("target");
    fs::create_dir(&expected).expect("create expected");
    fs::write(&target, "manual file").expect("write file");

    let error = FileSystemLinkStore
        .replace_symlink(&source_path(expected), &target_path(&target))
        .expect_err("regular file is not replaced");

    assert!(matches!(
        error,
        crate::application::ports::LinkApplyError::TargetNotSymlink { .. }
    ));
    assert_eq!(fs::read_to_string(target).expect("read file"), "manual file");
}

#[test]
fn replace_symlink_refuses_directory() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let expected = temp_dir.path().join("expected");
    let target = temp_dir.path().join("target");
    fs::create_dir(&expected).expect("create expected");
    fs::create_dir(&target).expect("create target directory");

    let error = FileSystemLinkStore
        .replace_symlink(&source_path(expected), &target_path(target))
        .expect_err("directory is not replaced");

    assert!(matches!(
        error,
        crate::application::ports::LinkApplyError::TargetNotSymlink { .. }
    ));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
cargo test --quiet infrastructure::fs::tests::replace_symlink
```

Expected: compile fails because `replace_symlink` is not implemented on `FileSystemLinkStore`.

- [ ] **Step 3: Add shared symlink creation helper and implement replacement**

In `src/infrastructure/fs.rs`, refactor `create_symlink` and add `replace_symlink`:

```rust
impl LinkApplier for FileSystemLinkStore {
    fn create_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        if fs::symlink_metadata(target.as_path()).is_ok() {
            return Err(LinkApplyError::TargetExists {
                path: display_path(target.as_path()),
            });
        }

        create_symlink_at(source.as_path(), target.as_path())
    }

    fn replace_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        let metadata = fs::symlink_metadata(target.as_path()).map_err(|source| {
            LinkApplyError::RemoveSymlink {
                path: display_path(target.as_path()),
                source,
            }
        })?;

        if !metadata.file_type().is_symlink() {
            return Err(LinkApplyError::TargetNotSymlink {
                path: display_path(target.as_path()),
            });
        }

        fs::remove_file(target.as_path()).map_err(|source| LinkApplyError::RemoveSymlink {
            path: display_path(target.as_path()),
            source,
        })?;

        create_symlink_at(source.as_path(), target.as_path())
    }
}

fn create_symlink_at(source: &Path, target: &Path) -> Result<(), LinkApplyError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| LinkApplyError::CreateParent {
            path: display_path(parent),
            source,
        })?;
    }

    let link_source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());

    std::os::unix::fs::symlink(&link_source, target).map_err(|error| {
        LinkApplyError::CreateSymlink {
            source: display_path(&link_source),
            target: display_path(target),
            error,
        }
    })
}
```

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test --quiet infrastructure::fs::tests::replace_symlink
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
but status -fv
but commit force-link-repair -m "Implement safe symlink replacement" --changes <fs-file-id>
```

---

### Task 3: Make `apply_link_plan` use `--force`

**Files:**
- Modify: `src/application/apply.rs`
- Test: `src/application/apply.rs`

- [ ] **Step 1: Update the fake applier**

In `src/application/apply.rs` tests, replace `FakeApplier` with this version:

```rust
#[derive(Default)]
struct FakeApplier {
    created: RefCell<Vec<(PathBuf, PathBuf)>>,
    replaced: RefCell<Vec<(PathBuf, PathBuf)>>,
}

impl LinkApplier for FakeApplier {
    fn create_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        self.created.borrow_mut().push((
            source.as_path().to_path_buf(),
            target.as_path().to_path_buf(),
        ));
        Ok(())
    }

    fn replace_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        self.replaced.borrow_mut().push((
            source.as_path().to_path_buf(),
            target.as_path().to_path_buf(),
        ));
        Ok(())
    }
}
```

- [ ] **Step 2: Add failing force tests**

Append these tests to `src/application/apply.rs`:

```rust
#[test]
fn unexpected_symlink_is_replaced_with_force() {
    let plan = LinkPlan::new(vec![item(PlanAction::DriftedSymlink {
        actual_source: PathBuf::from("other"),
    })]);
    let applier = FakeApplier::default();
    let lockfiles = FakeLockfileStore::default();

    apply_link_plan(
        &plan,
        &lockfile(),
        &applier,
        &lockfiles,
        ApplyOptions {
            force: true,
            skip_blocked_targets: false,
        },
    )
    .expect("force replaces drifted symlink");

    assert!(applier.created.borrow().is_empty());
    assert_eq!(applier.replaced.borrow().len(), 1);
    assert!(lockfiles.written.get());
}

#[test]
fn broken_symlink_is_replaced_with_force() {
    let plan = LinkPlan::new(vec![item(PlanAction::Conflict {
        reason: ConflictReason::BrokenSymlink,
    })]);
    let applier = FakeApplier::default();
    let lockfiles = FakeLockfileStore::default();

    apply_link_plan(
        &plan,
        &lockfile(),
        &applier,
        &lockfiles,
        ApplyOptions {
            force: true,
            skip_blocked_targets: false,
        },
    )
    .expect("force replaces broken symlink");

    assert!(applier.created.borrow().is_empty());
    assert_eq!(applier.replaced.borrow().len(), 1);
    assert!(lockfiles.written.get());
}

#[test]
fn regular_file_conflict_still_fails_with_force() {
    let plan = LinkPlan::new(vec![item(PlanAction::Conflict {
        reason: ConflictReason::RegularFile,
    })]);
    let applier = FakeApplier::default();
    let lockfiles = FakeLockfileStore::default();

    let error = apply_link_plan(
        &plan,
        &lockfile(),
        &applier,
        &lockfiles,
        ApplyOptions {
            force: true,
            skip_blocked_targets: false,
        },
    )
    .expect_err("regular file conflict fails even with force");

    assert!(matches!(error, ApplyError::Conflict { .. }));
    assert!(applier.created.borrow().is_empty());
    assert!(applier.replaced.borrow().is_empty());
    assert!(!lockfiles.written.get());
}

#[test]
fn directory_conflict_still_fails_with_force() {
    let plan = LinkPlan::new(vec![item(PlanAction::Conflict {
        reason: ConflictReason::Directory,
    })]);
    let applier = FakeApplier::default();
    let lockfiles = FakeLockfileStore::default();

    let error = apply_link_plan(
        &plan,
        &lockfile(),
        &applier,
        &lockfiles,
        ApplyOptions {
            force: true,
            skip_blocked_targets: false,
        },
    )
    .expect_err("directory conflict fails even with force");

    assert!(matches!(error, ApplyError::Conflict { .. }));
    assert!(applier.created.borrow().is_empty());
    assert!(applier.replaced.borrow().is_empty());
    assert!(!lockfiles.written.get());
}
```

- [ ] **Step 3: Run tests to verify failure**

Run:

```bash
cargo test --quiet application::apply::tests::unexpected_symlink_is_replaced_with_force application::apply::tests::broken_symlink_is_replaced_with_force
```

Expected: FAIL because `force` still does not allow or perform replacement.

- [ ] **Step 4: Implement force-aware validation and application**

In `src/application/apply.rs`, update `apply_link_plan` and add helpers:

```rust
pub fn apply_link_plan(
    plan: &LinkPlan,
    lockfile: &Lockfile,
    applier: &impl LinkApplier,
    lockfile_store: &impl LockfileStore,
    options: ApplyOptions,
) -> Result<(), ApplyError> {
    validate_plan_is_safe_to_apply(plan, options)?;

    for item in &plan.items {
        match &item.action {
            PlanAction::CreateSymlink => applier.create_symlink(&item.source, &item.target)?,
            action if options.force && is_force_replace_action(action) => {
                applier.replace_symlink(&item.source, &item.target)?;
            }
            _ => {}
        }
    }

    lockfile_store.write(lockfile)?;
    Ok(())
}

fn is_force_replace_action(action: &PlanAction) -> bool {
    matches!(
        action,
        PlanAction::DriftedSymlink { .. }
            | PlanAction::Conflict {
                reason: ConflictReason::BrokenSymlink,
            }
    )
}
```

Then update the `Conflict` and `DriftedSymlink` arms in `validate_plan_is_safe_to_apply` so `force` allows only `BrokenSymlink` and `DriftedSymlink` to proceed.

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test --quiet application::apply
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
but status -fv
but commit force-link-repair -m "Apply forced symlink repair actions" --changes <apply-file-id>
```

---

### Task 4: Thread force through direct CLI link commands

**Files:**
- Modify: `src/application/add.rs`
- Modify: `src/cli.rs`
- Test: `src/cli.rs`

- [ ] **Step 1: Add failing CLI parser tests**

Append this test in `src/cli.rs`'s existing `#[cfg(test)]` module:

```rust
#[test]
fn force_flags_parse_for_direct_link_commands() {
    Cli::try_parse_from(["sksync", "add", "owner/repo/skills/review", "--agent", "pi", "--force"])
        .expect("add --force parses");
    Cli::try_parse_from(["sksync", "attach", "review", "--agent", "pi", "--force"])
        .expect("attach --force parses");
    Cli::try_parse_from(["sksync", "install", "--force"])
        .expect("install --force parses");
}
```

- [ ] **Step 2: Run parser test to verify failure**

Run:

```bash
cargo test --quiet cli::tests::force_flags_parse_for_direct_link_commands
```

Expected: FAIL because these commands do not accept `--force` yet.

- [ ] **Step 3: Add force fields to CLI arg structs**

In `src/cli.rs`, add this field to `AddArgs`, `AttachArgs`, and `InstallArgs`:

```rust
/// Replace drifted or broken target symlinks during the final link apply step.
#[arg(long)]
force: bool,
```

- [ ] **Step 4: Pass force into `run_add_workflow`**

In `src/application/add.rs`, add a `force: bool` parameter to `run_add_workflow`, and use:

```rust
ApplyOptions {
    force,
    skip_blocked_targets: true,
}
```

Update `run_add` to pass `args.force`. Update existing `run_add_workflow` tests to pass `false`.

- [ ] **Step 5: Pass force in `attach` and `install`**

In `run_attach`, use:

```rust
ApplyOptions {
    force: args.force,
    skip_blocked_targets: true,
}
```

In `run_install`, use:

```rust
ApplyOptions {
    force: args.force,
    skip_blocked_targets: false,
}
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
cargo test --quiet cli::tests::force_flags_parse_for_direct_link_commands application::add
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
but status -fv
but commit force-link-repair -m "Thread force through direct link commands" --changes <cli-file-id>,<add-file-id>
```

---

### Task 5: Thread force through bundle link commands

**Files:**
- Modify: `src/cli.rs`
- Test: `src/cli.rs`

- [ ] **Step 1: Add failing parser tests for bundle force flags**

Append this test in `src/cli.rs`:

```rust
#[test]
fn force_flags_parse_for_bundle_link_commands() {
    Cli::try_parse_from(["sksync", "bundle", "add", "./bundles/review-workflow", "--agent", "pi", "--force"])
        .expect("bundle add --force parses");
    Cli::try_parse_from(["sksync", "bundle", "sync", "review-workflow", "--force"])
        .expect("bundle sync --force parses");
}
```

- [ ] **Step 2: Run parser tests to verify failure**

Run:

```bash
cargo test --quiet cli::tests::force_flags_parse_for_bundle_link_commands
```

Expected: FAIL because bundle link commands do not accept `--force` yet.

- [ ] **Step 3: Add force fields to bundle arg structs**

In `src/cli.rs`, add this field to `BundleAddArgs` and `BundleSyncArgs`:

```rust
/// Replace drifted or broken target symlinks during the final link apply step.
#[arg(long)]
force: bool,
```

Do not change `BundleRemoveArgs`. `BundleExportArgs` already has its own output-directory `--force` with different semantics.

- [ ] **Step 4: Pass force into bundle apply calls**

In `run_bundle_sync` and `run_bundle_add`, use:

```rust
ApplyOptions {
    force: args.force,
    skip_blocked_targets: true,
}
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test --quiet cli::tests::force_flags_parse_for_bundle_link_commands
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
but status -fv
but commit force-link-repair -m "Thread force through bundle link commands" --changes <cli-file-id>
```

---

### Task 6: Update command reference docs to match implemented flags

**Files:**
- Modify: `site/reference/commands.md`
- Optional modify if examples need parity: `README.md`

- [ ] **Step 1: Update command reference examples and flag descriptions**

Add examples and flag descriptions for:

```text
sksync add <source> --agent <agent> --force
sksync attach <skill> --agent <agent> --force
sksync install --force
sksync bundle add <source> --agent <agent> --force
sksync bundle sync <name> --force
```

Use this exact meaning:

```text
During the final link apply step, replace drifted or broken target symlinks only. Never replaces regular files or directories.
```

Keep `bundle export --force` described separately as replacing an existing generated output directory.

- [ ] **Step 2: Update safety rules**

In `site/reference/commands.md`, update the safety list to include:

```md
- Existing plain files and directories are never overwritten, even with `--force`.
- Link-applying commands support `--force` only for drifted or broken target symlinks; missing sources, regular files, and directories still fail.
```

- [ ] **Step 3: Run docs diff inspection**

Run:

```bash
but diff <site-reference-file-id>
```

Expected: command reference documents implemented flags and does not imply `remove --force` or `update --force` exist.

- [ ] **Step 4: Commit**

```bash
but status -fv
but commit force-link-repair -m "Document implemented force flags" --changes <site-reference-file-id>
```

---

### Task 7: Final verification and PR

**Files:**
- No planned source changes beyond previous tasks.

- [ ] **Step 1: Run formatting check**

Run:

```bash
cargo fmt --check
```

Expected: PASS.

- [ ] **Step 2: Run tests**

Run:

```bash
cargo test --quiet
```

Expected: PASS.

- [ ] **Step 3: Run release build**

Run:

```bash
cargo build --release --quiet
```

Expected: PASS.

- [ ] **Step 4: Run clippy**

Run:

```bash
cargo clippy --quiet -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Inspect branch**

Run:

```bash
but status -fv
but branch show force-link-repair -f
```

Expected: no unassigned changes from this work; branch contains focused commits for port, filesystem replacement, apply semantics, CLI threading, bundle threading, and docs.

- [ ] **Step 6: Create PR**

Run:

```bash
but pr new force-link-repair -m "Implement force link repair semantics

## Summary
- make apply --force replace drifted or broken target symlinks only
- thread --force through add, attach, install, bundle add, and bundle sync
- keep regular files, directories, and missing sources as blocking conflicts
- update command reference docs

## Verification
- cargo fmt --check
- cargo test --quiet
- cargo build --release --quiet
- cargo clippy --quiet -- -D warnings"
```

If GitButler forge integration cannot create the PR, push with `but push force-link-repair` and use `gh pr create` as a fallback.

---

## Self-review

- Spec coverage: the plan covers `apply`, `install`, `attach`, `add`, `bundle add`, `bundle sync`, `bundle export` distinction, and excludes `update`/`remove`/read-only commands.
- Safety coverage: regular files, directories, missing sources, and non-target paths remain protected; filesystem replacement rechecks symlink type before unlinking.
- Placeholder scan: no TBD/TODO placeholders are left; each implementation step includes concrete code or command text.
- Type consistency: the plan uses existing `ApplyOptions`, `PlanAction`, `ConflictReason`, `LinkApplier`, `SourcePath`, and `TargetPath` names.
