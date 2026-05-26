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
