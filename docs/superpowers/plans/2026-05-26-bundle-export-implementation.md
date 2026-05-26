# Bundle Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `sksync bundle export` so users can turn the current project or global dependency set into a shareable `sksync.bundle.json`, optionally copying installed skill bodies into a snapshot bundle.

**Architecture:** Reuse the existing bundle domain and JSON infrastructure. Add a small export planning layer in `src/application/bundle.rs`, raw dependency-source extraction in `src/infrastructure/json.rs`, and CLI orchestration in `src/cli.rs`. Manifest-only export writes only `sksync.bundle.json`; snapshot export validates and stages copied skill bodies before atomically replacing the output directory.

**Tech Stack:** Rust, clap, serde/serde_json, existing sksync application/domain/infrastructure layers, tempfile-based integration tests, VitePress docs.

---

## File map

- Modify `src/domain/bundle.rs`: add optional helper constructor for export manifests if useful.
- Modify `src/application/bundle.rs`: add export mode, export plan types, source conversion helpers, snapshot validation/copy planning helpers.
- Modify `src/infrastructure/json.rs`: add read-only raw dependency export API and bundle manifest writer.
- Modify `src/cli.rs`: register `sksync bundle export`, plan/print/apply export, rollback staging artifacts.
- Modify `tests/bundle_cli.rs`: add integration tests for manifest-only export, snapshot export, dry-run, subsets, overwrite safety, and global scope.
- Modify `README.md`, `docs/DESIGN.md`, `docs/ROADMAP.md`, `site/guides/bundles.md`, `site/reference/commands.md`: change `bundle export` from planned to implemented and add usage examples.

---

## Issue 1: Register the `bundle export` CLI shape

**Goal:** Make clap accept the final command shape without implementing behavior yet.

**Files:**
- Modify: `src/cli.rs`

**Steps:**

- [ ] Add a failing parser test near existing CLI registration tests in `src/cli.rs`:

```rust
#[test]
fn bundle_export_command_is_registered() {
    Cli::try_parse_from([
        "sksync",
        "bundle",
        "export",
        "team-baseline",
        "--output",
        "./bundles/team-baseline",
        "--skill",
        "review",
        "--snapshot",
        "--dry-run",
        "--force",
    ])
    .expect("bundle export should parse");
}
```

- [ ] Run the test and verify it fails because `export` is not a bundle subcommand:

```sh
cargo test --quiet bundle_export_command_is_registered
```

Expected failure: clap rejects `export` as an unknown subcommand.

- [ ] Add `Export(BundleExportArgs)` to `BundleCommand` in `src/cli.rs`:

```rust
enum BundleCommand {
    Inspect(BundleInspectArgs),
    Add(BundleAddArgs),
    Remove(BundleRemoveArgs),
    Export(BundleExportArgs),
}
```

- [ ] Add `BundleExportArgs` in `src/cli.rs`:

```rust
#[derive(Debug, Args)]
struct BundleExportArgs {
    /// Bundle name to write into sksync.bundle.json.
    name: String,
    /// Output directory that will contain sksync.bundle.json.
    #[arg(long)]
    output: PathBuf,
    /// Export from ~/.sksync/config.json instead of project config.
    #[arg(long)]
    global: bool,
    /// Copy currently installed skill bodies into the bundle directory.
    #[arg(long)]
    snapshot: bool,
    /// Export only selected dependency names. Repeatable.
    #[arg(long = "skill")]
    skills: Vec<String>,
    /// Show the export plan without writing files.
    #[arg(long)]
    dry_run: bool,
    /// Replace an existing generated output directory.
    #[arg(long)]
    force: bool,
}
```

- [ ] Wire dispatch with a placeholder implementation that returns an explicit error:

```rust
fn run_bundle(args: BundleArgs) -> Result<()> {
    match args.command {
        BundleCommand::Inspect(args) => run_bundle_inspect(args),
        BundleCommand::Add(args) => run_bundle_add(args),
        BundleCommand::Remove(args) => run_bundle_remove(args),
        BundleCommand::Export(args) => run_bundle_export(args),
    }
}

fn run_bundle_export(_args: BundleExportArgs) -> Result<()> {
    bail!("bundle export is not implemented yet")
}
```

- [ ] Run the parser test again:

```sh
cargo test --quiet bundle_export_command_is_registered
```

Expected: pass.

**Acceptance criteria:** CLI syntax is registered exactly as designed, but no file writes are implemented in this issue.

---

## Issue 2: Add read-only dependency export extraction

**Goal:** Read dependency names and source strings from config without agents, bundle provenance, or lockfile state.

**Files:**
- Modify: `src/infrastructure/json.rs`
- Test: `src/infrastructure/json.rs`

**Steps:**

- [ ] Add failing tests in `src/infrastructure/json.rs` for a read-only export API:

```rust
#[test]
fn bundle_export_dependencies_preserve_shorthand_sources_and_ignore_provenance() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("sksync.config.json");
    std::fs::write(
        &config_path,
        r#"{
          "dependencies": {
            "review": {
              "source": "github:org/repo/skills/review#main",
              "agents": ["pi"],
              "bundles": [{ "name": "baseline", "source": "./bundle" }],
              "managedByBundles": true
            }
          }
        }"#,
    )
    .expect("write config");
    let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

    let dependencies = store.load_bundle_export_dependencies().unwrap();

    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].name, "review");
    assert_eq!(dependencies[0].source, "github:org/repo/skills/review#main");
}
```

```rust
#[test]
fn bundle_export_dependencies_convert_structured_git_sources_to_tree_urls() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("sksync.config.json");
    std::fs::write(
        &config_path,
        r#"{
          "dependencies": {
            "review": {
              "source": {
                "provider": "git",
                "repo": "org/repo",
                "path": "skills/review",
                "ref": "main"
              },
              "agents": ["pi"]
            }
          }
        }"#,
    )
    .expect("write config");
    let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

    let dependencies = store.load_bundle_export_dependencies().unwrap();

    assert_eq!(dependencies[0].source, "https://github.com/org/repo/tree/main/skills/review");
}
```

- [ ] Run the tests and verify they fail because the API does not exist:

```sh
cargo test --quiet bundle_export_dependencies
```

- [ ] Add the export DTO near `FileDependencyConfigStore`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportDependencyConfig {
    pub name: String,
    pub source: String,
}
```

- [ ] Add `FileDependencyConfigStore::load_bundle_export_dependencies`:

```rust
pub fn load_bundle_export_dependencies(
    &self,
) -> Result<Vec<BundleExportDependencyConfig>, DependencyConfigStoreError> {
    let value = self.load_or_default()?;
    let empty_dependencies = serde_json::Map::new();
    let dependencies = value
        .get("dependencies")
        .and_then(|dependencies| dependencies.as_object())
        .unwrap_or(&empty_dependencies);
    let mut exported = Vec::with_capacity(dependencies.len());
    for (name, dependency) in dependencies {
        SkillName::new(name.clone()).map_err(|source| {
            DependencyConfigStoreError::InvalidField(format!(
                "invalid dependency name '{name}': {source}"
            ))
        })?;
        exported.push(BundleExportDependencyConfig {
            name: name.clone(),
            source: dependency_export_source(dependency, name)?,
        });
    }
    exported.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(exported)
}
```

- [ ] Add `dependency_export_source`:

```rust
fn dependency_export_source(
    dependency: &serde_json::Value,
    skill_name: &str,
) -> Result<String, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let source = dependency.get("source").ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is required"
        ))
    })?;
    if let Some(source) = source.as_str() {
        parse_source_string(source).map_err(|error| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name}.source is invalid: {error}"
            ))
        })?;
        return Ok(source.to_owned());
    }
    let raw = serde_json::from_value::<RawInstallSource>(source.clone()).map_err(|error| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is invalid: {error}"
        ))
    })?;
    let install_source = parse_install_source(skill_name, raw, None).map_err(|error| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is invalid: {error}"
        ))
    })?;
    Ok(match install_source {
        InstallSource::Git(git) => crate::application::bundle::git_source_to_config_string(&git),
        InstallSource::Local(path) => path.to_string_lossy().replace('\\', "/"),
    })
}
```

- [ ] Run tests:

```sh
cargo test --quiet bundle_export_dependencies
```

Expected: pass.

**Acceptance criteria:** export dependency extraction is read-only, deterministic, ignores agents/provenance, preserves string sources, and converts structured sources to manifest-compatible strings.

---

## Issue 3: Add bundle manifest writing

**Goal:** Write `BundleManifest` as strict `sksync.bundle.json` with deterministic entry order and schema reference.

**Files:**
- Modify: `src/infrastructure/json.rs`
- Test: `src/infrastructure/json.rs`

**Steps:**

- [ ] Add failing test:

```rust
#[test]
fn write_bundle_manifest_outputs_schema_and_sorted_entries() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("sksync.bundle.json");
    let manifest = BundleManifest {
        name: BundleName::new("team-baseline").unwrap(),
        description: "Exported from sksync project config.".to_owned(),
        entries: vec![
            BundleEntry {
                skill_name: SkillName::new("qa").unwrap(),
                source: "./skills/qa".to_owned(),
            },
            BundleEntry {
                skill_name: SkillName::new("review").unwrap(),
                source: "./skills/review".to_owned(),
            },
        ],
    };

    write_bundle_manifest(&path, &manifest).unwrap();

    let value = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&path).unwrap(),
    )
    .unwrap();
    assert_eq!(
        value["$schema"],
        "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json"
    );
    assert_eq!(
        value["entries"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
        vec!["qa".to_owned(), "review".to_owned()]
    );
}
```

- [ ] Run the test and verify it fails because `write_bundle_manifest` does not exist:

```sh
cargo test --quiet write_bundle_manifest_outputs_schema_and_sorted_entries
```

- [ ] Add raw serializable structs:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawWritableBundleManifest {
    #[serde(rename = "$schema")]
    schema: String,
    name: String,
    description: String,
    entries: BTreeMap<String, RawWritableBundleEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawWritableBundleEntry {
    source: String,
}
```

- [ ] Add writer:

```rust
pub fn write_bundle_manifest(
    path: impl AsRef<Path>,
    manifest: &BundleManifest,
) -> Result<(), BundleManifestJsonError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BundleManifestJsonError::Read {
            path: display_path(parent),
            source,
        })?;
    }
    let entries = manifest
        .entries
        .iter()
        .map(|entry| {
            (
                entry.skill_name.as_str().to_owned(),
                RawWritableBundleEntry {
                    source: entry.source.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let raw = RawWritableBundleManifest {
        schema: "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json".to_owned(),
        name: manifest.name.as_str().to_owned(),
        description: manifest.description.clone(),
        entries,
    };
    let content = serde_json::to_string_pretty(&raw).map_err(|source| {
        BundleManifestJsonError::Parse {
            path: display_path(path),
            source,
        }
    })?;
    std::fs::write(path, format!("{content}\n")).map_err(|source| BundleManifestJsonError::Read {
        path: display_path(path),
        source,
    })
}
```

- [ ] If the error variant names feel misleading during implementation, add `CreateDir` and `Write` variants to `BundleManifestJsonError` and update the writer to use them. Keep existing read/parse call sites unchanged.

- [ ] Run tests:

```sh
cargo test --quiet write_bundle_manifest_outputs_schema_and_sorted_entries
cargo test --quiet bundle_manifest
```

Expected: pass.

**Acceptance criteria:** the project can write a schema-bearing bundle manifest that roundtrips through the existing strict parser.

---

## Issue 4: Add export planning types and manifest-only planning

**Goal:** Convert export dependency config into a validated `BundleExportPlan` for manifest-only mode.

**Files:**
- Modify: `src/application/bundle.rs`
- Test: `src/application/bundle.rs`

**Steps:**

- [ ] Add failing tests:

```rust
#[test]
fn manifest_only_export_plan_preserves_dependency_sources() {
    let dependencies = vec![
        BundleExportDependencyConfig { name: "review".to_owned(), source: "github:org/repo/skills/review#main".to_owned() },
        BundleExportDependencyConfig { name: "qa".to_owned(), source: "./vendor/qa".to_owned() },
    ];
    let plan = build_bundle_export_plan(BundleExportPlanInput {
        name: "team-baseline".to_owned(),
        description: None,
        output: PathBuf::from("./bundles/team-baseline"),
        mode: BundleExportMode::ManifestOnly,
        selected_skills: Vec::new(),
        dependencies,
        resolved_skills: Vec::new(),
    })
    .unwrap();

    assert_eq!(plan.manifest.name.as_str(), "team-baseline");
    assert_eq!(plan.items[0].skill_name, "qa");
    assert_eq!(plan.items[0].manifest_source, "./vendor/qa");
    assert_eq!(plan.items[1].skill_name, "review");
    assert_eq!(plan.items[1].manifest_source, "github:org/repo/skills/review#main");
}
```

```rust
#[test]
fn export_plan_rejects_selected_skill_not_in_dependencies() {
    let error = build_bundle_export_plan(BundleExportPlanInput {
        name: "team-baseline".to_owned(),
        description: None,
        output: PathBuf::from("./bundles/team-baseline"),
        mode: BundleExportMode::ManifestOnly,
        selected_skills: vec!["missing".to_owned()],
        dependencies: Vec::new(),
        resolved_skills: Vec::new(),
    })
    .unwrap_err();

    assert!(error.to_string().contains("selected skill 'missing' is not a dependency"));
}
```

- [ ] Run tests and verify they fail because export planning types do not exist:

```sh
cargo test --quiet export_plan
```

- [ ] Add imports to `src/application/bundle.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use crate::domain::skill::{SkillName, SkillNameError};
use crate::infrastructure::json::BundleExportDependencyConfig;
```

- [ ] Add export mode and plan types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleExportMode {
    ManifestOnly,
    Snapshot,
}

impl BundleExportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManifestOnly => "manifest-only",
            Self::Snapshot => "snapshot",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BundleExportPlanInput {
    pub name: String,
    pub description: Option<String>,
    pub output: PathBuf,
    pub mode: BundleExportMode,
    pub selected_skills: Vec<String>,
    pub dependencies: Vec<BundleExportDependencyConfig>,
    pub resolved_skills: Vec<BundleExportResolvedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportResolvedSkill {
    pub name: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportPlanItem {
    pub skill_name: String,
    pub manifest_source: String,
    pub source_path: Option<PathBuf>,
    pub snapshot_destination: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportPlan {
    pub manifest: BundleManifest,
    pub output: PathBuf,
    pub mode: BundleExportMode,
    pub items: Vec<BundleExportPlanItem>,
}
```

- [ ] Add error type:

```rust
#[derive(Debug, Error)]
pub enum BundleExportError {
    #[error("invalid bundle name '{name}': {source}")]
    InvalidBundleName { name: String, #[source] source: BundleNameError },
    #[error("invalid skill name '{name}': {source}")]
    InvalidSkillName { name: String, #[source] source: SkillNameError },
    #[error("selected skill '{0}' is not a dependency")]
    UnknownSelectedSkill(String),
    #[error("no dependencies selected for export")]
    EmptySelection,
    #[error("invalid dependency source for '{skill}': {message}")]
    InvalidDependencySource { skill: String, message: String },
    #[error("snapshot source for '{skill}' is missing from resolved config")]
    MissingResolvedSkill { skill: String },
}
```

- [ ] Add `build_bundle_export_plan`:

```rust
pub fn build_bundle_export_plan(
    input: BundleExportPlanInput,
) -> Result<BundleExportPlan, BundleExportError> {
    let bundle_name = BundleName::new(input.name.clone()).map_err(|source| {
        BundleExportError::InvalidBundleName { name: input.name.clone(), source }
    })?;
    let selected = input.selected_skills.iter().cloned().collect::<BTreeSet<_>>();
    let dependencies = input
        .dependencies
        .into_iter()
        .map(|dependency| (dependency.name.clone(), dependency))
        .collect::<BTreeMap<_, _>>();

    for selected_skill in &selected {
        if !dependencies.contains_key(selected_skill) {
            return Err(BundleExportError::UnknownSelectedSkill(selected_skill.clone()));
        }
    }

    let resolved = input
        .resolved_skills
        .into_iter()
        .map(|skill| (skill.name.clone(), skill.source_path))
        .collect::<BTreeMap<_, _>>();

    let mut items = Vec::new();
    let mut entries = Vec::new();
    for (name, dependency) in dependencies {
        if !selected.is_empty() && !selected.contains(&name) {
            continue;
        }
        let skill_name = SkillName::new(name.clone()).map_err(|source| {
            BundleExportError::InvalidSkillName { name: name.clone(), source }
        })?;
        parse_install_source_string(&dependency.source).map_err(|error| {
            BundleExportError::InvalidDependencySource {
                skill: name.clone(),
                message: error.to_string(),
            }
        })?;
        let manifest_source = match input.mode {
            BundleExportMode::ManifestOnly => dependency.source.clone(),
            BundleExportMode::Snapshot => format!("./skills/{name}"),
        };
        let source_path = match input.mode {
            BundleExportMode::ManifestOnly => None,
            BundleExportMode::Snapshot => Some(
                resolved
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| BundleExportError::MissingResolvedSkill { skill: name.clone() })?,
            ),
        };
        let snapshot_destination = source_path
            .as_ref()
            .map(|_| input.output.join("skills").join(&name));
        entries.push(BundleEntry {
            skill_name: skill_name.clone(),
            source: manifest_source.clone(),
        });
        items.push(BundleExportPlanItem {
            skill_name: skill_name.as_str().to_owned(),
            manifest_source,
            source_path,
            snapshot_destination,
        });
    }

    if items.is_empty() {
        return Err(BundleExportError::EmptySelection);
    }

    Ok(BundleExportPlan {
        manifest: BundleManifest {
            name: bundle_name,
            description: input
                .description
                .unwrap_or_else(|| "Exported from sksync config.".to_owned()),
            entries,
        },
        output: input.output,
        mode: input.mode,
        items,
    })
}
```

- [ ] Run tests:

```sh
cargo test --quiet export_plan
```

Expected: pass.

**Acceptance criteria:** manifest-only export planning is deterministic, validates names/sources, supports subsets, and produces a `BundleManifest` without agents/provenance.

---

## Issue 5: Add snapshot planning and skill package validation

**Goal:** Ensure snapshot mode maps selected dependencies to installed source directories and validates each copied skill body before writing.

**Files:**
- Modify: `src/application/bundle.rs`
- Test: `src/application/bundle.rs`

**Steps:**

- [ ] Add failing test for snapshot destination mapping:

```rust
#[test]
fn snapshot_export_plan_rewrites_sources_to_manifest_relative_paths() {
    let dependencies = vec![BundleExportDependencyConfig {
        name: "review".to_owned(),
        source: "github:org/repo/skills/review#main".to_owned(),
    }];
    let resolved_skills = vec![BundleExportResolvedSkill {
        name: "review".to_owned(),
        source_path: PathBuf::from("./.sksync/skills/org/repo/review"),
    }];

    let plan = build_bundle_export_plan(BundleExportPlanInput {
        name: "team-baseline".to_owned(),
        description: None,
        output: PathBuf::from("./bundles/team-baseline"),
        mode: BundleExportMode::Snapshot,
        selected_skills: Vec::new(),
        dependencies,
        resolved_skills,
    })
    .unwrap();

    assert_eq!(plan.items[0].manifest_source, "./skills/review");
    assert_eq!(
        plan.items[0].source_path.as_deref(),
        Some(Path::new("./.sksync/skills/org/repo/review"))
    );
    assert_eq!(
        plan.items[0].snapshot_destination.as_deref(),
        Some(Path::new("./bundles/team-baseline/skills/review"))
    );
}
```

- [ ] Add failing validation tests:

```rust
#[test]
fn validate_snapshot_export_source_requires_valid_skill_manifest() {
    let temp = tempfile::tempdir().expect("temp dir");
    let skill_dir = temp.path().join("review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# Missing frontmatter\n").unwrap();

    let error = validate_snapshot_export_source("review", &skill_dir).unwrap_err();

    assert!(error.to_string().contains("invalid SKILL.md"));
}
```

```rust
#[test]
fn validate_snapshot_export_source_accepts_valid_skill_manifest() {
    let temp = tempfile::tempdir().expect("temp dir");
    let skill_dir = temp.path().join("review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: review\ndescription: Review skill\n---\n# Review\n",
    )
    .unwrap();

    validate_snapshot_export_source("review", &skill_dir).unwrap();
}
```

- [ ] Run tests and verify validation function is missing:

```sh
cargo test --quiet snapshot_export
cargo test --quiet validate_snapshot_export_source
```

- [ ] Add validation error variants:

```rust
#[error("snapshot source for '{skill}' does not exist: {path}")]
MissingSnapshotSource { skill: String, path: String },
#[error("snapshot source for '{skill}' is not a directory: {path}")]
SnapshotSourceNotDirectory { skill: String, path: String },
#[error("snapshot source for '{skill}' is invalid SKILL.md: {message}")]
InvalidSnapshotSkill { skill: String, message: String },
```

- [ ] Add `validate_snapshot_export_source`:

```rust
pub fn validate_snapshot_export_source(
    skill: &str,
    path: &Path,
) -> Result<(), BundleExportError> {
    if !path.exists() {
        return Err(BundleExportError::MissingSnapshotSource {
            skill: skill.to_owned(),
            path: path.display().to_string(),
        });
    }
    if !path.is_dir() {
        return Err(BundleExportError::SnapshotSourceNotDirectory {
            skill: skill.to_owned(),
            path: path.display().to_string(),
        });
    }
    let skill_md = path.join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md).map_err(|error| {
        BundleExportError::InvalidSnapshotSkill {
            skill: skill.to_owned(),
            message: error.to_string(),
        }
    })?;
    crate::domain::skill_manifest::parse_skill_manifest(&content).map_err(|error| {
        BundleExportError::InvalidSnapshotSkill {
            skill: skill.to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}
```

- [ ] Run tests:

```sh
cargo test --quiet snapshot_export
cargo test --quiet validate_snapshot_export_source
```

Expected: pass.

**Acceptance criteria:** snapshot planning rewrites manifest sources to `./skills/<name>` and validates installed source directories before copying.

---

## Issue 6: Implement export file application helpers

**Goal:** Apply a `BundleExportPlan` to disk safely for manifest-only and snapshot modes.

**Files:**
- Modify: `src/application/bundle.rs`
- Test: `src/application/bundle.rs`

**Steps:**

- [ ] Add failing tests for manifest-only output safety:

```rust
#[test]
fn apply_manifest_only_export_refuses_existing_output_without_force() {
    let temp = tempfile::tempdir().expect("temp dir");
    let output = temp.path().join("bundle");
    std::fs::create_dir_all(&output).unwrap();
    let plan = export_plan_for_test(output.clone(), BundleExportMode::ManifestOnly);

    let error = apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap_err();

    assert!(error.to_string().contains("already exists"));
}
```

```rust
#[test]
fn apply_manifest_only_export_writes_manifest_without_skills_dir() {
    let temp = tempfile::tempdir().expect("temp dir");
    let output = temp.path().join("bundle");
    let plan = export_plan_for_test(output.clone(), BundleExportMode::ManifestOnly);

    apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap();

    assert!(output.join("sksync.bundle.json").is_file());
    assert!(!output.join("skills").exists());
}
```

- [ ] Add failing test for snapshot staged copy:

```rust
#[test]
fn apply_snapshot_export_copies_skills_and_manifest() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source = temp.path().join("installed/review");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: review\ndescription: Review skill\n---\n# Review\n",
    )
    .unwrap();
    let output = temp.path().join("bundle");
    let mut plan = export_plan_for_test(output.clone(), BundleExportMode::Snapshot);
    plan.items[0].source_path = Some(source.clone());
    plan.items[0].snapshot_destination = Some(output.join("skills/review"));

    apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap();

    assert!(output.join("sksync.bundle.json").is_file());
    assert!(output.join("skills/review/SKILL.md").is_file());
}
```

- [ ] Add `export_plan_for_test` helper inside the test module with a complete one-entry plan.

- [ ] Run tests and verify `apply_bundle_export_plan` is missing:

```sh
cargo test --quiet apply_manifest_only_export
cargo test --quiet apply_snapshot_export
```

- [ ] Add apply options and error variants:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleExportApplyOptions {
    pub force: bool,
}
```

```rust
#[error("bundle export output already exists: {0}")]
OutputExists(String),
#[error("failed to create export directory {path}: {source}")]
CreateDir { path: String, #[source] source: std::io::Error },
#[error("failed to copy snapshot skill {skill} from {from} to {to}: {message}")]
CopySnapshotSkill { skill: String, from: String, to: String, message: String },
#[error("failed to replace export output {path}: {source}")]
ReplaceOutput { path: String, #[source] source: std::io::Error },
```

- [ ] Add `apply_bundle_export_plan`:

```rust
pub fn apply_bundle_export_plan(
    plan: &BundleExportPlan,
    options: BundleExportApplyOptions,
) -> Result<(), BundleExportError> {
    if plan.output.exists() && !options.force {
        return Err(BundleExportError::OutputExists(plan.output.display().to_string()));
    }
    let parent = plan.output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| BundleExportError::CreateDir {
        path: parent.display().to_string(),
        source,
    })?;
    let staging = plan.output.with_extension(format!(
        "sksync-export-staging-{}",
        std::process::id()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|source| BundleExportError::ReplaceOutput {
            path: staging.display().to_string(),
            source,
        })?;
    }
    std::fs::create_dir_all(&staging).map_err(|source| BundleExportError::CreateDir {
        path: staging.display().to_string(),
        source,
    })?;

    let result = (|| {
        if plan.mode == BundleExportMode::Snapshot {
            for item in &plan.items {
                let source = item.source_path.as_ref().expect("snapshot plan has source_path");
                validate_snapshot_export_source(&item.skill_name, source)?;
                let destination = staging.join("skills").join(&item.skill_name);
                copy_dir_all_for_bundle_export(source, &destination, &item.skill_name)?;
            }
        }
        crate::infrastructure::json::write_bundle_manifest(
            staging.join(BUNDLE_MANIFEST_FILE),
            &plan.manifest,
        )
        .map_err(|error| BundleExportError::InvalidSnapshotSkill {
            skill: plan.manifest.name.as_str().to_owned(),
            message: error.to_string(),
        })?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    if plan.output.exists() {
        std::fs::remove_dir_all(&plan.output).map_err(|source| BundleExportError::ReplaceOutput {
            path: plan.output.display().to_string(),
            source,
        })?;
    }
    std::fs::rename(&staging, &plan.output).map_err(|source| BundleExportError::ReplaceOutput {
        path: plan.output.display().to_string(),
        source,
    })?;
    Ok(())
}
```

- [ ] Add recursive copy helper that copies only regular files and directories:

```rust
fn copy_dir_all_for_bundle_export(
    from: &Path,
    to: &Path,
    skill: &str,
) -> Result<(), BundleExportError> {
    std::fs::create_dir_all(to).map_err(|source| BundleExportError::CreateDir {
        path: to.display().to_string(),
        source,
    })?;
    for entry in std::fs::read_dir(from).map_err(|source| BundleExportError::CopySnapshotSkill {
        skill: skill.to_owned(),
        from: from.display().to_string(),
        to: to.display().to_string(),
        message: source.to_string(),
    })? {
        let entry = entry.map_err(|source| BundleExportError::CopySnapshotSkill {
            skill: skill.to_owned(),
            from: from.display().to_string(),
            to: to.display().to_string(),
            message: source.to_string(),
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| BundleExportError::CopySnapshotSkill {
            skill: skill.to_owned(),
            from: source_path.display().to_string(),
            to: target_path.display().to_string(),
            message: source.to_string(),
        })?;
        if file_type.is_dir() {
            copy_dir_all_for_bundle_export(&source_path, &target_path, skill)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path).map_err(|source| {
                BundleExportError::CopySnapshotSkill {
                    skill: skill.to_owned(),
                    from: source_path.display().to_string(),
                    to: target_path.display().to_string(),
                    message: source.to_string(),
                }
            })?;
        }
    }
    Ok(())
}
```

- [ ] Run tests:

```sh
cargo test --quiet apply_manifest_only_export
cargo test --quiet apply_snapshot_export
```

Expected: pass.

**Acceptance criteria:** export writes through staging, refuses existing output unless forced, and never mutates source skill bodies.

---

## Issue 7: Wire `sksync bundle export` CLI behavior

**Goal:** Build and apply export plans from the real project/global config.

**Files:**
- Modify: `src/cli.rs`

**Steps:**

- [ ] Add imports in `src/cli.rs`:

```rust
use crate::application::bundle::{
    apply_bundle_export_plan, build_bundle_export_plan, BundleExportApplyOptions,
    BundleExportMode, BundleExportPlan, BundleExportPlanInput, BundleExportResolvedSkill,
};
```

- [ ] Replace placeholder `run_bundle_export` with:

```rust
fn run_bundle_export(args: BundleExportArgs) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    let config_path = config_path_for(args.global, &current_dir)?;
    let root_dir = if args.global {
        config_root_for_global()?
    } else {
        current_dir.clone()
    };
    let store = FileDependencyConfigStore::new(&config_path, default_skill_dir_for(args.global)?);
    let dependencies = store.load_bundle_export_dependencies()?;
    let resolved_config = load_config_from_path(&config_path, scope_for(args.global))?;
    let resolved_skills = resolved_config
        .skills
        .iter()
        .filter(|skill| skill.install_source.is_some())
        .map(|skill| BundleExportResolvedSkill {
            name: skill.name.as_str().to_owned(),
            source_path: skill.source.as_path().to_path_buf(),
        })
        .collect::<Vec<_>>();
    let plan = build_bundle_export_plan(BundleExportPlanInput {
        name: args.name,
        description: None,
        output: resolve_export_output_path(&args.output, &root_dir),
        mode: if args.snapshot {
            BundleExportMode::Snapshot
        } else {
            BundleExportMode::ManifestOnly
        },
        selected_skills: args.skills,
        dependencies,
        resolved_skills,
    })?;
    print_bundle_export_plan(&plan);
    if args.dry_run {
        return Ok(());
    }
    apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: args.force })?;
    print_success(format!(
        "Exported bundle: {} -> {}",
        plan.manifest.name,
        plan.output.display()
    ));
    Ok(())
}
```

- [ ] Add output path resolver:

```rust
fn resolve_export_output_path(output: &Path, root_dir: &Path) -> PathBuf {
    if output.is_absolute() {
        output.to_path_buf()
    } else {
        root_dir.join(output)
    }
}
```

- [ ] Add plan printer:

```rust
fn print_bundle_export_plan(plan: &BundleExportPlan) {
    print_section("Bundle export plan");
    println!("Name: {}", plan.manifest.name);
    println!("Mode: {}", plan.mode.as_str());
    println!("Output: {}", plan.output.display());
    print_section_with_count("Entries", plan.items.len());
    for item in &plan.items {
        match (&item.source_path, &item.snapshot_destination) {
            (Some(source), Some(destination)) => println!(
                "- {}: {} -> {}",
                item.skill_name,
                source.display(),
                destination.display()
            ),
            _ => println!("- {}: {}", item.skill_name, item.manifest_source),
        }
    }
}
```

- [ ] Run a targeted compile/test:

```sh
cargo test --quiet bundle_export_command_is_registered
```

Expected: pass.

**Acceptance criteria:** CLI can plan, dry-run, and apply export using project or global config without mutating config or lockfile.

---

## Issue 8: Add integration tests for `bundle export`

**Goal:** Verify the real binary behavior from temp project directories.

**Files:**
- Modify: `tests/bundle_cli.rs`

**Steps:**

- [ ] Add helper for dependency config with installed skill bodies:

```rust
fn write_export_project(root: &Path) {
    fs::write(
        root.join("sksync.config.json"),
        r#"{
          "skillDir": "./.sksync/skills",
          "dependencies": {
            "review": { "source": "github:org/repo/skills/review#main", "agents": ["pi"] },
            "qa": { "source": "./vendor/qa", "agents": ["pi"] }
          }
        }"#,
    )
    .expect("write config");
    write_bundle_skill(root, ".sksync/skills/review", "review");
    write_bundle_skill(root, ".sksync/skills/qa", "qa");
}
```

- [ ] Add manifest-only export test:

```rust
#[test]
fn bundle_export_writes_manifest_only_bundle() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &["bundle", "export", "team-baseline", "--output", "./bundle-out"],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["name"], "team-baseline");
    assert_eq!(manifest["entries"]["review"]["source"], "github:org/repo/skills/review#main");
    assert_eq!(manifest["entries"]["qa"]["source"], "./vendor/qa");
    assert!(!root.join("bundle-out/skills").exists());
}
```

- [ ] Add snapshot export test:

```rust
#[test]
fn bundle_export_snapshot_copies_installed_skill_bodies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &["bundle", "export", "team-baseline", "--output", "./bundle-out", "--snapshot"],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["entries"]["review"]["source"], "./skills/review");
    assert_eq!(manifest["entries"]["qa"]["source"], "./skills/qa");
    assert!(root.join("bundle-out/skills/review/SKILL.md").is_file());
    assert!(root.join("bundle-out/skills/qa/SKILL.md").is_file());
}
```

- [ ] Add dry-run no-write test:

```rust
#[test]
fn bundle_export_dry_run_does_not_create_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &["bundle", "export", "team-baseline", "--output", "./bundle-out", "--dry-run"],
    ));

    assert!(!root.join("bundle-out").exists());
}
```

- [ ] Add subset test:

```rust
#[test]
fn bundle_export_skill_filter_exports_only_selected_dependency() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);

    assert_success(sksync(
        root,
        &["bundle", "export", "team-baseline", "--output", "./bundle-out", "--skill", "qa"],
    ));

    let manifest = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(root.join("bundle-out/sksync.bundle.json")).unwrap(),
    )
    .unwrap();
    assert!(manifest["entries"].get("qa").is_some());
    assert!(manifest["entries"].get("review").is_none());
}
```

- [ ] Add overwrite safety test:

```rust
#[test]
fn bundle_export_refuses_existing_output_without_force() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    write_export_project(root);
    fs::create_dir_all(root.join("bundle-out")).unwrap();

    let error = assert_failure(sksync(
        root,
        &["bundle", "export", "team-baseline", "--output", "./bundle-out"],
    ));

    assert!(error.contains("already exists"));
}
```

- [ ] Run integration tests:

```sh
cargo test --quiet --test bundle_cli bundle_export
```

Expected: all new export tests pass.

**Acceptance criteria:** real binary covers manifest-only, snapshot, dry-run, subset, and overwrite behavior.

---

## Issue 9: Update docs from planned to implemented

**Goal:** Make user docs accurately describe `bundle export` as implemented.

**Files:**
- Modify: `README.md`
- Modify: `docs/DESIGN.md`
- Modify: `docs/ROADMAP.md`
- Modify: `site/guides/bundles.md`
- Modify: `site/reference/commands.md`

**Steps:**

- [ ] In `README.md`, replace "planned/not part of current CLI" wording with implemented examples:

```md
# Create only sksync.bundle.json from current dependencies.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline

# Copy installed skill bodies into the bundle directory.
cargo run -- bundle export team-baseline --output ./bundles/team-baseline --snapshot
```

- [ ] In `site/guides/bundles.md`, rename `## Planned export workflow` to `## Exporting a bundle from existing dependencies`.

- [ ] In `site/guides/bundles.md`, remove the sentence `bundle export is not in the current CLI yet; this section records the intended authoring workflow.`

- [ ] In `site/reference/commands.md`, add `export` to the `sksync bundle` command block:

```sh
sksync bundle export <name> --output <dir> [--snapshot]
sksync bundle export <name> --output <dir> --skill <skill> --dry-run
```

- [ ] In `site/reference/commands.md`, add flag rows for bundle export:

```md
| `--output <dir>` | Directory that will contain the exported `sksync.bundle.json`. Required for `bundle export`. |
| `--snapshot` | Copy installed skill bodies into the bundle directory and write `./skills/<name>` sources. |
| `--skill <name>` | Export only this dependency. Repeatable. |
| `--force` | Replace an existing export output directory. |
```

- [ ] In `docs/ROADMAP.md`, move bundle export from planned wording into completed baseline or current stabilization notes.

- [ ] Run docs verification:

```sh
bun run docs:build
git diff --check
```

Expected: docs build passes and whitespace check has no output.

**Acceptance criteria:** docs no longer imply `bundle export` is future-only after implementation lands.

---

## Issue 10: Final verification and review

**Goal:** Confirm the feature is safe, tested, documented, and ready for PR.

**Files:**
- No planned code files. This issue runs verification and handles review feedback.

**Steps:**

- [ ] Run formatting:

```sh
cargo fmt --check
```

Expected: no diff output.

- [ ] Run tests:

```sh
cargo test --quiet
```

Expected: all unit and integration tests pass.

- [ ] Run clippy:

```sh
cargo clippy --quiet -- -D warnings
```

Expected: no warnings.

- [ ] Run release build:

```sh
cargo build --release --quiet
```

Expected: exits successfully.

- [ ] Run docs build:

```sh
bun run docs:build
```

Expected: VitePress build completes.

- [ ] Run whitespace check:

```sh
git diff --check
```

Expected: no output.

- [ ] Request reviewer subagent review with this prompt:

```text
Review the bundle export implementation against docs/superpowers/specs/2026-05-26-bundle-export-design.md and docs/superpowers/plans/2026-05-26-bundle-export-implementation.md. Focus on dry-run no writes, force/overwrite safety, snapshot staging cleanup, manifest-only source preservation, no agent/provenance export, config/lockfile immutability, and docs accuracy. Verification passed: cargo fmt --check, cargo test --quiet, cargo clippy --quiet -- -D warnings, cargo build --release --quiet, bun run docs:build, git diff --check.
```

- [ ] Fix any Critical or Important review feedback and rerun the relevant tests.

- [ ] Use GitButler for PR flow:

```sh
but status -fv
but commit <branch-id-or-name> -m "Implement bundle export" --changes <ids-from-status> --status-after
```

- [ ] Open a PR with summary and verification, wait for GitHub checks, merge, then sync workspace through GitButler:

```sh
but pull --check
but pull --status-after
```

**Acceptance criteria:** all local verification passes, reviewer reports no blockers, GitHub checks pass, PR is merged, and workspace is clean.

---

## Suggested PR slicing

This can be one PR because the feature is cohesive. If implementation becomes large, split into:

1. **Export planning + manifest writer**: Issues 1-6.
2. **CLI + integration tests + docs**: Issues 7-10.

Each PR should pass `cargo test --quiet`; the final PR should pass the full verification list.

---

## Self-review notes

- Spec coverage: manifest-only export, snapshot export, `--global`, `--skill`, `--dry-run`, `--force`, no agents/provenance, no config/lockfile mutation, staging safety, and docs updates are each covered by at least one issue.
- Placeholder scan: this plan contains no unresolved placeholders or vague implementation instructions.
- Type consistency: export types use the `BundleExport*` prefix consistently; CLI args use `skills: Vec<String>` while the user-facing flag remains repeatable `--skill`.
