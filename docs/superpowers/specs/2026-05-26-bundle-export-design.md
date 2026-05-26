# Bundle Export Design

## Summary

`sksync bundle export` will turn the current project or global dependencies into a shareable bundle manifest. It supports two modes: a default manifest-only export that preserves existing dependency source references, and a snapshot export that copies the currently installed skill bodies into the bundle directory and rewrites entries to manifest-relative sources.

## Proposed command

```sh
sksync bundle export <name> --output <dir> [--global] [--snapshot] [--skill <name>...] [--dry-run] [--force]
```

- `<name>` becomes the bundle manifest `name`.
- `--output <dir>` is the destination directory that will contain `sksync.bundle.json`.
- `--global` exports from `~/.sksync/config.json`; without it, export reads the project `sksync.config.json`.
- `--skill <name>` may be repeated to export a subset of dependencies.
- `--dry-run` prints planned manifest entries and copy operations without writing.
- `--force` allows replacing an existing generated manifest or snapshot directory; without it, existing output is an error.

## Mode 1: manifest-only export

Manifest-only export is the default. It reads existing dependencies and writes only `sksync.bundle.json`.

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json",
  "name": "team-baseline",
  "description": "Exported from sksync project config.",
  "entries": {
    "review": { "source": "https://github.com/org/repo/tree/main/skills/review" },
    "qa": { "source": "./vendor/qa" }
  }
}
```

This is lightweight and preserves upstream source identity. It does not copy skill bodies.

## Mode 2: snapshot export

Snapshot export copies currently installed skill bodies into the output directory and writes manifest-relative sources.

```text
bundles/team-baseline/
├─ sksync.bundle.json
└─ skills/
   ├─ review/SKILL.md
   └─ qa/SKILL.md
```

```json
{
  "name": "team-baseline",
  "description": "Exported from sksync project config.",
  "entries": {
    "review": { "source": "./skills/review" },
    "qa": { "source": "./skills/qa" }
  }
}
```

This creates a portable snapshot of what is currently installed. It is useful when upstream sources are private, unstable, or not intended to be shared. It intentionally weakens upstream update provenance because the bundle now points at copied local skill bodies.

## Export rules

- Export only dependencies, not legacy `skills.*` entries in the initial version.
- Export selected dependencies in deterministic name order.
- Do not write agents into `sksync.bundle.json`.
- Do not export existing `bundles` provenance or `managedByBundles`.
- Validate each exported skill name with the same rules used by bundle entries.
- Validate each source string with the same parser used by dependencies.
- For snapshot mode, require the installed skill directory to exist and contain a valid `SKILL.md`.
- In dry-run mode, do not create the output directory.

## Safety

- Refuse to overwrite existing output unless `--force` is passed.
- In snapshot mode, copy into a staging directory and rename into place only after all selected skills validate and copy successfully.
- On failure, remove only staging artifacts created by this operation.
- Never delete or mutate existing project/global skill bodies.
- Never mutate the source config or lockfile.

## Output planning

Dry-run should show:

```text
Bundle export plan
Name: team-baseline
Mode: manifest-only
Output: ./bundles/team-baseline
Entries (2)
- review: https://github.com/org/repo/tree/main/skills/review
- qa: ./vendor/qa
```

Snapshot dry-run should additionally show planned copy destinations:

```text
Bundle export plan
Name: team-baseline
Mode: snapshot
Output: ./bundles/team-baseline
Entries (2)
- review: ./.sksync/skills/review -> ./bundles/team-baseline/skills/review
- qa: ./.sksync/skills/qa -> ./bundles/team-baseline/skills/qa
```

## Non-goals

- No wizard flow in the first version.
- No publishing to a registry or remote repository.
- No automatic source pinning beyond preserving the dependency source already in config.
- No lockfile-driven source rewriting in the first version.
- No exporting agent selections into the bundle manifest.
