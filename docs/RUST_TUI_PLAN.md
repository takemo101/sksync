# Rust / Prompt TUI Implementation Plan

## 方針

`sksync tui` は SkillKit のような質問形式の prompt / wizard として実装する。
CLI と prompt TUI は入口だけを分け、同期計画・検査・適用のロジックは共有する。

## 推奨スタック

- CLI: `clap`
- Prompt TUI: `inquire` による Select / MultiSelect / Text / Confirm
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
8. prompt TUI shell
9. prompt TUI から add / remove / remove-agent / check / apply を呼ぶ

## Prompt TUI MVP

- 最初に intent を質問する
  - skill を追加する
  - skill を削除する
  - 特定 agent から skill を外す
  - 状態を確認する
  - apply する
- intent ごとに必要な値を順番に質問する
- 破壊的操作前に summary / dry-run を表示する
- 明示確認後に CLI と同じ application usecase を呼ぶ

## 注意点

- pane / keybinding 中心の常駐型 UI は実装しない
- TUI から直接ファイル操作を実装しない
- 永続状態は config / lockfile / local state のみに保存する
- 質問途中の入力値だけを TUI state として持つ
