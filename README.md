# sksync

`sksync` は、複数のコーディングエージェントでばらばらになりがちな Agent Skills の配置先を、1つの設定ファイルから同期するためのツール構想です。

## 目的

- エージェントごとに異なる skills ディレクトリへ、共通の skill 実体からシンボリックリンクを作成する
- skill の元データを `.sksync/skills/` に集約し、agent 側へ安全に symlink する
- ビルトインで以下のエージェントに対応する
  - claude-code
  - codex
  - gemini
  - opencode
  - pi

## CLI MVP

現在の実装では、Rust 製 CLI として以下を試せます。

### Install on macOS

GitHub Releases の macOS binary を `~/.local/bin/sksync` にインストールできます。

```bash
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | sh
```

インストール先を変える場合:

```bash
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | INSTALL_DIR=/usr/local/bin sh
```

現在の installer は macOS のみ対応です。`aarch64-apple-darwin` / `x86_64-apple-darwin` の release asset を取得します。

### Uninstall

`install.sh` で入れた場合は、インストールした binary を削除します。

```bash
rm -f ~/.local/bin/sksync
```

`INSTALL_DIR` を変えてインストールした場合は、その場所の binary を削除してください。

```bash
rm -f /usr/local/bin/sksync
```

clone した repository から `just install` で入れた場合は、同じ `INSTALL_DIR` を指定して uninstall できます。

```bash
just uninstall
# or
INSTALL_DIR=/usr/local/bin just uninstall
```

global config / agent mapping / installed global skills も削除して完全に初期化する場合は、binary に加えて `~/.sksync` を削除します。

```bash
rm -f ~/.local/bin/sksync
rm -rf ~/.sksync
```

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
cargo run -- remove skill-name
cargo run -- outdated
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
# or initialize global config
cargo run -- init --global
```

project mode で作成されるもの:

- `sksync.config.json`
- `.sksync/skills/`

global mode (`--global`) で作成されるもの:

- `~/.sksync/config.json`
- `~/.sksync/agents.json`
- `~/.sksync/skills/`

既に対象 config が存在する場合は上書きせず失敗します。global mode で `agents.json` が既にある場合は上書きしません。

### `sksync add`

SkillKit の `add` に近い操作です。source と複数 agent を指定すると dependency config に追記し、skill を取得して symlink まで作成します。source が repo root や親ディレクトリを指していて直接 `SKILL.md` を持たない場合は、配下の `SKILL.md` を探索し、1件なら自動選択、複数なら複数選択プロンプトを表示します。`--name` を指定すると一致する discovered skill を自動選択します。`skills.sh` の direct URL が実際の repo path と一致しない場合も repo root discovery で slug に一致する skill を探します。プロンプトの候補では skill 名を太字・シアンで表示します。取得した skill は `SKILL.md` と YAML frontmatter の `name` / `description` を検証します。

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- add github:owner/repo/path/to/skill#main --agent pi
cargo run -- add owner/repo --name skill-name --agent pi
cargo run -- add skills.sh/owner/repo --agent pi
cargo run -- add https://www.skills.sh/owner/repo --agent pi
cargo run -- add ./local-skill --agent pi --agent gemini
```

`--global` を付けると `~/.sksync/config.json` に追加し、グローバル設定として扱います。

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --global
```

### `sksync remove`

指定した skill を dependency config / installed skill directory / managed symlink / lockfile から削除します。

```bash
cargo run -- remove cuekit-dogfood
cargo run -- remove cuekit-dogfood --global
cargo run -- remove cuekit-dogfood --keep-files
cargo run -- remove cuekit-dogfood --config-only
```

`--agent` を指定すると対象 agent の link だけを外します。

```bash
cargo run -- remove cuekit-dogfood --agent pi
cargo run -- remove cuekit-dogfood --agent pi --agent claude-code
```

この場合は `dependencies.<skill>.agents` と lockfile targets から指定 agent だけを外し、他 agent の link と `.sksync/skills/<skill>` 本体は残します。最後の agent を外した場合は skill 全体の削除と同じ扱いにします。

### `sksync outdated`

lockfile と upstream を比較して、更新可能な skill を表示します。Git source は remote ref の HEAD と lockfile の resolved commit を比較します。

```bash
cargo run -- outdated
cargo run -- outdated --global
cargo run -- outdated --json
```

### `sksync plan --dry-run`

`sksync.config.json` を読み込み、現在の target 状態を検査して、作成予定・同期済み・衝突・drift などを表示します。

```bash
cargo run -- plan --dry-run
cargo run -- plan --global
```

### `sksync install`

`sksync-lock.json` があれば lockfile に記録された source を優先して skill を再構成し、symlink まで作成します。lockfile がなければ config から取得して lockfile を作成します。

```bash
cargo run -- install
cargo run -- install --global
```

### `sksync update`

`dependencies` に書かれた SkillKit-style source から最新または指定versionの skill を `skillDir` にダウンロード / コピーし、`sksync-lock.json` を更新します。取得した skill は `SKILL.md` と YAML frontmatter の `name` / `description` を検証します。

```bash
cargo run -- update
cargo run -- update --global
```

対応する source 例:

```text
github:owner/repo/path/to/skill#main
owner/repo/path/to/skill#main
https://github.com/owner/repo/tree/main/path/to/skill
skills.sh/owner/repo/skill-name#version
https://www.skills.sh/owner/repo/skill-name#version
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
- `remove` は sksync が管理している symlink だけを削除します。
- `outdated` は Git source の remote ref と lockfile commit を比較します。
- `install` は lockfile があれば lockfile の resolved source を優先します。
- `update` は dependencies から最新を取得して lockfile を更新します。
- `apply` は create symlink action のみ実行します。
- project config は project scope、`--global` config は user scope として target を解決します。
- conflict / drift / source missing がある場合、`apply` は失敗します。
- target path の親ディレクトリは必要に応じて作成します。
- テスト・実行例では一時ディレクトリを使うと安全です。

### Generated files and gitignore

project-local の生成物は `.gitignore` します。

- `.sksync/` - downloaded/copied skill bodies (`.sksync/skills/<skill>`)
- `skills/` - legacy generated skill store from older defaults
- `sksync-lock.json` - lockfile v3 は portable ですが、チーム共有方針が固まるまでは project-local state として扱います

共有する基本ファイルは `sksync.config.json` です。

### Config / lockfile examples

- [`sksync.config.example.json`](sksync.config.example.json) - project/global install dependencies
- [`sksync.agents.example.json`](sksync.agents.example.json) - global and project agent target mappings (`~/.sksync/agents.json`) with SkillKit-compatible agent entries
- [`sksync-lock.example.json`](sksync-lock.example.json)
- [`schemas/sksync.schema.json`](schemas/sksync.schema.json) - JSON Schema for `config.json` / `sksync.config.json`
- [`schemas/sksync.agents.schema.json`](schemas/sksync.agents.schema.json) - JSON Schema for `agents.json`

## 今後の予定

npm-like な依存管理コマンド体系に寄せていきます。

- `sksync remove <skill> --agent <agent>` - 指定 agent の managed symlink だけを外す
- `sksync outdated` - lockfile と upstream/latest を比較して更新可能な skill を表示
- `sksync wizard` の追加UX（`ask` / `tui` aliases）
- Additional source URL transformers beyond `skills.sh`
- GitLab / gist support

`ci` 相当の専用コマンドは現時点では追加しません。lockfile 再現は `sksync install` に集約します。

詳細は以下を参照してください。

- [`docs/DESIGN.md`](docs/DESIGN.md) - 機能設計
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - アーキテクチャ設計原則
- [`docs/RUST_TUI_PLAN.md`](docs/RUST_TUI_PLAN.md) - Rust/TUI 実装計画
- [`docs/ROADMAP.md`](docs/ROADMAP.md) - 開発ロードマップ
