use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn test_command(root: &Path, program: &str) -> Command {
    let home = root.join("home");
    let config_home = home.join(".config");
    fs::create_dir_all(&config_home).unwrap();
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", config_home);
    command
}

fn sksync(root: &Path, args: &[&str]) -> Output {
    test_command(root, env!("CARGO_BIN_EXE_sksync"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_source(root: &Path, name: &str, body: &str) {
    let source = root.join("sources").join(name);
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\n---\n{body}\n"),
    )
    .unwrap();
    fs::write(source.join("extra.txt"), body).unwrap();
}

fn fixture(root: &Path) -> Vec<PathBuf> {
    for name in ["alpha", "beta"] {
        write_source(root, name, "old");
    }
    fs::write(root.join("sksync.config.json"), r#"{
        "skillDir": "./.sksync/skills",
        "agents": { "universal": { "scope": "project", "targetDir": ".agents/skills" } },
        "dependencies": {
            "alpha": { "source": "./sources/alpha", "agents": ["universal"], "include": ["SKILL.md"], "managedByBundles": true, "bundles": [{"name":"test", "source":"./bundle"}] },
            "beta": { "source": "./sources/beta", "agents": ["universal"] }
        }
    }"#).unwrap();
    assert_success(&sksync(root, &["install"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    ["alpha", "beta"]
        .iter()
        .map(|name| root.join(lock["skills"][name]["source"].as_str().unwrap()))
        .collect()
}

#[test]
fn full_update_second_prepare_failure_preserves_bodies_lock_config_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let old_bodies: Vec<_> = bodies
        .iter()
        .map(|path| fs::read(path.join("SKILL.md")).unwrap())
        .collect();
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    let old_config = fs::read(root.join("sksync.config.json")).unwrap();
    let links: Vec<_> = ["alpha", "beta"]
        .iter()
        .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap())
        .collect();
    write_source(root, "alpha", "new");
    fs::write(root.join("sources/beta/SKILL.md"), "invalid package").unwrap();
    let output = sksync(root, &["update"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
    for (path, old) in bodies.iter().zip(old_bodies) {
        assert_eq!(fs::read(path.join("SKILL.md")).unwrap(), old);
    }
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
    assert_eq!(
        fs::read(root.join("sksync.config.json")).unwrap(),
        old_config
    );
    for (name, link) in ["alpha", "beta"].iter().zip(links) {
        assert_eq!(
            fs::read_link(root.join(".agents/skills").join(name)).unwrap(),
            link
        );
    }
}

#[test]
fn full_update_success_preserves_config_links_layout_and_portable_filtered_content() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let old_config = fs::read(root.join("sksync.config.json")).unwrap();
    let links: Vec<_> = ["alpha", "beta"]
        .iter()
        .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap())
        .collect();
    for name in ["alpha", "beta"] {
        write_source(root, name, "new");
    }
    fs::write(bodies[0].join("SKILL.md"), "local edits").unwrap();
    assert_success(&sksync(root, &["update"]));
    assert_eq!(
        fs::read(root.join("sksync.config.json")).unwrap(),
        old_config
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["lockfileVersion"], 5);
    assert_eq!(lock["root"], ".");
    for ((name, path), link) in ["alpha", "beta"].iter().zip(&bodies).zip(links) {
        assert_eq!(
            root.join(lock["skills"][name]["source"].as_str().unwrap()),
            *path
        );
        assert!(fs::read_to_string(path.join("SKILL.md"))
            .unwrap()
            .contains("new"));
        assert!(lock["skills"][name].get("targets").is_none());
        assert_eq!(
            fs::read_link(root.join(".agents/skills").join(name)).unwrap(),
            link
        );
    }
    assert!(!bodies[0].join("extra.txt").exists());
    assert_eq!(
        lock["skills"]["alpha"]["include"],
        serde_json::json!(["SKILL.md"])
    );
    assert_eq!(
        lock["skills"]["alpha"]["files"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn full_update_empty_dependency_selection_does_not_write_lockfile() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("sksync.config.json"),
        r#"{"skillDir":"./.sksync/skills","dependencies":{}}"#,
    )
    .unwrap();
    for old in [None, Some(b"old raw lock bytes".as_slice())] {
        if let Some(old) = old {
            fs::write(root.join("sksync-lock.json"), old).unwrap();
        }
        let output = sksync(root, &["update"]);
        assert_success(&output);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
        assert_eq!(fs::read(root.join("sksync-lock.json")).ok().as_deref(), old);
    }
}

#[test]
fn full_update_legacy_hash_failure_precedes_body_publication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let old = fs::read(bodies[0].join("SKILL.md")).unwrap();
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    write_source(root, "alpha", "new");
    let config_path = root.join("sksync.config.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["skills"] =
        serde_json::json!({"legacy": {"source":"./missing-legacy", "agents":["universal"]}});
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let output = sksync(root, &["update"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
    assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old);
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
}

#[test]
fn full_update_builds_legacy_entries_without_resolving_agent_targets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    write_source(root, "alpha", "new");
    write_source(root, "legacy", "legacy content");
    let path = root.join("sksync.config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["agents"]["missing-target"] = serde_json::json!({"scope":"project"});
    config["dependencies"]["alpha"]["agents"] = serde_json::json!(["missing-target"]);
    config["skills"] =
        serde_json::json!({"legacy":{"source":"./sources/legacy","agents":["universal"]}});
    let config_bytes = serde_json::to_vec(&config).unwrap();
    fs::write(&path, &config_bytes).unwrap();
    let plan = sksync(root, &["plan"]);
    assert!(!plan.status.success());
    assert!(String::from_utf8_lossy(&plan.stderr).contains("targetDir"));
    let old_link = fs::read_link(root.join(".agents/skills/alpha")).unwrap();
    assert_success(&sksync(root, &["update"]));
    assert_eq!(fs::read(&path).unwrap(), config_bytes);
    assert_eq!(
        fs::read_link(root.join(".agents/skills/alpha")).unwrap(),
        old_link
    );
    assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
        .unwrap()
        .contains("new"));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["skills"]["legacy"]["source"], "sources/legacy");
    assert_eq!(
        lock["skills"]["legacy"]["files"].as_array().unwrap().len(),
        2
    );
}

fn add_legacy(root: &Path, name: &str, source: &Path) {
    let path = root.join("sksync.config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["skills"][name] = serde_json::json!({"source": source, "agents": ["universal"]});
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
}

#[test]
fn legacy_overlap_alias_uses_final_dependency_content_and_keeps_legacy_metadata() {
    use std::os::unix::fs::symlink;
    for physical_alias in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let source = if physical_alias {
            let alias = root.join("body-alias");
            symlink(&bodies[0], &alias).unwrap();
            alias
        } else {
            bodies[0].clone()
        };
        add_legacy(root, "legacy", &source);
        assert_success(&sksync(root, &["apply"]));
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_lock: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
        let old_link = fs::read_link(root.join(".agents/skills/legacy")).unwrap();
        write_source(root, "alpha", "new alias content");
        assert_success(&sksync(root, &["update"]));
        let lock: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
        assert_eq!(
            lock["skills"]["legacy"]["hash"],
            lock["skills"]["alpha"]["hash"]
        );
        assert_eq!(
            lock["skills"]["legacy"]["files"],
            lock["skills"]["alpha"]["files"]
        );
        for field in ["source", "installSource", "include"] {
            assert_eq!(
                lock["skills"]["legacy"][field],
                old_lock["skills"]["legacy"][field]
            );
        }
        assert_eq!(
            root.join(lock["skills"]["legacy"]["source"].as_str().unwrap()),
            source
        );
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(
            fs::read_link(root.join(".agents/skills/legacy")).unwrap(),
            old_link
        );
        assert_success(&sksync(root, &["check"]));
    }
}

#[test]
fn legacy_overlap_subtree_uses_final_relative_file_records() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let source_dir = root.join("sources/beta/docs");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("old.txt"), "old").unwrap();
    assert_success(&sksync(root, &["update"]));
    add_legacy(root, "legacy", &bodies[1].join("docs"));
    assert_success(&sksync(root, &["apply"]));
    fs::remove_file(source_dir.join("old.txt")).unwrap();
    fs::write(source_dir.join("new.txt"), "new subtree content").unwrap();
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["skills"]["legacy"]["files"].as_array().unwrap().len(),
        1
    );
    assert_eq!(lock["skills"]["legacy"]["files"][0]["path"], "new.txt");
    assert_success(&sksync(root, &["check"]));
}

#[test]
fn legacy_overlap_ancestor_fails_safely_before_publication() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let store_alias = root.join("store-alias");
    symlink(root.join(".sksync/skills"), &store_alias).unwrap();
    add_legacy(root, "legacy", &store_alias);
    let old_config = fs::read(root.join("sksync.config.json")).unwrap();
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    let old_bodies: Vec<_> = bodies
        .iter()
        .map(|path| fs::read(path.join("SKILL.md")).unwrap())
        .collect();
    write_source(root, "alpha", "new");
    let output = sksync(root, &["update"]);
    assert!(
        !output.status.success(),
        "ancestor hashing must not commit staging content"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("overlaps prepared dependency"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
    assert_eq!(
        fs::read(root.join("sksync.config.json")).unwrap(),
        old_config
    );
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
    for (path, old) in bodies.iter().zip(old_bodies) {
        assert_eq!(fs::read(path.join("SKILL.md")).unwrap(), old);
    }
    assert!(fs::read_dir(root.join(".sksync/skills"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".sksync-")));
}

#[test]
fn legacy_overlap_replaced_inner_symlink_uses_prepared_subtree() {
    use std::os::unix::fs::symlink;
    for source_alias in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let external = root.join("old-external-docs");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("old.txt"), "old external content").unwrap();
        symlink(&external, bodies[1].join("docs")).unwrap();
        let source = if source_alias {
            let alias = root.join("subtree-alias");
            symlink(bodies[1].join("docs"), &alias).unwrap();
            alias
        } else {
            bodies[1].join("docs")
        };
        add_legacy(root, "legacy", &source);
        assert_success(&sksync(root, &["apply"]));
        // apply canonicalizes this old inner symlink to the external directory.
        // Keep a fixture link to the logical body path so unchanged links remain healthy.
        let target = root.join(".agents/skills/legacy");
        fs::remove_file(&target).unwrap();
        symlink(&source, &target).unwrap();
        let old_link = fs::read_link(&target).unwrap();
        fs::create_dir(root.join("sources/beta/docs")).unwrap();
        fs::write(root.join("sources/beta/docs/new.txt"), "new content").unwrap();
        assert_success(&sksync(root, &["update"]));
        let lock: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
        assert_eq!(lock["skills"]["legacy"]["files"][0]["path"], "new.txt");
        assert_eq!(
            fs::read(external.join("old.txt")).unwrap(),
            b"old external content"
        );
        assert_eq!(fs::read_link(&target).unwrap(), old_link);
        assert_success(&sksync(root, &["check"]));
    }
}

#[test]
fn legacy_overlap_initially_absent_body_alias_uses_prepared_content() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    fs::remove_dir_all(&bodies[0]).unwrap();
    add_legacy(root, "legacy", &bodies[0]);
    write_source(root, "alpha", "new absent alias content");
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["skills"]["legacy"]["hash"],
        lock["skills"]["alpha"]["hash"]
    );
    assert_eq!(
        lock["skills"]["legacy"]["files"],
        lock["skills"]["alpha"]["files"]
    );
}

#[test]
fn legacy_overlap_same_inode_case_aliases_use_candidate_content_or_fail_ancestors_safely() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let alias = bodies[0].with_file_name("ALPHA");
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        return;
    };
    let actual = fs::metadata(&bodies[0]).unwrap();
    assert_eq!(
        (alias_metadata.dev(), alias_metadata.ino()),
        (actual.dev(), actual.ino())
    );
    eprintln!("same-inode case-alias regression exercised");
    add_legacy(root, "legacy", &alias);
    assert_success(&sksync(root, &["apply"]));
    write_source(root, "alpha", "new case alias content");
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["skills"]["legacy"]["hash"],
        lock["skills"]["alpha"]["hash"]
    );
    assert_eq!(
        lock["skills"]["legacy"]["files"],
        lock["skills"]["alpha"]["files"]
    );
    let ancestor = root.join(".sksync/SKILLS");
    add_legacy(root, "ancestor", &ancestor);
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    let old_body = fs::read(bodies[0].join("SKILL.md")).unwrap();
    write_source(root, "alpha", "must not publish");
    let output = sksync(root, &["update"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("overlaps prepared dependency"));
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
    assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_body);
}

#[test]
fn legacy_overlap_same_inode_unicode_alias_uses_candidate_content() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_source(root, "alpha", "old");
    let config_path = root.join("sksync.config.json");
    fs::write(
        &config_path,
        serde_json::to_vec(&serde_json::json!({
            "skillDir":"./.sksync/skills",
            "agents":{"universal":{"scope":"project","targetDir":".agents/skills"}},
            "dependencies":{"café":{"source":"./sources/alpha","agents":["universal"]}}
        }))
        .unwrap(),
    )
    .unwrap();
    assert_success(&sksync(root, &["install"]));
    let destination = root.join(".sksync/skills/café");
    let alias = root.join(".sksync/skills/cafe\u{301}");
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        return;
    };
    let actual = fs::metadata(&destination).unwrap();
    assert_eq!(
        (alias_metadata.dev(), alias_metadata.ino()),
        (actual.dev(), actual.ino())
    );
    eprintln!("same-inode Unicode-alias regression exercised");
    add_legacy(root, "legacy", &alias);
    assert_success(&sksync(root, &["apply"]));
    write_source(root, "alpha", "new Unicode alias content");
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["skills"]["legacy"]["hash"],
        lock["skills"]["café"]["hash"]
    );
    assert_eq!(
        lock["skills"]["legacy"]["files"],
        lock["skills"]["café"]["files"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn legacy_overlap_same_inode_volume_alias_uses_candidate_and_rejects_ancestor() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let canonical = bodies[0].canonicalize().unwrap();
    let alias = Path::new("/System/Volumes/Data").join(canonical.strip_prefix("/").unwrap());
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        return;
    };
    let actual = fs::metadata(&canonical).unwrap();
    assert_eq!(
        (alias_metadata.dev(), alias_metadata.ino()),
        (actual.dev(), actual.ino())
    );
    if alias.canonicalize().unwrap() == canonical {
        return;
    }
    eprintln!("same-inode distinct-canonical-volume-alias regression exercised");
    add_legacy(root, "legacy", &alias);
    assert_success(&sksync(root, &["apply"]));
    write_source(root, "alpha", "new volume alias content");
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["skills"]["legacy"]["hash"],
        lock["skills"]["alpha"]["hash"]
    );
    add_legacy(root, "ancestor", alias.parent().unwrap());
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    let old_body = fs::read(bodies[0].join("SKILL.md")).unwrap();
    write_source(root, "alpha", "must not publish");
    let output = sksync(root, &["update"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("overlaps prepared dependency"));
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
    assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_body);
}

#[test]
fn legacy_overlap_unprojectable_subtree_and_traversal_preserve_snapshots() {
    for traversal in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let docs = root.join("sources/beta/docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("old.txt"), "old subtree").unwrap();
        assert_success(&sksync(root, &["update"]));
        let source = if traversal {
            bodies[1].join("docs/..")
        } else {
            bodies[1].join("docs")
        };
        add_legacy(root, "legacy", &source);
        let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_bodies: Vec<_> = bodies
            .iter()
            .map(|path| fs::read(path.join("SKILL.md")).unwrap())
            .collect();
        let old_link = fs::read_link(root.join(".agents/skills/beta")).unwrap();
        fs::remove_file(docs.join("old.txt")).unwrap();
        write_source(root, "alpha", "must not publish");
        let output = sksync(root, &["update"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(if traversal {
                "unsupported traversal"
            } else {
                "no prepared file records"
            }),
            "{error}"
        );
        assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(
            fs::read(bodies[1].join("docs/old.txt")).unwrap(),
            b"old subtree"
        );
        assert_eq!(
            fs::read_link(root.join(".agents/skills/beta")).unwrap(),
            old_link
        );
        for (path, old) in bodies.iter().zip(old_bodies) {
            assert_eq!(fs::read(path.join("SKILL.md")).unwrap(), old);
        }
        assert!(fs::read_dir(root.join(".sksync/skills"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".sksync-")));
    }
}

#[test]
fn legacy_overlap_disjoint_private_looking_user_name_is_not_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let legacy = root.join(".sksync/skills/.sksync-staging-user");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("SKILL.md"), "valid user content").unwrap();
    fs::write(legacy.join(".sksync-backup-user"), "also user content").unwrap();
    add_legacy(root, "legacy", &legacy);
    assert_success(&sksync(root, &["apply"]));
    let old_lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    write_source(root, "alpha", "new");
    assert_success(&sksync(root, &["update"]));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["skills"]["legacy"], old_lock["skills"]["legacy"]);
    assert_eq!(
        lock["skills"]["legacy"]["files"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        fs::read(legacy.join(".sksync-backup-user")).unwrap(),
        b"also user content"
    );
    assert_success(&sksync(root, &["check"]));
}

#[test]
fn legacy_overlap_regular_file_root_is_rejected_before_commit_with_full_snapshots() {
    use std::os::unix::fs::symlink;
    fn snapshot(body: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        let mut entries = walkdir::WalkDir::new(body)
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                let bytes = entry
                    .file_type()
                    .is_file()
                    .then(|| fs::read(entry.path()).unwrap());
                (
                    entry.path().strip_prefix(body).unwrap().to_path_buf(),
                    bytes,
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }
    for alias in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let file_source = if alias {
            let path = root.join("legacy-file-source");
            symlink(bodies[0].join("SKILL.md"), &path).unwrap();
            path
        } else {
            bodies[0].join("SKILL.md")
        };
        add_legacy(root, "legacyfile", &file_source);
        let old_bodies: Vec<_> = bodies.iter().map(|body| snapshot(body)).collect();
        let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_links: Vec<_> = ["alpha", "beta"]
            .iter()
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap())
            .collect();
        write_source(root, "alpha", "must not publish file root");
        let output = sksync(root, &["update"]);
        assert!(
            !output.status.success(),
            "legacy regular-file source must fail before commit"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("must be a directory"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
        assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        for (body, old) in bodies.iter().zip(old_bodies) {
            assert_eq!(snapshot(body), old);
        }
        for (name, old) in ["alpha", "beta"].iter().zip(old_links) {
            assert_eq!(
                fs::read_link(root.join(".agents/skills").join(name)).unwrap(),
                old
            );
        }
        assert!(!root.join(".agents/skills/legacyfile").exists());
        assert!(fs::read_dir(root.join(".sksync/skills"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".sksync-")));
    }
}

fn nested_dependency_fixture_is_rejected(
    child_name: &str,
    existing: bool,
    missing_case_alias: bool,
) {
    use std::io::Write;
    use std::process::Stdio;

    fn tree_snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut entries = walkdir::WalkDir::new(path)
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                let value = if entry.file_type().is_dir() {
                    vec![b'D']
                } else if entry.file_type().is_symlink() {
                    let mut bytes = vec![b'L'];
                    bytes.extend(
                        fs::read_link(entry.path())
                            .unwrap()
                            .as_os_str()
                            .as_encoded_bytes(),
                    );
                    bytes
                } else {
                    let mut bytes = vec![b'F'];
                    bytes.extend(fs::read(entry.path()).unwrap());
                    bytes
                };
                (
                    entry.path().strip_prefix(path).unwrap().to_path_buf(),
                    value,
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = root.join("repo.git");
    assert_success(
        &test_command(root, "git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&repo)
            .output()
            .unwrap(),
    );
    let content = format!("---\nname: {child_name}\ndescription: Test skill\n---\nnew Git body\n");
    // Create only a local fixture commit, without modifying the workspace index/history.
    let input = format!(
        "commit refs/heads/main\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\ndata 7\nfixture\nM 100644 inline SKILL.md\ndata {}\n{}\n\n",
        content.len(), content
    );
    let mut git = test_command(root, "git")
        .arg("-C")
        .arg(&repo)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    git.stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert_success(&git.wait_with_output().unwrap());

    let original_prefix = PathBuf::from(
        repo.to_str()
            .unwrap()
            .trim_start_matches('/')
            .trim_end_matches(".git"),
    );
    let ancestor_name = original_prefix
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_str()
        .unwrap();
    let git_url = if missing_case_alias {
        let alias = PathBuf::from(format!("/{}", ancestor_name.to_uppercase()))
            .join(repo.strip_prefix(format!("/{ancestor_name}")).unwrap());
        if !alias.exists() {
            return;
        }
        eprintln!("nested dependency case-alias fixture exercised");
        alias
    } else {
        repo.clone()
    };
    let storage_prefix = PathBuf::from(
        git_url
            .to_str()
            .unwrap()
            .trim_start_matches('/')
            .trim_end_matches(".git"),
    );
    assert_ne!(ancestor_name, child_name);
    assert_eq!(child_name < ancestor_name, child_name == "alpha");
    write_source(root, ancestor_name, "old local body");
    let config_path = root.join("sksync.config.json");
    let mut config = serde_json::json!({
        "skillDir":"./.sksync/skills",
        "agents":{"universal":{"scope":"project","targetDir":".agents/skills"}},
        "dependencies": {ancestor_name: {"source":format!("./sources/{ancestor_name}"), "agents":["universal"]}}
    });
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    if existing {
        assert_success(&sksync(root, &["install"]));
    }
    config["dependencies"][child_name] = serde_json::json!({
        "source":{"provider":"git", "url":git_url, "path":"."}, "agents":["universal"]
    });
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let store = root.join(".sksync/skills");
    let ancestor = store.join(ancestor_name);
    let child = store.join(&storage_prefix).join(child_name);
    assert!(missing_case_alias || child.starts_with(&ancestor));
    if existing {
        fs::create_dir_all(&child).unwrap();
        fs::write(
            child.join("SKILL.md"),
            format!("---\nname: {child_name}\ndescription: Test skill\n---\nold Git body\n"),
        )
        .unwrap();
        fs::write(child.join("old-extra.txt"), "preserve this too").unwrap();
        assert_success(&sksync(root, &["apply"]));
        assert_success(&sksync(root, &["check"]));
    } else {
        // The writer guard may create the store, but neither selected body exists.
        fs::create_dir_all(&store).unwrap();
    }
    write_source(root, ancestor_name, "new local body");
    let old_tree = tree_snapshot(&store);
    let old_lock = fs::read(root.join("sksync-lock.json")).ok();
    let old_config = fs::read(&config_path).unwrap();
    let old_links = [ancestor_name, child_name]
        .map(|name| fs::read_link(root.join(".agents/skills").join(name)).ok());
    let output = sksync(root, &["update"]);
    eprintln!("nested dependencies: child={child_name}, existing={existing}, exit={:?}, child_exists={}, lock_preserved={}; stdout: {}; stderr: {}",
        output.status.code(), child.exists(), fs::read(root.join("sksync-lock.json")).ok() == old_lock,
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(
        !output.status.success(),
        "nested dependency destinations must be rejected before preparation"
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("dependency destinations overlap"), "{error}");
    for name in [ancestor_name, child_name] {
        assert!(error.contains(name), "{error}");
    }
    for path in [&ancestor, &child] {
        let physical = path.canonicalize().unwrap_or_else(|_| path.clone());
        assert!(
            error.contains(path.to_str().unwrap()) || error.contains(physical.to_str().unwrap()),
            "{error}"
        );
    }
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Wrote lockfile"));
    assert_eq!(tree_snapshot(&store), old_tree);
    assert_eq!(fs::read(root.join("sksync-lock.json")).ok(), old_lock);
    assert_eq!(fs::read(&config_path).unwrap(), old_config);
    assert_eq!(
        [ancestor_name, child_name]
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).ok()),
        old_links
    );
    assert!(!walkdir::WalkDir::new(&store).into_iter().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".sksync-")));
}

#[test]
fn full_update_nested_dependencies_child_first_existing_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("alpha", true, false);
}

#[test]
fn full_update_nested_dependencies_ancestor_first_existing_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("zzchild", true, false);
}

#[test]
fn full_update_nested_dependencies_child_first_absent_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("alpha", false, false);
}

#[test]
fn full_update_nested_dependencies_ancestor_first_absent_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("zzchild", false, false);
}

#[test]
fn full_update_nested_dependencies_case_alias_absent_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("alpha", false, true);
}

#[test]
fn full_update_nested_dependencies_case_alias_ancestor_first_absent_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("zzchild", false, true);
}

#[test]
fn full_update_nested_dependencies_case_alias_child_first_existing_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("alpha", true, true);
}

#[test]
fn full_update_nested_dependencies_case_alias_ancestor_first_existing_preserves_snapshots() {
    nested_dependency_fixture_is_rejected("zzchild", true, true);
}
