# Progress Phase Logs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show short colored phase logs during long-running `sksync` operations so CLI and wizard users can see what is happening while they wait.

**Architecture:** Add one small CLI output helper that writes progress messages to stderr and uses ANSI color only when stderr is a terminal. Insert phase logs at existing high-latency boundaries instead of adding detailed verbose logging or spinners. Keep stdout output and machine-readable command output unchanged.

**Tech Stack:** Rust stdlib, existing CLI functions in `src/cli.rs`, existing integration tests in `tests/bundle_cli.rs`.

---

### Task 1: Add a stderr progress output helper

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Write the failing test**

Add a unit test near the existing CLI helper tests:

```rust
#[test]
fn progress_message_is_colored_only_for_terminal_stderr() {
    assert_eq!(format_progress_message("Installing skills...", false), "→ Installing skills...");
    assert_eq!(
        format_progress_message("Installing skills...", true),
        "\u{1b}[36m→ Installing skills...\u{1b}[0m"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --quiet progress_message_is_colored_only_for_terminal_stderr`
Expected: FAIL because `format_progress_message` does not exist.

- [ ] **Step 3: Implement helper**

Add near `print_success` / `print_info`:

```rust
fn print_progress(message: impl AsRef<str>) {
    eprintln!(
        "{}",
        format_progress_message(message.as_ref(), std::io::stderr().is_terminal())
    );
}

fn format_progress_message(message: &str, color: bool) -> String {
    let plain = format!("→ {message}");
    if color {
        format!("\u{1b}[36m{plain}\u{1b}[0m")
    } else {
        plain
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --quiet progress_message_is_colored_only_for_terminal_stderr`
Expected: PASS.

---

### Task 2: Add phase logs to bundle operations

**Files:**
- Modify: `src/cli.rs`
- Test: `tests/bundle_cli.rs`

- [ ] **Step 1: Write failing integration test**

Add a test that runs `bundle sync --dry-run` and checks stderr contains phase logs while stdout still contains the plan:

```rust
#[test]
fn bundle_sync_dry_run_prints_progress_to_stderr() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_bundle_e2e_config(root);
    write_review_bundle(root);

    assert_success(sksync(root, &["bundle", "add", "./bundle", "--agent", "universal"]));

    let output = sksync(root, &["bundle", "sync", "review-workflow", "--dry-run"]);
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.contains("Bundle sync plan"), "stdout: {stdout}");
    assert!(stderr.contains("Loading bundle manifest"), "stderr: {stderr}");
    assert!(stderr.contains("Planning changes"), "stderr: {stderr}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --quiet bundle_sync_dry_run_prints_progress_to_stderr --test bundle_cli`
Expected: FAIL because stderr has no progress logs.

- [ ] **Step 3: Insert bundle phase logs**

Add `print_progress` calls:

```rust
// run_bundle_inspect / run_bundle_add / run_bundle_sync
print_progress("Loading bundle manifest...");
let bundle = load_bundle_from_source(...)?;

// before bundle plan construction
print_progress("Planning changes...");

// before update_dependencies in add/sync apply
print_progress("Installing skills...");

// before apply_link_plan in add/sync apply
print_progress("Applying links...");

// before bundle remove apply cleanup
print_progress("Removing bundle-managed skills...");
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --quiet bundle_sync_dry_run_prints_progress_to_stderr --test bundle_cli`
Expected: PASS.

---

### Task 3: Add phase logs to add/install/update/apply paths

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Add focused phase logs**

Insert minimal logs at existing boundaries:

```rust
// before run_add_workflow
print_progress("Resolving skill source...");

// before update_dependencies in add/attach/install/update
print_progress("Installing skills...");

// before build_link_plan/apply_link_plan in add/attach/install/apply
print_progress("Applying links...");

// before build_link_plan in plan/apply when only planning
print_progress("Planning links...");
```

Do not add logs to `--json` output paths except stderr-only progress logs that do not affect stdout.

- [ ] **Step 2: Verify core behavior**

Run:

```bash
cargo test --quiet
cargo clippy --quiet -- -D warnings
```

Expected: PASS.

---

### Task 4: Document progress logs and verify

**Files:**
- Modify: `README.md` or `site/reference/commands.md` only if needed

- [ ] **Step 1: Decide docs need**

If the logs are brief stderr hints and do not change command semantics, skip public docs. If adding docs, write one sentence: “Long-running operations print short progress phase messages to stderr; command output remains on stdout.”

- [ ] **Step 2: Run full verification**

Run:

```bash
cargo fmt --check
cargo test --quiet
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
bun run docs:build
git diff --check
```

Expected: all pass.

---

## Self-review

- Spec coverage: phase logs are stderr-only, colored when terminal, no spinner, minimal logs, CLI and wizard both benefit because wizard delegates to CLI.
- Placeholder scan: no TODO/TBD placeholders.
- Type consistency: helper names are `print_progress` and `format_progress_message`; tests reference the same names.
