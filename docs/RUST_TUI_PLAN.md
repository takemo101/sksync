# Rust / Prompt Wizard Implementation Plan

## Direction

`sksync wizard` is implemented as a prompt-style wizard. `ask` and `tui` remain compatible aliases. The CLI and prompt wizard are separate entry points only; synchronization planning, checking, and applying logic is shared.

## Recommended stack

- CLI: `clap`
- Prompt wizard: `inquire` Select / MultiSelect / Text / Confirm
- config: `serde` + `serde_json`
- errors: `anyhow` + `thiserror`
- hashing: `sha2`
- walking: `walkdir`
- tests: `tempfile` + `insta`

## Implementation order

1. Create the Cargo project.
2. Define Rust types for config and lockfile.
3. Add built-in agent mappings.
4. Add skill discovery and hash calculation.
5. Add the dry-run planner.
6. Add symlink apply.
7. Add check / list.
8. Add the prompt wizard shell.
9. Call add / remove / remove-agent / check / apply from the prompt wizard.

## Prompt Wizard MVP

- Runtime prompt labels, help, and confirmations are shown in English for international users.
- Ask for the user's intent first:
  - Add skill
  - Remove skill
  - Detach skill from agent
  - Show status
  - Apply links
- Ask for the required values for each intent in order.
- Remove / remove-agent flows select project/global scope first, then choose from skill and agent lists loaded from config.
- Remove mode is a single-select choice: `Normal removal (no option)`, `--keep-files`, or `--config-only`.
- Remove-agent selects agents from the selected skill's configured agent list.
- Show a summary / dry-run before destructive actions.
- After explicit confirmation, call the same application use case as the CLI.

## Notes

- Do not implement a persistent pane/keybinding-driven UI.
- Do not implement direct filesystem operations in the TUI.
- Persist state only in config, lockfile, or local state.
- TUI state should contain only temporary answers while a prompt flow is in progress.
