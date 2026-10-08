# Updating dependencies

```sh
sksync update                         # all dependency-managed skills
sksync update review                  # one config dependency key
sksync update review browser --global # only the global config and lockfile
```

Names are case-sensitive dependency keys, not frontmatter names, paths, bundle names, or patterns. Surrounding whitespace is trimmed; duplicates are processed once in stable name order. Unknown names, reserved `.`/`..` names, and explicit legacy `skills` entries without an install source fail before source preparation. Bundle membership does not expand the selection. No names retains full-update behavior, skipping legacy-only skills; an empty effective dependency selection succeeds without writing a lockfile.

`update` has no `--force`, `--dry-run`, or `--json` flag and adds no wizard selection menu. Link diagnosis/repair remains the responsibility of `plan`, `check`, and `apply`.

## Selected baseline and portable lockfile

Explicit names require a readable existing lockfile in the chosen scope, with entries for **every unselected configured skill**, including legacy skills. Missing/invalid baselines fail before fetching: run a full `install`/`update` to establish the baseline. A selected entry may initially be absent. There is no lookup fallback between project and global scope.

A selected update merges prepared content into that baseline, replacing only selected entries' resolved install source, include filter, directory hash, and per-file hashes. Every unselected entry is preserved, including stale entries absent from config. Unselected sources are not fetched, body content is not hashed/read, and target links are not inspected or repaired. Missing or locally edited unselected bodies do not block an otherwise disjoint selection, nor get blessed with fresh hashes. Later `check` can still report their drift.

Minimal path/symlink/ancestor identity observations and private, owned namespace probes prevent a selected body replacement from destroying an unselected dependency, legacy alias, or stale lockfile body view. Equal/nested/physically aliased protected paths fail **before preparation**; unrelated unselected-unselected overlap does not block a safe selection. If physical isolation cannot be established, the update fails closed with a safety error rather than guessing. These are namespace-safety checks, not unselected body-health checks or target inspection.

Normal writes use lockfile version 5, `root: "."`, and no target/owner records. Older supported lockfiles retain content semantics through existing normalization; legacy target metadata is not written into v5. Successful generation metadata can change. Full updates retain complete-lockfile reconciliation behavior; selective updates never prune unselected records.

The configured source/ref and include filter drive preparation. Git entries record the **exact commit actually prepared**. `outdated` is an observation, not a frozen approval plan: a moving ref can change between `outdated` and `update`. Existing resolved installed paths, including source-namespaced storage and existing flat-path compatibility, are retained. Config bytes, dependency agents, bundle provenance, mappings, and target links remain unchanged.

## Batch commit, rollback, and recovery limits

Both full and selected updates use the same guarded batch:

1. Acquire cooperative non-blocking writer guards for physical state parents and the managed store; reload and validate the guarded snapshot.
2. Prepare every selected body in private sibling staging directories, applying include filters, validating the package, and hashing staged content. Build and serialize the candidate lockfile before publication.
3. Rename old bodies to owned sibling backups, then publish prepared bodies.
4. Atomically publish the lockfile **last**; this is the commit point.
5. Clean up owned staging/backups. Cleanup failures after commit are warnings, not rollback.

A handled failure before commit restores the **whole selection** in reverse order, including original body absence, and preserves raw old lockfile bytes. Successful updates intentionally replace selected managed-body local edits. Backups are not undo history.

If rollback fails, the error includes the original failure, affected skills, and retained backup paths. Stop and recover manually from those paths; do not delete the only old copy or blindly rerun the update. Post-commit cleanup warnings identify retained paths while the new bodies/lockfile remain committed. Unknown leftovers are never automatically removed.

The guarantee is the final state after a handled failure with successful rollback, not instantaneous reader visibility, arbitrary external-editor isolation, power-loss durability, or crash/SIGKILL recovery. Agent readers may briefly observe mixed versions or missing destinations during cutover. Other mutators use the same cooperative guards and safer publication primitives, but do not gain a new command-wide transaction guarantee from this update contract. macOS and Linux are the supported platforms.
