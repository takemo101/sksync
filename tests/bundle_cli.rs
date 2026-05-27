use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sksync(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sksync"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("run sksync")
}

fn assert_success(output: Output) {
    assert!(
        output.status.success(),
        "expected success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: Output) -> String {
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_bundle_e2e_config(root: &Path) {
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "agents": {
            "universal": { "scope": "project", "targetDir": ".agents/skills" }
          },
          "dependencies": {}
        }"#,
    )
    .expect("write project config");
}

fn write_bundle_skill(root: &Path, relative: &str, name: &str) {
    let skill_dir = root.join(relative);
    fs::create_dir_all(&skill_dir).expect("create skill dir");
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {name} skill\n---\n# {name}\n"),
    )
    .expect("write skill manifest");
}

fn write_review_bundle(root: &Path) {
    fs::create_dir_all(root.join("bundle")).expect("create bundle");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" }
          }
        }"#,
    )
    .expect("write bundle manifest");
    write_bundle_skill(root, "bundle/skills/review", "review");
    write_bundle_skill(root, "bundle/skills/qa", "qa");
}

fn read_project_config(root: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(root.join("sksync.config.json")).unwrap())
        .expect("project config json")
}

fn write_export_project(root: &Path) {
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "dependencies": {
            "review": { "source": "github:org/repo/skills/review#main", "agents": ["pi"] },
            "qa": { "source": "./vendor/qa", "agents": ["pi"] }
          }
        }"#,
    )
    .expect("write config");
    write_bundle_skill(root, ".sksync/skills/review", "review");
    write_bundle_skill(root, ".sksync/skills/qa", "qa");
}

#[test]
fn bundle_add_and_remove_flow_manages_dependencies_files_and_symlinks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));

    let config = read_project_config(root);
    assert_eq!(config["dependencies"]["review"]["managedByBundles"], true);
    assert_eq!(config["dependencies"]["qa"]["managedByBundles"], true);
    assert!(root.join(".sksync/skills/review/SKILL.md").is_file());
    assert!(root.join(".sksync/skills/qa/SKILL.md").is_file());
    assert!(fs::symlink_metadata(root.join(".agents/skills/review"))
        .expect("review link")
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(root.join(".agents/skills/qa"))
        .expect("qa link")
        .file_type()
        .is_symlink());

    assert_success(sksync(root, &["bundle", "remove", "review-workflow"]));

    let config = read_project_config(root);
    assert!(config["dependencies"].as_object().unwrap().is_empty());
    assert!(!root.join(".sksync/skills/review").exists());
    assert!(!root.join(".sksync/skills/qa").exists());
    assert!(!root.join(".agents/skills/review").exists());
    assert!(!root.join(".agents/skills/qa").exists());
}

#[test]
fn bundle_sync_dry_run_reports_new_manifest_entry_without_writing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    write_bundle_skill(root, "bundle/skills/lint", "lint");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" },
            "lint": { "source": "./skills/lint" }
          }
        }"#,
    )
    .expect("write updated bundle manifest");
    let config_before = fs::read_to_string(root.join("sksync.config.json")).unwrap();
    let lock_before = fs::read_to_string(root.join("sksync-lock.json")).unwrap();

    let output = sksync(root, &["bundle", "sync", "review-workflow", "--dry-run"]);
    assert!(
        output.status.success(),
        "expected success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    assert!(stdout.contains("add lint"), "stdout: {stdout}");
    assert!(stdout.contains("keep: 2"), "stdout: {stdout}");
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read_to_string(root.join("sksync-lock.json")).unwrap(),
        lock_before
    );
    assert!(!root.join(".sksync/skills/lint").exists());
    assert!(!root.join(".agents/skills/lint").exists());
}

#[test]
fn bundle_sync_dry_run_prints_progress_to_stderr() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));

    let output = sksync(root, &["bundle", "sync", "review-workflow", "--dry-run"]);
    assert!(
        output.status.success(),
        "expected success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.contains("Bundle sync plan"), "stdout: {stdout}");
    assert!(
        stderr.contains("Loading bundle manifest"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("Planning changes"), "stderr: {stderr}");
}

#[test]
fn bundle_sync_adds_new_bundle_entry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    write_bundle_skill(root, "bundle/skills/lint", "lint");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" },
            "lint": { "source": "./skills/lint" }
          }
        }"#,
    )
    .expect("write updated bundle manifest");

    assert_success(sksync(root, &["bundle", "sync", "review-workflow"]));

    let config = read_project_config(root);
    assert_eq!(config["dependencies"]["lint"]["managedByBundles"], true);
    assert_eq!(
        config["dependencies"]["lint"]["bundles"][0]["name"],
        "review-workflow"
    );
    assert_eq!(
        config["dependencies"]["lint"]["agents"],
        serde_json::json!(["universal"])
    );
    assert!(root.join(".sksync/skills/lint/SKILL.md").is_file());
    assert!(fs::symlink_metadata(root.join(".agents/skills/lint"))
        .expect("lint link")
        .file_type()
        .is_symlink());
}

#[test]
fn bundle_sync_adopts_manual_same_source_entry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    fs::create_dir_all(root.join("bundle")).expect("create bundle");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" }
          }
        }"#,
    )
    .expect("write initial bundle manifest");
    write_bundle_skill(root, "bundle/skills/review", "review");
    write_bundle_skill(root, "bundle/skills/qa", "qa");

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    let mut config = read_project_config(root);
    config["dependencies"].as_object_mut().unwrap().insert(
        "qa".to_owned(),
        serde_json::json!({
            "source": "./bundle/skills/qa",
            "agents": ["universal"]
        }),
    );
    fs::write(
        root.join("sksync.config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("write config with manual qa");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" }
          }
        }"#,
    )
    .expect("write updated bundle manifest");

    assert_success(sksync(root, &["bundle", "sync", "review-workflow"]));

    let config = read_project_config(root);
    assert_eq!(
        config["dependencies"]["qa"]["managedByBundles"],
        serde_json::Value::Null
    );
    assert_eq!(
        config["dependencies"]["qa"]["bundles"][0]["name"],
        "review-workflow"
    );
    assert_eq!(
        config["dependencies"]["qa"]["agents"],
        serde_json::json!(["universal"])
    );
}

#[test]
fn bundle_sync_removes_deleted_bundle_managed_entry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" }
          }
        }"#,
    )
    .expect("write updated bundle manifest");

    assert_success(sksync(root, &["bundle", "sync", "review-workflow"]));

    let config = read_project_config(root);
    assert!(config["dependencies"].get("qa").is_none());
    assert!(!root.join(".sksync/skills/qa").exists());
    assert!(!root.join(".agents/skills/qa").exists());
}

#[test]
fn bundle_sync_detaches_deleted_manual_adopted_entry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    fs::create_dir_all(root.join("bundle")).expect("create bundle");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" }
          }
        }"#,
    )
    .expect("write initial bundle manifest");
    write_bundle_skill(root, "bundle/skills/review", "review");
    write_bundle_skill(root, "bundle/skills/qa", "qa");

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    let mut config = read_project_config(root);
    config["dependencies"].as_object_mut().unwrap().insert(
        "qa".to_owned(),
        serde_json::json!({
            "source": "./bundle/skills/qa",
            "agents": ["universal"]
        }),
    );
    fs::write(
        root.join("sksync.config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("write config with manual qa");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" }
          }
        }"#,
    )
    .expect("write manifest with qa");
    assert_success(sksync(root, &["bundle", "sync", "review-workflow"]));
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" }
          }
        }"#,
    )
    .expect("write manifest without qa");

    assert_success(sksync(root, &["bundle", "sync", "review-workflow"]));

    let config = read_project_config(root);
    assert!(config["dependencies"]["qa"].get("bundles").is_none());
    assert_eq!(
        config["dependencies"]["qa"]["managedByBundles"],
        serde_json::Value::Null
    );
    assert!(root.join(".sksync/skills/qa/SKILL.md").is_file());
    assert!(fs::symlink_metadata(root.join(".agents/skills/qa"))
        .expect("qa link")
        .file_type()
        .is_symlink());
}

#[test]
fn bundle_sync_requires_source_when_name_matches_multiple_sources() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "dependencies": {
            "one": {
              "source": "./one",
              "agents": ["pi"],
              "bundles": [{ "name": "review-workflow", "source": "./bundle-a" }],
              "managedByBundles": true
            },
            "two": {
              "source": "./two",
              "agents": ["pi"],
              "bundles": [{ "name": "review-workflow", "source": "./bundle-b" }],
              "managedByBundles": true
            }
          }
        }"#,
    )
    .expect("write config");

    let error = assert_failure(sksync(
        root,
        &["bundle", "sync", "review-workflow", "--dry-run"],
    ));

    assert!(error.contains("ambiguous"), "error: {error}");
    assert!(error.contains("--source <exact-source>"), "error: {error}");
}

#[test]
fn bundle_sync_reports_not_found_without_local_provenance() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);

    let error = assert_failure(sksync(
        root,
        &["bundle", "sync", "missing-workflow", "--dry-run"],
    ));

    assert!(
        error.contains("bundle provenance not found"),
        "error: {error}"
    );
}

#[test]
fn bundle_sync_aborts_when_manifest_name_changed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "renamed-workflow",
          "description": "Review workflow skills.",
          "entries": {
            "review": { "source": "./skills/review" },
            "qa": { "source": "./skills/qa" }
          }
        }"#,
    )
    .expect("write renamed manifest");

    let error = assert_failure(sksync(
        root,
        &["bundle", "sync", "review-workflow", "--dry-run"],
    ));

    assert!(error.contains("manifest name changed"), "error: {error}");
}

#[test]
fn bundle_add_conflict_leaves_config_and_files_unchanged() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "agents": {
            "universal": { "scope": "project", "targetDir": ".agents/skills" }
          },
          "dependencies": {
            "review": { "source": "./different-review", "agents": ["universal"] }
          }
        }"#,
    )
    .expect("write conflict config");
    let before = fs::read_to_string(root.join("sksync.config.json")).unwrap();

    let error = assert_failure(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));

    assert!(error.contains("conflict"));
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        before
    );
    assert!(!root.join(".sksync/skills/review").exists());
    assert!(!root.join(".sksync/skills/qa").exists());
    assert!(!root.join(".agents/skills/review").exists());
    assert!(!root.join(".agents/skills/qa").exists());
}

#[test]
fn bundle_add_adopts_manual_same_source_without_bundle_managing_it() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    fs::create_dir_all(root.join("bundle")).expect("create bundle");
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        r#"{
          "name": "review-workflow",
          "description": "Review workflow skills.",
          "entries": { "review": { "source": "./skills/review" } }
        }"#,
    )
    .expect("write bundle manifest");
    write_bundle_skill(root, "bundle/skills/review", "review");
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "agents": {
            "universal": { "scope": "project", "targetDir": ".agents/skills" }
          },
          "dependencies": {
            "review": { "source": "./bundle/skills/review", "agents": ["universal"] }
          }
        }"#,
    )
    .expect("write manual dependency config");

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "pi"],
    ));
    assert_success(sksync(root, &["bundle", "remove", "review-workflow"]));

    let config = read_project_config(root);
    let review = &config["dependencies"]["review"];
    assert_eq!(review["source"], "./bundle/skills/review");
    assert_eq!(review["managedByBundles"], serde_json::Value::Null);
    assert_eq!(review["bundles"], serde_json::Value::Null);
    assert!(root.join(".sksync/skills/review/SKILL.md").is_file());
}

#[test]
fn add_preserves_bundle_provenance_for_existing_same_source_dependency() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(
        root,
        &["bundle", "add", "./bundle", "--agent", "universal"],
    ));
    assert_success(sksync(
        root,
        &["add", "./bundle/skills/review", "--agent", "pi"],
    ));

    let config = read_project_config(root);
    let review = &config["dependencies"]["review"];
    assert_eq!(review["source"], "./bundle/skills/review");
    assert_eq!(review["managedByBundles"], true);
    assert_eq!(
        review["bundles"],
        serde_json::json!([{ "name": "review-workflow", "source": "./bundle" }])
    );
    assert_eq!(review["agents"], serde_json::json!(["universal", "pi"]));
}

#[test]
fn bundle_export_writes_manifest_only_bundle() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
        ],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["name"], "team-baseline");
    assert_eq!(
        manifest["entries"]["review"]["source"],
        "github:org/repo/skills/review#main"
    );
    assert_eq!(manifest["entries"]["qa"]["source"], "./vendor/qa");
    assert!(!root.join("bundle-out/skills").exists());
}

#[test]
fn bundle_export_snapshot_copies_installed_skill_bodies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
            "--snapshot",
        ],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["entries"]["review"]["source"], "./skills/review");
    assert_eq!(manifest["entries"]["qa"]["source"], "./skills/qa");
    assert!(root.join("bundle-out/skills/review/SKILL.md").is_file());
    assert!(root.join("bundle-out/skills/qa/SKILL.md").is_file());
}

#[test]
fn bundle_export_dry_run_does_not_create_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
            "--dry-run",
        ],
    ));

    assert!(!root.join("bundle-out").exists());
}

#[test]
fn bundle_export_skill_filter_exports_only_selected_dependency() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
            "--skill",
            "qa",
        ],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert!(manifest["entries"].get("qa").is_some());
    assert!(manifest["entries"].get("review").is_none());
}

#[test]
fn bundle_export_refuses_existing_output_without_force() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);
    fs::create_dir_all(root.join("bundle-out")).unwrap();

    let error = assert_failure(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
        ],
    ));

    assert!(error.contains("already exists"));
}

#[test]
fn bundle_export_snapshot_dry_run_validates_installed_skill_bodies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);
    fs::write(
        root.join(".sksync/skills/review/SKILL.md"),
        "# Missing frontmatter\n",
    )
    .expect("break installed skill");

    let error = assert_failure(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            "./bundle-out",
            "--snapshot",
            "--dry-run",
        ],
    ));

    assert!(error.contains("invalid SKILL.md"));
    assert!(!root.join("bundle-out").exists());
}

#[test]
fn bundle_export_rejects_output_that_overlaps_project_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);
    let before = fs::read_to_string(root.join("sksync.config.json")).unwrap();

    let error = assert_failure(sksync(
        root,
        &[
            "bundle",
            "export",
            "team-baseline",
            "--output",
            ".",
            "--force",
        ],
    ));

    assert!(error.contains("active config root") || error.contains("protected sksync state"));
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        before
    );
    assert!(root.join(".sksync/skills/review/SKILL.md").is_file());
}
