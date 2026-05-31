# Skill Include Packaging Design

## Summary

`sksync` currently treats a skill package as the entire directory that contains `SKILL.md`. That works for repositories that keep each skill in its own directory, but it is wasteful for repositories whose project root is also a skill. For example, `github:ogulcancelik/herdr` has a root `SKILL.md`; selecting that skill causes sksync to install the whole repository as the skill body.

Add an explicit `include` packaging filter. `include` lets a dependency or bundle entry copy only selected files from the resolved skill package root while preserving the existing default of copying the full package when no filter is present.

## Goals

- Support root-level skills such as `herdr` without installing the whole source repository.
- Support more than manifest-only packaging, including common skill companion directories like `references/` and `assets/`.
- Preserve current behavior for existing configs and bundles when `include` is absent.
- Keep source identity separate from packaging rules.
- Make filtered installs reproducible through the lockfile.

## Non-goals

- No `exclude` field in the first version.
- No arbitrary shell-style path expansion outside the resolved skill package root.
- No support for copying protected project internals such as `.git`.
- No change to target link semantics; `plan` and `apply` still link installed skill directories.

## Configuration shape

`include` is an independent field on dependencies, not part of the source string or structured source object.

```json
{
  "dependencies": {
    "herdr": {
      "source": "github:ogulcancelik/herdr#main",
      "include": ["SKILL.md"],
      "agents": ["pi"]
    }
  }
}
```

When `include` is absent, sksync copies the whole resolved skill package root exactly as it does today.

## CLI shape

`add` accepts repeatable `--include <pattern>` and a convenience `--manifest-only` flag.

```sh
sksync add ogulcancelik/herdr --name herdr -a pi --manifest-only
sksync add org/repo/skills/review -a pi --include SKILL.md --include references
sksync add org/repo/skills/review -a pi --include SKILL.md --include 'assets/*.png'
```

`--manifest-only` is equivalent to `--include SKILL.md`. Passing both `--manifest-only` and `--include` is an error because the combined intent is ambiguous.

`attach`, `install`, and `update` do not take include flags. They use the include rules already stored in config or lockfile.

## Include pattern semantics

Patterns are evaluated relative to the resolved skill package root: the directory that contains the selected `SKILL.md` after source parsing and discovery.

Examples:

- `github:ogulcancelik/herdr#main` with `--name herdr` resolves to the repository root, so `include: ["SKILL.md"]` copies only the root `SKILL.md`.
- `github:org/repo/skills/review#main` resolves to `skills/review`, so `include: ["SKILL.md", "references"]` copies `skills/review/SKILL.md` and `skills/review/references/**`.

Supported pattern forms:

- literal file paths, such as `SKILL.md`
- literal directory paths, such as `references`; directories copy recursively
- limited glob patterns, such as `references/**` or `assets/*.png`

Validation rules:

- `include` must be a non-empty array when present.
- Patterns must be relative paths.
- Absolute paths, empty patterns, and any `..` component are invalid.
- Each pattern must match at least one file or directory.
- The final staged package must contain a valid `SKILL.md`.
- Protected directories are never copied, even if matched by a broad pattern. Protected directories include `.git`, `.sksync`, and `node_modules`.

## Bundle manifest shape

Bundle entries also accept `include`.

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json",
  "name": "agent-tools",
  "description": "Shared agent helper skills.",
  "entries": {
    "herdr": {
      "source": "github:ogulcancelik/herdr#main",
      "include": ["SKILL.md"]
    },
    "review": {
      "source": "./skills/review",
      "include": ["SKILL.md", "references"]
    }
  }
}
```

Bundle behavior:

- `bundle add` stores the entry's `include` on the generated dependency.
- `bundle sync` compares `include` as part of manifest drift. A changed include filter is a source/config drift item that should be previewed in dry-run and applied by non-dry-run sync.
- `bundle export` writes dependency include rules into exported entries.
- `bundle remove` is unaffected because it removes provenance and dependency records, not installed package contents directly.

## Lockfile v5

Because `include` changes the installed package, new lockfiles will use `lockfileVersion: 5` and store the effective include list for each locked skill.

```json
{
  "lockfileVersion": 5,
  "skills": {
    "herdr": {
      "source": ".sksync/skills/ogulcancelik/herdr/herdr",
      "include": ["SKILL.md"],
      "installSource": {
        "type": "git",
        "url": "https://github.com/ogulcancelik/herdr.git",
        "ref": "<resolved-commit>",
        "path": "."
      },
      "hash": "...",
      "files": [
        { "path": "SKILL.md", "hash": "..." }
      ]
    }
  }
}
```

Compatibility:

- v4 lockfiles remain readable.
- Missing `include` in v4 or v5 means full-package install.
- New writes use v5 once this feature ships.
- `check` fails if config include rules differ from lockfile include rules, because the installed package may no longer correspond to the desired packaging filter. The suggested fix is `sksync update` or `sksync install`.

## Installation behavior

The installer receives the resolved install source plus an optional include filter.

1. Fetch or locate the source as today.
2. Resolve the skill package root as today.
3. If no include filter is present, copy the whole package root with existing behavior. The protected-directory denylist applies to filtered copies; broadening it to full-package copies is a separate safety change.
4. If include is present, copy only matched files/directories into staging while preserving relative paths.
5. Validate the staged package with the existing `SKILL.md` parser.
6. Replace the destination atomically as today.
7. Hash and lock the staged package contents as today.

Filtered copying should be conservative. It should never follow a match outside the package root, and symlink handling should stay aligned with current copy behavior unless a later design explicitly changes symlink support.

## Command impact

### `add`

- Parse `--include` and `--manifest-only`.
- Persist include rules in dependency config.
- Roll back config, lockfile, created skill dirs, and created links as today if install/apply fails.

### `install` / `update`

- Read include from config and apply it during package installation.
- Write lockfile v5 with effective include rules.

### `check`

- Compare config include rules to lockfile include rules.
- Continue checking source hashes, file hashes, missing targets, broken symlinks, and target drift.

### `bundle add` / `bundle sync`

- Carry include rules from bundle entries into dependency config.
- Treat include changes as drift during sync.

### `bundle export`

- Manifest-only export preserves dependency include rules.
- Snapshot export rewrites sources to `./skills/<name>` and omits include because the snapshot output already contains the filtered installed package.

### `plan` / `apply` / `list` / `remove` / `outdated`

- No direct behavior changes beyond displaying or comparing include metadata where useful.
- `remove` continues deleting installed bodies according to existing safety rules.

## Schema and docs impact

Update:

- `schemas/sksync.schema.json`
- `schemas/sksync.bundle.schema.json`
- `schemas/sksync-lock.schema.json`
- `README.md`
- `site/guides/sources.md`
- `site/guides/bundles.md`
- `site/guides/lockfile.md`
- `site/reference/commands.md`
- `docs/DESIGN.md`

## Testing strategy

Unit tests:

- parse dependency config with and without include
- reject empty include arrays, absolute paths, `..`, empty patterns, and no-match patterns
- install git root skill with `include: ["SKILL.md"]` and verify only `SKILL.md` is installed
- install with directory include such as `references` and verify recursive copy
- protected directories are not copied through broad includes
- lockfile v5 serializes/deserializes include and still reads v4 without include
- `check` fails on config/lockfile include mismatch
- bundle add stores entry include on dependencies
- bundle sync detects include drift
- bundle export emits include for manifest-only exports

Integration tests:

- `sksync add <repo> --name herdr --manifest-only -a pi` installs only `SKILL.md`
- `sksync add <repo> --include SKILL.md --include references -a pi` installs both manifest and references
- `sksync update` preserves filtered installation
- `sksync install` reconstructs filtered installation from config/lockfile

## Implementation notes

- Add a small `PackageInclude` / `PackageFilter` domain type rather than passing raw strings through every layer.
- Change `DependencyConfigStore::add_dependency` to accept an options struct so future dependency metadata does not keep widening positional parameters.
- Bundle plan item comparisons use normalized include lists to avoid ordering-only drift.
- Include pattern ordering is deterministic in config and lockfile output.
