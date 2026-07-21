# CodeWhale agent mapping design

## Goal

Add CodeWhale as a bundled sksync agent mapping so users can select it directly with `--agent codewhale` and install portable `SKILL.md` packages in CodeWhale's native skill directories.

## Scope

Add exactly one agent alias:

| Agent | Global targetDir | Project targetDir |
|---|---|---|
| `codewhale` | `~/.codewhale/skills` | `.codewhale/skills` |

The mapping is included in the bundled `sksync.agents.example.json`, so it becomes available when a user initializes or refreshes global agent mappings with `sksync init --agents` or `sksync agents refresh`.

## Open Interpreter

Do not add an `interpreter` alias. Open Interpreter formally discovers skills in `~/.agents/skills` and `.agents/skills`; sksync already exposes those locations through `universal`. Users can install for it with `--agent universal` without duplicate links or a second mapping.

## Compatibility

CodeWhale's native global skill directory is `~/.codewhale/skills`. Its native project directory is `.codewhale/skills`, which remains available when CodeWhale is configured to scan only its own roots. The dedicated mapping therefore works in both default compatibility mode and CodeWhale-only mode.

Existing bundled mappings, including `universal`, remain unchanged. Existing users' `~/.sksync/agents.json` files are never silently rewritten; users opt into the new mapping by refreshing bundled mappings.

## Implementation

1. Add `codewhale` entries to both `global` and `project` sections of `sksync.agents.example.json`.
2. Extend the bundled-mapping parser tests in `src/infrastructure/json.rs` to assert both exact target paths.
3. Extend the schema/example coverage in `tests/schema_files.rs` so `codewhale` is required in both mapping sections.
4. Update the Agent Mappings documentation with the CodeWhale paths and a note that Open Interpreter uses `universal`.

## Verification

- Run the targeted JSON and schema tests.
- Run `cargo fmt --check`, `cargo test --quiet`, `cargo build --release --quiet`, and `cargo clippy --quiet -- -D warnings`.
- Confirm the generated example parses and exposes both `codewhale` targets.

## Out of scope

- An `interpreter` alias.
- Changes to CodeWhale configuration or Open Interpreter configuration.
- Automatically refreshing or modifying a user's existing `~/.sksync/agents.json`.
