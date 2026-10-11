use serde_json::{json, Value};

#[test]
fn native_contract_workflow_covers_supported_os_and_compiler_floor() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github/workflows/state-contract.yml");
    let source = std::fs::read_to_string(path).expect("native contract workflow exists");
    let workflow: Value = serde_yaml::from_str(&source).expect("valid workflow YAML");
    assert!(
        workflow.get("permissions").is_none(),
        "no permission changes"
    );
    let job = &workflow["jobs"]["state-contract"];
    assert!(job.get("permissions").is_none(), "no permission changes");
    assert_eq!(job["runs-on"], "${{ matrix.os }}");
    assert_eq!(
        job["strategy"]["matrix"]["os"],
        json!(["ubuntu-latest", "macos-latest"])
    );
    assert_eq!(
        job["strategy"]["matrix"]["toolchain"],
        json!(["stable", "1.89.0"])
    );
    let steps = job["steps"].as_array().unwrap();
    let toolchain = steps
        .iter()
        .find(|s| s["uses"] == "dtolnay/rust-toolchain@stable")
        .unwrap();
    assert_eq!(toolchain["with"]["toolchain"], "${{ matrix.toolchain }}");
    assert_eq!(toolchain["with"]["components"], "rustfmt, clippy");
    let gates = steps.iter().find_map(|s| s["run"].as_str()).unwrap();
    for command in [
        "cargo fmt --check",
        "cargo test --quiet --locked",
        "cargo build --release --quiet --locked",
        "cargo clippy --quiet --locked -- -D warnings",
        "--test cli_json -- --nocapture",
        "--test update_cli -- --nocapture",
    ] {
        assert!(gates.contains(command), "missing gate {command}");
    }
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "TMPDIR",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ] {
        assert!(
            gates.contains(&format!("export {key}=")),
            "missing isolation {key}"
        );
    }
    assert!(gates.contains("SKIP non-UTF-8 CLI fixture"));
    assert!(gates.contains("RUNNER_OS"));
    let isolation = gates.find("cargo() (").expect("per-invocation subshell");
    let home = gates.find("export HOME=").unwrap();
    for capture in ["export CARGO_HOME=", "export RUSTUP_HOME=", "export PATH="] {
        assert!(gates.find(capture).unwrap() < isolation);
    }
    assert!(isolation < home);
    assert!(home < gates.find("command cargo \"$@\"").unwrap());
    assert!(gates.contains("set -euo pipefail"));
    assert!(gates.contains("tee \"$RUNNER_TEMP/cli-json.log\""));
    assert!(gates.contains("[[ \"$RUNNER_OS\" == Linux ]]"));
    assert!(gates.contains("exit 1"));
    assert!(!source.contains("site/"));
}
