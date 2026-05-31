# Skill Include Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `include` packaging filters so sksync can install only selected files from a resolved skill package root, including root-level `SKILL.md`-only skills such as `github:ogulcancelik/herdr`.

**Architecture:** Keep source identity and packaging rules separate. Add a small domain type for include filters, thread it through dependency config, bundle manifests, installer inputs, lockfile v5, and check diagnostics. Preserve current full-directory copying when `include` is absent.

**Tech Stack:** Rust 2021, clap, serde/serde_json, VitePress docs, JSON Schema, existing sksync config/lockfile/bundle/install architecture.

---

## Published issues

- Parent: [#142](https://github.com/takemo101/sksync/issues/142) Implement skill package include filters
- [#143](https://github.com/takemo101/sksync/issues/143) Add include filter config model
- [#144](https://github.com/takemo101/sksync/issues/144) Install filtered skill packages
- [#145](https://github.com/takemo101/sksync/issues/145) Persist include filters from add
- [#146](https://github.com/takemo101/sksync/issues/146) Record include filters in lockfile v5 and check
- [#147](https://github.com/takemo101/sksync/issues/147) Propagate include filters through bundles
- [#148](https://github.com/takemo101/sksync/issues/148) Cover include filters end to end
- [#149](https://github.com/takemo101/sksync/issues/149) Document include packaging filters
- [#150](https://github.com/takemo101/sksync/issues/150) Verify include packaging release readiness

## File map

- Create: `src/domain/package_filter.rs`
  - Owns normalized include patterns and validates path safety.
- Modify: `src/domain/mod.rs`
  - Exports the new domain module.
- Modify: `src/application/config.rs`
  - Adds optional include metadata to `ResolvedSkill`.
- Modify: `src/application/ports.rs`
  - Changes installer input from only `InstallSource` to an install request carrying source + include.
  - Adds include mismatch reporting types if needed for `check`.
- Modify: `src/application/update.rs`
  - Passes include filters to the installer and records them in update reports.
- Modify: `src/application/check.rs`
  - Fails when config include differs from lockfile include.
- Modify: `src/application/add.rs`
  - Carries include metadata during add workflow.
- Modify: `src/application/bundle.rs`
  - Carries include through loaded bundle entries, add/sync/export plans, and source-drift comparisons.
- Modify: `src/infrastructure/install.rs`
  - Implements filtered copy, limited glob matching, directory recursion, protected directory exclusion, and staged-package validation.
- Modify: `src/infrastructure/json.rs`
  - Reads/writes config include, bundle entry include, lockfile v5 include, and dependency store updates.
- Modify: `src/domain/lockfile.rs`
  - Bumps supported lockfile version to 5 and adds optional include to `LockedSkill`.
- Modify: `src/cli.rs`
  - Adds `sksync add --include <pattern>` and `--manifest-only`.
  - Threads include through add and bundle flows.
  - Updates plan/print output where include drift must be shown.
- Modify: `schemas/sksync.schema.json`, `schemas/sksync.bundle.schema.json`, `schemas/sksync-lock.schema.json`
  - Documents and validates include fields and lockfile v5.
- Modify: `README.md`, `site/guides/sources.md`, `site/guides/bundles.md`, `site/guides/lockfile.md`, `site/reference/commands.md`, `docs/DESIGN.md`
  - Documents include filters, manifest-only shortcut, bundle behavior, and lockfile v5.
- Modify/add integration tests under `tests/`
  - Covers CLI behavior end-to-end.

---

## Task 1: Add include filter domain model and config parsing

**Files:**
- Create: `src/domain/package_filter.rs`
- Modify: `src/domain/mod.rs`
- Modify: `src/application/config.rs`
- Modify: `src/infrastructure/json.rs`
- Modify: `schemas/sksync.schema.json`

- [ ] **Step 1: Write failing domain tests for include validation**

Add tests in `src/domain/package_filter.rs` for:

```rust
#[cfg(test)]
mod tests {
    use super::{PackageFilter, PackagePatternError};

    #[test]
    fn include_patterns_are_normalized_and_sorted() {
        let filter = PackageFilter::new(vec!["references".into(), "SKILL.md".into()]).unwrap();
        assert_eq!(filter.patterns(), &["SKILL.md", "references"]);
    }

    #[test]
    fn include_rejects_empty_patterns() {
        let error = PackageFilter::new(vec![" ".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::Empty);
    }

    #[test]
    fn include_rejects_absolute_paths() {
        let error = PackageFilter::new(vec!["/tmp/SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::Absolute);
    }

    #[test]
    fn include_rejects_parent_components() {
        let error = PackageFilter::new(vec!["../SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::ParentComponent);
    }
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test --quiet package_filter
```

Expected: compile/test failures because `PackageFilter` does not exist yet.

- [ ] **Step 3: Implement `PackageFilter`**

Create `src/domain/package_filter.rs`:

```rust
use std::path::{Component, Path};

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PackagePatternError {
    #[error("include pattern must not be empty")]
    Empty,
    #[error("include pattern must be relative")]
    Absolute,
    #[error("include pattern must not contain '..'")]
    ParentComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageFilter {
    patterns: Vec<String>,
}

impl PackageFilter {
    pub fn new(patterns: Vec<String>) -> Result<Self, PackagePatternError> {
        let mut normalized = Vec::new();
        for pattern in patterns {
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                return Err(PackagePatternError::Empty);
            }
            let path = Path::new(trimmed);
            if path.is_absolute() {
                return Err(PackagePatternError::Absolute);
            }
            if path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            {
                return Err(PackagePatternError::ParentComponent);
            }
            normalized.push(trimmed.replace('\\', "/"));
        }
        normalized.sort();
        normalized.dedup();
        Ok(Self { patterns: normalized })
    }

    pub fn manifest_only() -> Self {
        Self {
            patterns: vec!["SKILL.md".to_owned()],
        }
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}
```

Update `src/domain/mod.rs`:

```rust
pub mod package_filter;
```

- [ ] **Step 4: Thread optional include into resolved config**

Modify `src/application/config.rs`:

```rust
use crate::domain::package_filter::PackageFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: SkillName,
    pub source: SourcePath,
    pub install_source: Option<InstallSource>,
    pub include: Option<PackageFilter>,
    pub agents: Vec<AgentKind>,
}
```

Update tests and constructors that build `ResolvedSkill` to pass `include: None`.

- [ ] **Step 5: Parse `dependencies.*.include` from config**

Modify `RawDependencyConfig` in `src/infrastructure/json.rs`:

```rust
pub struct RawDependencyConfig {
    pub source: RawInstallSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub bundles: Vec<RawBundleProvenance>,
    #[serde(default)]
    pub managed_by_bundles: bool,
}
```

When resolving dependencies into `ResolvedSkill`, build:

```rust
let include = raw_dependency
    .include
    .clone()
    .map(PackageFilter::new)
    .transpose()
    .map_err(|error| ConfigResolveError::InvalidInstallSource {
        skill: name.clone(),
        message: error.to_string(),
    })?;
```

Then set `include` on `ResolvedSkill`.

- [ ] **Step 6: Update config schema**

In `schemas/sksync.schema.json`, add dependency property:

```json
"include": {
  "$ref": "#/$defs/includePatterns",
  "description": "Optional package filter evaluated relative to the resolved skill package root. When absent, the whole package is copied."
}
```

Add definition:

```json
"includePatterns": {
  "type": "array",
  "minItems": 1,
  "items": {
    "type": "string",
    "minLength": 1,
    "pattern": "^(?!/)(?!.*(?:^|/)\\.\\.(?:/|$)).*\\S.*$"
  },
  "uniqueItems": true
}
```

- [ ] **Step 7: Run focused tests**

Run:

```bash
cargo test --quiet package_filter
cargo test --quiet parses_dependency
```

Expected: all touched config/domain tests pass.

- [ ] **Step 8: Commit Task 1**

Use GitButler in the main workspace, or normal git only in a temporary clone:

```bash
but commit skill-include-packaging -c -m "Add include filter config model" --changes <ids>
```

---

## Task 2: Add filtered installer copy behavior

**Files:**
- Modify: `src/application/ports.rs`
- Modify: `src/application/update.rs`
- Modify: `src/application/add.rs`
- Modify: `src/infrastructure/install.rs`

- [ ] **Step 1: Write failing installer tests**

Add tests in `src/infrastructure/install.rs`:

```rust
#[test]
fn install_with_manifest_only_copies_only_skill_md() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote");
    let destination = temp.path().join("installed");
    std::fs::create_dir_all(remote.join("src")).unwrap();
    std::fs::write(remote.join("SKILL.md"), skill_md("herdr", "Herdr helper")).unwrap();
    std::fs::write(remote.join("Cargo.toml"), "[package]\nname = \"herdr\"\n").unwrap();
    std::fs::write(remote.join("src/main.rs"), "fn main() {}\n").unwrap();

    let request = SkillInstallRequest {
        source: InstallSource::Local(remote),
        include: Some(PackageFilter::manifest_only()),
    };
    FileSystemSkillInstaller
        .install_skill(&request, &destination, "herdr")
        .unwrap();

    assert!(destination.join("SKILL.md").is_file());
    assert!(!destination.join("Cargo.toml").exists());
    assert!(!destination.join("src").exists());
}

#[test]
fn install_with_directory_include_recursively_copies_references() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote");
    let destination = temp.path().join("installed");
    std::fs::create_dir_all(remote.join("references/nested")).unwrap();
    std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
    std::fs::write(remote.join("references/nested/guide.md"), "guide").unwrap();

    let request = SkillInstallRequest {
        source: InstallSource::Local(remote),
        include: Some(PackageFilter::new(vec!["SKILL.md".into(), "references".into()]).unwrap()),
    };
    FileSystemSkillInstaller
        .install_skill(&request, &destination, "review")
        .unwrap();

    assert!(destination.join("SKILL.md").is_file());
    assert!(destination.join("references/nested/guide.md").is_file());
}

#[test]
fn include_pattern_must_match() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote");
    let destination = temp.path().join("installed");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();

    let request = SkillInstallRequest {
        source: InstallSource::Local(remote),
        include: Some(PackageFilter::new(vec!["missing".into()]).unwrap()),
    };
    let error = FileSystemSkillInstaller
        .install_skill(&request, &destination, "review")
        .unwrap_err();

    assert!(error.to_string().contains("include pattern matched no files"));
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test --quiet install_with_manifest_only_copies_only_skill_md
cargo test --quiet install_with_directory_include_recursively_copies_references
cargo test --quiet include_pattern_must_match
```

Expected: compile failures because `SkillInstallRequest` and filtered copy do not exist.

- [ ] **Step 3: Add installer request type**

Modify `src/application/ports.rs`:

```rust
use crate::domain::package_filter::PackageFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallRequest {
    pub source: InstallSource,
    pub include: Option<PackageFilter>,
}

pub trait SkillInstaller {
    fn install_skill(
        &self,
        request: &SkillInstallRequest,
        destination: &Path,
        skill_name: &str,
    ) -> Result<InstalledSkillSource, SkillInstallError>;
}
```

Update fake installers in tests to accept `SkillInstallRequest` and use `request.source.clone()` where they previously used `source.clone()`.

- [ ] **Step 4: Pass include from update**

Modify `src/application/update.rs`:

```rust
let request = SkillInstallRequest {
    source: install_source.clone(),
    include: skill.include.clone(),
};
let installed = installer.install_skill(&request, &destination, skill.name.as_str())?;
```

- [ ] **Step 5: Implement filtered local/git installation**

Modify `src/infrastructure/install.rs` so `install_source_to_staging` receives `&SkillInstallRequest`.

For local sources:

```rust
copy_package_contents(path, staging, request.include.as_ref())?;
```

For git sources, after resolving `source_path`:

```rust
copy_package_contents(&source_path, staging, request.include.as_ref())?;
```

Add helpers:

```rust
const PROTECTED_DIRS: &[&str] = &[".git", ".sksync", "node_modules"];

fn copy_package_contents(
    from: &Path,
    to: &Path,
    include: Option<&PackageFilter>,
) -> Result<(), SkillInstallError> {
    match include {
        None => copy_dir_contents(from, to),
        Some(filter) => copy_filtered_contents(from, to, filter),
    }
}
```

Implement `copy_filtered_contents` with these rules:

- Resolve each pattern against `from`.
- Literal file path copies one file.
- Literal directory path recursively copies the directory.
- Glob patterns support `*` inside one path segment and terminal `/**` for recursive directory matches.
- Each pattern must match at least once.
- Skip protected dirs while walking.
- Preserve relative paths under `to`.
- Never copy paths whose canonical path does not start with canonical `from`.

- [ ] **Step 6: Run installer tests**

Run:

```bash
cargo test --quiet install_with_manifest_only_copies_only_skill_md
cargo test --quiet install_with_directory_include_recursively_copies_references
cargo test --quiet include_pattern_must_match
cargo test --quiet invalid_skill_package
```

Expected: all pass.

- [ ] **Step 7: Commit Task 2**

```bash
but commit skill-include-packaging -m "Install filtered skill packages" --changes <ids>
```

---

## Task 3: Add CLI include flags and persist dependency metadata

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/application/add.rs`
- Modify: `src/application/ports.rs`
- Modify: `src/infrastructure/json.rs`

- [ ] **Step 1: Write parser tests**

Add tests in `src/cli.rs`:

```rust
#[test]
fn add_include_flags_parse() {
    Cli::try_parse_from([
        "sksync",
        "add",
        "ogulcancelik/herdr",
        "--agent",
        "pi",
        "--include",
        "SKILL.md",
        "--include",
        "references",
    ])
    .expect("add --include parses");
}

#[test]
fn add_manifest_only_parses() {
    Cli::try_parse_from([
        "sksync",
        "add",
        "ogulcancelik/herdr",
        "--agent",
        "pi",
        "--manifest-only",
    ])
    .expect("add --manifest-only parses");
}

#[test]
fn add_manifest_only_conflicts_with_include() {
    Cli::try_parse_from([
        "sksync",
        "add",
        "ogulcancelik/herdr",
        "--agent",
        "pi",
        "--manifest-only",
        "--include",
        "SKILL.md",
    ])
    .expect_err("manifest-only conflicts with include");
}
```

- [ ] **Step 2: Update `AddArgs`**

Add fields:

```rust
#[arg(long = "include", value_name = "pattern", conflicts_with = "manifest_only")]
include: Vec<String>,
#[arg(long, conflicts_with = "include")]
manifest_only: bool,
```

- [ ] **Step 3: Convert CLI flags to package filter**

Add helper in `src/cli.rs`:

```rust
fn package_filter_from_add_args(args: &AddArgs) -> Result<Option<PackageFilter>> {
    if args.manifest_only {
        return Ok(Some(PackageFilter::manifest_only()));
    }
    if args.include.is_empty() {
        return Ok(None);
    }
    Ok(Some(PackageFilter::new(args.include.clone())?))
}
```

- [ ] **Step 4: Replace dependency add parameters with options struct**

Modify `src/application/ports.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddDependencyOptions {
    pub include: Option<PackageFilter>,
}
```

Change trait method:

```rust
fn add_dependency(
    &self,
    skill_name: &str,
    source: &str,
    agents: &[String],
    options: AddDependencyOptions,
) -> Result<(), DependencyConfigStoreError>;
```

Update call sites with `AddDependencyOptions { include: None }` until include is threaded.

- [ ] **Step 5: Persist include in config**

In `FileDependencyConfigStore::add_dependency`, when `options.include` is `Some(filter)`, insert:

```rust
object.insert("include".to_owned(), json!(filter.patterns()));
```

When `None`, remove stale include:

```rust
object.remove("include");
```

- [ ] **Step 6: Thread include through add workflow**

Update `AddSelection`:

```rust
pub struct AddSelection {
    pub skill_name: String,
    pub source: String,
    pub include: Option<PackageFilter>,
}
```

In `run_add_workflow`, pass `AddDependencyOptions { include: selection.include.clone() }`.

In `run_add`, set each selection's include from `package_filter_from_add_args(&args)?`.

- [ ] **Step 7: Run focused tests**

Run:

```bash
cargo test --quiet add_include_flags_parse
cargo test --quiet add_manifest_only
cargo test --quiet add_dependency
cargo test --quiet run_add_workflow
```

Expected: all pass.

- [ ] **Step 8: Commit Task 3**

```bash
but commit skill-include-packaging -m "Persist include filters from add" --changes <ids>
```

---

## Task 4: Add lockfile v5 include support and check mismatch diagnostics

**Files:**
- Modify: `src/domain/lockfile.rs`
- Modify: `src/infrastructure/json.rs`
- Modify: `src/cli.rs`
- Modify: `src/application/check.rs`
- Modify: `schemas/sksync-lock.schema.json`

- [ ] **Step 1: Write lockfile v5 tests**

Add tests in `src/infrastructure/json.rs`:

```rust
#[test]
fn writes_lockfile_v5_with_include() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sksync-lock.json");
    let mut skills = BTreeMap::new();
    skills.insert(
        SkillName::new("herdr").unwrap(),
        LockedSkill {
            source: SourcePath::new(".sksync/skills/ogulcancelik/herdr/herdr").unwrap(),
            install_source: Some(InstallSource::Git(GitInstallSource {
                url: "https://github.com/ogulcancelik/herdr.git".into(),
                reference: Some("abc123".into()),
                path: PathBuf::from("."),
            })),
            include: Some(PackageFilter::manifest_only()),
            hash: Digest::new("hash").unwrap(),
            files: vec![],
            targets: vec![],
        },
    );
    let lockfile = Lockfile {
        generated_by: "sksync@test".into(),
        generated_at: "2026-05-31T00:00:00Z".into(),
        root: PathBuf::from("."),
        skills,
    };
    write_lockfile(&path, &lockfile).unwrap();
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["lockfileVersion"], 5);
    assert_eq!(value["skills"]["herdr"]["include"], json!(["SKILL.md"]));
}

#[test]
fn reads_v4_lockfile_without_include_as_full_package() {
    // Use an inline v4 fixture with no include and assert locked_skill.include == None.
}
```

- [ ] **Step 2: Update lockfile domain**

Modify `src/domain/lockfile.rs`:

```rust
pub const SUPPORTED_LOCKFILE_VERSION: u32 = 5;
pub const LEGACY_LOCKFILE_VERSION_V4: u32 = 4;
```

Add to `LockedSkill`:

```rust
pub include: Option<PackageFilter>,
```

Keep existing v2/v3 constants and readers. Treat v4 as readable legacy.

- [ ] **Step 3: Serialize/deserialize include**

Modify `RawLockedSkill`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
include: Option<Vec<String>>,
```

Convert with `PackageFilter::new` on read and `filter.patterns().to_vec()` on write.

- [ ] **Step 4: Include filters when building lockfiles**

In `build_lockfile_from_plan` in `src/cli.rs`, set:

```rust
include: skill.include.clone(),
```

- [ ] **Step 5: Add check mismatch test**

In `src/application/check.rs`, add a diagnostic when `ResolvedSkill.include != LockedSkill.include`.

Test shape:

```rust
#[test]
fn check_reports_include_mismatch() {
    let mut config = config_with_skill("herdr");
    config.skills[0].include = Some(PackageFilter::manifest_only());
    let mut lockfile = lockfile();
    lockfile.skills.get_mut(&SkillName::new("review").unwrap()).unwrap().include = None;

    let report = check_config_against_lockfile(&config, Some(&lockfile), &hashes, &links, &targets).unwrap();
    assert!(report.problems.iter().any(|problem| problem.to_string().contains("include")));
}
```

Use the existing check test helpers and adapt exact names to the file.

- [ ] **Step 6: Update lockfile schema to v5**

In `schemas/sksync-lock.schema.json`:

- change `lockfileVersion.const` to `5`
- add optional `include` under `lockedSkill.properties`
- reuse the same include pattern definition as config schema

- [ ] **Step 7: Run lockfile/check tests**

Run:

```bash
cargo test --quiet lockfile
cargo test --quiet include_mismatch
cargo test --quiet check
```

Expected: all pass.

- [ ] **Step 8: Commit Task 4**

```bash
but commit skill-include-packaging -m "Record include filters in lockfile v5" --changes <ids>
```

---

## Task 5: Add bundle include propagation and sync drift

**Files:**
- Modify: `src/domain/bundle.rs`
- Modify: `src/application/bundle.rs`
- Modify: `src/infrastructure/json.rs`
- Modify: `src/cli.rs`
- Modify: `schemas/sksync.bundle.schema.json`

- [ ] **Step 1: Write bundle manifest parse tests**

In `src/infrastructure/json.rs`, add tests that read:

```json
{
  "name": "agent-tools",
  "description": "Shared tools",
  "entries": {
    "herdr": {
      "source": "github:ogulcancelik/herdr#main",
      "include": ["SKILL.md"]
    }
  }
}
```

Assert the resulting `BundleEntry` has `include: Some(PackageFilter::manifest_only())`.

- [ ] **Step 2: Add include to bundle domain types**

Modify `BundleEntry`:

```rust
pub struct BundleEntry {
    pub skill_name: SkillName,
    pub source: String,
    pub include: Option<PackageFilter>,
}
```

Modify loaded/plan item structs in `src/application/bundle.rs` to carry `include: Option<PackageFilter>`.

- [ ] **Step 3: Parse/write bundle entry include**

Modify `RawBundleEntry`:

```rust
pub struct RawBundleEntry {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
}
```

Read with `PackageFilter::new`; write with normalized patterns.

- [ ] **Step 4: Store include during bundle add**

Update `BundleAddPlanItem` with include and pass it into dependency creation:

```json
{
  "source": item.source,
  "include": item.include.as_ref().map(|filter| filter.patterns()),
  "agents": item.agents,
  "bundles": [...],
  "managedByBundles": true
}
```

When merging/adopting an existing dependency, include must match. If source matches but include differs, plan status is `Conflict` for bundle add.

- [ ] **Step 5: Detect include drift during bundle sync**

Extend sync comparison so same source but different include produces a drift item. The dry-run output should mention include drift, for example:

```text
update herdr (include changed: local <full package>, manifest SKILL.md)
```

Non-dry-run sync updates dependency include to the manifest include.

- [ ] **Step 6: Export include in manifest-only bundle export**

When `bundle export` uses manifest-only mode, include dependency filters in exported entries. When `--snapshot` is used, omit include because sources point at already-filtered snapshot directories.

- [ ] **Step 7: Update bundle schema**

In `schemas/sksync.bundle.schema.json`, add `include` to `bundleEntry.properties` with the shared include pattern rules.

- [ ] **Step 8: Run bundle tests**

Run:

```bash
cargo test --quiet bundle_include
cargo test --quiet bundle_add
cargo test --quiet bundle_sync
cargo test --quiet bundle_export
```

Expected: all pass.

- [ ] **Step 9: Commit Task 5**

```bash
but commit skill-include-packaging -m "Propagate include filters through bundles" --changes <ids>
```

---

## Task 6: Add end-to-end CLI tests for root SKILL.md and update/install preservation

**Files:**
- Modify: `tests/bundle_cli.rs` or create `tests/include_cli.rs`
- Modify: any existing test helpers used by integration tests

- [ ] **Step 1: Create local git fixture helper**

In `tests/include_cli.rs`, create helper:

```rust
fn init_git_repo(path: &Path) {
    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
}

fn commit_all(path: &Path, message: &str) {
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", message]);
}
```

Reuse existing command-runner helpers from integration tests if present instead of duplicating large helpers.

- [ ] **Step 2: Test `--manifest-only` root skill install**

Write an integration test that creates a local git repo with:

```text
SKILL.md
Cargo.toml
src/main.rs
```

Run:

```bash
sksync init
sksync add <repo-path> --name herdr -a pi --manifest-only
```

Assert:

```text
.sksync/skills/<storage>/herdr/SKILL.md exists
Cargo.toml does not exist under installed skill
src/main.rs does not exist under installed skill
sksync-lock.json has lockfileVersion 5
sksync-lock.json has include ["SKILL.md"]
```

- [ ] **Step 3: Test `--include SKILL.md --include references`**

Create a repo with:

```text
SKILL.md
references/guide.md
assets/logo.png
```

Run:

```bash
sksync add <repo-path> -a pi --include SKILL.md --include references
```

Assert `SKILL.md` and `references/guide.md` exist, while `assets/logo.png` does not.

- [ ] **Step 4: Test update preserves filtered install**

After Task 2's fixture is installed, add `README.md` to upstream repo and commit. Run:

```bash
sksync update
```

Assert `README.md` is not copied into the installed skill.

- [ ] **Step 5: Test install reconstructs filtered install**

Delete the installed skill directory, then run:

```bash
sksync install
```

Assert only included files are reconstructed.

- [ ] **Step 6: Run integration tests**

Run:

```bash
cargo test --quiet --test include_cli
cargo test --quiet
```

Expected: all pass.

- [ ] **Step 7: Commit Task 6**

```bash
but commit skill-include-packaging -m "Cover include filters end to end" --changes <ids>
```

---

## Task 7: Document include packaging

**Files:**
- Modify: `README.md`
- Modify: `site/guides/sources.md`
- Modify: `site/guides/bundles.md`
- Modify: `site/guides/lockfile.md`
- Modify: `site/reference/commands.md`
- Modify: `docs/DESIGN.md`

- [ ] **Step 1: Update README source docs**

Add examples:

```bash
sksync add ogulcancelik/herdr --name herdr -a pi --manifest-only
sksync add org/repo/skills/review -a pi --include SKILL.md --include references
```

Explain:

- include patterns are relative to resolved skill package root
- missing include means full package copy
- each include pattern must match
- final staged package must contain valid `SKILL.md`

- [ ] **Step 2: Update bundle docs**

Add manifest example:

```json
{
  "entries": {
    "herdr": {
      "source": "github:ogulcancelik/herdr#main",
      "include": ["SKILL.md"]
    }
  }
}
```

Explain bundle add/sync/export behavior.

- [ ] **Step 3: Update lockfile docs**

Document lockfile v5 and optional `include` on locked skills.

- [ ] **Step 4: Update command reference**

Add flags to `sksync add`:

| Flag | Meaning |
|---|---|
| `--include <pattern>` | Copy only matched files/directories from the resolved skill package root. Repeatable. |
| `--manifest-only` | Shortcut for `--include SKILL.md`. |

- [ ] **Step 5: Build docs**

Run:

```bash
bun install
bun run docs:build
```

Expected: VitePress build succeeds.

- [ ] **Step 6: Commit Task 7**

```bash
but commit skill-include-packaging -m "Document include packaging filters" --changes <ids>
```

---

## Task 8: Full verification and release-readiness cleanup

**Files:**
- Modify only files needed to fix verification failures.

- [ ] **Step 1: Run formatter**

```bash
cargo fmt --check
```

Expected: pass. If it fails, run `cargo fmt`, inspect the diff, and commit formatting with the relevant task commit or a final formatting commit.

- [ ] **Step 2: Run full tests**

```bash
cargo test --quiet
```

Expected: all tests pass.

- [ ] **Step 3: Run release build**

```bash
cargo build --release --quiet
```

Expected: build succeeds.

- [ ] **Step 4: Run clippy**

```bash
cargo clippy --quiet -- -D warnings
```

Expected: pass with no warnings.

- [ ] **Step 5: Run docs build**

```bash
bun run docs:build
```

Expected: VitePress build succeeds.

- [ ] **Step 6: Manual smoke test with herdr-style repo**

Use a temp project and a local git repo with root `SKILL.md` plus unrelated project files:

```bash
sksync init
sksync add /tmp/herdr-like --name herdr -a pi --manifest-only
find .sksync/skills -maxdepth 5 -type f | sort
```

Expected: installed skill contains `SKILL.md` and does not contain unrelated project files.

- [ ] **Step 7: Open PR and request review**

PR body includes:

```markdown
## Summary
- add include filters for dependency and bundle packaging
- add manifest-only shortcut for root-level SKILL.md skills
- write lockfile v5 include metadata
- document include behavior

## Verification
- cargo fmt --check
- cargo test --quiet
- cargo build --release --quiet
- cargo clippy --quiet -- -D warnings
- bun run docs:build
- manual herdr-style smoke test
```

- [ ] **Step 8: Merge and sync GitButler workspace**

After PR merge:

```bash
but pull --check
but pull --status-after
```

---

## Self-review

- Spec coverage: all design requirements are covered by tasks: config shape (Task 1), CLI shape (Task 3), include pattern semantics (Task 2), bundle manifest behavior (Task 5), lockfile v5 (Task 4), install/update/check impact (Tasks 2/4/6), docs/schema updates (Tasks 1/4/5/7), and verification (Task 8).
- Placeholder scan: no unfinished placeholder markers remain. Each task contains concrete file paths, test names, commands, and expected behavior.
- Type consistency: the plan consistently uses `PackageFilter`, `SkillInstallRequest`, `AddDependencyOptions`, `include: Option<PackageFilter>`, and lockfile v5 fields.
