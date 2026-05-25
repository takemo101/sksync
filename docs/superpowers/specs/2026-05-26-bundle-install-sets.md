# Bundle Install Sets Design

## Summary

`sksync` will support bundle manifests as curated install sets. A bundle is not a runtime object and agents never see bundle folders. Instead, `sksync bundle add` expands bundle entries into normal dependencies using agents chosen by the user, records bundle provenance on those dependencies, and keeps dependency lifecycle as the primary model.

## Domain language

- **Bundle**: a curated install set containing multiple bundle entries.
- **Bundle Entry**: a skill reference inside a bundle. It names the resulting skill and points to its source. It does not choose target agents.
- **Dependency**: the existing config unit that links one skill source to selected agents. Bundle entries become dependencies only after the user chooses agents at add time.
- **Bundle provenance**: local metadata on a dependency that records which bundles caused or also justify that dependency.

## Bundle manifest

Bundle sources point to a directory containing `sksync.bundle.json`. The manifest is looked up directly at `<source>/sksync.bundle.json`; recursive discovery is intentionally not part of the first version.

Minimal manifest:

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json",
  "name": "review-workflow",
  "description": "Skills for review and QA workflows.",
  "entries": {
    "review": {
      "source": "./skills/review"
    },
    "qa": {
      "source": "github:org/qa-skills/skills/qa#main"
    }
  }
}
```

Validation is strict:

- `name` is required and follows `SkillName`-like rules: non-empty and no path separators.
- `description` is a required non-empty string.
- `entries` is a required non-empty object.
- Each entry key is the final skill name and follows `SkillName` rules.
- Each entry source is a required non-empty string.
- Agents are not allowed in the manifest.
- Unknown fields are rejected by schema/parser.

## Source normalization

Bundle entries can reference both local/manifest-relative sources and external sources.

- GitHub bundle + relative entry source: resolve against the same repo/ref used to fetch the bundle manifest.
- Local bundle + relative entry source: resolve against the manifest directory, then store it relative to the config root when possible; otherwise store an absolute path.
- External sources use the same normalization policy as `sksync add`.
- `skills.sh` input is accepted as input only and is saved as an exact GitHub tree URL after selection/normalization.
- Config stores normalized dependency sources, not the raw bundle entry strings.

Bundle provenance stores the exact manifest source only. It does not store the user's originally requested source string.

## Config provenance

`dependencies.*` gains two optional fields:

```json
{
  "dependencies": {
    "review": {
      "source": "https://github.com/org/review/tree/abc123/skills/review",
      "agents": ["pi"],
      "bundles": [
        {
          "name": "team-baseline",
          "source": "https://github.com/org/bundles/tree/abc123/review-workflow"
        }
      ],
      "managedByBundles": true
    }
  }
}
```

Rules:

- `bundles` is an array because one dependency may come from multiple bundles.
- `managedByBundles` defaults to `false` when omitted.
- New dependencies created by `bundle add` use `managedByBundles: true`.
- Existing manual dependencies that receive bundle provenance keep `managedByBundles: false`.
- Lockfile v4 does not store bundle provenance; provenance is config/UX metadata, not content-reproducibility data.

## CLI

Initial commands:

```bash
sksync bundle inspect <source>
sksync bundle add <source> --agent pi [--agent claude-code] [--global] [--dry-run]
sksync bundle remove <name> [--source <exact-source>] [--global] [--dry-run]
```

The first version is CLI-only. Wizard flows are intentionally deferred.

### `sksync bundle inspect <source>`

Read-only. It fetches/parses the manifest and displays:

- bundle name
- description
- entry skill names
- original entry source
- normalized source

It does not inspect the current config. Config-aware planning belongs to `bundle add --dry-run`.

### `sksync bundle add <source>`

Agents are required in CLI mode because bundle manifests do not choose target agents.

Behavior:

- Fetch and strictly validate `sksync.bundle.json`.
- Normalize all entry sources.
- Build a plan before writing anything.
- If any entry conflicts, stop the whole operation and write nothing.
- If no conflicts exist, apply all dependency changes atomically at config/lockfile level.
- Run install/apply behavior for the resulting dependencies.
- On failure, roll back config and lockfile. Filesystem cleanup is best-effort and limited to sksync-managed artifacts created by this operation.

`bundle add --dry-run` classifies entries as:

- `create`: a new dependency would be created.
- `merge`: an existing dependency with the same source would receive union-merged agents and bundle provenance.
- `conflict`: the skill name exists with a different source or another unsafe condition.
- `skipped`: the same bundle provenance and requested agents are already present.

Existing dependency rules:

- Same skill name + same normalized source: union-merge agents and add bundle provenance.
- Same skill name + different source: conflict.
- Conflicts abort the whole add; partial add is not supported.

### `sksync bundle remove <name>`

Removal uses local provenance only and never refetches the remote bundle manifest.

Identification:

- The user normally passes bundle name.
- If multiple provenance entries have the same name but different sources, return an ambiguous error and ask for `--source <exact-source>`.
- `--source` disambiguates by exact stored provenance source.

Execution:

- Remove the matching provenance from each dependency's `bundles` list.
- If `bundles` becomes empty and `managedByBundles == true`, remove the dependency using existing remove semantics.
- If `managedByBundles == false`, keep the dependency and only detach provenance.

`bundle remove --dry-run` classifies affected dependencies as:

- `remove`: dependency would be removed because bundle provenance becomes empty and `managedByBundles` is true.
- `detach-provenance`: provenance would be removed but dependency would remain.
- `ambiguous`: the bundle name matches multiple stored sources and needs `--source`.
- `not-found`: no dependency has the requested bundle provenance.

## Rollback and safety

`bundle add` is atomic at the config/lockfile level. A single conflict or failure aborts the whole operation. Filesystem rollback is best-effort and only touches artifacts known to be created by the failed operation and managed by sksync. Unmanaged files are never removed or overwritten.

`bundle remove` delegates actual dependency deletion to existing remove semantics, so installed files are deleted only when they are managed and under the configured `skillDir`.

## Schema and docs impact

Implementation should update:

- `schemas/sksync.bundle.schema.json` for bundle manifests.
- `schemas/sksync.schema.json` to allow `dependencies.*.bundles` and `managedByBundles`.
- `sksync.config.example.json` or a new bundle example fixture.
- `README.md` command overview and bundle section.
- `docs/DESIGN.md` domain/design details.
- `site/guides/project-config.md` for provenance fields.
- `site/guides/bundles.md` or `site/guides/sources.md` for bundle manifests and source behavior.
- `site/reference/commands.md` for `sksync bundle` commands.
- Tests for schema files and example fixtures.

## Explicit non-goals for the first version

- No wizard support.
- No recursive bundle manifest discovery.
- No bundle-level installed object.
- No lockfile bundle provenance.
- No partial bundle add.
- No automatic remote manifest fetch during `bundle remove`.
- No agents inside `sksync.bundle.json`.
