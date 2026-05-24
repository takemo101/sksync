# sksync Architecture

このドキュメントは、`okite-ai/skills` のアーキテクチャ系スキルから、`sksync` に取り入れる設計原則を整理したものです。

対象にした主な考え方:

- Clean Architecture
- Package Design / Package Refactoring
- Domain Model First
- Parse, Don't Validate
- Domain Primitives / Always-Valid Domain Model
- Tell, Don't Ask
- Law of Demeter
- Intent-based Deduplication
- Error Handling / Error Classification
- Backward Compatibility Governance

## 1. アーキテクチャ方針

`sksync` は Rust 製の CLI/TUI アプリケーションだが、中心にあるのは「skill 同期のドメインロジック」である。
CLI/TUI/ファイルシステム操作は入口・出口として扱い、同期計画の判断は core 層に閉じ込める。

```text
┌─────────────────────────────────────────────┐
│ Interface Layer                              │
│  - CLI: clap                                 │
│  - TUI: prompt/wizard adapter                │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Application Layer                            │
│  - init                                      │
│  - plan / dry-run                            │
│  - apply                                     │
│  - check                                     │
│  - list                                      │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Domain Layer                                 │
│  - Skill                                     │
│  - Agent                                     │
│  - Scope                                     │
│  - TargetPath                                │
│  - LinkPlan                                  │
│  - Lockfile model                            │
│  - Drift / Conflict / BrokenLink             │
└───────────────────────┬─────────────────────┘
                        │
┌───────────────────────▼─────────────────────┐
│ Infrastructure Layer                         │
│  - filesystem                                │
│  - symlink                                   │
│  - hash                                      │
│  - config/lockfile JSON I/O                  │
└─────────────────────────────────────────────┘
```

## 2. 依存方向

Clean Architecture の依存ルールに従う。

- `domain` は他層に依存しない
- `application` は `domain` に依存する
- `cli` / `tui` は `application` を呼ぶだけにする
- `infrastructure` は `application` が定義した port / trait を実装する
- TUI から直接 symlink 作成・lockfile 書き込みをしない

依存方向:

```text
cli/tui ──▶ application ──▶ domain
              ▲
              │ trait
              │
infrastructure ┘
```

## 3. Rust モジュール構成案

```text
src/
  main.rs
  cli.rs
  tui/
    mod.rs
    app.rs
    ui.rs
    events.rs
  application/
    mod.rs
    init.rs
    add.rs
    install.rs
    update.rs
    remove.rs
    outdated.rs
    plan.rs
    apply.rs
    check.rs
    list.rs
    lockfile_build.rs
    ports.rs
  domain/
    mod.rs
    agent.rs
    skill.rs
    scope.rs
    target.rs
    link_plan.rs
    lockfile.rs
    problem.rs
  infrastructure/
    mod.rs
    fs.rs
    json/
      config.rs
      dependency_config.rs
      lockfile.rs
      agents.rs
    hash.rs
    builtin_agents.rs
```

## 4. Package Design 原則

### 技術名だけで分けない

`models`, `utils`, `helpers` のような技術・雑多な名前を避ける。
責務・変更理由でモジュールを分ける。

良い分割:

- `domain::skill`
- `domain::agent`
- `application::plan`
- `application::apply`
- `infrastructure::builtin_agents`

避ける分割:

- `utils`
- `types`
- `services`
- `managers`

### 公開インターフェースを小さくする

各 module は `mod.rs` で公開する型・関数を絞る。
内部実装は `pub(crate)` または private を優先する。

## 5. Domain Model First

実装は以下の順序で進める。

1. ドメイン型を定義する
2. ドメイン型の不変条件をテストする
3. dry-run の `LinkPlan` を作る
4. application usecase を作る
5. CLI/TUI を接続する
6. filesystem 実装を接続する

最初から TUI やファイル操作を中心にしない。
`sksync` の核は「何をどこへリンクすべきか」「現在状態との差分は何か」を安全に判断すること。

## 6. Parse, Don't Validate

外部入力は、読み込んだ後に何度も `if valid` で検査し続けない。
config / lockfile / CLI args は、境界で parse して妥当な domain 型へ変換する。

例:

- raw string の agent 名 → `AgentKind`
- raw string の scope → `Scope`
- raw path → `SourcePath` / `TargetPath`
- JSON config → `Config` → `ResolvedConfig`

```text
JSON / CLI args / filesystem
  ↓ parse
valid domain types
  ↓ use
planner / apply / check
```

## 7. Domain Primitives / Always-Valid Domain Model

Rust の newtype を使い、意味のある値をプリミティブ型のまま運ばない。

候補:

```rust
struct SkillName(String);
struct AgentName(String);
enum AgentKind { Pi, ClaudeCode, Codex, Gemini, OpenCode, Custom(String) }
enum Scope { User, Project }
struct SourcePath(PathBuf);
struct TargetPath(PathBuf);
struct Sha256Digest(String);
```

不変条件の例:

- `SkillName` は空文字を許可しない
- `SkillName` に path separator を許可しない
- `TargetPath` は agent mapping から解決済みである
- `ResolvedConfig` は存在しない agent 参照を含まない
- `LinkPlan` は source と target が同一 path になる操作を含まない

## 8. Tell, Don't Ask

状態を外に取り出して呼び出し側で判断しすぎない。
判断は対象オブジェクトの振る舞いとして持たせる。

避けたい形:

```rust
if entry.is_symlink && entry.target == expected {
    // ok
}
```

望ましい形:

```rust
match current_link.compare_with(expected_link) {
    LinkStatus::Synced => ...,
    LinkStatus::Drifted(problem) => ...,
}
```

## 9. Law of Demeter

深い構造を辿って判断しない。
特に TUI / CLI から domain の内部構造へ直接アクセスしない。

避けたい形:

```rust
app.config.skills[skill].agents[agent].target.path
```

望ましい形:

```rust
let rows = app.view_model().skill_rows();
```

TUI は表示用 ViewModel を受け取り、domain 内部を知りすぎない。

## 10. Intent-based Deduplication

字面が似ているだけで共通化しない。
意図が同じものだけ共通化する。

例:

- `SourcePath` と `TargetPath` はどちらも `PathBuf` だが意図が違うので別型にする
- `ConfigSkill` と `LockedSkill` は似ていても役割が違うので分ける
- CLI 表示用 row と TUI 表示用 row は同じに見えても、変更理由が違うなら別型にする

## 11. Error Handling / Error Classification

エラーは「回復可能か」「ユーザーが直せるか」「プログラム欠陥か」で分類する。

| 種類        | 例                                       | 対応                           |
| ----------- | ---------------------------------------- | ------------------------------ |
| UserFixable | config 不正、source 不存在、target 衝突  | 分かりやすいメッセージを出す   |
| Environment | symlink 権限なし、ホームディレクトリ不明 | OS 別の解決策を提示する        |
| Drift       | lockfile hash 差分、壊れた symlink       | `check` / TUI で修復候補を出す |
| Bug         | 到達不能状態、parse 済み型の不変条件違反 | internal error として扱う      |

Rust 実装では以下を使い分ける。

- library/domain error: `thiserror`
- CLI/TUI entrypoint: `anyhow`
- ユーザー表示: context 付きの短い説明

## 12. Backward Compatibility Governance

`sksync.config.json` と `sksync-lock.json` は公開 API とみなす。
lockfile v4 は package-lock 的に portable な source / hash / resolved install source だけを保存し、machine-local な target path は runtime に config から再計算する。v2 / v3 は読み込み互換として維持し、新規書き込みは v4 に統一する。
破壊的変更を避けるため、以下を守る。

- `lockfileVersion` を必ず持つ
- config schema の version 導入を検討する
- 古い lockfile は migrate できるようにする
- v2 lockfile の `targets` は読み込み互換のみ維持し、新規書き込みでは出力しない
- 互換層は `infrastructure::json` または専用 migration module に閉じ込める
- domain model 内に過去形式の都合を持ち込まない

## 13. Repository / Port Placement

永続化・ファイルシステム操作の trait は application 層に置く。
domain 層は永続化を知らない。

例:

```rust
trait ConfigStore {
    fn load_config(&self) -> Result<RawConfig>;
    fn save_config(&self, config: &RawConfig) -> Result<()>;
}

trait LinkStore {
    fn inspect_target(&self, target: &TargetPath) -> Result<TargetState>;
    fn create_symlink(&self, source: &SourcePath, target: &TargetPath) -> Result<()>;
}
```

## 14. TUI 設計原則

TUI は application の thin adapter とする。ここでの TUI は質問形式の wizard / prompt UI とする。

- TUI は `AddUseCase`, `RemoveUseCase`, `PlanUseCase`, `ApplyUseCase`, `CheckUseCase` を呼ぶ
- TUI は filesystem を直接触らない
- TUI state は質問途中の回答、選択中 option、確認待ちだけにする
- 追加・削除・agent 変更は質問フローで必要情報を集める
- 破壊的操作は dry-run summary を表示し、明示確認後に実行する
- 常駐型の一覧画面は持たず、必要な状態確認は `list` / `check` の summary として表示する

## 15. 設計チェックリスト

実装レビュー時は以下を確認する。

- [ ] domain が CLI/TUI/filesystem crate に依存していない
- [ ] config は境界で parse され、core では valid type を使っている
- [ ] `String` / `PathBuf` の裸利用が domain の重要概念に残っていない
- [ ] `utils` 的な雑多 module が増えていない
- [ ] TUI から直接 symlink / lockfile 操作をしていない
- [ ] lockfile/config の互換性方針を破っていない
- [ ] 既存ファイルを `--force` なしで上書きしない
- [ ] エラーがユーザー修正可能か internal bug か分類されている
