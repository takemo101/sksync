# Root bundle export design

## Goal

Make a project or global sksync configuration directly publishable as a bundle manifest in its configuration root. This supports repositories that keep committed local bundle skills under `./skills/` alongside `sksync.config.json`.

## CLI

Add `--root` to `sksync bundle export`.

```sh
# Project configuration root
sksync bundle export team --root

# Global configuration root
sksync bundle export team --global --root
```

`--root` and `--output <dir>` are mutually exclusive. The existing `--output` behavior remains unchanged.

`--root` is manifest-only: it conflicts with `--snapshot`. A snapshot must continue to use a dedicated output directory because it creates a `skills/` tree.

In project mode, `--root` writes `<current-project-root>/sksync.bundle.json`. With `--global`, it writes `~/.sksync/sksync.bundle.json`.

If that manifest already exists, the command fails without changing it. `--force` permits replacing only `sksync.bundle.json`; it never permits replacement of the configuration root or any sibling path. `--dry-run` prints the plan and writes nothing.

## Local skill layout

The recommended layout for a self-contained local bundle is:

```text
repo/
├─ sksync.config.json
├─ sksync.bundle.json
├─ skills/                 # committed local bundle sources
│  └─ review/SKILL.md
├─ .sksync/                # generated; ignored
└─ sksync-lock.json        # local state; currently ignored
```

A dependency whose source is `./skills/review` is preserved unchanged in a manifest-only export. When another project adds the bundle from this repository, the relative entry resolves from the manifest's parent directory: the source is the bundle repository's `skills/review`, not the consuming project's `./skills/review`. For a remote repository, sksync normalizes this to the corresponding repository tree source and installs the result into the consuming project's configured skill directory.

`--root` does not create, copy, delete, or ignore `skills/`. Users who use it as local bundle source content must commit it. Documentation must no longer recommend blanket-ignoring `skills/`; it may only be ignored in projects retaining the legacy generated skill store.

## Implementation design

Represent bundle-export destinations explicitly instead of treating the configuration root as a replaceable directory:

- **Directory destination:** retain the current staging-directory and replacement workflow for `--output <dir>`, including snapshot support.
- **Manifest destination:** add a file-oriented target for `--root`. Serialize the manifest to a temporary sibling file and atomically rename it to `sksync.bundle.json` after validation.

The manifest destination accepts only `BundleExportMode::ManifestOnly`. Its existing-output check applies to the manifest file, not its parent directory. Existing root/protected-state checks remain in force for directory destinations; the root manifest target bypasses only the forbidden-root-directory case because it never replaces that directory.

## Errors

- `--root` with `--output`: reject during argument parsing.
- `--root` with `--snapshot`: reject during argument parsing or with a clear validation error.
- Existing root manifest without `--force`: report the existing manifest path and leave it unchanged.
- Serialization, temporary-file, or rename failures: leave the prior manifest and all other root files unchanged; clean up best-effort temporary files.

## Tests

Add CLI coverage for:

1. Project `--root` exports `sksync.bundle.json` and preserves relative `./skills/<name>` entries.
2. Global `--global --root` exports to the global configuration root.
3. Existing manifest is refused without `--force`; `--force` replaces only that manifest.
4. `sksync.config.json`, `sksync-lock.json`, `.sksync/`, and committed `skills/` sentinel content remain unchanged for normal and forced root export.
5. `--dry-run` does not create or modify the manifest.
6. `--root --snapshot` and `--root --output` are rejected.
7. Existing directory-output and snapshot-export tests remain unchanged, protecting backward compatibility.

## Documentation

Update the bundle guide and command reference with root-export commands, its manifest-only constraint, and its overwrite rule. Update project configuration / ignore guidance to distinguish generated `.sksync/` from version-controlled local bundle sources in `skills/`.

## Out of scope

- Root snapshot export.
- Copying installed skill bodies into `./skills/` during root export.
- Automatically editing `.gitignore` or creating local skill directories.
