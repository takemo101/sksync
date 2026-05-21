# Rust / Prompt Wizard Implementation Plan

## 方針

`sksync wizard` は SkillKit のような質問形式の prompt / wizard として実装する。
`ask` / `tui` は互換 alias として残す。CLI と prompt wizard は入口だけを分け、同期計画・検査・適用のロジックは共有する。

## 推奨スタック

- CLI: `clap`
- Prompt wizard: `inquire` による Select / MultiSelect / Text / Confirm
- config: `serde` + `serde_json`
- error: `anyhow` + `thiserror`
- hashing: `sha2`
- walking: `walkdir`
- tests: `tempfile` + `insta`

## 実装順

1. Cargo プロジェクト作成
2. config / lockfile の Rust 型定義
3. built-in agent mapping
4. skill discovery / hash 計算
5. dry-run planner
6. symlink apply
7. check / list
8. prompt wizard shell
9. prompt wizard から add / remove / remove-agent / check / apply を呼ぶ

## Prompt Wizard MVP

- Runtime prompt labels / help / confirmations are shown in English for international users.
- 最初に intent を質問する
  - Add skill
  - Remove skill
  - Detach skill from agent
  - Show status
  - Apply links
- intent ごとに必要な値を順番に質問する
- remove / remove-agent は project/global scope を先に選び、config から読み込んだ skill list から対象を選択する
- remove は `Normal removal (no option)` / `--keep-files` / `--config-only` の削除モードを単一選択する
- remove-agent は選択した skill に設定済みの agent list から外す agent を選択する
- 破壊的操作前に summary / dry-run を表示する
- 明示確認後に CLI と同じ application usecase を呼ぶ

## 注意点

- pane / keybinding 中心の常駐型 UI は実装しない
- TUI から直接ファイル操作を実装しない
- 永続状態は config / lockfile / local state のみに保存する
- 質問途中の入力値だけを TUI state として持つ
