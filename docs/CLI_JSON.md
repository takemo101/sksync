# CLI JSON output

`list`, `plan`, `check`, and `outdated` default to human output. Use `--json` for one newline-terminated version-1 JSON response:

```sh
sksync list --json
sksync list --json --global
sksync plan --json
sksync plan --json --global
sksync check --json
sksync check --json --global
sksync outdated --json
sksync outdated --json --global
```

The invocation uses only the selected project or global config. Listing does not fetch sources, hash current skill bodies, create a skill store, repair targets, or write state.

## Envelope

Every delivered response has `schemaVersion: 1`, `command: "list"`, `"plan"`, `"check"`, or `"outdated"`, `scope: "project"` or `"global"`, `ok`, `data`, and `error`. Success has `error: null`. Input-loading or serialization failures have `data: null`; list resolution/inspection failures retain the collected report in `data`.

```json
{
  "schemaVersion": 1,
  "command": "list",
  "scope": "project",
  "ok": true,
  "data": { "skills": [], "lockfileStatus": "missing" },
  "error": null
}
```

The published schema is [`schemas/sksync-output.schema.json`](../schemas/sksync-output.schema.json). Error objects contain a stable `code`, an explanatory `message`, and an optional `hint`. Clients should ignore unknown fields and use codes rather than parsing messages.

## List data

- `lockfileStatus` is `available` or `missing`.
- `skills` is an array sorted by skill name. Each row has `name`, its managed body `source`, nullable `installSource`, nullable `include`, nullable `lockedHash`, and a `targets` array.
- `installSource` represents the configured source: `{ "type": "local", "path": "..." }` or `{ "type": "git", "url": "...", "path": "...", "ref": "..." }`. An absent Git ref is omitted; a legacy skill without an install source has `installSource: null`.
- `include` is the configured string-array filter or `null`. `lockedHash` is the recorded hash or `null`, not a newly calculated body hash.
- Targets are sorted by agent/path. Each target has `agent`, a nullable resolved `target` path, and a `status`. Resolution failures have `target: null`; resolution/inspection failures also have a structured `error` object. Other statuses omit `error`.
- Status tags are `synced`, `missing`, `drifted`, `conflict`, `brokenSymlink`, `sourceMissing`, `resolveFailed`, and `inspectFailed`.
- Empty arrays remain arrays; explicitly nullable fields remain present.

## Plan data

Planning is read-only: it does not fetch or validate packages, hash bodies, read/write the lockfile, create directories, acquire writer guards, or repair links. `--dry-run` remains accepted but is redundant: every plan is a dry run. `--global` uses only the global config and injected/current home, with no project fallback.

- `items` contains one row per physical target, sorted by target path. Shared agents remain separate structured `owners: [{ "skill": "review", "agent": "fx" }, { "skill": "review", "agent": "universal" }]`, sorted by skill then agent; owners are never comma-joined strings.
- Each item has `source`, `target`, and `action`. Action tags are `createSymlink`, `alreadySynced`, `conflict`, `driftedSymlink`, and `sourceMissing`.
- Only `conflict` has a `reason`: `regularFile`, `directory`, or `brokenSymlink`. Only `driftedSymlink` has `actualSource`, preserving the observed symlink destination (which can be relative). Inapplicable variant fields are omitted.
- `applicable` is true only when every action is `createSymlink` or `alreadySynced`; an empty plan is applicable. It describes normal **non-force** apply, not whether force could repair a symlink.

```json
{
  "schemaVersion": 1,
  "command": "plan",
  "scope": "project",
  "ok": true,
  "data": {
    "items": [{
      "owners": [{ "skill": "review", "agent": "universal" }],
      "source": "/work/project/body",
      "target": "/work/project/.agents/skills/review",
      "action": "conflict",
      "reason": "regularFile"
    }],
    "applicable": false
  },
  "error": null
}
```

A completed plan always exits `0`, including blockers (`ok: true`, `applicable: false`). Loading/planning errors exit `1` with `ok: false` and `data: null`: config errors use `CONFIG_NOT_FOUND`/`INVALID_CONFIG`, target resolution uses `TARGET_RESOLUTION_FAILED`, target inspection uses `INSPECTION_FAILED`, and source inspection I/O uses `IO_ERROR`. Human mode retains the same plan and exits. Serialization/write failures follow the shared behavior below.

## Check data

Checking is read-only and local: it loads the required lockfile and selected config, hashes recorded bodies, and inspects desired targets. It does not fetch sources, create directories, acquire writer guards, write state, or repair links. `--global` uses only the global config/lockfile and current home, without project fallback.

- `healthy` is true only when `problems` is empty; empty arrays remain arrays.
- Problems are sorted lexicographically by kind, skill, agent, then path. Fields not relevant to a variant are omitted.
- `sourceHashDrift` and `includeMismatch` have `skill`, `expected`, and `actual`. Include values retain the existing report strings (comma-separated patterns or `<full package>`).
- Target findings (including `inspectFailed`) emit one JSON row per actual `(skill, agent)` owner pair. A shared physical target is inspected only once; owner identifiers are never comma-joined or combined into inferred pairs. Human output keeps one grouped physical finding and its existing owner labels/count.
- `targetMissing` has `skill`, `agent`, and `path`.
- `targetUnexpectedSymlink` and `brokenSymlink` also have `actualSource`, preserving the observed symlink destination.
- `targetConflict` has `skill`, `agent`, `path`, and the report's explanatory `reason` string.
- `inspectFailed` has `skill`, `agent`, and `message`; `hashFailed` has `skill` and `message`. These findings remain in the completed report alongside other problems.

```json
{
  "schemaVersion": 1,
  "command": "check",
  "scope": "project",
  "ok": false,
  "data": {
    "healthy": false,
    "problems": [{
      "kind": "targetMissing",
      "skill": "review",
      "agent": "universal",
      "path": "/work/project/.agents/skills/review"
    }]
  },
  "error": {
    "code": "CHECK_FAILED",
    "message": "Installed state does not match the lockfile."
  }
}
```

Healthy checks exit `0` with `ok: true` and `error: null`. Completed unhealthy checks exit `1` with `CHECK_FAILED` and the **full report** in `data`, including hash/inspection failures. Aborted input-loading or target-resolution failures also exit `1`, but have `data: null`: missing required lockfiles use `LOCKFILE_NOT_FOUND`, malformed ones use `INVALID_LOCKFILE`, unreadable ones (including dangling lockfile symlinks) use `IO_ERROR`, and config/resolution errors use the shared typed codes. Human mode retains the same report, grouping, and exit decisions; no duplicate summary is appended to JSON.

## Outdated data and migration

`outdated` queries configured Git refs against recorded Git install sources in the required lockfile. It does not install content, hash bodies, inspect/repair links, acquire writer guards, or write state. Local dependencies and up-to-date Git sources are omitted. This is a current observation, not a frozen update plan: a moving ref may change before an update.

- `rows` contains successful outdated results sorted by skill, preserving the six string fields `skill`, `current`, `wanted`, `latest`, `source`, and `status` (`outdated`).
- `problems` contains failed probes sorted by skill, with `skill`, `source`, `wanted`, and structured `error` (`code: "REMOTE_QUERY_FAILED"` and an explanatory `message`). Failed probes never appear as error text in `latest`. Remaining probes continue after failures.
- Empty results use `rows: []` and `problems: []`. All-success probes exit `0`, even when updates are available. Any failed probe exits `1` in **both** output modes; JSON has `ok: false`, top-level `REMOTE_QUERY_FAILED`, and retains successful rows plus every problem in `data`. Human output shows both rows and failures, without claiming everything is current.
- Input-loading failures exit `1` with `data: null`: `CONFIG_NOT_FOUND`, `INVALID_CONFIG`, `LOCKFILE_NOT_FOUND`, `INVALID_LOCKFILE`, or `IO_ERROR`. Global scope uses only the global config/lockfile, without project fallback.

**Breaking change:** `outdated --json` now emits the shared envelope instead of a bare array. There is no legacy-array flag. Replace `jq '.[]'` with `jq '.data.rows'`, and inspect `.ok`, `.error`, and `.data.problems`. Exit `1` can carry usable partial rows:

```sh
status=0
sksync outdated --json > outdated.json || status=$?
jq '.ok, .error, .data.problems, .data.rows' outdated.json
printf 'sksync exit status: %s\n' "$status"
```

## Exit behavior and compatibility

For `list`, a missing optional lockfile succeeds (exit `0`) with null locked hashes. Existing malformed lockfiles now fail instead of being silently ignored (`INVALID_LOCKFILE`). Missing configs use `CONFIG_NOT_FOUND`; invalid configs use `INVALID_CONFIG`; unreadable inputs use `IO_ERROR`.

For `list`, resolution/inspection failures use `TARGET_RESOLUTION_FAILED` or `INSPECTION_FAILED`, set `ok: false`, retain collected rows in JSON, and exit `1` in both human and JSON modes. Missing links, source-missing bodies, drift, broken symlinks, and conflicts remain successful observations in both modes.

A pre-write serialization failure, including an unrepresentable Unix path, emits one failed envelope with `data: null`, `SERIALIZATION_FAILED`, and exit `1`. A stdout write failure may prevent JSON delivery and is propagated without retrying or emitting a second response. Malformed CLI syntax retains Clap's stderr text and exit `2`; it is outside the JSON envelope contract.
