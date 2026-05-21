# sksync Implementation Plan

## 現状分析

このリポジトリは現在、実装コードをまだ持たない設計ベースライン段階です。
既存ファイルは以下です。

- `README.md`: 目的・初期スコープ・想定コマンド
- `docs/DESIGN.md`: 機能設計、設定/lockfile形式、CLI/TUI案、安全ルール
- `docs/ARCHITECTURE.md`: Clean Architecture、Domain Model First、エラー分類、TUI原則
- `docs/ROADMAP.md`: Phase 0〜4 のロードマップ
- `docs/RUST_TUI_PLAN.md`: Rust/TUI 実装順と推奨crate
- `sksync.config.example.json`: config例
- `sksync-lock.example.json`: lockfile例

したがって、最初の実装対象は「Rust CLI MVP」です。TUIやinstall/registryは、coreが安定した後に分離して進めます。

## 実装方針

### 1. CLI MVPを先に作る

TUIは魅力的ですが、`sksync` の核は「設定から期待リンク状態を作り、現在状態との差分を安全に判定・適用すること」です。
そのため、最初は以下のCLIだけを動かします。

```bash
sksync init
sksync plan --dry-run
sksync apply
sksync check
sksync list
```

`install` と `tui` は後続フェーズに回します。

### 2. core logicをUIから分離する

`cli` は application layer を呼ぶだけにします。
filesystem / symlink / JSON I/O は infrastructure layer に閉じ込めます。

推奨初期構成:

```text
src/
  main.rs
  cli.rs
  application/
    mod.rs
    init.rs
    plan.rs
    apply.rs
    check.rs
    list.rs
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
    builtin_agents.rs
    fs.rs
    hash.rs
    json.rs
```

### 3. Parse, Don't Validate を守る

JSONをそのままcoreで扱わず、境界で以下へ変換します。

- `RawConfig` → `ResolvedConfig`
- raw agent name → `AgentKind`
- raw scope → `Scope`
- raw path → `SourcePath` / `TargetPath`
- raw lockfile → version-aware `Lockfile`

### 4. 安全性をMVP要件に含める

`apply` は便利機能ではなく危険操作です。最初から以下を必須にします。

- 通常ファイルを上書きしない
- 予期しない既存symlinkを勝手に差し替えない
- `--force` なしで破壊的変更をしない
- `plan` で事前に操作一覧を見られる
- lockfileを生成/更新する

## フェーズ計画

## Phase 1: Rust CLI MVP

目的: 設計済みの基本同期フローを、テスト付きで動作させる。

1. Cargo project / CI / lint / format の土台を作る
2. config / lockfile / domain primitive を定義する
3. built-in agent mapping を実装する
4. skill discovery とhash計算を実装する
5. dry-run plannerを実装する
6. symlink applyとlockfile生成を実装する
7. check/list CLIを実装する
8. READMEに実行手順を追加する

完了条件:

- `cargo test` が通る
- example configを使って `plan/check/list` が動く
- temp directory上で `apply` の安全ルールがテストされている

## Phase 2: Prompt Wizard MVP

目的: CLIと同じcore logicを利用して、質問形式で skill の追加・削除・確認を実行できるようにする。

1. prompt / wizard の intent 選択を実装
2. add / remove / remove-agent / check / apply に必要な値を順番に質問する
3. remove / remove-agent は scope 選択後に config 由来の skill list / agent list から選ばせる
4. 破壊的操作前に summary / dry-run を表示する
5. 明示確認後に CLI と同じ application usecase を呼ぶ

完了条件:

- wizardがfilesystemを直接触らない
- pane / keybinding 中心の常駐型 UI を持たない
- apply / remove 前に確認が必要
- CLIとwizardでplan結果が一致する

## Phase 3: Portability / install workflow

目的: 実利用の幅を広げる。

1. custom agent mapping
2. config/lockfile migration
3. Windows symlink/junction strategy
4. `install/update/remove` workflow

## 優先順位

1. **最優先**: config parse、domain model、planner、safe apply
2. **次点**: check/list、lockfile drift検出
3. **後続**: TUI、install、registry、Windows特殊対応

## 実装上の注意

- `domain` から `clap`, `serde_json`, `std::fs`, prompt UI crate に依存しない
- `SourcePath` と `TargetPath` を同じ `PathBuf` として扱わない
- `ConfigSkill` と `LockedSkill` を安易に共通化しない
- testでは `tempfile` を使い、実ユーザーディレクトリを触らない
- snapshot testは plan/check の表示安定後に `insta` で導入する
