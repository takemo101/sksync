# Agent Notes

This file gives coding agents project-specific operating guidance.

## GitButler workflow

This repository is commonly managed with GitButler. Avoid actions that make
GitButler think commits were written directly on `gitbutler/workspace`.

- Prefer `but` for version-control writes in the main working tree.
- Do not run `git reset`, `git checkout`, `git switch`, merge, rebase, or other
  direct branch-sync commands on `gitbutler/workspace`.
- After PRs are merged, sync the main working tree with GitButler:
  - `but pull --check`
  - `but pull --status-after`
- If `but pull` reports direct commits on `gitbutler/workspace`, do not keep
  trying random git mutations. First confirm the worktree is clean, then recover
  deliberately:
  - try `but teardown --status-after`
  - if teardown succeeds, return to `main`, sync it with `origin/main`, run
    `but setup`, then `but pull --status-after`
  - if teardown fails with `No active branches found`, avoid touching local
    changes, switch to normal `main`, sync it with `origin/main`, then run
    `but setup` and `but pull --status-after`
- Temporary clones may be used for PR creation/merge work when the GitButler
  workspace is awkward, but the main working tree should still be synced through
  `but pull` afterward.

## Verification

Before opening or merging a PR that changes code, run:

```bash
cargo fmt --check
cargo test --quiet
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
```

For docs-only changes, at minimum inspect the diff and ensure Markdown renders
cleanly.

## Project safety rules

- Tests must not touch the real home directory; use temporary directories.
- Symlink creation and deletion must stay conservative and avoid overwriting
  unmanaged files.
- Generated skill bodies belong under `.sksync/skills/`.
- `sksync-lock.json` is currently treated as local state and remains ignored
  until the sharing policy is finalized.
