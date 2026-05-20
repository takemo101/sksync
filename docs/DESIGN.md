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
  ├─ skills/foo/SKILL.md
  └─ skills/bar/SKILL.md

sksync.config.json / ~/.config/sksync/config.json
  └─ dependencies: GitHub/local source + target agents

~/.config/sksync/agents.json
  └─ global-only target directories per agent

sksync update
  └─ GitHub/local source -> <project>/skills/foo

sksync apply
  ├─ ~/.pi/agent/skills/foo -> <project>/skills/foo
  ├─ ~/.claude/skills/foo -> <project>/skills/foo
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

1. **install dependency config**: どの source から skill を取得し、どの agent へ symlink するか。project (`sksync.config.json`) と global (`~/.config/sksync/config.json`) の両方で利用できる。
2. **agent target mapping**: agent ごとの symlink 先ディレクトリ。global-only (`~/.config/sksync/agents.json`)。

### install dependency config

```json
{
  "$schema": "https://example.com/sksync.schema.json",
  "skillDir": "./skills",
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

SkillKit と同様に source は短い文字列を基本にする。`sksync add <source> --agent <agent>` はこの `dependencies` を更新し、そのまま update/apply まで実行する。`--global` 付きなら `~/.config/sksync/config.json` を更新する。

```text
github:owner/repo/path/to/skill#ref
owner/repo/path/to/skill#ref
https://github.com/owner/repo/tree/ref/path/to/skill
registry:skills.sh/owner/repo/skill#version
registry:example.com/owner/repo/skill#version
./local-skill
```

内部的には `repo/ref/path` または `registry:<host>/<package>#version` に正規化する。`sksync update` は dependencies から最新を取得して lockfile を更新し、`sksync install` は lockfile があれば lockfile の source を優先して再構成する。registry は `InstallSource::Registry` として分岐させ、`skills.sh` も他の registry と同じ provider 実装として扱う。

### global-only agent target mapping

```json
{
  "$schema": "https://example.com/sksync.agents.schema.json",
  "agents": {
    "pi": { "targetDir": "~/.pi/agent/skills" },
    "claude-code": { "targetDir": "~/.claude/skills" }
  }
}
```

### 設定方針

- `skillDir` は相対パス可能
- `dependencies.*.source` がある skill は `sksync update` / `sksync install` で `skillDir/<skillName>` に配置する
- project config は project scope、global config (`--global`) は user scope として agent target を解決する
- `sksync plan/apply/check/list/install/update` は `--global` で global config / lockfile を対象にできる
- 既存互換として `skills.*.source` は local-only skill として扱う
- agent ごとの実際の target path は built-in mapping または global-only `agents.json` から解決する

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

```json
{
  "lockfileVersion": 1,
  "generatedBy": "sksync@0.1.0",
  "generatedAt": "2026-05-17T00:00:00.000Z",
  "root": ".",
  "skills": {
    "reviewer": {
      "source": "skills/reviewer",
      "hash": "sha256-...",
      "files": [
        {
          "path": "SKILL.md",
          "hash": "sha256-..."
        }
      ],
      "targets": [
        {
          "agent": "pi",
          "scope": "user",
          "path": "~/.pi/agent/skills/reviewer",
          "linkType": "symlink"
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
- 同期した agent / scope / target path
- symlink か copy か
- sksync のバージョン

## 7. CLI / TUI コマンド案

`sksync` は Rust 製の単一バイナリとして提供する。
通常の自動化・スクリプト用途では CLI、手元で状態を確認しながら実行したい場合は TUI を使う。
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
| `sksync tui`                            | n/a                             | 状態確認と操作を TUI で行う                                                               |

### `sksync init`

- `sksync.config.json` を作成
- `skills/` ディレクトリを作成
- built-in agent mapping のコメント付き例を出す

### `sksync add <source> --agent <agent>`

- SkillKit-style source を受け取る
- `dependencies.<skill>` を config に追加する
- `--agent` は複数指定できる
- `--global` の場合は `~/.config/sksync/config.json` を更新する
- 追加後に install/apply 相当の処理を実行する

### `sksync install`

- lockfile があれば `installSource` を優先して `skills/<name>` を再構成する
- lockfile がなければ config の dependencies から取得して lockfile を作成する
- 再構成後に managed symlink を apply する
- `--global` で global config / lockfile を対象にする

### `sksync update`

- config の dependencies を元に最新または指定 version を取得する
- Git source は取得後に exact commit SHA に解決し、lockfile に保存する
- registry source は resolved version / artifact URL / integrity を lockfile に保存する想定
- `update` 自体は dependency 更新と lockfile 更新を主目的とし、symlink 反映は `install` / `apply` に寄せる

### `sksync remove <skill>`

- config の `dependencies.<skill>` を削除する
- `skills/<skill>` を削除する
- sksync が管理している symlink のみ削除する
- lockfile の該当 entry を削除する
- `--global` で global config / lockfile を対象にする
- `--keep-files` / `--config-only` で削除範囲を制御する

### `sksync remove <skill> --agent <agent>`

将来追加予定の agent 単位削除。

- `--agent` は複数指定できる
- 指定 agent の managed symlink のみ削除する
- config の `dependencies.<skill>.agents` から指定 agent だけを削除する
- lockfile の `skills.<skill>.targets` から指定 agent の target だけを削除する
- `skills/<skill>` 本体と他 agent の symlink は残す
- 最後の agent を削除した場合は `sksync remove <skill>` と同じ全体削除にフォールバックする
- `--global` で global config / lockfile を対象にする

### `sksync outdated`

- config と lockfile を読み込む
- Git source は lockfile の commit と remote ref の HEAD を比較する
- registry source は provider 未実装時に `registry-provider-missing` として表示する
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

### `sksync tui`

- TUI を起動する
- config / lockfile / symlink 状態を一覧表示する
- dry-run 結果を確認してから apply できる
- agent ごと、skill ごとに同期対象を一時的に切り替えられる
- 衝突・壊れた symlink・hash 差分を画面上で確認できる

## 8. TUI 設計

初期 TUI は `ratatui` + `crossterm` を想定する。

### 画面案

```text
┌ sksync ───────────────────────────────────────────────┐
│ Project: ~/workspace/my-project                       │
│ Config : sksync.config.json                           │
│ Lock   : sksync-lock.json                             │
├ Agents ───────────────┬ Skills ───────────────────────┤
│ [x] pi                │ reviewer      synced          │
│ [x] claude-code       │ browser       drifted         │
│ [ ] codex             │ planner       missing target  │
│ [x] gemini            │                               │
│ [x] opencode          │                               │
├ Details ───────────────────────────────────────────────┤
│ reviewer -> ~/.pi/agent/skills/reviewer               │
│ status: symlink ok                                    │
├ Actions ───────────────────────────────────────────────┤
│ [d] dry-run  [a] apply  [c] check  [l] lock  [q] quit │
└────────────────────────────────────────────────────────┘
```

### TUI 操作

| key     | action                    |
| ------- | ------------------------- |
| `↑/↓`   | skill / agent の移動      |
| `tab`   | pane 切り替え             |
| `space` | 一時的な enabled 切り替え |
| `d`     | dry-run                   |
| `a`     | apply                     |
| `c`     | check                     |
| `l`     | lockfile 再生成           |
| `q`     | quit                      |

### TUI の原則

- デフォルトは必ず dry-run 表示
- 破壊的・上書き系操作は確認モーダルを出す
- CLI と同じ core logic を呼び出す
- TUI 独自の状態は一時的な selection のみにする
- 実際の永続状態は config / lockfile にだけ保存する

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

| 用途                        | crate                  |
| --------------------------- | ---------------------- |
| CLI parser                  | `clap`                 |
| config / lockfile serialize | `serde`, `serde_json`  |
| path / home dir 解決        | `dirs`, `shellexpand`  |
| hash                        | `sha2`, `hex`          |
| glob / walk                 | `walkdir`, `ignore`    |
| error handling              | `anyhow`, `thiserror`  |
| TUI                         | `ratatui`, `crossterm` |
| snapshot / temp tests       | `insta`, `tempfile`    |

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
    mod.rs         # TUI app entry
    app.rs         # state machine
    ui.rs          # ratatui rendering
    events.rs      # key handling
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
3. `skills/*` を対象 agent ディレクトリへ symlink
4. `sksync-lock.json` を生成
5. `sksync check` で差分検出

## 12. 未確定事項

- 各エージェントの正式な skill ディレクトリ仕様
- skill 形式が違う agent 向けに変換レイヤーを入れるか
- registry / package manager 的な install をどこまでやるか
- project scope と user scope の優先順位
- Windows での symlink 権限と junction 対応
- TUI を初期 MVP に含めるか、CLI MVP 後に追加するか
