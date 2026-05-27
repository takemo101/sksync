# Lockfile & Sync

`sksync-lock.json` records the exact resolved source and per-file hashes for every installed skill, so the same skills can be reconstructed and verified later. It is the portable lockfile **v4**.

## What the lockfile pins

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync-lock.schema.json",
  "lockfileVersion": 4,
  "generatedBy": "sksync@0.0.7",
  "generatedAt": "2026-05-17T00:00:00.000Z",
  "root": ".",
  "skills": {
    "example-skill": {
      "source": ".sksync/skills/example-skill",
      "installSource": {
        "type": "git",
        "url": "https://github.com/owner/repo.git",
        "ref": "0123456789abcdef0123456789abcdef01234567",
        "path": "path/to/skills/example-skill"
      },
      "hash": "sha256-placeholder",
      "files": [
        { "path": "SKILL.md", "hash": "sha256-placeholder" }
      ]
    }
  }
}
```

| Field | Meaning |
|---|---|
| `lockfileVersion` | Lockfile schema version (currently `4`). |
| `generatedBy` / `generatedAt` | The sksync version and timestamp that produced the lockfile. |
| `skills.<name>.source` | The local skill body path under `skillDir`. |
| `skills.<name>.installSource` | The resolved upstream — for Git, the **commit** `ref`, `url`, and subpath. |
| `skills.<name>.hash` / `files[]` | Aggregate and per-file SHA-256 hashes used by `check`. |

Because the Git `installSource.ref` is a resolved commit (not a moving branch), `sksync install` reconstructs the same content. The lockfile is portable across macOS and Linux; Linux release assets use musl binaries so the same project lockfile can be replayed across common Debian / Ubuntu environments.

## The sync commands

| Command | Reads | Writes | Purpose |
|---|---|---|---|
| [`plan --dry-run`](#plan) | config + targets | — | Preview create / in-sync / conflict / drift. |
| [`apply`](#apply) | config + targets | symlinks, lockfile | Create the planned symlinks, then write the lockfile. |
| [`install`](#install) | lockfile (or config) | bodies, symlinks, lockfile | Reconstruct skills, preferring locked sources. |
| [`update`](#update) | config (`dependencies`) | bodies, lockfile | Fetch latest from sources and re-lock. |
| [`outdated`](#outdated) | lockfile + upstream | — | Report skills with newer upstream commits. |
| [`check`](#check) | lockfile + targets | — | Verify hashes, targets, and links; non-zero on problems. |

### plan {#plan}

Reads `sksync.config.json`, inspects current target state, and reports what would be created, what is already in sync, and any conflicts or drift — without writing anything.

```sh
sksync plan --dry-run
sksync plan --global
```

### apply {#apply}

Runs only the planner's *create symlink* actions, then writes `sksync-lock.json`. Fails on missing source, conflict, or drift. `--force` allows updating a target **only** when it is an existing sksync-managed link that is safe to replace.

```sh
sksync apply
sksync apply --force
sksync apply --global
```

### install {#install}

If `sksync-lock.json` exists, install prefers the lockfile's resolved sources to rebuild skill bodies and recreate symlinks — ideal on a fresh clone. Without a lockfile, it fetches from config and creates one.

```sh
sksync install
sksync install --global
```

### update {#update}

Downloads/copies the latest (or pinned) skill from each `dependencies` source into `skillDir` and updates `sksync-lock.json`. Fetched skills are validated (`SKILL.md` + frontmatter `name`/`description`). Repo-root discovery is resolved at `add` time and stored, so `update` and `install` re-fetch the saved path.

```sh
sksync update
sksync update --global
```

### outdated {#outdated}

Compares the lockfile against upstream and lists skills that can be updated. For Git sources, it compares the remote ref's HEAD against the lockfile's resolved commit.

```sh
sksync outdated
sksync outdated --global
sksync outdated --json
```

### check {#check}

Compares `sksync-lock.json` against current state to detect source hash drift, missing targets, and broken symlinks. Source hashes come from the lockfile; target health is recomputed from the current config / agent mapping. Exits non-zero on any problem — suitable for CI-like verification.

```sh
sksync check
sksync check --global
```

::: info
There is intentionally no dedicated `ci` command. Reproducible reconstruction is consolidated into `sksync install`.
:::

## Sharing the lockfile

`sksync-lock.json` is a project-local generated artifact and is git-ignored by default. It is portable, so sharing it lets collaborators reproduce the same skills with `sksync install` across macOS and Linux. It is currently treated as local state until the sharing policy is finalized — the canonical file to commit is `sksync.config.json`.

## Examples & schema

- [`sksync-lock.example.json`](https://github.com/takemo101/sksync/blob/main/sksync-lock.example.json)
- [`schemas/sksync-lock.schema.json`](https://github.com/takemo101/sksync/blob/main/schemas/sksync-lock.schema.json)

## Related

- [Sources & Discovery](/guides/sources) — how sources are resolved before being locked.
- [Commands](/reference/commands) — full flag reference for each command above.
