# fx Shared Target Ownership Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `fx` agent mapping and make multiple logical agent assignments safely share one physical skill symlink.

**Architecture:** `ResolvedConfig` remains the logical ownership source. Planning resolves enabled skill-agent assignments, groups them by lexical `TargetPath`, rejects groups with multiple sources, and emits one physical `LinkPlanItem` containing all owners. Apply, check, display, removal, and list consume the grouped physical state while preserving per-agent visibility where useful.

**Tech Stack:** Rust 2021, `thiserror`, `serde_json`, Cargo unit/integration tests, GitButler CLI.

## Global Constraints

- Global fx target: `~/.fx/skills`.
- Project fx target: `.agents/skills`.
- Portable lockfile remains version 5 and gains no target or owner fields.
- Shared targets are valid only when every owner resolves to the same source path.
- Desired target/source conflicts block all workflows and cannot be bypassed by `--force` or `skip_blocked_targets`.
- Grouping uses lexical resolved `TargetPath` and `SourcePath` equality; do not canonicalize parent symlinks or case aliases.
- Tests must use temporary directories and must not touch the real home directory.
- Symlink safety behavior remains conservative: never replace regular files or directories.

---

### Task 1: Reject normalized skill-name collisions

**Files:**

- Modify: `src/application/config.rs`
- Modify: `src/infrastructure/json.rs:443-524`
- Test: `src/infrastructure/json.rs` test module

**Interfaces:**

- Consumes: `SkillName: Ord` and the existing `RawConfig::resolve_with_default_scope_and_root` loops.
- Produces: `ConfigResolveError::DuplicateSkillName { name: String }` and one resolved skill per normalized name.

- [ ] **Step 1: Write failing config-resolution tests**

Add tests that parse one collision across `skills` and `dependencies` and one collision caused by trimming:

```rust
#[test]
fn rejects_duplicate_skill_name_across_skills_and_dependencies() {
    let raw = serde_json::from_str::<RawConfig>(
        r#"{
          "agents": { "pi": {} },
          "skills": {
            "review": { "source": "skills/review", "agents": ["pi"] }
          },
          "dependencies": {
            "review": { "source": "owner/repo", "agents": ["pi"] }
          }
        }"#,
    )
    .expect("raw config parses");
    let error = raw.resolve().expect_err("duplicate skill is rejected");

    assert!(matches!(
        error,
        ConfigResolveError::DuplicateSkillName { ref name } if name == "review"
    ));
}

#[test]
fn rejects_skill_names_that_collide_after_trimming() {
    let raw = serde_json::from_str::<RawConfig>(
        r#"{
          "agents": { "pi": {} },
          "skills": {
            "review": { "source": "skills/review", "agents": ["pi"] },
            " review ": { "source": "skills/other", "agents": ["pi"] }
          }
        }"#,
    )
    .expect("raw config parses");
    let error = raw.resolve().expect_err("normalized duplicate is rejected");

    assert!(matches!(
        error,
        ConfigResolveError::DuplicateSkillName { ref name } if name == "review"
    ));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```sh
cargo test --quiet rejects_duplicate_skill_name
cargo test --quiet rejects_skill_names_that_collide_after_trimming
```

Expected: compilation fails because `DuplicateSkillName` does not exist, or the tests observe successful resolution instead of an error.

- [ ] **Step 3: Add the error and reserve normalized names**

Add this error variant in `src/application/config.rs`:

```rust
#[error("duplicate normalized skill name '{name}'")]
DuplicateSkillName { name: String },
```

In `resolve_with_default_scope_and_root`, create one set before either config loop and reserve every parsed name before constructing its `ResolvedSkill`:

```rust
let mut skill_names = BTreeSet::new();

fn reserve_skill_name(
    names: &mut BTreeSet<SkillName>,
    name: &SkillName,
) -> Result<(), ConfigResolveError> {
    if !names.insert(name.clone()) {
        return Err(ConfigResolveError::DuplicateSkillName {
            name: name.as_str().to_owned(),
        });
    }
    Ok(())
}
```

Call `reserve_skill_name(&mut skill_names, &skill_name)?;` in both the `skills` and `dependencies` loops. Import `BTreeSet` where needed.

- [ ] **Step 4: Run focused and JSON tests and verify GREEN**

Run:

```sh
cargo test --quiet rejects_duplicate_skill_name
cargo test --quiet rejects_skill_names_that_collide_after_trimming
cargo test --quiet infrastructure::json::tests
```

Expected: all selected tests pass.

- [ ] **Step 5: Commit the config invariant**

Run:

```sh
but commit feat/fx-shared-target-ownership -c -m "fix: reject duplicate normalized skill names"
```

Expected: GitButler creates the feature branch and one commit containing only the plan/config-validation work currently present.

---

### Task 2: Group logical owners into physical link plan items

**Files:**

- Modify: `src/domain/link_plan.rs`
- Modify: `src/application/plan.rs`
- Test: `src/application/plan.rs` test module

**Interfaces:**

- Consumes: enabled `ResolvedSkill.agents`, `TargetResolver`, lexical `TargetPath`/`SourcePath` equality.
- Produces:
  - `LinkOwner { skill: SkillName, agent: AgentKind }`
  - `LinkPlanItem { owners: Vec<LinkOwner>, source, target, action }`
  - `LinkPlanItem::skill_label() -> String`
  - `LinkPlanItem::agent_label() -> String`
  - `LinkPlanItem::has_owner(&SkillName, &AgentKind) -> bool`
  - `PlanError::TargetSourceConflict { target: String, details: String }`

- [ ] **Step 1: Write failing shared-plan tests**

Replace the fixed target resolver with a resolver that returns the same target directory for every agent, and count filesystem inspections with `Cell<usize>`. Add these fixtures:

```rust
struct CountingLinkStore {
    state: TargetState,
    inspections: Cell<usize>,
}

impl CountingLinkStore {
    fn new(state: TargetState) -> Self {
        Self {
            state,
            inspections: Cell::new(0),
        }
    }
}

impl LinkStore for CountingLinkStore {
    fn inspect_target(
        &self,
        _target: &TargetPath,
        _expected_source: &SourcePath,
    ) -> Result<TargetState, LinkStoreError> {
        self.inspections.set(self.inspections.get() + 1);
        Ok(self.state.clone())
    }
}

struct SharedTargetResolver;

impl TargetResolver for SharedTargetResolver {
    fn resolve_agent_target(
        &self,
        agent: &AgentKind,
        _scope: Scope,
        _target_dir_override: Option<&Path>,
    ) -> Result<TargetPath, TargetResolverError> {
        TargetPath::new("/targets/shared").map_err(|error| TargetResolverError::Resolve {
            agent: agent.as_str().to_owned(),
            scope: Scope::User,
            message: error.to_string(),
        })
    }
}

fn shared_config(source: SourcePath, skill_agents: &[AgentKind]) -> ResolvedConfig {
    let agents = skill_agents
        .iter()
        .cloned()
        .map(|kind| {
            (
                kind.as_str().to_owned(),
                ResolvedAgent {
                    kind,
                    enabled: true,
                    scope: Scope::User,
                    target_dir: None,
                },
            )
        })
        .collect();

    ResolvedConfig {
        skill_dir: SourcePath::new("skills").unwrap(),
        agents,
        skills: vec![ResolvedSkill {
            name: SkillName::new("review").unwrap(),
            source,
            install_source: None,
            include: None,
            agents: skill_agents.to_vec(),
        }],
        default_agents: Vec::new(),
    }
}
```

Then add:

```rust
#[test]
fn shared_agents_produce_one_physical_plan_item_and_one_inspection() {
    let config = shared_config(
        SourcePath::new("skills/review").unwrap(),
        &[AgentKind::Pi, AgentKind::custom("universal").unwrap()],
    );
    let links = CountingLinkStore::new(TargetState::Missing);

    let plan = build_link_plan(
        &config,
        &FakeSourceStore { exists: true },
        &links,
        &SharedTargetResolver,
    )
    .expect("plan builds");

    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].owners.len(), 2);
    assert_eq!(links.inspections.get(), 1);
    assert_eq!(plan.items[0].action, PlanAction::CreateSymlink);
}
```

Add a defensive source-conflict test by manually constructing an invalid `ResolvedConfig`; runtime JSON resolution will reject this shape in Task 1, while the planner remains defensive for programmatic callers:

```rust
#[test]
fn same_target_with_different_sources_is_rejected_before_inspection() {
    let universal = AgentKind::custom("universal").unwrap();
    let mut config = shared_config(
        SourcePath::new("skills/review-a").unwrap(),
        &[AgentKind::Pi, universal.clone()],
    );
    config.skills[0].agents = vec![AgentKind::Pi];
    config.skills.push(ResolvedSkill {
        name: SkillName::new("review").unwrap(),
        source: SourcePath::new("skills/review-b").unwrap(),
        install_source: None,
        include: None,
        agents: vec![universal],
    });
    let links = CountingLinkStore::new(TargetState::Missing);

    let error = build_link_plan(
        &config,
        &FakeSourceStore { exists: true },
        &links,
        &SharedTargetResolver,
    )
    .expect_err("desired source conflict blocks planning");

    assert!(matches!(error, PlanError::TargetSourceConflict { .. }));
    assert_eq!(links.inspections.get(), 0);
}
```

Add equivalent grouping coverage for `build_desired_link_plan`.

- [ ] **Step 2: Run planner tests and verify RED**

Run:

```sh
cargo test --quiet application::plan::tests
```

Expected: compilation fails because `LinkOwner`, `owners`, and `TargetSourceConflict` do not exist.

- [ ] **Step 3: Replace singular plan ownership with grouped ownership**

Define in `src/domain/link_plan.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOwner {
    pub skill: SkillName,
    pub agent: AgentKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlanItem {
    pub owners: Vec<LinkOwner>,
    pub source: SourcePath,
    pub target: TargetPath,
    pub action: PlanAction,
}

impl LinkPlanItem {
    pub fn skill_label(&self) -> String {
        let mut values = self
            .owners
            .iter()
            .map(|owner| owner.skill.as_str())
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values.join(", ")
    }

    pub fn agent_label(&self) -> String {
        let mut values = self
            .owners
            .iter()
            .map(|owner| owner.agent.as_str())
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values.join(", ")
    }

    pub fn has_owner(&self, skill: &SkillName, agent: &AgentKind) -> bool {
        self.owners
            .iter()
            .any(|owner| &owner.skill == skill && &owner.agent == agent)
    }
}
```

Add the unskippable planner error:

```rust
#[error("target '{target}' resolves to multiple desired sources: {details}")]
TargetSourceConflict { target: String, details: String },
```

- [ ] **Step 4: Build desired groups before inspecting targets**

Add a private pending-group type in `src/application/plan.rs`:

```rust
struct DesiredGroup {
    source: SourcePath,
    target: TargetPath,
    owners: Vec<LinkOwner>,
}
```

Resolve all enabled assignments first into a `BTreeMap<PathBuf, DesiredGroup>`. On an occupied target key:

```rust
if group.source != skill.source {
    return Err(PlanError::TargetSourceConflict {
        target: target.as_path().display().to_string(),
        details: format!(
            "{} ({}) vs {} ({})",
            group.source.as_path().display(),
            group
                .owners
                .iter()
                .map(|owner| owner.agent.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            skill.source.as_path().display(),
            agent.as_str(),
        ),
    });
}
group.owners.push(LinkOwner {
    skill: skill.name.clone(),
    agent: agent.clone(),
});
```

Sort each owner vector by `(skill.as_str(), agent.as_str())`. `build_link_plan` inspects each completed group once; `build_desired_link_plan` assigns `CreateSymlink` without inspection.

- [ ] **Step 5: Migrate existing planner fixtures and verify GREEN**

Update existing assertions from `item.skill`/`item.agent` to `item.owners[0].skill`/`item.owners[0].agent` or the label helpers. Then run:

```sh
cargo test --quiet application::plan::tests
cargo test --quiet domain::link_plan
```

Expected: planner and link-plan tests pass, including one inspection for shared targets and zero inspections for desired-source conflicts.

- [ ] **Step 6: Commit the physical plan model**

Run:

```sh
but commit feat/fx-shared-target-ownership -m "feat: group shared skill targets"
```

Expected: a second commit records the domain/planner migration.

---

### Task 3: Integrate grouped plans with apply, check, and CLI presentation

**Files:**

- Modify: `src/application/apply.rs`
- Modify: `src/application/check.rs`
- Modify: `src/cli.rs:1330-1448`
- Modify: `src/cli.rs:2967-3012`
- Test: unit tests in the same modules

**Interfaces:**

- Consumes: grouped `LinkPlanItem`, `skill_label()`, and `agent_label()` from Task 2.
- Produces: one apply action per physical item, one check inspection/problem per physical item, owner-aware plan output, and deduplicated bundle cleanup targets.

- [ ] **Step 1: Write failing apply and check tests**

Update the apply fixture to construct two owners on one item:

```rust
fn shared_item(action: PlanAction) -> LinkPlanItem {
    LinkPlanItem {
        owners: vec![
            LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::Pi,
            },
            LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::custom("universal").unwrap(),
            },
        ],
        source: SourcePath::new("skills/review").unwrap(),
        target: TargetPath::new("targets/review").unwrap(),
        action,
    }
}
```

Assert `CreateSymlink` calls the fake applier once and source/conflict/drift error strings include `pi, universal`.

In check tests, pass one shared plan item to `check_lockfile_with_plan`, count `LinkStore::inspect_target` calls, and assert one missing-target problem whose agent label is `pi, universal`.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```sh
cargo test --quiet application::apply::tests
cargo test --quiet application::check::tests
```

Expected: compilation fails at singular `item.skill` and `item.agent` consumers.

- [ ] **Step 3: Migrate apply validation to owner labels**

Keep the existing physical apply loop. Replace singular error extraction with:

```rust
let skill = item.skill_label();
let agent = item.agent_label();
```

Use these labels for `SourceMissing`, `Conflict`, and `DriftedSymlink`. Because planner source conflicts return `PlanError`, they cannot enter apply or be skipped by `ApplyOptions`.

- [ ] **Step 4: Inspect each planned physical target once in check**

Change `inspect_target` to accept `agent: &str` instead of `&AgentKind`. Legacy lockfile checking passes `target.agent.as_str()`. Planned target checking passes:

```rust
inspect_target(
    problems,
    &item.skill_label(),
    &item.agent_label(),
    &item.target,
    &item.source,
    link_store,
);
```

Keep the existing `CheckProblem` shape so grouped diagnostics naturally display a comma-separated owner label without a lockfile change.

- [ ] **Step 5: Update plan output and bundle cleanup collection**

In `print_plan_item`, render:

```rust
println!(
    "{badge:<8} {} → {}",
    item.skill_label(),
    item.agent_label()
);
```

The bundle add rollback target collection now receives already-grouped `link_plan.items`; collect into a `BTreeSet<PathBuf>` before converting to `Vec<PathBuf>` so its deduplication is explicit and deterministic.

- [ ] **Step 6: Run focused workflow tests and verify GREEN**

Run:

```sh
cargo test --quiet application::apply::tests
cargo test --quiet application::check::tests
cargo test --quiet cli::tests
cargo test --quiet application::add::tests
```

Expected: all selected tests pass and shared items perform one physical mutation.

- [ ] **Step 7: Commit apply/check/presentation integration**

Run:

```sh
but commit feat/fx-shared-target-ownership -m "feat: apply shared targets once"
```

Expected: one commit contains apply, check, display, and rollback bookkeeping changes.

---

### Task 4: Preserve shared links during agent removal and reuse list inspections

**Files:**

- Modify: `src/cli.rs:2038-2272`
- Modify: `src/application/list.rs:79-159`
- Test: `src/cli.rs` test module
- Test: `src/application/list.rs` test module

**Interfaces:**

- Consumes: grouped owners on each `LinkPlanItem`.
- Produces:
  - removal predicates that unlink only a final active owner;
  - one filesystem inspection per unique `(target, source)` in `list`;
  - unchanged per-agent `ListedTarget` rows.

- [ ] **Step 1: Write failing removal predicate tests**

Extract pure predicates so filesystem behavior is independently testable:

```rust
fn removes_owner(owner: &LinkOwner, skill: &str, agents: &[AgentKind]) -> bool {
    owner.skill.as_str() == skill && agent_kinds_contain(agents, &owner.agent)
}

fn should_remove_target_for_agents(
    item: &LinkPlanItem,
    skill: &str,
    agents: &[AgentKind],
) -> bool {
    item.owners.iter().any(|owner| removes_owner(owner, skill, agents))
        && item
            .owners
            .iter()
            .all(|owner| removes_owner(owner, skill, agents))
}
```

Add this test fixture and the predicate tests:

```rust
fn shared_plan_item() -> LinkPlanItem {
    LinkPlanItem {
        owners: vec![
            LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::Pi,
            },
            LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::custom("universal").unwrap(),
            },
        ],
        source: SourcePath::new("skills/review").unwrap(),
        target: TargetPath::new("targets/review").unwrap(),
        action: PlanAction::AlreadySynced,
    }
}

#[test]
fn removing_one_shared_owner_keeps_target() {
    let item = shared_plan_item();
    assert!(!should_remove_target_for_agents(
        &item,
        "review",
        &[AgentKind::Pi],
    ));
}

#[test]
fn removing_all_shared_owners_removes_target_once() {
    let item = shared_plan_item();
    assert!(should_remove_target_for_agents(
        &item,
        "review",
        &[AgentKind::Pi, AgentKind::custom("universal").unwrap()],
    ));
}
```

Also test whole-skill removal keeps a physical target only if an owner from another skill remains.

- [ ] **Step 2: Run removal tests and verify RED**

Run:

```sh
cargo test --quiet removing_one_shared_owner_keeps_target
cargo test --quiet removing_all_shared_owners_removes_target_once
```

Expected: tests fail because the predicates and grouped-owner removal behavior do not exist.

- [ ] **Step 3: Make removal owner-aware**

Update `remove_managed_symlinks_for_agents` to call `remove_managed_symlink_target` only when `should_remove_target_for_agents` is true.

Update whole-skill removal with:

```rust
fn should_remove_target_for_skill(item: &LinkPlanItem, skill: &str) -> bool {
    item.owners.iter().any(|owner| owner.skill.as_str() == skill)
        && item
            .owners
            .iter()
            .all(|owner| owner.skill.as_str() == skill)
}
```

This preserves the link while any active owner in the current resolved config remains. Disabled agents are absent from the grouped plan and therefore do not retain a link.

- [ ] **Step 4: Write a failing list inspection-cache test**

Use two agents resolving to the same target and a fake link store with a `Cell<usize>` counter. Add the module-local fake and extend the existing config fixture with `Pi` plus custom `universal`, both resolved by the existing fake resolver to the same target directory:

```rust
struct CountingLinkStore {
    state: TargetState,
    inspections: Cell<usize>,
}

impl CountingLinkStore {
    fn new(state: TargetState) -> Self {
        Self {
            state,
            inspections: Cell::new(0),
        }
    }
}

impl LinkStore for CountingLinkStore {
    fn inspect_target(
        &self,
        _target: &TargetPath,
        _expected_source: &SourcePath,
    ) -> Result<TargetState, LinkStoreError> {
        self.inspections.set(self.inspections.get() + 1);
        Ok(self.state.clone())
    }
}

fn shared_list_config() -> ResolvedConfig {
    let universal = AgentKind::custom("universal").unwrap();
    let mut config = config(SourcePath::new("skills/review").unwrap());
    config.agents.insert(
        universal.as_str().to_owned(),
        ResolvedAgent {
            kind: universal.clone(),
            enabled: true,
            scope: Scope::User,
            target_dir: None,
        },
    );
    config.skills[0].agents.push(universal);
    config
}

#[test]
fn shared_agent_rows_reuse_one_target_inspection() {
    let store = CountingLinkStore::new(TargetState::SymlinkToExpectedSource);
    let report = list_skills(
        &shared_list_config(),
        None,
        &store,
        &SharedTargetResolver,
    );

    assert_eq!(report.skills[0].targets.len(), 2);
    assert_eq!(store.inspections.get(), 1);
    assert!(report.skills[0]
        .targets
        .iter()
        .all(|target| target.state == ListedTargetState::Synced));
}
```

- [ ] **Step 5: Cache list target states while retaining logical rows**

Inside each skill, add:

```rust
let mut inspected = BTreeMap::<(PathBuf, PathBuf), Result<TargetState, String>>::new();
```

Key by `(target path, source path)`. Use `entry(...).or_insert_with(...)` to inspect once, clone the cached `TargetState`, and still append one `ListedTarget` per agent. Convert cached error strings to `ListedTargetState::InspectFailed`.

- [ ] **Step 6: Run removal/list tests and verify GREEN**

Run:

```sh
cargo test --quiet removing_one_shared_owner_keeps_target
cargo test --quiet removing_all_shared_owners_removes_target_once
cargo test --quiet application::list::tests
cargo test --quiet cli::tests
```

Expected: shared partial removal keeps the link, final removal removes it once, and list renders two logical rows after one inspection.

- [ ] **Step 7: Commit removal and list behavior**

Run:

```sh
but commit feat/fx-shared-target-ownership -m "fix: preserve links with shared owners"
```

Expected: one commit contains the ownership-aware removal and list cache.

---

### Task 5: Add the bundled fx mapping and user documentation

**Files:**

- Modify: `sksync.agents.example.json`
- Modify: `src/infrastructure/json.rs` test module
- Modify: `tests/schema_files.rs`
- Modify: `site/guides/agent-mappings.md`
- Modify: `README.md` only if its explicit bundled-agent list requires fx

**Interfaces:**

- Consumes: bundled mapping loader using `sksync.agents.example.json`.
- Produces: `fx` global target `~/.fx/skills` and project target `.agents/skills`.

- [ ] **Step 1: Write failing bundled-mapping tests**

Add:

```rust
#[test]
fn bundled_agent_mappings_include_fx() {
    let mappings = default_agent_mapping_config().expect("bundled mappings parse");

    assert_eq!(mappings.global["fx"], Path::new("~/.fx/skills"));
    assert_eq!(mappings.project["fx"], Path::new(".agents/skills"));
}
```

Extend `agents_example_uses_documented_skill_directories` with the same exact paths and add `fx` to the compatibility-list coverage.

- [ ] **Step 2: Run mapping tests and verify RED**

Run:

```sh
cargo test --quiet bundled_agent_mappings_include_fx
cargo test --quiet agents_example_uses_documented_skill_directories
```

Expected: assertions fail because `fx` is absent.

- [ ] **Step 3: Add sorted fx mapping entries**

Add to `sksync.agents.example.json` in alphabetical position:

```json
"fx": {
  "targetDir": "~/.fx/skills"
}
```

and:

```json
"fx": {
  "targetDir": ".agents/skills"
}
```

Do not add `.fx/skills` as a project target because fx does not discover that project directory.

- [ ] **Step 4: Document shared project ownership**

Add `fx` to the bundled-default table in `site/guides/agent-mappings.md` and explain:

```markdown
**fx** uses `~/.fx/skills` for its managed user-level skills. For project scope,
fx reads the shared `.agents/skills` directory, so `fx`, `universal`, and other
compatible agents may intentionally share one physical sksync link.
```

Update the full bundled-agent list and any count-based schema assertions from 46 to 47 where the new entry changes the expected floor.

- [ ] **Step 5: Run mapping/schema tests and verify GREEN**

Run:

```sh
cargo test --quiet bundled_agent_mappings_include_fx
cargo test --quiet agents_example_uses_documented_skill_directories
cargo test --quiet agents_example_includes_skillkit_compatible_mappings
cargo test --quiet --test schema_files
```

Expected: all mapping and schema/example tests pass.

- [ ] **Step 6: Commit fx support**

Run:

```sh
but commit feat/fx-shared-target-ownership -m "feat: add fx agent mapping"
```

Expected: one commit contains mapping data, tests, and user-facing documentation.

---

### Task 6: Complete verification and independent review

**Files:**

- Modify only files required to fix findings from diagnostics, tests, or review.

**Interfaces:**

- Consumes: all prior tasks.
- Produces: a clean, reviewed branch ready for PR.

- [ ] **Step 1: Run proactive diagnostics**

Run `lsp_diagnostics` on:

```text
src/application/config.rs
src/infrastructure/json.rs
src/domain/link_plan.rs
src/application/plan.rs
src/application/apply.rs
src/application/check.rs
src/application/list.rs
src/cli.rs
tests/schema_files.rs
```

Expected: no errors.

Run `lens_diagnostics` with `mode=all` for the edited paths. Expected: no blocking diagnostics.

- [ ] **Step 2: Run the complete repository verification suite**

Run each command separately:

```sh
cargo fmt --check
cargo test --quiet
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
```

Expected: every command exits successfully with no warnings promoted to errors.

- [ ] **Step 3: Inspect the final diff**

Run:

```sh
but show feat/fx-shared-target-ownership
```

Expected: only the approved implementation, tests, mapping data, plan, and documentation appear in the branch commits. Confirm the lockfile schema/version is unchanged.

- [ ] **Step 4: Request independent code review**

Ask a reviewer to compare the implementation against:

```text
docs/superpowers/specs/2026-08-22-fx-shared-target-ownership-design.md
docs/superpowers/plans/2026-08-22-fx-shared-target-ownership.md
```

The review must specifically check duplicate physical operations, unskippable source conflicts, final-owner removal, disabled-agent behavior, lockfile v5 compatibility, and fx path correctness.

- [ ] **Step 5: Address findings test-first and rerun verification**

For each valid finding, add or adjust a failing regression test, apply the minimal fix, and rerun the focused test plus the four repository verification commands.

- [ ] **Step 6: Commit review fixes if any**

If review required changes, run:

```sh
but commit feat/fx-shared-target-ownership -m "fix: address shared target review"
```

If review found no issues, do not create an empty commit.
