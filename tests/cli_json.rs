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
