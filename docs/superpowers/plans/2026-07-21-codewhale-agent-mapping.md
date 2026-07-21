# CodeWhale Agent Mapping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `codewhale` as a bundled agent mapping with native global and project skill directories.

**Architecture:** Keep mappings data-driven in `sksync.agents.example.json`, which `default_agent_mapping_config()` embeds at compile time. Add focused parser/schema tests and document CodeWhale separately from Open Interpreter, which continues to use `universal`.

**Tech Stack:** Rust, serde JSON, cargo tests, VitePress Markdown.

## Global Constraints

- Add only `codewhale`; do not add an `interpreter` alias.
- Map CodeWhale global skills to `~/.codewhale/skills` and project skills to `.codewhale/skills`.
- Keep `universal` unchanged at `~/.agents/skills` / `.agents/skills` for Open Interpreter.
- Do not modify user-owned `~/.sksync/agents.json`; users opt in through `sksync agents refresh` or `sksync init --agents`.
- Tests must not access the real home directory.
- Use `but` for commits; do not use Git write commands.

---

## Task 1: Add and validate CodeWhale bundled mappings

**Files:**

- Modify: `sksync.agents.example.json`
- Modify: `src/infrastructure/json.rs:3691-3731`
- Modify: `tests/schema_files.rs:150-213`

**Interfaces:**

- Consumes: `default_agent_mapping_config()`, which embeds `sksync.agents.example.json`.
- Produces: `mappings.global["codewhale"] == Path::new("~/.codewhale/skills")` and `mappings.project["codewhale"] == Path::new(".codewhale/skills")`.

- [ ] **Step 1: Write failing tests**

  In `src/infrastructure/json.rs`, add:

  ```rust
  #[test]
  fn bundled_agent_mappings_include_codewhale() {
      let mappings = default_agent_mapping_config().expect("bundled mappings parse");

      assert_eq!(
          mappings.global["codewhale"],
          Path::new("~/.codewhale/skills")
      );
      assert_eq!(
          mappings.project["codewhale"],
          Path::new(".codewhale/skills")
      );
  }
  ```

  In `tests/schema_files.rs`, add `"codewhale"` to the required-agent list in `agents_example_includes_skillkit_compatible_mappings` and add exact global/project target assertions to `agents_example_uses_documented_skill_directories`.

- [ ] **Step 2: Run the tests and verify they fail**

  Run:

  ```sh
  cargo test --quiet bundled_agent_mappings_include_codewhale
  cargo test --quiet --test schema_files agents_example_includes_skillkit_compatible_mappings
  ```

  Expected: the parser test panics because the `codewhale` mapping is absent; schema coverage reports a missing mapping.

- [ ] **Step 3: Add the two JSON entries**

  Add these alphabetically positioned entries in `sksync.agents.example.json`:

  ```json
  "codewhale": {
    "targetDir": "~/.codewhale/skills"
  }
  ```

  under `global`, and:

  ```json
  "codewhale": {
    "targetDir": ".codewhale/skills"
  }
  ```

  under `project`.

- [ ] **Step 4: Run focused tests and format checks**

  Run:

  ```sh
  cargo fmt --check
  cargo test --quiet bundled_agent_mappings_include_codewhale
  cargo test --quiet --test schema_files
  ```

  Expected: all commands pass and no mapping count threshold changes are needed because the new entry increases coverage.

- [ ] **Step 5: Commit the mapping and tests**

  Run `but diff`, select only `sksync.agents.example.json`, `src/infrastructure/json.rs`, and `tests/schema_files.rs`, then commit:

  ```sh
  but commit codewhale-agent-mapping-design -m "feat: add CodeWhale agent mapping" --changes <mapping-id>,<json-test-id>,<schema-test-id>
  ```

## Task 2: Document supported use and verify the repository

**Files:**

- Modify: `site/guides/agent-mappings.md:Bundled defaults (selection)`

**Interfaces:**

- Consumes: the exact mappings added in Task 1.
- Produces: user documentation for `--agent codewhale` and `--agent universal` with Open Interpreter.

- [ ] **Step 1: Update the bundled-defaults table and info guidance**

  Add:

  ```markdown
  | `codewhale` | `~/.codewhale/skills` | `.codewhale/skills` |
  ```

  to the representative mapping table. In the information callout, state that CodeWhale's native directories remain available when CodeWhale is configured to scan only its own roots. State that Open Interpreter uses the existing `universal` mapping because it discovers `~/.agents/skills` and `.agents/skills`; do not document an `interpreter` alias.

- [ ] **Step 2: Run full verification**

  Run:

  ```sh
  cargo fmt --check
  cargo test --quiet
  cargo build --release --quiet
  cargo clippy --quiet -- -D warnings
  ```

  Expected: all commands pass.

- [ ] **Step 3: Commit documentation**

  Run `but diff`, select `site/guides/agent-mappings.md`, then commit:

  ```sh
  but commit codewhale-agent-mapping-design -m "docs: describe CodeWhale skill mapping" --changes <agent-mappings-doc-id>
  ```
