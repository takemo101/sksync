# Agent Skill Mappings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct Pi's project skill mapping and add the documented Grok CLI and Zero skill targets to sksync's bundled mappings.

**Architecture:** The bundled `sksync.agents.example.json` is the canonical mapping source. Pi's project target changes to `.pi/skills`; Grok receives global and project mappings; Zero receives only its documented user-level data-directory mapping. Existing docs and focused tests are updated to state the same contract.

**Tech Stack:** Rust, JSON Schema, Markdown, Cargo test suite.

## Global Constraints

- Preserve Pi global target `~/.pi/agent/skills`.
- Use `.pi/skills` only for Pi project scope.
- Use `~/.grok/skills` and `.grok/skills` for Grok.
- Add Zero only at `~/.local/share/zero/skills`; do not add a fictional project mapping.
- Do not change the `AgentKind` enum: `grok` and `zero` are custom mapping keys.

---

### Task 1: Update bundled mappings and documentation

**Files:**

- Modify: `sksync.agents.example.json`
- Modify: `README.md`
- Modify: `docs/DESIGN.md`
- Modify: `docs/ISSUES.md`

**Interfaces:**

- Consumes: `sksync.agents.example.json` mapping schema, where each agent key has a `targetDir` string.
- Produces: bundled global and project mapping data with the documented Pi/Grok/Zero destinations.

- [ ] **Step 1: Update the JSON mappings**

Add this global mapping alongside the existing agent entries:

```json
"grok": { "targetDir": "~/.grok/skills" },
"zero": { "targetDir": "~/.local/share/zero/skills" }
```

Add this project mapping alongside the existing entries:

```json
"grok": { "targetDir": ".grok/skills" }
```

Replace Pi's project value with:

```json
"pi": { "targetDir": ".pi/skills" }
```

- [ ] **Step 2: Update documentation**

Make the README mapping table and design documents state Pi's corrected project directory. Add Grok's two paths and Zero's global-only path, explicitly noting that Zero has no project discovery path.

- [ ] **Step 3: Inspect the documentation and JSON diff**

Run: `but diff`

Expected: only the Pi project correction, Grok/Zero mapping additions, and matching documentation changes.

### Task 2: Cover mapping resolution with tests

**Files:**

- Modify: `src/cli.rs`
- Modify: `src/infrastructure/builtin_agents.rs`

**Interfaces:**

- Consumes: `AgentMappingConfig` global/project maps and `TargetPathResolver`.
- Produces: assertions that project precedence keeps Pi at `.pi/skills`, and that custom Grok/Zero map entries resolve to their documented destinations.

- [ ] **Step 1: Update the existing project mapping precedence test**

Use this fixture data in `project_agent_mappings_override_global_mappings`:

```rust
global: BTreeMap::from([("pi".to_owned(), PathBuf::from("~/.pi/agent/skills"))]),
project: BTreeMap::from([("pi".to_owned(), PathBuf::from(".pi/skills"))]),
```

Assert `target_dir == Path::new(".pi/skills")`.

- [ ] **Step 2: Add custom target resolver coverage**

Add tests using `AgentKind::Custom(AgentName::new("grok").unwrap())` and `AgentKind::Custom(AgentName::new("zero").unwrap())`, passing explicit paths to `TargetPathResolver::resolve`. Assert that user scope expands `~/.grok/skills` and `~/.local/share/zero/skills`, and project scope resolves `.grok/skills` below the project root.

- [ ] **Step 3: Run focused tests**

Run: `cargo test --quiet project_agent_mappings_override_global_mappings`

Expected: PASS.

Run: `cargo test --quiet custom_agent_requires_override`

Expected: PASS, confirming no built-in agent behavior was broadened.

### Task 3: Validate, commit, and submit

**Files:**

- Modify: all files above plus this plan as implementation record.

**Interfaces:**

- Consumes: completed mapping changes and passing test suite.
- Produces: a GitButler branch commit, GitHub pull request, and merged change.

- [ ] **Step 1: Run required verification**

Run:

```bash
cargo fmt --check
cargo test --quiet
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
```

Expected: all commands exit successfully.

- [ ] **Step 2: Review the final diff**

Run: `but diff`

Expected: the source/test/documentation updates match the specification without unrelated files.

- [ ] **Step 3: Commit the changes on `agent-skill-mappings`**

Run `but commit agent-skill-mappings -m "fix: update agent skill mappings" --changes <ids>` using current diff IDs.

Expected: branch contains both the approved design commit and the implementation commit.

- [ ] **Step 4: Push, create a PR, and merge it**

Push with `but push agent-skill-mappings`; create a PR against `main` with GitHub CLI; inspect CI and merge only after required checks pass. Sync the GitButler workspace afterward with `but pull --check` and `but pull --status-after`.
