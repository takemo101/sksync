# Bundles

Bundles are curated install sets for teams that want to add several skills together. A bundle is **not** a runtime folder and agents never read bundle directories. `sksync bundle add` expands bundle entries into normal dependencies, using the agents you choose at add time.

Use a bundle when you want a repeatable team baseline such as:

- review + QA skills for every project,
- language-specific helper skills for a repository type,
- onboarding skills that every developer should install into their preferred agents.

Do **not** use a bundle when you only need one skill. Use [`sksync add`](/reference/commands#sksync-add) instead.

## Quick workflow

```sh
# 1. Inspect what a bundle contains.
sksync bundle inspect ./bundles/review-workflow

# 2. Preview what it would change locally.
sksync bundle add ./bundles/review-workflow --agent pi --agent claude-code --dry-run

# 3. Install all entries into the chosen agents.
sksync bundle add ./bundles/review-workflow --agent pi --agent claude-code

# 4. Later, remove the bundle's local provenance.
sksync bundle remove review-workflow --dry-run
sksync bundle remove review-workflow
```

`bundle add` and `bundle remove` accept `--global` to use `~/.sksync/config.json` and global agent targets. `bundle inspect` is manifest-only and has no scope flag.

## Manifest

A bundle source is a directory containing `sksync.bundle.json`.

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

- `name` is used for local provenance and `bundle remove`.
- `description` is shown by `bundle inspect`.
- `entries` keys are final skill names in `sksync.config.json`.
- Entry `source` values use the same source forms as `sksync add`.
- Relative entry sources resolve from the bundle manifest directory.
- Agents are intentionally not allowed in the manifest.

A local bundle can keep its skills next to the manifest:

```text
bundles/review-workflow/
├─ sksync.bundle.json
└─ skills/
   ├─ review/SKILL.md
   └─ qa/SKILL.md
```

## Source behavior

Relative entry sources are resolved against the bundle manifest source:

| Bundle source | Entry source | Saved dependency source |
|---|---|---|
| `./bundles/review-workflow` | `./skills/review` | `./bundles/review-workflow/skills/review` |
| `https://github.com/org/bundles/tree/main/review` | `./skills/review` | `https://github.com/org/bundles/tree/main/review/skills/review` |
| any bundle | `github:org/repo/skills/qa#main` | normalized GitHub tree source |

`sksync` stores the normalized dependency source in config. `skills.sh` entries are accepted as input but are saved as exact GitHub tree URLs, just like `sksync add`.

## Inspect output

`inspect` is read-only. It parses the manifest and shows normalized entry sources without looking at your current config.

```sh
sksync bundle inspect ./bundles/review-workflow
```

Example output:

```text
Bundle
Name: review-workflow
Description: Skills for review and QA workflows.
Source: ./bundles/review-workflow

Entries (2)
- qa: github:org/qa-skills/skills/qa#main -> https://github.com/org/qa-skills/tree/main/skills/qa
- review: ./skills/review -> ./bundles/review-workflow/skills/review
```

## Add and dry-run statuses

`bundle add` requires explicit `--agent` flags because the bundle manifest does not choose targets.

```sh
sksync bundle add ./bundles/review-workflow --agent pi --agent claude-code --dry-run
```

Example dry-run output:

```text
Bundle add plan (2)
create review <- ./bundles/review-workflow/skills/review
merge qa <- https://github.com/org/qa-skills/tree/main/skills/qa
```

| Status | Meaning |
|---|---|
| `create` | A new dependency will be created with `managedByBundles: true`. |
| `merge` | An existing dependency with the same source will receive merged agents and provenance. |
| `conflict` | The skill name already exists with a different source or another unsafe condition. |
| `skipped` | The requested agents and bundle provenance are already present. |

Any conflict aborts the whole add. Config and lockfile writes are rolled back on failure, and sksync best-effort cleans up artifacts it created during the failed operation.

## Remove and dry-run statuses

`bundle remove` uses local config provenance only. It does not fetch the remote manifest again.

```sh
sksync bundle remove review-workflow --dry-run
```

Example dry-run output:

```text
Bundle remove plan (2)
remove review (*)
detach-provenance qa (*)
```

| Status | Meaning |
|---|---|
| `remove` | The dependency was created by bundles and loses its last bundle provenance, so it will be removed. |
| `detach-provenance` | Provenance will be removed, but the dependency remains. |
| `ambiguous` | The same bundle name is present from multiple sources; pass `--source`. |
| `not-found` | No matching bundle provenance exists locally. |

If the same bundle name appears from multiple stored sources, disambiguate with the exact source shown in config:

```sh
sksync bundle remove review-workflow --source ./bundles/review-workflow
```

## Config provenance

Bundle membership is stored on dependencies:

```json
{
  "dependencies": {
    "review": {
      "source": "https://github.com/org/repo/tree/main/skills/review",
      "agents": ["pi"],
      "bundles": [
        { "name": "review-workflow", "source": "./bundles/review-workflow" }
      ],
      "managedByBundles": true
    }
  }
}
```

The lockfile does not store bundle provenance. It only stores the content needed to reproduce installed skill bodies.

## Migrating an existing dependency into a bundle

If a dependency already exists with the same normalized source as a bundle entry, `bundle add` adopts it instead of replacing it:

1. Existing dependency remains in config.
2. Requested agents are union-merged.
3. Bundle provenance is added.
4. `managedByBundles` stays `false` unless it was already `true`.

That means a later `bundle remove` detaches provenance but keeps the manual dependency. This is the safest way to introduce bundles to an existing project.

If the skill name exists with a different source, `bundle add` reports `conflict` and writes nothing. Resolve that manually by renaming the bundle entry, removing the old dependency, or updating the dependency source.

## Authoring best practices

- Keep bundle names stable. Users type them during `bundle remove`.
- Keep entry keys stable. They become dependency names.
- Prefer explicit refs for remote entries; use tags or commits when strict reproducibility matters.
- Use manifest-relative sources when the bundle and skills live in the same repository.
- Do not put agents in the manifest. Let each user choose their own targets at add time.
- Keep bundles focused. A review bundle, language bundle, or onboarding bundle is easier to reason about than one large catch-all bundle.
- Run `sksync bundle inspect` before sharing a bundle, then run `bundle add --dry-run` in a clean temp project to verify statuses.

## Troubleshooting

| Symptom | What to do |
|---|---|
| `conflict` during add | A skill name already exists with a different source. Rename the entry or resolve the existing dependency first. |
| `ambiguous` during remove | Pass `--source <exact-source>` from the stored provenance. |
| bundle add succeeds but target links are blocked | Inspect with `sksync plan --dry-run`; existing unmanaged files are never overwritten. |
| remote bundle cannot be read | Verify the source points to a directory containing `sksync.bundle.json` and that `git clone` works locally. |

## Related

- [Project Config](/guides/project-config)
- [Sources & Discovery](/guides/sources)
- [Commands → bundle](/reference/commands#sksync-bundle)
