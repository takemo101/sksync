# Install

sksync ships as a single binary. The recommended path is the install script for macOS and Linux; building from source works on any platform Rust supports.

## Requirements

- **macOS** (Apple Silicon or Intel) or **Linux** (x86_64 / aarch64) for prebuilt binaries.
- For other platforms, a **[Rust](https://www.rust-lang.org/tools/install) toolchain** (`cargo`) to build from source.
- `git` on `PATH` — sksync delegates all repository access (including private repos) to your local Git auth.

## From the install script (macOS / Linux, recommended)

Installs the latest release binary to `~/.local/bin/sksync`:

```sh
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | sh
```

Pick a different install directory:

```sh
curl -fsSL https://raw.githubusercontent.com/takemo101/sksync/main/install.sh | INSTALL_DIR=/usr/local/bin sh
```

The script verifies the release checksum when `sha256sum` or `shasum` is available and warns (rather than failing) if the checksum file is missing.

Supported prebuilt targets:

| OS | Architecture | Asset target |
|---|---|---|
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| macOS | Intel | `x86_64-apple-darwin` |
| Linux | x86_64 / amd64 | `x86_64-unknown-linux-musl` |
| Linux | arm64 / aarch64 | `aarch64-unknown-linux-musl` |

Linux uses musl assets by default for portability across Debian / Ubuntu distributions. CI smoke-tests the x86_64 Linux binary in `debian:bookworm`, `debian:trixie`, `ubuntu:22.04`, and `ubuntu:24.04`. Linux assets are available from releases that include this support; if `latest` predates them, build from source or set `VERSION=v...` to a newer tag. Windows is not supported yet.

::: tip
If `~/.local/bin` is not on your `PATH`, the installer prints a warning. Add it to your shell profile, e.g. `export PATH="$HOME/.local/bin:$PATH"`.
:::

## From source

Clone and build with Cargo:

```sh
git clone https://github.com/takemo101/sksync
cd sksync
cargo build --release        # binary at target/release/sksync
```

Install into `INSTALL_DIR` from a clone with the bundled `justfile`:

```sh
just install
# or choose the location
INSTALL_DIR=/usr/local/bin just install
```

To run commands without installing, use `cargo run --`:

```sh
cargo run -- --help
cargo run -- init
```

::: info
Throughout these docs commands are written as `sksync <command>` for an installed binary. From a clone, the equivalent is `cargo run -- <command>` or `./target/debug/sksync <command>`.
:::

## Verify

```sh
sksync --help
sksync --version
```

## Uninstall

Remove the binary installed by the script:

```sh
rm -f ~/.local/bin/sksync
```

If you used a custom `INSTALL_DIR`, delete the binary from that location instead:

```sh
rm -f /usr/local/bin/sksync
```

From a clone, `just uninstall` removes the binary from the same `INSTALL_DIR`:

```sh
just uninstall
# or
INSTALL_DIR=/usr/local/bin just uninstall
```

To fully reset — removing global config, agent mappings, and installed global skills — also delete `~/.sksync`:

```sh
rm -f ~/.local/bin/sksync
rm -rf ~/.sksync
```

## What gets created

| Path | Scope | Purpose |
|---|---|---|
| `sksync.config.json` | project | Dependencies, `skillDir`, `defaultAgents`, optional inline `agents` override. The file you share. |
| `.sksync/skills/<skill>/` | project | Downloaded/copied skill bodies. Git-ignored. |
| `sksync-lock.json` | project | Portable lockfile v4 — resolved sources and file hashes. Git-ignored by default. |
| `~/.sksync/config.json` | global | Global dependencies (`--global`). |
| `~/.sksync/agents.json` | global | Agent target directory mappings (global + project). |
| `~/.sksync/skills/<skill>/` | global | Globally installed skill bodies. |

## Next

- [Quickstart](/quickstart) — add your first skill and sync it.
- [Project Config](/guides/project-config) — the shape of `sksync.config.json`.
- [Agent Mappings](/guides/agent-mappings) — where each agent's skills live.
