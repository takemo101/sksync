# State safety

These contracts are included in v0.0.16. The repaired code head passed native acceptance on macOS and Linux with stable Rust and Rust 1.89; the evidence and coverage limits are recorded below. Every later release candidate requires a successful native gate for its own HEAD. See [CLI JSON output](CLI_JSON.md) and [Updating dependencies](UPDATES.md) for the related contracts.

## Complete-file publication

Config mutations, agent-map refreshes, lockfile writes, and config-backup restoration serialize complete bytes before publication. They exclusively create a temporary regular file in the destination's physical parent, write and synchronize it, then rename it over the destination. They never truncate or unlink the old file first. A handled pre-publication error preserves the original bytes or original absence.

Initial config creation and conditional initial agent-map creation are **create-only**: same-directory hard-link publication cannot overwrite a competing creator. Initial config creation fails if the config already exists; a concurrently created initial mapping is left untouched. Unsupported hard-link publication fails safely without an overwriting rename fallback. Explicit agent-map refresh is a replacement, not create-only.

Only operation-owned temporary paths are cleaned up. Failure to remove an extra temporary name after create-only publication produces a warning with the retained path; it does not turn a committed write into a reported failure.

## Symlinks and permissions

An existing config/lockfile symlink to a regular file stays a symlink: publication replaces its resolved referent in that file's own parent. Dangling links and non-regular referents are rejected. Resolving the physical write destination does not change the logical base of config-relative sources or portable lockfile paths.

Replacement preserves the existing regular file's permission bits; new state files use normal creation permissions and umask. This is not a promise to preserve every filesystem metadata attribute or to provide power-loss durability.

Managed skill-body destinations must be directories strictly within the canonical configured `skillDir`. Existing body symlinks, regular files, special files, escaping parent aliases, and the store root itself are rejected. The configured store root may itself be a user-authorized symlink. Reserved skill names `.` and `..` are invalid; legitimate dot-prefixed names remain allowed. Agent target files/directories are not overwritten; `--force` remains symlink-repair-only, not an update option.

## Cooperative writer guards

Mutating entry points acquire non-blocking advisory locks on canonical physical state-parent directories and the managed store, deduplicated in deterministic order. State-file symlink referents determine the physical parents. Busy resources fail before fetching or replacement. The guarded snapshot is reloaded and checked for resource changes; guards remain held through rollback and cleanup. Directory handles, not replaceable state-file inodes or PID-marker files, own the locks. The OS releases them on process exit. Rust 1.89 is the compiler floor.

These guards coordinate sksync writers, not agent readers or arbitrary external editors. Detected unexpected destination changes block publication. Readers can still see mixed versions or briefly missing destinations during body cutover. Pure inspection commands do not acquire writer guards or create state directories.

## Update boundary and recovery

[Updating dependencies](UPDATES.md#batch-commit-rollback-and-recovery-limits) defines the full/selected batch, lockfile-last commit point, whole-selection rollback, retained backup paths, and manual-recovery limits. Successful updates replace selected managed-body local edits; backups are not undo history. Other mutators gain safer publication and guards, but not a new command-wide transaction guarantee.

Crash, SIGKILL, power-loss recovery, and isolation from non-cooperating writers are not promised. Unknown staging/backup leftovers are never automatically deleted. Stop and follow reported recovery paths after rollback failure rather than deleting the only old copy or blindly retrying.

## Native acceptance

`.github/workflows/state-contract.yml` defines four native jobs: `ubuntu-latest` and `macos-latest`, each with stable and Rust 1.89.0. Each Cargo invocation starts with temporary HOME, USERPROFILE, XDG_CONFIG_HOME, and TMPDIR while retaining toolchain homes and PATH. The jobs run formatting, the full tests, release build, clippy, and complete `cli_json`/`update_cli` suites with visible fixture diagnostics.

[State contract run 38109448646](https://github.com/takemo101/sksync/actions/runs/38109448646) passed all four jobs for repaired code head `e6a655d8f23efe315e638237ac974398d28c0d37`, using stable Rust 1.99.0 and Rust 1.89.0. Complete logs show 594 harness passes on macOS and 593 on Linux (one volume-alias test is macOS-only), including CLI 25 and update 46/45 respectively. These totals are **not all-exercised coverage**.

Both macOS jobs visibly omit two non-UTF-8 CLI fixture bodies because of EILSEQ, while exercising 16 nested-update scenarios, eight nested case-alias fixtures, and nine root-only routes. Both Linux jobs exercise the non-UTF-8 CLI bodies, eight nested scenarios, and six root-only routes. Their case-sensitive filesystem omits eight nested case-alias scenarios and reports seven other case/Unicode-alias capability omissions. A later changed HEAD requires a new successful gate; this checkpoint is not a release or deployment.

Linux rejects a skipped non-UTF-8 CLI fixture. macOS filesystems may reject those path bytes with EILSEQ: such skips remain visible and are **not exercised coverage**. Filesystem capability omissions likewise must be reported separately from test pass totals. The workflow definition and local Darwin checks alone are not proof that the native matrix passed; all four published job results are required for acceptance.
