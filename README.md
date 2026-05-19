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

## CLI MVP

現在の実装では、Rust 製 CLI として以下を試せます。

### Build

```bash
cargo build
cargo test
cargo run -- --help
```

ローカルでコマンドを実行する場合は、以下のように `cargo run --` 経由で起動できます。

```bash
cargo run -- init
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- plan --dry-run
cargo run -- update
cargo run -- apply
cargo run -- check
cargo run -- list
```

ビルド済みバイナリを使う場合は `cargo build` 後に `./target/debug/sksync ...` を実行してください。

### `sksync init`

新規プロジェクト用の雛形を作成します。

```bash
cargo run -- init
```

作成されるもの:

- `sksync.config.json`
- `skills/`

既に `sksync.config.json` が存在する場合は上書きせず失敗します。

### `sksync add`

SkillKit の `add` に近い操作です。source と複数 agent を指定すると dependency config に追記し、skill を取得して symlink まで作成します。

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- add github:owner/repo/path/to/skill#main --agent pi
cargo run -- add registry:skills.sh/owner/repo/skill#version --agent pi
cargo run -- add ./local-skill --agent pi --agent gemini
```

`--global` を付けると `~/.config/sksync/config.json` に追加し、グローバル設定として扱います。

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --global
```

### `sksync plan --dry-run`

`sksync.config.json` を読み込み、現在の target 状態を検査して、作成予定・同期済み・衝突・drift などを表示します。

```bash
cargo run -- plan --dry-run
cargo run -- plan --global
```

### `sksync update`

`dependencies` に書かれた SkillKit-style source から最新の skill を `skillDir` にダウンロード / コピーします。

```bash
cargo run -- update
cargo run -- update --global
```

対応する source 例:

```text
github:owner/repo/path/to/skill#main
owner/repo/path/to/skill#main
https://github.com/owner/repo/tree/main/path/to/skill
registry:skills.sh/owner/repo/skill#version
registry:example.com/owner/repo/skill#version
./local-skill
```

### `sksync apply`

planner の create symlink action だけを実行し、成功後に `sksync-lock.json` を書き出します。

```bash
cargo run -- apply
cargo run -- apply --global
```

### `sksync check`

`sksync-lock.json` と現在状態を比較し、source hash drift、target missing、broken symlink などを検出します。問題がある場合は非ゼロ終了します。

```bash
cargo run -- check
cargo run -- check --global
```

### `sksync list`

設定済み skill と agent ごとの target path / 状態を一覧表示します。`sksync-lock.json` がある場合は locked hash も表示します。

```bash
cargo run -- list
cargo run -- list --global
```

### Safety rules

- 既存の通常ファイルは上書きしません。
- `apply` は create symlink action のみ実行します。
- project config は project scope、`--global` config は user scope として target を解決します。
- conflict / drift / source missing がある場合、`apply` は失敗します。
- target path の親ディレクトリは必要に応じて作成します。
- テスト・実行例では一時ディレクトリを使うと安全です。

### Config / lockfile examples

- [`sksync.config.example.json`](sksync.config.example.json) - project/global install dependencies
- [`sksync.agents.example.json`](sksync.agents.example.json) - global-only agent target mapping (`~/.config/sksync/agents.json`)
- [`sksync-lock.example.json`](sksync-lock.example.json)

## 今後の予定

以下は設計済みですが、CLI MVP ではまだ未実装または placeholder です。

- `sksync install` / `sksync add`
- `sksync tui` の追加UX
- registry / GitLab / gist support

詳細は以下を参照してください。

- [`docs/DESIGN.md`](docs/DESIGN.md) - 機能設計
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - アーキテクチャ設計原則
- [`docs/RUST_TUI_PLAN.md`](docs/RUST_TUI_PLAN.md) - Rust/TUI 実装計画
- [`docs/ROADMAP.md`](docs/ROADMAP.md) - 開発ロードマップ
