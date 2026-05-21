# sksync Implementation Issues

このファイルは、AIコーディングエージェントが1 issueずつ実装しやすい粒度に分割した実装タスク一覧です。
各issueは原則として「1〜2時間程度で完了」「明確な受け入れ条件」「触るファイル範囲が限定的」になるようにしています。

## Milestone 1: Rust CLI MVP foundation

### Issue 1: Cargoプロジェクトと基本crate構成を作成する

**目的**  
Rust CLIとしてビルド・テストできる最小プロジェクトを作る。

**作業内容**

- `Cargo.toml` を作成する
- `src/main.rs` を作成する
- 初期module treeを作成する
  - `src/cli.rs`
  - `src/application/mod.rs`
  - `src/domain/mod.rs`
  - `src/infrastructure/mod.rs`
- dependenciesを追加する
  - runtime: `clap`, `serde`, `serde_json`, `anyhow`, `thiserror`, `dirs`, `shellexpand`, `sha2`, `hex`, `walkdir`
  - dev: `tempfile`
- `cargo test` が通る状態にする

**受け入れ条件**

- `cargo build` が成功する
- `cargo test` が成功する
- `sksync --help` が表示できる

**依存**: なし

---

### Issue 2: CLI command定義を実装する

**目的**  
想定コマンドを `clap` で定義し、未実装でも入口を確保する。

**作業内容**

- `src/cli.rs` に command enumを定義する
- 以下のsubcommandを追加する
  - `init`
  - `plan` / `--dry-run`
  - `apply` / `--force`
  - `check`
  - `list`
  - `tui` は placeholderでよい
- `main.rs` からcommand dispatchする
- 未実装コマンドは明確なエラーメッセージを返す

**受け入れ条件**

- `sksync init --help` が表示できる
- `sksync plan --help` が表示できる
- 未実装placeholderがpanicしない

**依存**: Issue 1

---

## Milestone 2: Domain model and config parsing

### Issue 3: domain primitiveを定義する

**目的**  
重要概念を裸の `String` / `PathBuf` で運ばないための型を作る。

**作業内容**

- `src/domain/skill.rs`
  - `SkillName`
  - `SourcePath`
- `src/domain/agent.rs`
  - `AgentKind`
  - `AgentName` or custom agent representation
- `src/domain/scope.rs`
  - `Scope::{User, Project}`
- `src/domain/target.rs`
  - `TargetPath`
- constructorで不変条件を検査する
  - skill名は空不可
  - skill名にpath separator不可
  - scopeは `user` / `project` のみ

**受け入れ条件**

- 正常系/異常系unit testがある
- 不正なskill名がdomain type化できない
- domain moduleがCLI/TUI/filesystemに依存しない

**依存**: Issue 1

---

### Issue 4: config JSON modelとloaderを実装する

**目的**  
`sksync.config.json` を読み込み、domainで使える設定へ変換する。

**作業内容**

- `src/infrastructure/json.rs` に raw config modelを定義する
- `src/application/ports.rs` に `ConfigStore` traitを定義する
- file-based `ConfigStore` 実装を作る
- `RawConfig` → `ResolvedConfig` 変換を実装する
- `skills.*.source` 省略時に `skillDir/<skillName>` を使う
- example configをfixtureとしてtestする

**受け入れ条件**

- `sksync.config.example.json` をparseできる
- 存在しないagent参照はエラーになる
- source省略時の補完testがある

**依存**: Issue 3

---

### Issue 5: lockfile modelとwriter/readerを実装する

**目的**  
同期結果を再現可能にするためのlockfile形式を実装する。

**作業内容**

- `src/domain/lockfile.rs` にdomain寄りのlockfile型を定義する
- `src/infrastructure/json.rs` にJSON reader/writerを追加する
- `lockfileVersion: 1` を扱う
- `generatedBy`, `generatedAt`, `root`, `skills` を出力する
- unknown versionは分かりやすくエラーにする

**受け入れ条件**

- `sksync-lock.example.json` をparseできる
- lockfileを書いて再読込できるroundtrip testがある
- unsupported version testがある

**依存**: Issue 3

---

## Milestone 3: Agent mapping and skill discovery

### Issue 6: built-in agent mappingを実装する

**目的**  
agent + scopeからtarget directoryを解決できるようにする。

**作業内容**

- `src/infrastructure/builtin_agents.rs` を実装する
- default mappingを定義する
  - `pi`: user `~/.pi/agent/skills`, project `.pi/agent/skills`
  - `claude-code`: user `~/.claude/skills`, project `.claude/skills`
  - `codex`: user `~/.codex/skills`, project `.codex/skills`
  - `gemini`: user `~/.gemini/skills`, project `.gemini/skills`
  - `opencode`: user `~/.config/opencode/skills`, project `.opencode/skills`
- `agents.*.targetDir` overrideに備えたAPIにする
- `~` expansionとproject root相対解決を行う

**受け入れ条件**

- 各agent/scopeのtarget path testがある
- user scopeの `~` が展開される
- project scopeがproject root相対になる

**依存**: Issue 4

---

### Issue 7: skill discoveryとSHA-256 hash計算を実装する

**目的**  
source directoryの内容をlockfileへ記録できるようにする。

**作業内容**

- `src/infrastructure/hash.rs` を実装する
- source directory配下のファイル一覧を安定順で収集する
- 各ファイルのSHA-256を計算する
- directory全体のhashを計算する
- `.git`, target artifactsなどを除外する方針を最小実装する

**受け入れ条件**

- 同じ内容なら同じhashになる
- ファイル内容変更でhashが変わる
- ファイル順に依存しないtestがある

**依存**: Issue 4

---

## Milestone 4: Planner and apply

### Issue 8: current target state inspectionを実装する

**目的**  
target pathの現在状態を安全に判定する。

**作業内容**

- `src/application/ports.rs` に `LinkStore` traitを定義する
- `TargetState` を定義する
  - missing
  - symlink to expected source
  - symlink to unexpected source
  - regular file/directory conflict
  - broken symlink
- `src/infrastructure/fs.rs` に実装する
- symlink metadataを正しく読む

**受け入れ条件**

- tempdirで各状態のtestがある
- 通常ファイルをsymlink扱いしない
- broken symlinkを検出できる

**依存**: Issue 6

---

### Issue 9: dry-run plannerを実装する

**目的**  
configとcurrent stateから、安全な操作計画を作る。

**作業内容**

- `src/domain/link_plan.rs` を定義する
- plan actionを定義する
  - create symlink
  - already synced
  - conflict
  - drifted symlink
  - source missing
- `src/application/plan.rs` を実装する
- plan resultをCLI表示できる形式へ変換する

**受け入れ条件**

- missing targetならcreate actionになる
- synced targetならno-opになる
- existing regular fileならconflictになる
- unexpected symlinkならdriftedになる
- `sksync plan --dry-run` が操作一覧を表示する

**依存**: Issue 8

---

### Issue 10: safe symlink applyを実装する

**目的**  
planner結果にもとづいて安全にsymlinkを作成する。

**作業内容**

- `src/application/apply.rs` を実装する
- create actionのみ実行する
- parent directoryを必要に応じて作る
- conflict/driftがある場合は失敗する
- `--force` なしで既存targetを上書きしない
- apply後にlockfileを書き出す

**受け入れ条件**

- missing targetにsymlinkが作られる
- regular file conflictでは失敗する
- unexpected symlinkは `--force` なしで失敗する
- apply後に `sksync-lock.json` が作られる

**依存**: Issue 5, Issue 9

---

## Milestone 5: Check/list and usability

### Issue 11: check commandを実装する

**目的**  
lockfileと現在状態の差分を検出する。

**作業内容**

- `src/application/check.rs` を実装する
- lockfileのsource hashと現在hashを比較する
- target symlinkの有無/リンク先を検査する
- broken symlinkを報告する
- exit codeを成功/問題ありで分ける

**受け入れ条件**

- synced状態で成功する
- source変更でdriftを検出する
- target削除でmissingを検出する
- broken symlinkを検出する

**依存**: Issue 7, Issue 10

---

### Issue 12: list commandを実装する

**目的**  
管理中skillとagentごとの状態を一覧表示する。

**作業内容**

- `src/application/list.rs` を実装する
- configベースのskill一覧を表示する
- agentごとのtarget pathと状態を表示する
- lockfileがある場合はlocked hashも表示する

**受け入れ条件**

- `sksync list` がskill名を表示する
- agentごとのtarget pathが表示される
- missing/synced/conflictなどの状態が分かる

**依存**: Issue 9, Issue 11

---

### Issue 13: init commandを実装する

**目的**  
新規プロジェクトで必要な雛形を生成できるようにする。

**作業内容**

- `src/application/init.rs` を実装する
- `sksync.config.json` を作る
- `skills/` directoryを作る
- 既存configがある場合は上書きしない
- `--force` は別issueに回してよい

**受け入れ条件**

- 空ディレクトリで `sksync init` が成功する
- configとskills directoryが作られる
- 既存configがある場合は失敗する

**依存**: Issue 4

---

### Issue 14: READMEにCLI MVPの使用方法を追記する

**目的**  
実装済み機能をユーザーが試せるようにする。

**作業内容**

- build手順を追記する
- `init`, `plan`, `apply`, `check`, `list` の使い方を追記する
- 安全ルールを短く説明する
- example config / lockfileへのリンクを維持する

**受け入れ条件**

- READMEだけでローカル実行手順が分かる
- まだ未実装の `install` / `tui` は将来予定として明記する

**依存**: Issue 10, Issue 11, Issue 12, Issue 13

---

## Milestone 6: Prompt Wizard MVP

### Issue 15: Prompt wizard shellを追加する

**目的**  
SkillKit のような質問形式で操作できる `sksync wizard` を起動できるようにする。`ask` / `tui` は互換 alias として残す。

**作業内容**

- `src/tui/mod.rs` に prompt / wizard flow を作る
- `sksync wizard` から起動する
- 最初に add / remove / remove-agent / check / apply / quit の intent を選ばせる
- pane / keybinding 中心の常駐型 UI は作らない

**受け入れ条件**

- `sksync wizard` が起動する
- intent を選んで終了できる
- filesystem操作はapplication経由のみ

**依存**: Issue 12

---

### Issue 16: Prompt wizardにadd/remove/check/apply操作を接続する

**目的**  
CLI MVPで作ったcore logicを質問形式TUIから実行できるようにする。

**作業内容**

- Runtime prompt labels / help / confirmations are English.
- add: source / name override / agent / global scope を質問する
- remove: project/global scope を先に選び、config 由来の skill list から skill を選ばせ、Normal removal / keep-files / config-only の削除モードを選ばせる
- remove-agent: project/global scope を先に選び、config 由来の skill list と選択 skill の agent list から対象を選ばせる
- check/list: scope と表示詳細を質問する
- apply: plan summary を表示し、確認後に apply する

**受け入れ条件**

- wizardから安全に add/remove/remove-agent を実行できる
- 破壊的操作前に確認が必要
- wizardが直接symlinkやlockfileを書かない

**依存**: Issue 15

---

## Suggested execution order

1. Issue 1
2. Issue 2
3. Issue 3
4. Issue 4
5. Issue 5
6. Issue 6
7. Issue 7
8. Issue 8
9. Issue 9
10. Issue 10
11. Issue 11
12. Issue 12
13. Issue 13
14. Issue 14
15. Issue 15
16. Issue 16

## Notes for AI implementers

- 1 issueにつき、基本的に関連ファイルだけ触ること
- `cargo test` を毎issueの完了条件に含めること
- 実ユーザーのhome directoryをtestで触らないこと
- symlink関連testは必ず `tempfile` 配下で行うこと
- 不明点がある場合は、既存docsの安全ルールを優先すること
