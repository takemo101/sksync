# Bundle Manifest Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `sksync.bundle.json` discovery to `sksync bundle add`, `sksync bundle inspect`, and TUI bundle add while preserving deterministic bundle provenance.

**Architecture:** Add a bundle-manifest resolver in the application layer that finds and normalizes manifest candidates without doing terminal UI. Keep selection in CLI/TUI, matching `sksync add`: exact-first, one automatic candidate, TTY single-select for multiple candidates, and non-TTY failure with candidate guidance. `bundle sync` and `bundle remove` keep using exact stored provenance only.

**Tech Stack:** Rust 2021, clap, inquire, anyhow, serde_json, tempfile; GitButler CLI (`but`) for version-control operations.

---

## Files and responsibilities

- Modify `src/application/source.rs`: accept GitHub `/blob/<ref>/...` URLs so a copied manifest file URL parses as a Git source.
- Modify `src/application/bundle.rs`: define bundle manifest candidates, discover local/git candidates, normalize selected manifest parent sources, and convert candidates into `LoadedBundle`.
- Modify `src/cli.rs`: add `--name` to `bundle add`/`bundle inspect`, add single-selection UI and non-TTY error behavior, and route both commands through the resolver.
- Modify `src/tui/bundle.rs`: use the shared resolver for preview, select exactly one candidate when needed, and pass the resolved source to dry-run/apply commands to avoid duplicate selection.
- Modify `src/tui/commands.rs`: no signature change required unless you choose to pass a future name selector; ensure tests cover resolved source passthrough if changed.
- Tests live in existing `#[cfg(test)]` modules in `src/application/source.rs`, `src/application/bundle.rs`, `src/cli.rs`, `src/tui/bundle.rs`, and `src/tui/commands.rs`.
- Docs already updated: `CONTEXT.md`, `docs/DESIGN.md`.

## Task 1: Parse GitHub blob URLs as Git sources

**Files:**
- Modify: `src/application/source.rs`
- Test: `src/application/source.rs` tests module

- [ ] **Step 1: Write failing source-parser tests**

Add these tests in `src/application/source.rs` inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn github_blob_url_source_parses_as_git_source() {
    let source = parse_install_source_string(
        "https://github.com/owner/repo/blob/main/bundles/base/sksync.bundle.json",
    )
    .expect("source parses");
    let InstallSource::Git(git) = source else {
        panic!("expected git source");
    };

    assert_eq!(git.url, "https://github.com/owner/repo.git");
    assert_eq!(git.reference.as_deref(), Some("main"));
    assert_eq!(git.path, Path::new("bundles/base/sksync.bundle.json"));
}
```

- [ ] **Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test github_blob_url_source_parses_as_git_source --quiet
```

Expected: FAIL because `parse_install_source_string` currently does not accept `/blob/` GitHub URLs.

- [ ] **Step 3: Implement minimal blob parsing**

In `parse_github_tree_url`, accept either `tree` or `blob` in path segment 2. Replace the current branch:

```rust
if parts.get(2) == Some(&"tree") && parts.len() >= 4 {
    reference = Some(parts[3].to_owned());
    if parts.len() > 4 {
        path = PathBuf::from(parts[4..].join("/"));
    }
}
```

with:

```rust
if matches!(parts.get(2), Some(&"tree") | Some(&"blob")) && parts.len() >= 4 {
    reference = Some(parts[3].to_owned());
    if parts.len() > 4 {
        path = PathBuf::from(parts[4..].join("/"));
    }
}
```

- [ ] **Step 4: Run focused parser tests**

Run:

```bash
cargo test github_ --quiet
```

Expected: both tests PASS.

## Task 2: Add bundle manifest candidate discovery in the application layer

**Files:**
- Modify: `src/application/bundle.rs`
- Test: `src/application/bundle.rs` tests module

- [ ] **Step 1: Write failing local discovery tests**

Add tests in `src/application/bundle.rs` tests module. Use helper writes like this:

```rust
fn write_bundle_manifest(path: &Path, name: &str, description: &str) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(
        path.join("sksync.bundle.json"),
        format!(
            r#"{{
              "name": "{name}",
              "description": "{description}",
              "entries": {{ "review": {{ "source": "./skills/review" }} }}
            }}"#
        ),
    )
    .unwrap();
}

#[test]
fn discovers_bundle_manifests_under_local_source() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().join("repo");
    write_bundle_manifest(&root.join("bundles/base"), "team-baseline", "Team baseline");
    write_bundle_manifest(&root.join("node_modules/ignored"), "ignored", "Ignored bundle");

    let candidates = discover_bundle_manifest_candidates(
        &format!("./{}", root.strip_prefix(temp.path()).unwrap().display()),
        temp.path(),
    )
    .expect("discover candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].manifest.name.as_str(), "team-baseline");
    assert_eq!(candidates[0].relative_path, Path::new("bundles/base"));
    assert_eq!(candidates[0].resolved_source, "./repo/bundles/base");
}

#[test]
fn direct_bundle_manifest_file_resolves_to_parent_source() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bundle_dir = temp.path().join("repo/bundles/base");
    write_bundle_manifest(&bundle_dir, "team-baseline", "Team baseline");

    let candidates = discover_bundle_manifest_candidates(
        "./repo/bundles/base/sksync.bundle.json",
        temp.path(),
    )
    .expect("discover candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].resolved_source, "./repo/bundles/base");
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test bundle_manifest --quiet
```

Expected: FAIL because `discover_bundle_manifest_candidates` does not exist.

- [ ] **Step 3: Add candidate types and public discovery entrypoint**

Near the other bundle structs in `src/application/bundle.rs`, add:

```rust
pub const BUNDLE_MANIFEST_DISCOVERY_MAX_DEPTH: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifestCandidate {
    pub manifest: BundleManifest,
    pub provenance: BundleProvenance,
    pub entries: Vec<LoadedBundleEntry>,
    pub relative_path: PathBuf,
    pub resolved_source: String,
}

impl BundleManifestCandidate {
    pub fn into_loaded_bundle(self) -> LoadedBundle {
        LoadedBundle {
            manifest: self.manifest,
            provenance: self.provenance,
            entries: self.entries,
        }
    }
}
```

Then add a public function:

```rust
pub fn discover_bundle_manifest_candidates(
    raw_source: &str,
    config_root: &Path,
) -> Result<Vec<BundleManifestCandidate>> {
    let source = parse_install_source_string(raw_source)
        .with_context(|| format!("invalid bundle source {raw_source:?}"))?;
    match source {
        InstallSource::Local(path) => discover_local_bundle_manifest_candidates(raw_source, &path, config_root),
        InstallSource::Git(git) => discover_git_bundle_manifest_candidates(&git),
    }
}
```

- [ ] **Step 4: Implement local discovery**

Add helpers in `src/application/bundle.rs` using existing `read_bundle_manifest`, `normalize_local_source_for_config`, and `normalize_path_without_fs`:

```rust
fn discover_local_bundle_manifest_candidates(
    _raw_source: &str,
    path: &Path,
    config_root: &Path,
) -> Result<Vec<BundleManifestCandidate>> {
    let resolved = absolutize_config_path(path, config_root);
    let root = if is_bundle_manifest_file_path(&resolved) {
        resolved
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        resolved
    };

    if root.join(BUNDLE_MANIFEST_FILE).is_file() {
        return Ok(vec![load_local_bundle_manifest_candidate(&root, &root, config_root)?]);
    }

    let mut candidates = Vec::new();
    discover_local_bundle_manifest_candidates_inner(
        &root,
        &root,
        config_root,
        BUNDLE_MANIFEST_DISCOVERY_MAX_DEPTH,
        0,
        &mut candidates,
    )?;
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(candidates)
}
```

Use the same skip names as skill discovery:

```rust
fn is_skipped_bundle_discovery_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | ".sksync"))
}

fn is_bundle_manifest_file_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        == Some(BUNDLE_MANIFEST_FILE)
}
```

`load_local_bundle_manifest_candidate` should read the manifest once, normalize entries exactly like `load_local_bundle`, and return `relative_path` as `.` when the selected manifest is the root.

- [ ] **Step 5: Run local discovery tests**

Run:

```bash
cargo test bundle_manifest --quiet
```

Expected: PASS.

## Task 3: Add Git bundle manifest discovery and source normalization tests

**Files:**
- Modify: `src/application/bundle.rs`
- Test: `src/application/bundle.rs` tests module

- [ ] **Step 1: Write pure normalization tests for Git candidate sources**

Add tests that do not clone from the network:

```rust
#[test]
fn git_bundle_manifest_file_source_resolves_to_parent_tree_source() {
    let git = GitInstallSource {
        url: "https://github.com/org/bundles.git".to_owned(),
        reference: Some("main".to_owned()),
        path: PathBuf::from("bundles/base/sksync.bundle.json"),
    };

    let parent = bundle_manifest_parent_git_source(&git).unwrap();

    assert_eq!(parent.path, Path::new("bundles/base"));
    assert_eq!(
        git_source_to_config_string(&parent),
        "https://github.com/org/bundles/tree/main/bundles/base"
    );
}
```

- [ ] **Step 2: Implement parent-source helper**

Add:

```rust
fn bundle_manifest_parent_git_source(git: &GitInstallSource) -> Result<GitInstallSource> {
    if !is_bundle_manifest_file_path(&git.path) {
        return Ok(git.clone());
    }
    let parent = git
        .path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(GitInstallSource {
        url: git.url.clone(),
        reference: git.reference.clone(),
        path: parent,
    })
}
```

- [ ] **Step 3: Implement Git discovery using existing clone boundary**

Implement `discover_git_bundle_manifest_candidates(&GitInstallSource)` by cloning once with `GitClient.clone_checkout`, converting file-path sources to their parent before searching, and loading each candidate from the clone. For each selected manifest directory, compute the source path relative to the clone root and save:

```rust
let resolved_git = GitInstallSource {
    url: base_git.url.clone(),
    reference: base_git.reference.clone(),
    path: selected_git_path,
};
let resolved_source = git_source_to_config_string(&resolved_git);
```

Relative bundle entry sources must continue to resolve relative to the selected manifest directory by reusing `normalize_bundle_entry_source(&entry.source, None, Some(&resolved_git), Path::new("."))`.

- [ ] **Step 4: Run focused bundle tests**

Run:

```bash
cargo test bundle_manifest --quiet
```

Expected: all bundle manifest discovery and source-normalization tests PASS.

## Task 4: Add CLI `--name` and bundle candidate selection

**Files:**
- Modify: `src/cli.rs`
- Test: `src/cli.rs` tests module

- [ ] **Step 1: Write failing clap and selection tests**

In `src/cli.rs` tests module, add parser tests:

```rust
#[test]
fn bundle_add_accepts_name_selector() {
    Cli::try_parse_from([
        "sksync",
        "bundle",
        "add",
        "owner/repo",
        "--name",
        "team-baseline",
        "--agent",
        "pi",
    ])
    .expect("bundle add --name parses");
}

#[test]
fn bundle_inspect_accepts_name_selector() {
    Cli::try_parse_from([
        "sksync",
        "bundle",
        "inspect",
        "owner/repo",
        "--name",
        "team-baseline",
    ])
    .expect("bundle inspect --name parses");
}
```

Also add a pure selector test after importing `BundleManifestCandidate` and helper constructors:

```rust
#[test]
fn bundle_name_selector_matches_manifest_name_or_parent_dir() {
    let selected = select_bundle_manifest_candidates(
        "owner/repo",
        Some("base"),
        vec![
            bundle_candidate_for_test("review-workflow", "bundles/review"),
            bundle_candidate_for_test("team-baseline", "bundles/base"),
        ],
    )
    .expect("select candidate");

    assert_eq!(selected.relative_path, Path::new("bundles/base"));
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test bundle_ --quiet
```

Expected: FAIL because CLI args and selector do not exist.

- [ ] **Step 3: Add `name` fields to bundle args**

Update structs:

```rust
#[derive(Debug, Args)]
struct BundleInspectArgs {
    /// Bundle source directory, repo, or sksync.bundle.json file.
    source: String,
    /// Select one discovered bundle by manifest name or manifest parent directory name.
    #[arg(long)]
    name: Option<String>,
}

#[derive(Debug, Args)]
struct BundleAddArgs {
    /// Bundle source directory, repo, or sksync.bundle.json file.
    source: String,
    /// Select one discovered bundle by manifest name or manifest parent directory name.
    #[arg(long)]
    name: Option<String>,
    // keep existing agents/global/dry_run/force fields
}
```

- [ ] **Step 4: Add CLI selection helper**

Import `BundleManifestCandidate` and `discover_bundle_manifest_candidates`. Add a helper modeled on `select_skill_candidates`:

```rust
fn select_bundle_manifest_candidates(
    source: &str,
    requested_name: Option<&str>,
    candidates: Vec<BundleManifestCandidate>,
) -> Result<BundleManifestCandidate> {
    if candidates.is_empty() {
        bail!("no sksync.bundle.json files found under source '{source}'");
    }

    if let Some(name) = requested_name {
        let matches = candidates
            .into_iter()
            .filter(|candidate| bundle_candidate_matches_name(candidate, name))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [candidate] => Ok(candidate.clone()),
            [] => bail!("no discovered bundle named '{name}' under source '{source}'"),
            _ => bail!("multiple discovered bundles matched '{name}' under source '{source}'"),
        };
    }

    if candidates.len() == 1 {
        return Ok(candidates.into_iter().next().expect("one candidate"));
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "multiple bundles found under source '{source}'; pass --name <bundle> or use a more specific source"
        );
    }

    let choices = candidates.into_iter().map(BundleChoice).collect::<Vec<_>>();
    Ok(inquire::Select::new("Select bundle to add", choices)
        .with_scorer(&score_bundle_choice)
        .prompt()?
        .0)
}
```

Implement `BundleChoice`, `bundle_candidate_matches_name`, and `score_bundle_choice` with path/name/description matching. Keep display compact, for example `team-baseline  bundles/base`.

- [ ] **Step 5: Route inspect/add through discovery and selection**

In `run_bundle_inspect`, replace direct loading:

```rust
let bundle = load_bundle_from_source(&args.source, &current_dir)?;
```

with:

```rust
let candidates = discover_bundle_manifest_candidates(&args.source, &current_dir)?;
let bundle = select_bundle_manifest_candidates(&args.source, args.name.as_deref(), candidates)?
    .into_loaded_bundle();
```

In `run_bundle_add`, use `root_dir` for config-root-relative resolution:

```rust
let candidates = discover_bundle_manifest_candidates(&args.source, &root_dir)?;
let bundle = select_bundle_manifest_candidates(&args.source, args.name.as_deref(), candidates)?
    .into_loaded_bundle();
```

- [ ] **Step 6: Run CLI tests**

Run:

```bash
cargo test bundle_ --quiet
```

Expected: PASS.

## Task 5: Update TUI bundle add to use resolved source once

**Files:**
- Modify: `src/tui/bundle.rs`
- Modify if needed: `src/tui/commands.rs`
- Test: `src/tui/commands.rs` and `src/tui/bundle.rs` tests modules

- [ ] **Step 1: Write a command helper test for resolved source passthrough**

If `bundle_add_args` stays unchanged, extend the existing `bundle_add_args_include_agents_scope_and_dry_run` test to use a discovered source string:

```rust
assert_eq!(
    bundle_add_args(
        "./repo/bundles/base",
        &["pi".to_owned()],
        false,
        true,
    ),
    vec!["bundle", "add", "./repo/bundles/base", "--agent", "pi", "--dry-run"]
);
```

- [ ] **Step 2: Add TUI resolver helper**

In `src/tui/bundle.rs`, import the application resolver:

```rust
use crate::application::bundle::{discover_bundle_manifest_candidates, BundleManifestCandidate};
```

Add a local selection helper:

```rust
fn select_bundle_manifest_candidate(
    source: &str,
    candidates: Vec<BundleManifestCandidate>,
) -> Result<BundleManifestCandidate> {
    match candidates.len() {
        0 => anyhow::bail!("no sksync.bundle.json files found under source '{source}'"),
        1 => Ok(candidates.into_iter().next().expect("one candidate")),
        _ => Select::new("Select bundle to add", candidates)
            .prompt()
            .context("failed to read bundle selection"),
    }
}
```

If `BundleManifestCandidate` cannot implement `Display` in application without UI formatting concerns, wrap it in a TUI-local `BundleManifestChoice` just like CLI wraps skill candidates.

- [ ] **Step 3: Use resolved source for preview, dry-run, and apply**

In `run_add`, replace:

```rust
let bundle = load_bundle_from_source(&source, &root_dir)?;
```

with:

```rust
let candidates = discover_bundle_manifest_candidates(&source, &root_dir)?;
let candidate = select_bundle_manifest_candidate(&source, candidates)?;
let resolved_source = candidate.resolved_source.clone();
let bundle = candidate.into_loaded_bundle();
```

Then change both command calls:

```rust
let dry_run_args = bundle_add_args(&resolved_source, &agents, scope.is_global(), true);
let apply_args = bundle_add_args(&resolved_source, &agents, scope.is_global(), false);
```

- [ ] **Step 4: Run TUI-related tests**

Run:

```bash
cargo test bundle_add_args --quiet
cargo test bundle_provenance_choices --quiet
```

Expected: PASS.

## Task 6: Tighten diagnostics and docs alignment

**Files:**
- Modify: `src/cli.rs`
- Modify if needed: `README.md` only if you intentionally update user-facing examples beyond docs.
- Validate: `docs/DESIGN.md`, `CONTEXT.md`

- [ ] **Step 1: Ensure error messages include candidate guidance**

Update non-TTY and `--name` ambiguity errors to include enough guidance. The minimal acceptable messages are:

```text
multiple bundles found under source '<source>'; pass --name <bundle> or use a more specific source
no discovered bundle named '<name>' under source '<source>'
multiple discovered bundles matched '<name>' under source '<source>'
```

If a candidate list helper is added, print candidate rows as:

```text
- bundles/base (team-baseline)
- bundles/review (review-workflow)
```

- [ ] **Step 2: Check docs still match implementation**

Read `docs/DESIGN.md` section `Bundle manifest discovery` and verify the implementation supports every bullet:

```bash
rg -n "Bundle manifest discovery|--name <bundle>|blob|depth 5|node_modules" docs/DESIGN.md
```

Expected: the section mentions all implemented behavior.

- [ ] **Step 3: Run formatting**

Run:

```bash
cargo fmt --check
```

Expected: PASS. If it fails, run `cargo fmt`, then rerun `cargo fmt --check`.

## Task 7: Full verification

**Files:**
- All modified Rust/docs files

- [ ] **Step 1: Run targeted tests**

Run:

```bash
cargo test bundle_manifest --quiet
cargo test bundle_ --quiet
cargo test github_ --quiet
```

Expected: all PASS.

- [ ] **Step 2: Run full project tests**

Run:

```bash
cargo test --quiet
```

Expected: PASS.

- [ ] **Step 3: Run release-quality checks required by AGENTS.md**

Run:

```bash
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
```

Expected: both PASS.

- [ ] **Step 4: Inspect GitButler diff**

Run:

```bash
but diff
```

Expected: changes are limited to the intended files for this feature plus the already-existing unrelated `scripts/mikan/ready-claude.sh` if it is still present. Do not edit or commit unrelated changes.

- [ ] **Step 5: Commit with GitButler if requested**

Run:

```bash
but status -fv
```

Use the change IDs shown by GitButler for this feature's files only, then commit with a message like:

```bash
but commit <branch-id> -m "feat: discover bundle manifests" --changes <change-id>,<change-id>
```

Do not run raw `git add` or `git commit` in this repository.

## Self-review checklist

- Spec coverage: The plan covers add/inspect/TUI, exact-first resolution, direct manifest files, GitHub blob URLs, depth/skip rules, TTY single select, non-TTY failure, `--name`, resolved provenance, and tests without real network.
- Placeholder scan: No TBD/TODO steps; GitButler IDs are intentionally discovered at execution time via `but status -fv` because they cannot be known before implementation.
- Type consistency: `BundleManifestCandidate`, `discover_bundle_manifest_candidates`, and `into_loaded_bundle` are introduced before CLI/TUI tasks consume them.
