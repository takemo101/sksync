# sksync

`sksync` は、複数のコーディングエージェントでばらばらになりがちな Agent Skills の配置先を、1つの設定ファイルから同期する CLI ツールです。

## 目的

- エージェントごとに異なる skills ディレクトリへ、共通の skill 実体からシンボリックリンクを作成する
- skill の元データを `.sksync/skills/` に集約し、agent 側へ安全に symlink する
- bundled agent mapping で Claude Code / Codex / Gemini / jcode / OpenCode / Pi / Antigravity など主要エージェントの配置先に対応する
- GitHub / local directory / skills.sh URL から skill を追加し、lockfile で再現可能にする

## CLI

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
cargo run -- init --global
cargo run -- init --agents
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- attach skill-name --agent gemini
cargo run -- agents list
cargo run -- agents doctor
cargo run -- agents refresh
cargo run -- doctor
cargo run -- import ~/.claude/skills --agent claude-code --dry-run
cargo run -- remove skill-name
cargo run -- outdated
cargo run -- install
cargo run -- update
cargo run -- plan --dry-run
cargo run -- apply
cargo run -- check
cargo run -- list
cargo run -- wizard
```

ビルド済みバイナリを使う場合は `cargo build` 後に `./target/debug/sksync ...` を実行してください。

### `sksync init`

新規プロジェクト用の雛形を作成します。

```bash
cargo run -- init
# or initialize global config
cargo run -- init --global
# or force-refresh only ~/.sksync/agents.json
cargo run -- init --agents
```

project mode で作成されるもの:

- `sksync.config.json`
- `.sksync/skills/`

global mode (`--global`) で作成されるもの:

- `~/.sksync/config.json`
- `~/.sksync/agents.json`
- `~/.sksync/skills/`

既に対象 config が存在する場合は上書きせず失敗します。global mode で `agents.json` が既にある場合は上書きしません。

`init --agents` は config / skills directory には触らず、bundled default mapping で `~/.sksync/agents.json` だけを強制的に上書きします。新しい agent mapping を取り込む場合に使います。通常は同じ目的で `sksync agents refresh` も使えます。

#### Agent target mappings

`~/.sksync/agents.json` は global / project 両方の agent target directory mapping を持ちます。project config では `project` mapping、global config (`--global`) では `global` mapping を使います。inline config の `agents` override がある場合は、それが最優先です。

bundled mapping には SkillKit-compatible な agent entries を含めています。例:

| Agent                   | Global targetDir               | Project targetDir  |
| ----------------------- | ------------------------------ | ------------------ |
| `pi`                    | `~/.pi/agent/skills`           | `.pi/agent/skills` |
| `claude-code`           | `~/.claude/skills`             | `.claude/skills`   |
| `codex`                 | `~/.codex/skills`              | `.codex/skills`    |
| `jcode`                 | `~/.jcode/skills`              | `.jcode/skills`    |
| `gemini` / `gemini-cli` | `~/.gemini/skills`             | `.gemini/skills`   |
| `opencode`              | `~/.config/opencode/skills`    | `.opencode/skills` |
| `antigravity`           | `~/.gemini/antigravity/skills` | `.agents/skills`   |
| `universal`             | `~/.agents/skills`             | `.agents/skills`   |

Antigravity は公式仕様に合わせ、workspace default の `.agents/skills` を使います。`.agent/skills` は Antigravity 側では後方互換として扱われますが、sksync の bundled default は `.agents/skills` です。

`universal` は Agent Skills ecosystem の canonical directory です。global は `~/.agents/skills`、project は `.agents/skills` に配置します。

### `sksync add`

SkillKit の `add` に近い操作です。source と複数 agent を指定すると dependency config に追記し、skill を取得して symlink まで作成します。途中で install / plan / apply に失敗した場合は、変更前の config に rollback します。

```bash
cargo run -- add <source> --agent pi [--agent claude-code]
```

よく使う例:

```bash
# GitHub shorthand / prefix / tree URL
cargo run -- add owner/repo/path/to/skill --agent pi --agent claude-code
cargo run -- add github:owner/repo/path/to/skill#main --agent pi
cargo run -- add https://github.com/owner/repo/tree/main/path/to/skill --agent pi

# repo root から SKILL.md を discovery して選択
cargo run -- add owner/repo --agent pi
cargo run -- add owner/repo --name skill-name --agent pi

# skills.sh URL / shorthand
cargo run -- add skills.sh/owner/repo --agent pi
cargo run -- add https://www.skills.sh/owner/repo --agent pi
cargo run -- add https://www.skills.sh/owner/repo/skill-name --agent pi

# local directory
cargo run -- add ./local-skill --agent pi --agent gemini
```

#### Source formats

| Format                                                 | Meaning                                                                                         |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| `owner/repo/path/to/skill#ref`                         | GitHub shorthand. `owner/repo` を clone し、`path/to/skill` を skill directory として使います。 |
| `github:owner/repo/path/to/skill#ref`                  | GitHub shorthand を明示します。                                                                 |
| `https://github.com/owner/repo/tree/ref/path/to/skill` | GitHub tree URL。`ref` と path をそのまま使います。                                             |
| `owner/repo#ref`                                       | repo root / 親ディレクトリとして扱い、配下の `SKILL.md` を discovery します。                   |
| `skills.sh/owner/repo[/skill-or-path]#ref`             | skills.sh source。内部的には GitHub repo に変換します。                                         |
| `https://www.skills.sh/owner/repo[/skill-or-path]#ref` | skills.sh URL。direct URL の推測 path が外れた場合も repo root discovery で探します。           |
| `./local-skill`, `../skills/foo`, `/abs/path`          | local directory。相対 path は config file のある directory から解決します。                     |

`registry:<host>/<package>` と `--provider` はサポートしていません。source URL transformer は source 文字列から自動判定します。

#### Private repositories

private Git repository は sksync 独自の token 管理ではなく、ローカルの `git` 認証設定に委譲します。つまり、その環境で `git clone <repo>` が通る source であれば sksync でも利用できます。

- GitHub shorthand (`owner/repo/path#ref`) は `https://github.com/owner/repo.git` に変換します。private repo の場合は Git credential helper / GitHub CLI / PAT などで HTTPS 認証済みにしてください。
- SSH URL を使いたい場合は structured source を使います。

```json
{
  "dependencies": {
    "my-skill": {
      "source": {
        "provider": "git",
        "url": "git@github.com:org/private-skills.git",
        "path": "skills/my-skill",
        "ref": "main"
      },
      "agents": ["pi"]
    }
  }
}
```

sksync は `--token` / `--github-token` のような認証オプションを持たず、credentials を config に保存しません。認証エラーは underlying `git` command のエラーとして表示されます。`skills.sh` URL は基本的に public source 前提です。

#### Discovery behavior

source が直接 `SKILL.md` を持たない repo root / 親ディレクトリを指す場合、配下の `SKILL.md` を最大 depth 5 で探索します。

- 1件だけ見つかった場合: 自動選択します。
- 複数見つかった場合: 対話環境では複数選択プロンプトを表示します。
- 非対話環境で複数見つかった場合: エラーにして `--name <skill>` またはより具体的な source を案内します。
- `--name` 指定時: frontmatter `name` または directory name に一致する discovered skill を1件だけ自動選択します。
- `.git` / `node_modules` / `.sksync` は探索対象から除外します。

複数選択プロンプトでは skill 名を太字・シアンで表示します。

#### skills.sh mapping

`skills.sh` は registry としてではなく、GitHub source への URL transformer として扱います。入力には `skills.sh` URL / shorthand を使えますが、config には選択後の実 GitHub path を `https://github.com/<owner>/<repo>/tree/<ref>/<path>` として保存します。

```text
https://www.skills.sh/vercel-labs/skills/find-skills
→ https://github.com/vercel-labs/skills.git
→ skills/find-skills
→ source saved as https://github.com/vercel-labs/skills/tree/HEAD/skills/find-skills
```

`skills.sh` の URL slug と GitHub repo 内 path が一致しない場合も、repo root discovery で実際の path を探し、その exact GitHub tree URL を config に保存します。

```text
https://www.skills.sh/gitbutlerapp/gitbutler/but
→ discovers crates/but/skill
→ source saved as https://github.com/gitbutlerapp/gitbutler/tree/HEAD/crates/but/skill
```

#### Skill validation

取得した skill は install 前に検証します。

- `SKILL.md` が存在する
- `SKILL.md` がファイルである
- YAML frontmatter が存在する
- frontmatter に non-empty string の `name` / `description` がある

検証に失敗した場合は destination を置き換えず、staging directory を削除してエラーにします。

`--global` を付けると `~/.sksync/config.json` に追加し、グローバル設定として扱います。

```bash
cargo run -- add owner/repo/path/to/skill --agent pi --global
```

### `sksync attach`

既存の dependency-managed skill を追加 agent に紐づけ、既存 source 表現を保ったまま skill 取得と symlink 作成まで実行します。

```bash
cargo run -- attach cuekit-dogfood --agent claude-code
cargo run -- attach cuekit-dogfood --agent pi --agent gemini --global
```

### `sksync agents`

agent target mapping を確認・更新します。`doctor` は read-only で targetDir の存在や書き込み可否を診断します。

```bash
cargo run -- agents list
cargo run -- agents doctor
cargo run -- agents refresh
```

### `sksync doctor`

config / lockfile / source / target / agent mapping を read-only で総合診断し、問題がある場合は次に試すコマンドを表示して非ゼロ終了します。自動修復や directory 作成は行いません。

```bash
cargo run -- doctor
cargo run -- doctor --global
```

### `sksync import`

既存 agent skill directory から skill を copy-only で `.sksync/skills` または `~/.sksync/skills` に取り込み、指定 agent の dependency として config に登録します。元の directory は変更・削除・symlink 置換しません。target への symlink 反映は別途 `plan` / `apply` で確認します。

```bash
cargo run -- import ~/.claude/skills --agent claude-code --dry-run
cargo run -- import ~/.claude/skills --agent claude-code
cargo run -- import ~/.jcode/skills --agent jcode --global
```

### `sksync remove`

指定した skill を dependency config / installed skill directory / managed symlink / lockfile から削除します。installed skill directory は configured `skillDir` 配下の sksync-managed directory の場合だけ削除し、local / legacy の unmanaged source directory は削除しません。

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

対応する source は `sksync add` と同じです。repo root / 親ディレクトリの discovery は `add` 時に選択済み path として config に保存されるため、`update` / `install` は保存済み source を再取得します。

### `sksync apply`

planner の create symlink action だけを実行し、成功後に `sksync-lock.json` を書き出します。source missing / conflict / drift がある場合は失敗します。

```bash
cargo run -- apply
cargo run -- apply --global
```

### `sksync check`

`sksync-lock.json` と現在状態を比較し、source hash drift、target missing、broken symlink などを検出します。source hash は lockfile から、target health は現在の config / agent mapping から再計算した target path から確認します。問題がある場合は非ゼロ終了します。

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

### `sksync wizard`

質問形式の prompt wizard で、add / attach / detach / remove / list+check / plan+apply を対話的に実行できます。`ask` と `tui` は互換 alias です。

```bash
cargo run -- wizard
cargo run -- ask
cargo run -- tui
```

### Safety rules

- 既存の通常ファイルは上書きしません。
- `add` は失敗時に dependency config を rollback します。
- `remove` は sksync が管理している symlink だけを削除します。
- `remove` が installed files を削除するのは configured `skillDir` 配下にある場合だけです。
- Git source の subpath は absolute path / `..` を拒否し、clone directory 外へ出ないことを確認します。
- project scope の agent targetDir は project root 外へ出られません。
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

npm-like な依存管理コマンド体系をベースに、source integration と portability を広げていきます。

- Additional source URL transformers beyond `skills.sh`
- GitLab / gist support
- Lockfile sharing policy の確定
- Cross-platform symlink / junction behavior の強化

`ci` 相当の専用コマンドは現時点では追加しません。lockfile 再現は `sksync install` に集約します。

詳細は以下を参照してください。

- [`docs/DESIGN.md`](docs/DESIGN.md) - 機能設計
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - アーキテクチャ設計原則
- [`docs/ROADMAP.md`](docs/ROADMAP.md) - 開発ロードマップ
