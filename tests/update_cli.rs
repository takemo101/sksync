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
    nested_dependency_fixture(child_name, existing, missing_case_alias, None);
}

fn nested_dependency_fixture(
    child_name: &str,
    existing: bool,
    missing_case_alias: bool,
    select_child: Option<bool>,
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
    if select_child.is_some() && !existing {
        fs::write(
            root.join("sksync-lock.json"),
            serde_json::to_vec(&serde_json::json!({
                "lockfileVersion":5, "generatedBy":"fixture", "generatedAt":"fixture", "root":".",
                "skills": {
                    ancestor_name: {"source":ancestor,"hash":"old-ancestor","files":[]},
                    child_name: {"source":child,"hash":"old-child","files":[]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }
    write_source(root, ancestor_name, "new local body");
    let old_tree = tree_snapshot(&store);
    let old_lock = fs::read(root.join("sksync-lock.json")).ok();
    let old_config = fs::read(&config_path).unwrap();
    let old_links = [ancestor_name, child_name]
        .map(|name| fs::read_link(root.join(".agents/skills").join(name)).ok());
    let output = match select_child {
        Some(true) => sksync(root, &["update", child_name]),
        Some(false) => sksync(root, &["update", ancestor_name]),
        None => sksync(root, &["update"]),
    };
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

#[test]
fn selected_update_preserves_edited_or_missing_unselected_content_and_lock_entry() {
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_lock = read_lock(root);
        let links = ["alpha", "beta"]
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
        write_source(root, "alpha", "selected new");
        write_source(root, "beta", "unselected new");
        if missing {
            fs::remove_dir_all(&bodies[1]).unwrap();
        } else {
            fs::write(bodies[1].join("SKILL.md"), "unselected local edits").unwrap();
        }
        assert_success(&sksync(root, &["update", "alpha", "alpha"]));
        assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
            .unwrap()
            .contains("selected new"));
        assert!(!bodies[0].join("extra.txt").exists());
        if missing {
            assert!(!bodies[1].exists());
        } else {
            assert_eq!(
                fs::read(bodies[1].join("SKILL.md")).unwrap(),
                b"unselected local edits"
            );
        }
        let lock = read_lock(root);
        assert_eq!(lock["skills"]["beta"], old_lock["skills"]["beta"]);
        assert_ne!(
            lock["skills"]["alpha"]["hash"],
            old_lock["skills"]["alpha"]["hash"]
        );
        assert_eq!(
            lock["skills"]["alpha"]["source"],
            old_lock["skills"]["alpha"]["source"]
        );
        assert_eq!(lock["lockfileVersion"], 5);
        assert_eq!(lock["root"], ".");
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(
            ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
            links
        );
    }
}

fn read_lock(root: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(root.join("sksync-lock.json")).unwrap()).unwrap()
}

#[test]
fn selected_update_disjoint_unselected_namespace_does_not_require_body_health() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for state in ["file", "dangling", "unreadable-content"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let old_lock = read_lock(root);
        fs::remove_dir_all(&bodies[1]).unwrap();
        match state {
            "file" => fs::write(&bodies[1], "unmanaged body file").unwrap(),
            "dangling" => symlink(root.join("missing-unselected-referent"), &bodies[1]).unwrap(),
            _ => {
                fs::create_dir(&bodies[1]).unwrap();
                fs::write(bodies[1].join("SKILL.md"), "private edits").unwrap();
                fs::set_permissions(bodies[1].join("SKILL.md"), fs::Permissions::from_mode(0))
                    .unwrap();
            }
        }
        write_source(root, "alpha", "new selected content");
        // Neither a broken source nor broken agent placement is relevant to unselected health.
        fs::remove_dir_all(root.join("sources/beta")).unwrap();
        fs::remove_file(root.join(".agents/skills/beta")).unwrap();
        fs::write(root.join(".agents/skills/beta"), "unmanaged target").unwrap();
        assert_success(&sksync(root, &["update", "alpha"]));
        assert_eq!(
            read_lock(root)["skills"]["beta"],
            old_lock["skills"]["beta"]
        );
        assert_eq!(
            fs::read(root.join(".agents/skills/beta")).unwrap(),
            b"unmanaged target"
        );
        match state {
            "file" => assert_eq!(fs::read(&bodies[1]).unwrap(), b"unmanaged body file"),
            "dangling" => assert_eq!(
                fs::read_link(&bodies[1]).unwrap(),
                root.join("missing-unselected-referent")
            ),
            _ => {
                assert_eq!(
                    fs::metadata(bodies[1].join("SKILL.md"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0
                );
                fs::set_permissions(
                    bodies[1].join("SKILL.md"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                assert_eq!(
                    fs::read(bodies[1].join("SKILL.md")).unwrap(),
                    b"private edits"
                );
            }
        }
    }
}

#[test]
fn selected_update_retains_disjoint_legacy_stale_records_and_ignores_their_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    add_legacy(root, "legacy", &bodies[1]);
    assert_success(&sksync(root, &["apply"]));
    let mut old_lock = read_lock(root);
    old_lock["skills"]["stale"] = old_lock["skills"]["beta"].clone();
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::to_vec(&old_lock).unwrap(),
    )
    .unwrap();
    fs::remove_dir_all(&bodies[1]).unwrap();
    write_source(root, "alpha", "new");
    assert_success(&sksync(root, &["update", "alpha"]));
    let lock = read_lock(root);
    for name in ["beta", "legacy", "stale"] {
        assert_eq!(lock["skills"][name], old_lock["skills"][name]);
    }
    assert!(!bodies[1].exists());
}

#[test]
fn selected_update_rejects_collateral_legacy_or_stale_body_views_before_preparation() {
    use std::os::unix::fs::symlink;
    for stale in [false, true] {
        for relation in ["equal", "child", "ancestor", "alias"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let bodies = fixture(root);
            let protected = match relation {
                "equal" => bodies[0].clone(),
                "child" => bodies[0].join("missing-child"),
                "ancestor" => root.join(".sksync/skills"),
                _ => {
                    let alias = root.join("legacy-body-alias");
                    symlink(&bodies[0], &alias).unwrap();
                    alias
                }
            };
            if stale {
                let mut lock = read_lock(root);
                lock["skills"]["stale"] = lock["skills"]["alpha"].clone();
                lock["skills"]["stale"]["source"] = serde_json::json!(protected);
                fs::write(
                    root.join("sksync-lock.json"),
                    serde_json::to_vec(&lock).unwrap(),
                )
                .unwrap();
            } else {
                add_legacy(root, "legacy", &protected);
                let mut lock = read_lock(root);
                lock["skills"]["legacy"] = lock["skills"]["alpha"].clone();
                lock["skills"]["legacy"]["source"] = serde_json::json!(protected);
                fs::write(
                    root.join("sksync-lock.json"),
                    serde_json::to_vec(&lock).unwrap(),
                )
                .unwrap();
            }
            let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
            let old_config = fs::read(root.join("sksync.config.json")).unwrap();
            let old_body = fs::read(bodies[0].join("SKILL.md")).unwrap();
            // A safety rejection must precede preparation of this invalid source.
            fs::write(root.join("sources/alpha/SKILL.md"), "invalid source").unwrap();
            let output = sksync(root, &["update", "alpha"]);
            assert!(!output.status.success(), "{stale} {relation}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("destinations overlap"),
                "{stale} {relation}: {error}"
            );
            assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
            assert_eq!(
                fs::read(root.join("sksync.config.json")).unwrap(),
                old_config
            );
            assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_body);
            assert_no_private_residue(root);
        }
    }
}

fn assert_no_private_residue(root: &Path) {
    assert!(!walkdir::WalkDir::new(root.join(".sksync/skills"))
        .into_iter()
        .any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".sksync-")
        }));
}

#[test]
fn selected_update_name_and_baseline_errors_precede_source_preparation() {
    for case in [
        "unknown",
        "legacy",
        "reserved-dot",
        "reserved-parent",
        "case",
        "missing-lock",
        "invalid-lock",
        "missing-unselected",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let name = match case {
            "unknown" => "unknown",
            "legacy" => {
                add_legacy(root, "legacy", &root.join("absent-legacy"));
                "legacy"
            }
            "reserved-dot" => ".",
            "reserved-parent" => "..",
            "case" => "ALPHA",
            _ => "alpha",
        };
        match case {
            "missing-lock" => fs::remove_file(root.join("sksync-lock.json")).unwrap(),
            "invalid-lock" => fs::write(root.join("sksync-lock.json"), "invalid raw lock").unwrap(),
            "missing-unselected" => {
                let mut lock = read_lock(root);
                lock["skills"].as_object_mut().unwrap().remove("beta");
                fs::write(
                    root.join("sksync-lock.json"),
                    serde_json::to_vec(&lock).unwrap(),
                )
                .unwrap();
            }
            _ => {}
        }
        let old_lock = fs::read(root.join("sksync-lock.json")).ok();
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_bodies = bodies
            .iter()
            .map(|path| fs::read(path.join("SKILL.md")).unwrap())
            .collect::<Vec<_>>();
        fs::remove_dir_all(root.join("sources")).unwrap();
        // Include a valid name first to ensure the entire set is validated.
        let output = sksync(root, &["update", "alpha", name]);
        assert_eq!(output.status.code(), Some(1), "{case}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("Preparing skills"), "{case}: {error}");
        if case.contains("lock") || case == "missing-unselected" {
            assert!(error.contains("full install/update"), "{case}: {error}");
        }
        assert_eq!(fs::read(root.join("sksync-lock.json")).ok(), old_lock);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        for (body, old) in bodies.iter().zip(old_bodies) {
            assert_eq!(fs::read(body.join("SKILL.md")).unwrap(), old);
        }
        assert_no_private_residue(root);
    }
}

#[test]
fn selected_update_multiple_prepare_failure_preserves_entire_selection_and_absence() {
    for absent in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        if absent {
            fs::remove_dir_all(&bodies[0]).unwrap();
        }
        let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_alpha = fs::read(bodies[0].join("SKILL.md")).ok();
        let old_beta = fs::read(bodies[1].join("SKILL.md")).unwrap();
        let links = ["alpha", "beta"]
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
        write_source(root, "alpha", "valid new body");
        fs::write(root.join("sources/beta/SKILL.md"), "invalid second body").unwrap();
        let output = sksync(root, &["update", "beta", "alpha", "beta"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Updated skill:"));
        assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(fs::read(bodies[0].join("SKILL.md")).ok(), old_alpha);
        assert_eq!(fs::read(bodies[1].join("SKILL.md")).unwrap(), old_beta);
        assert_eq!(
            ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
            links
        );
        assert_no_private_residue(root);
    }
}

#[test]
fn selected_update_inserts_initially_absent_selected_entry_and_keeps_stale_records() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let mut old_lock = read_lock(root);
    old_lock["skills"]["stale"] = old_lock["skills"]["beta"].clone();
    old_lock["skills"]["stale"]["source"] = serde_json::json!("old-unconfigured-body");
    old_lock["skills"].as_object_mut().unwrap().remove("alpha");
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::to_vec(&old_lock).unwrap(),
    )
    .unwrap();
    fs::remove_dir_all(&bodies[0]).unwrap();
    write_source(root, "alpha", "new selected entry");
    assert_success(&sksync(root, &["update", " alpha "]));
    let lock = read_lock(root);
    assert!(lock["skills"]["alpha"].is_object());
    for name in ["beta", "stale"] {
        assert_eq!(lock["skills"][name], old_lock["skills"][name]);
    }
    assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
        .unwrap()
        .contains("new selected entry"));
}

#[test]
fn selected_update_preserves_shared_target_owners_without_target_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let path = root.join("sksync.config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["agents"]["fx"] = serde_json::json!({"scope":"project","targetDir":".agents/skills"});
    config["dependencies"]["alpha"]["agents"] = serde_json::json!(["universal", "fx"]);
    config["agents"]["missing-target"] = serde_json::json!({"scope":"project"});
    config["dependencies"]["beta"]["agents"] = serde_json::json!(["missing-target"]);
    let config_bytes = serde_json::to_vec(&config).unwrap();
    fs::write(&path, &config_bytes).unwrap();
    let old_lock = read_lock(root);
    let link = fs::read_link(root.join(".agents/skills/alpha")).unwrap();
    write_source(root, "alpha", "new shared content");
    assert_success(&sksync(root, &["update", "alpha"]));
    assert_eq!(fs::read(&path).unwrap(), config_bytes);
    assert_eq!(
        fs::read_link(root.join(".agents/skills/alpha")).unwrap(),
        link
    );
    assert_eq!(
        read_lock(root)["skills"]["beta"],
        old_lock["skills"]["beta"]
    );
    assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
        .unwrap()
        .contains("new shared content"));
}

#[test]
fn selected_update_project_and_global_scopes_are_isolated_without_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let project_bodies = fixture(root);
    let global_root = root.join("home/.sksync");
    fs::create_dir_all(&global_root).unwrap();
    write_source(&global_root, "global-only", "old global body");
    fs::write(global_root.join("config.json"), r#"{
        "skillDir":"./skills", "dependencies":{"global-only":{"source":"./sources/global-only","agents":["universal"]}}
    }"#).unwrap();
    assert_success(&sksync(root, &["install", "--global"]));
    let global_lock_bytes = fs::read(global_root.join("sksync-lock.json")).unwrap();
    let project_lock_bytes = fs::read(root.join("sksync-lock.json")).unwrap();
    assert_eq!(
        sksync(root, &["update", "global-only"]).status.code(),
        Some(1)
    );
    assert_eq!(
        sksync(root, &["update", "alpha", "--global"]).status.code(),
        Some(1)
    );
    write_source(root, "alpha", "new project");
    write_source(&global_root, "global-only", "new global");
    assert_success(&sksync(root, &["update", "global-only", "--global"]));
    assert_eq!(
        fs::read(root.join("sksync-lock.json")).unwrap(),
        project_lock_bytes
    );
    assert!(fs::read_to_string(project_bodies[0].join("SKILL.md"))
        .unwrap()
        .contains("old"));
    assert_ne!(
        fs::read(global_root.join("sksync-lock.json")).unwrap(),
        global_lock_bytes
    );
    let global_after = fs::read(global_root.join("sksync-lock.json")).unwrap();
    assert_success(&sksync(root, &["update", "alpha"]));
    assert_eq!(
        fs::read(global_root.join("sksync-lock.json")).unwrap(),
        global_after
    );
}

#[test]
fn selected_update_does_not_add_force_dry_run_or_json_flags() {
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--force", "--dry-run", "--json"] {
        assert_eq!(
            sksync(dir.path(), &["update", "alpha", flag]).status.code(),
            Some(2)
        );
    }
}

#[test]
fn selected_update_dependency_nesting_existing_and_absent_preserves_snapshots() {
    for existing in [false, true] {
        for select_child in [false, true] {
            nested_dependency_fixture("alpha", existing, false, Some(select_child));
        }
    }
}

#[test]
fn selected_update_dependency_missing_prefix_case_alias_preserves_snapshots() {
    for existing in [false, true] {
        for select_child in [false, true] {
            nested_dependency_fixture("alpha", existing, true, Some(select_child));
        }
    }
}

fn local_git_commit(root: &Path, repo: &Path, body: &str) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let content = format!("---\nname: review\ndescription: Local Git test\n---\n{body}\n");
    let previous = test_command(root, "git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "main"])
        .output()
        .unwrap();
    let parent = if previous.status.success() {
        format!(
            "from {}\n",
            String::from_utf8(previous.stdout).unwrap().trim()
        )
    } else {
        String::new()
    };
    let pin = if parent.is_empty() {
        "reset refs/heads/pinned\nfrom :1\n\n"
    } else {
        ""
    };
    let input = format!("commit refs/heads/main\nmark :1\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\ndata 7\nfixture\n{parent}M 100644 inline SKILL.md\ndata {}\n{}\nM 100644 inline docs/details.txt\ndata {}\n{}\nM 100644 inline excluded.txt\ndata 8\nexcluded\n\n{pin}", content.len(), content, body.len(), body);
    let mut child = test_command(root, "git")
        .arg("-C")
        .arg(repo)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert_success(&child.wait_with_output().unwrap());
    let output = test_command(root, "git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "main"])
        .output()
        .unwrap();
    assert_success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn selected_update_git_records_actual_ref_and_include_without_freezing_outdated_observation() {
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
    let first = local_git_commit(root, &repo, "first");
    let config = serde_json::json!({
        "skillDir":"./.sksync/skills",
        "agents":{"universal":{"scope":"project","targetDir":".agents/skills"}},
        "dependencies":{
            "review":{"source":{"provider":"git","url":repo,"path":".","ref":"main"},"agents":["universal"],"include":["SKILL.md","docs"]},
            "qa":{"source":{"provider":"git","url":repo,"path":".","ref":"pinned"},"agents":["universal"],"include":["SKILL.md"]}
        }
    });
    let config_bytes = serde_json::to_vec(&config).unwrap();
    fs::write(root.join("sksync.config.json"), &config_bytes).unwrap();
    assert_success(&sksync(root, &["install"]));
    let old_lock = read_lock(root);
    assert_eq!(old_lock["skills"]["qa"]["installSource"]["ref"], first);
    let review_body = root.join(old_lock["skills"]["review"]["source"].as_str().unwrap());
    let qa_body = root.join(old_lock["skills"]["qa"]["source"].as_str().unwrap());
    fs::write(qa_body.join("SKILL.md"), "unselected Git local edits").unwrap();
    let links =
        ["review", "qa"].map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
    let observed = local_git_commit(root, &repo, "observed second");
    let outdated = sksync(root, &["outdated", "--json"]);
    assert_success(&outdated);
    let envelope: serde_json::Value = serde_json::from_slice(&outdated.stdout).unwrap();
    assert_eq!(envelope["data"]["rows"][0]["latest"], observed);
    let prepared = local_git_commit(root, &repo, "actually prepared third");
    assert_ne!(prepared, observed);
    assert_success(&sksync(root, &["update", "review"]));
    let lock = read_lock(root);
    assert_eq!(lock["skills"]["review"]["installSource"]["ref"], prepared);
    assert_eq!(lock["skills"]["qa"], old_lock["skills"]["qa"]);
    assert_eq!(
        lock["skills"]["review"]["source"],
        old_lock["skills"]["review"]["source"]
    );
    assert_eq!(
        lock["skills"]["review"]["include"],
        serde_json::json!(["SKILL.md", "docs"])
    );
    assert_eq!(
        lock["skills"]["review"]["files"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        fs::read(qa_body.join("SKILL.md")).unwrap(),
        b"unselected Git local edits"
    );
    assert!(!review_body.join("excluded.txt").exists());
    assert_eq!(
        fs::read(review_body.join("docs/details.txt")).unwrap(),
        b"actually prepared third"
    );
    assert_eq!(
        fs::read(root.join("sksync.config.json")).unwrap(),
        config_bytes
    );
    assert_eq!(
        ["review", "qa"].map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
        links
    );
}

#[test]
fn selected_update_missing_body_unicode_or_case_alias_of_stale_entry_is_rejected() {
    use std::os::unix::fs::MetadataExt;
    for alias in ["SKSYNC", "sKsync", "ſksync"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let capability = root.join("sksync");
        fs::create_dir(&capability).unwrap();
        let Ok(alias_metadata) = fs::metadata(root.join(alias)) else {
            eprintln!("selected missing alias skipped: filesystem does not alias {alias}");
            continue;
        };
        let metadata = fs::metadata(&capability).unwrap();
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (alias_metadata.dev(), alias_metadata.ino())
        );
        eprintln!("selected missing alias exercised: {alias}");
        write_source(root, "review", "valid selected source");
        fs::write(root.join("sksync.config.json"), serde_json::to_vec(&serde_json::json!({
            "skillDir":"./.sksync/skills", "dependencies":{"sksync":{"source":"./sources/review","agents":["universal"]}}
        })).unwrap()).unwrap();
        fs::write(root.join("sksync-lock.json"), serde_json::to_vec(&serde_json::json!({
            "lockfileVersion":5, "generatedBy":"fixture", "generatedAt":"fixture", "root":".",
            "skills":{"stale":{"source":format!(".sksync/skills/{alias}/missing-child"),"hash":"old-hash","files":[]}}
        })).unwrap()).unwrap();
        let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
        let output = sksync(root, &["update", "sksync"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("destinations overlap"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Preparing skills"));
        assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
        assert!(!root.join(".sksync/skills/sksync").exists());
        assert_no_private_residue(root);
    }
}

#[test]
fn selected_update_existing_case_alias_of_legacy_entry_is_rejected() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let alias = bodies[0].with_file_name("ALPHA");
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        eprintln!("selected existing case alias skipped: case-sensitive filesystem");
        return;
    };
    let metadata = fs::metadata(&bodies[0]).unwrap();
    assert_eq!(
        (metadata.dev(), metadata.ino()),
        (alias_metadata.dev(), alias_metadata.ino())
    );
    eprintln!("selected existing case alias exercised");
    add_legacy(root, "legacy", &alias);
    let mut lock = read_lock(root);
    lock["skills"]["legacy"] = lock["skills"]["alpha"].clone();
    lock["skills"]["legacy"]["source"] = serde_json::json!(alias);
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
    let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
    let old_body = fs::read(bodies[0].join("SKILL.md")).unwrap();
    write_source(root, "alpha", "must not prepare");
    let output = sksync(root, &["update", "alpha"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("destinations overlap"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Preparing skills"));
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
    assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_body);
    assert_no_private_residue(root);
}

#[test]
fn selected_update_preserves_disjoint_legacy_parent_traversal_and_relative_dangling_alias() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bodies = fixture(root);
    let legacy_parent = root.join("legacy-container/subdir");
    fs::create_dir_all(&legacy_parent).unwrap();
    let legacy_path = legacy_parent.join("../legacy-body");
    fs::write(root.join("legacy-container/legacy-body"), "legacy file").unwrap();
    add_legacy(root, "legacy", &legacy_path);
    let mut old_lock = read_lock(root);
    old_lock["skills"]["legacy"] = old_lock["skills"]["beta"].clone();
    old_lock["skills"]["legacy"]["source"] =
        serde_json::json!(legacy_path.strip_prefix(root).unwrap());
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::to_vec(&old_lock).unwrap(),
    )
    .unwrap();
    fs::remove_dir_all(&bodies[1]).unwrap();
    symlink("../../missing-body", &bodies[1]).unwrap();
    write_source(root, "alpha", "new");
    assert_success(&sksync(root, &["update", "alpha"]));
    let lock = read_lock(root);
    assert_eq!(lock["skills"]["legacy"], old_lock["skills"]["legacy"]);
    assert_eq!(lock["skills"]["beta"], old_lock["skills"]["beta"]);
    assert_eq!(
        fs::read(root.join("legacy-container/legacy-body")).unwrap(),
        b"legacy file"
    );
    assert_eq!(
        fs::read_link(&bodies[1]).unwrap(),
        Path::new("../../missing-body")
    );
}

#[test]
fn selected_update_parent_traversal_preserves_canceled_lookup_directories_and_symlinks() {
    use std::os::unix::fs::symlink;
    for stale in [false, true] {
        for route in ["directory", "symlink", "symlink-referent"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let bodies = fixture(root);
            let source = if route == "directory" {
                fs::create_dir(bodies[0].join("subdir")).unwrap();
                bodies[0].join("subdir/../../beta")
            } else {
                let outside = root.join("outside");
                fs::create_dir_all(outside.join("subdir")).unwrap();
                fs::create_dir(outside.join("legacy")).unwrap();
                fs::write(outside.join("legacy/SKILL.md"), "outside legacy content").unwrap();
                symlink(outside.join("subdir"), bodies[0].join("escape")).unwrap();
                let traversal = bodies[0].join("escape/../legacy");
                if route == "symlink-referent" {
                    let alias = root.join("legacy-alias");
                    symlink(&traversal, &alias).unwrap();
                    alias
                } else {
                    traversal
                }
            };
            let name = if stale { "stale" } else { "legacy" };
            if !stale {
                add_legacy(root, name, &source);
            }
            let mut lock = read_lock(root);
            lock["skills"][name] = lock["skills"]["beta"].clone();
            lock["skills"][name]["source"] = serde_json::json!(source.strip_prefix(root).unwrap());
            fs::write(
                root.join("sksync-lock.json"),
                serde_json::to_vec(&lock).unwrap(),
            )
            .unwrap();
            let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
            let old_config = fs::read(root.join("sksync.config.json")).unwrap();
            let old_alpha = fs::read(bodies[0].join("SKILL.md")).unwrap();
            let old_legacy = fs::read(source.join("SKILL.md")).unwrap();
            let links = ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
            write_source(root, "alpha", "new body without lookup prerequisites");
            let output = sksync(root, &["update", "alpha"]);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{stale} {route}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("namespace prerequisite"),
                "{stale} {route}: {error}"
            );
            assert!(!error.contains("Preparing skills"), "{error}");
            assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
            assert_eq!(
                fs::read(root.join("sksync.config.json")).unwrap(),
                old_config
            );
            assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_alpha);
            assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), old_legacy);
            assert_eq!(
                ["alpha", "beta"]
                    .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
                links
            );
            if route == "directory" {
                assert!(bodies[0].join("subdir").is_dir());
            } else {
                assert_eq!(
                    fs::read_link(bodies[0].join("escape")).unwrap(),
                    root.join("outside/subdir")
                );
            }
            assert_no_private_residue(root);
        }
    }
}

#[test]
fn selected_update_symlinked_store_or_disjoint_legacy_parent_preserves_state() {
    use std::os::unix::fs::symlink;
    for managed_store in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        let alias = if managed_store {
            let logical = root.join(".sksync/skills");
            fs::rename(&logical, root.join(".sksync/physical-skills")).unwrap();
            symlink("physical-skills", &logical).unwrap();
            logical
        } else {
            let actual = root.join("outside/legacy");
            fs::create_dir_all(&actual).unwrap();
            fs::write(actual.join("SKILL.md"), "outside legacy content").unwrap();
            let alias = root.join("outside-alias");
            symlink("outside", &alias).unwrap();
            let source = alias.join("legacy");
            add_legacy(root, "legacy", &source);
            let mut lock = read_lock(root);
            lock["skills"]["legacy"] = lock["skills"]["beta"].clone();
            lock["skills"]["legacy"]["source"] =
                serde_json::json!(source.strip_prefix(root).unwrap());
            fs::write(
                root.join("sksync-lock.json"),
                serde_json::to_vec(&lock).unwrap(),
            )
            .unwrap();
            alias
        };
        let old_lock = read_lock(root);
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_beta = fs::read(bodies[1].join("SKILL.md")).unwrap();
        let old_alias = fs::read_link(&alias).unwrap();
        let links = ["alpha", "beta"]
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
        write_source(root, "alpha", "new selected body");
        assert_success(&sksync(root, &["update", "alpha"]));
        assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
            .unwrap()
            .contains("new selected body"));
        assert_eq!(fs::read(bodies[1].join("SKILL.md")).unwrap(), old_beta);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(fs::read_link(&alias).unwrap(), old_alias);
        assert_eq!(
            ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
            links
        );
        let lock = read_lock(root);
        assert_eq!(lock["skills"]["beta"], old_lock["skills"]["beta"]);
        if !managed_store {
            assert_eq!(lock["skills"]["legacy"], old_lock["skills"]["legacy"]);
            assert_eq!(
                fs::read(alias.join("legacy/SKILL.md")).unwrap(),
                b"outside legacy content"
            );
        }
        assert_no_private_residue(root);
        let physical_store = root.join(".sksync/skills").canonicalize().unwrap();
        assert!(!fs::read_dir(physical_store).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".sksync-")));
    }
}

#[test]
fn selected_update_parent_traversal_through_shared_ancestor_does_not_protect_its_subtrees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let legacy = root.join("legacy");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("SKILL.md"), "disjoint content").unwrap();
    let source = root.join(".sksync/skills/../../legacy");
    add_legacy(root, "legacy", &source);
    let mut old_lock = read_lock(root);
    old_lock["skills"]["legacy"] = old_lock["skills"]["beta"].clone();
    old_lock["skills"]["legacy"]["source"] = serde_json::json!(source.strip_prefix(root).unwrap());
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::to_vec(&old_lock).unwrap(),
    )
    .unwrap();
    write_source(root, "alpha", "new");
    assert_success(&sksync(root, &["update", "alpha"]));
    let lock = read_lock(root);
    assert_eq!(lock["skills"]["legacy"], old_lock["skills"]["legacy"]);
    assert_eq!(
        fs::read(source.join("SKILL.md")).unwrap(),
        b"disjoint content"
    );
    assert_no_private_residue(root);
}

#[test]
fn selected_update_root_only_lookup_prerequisite_survives_directory_replacement() {
    use std::os::unix::fs::{symlink, MetadataExt};
    for record in ["configured", "legacy", "stale"] {
        for route in ["direct", "external-root-alias", "case-root-alias"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let bodies = fixture(root);
            let alias = match route {
                "external-root-alias" => {
                    let alias = root.join("alpha-root-alias");
                    symlink(&bodies[0], &alias).unwrap();
                    alias
                }
                "case-root-alias" => {
                    let alias = bodies[0].with_file_name("ALPHA");
                    let Ok(alias_metadata) = fs::metadata(&alias) else {
                        eprintln!(
                            "SKIP root-only lookup {record}: filesystem lacks case root alias"
                        );
                        continue;
                    };
                    let metadata = fs::metadata(&bodies[0]).unwrap();
                    assert_eq!(
                        (metadata.dev(), metadata.ino()),
                        (alias_metadata.dev(), alias_metadata.ino())
                    );
                    alias
                }
                _ => bodies[0].clone(),
            };
            let source = alias.join("../beta");
            let name = if record == "configured" {
                "beta"
            } else {
                record
            };
            if record == "legacy" {
                add_legacy(root, name, &source);
            }
            install_traversal_lock_record(root, name, &source);
            write_source(root, "beta", "unselected source must not be fetched");
            fs::write(bodies[1].join("SKILL.md"), "unselected local edits").unwrap();
            let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
            let old_config = fs::read(root.join("sksync.config.json")).unwrap();
            let old_beta = fs::read(bodies[1].join("SKILL.md")).unwrap();
            let old_extra = fs::read(bodies[1].join("extra.txt")).unwrap();
            let links = ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
            write_source(root, "alpha", "new selected root content");
            let output = sksync(root, &["update", "alpha"]);
            assert_success(&output);
            eprintln!("root-only lookup exercised: {record} {route}");
            assert!(fs::read_to_string(bodies[0].join("SKILL.md"))
                .unwrap()
                .contains("new selected root content"));
            assert!(bodies[0].is_dir());
            assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), old_beta);
            assert_eq!(fs::read(bodies[1].join("extra.txt")).unwrap(), old_extra);
            assert_eq!(
                fs::read(root.join("sksync.config.json")).unwrap(),
                old_config
            );
            assert_eq!(
                ["alpha", "beta"]
                    .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
                links
            );
            let new_lock = fs::read(root.join("sksync-lock.json")).unwrap();
            for unselected in ["beta", name] {
                assert_eq!(
                    raw_lock_record(&new_lock, unselected),
                    raw_lock_record(&old_lock, unselected)
                );
            }
            if route == "external-root-alias" {
                assert_eq!(fs::read_link(&alias).unwrap(), bodies[0]);
            }
            assert_no_private_residue(root);
        }
    }
}

fn raw_lock_record(bytes: &[u8], name: &str) -> String {
    let text = std::str::from_utf8(bytes).unwrap();
    let start = text.find(&format!("    \"{name}\": {{")).unwrap();
    let end = start + text[start..].find("\n    }").unwrap() + "\n    }".len();
    text[start..end].to_owned()
}

fn install_traversal_lock_record(root: &Path, name: &str, source: &Path) {
    let path = root.join("sksync-lock.json");
    let old = fs::read(&path).unwrap();
    let original = raw_lock_record(&old, "beta");
    let source_before = read_lock(root)["skills"]["beta"]["source"]
        .as_str()
        .unwrap()
        .to_owned();
    let record = original
        .replace("    \"beta\": {", &format!("    \"{name}\": {{"))
        .replace(
            &format!(
                "\"source\": {}",
                serde_json::to_string(&source_before).unwrap()
            ),
            &format!(
                "\"source\": {}",
                serde_json::to_string(source.strip_prefix(root).unwrap()).unwrap()
            ),
        );
    let mut text = String::from_utf8(old).unwrap();
    if name == "beta" {
        text = text.replace(&original, &record);
    } else {
        let end = text.rfind("\n  }").unwrap();
        text.insert_str(end, &format!(",\n{record}"));
    }
    fs::write(path, text).unwrap();
}

#[test]
fn selected_update_descendant_alias_location_is_protected_even_when_referent_is_root() {
    use std::os::unix::fs::symlink;
    for name in ["legacy", "stale"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bodies = fixture(root);
        symlink(&bodies[0], bodies[0].join("escape")).unwrap();
        let source = bodies[0].join("escape/../beta");
        if name == "legacy" {
            add_legacy(root, name, &source);
        }
        install_traversal_lock_record(root, name, &source);
        let old_lock = fs::read(root.join("sksync-lock.json")).unwrap();
        let old_config = fs::read(root.join("sksync.config.json")).unwrap();
        let old_alpha = fs::read(bodies[0].join("SKILL.md")).unwrap();
        let old_beta = fs::read(source.join("SKILL.md")).unwrap();
        let links = ["alpha", "beta"]
            .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap());
        write_source(root, "alpha", "must not replace alias location");
        let output = sksync(root, &["update", "alpha"]);
        assert_eq!(output.status.code(), Some(1));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("namespace prerequisite"), "{error}");
        assert!(!error.contains("Preparing skills"), "{error}");
        assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), old_lock);
        assert_eq!(
            fs::read(root.join("sksync.config.json")).unwrap(),
            old_config
        );
        assert_eq!(fs::read(bodies[0].join("SKILL.md")).unwrap(), old_alpha);
        assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), old_beta);
        assert_eq!(fs::read_link(bodies[0].join("escape")).unwrap(), bodies[0]);
        assert_eq!(
            ["alpha", "beta"]
                .map(|name| fs::read_link(root.join(".agents/skills").join(name)).unwrap()),
            links
        );
        assert_no_private_residue(root);
    }
}
