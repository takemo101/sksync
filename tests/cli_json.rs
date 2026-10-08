use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sksync(root: &Path, args: &[&str]) -> Output {
    let home = root.join("home");
    let config_home = home.join(".config");
    fs::create_dir_all(&config_home).unwrap();
    Command::new(env!("CARGO_BIN_EXE_sksync"))
        .current_dir(root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .args(args)
        .output()
        .unwrap()
}

fn json_response(output: &Output) -> serde_json::Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).expect("exactly one JSON response without human text")
}

#[test]
fn rendered_check_failure_is_not_printed_again_by_main() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(
        root.join("source/SKILL.md"),
        "---\nname: review\ndescription: Test skill\n---\nBody\n",
    )
    .unwrap();
    fs::write(
        root.join("sksync.config.json"),
        r#"{
        "skillDir": "./.sksync/skills",
        "agents": {"universal": {"scope": "project", "targetDir": ".agents/skills"}},
        "dependencies": {"review": {"source": "./source", "agents": ["universal"]}}
    }"#,
    )
    .unwrap();
    let installed = sksync(root, &["install"]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    fs::remove_file(root.join(".agents/skills/review")).unwrap();
    let output = sksync(root, &["check"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Target missing"));
    assert!(
        output.stderr.is_empty(),
        "already rendered report was printed again: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unrendered_application_failure_prints_once_to_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let output = sksync(dir.path(), &["list"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("Error:").count(), 1, "{stderr}");
}

#[test]
fn clap_syntax_help_and_version_keep_their_exit_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = sksync(dir.path(), &["list", "--not-a-flag"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unexpected argument"));
    for args in [&["--help"][..], &["--version"][..]] {
        let output = sksync(dir.path(), args);
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn list_json_missing_lockfile_and_invalid_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let missing_config = sksync(root, &["list", "--json"]);
    assert_eq!(missing_config.status.code(), Some(1));
    assert_eq!(
        json_response(&missing_config)["error"]["code"],
        "CONFIG_NOT_FOUND"
    );
    assert!(missing_config.stderr.is_empty());
    fs::write(
        root.join("sksync.config.json"),
        r#"{"skillDir":"./.sksync/skills","dependencies":{}}"#,
    )
    .unwrap();
    let output = sksync(root, &["list", "--json"]);
    assert!(output.status.success(), "{:?}", output);
    let value = json_response(&output);
    assert_eq!(
        value,
        serde_json::json!({"schemaVersion":1,"command":"list","scope":"project","ok":true,
        "data":{"skills":[],"lockfileStatus":"missing"},"error":null})
    );
    assert!(output.stderr.is_empty());
    fs::write(root.join("sksync-lock.json"), b"invalid").unwrap();
    for args in [&["list", "--json"][..], &["list"][..]] {
        let output = sksync(root, args);
        assert_eq!(output.status.code(), Some(1));
        if args.len() == 2 {
            let value = json_response(&output);
            assert_eq!(value["ok"], false);
            assert!(value["data"].is_null());
            assert_eq!(value["error"]["code"], "INVALID_LOCKFILE");
            assert!(output.stderr.is_empty());
        }
    }
    fs::write(root.join("sksync.config.json"), b"invalid").unwrap();
    let output = sksync(root, &["list", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_response(&output)["error"]["code"], "INVALID_CONFIG");
}

#[test]
fn list_json_resolution_and_inspection_failures_retain_rows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("body")).unwrap();
    fs::write(root.join("blocked"), "unmanaged").unwrap();
    for (target, tag, code, resolved) in [
        (
            "../outside",
            "resolveFailed",
            "TARGET_RESOLUTION_FAILED",
            false,
        ),
        ("blocked/skills", "inspectFailed", "INSPECTION_FAILED", true),
    ] {
        fs::write(
            root.join("sksync.config.json"),
            serde_json::json!({
                "agents":{"custom":{"scope":"project","targetDir":target}},
                "skills":{"review":{"source":"./body","agents":["custom"]}}
            })
            .to_string(),
        )
        .unwrap();
        for args in [&["list", "--json"][..], &["list"][..]] {
            let output = sksync(root, args);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            if args.len() == 2 {
                let value = json_response(&output);
                assert_eq!(value["ok"], false);
                assert_eq!(value["error"]["code"], code);
                let row = &value["data"]["skills"][0];
                assert_eq!(row["name"], "review");
                for field in ["installSource", "include", "lockedHash"] {
                    assert!(row[field].is_null());
                }
                let target = &row["targets"][0];
                assert_eq!(target["status"], tag);
                assert_eq!(target["target"].is_string(), resolved);
                assert_eq!(target["error"]["code"], code);
                assert!(target["error"]["message"].as_str().unwrap().len() > 0);
                assert!(output.stderr.is_empty());
            }
        }
    }
    assert_eq!(
        fs::read_to_string(root.join("blocked")).unwrap(),
        "unmanaged"
    );
}

#[test]
fn list_json_global_scope_uses_only_injected_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let global = root.join("home/.sksync");
    fs::create_dir_all(&global).unwrap();
    fs::write(root.join("sksync.config.json"), "invalid project config").unwrap();
    fs::write(global.join("config.json"), r#"{"dependencies":{"review":{"source":"./source","agents":["pi"],"include":["SKILL.md"]}}}"#).unwrap();
    let output = sksync(root, &["list", "--json", "--global"]);
    assert!(output.status.success(), "{output:?}");
    let value = json_response(&output);
    assert_eq!(value["scope"], "global");
    assert_eq!(value["data"]["lockfileStatus"], "missing");
    let row = &value["data"]["skills"][0];
    assert_eq!(row["installSource"]["type"], "local");
    assert_eq!(
        row["installSource"]["path"],
        global.join("./source").display().to_string()
    );
    assert_eq!(row["include"], serde_json::json!(["SKILL.md"]));
    assert!(row["lockedHash"].is_null());
    assert_eq!(row["targets"][0]["status"], "sourceMissing");
    assert_eq!(
        row["targets"][0]["target"],
        root.join("home/.pi/agent/skills/review")
            .display()
            .to_string()
    );
    assert!(!global.join("skills").exists());
    assert!(!global.join("sksync-lock.json").exists());
}

#[cfg(unix)]
#[test]
fn list_json_unreadable_lockfile_is_not_missing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("sksync.config.json"), "{}").unwrap();
    fs::create_dir(root.join("sksync-lock.json")).unwrap();
    let output = sksync(root, &["list", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_response(&output)["error"]["code"], "IO_ERROR");
    fs::remove_dir(root.join("sksync-lock.json")).unwrap();
    std::os::unix::fs::symlink(root.join("absent"), root.join("sksync-lock.json")).unwrap();
    let output = sksync(root, &["list", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_response(&output)["error"]["code"], "IO_ERROR");
}

#[cfg(unix)]
#[test]
fn list_json_sorted_observations_and_configured_git_source_do_not_fetch_or_hash() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let body = root.join("body");
    fs::create_dir(&body).unwrap();
    // Deliberately not a valid package: list observes paths, never validates or hashes content.
    fs::write(body.join("SKILL.md"), "invalid edited body").unwrap();
    fs::create_dir(root.join("other")).unwrap();
    fs::create_dir(root.join("targets")).unwrap();
    for agent in ["synced", "drifted", "conflict", "broken"] {
        fs::create_dir(root.join("targets").join(agent)).unwrap();
    }
    symlink(&body, root.join("targets/synced/review")).unwrap();
    symlink(root.join("other"), root.join("targets/drifted/review")).unwrap();
    symlink(root.join("absent"), root.join("targets/broken/review")).unwrap();
    fs::write(root.join("targets/conflict/review"), "unmanaged").unwrap();
    let agents = ["synced", "missing", "drifted", "conflict", "broken"];
    let mappings: serde_json::Map<String, serde_json::Value> = agents
        .iter()
        .map(|agent| {
            (
                (*agent).to_owned(),
                serde_json::json!({"scope":"project", "targetDir":format!("targets/{agent}")}),
            )
        })
        .collect();
    let unavailable_remote = root.join("unavailable.git").display().to_string();
    let config = serde_json::json!({
        "agents": mappings,
        "skills":{"review":{"source":"./body", "agents":agents}},
        "dependencies":{"aaa":{"source":{"provider":"git", "url":unavailable_remote, "ref":"topic", "path":"skills/aaa"}, "agents":["missing"], "include":["references/**", "SKILL.md"]}}
    });
    let lock = serde_json::json!({"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".",
        "skills":{"review":{"source":"./body","hash":"sha256-static","files":[]}}});
    fs::write(root.join("sksync.config.json"), config.to_string()).unwrap();
    fs::write(root.join("sksync-lock.json"), lock.to_string()).unwrap();
    let output = sksync(root, &["list", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let value = json_response(&output);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["lockfileStatus"], "available");
    let rows = value["data"]["skills"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["aaa", "review"]
    );
    assert_eq!(
        rows[0]["installSource"],
        serde_json::json!({"type":"git","url":unavailable_remote,"ref":"topic","path":"skills/aaa"})
    );
    assert_eq!(
        rows[0]["include"],
        serde_json::json!(["SKILL.md", "references/**"])
    );
    assert!(rows[0]["lockedHash"].is_null());
    assert_eq!(rows[0]["targets"][0]["status"], "sourceMissing");
    assert_eq!(rows[1]["lockedHash"], "sha256-static");
    let targets = rows[1]["targets"].as_array().unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|target| target["agent"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["broken", "conflict", "drifted", "missing", "synced"]
    );
    assert_eq!(
        targets
            .iter()
            .map(|target| target["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["brokenSymlink", "conflict", "drifted", "missing", "synced"]
    );
    assert!(targets.iter().all(|target| target.get("error").is_none()));
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        config.to_string()
    );
    assert_eq!(
        fs::read_to_string(root.join("sksync-lock.json")).unwrap(),
        lock.to_string()
    );
    assert_eq!(
        fs::read_to_string(body.join("SKILL.md")).unwrap(),
        "invalid edited body"
    );
    assert_eq!(
        fs::read_to_string(root.join("targets/conflict/review")).unwrap(),
        "unmanaged"
    );
    assert!(!root.join(".sksync").exists());
}

#[cfg(unix)]
#[test]
fn list_json_non_utf8_project_root_emits_serialization_failure() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join(OsString::from_vec(b"project-\xff".to_vec()));
    if !create_non_utf8_test_root(&root) {
        return;
    }
    let config = br#"{"dependencies":{"review":{"source":"./source","agents":["pi"]}}}"#;
    fs::write(root.join("sksync.config.json"), config).unwrap();

    let output = sksync(&root, &["list", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "list");
    assert_eq!(value["scope"], "project");
    assert_eq!(value["ok"], false);
    assert!(value["data"].is_null());
    assert_eq!(value["error"]["code"], "SERIALIZATION_FAILED");
    assert!(value["error"]["message"].is_string());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), config);
    assert!(!root.join(".sksync").exists());
    assert!(!root.join("sksync-lock.json").exists());
}

#[cfg(unix)]
#[test]
fn list_json_non_utf8_home_emits_serialization_failure() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let home = root.join(OsString::from_vec(b"home-\xff".to_vec()));
    let config_home = home.join(".config");
    let global = home.join(".sksync");
    if !create_non_utf8_test_root(&home) {
        return;
    }
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&global).unwrap();
    let config = br#"{"dependencies":{"review":{"source":"./source","agents":["pi"]}}}"#;
    fs::write(global.join("config.json"), config).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sksync"))
        .current_dir(root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["list", "--json", "--global"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "list");
    assert_eq!(value["scope"], "global");
    assert_eq!(value["ok"], false);
    assert!(value["data"].is_null());
    assert_eq!(value["error"]["code"], "SERIALIZATION_FAILED");
    assert!(value["error"]["message"].is_string());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(fs::read(global.join("config.json")).unwrap(), config);
    assert!(!global.join("skills").exists());
    assert!(!global.join("sksync-lock.json").exists());
}

#[cfg(unix)]
fn create_non_utf8_test_root(path: &Path) -> bool {
    // EILSEQ is 92 on macOS and 84 on Linux; no other setup failure is skipped.
    const EILSEQ: i32 = if cfg!(target_os = "macos") { 92 } else { 84 };
    match fs::create_dir_all(path) {
        Ok(()) => true,
        Err(error) if error.raw_os_error() == Some(EILSEQ) => {
            eprintln!("SKIP non-UTF-8 CLI fixture: filesystem rejects path bytes with EILSEQ; renderer unit regressions exercise the handler boundary.");
            false
        }
        Err(error) => panic!("failed to create non-UTF-8 CLI fixture: {error}"),
    }
}

#[test]
fn plan_json_regular_file_blocker_preserves_unmanaged_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("body")).unwrap();
    fs::create_dir_all(root.join(".agents/skills")).unwrap();
    let config = br#"{"agents":{"universal":{"scope":"project","targetDir":".agents/skills"}},"skills":{"review":{"source":"./body","agents":["universal"]}}}"#;
    fs::write(root.join("sksync.config.json"), config).unwrap();
    let target = root.join(".agents/skills/review");
    fs::write(&target, b"unmanaged bytes").unwrap();
    let output = sksync(root, &["plan", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = json_response(&output);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "plan");
    assert_eq!(value["scope"], "project");
    assert_eq!(value["ok"], true);
    assert!(value["error"].is_null());
    assert_eq!(value["data"]["applicable"], false);
    assert_eq!(
        value["data"]["items"],
        serde_json::json!([{
            "owners":[{"skill":"review","agent":"universal"}],
            "source":root.join("./body"), "target":target,
            "action":"conflict", "reason":"regularFile"
        }])
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(&target).unwrap(), b"unmanaged bytes");
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), config);
    assert!(!root.join(".sksync").exists());
    assert!(!root.join("sksync-lock.json").exists());
    let human = sksync(root, &["plan"]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("regular file"));
}

#[cfg(unix)]
#[test]
fn plan_json_sorted_shared_owners_and_all_actions_are_read_only() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("body")).unwrap();
    fs::write(root.join("body/SKILL.md"), "not validated or hashed").unwrap();
    fs::create_dir(root.join("other")).unwrap();
    fs::create_dir_all(root.join(".agents/skills")).unwrap();
    let targets = root.join(".agents/skills");
    symlink(root.join("body"), targets.join("synced")).unwrap();
    symlink(root.join("other"), targets.join("drifted")).unwrap();
    symlink(root.join("absent"), targets.join("broken")).unwrap();
    fs::create_dir(targets.join("directory")).unwrap();
    let names = [
        "synced",
        "missing",
        "drifted",
        "directory",
        "create",
        "broken",
    ];
    let skills: serde_json::Map<String, serde_json::Value> = names.iter().map(|name| (
        (*name).to_owned(), serde_json::json!({"source":if *name=="missing" {"./absent"} else {"./body"}, "agents":["universal","fx"]})
    )).collect();
    let config = serde_json::json!({
        "agents":{"universal":{"scope":"project","targetDir":".agents/skills"}, "fx":{"scope":"project","targetDir":".agents/skills"}},
        "skills":skills,
        "dependencies":{"remote":{"source":{"provider":"git","url":root.join("unavailable.git"),"path":"skills/remote"},"agents":["universal","fx"]}}
    }).to_string();
    fs::write(root.join("sksync.config.json"), &config).unwrap();
    // Planning does not need or rewrite a lockfile.
    fs::write(root.join("sksync-lock.json"), b"not read by plan").unwrap();
    let output = sksync(root, &["plan", "--json", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let value = json_response(&output);
    assert_eq!(value["data"]["applicable"], false);
    let items = value["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 7, "one row per physical target");
    let mut paths = items
        .iter()
        .map(|item| item["target"].as_str().unwrap())
        .collect::<Vec<_>>();
    let original = paths.clone();
    paths.sort_unstable();
    assert_eq!(original, paths);
    for (name, action, reason) in [
        ("broken", "conflict", Some("brokenSymlink")),
        ("create", "createSymlink", None),
        ("directory", "conflict", Some("directory")),
        ("drifted", "driftedSymlink", None),
        ("missing", "sourceMissing", None),
        ("remote", "sourceMissing", None),
        ("synced", "alreadySynced", None),
    ] {
        let item = items
            .iter()
            .find(|item| item["target"] == targets.join(name).display().to_string())
            .unwrap();
        assert_eq!(
            item["owners"],
            serde_json::json!([{"skill":name,"agent":"fx"},{"skill":name,"agent":"universal"}])
        );
        assert_eq!(item["action"], action);
        assert_eq!(item.get("reason").and_then(|x| x.as_str()), reason);
        if name == "drifted" {
            assert_eq!(
                item["actualSource"],
                root.join("other").display().to_string()
            );
        } else {
            assert!(item.get("actualSource").is_none());
        }
    }
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        config
    );
    assert_eq!(
        fs::read(root.join("sksync-lock.json")).unwrap(),
        b"not read by plan"
    );
    assert_eq!(
        fs::read_to_string(root.join("body/SKILL.md")).unwrap(),
        "not validated or hashed"
    );
    assert_eq!(
        fs::read_link(targets.join("drifted")).unwrap(),
        root.join("other")
    );
    assert_eq!(
        fs::read_link(targets.join("broken")).unwrap(),
        root.join("absent")
    );
    assert!(!targets.join("create").exists());
    assert!(!root.join(".sksync").exists());
    // Only create and already-synced actions are normally applicable.
    let mut config: serde_json::Value = serde_json::from_str(&config).unwrap();
    config["dependencies"] = serde_json::json!({});
    config["skills"]
        .as_object_mut()
        .unwrap()
        .retain(|name, _| name == "create" || name == "synced");
    fs::write(root.join("sksync.config.json"), config.to_string()).unwrap();
    assert_eq!(
        json_response(&sksync(root, &["plan", "--json"]))["data"]["applicable"],
        true
    );
}

#[test]
fn plan_json_loading_and_planning_errors_have_null_data() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("body")).unwrap();
    fs::write(root.join("blocked"), "unmanaged").unwrap();
    for (config, code) in [
        (None, "CONFIG_NOT_FOUND"),
        (Some("invalid".to_owned()), "INVALID_CONFIG"),
        (Some(serde_json::json!({"agents":{"custom":{"scope":"project","targetDir":"../outside"}},"skills":{"review":{"source":"./body","agents":["custom"]}}}).to_string()), "TARGET_RESOLUTION_FAILED"),
        (Some(serde_json::json!({"agents":{"custom":{"scope":"project","targetDir":"blocked/skills"}},"skills":{"review":{"source":"./body","agents":["custom"]}}}).to_string()), "INSPECTION_FAILED"),
        (Some(serde_json::json!({"agents":{"custom":{"scope":"project","targetDir":"targets"}},"skills":{"review":{"source":"./blocked/body","agents":["custom"]}}}).to_string()), "IO_ERROR"),
    ] {
        if let Some(config) = config { fs::write(root.join("sksync.config.json"), config).unwrap(); }
        for args in [&["plan", "--json"][..], &["plan"][..]] {
            let output = sksync(root, args);
            assert_eq!(output.status.code(), Some(1), "{code}: {output:?}");
            if args.len()==2 {
                let value = json_response(&output);
                assert_eq!(value["command"], "plan");
                assert_eq!(value["ok"], false);
                assert!(value["data"].is_null());
                assert_eq!(value["error"]["code"], code);
                assert!(output.stderr.is_empty());
            }
        }
    }
    assert_eq!(
        fs::read_to_string(root.join("blocked")).unwrap(),
        "unmanaged"
    );
    assert!(!root.join("targets").exists());
    assert!(!root.join(".sksync").exists());
}

#[test]
fn plan_json_empty_global_scope_does_not_create_store_or_targets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let global = root.join("home/.sksync");
    fs::create_dir_all(&global).unwrap();
    fs::write(root.join("sksync.config.json"), "invalid project").unwrap();
    fs::write(global.join("config.json"), "{}").unwrap();
    let output = sksync(root, &["plan", "--json", "--global"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        json_response(&output),
        serde_json::json!({"schemaVersion":1,"command":"plan","scope":"global","ok":true,"data":{"items":[],"applicable":true},"error":null})
    );
    assert!(output.stderr.is_empty());
    fs::write(
        global.join("config.json"),
        r#"{"dependencies":{"review":{"source":"./source","agents":["pi"]}}}"#,
    )
    .unwrap();
    let output = sksync(root, &["plan", "--json", "--global"]);
    assert!(output.status.success());
    let value = json_response(&output);
    assert_eq!(value["data"]["items"][0]["action"], "sourceMissing");
    assert_eq!(
        value["data"]["items"][0]["target"],
        root.join("home/.pi/agent/skills/review")
            .display()
            .to_string()
    );
    assert!(!root.join("home/.pi").exists());
    assert!(!global.join("skills").exists());
    assert!(!global.join("sksync-lock.json").exists());
}

#[cfg(unix)]
#[test]
fn check_json_unhealthy_target_and_restored_healthy_report() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(
        root.join("source/SKILL.md"),
        "---\nname: review\ndescription: Local test\n---\nBody\n",
    )
    .unwrap();
    fs::write(root.join("sksync.config.json"), r#"{"skillDir":"./.sksync/skills","agents":{"universal":{"scope":"project","targetDir":".agents/skills"}},"dependencies":{"review":{"source":"./source","agents":["universal"]}}}"#).unwrap();
    let installed = sksync(root, &["install"]);
    assert!(installed.status.success(), "{installed:?}");
    let target = root.join(".agents/skills/review");
    let source = fs::read_link(&target).unwrap();
    let config = fs::read(root.join("sksync.config.json")).unwrap();
    let lock = fs::read(root.join("sksync-lock.json")).unwrap();
    fs::remove_file(&target).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let value = json_response(&output);
    assert_eq!(value["command"], "check");
    assert_eq!(value["scope"], "project");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "CHECK_FAILED");
    assert_eq!(
        value["data"],
        serde_json::json!({"healthy":false,"problems":[{"kind":"targetMissing","skill":"review","agent":"universal","path":target}]})
    );
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(!target.exists(), "check must not repair links");
    std::os::unix::fs::symlink(&source, &target).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        json_response(&output),
        serde_json::json!({"schemaVersion":1,"command":"check","scope":"project","ok":true,"data":{"healthy":true,"problems":[]},"error":null})
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), config);
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), lock);
    assert_eq!(fs::read_link(&target).unwrap(), source);
}

#[test]
fn check_json_execution_errors_have_null_data() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("sksync.config.json"), "{}").unwrap();
    for (lock, config, code) in [
        (None, Some("{}"), "LOCKFILE_NOT_FOUND"),
        (Some("invalid"), Some("{}"), "INVALID_LOCKFILE"),
        (
            Some(
                r#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}}"#,
            ),
            Some("invalid"),
            "INVALID_CONFIG",
        ),
        (
            Some(
                r#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}}"#,
            ),
            None,
            "CONFIG_NOT_FOUND",
        ),
        (
            Some(
                r#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}}"#,
            ),
            Some(
                r#"{"agents":{"custom":{"scope":"project","targetDir":"../outside"}},"skills":{"review":{"source":"./body","agents":["custom"]}}}"#,
            ),
            "TARGET_RESOLUTION_FAILED",
        ),
    ] {
        if let Some(lock) = lock {
            fs::write(root.join("sksync-lock.json"), lock).unwrap();
        }
        if let Some(config) = config {
            fs::write(root.join("sksync.config.json"), config).unwrap();
        } else {
            fs::remove_file(root.join("sksync.config.json")).unwrap();
        }
        for args in [&["check", "--json"][..], &["check"][..]] {
            let output = sksync(root, args);
            assert_eq!(output.status.code(), Some(1), "{code}: {output:?}");
            if args.len() == 2 {
                let value = json_response(&output);
                assert_eq!(value["command"], "check");
                assert_eq!(value["ok"], false);
                assert!(value["data"].is_null());
                assert_eq!(value["error"]["code"], code);
                assert!(output.stderr.is_empty());
            }
        }
    }
}

#[test]
fn check_json_global_scope_uses_only_injected_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let global = root.join("home/.sksync");
    fs::create_dir_all(&global).unwrap();
    fs::write(root.join("sksync.config.json"), "invalid project").unwrap();
    fs::write(global.join("config.json"), "{}").unwrap();
    let lock = br#"{"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}}"#;
    fs::write(global.join("sksync-lock.json"), lock).unwrap();
    let output = sksync(root, &["check", "--json", "--global"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        json_response(&output),
        serde_json::json!({"schemaVersion":1,"command":"check","scope":"global","ok":true,"data":{"healthy":true,"problems":[]},"error":null})
    );
    assert!(output.stderr.is_empty());
    assert!(!global.join("skills").exists());
    assert_eq!(fs::read(global.join("sksync-lock.json")).unwrap(), lock);
}

#[cfg(unix)]
#[test]
fn check_json_unreadable_required_lockfile_is_execution_failure() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("sksync.config.json"), "{}").unwrap();
    let lock = root.join("sksync-lock.json");
    fs::create_dir(&lock).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "IO_ERROR");
    assert!(value["data"].is_null());
    assert!(output.stderr.is_empty());
    fs::remove_dir(&lock).unwrap();
    std::os::unix::fs::symlink(root.join("absent"), &lock).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "IO_ERROR");
    assert!(value["data"].is_null());
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_link(&lock).unwrap(), root.join("absent"));
}

#[cfg(unix)]
#[test]
fn check_json_typed_findings_are_complete_sorted_and_read_only() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("review")).unwrap();
    fs::write(
        root.join("review/SKILL.md"),
        "---\nname: review\ndescription: Local test\n---\nBody\n",
    )
    .unwrap();
    fs::create_dir(root.join("other")).unwrap();
    fs::write(root.join("blocked"), "unmanaged parent").unwrap();
    let agents = [
        "missing",
        "unexpected",
        "broken",
        "file",
        "directory",
        "inspect",
    ];
    let mappings: serde_json::Map<String, serde_json::Value> = agents.iter().map(|agent| (
        (*agent).to_owned(), serde_json::json!({"scope":"project", "targetDir":if *agent=="inspect" {"blocked/skills".to_owned()} else {format!("targets/{agent}")}})
    )).collect();
    for agent in ["unexpected", "broken", "file", "directory"] {
        fs::create_dir_all(root.join("targets").join(agent)).unwrap();
    }
    symlink(root.join("other"), root.join("targets/unexpected/review")).unwrap();
    symlink(root.join("absent"), root.join("targets/broken/review")).unwrap();
    fs::write(root.join("targets/file/review"), "unmanaged target").unwrap();
    fs::create_dir(root.join("targets/directory/review")).unwrap();
    let config = serde_json::json!({"skillDir":".","agents":mappings,"dependencies":{"review":{"source":"./unavailable","agents":agents,"include":["SKILL.md"]}}}).to_string();
    let lock = serde_json::json!({"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{"review":{"source":"./review","hash":"sha256-old","files":[]}}}).to_string();
    fs::write(root.join("sksync.config.json"), &config).unwrap();
    fs::write(root.join("sksync-lock.json"), &lock).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty());
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "CHECK_FAILED");
    assert_eq!(value["data"]["healthy"], false);
    let problems = value["data"]["problems"].as_array().unwrap();
    assert_eq!(
        problems
            .iter()
            .map(|p| p["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "brokenSymlink",
            "includeMismatch",
            "inspectFailed",
            "sourceHashDrift",
            "targetConflict",
            "targetConflict",
            "targetMissing",
            "targetUnexpectedSymlink"
        ]
    );
    assert_eq!(
        problems[0]["actualSource"],
        root.join("absent").display().to_string()
    );
    assert_eq!(problems[1]["expected"], "SKILL.md");
    assert_eq!(problems[1]["actual"], "<full package>");
    assert!(problems[2]["message"].is_string());
    assert_eq!(problems[3]["expected"], "sha256-old");
    assert!(problems[3]["actual"]
        .as_str()
        .unwrap()
        .starts_with("sha256-"));
    assert_eq!(problems[4]["agent"], "directory");
    assert_eq!(problems[4]["reason"], "directory exists");
    assert_eq!(problems[5]["reason"], "regular file exists");
    assert_eq!(
        problems[7]["actualSource"],
        root.join("other").display().to_string()
    );
    assert_eq!(json_response(&sksync(root, &["check", "--json"])), value);
    let human = sksync(root, &["check"]);
    assert_eq!(human.status.code(), Some(1));
    assert!(human.stderr.is_empty());
    assert!(String::from_utf8_lossy(&human.stdout).contains("Include mismatch"));
    fs::remove_dir_all(root.join("review")).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "CHECK_FAILED");
    let problems = value["data"]["problems"].as_array().unwrap();
    assert!(problems
        .iter()
        .any(|p| p["kind"] == "hashFailed" && p["message"].is_string()));
    assert!(problems.iter().any(|p| p["kind"] == "targetMissing"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(root.join("sksync.config.json")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(root.join("sksync-lock.json")).unwrap(),
        lock
    );
    assert_eq!(
        fs::read_to_string(root.join("blocked")).unwrap(),
        "unmanaged parent"
    );
    assert_eq!(
        fs::read_to_string(root.join("targets/file/review")).unwrap(),
        "unmanaged target"
    );
    assert!(root.join("targets/directory/review").is_dir());
    assert_eq!(
        fs::read_link(root.join("targets/broken/review")).unwrap(),
        root.join("absent")
    );
    assert_eq!(
        fs::read_link(root.join("targets/unexpected/review")).unwrap(),
        root.join("other")
    );
    assert!(!root.join("targets/missing").exists());
    assert!(!root.join(".sksync").exists());
}

#[cfg(unix)]
#[test]
fn check_json_shared_target_emits_singular_universal_pi_fx_owners() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(
        root.join("source/SKILL.md"),
        "---\nname: review\ndescription: Local test\n---\nBody\n",
    )
    .unwrap();
    let mappings: serde_json::Map<String, serde_json::Value> = ["universal", "pi", "fx"]
        .into_iter()
        .map(|agent| {
            (
                agent.to_owned(),
                serde_json::json!({"scope":"project","targetDir":".agents/skills"}),
            )
        })
        .collect();
    fs::write(root.join("sksync.config.json"), serde_json::json!({"agents":mappings,"dependencies":{"review":{"source":"./source","agents":["universal","pi","fx"]}}}).to_string()).unwrap();
    let installed = sksync(root, &["install"]);
    assert!(installed.status.success(), "{installed:?}");
    let target = root.join(".agents/skills/review");
    let source = fs::read_link(&target).unwrap();
    let config = fs::read(root.join("sksync.config.json")).unwrap();
    let lock = fs::read(root.join("sksync-lock.json")).unwrap();
    fs::remove_file(&target).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value = json_response(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "CHECK_FAILED");
    assert_eq!(
        value["data"],
        serde_json::json!({"healthy":false,"problems":[
            {"kind":"targetMissing","skill":"review","agent":"fx","path":target},
            {"kind":"targetMissing","skill":"review","agent":"pi","path":target},
            {"kind":"targetMissing","skill":"review","agent":"universal","path":target}
        ]})
    );
    assert!(!target.exists());
    let human = sksync(root, &["check"]);
    assert_eq!(human.status.code(), Some(1));
    assert!(human.stderr.is_empty());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("Target missing (1)"), "{text}");
    assert!(text.contains("review · fx, pi, universal ·"), "{text}");
    std::os::unix::fs::symlink(&source, &target).unwrap();
    let output = sksync(root, &["check", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let value = json_response(&output);
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["data"],
        serde_json::json!({"healthy":true,"problems":[]})
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(root.join("sksync.config.json")).unwrap(), config);
    assert_eq!(fs::read(root.join("sksync-lock.json")).unwrap(), lock);
    assert_eq!(fs::read_link(&target).unwrap(), source);
}

#[test]
fn outdated_json_empty_and_input_errors_are_single_envelopes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let output = sksync(root, &["outdated", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_response(&output)["error"]["code"], "CONFIG_NOT_FOUND");
    assert!(output.stderr.is_empty());
    fs::write(root.join("sksync.config.json"), r#"{"dependencies":{}}"#).unwrap();
    let output = sksync(root, &["outdated", "--json"]);
    assert_eq!(
        json_response(&output)["error"]["code"],
        "LOCKFILE_NOT_FOUND"
    );
    fs::write(root.join("sksync-lock.json"), "invalid").unwrap();
    let output = sksync(root, &["outdated", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "INVALID_LOCKFILE");
    assert!(value["data"].is_null());
    assert_eq!(value["ok"], false);
    assert!(output.stderr.is_empty());
    fs::write(root.join("sksync.config.json"), "invalid").unwrap();
    let output = sksync(root, &["outdated", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = json_response(&output);
    assert_eq!(value["error"]["code"], "INVALID_CONFIG");
    assert!(value["data"].is_null());
    assert_eq!(value["ok"], false);
    assert!(output.stderr.is_empty());
    fs::write(root.join("sksync.config.json"), r#"{"dependencies":{}}"#).unwrap();
    fs::write(
        root.join("sksync-lock.json"),
        serde_json::json!({
            "lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}
        })
        .to_string(),
    )
    .unwrap();
    let output = sksync(root, &["outdated", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        json_response(&output),
        serde_json::json!({
            "schemaVersion":1,"command":"outdated","scope":"project","ok":true,
            "data":{"rows":[],"problems":[]},"error":null
        })
    );
    assert!(output.stderr.is_empty());
}

fn outdated_git_command(root: &Path) -> Command {
    let home = root.join("home");
    let config = home.join(".config");
    fs::create_dir_all(&config).unwrap();
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    command
}

#[test]
fn outdated_json_local_git_partial_report_and_human_exit() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = root.join("repo.git");
    let output = outdated_git_command(root)
        .args(["init", "--bare", "--initial-branch=main"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    // Local fixture history only; never mutate the workspace's index/history.
    let input = b"commit refs/heads/main\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\ndata 7\nfixture\nM 100644 inline SKILL.md\ndata 5\nbody\n\n\n";
    let mut child = outdated_git_command(root)
        .arg("-C")
        .arg(&repo)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = outdated_git_command(root)
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "main"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let latest = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let unavailable = root.join("unavailable.git");
    let mut dependencies = serde_json::Map::new();
    let mut locked = serde_json::Map::new();
    for (name, url, current) in [
        ("zeta", &repo, "old"),
        ("alpha", &repo, "old"),
        ("current", &repo, latest.as_str()),
        ("failed-z", &unavailable, "old"),
        ("failed-a", &unavailable, "old"),
    ] {
        dependencies.insert(name.into(), serde_json::json!({"source":{"provider":"git","url":url,"path":".","ref":"main"},"agents":["universal"]}));
        locked.insert(name.into(), serde_json::json!({"source":format!(".sksync/skills/{name}"),
            "installSource":{"type":"git","url":url,"path":".","ref":current},"hash":"sha256-test","files":[]}));
    }
    dependencies.insert(
        "local".into(),
        serde_json::json!({"source":"./absent-local","agents":["universal"]}),
    );
    let config_path = root.join("sksync.config.json");
    let lock_path = root.join("sksync-lock.json");
    fs::write(
        &config_path,
        serde_json::json!({"dependencies":dependencies}).to_string(),
    )
    .unwrap();
    fs::write(&lock_path, serde_json::json!({"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":locked}).to_string()).unwrap();
    let config_bytes = fs::read(&config_path).unwrap();
    let lock_bytes = fs::read(&lock_path).unwrap();
    for args in [&["outdated", "--json"][..], &["outdated"][..]] {
        let output = sksync(root, args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        if args.len() == 2 {
            let value = json_response(&output);
            assert_eq!(value["ok"], false);
            assert_eq!(value["error"]["code"], "REMOTE_QUERY_FAILED");
            let rows = value["data"]["rows"].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            for (row, name) in rows.iter().zip(["alpha", "zeta"]) {
                assert_eq!(
                    row,
                    &serde_json::json!({"skill":name,"current":"old","wanted":"main","latest":latest,"source":repo,"status":"outdated"})
                );
            }
            let problems = value["data"]["problems"].as_array().unwrap();
            assert_eq!(problems.len(), 2);
            for (problem, name) in problems.iter().zip(["failed-a", "failed-z"]) {
                assert_eq!(problem["skill"], name);
                assert_eq!(problem["source"], unavailable.display().to_string());
                assert_eq!(problem["wanted"], "main");
                assert_eq!(problem["error"]["code"], "REMOTE_QUERY_FAILED");
                assert!(!problem["error"]["message"].as_str().unwrap().is_empty());
                assert!(problem["error"].get("hint").is_none());
            }
        } else {
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("alpha") && text.contains("zeta"));
            assert!(text.contains("failed-a") && text.contains("failed-z"));
            assert!(!text.contains("All skills are up to date"));
        }
    }
    assert_eq!(fs::read(&config_path).unwrap(), config_bytes);
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes);
    assert!(!root.join(".sksync").exists());
    // All-success probes still exit zero when updates exist, in both modes.
    let mut config: serde_json::Value = serde_json::from_slice(&config_bytes).unwrap();
    config["dependencies"]
        .as_object_mut()
        .unwrap()
        .retain(|name, _| !name.starts_with("failed-"));
    fs::write(&config_path, config.to_string()).unwrap();
    for args in [&["outdated", "--json"][..], &["outdated"][..]] {
        let output = sksync(root, args);
        assert!(output.status.success(), "{output:?}");
        if args.len() == 2 {
            let value = json_response(&output);
            assert_eq!(value["ok"], true);
            assert!(value["error"].is_null());
            assert_eq!(value["data"]["rows"].as_array().unwrap().len(), 2);
            assert_eq!(value["data"]["problems"], serde_json::json!([]));
        }
    }
    // A report with only failed probes is still a usable failed report, not "up to date".
    let mut config: serde_json::Value = serde_json::from_slice(&config_bytes).unwrap();
    config["dependencies"]
        .as_object_mut()
        .unwrap()
        .retain(|name, _| name.starts_with("failed-"));
    fs::write(&config_path, config.to_string()).unwrap();
    for args in [&["outdated", "--json"][..], &["outdated"][..]] {
        let output = sksync(root, args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty());
        if args.len() == 2 {
            let value = json_response(&output);
            assert_eq!(value["data"]["rows"], serde_json::json!([]));
            assert_eq!(value["data"]["problems"].as_array().unwrap().len(), 2);
        } else {
            assert!(!String::from_utf8_lossy(&output.stdout).contains("All skills are up to date"));
        }
    }
    config["dependencies"].as_object_mut().unwrap().clear();
    fs::write(&config_path, config.to_string()).unwrap();
    let output = sksync(root, &["outdated"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("All skills are up to date"));
}

#[test]
fn outdated_json_global_scope_uses_only_injected_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let global = root.join("home/.sksync");
    fs::create_dir_all(&global).unwrap();
    fs::write(root.join("sksync.config.json"), "invalid project config").unwrap();
    fs::write(global.join("config.json"), r#"{"dependencies":{}}"#).unwrap();
    fs::write(global.join("sksync-lock.json"), serde_json::json!({"lockfileVersion":5,"generatedBy":"test","generatedAt":"test","root":".","skills":{}}).to_string()).unwrap();
    let output = sksync(root, &["outdated", "--json", "--global"]);
    assert!(output.status.success(), "{output:?}");
    let value = json_response(&output);
    assert_eq!(value["scope"], "global");
    assert_eq!(value["data"], serde_json::json!({"rows":[],"problems":[]}));
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn outdated_json_unreadable_required_lockfile_is_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("sksync.config.json"), r#"{"dependencies":{}}"#).unwrap();
    let lock = root.join("sksync-lock.json");
    fs::create_dir(&lock).unwrap();
    for dangling in [false, true] {
        if dangling {
            fs::remove_dir(&lock).unwrap();
            std::os::unix::fs::symlink(root.join("absent"), &lock).unwrap();
        }
        let output = sksync(root, &["outdated", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        let value = json_response(&output);
        assert_eq!(value["error"]["code"], "IO_ERROR");
        assert_eq!(value["ok"], false);
        assert!(value["data"].is_null());
        assert!(output.stderr.is_empty());
    }
}
