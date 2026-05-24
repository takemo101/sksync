# Linux Distro Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make sksync installable and smoke-tested on common Debian/Ubuntu Linux distributions via release assets, Docker verification, installer support, and documentation.

**Architecture:** Keep the Rust application unchanged and add platform support at the packaging boundary. Release builds produce macOS assets as before plus Linux x86_64/aarch64 musl assets for broad distro compatibility, while a Docker smoke workflow validates the x86_64 Linux binary against Debian and Ubuntu images using local skill sources and symlink operations.

**Tech Stack:** GitHub Actions, Rust stable targets, Docker, POSIX shell installer, existing README/design/site docs.

---

### Task 1: Release Linux assets

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] Add a Linux build job that runs on `ubuntu-latest` and builds `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` release binaries.
- [ ] Package each Linux binary as `dist/sksync-<target>.tar.gz`, matching the existing macOS asset naming convention.
- [ ] Make `publish` depend on both macOS and Linux build jobs so checksums include all assets.

### Task 2: Docker smoke workflow

**Files:**
- Create: `.github/workflows/linux-smoke.yml`

- [ ] Build the `x86_64-unknown-linux-musl` release binary once.
- [ ] Run the binary inside `debian:bookworm`, `debian:trixie`, `ubuntu:22.04`, and `ubuntu:24.04` containers.
- [ ] In each container, use a temp HOME/project, create a local `SKILL.md`, and run `init`, `add`, `plan --dry-run`, `apply`, `check`, `list`, and `remove`.
- [ ] Assert `.agents/skills/<skill>` is a symlink after add/apply and removed after remove.

### Task 3: Linux installer support

**Files:**
- Modify: `install.sh`

- [ ] Detect `Linux` in addition to `Darwin`.
- [ ] Map `x86_64|amd64` to `x86_64-unknown-linux-musl` on Linux.
- [ ] Map `arm64|aarch64` to `aarch64-unknown-linux-musl` on Linux.
- [ ] Verify checksums with `sha256sum` when available, otherwise `shasum -a 256`.
- [ ] Keep macOS behavior and asset names unchanged.

### Task 4: Documentation and design updates

**Files:**
- Modify: `README.md`
- Modify: `docs/DESIGN.md`
- Modify: `docs/ROADMAP.md`
- Modify: `site/install.md`
- Modify: `site/guides/lockfile.md`

- [ ] Update install docs from macOS-only to macOS/Linux.
- [ ] Document supported Linux release targets and Debian/Ubuntu Docker smoke coverage.
- [ ] State that Linux installer defaults to musl assets for distro portability.
- [ ] Keep Windows explicitly out of scope for now.

### Task 5: Verification and PR

**Files:**
- All changed files

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --quiet`.
- [ ] Run `cargo build --release --quiet`.
- [ ] Run `cargo clippy --quiet -- -D warnings`.
- [ ] Review uncommitted diff to ensure unrelated workspace files are excluded.
- [ ] Commit, open PR, merge, and sync GitButler workspace.
