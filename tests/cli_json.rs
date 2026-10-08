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

// P09–P12 consume this single-response helper when their flags/adapters land.
#[allow(dead_code)]
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
