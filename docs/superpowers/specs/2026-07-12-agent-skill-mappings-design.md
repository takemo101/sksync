# Agent Skill Mapping Corrections and Additions

## Goal

Bring sksync's bundled agent mappings into line with the documented skill-discovery paths for Pi, Grok CLI, and Zero.

## Scope

- Keep Pi's global mapping at `~/.pi/agent/skills`.
- Change Pi's project mapping from `.pi/agent/skills` to `.pi/skills`.
- Add Grok CLI mappings:
  - global: `~/.grok/skills`
  - project: `.grok/skills`
- Add Zero's supported global mapping only: `~/.local/share/zero/skills`.
- Do not invent a Zero project mapping: Zero discovers a single user-level path (`$XDG_DATA_HOME/zero/skills`, defaulting to `~/.local/share/zero/skills`) unless `ZERO_SKILLS_DIR` is explicitly overridden.
- Update user-facing documentation and automated tests that state the old Pi project path or need to cover the new mappings.

## Design

`sksync.agents.example.json` remains the source of bundled mappings. The new `grok` and `zero` keys are custom agent names, so they require no `AgentKind` enum additions. Grok has mappings in both `global` and `project`; Zero is placed only under `global`.

The Pi correction applies only to project scope. Global Pi skills remain under the agent configuration directory, as required by Pi's documentation.

## Validation

- Run schema tests and the focused mapping tests.
- Run the repository-required Rust verification suite before opening the PR:
  - `cargo fmt --check`
  - `cargo test --quiet`
  - `cargo build --release --quiet`
  - `cargo clippy --quiet -- -D warnings`
- Inspect the final diff before committing and opening a PR.

## Non-goals

- Changing Pi's global skills directory.
- Adding unsupported Zero project skill discovery.
- Configuring Grok or Zero provider credentials/models.
