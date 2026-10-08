use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Output};

fn sksync(root: &Path, home: &Path, args: &[&str]) -> Output {
    fs::create_dir_all(home.join(".config")).unwrap();
    Command::new(env!("CARGO_BIN_EXE_sksync"))
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .args(args)
        .output()
        .unwrap()
}

struct Busy(File);

impl Drop for Busy {
    fn drop(&mut self) {
        // Parallel Command::spawn can briefly inherit the open-file description
        // before exec. Release explicitly, as production WriteGuard does.
        self.0.unlock().unwrap();
    }
}

fn busy(directory: &Path) -> Busy {
    let handle = File::open(directory).unwrap();
    handle.try_lock().unwrap();
    Busy(handle)
}

fn assert_busy(output: Output) {
    assert!(!output.status.success(), "unexpected success: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("writer guard"),
        "expected writer guard failure: {output:?}"
    );
}

fn write_config(root: &Path, store: &Path) {
    fs::write(
        root.join("sksync.config.json"),
        serde_json::to_vec(&serde_json::json!({
            "skillDir": store,
            "dependencies": {},
            "unknown": 42
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn writer_guard_busy_state_parent_blocks_mutators_and_releases_after_error() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let home = temp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    let store = root.join("skills");
    fs::create_dir(&store).unwrap();
    write_config(&root, &store);
    let original = fs::read(root.join("sksync.config.json")).unwrap();
    let guard = busy(&root);
    for args in [
        vec!["update"],
        vec!["install"],
        vec!["apply"],
        vec!["remove", "review"],
        vec!["attach", "review", "--agent", "pi"],
    ] {
        assert_busy(sksync(&root, &home, &args));
        assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), original);
        assert!(!root.join("sksync-lock.json").exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0);
    }
    drop(guard);
    // A failed mutation must not retain any earlier acquired guard.
    let output = sksync(&root, &home, &["attach", "missing", "--agent", "pi"]);
    assert!(!output.status.success());
    let _released = busy(&root);
    let _store_released = busy(&store);
}

#[test]
fn writer_guard_shared_store_blocks_two_project_mutations_before_copy() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("shared");
    let home = temp.path().join("home");
    fs::create_dir(&store).unwrap();
    let _guard = busy(&store);
    for name in ["one", "two"] {
        let root = temp.path().join(name);
        fs::create_dir(&root).unwrap();
        write_config(&root, &store);
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\nold",
        )
        .unwrap();
        let before = fs::read(root.join("sksync.config.json")).unwrap();
        assert_busy(sksync(&root, &home, &["add", "./source", "--agent", "pi"]));
        assert_busy(sksync(
            &root,
            &home,
            &["import", "./source", "--agent", "pi"],
        ));
        assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), before);
        assert!(!root.join("sksync-lock.json").exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0);
    }
}

#[test]
fn writer_guard_state_alias_locks_referent_parent_for_config_and_lockfile() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let physical = temp.path().join("dotfiles");
    let home = temp.path().join("home");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&physical).unwrap();
    let store = root.join("skills");
    fs::create_dir(&store).unwrap();
    write_config(&physical, &store);
    let original = fs::read(physical.join("sksync.config.json")).unwrap();
    symlink(
        physical.join("sksync.config.json"),
        root.join("sksync.config.json"),
    )
    .unwrap();
    let guard = busy(&physical);
    assert_busy(sksync(&root, &home, &["update"]));
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), original);
    drop(guard);
    fs::remove_file(root.join("sksync.config.json")).unwrap();
    write_config(&root, &store);
    fs::write(physical.join("lock.json"), br#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"unix:1","root":".","skills":{}}"#).unwrap();
    symlink(physical.join("lock.json"), root.join("sksync-lock.json")).unwrap();
    let _guard = busy(&physical);
    assert_busy(sksync(&root, &home, &["apply"]));
    assert_eq!(fs::read(physical.join("lock.json")).unwrap(), br#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"unix:1","root":".","skills":{}}"#);
}

#[test]
fn writer_guard_init_and_agents_refresh_respect_busy_parents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let home = temp.path().join("home");
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(home.join(".sksync")).unwrap();
    let _project = busy(&root);
    let _global = busy(&home.join(".sksync"));
    for args in [
        vec!["init"],
        vec!["init", "--global"],
        vec!["init", "--agents"],
        vec!["agents", "refresh"],
    ] {
        assert_busy(sksync(&root, &home, &args));
    }
    assert!(!root.join("sksync.config.json").exists());
    assert!(!home.join(".sksync/config.json").exists());
    assert!(!home.join(".sksync/agents.json").exists());
}

#[test]
fn writer_guard_read_commands_and_dry_runs_do_not_lock_or_create_store() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let home = temp.path().join("home");
    fs::create_dir(&root).unwrap();
    let missing = root.join("missing-store");
    write_config(&root, &missing);
    fs::write(root.join("sksync-lock.json"), br#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"unix:1","root":".","skills":{}}"#).unwrap();
    fs::write(root.join("sksync.config.json"), serde_json::to_vec(&serde_json::json!({"skillDir": missing, "dependencies": {"review": {"source": "./source", "agents": ["pi"]}}})).unwrap()).unwrap();
    let _guard = busy(&root);
    for args in [
        vec!["list"],
        vec!["plan"],
        vec!["outdated"],
        vec![
            "bundle",
            "export",
            "empty",
            "--output",
            "./export",
            "--dry-run",
        ],
    ] {
        let output = sksync(&root, &home, &args);
        assert!(output.status.success(), "{output:?}");
    }
    assert!(!missing.exists());
    assert!(!root.join("export").exists());
}

#[test]
fn writer_guard_bundle_export_locks_only_output_parent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let output_parent = temp.path().join("exports");
    let home = temp.path().join("home");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&output_parent).unwrap();
    let store = root.join("skills");
    fs::create_dir(&store).unwrap();
    write_config(&root, &store);
    fs::write(root.join("sksync.config.json"), serde_json::to_vec(&serde_json::json!({"skillDir": store, "dependencies": {"review": {"source": "./source", "agents": ["pi"]}}})).unwrap()).unwrap();
    let output_path = output_parent.join("bundle");
    let args = [
        "bundle",
        "export",
        "empty",
        "--output",
        output_path.to_str().unwrap(),
    ];
    let guard = busy(&output_parent);
    assert_busy(sksync(&root, &home, &args));
    assert!(!output_path.exists());
    drop(guard);
    let _source_parent = busy(&root);
    let _source_store = busy(&store);
    let output = sksync(&root, &home, &args);
    assert!(output.status.success(), "{output:?}");
    assert!(output_path.join("sksync.bundle.json").exists());
}

#[test]
fn writer_guard_bundle_mutators_share_state_and_store_convention() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let home = temp.path().join("home");
    let store = root.join("skills");
    fs::create_dir_all(&store).unwrap();
    write_config(&root, &store);
    let original = fs::read(root.join("sksync.config.json")).unwrap();
    fs::create_dir(root.join("bundle")).unwrap();
    fs::write(
        root.join("bundle/sksync.bundle.json"),
        br#"{
        "name":"workflow", "description":"Local workflow",
        "entries":{"review":{"source":"../source"}}
    }"#,
    )
    .unwrap();
    for resource in [&root, &store] {
        let _guard = busy(resource);
        for args in [
            vec!["bundle", "add", "./bundle", "--agent", "pi"],
            vec!["bundle", "remove", "workflow"],
            vec!["bundle", "sync", "workflow"],
        ] {
            assert_busy(sksync(&root, &home, &args));
            assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), original);
            assert!(!root.join("sksync-lock.json").exists());
            assert_eq!(fs::read_dir(&store).unwrap().count(), 0);
        }
    }
}

#[test]
fn writer_guard_global_mutation_uses_global_state_not_project() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let home = temp.path().join("home");
    let global = home.join(".sksync");
    let store = global.join("skills");
    fs::create_dir(&project).unwrap();
    fs::create_dir_all(&store).unwrap();
    write_config(&global, &store);
    fs::rename(
        global.join("sksync.config.json"),
        global.join("config.json"),
    )
    .unwrap();
    let _guard = busy(&global);
    assert_busy(sksync(&project, &home, &["update", "--global"]));
    assert!(!project.join("sksync.config.json").exists());
    assert!(!project.join(".sksync").exists());
    assert!(!global.join("sksync-lock.json").exists());
}

#[test]
fn writer_guard_init_existing_config_does_not_create_unused_store() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let home = temp.path().join("home");
    fs::create_dir(&root).unwrap();
    write_config(&root, &root.join("custom-store"));
    let before = fs::read(root.join("sksync.config.json")).unwrap();
    let output = sksync(&root, &home, &["init"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("config already exists"));
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), before);
    assert!(!root.join(".sksync").exists());
    assert!(!root.join("custom-store").exists());
}
