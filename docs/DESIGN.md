# sksync Design

アーキテクチャ上の設計原則は [`ARCHITECTURE.md`](ARCHITECTURE.md) を参照する。

## 1. 背景

Agent Skills はエージェントごとに配置先・ファイル構成が異なる。

例:

- Claude Code 用 skills
- Codex 用 instructions / skills
- Gemini CLI 用 context / extensions
- OpenCode 用 command / agent config
- pi 用 `.pi/agent/skills` またはユーザー設定配下

この差分を毎回手作業で管理すると、以下の問題が出る。

- 同じ skill を複数エージェントへコピーして内容がズレる
- 別PCへ移行したときに何を入れていたか分からない
- プロジェクトローカル skill とユーザーグローバル skill が混ざる
- エージェント追加時に再設定が面倒

`sksync` は、skill の実体を1箇所に置き、設定ファイルの mapping に基づいて各エージェントの期待するディレクトリへ symlink を張る。

## 2. コンセプト

```text
shared skill store
  └─ .sksync/skills/
      ├─ foo/SKILL.md
      └─ bar/SKILL.md

sksync.config.json / ~/.sksync/config.json
  └─ dependencies: GitHub/local source + target agents

~/.sksync/agents.json
  └─ global and project target directories per agent

sksync update
  └─ GitHub/local source -> <project>/.sksync/skills/foo

sksync apply
  ├─ ~/.pi/agent/skills/foo -> <project>/.sksync/skills/foo
  ├─ ~/.claude/skills/foo -> <project>/.sksync/skills/foo
  └─ ...
```

## 3. 用語

| 用語       | 意味                                                                |
| ---------- | ------------------------------------------------------------------- |
| skill      | エージェントが読み込む再利用可能な指示・ツール説明・テンプレート    |
| source     | SkillKit-style install source または skill の実体ディレクトリ       |
| dependency | どこから skill を取得し、どの agent へリンクするかの設定            |
| target     | 各エージェントが参照する配置先                                      |
| mapping    | agent ごとの target directory 設定                                  |
| lockfile   | 実際に同期した skill の内容・バージョン・リンク先を固定するファイル |

## 4. 設定ファイル案

`sksync` は設定を2種類に分ける。

1. **install dependency config**: どの source から skill を取得し、どの agent へ symlink するか。project (`sksync.config.json`) と global (`~/.sksync/config.json`) の両方で利用できる。
2. **agent target mapping**: agent ごとの symlink 先ディレクトリ。global/user scope と全 project 共通の project scope を `~/.sksync/agents.json` に保存する。

### install dependency config

Schema: [`schemas/sksync.schema.json`](../schemas/sksync.schema.json)

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
  "skillDir": "./.sksync/skills",
  "dependencies": {
    "reviewer": {
      "source": "github:owner/repo/skills/reviewer#main",
      "agents": ["pi", "claude-code", "codex"]
    },
    "browser": {
      "source": "https://github.com/owner/repo/tree/main/skills/browser",
      "agents": ["pi", "gemini", "opencode"]
    },
    "local-helper": {
      "source": "./vendor/local-helper",
      "agents": ["pi"]
    }
  }
}
```

SkillKit と同様に source は短い文字列を基本にする。`sksync add <source> --agent <agent>` はこの `dependencies` を更新し、そのまま update/apply まで実行する。`--global` 付きなら `~/.sksync/config.json` を更新する。

#### source formats

```text
github:owner/repo/path/to/skill#ref
owner/repo/path/to/skill#ref
owner/repo#ref
https://github.com/owner/repo/tree/ref/path/to/skill
skills.sh/owner/repo/skill-name#ref
skills.sh/owner/repo#ref
https://www.skills.sh/owner/repo/skill-name#ref
./local-skill
```

`registry:<host>/<package>` と `--provider` は扱わない。source URL transformer は source string から自動判定する。

#### add-time discovery

source が repo root や親ディレクトリを指す場合は配下の `SKILL.md` を最大 depth 5 で探索する。

- direct source に `SKILL.md` がある場合はその source を使う
- 1件だけ見つかった場合は自動選択する
- 複数見つかった場合は TTY では複数選択 prompt を出す
- 非対話環境で複数見つかった場合はエラーにする
- `--name` 指定時は frontmatter `name` または directory name に一致する discovered skill を1件だけ自動選択する
- `.git` / `node_modules` / `.sksync` は探索しない
- prompt の候補では skill 名を太字・シアンで表示する

選択した skill は、実際に見つかった subpath を反映した source として config に保存する。たとえば `owner/repo` から `skills/foo` を選んだ場合は `owner/repo/skills/foo` として保存する。

#### skills.sh transformer

`skills.sh` は registry ではなく GitHub source への URL transformer として扱う。

```text
https://www.skills.sh/vercel-labs/skills/find-skills
→ https://github.com/vercel-labs/skills.git
→ skills/find-skills
```

`skills.sh` の direct URL が実際の GitHub repo path と一致しない場合は、repo root discovery で URL slug に一致する skill を探し、実際の path を反映した source を保存する。

```text
https://www.skills.sh/mattpocock/skills/grill-me
→ discovers skills/productivity/grill-me
→ saved source: https://www.skills.sh/mattpocock/skills/productivity/grill-me
```

#### install validation

`sksync add` / `update` / `install` で取得した skill は、destination を置き換える前に以下を検証する。

- `SKILL.md` が存在する
- `SKILL.md` がファイルである
- YAML frontmatter が存在する
- frontmatter に non-empty string の `name` / `description` がある

検証に失敗した場合は destination を置き換えず、staging directory を削除してエラーにする。

内部的には source URL transformer を順に適用し、`sksync update` は dependencies から最新を取得して lockfile を更新し、`sksync install` は lockfile があれば lockfile の source を優先して再構成する。

### agent target mapping

Schema: [`schemas/sksync.agents.schema.json`](../schemas/sksync.agents.schema.json)

```json
{
  "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json",
  "global": {
    "claude-code": { "targetDir": "~/.claude/skills" },
    "cursor": { "targetDir": "~/.cursor/skills" }
  },
  "project": {
    "claude-code": { "targetDir": ".claude/skills" },
    "cursor": { "targetDir": ".cursor/skills" }
  }
}
```

`sksync.agents.example.json` は SkillKit の supported agents に近い agent keys を含め、`sksync init --global` で `~/.sksync/agents.json` として生成する。`global` は global/user scope、`project` は全 project 共通の project scope として扱う。

### 設定方針

- `skillDir` は相対パス可能
- `dependencies.*.source` がある skill は `sksync update` / `sksync install` で `skillDir/<skillName>` に配置する
- project config は project scope、global config (`--global`) は user scope として agent target を解決する
- `sksync plan/apply/check/list/install/update` は `--global` で global config / lockfile を対象にできる
- 既存互換として `skills.*.source` は local-only skill として扱う
- agent ごとの実際の target path は built-in mapping または `~/.sksync/agents.json` から解決する
- project config では全 project 共通の `project` が `global` より優先される

## 5. Built-in Agent Mapping 案

> 実際のパスは各ツールの仕様確認後に確定する。ここでは初期設計として override 可能な default を置く。

| agent       | user scope default          | project scope default | 備考                                  |
| ----------- | --------------------------- | --------------------- | ------------------------------------- |
| pi          | `~/.pi/agent/skills`        | `.pi/agent/skills`    | 既存 pi skill 形式に合わせる          |
| claude-code | `~/.claude/skills`          | `.claude/skills`      | Claude Code の skill 配置先として扱う |
| codex       | `~/.codex/skills`           | `.codex/skills`       | 将来 instructions 変換が必要かも      |
| gemini      | `~/.gemini/skills`          | `.gemini/skills`      | Gemini CLI 側仕様に合わせて調整       |
| opencode    | `~/.config/opencode/skills` | `.opencode/skills`    | OS 差分に注意                         |

## 6. Lockfile 案

ファイル名: `sksync-lock.json`

lockfile v3 は portable な情報だけを保持する。agent target path は machine-local なので保存せず、`apply` / `check` / `list` 実行時に config と agent mapping から再計算する。

```json
{
  "lockfileVersion": 3,
  "generatedBy": "sksync@0.1.0",
  "generatedAt": "2026-05-17T00:00:00.000Z",
  "root": ".",
  "skills": {
    "reviewer": {
      "source": ".sksync/skills/reviewer",
      "hash": "sha256-...",
      "files": [
        {
          "path": "SKILL.md",
          "hash": "sha256-..."
        }
      ]
    }
  }
}
```

### Lockfile に入れる情報

- skill 名
- source path
- skill ディレクトリ全体の hash
- ファイルごとの hash
- install source の resolved ref / version
- target path や symlink 状態は入れない
- sksync のバージョン

## 7. CLI / TUI コマンド案

`sksync` は Rust 製の単一バイナリとして提供する。
通常の自動化・スクリプト用途では CLI、手元で質問に答えながら実行したい場合は wizard を使う。
コマンド体系は npm に近い依存管理モデルに寄せる。ただし `ci` 相当のコマンドは現時点では設けない。

### npm-like command model

| command                                 | npm analog                      | 役割                                                                                      |
| --------------------------------------- | ------------------------------- | ----------------------------------------------------------------------------------------- |
| `sksync add <source> --agent <agent>`   | `npm install <pkg>` / `npm add` | dependency config に追加し、取得・link まで実行する                                       |
| `sksync install`                        | `npm install`                   | lockfile があれば lockfile を優先して再現し、なければ config から構成して lockfile を作る |
| `sksync update`                         | `npm update`                    | config の dependencies から最新または指定 version を取得し、lockfile を更新する           |
| `sksync remove <skill>`                 | `npm uninstall`                 | dependency / installed skill / lockfile entry / managed symlink を削除する                |
| `sksync remove <skill> --agent <agent>` | npm optional dependency removal | 指定 agent の dependency target / managed symlink だけを削除する                          |
| `sksync outdated`                       | `npm outdated`                  | lockfile の resolved source と upstream/latest を比較し、更新可能な skill を表示する      |
| `sksync apply`                          | sksync specific                 | installed skill から agent target へ symlink を反映する                                   |
| `sksync check`                          | `npm ls` / health check         | lockfile hash、source、target symlink の drift を検査する                                 |
| `sksync list`                           | `npm ls`                        | 管理中 skill と agent ごとの link 状態を一覧表示する                                      |
| `sksync wizard`                         | n/a                             | 質問形式の wizard で状態確認と操作を行う                                                  |

### `sksync init`

- project mode では `sksync.config.json` を作成
- project mode では `.sksync/skills/` ディレクトリを作成
- `--global` では `~/.sksync/config.json` を作成
- `--global` では `~/.sksync/agents.json` を作成
- `--global` では `~/.sksync/skills/` ディレクトリを作成
- 既存 config は上書きしない
- 既存 `agents.json` は上書きしない
- `--agents` では config / skills directory には触らず、`~/.sksync/agents.json` だけを bundled default mapping で強制上書きする
- built-in agent mapping のコメント付き例を出す

### `sksync add <source> --agent <agent>`

- SkillKit-style source を受け取る
- `dependencies.<skill>` を config に追加する
- `--agent` は複数指定できる
- `--global` の場合は `~/.sksync/config.json` を更新する
- 追加後に install/apply 相当の処理を実行する

### `sksync install`

- lockfile があれば `installSource` を優先して `.sksync/skills/<name>` を再構成する
- lockfile がなければ config の dependencies から取得して lockfile を作成する
- 再構成後に managed symlink を apply する
- `--global` で global config / lockfile を対象にする

### `sksync update`

- config の dependencies を元に最新または指定 version を取得する
- Git source は取得後に exact commit SHA に解決し、lockfile に保存する
- source URL transformer で Git source に変換された source は resolved commit / integrity を lockfile に保存する想定
- `update` 自体は dependency 更新と lockfile 更新を主目的とし、symlink 反映は `install` / `apply` に寄せる

### `sksync remove <skill>`

- config の `dependencies.<skill>` を削除する
- `.sksync/skills/<skill>` を削除する
- sksync が管理している symlink のみ削除する
- lockfile の該当 entry を削除する
- `--global` で global config / lockfile を対象にする
- `--keep-files` / `--config-only` で削除範囲を制御する

### `sksync remove <skill> --agent <agent>`

agent 単位削除。

- `--agent` は複数指定できる
- 指定 agent の managed symlink のみ削除する
- config の `dependencies.<skill>.agents` から指定 agent だけを削除する
- lockfile の `skills.<skill>.targets` から指定 agent の target だけを削除する
- `.sksync/skills/<skill>` 本体と他 agent の symlink は残す
- 最後の agent を削除した場合は `sksync remove <skill>` と同じ全体削除にフォールバックする
- `--global` で global config / lockfile を対象にする

### `sksync outdated`

- config と lockfile を読み込む
- Git source は lockfile の commit と remote ref の HEAD を比較する
- source URL transformer で Git source に変換された source は Git remote ref と比較する
- 更新可能な skill を `current / wanted / latest / source / status` 形式で表示する
- `--global` と `--json` をサポートする

### `sksync apply`

- config を読み込む
- target path を解決する
- 既存ファイルと衝突しないか検査する
- symlink を作成・更新する
- lockfile を生成する

### `sksync check`

- config と lockfile の差分を確認
- symlink が壊れていないか確認
- source hash と lockfile hash のズレを確認

### `sksync list`

- 管理中 skill 一覧
- agent ごとの link 状態

### `sksync wizard`

- SkillKit のような質問形式の対話フローを起動する
- `sksync ask` / `sksync tui` は互換 alias として扱う
- ユーザーに「追加 / 削除 / agent 変更 / 状態確認」などの intent を選ばせる
- intent ごとに必要な source / skill / agent / scope を順番に質問する
- 最後に dry-run plan を要約表示し、確認後に `add` / `remove` / `apply` 相当の usecase を実行する
- wizard / prompt 型の操作体験にする

## 8. Wizard 設計

`sksync wizard` は質問形式の interactive wizard とする。目的は「コマンド引数を覚えなくても安全に skill を追加・削除できること」。`sksync ask` / `sksync tui` は alias として残す。

### 対話フロー案

```text
? What would you like to do?
  > Add skill
    Remove skill
    Detach skill from agent
    Show status
    Apply links

? Skill source
  github:owner/repo/path/to/skill#main

? Select agent(s)
  [x] pi
  [x] claude-code
  [ ] codex
  [ ] gemini

Planned changes:
  add dependency: cuekit-dogfood
  install source -> .sksync/skills/cuekit-dogfood
  create symlink: .pi/agent/skills/cuekit-dogfood
  create symlink: .claude/skills/cuekit-dogfood

? Apply these changes? (y/N)
```

### TUI 操作モデル

| intent            | prompts                                                            | usecase           |
| ----------------- | ------------------------------------------------------------------ | ----------------- |
| add skill         | source, name override, agent, global scope                         | `add`             |
| remove skill      | project/global scope, configured skill list, remove mode           | `remove`          |
| detach from agent | project/global scope, configured skill list, configured agent list | `remove --agent`  |
| status            | global scope, output detail                                        | `list` / `check`  |
| apply             | global scope, force, confirmation                                  | `plan` -> `apply` |

### TUI の原則

- Runtime TUI copy is English so the prompt flow is accessible to international users.
- TUI は質問と確認に徹し、core logic を持たない
- 削除時は `Normal removal (no option)` / `--keep-files` / `--config-only` を単一選択にし、通常削除で symlink も削除する意図を明示する
- 各フローは CLI と同じ application usecase を呼ぶ
- 破壊的操作は必ず plan / summary を表示してから確認する
- TUI state は回答途中の一時入力だけにする
- 永続状態は config / lockfile / local state にだけ保存する
- 一覧確認は `list` / `check` の summary として表示し、常駐型画面は持たない

## 9. 安全ルール

- 既存の通常ファイルは上書きしない
- sksync が作った symlink だけ更新・削除できる
- target が既存 symlink の場合でも、リンク先が想定外なら警告する
- `--force` なしでは破壊的変更をしない
- `--dry-run` を標準サポートする
- lockfile に存在しないリンクを勝手に削除しない

## 10. 実装方針

初期実装は Rust を想定する。

### crate 候補

| 用途                        | crate                 |
| --------------------------- | --------------------- |
| CLI parser                  | `clap`                |
| config / lockfile serialize | `serde`, `serde_json` |
| path / home dir 解決        | `dirs`, `shellexpand` |
| hash                        | `sha2`, `hex`         |
| glob / walk                 | `walkdir`, `ignore`   |
| error handling              | `anyhow`, `thiserror` |
| Prompt wizard               | `inquire`             |
| snapshot / temp tests       | `insta`, `tempfile`   |

### モジュール構成案

```text
src/
  main.rs          # clap entrypoint
  cli.rs           # command definitions
  config.rs        # sksync.config.json model / loader
  lockfile.rs      # sksync-lock.json model / writer
  agent.rs         # built-in agent mapping
  skill.rs         # skill discovery / hashing
  planner.rs       # desired link plan / dry-run result
  apply.rs         # symlink create/update
  check.rs         # drift / broken link detection
  tui/
    mod.rs         # prompt / wizard entry
```

### アーキテクチャ方針

- `planner` が desired state と current state の差分を作る
- `apply` は planner の結果だけを実行する
- CLI と TUI は同じ `planner` / `apply` / `check` を使う
- TUI は core logic を持たない
- OS 差分は `agent` / `apply` に閉じ込める

## 11. 最小 MVP

1. `sksync.config.json` を読む
2. built-in agent mapping で target path を解決
3. `.sksync/skills/*` を対象 agent ディレクトリへ symlink
4. `sksync-lock.json` を生成
5. `sksync check` で差分検出

## 12. 未確定事項

- 各エージェントの正式な skill ディレクトリ仕様
- skill 形式が違う agent 向けに変換レイヤーを入れるか
- source URL transformer / package manager 的な install をどこまでやるか
- project scope と user scope の優先順位
- Windows での symlink 権限と junction 対応
- TUI を初期 MVP に含めるか、CLI MVP 後に追加するか
