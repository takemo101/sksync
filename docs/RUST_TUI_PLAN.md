# Rust / TUI Implementation Plan

## 方針

`sksync` は Rust 製の単一バイナリとして実装する。
CLI と TUI は入口だけを分け、同期計画・検査・適用のロジックは共有する。

## 推奨スタック

- CLI: `clap`
- TUI: `ratatui` + `crossterm`
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
7. check
8. TUI shell
9. TUI から dry-run / apply / check を呼ぶ

## 最初の TUI MVP

- agent 一覧
- skill 一覧
- dry-run result 表示
- `a` で apply
- `c` で check
- `q` で quit

## 注意点

TUI から直接ファイル操作を実装しない。
必ず CLI と同じ core API を呼ぶ。
