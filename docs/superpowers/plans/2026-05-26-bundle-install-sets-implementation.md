# Bundle Install Sets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `sksync bundle inspect/add/remove` so users can install curated skill sets from `sksync.bundle.json` manifests while preserving flat agent skill targets and dependency-centric lifecycle.

**Architecture:** Introduce bundle domain primitives and a bundle application layer that parses strict manifests, normalizes bundle entry sources, plans add/remove actions, and applies them through existing dependency/install/apply/remove use cases. Store bundle provenance only in config (`dependencies.*.bundles`, `managedByBundles`); lockfile v4 remains content-focused and does not store bundle metadata.

**Tech Stack:** Rust, clap, serde/serde_json, existing sksync application/domain/infrastructure layers, VitePress docs, JSON Schema tests.

---

## File map

- Create `src/domain/bundle.rs`: `BundleName`, `BundleManifest`, `BundleEntry`, `BundleProvenance`, bundle validation types.
- Modify `src/domain/mod.rs`: expose `bundle` module.
- Create `src/application/bundle.rs`: manifest loading/normalization, add/remove plan types, add/remove orchestration helpers.
- Modify `src/application/mod.rs`: expose `bundle` module.
- Modify `src/application/ports.rs`: add bundle-aware dependency config operations or dedicated config transaction helpers.
- Modify `src/infrastructure/json.rs`: parse/write bundle manifests, config `bundles`/`managedByBundles`, schema fixture tests.
- Modify `src/application/source.rs` and/or `src/application/discovery.rs`: reusable helpers for exact source normalization and manifest-relative source resolution.
- Modify `src/cli.rs`: add `sksync bundle inspect/add/remove` commands and output.
- Create `schemas/sksync.bundle.schema.json`: strict bundle manifest schema.
- Modify `schemas/sksync.schema.json`: allow dependency provenance fields.
- Add `sksync.bundle.example.json`: example bundle manifest fixture.
- Modify `tests/schema_files.rs`: validate new schema/example and config schema fields.
- Modify docs: `README.md`, `docs/DESIGN.md`, `site/guides/project-config.md`, new `site/guides/bundles.md`, `site/reference/commands.md`, `site/.vitepress/config.ts`.

---

## Issue 1: Add bundle domain primitives

**Goal:** Represent bundle names, manifests, entries, and provenance with always-valid domain types.

**Files:**
- Create: `src/domain/bundle.rs`
- Modify: `src/domain/mod.rs`

**Steps:**

- [ ] Add failing tests in `src/domain/bundle.rs` for `BundleName`:
  - accepts `review-workflow`
  - rejects empty string
  - rejects path separators such as `team/review`
- [ ] Implement `BundleName` with `new`, `as_str`, `Display`, `Clone`, `Eq`, `Ord`, and `Hash` as needed.
- [ ] Add `BundleProvenance { name: BundleName, source: String }` and ensure equality/dedup works by both `name` and `source`.
- [ ] Add `BundleEntry { skill_name: SkillName, source: String }` and `BundleManifest { name, description, entries }`.
- [ ] Add tests that a manifest cannot be empty after parsing/validation helpers are introduced in later issues.
- [ ] Export `pub mod bundle;` from `src/domain/mod.rs`.
- [ ] Run `cargo test --quiet bundle`.

**Acceptance criteria:** bundle names follow `SkillName`-like safety rules, and bundle domain types are independent of CLI/filesystem code.

---

## Issue 2: Add strict bundle manifest JSON parsing

**Goal:** Read and validate `sksync.bundle.json` with strict schema-compatible behavior.

**Files:**
- Modify: `src/infrastructure/json.rs`
- Test: `src/infrastructure/json.rs`

**Steps:**

- [ ] Add raw manifest structs near existing raw config structs:
  - `RawBundleManifest { schema, name, description, entries }`
  - `RawBundleEntry { source }`
  - use `#[serde(rename_all = "camelCase", deny_unknown_fields)]`.
- [ ] Add `read_bundle_manifest(path: impl AsRef<Path>) -> Result<BundleManifest, BundleManifestJsonError>`.
- [ ] Define `BundleManifestJsonError` with read, parse, invalid name, invalid entry name, missing/empty description, empty entries, and empty source variants.
- [ ] Add tests using temp files:
  - valid manifest parses
  - unknown top-level field fails
  - unknown entry field such as `agents` fails
  - empty `entries` fails
  - invalid entry skill name fails
  - empty source fails
- [ ] Keep parser free of source normalization; this issue only validates raw manifest contents.
- [ ] Run `cargo test --quiet bundle_manifest`.

**Acceptance criteria:** invalid bundle manifests fail before any config/install work can happen, and agents are not allowed in manifests.

---

## Issue 3: Add bundle schema and schema tests

**Goal:** Publish JSON Schema coverage for bundle manifests and config provenance fields.

**Files:**
- Create: `schemas/sksync.bundle.schema.json`
- Create: `sksync.bundle.example.json`
- Modify: `schemas/sksync.schema.json`
- Modify: `tests/schema_files.rs`

**Steps:**

- [ ] Add `schemas/sksync.bundle.schema.json` with:
  - `$id`: `https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json`
  - `additionalProperties: false`
  - required `name`, `description`, `entries`
  - `entries` object with `minProperties: 1`
  - entry object requires `source`, rejects unknown fields.
- [ ] Add `sksync.bundle.example.json` matching the agreed example.
- [ ] Extend `schemas/sksync.schema.json` dependency config with optional:
  - `bundles`: array of `{ name, source }`, unique items
  - `managedByBundles`: boolean, default false
- [ ] Update `tests/schema_files.rs` constants and tests to parse the bundle schema and verify example `$schema` references.
- [ ] Add assertions that config schema dependency config includes `bundles` and `managedByBundles`.
- [ ] Run `cargo test --quiet schema_files`.

**Acceptance criteria:** bundle manifest schema and config provenance schema are versioned, referenced by examples, and tested.

---

## Issue 4: Extend config model for provenance without changing lockfile

**Goal:** Preserve and resolve dependency `bundles` and `managedByBundles` from config while keeping lockfile unchanged.

**Files:**
- Modify: `src/application/config.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/infrastructure/json.rs`

**Steps:**

- [ ] Add application-level provenance fields to `ResolvedSkill` only if CLI/list/doctor need them; otherwise keep provenance in infrastructure config mutation layer to avoid widening core unnecessarily.
- [ ] Extend `RawDependencyConfig` with:
  - `#[serde(default)] bundles: Vec<RawBundleProvenance>`
  - `#[serde(default)] managed_by_bundles: bool`
- [ ] Ensure existing config without those fields still parses and resolves.
- [ ] Ensure lockfile writer/reader is not modified to write bundle provenance.
- [ ] Add tests:
  - missing `managedByBundles` resolves/loads as false
  - config with multiple `bundles` parses
  - lockfile roundtrip output remains unchanged by config provenance support.
- [ ] Run `cargo test --quiet parses_default_agents` and full `cargo test --quiet` for regression.

**Acceptance criteria:** existing configs remain backward-compatible, provenance fields are accepted in config, and lockfile v4 remains provenance-free.

---

## Issue 5: Add bundle-aware config transaction operations

**Goal:** Support atomic add/remove planning and mutation of dependency config with bundle provenance.

**Files:**
- Modify: `src/application/ports.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/infrastructure/json.rs`

**Steps:**

- [ ] Add types in `src/application/bundle.rs` or `ports.rs`:
  - `BundleAddDependencyChange { skill_name, source, agents, provenance }`
  - `BundleRemoveTarget { name, source }`
- [ ] Add infrastructure helper methods on `FileDependencyConfigStore` rather than broad trait methods if only CLI bundle code needs file JSON transactions:
  - `plan_bundle_add(...)`
  - `apply_bundle_add(...)`
  - `plan_bundle_remove(...)`
  - `apply_bundle_remove(...)`
- [ ] In add planning:
  - missing dependency → `create`
  - same normalized source → `merge`
  - same skill name different source → `conflict`
  - same agents and same provenance already present → `skipped`
- [ ] In add apply:
  - union-merge agents
  - append unique provenance in `bundles`
  - new dependencies get `managedByBundles: true`
  - existing dependencies keep existing `managedByBundles`, defaulting to false.
- [ ] In remove planning:
  - no matching provenance → `not-found`
  - same name multiple sources and no `--source` → `ambiguous`
  - matching provenance removed and dependency remains → `detach-provenance`
  - matching provenance removed, bundles empty, managedByBundles true → `remove`.
- [ ] Add tests for each add/remove classification and mutation.
- [ ] Ensure write is one JSON write per operation after planning succeeds.

**Acceptance criteria:** config mutation is atomic at the JSON file level and exactly implements the provenance/managedByBundles rules.

---

## Issue 6: Implement bundle source loading and relative entry normalization

**Goal:** Fetch a bundle manifest from a directory source and normalize each entry source consistently with `sksync add`.

**Files:**
- Create/Modify: `src/application/bundle.rs`
- Modify: `src/application/source.rs` or `src/application/discovery.rs` if helpers need extraction
- Test: `src/application/bundle.rs`

**Steps:**

- [ ] Add `load_bundle_from_source(raw_source: &str, config_root: &Path) -> Result<LoadedBundle>`.
- [ ] For local bundle source:
  - parse as `InstallSource::Local`
  - read `<source>/sksync.bundle.json`
  - resolve `./entry` relative to manifest directory
  - store relative to config root when possible; otherwise absolute.
- [ ] For Git bundle source:
  - parse as `InstallSource::Git`
  - clone/checkout using existing `GitClient`
  - read `<git.path>/sksync.bundle.json`
  - resolve relative entry sources against the same repo/ref and bundle directory.
- [ ] Normalize external entry sources through the same source rewrite pipeline used by `sksync add`; if helper extraction is needed, do it in small functions and preserve existing add tests.
- [ ] Save bundle provenance source as exact manifest directory source, not requested input.
- [ ] Add tests for local relative entries, Git relative source string construction, and external source passthrough/normalization.

**Acceptance criteria:** bundle-relative sources are deterministic and use the manifest source ref/location, while external sources behave like normal `add` sources.

---

## Issue 7: Implement `sksync bundle inspect`

**Goal:** Add a read-only command that displays bundle manifest metadata and normalized entry sources.

**Files:**
- Modify: `src/cli.rs`
- Modify/Create: `src/application/bundle.rs`
- Test: `src/cli.rs` unit tests if existing CLI test style supports it

**Steps:**

- [ ] Add `Bundle(BundleArgs)` to `Command`.
- [ ] Add `BundleCommand::{Inspect, Add, Remove}` with only Inspect wired in this issue; Add/Remove can return clear placeholder errors until later issues or be wired in subsequent tasks.
- [ ] Add `BundleInspectArgs { source: String }`.
- [ ] Implement `run_bundle_inspect` to call `load_bundle_from_source` and print:
  - bundle name
  - description
  - each entry name
  - original source
  - normalized source
- [ ] Add tests for output formatting using a local temp bundle if practical; otherwise cover application layer formatting with pure unit tests.
- [ ] Run `cargo run -- bundle inspect <local-fixture>` manually with a temp fixture.

**Acceptance criteria:** inspect is read-only, does not load current config, and shows normalized sources.

---

## Issue 8: Implement `sksync bundle add --dry-run`

**Goal:** Let users preview bundle add effects without writes.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/application/bundle.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/application/bundle.rs`, `src/infrastructure/json.rs`

**Steps:**

- [ ] Add CLI args:
  - `sksync bundle add <source> --agent <agent>... [--global] [--dry-run]`
  - agents required.
- [ ] Load bundle and current config for selected scope.
- [ ] Build add plan using normalized entries and requested agents.
- [ ] Print entries grouped or listed with statuses: `create`, `merge`, `conflict`, `skipped`.
- [ ] If conflicts exist, dry-run exits non-zero or clearly reports conflict status according to existing CLI conventions; choose the convention used by `plan/check` and stay consistent.
- [ ] Ensure no config, lockfile, source, or target filesystem writes occur.
- [ ] Add tests for dry-run plan statuses.

**Acceptance criteria:** dry-run gives complete add classification and does not mutate files.

---

## Issue 9: Implement `sksync bundle add` execution with rollback

**Goal:** Apply an atomic bundle add and run install/apply behavior safely.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/application/bundle.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/application/bundle.rs`, `src/cli.rs` integration-style unit tests where existing patterns permit

**Steps:**

- [ ] Reuse the same add planning logic as dry-run before writing anything.
- [ ] If any conflict exists, abort before config writes.
- [ ] Snapshot original config and lockfile content/path state before mutation, following existing `add` rollback patterns.
- [ ] Apply dependency config changes in one pass.
- [ ] Load resolved config and call existing `update_dependencies`, `build_link_plan`, and `apply_link_plan` with `skip_blocked_targets: true` matching `add` behavior unless a spec update says otherwise.
- [ ] On any failure after config mutation, restore config and lockfile snapshots.
- [ ] Best-effort clean only sksync-managed artifacts created by this operation; never delete unmanaged files.
- [ ] Add tests for:
  - create multiple dependencies
  - conflict prevents all writes
  - same source merges agents/provenance
  - install/apply failure restores config.

**Acceptance criteria:** bundle add behaves like atomic multi-add at config/lockfile level and never partially writes config on conflict/failure.

---

## Issue 10: Implement `sksync bundle remove --dry-run`

**Goal:** Preview local provenance-based bundle removal.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/application/bundle.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/application/bundle.rs`, `src/infrastructure/json.rs`

**Steps:**

- [ ] Add CLI args:
  - `sksync bundle remove <name> [--source <exact-source>] [--global] [--dry-run]`.
- [ ] Validate `<name>` as `BundleName`.
- [ ] Load current config only; do not fetch remote bundle manifests.
- [ ] Build removal plan and print statuses:
  - `remove`
  - `detach-provenance`
  - `ambiguous`
  - `not-found`
- [ ] If name is ambiguous and `--source` is omitted, return a clear error listing matching sources.
- [ ] Ensure dry-run performs no writes.
- [ ] Add tests for not-found, ambiguous, remove, and detach-provenance classifications.

**Acceptance criteria:** remove dry-run is local-only, clear, and safe.

---

## Issue 11: Implement `sksync bundle remove` execution

**Goal:** Remove bundle provenance and delete only bundle-managed dependencies that no longer have bundle provenance.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/application/bundle.rs`
- Modify: `src/infrastructure/json.rs`
- Test: `src/application/bundle.rs`, `src/cli.rs`

**Steps:**

- [ ] Reuse the remove planning logic from dry-run.
- [ ] If ambiguous or not-found, do not write.
- [ ] For `detach-provenance`, update config only.
- [ ] For `remove`, delegate to existing remove semantics or shared removal helpers so installed files/symlinks/lockfile are treated the same as `sksync remove`.
- [ ] Preserve manual dependencies when `managedByBundles` is false.
- [ ] Remove empty `bundles` arrays from JSON if that matches current config style; otherwise keep a stable style and document it in tests.
- [ ] Add tests for mixed dependencies:
  - `[A, B]` removing A leaves B and keeps dependency
  - `[A] + managedByBundles true` removes dependency
  - `[A] + managedByBundles false` keeps dependency without bundles.

**Acceptance criteria:** bundle remove cannot delete manual dependencies solely because they once had bundle provenance.

---

## Issue 12: Documentation and examples

**Goal:** Make bundle behavior discoverable and keep docs/schema examples aligned.

**Files:**
- Modify: `README.md`
- Modify: `docs/DESIGN.md`
- Create/Modify: `site/guides/bundles.md`
- Modify: `site/guides/project-config.md`
- Modify: `site/reference/commands.md`
- Modify: `site/.vitepress/config.ts`
- Modify/Create examples as needed: `sksync.bundle.example.json`

**Steps:**

- [ ] Add README section for `sksync bundle inspect/add/remove` with examples and safety notes.
- [ ] Update `docs/DESIGN.md` with Bundle / Bundle Entry terms, config provenance, and command semantics.
- [ ] Add `site/guides/bundles.md` covering manifest shape, relative sources, add/remove dry-runs, provenance, and non-goals.
- [ ] Link the bundles guide from VitePress nav/sidebar.
- [ ] Update project config guide to document `dependencies.*.bundles` and `managedByBundles`.
- [ ] Update command reference.
- [ ] Run `bun run docs:build`.

**Acceptance criteria:** user-facing docs explain that bundles are install sets, not runtime folders, and all examples match schema.

---

## Issue 13: End-to-end and regression tests

**Goal:** Cover the full bundle flow across CLI/config/install/remove behavior.

**Files:**
- Modify: `src/cli.rs` tests or add suitable integration-style tests following existing repository patterns.
- Modify: `tests/schema_files.rs`

**Steps:**

- [ ] Add an end-to-end tempdir test for local bundle add:
  - create `sksync.bundle.json`
  - create two local skill directories
  - run bundle add with `--agent universal`
  - assert config dependencies contain normalized sources, agents, `bundles`, and `managedByBundles: true`.
- [ ] Assert symlinks are created after non-dry-run add.
- [ ] Add an end-to-end remove test:
  - remove the bundle
  - assert dependencies created only by bundle are removed
  - assert managed symlinks/installed bodies follow existing remove semantics.
- [ ] Add conflict test where one entry matches an existing skill name with different source and assert no config changes.
- [ ] Add same-source merge test with existing manual dependency and assert `managedByBundles` remains false.
- [ ] Run full verification:
  - `cargo fmt --check`
  - `cargo test --quiet`
  - `cargo build --release --quiet`
  - `cargo clippy --quiet -- -D warnings`
  - `bun run docs:build`

**Acceptance criteria:** all agreed bundle semantics are covered by tests, including rollback/no-op on conflict and manual dependency preservation.

---

## Suggested PR slicing

If this is too large for one PR, split it as:

1. **Bundle domain + schema + manifest parser**: Issues 1-3.
2. **Config provenance + planning**: Issues 4-6.
3. **CLI inspect/add/remove behavior**: Issues 7-11.
4. **Docs and end-to-end hardening**: Issues 12-13.

Each PR should pass `cargo test --quiet`; the final PR should also pass full release-level verification.
