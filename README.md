# sksync

`sksync` は、複数のコーディングエージェントでばらばらになりがちな Agent Skills の配置先を、1つの設定ファイルから同期するためのツール構想です。

## 目的

- エージェントごとに異なる skills ディレクトリへ、共通の skill 実体からシンボリックリンクを作成する
- `package-lock.json` のような lockfile を残し、別PC・別プロジェクトでも同じ skill セットを再現できるようにする
- ビルトインで以下のエージェントに対応する
  - claude-code
  - codex
  - gemini
  - opencode
  - pi

## 初期スコープ

まずは設計フェーズとして、以下を定義します。

- Rust 製 CLI / TUI アプリケーションとしての構成
- 設定ファイル形式
- lockfile 形式
- built-in agent mapping
- CLI コマンド案
- TUI 実行フロー
- symlink 同期の安全ルール

## 想定コマンド

```bash
sksync init
sksync install
sksync apply
sksync check
sksync list
sksync tui
```

詳細は以下を参照してください。

- [`docs/DESIGN.md`](docs/DESIGN.md) - 機能設計
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - アーキテクチャ設計原則
- [`docs/RUST_TUI_PLAN.md`](docs/RUST_TUI_PLAN.md) - Rust/TUI 実装計画
- [`docs/ROADMAP.md`](docs/ROADMAP.md) - 開発ロードマップ
