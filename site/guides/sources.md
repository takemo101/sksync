# Sources & Discovery

A skill source tells sksync where to fetch a skill body from. The same source forms are accepted by `sksync add`, `sksync attach`, and (via stored config) `sksync update` / `install`. sksync infers the source type from the string — there is no `--provider` flag and `registry:<host>/<package>` is not supported.

## Source formats

| Format | Meaning |
|---|---|
| `owner/repo/path/to/skill#ref` | GitHub shorthand. Clones `owner/repo` and uses `path/to/skill` as the skill directory. |
| `github:owner/repo/path/to/skill#ref` | Explicit GitHub shorthand. |
| `https://github.com/owner/repo/tree/ref/path/to/skill` | GitHub tree URL. Uses `ref` and path as-is. |
| `owner/repo#ref` | Repo root / parent directory. sksync discovers `SKILL.md` underneath it. |
| `skills.sh/owner/repo[/skill-or-path]#ref` | skills.sh source. Resolved internally to a GitHub repo. |
| `https://www.skills.sh/owner/repo[/skill-or-path]#ref` | skills.sh URL. Falls back to repo-root discovery if the guessed path misses. |
| `./local-skill`, `../skills/foo`, `/abs/path` | Local directory. Relative paths resolve from the directory containing the config file. |

The `#ref` suffix is optional and pins a branch, tag, or commit.

## Discovery behavior

When a source points at a repo root or parent directory rather than a directory that directly contains `SKILL.md`, sksync searches underneath it (up to depth 5):

- **Exactly one** `SKILL.md` found → selected automatically.
- **Multiple** found, interactive terminal → a multi-select prompt (skill names shown bold/cyan).
- **Multiple** found, non-interactive → error, with guidance to pass `--name <skill>` or a more specific source.
- **`--name` given** → auto-selects the single discovered skill whose frontmatter `name` or directory name matches.
- `.git`, `node_modules`, and `.sksync` are excluded from the search.

## skills.sh mapping

`skills.sh` is treated as a **URL transformer to a GitHub source**, not a registry. You can pass a `skills.sh` URL or shorthand as input, but the config stores the resolved GitHub tree URL — `https://github.com/<owner>/<repo>/tree/<ref>/<path>` — after selection.

```text
https://www.skills.sh/vercel-labs/skills/find-skills
→ https://github.com/vercel-labs/skills.git
→ skills/find-skills
→ source saved as https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills
```

When the `skills.sh` URL slug does not match the path inside the GitHub repo, repo-root discovery finds the real path and the exact GitHub tree URL is saved:

```text
https://www.skills.sh/gitbutlerapp/gitbutler/but
→ discovers crates/but/skill
→ source saved as https://github.com/gitbutlerapp/gitbutler/tree/HEAD/crates/but/skill
```

## Private repositories

sksync delegates all Git access to your local `git` auth — it has no token management of its own. If `git clone <repo>` works in your environment, the source works in sksync.

- GitHub shorthand (`owner/repo/path#ref`) becomes `https://github.com/owner/repo.git`. For private repos, be authenticated over HTTPS via a Git credential helper, GitHub CLI, or PAT.
- To use an SSH URL, use the structured source form in config:

```json
{
  "dependencies": {
    "my-skill": {
      "source": {
        "provider": "git",
        "url": "git@github.com:org/private-skills.git",
        "path": "skills/my-skill",
        "ref": "main"
      },
      "agents": ["pi"]
    }
  }
}
```

Auth errors surface as the underlying `git` command's error. `skills.sh` URLs assume public sources.

## Skill validation

Every fetched skill is validated **before** it replaces anything at the destination:

- `SKILL.md` exists,
- `SKILL.md` is a file,
- it has YAML frontmatter,
- the frontmatter has non-empty string `name` and `description`.

On failure, sksync deletes the staging directory, leaves the destination untouched, and errors out.

## Path safety

- Git subpaths reject absolute paths and `..`, and are verified to stay inside the clone directory.
- Project-scope agent target directories cannot resolve outside the project root.
- Existing plain files are never overwritten.

## Examples

```sh
# GitHub shorthand / explicit prefix / tree URL
sksync add owner/repo/path/to/skill --agent pi --agent claude-code
sksync add github:owner/repo/path/to/skill#main --agent pi
sksync add https://github.com/owner/repo/tree/main/path/to/skill --agent pi

# Repo-root discovery
sksync add owner/repo --agent pi
sksync add owner/repo --name skill-name --agent pi

# skills.sh URL / shorthand
sksync add skills.sh/owner/repo --agent pi
sksync add https://www.skills.sh/owner/repo/skill-name --agent pi

# Local directory
sksync add ./local-skill --agent pi --agent gemini
```

## Related

- [Project Config](/guides/project-config) — where sources are stored.
- [Lockfile & Sync](/guides/lockfile) — how stored sources are re-fetched.
- [Commands → add](/reference/commands#sksync-add) and [import](/reference/commands#sksync-import).
