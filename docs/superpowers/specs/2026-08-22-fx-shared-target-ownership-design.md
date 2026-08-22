# fx agent mapping and shared target ownership design

## Goal

Add first-class `fx` skill-directory support without treating agent names as exclusive owners of filesystem targets.

Multiple agents may intentionally resolve the same skill to the same target path. sksync must preserve each agent assignment as logical configuration while planning and mutating only one physical symlink per target.

## Motivation

fx discovers project skills from several compatible roots, including `.agents/skills`. Using that shared Agent Skills directory gives fx interoperability with other tools and avoids introducing a visible top-level `skills/` directory that may conflict with application content.

The intended mapping is:

| Agent | Global `targetDir` | Project `targetDir` |
|-------|--------------------|---------------------|
| `fx`  | `~/.fx/skills`     | `.agents/skills`    |

The project path is intentionally shared with the existing `universal` mapping. The repository already has another shared mapping: `antigravity` and `universal` both resolve project skills to `.agents/skills`. Shared target ownership is therefore a general correctness requirement, not an fx-only special case.

## Current problem

The current planner emits one `LinkPlanItem` per skill-agent pair. If two enabled agents resolve the same skill to the same target, both items inspect the missing path and both become `CreateSymlink` actions.

Apply then behaves as follows:

1. The first item creates the symlink.
2. The second item attempts to create the same symlink.
3. The filesystem adapter rejects the existing target.
4. Apply exits before writing the new lockfile.

Selected-agent removal has the inverse problem: detaching one agent immediately removes its target even when another configured agent still owns the same physical link.

## Design principles

1. Agent assignments are logical ownership.
2. Filesystem links are physical resources.
3. Logical ownership remains in config.
4. Physical operations are grouped by resolved target.
5. A shared target is valid only when all owners require the same source.
6. Removing an owner must not remove a link that still has another active owner.
7. Desired-state conflicts must be detected before filesystem mutation.

## Domain model

### Logical desired links

Resolve every enabled skill-agent assignment into a logical desired link:

```rust
struct LinkOwner {
    skill: SkillName,
    agent: AgentKind,
}

struct DesiredLink {
    owner: LinkOwner,
    source: SourcePath,
    target: TargetPath,
}
```

`ResolvedConfig` remains the source of truth for these assignments. Disabled agents do not become active owners and do not keep a physical link alive.

### Physical link plan

Group desired links before inspecting the filesystem. A physical plan item contains all logical owners that share one target and source:

```rust
struct LinkPlanItem {
    owners: Vec<LinkOwner>,
    source: SourcePath,
    target: TargetPath,
    action: PlanAction,
}
```

Owners use deterministic ordering so plan output and tests remain stable.

The planner produces at most one physical item for a resolved target. The filesystem is inspected once per item, and apply creates or repairs the link at most once.

## Planning rules

Planning has two phases.

### Phase 1: build desired groups

1. Resolve each enabled skill-agent assignment to a `DesiredLink`.
2. Group links by resolved `TargetPath`.
3. Partition each target group by `SourcePath`.
4. If a target has one source group, combine its owners into one physical item.
5. If a target has multiple source groups, return a desired-state conflict before filesystem inspection or mutation.

### Phase 2: inspect physical state

For each valid physical item, inspect the target once and retain the existing action semantics:

- missing target → `CreateSymlink`
- expected symlink → `AlreadySynced`
- unexpected symlink → `DriftedSymlink`
- regular file or directory → `Conflict`
- broken symlink → existing broken-link handling
- missing source → `SourceMissing`

`build_desired_link_plan` follows the same grouping rules even though it does not inspect the filesystem.

## Desired-state conflicts

The same target resolving to different sources is a configuration conflict, not an ordinary filesystem conflict.

It must:

- fail before any filesystem mutation;
- identify the target, sources, skills, and agents involved;
- remain blocking with `--force`;
- remain blocking when add, attach, or bundle workflows use `skip_blocked_targets`;
- never be represented as a skippable regular-file or drift action.

Distinct source paths remain distinct even if their content hashes match.

## Skill-name uniqueness

Config resolution must reject duplicate normalized `SkillName` values across both `skills` and `dependencies`.

This closes an existing ambiguity where keys that differ before trimming can normalize to the same skill name, resolve to the same target leaf, and overwrite one another when the lockfile builds its skill map.

The error must be raised during config resolution, before link planning.

## Apply behavior

`apply_link_plan` validates and executes physical plan items rather than logical owner rows.

For a valid shared item:

```text
owners: [fx, universal]
source: .sksync/skills/example-skill
target: .agents/skills/example-skill
```

apply performs exactly one create, replace, or no-op.

Existing safety rules remain unchanged:

- regular files and directories are never replaced;
- `--force` may repair only the existing supported symlink drift cases;
- missing sources and desired-state conflicts block apply;
- the lockfile is written only after all planned operations succeed.

Bundle rollback bookkeeping must consume deduplicated physical targets so cleanup never relies on repeated per-owner rows. Making all apply workflows fully transactional is a separate concern and is not required by this change.

## Removal behavior

Removal is an ownership change followed by a physical-resource decision.

For each affected physical item:

1. Determine the owners selected for removal.
2. Determine the active owners that remain after the requested config change.
3. Keep the symlink when at least one active owner remains.
4. Remove the symlink only when the final active owner is removed.
5. Preserve the existing safeguard that only removes a symlink pointing to the managed source.

Examples:

| Before                 | Request      | Result                                             |
|------------------------|--------------|----------------------------------------------------|
| `[fx, universal]`      | remove `fx`  | keep link; owner becomes `universal`               |
| `[fx, universal]`      | remove both  | remove link once                                   |
| `[fx]`                 | remove skill | remove link and perform normal source cleanup      |
| `[fx, disabled-agent]` | remove `fx`  | remove link; disabled agent is not an active owner |

`--config-only` retains its current meaning and does not mutate links.

## Lockfile compatibility

The portable v5 lockfile remains unchanged.

Agent ownership and resolved target paths are intentionally not persisted in v5. The canonical ownership record is the current config, and target health is recomputed from config plus the current agent mappings.

The legacy in-memory `LockedTarget` representation must not be repurposed for this feature. No lockfile version bump or migration is required.

## Check, doctor, list, and outdated

### Check and doctor

Inspect each physical target once. Problems include the complete deterministic owner list so users can see which agent assignments share the affected link.

Desired-state source conflicts are reported directly instead of appearing as a mixture of synced and drifted owner rows.

### List

`list` may retain one row per logical agent assignment because users still need to see which agents receive a skill. Internally, it should reuse one physical inspection result for rows that share a target and source.

### Outdated

`outdated` remains skill-level. Shared ownership must not multiply remote lookups or output rows.

## Path identity

Grouping uses the resolved lexical `TargetPath` produced after the existing scope, project-root, and tilde resolution rules. Source comparison uses the resolved `SourcePath` values from config.

The implementation does not canonicalize non-existent target leaves or merge paths solely because they resolve through parent-directory symlinks, filesystem case folding, or equal content hashes. Those cases remain ordinary filesystem/path-configuration concerns.

## Presentation

`plan` displays one physical operation and its owners:

```text
CREATE   example-skill → fx, universal
         target: .agents/skills/example-skill
         source: .sksync/skills/example-skill
```

This avoids presenting two create operations when only one link will be created.

Conflict output lists every contender:

```text
CONFLICT target: .agents/skills/example-skill
         source A: ... owned by fx
         source B: ... owned by universal
```

## Compatibility

- Existing configs with unique target directories behave unchanged.
- Existing shared mappings become safe without config migration.
- Existing lockfiles remain readable and writable as v5.
- Existing per-agent configuration and CLI selection remain available.
- The `fx` mapping can safely use `.agents/skills` for project scope after shared ownership is implemented.

## Verification

Add automated coverage for:

1. Two enabled agents with the same target and source produce one inspection and one create.
2. Shared targets behave correctly when missing, synced, drifted, broken, or blocked by a file or directory.
3. A target with different desired sources fails before mutation.
4. `--force` and `skip_blocked_targets` cannot bypass a desired-state conflict.
5. Add, attach, bundle add, install, and apply handle shared mappings.
6. Removing one owner keeps the link.
7. Removing the final owner removes the link once.
8. Batch agent removal and `--config-only` preserve their documented behavior.
9. Check and doctor inspect shared targets once and report all owners.
10. List retains logical agent visibility while reusing physical state.
11. Duplicate normalized skill names across `skills` and `dependencies` are rejected.
12. The fx bundled mapping resolves to `~/.fx/skills` globally and `.agents/skills` per project.
13. The v5 lockfile round-trips without target or owner fields.

Before merging implementation changes, run the repository verification suite:

```sh
cargo fmt --check
cargo test --quiet
cargo build --release --quiet
cargo clippy --quiet -- -D warnings
```

For this design-only change, inspect the diff and confirm the Markdown renders cleanly.

## Out of scope

- Reference counting across separate project and global configs.
- Automatic cleanup of old links after an agent mapping path changes.
- Automatic cleanup when an agent is disabled by directly editing configuration.
- A new lockfile version or machine-local applied-state database.
- Full transactional rollback for every apply workflow.
- Filesystem identity across symlinked parent directories or case-insensitive aliases.
