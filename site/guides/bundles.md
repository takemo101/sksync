# Bundles

Bundles are curated install sets for teams that want to add several skills together. A bundle is **not** a runtime folder and agents never read bundle directories. `sksync bundle add` expands bundle entries into normal dependencies, using the agents you choose at add time.

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
- `entries` keys are final skill names.
- Entry `source` values use the same source forms as `sksync add`.
- Relative entry sources resolve from the bundle manifest directory.
- Agents are intentionally not allowed in the manifest.

## Inspect

```sh
sksync bundle inspect ./bundles/review-workflow
```

`inspect` is read-only. It prints the bundle metadata and each entry's normalized source. It does not inspect your current config.

## Add

```sh
sksync bundle add ./bundles/review-workflow --agent pi --agent claude-code
sksync bundle add ./bundles/review-workflow --agent pi --dry-run
```

`bundle add` requires explicit `--agent` flags because the bundle manifest does not choose targets. The add plan classifies entries as:

| Status | Meaning |
|---|---|
| `create` | A new dependency will be created. |
| `merge` | An existing dependency with the same source will receive merged agents and provenance. |
| `conflict` | The skill name already exists with a different source or another unsafe condition. |
| `skipped` | The requested agents and bundle provenance are already present. |

Any conflict aborts the whole add. Config and lockfile writes are rolled back on failure.

## Remove

```sh
sksync bundle remove review-workflow --dry-run
sksync bundle remove review-workflow
sksync bundle remove review-workflow --source ./bundles/review-workflow
```

`bundle remove` uses local config provenance only. It does not fetch the remote manifest again.

| Status | Meaning |
|---|---|
| `remove` | The dependency was created by bundles and loses its last bundle provenance, so it will be removed. |
| `detach-provenance` | Provenance will be removed, but the dependency remains. |
| `ambiguous` | The same bundle name is present from multiple sources; pass `--source`. |
| `not-found` | No matching bundle provenance exists locally. |

Manual dependencies keep `managedByBundles: false`, so removing a bundle only detaches provenance from them.

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

## Related

- [Project Config](/guides/project-config)
- [Sources & Discovery](/guides/sources)
- [Commands → bundle](/reference/commands#sksync-bundle)
