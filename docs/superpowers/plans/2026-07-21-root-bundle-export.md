# Root Bundle Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow `sksync bundle export <name> --root` to safely write only `sksync.bundle.json` in the active project or global configuration root.

**Architecture:** Model export destinations as either a replaceable bundle directory or a manifest file. Keep directory staging and snapshot behavior unchanged; route the root manifest target through a sibling temporary file and rename so it cannot replace configuration-root contents.

**Tech Stack:** Rust, clap derive, serde JSON, tempfile-backed integration tests, VitePress Markdown.

## Global Constraints

- `--root` and `--output <dir>` are mutually exclusive; one is required for `bundle export`.
- `--root` is manifest-only and conflicts with `--snapshot`.
- Project root writes `<cwd>/sksync.bundle.json`; `--global --root` writes `~/.sksync/sksync.bundle.json`.
- Existing root manifest is unchanged without `--force`; with `--force`, only that file is replaced.
- Root export never creates, copies, deletes, or ignores `skills/`; committed `./skills/<name>` sources remain unchanged in the generated manifest.
- Tests must use temporary directories and must never touch the real home directory.
- Use `but` for commits; do not use Git write commands.

---

## File structure

- `src/application/bundle.rs` — owns the destination enum, validates snapshot/destination compatibility, and applies either directory replacement or manifest-file replacement.
- `src/cli.rs` — parses `--root`, selects the project/global manifest destination, preserves protected-path checks for directory outputs, and prints the resolved destination.
- `tests/bundle_cli.rs` — verifies root export behavior through the compiled CLI, including isolated `HOME` coverage for global mode.
- `site/guides/bundles.md` — documents root export, local `skills/` bundle layout, and consumer-side relative-source resolution.
- `site/guides/project-config.md` — distinguishes generated `.sksync/` data from version-controlled local `skills/` sources.
- `site/reference/commands.md` — documents syntax, flags, and `--force` semantics for root export.

## Task 1: Add an explicit manifest-file export destination

**Files:**

- Modify: `src/application/bundle.rs:210-610`
- Test: `src/application/bundle.rs:1294-1485`

**Interfaces:**

- Consumes: `BundleExportPlanInput`, `BundleExportPlan`, `BundleExportMode`, and `write_bundle_manifest`.
- Produces: `BundleExportDestination`, `BundleExportDestination::path()`, and `apply_bundle_export_plan` support for directory and manifest-file outputs.

- [ ] **Step 1: Write failing application tests for the new destination semantics**

  Add these tests next to the existing export-plan/apply tests:

  ```rust
  #[test]
  fn manifest_file_export_plan_preserves_manifest_only_sources() {
      let temp = tempfile::tempdir().unwrap();
      let manifest_path = temp.path().join("sksync.bundle.json");
      let plan = export_plan_for_test(
          BundleExportDestination::ManifestFile(manifest_path.clone()),
          BundleExportMode::ManifestOnly,
      );

      assert_eq!(plan.destination, BundleExportDestination::ManifestFile(manifest_path));
      assert_eq!(plan.items[0].manifest_source, "github:org/repo/skills/review#main");
      assert_eq!(plan.items[0].snapshot_destination, None);
  }

  #[test]
  fn manifest_file_destination_rejects_snapshot_export() {
      let temp = tempfile::tempdir().unwrap();
      let error = build_bundle_export_plan(BundleExportPlanInput {
          name: "team-baseline".to_owned(),
          description: None,
          destination: BundleExportDestination::ManifestFile(
              temp.path().join("sksync.bundle.json"),
          ),
          mode: BundleExportMode::Snapshot,
          selected_skills: vec![],
          dependencies: vec![BundleExportDependencyConfig {
              name: "review".to_owned(),
              source: "github:org/repo/skills/review#main".to_owned(),
              include: None,
          }],
          resolved_skills: vec![],
      })
      .unwrap_err();

      assert!(error.to_string().contains("snapshot export requires a directory"));
  }
  ```

  Update the existing `export_plan_for_test` helper to accept `BundleExportDestination`, and update its existing callers to pass `BundleExportDestination::Directory(output)`.

- [ ] **Step 2: Run the focused tests and verify they fail**

  Run:

  ```sh
  cargo test --quiet application::bundle::tests::manifest_file_export_plan_preserves_manifest_only_sources
  cargo test --quiet application::bundle::tests::manifest_file_destination_rejects_snapshot_export
  ```

  Expected: compilation failure because `BundleExportDestination` and the `destination` field do not exist.

- [ ] **Step 3: Introduce the destination model and destination-aware plan construction**

  Replace the `output: PathBuf` fields in `BundleExportPlanInput` and `BundleExportPlan` with `destination: BundleExportDestination`. Define the enum beside `BundleExportMode`:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum BundleExportDestination {
      Directory(PathBuf),
      ManifestFile(PathBuf),
  }

  impl BundleExportDestination {
      pub fn path(&self) -> &Path {
          match self {
              Self::Directory(path) | Self::ManifestFile(path) => path,
          }
      }
  }
  ```

  At the start of `build_bundle_export_plan`, reject `BundleExportMode::Snapshot` with `BundleExportDestination::ManifestFile(_)` using a new `BundleExportError::SnapshotRequiresDirectoryDestination` error whose display message is `snapshot export requires a directory destination`.

  Preserve `dependency.source` for manifest-only entries. Populate `snapshot_destination` only for `(BundleExportDestination::Directory(output), BundleExportMode::Snapshot)`:

  ```rust
  let snapshot_destination = match (&input.destination, input.mode) {
      (BundleExportDestination::Directory(output), BundleExportMode::Snapshot) => {
          Some(output.join("skills").join(&name))
      }
      _ => None,
  };
  ```

- [ ] **Step 4: Split application into directory and manifest-file apply paths**

  Keep the current staging-directory behavior in a private `apply_directory_bundle_export(plan, output, options)` function. Add a private manifest path:

  ```rust
  fn apply_manifest_file_bundle_export(
      manifest: &BundleManifest,
      output: &Path,
      options: BundleExportApplyOptions,
  ) -> std::result::Result<(), BundleExportError> {
      if output.exists() && !options.force {
          return Err(BundleExportError::OutputExists(output.display().to_string()));
      }
      let staging = temporary_bundle_export_staging_file(output);
      write_bundle_manifest(&staging, manifest)?;
      std::fs::rename(&staging, output).map_err(|source| BundleExportError::ReplaceOutput {
          path: output.display().to_string(),
          source,
      })
  }
  ```

  Implement `temporary_bundle_export_staging_file` with the same parent directory, process ID, and nonce strategy as `temporary_bundle_export_staging_dir`, but append the nonce to a hidden file name such as `.sksync.bundle.json.sksync-export-staging-<pid>-<nonce>`. On write or rename failure, best-effort remove that temporary file and return the original error. Do not call `remove_dir_all`, `remove_file`, or `replace_bundle_export_output` on a `ManifestFile` destination.

  Dispatch in `apply_bundle_export_plan`:

  ```rust
  match &plan.destination {
      BundleExportDestination::Directory(output) => {
          apply_directory_bundle_export(plan, output, options)
      }
      BundleExportDestination::ManifestFile(output) => {
          apply_manifest_file_bundle_export(&plan.manifest, output, options)
      }
  }
  ```

- [ ] **Step 5: Run application tests and formatting**

  Run:

  ```sh
  cargo fmt --check
  cargo test --quiet application::bundle::tests
  ```

  Expected: all bundle application-unit tests pass, including the two new destination tests.

- [ ] **Step 6: Commit the application-layer change**

  Run `but diff`, note only the IDs for `src/application/bundle.rs`, then commit them:

  ```sh
  but commit root-bundle-export -c -m "feat: add manifest file bundle export destination" --changes <bundle-rust-change-id>
  ```

  Expected: GitButler reports the created commit and no unrelated changes are included.

## Task 2: Parse `--root` and cover CLI behavior

**Files:**

- Modify: `src/cli.rs:292-313, 931-1065`
- Modify: `tests/bundle_cli.rs:1-110, 840-1030`

**Interfaces:**

- Consumes: `BundleExportDestination`, `BundleExportPlanInput`, `BundleExportDestination::path()`, `config_root_for_global()`, and existing `sksync` integration-test helper.
- Produces: `sksync bundle export <name> --root`, `sksync bundle export <name> --global --root`, and clap errors for invalid flag combinations.

- [ ] **Step 1: Add failing root-export integration tests**

  Add a helper that launches the compiled binary with an isolated home directory:

  ```rust
  fn sksync_with_home(root: &Path, home: &Path, args: &[&str]) -> Output {
      Command::new(env!("CARGO_BIN_EXE_sksync"))
          .current_dir(root)
          .env("HOME", home)
          .env("USERPROFILE", home)
          .args(args)
          .output()
          .expect("run sksync")
  }
  ```

  Add tests with these exact assertions:

  1. `bundle_export_root_writes_manifest_without_touching_project_state` creates `skills/local/SKILL.md`, configures `local` with source `./skills/local`, creates sentinel content in `sksync-lock.json` and `.sksync/skills/review/SKILL.md`, runs `bundle export team-baseline --root`, then asserts:

     ```rust
     assert_eq!(manifest["entries"]["local"]["source"], "./skills/local");
     assert_eq!(fs::read_to_string(root.join("sksync-lock.json")).unwrap(), "lock sentinel");
     assert_eq!(fs::read_to_string(root.join("skills/local/SKILL.md")).unwrap(), local_skill_before);
     assert!(root.join(".sksync/skills/review/SKILL.md").is_file());
     ```

  2. `bundle_export_root_requires_force_to_replace_manifest` writes an old root `sksync.bundle.json`, asserts `--root` fails with `already exists`, then runs `--root --force` and asserts the generated manifest name is `team-baseline`; retain sentinels for config, lockfile, `.sksync`, and `skills/` before both invocations and assert each is unchanged.

  3. `bundle_export_root_dry_run_does_not_write_manifest` runs `--root --dry-run` and asserts `!root.join("sksync.bundle.json").exists()`.

  4. `bundle_export_root_rejects_snapshot_and_output` invokes `--root --snapshot` and `--root --output ./bundle-out`; assert both fail and neither root manifest nor `bundle-out` exists.

  5. `bundle_export_global_root_writes_manifest_under_isolated_home` creates `home/.sksync/config.json` containing a manifest-only remote dependency, runs from a separate temporary working directory through `sksync_with_home(workdir, home, &["bundle", "export", "team-baseline", "--global", "--root"])`, then asserts `home/.sksync/sksync.bundle.json` exists and contains `team-baseline`. Do not use the real home directory.

- [ ] **Step 2: Run the new CLI tests and verify they fail**

  Run:

  ```sh
  cargo test --quiet --test bundle_cli bundle_export_root
  cargo test --quiet --test bundle_cli bundle_export_global_root_writes_manifest_under_isolated_home
  ```

  Expected: the CLI rejects `--root` as an unknown argument.

- [ ] **Step 3: Make `--output` optional only when `--root` is supplied**

  Change `BundleExportArgs` to use clap constraints:

  ```rust
  /// Output directory that will contain sksync.bundle.json.
  #[arg(long, required_unless_present = "root", conflicts_with = "root")]
  output: Option<PathBuf>,

  /// Write only sksync.bundle.json into the active configuration root.
  #[arg(long, conflicts_with_all = ["output", "snapshot"])]
  root: bool,
  ```

  Update the `--force` help text to say it replaces an existing generated output directory **or root manifest**, as appropriate.

- [ ] **Step 4: Select the correct destination and preserve directory safety checks**

  In `run_bundle_export`, derive the destination before building the plan:

  ```rust
  let destination = if args.root {
      BundleExportDestination::ManifestFile(root_dir.join(BUNDLE_MANIFEST_FILE))
  } else {
      BundleExportDestination::Directory(resolve_export_output_path(
          args.output.as_deref().expect("clap requires --output without --root"),
          &root_dir,
      ))
  };
  ```

  Pass `destination` to `BundleExportPlanInput`. Run `validate_bundle_export_output_safety` only for `BundleExportDestination::Directory(output)` so that a root manifest file is allowed while directory outputs continue to reject the config root, config file, lockfile, and skill directory.

  Replace all `plan.output` display uses with `plan.destination.path()`, including `print_bundle_export_plan` and the final success message.

- [ ] **Step 5: Run focused integration tests and the full bundle suite**

  Run:

  ```sh
  cargo fmt --check
  cargo test --quiet --test bundle_cli
  ```

  Expected: every existing bundle CLI test and the five root/global tests pass.

- [ ] **Step 6: Commit the CLI and test change**

  Run `but diff`, select only `src/cli.rs` and `tests/bundle_cli.rs`, then commit:

  ```sh
  but commit root-bundle-export -m "feat: export bundle manifests at config roots" --changes <cli-change-id>,<bundle-cli-test-change-id>
  ```

  Expected: GitButler reports the commit; the separate documentation change remains uncommitted for the next task.

## Task 3: Document root bundles and run repository verification

**Files:**

- Modify: `site/guides/bundles.md:Exporting a bundle from existing dependencies`
- Modify: `site/guides/project-config.md:Generated files and .gitignore`
- Modify: `site/reference/commands.md:64-100`

**Interfaces:**

- Consumes: the delivered `--root` and `--global --root` CLI behavior.
- Produces: consistent command reference, root bundle usage examples, and an explicit version-control policy for local `skills/` sources.

- [ ] **Step 1: Add documentation assertions to the implementation review checklist**

  Before editing prose, verify the final commands and constraints against the CLI tests:

  ```text
  sksync bundle export team-baseline --root
  sksync bundle export team-baseline --global --root
  sksync bundle export team-baseline --root --force
  ```

  Confirm from the tests that `--root --snapshot` and `--root --output` fail, and that root export leaves `skills/` unchanged.

- [ ] **Step 2: Update the bundle guide**

  In `site/guides/bundles.md`, add a root-export example before the existing dedicated-directory examples:

  ```sh
  # Write only ./sksync.bundle.json from the current project config.
  sksync bundle export team-baseline --root

  # Replace an existing root manifest, without touching config, lockfile, or skills/.
  sksync bundle export team-baseline --root --force
  ```

  State that root export is manifest-only; `--snapshot` requires `--output <directory>`. Add the approved local-bundle layout, explain that `./skills/<name>` entries resolve from the bundle manifest's parent directory, and state that a consumer adding the remote bundle receives content from the bundle repository's `skills/<name>`, not its own project-relative `skills/` directory.

- [ ] **Step 3: Correct project configuration ignore guidance**

  Replace the unconditional `skills/ # legacy generated skill store from older defaults` line in `site/guides/project-config.md` with prose that keeps `.sksync/` and `sksync-lock.json` as generated/local-state guidance but says:

  ```text
  Do not ignore `skills/` when it contains local skill sources that a root bundle publishes. Commit those sources with `sksync.bundle.json`. Only ignore `skills/` in an older project where it is still a generated skill store.
  ```

  Preserve the existing current default `./.sksync/skills` explanation so users do not confuse local sources with managed installed bodies.

- [ ] **Step 4: Update the command reference**

  Add root syntax beside current export syntax:

  ```text
  sksync bundle export <name> --root
  sksync bundle export <name> --global --root
  ```

  Change the `--output` row to say it is required unless `--root` is used. Add a `--root` row describing project/global manifest targets and manifest-only behavior. Update the `--snapshot` row to say it conflicts with `--root`, and update the export portion of the `--force` row to say it replaces either the generated output directory or the root `sksync.bundle.json`, never sibling configuration state.

- [ ] **Step 5: Run documentation and full code verification**

  Run:

  ```sh
  cargo fmt --check
  cargo test --quiet
  cargo build --release --quiet
  cargo clippy --quiet -- -D warnings
  ```

  Then inspect the documentation diff with:

  ```sh
  but diff
  ```

  Expected: all Rust checks pass; the diff contains only the three documentation files for this task and accurately states the tested CLI behavior.

- [ ] **Step 6: Commit documentation**

  Commit the documentation files selected from `but diff`:

  ```sh
  but commit root-bundle-export -m "docs: explain root bundle exports" --changes <bundles-guide-change-id>,<project-config-guide-change-id>,<command-reference-change-id>
  ```

  Expected: GitButler reports the commit and no unassigned changes remain for this feature.
